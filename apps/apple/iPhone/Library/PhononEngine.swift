import Foundation
#if PHONON
import AVFoundation

/// Wiedergabe für Phonon. Eigener Player statt `PhononPlayer`, weil das Vorlesen echtes
/// Pausieren braucht. Das Tempo steckt schon im eingereihten Ton (`SpeechStretcher`):
/// `AVAudioUnitTimePitch` klang auch in höchster Qualität blechern (#314).
///
/// `AVAudioPlayerNode.stop()` wartet synchron auf die Rückrufe der Node. Deshalb läuft alles,
/// was blockieren kann, auf einer eigenen Warteschlange: nie auf dem Main Thread und nie in
/// einem Cancel-Handler, dort hält die Laufzeit die Sperre der Task, die der Rückruf zum
/// Fortsetzen braucht (Einfrieren am 05.11.2026 beim Wechsel Vorlesen → Hörbuch).
private final class PhononAudio: @unchecked Sendable {
    private static let queue = DispatchQueue(label: "phonon-audio", qos: .userInitiated)

    private let engine = AVAudioEngine()
    private let node = AVAudioPlayerNode()
    private let format = AVAudioFormat(standardFormatWithSampleRate: Phonon.sampleRate, channels: 1)!
    private let played: @Sendable () -> Void
    private let lock = NSLock()
    private var pending = 0
    private var stopped = false
    private var waiters: [(limit: Int, done: CheckedContinuation<Void, Never>)] = []
    private var stalled = false
    private var observer: NSObjectProtocol?

    static func make(played: @escaping @Sendable () -> Void) async throws -> PhononAudio {
        try await withCheckedThrowingContinuation { done in
            queue.async { done.resume(with: Result { try PhononAudio(played: played) }) }
        }
    }

    private init(played: @escaping @Sendable () -> Void) throws {
        self.played = played
        engine.attach(node)
        engine.connect(node, to: engine.mainMixerNode, format: format)
        try engine.start()
        node.play()
        // Neues Hardwareformat (AirPods, Mediendienste): Die Engine steht dann, ohne dass Rückrufe kommen.
        observer = NotificationCenter.default.addObserver(forName: .AVAudioEngineConfigurationChange, object: engine, queue: nil) { [weak self] _ in
            Self.queue.async {
                guard let self, !self.engine.isRunning else { return }
                self.lock.withLock { self.stalled = true }
                self.shutdown()
            }
        }
    }

