import SwiftUI
import UIKit
import Combine

// Gesprächsmodus „Sprechen“ (Paket A): Steuerung. Verbindet das durchgehende Zuhören (`AssistantListener`), die
// Entscheidungslogik für Vollduplex (`DuplexTurnPolicy`, Kern), die Antwort (`AssistantSession`, Kern) und die Stimme
// (`BookTalkSpeaker` im Gesprächsmodus). Muster wie `BookTalkController`: Vordergrundregel, Token/Generationen gegen
// verspätete Rückrufe, eine Kette für Abbruch und Verlauf, VoiceOver-Ansagen nur bei Zustandswechseln.
//
// Ablauf einer Äußerung: Pegel und Erkenner melden Sprache, Text und Stille. Die Policy entscheidet:
// - `speculate`: Antwort schon vorbereiten (lokale Modelle), gepuffert, nichts wird gesprochen;
// - `commit`: die Äußerung gilt; eine passende Vorarbeit wird übernommen, sonst beginnt eine neue Anfrage;
// - `cancelAnswer`: der Nutzer redet in die Antwort; vor dem ersten fertig gesprochenen Satz wird ergänzt (`merge`),
//   danach gilt es als Unterbrechung (das Gesprochene kommt als „unterbrochen“ in den Verlauf);
// - `holdAnswer`/`releaseAnswer`: Der Nutzer spricht, keine Einheit klingt: Die Stimme beginnt nichts Neues (auch keine Ansage), bis die
//   Richtlinie freigibt oder die Antwort endet; `holdStalled` zeigt einen Hinweis mit Handbedienung (`holdNotice`).
// Eine Antwort steht erst im Verlauf, wenn sie zu Ende gesprochen ist (oder unterbrochen wurde).

enum AssistantPhase: Equatable { case idle, listening, hearing, thinking, speaking }

/// Warum die Antwort zurückgehalten wird und nicht weiterkommt (die Richtlinie hält, solange der Nutzer spricht; `holdStalled`).
/// `activity`: Dauergeräusch ohne Wort: Hinweis „Ich warte, bis du fertig bist.“ mit beiden Knöpfen („Antwort jetzt hören“, „Verwerfen“).
/// `recognition`: Die Erkennung hinkt nach: „Die Erkennung kommt nicht hinterher.“, nur „Verwerfen“ (eine Handfreigabe wäre dann
/// verfrüht: Die Richtlinie nähme sie nicht an). Der Grund kann während eines Haltens wechseln; die Steuerung überschreibt dann den Wert.
enum AssistantHoldNotice: Equatable {
    case activity, recognition

    var text: String {
        switch self {
        case .activity: "Ich warte, bis du fertig bist."
        case .recognition: "Die Erkennung kommt nicht hinterher."
        }
    }

    /// Soll die Ansicht „Antwort jetzt hören“ (`releaseHold()`) anbieten?
    var offersRelease: Bool { self == .activity }

    /// Soll die Ansicht „Verwerfen“ (`discardHeldAnswer()`) anbieten? Immer.
    var offersDiscard: Bool { true }
}

/// Länge der Sprechpause, nach der eine Äußerung als beendet gilt (alle Fristen der Richtlinie werden mit `scale` gestreckt oder gekürzt).
enum AssistantPauseSetting: String, CaseIterable, Identifiable {
    case short, normal, long

    var id: String { rawValue }

    var scale: Double {
        switch self {
        case .short: 0.8
        case .normal: 1.0
        case .long: 1.5
        }
    }

    var title: String {
        switch self {
        case .short: "Kurz"
        case .normal: "Normal"
        case .long: "Lang"
        }
    }
}

/// Graue Denk-Anzeige unter der letzten Antwort (statt fester Floskeln): `active` solange noch gedacht wird (animierte Anzeige),
/// `steps` die Schritte in Reihenfolge, `draft` der gestreamte Rohtext des Denkers (leer, solange keiner kommt). Gehört zur Anfrage, die
/// zuletzt festgeschrieben wurde; der letzte Stand bleibt (inaktiv) stehen, bis die nächste Anfrage festgeschrieben wird. Nichts davon
/// wird gespeichert oder protokolliert.
struct AssistantThinking: Equatable {
    /// `true`: Es wird noch gedacht (animierte Anzeige).
    var active: Bool
    /// Schritte in Reihenfolge.
    var steps: [String]
    /// Gestreamter Rohtext des Denkers (`""`, solange keiner da ist).
    var draft: String
}

extension AssistantThinking {
    /// Höchstens so viele Titel nennt der Schritt „Auszüge gefunden“.
    private static let shownTitles = 4

    /// Der Schritt in Worten zu einem Fortschritt des Kerns; `nil`, wenn er keinen Schritt ergibt (`.speaker`, `.draft`).
    static func step(for progress: AssistantProgress) -> String? {
        switch progress {
        case .speaker, .draft:
            return nil
        case .searching(let query):
            return query.isEmpty ? "Suche in deinen Inhalten." : "Suche in deinen Inhalten: „\(query)“"
        case .found(let titles):
            guard !titles.isEmpty else { return "Nichts gefunden." }
            let noun = titles.count == 1 ? "1 Auszug" : "\(titles.count) Auszüge"
            let shown = titles.prefix(shownTitles).joined(separator: ", ")
            return "\(noun): \(shown)\(titles.count > shownTitles ? ", …" : "")"
        case .composing:
            return "Formuliere die Antwort."
        }
    }
}

struct AssistantNotice: Equatable {
    enum Action: Equatable { case retrySend, resume, retryStart, retryAudit, retryRelease, chooseModel, openLocalModels, openAreas }
    var text: String
    /// Zweite, kurze Zeile (nur wenn zutreffend, z. B. „Das Apple-Systemmodell wird noch geladen.“).
    var detail: String?
    var action: Action?
    /// Zweiter Knopf neben `action`.
    var secondary: Action?

    init(text: String, detail: String? = nil, action: Action?, secondary: Action? = nil) {
        self.text = text
        self.detail = detail
        self.action = action
        self.secondary = secondary
    }
}

/// Zustimmung vor dem ersten Versand an ein Cloud-Ziel (Anbieter/Host und betroffene Bereiche).
@MainActor
struct AssistantConsentRequest {
    let target: AssistantCloudTarget
    let areas: Set<AssistantArea>
    /// Die Sitzung, über die die Zustimmung läuft (Klasse A: Generation, Zeilen, Zustandsdatei). Beim Wechsel zu einem Cloud-Modell
    /// ist es eine Sitzung für das neue Ziel, die noch nicht übernommen ist; ohne sie kann nicht zugestimmt werden.
    let session: AssistantSession?
    var title: String { AssistantConsent.title(target) }
    var message: String { AssistantConsent.body(target, areas: areas, desktop: BookTalkRelayStore.shared.desktopName) }
}

/// Pegel für die Anzeige, eigenes Objekt, damit nur die Pegelansicht neu zeichnet.
@MainActor
final class AssistantLevelMeter: ObservableObject {
    @Published private(set) var level = 0.0

    func push(_ rms: Float) {
        let raw = Double(rms.isFinite ? max(0, rms) : 0).squareRoot() * 2.4 - 0.2
        let target = min(1, max(0, raw))
        let next = max(target, level * 0.85)
        if abs(next - level) > 0.04 || (next < 0.01 && level != 0) { level = next < 0.01 ? 0 : next }
    }

    func reset() { if level != 0 { level = 0 } }
}

/// Eine Anfrage an das Modell samt gepuffertem Strom (Vorarbeit liefert, bevor sie gilt).
private final class AssistantRun {
    /// Ein empfangenes `sentence`-Ereignis; seine Stelle in `sentences` ist die Nummer für die Satzspur im Protokoll.
    struct Sentence {
        let text: String
        let speech: String
        let status: AssistantSentenceStatus
        let sources: [AssistantPassage]
        /// Wörtlicher Auszug (mit Anführungszeichen und Quellzeile anzeigen).
        let quote: Bool
        /// Freigabe-Generation des Ereignisses; vor Anzeige und Audiostart gegen die aktuelle geprüft.
        let generation: Int
    }

    /// Ein empfangenes `progress`-Ereignis. `before`: Zahl der Sätze, die schon da waren, als es kam; es gilt vor dem Satz mit dieser
    /// Nummer (so bleibt die Reihenfolge von Sätzen und Fortschritt erhalten, auch wenn Vorarbeit gepuffert war).
    struct Progress {
        let progress: AssistantProgress
        let generation: Int
        let before: Int
    }

    let id = UUID().uuidString
    /// Freigabe-Generation des Laufs (aus dem ersten Ereignis); unbekannt, solange noch kein Ereignis da ist.
    var generation: Int?
    /// Fortschritt des Kerns, gepuffert wie die Sätze (Vorarbeit zeigt nichts, bis sie gilt); `progressShown` = schon angewendet.
    var progressEvents: [Progress] = []
    var progressShown = 0
    /// Eine Suche läuft: Ein Satz davor (Zwischensatz des Sprechers) beendet das Denken nicht, erst der erste Satz nach `composing`
    /// (oder nach „nichts gefunden“).
    var thinkingWaitsForFinal = false
    let text: String
    let afterInterruption: Bool
    var task: Task<Void, Never>?
    var committed = false
    var abandoned = false
    var cancelled = false
    var completed = false
    var sources: [AssistantPassage]?
    var sentences: [Sentence] = []
    var answer: AssistantAnswer?
    var failure: AssistantFailure?
    /// Der Strom ist zu Ende (`done` oder `failed`).
    var finished = false
    var begun = false
    var delivered = 0
    var inputEnded = false
    /// Indizes der an die Stimme gegebenen Sätze (Reihenfolge = Sprechreihenfolge).
    var enqueued: [Int] = []
    var startMs: [Int] = []
    var finishedSpeaking: [Int] = []
    /// Geschätztes Ende der Äußerung (ms seit Gesprächsbeginn) für „erster Ton“.
    var endedAtMs = 0
    /// Latenzmessung dieses Laufs (lva#521): läuft ab `beginAnswer` (Ende der Äußerung), endet in `completeAnswer`/`cancelAnswer`.
    /// Kalt/lokal/Modellart stehen beim Beginn fest (ein Modellwechsel mitten in der Runde ändert die Messung nicht).
    var latency = AssistantLatencyClock()
    var latencyCold = false
    var latencyLocal = true
    var latencyModel = AssistantModelKind.other

    init(text: String, afterInterruption: Bool) {
        self.text = text
        self.afterInterruption = afterInterruption
    }

    /// Puffert ein `progress`-Ereignis vor dem nächsten Satz. Der Rohtext des Denkers (`draft`) ist kumulativ: Ein noch nicht angewendeter
    /// Stand vor demselben Satz wird ersetzt, nicht angehängt (sonst wüchse der Puffer einer Vorarbeit quadratisch).
    func bufferProgress(_ event: AssistantProgress, generation: Int) {
        let entry = Progress(progress: event, generation: generation, before: sentences.count)
        if case .draft = event, progressEvents.count > progressShown, let last = progressEvents.last, case .draft = last.progress,
           last.before == entry.before {
            progressEvents[progressEvents.count - 1] = entry
            return
        }
        progressEvents.append(entry)
    }

    /// Nur die Einheiten, die ganz gesprochen wurden (Ansagen wie „Wörtlicher Auszug aus …“ gehören nicht zum Beitrag): ihre Nummern
    /// als Einheiten für den Kern. Der Beitrag besteht aus diesen Einheiten selbst, nicht aus einem Text: Ein Quellblock, dessen
    /// Ausgabe unterbrochen wurde, steht ganz darin (er wurde ganz gesprochen) oder gar nicht.
    var spokenUnits: AssistantSpokenUnits {
        let spoken = finishedSpeaking.filter { sentences.indices.contains($0) && sentences[$0].status != .announcement }
        return spoken.isEmpty ? .nothing : .events(spoken)
    }
}

@MainActor
final class AssistantController: ObservableObject {
    static let shared = AssistantController()

    static let bargeInKey = "assistant_barge_in"
    static var storedBargeIn: Bool { UserDefaults.standard.object(forKey: bargeInKey) as? Bool ?? true }
    static let pauseKey = "assistant_pause"
    static var storedPause: AssistantPauseSetting {
        UserDefaults.standard.string(forKey: pauseKey).flatMap(AssistantPauseSetting.init(rawValue:)) ?? .normal
    }
    static let styleKey = "assistant_style"
    /// Standard ist der freie Stil (`natural`); der wörtliche Pfad (`verbatim`) bleibt wählbar.
    static var storedStyle: AssistantStyle {
        UserDefaults.standard.string(forKey: styleKey).flatMap(AssistantStyle.init(rawValue:)) ?? .natural
    }
    /// Die erste Runde je Prozess misst „kalt“ (Latenzmessung, lva#521).
    private static var coldRoundPending = true

    // MARK: Veröffentlicht (Anzeige)

