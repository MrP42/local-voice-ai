import Foundation
import CryptoKit
import Network
import Security

/// Abgleich der Bibliothek mit dem Hub (Epic #210, P6), nach
/// docs/superpowers/specs/2026-11-02-bibliothek-sync.md. Gleiches Konto und
/// gleicher Schlüssel wie die Desktop-App; der Server sieht nur Chiffrat.
/// Anfragen an den Hub folgen keiner Umleitung: Sonst gingen Token, Schlüsseldaten oder der
/// Kopplungs-Claim an einen Server, den niemand bestätigt hat. Die 3xx-Antwort gilt als Fehler.
final class NoRedirects: NSObject, URLSessionTaskDelegate, Sendable {
    static let shared = NoRedirects()

    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest) async -> URLRequest? { nil }
}

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
    @Published var error: String? {
        didSet {
            #if DEBUG
            // Diagnose für Simulator-Proben: letzter Fehler als Datei.
            if let error { try? error.write(to: library.root.appendingPathComponent("sync-letzter-fehler.txt"), atomically: true, encoding: .utf8) }
            #endif
        }
    }
    @Published var wifiOnly: Bool = UserDefaults.standard.object(forKey: "library.wifiOnly") as? Bool ?? true {
        didSet {
            UserDefaults.standard.set(wifiOnly, forKey: "library.wifiOnly")
            if wifiOnly != oldValue { AudioDownloads.shared.networkRuleChanged() }
        }
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
            guard SecRandomCopyBytes(kSecRandomDefault, 16, &id) == errSecSuccess else { throw SyncError.http(0, "Zufallszahlen nicht verfügbar.") }
            let acc = Account(url: base, email: mail, userId: uid, token: token, key: Data(key),
                              deviceId: id.map { String(format: "%02x", $0) }.joined(), deviceName: deviceName)
            guard Keychain.save(acc) else { throw SyncError.http(0, "Das Konto konnte nicht im Schlüsselbund gespeichert werden.") }
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

    /// Konto aus der Geräte-Kopplung übernehmen: wie `login`, nur ohne Passwort.
    func adopt(_ paired: PairedAccount) throws {
        guard account == nil else { throw PairingError.adoptFailed("Dieses iPhone ist schon angemeldet.") }
        var id = [UInt8](repeating: 0, count: 16)
        guard SecRandomCopyBytes(kSecRandomDefault, 16, &id) == errSecSuccess else { throw PairingError.adoptFailed("Zufallszahlen nicht verfügbar.") }
        let mail = paired.email.lowercased()
        let acc = Account(url: paired.hub, email: mail, userId: paired.userId, token: paired.token, key: Data(paired.key),
                          deviceId: id.map { String(format: "%02x", $0) }.joined(), deviceName: paired.deviceName)
        guard Keychain.save(acc) else { throw PairingError.adoptFailed("Das Konto konnte nicht im Schlüsselbund gespeichert werden.") }
        account = acc
        error = nil
        ledger = SyncLedger(); saveLedger()
        status = "Angemeldet als \(mail)"
        Task { await self.run() }
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
            library.flush()
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
        var more = true, restarted = false
        while more {
            let (code, data) = try await Self.send(acc.url + "/api/v1/voice/sync/pull?since=\(ledger.cursor)&limit=100", method: "GET", token: acc.token)
            if code == 410 && !restarted { ledger.cursor = 0; restarted = true; continue }
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
            if remote.collection == LibrarySyncFormat.collectionBook, let local = library.books.first(where: { $0.id == remote.objectId }) {
                let hash = try localHash(local)
                // Ungesicherte Änderung: Buch behalten, der nächste Lauf sendet es über den Grabstein hinweg neu.
                if !SyncLedger.keepsAgainstTombstone(ledger[remote.collection, remote.objectId], localHash: hash) { library.removeRemote(id: remote.objectId) }
            }
            ledger[remote.collection, remote.objectId] = .init(revision: remote.revision, hash: "", deleted: true)
            return
        }
        // Eigenes Echo (Desktop: engine.rs): Stand ist schon im Ledger.
        if remote.deviceId == acc.deviceId, let e = ledger[remote.collection, remote.objectId], e.revision >= remote.revision { return }
        guard remote.keyId == acc.keyId else { throw SyncError.wrongKey }
        let plain = Data(try SyncCrypto.open(key: [UInt8](acc.key), aad: SyncCrypto.aad(userId: acc.userId, collection: remote.collection, objectId: remote.objectId),
                                             sealedBase64: remote.payload))
        // Ledger-Hash = Hash der LOKALEN kanonischen Form nach dem Anwenden; sonst
        // schickt jedes Gerät den gerade erhaltenen Stand in eigener Schreibweise zurück.
        var hash = LibrarySyncFormat.canonicalHash(plain)
        switch remote.collection {
        case LibrarySyncFormat.collectionBook:
            let synced = try SyncedBook.decode(plain)
            let local = library.books.first { $0.id == synced.book.id }
            let localMeta = local.map { library.meta($0) }
            if let local, let localMeta, localMeta.updated > synced.updated {
                // Eigener Stand ist neuer und wird beim Senden übertragen. War es nur ein Stern, gilt sonst das Konto.
                if let favorite = localMeta.pendingFavorite,
                   SyncLedger.onlyFavoriteChanged(pendingFields: localMeta.pendingFields, localFields: try SyncLedger.fieldsHash(local, files: localMeta.files)) {
                    let book = SyncLedger.mergingPendingFavorite(favorite, into: synced.book, local: local)
                    let fields = try SyncLedger.fieldsHash(book, files: synced.files)
                    try library.storeRemote(book, meta: .init(files: synced.files, updated: localMeta.updated, pendingFavorite: favorite, pendingFields: fields))
                }
            } else {
                var book = synced.book
                // Lokale Fassungen, die das Konto nicht kennt, nicht verlieren.
                if book.text == nil, let t = local?.text { book.text = t }
                try library.storeRemote(book, meta: .init(files: synced.files, updated: synced.updated))
                if book.text == synced.book.text {
                    hash = LibrarySyncFormat.canonicalHash(try SyncedBook(book: book, files: synced.files, updated: synced.updated).json())
                }
            }
        case LibrarySyncFormat.collectionProgress:
            let p = try JSONDecoder().decode(SyncedProgress.self, from: plain)
            library.mergeRemote(p.key, p.record)
            // Unverändert gegenüber dem Konto: lokale Schreibweise merken, sonst neu senden lassen.
            if let merged = library.record(p.key), merged == p.record {
                hash = LibrarySyncFormat.canonicalHash(try JSONEncoder().encode(SyncedProgress(key: p.key, record: merged)))
            }
        default: break
        }
        ledger[remote.collection, remote.objectId] = .init(revision: remote.revision, hash: hash, deleted: false)
    }

    private func pushBooks(_ acc: Account) async throws {
        // Texte sind klein: bei jedem Lauf fehlende nachladen (überlebt Abbrüche).
        for book in library.books {
            if let ref = library.meta(book).files.text, !library.hasText(book) {
                try await downloadBlob(ref, acc, to: library.textURL(book)) { _ in }
            }
            // Kurzbücher sind klein: wie Texte gleich mitnehmen.
            for ref in library.meta(book).files.kurzbuecher where !FileManager.default.fileExists(atPath: library.kurzbuchFile(book, ref).path) {
                let target = library.kurzbuchFile(book, ref)
                try? FileManager.default.createDirectory(at: target.deletingLastPathComponent(), withIntermediateDirectories: true)
                do { try await downloadBlob(ref, acc, to: target) { _ in } }
                catch SyncError.http(401, let msg) { throw SyncError.http(401, msg) }
                catch { /* Nächster Lauf versucht es erneut. */ }
            }
            // Cover wie am Desktop: neues oder geändertes Cover aus dem Konto holen.
            if let ref = library.meta(book).files.cover, library.coverBlob(book) != ref.blob {
                do { try await downloadCover(ref, book, acc) }
                catch SyncError.http(401, let msg) { throw SyncError.http(401, msg) }
                catch { /* Erzeugtes Cover bleibt; nächster Lauf versucht es erneut. */ }
            }
        }

        var objects: [(HubPushObject, String)] = []
        for book in library.books {
            var meta = library.meta(book)
            // Eigene Dateien zuerst hochladen (Reihenfolge laut Spezifikation).
            if meta.files.text == nil, library.hasText(book) {
                let ref = try await uploadFile(library.textURL(book), acc, type: "txt", file: nil)
                library.updateFiles(book) { $0.text = ref }
                meta = library.meta(book)
            }
            // Eigenes Cover (aus Hörbuchdatei oder EPUB) für die anderen Geräte hochladen.
            if meta.files.cover == nil, let url = library.coverURL(book) {
                let ref = try await uploadFile(url, acc, type: url.pathExtension.lowercased(), file: nil)
                library.updateFiles(book) { $0.cover = ref }
                meta = library.meta(book)
                library.setCoverBlob(book, ref.blob)
            }
            if meta.files.audio.isEmpty, library.hasAudio(book), let tracks = book.audio?.allTracks {
                var refs: [BlobRef] = []
                for (t, url) in zip(tracks, library.audioURLs(book)) {
                    status = "Lade „\(book.title)“ hoch …"
                    refs.append(try await uploadFile(url, acc, type: nil, file: t.file))
                }
                meta = library.meta(book)
                meta.files.audio = refs
                meta.localAudio = Dictionary(library.audioParts(refs).map { ($0.file, $0.blob) }, uniquingKeysWith: { $1 })
                library.setMeta(book, meta)
            }
            // Während der Uploads kann sich der Stand geändert haben (z. B. ein Stern).
            let current = library.books.first { $0.id == book.id } ?? book
            meta = library.meta(current)
            let value = try SyncedBook(book: current, files: meta.files, updated: meta.updated).json()
            let hash = LibrarySyncFormat.canonicalHash(value)
            let entry = ledger[LibrarySyncFormat.collectionBook, book.id]
            if entry?.hash == hash || ledger.dead[SyncLedger.key(LibrarySyncFormat.collectionBook, book.id)] == hash { continue }
            if value.count > LibrarySyncFormat.maxPlaintext { markDead(LibrarySyncFormat.collectionBook, book.id, hash, "„\(book.title)“ ist zu groß für den Abgleich."); continue }
            objects.append((try sealObject(LibrarySyncFormat.collectionBook, book.id, value, base: entry?.revision ?? 0, acc), hash))
        }
        try await push(objects, acc)
        for (object, hash) in objects {
            guard let book = library.books.first(where: { $0.id == object.objectId }), let local = try? localHash(book),
                  SyncLedger.clearsPendingFavorite(accepted: ledger[object.collection, object.objectId]?.hash, sent: hash, local: local) else { continue }
            library.clearPendingFavorite(bookId: book.id)
        }
    }

    private func localHash(_ book: LibraryBook) throws -> String {
        let meta = library.meta(book)
        return LibrarySyncFormat.canonicalHash(try SyncedBook(book: book, files: meta.files, updated: meta.updated).json())
    }

    /// Stern setzen. War der Stand abgeglichen, darf ein neuerer Stand aus dem Konto später alles außer dem Stern ersetzen.
    func setFavorite(_ book: LibraryBook, _ favorite: Bool) {
        let meta = library.meta(book)
        let clean = (try? localHash(book)) == ledger[LibrarySyncFormat.collectionBook, book.id]?.hash
        library.setFavorite(book, favorite, pendingFields: clean ? try? SyncLedger.fieldsHash(book, files: meta.files) : nil)
    }

    private func pushProgress(_ acc: Account) async throws {
        var objects: [(HubPushObject, String)] = []
        for key in library.progressKeys {
            guard let rec = library.record(key) else { continue }
            let id = LibrarySyncFormat.progressObjectId(key)
            let value = try JSONEncoder().encode(SyncedProgress(key: key, record: rec))
            let hash = LibrarySyncFormat.canonicalHash(value)
            let entry = ledger[LibrarySyncFormat.collectionProgress, id]
            if entry?.hash == hash || ledger.dead[SyncLedger.key(LibrarySyncFormat.collectionProgress, id)] == hash { continue }
            if value.count > LibrarySyncFormat.maxPlaintext { markDead(LibrarySyncFormat.collectionProgress, id, hash, "Ein Fortschritt ist zu groß für den Abgleich."); continue }
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
                    ledger[sent.0.collection, sent.0.objectId] = .init(revision: r.revision ?? 0, hash: sent.1, deleted: sent.0.deleted)
                case "superseded":
                    if let current = r.current { try apply(current, acc) }
                    else { ledger[sent.0.collection, sent.0.objectId] = nil }
                    again = true
                case "rejected":
                    markDead(sent.0.collection, sent.0.objectId, sent.1, "Der Server lehnt ein Objekt ab (\(r.reason ?? "unbekannt")).")
                default: break
                }
            }
            start = end
        }
    }

    private func markDead(_ collection: String, _ id: String, _ hash: String, _ message: String) {
        ledger.dead[SyncLedger.key(collection, id)] = hash
        error = message
    }

    /// Buch auf allen Geräten löschen: Grabstein senden, dann lokal entfernen.
    func deleteEverywhere(_ book: LibraryBook) async {
        if let acc = account, let entry = ledger[LibrarySyncFormat.collectionBook, book.id], !entry.deleted {
            let tomb = HubPushObject(collection: LibrarySyncFormat.collectionBook, objectId: book.id, baseRevision: entry.revision,
                                     payload: "", keyId: acc.keyId, deleted: true)
            do { try await push([(tomb, "")], acc); ledger[LibrarySyncFormat.collectionBook, book.id]?.deleted = true; saveLedger() }
            catch { self.error = "Löschen im Konto fehlgeschlagen: \(error.localizedDescription)"; return }
        }
        AudioDownloads.shared.discard(book.id)
        library.delete(book)
    }

    /// Kennt das Konto dieses Buch?
    func isSynced(_ book: LibraryBook) -> Bool { account != nil && ledger[LibrarySyncFormat.collectionBook, book.id].map { !$0.deleted } == true }

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
        let chunks = LibrarySyncFormat.chunkCount(size: size)
        let ref = BlobRef(blob: blob, size: size, chunks: chunks, type: type, file: file)
        if ledger.uploaded.contains(blob) { return ref }
        let (infoCode, infoData) = try await Self.send(acc.url + "/api/v1/voice/blobs/\(blob)", method: "GET", token: acc.token, bulk: wifiOnly)
        if infoCode != 200 && infoCode != 404 { throw Self.httpError(infoCode, infoData) }
        var have = Set<Int>()
        if infoCode == 200, let info = try JSONSerialization.jsonObject(with: infoData) as? [String: Any] {
            if info["complete"] as? Bool == true { ledger.uploaded.insert(blob); return ref }
            have = Set((info["chunks"] as? [[String: Any]] ?? []).compactMap { ($0["idx"] as? NSNumber)?.intValue })
        }
        for index in 0..<chunks where !have.contains(index) {
            let userId = acc.userId
            let body = try await Task.detached(priority: .utility) { () throws -> Data in
                let h = try FileHandle(forReadingFrom: url); defer { try? h.close() }
                try h.seek(toOffset: UInt64(index * LibrarySyncFormat.chunkPlainBytes))
                let plain = try h.read(upToCount: max(1, LibrarySyncFormat.chunkLimit(size: size))) ?? Data()
                let sealed = Data(try LibrarySyncFormat.sealChunk(key: key, userId: userId, blobId: blob, index: index, plain: [UInt8](plain)))
                return try JSONSerialization.data(withJSONObject: [
                    "data": sealed.base64EncodedString(),
                    "sha256": SHA256.hash(data: sealed).map { String(format: "%02x", $0) }.joined(),
                ], options: .withoutEscapingSlashes)  // „\\/“ machte die Anfrage rund 1,5 % größer
            }.value
            let (code, data) = try await Self.send(acc.url + "/api/v1/voice/blobs/\(blob)/chunks/\(index)", method: "POST", token: acc.token, body: body, bulk: wifiOnly)
            guard code == 200 else { throw Self.httpError(code, data) }
        }
        let (code, data) = try await Self.send(acc.url + "/api/v1/voice/blobs/\(blob)/commit", method: "POST", token: acc.token,
                                               body: try JSONSerialization.data(withJSONObject: ["chunks": chunks]))
        guard code == 200 else { throw Self.httpError(code, data) }
        ledger.uploaded.insert(blob); saveLedger()
        return ref
    }

    /// Blob Stück für Stück laden, prüfen, entschlüsseln und in `<ziel>.part`
    /// schreiben; erst vollständig wird umbenannt. Ein Abbruch setzt beim
    /// nächsten Mal am ersten fehlenden Stück wieder an. Nie die ganze Datei im Speicher.
    private func downloadBlob(_ ref: BlobRef, _ acc: Account, to target: URL, progress: (Int) -> Void) async throws {
        let part = target.appendingPathExtension("part")
        let fm = FileManager.default
        if !fm.fileExists(atPath: part.path) {
            fm.createFile(atPath: part.path, contents: nil, attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication])
        }
        let chunk = LibrarySyncFormat.chunkPlainBytes
        let partial = Self.fileSize(part) ?? 0
        // Der Bereich der Alt-Blobs kennt keine sichere Wiederaufnahme; hier läuft alles nacheinander.
        let legacy = LibrarySyncFormat.mayBeLegacySingleChunk(size: ref.size) && partial != ref.size
        let first = legacy ? 0 : LibrarySyncFormat.resumeChunk(size: ref.size, chunks: ref.chunks, partialBytes: partial) ?? ref.chunks
        let key = [UInt8](acc.key), userId = acc.userId
        progress(first * chunk)
        for index in first..<ref.chunks {
            let (code, data) = try await Self.send(acc.url + "/api/v1/voice/blobs/\(ref.blob)/chunks/\(index)", method: "GET", token: acc.token, bulk: wifiOnly)
            guard code == 200 else { throw Self.httpError(code, data) }
            let written = try await Task.detached(priority: .utility) { () throws -> Int in
                guard let json = try JSONSerialization.jsonObject(with: data) as? [String: String],
                      let raw = json["data"].flatMap({ Data(base64Encoded: $0) }) else { throw SyncError.corrupt("Antwort") }
                guard SHA256.hash(data: raw).map({ String(format: "%02x", $0) }).joined() == json["sha256"] else { throw SyncError.corrupt("Prüfsumme") }
                let plain = try LibrarySyncFormat.openChunk(key: key, userId: userId, blobId: ref.blob, index: index, sealed: [UInt8](raw))
                let h = try FileHandle(forWritingTo: part); defer { try? h.close() }
                try h.truncate(atOffset: UInt64(index * chunk))
                try h.write(contentsOf: plain)
                return index * chunk + plain.count
            }.value
            progress(written)
            if written == ref.size { break }  // Alt-Blob: ein Stück, obwohl die Rechnung zwei ergibt
        }
        guard Self.fileSize(part) == ref.size else {
            try? fm.removeItem(at: part); throw SyncError.corrupt("Größe")
        }
        _ = try? fm.removeItem(at: target)
        try fm.moveItem(at: part, to: target)
        // Vollständig aus dem Konto geladen: dort also vollständig vorhanden.
        ledger.uploaded.insert(ref.blob); saveLedger()
    }

    /// Cover laden (klein, meist ein Stück), als JPEG ablegen und merken,
    /// welcher Blob jetzt lokal liegt. Unlesbare Bilder bleiben unbeachtet.
    private func downloadCover(_ ref: BlobRef, _ book: LibraryBook, _ acc: Account) async throws {
        guard ref.size > 0, ref.size <= Self.maxCoverBytes else { throw SyncError.corrupt("Cover") }
        let target = library.dir(book).appendingPathComponent("cover.download")
        defer { try? FileManager.default.removeItem(at: target) }
        try await downloadBlob(ref, acc, to: target) { _ in }
        let data = try Data(contentsOf: target)
        library.storeCover(data, for: book)
        guard library.coverURL(book) != nil else { throw SyncError.corrupt("Cover") }
        library.setCoverBlob(book, ref.blob)
    }
    static let maxCoverBytes = 20 * 1024 * 1024

    /// Hörbuch beim ersten Öffnen laden: startet (oder setzt fort) den Ladevorgang
    /// im Hintergrund (`AudioDownloads`) und wartet, bis er fertig ist. `false`, wenn
    /// er pausiert wird oder fehlschlägt; der Zustand steht dann auf der Buchseite.
    func downloadAudio(_ book: LibraryBook) async -> Bool {
        guard account != nil else { error = SyncError.notLoggedIn.localizedDescription; return false }
        guard !library.meta(book).files.audio.isEmpty, !library.hasAudio(book) else { return library.hasAudio(book) }
        let downloads = AudioDownloads.shared
        downloads.start(book)
        // Hält „Nur im WLAN“ das Laden an, nicht stundenlang warten: der Zustand steht auf der Buchseite.
        if downloads.waitsForWifi(book.id) { error = SyncError.offlineRule.localizedDescription; return false }
        return await downloads.waitUntilDone(book.id)
    }

    /// Ein Blob ist im Konto vollständig (z. B. gerade vollständig von dort geladen).
    func markConfirmed(_ blob: String) { ledger.uploaded.insert(blob); saveLedger() }

    // MARK: Hörbuch vom iPhone entfernen (P8 Paket 1)

    /// Teile laut Konto in Hörreihenfolge.
    private func accountFiles(_ book: LibraryBook) -> [AudioOffload.AccountFile] {
        library.meta(book).files.audio.map { AudioOffload.AccountFile(blob: $0.blob, size: $0.size) }
    }

    /// Darf das Hörbuch vom iPhone? Ohne Netz, nur aus dem bekannten Stand.
    func offloadVerdict(_ book: LibraryBook, confirmed: Set<String>? = nil) -> AudioOffload.Verdict {
        AudioOffload.verdict(loggedIn: account != nil, synced: isSynced(book), downloading: AudioDownloads.shared.isActive(book.id),
                             localSizes: library.localAudioSizes(book), hasLeftovers: AudioOffload.hasLeftovers(in: library.dir(book)),
                             account: accountFiles(book), confirmed: confirmed ?? ledger.uploaded)
    }

    /// Teile beim Hub frisch prüfen. Vollständige landen in `ledger.uploaded`,
    /// fehlende fliegen heraus (der nächste Abgleich lädt sie dann wieder hoch).
    /// `nil`: Konto nicht erreichbar.
    private func confirmInAccount(_ files: [AudioOffload.AccountFile], _ acc: Account) async -> Set<String>? {
        var confirmed = Set<String>()
        defer { saveLedger() }
        for blob in AudioOffload.blobsToCheck(files) {
            guard let reply = try? await Self.send(acc.url + "/api/v1/voice/blobs/\(blob)", method: "GET", token: acc.token) else { return nil }
            let (code, data) = reply
            let complete = code == 200 && ((try? JSONSerialization.jsonObject(with: data)) as? [String: Any])?["complete"] as? Bool == true
            if complete { confirmed.insert(blob); ledger.uploaded.insert(blob) }
            else if code == 200 || code == 404 { ledger.uploaded.remove(blob) }
            else { return nil }
        }
        return confirmed
    }

    /// Hörbuchdateien vom iPhone entfernen, wenn sie sicher im Konto liegen.
    /// Eintrag, Text und Fortschritt bleiben. Liefert das Urteil; nur bei
    /// `.allowed` wurde gelöscht. `freed`: frei gewordener Platz in Bytes.
    @discardableResult
    func removeDownloads(_ book: LibraryBook) async -> (verdict: AudioOffload.Verdict, freed: Int) {
        var verdict = offloadVerdict(book)
        switch verdict {
        case .allowed, .unconfirmed: break
        default: return (verdict, 0)
        }
        guard let acc = account else { return (.notLoggedIn, 0) }
        // Vor dem Löschen immer frisch beim Konto nachfragen. Ohne Verbindung wird
        // nicht gelöscht: der Hub könnte einen Blob inzwischen verloren haben, und
        // dann läge die letzte Kopie auf diesem iPhone. Geprüft wird gegen die
        // lokalen Verweise in sync.json; genau die nutzt auch das erneute Laden.
        guard let fresh = await confirmInAccount(accountFiles(book), acc) else { return (.unreachable, 0) }
        verdict = offloadVerdict(book, confirmed: fresh)
        // Zwischenzeitlich gestartet (Download, Abgleich)? Dann nicht anfassen.
        guard verdict.allowed, !AudioDownloads.shared.isActive(book.id) else { return (verdict.allowed ? .downloading : verdict, 0) }
        let playback = LibraryPlayback.shared
        if playback.book?.id == book.id { playback.stop() }
        AudioDownloads.shared.discard(book.id)  // pausierte oder fehlgeschlagene Ladevorgänge
        do {
            let freed = try AudioOffload.remove(bookDir: library.dir(book))
            library.reload()
            return (.allowed, freed)
        } catch {
            self.error = "Entfernen fehlgeschlagen: \(error.localizedDescription)"
            library.reload()
            return (verdict, 0)
        }
    }

    /// Dateiname eines Hörbuchteils auf dem iPhone: nur ein Name, nie ein Pfad.
    /// Gemeinsam für Download und Größenprüfung vor dem Entfernen.
    static func audioFileName(_ given: String?, index: Int) -> String {
        let name = (given ?? "").split(separator: "/").last.map(String.init) ?? ""
        return name.isEmpty || name.hasPrefix(".") ? String(format: "teil_%03d.m4a", index + 1) : name
    }

    // MARK: Speicherübersicht (P8 Paket 2)

    /// Hörbücher, die auf dem iPhone Platz belegen, größte zuerst.
    func storageItems() -> [AudioStorage.Item] {
        AudioStorage.sorted(library.books.compactMap { book in
            let bytes = library.localAudioBytes(book)
            guard bytes > 0 else { return nil }
            let rec = library.record(library.audioKey(book))
            let verdict = offloadVerdict(book)
            let removable: Bool, localOnly: Bool
            switch verdict {
            case .allowed, .unconfirmed: removable = true; localOnly = false
            case .notInAccount, .notLoggedIn: removable = false; localOnly = true
            default: removable = false; localOnly = false
            }
            return AudioStorage.Item(id: book.id, title: book.title, bytes: bytes, fraction: LibraryProgress.fraction(rec),
                                     finished: book.finished || rec?.finished == true,
                                     lastUsedMs: rec.map(\.updated).flatMap { $0 > 0 ? $0 : nil },
                                     partial: !library.hasAudio(book), removable: removable, localOnly: localOnly)
        })
    }

    /// Mehrere Hörbücher nacheinander entfernen; jedes wird einzeln geprüft.
    func removeDownloads(ids: [String]) async -> (removed: Int, freed: Int, kept: Int) {
        var removed = 0, freed = 0, kept = 0
        for id in ids {
            guard let book = library.books.first(where: { $0.id == id }) else { continue }
            let result = await removeDownloads(book)
            if result.verdict.allowed { removed += 1; freed += result.freed } else { kept += 1 }
        }
        return (removed, freed, kept)
    }

    /// Dateigröße frisch vom Dateisystem (URL-Ressourcenwerte werden zwischengespeichert).
    static func fileSize(_ url: URL) -> Int? {
        (try? FileManager.default.attributesOfItem(atPath: url.path)[.size] as? NSNumber)?.intValue
    }

    // MARK: HTTP

    /// `bulk`: Dateiverkehr unter „nur im WLAN“ – kein Mobilfunk, kein Datensparmodus.
    static func send(_ url: String, method: String, token: String?, body: Data? = nil, bulk: Bool = false) async throws -> (Int, Data) {
        guard let u = URL(string: url) else { throw SyncError.http(0, "Ungültige Server-Adresse.") }
        var req = URLRequest(url: u, timeoutInterval: 60)
        if bulk { req.allowsExpensiveNetworkAccess = false; req.allowsConstrainedNetworkAccess = false }
        req.httpMethod = method
        req.setValue("application/json", forHTTPHeaderField: "Accept")
        if let body { req.httpBody = body; req.setValue("application/json", forHTTPHeaderField: "Content-Type") }
        if let token { req.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization") }
        let (data, response) = try await URLSession.shared.data(for: req, delegate: NoRedirects.shared)
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
    @discardableResult static func save(_ account: LibrarySync.Account) -> Bool {
        guard let data = try? JSONEncoder().encode(account) else { return false }
        delete()
        return SecItemAdd([kSecClass: kSecClassGenericPassword, kSecAttrService: service, kSecAttrAccount: "account",
                           kSecAttrAccessible: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly, kSecValueData: data] as CFDictionary, nil) == errSecSuccess
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
