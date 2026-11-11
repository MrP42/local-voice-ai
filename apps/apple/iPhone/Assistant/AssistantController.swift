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
//   danach gilt es als Unterbrechung (das Gesprochene kommt als „unterbrochen“ in den Verlauf).
// Eine Antwort steht erst im Verlauf, wenn sie zu Ende gesprochen ist (oder unterbrochen wurde).

enum AssistantPhase: Equatable { case idle, listening, hearing, thinking, speaking }

struct AssistantNotice: Equatable {
    enum Action: Equatable { case retrySend, resume, retryStart, retryAudit, chooseModel, openLocalModels }
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

    let id = UUID().uuidString
    /// Freigabe-Generation des Laufs (aus dem ersten Ereignis); unbekannt, solange noch kein Ereignis da ist.
    var generation: Int?
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

    init(text: String, afterInterruption: Bool) {
        self.text = text
        self.afterInterruption = afterInterruption
    }

    /// Nur die Einheiten, die ganz gesprochen wurden (Ansagen wie „Wörtlicher Auszug aus …“ gehören nicht zum Text).
    var spokenText: String {
        finishedSpeaking.map { sentences[$0] }.filter { $0.status != .announcement }.map(\.text).joined(separator: " ")
    }
}

@MainActor
final class AssistantController: ObservableObject {
    static let shared = AssistantController()

    static let bargeInKey = "assistant_barge_in"
    static var storedBargeIn: Bool { UserDefaults.standard.object(forKey: bargeInKey) as? Bool ?? true }

    // MARK: Veröffentlicht (Anzeige)

    @Published private(set) var phase: AssistantPhase = .idle
    @Published private(set) var turns: [AssistantTurn] = []
    /// Gesendete Äußerung, deren Antwort noch läuft oder fehlschlug (noch kein Beitrag im Verlauf).
    @Published private(set) var currentUser: String?
    /// Bei „Ergänzen“: die verworfene Äußerung, vor die der neue Text kommt.
    @Published private(set) var mergePrefix = ""
    @Published private(set) var liveStable = ""
    @Published private(set) var liveVolatile = ""
    @Published private(set) var liveAnswer = ""
    /// Die laufende Antwort in Abschnitten, für die Anzeige: Jeder Satz trägt seine Quellzeile (Zitat) oder seine Kennzeichnung
    /// (feste Antwort, Bestandsangabe); es gibt keine gesammelte Quellenliste mehr.
    @Published private(set) var liveParts: [AssistantTurnPart] = []
    @Published private(set) var notice: AssistantNotice?
    @Published private(set) var modelLabel = "Kein Modell"
    @Published private(set) var isCloud = false
    @Published private(set) var selectedChoice: AssistantModelChoice?
    @Published private(set) var preparing: AssistantSources.Progress?
    /// Messwert: erster Ton in ms ab Äußerungsende.
    @Published private(set) var firstSoundMs: Int?
    @Published private(set) var muted = false
    @Published private(set) var bargeIn = AssistantController.storedBargeIn
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
    private let audit = AssistantAuditLog(directory: AssistantAuditLog.standardDirectory())
    private var auditFailureShown = false
    /// Änderungen der Freigabe (Bereiche, Zustimmung) laufen nacheinander: Zeile, dann Einstellung.
    private var areaChain: Task<Void, Never>?
    private var grantingConsent = false
    private let hub = AssistantSources.shared
    private var engine = AssistantEngine.unavailable("")
    private var session: AssistantSession?
    private var knowledge: AssistantKnowledge?
    private var policy = DuplexTurnPolicy()
    private var current: AssistantRun?
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
    private var bag: Set<AnyCancellable> = []
    private var memoryObserver: NSObjectProtocol?
    private var overflowShown = false

