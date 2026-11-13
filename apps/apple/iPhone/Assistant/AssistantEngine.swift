import Foundation
import FoundationModels

// Gesprächsmodus „Sprechen“ (Paket A): Modellwahl, Bereichsschalter und Cloud-Zustimmung.
// Unabhängig vom Buch-Gespräch: nicht an `book_talk_preview_enabled` und nicht an das Eval-Tor gebunden. Wiederverwendet
// werden die Modelle (`FoundationBookTalkModel`, `LlamaBookTalkModel`), der Relay-Speicher und der Endpunkt-Speicher.

// MARK: - Wahl

/// Modell des Gesprächs, je Gerät in `assistant_model` gemerkt.
enum AssistantModelChoice: Hashable, Identifiable {
    case apple, gemma
    /// Abo-Modell, das der PC veröffentlicht (Modell-ID).
    case relay(String)
    /// Eigener OpenAI-kompatibler Endpunkt (Einstellungen › Eigener Endpunkt).
    case endpoint

    static let key = "assistant_model"

    var id: String { rawValue }

    var rawValue: String {
        switch self {
        case .apple: "apple"
        case .gemma: "gemma"
        case .relay(let id): "relay:" + id
        case .endpoint: "endpoint"
        }
    }

    init?(rawValue: String) {
        switch rawValue {
        case "apple": self = .apple
        case "gemma": self = .gemma
        case "endpoint": self = .endpoint
        default:
            guard rawValue.hasPrefix("relay:"), rawValue.count > 6 else { return nil }
            self = .relay(String(rawValue.dropFirst(6)))
        }
    }

    /// Gespeicherte Wahl; `nil`, solange nie gewählt wurde (dann gilt die automatische Reihenfolge).
    static var stored: AssistantModelChoice? {
        UserDefaults.standard.string(forKey: key).flatMap(AssistantModelChoice.init)
    }

    static func store(_ choice: AssistantModelChoice) {
        UserDefaults.standard.set(choice.rawValue, forKey: key)
    }
}

// MARK: - Bereiche

/// Welche Bereiche der Assistent nutzen darf, getrennt für lokale und Cloud-Modelle (Standard: lokal alle, Cloud nur Bücher).
/// Die Quelle des Stands ist die Zustandsdatei der Freigaben (`AssistantReleaseFiles`, Kern): Bereiche, Schalter und Zustimmung
/// stehen dort, nicht mehr in UserDefaults. Die alten Schlüssel werden nur noch GELESEN, und nur einmalig beim allerersten Start
/// (nie geschrieben, nie danach), und auch dann nur einschränkend (Schnittmenge mit dem Auslieferungsstand, nie eine Zustimmung).
enum AssistantAreaSettings {
    static let localKey = "assistant_areas_local"
    static let cloudKey = "assistant_areas_cloud"

    /// Der gespeicherte, bestätigte Stand der Bereiche (gesperrte Datei: keine).
    @MainActor
    static func areas(cloud: Bool) -> Set<AssistantArea> {
        AssistantReleaseFiles.store.record.areas(cloud: cloud)
    }

    /// Alte Einstellungen für die einmalige Übernahme beim allerersten Start; `nil`, wenn es keine gibt.
    static func legacyRecord() -> AssistantReleaseRecord? {
        let defaults = UserDefaults.standard
        func read(_ key: String) -> Set<AssistantArea>? {
            defaults.stringArray(forKey: key).map { Set($0.compactMap(AssistantArea.init(rawValue:))) }
        }
        let local = read(localKey)
        let cloud = read(cloudKey)
        guard local != nil || cloud != nil else { return nil }
        return AssistantReleaseRecord(localAreas: local ?? Set(AssistantArea.allCases), cloudAreas: cloud ?? [.books])
    }

    /// Anzeigename im Menü und im Zustimmungstext.
    static func title(_ area: AssistantArea) -> String {
        switch area {
        case .books: "Bücher"
        case .transcripts: "Besprechungen"
        case .files: "Ablage"
        case .notes: "Sprachnotizen"
        }
    }

    static func list(_ areas: Set<AssistantArea>) -> String {
        AssistantArea.allCases.filter { areas.contains($0) }.map(title).joined(separator: ", ")
    }
}

