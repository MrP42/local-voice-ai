import Foundation
import AVFoundation
import CoreMedia
import Speech

/// Durchgehendes Zuhören für das Gespräch (Paket A): Engine, Voice Processing und `SpeechAnalyzer` laufen vom Start bis
/// zum Ende, ohne Abbau zwischen den Äußerungen und ohne Kategoriewechsel (anders als `BookTalkListener`, eine Äußerung
/// je Start). Die Steuerung bekommt Ereignisse und entscheidet über Antwort, Unterbrechung und Ende der Äußerung.
/// - Erkennung: nur Apple (`SpeechTranscriber`), nie Server, nie still nachgeladene Sprachdateien, kein Whisper-Rückfall.
/// - Audio: `.playAndRecord`, `.voiceChat` (Echo-Unterdrückung für die Antwort), Ducking minimal. Audio wird nie gespeichert.
/// - Zeitachse der Ergebnisse: Summe der an den Analyzer gegebenen Frames (`AnalyzerInput` ohne Startzeit).
@MainActor
final class AssistantListener {
    /// Warum das Zuhören endete, ohne dass die Steuerung es wollte.
    enum Interruption: Equatable {
        case call, route, device, recognizer, mediaServices

        var message: String {
            switch self {
            case .call: "Das Gespräch wurde unterbrochen (Anruf oder Systemton)."
            case .route: "Mikrofon oder Kopfhörer wurden getrennt. Das Gespräch wurde beendet."
            case .device: "Das Audiogerät hat gewechselt und ließ sich nicht neu starten. Das Gespräch wurde beendet."
            case .recognizer: "Die Spracherkennung wurde unerwartet beendet. Das Gespräch wurde beendet."
            case .mediaServices: "Der Audiodienst des Geräts wurde neu gestartet. Das Gespräch wurde beendet."
            }
        }
    }

    enum ListenError: LocalizedError {
        case microphone, speechAssets, format, engine, busy
        var errorDescription: String? {
            switch self {
            case .microphone: "Kein Zugriff auf das Mikrofon. Bitte in den Einstellungen erlauben."
            case .speechAssets: "Für das Gespräch werden die Apple-Sprachdateien gebraucht (Einstellungen)."
            case .format: "Das Mikrofonformat wird nicht unterstützt."
            case .engine: "Das Mikrofon konnte nicht gestartet werden."
            case .busy: "Das Mikrofon wird noch freigegeben. Bitte gleich noch einmal versuchen."
            }
        }
    }

    /// Der Nutzer beginnt zu sprechen (auch nach kurzer Pause wieder).
    var onSpeechStarted: (() -> Void)?
    /// Gesamter Text der laufenden Äußerung: fester und vorläufiger Teil. Zweiter Wert: wie weit (Sekunden) das Ende des Audiobereichs
    /// des Ergebnisses hinter dem bisher an den Erkenner gegebenen Ton liegt (nie negativ; `nil`, wenn das Ergebnis keinen Bereich
    /// hat). Die Steuerung rechnet damit die Audiozeit auf ihre Millisekundenuhr um (`nowMs - lag`).
    var onPartial: ((String, Double?) -> Void)?
    /// Dauer der aktuellen Stille in ms (je Puffer, solange eine Äußerung läuft); nach dem Erkenner-Tor.
    var onSilence: ((Int) -> Void)?
    var onInterrupted: ((Interruption) -> Void)?
    /// Pegel 0…1 je Puffer (nur für die Anzeige).
    var onLevel: ((Float) -> Void)?
    /// Puffer des Erkenners liefen über (gezählt, nicht verschluckt); Parameter: Gesamtzahl seit Start.
    var onOverflow: ((Int) -> Void)?
    /// Stand der Erkennung bei JEDEM Stille-Ereignis, unmittelbar VOR `onSilence`. `caughtUp`: Sie hat den Ton bis zum letzten lauten
    /// Puffer verarbeitet (ihre Ergebnisse reichen bis `lastLoudAt - AssistantSilenceGate.tolerance`) oder ein Abschluss bis dorthin
    /// (`catchUpOnSilence`) ist zurückgekehrt. `following`: Die Erkennung folgt dem Mikrofon; `false` nur, wenn ein angeforderter
    /// Abschluss seit mehr als `followingLimitMs` aussteht (dann ist belegt, dass die Analyse mindestens so weit hinterherhinkt);
    /// ohne Beleg `true` (die Doku nennt keine zuverlässige Größe für den Rückstand, siehe `requestCatchUp`). Die gemeldete Stille selbst
    /// (`onSilence`) bleibt davon unberührt.
    var onRecognition: ((_ caughtUp: Bool, _ following: Bool) -> Void)?
    /// Solange `true` (die Steuerung setzt es, solange eine Antwort aktiv ist), fordert der Hörer bei Stille, wenn die Erkennung dem
    /// letzten lauten Puffer nicht folgt, den Abschluss bis dorthin an (`SpeechAnalyzer.finalize(through:)`). Nach einem Geräusch ohne
    /// Wort (Husten) kommt sonst nie ein Ergebnis, und die Erkennung gälte für immer als „nicht eingeholt“. Aus: nichts wird angefordert.
    var catchUpOnSilence = false