    init() {
        hub.$progress.receive(on: RunLoop.main).sink { [weak self] value in
            MainActor.assumeIsolated { self?.preparing = value }
        }.store(in: &bag)

        listener.onSpeechStarted = { [weak self] in self?.speechStarted() }
        listener.onPartial = { [weak self] text in self?.partial(text) }
        listener.onSilence = { [weak self] ms in self?.silence(ms) }
        listener.onInterrupted = { [weak self] why in self?.listenerInterrupted(why) }
        listener.onLevel = { [weak self] level in self?.meter.push(level) }
        listener.onOverflow = { [weak self] count in self?.overflow(count) }

        speaker.onAudible = { [weak self] in self?.audible() }
        speaker.onSentenceStarted = { [weak self] text in self?.sentenceStarted(text) }
        speaker.onSentenceFinished = { [weak self] text in self?.sentenceFinished(text) }
        speaker.onIdle = { [weak self] in self?.speakerIdle() }
    }

    private var nowMs: Int { Int((ProcessInfo.processInfo.systemUptime - origin) * 1000) }

    #if DEBUG
    // MARK: Beispielrunde (nur DEBUG)

    /// UI-Tests: `--assistant-demo-quote` öffnet das Gespräch mit einer eingebauten Beispielrunde aus synthetischem Inhalt (ein
    /// wörtlicher Auszug, ein ganzer Block, mit Quellzeile). `--assistant-demo-multi` öffnet stattdessen die Beispielrunde mit
    /// mehreren Blöcken (Marke je Block: drei Auszüge aus zwei Quellen, einer davon mehrzeilig, je mit eigener Quellzeile „Wörtlicher
    /// Auszug aus … · S. 12“, dazu ein Satz mit Kennzeichnung). Kein Modell, kein Abruf, kein Speicher, kein Protokoll: `start`
    /// kehrt sofort zurück, und Verlauf und Protokoll bleiben unberührt (`persist`, `deleteStoredHistory`, `revokeConsent`).
    static let demoMulti = ProcessInfo.processInfo.arguments.contains("--assistant-demo-multi")
    static let demoQuote = demoMulti || ProcessInfo.processInfo.arguments.contains("--assistant-demo-quote")

    private func showDemo() {
        opened = true
        historyLoaded = true
        notice = nil
        turns = Self.demoMulti ? Self.demoMultiTurns : Self.demoTurns
        modelLabel = "Beispiel"
    }

    /// Ein gespeicherter Block der Beispielrunde (`isBlock: true`: ganz, unverändert, zitierbar).
    private static func demoBlock(_ area: AssistantArea, _ id: String, _ title: String, _ locator: String, _ text: String) -> AssistantPassage {
        AssistantPassage(area: area, itemId: id, title: title, locator: locator, text: text, score: 1, isBlock: true)
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
        hub.refresh()
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
            let allowed = await askConsent(target, areas: AssistantAreaSettings.areas(cloud: true))
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
        interruptedPending = false
        overflowShown = false
        overflowCount = 0
        policy.reset()
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
        if current != nil { cancelAnswer(merge: false) }
        discardSpeculation()
        speaker.stop()
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

    /// Modell, Wissen und Sitzung übernehmen; der Verlauf bleibt.
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
        knowledge = kn
        let stored = AssistantTurn.admitted(await session?.history ?? turns, released: AssistantUnsourcedConsent.isReleased)
        // Nach dem letzten Wartepunkt: steht „Neues Gespräch“ noch aus, kommt der alte Verlauf nicht in die neue Sitzung.
        let history = clearRequested == clearDone ? stored : []
        session?.setReleaseObserver(nil)
        if let model = built.model {
            // Zustimmung zum Cloud-Ziel aus dem gespeicherten Stand; für lokale Modelle ohne Belang. Der Kern lässt ein
            // Cloud-Modell ohne Zustimmung gar nicht erst laufen (`consentRequired`).
            let consented = built.target.map { AssistantConsent.isGranted($0, areas: active) } ?? false
            let next = AssistantSession(model: model, knowledge: kn, config: built.config, history: history, audit: audit,
                                        cloudConsent: consented)
            observeRelease(of: next)
            speaker.gate = next.playbackGate
            session = next
        } else {
            speaker.gate = nil
            session = nil
        }
        turns = history
        rebuildPolicy()
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
        guard let run = current, !run.completed, let started = run.generation, started < generation else { return }
        stopForRevocation(Self.revokedText, retry: run.text)
    }

