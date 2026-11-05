import Foundation
import Security
import SwiftUI
import VisionKit
import AVFoundation

// Buch-Gespräch (lva#320, Spec §11.3, §11.7): Relay-Zugang des iPhones. Hub-Kanal (verschlüsselte Objekte der Sammlung
// `booktalk_relay` mit Konto-Zugang und -Schlüssel wie die Bibliothek), Relay-Schlüssel je Gerät im Schlüsselbund (R1),
// Freigaben je Modell (R9, R10), Modellliste des PCs, Eval-Tabelle (§11.5).
// Der Relay-Kern (Anfrage, HMAC, Fristen) steht in VoiceCore `BookTalkRelay.swift`.

// MARK: - Schlüsselbund (R1)

/// Der Relay-Schlüssel gehört genau diesem Gerät und liegt nur im Schlüsselbund (nie in `UserDefaults`, nie in Sicherungen).
enum RelayKeychain {
    private static let service = "de.localvoice.booktalk-relay"

    @discardableResult static func save(_ e: BookTalkRelayEnrollment) -> Bool {
        guard let data = try? JSONEncoder().encode(e) else { return false }
        delete()
        return SecItemAdd([kSecClass: kSecClassGenericPassword, kSecAttrService: service, kSecAttrAccount: "enrollment",
                           kSecAttrAccessible: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly, kSecValueData: data] as CFDictionary, nil) == errSecSuccess
    }

    static func load() -> BookTalkRelayEnrollment? {
        var out: CFTypeRef?
        guard SecItemCopyMatching([kSecClass: kSecClassGenericPassword, kSecAttrService: service, kSecAttrAccount: "enrollment",
                                   kSecReturnData: true] as CFDictionary, &out) == errSecSuccess, let data = out as? Data else { return nil }
        return try? JSONDecoder().decode(BookTalkRelayEnrollment.self, from: data)
    }

    static func delete() {
        SecItemDelete([kSecClass: kSecClassGenericPassword, kSecAttrService: service, kSecAttrAccount: "enrollment"] as CFDictionary)
    }
}

// MARK: - Hub-Kanal