    private func buffer(_ samples: [Float]) -> AVAudioPCMBuffer? {
        guard !samples.isEmpty, let buf = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(samples.count)) else { return nil }
        buf.frameLength = AVAudioFrameCount(samples.count)
        samples.withUnsafeBufferPointer { buf.floatChannelData![0].update(from: $0.baseAddress!, count: samples.count) }
        return buf
    }

    var isStopped: Bool { lock.withLock { stopped } }
    var isStalled: Bool { lock.withLock { stalled } }

    /// Reiht Ton ein. Mit `start` (Gesprächsmodus, Widerrufsgrenze) ist nur das Einreihen des ERSTEN hörbaren Tons, also der
    /// tatsächliche Beginn der Wiedergabe, der Body von `start`; der Puffer entsteht vorher, damit der Body kurz bleibt. `false`:
    /// `start` hat abgewiesen, nichts wurde eingereiht.
    func enqueue(_ samples: [Float], start: PhononStart? = nil) -> Bool {
        guard !isStopped, let buf = buffer(samples) else { return true }
        guard let start else { node.scheduleBuffer(buf); return true }
        return start { node.scheduleBuffer(buf) }
    }

    /// Satzende. Der Rückruf kommt, wenn alles davor wirklich gespielt ist.
    func mark() {
        guard let tail = buffer([Float](repeating: 0, count: 240)) else { return }
        let accepted = lock.withLock { () -> Bool in
            if !stopped { pending += 1 }
            return !stopped
        }
        if accepted { node.scheduleBuffer(tail, completionCallbackType: .dataPlayedBack) { [self] _ in markPlayed() } }
    }

    private func markPlayed() {
        var ready: [CheckedContinuation<Void, Never>] = []
        let report = lock.withLock { () -> Bool in
            pending -= 1
            waiters.removeAll { waiter in
                guard pending < waiter.limit else { return false }
                ready.append(waiter.done)
                return true
            }
            return !stopped
        }
        if report { played() }  // vor dem Aufwecken, damit „Ende“ nie vor dem letzten Satz ankommt
        ready.forEach { $0.resume() }
    }

    /// Wartet, bis weniger als `limit` Sätze ungespielt sind. Eine Pause verlängert das Warten.
    func wait(below limit: Int) async {
        await withTaskCancellationHandler {
            await withCheckedContinuation { (done: CheckedContinuation<Void, Never>) in
                let goOn = lock.withLock { () -> Bool in
                    if stopped || pending < limit { return true }
                    waiters.append((limit, done))
                    return false
                }
                if goOn { done.resume() }
            }
        } onCancel: { [self] in
            Self.queue.async { self.shutdown() }
        }
    }

    func pause() { Self.queue.async { [self] in node.pause() } }

    /// `false`, wenn die Audio-Engine inzwischen steht (Anruf, Aufnahme, Routenwechsel).
    func resume() -> Bool {
        guard engine.isRunning else { return false }
        Self.queue.async { [self] in node.play() }
        return true
    }

    /// Blockiert nicht und darf mehrfach kommen.
    func shutdown() {
        let waiting = lock.withLock { () -> [CheckedContinuation<Void, Never>]? in
            if stopped { return nil }
            stopped = true
            defer { waiters = [] }
            return waiters.map(\.done)
        }
        guard let waiting else { return }
        waiting.forEach { $0.resume() }
        Self.queue.async { [self] in
            if let observer { NotificationCenter.default.removeObserver(observer) }
            node.stop(); engine.stop()
        }
    }
}
#endif

#if PHONON
/// Zuletzt erzeugte Sätze je Stimme. Das Tempo kommt erst bei der Wiedergabe dazu, also passt
/// ein Eintrag immer: Zurückspringen, Pause und Weiterlesen klingen sofort, ohne neu zu erzeugen.
private final class SentenceAudioCache: @unchecked Sendable {
    private let lock = NSLock()
    private var entries: [(key: String, samples: [Float])] = []
    private let limit = 12  // drei zurück, drei voraus und etwas Spielraum; rund 1 MB je Satz

    func samples(_ voice: String, _ text: String) -> [Float]? {
        lock.withLock { entries.first { $0.key == voice + "|" + text }?.samples }
    }

    func store(_ voice: String, _ text: String, _ samples: [Float]) {
        guard !samples.isEmpty else { return }
        let key = voice + "|" + text
        lock.withLock {
            entries.removeAll { $0.key == key }
            entries.append((key, samples))
            if entries.count > limit { entries.removeFirst(entries.count - limit) }
        }
    }
}
#endif

/// Phonon-Stimmen fürs Vorlesen (Neural Engine, alles auf dem Gerät). Das Paket liegt
/// nur als arm64 vor, daher gibt es Phonon nur im Gerätebau (`PHONON`), nicht im
/// Simulator auf Intel-Macs.
///
/// Je Sprache ein eigenes Modell. Die Modelle stehen nicht im Repo und nicht in der App:
/// Die deutschen Gewichte sind CC BY-NC. Sie werden einmalig nach `Documents/PhononDE`
/// bzw. `Documents/PhononEN` gelegt und von dort nach Application Support übernommen.
/// Ein Satz zum Vorlesen; am Absatzende folgt eine längere Pause.
struct PhononSentence: Sendable {
    var text: String
    var paragraphEnd: Bool
    var voice: String? = nil
    var style: String? = nil
    /// Eine Fortschrittsmarke bleibt ein Rohsatz, auch mit mehreren Stimmen/Pausen.
    var parts: [SpeakerScript.SpeechPart]? = nil
}