    private func rebuildPolicy() {
        var config = DuplexTurnPolicy.Config()
        config.speculate = !engine.isCloud
        config.bargeIn = bargeIn
        policy = DuplexTurnPolicy(config: config)
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
            let allowed = await askConsent(target, areas: AssistantAreaSettings.areas(cloud: true))
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
        announce("Modell: \(built.label)")
    }

    func toggleArea(_ area: AssistantArea) {
        var next = areas
        if next.contains(area) { next.remove(area) } else { next.insert(area) }
        applyAreas(next)
    }

    /// Bereiche ändern, in fester Reihenfolge (Freigabe zuerst, Einstellung danach):
    /// 1. Kern, SYNCHRON: EINE Erhöhung der Generation. Was einschränkt (Bereich weg, Zustimmung weg), gilt sofort; die laufende
    ///    Antwort endet, die Zeile `release` ist eingereiht. Erweiterungen (Bereich dazu) gelten noch nicht.
    /// 2. In der Kette: `settle` wartet auf die dauerhafte Zeile; erst danach wird eine Erweiterung wirksam. Misslingt das Schreiben,
    ///    bleiben Einschränkungen in Kraft, Erweiterungen nicht, und der Fehler wird gemeldet.
    /// 3. Erst dann die gespeicherte Einstellung, und zwar der WIRKSAME Stand (nicht der gewünschte).
    /// Für ein Cloud-Ziel geht die Zustimmung für den neuen Stand mit in die Änderung ein: Reichen die Bereiche der Zustimmung
    /// nicht, gilt sie nicht mehr (sofort), bis der Nutzer zustimmt; gesendet wird in der Zwischenzeit nichts.
    private func applyAreas(_ next: Set<AssistantArea>) {
        let previous = areas
        let cloud = engine.isCloud
        let target = engine.target
        let knowledge = self.knowledge
        let session = self.session
        areas = next
        let pending = session?.applyAreas(next, cloudConsent: target.map { AssistantConsent.isGranted($0, areas: next) })
        // Ein Bereich, aus dem die laufende Antwort Auszüge hat, wird entzogen: wartende Sätze werden nicht mehr gesprochen.
        let removed = previous.subtracting(next)
        if let run = current, run.sources?.contains(where: { removed.contains($0.area) }) == true {
            stopForRevocation("Antwort angehalten: Ein Bereich wurde abgeschaltet.")
        }
        inAreaChain { [weak self] in
            guard let self else { return }
            var effective = await self.settleAreas(pending, session: session, knowledge: knowledge, wanted: next)
            AssistantAreaSettings.set(effective, cloud: cloud)
            if self.session === session { self.areas = effective }
            guard let target, !AssistantConsent.isGranted(target, areas: effective) else { return }
            // Mehr Bereiche als zugestimmt: fragen, sonst zurück. Gesendet wird in der Zwischenzeit nichts (Guard beim Senden).
            if await self.askConsent(target, areas: effective) { return }
            let back = session?.applyAreas(previous, cloudConsent: AssistantConsent.isGranted(target, areas: previous))
            effective = await self.settleAreas(back, session: session, knowledge: knowledge, wanted: previous)
            AssistantAreaSettings.set(effective, cloud: cloud)
            if self.session === session { self.areas = effective }
        }
    }