    @Published private(set) var phase: AssistantPhase = .idle
    @Published private(set) var turns: [AssistantTurn] = []
    /// Gesendete Äußerung, deren Antwort noch läuft oder fehlschlug (noch kein Beitrag im Verlauf).
    @Published private(set) var currentUser: String?
    /// Bei „Ergänzen“: die verworfene Äußerung, vor die der neue Text kommt.
    @Published private(set) var mergePrefix = ""
    @Published private(set) var liveStable = ""
    @Published private(set) var liveVolatile = ""
    /// Die laufende Antwort in Abschnitten, für die Anzeige: Jede Ausgabeeinheit trägt ihre Quellzeile (Zitat, ganzer Quellblock) oder ihre
    /// Kennzeichnung (feste Antwort, Bestandsangabe); es gibt keine gesammelte Quellenliste mehr. Es gibt keine zweite Fassung als bloßer Text
    /// (Runde 8, R1/U2): Die Ansicht zeigt nur Abschnitte mit vollständiger Marke.
    @Published private(set) var liveParts: [AssistantTurnPart] = []
    @Published private(set) var notice: AssistantNotice?
    /// Zustand der Freigaben, der bestehen bleibt, bis er behoben ist: „Freigaben prüfen“ (Zustandsdatei fehlt oder ungültig, nichts aus
    /// eigenen Inhalten) oder „Freigaben nicht gesichert“ (eine Einschränkung ließ sich nicht dauerhaft sichern).
    @Published private(set) var releaseAlert: AssistantNotice?
    @Published private(set) var modelLabel = "Kein Modell"
    @Published private(set) var isCloud = false
    @Published private(set) var selectedChoice: AssistantModelChoice?
    @Published private(set) var preparing: AssistantSources.Progress?
    /// Messwert: erster Ton in ms ab Äußerungsende.
    @Published private(set) var firstSoundMs: Int?
    @Published private(set) var muted = false
    @Published private(set) var bargeIn = AssistantController.storedBargeIn
    /// Sprechpause (Äußerungsende nach Inhalt, gestreckt oder gekürzt); wirkt auf die Richtlinie.
    @Published private(set) var pause = AssistantController.storedPause
    /// Antwortstil: `natural` (frei formuliert, Suche im Hintergrund) oder `verbatim` (nur wörtliche Auszüge). Wirkt ab der nächsten
    /// Anfrage; die Sitzung wird dazu neu aufgebaut, sobald nichts läuft.
    @Published private(set) var style = AssistantController.storedStyle
    /// Die Antwort wird zurückgehalten, weil der Nutzer spricht, und kommt nicht weiter: Hinweis mit „Antwort jetzt hören“
    /// (`releaseHold()`), bei `recognition` auch „Verwerfen“ (`discardHeldAnswer()`). `nil`, solange nichts hängt.
    @Published private(set) var holdNotice: AssistantHoldNotice?
    /// Graue Denk-Anzeige der zuletzt festgeschriebenen Anfrage (Schritte, Rohtext des Denkers). `nil`: nichts zu zeigen (keine Anfrage
    /// seit Beginn des Gesprächs, Abbruch ohne Schritt, Runde ohne Suche, Freigabe entzogen).
    @Published private(set) var thinking: AssistantThinking?
    @Published private(set) var running = false
    @Published private(set) var loading = false
    /// Bereiche, die der Assistent im aktuellen Modus (lokal oder Cloud) nutzen darf.
    @Published private(set) var areas: Set<AssistantArea> = []
    @Published private(set) var overflowCount = 0
    @Published var consentRequest: AssistantConsentRequest?
    let meter = AssistantLevelMeter()

    var liveUser: String {
        [mergePrefix, liveStable, liveVolatile].filter { !$0.isEmpty }.joined(separator: " ")
    }

    /// Ein Modell ist da: tippen und Senden gehen auch ohne Mikrofon.
    var canSend: Bool { session != nil }

    // MARK: Intern

    private let listener = AssistantListener()
    private let speaker = BookTalkSpeaker()
    private let store = AssistantHistoryStore(directory: AssistantHistoryStore.standardDirectory())
    private let audit = AssistantReleaseFiles.audit
    /// Die Zustandsdatei der Freigaben (Kern): einzige Quelle des Stands beim Start.
    private var releaseStore: AssistantReleaseStore { AssistantReleaseFiles.store }
    private var auditFailureShown = false
    /// Änderungen der Freigabe (Bereiche, Zustimmung) laufen nacheinander (Controller; der Kern reiht seine Sicherungsfolgen zusätzlich selbst).
    private var areaChain: Task<Void, Never>?
    private var grantingConsent = false
    private let hub = AssistantSources.shared
    private var engine = AssistantEngine.unavailable("")
    private var session: AssistantSession?
    private var policy = DuplexTurnPolicy()
    /// Die laufende Antwort. Solange eine läuft, fordert der Hörer bei Stille den Abschluss der Erkennung an (`catchUpOnSilence`).
    private var current: AssistantRun? {
        didSet { listener.catchUpOnSilence = current != nil }
    }
    private var speculation: AssistantRun?
    private var currentRunID: String?
    /// Abbruch, Verlauf und Aufräumen laufen hintereinander; neue Anfragen warten darauf.
    private var chain: Task<Void, Never>?
    private var saveChain: Task<Void, Never>?
    private var teardownTask: Task<Void, Never>?
    private var generation = 0
    private var opened = false
    private var starting = false
    private var historyLoaded = false
    private var origin = ProcessInfo.processInfo.systemUptime
    private var lastSilenceMs = 0
    /// Zeitpunkt (ms seit Gesprächsbeginn) der zuletzt gemeldeten Stille `lastSilenceMs`.
    private var lastSilenceAtMs = 0
    /// Eine Unterbrechung steht aus: die nächste Frage trägt `afterInterruption`.
    private var interruptedPending = false
    private var userMuted = false
    private var autoMuted = false
    private var retryText: String?
    private var consentContinuation: CheckedContinuation<Bool, Never>?
    /// „Neues Gespräch“: angefordert und in der Kette erledigt. Solange beides nicht gleich ist, schreiben ältere Aufträge
    /// den alten Verlauf nicht zurück (Anzeige, Datei) und eine neue Sitzung startet leer.
    private var clearRequested = 0
    private var clearDone = 0
    private var mergeWatch: Task<Void, Never>?
    /// Die Antwort ist zurückgehalten (Richtlinie: `holdAnswer` bis `releaseAnswer`/Abbruch/Ende); der Sprecher ist dann gehalten. Der
    /// Sprecher vergisst das Halten bei `begin()`/`stop()`; die Steuerung setzt es nach `begin()` wieder (`deliver`).
    private var holdActive = false
    /// Stil, mit dem die laufende Sitzung gebaut ist (ihre Konfiguration ist unveränderlich); weicht er von `style` ab, wird sie neu gebaut.
    private var appliedStyle = AssistantController.storedStyle
    private var refreshingSession = false
    /// Die Sprechpause wurde geändert, während eine Antwort lief: Die Richtlinie wird nach deren Ende neu gebaut (ihr Zustand ginge sonst verloren).
    private var policyStale = false
    private var bag: Set<AnyCancellable> = []
    private var memoryObserver: NSObjectProtocol?
    private var overflowShown = false

    init() {
        hub.$progress.receive(on: RunLoop.main).sink { [weak self] value in
            MainActor.assumeIsolated { self?.preparing = value }
        }.store(in: &bag)

        listener.onSpeechStarted = { [weak self] in self?.speechStarted() }
        // Zweiter Wert: Rückstand des Ergebnisendes hinter dem eingespeisten Audio in Sekunden (nil, wenn die Erkennung keinen Zeitbereich liefert).
        listener.onPartial = { [weak self] text, lag in self?.partial(text, lag: lag) }
        listener.onSilence = { [weak self] ms in self?.silence(ms) }
        listener.onInterrupted = { [weak self] why in self?.listenerInterrupted(why) }
        listener.onLevel = { [weak self] level in self?.meter.push(level) }
        listener.onOverflow = { [weak self] count in self?.overflow(count) }
        // Vor jedem `onSilence`: Stand der Erkennung an die Richtlinie (Freigabe des Haltens erst bei eingeholter Erkennung).
        listener.onRecognition = { [weak self] caughtUp, following in self?.recognition(caughtUp, following: following) }

        speaker.onAudible = { [weak self] in self?.audible() }
        speaker.onSentenceStarted = { [weak self] text in self?.sentenceStarted(text) }
        speaker.onSentenceFinished = { [weak self] text in self?.sentenceFinished(text) }
        speaker.onUnitStarted = { [weak self] in self?.unitStarted() }
        speaker.onUnitEnded = { [weak self] in self?.unitEnded() }
        speaker.onIdle = { [weak self] in self?.speakerIdle() }
        // Schreibfehler des Protokolls (E4.2b, Schreibfolge 7): Die Ausgabe stoppt SOFORT, auch mitten in der Wiedergabe und ohne
        // dass eine weitere Einheit startet. Der Beobachter kommt von der Schreibwarteschlange; die Arbeit geschieht auf dem MainActor.
        audit.setFailureObserver { [weak self] key in
            Task { @MainActor in self?.auditWriteFailed(key: key) }
        }
    }

    private var nowMs: Int { Int((ProcessInfo.processInfo.systemUptime - origin) * 1000) }

    #if DEBUG
    // MARK: Beispielrunde (nur DEBUG)

    /// UI-Tests: `--assistant-demo-quote` öffnet das Gespräch mit einer eingebauten Beispielrunde aus synthetischem Inhalt (ein
    /// wörtlicher Auszug, ein ganzer Block, mit Quellzeile). `--assistant-demo-multi` öffnet stattdessen die Beispielrunde mit
    /// mehreren Blöcken (Marke je Block: drei Auszüge aus zwei Quellen, einer davon mehrzeilig, je mit eigener Quellzeile „Wörtlicher
    /// Auszug aus … · S. 12“, dazu ein Satz mit Kennzeichnung). `--assistant-demo-locked` zeigt den Zustand „Freigaben prüfen“
    /// (Zustandsdatei fehlt oder ungültig), `--assistant-demo-audit` den Zustand „Protokoll nicht schreibbar“, je mit der
    /// Beispielrunde darunter. Kein Modell, kein Abruf, kein Speicher, kein Protokoll, keine Zustandsdatei: `start` kehrt sofort
    /// zurück, und Verlauf, Protokoll und Freigaben bleiben unberührt (`persist`, `deleteStoredHistory`, `revokeConsent`).
    /// `--assistant-demo-interrupted`: eine unterbrochene Antwort (der ganz gesprochene Auszug mit Quellzeile, die Unterbrechung als
    /// eigene Zeile darunter). `--assistant-demo-legacy`: ältere Beiträge ohne Marken fallen beim Laden heraus, gezeigt wird nur der
    /// gültige.
    static let demoMulti = ProcessInfo.processInfo.arguments.contains("--assistant-demo-multi")
    static let demoLocked = ProcessInfo.processInfo.arguments.contains("--assistant-demo-locked")
    static let demoAudit = ProcessInfo.processInfo.arguments.contains("--assistant-demo-audit")
    static let demoInterrupted = ProcessInfo.processInfo.arguments.contains("--assistant-demo-interrupted")
    static let demoLegacy = ProcessInfo.processInfo.arguments.contains("--assistant-demo-legacy")
    static let demoQuote = demoMulti || demoLocked || demoAudit || demoInterrupted || demoLegacy
        || ProcessInfo.processInfo.arguments.contains("--assistant-demo-quote")

    private func showDemo() {
        opened = true
        historyLoaded = true
        notice = Self.demoAudit ? Self.auditUnavailableNotice() : nil
        releaseAlert = Self.demoLocked ? Self.releaseLockedNotice : nil
        if Self.demoLegacy {
            // Wie beim Laden und Wiederherstellen: Beiträge ohne gespeicherte, vollständig markierte Einheiten fallen heraus.
            turns = AssistantTurn.admitted(Self.demoLegacyTurns, released: AssistantUnsourcedConsent.isReleased)
        } else if Self.demoInterrupted {
            turns = Self.demoInterruptedTurns
        } else {
            turns = Self.demoMulti ? Self.demoMultiTurns : Self.demoTurns
        }
        modelLabel = "Beispiel"
    }

    /// Ein gespeicherter Block der Beispielrunde (`isBlock: true`: ganz, unverändert, zitierbar).
    private static func demoBlock(_ area: AssistantArea, _ id: String, _ title: String, _ locator: String, _ text: String) -> AssistantPassage {
        AssistantPassage(area: area, itemId: id, title: title, locator: locator, text: text, score: 1, isBlock: true, quotable: true, raw: text)
    }

    /// Beispielrunde mit mehreren Blöcken (synthetisch): drei wörtliche Auszüge aus zwei Quellen, der zweite mehrzeilig, je Block
    /// seine Quellzeile, dazu eine feste Antwort mit ihrer Kennzeichnung. Die Abschnitte entstehen mit derselben Zuordnung wie bei
    /// einer echten Antwort (`AssistantTurnPart.append`).
    private static var demoMultiTurns: [AssistantTurn] {
        let book = "Anna übernimmt die Planung und stimmt sich jede Woche mit dem Team ab."
        let book2 = "Die Abnahme der ersten Stufe ist für das Frühjahr vorgesehen.\nDie zweite Stufe folgt im Herbst, sobald das Budget freigegeben ist."
        let talk = "Das Budget für Projekt Nordlicht wurde auf zwei Etappen verteilt."
        let p1 = demoBlock(.books, "demo-buch", "Beispielbuch", "S. 12", book)
        let p2 = demoBlock(.books, "demo-buch", "Beispielbuch", "S. 14", book2)
        let p3 = demoBlock(.transcripts, "demo-besprechung", "Besprechung Nordlicht", "00:14:03", talk)
        let fixed = AssistantGrounding.fixedSentence(allowUnsourced: false, hint: AssistantUnsourcedConsent.isReleased)
        var parts: [AssistantTurnPart] = []
        AssistantTurnPart.append(to: &parts, text: book, status: .grounded, quote: true, sources: [p1])
        AssistantTurnPart.append(to: &parts, text: book2, status: .grounded, quote: true, sources: [p2])
        AssistantTurnPart.append(to: &parts, text: talk, status: .grounded, quote: true, sources: [p3])
        AssistantTurnPart.append(to: &parts, text: fixed, status: .neutral, quote: false, sources: [])
        return [
            AssistantTurn(user: "Wie steht es um die Planung und das Budget?", assistant: [book, book2, talk, fixed].joined(separator: " "),
                          sources: [p1, p2, p3], parts: parts, model: "Beispiel", round: .data),
        ]
    }