struct PhononUnavailable: LocalizedError {
    var errorDescription: String? { "Phonon ist nicht verfügbar." }
}

/// Die Audio-Engine steht von außen still; die Lesung kann neu beginnen.
struct PhononAudioStalled: Error {}

/// Der tatsächliche Start einer Äußerung als Body einer atomaren Prüfung (Gesprächsmodus, `AssistantPlaybackGate.start`): ruft
/// den Body genau dann auf, wenn der Satz noch starten darf, im selben Lock wie die Erhöhung der Freigabe-Generation, und
/// meldet `false`, wenn nicht. Der Body ist kurz und synchron und ruft nie zurück.
typealias PhononStart = @Sendable (() -> Void) -> Bool

/// Der Start wurde abgewiesen (Freigabe widerrufen): Nichts wurde gestartet, der Satz zählt als nicht gesprochen. Kein Fehler
/// der Stimme: Der Aufrufer fällt nicht auf eine andere Stimme zurück.
struct PhononStartRefused: Error {}

@MainActor
final class PhononEngine: ObservableObject {
    static let shared = PhononEngine()
    static let voicePrefix = "phonon:"
    static let languages = ["de", "en"]

    /// Stimmen je Sprache aus den tatsächlich installierten voices/-Ordnern.
    @Published private(set) var voices: [String: [String]] = [:]
    /// Eigene Stimmen je Sprache (über die Ablage von anderen Geräten), Teilmenge von `voices`.
    @Published private(set) var ownVoices: [String: Set<String>] = [:]
    @Published private(set) var loading = false

    /// „phonon:de:Nele“ → „de:Nele“; alles andere ist keine Phonon-Stimme.
    static func voiceName(_ identifier: String?) -> String? {
        guard let identifier, identifier.hasPrefix(voicePrefix) else { return nil }
        let rest = String(identifier.dropFirst(voicePrefix.count))
        return rest.contains(":") ? rest : nil
    }

    static func identifier(language: String, voice: String) -> String { "\(voicePrefix)\(language):\(voice)" }

    nonisolated static func parts(_ name: String) -> (language: String, voice: String)? {
        guard let i = name.firstIndex(of: ":") else { return nil }
        return (String(name[..<i]), String(name[name.index(after: i)...]))
    }

    static func displayName(_ name: String) -> String { parts(name)?.voice ?? name }

    func scriptSpeakers(language: String?) -> [SpeakerScript.Speaker] {
        let languages = language.map { [$0] } ?? Self.languages
        return languages.flatMap { language in
            SpeakerScript.roster((voices[language] ?? []).map { .init(id: $0, label: $0) }).map { speaker in
                var speaker = speaker
                speaker.id = "\(language):\(speaker.id)"
                return speaker
            }
        }
    }

    static func sectionTitle(_ language: String) -> String { language == "de" ? "Phonon Deutsch" : "Phonon Englisch" }
    static func ownSectionTitle(_ language: String) -> String { language == "de" ? "Meine Phonon-Stimmen · Deutsch" : "Meine Phonon-Stimmen · Englisch" }

    #if PHONON
    /// Höchstens ein Modell im Speicher; ein laufendes Laden wird geteilt.
    private var loaded: (language: String, tts: Phonon)?
    private var loading_: (language: String, task: Task<Phonon, Error>)?
    private var player: PhononAudio?
    private var requestedRate: Float = 1
    private var cache = SentenceAudioCache()
    private var modelGeneration = 0
    private var prefetch: Task<Void, Never>?
    private var refreshTask: Task<Void, Never>?
    private var stimmenRevision = 0
    private var readers = 0

    private func dropped(_ language: String) -> URL {
        FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Phonon\(language.uppercased())", isDirectory: true)
    }

    private func installed(_ language: String) -> URL {
        PhononModels.defaultDirectory.deletingLastPathComponent().appendingPathComponent("models-\(language)", isDirectory: true)
    }