    private let engine = AVAudioEngine()
    private let tap = AssistantTapState()
    private var analyzer: SpeechAnalyzer?
    private var results: Task<Void, Never>?
    private var pump: Task<Void, Never>?
    private var observers: [NSObjectProtocol] = []
    private var utterance = AssistantUtterance()
    private(set) var running = false
    private var finishing = false
    private var starting = false
    /// Jeder Start und jedes `stop()` zählt hoch; ein Start mit veraltetem Token baut sich selbst zurück.
    private var token = 0
    private var startedAt: ContinuousClock.Instant?
    /// Zeitachse (s), bis zu der ein angeforderter Abschluss zurückgekehrt ist (nach `finalize(through:)`), und der laufende Abschluss.
    private var finalizedThrough = 0.0
    private var finalizingThrough: Double?
    private var finalizeTask: Task<Void, Never>?
    /// Angeforderte, noch nicht zurückgekehrte Abschlüsse (Zeitachse des letzten lauten Puffers → Beginn der Anforderung).
    private var pendingFinalizes: [Double: ContinuousClock.Instant] = [:]
    private(set) var overflowCount = 0
    private var recoveries: [ContinuousClock.Instant] = []

    /// Obergrenze für Neustarts der Engine nach Konfigurationswechseln (innerhalb von `recoveryWindow`).
    private static let maxRecoveries = 4
    private static let recoveryWindow: Duration = .seconds(20)
    /// Routenwechsel kurz nach dem Start stammen von der eigenen Sitzungsaktivierung.
    private static let routeGrace: Duration = .milliseconds(1_500)
    /// Erst bei so viel Stille (ms) nach dem letzten lauten Puffer wird der Abschluss angefordert (kein Eingriff mitten im Satz).
    private static let catchUpAfterMs = 400
    /// Nach der Rückkehr des Abschlusses so lange (ms) warten: Die Doku sagt, die Ergebnisse stehen veröffentlicht im Strom, die App hat
    /// sie womöglich noch nicht gelesen. Die Ergebnisschleife läuft auf demselben Actor; die Frist gibt ihr sicher Zeit dafür.
    private static let catchUpSettleMs = 120
    /// Steht ein Abschluss so lange (ms) aus, hinkt die Erkennung nachweislich hinterher (`following == false`).
    private static let followingLimitMs = 1_500

    /// Start, Zuhören oder Rückbau laufen.
    var isBusy: Bool { starting || running || finishing }

    /// Bisher erkannter Text der laufenden Äußerung: fester und vorläufiger Teil (für die Anzeige).
    var liveParts: (stable: String, volatile: String) { (utterance.stable, utterance.volatile) }

    // MARK: Start

    /// Wirft `ListenError.busy`, solange Start, Zuhören oder Rückbau laufen: wer nicht zuhört, darf nicht „läuft“ melden.
    func start(languageCode: String?) async throws {
        guard !isBusy else { throw ListenError.busy }
        token += 1
        let mine = token
        starting = true
        defer { starting = false }
        do {
            try await run(languageCode: languageCode, token: mine)
        } catch {
            // Auch bei Abbruch während des Starts: dieser Aufruf besitzt die Ressourcen und räumt sie auf.
            await teardown()
            throw error
        }
    }