/// Spricht mit dem Hub (`sync/pull`, `sync/push`) über denselben Zugang wie `LibrarySync`; Klartext verlässt den Kanal nie
/// unverschlüsselt. Hält einen eigenen Pull-Cursor, damit der Bibliotheks-Abgleich unberührt bleibt.
actor BookTalkRelayHub: BookTalkRelayChannel {
    private struct Slot { var revision: Int64; var json: Data? }

    private let account: LibrarySync.Account
    private var cursor: Int64 = 0
    private var slots: [String: Slot] = [:]
    /// `request_id`s eigener, noch laufender Anfragen: ihre Objekte räumt das Aufräumen nie ab (R5).
    private var openRequests: Set<String> = []
    /// Wann dieses iPhone ein Objekt zum ersten Mal sah (Unix-Sekunden); `resp` trägt keinen eigenen Zeitstempel.
    private var firstSeen: [String: Int] = [:]

    init(account: LibrarySync.Account) { self.account = account }

    // MARK: BookTalkRelayChannel

    func put(objectId: String, json: Data) async throws {
        try await push(objectId: objectId, json: json, deleted: false)
        if objectId.hasPrefix("req_") { openRequests.insert(String(objectId.dropFirst(4))) }
    }

    func fetch(objectId: String) async throws -> Data? {
        try await pull()
        return slots[objectId]?.json
    }

    func remove(objectId: String) async throws {
        // Die Anfrage gilt als beendet, sobald ihre Antwort abgeräumt wird (immer vor dem Löschversuch, auch wenn nichts mehr da ist).
        if objectId.hasPrefix("resp_") { openRequests.remove(String(objectId.dropFirst(5))) }
        if slots[objectId] == nil { try await pull() }
        guard let slot = slots[objectId], slot.json != nil else { return }
        try await push(objectId: objectId, json: nil, deleted: true)
    }

    // MARK: Modelle und Aufräumen

    /// Vom PC veröffentlichte Liste (noch ungeprüft: die Signatur prüft der Aufrufer mit `isAuthentic`), `nil` wenn keine da ist.
    func modelList(desktopDeviceId: String) async throws -> BookTalkRelayModelList? {
        try await pull()
        guard let data = slots[BookTalkRelay.objectId(.models, desktopDeviceId)]?.json else { return nil }
        return try? JSONDecoder().decode(BookTalkRelayModelList.self, from: data)
    }

    /// Eigene Relay-Objekte abräumen (R5, Entscheidung in `BookTalkRelayCleanup`): nie das Objekt einer offenen Anfrage,
    /// `req` erst nach 10 min, `resp` erst nach 10 min Sichtbarkeit; fremde Objekte nie.
    func cleanup(deviceId: String, now: Int) async {
        try? await pull()
        var live: [BookTalkRelayCleanup.Object] = []
        for (id, slot) in slots {
            guard let json = slot.json else { firstSeen[id] = nil; continue }
            let seen = firstSeen[id] ?? now
            firstSeen[id] = seen
            live.append(.init(id: id, json: json, firstSeen: seen))
        }
        for id in BookTalkRelayCleanup.stale(objects: live, deviceId: deviceId, openRequestIds: openRequests, now: now) {
            try? await remove(objectId: id)
        }
    }

    // MARK: Hub

    private func pull() async throws {
        var more = true, restarted = false
        while more {
            let (code, data) = try await LibrarySync.send(account.url + "/api/v1/voice/sync/pull?since=\(cursor)&limit=100", method: "GET", token: account.token)
            if code == 410 && !restarted { cursor = 0; slots = [:]; restarted = true; continue }
            guard code == 200 else { throw await LibrarySync.httpError(code, data) }
            let page = try JSONDecoder().decode(HubPullResponse.self, from: data)
            for remote in page.objects where remote.collection == BookTalkRelay.collection { apply(remote) }
            if page.cursor <= cursor && page.more { break }
            cursor = page.cursor
            more = page.more
        }
    }

    private func apply(_ remote: HubRemoteObject) {
        if remote.deleted { slots[remote.objectId] = Slot(revision: remote.revision, json: nil); return }
        guard remote.keyId == account.keyId,
              let plain = try? SyncCrypto.open(key: [UInt8](account.key), aad: SyncCrypto.aad(userId: account.userId, collection: remote.collection, objectId: remote.objectId),
                                               sealedBase64: remote.payload) else { return }
        slots[remote.objectId] = Slot(revision: remote.revision, json: Data(plain))
    }

    private func push(objectId: String, json: Data?, deleted: Bool, retry: Bool = true) async throws {
        let payload = try json.map {
            try SyncCrypto.seal(key: [UInt8](account.key), aad: SyncCrypto.aad(userId: account.userId, collection: BookTalkRelay.collection, objectId: objectId),
                                plaintext: [UInt8]($0))
        } ?? ""
        let object = HubPushObject(collection: BookTalkRelay.collection, objectId: objectId, baseRevision: slots[objectId]?.revision ?? 0,
                                   payload: payload, keyId: account.keyId, deleted: deleted)
        let body = try JSONSerialization.data(withJSONObject: [
            "device_id": account.deviceId,
            "objects": [try JSONSerialization.jsonObject(with: JSONEncoder().encode(object))],
        ])
        let (code, data) = try await LibrarySync.send(account.url + "/api/v1/voice/sync/push", method: "POST", token: account.token, body: body)
        guard code == 200 else { throw await LibrarySync.httpError(code, data) }
        struct Response: Decodable { var results: [HubPushResult] }
        guard let result = try JSONDecoder().decode(Response.self, from: data).results.first else { return }
        switch result.outcome {
        case "accepted":
            slots[objectId] = Slot(revision: result.revision ?? 0, json: deleted ? nil : json)
        case "superseded":
            if let current = result.current { apply(current) }
            if retry { try await push(objectId: objectId, json: json, deleted: deleted, retry: false) }
        default:
            throw LibrarySync.SyncError.http(0, "Der Hub lehnt das Relay-Objekt ab.")
        }
    }
}

// MARK: - Eval-Tabelle (§11.5)

enum BookTalkEval {
    /// `book_talk_eval.json` aus dem App-Bundle (mit dem Build signiert). Fehlt sie, ist alles ungeprüft.
    static let table: BookTalkEvalTable = BookTalkEvalTable.load(
        Bundle.main.url(forResource: "book_talk_eval", withExtension: "json").flatMap { try? Data(contentsOf: $0) })