    nonisolated private static func built(_ dir: URL) -> Int? {
        guard let data = try? Data(contentsOf: dir.appendingPathComponent("bundle.json")),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
        return (json["built"] as? NSNumber)?.intValue
    }

    private func invalidateModels() {
        modelGeneration += 1
        prefetch?.cancel(); prefetch = nil
        loaded = nil; loading_ = nil
        loading = false
        cache = SentenceAudioCache()
        // Eine Lesung hält ihr tts und ihren Cache selbst bis zum Ende; den Player nicht stoppen.
    }

    /// Nach dem Ablage-Pull auch dann invalidieren, wenn die Injektion schon im AblageLauf geschah.
    func stimmenAktualisieren() async {
        invalidateModels()
        stimmenRevision += 1
        await refresh()
    }

    var modelAvailable: Bool { !voices.isEmpty }

    /// Modelle übernehmen und ihre Stimmen zeigen; geladen wird erst beim Sprechen.
    func refresh() async {
        // Gleichzeitige Aufrufe warten auf denselben Lauf, sonst sähen sie noch keine Stimmen.
        if let running = refreshTask { await running.value; return }
        let task = Task {
            // Ein Pull während des Scans darf nicht hinter dessen alter Stimmliste verschwinden.
            while true {
                let revision = stimmenRevision
                await scanModels()
                if revision == stimmenRevision { break }
            }
            refreshTask = nil
        }
        refreshTask = task
        await task.value
    }

    private func scanModels() async {
        var found: [String: [String]] = [:]
        var own: [String: Set<String>] = [:]
        for language in Self.languages {
            let source = dropped(language), target = installed(language)
            let scan = await Task.detached(priority: .userInitiated) { () -> (list: [String], own: Set<String>) in
                PhononStimmenInjektion.exklusiv {
                    // Nur eine vollständige, neue Kopie übernehmen, sonst bleibt das Vorhandene samt
                    // übersetzten Modellen. `isInstalled` taugt nur für die Quelle: Core ML lässt im
                    // Ziel nach dem Übersetzen nicht alle Dateien des Pakets stehen.
                    if PhononModels.isInstalled(at: source), Self.built(source) != Self.built(target) {
                        try? PhononModels.install(bundled: source, to: target)
                    }
                    if Self.built(source) != nil, Self.built(source) == Self.built(target) {
                        try? FileManager.default.removeItem(at: source)  // nicht doppelt belegen
                    }
                    try? PhononStimmenInjektion.abgleichen(wurzel: target.deletingLastPathComponent().appendingPathComponent("eigene-stimmen"))
                    let list = PhononStimmenInjektion.stimmen(in: target)
                    let mine = (try? PhononStimmenInjektion.eigeneNamen(in: target)) ?? []
                    return (list, mine.intersection(list))
                }
            }.value
            if !scan.list.isEmpty { found[language] = scan.list }
            if !scan.own.isEmpty { own[language] = scan.own }
        }
        invalidateModels()
        voices = found
        ownVoices = own
        // Vorgabe einmalig: Anton, sobald das Modell da ist; danach gilt die eigene Wahl.
        let defaults = UserDefaults.standard
        if !defaults.bool(forKey: "voice.antonDefault"), found["de"]?.contains("Anton") == true {
            defaults.set(true, forKey: "voice.antonDefault")
            VoiceModel.shared.selectedVoiceID = Self.identifier(language: "de", voice: "Anton")
        }
    }

    /// Beim ersten Mal übersetzt Core ML das Modell für dieses Gerät (einige Sekunden).
    private func engine(_ language: String) async throws -> Phonon {
        if let loaded, loaded.language == language { return loaded.tts }
        if let loading_, loading_.language == language { return try await loading_.task.value }
        if voices.isEmpty { await refresh() }
        let dir = installed(language), lang: Language = language == "de" ? .german : .english
        let generation = modelGeneration
        let task = Task { try await Phonon.load(models: dir, language: lang) }
        loading_ = (language, task)
        loading = true
        defer {
            if generation == modelGeneration { loading = false; if loading_?.language == language { loading_ = nil } }
        }
        let tts = try await task.value
        if generation == modelGeneration { loaded = (language, tts) }
        return tts
    }