    private func run(languageCode: String?, token mine: Int) async throws {
        guard await AVAudioApplication.requestRecordPermission() else { throw ListenError.microphone }
        try checkCurrent(mine)
        utterance.reset()
        overflowCount = 0
        recoveries = []
        finalizedThrough = 0
        finalizingThrough = nil
        pendingFinalizes = [:]
        let english = languageCode?.lowercased().hasPrefix("en") == true
        let locale = Locale(identifier: english ? "en-US" : "de-DE")
        guard SpeechTranscriber.isAvailable, let supported = await SpeechTranscriber.supportedLocale(equivalentTo: locale) else {
            throw ListenError.speechAssets
        }
        try checkCurrent(mine)
        let transcriber = SpeechTranscriber(locale: supported, transcriptionOptions: [], reportingOptions: [.volatileResults], attributeOptions: [])
        let status = await AssetInventory.status(forModules: [transcriber])
        try checkCurrent(mine)
        guard status == .installed else { throw ListenError.speechAssets }

        let session = AVAudioSession.sharedInstance()
        try session.setCategory(.playAndRecord, mode: .voiceChat, options: [.allowBluetoothHFP, .defaultToSpeaker])
        try session.setActive(true)
        // Echo-Unterdrückung und Rauschminderung; ohne sie hörte das Mikrofon die Antwort mit: lieber sauber scheitern.
        do { try engine.inputNode.setVoiceProcessingEnabled(true) } catch { throw ListenError.engine }
        // Andere Töne (Musik) nur so leise machen wie nötig; ab iOS 17, das Mindestziel liegt darüber.
        engine.inputNode.voiceProcessingOtherAudioDuckingConfiguration =
            AVAudioVoiceProcessingOtherAudioDuckingConfiguration(enableAdvancedDucking: false, duckingLevel: .min)
        try checkCurrent(mine)

        try await startAnalyzer(transcriber, token: mine)
        try checkCurrent(mine)
        observe()
        try installTapAndStart()
        // Ein `stop()` zwischen Engine-Start und hier lässt `start()` zurückbauen.
        try checkCurrent(mine)
    }

    private func checkCurrent(_ mine: Int) throws {
        if token != mine { throw CancellationError() }
    }

