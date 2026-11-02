import Foundation
import CryptoKit
import Network
import Security

/// Abgleich der Bibliothek mit dem Hub (Epic #210, P6), nach
/// docs/superpowers/specs/2026-11-02-bibliothek-sync.md. Gleiches Konto und
/// gleicher Schlüssel wie die Desktop-App; der Server sieht nur Chiffrat.
@MainActor
final class LibrarySync: ObservableObject {
    static let shared = LibrarySync()
    static let defaultHub = "https://portal.wolffappliedai.de"

    struct Account: Codable {
        var url: String
        var email: String
        var userId: Int64
        var token: String
        var key: Data
        var deviceId: String
        var deviceName: String
        var keyId: String { SyncCrypto.keyId([UInt8](key)) }
    }

    enum SyncError: LocalizedError {
        case http(Int, String), notLoggedIn, wrongKey, offlineRule, corrupt(String)
        var errorDescription: String? {
            switch self {
            case .http(let code, let msg): return msg.isEmpty ? "Der Server antwortet mit Fehler \(code)." : msg
            case .notLoggedIn: return "Nicht angemeldet."
            case .wrongKey: return "Daten im Konto stammen von einem anderen Passwort. Bitte ab- und neu anmelden."
            case .offlineRule: return "Hörbücher werden nur im WLAN geladen (Einstellung in „Konto & Sync“)."
            case .corrupt(let what): return "Datei im Konto beschädigt: \(what)."
            }
        }
    }

    @Published private(set) var account: Account?
    @Published private(set) var running = false
    @Published private(set) var status = "Nicht angemeldet"
    @Published private(set) var lastSync: Date?
    @Published var error: String?
    /// Ladefortschritt je Buch (0…1) während eines Hörbuch-Downloads.
    @Published private(set) var downloads: [String: Double] = [:]
    @Published var wifiOnly: Bool = UserDefaults.standard.object(forKey: "library.wifiOnly") as? Bool ?? true {
        didSet { UserDefaults.standard.set(wifiOnly, forKey: "library.wifiOnly") }
    }
    @Published private(set) var expensiveNetwork = false

    private let library: PhoneLibrary
    private var ledger = SyncLedger()
    private let monitor = NWPathMonitor()
    private var again = false

    init(library: PhoneLibrary? = nil) {
        self.library = library ?? .shared
        account = Keychain.load()
        ledger = (try? JSONDecoder().decode(SyncLedger.self, from: Data(contentsOf: ledgerURL))) ?? SyncLedger()
        status = account == nil ? "Nicht angemeldet" : "Angemeldet als \(account!.email)"
        monitor.pathUpdateHandler = { [weak self] path in
            let expensive = path.isExpensive || path.isConstrained
            Task { @MainActor in self?.expensiveNetwork = expensive }
        }
        monitor.start(queue: DispatchQueue(label: "library.sync.path"))
    }

    private var ledgerURL: URL { library.root.appendingPathComponent("sync-ledger.json") }
    private func saveLedger() { try? JSONEncoder().encode(ledger).write(to: ledgerURL, options: .atomic) }

    // MARK: Konto

    func login(email: String, password: String, url: String, deviceName: String) async {
        error = nil
        let mail = email.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let base = url.trimmingCharacters(in: .whitespacesAndNewlines).trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        guard !mail.isEmpty, !password.isEmpty else { error = "E-Mail und Passwort eingeben."; return }
        #if DEBUG
        let allowed = base.hasPrefix("https://") || base.hasPrefix("http://127.0.0.1")  // lokaler Test-Hub im Simulator
        #else
        let allowed = base.hasPrefix("https://")
        #endif
        guard allowed else { error = "Die Server-Adresse muss mit https:// beginnen."; return }
        running = true; status = "Schlüssel wird berechnet …"
        defer { running = false }
        // Argon2id mit 64 MiB: nicht auf dem Haupt-Thread.
        let key = await Task.detached(priority: .userInitiated) { SyncCrypto.deriveKey(password: password, email: mail) }.value
        status = "Anmeldung …"
        do {
            let body = try JSONSerialization.data(withJSONObject: ["email": mail, "password": password, "device_name": deviceName])
            let (code, data) = try await Self.send(base + "/api/v1/voice/auth/login", method: "POST", token: nil, body: body)
            guard code == 200, let json = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let token = json["token"] as? String, let user = json["user"] as? [String: Any],
                  let uid = (user["id"] as? NSNumber)?.int64Value else { throw Self.httpError(code, data) }
            var id = [UInt8](repeating: 0, count: 16)
            _ = SecRandomCopyBytes(kSecRandomDefault, 16, &id)
            let acc = Account(url: base, email: mail, userId: uid, token: token, key: Data(key),
                              deviceId: id.map { String(format: "%02x", $0) }.joined(), deviceName: deviceName)
            Keychain.save(acc)
            account = acc
            ledger = SyncLedger(); saveLedger()
            status = "Angemeldet als \(mail)"
            // Erst nach dem Ende dieser Anmeldung (running wird im defer frei).
            Task { await self.run() }
        } catch {
            self.error = error.localizedDescription
            status = "Nicht angemeldet"
        }
    }