    /// Modell und Stimme laden und die ersten Sätze vorab erzeugen, damit Abspielen sofort klingt.
    func prepare(voice name: String, sentences: [String] = [], items: [PhononSentence] = []) {
        guard readers == 0 else { return }
        let previous = prefetch
        previous?.cancel()
        prefetch = Task { [weak self] in
            await previous?.value
            guard let self, let parts = Self.parts(name) else { return }
            let cache = self.cache
            guard let tts = try? await self.engine(parts.language), !Task.isCancelled else { return }
            let pieces = items.isEmpty ? sentences.map { PhononSentence(text: $0, paragraphEnd: false) } : items
            await Self.preparePieces(tts, voice: name, language: parts.language, items: pieces, cache: cache)
        }
    }

    nonisolated private static func preparePieces(_ tts: Phonon, voice: String, language: String,
                                                  items: [PhononSentence], cache: SentenceAudioCache) async {
        if items.isEmpty {
            guard !Task.isCancelled, let selected = Self.parts(voice) else { return }
            if tts.voice != selected.voice { try? await tts.setVoice(selected.voice) }
            return
        }
        for item in items {
            for part in item.parts ?? SpeakerScript(item.text).speechParts(fallbackVoice: item.voice ?? voice) {
                guard !Task.isCancelled else { return }
                guard part.pauseMillis == 0, cache.samples(part.voice, part.text) == nil,
                      let selected = Self.parts(part.voice), selected.language == language else { continue }
                do {
                    if tts.voice != selected.voice { try await tts.setVoice(selected.voice) }
                    try Task.checkCancellation()
                    let samples = try await tts.synthesize(SpeechText.spoken(part.text, language: language))
                    try Task.checkCancellation()
                    cache.store(part.voice, part.text, samples)
                } catch { return }
            }
        }
    }

    /// Text sprechen und zurückkehren, wenn er zu Ende gespielt ist. Abbruch über die Task.
    /// `start` (optional, Standard `nil`: Verhalten wie bisher): der tatsächliche Beginn der Wiedergabe (erster eingereihter
    /// hörbarer Ton) läuft als Body von `start`; weist es ab, endet die Äußerung mit `PhononStartRefused`, ohne Ton.
    ///
    /// Gesprächsmodus: Ein langer Block geht als EINE Einheit durch Freigabe und Protokoll. Nur für die Synthese wird er in Sätze
    /// zerlegt (Audioebene, der Text bleibt unverändert und lückenlos), damit der erste Ton nicht auf die ganze Synthese warten muss.
    /// Das erste hörbare Stück geht durch `start` (Marke, atomar mit der Generation), jedes weitere durch `keep` (nur die
    /// Wiedergabesperre, keine Marke): Nach der Rückkehr einer Erhöhung wird kein Stück der alten Generation mehr eingereiht.
    func speak(_ text: String, voice name: String, rate: Float = 1, start: PhononStart? = nil, keep: PhononStart? = nil) async throws {
        var pieces = Self.synthesisPieces(text)[...]
        try await read(voice: name, rate: rate, next: { pieces.popFirst() }, audible: {}, played: {}, start: start, keep: keep)
    }

    /// Zerlegt einen Block für die Synthese in Sätze (nur auf der Audioebene). Zeilenenden werden zu Absatzpausen.
    nonisolated static func synthesisPieces(_ text: String) -> [PhononSentence] {
        var out: [PhononSentence] = []
        for line in text.components(separatedBy: .newlines) {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            guard !trimmed.isEmpty else { continue }
            let units = Array(trimmed.utf16)
            let sentences = BookTalkSentences.split(SpeakerScript.maskedControls(trimmed)).map { String(decoding: units[$0.start..<$0.end], as: UTF16.self) }
                .map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
            let pieces = sentences.isEmpty ? [trimmed] : sentences
            for (i, piece) in pieces.enumerated() { out.append(PhononSentence(text: piece, paragraphEnd: i == pieces.count - 1 && line != text)) }
        }
        return out.isEmpty ? [PhononSentence(text: text, paragraphEnd: false)] : out
    }