    private func startAnalyzer(_ transcriber: SpeechTranscriber, token mine: Int) async throws {
        guard let format = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [transcriber]) else { throw ListenError.format }
        // Läuft der Erkenner dem Mikrofon hinterher, geht das Älteste verloren und wird gezählt (`onOverflow`).
        let (input, inputContinuation) = AsyncStream<AnalyzerInput>.makeStream(bufferingPolicy: .bufferingNewest(256))
        let (events, eventContinuation) = AsyncStream<AssistantTapEvent>.makeStream()
        tap.configure(input: inputContinuation, events: eventContinuation, format: format)
        let analyzer = SpeechAnalyzer(modules: [transcriber])
        self.analyzer = analyzer
        try await analyzer.start(inputSequence: input)
        results = Task { [weak self] in
            do {
                for try await result in transcriber.results {
                    let text = String(result.text.characters)
                    let range = result.range
                    let start = range.start.seconds, end = range.end.seconds
                    self?.received(text, start: start.isFinite ? start : nil, end: end.isFinite ? end : nil, final: result.isFinal, token: mine)
                }
                self?.recognizerEnded(token: mine)
            } catch {
                self?.recognizerEnded(token: mine)
            }
        }
        pump = Task { [weak self] in
            for await event in events { self?.handle(event) }
        }
    }

    private func installTapAndStart() throws {
        guard tap.install(on: engine) else { throw ListenError.format }
        engine.prepare()
        do { try engine.start() } catch { throw ListenError.engine }
        tap.setAccepting(true)
        running = true
        startedAt = ContinuousClock.now
    }

    // MARK: Ereignisse

    private func handle(_ event: AssistantTapEvent) {
        guard running, !finishing else { return }
        onLevel?(event.level)
        if event.overflowTotal > overflowCount {
            overflowCount = event.overflowTotal
            onOverflow?(overflowCount)
        }
        if event.started { onSpeechStarted?() }
        if event.speech { utterance.noteSpeech(ms: event.ms) }
        if let ms = event.silenceMs {
            let recognized = utterance.latestEnd >= event.lastLoudAt - AssistantSilenceGate.tolerance
            let caughtUp = recognized || finalizedThrough >= event.lastLoudAt - Self.timelineEpsilon
            onRecognition?(caughtUp, recognitionFollows())
            if !caughtUp { requestCatchUp(loudUntil: event.lastLoudAt, silenceMs: ms) }
            onSilence?(AssistantSilenceGate.reported(silenceMs: ms, lastLoudAt: event.lastLoudAt, recognizedUntil: utterance.latestEnd))
        }
    }

    /// Rundungsspielraum beim Vergleich von Zeitachsenwerten (s).
    private static let timelineEpsilon = 0.001

    /// Fordert den Abschluss der Erkennung bis zum letzten lauten Puffer an (`SpeechAnalyzer.finalize(through:)`, Doku: Rückkehr heißt
    /// „Eingabe bis einschließlich dieser Zeit ist analysiert, die finalen Ergebnisse sind im Strom veröffentlicht“, auch wenn es kein
    /// Ergebnis gab; liegt die Zeit schon vor dem flüchtigen Bereich, tut der Aufruf nichts). Danach gilt die Erkennung als eingeholt.
    /// Höchstens ein Aufruf je Position des letzten lauten Puffers; ein Fehler (Abbruch, Analyse beendet) markiert nichts.
    private func requestCatchUp(loudUntil: Double, silenceMs: Int) {
        guard catchUpOnSilence, running, !finishing, silenceMs >= Self.catchUpAfterMs, loudUntil > 0,
              finalizingThrough != loudUntil, finalizedThrough < loudUntil, let analyzer else { return }
        finalizingThrough = loudUntil
        pendingFinalizes[loudUntil] = ContinuousClock.now
        let mine = token
        finalizeTask = Task { [weak self] in
            var completed = false
            do {
                try await analyzer.finalize(through: CMTime(seconds: loudUntil, preferredTimescale: 1_000))
                try await Task.sleep(for: .milliseconds(Self.catchUpSettleMs))
                completed = true
            } catch {
                completed = false
            }
            self?.catchUpEnded(loudUntil: loudUntil, completed: completed, token: mine)
        }
    }

    private func catchUpEnded(loudUntil: Double, completed: Bool, token mine: Int) {
        guard token == mine, running else { return }
        pendingFinalizes[loudUntil] = nil
        if completed { finalizedThrough = max(finalizedThrough, loudUntil) }
    }

    /// Folgt die Erkennung dem Mikrofon? Nur belegt „nein“, wenn ein angeforderter Abschluss zu lange aussteht: Er kehrt erst zurück, wenn
    /// die Eingabe bis zur angefragten Zeit analysiert ist, und diese Zeit liegt mindestens `catchUpAfterMs` hinter dem Mikrofon.
    /// (`volatileRange` taugt dafür nicht: Ihr Ende ist die Stelle, bis zu der Eingabe aus der Folge geholt wurde, einschließlich der
    /// noch ausstehenden Analysen, nicht die Stelle, bis zu der analysiert ist.)
    private func recognitionFollows() -> Bool {
        let now = ContinuousClock.now
        return !pendingFinalizes.values.contains { $0.duration(to: now) >= .milliseconds(Self.followingLimitMs) }
    }

    private func received(_ text: String, start: Double?, end: Double?, final: Bool, token mine: Int) {
        guard running, token == mine else { return }
        guard utterance.apply(text, start: start, end: end, isFinal: final) else { return }
        // Der Erkenner hört Sprache, die der Pegelmesser nicht als solche zählte (leise Stimme): Stille ab jetzt zählen.
        tap.assumeSpeech()
        // Rückstand des Ergebnisses gegenüber dem Mikrofon (die Zeitachse läuft nur mit den an den Erkenner gegebenen Puffern).
        let lag = end.map { max(0, tap.timeline() - $0) }
        onPartial?(utterance.text, lag)
    }

    private func recognizerEnded(token mine: Int) {
        guard running, token == mine else { return }
        interrupted(.recognizer)
    }

    // MARK: Steuerung

    /// Äußerung abschließen: Text abholen und die Erkennung für die nächste Äußerung zurücksetzen. Ergebnisse, die vor
    /// diesem Zeitpunkt enden, zählen nicht zur nächsten Äußerung. Aufrufen bei `.commit`, `.stopCommand` und `.releaseAnswer` (und beim
    /// Zurücksetzen: neues Gespräch, Senden von Hand). NICHT bei `.ignored`: Die Richtlinie zieht Echo und Rückmeldelaute selbst über die
    /// Wortposition im kumulativen Text ab; `onPartial` muss bis zum Äußerungsende kumulativ bleiben.
    @discardableResult
    func consumeUtterance() -> String {
        let text = utterance.consume(at: tap.timeline())
        tap.rearm()
        return text
    }

    /// Stumm: Mikrofonpuffer werden verworfen (weder Pegel noch Erkenner), laufender Text wird verworfen.
    func setMuted(_ muted: Bool) {
        tap.setMuted(muted)
        if muted { _ = utterance.consume(at: tap.timeline()) }
    }

    /// Beenden, ohne zu warten. Ein laufender Start baut sich selbst zurück.
    func stop() {
        token += 1
        guard running, !finishing, !starting else { return }
        finishing = true
        Task {
            await teardown()
            finishing = false
        }
    }

    /// Wie `stop()`, kehrt aber erst zurück, wenn Mikrofon und Sitzung wirklich frei sind (höchstens ~4 s).
    func stopAndWait() async {
        stop()
        var waited = 0
        while isBusy, waited < 200 {
            try? await Task.sleep(for: .milliseconds(20))
            waited += 1
        }
    }

    private func interrupted(_ why: Interruption) {
        guard running, !finishing else { return }
        finishing = true
        let mine = token
        Task {
            await teardown()
            finishing = false
            guard token == mine else { return }
            onInterrupted?(why)
        }
    }

    /// Rückbau in fester Reihenfolge: sperren, Tap und Engine, Beobachter, Eingabestrom, Analyzer, Ergebnisse, Sitzung.
    /// Alle Wartezeiten sind begrenzt; danach steht die Sitzung auf Wiedergabe (Antwort und Vorlesen brauchen sie).
    private func teardown() async {
        let sinks = tap.stop()
        engine.inputNode.removeTap(onBus: 0)
        engine.stop()
        running = false
        observers.forEach(NotificationCenter.default.removeObserver)
        observers = []
        sinks.input?.finish()
        sinks.events?.finish()
        pump?.cancel()
        pump = nil
        finalizeTask?.cancel()
        finalizeTask = nil
        finalizingThrough = nil
        if let analyzer {
            _ = await LiveCapture.bounded(2) { await analyzer.cancelAndFinishNow() }
        }
        if let results {
            results.cancel()
            _ = await LiveCapture.bounded(2) { await results.value }
        }
        results = nil
        analyzer = nil
        tap.clear()
        startedAt = nil
        try? AVAudioSession.sharedInstance().setCategory(.playback, mode: .spokenAudio)
        try? AVAudioSession.sharedInstance().setActive(true)
    }

    // MARK: Beobachter und Wiederherstellung

    private func observe() {
        let center = NotificationCenter.default
        observers.append(center.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: .main) { [weak self] note in
            guard let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
                  AVAudioSession.InterruptionType(rawValue: raw) == .began else { return }
            MainActor.assumeIsolated { self?.interrupted(.call) }
        })
        observers.append(center.addObserver(forName: AVAudioSession.routeChangeNotification, object: nil, queue: .main) { [weak self] note in
            guard let raw = note.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt,
                  AVAudioSession.RouteChangeReason(rawValue: raw) == .oldDeviceUnavailable else { return }
            MainActor.assumeIsolated { self?.routeLost() }
        })
        observers.append(center.addObserver(forName: .AVAudioEngineConfigurationChange, object: engine, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.configurationChanged() }
        })
        observers.append(center.addObserver(forName: AVAudioSession.mediaServicesWereResetNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.interrupted(.mediaServices) }
        })
    }

    private func routeLost() {
        if let startedAt, startedAt.duration(to: .now) < Self.routeGrace { return }
        interrupted(.route)
    }

    /// Die Engine hat ihre Konfiguration geändert (Gerätewechsel, Start der zweiten Engine für die Stimme): neu
    /// aufsetzen und weiterlaufen. Nur wenn das scheitert oder zu oft vorkommt, endet das Gespräch.
    private func configurationChanged() {
        guard running, !finishing else { return }
        let now = ContinuousClock.now
        recoveries = recoveries.filter { $0.duration(to: now) < Self.recoveryWindow }
        guard recoveries.count < Self.maxRecoveries else { return interrupted(.device) }
        recoveries.append(now)
        let mine = token
        Task { await recover(token: mine) }
    }

    private func recover(token mine: Int) async {
        for attempt in 0..<3 {
            guard token == mine, running, !finishing else { return }
            if attempt > 0 { try? await Task.sleep(for: .milliseconds(150)) }
            guard token == mine, running, !finishing else { return }
            if engine.isRunning, tap.isInstalled { return }
            if tap.install(on: engine) {
                engine.prepare()
                if (try? engine.start()) != nil { return }
            }
        }
        guard token == mine, running, !finishing else { return }
        interrupted(.device)
    }
}