    /// Wartet auf die Zeile der Änderung und gibt die WIRKSAMEN Bereiche zurück. Ein Schreibfehler wird gemeldet; dann gilt, was
    /// einschränkt, und was erweitert, nicht.
    private func settleAreas(_ pending: AssistantReleasePending?, session: AssistantSession?, knowledge: AssistantKnowledge?,
                             wanted: Set<AssistantArea>) async -> Set<AssistantArea> {
        guard let session, let pending else {
            await knowledge?.setAreas(wanted)
            return wanted
        }
        let result = await session.settle(pending)
        if result.error != nil || !result.effective { auditResult(false, detail: Self.releaseNotSavedDetail) }
        return session.effectiveAreas
    }

    /// Änderungen der Freigabe laufen nacheinander: Eine spätere beginnt erst, wenn die frühere ihre Zeile und die Einstellung hinter
    /// sich hat (sonst überholte eine ältere Einstellung die neuere).
    private func inAreaChain(_ work: @escaping @MainActor () async -> Void) {
        let previous = areaChain
        areaChain = Task {
            await previous?.value
            await work()
        }
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
        interruptedPending = false
        phase = running ? .listening : .idle
        if let retry { retryText = retry }
        notice = AssistantNotice(text: text, action: retry == nil ? nil : AssistantNotice.Action.retrySend)
        announce(text)
    }

    /// Text bei geänderter Freigabe (Generation veraltet oder Lauf im Kern widerrufen); „Erneut senden“ stellt die Frage neu.
    private static let revokedText = "Antwort angehalten: Die Freigabe hat sich geändert."

    /// Cloud-Zustimmung entzogen (z. B. die Bereiche reichen nicht mehr): sofort im Kern (Generation steigt, laufende Antworten
    /// enden, die Zeile `release` ist eingereiht), das Schreiben der Zeile danach in der Kette. Ohne Sitzung (Modellstart, noch
    /// keine Freigabe) gibt es keine Erhöhung: dann steht das Ereignis hier (Generation 0).
    private func revokeConsent(areas consented: Set<AssistantArea>) {
        #if DEBUG
        guard !Self.demoQuote else { return }
        #endif
        guard let session else {
            Task { [weak self] in _ = await self?.recordConsentEntry(.cloudRevoked, areas: consented) }
            return
        }
        let pending = session.applyCloudConsent(false)
        inAreaChain { [weak self] in
            let result = await session.settle(pending)
            if result.error != nil { self?.auditResult(false, detail: Self.releaseNotSavedDetail) }
        }
    }

    /// Cloud-Zustimmung erteilt: erst im Kern (die Erteilung gilt erst mit der dauerhaften Zeile), danach die Einstellung.
    /// `true`, wenn die Zustimmung gilt. Misslingt das Schreiben, gilt sie nicht (Widerrufbarkeit hat Vorrang vor Bequemlichkeit).
    private func grantConsentInCore(areas consented: Set<AssistantArea>) async -> Bool {
        #if DEBUG
        guard !Self.demoQuote else { return true }
        #endif
        guard let session else { return await recordConsentEntry(.cloudGranted, areas: consented) }
        let result = await session.setCloudConsent(true)
        if result.error != nil { auditResult(false, detail: Self.releaseNotSavedDetail) }
        return result.effective
    }

    private func recordConsentEntry(_ kind: AssistantAuditEntry.Kind, areas consented: Set<AssistantArea>) async -> Bool {
        let entry = AssistantAuditEntry(kind: kind, model: engine.label, cloud: true, consentVersion: 0,
                                        areas: AssistantArea.allCases.filter(consented.contains), generation: 0)
        let ok = await audit.record(entry)
        auditResult(ok)
        return ok
    }

    /// Das Protokoll ließ sich nicht schreiben: ein Hinweis, nie still. Einmal je Störung (`retry` setzt ihn zurück).
    private func auditResult(_ ok: Bool, detail: String? = nil) {
        guard !ok, !auditFailureShown else { return }
        auditFailureShown = true
        notice = Self.auditUnavailableNotice(detail: detail)
    }

    private static let releaseNotSavedDetail = "Die Änderung der Freigabe wurde nicht gesichert: Einschränkungen gelten, Erweiterungen nicht."