    static func status(provider: String, model: String, budget: String) -> BookTalkEvalStatus {
        table.status(for: BookTalkEvalTuple(provider: provider, model: model, budget: budget))
    }
}

// MARK: - Store: Einrichtung, Modelle, Freigaben

@MainActor
final class BookTalkRelayStore: ObservableObject {
    static let shared = BookTalkRelayStore()
    private static let consentKey = "book_talk_relay_consent"
    private static let modelsKey = "book_talk_relay_models"

    @Published private(set) var enrollment: BookTalkRelayEnrollment?
    @Published private(set) var models: [BookTalkRelayModelInfo] = []
    @Published private(set) var consented: Set<String> = []
    @Published private(set) var refreshing = false
    @Published private(set) var status = ""

    private var hub: BookTalkRelayHub?
    private var hubDevice: String?

    init() {
        enrollment = RelayKeychain.load()
        consented = Set(UserDefaults.standard.stringArray(forKey: Self.consentKey) ?? [])
        if let data = UserDefaults.standard.data(forKey: Self.modelsKey), let list = try? JSONDecoder().decode([BookTalkRelayModelInfo].self, from: data) {
            models = enrollment?.allowed(list) ?? []
        }
        if let e = enrollment, e.isExpired(now: Self.now) { status = "Die Einrichtung für \(e.desktopName) ist abgelaufen. Am PC neu freigeben." }
    }

    static var now: Int { Int(Date().timeIntervalSince1970) }

    var usable: Bool {
        guard let e = enrollment else { return false }
        return !e.isExpired(now: Self.now) && LibrarySync.shared.account?.deviceId == e.deviceId
    }

    var desktopName: String { enrollment?.desktopName ?? "PC" }

    // MARK: Einrichtung (R1)

    /// Einrichtungscode vom PC (QR oder Einfügen). Nur für genau dieses Gerät; der Code selbst läuft nach ca. 10 min ab
    /// (`expires_at`), die Freigabe danach bis `grant_expires_at`.
    func enroll(_ text: String) -> String? {
        guard let e = BookTalkRelayEnrollment.parse(text) else { return "Das ist kein Einrichtungscode vom PC." }
        guard let account = LibrarySync.shared.account else { return "Zuerst unter „Konto & Sync“ anmelden oder koppeln." }
        guard e.deviceId == account.deviceId else { return "Dieser Code gehört zu einem anderen Gerät." }
        guard !e.codeExpired(now: Self.now) else { return "Dieser Code ist abgelaufen. Am PC einen neuen erzeugen." }
        guard !e.isExpired(now: Self.now) else { return "Die Freigabe in diesem Code ist abgelaufen. Am PC neu freigeben." }
        guard RelayKeychain.save(e) else { return "Der Schlüssel konnte nicht im Schlüsselbund gespeichert werden." }
        enrollment = e
        status = ""
        Task { await refresh() }
        return nil
    }

    /// Einrichtung und alle Freigaben löschen.
    func forget() {
        RelayKeychain.delete()
        enrollment = nil
        models = []
        hub = nil
        hubDevice = nil
        revokeAll()
        UserDefaults.standard.removeObject(forKey: Self.modelsKey)
    }

    // MARK: Modelle

    func refresh() async {
        guard let e = enrollment, !refreshing else { return }
        guard let account = LibrarySync.shared.account else { status = "Nicht angemeldet."; return }
        guard !e.isExpired(now: Self.now) else { status = "Die Einrichtung ist abgelaufen. Am PC neu freigeben."; return }
        refreshing = true
        defer { refreshing = false }
        let hub = channel(for: account)
        do {
            if let list = try await hub.modelList(desktopDeviceId: e.desktopDeviceId) {
                // Nur eine vom PC mit dem Relay-Schlüssel signierte Liste für genau dieses Gerät zählt (Befund 12).
                guard list.isAuthentic(key: e.key, desktopDeviceId: e.desktopDeviceId, deviceId: e.deviceId) else {
                    models = []
                    status = "Die Modellliste von \(e.desktopName) ist nicht signiert oder gehört nicht zu diesem iPhone und wird ignoriert."
                    return
                }
                models = e.allowed(list.models)
                UserDefaults.standard.set(try? JSONEncoder().encode(list.models), forKey: Self.modelsKey)
                status = models.isEmpty ? "\(e.desktopName) hat keine Abo-Modelle für dieses iPhone freigegeben." : ""
                // Revozierte Modelle verlieren auch hier die Freigabe.
                let ids = Set(models.map { Self.consentId($0.id, e.desktopDeviceId) })
                if !consented.isSubset(of: ids) { setConsented(consented.intersection(ids)) }
            } else {
                models = []
                status = "\(e.desktopName) hat noch keine Modelle veröffentlicht."
            }
            await hub.cleanup(deviceId: e.deviceId, now: Self.now)
        } catch {
            status = "Dein PC ist nicht erreichbar: \(error.localizedDescription)"
        }
    }