/// Ein Mikrofonpuffer, vom Audio-Thread zur Steuerung gemeldet.
struct AssistantTapEvent: Sendable {
    var level: Float
    var ms: Int
    var started: Bool
    var speech: Bool
    var silenceMs: Int?
    /// Zeitachse (s) des Endes des letzten Sprachpuffers.
    var lastLoudAt: Double
    var overflowTotal: Int
}

/// Alles, was der Audio-Thread anfasst: unter `lock`, ohne Main-Actor-Bezug (wie `TapState` im Buch-Gespräch).
/// Der Lock wird nur kurz gehalten (Zustand lesen/schreiben); Umwandlung, Weitergabe an den Erkenner und Ereignis
/// laufen außerhalb. Kein I/O, keine Warteschlange mit Wartezeit.
private final class AssistantTapState: @unchecked Sendable {
    private let lock = NSLock()
    private var accepting = false
    private var muted = false
    private var vad = AssistantVAD()
    private var input: AsyncStream<AnalyzerInput>.Continuation?
    private var events: AsyncStream<AssistantTapEvent>.Continuation?
    private var analyzerFormat: AVAudioFormat?
    private var converter: AVAudioConverter?
    private var frames: Int64 = 0
    private var lastLoudAt = 0.0
    private var overflows = 0
    private var tapInstalled = false