    /// Zustand „Protokoll nicht schreibbar“: Ohne Protokoll gibt der Assistent nichts aus deinen Inhalten aus.
    private static func auditUnavailableNotice(detail: String? = nil) -> AssistantNotice {
        AssistantNotice(text: "Protokoll nicht schreibbar",
                        detail: detail ?? "Ohne Protokoll gibt der Assistent nichts aus deinen Inhalten aus. Prüfe den freien Speicher.",
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
        liveAnswer = ""
        liveParts = []
        notice = nil
        retryText = nil
        interruptedPending = false
        policy.reset()
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

    private func partial(_ text: String) {
        guard running else { return }
        let parts = listener.liveParts
        liveStable = parts.stable
        liveVolatile = parts.volatile
        if phase == .listening, !text.isEmpty { phase = .hearing }
        perform(policy.feed(.partial(text: text, atMs: nowMs)))
    }

    private func silence(_ ms: Int) {
        guard running else { return }
        lastSilenceMs = ms
        perform(policy.feed(.silence(sinceMs: ms, atMs: nowMs)))
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
            }
        }
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
        run.endedAtMs = max(0, nowMs - lastSilenceMs)
        beginAnswer(run)
    }

    private func stopCommand() {
        listener.consumeUtterance()
        clearLive()
        interruptedPending = false
        phase = running ? .listening : .idle
    }

    /// Echo oder Rückmeldelaut: Text verbrauchen, damit er nicht beim nächsten Teilergebnis wieder vorn steht.
    private func ignore() {
        listener.consumeUtterance()
        clearLive()
    }