    /// Sätze ohne Lücke nacheinander sprechen. `next` liefert den nächsten Satz (erzeugt wird
    /// etwas im Voraus; asynchron, damit die Live-Übersetzung ihn nachreichen kann, lva#358), `audible` meldet den ersten Ton, `played` das Ende jedes Satzes.
    func read(voice name: String, rate: Float, next: @escaping @MainActor () async -> PhononSentence?,
              audible: @escaping @MainActor () -> Void, played: @escaping @MainActor () -> Void,
              start: PhononStart? = nil, keep: PhononStart? = nil) async throws {
        guard let parts = Self.parts(name) else { throw PhononUnavailable() }
        readers += 1
        defer { readers -= 1 }
        let preparing = prefetch
        preparing?.cancel()
        await preparing?.value
        try Task.checkCancellation()
        requestedRate = rate
        let cache = self.cache
        let tts = try await engine(parts.language)
        try Task.checkCancellation()
        // Je Lesung ein neuer Player: Nach Anruf, Aufnahme oder Routenwechsel steht die alte Audio-Engine.
        let audio = try await PhononAudio.make {
            DispatchQueue.main.async { MainActor.assumeIsolated { played() } }
        }
        defer { audio.shutdown(); if player === audio { player = nil } }
        try Task.checkCancellation()
        player = audio
        try await Self.pump(tts, audio, voice: name, language: parts.language, rate: Double(requestedRate), flow: SpeechFlow.stored(),
                            cache: cache, next: next, audible: audible, start: start, keep: keep)
    }

    /// Erzeugen und Einreihen abseits des Main Threads. Zum hörbaren Satz liegen drei weitere bereit.
    /// Je Satz: Ränder glätten und Pausen nach dem Sprachfluss setzen (`SentenceShaper`),
    /// dann tonhöhensynchron aufs Tempo bringen (`SpeechStretcher`). Ein neues Tempo oder ein
    /// neuer Sprachfluss gilt ab dem nächsten Lesen; `LibraryPlayback` setzt dafür am Satz neu an.
    nonisolated private static func pump(_ tts: Phonon, _ audio: PhononAudio, voice: String, language: String, rate: Double,
                                         flow: SpeechFlow, cache: SentenceAudioCache,
                                         next: @MainActor () async -> PhononSentence?, audible: @MainActor () -> Void,
                                         start: PhononStart?, keep: PhononStart?) async throws {
        var silent = true
        var shaper = flow.shaper(sampleRate: Phonon.sampleRate)
        var stretcher = SpeechStretcher(sampleRate: Phonon.sampleRate, speed: rate)
        while let item = await next() {
            await audio.wait(below: 4)
            try check(audio)
            let parts = item.parts ?? SpeakerScript(item.text).speechParts(fallbackVoice: item.voice ?? voice)
            for part in parts {
                try check(audio)
                if part.pauseMillis > 0 {
                    let rest = stretcher.push(shaper.finish(paragraphEnd: false)) + stretcher.flush()
                    try put(rest, audio, first: silent, start: start, keep: keep)
                    let silence = [Float](repeating: 0, count: Int(Phonon.sampleRate) * part.pauseMillis / 1000)
                    try put(silence, audio, first: silent, start: start, keep: keep)
                    if silent, !silence.isEmpty { silent = false; await audible() }
                    continue
                }
                let sentence = part.text
                guard let parts = Self.parts(part.voice), parts.language == language else { throw PhononUnavailable() }
                if let samples = cache.samples(part.voice, sentence) {
                    let shaped = stretcher.push(shaper.push(samples))
                    try put(shaped, audio, first: silent, start: start, keep: keep)
                    if silent, !shaped.isEmpty { silent = false; await audible() }
                } else {
                    // Nur auf dem Synthese-Executor schalten, nie synchron im
                    // MainActor oder Audio-Callback.
                    if tts.voice != parts.voice { try await tts.setVoice(parts.voice) }
                    try check(audio)
                    var samples: [Float] = []
                    for try await chunk in tts.stream(SpeechText.spoken(sentence, language: language)) {
                        try check(audio)
                        let shaped = stretcher.push(shaper.push(chunk))
                        try put(shaped, audio, first: silent, start: start, keep: keep)
                        samples += chunk
                        // Erst melden, wenn wirklich Ton eingereiht ist (die Stille davor fällt weg).
                        if silent, !shaped.isEmpty { silent = false; await audible() }
                    }
                    // Ein Abbruch beendet den Strom auch ohne Fehler: Teilsätze nie aufheben.
                    try check(audio)
                    cache.store(part.voice, sentence, samples)
                }
            }
            // Satzende: Strecker leeren, damit die Markierung genau am Satzende sitzt.
            let rest = stretcher.push(shaper.finish(paragraphEnd: item.paragraphEnd)) + stretcher.flush()
            try put(rest, audio, first: silent, start: start, keep: keep)
            if silent, !rest.isEmpty { silent = false; await audible() }
            audio.mark()
        }
        await audio.wait(below: 1)
        try check(audio)
    }