    func configure(input: AsyncStream<AnalyzerInput>.Continuation, events: AsyncStream<AssistantTapEvent>.Continuation, format: AVAudioFormat) {
        lock.lock(); defer { lock.unlock() }
        self.input = input
        self.events = events
        analyzerFormat = format
        vad = AssistantVAD()
        frames = 0
        lastLoudAt = 0
        overflows = 0
        muted = false
    }

    /// Wandler für das aktuelle Mikrofonformat und Tap auf der Engine. Absichtlich hier (nicht isoliert), damit der
    /// Rückruf nicht den Main Actor erbt: er läuft auf dem Audio-Thread. `false`, wenn ein Wandler fehlt.
    func install(on engine: AVAudioEngine) -> Bool {
        let node = engine.inputNode
        node.removeTap(onBus: 0)
        setInstalled(false)
        let mic = node.outputFormat(forBus: 0)
        guard mic.sampleRate > 0, mic.channelCount > 0, prepareConverter(from: mic) else { return false }
        node.installTap(onBus: 0, bufferSize: 4096, format: mic) { [self] buffer, _ in handle(buffer) }
        setInstalled(true)
        return true
    }

    private func prepareConverter(from mic: AVAudioFormat) -> Bool {
        lock.lock(); defer { lock.unlock() }
        guard let analyzerFormat else { return false }
        let made = AVAudioConverter(from: mic, to: analyzerFormat)
        // Mehrkanaleingang (Voice Processing kann mehr als einen Kanal melden): Ausgang aus den ersten Kanälen ableiten,
        // wie auch der Pegel nur Kanal 0 misst.
        if mic.channelCount > analyzerFormat.channelCount {
            made?.channelMap = (0..<Int(analyzerFormat.channelCount)).map { NSNumber(value: $0) }
        }
        converter = made
        return made != nil
    }

    private func setInstalled(_ value: Bool) {
        lock.lock(); tapInstalled = value; lock.unlock()
    }

    var isInstalled: Bool {
        lock.lock(); defer { lock.unlock() }
        return tapInstalled
    }

    func setAccepting(_ value: Bool) {
        lock.lock(); accepting = value; lock.unlock()
    }