    /// Abbrechen der laufenden Antwort. `merge`: die Äußerung wird fortgesetzt, nichts kommt in den Verlauf; sonst
    /// kommt das tatsächlich Gesprochene als „unterbrochen“ in den Verlauf.
    private func cancelAnswer(merge: Bool, revoked: Bool = false) {
        guard let run = current else { return }
        speaker.stop()
        current = nil
        run.cancelled = true
        if !run.completed { _ = policy.feed(.answerFinished(requestId: run.id)) }
        run.completed = true
        releaseAutoMute()
        let spoken = run.spokenText
        let session = self.session
        let task = run.task
        if merge {
            mergePrefix = run.text
            currentUser = nil
            liveAnswer = ""
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
            let ok = await session?.commit(requestId: run.id, user: run.text, spoken: merge ? "" : spoken, interrupted: true, revoked: revoked)
            self?.auditResult(ok ?? true)
            await self?.refreshTurns(after: run, keepLive: merge)
        }
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
            let ok = await session?.commit(requestId: id, user: text, spoken: "", interrupted: true)
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
                if await askConsent(target, areas: areas) {
                    send(text: text)
                } else if mine == generation, consentRequest == nil {
                    // Abgelehnt (nicht: Gespräch beendet oder Dialog ersetzt): die Äußerung bleibt in `retryText`.
                    notice = AssistantNotice(text: "Ohne Zustimmung wurde nichts gesendet.", action: .retrySend)
                }
            }
            return
        }
        run.committed = true
        current = run
        currentRunID = run.id
        currentUser = run.text
        liveAnswer = ""
        liveParts = []
        firstSoundMs = nil
        phase = .thinking
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
        /// Der Body lief, aber das Protokoll dieser Anfrage hat einen Schreibfehler: nichts Weiteres wird ausgegeben.
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
        if !result.ok { return .revoked }
        return result.auditFailed ? .auditFailed : .done
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

    /// Der Start ließ sich nicht auslösen, obwohl nichts widerrufen war (Audio steht, Strom abgebrochen): Marke „fehlgeschlagen“.
    private func audioStartFailed(_ run: AssistantRun, index: Int, generation: Int, session: AssistantSession?) {
        session?.noteCommit(requestId: run.id, index: index, stage: .audio, ok: false, generation: generation, failed: true)
    }

    /// Das Protokoll ist nicht schreibbar (Ergebnis- oder Pflichtzeile): nichts wird mehr angezeigt oder gesprochen; der Zustand
    /// „Protokoll nicht schreibbar“ mit „Erneut versuchen“.
    private func stopForAuditFailure(_ run: AssistantRun) {
        guard current != nil else { return }
        cancelAnswer(merge: false)
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
        }
        while run.delivered < run.sentences.count {
            let index = run.delivered
            let sentence = run.sentences[index]
            // Ansagen („Wörtlicher Auszug aus …“) werden nur gesprochen, alles andere auch angezeigt.
            if sentence.status != .announcement {
                switch commitOutput(run, index, .display, { showLive(sentence) }) {
                case .done: break
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
        liveAnswer += (liveAnswer.isEmpty ? "" : " ") + sentence.text
        AssistantTurnPart.append(to: &liveParts, text: sentence.text, status: sentence.status, quote: sentence.quote, sources: sentence.sources)
    }

    // MARK: Stimme (Speaker → Policy, Abschluss)

    private func audible() {
        guard let run = current else { return }
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

    private func speakerIdle() {
        guard let run = current, run.inputEnded else { return }
        completeAnswer(run)
    }

    /// Alles gesprochen: Beitrag in den Verlauf (bei Fehler nach gesprochenen Sätzen als „unterbrochen“), Fehler melden.
    private func completeAnswer(_ run: AssistantRun) {
        guard !run.completed else { return }
        run.completed = true
        current = nil
        _ = policy.feed(.answerFinished(requestId: run.id))
        releaseAutoMute()
        phase = running ? .listening : .idle
        let session = self.session
        let failure = run.failure
        let spoken = run.spokenText
        let whole = run.answer?.text ?? spoken
        let keep = failure != nil && spoken.isEmpty
        if let failure { showFailure(failure, run: run) }
        enqueue { [weak self] in
            // Für JEDE Anfrage entsteht ein Protokolleintrag, auch bei Fehler ohne Gesprochenes; einen Beitrag im Verlauf
            // schreibt der Kern nur für tatsächlich Gesprochenes und nur bei noch gültiger Generation.
            let written = await session?.commit(requestId: run.id, user: run.text, spoken: failure == nil ? whole : spoken,
                                                interrupted: failure != nil, revoked: failure == .revoked)
            self?.auditResult(written ?? true)
            await self?.refreshTurns(after: run, keepLive: keep)
        }
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
            liveAnswer = ""
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
        saveChain = Task {
            await previous?.value
            await Task.detached(priority: .utility) { store.clear() }.value
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

    private func askConsent(_ target: AssistantCloudTarget, areas: Set<AssistantArea>) async -> Bool {
        resolveConsent(false)
        return await withCheckedContinuation { (continuation: CheckedContinuation<Bool, Never>) in
            consentContinuation = continuation
            consentRequest = AssistantConsentRequest(target: target, areas: areas)
        }
    }

    /// Die Zustimmung ändert die Freigabe: Der Kern zuerst (neue Generation, Zeile `release`; die Erteilung gilt erst mit der
    /// dauerhaften Zeile), danach die gespeicherte Zustimmung. Gilt sie nicht (Protokoll nicht schreibbar), bleibt sie aus.
    func grantConsent() {
        guard let request = consentRequest, !grantingConsent else { return }
        grantingConsent = true
        Task { [weak self] in
            guard let self else { return }
            let effective = await self.grantConsentInCore(areas: request.areas)
            if effective { AssistantConsent.grant(request.target, areas: request.areas) }
            self.grantingConsent = false
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

    /// Zeile der Quellen-Chips: „Buch · Titel · S. 12“, „Transkript · Titel · 00:14:03“, „Ablage · Titel“, „Notiz · Datum“.
    static func chip(_ passage: AssistantPassage) -> String { passage.sourceLabel }
}