    /// Beispielrunde „unterbrochen“ (`--assistant-demo-interrupted`): Der erste Auszug wurde ganz gesprochen und steht mit seiner
    /// Quellzeile im Beitrag; der zweite wurde unterbrochen und steht GAR NICHT darin (ganz oder gar nicht, nie gekürzt). Die
    /// Unterbrechung zeigt die Ansicht als eigene Zeile unter dem Beitrag, nie im Text des Auszugs.
    private static var demoInterruptedTurns: [AssistantTurn] {
        let quote = "Anna übernimmt die Planung und stimmt sich jede Woche mit dem Team ab."
        let passage = demoBlock(.books, "demo-buch", "Beispielbuch", "S. 12", quote)
        return [
            AssistantTurn(user: "Wer übernimmt die Planung und wie geht es weiter?", assistant: quote, interrupted: true, sources: [passage],
                          parts: [AssistantTurnPart(text: quote, unsourced: false, quote: true, source: passage.excerptLine)],
                          model: "Beispiel", round: .data),
        ]
    }

    /// Beispielrunde „ältere Beiträge“ (`--assistant-demo-legacy`): ein Beitrag ohne gespeicherte Einheiten (Text ohne Marke), einer
    /// mit einem Zitat ohne Quellzeile und ein gültiger. Gezeigt wird nur der gültige; die beiden anderen fallen beim Laden heraus
    /// (`AssistantTurn.admitted`) und erscheinen nirgends, weder als Text noch als „Quelle nicht gespeichert“.
    private static var demoLegacyTurns: [AssistantTurn] {
        let quote = "Anna übernimmt die Planung und stimmt sich jede Woche mit dem Team ab."
        let passage = demoBlock(.books, "demo-buch", "Beispielbuch", "S. 12", quote)
        let old = "Dieser ältere Beitrag ohne gespeicherte Marke darf nie erscheinen."
        let unsourcedQuote = "Dieses ältere Zitat ohne Quellzeile darf nie erscheinen."
        return [
            AssistantTurn(user: "Ältere Frage ohne Marke", assistant: old, sources: [passage], model: "Beispiel", round: .data),
            AssistantTurn(user: "Ältere Frage mit Zitat ohne Quelle", assistant: unsourcedQuote,
                          parts: [AssistantTurnPart(text: unsourcedQuote, unsourced: false, quote: true)], model: "Beispiel", round: .data),
            AssistantTurn(user: "Was steht im Beispielbuch über die Planung?", assistant: quote, sources: [passage],
                          parts: [AssistantTurnPart(text: quote, unsourced: false, quote: true, source: passage.excerptLine)],
                          model: "Beispiel", round: .data),
        ]
    }

    /// So steht der Beitrag nach einer echten Antwort im Verlauf: ein wörtlicher Auszug (ganzer Block) mit Quellzeile (Datenrunde).
    /// Die hörbare Ansage („Wörtlicher Auszug aus ‚Beispielbuch‘:“) gehört nicht zum Text und erscheint in der Ansicht nie; sichtbar
    /// ist die Quellzeile „Wörtlicher Auszug aus Beispielbuch · S. 12“. Eine Runde ohne Quelle gibt es nicht (der freie Modus ist
    /// abgeschaltet), also auch keine Beispielrunde dazu.
    private static var demoTurns: [AssistantTurn] {
        let quote = "Anna übernimmt die Planung und stimmt sich jede Woche mit dem Team ab."
        let passage = demoBlock(.books, "demo-buch", "Beispielbuch", "S. 12", quote)
        return [
            AssistantTurn(user: "Was steht im Beispielbuch über die Planung?", assistant: quote, sources: [passage],
                          parts: [AssistantTurnPart(text: quote, unsourced: false, quote: true, source: passage.excerptLine)],
                          model: "Beispiel", round: .data),
        ]
    }
    #endif

    // MARK: Start und Ende

    /// Gespräch beginnen: Wiedergabe der Bibliothek und alte Sprachausgabe stoppen, Modell wählen (Zustimmung bei Cloud),
    /// Zuhören starten. Der Erstaufbau des Indexes läuft im Hintergrund weiter und hält nichts auf.
    func start(choice explicit: AssistantModelChoice? = nil) async {
        guard !running, !starting else { return }
        #if DEBUG
        if Self.demoQuote { showDemo(); return }
        #endif
        opened = true
        starting = true
        loading = true
        generation += 1
        let mine = generation
        notice = nil
        retryText = nil
        await teardownTask?.value
        guard mine == generation else { return }

        if LibraryPlayback.shared.busy { LibraryPlayback.shared.pause() }
        VoiceModel.shared.yieldToAssistant()
        installGuards()
        hub.open()   // Klasse B: der Bestand gilt als nicht aktuell, bis ein Durchgang nach diesem Moment endet; die Meldung stößt ihn an
        if !historyLoaded {
            historyLoaded = true
            // Beiträge aus Runden ohne Quelle sind bei abgeschaltetem freien Modus nie sichtbar und nie im Prompt; die Datei bleibt
            // bis zum nächsten Speichern unberührt, dann fehlen sie (`persist` schreibt, was angezeigt wird).
            turns = AssistantTurn.admitted(store.load(), released: AssistantUnsourcedConsent.isReleased)
        }

        var built = await AssistantEngine.make(choice: explicit)
        guard mine == generation else { built.llama?.release(); return }
        var declined = false
        if let target = built.target, !AssistantConsent.isGranted(target, areas: AssistantAreaSettings.areas(cloud: true)) {
            let via = await consentSession(for: built)
            guard mine == generation else { built.llama?.release(); return }
            let allowed = await askConsent(target, areas: AssistantAreaSettings.areas(cloud: true), via: via)
            guard mine == generation else { built.llama?.release(); return }
            if !allowed {
                declined = true
                built = await localFallback()
            }
        }
        await adopt(built)
        guard mine == generation else { return }
        if declined { notice = AssistantNotice(text: "Ohne Zustimmung läuft kein Cloud-Modell\(engine.hasModel ? ". Es läuft \(engine.label)." : ".")", action: nil) }
        guard engine.hasModel else {
            notice = noModelNotice(declined: declined)
            finishStart(mine)
            return
        }
        if let hint = engine.note, !declined { notice = AssistantNotice(text: hint, action: nil) }

        origin = ProcessInfo.processInfo.systemUptime
        lastSilenceMs = 0
        lastSilenceAtMs = 0
        interruptedPending = false
        overflowShown = false
        overflowCount = 0
        thinking = nil
        clearHold()
        // Eine geänderte Sprechpause, die noch aussteht, gilt ab diesem Gespräch (`rebuildPolicy` setzt den Zustand ohnehin zurück).
        if policyStale { rebuildPolicy() } else { policy.reset() }
        prewarmModel()
        speaker.voiceID = VoiceModel.shared.selectedVoiceID
        speaker.conversationMode = true
        speaker.prepare()
        do {
            try await listener.start(languageCode: nil)
        } catch is CancellationError {
            // Abbruch durch `end()`/`suspend()` (Generation weitergezählt, Zustand dort zurückgesetzt) oder durch die Aufgabe selbst.
            if mine == generation { speaker.conversationMode = false; finishStart(mine) }
            return
        } catch {
            guard mine == generation else { return }
            speaker.conversationMode = false
            notice = AssistantNotice(text: error.localizedDescription, action: .retryStart)
            finishStart(mine)
            return
        }
        guard mine == generation else { return }
        userMuted = false
        autoMuted = false
        muted = false
        running = true
        phase = .listening
        finishStart(mine)
        announce("Gespräch gestartet. Ich höre zu.")
    }

    /// „Kein Modell verfügbar.“ kurz, mit zwei Wegen heraus. Eine Zeile Ursache nur, wenn sie zutrifft und hilft: das
    /// Systemmodell lädt noch, oder das vom Nutzer gewählte Modell fehlt (dann dessen Grund, z. B. „Gemma … nicht geladen“).
    private func noModelNotice(declined: Bool = false) -> AssistantNotice {
        let reason = declined ? nil : engine.note.flatMap { $0.isEmpty || $0 == AssistantEngine.noModelText ? nil : $0 }
        return AssistantNotice(text: declined ? "Ohne Zustimmung läuft kein Cloud-Modell, und es ist kein lokales Modell verfügbar." : AssistantEngine.noModelText,
                               detail: reason ?? engine.detail, action: .chooseModel, secondary: .openLocalModels)
    }

    private func finishStart(_ mine: Int) {
        guard mine == generation else { return }
        starting = false
        loading = false
    }

    /// Schließen: alles beenden, Mikrofon und Modell freigeben, Verlauf sichern.
    func end() {
        guard opened || running || starting else { return }
        opened = false
        generation += 1
        starting = false
        loading = false
        resolveConsent(false)
        shutdown(announceText: "Gespräch beendet.")
    }

    /// Hält an, ohne die Ansicht zu schließen („Weiter“ startet neu): Anruf, Hintergrund, Fehler des Erkenners.
    private func suspend(_ text: String, action: AssistantNotice.Action?) {
        generation += 1
        starting = false
        loading = false
        resolveConsent(false)
        shutdown(announceText: text)
        notice = AssistantNotice(text: text, action: action)
    }

    private func shutdown(announceText: String) {
        // Klasse B: Die Meldestelle hängt an der Ordnung des Prozesses, nicht an einem Gespräch (sie wirkt weiter); die Quellen stoßen
        // ohne Gespräch keine Indexierung mehr an.
        hub.close()
        if current != nil { cancelAnswer(merge: false) }
        discardSpeculation()
        // `stop()` löscht auch das Halten des Sprechers; der Zustand der Steuerung und der Hinweis folgen.
        speaker.stop()
        clearHold()
        speaker.conversationMode = false
        running = false
        phase = .idle
        meter.reset()
        clearLive()
        mergePrefix = ""
        userMuted = false
        autoMuted = false
        muted = false
        LibraryPlayback.shared.remoteIntercept = nil
        if let memoryObserver { NotificationCenter.default.removeObserver(memoryObserver) }
        memoryObserver = nil
        let listener = self.listener
        let llama = engine.llama
        let pending = chain
        teardownTask = Task {
            await listener.stopAndWait()
            await pending?.value
            llama?.release()
        }
        announce(announceText)
    }

    /// Vordergrundregel: sobald die App nicht mehr aktiv ist, endet das Gespräch (kein Mikrofon im Hintergrund).
    func sceneInactive(background: Bool) {
        guard opened, !starting else { return }
        if background { engine.llama?.release() }
        guard running else { return }
        suspend("Das Gespräch wurde angehalten. Tippe auf Weiter.", action: .resume)
    }

    func resume() { Task { await start() } }