// MARK: - Cloud-Ziel und Zustimmung

/// Wohin ein Cloud-Modell die Anfragen schickt (für Zustimmung und Anzeige).
enum AssistantCloudTarget: Equatable {
    case relay(BookTalkRelayModelInfo)
    case endpoint(AssistantEndpoint)

    var id: String {
        switch self {
        case .relay(let info): "relay:" + info.id
        case .endpoint(let endpoint): "endpoint:" + AssistantEndpointStore.consentId(endpoint)
        }
    }

    /// Empfänger der Daten, wie im Dialog genannt.
    var recipient: String {
        switch self {
        case .relay(let info): info.provider.recipient
        case .endpoint(let endpoint): AssistantEndpointStore.hostName(of: endpoint.baseURL) ?? "dem eingetragenen Server"
        }
    }
}

/// Die Zustandsdatei der Freigaben dieses Prozesslaufs (`AssistantReleaseStore`): liegt neben dem Protokoll, wird beim ersten Zugriff
/// geöffnet (gültig: geladen; fehlt bei vorhandenem Protokoll oder ungültig: gesperrt, „Freigaben prüfen“; allererster Start: der
/// Auslieferungsstand, erst gesichert, dann nutzbar).
/// Beim Anlegen werden Protokoll und Meldestelle der Quellen an die EINE Ordnung des Prozesses gehängt (Runde 8, R4): Auch ohne
/// Gespräch und ohne Modell erhöht jede gemeldete Änderung der Quellen die Generation und schreibt ihre Zeile „Inhalt“.
@MainActor
enum AssistantReleaseFiles {
    /// Das Protokoll dieses Prozesslaufs: EIN Objekt (ein Schreiber je Datei) für Controller, Zustandsdatei und Meldestelle.
    static let audit = AssistantAuditLog(directory: AssistantAuditLog.standardDirectory())

    static let store: AssistantReleaseStore = {
        let store = AssistantReleaseStore(directory: AssistantAuditLog.standardDirectory(), adopt: AssistantAreaSettings.legacyRecord())
        store.bind(watch: AssistantContentWatch.shared, audit: audit)
        return store
    }()

    /// Beim Prozessstart gerufen (Delegat der App) und beim Anlegen der Quellen: öffnet die Zustandsdatei und hängt die Ordnung ein.
    static func start() { _ = store }
}

enum AssistantConsent {
    /// Zustimmung gilt nur, wenn sie für dieses Ziel gegeben wurde (Zustandsdatei), ihre Bereiche die jetzigen umfassen (weniger
    /// Bereiche brauchen keine neue Frage, mehr schon) und der Speicher des Ziels sie (noch) kennt: ein Widerruf in den
    /// Einstellungen macht sie ungültig. Maßgeblich ist der BESTÄTIGTE Stand der Datei; eine Zustimmung, deren Sicherung scheiterte,
    /// gilt nicht.
    @MainActor
    static func isGranted(_ target: AssistantCloudTarget, areas: Set<AssistantArea>) -> Bool {
        guard storeKnows(target) else { return false }
        return AssistantReleaseFiles.store.record.consentedAreas(target.id).isSuperset(of: areas)
    }

    @MainActor
    private static func storeKnows(_ target: AssistantCloudTarget) -> Bool {
        switch target {
        case .relay(let info): BookTalkRelayStore.shared.isConsented(info)
        case .endpoint: AssistantEndpointStore.shared.consented
        }
    }

    /// Gibt das Ziel im jeweiligen Speicher frei (dort prüft das Modell die Zustimmung selbst). Die Zustimmung selbst steht schon in
    /// der Zustandsdatei (Kern, `AssistantSession.setCloudConsent`); diese Freigabe kommt erst danach.
    @MainActor
    static func grant(_ target: AssistantCloudTarget) {
        switch target {
        case .relay(let info): BookTalkRelayStore.shared.grant(info)
        case .endpoint: AssistantEndpointStore.shared.grantConsent()
        }
    }

    static func title(_ target: AssistantCloudTarget) -> String { "An \(target.recipient) senden?" }