    func logout() async {
        if let acc = account { _ = try? await Self.send(acc.url + "/api/v1/voice/auth/logout", method: "POST", token: acc.token, body: Data("{}".utf8)) }
        Keychain.delete()
        account = nil
        ledger = SyncLedger(); saveLedger()
        status = "Nicht angemeldet"
    }

    // MARK: Abgleich

    /// Ein Lauf: holen, anwenden, eigene Änderungen hochladen und senden.
    func run() async {
        guard let acc = account else { return }
        if running { again = true; return }
        running = true; error = nil
        defer {
            running = false
            if again { again = false; Task { await self.run() } }
        }
        do {
            status = "Abgleich …"
            try await pull(acc)
            try await pushBooks(acc)
            try await pushProgress(acc)
            saveLedger()
            library.reload()
            lastSync = Date()
            status = "Synchronisiert"
        } catch SyncError.http(401, _) {
            error = "Die Anmeldung ist abgelaufen oder wurde widerrufen. Bitte neu anmelden."
            status = "Abgemeldet"
        } catch {
            saveLedger()
            self.error = error.localizedDescription
            status = "Abgleich unterbrochen"
        }
    }

    private func pull(_ acc: Account) async throws {
        var more = true
        while more {
            let (code, data) = try await Self.send(acc.url + "/api/v1/voice/sync/pull?since=\(ledger.cursor)&limit=100", method: "GET", token: acc.token)
            if code == 410 { ledger.cursor = 0; continue }
            guard code == 200 else { throw Self.httpError(code, data) }
            let page = try JSONDecoder().decode(HubPullResponse.self, from: data)
            for remote in page.objects { try apply(remote, acc) }
            if page.cursor <= ledger.cursor && page.more { break }
            ledger.cursor = page.cursor
            more = page.more
        }
    }

    private func apply(_ remote: HubRemoteObject, _ acc: Account) throws {
        let collections = [LibrarySyncFormat.collectionBook, LibrarySyncFormat.collectionProgress]
        guard collections.contains(remote.collection) else { return }
        if remote.deleted {
            if remote.collection == LibrarySyncFormat.collectionBook { library.removeRemote(id: remote.objectId) }
            ledger[remote.collection, remote.objectId] = .init(revision: remote.revision, hash: "", deleted: true)
            return
        }
        guard remote.keyId == acc.keyId else { throw SyncError.wrongKey }
        let plain = Data(try SyncCrypto.open(key: [UInt8](acc.key), aad: SyncCrypto.aad(userId: acc.userId, collection: remote.collection, objectId: remote.objectId),
                                             sealedBase64: remote.payload))
        let hash = LibrarySyncFormat.canonicalHash(plain)
        switch remote.collection {
        case LibrarySyncFormat.collectionBook:
            let synced = try SyncedBook.decode(plain)
            let local = library.books.first { $0.id == synced.book.id }
            let localMeta = local.map { library.meta($0) }
            if let localMeta, localMeta.updated > synced.updated {
                // Eigener Stand ist neuer: bleibt, wird beim Senden übertragen.
            } else {
                var book = synced.book
                // Lokale Fassungen, die das Konto nicht kennt, nicht verlieren.
                if book.text == nil, let t = local?.text { book.text = t }
                try library.storeRemote(book, meta: .init(files: synced.files, updated: synced.updated))
                if let text = synced.files.text, !library.hasText(book) { pendingText.append((book, text)) }
            }
        case LibrarySyncFormat.collectionProgress:
            let p = try JSONDecoder().decode(SyncedProgress.self, from: plain)
            library.mergeRemote(p.key, p.record)
        default: break
        }
        ledger[remote.collection, remote.objectId] = .init(revision: remote.revision, hash: hash, deleted: false)
    }