    /// Reiht Ton ein. Der erste hörbare Ton (`first`, nicht leer) geht über `start`, falls vorhanden (Widerrufsgrenze, atomar mit
    /// der Erhöhung der Freigabe-Generation); Folgetöne des begonnenen Satzes ohne Prüfung: Ein begonnener Satz verstummt über
    /// `stop()` im MainActor. Weist `start` ab: `PhononStartRefused` (nichts eingereiht, kein Ton).
    nonisolated private static func put(_ samples: [Float], _ audio: PhononAudio, first: Bool, start: PhononStart?,
                                        keep: PhononStart?) throws {
        guard !samples.isEmpty else { _ = audio.enqueue(samples); return }
        // Folgestücke: durch die Wiedergabesperre (`keep`), ohne Marke; weist sie ab, endet die Äußerung ohne weiteren Ton.
        guard first else {
            if !audio.enqueue(samples, start: keep) { throw PhononStartRefused() }
            return
        }
        if !audio.enqueue(samples, start: start) { throw PhononStartRefused() }
    }

    /// Eine von außen beendete Wiedergabe beendet auch das Erzeugen.
    nonisolated private static func check(_ audio: PhononAudio) throws {
        try Task.checkCancellation()
        if audio.isStalled { throw PhononAudioStalled() }
        if audio.isStopped { throw CancellationError() }
    }

    func pause() { player?.pause() }
    func resume() -> Bool { player?.resume() ?? false }
    func setRate(_ rate: Float) { requestedRate = rate }
    func silence() { player?.shutdown(); player = nil }
    #else
    var modelAvailable: Bool { false }
    func refresh() async {}
    func stimmenAktualisieren() async {}
    func prepare(voice name: String, sentences: [String] = [], items: [PhononSentence] = []) {}
    func speak(_ text: String, voice name: String, rate: Float = 1, start: PhononStart? = nil, keep: PhononStart? = nil) async throws { throw PhononUnavailable() }
    func read(voice name: String, rate: Float, next: @escaping @MainActor () async -> PhononSentence?,
              audible: @escaping @MainActor () -> Void, played: @escaping @MainActor () -> Void,
              start: PhononStart? = nil, keep: PhononStart? = nil) async throws { throw PhononUnavailable() }
    func pause() {}
    func resume() -> Bool { false }
    func setRate(_ rate: Float) {}
    func silence() {}
    #endif
}