    static func body(_ target: AssistantCloudTarget, areas: Set<AssistantArea>, desktop: String) -> String {
        var text: String
        switch target {
        case .relay(let info):
            text = "Deine Fragen, die letzten Gesprächsbeiträge und Auszüge aus deinen Inhalten gehen verschlüsselt an deinen PC ‚\(desktop)‘ und von dort an \(info.provider.recipient) (\(info.label), dein Abo)."
        case .endpoint(let endpoint):
            text = "Deine Fragen, die letzten Gesprächsbeiträge und Auszüge aus deinen Inhalten gehen an den Server ‚\(AssistantEndpointStore.hostName(of: endpoint.baseURL) ?? "?")‘ (Modell \(endpoint.label))."
        }
        if areas.isEmpty {
            text += "\n\nFreigegebene Bereiche: keine. Es gehen nur deine Fragen und der Gesprächsverlauf hinaus."
        } else {
            text += "\n\nFreigegebene Bereiche: \(AssistantAreaSettings.list(areas)). Je Frage nur wenige Auszüge, nie Audio, nie ganze Dokumente."
            if areas.contains(.books) { text += " Bücher: bei begonnenen Büchern nur bis zu deiner Lesestelle." }
            if areas.contains(.files) { text += " Ablage: nur Dokumente, die als „öffentlich“ eingestuft sind." }
        }
        text += "\n\nRegion, Speicherung und Training richten sich nach dem Vertrag mit dem Empfänger; die App kann das nicht prüfen. Du kannst die Bereiche und die Zustimmung jederzeit ändern."
        return text
    }
}

// MARK: - Engine

/// Was das Gespräch an Modell braucht: Modell, Konfiguration für `AssistantSession` und Hinweise.
struct AssistantEngine {
    let model: (any BookTalkModel)?
    let config: AssistantConfig
    let choice: AssistantModelChoice?
    /// Hinweis für die Anzeige: warum kein Modell läuft oder warum es nicht das gewählte ist; sonst nil.
    let note: String?
    /// Kurzer Zusatz als eigene Zeile (z. B. „Das Apple-Systemmodell wird noch geladen.“), nur wenn er zutrifft; sonst nil.
    let detail: String?
    /// Zum Freigeben beim Beenden und bei Speicherwarnung.
    let llama: LlamaBookTalkModel?
    /// Nur bei Cloud-Modellen.
    let target: AssistantCloudTarget?

    /// Kurzer Haupttext, wenn kein Modell läuft; Ursachen stehen (falls zutreffend) in `detail` oder `note`.
    static let noModelText = "Kein Modell verfügbar."

    var isCloud: Bool { config.isCloud }
    var hasModel: Bool { model != nil }
    var label: String { config.modelLabel }

    /// Die Konfiguration für eine Sitzung im gewählten Antwortstil (die Konfiguration der Sitzung ist unveränderlich). Es werden keine
    /// festen Floskeln gesprochen oder geschrieben (kein Überbrückungssatz: `bridge` immer aus; die Ansagen für langsame Antworten,
    /// `fillers`, bleiben auf ihrem Standard `false` und werden hier nicht gesetzt). Stattdessen zeigt die Ansicht die graue Denk-Anzeige
    /// (`AssistantController.thinking`). Die Auszüge für den Denker sind auf dem Gerät auf 2400 Zeichen begrenzt, sonst auf 6000.
    func sessionConfig(style: AssistantStyle) -> AssistantConfig {
        var next = config
        next.style = style
        next.bridge = false
        next.naturalPassageChars = isCloud ? 6_000 : 2_400
        return next
    }

    /// Lädt das Apple-Systemmodell vor, damit die erste Antwort nicht auf das Laden warten muss (`LanguageModelSession.prewarm`; die
    /// Dokumentation verlangt mindestens eine Sekunde bis zur Anfrage: Beim Start des Gesprächs spricht noch niemand). Nur wenn das
    /// Modell verfügbar ist. Es entsteht keine Anfrage, kein Inhalt verlässt das Gerät, nichts wird geschrieben. Die Sitzung bleibt in
    /// einer Variablen stehen, bis eine neue sie ersetzt: Eine sofort freigegebene Sitzung könnte das Laden abbrechen (die Dokumentation
    /// sagt dazu nichts). Das Laden ist ausdrücklich nicht garantiert (Hintergrund, Last).
    @MainActor
    static func prewarmApple() {
        guard FoundationBookTalkModel.isAvailable else { return }
        let session = LanguageModelSession(instructions: "Antworte kurz auf Deutsch.")
        session.prewarm()
        prewarmed = session
    }