    /// Texte sind klein und werden nach dem Pull sofort geladen.
    private var pendingText: [(LibraryBook, BlobRef)] = []

    private func pushBooks(_ acc: Account) async throws {
        for (book, ref) in pendingText {
            let data = try await downloadBlob(ref, acc) { _ in }
            try data.write(to: library.textURL(book), options: .atomic)
        }
        pendingText.removeAll()

        var objects: [(HubPushObject, String)] = []
        for book in library.books {
            var meta = library.meta(book)
            // Eigene Dateien zuerst hochladen (Reihenfolge laut Spezifikation).
            if meta.files.text == nil, library.hasText(book) {
                meta.files.text = try await uploadFile(library.textURL(book), acc, type: "txt", file: nil)
                library.setMeta(book, meta)
            }
            if meta.files.audio.isEmpty, library.hasAudio(book), let tracks = book.audio?.allTracks {
                var refs: [BlobRef] = []
                for (t, url) in zip(tracks, library.audioURLs(book)) {
                    status = "Lade „\(book.title)“ hoch …"
                    refs.append(try await uploadFile(url, acc, type: nil, file: t.file))
                }
                meta.files.audio = refs
                library.setMeta(book, meta)
            }
            let value = try SyncedBook(book: book, files: meta.files, updated: meta.updated).json()
            let hash = LibrarySyncFormat.canonicalHash(value)
            let entry = ledger[LibrarySyncFormat.collectionBook, book.id]
            if entry?.hash == hash { continue }
            objects.append((try sealObject(LibrarySyncFormat.collectionBook, book.id, value, base: entry?.revision ?? 0, acc), hash))
        }
        try await push(objects, acc)
    }

    private func pushProgress(_ acc: Account) async throws {
        var objects: [(HubPushObject, String)] = []
        for key in library.progressKeys {
            guard let rec = library.record(key) else { continue }
            let id = LibrarySyncFormat.progressObjectId(key)
            let value = try JSONEncoder().encode(SyncedProgress(key: key, record: rec))
            let hash = LibrarySyncFormat.canonicalHash(value)
            let entry = ledger[LibrarySyncFormat.collectionProgress, id]
            if entry?.hash == hash { continue }
            objects.append((try sealObject(LibrarySyncFormat.collectionProgress, id, value, base: entry?.revision ?? 0, acc), hash))
        }
        try await push(objects, acc)
    }

    private func sealObject(_ collection: String, _ id: String, _ value: Data, base: Int64, _ acc: Account) throws -> HubPushObject {
        let payload = try SyncCrypto.seal(key: [UInt8](acc.key), aad: SyncCrypto.aad(userId: acc.userId, collection: collection, objectId: id),
                                          plaintext: [UInt8](value))
        return HubPushObject(collection: collection, objectId: id, baseRevision: base, payload: payload, keyId: acc.keyId, deleted: false)
    }

    private func push(_ objects: [(HubPushObject, String)], _ acc: Account) async throws {
        var start = 0
        while start < objects.count {
            // Höchstens 20 Objekte und ~400 KB je Anfrage (Hub-Grenze 512 KB).
            var end = start, bytes = 0
            while end < objects.count, end - start < 20, bytes + objects[end].0.payload.count < 400_000 || end == start {
                bytes += objects[end].0.payload.count; end += 1
            }
            let batch = Array(objects[start..<end])
            let body = try JSONSerialization.data(withJSONObject: [
                "device_id": acc.deviceId,
                "objects": try batch.map { try JSONSerialization.jsonObject(with: JSONEncoder().encode($0.0)) },
            ])
            let (code, data) = try await Self.send(acc.url + "/api/v1/voice/sync/push", method: "POST", token: acc.token, body: body)
            if code == 503 { throw SyncError.http(503, "Der Server ist gerade belegt; der nächste Abgleich holt es nach.") }
            guard code == 200 else { throw Self.httpError(code, data) }
            struct Response: Decodable { var results: [HubPushResult] }
            for r in try JSONDecoder().decode(Response.self, from: data).results {
                guard let sent = batch.first(where: { $0.0.collection == r.collection && $0.0.objectId == r.objectId }) else { continue }
                switch r.outcome {
                case "accepted":
                    ledger[sent.0.collection, sent.0.objectId] = .init(revision: r.revision ?? 0, hash: sent.1, deleted: false)
                case "superseded":
                    if let current = r.current { try apply(current, acc) }
                    else { ledger[sent.0.collection, sent.0.objectId] = nil }
                    again = true
                default: break
                }
            }
            start = end
        }
    }