    func setMuted(_ value: Bool) {
        lock.lock()
        muted = value
        if value { vad.rearm() }
        lock.unlock()
    }

    /// Der Erkenner hört Sprache: Stille ab jetzt zählen, falls der Pegelmesser sie nicht erkannt hat.
    func assumeSpeech() {
        lock.lock()
        if !muted, !vad.inUtterance {
            vad.assumeSpeech()
            lastLoudAt = analyzerSeconds()
        }
        lock.unlock()
    }

    func rearm() {
        lock.lock(); vad.rearm(); lock.unlock()
    }

    /// Position der Zeitachse (s): Summe der an den Erkenner gegebenen Frames.
    func timeline() -> Double {
        lock.lock(); defer { lock.unlock() }
        return analyzerSeconds()
    }

    private func analyzerSeconds() -> Double {
        guard let rate = analyzerFormat?.sampleRate, rate > 0 else { return 0 }
        return Double(frames) / rate
    }

    /// Sperrt (wartet höchstens auf einen laufenden Zustandszugriff) und gibt die Ströme zum Schließen zurück.
    func stop() -> (input: AsyncStream<AnalyzerInput>.Continuation?, events: AsyncStream<AssistantTapEvent>.Continuation?) {
        lock.lock(); defer { lock.unlock() }
        accepting = false
        let sinks = (input: input, events: events)
        input = nil
        events = nil
        return sinks
    }

    func clear() {
        lock.lock(); defer { lock.unlock() }
        converter = nil
        analyzerFormat = nil
        input = nil
        events = nil
        tapInstalled = false
    }

    func handle(_ buffer: AVAudioPCMBuffer) {
        let count = Int(buffer.frameLength)
        let rate = buffer.format.sampleRate
        guard count > 0, rate > 0 else { return }
        let ms = max(1, Int(Double(count) / rate * 1000))

        lock.lock()
        let live = accepting && !muted
        let converter = self.converter, format = analyzerFormat, input = self.input
        lock.unlock()
        guard live else { return }

        let level = Self.rms(buffer)
        var converted: AVAudioPCMBuffer?
        if let converter, let format { converted = Self.convert(buffer, with: converter, to: format) }
        var dropped = false
        if let converted, let input, case .dropped = input.yield(AnalyzerInput(buffer: converted)) { dropped = true }

        lock.lock()
        guard accepting, !muted else { lock.unlock(); return }
        // Bei Überlauf fällt der älteste Puffer weg, der neue kommt hinein: die Frames am Erkenner bleiben netto gleich.
        if let converted, !dropped, input != nil { frames += Int64(converted.frameLength) }
        if dropped { overflows += 1 }
        let result = vad.feed(rms: level, ms: ms)
        let seconds = analyzerSeconds()
        let heard = vad.heardSpeech
        if heard { lastLoudAt = seconds }
        var started = false
        var silence: Int?
        switch result {
        case .speechStarted: started = true
        case .silence(let value): silence = value
        case .none: break
        }
        let event = AssistantTapEvent(level: level, ms: ms, started: started, speech: heard, silenceMs: silence,
                                      lastLoudAt: lastLoudAt, overflowTotal: overflows)
        let sink = events
        lock.unlock()
        sink?.yield(event)
    }

    private static func rms(_ buffer: AVAudioPCMBuffer) -> Float {
        guard let data = buffer.floatChannelData, buffer.frameLength > 0 else { return 0 }
        let count = Int(buffer.frameLength)
        let channel = data[0]
        var sum: Float = 0
        for i in 0..<count { sum += channel[i] * channel[i] }
        return (sum / Float(count)).squareRoot()
    }

    private static func convert(_ buffer: AVAudioPCMBuffer, with converter: AVAudioConverter, to format: AVAudioFormat) -> AVAudioPCMBuffer? {
        let ratio = format.sampleRate / buffer.format.sampleRate
        let capacity = AVAudioFrameCount(Double(buffer.frameLength) * ratio) + 32
        guard let out = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: capacity) else { return nil }
        var fed = false
        var error: NSError?
        converter.convert(to: out, error: &error) { _, status in
            if fed { status.pointee = .noDataNow; return nil }
            fed = true; status.pointee = .haveData; return buffer
        }
        return error == nil && out.frameLength > 0 ? out : nil
    }
}