    @MainActor private static var prewarmed: LanguageModelSession?

    static func unavailable(_ note: String, detail: String? = nil) -> AssistantEngine {
        AssistantEngine(model: nil, config: AssistantConfig(modelLabel: "Kein Modell", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400,
                                                            userName: nil, timeout: 45),
                        choice: nil, note: note, detail: detail, llama: nil, target: nil)
    }

    func withNote(_ text: String) -> AssistantEngine {
        AssistantEngine(model: model, config: config, choice: choice, note: text, detail: detail, llama: llama, target: target)
    }

    /// Baut das Modell. Ohne `explicit` gilt die gespeicherte Wahl, sonst die Reihenfolge Apple, Gemma, kein Modell; ist
    /// die gespeicherte Wahl nicht benutzbar, läuft die lokale Reihenfolge mit Hinweis (nie automatisch in die Cloud).
    /// Mit `explicit` (Auswahl im Menü) wird genau dieses Modell gebaut und bei Misserfolg ohne Modell gemeldet.
    @MainActor
    static func make(choice explicit: AssistantModelChoice? = nil) async -> AssistantEngine {
        #if DEBUG
        // UI-Tests des Zustands „Kein Modell“ (Simulatoren haben je nach Host ein Systemmodell).
        if ProcessInfo.processInfo.arguments.contains("--assistant-no-model") { return .unavailable(noModelText) }
        #endif
        if let explicit { return await build(explicit) }
        if let stored = AssistantModelChoice.stored {
            let built = await build(stored)
            if built.hasModel { return built }
            let fallback = await automatic()
            let why = built.note ?? "Das gewählte Modell ist nicht verfügbar."
            return fallback.hasModel ? fallback.withNote("\(why) Es läuft stattdessen \(fallback.label).") : fallback.withNote(why)
        }
        return await automatic()
    }

    @MainActor
    private static func automatic() async -> AssistantEngine {
        let apple = await build(.apple)
        if apple.hasModel { return apple }
        let gemma = await build(.gemma)
        if gemma.hasModel { return gemma }
        return unavailable(noModelText, detail: apple.detail)
    }

    @MainActor
    private static func build(_ choice: AssistantModelChoice) async -> AssistantEngine {
        switch choice {
        case .apple: return apple()
        case .gemma: return await Task.detached(priority: .userInitiated) { gemma() }.value
        case .relay(let id): return relay(id, store: BookTalkRelayStore.shared)
        case .endpoint: return endpoint(AssistantEndpointStore.shared)
        }
    }

    // MARK: Apple

    private static func apple() -> AssistantEngine {
        switch FoundationBookTalkModel.availability {
        case .available:
            return AssistantEngine(model: FoundationBookTalkModel(),
                                   config: AssistantConfig(modelLabel: "Apple-Systemmodell", isCloud: false, contextTokens: BookTalkBudget.appleFM1.contextTokens,
                                                           maxAnswerTokens: 400, userName: nil, timeout: 45),
                                   choice: .apple, note: nil, detail: nil, llama: nil, target: nil)
        case .unavailable(let reason):
            let note: String
            var detail: String?
            switch reason {
            case .appleIntelligenceNotEnabled: note = "Apple Intelligence ist ausgeschaltet (iOS-Einstellungen)."
            case .deviceNotEligible: note = "Dieses iPhone unterstützt das Apple-Systemmodell nicht."
            case .modelNotReady:
                note = "Das Apple-Systemmodell wird noch geladen. Bitte später erneut versuchen."
                detail = "Das Apple-Systemmodell wird noch geladen."
            @unknown default: note = "Das Apple-Systemmodell ist auf diesem iPhone nicht verfügbar."
            }
            return .unavailable(note, detail: detail)
        @unknown default:
            return .unavailable("Das Apple-Systemmodell ist auf diesem iPhone nicht verfügbar.")
        }
    }

    // MARK: Gemma

    private static let gemmaCheckedKey = "book_talk_gemma_checked"