    // MARK: Dateien

    /// Datei verschlüsselt in Stücken hochladen; bekannte Blobs werden übersprungen.
    private func uploadFile(_ url: URL, _ acc: Account, type: String?, file: String?) async throws -> BlobRef {
        let key = [UInt8](acc.key)
        let (size, digest) = try await Task.detached(priority: .utility) { () throws -> (Int, Data) in
            let h = try FileHandle(forReadingFrom: url); defer { try? h.close() }
            var hasher = SHA256(); var size = 0
            while let chunk = try h.read(upToCount: 1 << 20), !chunk.isEmpty { hasher.update(data: chunk); size += chunk.count }
            return (size, Data(hasher.finalize()))
        }.value
        let blob = LibrarySyncFormat.blobId(key: key, plaintextSha256: digest)
        let chunks = max(1, (size + LibrarySyncFormat.chunkPlainBytes - 1) / LibrarySyncFormat.chunkPlainBytes)
        let ref = BlobRef(blob: blob, size: size, chunks: chunks, type: type, file: file)
        if ledger.uploaded.contains(blob) { return ref }
        let (infoCode, infoData) = try await Self.send(acc.url + "/api/v1/voice/blobs/\(blob)", method: "GET", token: acc.token)
        var have = Set<Int>()
        if infoCode == 200, let info = try JSONSerialization.jsonObject(with: infoData) as? [String: Any] {
            if info["complete"] as? Bool == true { ledger.uploaded.insert(blob); return ref }
            have = Set((info["chunks"] as? [[String: Any]] ?? []).compactMap { ($0["idx"] as? NSNumber)?.intValue })
        }
        for index in 0..<chunks where !have.contains(index) {
            let sealed = try await Task.detached(priority: .utility) { () throws -> [UInt8] in
                let h = try FileHandle(forReadingFrom: url); defer { try? h.close() }
                try h.seek(toOffset: UInt64(index * LibrarySyncFormat.chunkPlainBytes))
                let plain = try h.read(upToCount: LibrarySyncFormat.chunkPlainBytes) ?? Data()
                return try LibrarySyncFormat.sealChunk(key: key, userId: acc.userId, blobId: blob, index: index, plain: [UInt8](plain))
            }.value
            let body = try JSONSerialization.data(withJSONObject: [
                "data": Data(sealed).base64EncodedString(),
                "sha256": SHA256.hash(data: Data(sealed)).map { String(format: "%02x", $0) }.joined(),
            ])
            let (code, data) = try await Self.send(acc.url + "/api/v1/voice/blobs/\(blob)/chunks/\(index)", method: "POST", token: acc.token, body: body)
            guard code == 200 else { throw Self.httpError(code, data) }
        }
        let (code, data) = try await Self.send(acc.url + "/api/v1/voice/blobs/\(blob)/commit", method: "POST", token: acc.token,
                                               body: try JSONSerialization.data(withJSONObject: ["chunks": chunks]))
        guard code == 200 else { throw Self.httpError(code, data) }
        ledger.uploaded.insert(blob); saveLedger()
        return ref
    }

    /// Blob laden, Stück für Stück prüfen und entschlüsseln.
    private func downloadBlob(_ ref: BlobRef, _ acc: Account, progress: (Int) -> Void) async throws -> Data {
        var out = Data(); out.reserveCapacity(ref.size)
        for index in 0..<ref.chunks {
            let (code, data) = try await Self.send(acc.url + "/api/v1/voice/blobs/\(ref.blob)/chunks/\(index)", method: "GET", token: acc.token)
            guard code == 200, let json = try JSONSerialization.jsonObject(with: data) as? [String: String],
                  let raw = json["data"].flatMap({ Data(base64Encoded: $0) }) else { throw Self.httpError(code, data) }
            guard SHA256.hash(data: raw).map({ String(format: "%02x", $0) }).joined() == json["sha256"] else { throw SyncError.corrupt("Prüfsumme") }
            let plain = try LibrarySyncFormat.openChunk(key: [UInt8](acc.key), userId: acc.userId, blobId: ref.blob, index: index, sealed: [UInt8](raw))
            out.append(contentsOf: plain)
            progress(out.count)
        }
        guard out.count == ref.size else { throw SyncError.corrupt("Größe") }
        return out
    }