    private func channel(for account: LibrarySync.Account) -> BookTalkRelayHub {
        if let hub, hubDevice == account.deviceId { return hub }
        let made = BookTalkRelayHub(account: account)
        hub = made
        hubDevice = account.deviceId
        return made
    }

    /// Modell für die Sitzung, nur mit gültiger Einrichtung und passendem Konto.
    func makeModel(_ info: BookTalkRelayModelInfo) -> BookTalkRelayModel? {
        guard usable, let e = enrollment, let account = LibrarySync.shared.account else { return nil }
        let consent = Self.consentId(info.id, e.desktopDeviceId)
        let budget = BookTalkBudget.relayCLI1(contextTokens: info.contextTokens).name
        let provider = info.provider.rawValue
        // Prüfstatus nach dem tatsächlichen Modell der Antwort (R7, §11.5), nie nach dem Alias.
        return BookTalkRelayModel(info: info, enrollment: e, channel: channel(for: account),
                                  authorized: { Self.isConsented(consent) },
                                  evalStatus: { BookTalkEval.status(provider: provider, model: $0, budget: budget) })
    }

    func info(_ id: String) -> BookTalkRelayModelInfo? { models.first { $0.id == id } }

    // MARK: Freigabe je Modell (R9, R10)

    nonisolated static func consentId(_ modelId: String, _ desktop: String) -> String { modelId + "@" + desktop }

    nonisolated static func isConsented(_ id: String) -> Bool {
        (UserDefaults.standard.stringArray(forKey: consentKey) ?? []).contains(id)
    }

    func isConsented(_ model: BookTalkRelayModelInfo) -> Bool {
        guard let e = enrollment else { return false }
        return consented.contains(Self.consentId(model.id, e.desktopDeviceId))
    }

    func grant(_ model: BookTalkRelayModelInfo) {
        guard let e = enrollment else { return }
        setConsented(consented.union([Self.consentId(model.id, e.desktopDeviceId)]))
    }

    func revoke(_ model: BookTalkRelayModelInfo) {
        guard let e = enrollment else { return }
        setConsented(consented.subtracting([Self.consentId(model.id, e.desktopDeviceId)]))
    }

    func revokeAll() { setConsented([]) }

    private func setConsented(_ value: Set<String>) {
        consented = value
        UserDefaults.standard.set(Array(value).sorted(), forKey: Self.consentKey)
    }
}

// MARK: - Scanner für den Einrichtungscode

struct RelayCodeScanner: UIViewControllerRepresentable {
    let found: (String) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(found: found) }

    func makeUIViewController(context: Context) -> DataScannerViewController {
        let scanner = DataScannerViewController(recognizedDataTypes: [.barcode(symbologies: [.qr])], qualityLevel: .balanced,
                                                recognizesMultipleItems: false, isHighFrameRateTrackingEnabled: false, isHighlightingEnabled: true)
        scanner.delegate = context.coordinator
        return scanner
    }

    func updateUIViewController(_ scanner: DataScannerViewController, context: Context) {
        if !scanner.isScanning { try? scanner.startScanning() }
    }

    static func dismantleUIViewController(_ scanner: DataScannerViewController, coordinator: Coordinator) { scanner.stopScanning() }

    final class Coordinator: NSObject, DataScannerViewControllerDelegate {
        let found: (String) -> Void
        init(found: @escaping (String) -> Void) { self.found = found }

        func dataScanner(_ dataScanner: DataScannerViewController, didAdd addedItems: [RecognizedItem], allItems: [RecognizedItem]) {
            for case .barcode(let code) in addedItems {
                if let text = code.payloadStringValue, text.hasPrefix("lvrelay1:") { found(text); return }
            }
        }
    }
}