    /// Gemma-Datei schnell prüfen (nur Größe), für das Menü.
    static func gemmaInstalled() -> Bool {
        let manager = FileManager.default
        return BookTalkGemmaVariant.all.contains { variant in
            let file = ModelLibrary.folder.appendingPathComponent(variant.fileName)
            guard let attributes = try? manager.attributesOfItem(atPath: file.path) else { return false }
            return (attributes[.size] as? NSNumber)?.int64Value == variant.bytes
        }
    }

    /// Zugelassene Gemma-Datei, gleiche Prüfung und gleicher Merker wie im Buch-Gespräch (`BookTalkEngine`, dort privat).
    /// Der Hash wird je Datei einmal gerechnet (3 GB, einige Sekunden) und mit Größe und Änderungsdatum gemerkt.
    private static func gemmaFile() -> URL? {
        let manager = FileManager.default
        for variant in BookTalkGemmaVariant.all {
            let file = ModelLibrary.folder.appendingPathComponent(variant.fileName)
            guard let attributes = try? manager.attributesOfItem(atPath: file.path),
                  (attributes[.size] as? NSNumber)?.int64Value == variant.bytes else { continue }
            let stamp = "\(variant.sha256)|\((attributes[.modificationDate] as? Date)?.timeIntervalSince1970 ?? 0)"
            let defaults = UserDefaults.standard
            if defaults.stringArray(forKey: gemmaCheckedKey)?.contains(stamp) == true { return file }
            guard (try? BookTalkGemmaVariant.identify(file: file)) == variant else { continue }
            defaults.set((defaults.stringArray(forKey: gemmaCheckedKey) ?? []) + [stamp], forKey: gemmaCheckedKey)
            return file
        }
        return nil
    }

    /// Abseits des Hauptthreads aufrufen (Hash).
    private static func gemma() -> AssistantEngine {
        guard let file = gemmaFile() else { return .unavailable("Gemma 4 E2B ist nicht geladen (Einstellungen › Lokale Sprachmodelle).") }
        let context = BookTalkBudget.large.contextTokens
        let model = LlamaBookTalkModel(modelURL: file, contextTokens: context)
        return AssistantEngine(model: model,
                               config: AssistantConfig(modelLabel: "Gemma 4 E2B", isCloud: false, contextTokens: context, maxAnswerTokens: 600,
                                                       userName: nil, timeout: 60),
                               choice: .gemma, note: nil, detail: nil, llama: model, target: nil)
    }

    // MARK: Relay

    @MainActor
    private static func relay(_ id: String, store relays: BookTalkRelayStore) -> AssistantEngine {
        guard relays.enrollment != nil else { return .unavailable("Abo-Modelle brauchen eine Einrichtung am PC (Einstellungen › Buch-Gespräch).") }
        guard relays.usable else { return .unavailable("Die Einrichtung für Abo-Modelle ist abgelaufen oder gehört zu einem anderen Konto. Am PC neu freigeben.") }
        guard let info = relays.info(id), let model = relays.makeModel(info) else {
            return .unavailable("Dieses Abo-Modell ist am PC nicht freigegeben oder dein PC hat es nicht veröffentlicht.")
        }
        return AssistantEngine(model: model,
                               config: AssistantConfig(modelLabel: info.label, isCloud: true, contextTokens: info.contextTokens, maxAnswerTokens: 700,
                                                       userName: nil, timeout: 75),
                               choice: .relay(id), note: nil, detail: nil, llama: nil, target: .relay(info))
    }

    // MARK: Eigener Endpunkt

    @MainActor
    private static func endpoint(_ store: AssistantEndpointStore) -> AssistantEngine {
        guard let endpoint = store.endpoint else {
            return .unavailable("Es ist noch kein eigener Endpunkt eingerichtet (Menü › Eigener Endpunkt …).")
        }
        guard let model = store.makeModel() else { return .unavailable("Der eigene Endpunkt ist nicht benutzbar.") }
        let cloud = store.isCloud
        return AssistantEngine(model: model,
                               config: AssistantConfig(modelLabel: endpoint.label, isCloud: cloud, contextTokens: endpoint.contextTokens, maxAnswerTokens: 700,
                                                       userName: nil, timeout: 60),
                               choice: .endpoint, note: nil, detail: nil, llama: nil, target: cloud ? .endpoint(endpoint) : nil)
    }
}