    /// Hörbuch beim ersten Öffnen laden. Teile werden einzeln geprüft und atomar abgelegt.
    func downloadAudio(_ book: LibraryBook) async -> Bool {
        guard let acc = account else { error = SyncError.notLoggedIn.localizedDescription; return false }
        let refs = library.meta(book).files.audio
        guard !refs.isEmpty else { return library.hasAudio(book) }
        if wifiOnly && expensiveNetwork { error = SyncError.offlineRule.localizedDescription; return false }
        let total = max(1, refs.reduce(0) { $0 + $1.size })
        var done = 0
        downloads[book.id] = 0
        defer { downloads[book.id] = nil }
        do {
            let audioDir = library.dir(book).appendingPathComponent("audio", isDirectory: true)
            try FileManager.default.createDirectory(at: audioDir, withIntermediateDirectories: true)
            for ref in refs {
                let name = ref.file ?? "teil.m4a"
                let target = audioDir.appendingPathComponent(name)
                if FileManager.default.fileExists(atPath: target.path),
                   (try? target.resourceValues(forKeys: [.fileSizeKey]).fileSize) == ref.size { done += ref.size; continue }
                let base = done
                let data = try await downloadBlob(ref, acc) { n in self.downloads[book.id] = Double(base + n) / Double(total) }
                try data.write(to: target, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
                done += ref.size
            }
            library.reload()
            return library.hasAudio(book)
        } catch {
            self.error = "Laden fehlgeschlagen: \(error.localizedDescription) Bereits geladene Teile bleiben erhalten."
            return false
        }
    }

    /// Geladene Dateien vom iPhone entfernen; Eintrag und Fortschritt bleiben im Konto.
    func removeDownloads(_ book: LibraryBook) {
        try? FileManager.default.removeItem(at: library.dir(book).appendingPathComponent("audio"))
        library.reload()
    }

    // MARK: HTTP

    static func send(_ url: String, method: String, token: String?, body: Data? = nil) async throws -> (Int, Data) {
        guard let u = URL(string: url) else { throw SyncError.http(0, "Ungültige Server-Adresse.") }
        var req = URLRequest(url: u, timeoutInterval: 60)
        req.httpMethod = method
        req.setValue("application/json", forHTTPHeaderField: "Accept")
        if let body { req.httpBody = body; req.setValue("application/json", forHTTPHeaderField: "Content-Type") }
        if let token { req.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization") }
        let (data, response) = try await URLSession.shared.data(for: req)
        return ((response as? HTTPURLResponse)?.statusCode ?? 0, data)
    }

    static func httpError(_ code: Int, _ data: Data) -> SyncError {
        let message = ((try? JSONSerialization.jsonObject(with: data)) as? [String: Any])
            .flatMap { $0["error"] as? [String: Any] }.flatMap { $0["message"] as? String } ?? ""
        return .http(code, code == 404 && message.isEmpty ? "Der Server kennt die Bibliothek-Ablage noch nicht." : message)
    }
}

/// Konto und Schlüssel im Schlüsselbund, nur auf diesem Gerät.
enum Keychain {
    private static let service = "de.localvoice.library-sync"
    static func save(_ account: LibrarySync.Account) {
        guard let data = try? JSONEncoder().encode(account) else { return }
        delete()
        SecItemAdd([kSecClass: kSecClassGenericPassword, kSecAttrService: service, kSecAttrAccount: "account",
                    kSecAttrAccessible: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly, kSecValueData: data] as CFDictionary, nil)
    }
    static func load() -> LibrarySync.Account? {
        var out: CFTypeRef?
        guard SecItemCopyMatching([kSecClass: kSecClassGenericPassword, kSecAttrService: service, kSecAttrAccount: "account",
                                   kSecReturnData: true] as CFDictionary, &out) == errSecSuccess, let data = out as? Data else { return nil }
        return try? JSONDecoder().decode(LibrarySync.Account.self, from: data)
    }
    static func delete() {
        SecItemDelete([kSecClass: kSecClassGenericPassword, kSecAttrService: service, kSecAttrAccount: "account"] as CFDictionary)
    }
}