    private func installGuards() {
        LibraryPlayback.shared.remoteIntercept = { [weak self] command in
            guard let self else { return false }
            // Läuft eine Antwort, hält jede Fernbedienung sie an. Sonst darf „Play“ nicht unter dem Gespräch vorlesen.
            if self.phase == .thinking || self.phase == .speaking { self.stopAnswer(); return true }
            return command != .pause
        }
        if memoryObserver == nil {
            memoryObserver = NotificationCenter.default.addObserver(forName: UIApplication.didReceiveMemoryWarningNotification, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.memoryWarning() }
            }
        }
    }

    /// Speicherwarnung: das große Modell nur freigeben, wenn gerade nichts damit läuft (es lädt beim nächsten Mal neu).
    private func memoryWarning() {
        guard phase != .thinking, phase != .speaking else { return }
        engine.llama?.release()
    }

    private func localFallback() async -> AssistantEngine {
        let apple = await AssistantEngine.make(choice: .apple)
        if apple.hasModel { return apple }
        let gemma = await AssistantEngine.make(choice: .gemma)
        return gemma.hasModel ? gemma : apple
    }

    /// Eine Sitzung für das neue Cloud-Ziel, über die die Zustimmung läuft, BEVOR das Modell übernommen ist (Klasse A: die Erteilung
    /// geht durch den Kern: Zeile „beabsichtigt“, Zustandsdatei, erst dann wirksam, Zeile „wirksam“). Sie wird nicht übernommen; die
    /// übernommene Sitzung liest die Zustimmung danach aus der Zustandsdatei. `nil` ohne Modell.
    private func consentSession(for built: AssistantEngine) async -> AssistantSession? {
        guard let model = built.model, let target = built.target else { return nil }
        let kn = await hub.knowledge(cloud: true, areas: AssistantAreaSettings.areas(cloud: true))
        return AssistantSession(model: model, knowledge: kn, config: built.sessionConfig(style: style), audit: audit, cloudConsent: false,
                                releaseStore: releaseStore, cloudTarget: target.id)
    }

    /// Modell, Wissen und Sitzung übernehmen; der Verlauf bleibt. Bereiche und Zustimmung kommen aus der Zustandsdatei (über die
    /// Sitzung); ist sie gesperrt, sind beide leer.
    private func adopt(_ built: AssistantEngine) async {
        let previous = engine
        engine = built
        if previous.llama !== built.llama { previous.llama?.release() }
        modelLabel = built.label
        isCloud = built.isCloud
        selectedChoice = built.choice
        let active = AssistantAreaSettings.areas(cloud: built.isCloud)
        areas = active
        let kn = await hub.knowledge(cloud: built.isCloud, areas: active)
        let stored = AssistantTurn.admitted(await session?.history ?? turns, released: AssistantUnsourcedConsent.isReleased)
        // Nach dem letzten Wartepunkt: steht „Neues Gespräch“ noch aus, kommt der alte Verlauf nicht in die neue Sitzung.
        let history = clearRequested == clearDone ? stored : []
        session?.setReleaseObserver(nil)
        if let model = built.model {
            // Die Zustimmung zum Cloud-Ziel kommt aus der Zustandsdatei (für lokale Modelle ohne Belang). Der Kern lässt ein
            // Cloud-Modell ohne Zustimmung gar nicht erst laufen (`consentRequired`).
            let next = AssistantSession(model: model, knowledge: kn, config: built.sessionConfig(style: style), history: history, audit: audit,
                                        cloudConsent: false, releaseStore: releaseStore, cloudTarget: built.target?.id)
            observeRelease(of: next)
            speaker.gate = next.playbackGate
            session = next
            appliedStyle = style
            // Ein Lesen für Bereiche und Zustimmung (nicht mehrere Lesungen, zwischen denen sich der Stand ändern kann).
            let effective = next.effectiveState
            areas = effective.areas
            // Der Speicher des Ziels kennt die Zustimmung nicht mehr (Widerruf in den Einstellungen): sie ist auch in der Datei weg.
            if let target = built.target, effective.cloudConsent, !AssistantConsent.isGranted(target, areas: effective.areas) {
                revokeConsent(areas: effective.areas)
            }
        } else {
            speaker.gate = nil
            session = nil
        }
        turns = history
        rebuildPolicy()
        refreshReleaseAlert()
    }

    /// Rückruf des Kerns bei JEDER Erhöhung der Freigabe-Generation (Bereiche, Zustimmung; Opt-in und Fortsetzen des
    /// freien Modus gibt es in der Produktfassung nicht, der Kern kennt sie nur für Tests). Er kommt auf beliebigem Thread und springt auf den MainActor; der Controller hält dann Stimme und Warteschlange an,
    /// auch wenn kein Satz mehr unterwegs ist, den der Kern selbst noch widerrufen könnte. Was SYNCHRON beim Auslöser wirkt,
    /// liegt im Kern: Die Wiedergabesperre (`speaker.gate`) ist beim Rückkehren der Erhöhung schon vorgerückt, ein neuer Start
    /// eines Satzes älterer Generation ist damit ausgeschlossen. Der Speaker selbst gehört dem MainActor und lässt sich nicht
    /// von diesem Thread stoppen: Ein schon klingender Satz verstummt erst mit `stop()` hier (MainActor).
    private func observeRelease(of session: AssistantSession) {
        let id = ObjectIdentifier(session)
        session.setReleaseObserver { [weak self] generation in
            Task { @MainActor in self?.releaseBumped(generation, of: id) }
        }
    }

    /// Eine laufende Antwort, deren Generation älter ist als die neue, verstummt sofort: Wiedergabe stoppt, die Warteschlange
    /// wird geleert. Ausdrückliche Widerrufe (`stopForRevocation` mit eigenem Text) haben `current` schon geleert und gewinnen.
    /// Ein Lauf ohne Ereignis ist noch nicht hörbar; ihn widerruft der Kern (`failed(.revoked)`). Eine verspätete Meldung
    /// einer früheren Sitzung oder für einen jüngeren Lauf bleibt wirkungslos.
    private func releaseBumped(_ generation: Int, of observed: ObjectIdentifier) {
        guard let session, ObjectIdentifier(session) == observed else { return }
        // Die Anzeige der Bereiche folgt dem Stand der Ordnung nach JEDER Erhöhung, auch einer anderen Sitzung, einer Änderung ohne
        // Sitzung oder der Blockade (Runde 8, R3: keine Kopie, die dem Stand hinterherläuft).
        areas = session.effectiveAreas
        guard let run = current, !run.completed, let started = run.generation, started < generation else { return }
        stopForRevocation(Self.revokedText, retry: run.text)
    }

    private func rebuildPolicy() {
        var config = DuplexTurnPolicy.Config()
        config.speculate = !engine.isCloud
        config.bargeIn = bargeIn
        config.pauseScale = pause.scale
        config.adaptiveEnd = true
        policy = DuplexTurnPolicy(config: config)
        policyStale = false
        // Die neue Richtlinie kennt das Halten nicht: Die Antwort darf nicht gehalten zurückbleiben (ohne Hinweis, ohne Freigabe).
        if holdActive {
            clearHold()
            speaker.release()
        }
    }

    /// Die Richtlinie trägt ihre Fristen unveränderlich; eine Änderung der Sprechpause baut sie neu, aber nur, wenn ihr Zustand nichts
    /// mehr braucht (keine Antwort, keine Vorarbeit, keine zusammenzuführende Äußerung). Sonst nach dem Ende der Antwort.
    private func applyPolicyConfig() {
        if policyIsIdle { rebuildPolicy() } else { policyStale = true }
    }

    /// Die Richtlinie trägt keinen Zustand, der verloren gehen dürfte: keine Antwort, keine Vorarbeit, keine zusammenzuführende und keine
    /// laufende Äußerung.
    private var policyIsIdle: Bool {
        current == nil && speculation == nil && mergePrefix.isEmpty && liveStable.isEmpty && liveVolatile.isEmpty
    }

    /// Die Sitzung trägt ihre Konfiguration (auch den Stil) unveränderlich (`AssistantSession.config`). Weicht `style` vom Stil der
    /// Sitzung ab, wird sie mit derselben Engine, demselben Verlauf und denselben Freigaben neu aufgebaut, sobald nichts läuft; der
    /// Anstoß kommt bei der Änderung und am Ende jeder Antwort. Bis dahin gilt der bisherige Stil.
    private func refreshSessionStyle() {
        guard session != nil, appliedStyle != style, !refreshingSession else { return }
        refreshingSession = true
        Task { [weak self] in
            await self?.installSessionForStyle()
            self?.refreshingSession = false
        }
    }

    /// Alle Wartepunkte (Kette der Anfragen, Wissen, Verlauf der alten Sitzung) liegen VOR der Prüfung; zwischen Prüfung und Tausch gibt
    /// es keinen mehr. Eine Anfrage, die dazwischen begann, hätte die alte Sitzung gebunden: Die Prüfung (`current`, `speculation`) sieht
    /// sie, und es wird nach kurzer Pause noch einmal versucht (höchstens dreimal; danach bleibt es für das Ende der nächsten Antwort).
    private func installSessionForStyle() async {
        for attempt in 0..<3 {
            guard let old = session, let model = engine.model, appliedStyle != style else { return }
            if attempt > 0 { try? await Task.sleep(for: .milliseconds(250)) }
            let built = engine
            let waiting = chain
            await waiting?.value
            let knowledge = await hub.knowledge(cloud: built.isCloud, areas: AssistantAreaSettings.areas(cloud: built.isCloud))
            let stored = AssistantTurn.admitted(await old.history, released: AssistantUnsourcedConsent.isReleased)
            guard session === old, chain == waiting, current == nil, speculation == nil, mergePrefix.isEmpty, consentRequest == nil,
                  clearRequested == clearDone else { continue }
            old.setReleaseObserver(nil)
            let next = AssistantSession(model: model, knowledge: knowledge, config: built.sessionConfig(style: style), history: stored,
                                        audit: audit, cloudConsent: false, releaseStore: releaseStore, cloudTarget: built.target?.id)
            observeRelease(of: next)
            speaker.gate = next.playbackGate
            session = next
            appliedStyle = style
            areas = next.effectiveAreas
            refreshReleaseAlert()
            return
        }
    }

    // MARK: Einstellungen im Gespräch

    func selectModel(_ choice: AssistantModelChoice) {
        guard opened, !loading else { return }
        #if DEBUG
        // UI-Tests ohne Modell: Die Wahl (z. B. beim Schließen der Modellwahl) bleibt folgenlos, auch für spätere Starts.
        if !ProcessInfo.processInfo.arguments.contains("--assistant-no-model") { AssistantModelChoice.store(choice) }
        #else
        AssistantModelChoice.store(choice)
        #endif
        Task { await switchModel(choice) }
    }

    private func switchModel(_ choice: AssistantModelChoice) async {
        guard running else {
            await start(choice: choice)
            return
        }
        let mine = generation
        if current != nil { cancelAnswer(merge: false) }
        discardSpeculation()
        listener.consumeUtterance()
        clearLive()
        await chain?.value
        let built = await AssistantEngine.make(choice: choice)
        guard mine == generation else { built.llama?.release(); return }
        if let target = built.target, !AssistantConsent.isGranted(target, areas: AssistantAreaSettings.areas(cloud: true)) {
            let via = await consentSession(for: built)
            guard mine == generation else { built.llama?.release(); return }
            let allowed = await askConsent(target, areas: AssistantAreaSettings.areas(cloud: true), via: via)
            guard mine == generation else { built.llama?.release(); return }
            if !allowed {
                notice = AssistantNotice(text: "Ohne Zustimmung bleibt \(engine.label) aktiv.", action: nil)
                return
            }
        }
        guard built.hasModel else {
            notice = AssistantNotice(text: built.note ?? "Dieses Modell ist nicht verfügbar.", action: nil)
            return
        }
        // Während der Wartepunkte kann eine Antwort mit der alten Sitzung begonnen haben: abbrechen, bevor die neue übernimmt,
        // sonst schriebe ein Lauf der alten Sitzung in die neue. Die Kette wartet, bis der Beitrag der alten Sitzung steht.
        if current != nil { cancelAnswer(merge: false) }
        discardSpeculation()
        await chain?.value
        guard mine == generation else { built.llama?.release(); return }
        await adopt(built)
        notice = built.note.map { AssistantNotice(text: $0, action: nil) }
        prewarmModel()
        announce("Modell: \(built.label)")
    }

    /// Das Apple-Systemmodell vorladen, solange der Nutzer noch nicht spricht (nur wenn es gewählt und verfügbar ist; kein Netz, keine
    /// Inhalte, nichts wird geschrieben). Für alle anderen Modelle ohne Wirkung.
    private func prewarmModel() {
        guard engine.choice == .apple, engine.hasModel else { return }
        AssistantEngine.prewarmApple()
    }

    func toggleArea(_ area: AssistantArea) {
        var next = areas
        if next.contains(area) { next.remove(area) } else { next.insert(area) }
        applyAreas(next)
    }

    /// Bereiche ändern (Klasse A), in fester Reihenfolge:
    /// 1. Kern, SYNCHRON: EINE Erhöhung der Generation. Was einschränkt (Bereich weg, Zustimmung weg), gilt sofort; die laufende
    ///    Antwort endet. Erweiterungen (Bereich dazu) gelten noch nicht.
    /// 2. In der Kette: `settle` ersetzt die Zustandsdatei dauerhaft (Einschränkung: danach die Zeile `release`; Erweiterung: erst
    ///    Zeile „beabsichtigt“, dann die Datei, dann wirksam). Misslingt das, bleiben Einschränkungen in Kraft, Erweiterungen nicht, und
    ///    der Zustand wird gemeldet. Es gibt keine zweite Einstellung daneben: Die Datei ist die Quelle.
    /// 3. Erst danach gilt die Anzeige der WIRKSAMEN Bereiche (nicht der gewünschten).
    /// Ohne Sitzung (kein Modell) macht der Speicher im Kern (`AssistantReleaseStore.updateAreas`) dasselbe: Datei ersetzen und die
    /// Zeilen des Protokolls schreiben (Einschränkung: Datei, dann `release`; Erweiterung: „beabsichtigt“, Datei, „wirksam“), nur ohne
    /// Generation, denn ohne Sitzung gibt es keine. Eine gescheiterte Einschränkung merkt der Speicher als engeren Stand.
    /// Für ein Cloud-Ziel geht die Zustimmung für den neuen Stand mit in die Änderung ein: Reichen die Bereiche der Zustimmung
    /// nicht, gilt sie nicht mehr (sofort), bis der Nutzer zustimmt; gesendet wird in der Zwischenzeit nichts.
    private func applyAreas(_ next: Set<AssistantArea>) {
        let previous = areas
        let cloud = engine.isCloud
        let target = engine.target
        let session = self.session
        areas = next
        let pending = session?.applyAreas(next, cloudConsent: target.map { AssistantConsent.isGranted($0, areas: next) })
        // Ein Bereich, aus dem die laufende Antwort Auszüge hat, wird entzogen: wartende Sätze werden nicht mehr gesprochen.
        let removed = previous.subtracting(next)
        // Titel und Rohtext aus einem entzogenen Bereich bleiben nicht stehen (die Denk-Anzeige kennt die Herkunft nicht: ganz weg).
        if !removed.isEmpty { thinking = nil }
        if let run = current, run.sources?.contains(where: { removed.contains($0.area) }) == true {
            stopForRevocation("Antwort angehalten: Ein Bereich wurde abgeschaltet.")
        }
        inAreaChain { [weak self] in
            guard let self else { return }
            let effective = await self.settleAreas(pending, session: session, cloud: cloud, wanted: next)
            if self.session === session { self.areas = effective }
            self.refreshReleaseAlert()
            guard let target, session != nil, !AssistantConsent.isGranted(target, areas: effective) else { return }
            // Mehr Bereiche als zugestimmt: fragen, sonst zurück. Die Frage wartet auf den Nutzer und gehört deshalb NICHT in die
            // Kette (sie hielte spätere Änderungen auf, und die Zustimmung selbst wartet in der Sicherungsfolge des Kerns auf deren
            // Reihe: ein Kreis). Gesendet wird in der Zwischenzeit nichts (Guard beim Senden).
            self.askConsentOrRevert(target, wider: effective, previous: previous, session: session, cloud: cloud)
        }
    }

    /// Fragt nach der Zustimmung für mehr Bereiche; lehnt der Nutzer ab, gehen die Bereiche auf den früheren Stand zurück (eine
    /// weitere Änderung in der Kette, in der Reihenfolge des Anwendens).
    private func askConsentOrRevert(_ target: AssistantCloudTarget, wider: Set<AssistantArea>, previous: Set<AssistantArea>,
                                    session: AssistantSession?, cloud: Bool) {
        Task { [weak self] in
            guard let self else { return }
            if await self.askConsent(target, areas: wider, via: session) { return }
            let back = session?.applyAreas(previous, cloudConsent: AssistantConsent.isGranted(target, areas: previous))
            if self.session === session { self.areas = session?.effectiveAreas ?? previous }
            self.inAreaChain { [weak self] in
                guard let self else { return }
                let effective = await self.settleAreas(back, session: session, cloud: cloud, wanted: previous)
                if self.session === session { self.areas = effective }
                self.refreshReleaseAlert()
            }
        }
    }

    /// Wartet auf die Sicherung der Änderung und gibt die WIRKSAMEN Bereiche zurück. Ein Fehler wird gemeldet (Protokoll:
    /// „Protokoll nicht schreibbar“; Zustandsdatei: „Freigaben nicht gesichert“); dann gilt, was einschränkt, und was erweitert, nicht.
    private func settleAreas(_ pending: AssistantReleasePending?, session: AssistantSession?, cloud: Bool,
                             wanted: Set<AssistantArea>) async -> Set<AssistantArea> {
        guard let session, let pending else {
            let change = await releaseStore.updateAreas(wanted, cloud: cloud, audit: audit)
            if change.auditError != nil { auditResult(false) }
            return change.effective
        }
        let result = await session.settle(pending)
        if result.error != nil { auditResult(false) }
        return session.effectiveAreas
    }

    /// Änderungen der Freigabe laufen nacheinander: Eine spätere beginnt erst, wenn die frühere gesichert ist und ihre Anzeige gesetzt hat
    /// (sonst überholte eine ältere Anzeige die neuere).
    private func inAreaChain(_ work: @escaping @MainActor () async -> Void) {
        let previous = areaChain
        areaChain = Task {
            await previous?.value
            await work()
        }
    }

    /// Sprechpause (Kurz, Normal, Lang): streckt oder kürzt alle Fristen, nach denen eine Äußerung als beendet gilt.
    func setPause(_ value: AssistantPauseSetting) {
        UserDefaults.standard.set(value.rawValue, forKey: Self.pauseKey)
        guard value != pause else { return }
        pause = value
        applyPolicyConfig()
    }

    /// Antwortstil. Wirkt ab der nächsten Anfrage.
    func setStyle(_ value: AssistantStyle) {
        UserDefaults.standard.set(value.rawValue, forKey: Self.styleKey)
        guard value != style else { return }
        style = value
        refreshSessionStyle()
    }

    func setBargeIn(_ on: Bool) {
        UserDefaults.standard.set(on, forKey: Self.bargeInKey)
        bargeIn = on
        rebuildPolicy()
        if on { releaseAutoMute() } else if phase == .thinking || phase == .speaking { autoMuted = true; updateMute() }
    }

    /// Entzug (Bereich, Cloud-Zustimmung) mitten in der Antwort: Sprache stoppt, das bisher Gesprochene steht als
    /// unterbrochen im Verlauf (und im Protokoll als widerrufen).
    private func stopForRevocation(_ text: String, retry: String? = nil) {
        guard current != nil else { return }
        cancelAnswer(merge: false, revoked: true)
        // Was entzogen ist, erscheint nicht mehr: auch nicht die Titel und der Rohtext der Denk-Anzeige.
        thinking = nil
        interruptedPending = false
        phase = running ? .listening : .idle
        if let retry { retryText = retry }
        notice = AssistantNotice(text: text, action: retry == nil ? nil : AssistantNotice.Action.retrySend)
        announce(text)
    }

    /// Text bei geänderter Freigabe (Generation veraltet oder Lauf im Kern widerrufen); „Erneut senden“ stellt die Frage neu.
    private static let revokedText = "Antwort angehalten: Die Freigabe hat sich geändert."

    /// Cloud-Zustimmung entzogen (z. B. die Bereiche reichen nicht mehr): sofort im Kern (Generation steigt, laufende Antworten
    /// enden), danach in der Kette: Zustandsdatei ersetzen, dann die Zeile `release`. Ohne Sitzung (Modellstart, noch keine Freigabe)
    /// gibt es keine Erhöhung: dann steht das Ereignis hier (Generation 0).
    private func revokeConsent(areas consented: Set<AssistantArea>) {
        #if DEBUG
        guard !Self.demoQuote else { return }
        #endif
        thinking = nil
        guard let session else {
            Task { [weak self] in _ = await self?.recordConsentEntry(.cloudRevoked, areas: consented) }
            return
        }
        let pending = session.applyCloudConsent(false)
        inAreaChain { [weak self] in
            let result = await session.settle(pending)
            if result.error != nil { self?.auditResult(false) }
            self?.refreshReleaseAlert()
        }
    }

    /// Cloud-Zustimmung erteilt: über die Sitzung der Anfrage (Kern). `true`, wenn die Zustimmung gilt, also die Zustandsdatei sie
    /// bestätigt hat. Misslingt das Sichern, gilt sie nicht (Widerrufbarkeit hat Vorrang vor Bequemlichkeit).
    private func grantConsentInCore(_ request: AssistantConsentRequest) async -> Bool {
        #if DEBUG
        guard !Self.demoQuote else { return true }
        #endif
        guard let session = request.session else { return false }
        let result = await session.setCloudConsent(true)
        if result.error != nil { auditResult(false) }
        return result.effective && result.storeError == nil
    }

    private func recordConsentEntry(_ kind: AssistantAuditEntry.Kind, areas consented: Set<AssistantArea>) async -> Bool {
        let entry = AssistantAuditEntry(kind: kind, model: engine.label, cloud: true, consentVersion: 0,
                                        areas: AssistantArea.allCases.filter(consented.contains), generation: 0)
        let ok = await audit.record(entry)
        auditResult(ok)
        return ok
    }

    /// Das Protokoll ließ sich nicht schreiben: ein Hinweis, nie still. Einmal je Störung (`retry` setzt ihn zurück).
    private func auditResult(_ ok: Bool) {
        guard !ok, !auditFailureShown else { return }
        auditFailureShown = true
        notice = Self.auditUnavailableNotice()
    }

    /// Zustand „Freigaben prüfen“: Die Zustandsdatei fehlt (bei vorhandenem Protokoll) oder ist ungültig. Es gibt keinen Bereich und
    /// keine Cloud-Zustimmung, bis der Nutzer die Bereiche neu setzt und das gesichert ist. Der Knopf öffnet die Bereiche direkt.
    private static let releaseLockedNotice = AssistantNotice(
        text: "Freigaben prüfen",
        detail: "Die gespeicherten Freigaben sind nicht lesbar. Bis du die Bereiche neu setzt, gibt der Assistent nichts aus deinen Inhalten aus.",
        action: .openAreas)

    /// Eine Einschränkung ließ sich nicht dauerhaft sichern: Sie gilt, Erweiterungen nicht, und aus eigenen Inhalten kommt in diesem
    /// Lauf nichts mehr. „Erneut sichern“ versucht es noch einmal.
    private static let releaseUnsavedNotice = AssistantNotice(
        text: "Freigaben nicht gesichert",
        detail: "Einschränkungen gelten, Erweiterungen nicht. Aus deinen Inhalten kommt in diesem Lauf nichts mehr.",
        action: .retryRelease)

    /// Eine Einschränkung wurde nachträglich gesichert; die Sperre aus eigenen Inhalten bleibt bis zum nächsten Start der App.
    private static let releaseBlockedNotice = AssistantNotice(
        text: "Freigaben gesichert",
        detail: "Der Assistent gibt in diesem Lauf nichts mehr aus deinen Inhalten aus. Starte die App neu, um wieder auf sie zuzugreifen.",
        action: nil)

    /// Setzt den bestehenden Hinweis zu den Freigaben nach dem Stand von Sitzung und Zustandsdatei.
    private func refreshReleaseAlert() {
        #if DEBUG
        guard !Self.demoQuote else { return }
        #endif
        let store = releaseStore
        let locked = session?.isReleaseLocked ?? store.isLocked
        let blocked = session?.isContentBlocked ?? store.isBlocked
        let unsaved = session?.needsSave ?? store.isDirty
        let next: AssistantNotice?
        if locked {
            next = Self.releaseLockedNotice
        } else if unsaved {
            next = Self.releaseUnsavedNotice
        } else if blocked {
            next = Self.releaseBlockedNotice
        } else {
            next = nil
        }
        if releaseAlert != next { releaseAlert = next }
    }

    /// „Erneut sichern“: die Zustandsdatei noch einmal ersetzen (streng nacheinander mit den anderen Änderungen der Freigabe). Mit
    /// Sitzung über die Sitzung, ohne sie über den Speicher (er sichert den gemerkten engeren Stand).
    func retryRelease() {
        let session = self.session
        inAreaChain { [weak self] in
            guard let self else { return }
            if let session { _ = await session.retrySave() } else { _ = await self.releaseStore.retrySave() }
            self.refreshReleaseAlert()
        }
    }

    /// Zustand „Protokoll nicht schreibbar“: Ohne Protokoll gibt der Assistent nichts aus deinen Inhalten aus.
    private static func auditUnavailableNotice() -> AssistantNotice {
        AssistantNotice(text: "Protokoll nicht schreibbar",
                        detail: "Ohne Protokoll gibt der Assistent nichts aus deinen Inhalten aus. Prüfe den freien Speicher.",
                        action: .retryAudit)
    }

    func toggleMute() {
        userMuted.toggle()
        muted = userMuted
        if userMuted {
            discardSpeculation()
            clearLive()
            mergePrefix = ""
        }
        updateMute()
        announce(userMuted ? "Stumm. Ich höre nicht zu." : "Mikrofon an. Ich höre zu.")
    }

    private func updateMute() { listener.setMuted(userMuted || autoMuted) }

    /// Halbduplex: nach der Antwort kurz warten, bis der Nachhall verklungen ist, dann wieder zuhören.
    private func releaseAutoMute() {
        guard autoMuted else { return }
        let mine = generation
        Task {
            try? await Task.sleep(for: .milliseconds(400))
            guard mine == generation, autoMuted, current == nil else { return }
            autoMuted = false
            updateMute()
        }
    }

    /// Neues Gespräch: Verlauf löschen (Sitzung und Datei). In fester Reihenfolge in der Kette: laufende Antwort abbrechen,
    /// Sitzung leeren, Anzeige leeren, Datei über `saveChain` löschen. Frühere Aufträge der Kette (Abbruch, Abschluss) sehen
    /// `clearRequested != clearDone` und bringen den alten Verlauf nicht zurück.
    func newConversation() {
        clearRequested += 1
        if current != nil { cancelAnswer(merge: true) }
        mergePrefix = ""
        discardSpeculation()
        listener.consumeUtterance()
        clearLive()
        currentUser = nil
        currentRunID = nil
        liveParts = []
        notice = nil
        retryText = nil
        interruptedPending = false
        thinking = nil
        clearHold()
        if policyStale { rebuildPolicy() } else { policy.reset() }
        turns = []
        enqueue { [weak self] in
            // Die Sitzung erst jetzt nachschlagen: ein Modellwechsel dazwischen hat sie vielleicht ersetzt.
            await self?.session?.clearHistory()
            guard let self else { return }
            self.clearDone += 1
            self.turns = []
            self.deleteStoredHistory()
        }
        announce("Neues Gespräch.")
    }

    // MARK: Hören (Listener → Policy)

    private func speechStarted() {
        guard running else { return }
        if phase == .listening { phase = .hearing }
        perform(policy.feed(.speechStarted(atMs: nowMs)))
    }

    /// `lag`: Rückstand des Ergebnisendes hinter dem eingespeisten Audio in Sekunden. Daraus die Audiozeit des erkannten Bereichs auf der
    /// Uhr der Richtlinie (jetzt minus Rückstand): für den Echo-Abgleich. Die Umrechnung ist näherungsweise (das Audio im Tap liegt
    /// selbst etwas hinter der Uhr, grob 100 ms); ohne Zeitbereich der Erkennung bleibt `audioMs` leer, und die Richtlinie rechnet mit
    /// der Ankunftszeit.
    private func partial(_ text: String, lag: Double?) {
        guard running else { return }
        let parts = listener.liveParts
        liveStable = parts.stable
        liveVolatile = parts.volatile
        if phase == .listening, !text.isEmpty { phase = .hearing }
        let now = nowMs
        let audioMs: Int? = lag.flatMap { $0.isFinite && $0 >= 0 && $0 < 3_600 ? max(0, now - Int(($0 * 1_000).rounded())) : nil }
        perform(policy.feed(.partial(text: text, atMs: now, audioMs: audioMs)))
    }

    /// Stand der Erkennung (Listener, immer VOR dem zugehörigen `onSilence`): `caughtUp` = der letzte laute Puffer ist im Text angekommen;
    /// `following` = der bestätigte Stand liegt höchstens 1,5 s hinter dem Mikrofon (nur bei nachweislich hängender Anforderung falsch).
    private func recognition(_ caughtUp: Bool, following: Bool) {
        guard running else { return }
        perform(policy.feed(.recognition(caughtUp: caughtUp, following: following, atMs: nowMs)))
    }

    private func silence(_ ms: Int) {
        guard running else { return }
        let now = nowMs
        lastSilenceMs = ms
        lastSilenceAtMs = now
        perform(policy.feed(.silence(sinceMs: ms, atMs: now)))
    }

    private func overflow(_ count: Int) {
        overflowCount = count
        guard !overflowShown else { return }
        overflowShown = true
        notice = AssistantNotice(text: "Die Spracherkennung kommt nicht hinterher; es können Wörter fehlen.", action: nil)
    }

    private func listenerInterrupted(_ why: AssistantListener.Interruption) {
        guard running else { return }
        suspend(why.message, action: .resume)
    }

    private func clearLive() {
        liveStable = ""
        liveVolatile = ""
    }

    // MARK: Aktionen der Policy

    private func perform(_ actions: [DuplexTurnPolicy.Action]) {
        for action in actions {
            switch action {
            case .speculate(let text): speculate(text)
            case .discardSpeculation: discardSpeculation()
            case .commit(let text, _, let afterInterruption): commit(text: text, afterInterruption: afterInterruption)
            case .cancelAnswer(let merge): cancelAnswer(merge: merge)
            case .stopCommand: stopCommand()
            case .ignored: ignore()
            case .holdAnswer: holdAnswer()
            case .releaseAnswer: releaseAnswer()
            case .holdStalled(let reason): holdStalled(reason)
            }
        }
    }

    // MARK: Zurückhalten der Antwort (Sprecherwechsel)

    /// `holdAnswer`: Der Nutzer spricht, keine Einheit klingt. Nichts Neues beginnt, und was die Stimme schon bekam, aber noch nicht
    /// klingt, wird zurückgenommen (der Sprecher stellt es vorn in die Warteschlange zurück). Das Modell läuft weiter; Anzeige und
    /// Übergabe an die Stimme laufen unverändert (die Stimme hält nur das Starten).
    private func holdAnswer() {
        guard current != nil else { return }
        holdActive = true
        if holdNotice != nil { holdNotice = nil }
        speaker.hold()
    }

    /// `releaseAnswer`: Das Halten ist vorbei (Stille, Handfreigabe oder eine Einheit klang schon). Die Antwort darf sprechen; der
    /// Erkennertext der Zwischenzeit wird verbraucht (er war Geräusch, kein Wort).
    private func releaseAnswer() {
        clearHold()
        speaker.release()
        listener.consumeUtterance()
        clearLive()
    }

    /// `holdStalled`: Das Halten dauert, ohne dass ein Wort oder eine Freigabe kommt. Der Hinweis wird veröffentlicht (die Ansicht
    /// zeigt ihn mit den Knöpfen, die `offersRelease`/`offersDiscard` nennen); gehalten wird weiter. Ein anderer Grund überschreibt den
    /// Hinweis (die Richtlinie meldet je Grund einmal je Halten).
    private func holdStalled(_ reason: DuplexTurnPolicy.StallReason) {
        guard current != nil, holdActive else { return }
        let next: AssistantHoldNotice
        switch reason {
        case .activity: next = .activity
        case .recognition: next = .recognition
        }
        guard holdNotice != next else { return }
        holdNotice = next
        announce(next.text)
    }

    /// Das Halten ist zu Ende (oder die Antwort selbst): nur der Zustand der Steuerung und der Hinweis. Den Sprecher lösen `release()`
    /// (Freigabe) und `stop()` (Abbruch) selbst; ein neues `begin()` löscht das Halten ebenfalls.
    private func clearHold() {
        holdActive = false
        if holdNotice != nil { holdNotice = nil }
    }

    /// „Antwort jetzt hören“ (Handfreigabe bei `holdNotice == .activity`). Die Richtlinie nimmt sie nur an, solange gehalten wird und die
    /// Erkennung folgt; sonst geschieht nichts (kein Erzwingen am Richtlinienzustand vorbei: Sprecher und Richtlinie blieben sonst
    /// auseinander).
    func releaseHold() {
        guard current != nil, holdActive else { return }
        perform(policy.feed(.manualRelease(atMs: nowMs)))
    }

    /// „Verwerfen“ (immer anbietbar, solange `holdNotice` gesetzt ist): beendet die Antwort wie ein Stoppbefehl; die Richtlinie
    /// antwortet mit `cancelAnswer(merge: false)` und `stopCommand`. Antwortet sie nicht (die Antwort war für sie schon beendet),
    /// hält `stopAnswer()` die Antwort trotzdem an: Der Knopf wirkt immer.
    func discardHeldAnswer() {
        // Nur solange gehalten wird: Ein Tippen, das erst nach der Freigabe ankommt, soll keine schon klingende Antwort beenden.
        guard current != nil, holdActive else { return }
        let actions = policy.feed(.manualDiscard(atMs: nowMs))
        guard !actions.isEmpty else {
            stopAnswer()
            return
        }
        perform(actions)
    }

    private func speculate(_ text: String) {
        guard session != nil else { return }
        discardSpeculation()
        speculation = makeRun(text: text, afterInterruption: interruptedPending)
    }

    private func discardSpeculation() {
        guard let run = speculation else { return }
        speculation = nil
        abandon(run)
    }

    private func commit(text: String, afterInterruption: Bool) {
        listener.consumeUtterance()
        clearLive()
        mergePrefix = ""
        interruptedPending = false
        let run: AssistantRun
        if let spec = speculation, !spec.abandoned, AssistantWords.same(spec.text, text) {
            // Vorarbeit mit identischem Text: übernehmen. `afterInterruption` kannte sie nicht; der Hinweissatz fehlt dann im Prompt.
            run = spec
            speculation = nil
        } else {
            discardSpeculation()
            guard session != nil else {
                notice = noModelNotice()
                policy.reset()
                return
            }
            run = makeRun(text: text, afterInterruption: afterInterruption)
        }
        // Ende der Äußerung = Beginn der zuletzt gemeldeten Stille. Das `commit` kann der Meldung folgen (klingt bei der Unterbrechung noch
        // eine Einheit, kommt es erst mit `unitEnded`): `nowMs - lastSilenceMs` ginge dann zu spät an und verkürzte die Messung.
        run.endedAtMs = max(0, lastSilenceAtMs - lastSilenceMs)
        beginAnswer(run)
    }

    /// „Stopp“ (auch „Verwerfen“): Die Richtlinie liefert vorher `cancelAnswer(merge: false)`. Danach läuft nichts wieder an: Sprecher
    /// nicht mehr gehalten und leer (`stop()`), die Antwort abgeschlossen (verspätete Ergebnisse und Sätze werden nicht mehr
    /// ausgegeben), kein `releaseAnswer` mehr, keine neue Anfrage.
    private func stopCommand() {
        if current != nil { cancelAnswer(merge: false) }
        clearHold()
        listener.consumeUtterance()
        clearLive()
        interruptedPending = false
        phase = running ? .listening : .idle
    }

    /// Echo oder Rückmeldelaut: Die Richtlinie zieht die Wörter selbst über ihre Position im kumulativen Text ab (`consumedPrefixWords`).
    /// Der Erkenner wird deshalb NICHT verbraucht: `partial.text` bleibt bis zum Äußerungsende kumulativ (sonst würde ein Wort doppelt
    /// abgezogen). Verbraucht wird bei `commit`, `stopCommand` und `releaseAnswer`. Nur die Anzeige wird geleert: Das Echo erscheint
    /// nie als Text des Nutzers (jedes weitere Teilergebnis setzt die Anzeige neu, die Richtlinie meldet es erneut als Echo).
    private func ignore() {
        clearLive()
    }

    /// Abbrechen der laufenden Antwort. `merge`: die Äußerung wird fortgesetzt, nichts kommt in den Verlauf; sonst
    /// kommt das tatsächlich Gesprochene als „unterbrochen“ in den Verlauf.
    private func cancelAnswer(merge: Bool, revoked: Bool = false) {
        guard let run = current else { return }
        // `current` zuerst leeren: `speaker.stop()` meldet das Ende der klingenden Einheit SYNCHRON (`onUnitEnded`); die Steuerung
        // ignoriert es dann (`unitEnded()` prüft `current`), und die Richtlinie kann nicht mitten im Abbruch erneut abbrechen lassen.
        current = nil
        run.cancelled = true
        // `stop()` leert die Warteschlange, nimmt Übergebenes zurück und löscht das Halten des Sprechers; ab jetzt wird von dieser
        // Antwort nichts mehr ausgegeben (`deliverIfCurrent`: `run === current`; `deliver`: `run.completed`).
        speaker.stop()
        clearHold()
        if !run.completed { _ = policy.feed(.answerFinished(requestId: run.id)) }
        run.completed = true
        finishLatency(run)
        deactivateThinking()
        releaseAutoMute()
        let spoken = run.spokenUnits
        let session = self.session
        let task = run.task
        if merge {
            mergePrefix = run.text
            currentUser = nil
            liveParts = []
            watchMerge(run.text)
        } else {
            interruptedPending = true
        }
        phase = running ? .hearing : .idle
        enqueue { [weak self] in
            await session?.cancel(requestId: run.id)
            _ = await LiveCapture.bounded(1.5) { _ = await task?.value }
            task?.cancel()
            // Jede Anfrage bekommt ihren Protokolleintrag, auch bei „Ergänzen“ und ohne Gesprochenes. Einen Beitrag im Verlauf
            // gibt es nur für tatsächlich Gesprochenes (nie bei „Ergänzen“; die Ansicht zeigt sonst nichts Bleibendes).
            let ok = await session?.commit(requestId: run.id, user: run.text, spoken: merge ? .nothing : spoken, interrupted: true, revoked: revoked)
            self?.auditResult(ok ?? true)
            await self?.refreshTurns(after: run, keepLive: merge)
        }
        refreshSessionStyle()
    }

    /// Sicherheitsnetz für „Ergänzen“: Folgt auf die verworfene Äußerung binnen etwa 4 s nichts Neues (Räuspern, der
    /// Erkenner liefert nichts), wird die ursprüngliche Frage erneut gestellt, statt unbeantwortet zu bleiben. Wer
    /// weiterspricht, schiebt die Prüfung hinaus (höchstens fünfmal); ein neuer Beitrag leert `mergePrefix` und beendet sie.
    private func watchMerge(_ text: String) {
        mergeWatch?.cancel()
        let mine = generation
        mergeWatch = Task { [weak self] in
            for _ in 0..<5 {
                try? await Task.sleep(for: .seconds(4))
                guard !Task.isCancelled, let self, mine == self.generation, self.running, self.current == nil,
                      self.mergePrefix == text else { return }
                if self.liveStable.isEmpty, self.liveVolatile.isEmpty {
                    self.send(text: text)
                    return
                }
            }
        }
    }

    /// „Antwort anhalten“ (Taste, Fernbedienung): wie eine Unterbrechung ohne neue Äußerung.
    func stopAnswer() {
        guard current != nil else { return }
        cancelAnswer(merge: false)
        interruptedPending = false
        phase = running ? .listening : .idle
    }

    // MARK: Anfragen

    /// Getippte Äußerung senden (Tastatur, „Erneut senden“).
    func send(text: String) {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        guard session != nil else {
            notice = noModelNotice()
            return
        }
        let cutOff = current.map { !$0.finishedSpeaking.isEmpty } ?? false
        if current != nil { cancelAnswer(merge: false) }
        discardSpeculation()
        listener.consumeUtterance()
        clearLive()
        mergePrefix = ""
        policy.reset()
        notice = nil
        retryText = nil
        interruptedPending = false
        let run = makeRun(text: trimmed, afterInterruption: cutOff)
        run.endedAtMs = nowMs
        beginAnswer(run)
    }

    /// „Erneut senden“ im Hinweis nach einem Fehler.
    func retry() {
        guard let text = retryText else { return }
        auditFailureShown = false   // eine neue Störung wird wieder gemeldet
        send(text: text)
    }

    private func makeRun(text: String, afterInterruption: Bool) -> AssistantRun {
        let run = AssistantRun(text: text, afterInterruption: afterInterruption)
        guard let session else { run.abandoned = true; return run }
        let waiting = chain
        let request = AssistantRequest(requestId: run.id, text: text, afterInterruption: afterInterruption)
        run.task = Task { [weak self] in
            await waiting?.value
            guard !run.abandoned else { return }
            for await event in session.events(request) {
                guard let self else { return }
                self.receive(event, for: run)
            }
            self?.streamEnded(run)
        }
        return run
    }

    private func abandon(_ run: AssistantRun) {
        run.abandoned = true
        run.task?.cancel()
        let session = self.session
        let id = run.id
        let text = run.text
        Task { [weak self] in
            await session?.cancel(requestId: id)
            // Auch Vorarbeit, die verworfen wird, war eine Anfrage an das Modell: ein Protokolleintrag, nie ein Beitrag.
            // Hat der Kern die Anfrage nie gesehen (vor dem Start verworfen), gibt es keinen Eintrag.
            let ok = await session?.commit(requestId: id, user: text, spoken: .nothing, interrupted: true)
            self?.auditResult(ok ?? true)
        }
    }

    /// Die Äußerung gilt: ab jetzt wird die Antwort angezeigt und gesprochen.
    private func beginAnswer(_ run: AssistantRun) {
        if let target = engine.target, !AssistantConsent.isGranted(target, areas: areas) {
            // Guard vor dem Senden: ohne Zustimmung geht nichts an die Cloud. Nach der Zustimmung wird die Äußerung gesendet.
            abandon(run)
            policy.reset()
            phase = running ? .listening : .idle
            let text = run.text
            retryText = text
            let mine = generation
            Task {
                if await askConsent(target, areas: areas, via: session) {
                    send(text: text)
                } else if mine == generation, consentRequest == nil {
                    // Abgelehnt (nicht: Gespräch beendet oder Dialog ersetzt): die Äußerung bleibt in `retryText`.
                    notice = AssistantNotice(text: "Ohne Zustimmung wurde nichts gesendet.", action: .retrySend)
                }
            }
            return
        }
        run.committed = true
        // Stil `natural`: Ab jetzt zählt die Äußerung als Frage (Zeitgeber der Ansagen bei langsamer Antwort); bei Vorarbeit erst hier,
        // nicht schon beim Start der Vorarbeit. Ohne Wirkung im Stil `verbatim`. Aus jedem Thread, ohne Warten.
        session?.noteCommitted(requestId: run.id)
        beginLatency(run)
        current = run
        currentRunID = run.id
        currentUser = run.text
        liveParts = []
        firstSoundMs = nil
        phase = .thinking
        // Die Denk-Anzeige gehört zu dieser Anfrage; der Stand der vorigen wird ersetzt. Gepufferter Fortschritt der Vorarbeit folgt in `deliver`.
        thinking = AssistantThinking(active: true, steps: [], draft: "")
        _ = policy.feed(.answerStarted(requestId: run.id))
        if !bargeIn { autoMuted = true; updateMute() }
        deliver(run)
    }

    // MARK: Strom

    private func receive(_ event: AssistantEvent, for run: AssistantRun) {
        switch event {
        case .sources(_, let generation, let list):
            run.generation = run.generation ?? generation
            run.sources = list
        case .sentence(_, let generation, let text, let speech, let status, let covering, let quote):
            run.generation = run.generation ?? generation
            run.sentences.append(.init(text: text, speech: speech, status: status, sources: covering, quote: quote, generation: generation))
        case .progress(_, let generation, let progress):
            // Wie ein Satz gepuffert: Angezeigt wird erst in `deliver`, nur für den festgeschriebenen Lauf und nur bei gültiger Generation.
            run.generation = run.generation ?? generation
            run.bufferProgress(progress, generation: generation)
        case .done(_, let answer):
            run.answer = answer
            run.finished = true
        case .failed(_, let failure):
            run.failure = failure
            run.finished = true
        }
        deliverIfCurrent(run)
    }

    private func streamEnded(_ run: AssistantRun) {
        guard !run.finished else { return }
        run.finished = true
        if run.failure == nil, run.answer == nil { run.failure = run.abandoned || run.cancelled ? .cancelled : .modelFailed }
        deliverIfCurrent(run)
    }

    private func deliverIfCurrent(_ run: AssistantRun) {
        guard run.committed, run === current else { return }
        // Die Zustimmung für das Cloud-Ziel ist unterwegs entfallen: nichts Weiteres von dieser Antwort wird gesprochen.
        if let target = engine.target, !AssistantConsent.isGranted(target, areas: areas) {
            // Die Zustimmung ist ein Teil der Freigabe: neue Generation (laufende Antworten enden im Kern), im Protokoll vermerkt.
            revokeConsent(areas: areas)
            stopForRevocation("Antwort angehalten: Die Zustimmung für das Cloud-Modell fehlt.")
            return
        }
        deliver(run)
    }

    /// Ergebnis eines geordneten Ausgabe-Commits.
    private enum Commit {
        /// Die Generation galt, der Body lief.
        case done
        /// Die Generation war veraltet (oder keine Sitzung): der Body lief nicht.
        case revoked
        /// Das Protokoll dieser Anfrage hat einen Schreibfehler (der Body lief nicht oder lief noch): nichts Weiteres wird
        /// ausgegeben. Ein Schreibfehler geht dem Widerruf voran.
        case auditFailed
    }

    /// Geordneter Ausgabe-Commit (Anzeige, Übergabe an die Stimme): `body` läuft im Kern IM selben Lock, in dem die
    /// Freigabe-Generation steigt (`AssistantSession.commitOutput`), und nur, wenn die Generation des Satzes noch die
    /// aktuelle ist; nach der Rückkehr einer Erhöhung läuft kein `body` einer älteren Generation mehr. Das Ergebnis geht mit
    /// der geprüften Generation in die Satzspur des Protokolls. `reportPass: false`: nur eine Ablehnung melden.
    /// `body`: kurz und synchron, ohne I/O, und er ruft NIE in die Sitzung zurück (`releaseGeneration`, `setAllowUnsourced`,
    /// `releaseDidChange`, `commitOutput` hängen dort, `NSLock` ist nicht rekursiv); nur Zustand der Anzeige und Einreihen in
    /// die Stimme. Folgearbeit (Richtlinie, Meldungen) gehört nach den Aufruf. Der Audiostart ist KEIN solcher Commit mehr: er
    /// läuft als `start` des Satzes in der Stimme (`audioStart`), im selben Lock wie der tatsächliche Start.
    private func commitOutput(_ run: AssistantRun, _ index: Int, _ stage: AssistantCommitStage, reportPass: Bool = true,
                              _ body: () -> Void) -> Commit {
        guard let session else { return .revoked }
        let result = session.commitOutput(requestId: run.id, index: index, generation: run.sentences[index].generation, stage: stage,
                                          reportPass: reportPass, body)
        // Ein Schreibfehler des Protokolls geht dem Widerruf voran (der Kern lässt in dem Fall nichts ausführen: `ok` ist dann falsch).
        if result.auditFailed { return .auditFailed }
        return result.ok ? .done : .revoked
    }

    /// Der Start eines Satzes in der Stimme: Prüfung der Generation, TATSÄCHLICHER Start und Ergebniszeile im selben Lock
    /// (`AssistantSession.commitStart`). Die Stimme ruft ihn mit dem Start als Body; er gibt zurück, ob gestartet wurde. Eine
    /// veraltete Generation startet nie (Marke „verworfen“); `run.startMs` ist nur noch Messung (`sentenceStarted`).
    private func audioStart(_ run: AssistantRun, index: Int, generation: Int) -> PhononStart? {
        guard let session else { return nil }
        let id = run.id
        return { body in
            session.commitStart(requestId: id, index: index, generation: generation) { body(); return true }.ok
        }
    }

    /// Der Start ließ sich nicht auslösen, obwohl nichts widerrufen war. Ein Schreibfehler des Protokolls dieser Anfrage geht
    /// allem anderen vor (der Kern hat den Start verweigert und die Marke „fehlgeschlagen“ schon geschrieben): Die Ausgabe stoppt
    /// SOFORT, Wartende werden verworfen, die Anfrage endet mit „Fehler“. Sonst (Audio steht, Strom abgebrochen): Marke
    /// „fehlgeschlagen“, nie „ausgeführt“.
    private func audioStartFailed(_ run: AssistantRun, index: Int, generation: Int, session: AssistantSession?) {
        if session?.hasAuditFailure(requestId: run.id) == true {
            stopForAuditFailure(run)
            return
        }
        session?.noteCommit(requestId: run.id, index: index, stage: .audio, ok: false, generation: generation, failed: true)
    }

    /// Ein Schreibfehler einer Protokollzeile ist gemeldet (Beobachter am Protokoll): Gilt er der laufenden Antwort, stoppt die
    /// Ausgabe jetzt, auch die gerade klingende Einheit und alle wartenden.
    private func auditWriteFailed(key: String) {
        guard let run = current, run.id == key, !run.completed else { return }
        stopForAuditFailure(run)
    }

    /// Das Protokoll ist nicht schreibbar (Ergebnis- oder Pflichtzeile): nichts wird mehr angezeigt oder gesprochen; der Zustand
    /// „Protokoll nicht schreibbar“ mit „Erneut versuchen“.
    private func stopForAuditFailure(_ run: AssistantRun) {
        guard let active = current, active === run else { return }
        cancelAnswer(merge: false)
        // Ohne Protokoll gibt der Assistent nichts aus eigenen Inhalten aus: Die Denk-Anzeige verschwindet ebenfalls.
        thinking = nil
        interruptedPending = false
        phase = running ? .listening : .idle
        retryText = run.text
        auditFailureShown = true
        let shown = Self.auditUnavailableNotice()
        notice = shown
        announce(shown.text)
    }

    /// Neue Quellen, Sätze und das Ende an Anzeige und Stimme weitergeben. Leere Sprechtexte (reine Adressen) werden
    /// angezeigt, aber nicht gesprochen. Vor jedem Commit gilt die Generation: ein veraltetes Ereignis wird weder angezeigt
    /// noch gesprochen, die Antwort hält an.
    private func deliver(_ run: AssistantRun) {
        guard !run.completed else { return }
        // Schreibfehler des Protokolls vor allem anderen (auch vor dem Widerruf): nichts Weiteres wird angezeigt oder gesprochen.
        if session?.hasAuditFailure(requestId: run.id) == true {
            stopForAuditFailure(run)
            return
        }
        // Widerruf im Kern: Der Lauf ist beendet; Wartendes und der laufende Satz verstummen sofort, nichts Weiteres erscheint.
        if run.failure == .revoked {
            stopForRevocation(Self.revokedText, retry: run.text)
            return
        }
        if !run.begun {
            run.begun = true
            speaker.conversationMode = running
            speaker.voiceID = VoiceModel.shared.selectedVoiceID
            speaker.begin(language: .de)
            // `begin()` löscht das Halten des Sprechers; besteht es in der Steuerung, wird es hier erneuert.
            if holdActive { speaker.hold() }
        }
        while run.delivered < run.sentences.count {
            let index = run.delivered
            let sentence = run.sentences[index]
            showProgress(of: run, before: index)
            if index == 0 { run.latency.markUnit(Self.latencyClass(of: sentence.status)) }
            // Ansagen („Wörtlicher Auszug aus …“) werden nur gesprochen, alles andere auch angezeigt.
            if sentence.status != .announcement {
                switch commitOutput(run, index, .display, { showLive(sentence) }) {
                case .done:
                    run.latency.markVisible()
                    // Ein angezeigter Antwortsatz beendet das Denken, außer es läuft eine Suche und `composing` fehlt noch (Zwischensatz).
                    if !run.thinkingWaitsForFinal { deactivateThinking() }
                case .revoked:
                    stopForRevocation(Self.revokedText, retry: run.text)
                    return
                case .auditFailed:
                    stopForAuditFailure(run)
                    return
                }
            }
            if !sentence.speech.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                // Übergabe an die Stimme im Commit (nur eine Ablehnung wird gemeldet). Der Audiostart selbst ist der `start` des
                // Satzes: Er läuft in der Stimme, im Lock der Freigabe, zusammen mit dem tatsächlichen Start und der Marke.
                let start = audioStart(run, index: index, generation: sentence.generation)
                let session = self.session
                let failed: @MainActor () -> Void = { [weak self] in
                    self?.audioStartFailed(run, index: index, generation: sentence.generation, session: session)
                }
                let handedOver = commitOutput(run, index, .audio, reportPass: false) {
                    run.enqueued.append(index)
                    speaker.enqueue(sentence.speech, generation: sentence.generation, start: start, failed: failed)
                }
                switch handedOver {
                case .done: break
                case .revoked:
                    stopForRevocation(Self.revokedText, retry: run.text)
                    return
                case .auditFailed:
                    stopForAuditFailure(run)
                    return
                }
            }
            run.delivered += 1
        }
        showProgress(of: run, before: Int.max)
        guard run.finished, !run.inputEnded else { return }
        run.inputEnded = true
        // Alles gesprochen (oder nichts zu sprechen): `onIdle` kommt, auch sofort.
        speaker.endOfInput()
    }

    /// Einheit in die Live-Anzeige (Marke je Einheit): Ein wörtlicher Auszug bekommt einen eigenen Abschnitt mit seiner Quellzeile
    /// (Anführungszeichen setzt die Ansicht), ein Satz ohne Quelle (feste Antwort, Bestandsangabe) seine Kennzeichnung. Die
    /// Zuordnung steht im Kern (`AssistantTurnPart.append`), dieselbe wie beim Beitrag im Verlauf. Verworfene Sätze und Ansagen
    /// kommen nie hierher.
    private func showLive(_ sentence: AssistantRun.Sentence) {
        AssistantTurnPart.append(to: &liveParts, text: sentence.text, status: sentence.status, quote: sentence.quote, sources: sentence.sources)
    }

    // MARK: Denk-Anzeige

    /// Wendet den gepufferten Fortschritt an, der vor dem Satz mit der Nummer `sentenceIndex` kam (`Int.max`: allen Rest). Nur für den
    /// festgeschriebenen, laufenden Lauf (`deliver`). Ein Ereignis einer verworfenen Generation zeigt nichts: Vorprüfung an der
    /// Wiedergabesperre der Sitzung (wie vor dem Start eines Satzes; der Widerruf selbst läuft über `releaseBumped` und den nächsten
    /// Satz). Ohne Sitzung zeigt nichts.
    private func showProgress(of run: AssistantRun, before sentenceIndex: Int) {
        while run.progressShown < run.progressEvents.count, run.progressEvents[run.progressShown].before <= sentenceIndex {
            let entry = run.progressEvents[run.progressShown]
            run.progressShown += 1
            guard session?.playbackGate.permits(entry.generation) == true else { continue }
            applyProgress(entry.progress, for: run)
        }
        // Alles angewendet: Puffer leeren (Nummern gelten relativ zum Puffer, nicht zur Zählung des Laufs).
        if run.progressShown == run.progressEvents.count, run.progressShown > 0 {
            run.progressEvents.removeAll(keepingCapacity: true)
            run.progressShown = 0
        }
    }

    /// Fortschritt in die Denk-Anzeige: `.speaker` ist kein Schritt, `.draft` setzt den Rohtext, die anderen hängen einen Schritt an.
    /// `.searching` öffnet die Anzeige wieder, falls ein Zwischensatz des Sprechers sie schon beendet hatte.
    private func applyProgress(_ progress: AssistantProgress, for run: AssistantRun) {
        switch progress {
        case .speaker:
            return
        case .searching:
            run.thinkingWaitsForFinal = true
        case .found(let titles):
            if titles.isEmpty { run.thinkingWaitsForFinal = false }
        case .composing:
            run.thinkingWaitsForFinal = false
        case .draft(let text):
            guard var state = thinking, state.draft != text else { return }
            state.draft = text
            thinking = state
            return
        }
        guard let step = AssistantThinking.step(for: progress) else { return }
        var state = thinking ?? AssistantThinking(active: true, steps: [], draft: "")
        state.steps.append(step)
        if case .searching = progress { state.active = true }
        thinking = state
    }

    /// Das Denken ist zu Ende (erster Antwortsatz angezeigt, Ende, Abbruch, Stopp, Fehler): Der letzte Stand bleibt inaktiv stehen;
    /// gab es keinen Schritt und keinen Rohtext, gibt es nichts zu zeigen (`nil`).
    private func deactivateThinking() {
        guard let state = thinking else { return }
        if state.steps.isEmpty, state.draft.isEmpty {
            thinking = nil
            return
        }
        guard state.active else { return }
        thinking = AssistantThinking(active: false, steps: state.steps, draft: state.draft)
    }

    // MARK: Stimme (Speaker → Policy, Abschluss)

    private func audible() {
        guard let run = current else { return }
        run.latency.markAudio()
        phase = .speaking
        if firstSoundMs == nil { firstSoundMs = max(0, nowMs - run.endedAtMs) }
    }

    /// Die Stimme meldet den Beginn eines Satzes NACH dem tatsächlichen Start. Das ist nur noch Messung (Zeit für den Echo-Wächter
    /// und die Richtlinie): Die Marke „ausgeführt“ hat der Kern schon im Lock des Starts geschrieben (`audioStart`), nie vorher.
    private func sentenceStarted(_ text: String) {
        guard let run = current, run.startMs.count < run.enqueued.count else { return }
        let at = nowMs
        run.startMs.append(at)
        perform(policy.feed(.sentenceSpoken(text: text, startedAtMs: at, endedAtMs: nil)))
    }

    /// Die Stimme meldet nur, was wirklich erklang; ein Satz bei hängendem Audio oder Abbruch fehlt. Das Ende gehört
    /// deshalb immer zum zuletzt begonnenen Satz (die Stimme spricht nacheinander), nicht zum nächsten in der Zählung.
    private func sentenceFinished(_ text: String) {
        guard let run = current, let last = run.startMs.indices.last else { return }
        let index = run.enqueued[last]
        guard run.finishedSpeaking.last != index else { return }
        run.finishedSpeaking.append(index)
        // Eine Ansage („Wörtlicher Auszug aus …“) ist kein fertiger Antwortsatz: sonst gälte ein Dazwischenreden schon als Unterbrechung.
        guard run.sentences[index].status != .announcement else { return }
        perform(policy.feed(.sentenceSpoken(text: text, startedAtMs: run.startMs[last], endedAtMs: nowMs)))
    }

    /// Eine Einheit der Antwort klingt jetzt (JEDE Einheit, auch Ansagen und Überbrückung; NACH dem tatsächlichen Start). Ohne laufende
    /// Antwort (abgebrochen, abgeschlossen) wird nichts gemeldet: Die Richtlinie hat die Antwort dann schon beendet.
    private func unitStarted() {
        guard current != nil else { return }
        perform(policy.feed(.unitStarted(atMs: nowMs)))
    }

    /// Die Einheit klingt nicht mehr (zu Ende gesprochen oder abgebrochen). Die Richtlinie kann hier nach einem Wort des Nutzers in der
    /// Lücke `cancelAnswer` liefern oder das Halten beginnen (`holdAnswer`, ab Klangende).
    private func unitEnded() {
        guard current != nil else { return }
        perform(policy.feed(.unitEnded(atMs: nowMs)))
    }

    private func speakerIdle() {
        guard let run = current, run.inputEnded else { return }
        completeAnswer(run)
    }

    /// Alles gesprochen: Beitrag in den Verlauf (bei Fehler nach gesprochenen Sätzen als „unterbrochen“), Fehler melden.
    private func completeAnswer(_ run: AssistantRun) {
        guard !run.completed else { return }
        run.completed = true
        current = nil
        // Regulär kann das Ende nur kommen, wenn der Sprecher nicht gehalten ist (`finishIfDrained`); sicherheitshalber lösen.
        if holdActive { speaker.release() }
        clearHold()
        _ = policy.feed(.answerFinished(requestId: run.id))
        finishLatency(run)
        deactivateThinking()
        if policyStale, policyIsIdle { rebuildPolicy() }
        releaseAutoMute()
        phase = running ? .listening : .idle
        let session = self.session
        let failure = run.failure
        // Regulär beendet: alle ausgegebenen Einheiten; sonst (Fehler, Abbruch) nur die, die ganz gesprochen wurden.
        let spoken: AssistantSpokenUnits = failure == nil ? .all : run.spokenUnits
        let keep = failure != nil && spoken == .nothing
        if let failure { showFailure(failure, run: run) }
        enqueue { [weak self] in
            // Für JEDE Anfrage entsteht ein Protokolleintrag, auch bei Fehler ohne Gesprochenes; einen Beitrag im Verlauf
            // schreibt der Kern nur für tatsächlich Gesprochenes und nur bei noch gültiger Generation.
            let written = await session?.commit(requestId: run.id, user: run.text, spoken: spoken, interrupted: failure != nil,
                                                revoked: failure == .revoked)
            self?.auditResult(written ?? true)
            await self?.refreshTurns(after: run, keepLive: keep)
        }
        refreshSessionStyle()
    }

    private func showFailure(_ failure: AssistantFailure, run: AssistantRun) {
        let text: String
        switch failure {
        case .cancelled: text = "Die Antwort wurde abgebrochen."
        case .timeout: text = "Das Modell hat zu lange nicht geantwortet."
        case .noModel: text = AssistantEngine.noModelText
        case .modelFailed: text = "Die Antwort ist fehlgeschlagen."
        case .offline: text = engine.isCloud || engine.choice == .endpoint ? "Das Modell ist nicht erreichbar. Prüfe die Verbindung." : "Das Modell ist nicht erreichbar."
        case .consentRequired: text = "Für dieses Modell fehlt die Zustimmung."
        case .revoked: text = Self.revokedText
        case .auditUnavailable:
            // Pflichtzeile oder Ergebniszeile nicht schreibbar: Ohne Protokoll gibt es keine Ausgabe aus eigenen Inhalten.
            retryText = run.text
            auditFailureShown = true
            let shown = Self.auditUnavailableNotice()
            notice = shown
            announce(shown.text)
            return
        }
        retryText = run.text
        notice = AssistantNotice(text: text, action: .retrySend)
        announce(text)
    }

    // MARK: Latenzmessung (lva#521)

    /// Die Messwerte als Texttabelle zum Kopieren (nur Zeiten, Klassen und Zähler, nie Inhalte), samt fsync-Dauer des Protokolls. Nicht
    /// veröffentlicht: Die Ansicht liest sie beim Anzeigen und nach `resetLatency()`.
    var latencyText: String { AssistantLatencyLog.shared.summaryText(durable: audit.durableStats) }

    /// Löscht alle Messwerte (Speicher und Datei).
    func resetLatency() { AssistantLatencyLog.shared.reset() }

    /// Die Messung beginnt beim Ende der Äußerung (`endedAtMs`, bei getipptem Text: jetzt), auf der monotonen Uhr der Messung
    /// (`systemUptime`, wie `origin`). Kalt: die erste Runde je Prozess. Lokal: Modell auf dem Gerät oder Endpunkt im privaten Netz
    /// (nicht Cloud). Beides und die Modellart stehen hier fest; ein Modellwechsel mitten in der Runde ändert die Messung nicht.
    private func beginLatency(_ run: AssistantRun) {
        run.latencyCold = Self.coldRoundPending
        Self.coldRoundPending = false
        run.latencyLocal = !engine.isCloud
        run.latencyModel = latencyModelKind
        run.latency.begin(at: origin + Double(run.endedAtMs) / 1_000)
    }

    /// Beendet die Messung der Runde und schreibt sie abseits des MainActors (kleine Datei, atomar; ein Schreibfehler bleibt folgenlos:
    /// Messwerte sind keine Pflichtdaten). Ohne sichtbares Zeichen und ohne Ton entsteht keine Messung.
    private func finishLatency(_ run: AssistantRun) {
        guard let sample = run.latency.finish(cold: run.latencyCold, local: run.latencyLocal, model: run.latencyModel) else { return }
        Task.detached(priority: .utility) { AssistantLatencyLog.shared.add(sample) }
    }

    private var latencyModelKind: AssistantModelKind {
        switch engine.choice {
        case .apple?: .apple
        case .gemma?: .gemma
        default: .other
        }
    }

    /// Klasse der ersten Einheit: Vorlage (fester Satz, Gruß, Uhrzeit) und freier Satz ohne Inhalte des Nutzers gelten als `template`;
    /// Auszug, Bestandsangabe, Satz ohne Quelle, Zusammenfassung und jede Ansage (davor folgt ein Abruf) als `content`.
    private static func latencyClass(of status: AssistantSentenceStatus) -> AssistantLatencyUnitClass {
        switch status {
        case .neutral, .conversational: .template
        case .grounded, .inventory, .unsourced, .summarized, .announcement: .content
        }
    }

    // MARK: Verlauf

    /// Nach dem Eintrag in den Verlauf: Anzeige auf den Stand der Sitzung setzen. Die Live-Anzeige der Anfrage bleibt
    /// stehen, wenn sie noch gebraucht wird (Fehler ohne gesprochene Sätze: „Erneut senden“).
    private func refreshTurns(after run: AssistantRun, keepLive: Bool) async {
        guard let session else { return }
        let history = await session.history
        // „Neues Gespräch“ steht in der Kette noch aus: der alte Verlauf käme sonst in Anzeige und Datei zurück.
        guard clearRequested == clearDone else { return }
        turns = history
        if currentRunID == run.id, !keepLive {
            currentUser = nil
            liveParts = []
            currentRunID = nil
        }
        persist(history)
    }

    /// Schreibt den Verlauf (atomar, im Kern). Fehler (Speicher voll) erscheinen als Hinweis, nie still.
    private func persist(_ history: [AssistantTurn]) {
        #if DEBUG
        guard !Self.demoQuote else { return }
        #endif
        let store = self.store
        let previous = saveChain
        saveChain = Task { [weak self] in
            await previous?.value
            let ok = await Task.detached(priority: .utility) { (try? store.save(history)) != nil }.value
            if !ok { self?.notice = AssistantNotice(text: "Verlauf konnte nicht gespeichert werden.", action: nil) }
        }
    }

    /// Löscht die Verlaufsdatei in der Reihenfolge des Speicherns: ein früher begonnenes Speichern legt sie danach nicht neu an.
    private func deleteStoredHistory() {
        #if DEBUG
        guard !Self.demoQuote else { return }
        #endif
        let store = self.store
        let previous = saveChain
        saveChain = Task { [weak self] in
            await previous?.value
            let deleted = await Task.detached(priority: .utility) { store.clear() }.value
            if !deleted { self?.notice = AssistantNotice(text: "Der gespeicherte Verlauf konnte nicht gelöscht werden.", action: nil) }
        }
    }

    private func enqueue(_ work: @escaping @MainActor () async -> Void) {
        let previous = chain
        chain = Task {
            await previous?.value
            await work()
        }
    }

    // MARK: Zustimmung

    private func askConsent(_ target: AssistantCloudTarget, areas: Set<AssistantArea>, via: AssistantSession?) async -> Bool {
        resolveConsent(false)
        return await withCheckedContinuation { (continuation: CheckedContinuation<Bool, Never>) in
            consentContinuation = continuation
            consentRequest = AssistantConsentRequest(target: target, areas: areas, session: via)
        }
    }

    /// Die Zustimmung ändert die Freigabe: Der Kern zuerst (Zeile „beabsichtigt“, Zustandsdatei ersetzen, erst nach bestätigtem Austausch
    /// wirksam mit neuer Generation, Zeile „wirksam“), danach die Freigabe im Speicher des Ziels. Gilt sie nicht (Zustandsdatei oder
    /// Protokoll nicht schreibbar), bleibt sie aus.
    func grantConsent() {
        guard let request = consentRequest, !grantingConsent else { return }
        grantingConsent = true
        Task { [weak self] in
            guard let self else { return }
            let effective = await self.grantConsentInCore(request)
            if effective { AssistantConsent.grant(request.target) }
            self.grantingConsent = false
            self.refreshReleaseAlert()
            self.finishConsent(effective)
        }
    }

    func declineConsent() { finishConsent(false) }

    private func resolveConsent(_ value: Bool) {
        if consentContinuation != nil { finishConsent(value) }
    }

    private func finishConsent(_ value: Bool) {
        consentRequest = nil
        let continuation = consentContinuation
        consentContinuation = nil
        continuation?.resume(returning: value)
    }

    // MARK: Ansagen

    private func announce(_ text: String) {
        guard !text.isEmpty else { return }
        AccessibilityNotification.Announcement(text).post()
    }
}
