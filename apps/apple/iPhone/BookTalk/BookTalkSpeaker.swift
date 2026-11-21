import Foundation
import AVFoundation

/// Sprechen der Antwort (Epic #307, G5): `sentence.speech` je Ereignis sofort an die Stimme, Stimme Anton
/// (Phonon, unabhängig von der Vorlesestimme), ohne Phonon die beste deutsche Systemstimme. Satzweise Warteschlange;
/// `stop()` leert sie, hält die Ausgabe an und verwirft Vorsynthetisiertes (Barge-in, Bereichswechsel, Hintergrund).
@MainActor
final class BookTalkSpeaker {
    static let voiceName = "Anton"

    /// Warteschlange leer, alles gesprochen (nur nach `endOfInput()`).
    var onIdle: (() -> Void)?
    /// Erster Satz klingt (Zustand „Anton spricht“).
    var onAudible: (() -> Void)?
    /// Gesprächsmodus (Paket A, „Sprechen“): die Sitzung gehört dem durchgehenden Zuhören (`.playAndRecord`/`.voiceChat`),
    /// deshalb wird sie vor der Antwort nicht auf Wiedergabe umgestellt. Standard aus = Verhalten des Buch-Gesprächs.
    var conversationMode = false
    /// Gewählte Stimme (Kennung wie `VoiceModel.selectedVoiceID`): Phonon, sonst Systemstimme dieser Kennung; leer =
    /// Standardstimme des Systems. `nil` (Standard) = Anton wie bisher.
    var voiceID: String?
    /// Satzbeginn und -ende der Wiedergabe (für Echo-Wächter und Verlauf); das Ende kommt nur, wenn der Satz wirklich zu
    /// Ende erklang, nicht nach `stop()`, nicht bei hängendem Audio (`PhononAudioStalled`) und nicht bei abgebrochener Systemstimme.
    /// Der Beginn wird NACH dem tatsächlichen Start gemeldet (Messung, kein Commit): Wer eine Marke setzt, tut das im `start` des
    /// Satzes (`enqueue(_:generation:start:)`), im selben Lock wie die Erhöhung der Generation.
    var onSentenceStarted: ((String) -> Void)?
    var onSentenceFinished: ((String) -> Void)?
    /// Gesprächsmodus („Sprechen“): Beginn und Ende JEDER Einheit der Warteschlange, auch von Ansagen und Überbrückungen. Der Beginn
    /// kommt NACH dem tatsächlichen Klangbeginn (wie `onSentenceStarted`, auf dem MainActor, nie im Lock des Starts); das Ende immer,
    /// wenn eine begonnene Einheit nicht mehr klingt: zu Ende gespielt, abgebrochen (hängendes Audio, abgebrochene Systemstimme)
    /// oder durch `stop()`. Nie ein Ende ohne Beginn und nie zwei Enden zu einem Beginn. Bei einem Satz kommt `onSentenceFinished`
    /// vor `onUnitEnded`. Ein `stop()` meldet das Ende SYNCHRON (der Aufrufer von `stop()` ist dann noch mittendrin).
    var onUnitStarted: (() -> Void)?
    var onUnitEnded: (() -> Void)?
    /// Wiedergabesperre des Gesprächsmodus („verworfen ab Generation g“, vom Kern im Lock der Freigabe vorgerückt). Ein Satz einer
    /// verworfenen Generation startet nie: Der TATSÄCHLICHE Start (Systemstimme: `synth.speak`, Phonon: Einreihen des ersten
    /// hörbaren Tons) läuft als Body von `gate.start(generation, body)`, also im selben Lock, in dem die Generation steigt; nach
    /// der Rückkehr einer Erhöhung beginnt kein Satz der alten Generation mehr. Davor sind nur Vorprüfungen (`permits`: früh
    /// aussteigen, ohne Start): vor der Übernahme aus der Warteschlange und unmittelbar vor der Stimme. `nil` (Standard,
    /// Buch-Gespräch): keine Sperre, Verhalten wie bisher.
    /// Grenze: Dieser Speaker gehört dem MainActor und lässt sich von anderen Threads nicht stoppen; ein Satz, dessen Klang
    /// schon begonnen hat, verstummt erst mit `stop()` (Controller, MainActor, einige Millisekunden), nicht schon beim Auslöser.
    var gate: AssistantPlaybackGate?

    private(set) var isActive = false
    /// Ein Satz der Warteschlange. `start`: der Start dieses Satzes als atomarer Body (Gesprächsmodus: `commitStart` des Kerns,
    /// Prüfung der Generation, tatsächlicher Start und Ergebniszeile im selben Lock); `nil`: die Wiedergabesperre `gate` (oder gar
    /// keine Prüfung). `failed`: der Start ließ sich nicht auslösen, obwohl nichts widerrufen war (Marke „fehlgeschlagen“).
    private struct Item {
        let text: String
        let generation: Int
        let start: PhononStart?
        let failed: (@MainActor () -> Void)?
    }

    private var queue: [Item] = []
    private var announced = false
    private var generation = 0
    private var inputFinished = true
    private var task: Task<Void, Never>?
    private var language = BookTalkLanguage.de
    private var phononFailed = false
    private let system = SystemSpeech()
    /// Haltezustand (Gesprächsmodus, `hold()`/`release()`); eigene Klasse, weil der Start einer Einheit ihn auf beliebigem Thread liest.
    private let holdState = HoldState()
    /// Eine Einheit klingt (Beginn gemeldet, Ende noch nicht): Grundlage dafür, dass jedem `onUnitStarted` genau ein `onUnitEnded` folgt.
    private var soundingUnit = false

    /// Neue Antwort: alles Alte verwerfen (auch das Halten).
    func begin(language: BookTalkLanguage) {
        stop()
        self.language = language
        inputFinished = false
        announced = false
    }

    /// Gesprächsmodus: Der Nutzer spricht. Ab jetzt beginnt nichts Neues. Eine Einheit, die schon klingt, klingt weiter (das
    /// entscheidet die Richtlinie vorher: sie meldet Halten nur, wenn nichts klingt). Noch nicht Begonnenes wird zurückgenommen:
    /// Die Warteschlange gibt nichts mehr heraus (kein Warten in einer Schleife: die Ausgabe endet, `release()` stößt sie wieder an),
    /// und eine Einheit, die schon in der Synthese ist, aber noch nicht klingt, wird am Start abgewiesen (siehe `start`): kein Aufruf
    /// des Starts im Kern (kein `commitStart`, keine Marke), kein Ton. Sie wird danach wieder vorn eingereiht und später neu gesprochen.
    /// Ohne `hold()` bleibt das Verhalten unverändert (Buch-Gespräch).
    func hold() { holdState.hold() }

    /// Gesprächsmodus: Halten beenden. Die Ausgabe läuft mit dem, was wartet, weiter; ist nichts mehr da und kam das Ende der Eingabe
    /// schon, meldet `onIdle`. Ohne vorheriges `hold()` wirkungslos.
    func release() {
        guard holdState.release() else { return }
        // Läuft die Ausgabe noch (eine Einheit in der Synthese), reiht sie sich am Ende von `pump()` selbst wieder ein.
        guard task == nil else { return }
        if queue.isEmpty { finishIfDrained() } else { pump() }
    }

    /// `generation`: Freigabe-Generation des Satzes (nur mit `gate`; sonst ohne Bedeutung). `start`: der atomare Start dieses
    /// Satzes (siehe `Item`); `failed`: wird gerufen, wenn der Start sich nicht auslösen ließ, ohne dass die Freigabe widerrufen war.
    func enqueue(_ speech: String, generation: Int = 0, start: PhononStart? = nil, failed: (@MainActor () -> Void)? = nil) {
        let text = speech.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, !inputFinished else { return }
        queue.append(Item(text: text, generation: generation, start: start, failed: failed))
        pump()
    }

    /// Es kommen keine Sätze mehr; ist alles gesprochen, meldet `onIdle`.
    func endOfInput() {
        inputFinished = true
        if task == nil { finishIfDrained() }
    }

    /// Sofort still: Warteschlange leer, Ausgabe aus, Vorsynthetisiertes verworfen, Halten aufgehoben. `onIdle` kommt nicht. Klang eine
    /// Einheit, kommt `onUnitEnded` (zuletzt, nach dem Aufräumen).
    func stop() {
        generation += 1
        queue.removeAll()
        inputFinished = true
        task?.cancel()
        task = nil
        isActive = false
        PhononEngine.shared.silence()
        system.stop()
        // Das Halten erst NACH dem Stillstellen der Stimme löschen: Ein Start, der auf einem anderen Thread gerade jetzt ankommt, wird
        // so noch abgewiesen (oder trifft auf eine stillgelegte Stimme), statt zwischen Löschen und Stillstellen durchzurutschen.
        holdState.reset()
        endUnit()
    }

    /// Modell und Stimme vorwärmen, damit der erste Satz schnell klingt.
    func prepare() {
        Task { [weak self] in
            await PhononEngine.shared.refresh()
            guard let name = self?.phononName(for: .de) else { return }
            PhononEngine.shared.prepare(voice: name)
        }
    }

    // MARK: Ablauf

    private func pump() {
        guard task == nil else { return }
        isActive = true
        // Gehalten: nichts Neues aus der Warteschlange; `release()` stößt die Ausgabe wieder an.
        guard !holdState.isHeld else { return }
        let g = generation
        task = Task { [weak self] in
            if self?.conversationMode != true { Self.activatePlayback() }
            while let self, g == self.generation, !self.queue.isEmpty, !self.holdState.isHeld {
                let item = self.queue.removeFirst()
                let text = item.text
                // Verworfene Generation: der Satz startet nicht (die Sperre steht, sobald die Erhöhung zurückgekehrt ist). Die Ausgabe
                // endet still (kein `onIdle`): Der Beobachter des Controllers räumt auf und meldet den Widerruf. Der Kern schreibt die
                // Marke „verworfen“ erst im `start`; hier wird nur früh ausgestiegen.
                guard self.permits(item.generation) else { self.reject(item); self.stop(); break }
                let begun = StartFlag()
                self.holdState.clearWithheld()
                let played = await self.speak(item, generation: g, begun: begun)
                // Wurde der Start dieser Einheit wegen des Haltens abgewiesen? (Merker wird hier verbraucht, nie mitgeschleppt.)
                let withheld = self.holdState.takeWithheld()
                // `stop()` (auch aus einem Rückruf) hat aufgeräumt, einschließlich des Endes einer klingenden Einheit.
                guard g == self.generation else { break }
                if played {
                    self.onSentenceFinished?(text)
                    guard g == self.generation else { break }
                }
                // Eine begonnene Einheit klingt nicht mehr (fertig oder abgebrochen); nach dem Ende des Satzes. Der Rückruf kann die
                // Antwort abbrechen (`stop()`): danach nichts mehr anfassen.
                self.endUnit()
                guard g == self.generation else { break }
                // Start abgewiesen (Widerruf): nichts begann, der Satz zählt als nicht gesprochen; die Ausgabe endet still.
                if !played, !self.permits(item.generation) { self.stop(); break }
                // Zurückgehalten (Nutzer spricht): nichts klang, nichts ist markiert, kein Fehler und kein `stop()`. Die Einheit kommt
                // wieder nach vorn; die Ausgabe endet hier und läuft mit `release()` weiter.
                if !played, withheld { self.queue.insert(item, at: 0); break }
                // Kein Start, aber auch kein Widerruf (Audio steht, Strom abgebrochen): „fehlgeschlagen“, nie „ausgeführt“.
                if !played, !begun.value { item.failed?() }
            }
            guard let self, g == self.generation else { return }
            self.task = nil
            // Gehalten: weder „alles gesprochen“ melden noch weitermachen; `release()` entscheidet. Sonst: nichts mehr da (fertig) oder
            // ein `release()` kam schon, während die zurückgehaltene Einheit noch zurückkehrte (dann läuft sie jetzt).
            guard !self.holdState.isHeld else { return }
            if self.queue.isEmpty { self.finishIfDrained() } else { self.pump() }
        }
    }

    /// Nach dem Zuhören steht die Sitzung auf `.playAndRecord`/`.voiceChat` oder ist aus: vor jeder Antwort
    /// wieder die Wiedergabe-Kategorie, wie sie auch das Vorlesen nutzt.
    private static func activatePlayback() {
        let session = AVAudioSession.sharedInstance()
        try? session.setCategory(.playback, mode: .spokenAudio)
        try? session.setActive(true)
    }

    /// Vorprüfung ohne Start (billig, früh aussteigen). Entscheidend ist `start`.
    private func permits(_ sentenceGeneration: Int) -> Bool { gate?.permits(sentenceGeneration) ?? true }

    /// Der tatsächliche Start eines Satzes als atomarer Body: der Start des Satzes (Kern, mit Marke) oder die Wiedergabesperre
    /// (ohne Marke) oder, ohne beides, einfach der Body. Nach einem Start wird der Beginn gemeldet (Messung auf dem MainActor,
    /// nie im Lock) und gemerkt.
    private func start(_ item: Item, generation g: Int, begun: StartFlag) -> PhononStart {
        var base: PhononStart? = item.start
        if base == nil, let gate {
            let sentenceGeneration = item.generation
            base = { body in gate.start(sentenceGeneration, body) }
        }
        let chosen = base
        let text = item.text
        let holder = holdState
        return { [weak self] body in
            // Gehalten (Nutzer spricht): VOR dem Start im Kern. `chosen` wird nicht gerufen (kein `commitStart`, keine Marke, kein Ton),
            // die Einheit merkt „zurückgehalten“ und meldet `false`; `pump()` reiht sie wieder vorn ein, ohne `failed` und ohne `stop()`.
            // Der Lock des Halters ist ein Blatt: Er wird nie gehalten, während der Lock des Kerns genommen wird. Ein Halten, das genau
            // jetzt beginnt, kommt zu spät; die Richtlinie fängt diesen Wettlauf über `unitStarted` (`releaseAnswer`) auf.
            if holder.refuseIfHeld() { return false }
            var started = true
            if let chosen { started = chosen(body) } else { body() }
            guard started else { return false }
            begun.set()
            DispatchQueue.main.async { MainActor.assumeIsolated { self?.noteStarted(text, generation: g) } }
            return true
        }
    }

    /// Ein Satz einer verworfenen Generation startet nicht. Der Kern schreibt dazu die Marke „verworfen“ mit der vorgefundenen
    /// Generation: Sein `start` läuft mit einem leeren Body, der nie gebraucht wird (die Generation ist veraltet, `permits` ist
    /// monoton). Ohne eigenen Start des Satzes (Buch-Gespräch) geschieht nichts.
    private func reject(_ item: Item) { _ = item.start?({}) }

    /// Der erste hörbare Satz meldet „klingt“, jeder Satz seinen Beginn; eine Meldung nach `stop()` zählt nicht mehr.
    private func noteStarted(_ text: String, generation g: Int) {
        guard g == generation else { return }
        if !announced { announced = true; onAudible?() }
        onSentenceStarted?(text)
        // Der Rückruf des Satzes kann die Ausgabe abgebrochen haben (`stop()`): dann gab es für diese Einheit keinen Beginn, also auch
        // kein Ende zu melden.
        guard g == generation else { return }
        soundingUnit = true
        onUnitStarted?()
    }

    /// Eine begonnene Einheit klingt nicht mehr: genau einmal `onUnitEnded` zu jedem gemeldeten `onUnitStarted`.
    private func endUnit() {
        guard soundingUnit else { return }
        soundingUnit = false
        onUnitEnded?()
    }

    /// Folgetöne eines begonnenen Satzes (Synthese in Stücken, nur auf der Audioebene): jedes Stück geht durch dieselbe
    /// Wiedergabesperre wie der Start, damit nach der Rückkehr einer Erhöhung kein Stück der alten Generation mehr eingereiht wird.
    private func keep(_ sentenceGeneration: Int) -> PhononStart? {
        guard let gate else { return nil }
        return { body in gate.start(sentenceGeneration, body) }
    }

    private func finishIfDrained() {
        // Gehalten ist nicht „alles gesprochen“: `release()` meldet das Ende, wenn nichts mehr wartet.
        guard queue.isEmpty, inputFinished, !holdState.isHeld else { return }
        isActive = false
        onIdle?()
    }

    private func phononName(for language: BookTalkLanguage) -> String? {
        guard !phononFailed else { return nil }
        let code = language == .de ? "de" : "en"
        guard let voices = PhononEngine.shared.voices[code], !voices.isEmpty else { return nil }
        if let voiceID {
            // Gewählte Stimme: Phonon nur, wenn sie zur Antwortsprache passt und vorhanden ist; eine Systemstimme (oder
            // keine Wahl) bleibt bei der Systemstimme. Eine Phonon-Stimme der anderen Sprache fällt auf die Vorgabe zurück.
            guard let name = PhononEngine.voiceName(voiceID) else { return nil }
            if let parts = PhononEngine.parts(name), parts.language == code, voices.contains(parts.voice) { return name }
        }
        if language == .de { return voices.contains(Self.voiceName) ? "de:\(Self.voiceName)" : nil }
        return "en:\(voices[0])"
    }

    /// `true`, wenn der Satz wirklich bis zum Ende erklang; `false` bei Abbruch oder hängendem Audio (Satz zählt als nicht gesprochen).
    private func speak(_ item: Item, generation g: Int, begun: StartFlag) async -> Bool {
        let text = item.text
        let sentenceGeneration = item.generation
        // Letzte Prüfung unmittelbar vor dem Start der Stimme.
        guard g == generation else { return false }
        guard permits(sentenceGeneration) else { reject(item); return false }
        if let name = phononName(for: language) {
            do {
                try await PhononEngine.shared.speak(text, voice: name, rate: 1, start: start(item, generation: g, begun: begun),
                                                    keep: keep(sentenceGeneration))
                return true
            } catch is CancellationError {
                return false
            } catch is PhononStartRefused {
                return false   // widerrufen: kein Ton, kein Rückfall auf die Systemstimme
            } catch is PhononAudioStalled {
                return false
            } catch {
                // Nicht verstummen: für diese Sitzung bei der Systemstimme bleiben.
                phononFailed = true
            }
        }
        guard g == generation else { return false }
        guard permits(sentenceGeneration) else { reject(item); return false }
        return await system.speak(text, language: language, voiceID: voiceID, start: start(item, generation: g, begun: begun))
    }
}

/// Haltezustand des Sprechers (Gesprächsmodus). Gesetzt und gelesen auf dem MainActor, gelesen und „zurückgehalten“ gemerkt aber auch
/// im Start einer Einheit auf beliebigem Thread (der unter dem Lock des Kerns läuft): eigener kurzer Lock, nur `withLock`, nie I/O,
/// nie ein Rückruf, und nie gehalten, während ein anderer Lock genommen wird (Blatt in der Lockordnung).
private final class HoldState: @unchecked Sendable {
    private let lock = NSLock()
    private var held = false
    private var withheld = false

    var isHeld: Bool { lock.withLock { held } }

    func hold() { lock.withLock { held = true } }

    /// Beendet das Halten; `true`, wenn es gehalten war.
    func release() -> Bool { lock.withLock { let was = held; held = false; return was } }

    /// Aus dem Start einer Einheit: ist gehalten, merkt es sich „zurückgehalten“ im selben Zug (atomar mit der Prüfung) und meldet `true`.
    func refuseIfHeld() -> Bool {
        lock.withLock {
            if held { withheld = true }
            return held
        }
    }

    func clearWithheld() { lock.withLock { withheld = false } }

    /// Liest und löscht den Merker „zurückgehalten“.
    func takeWithheld() -> Bool { lock.withLock { let was = withheld; withheld = false; return was } }

    /// `stop()`/`begin()`: das Halten ist aufgehoben, nichts bleibt zurück.
    func reset() { lock.withLock { held = false; withheld = false } }
}

/// Merkt, ob der tatsächliche Start eines Satzes lief (gesetzt im Lock des Starts, gelesen auf dem MainActor).
private final class StartFlag: @unchecked Sendable {
    private let lock = NSLock()
    private var started = false
    func set() { lock.lock(); started = true; lock.unlock() }
    var value: Bool { lock.lock(); defer { lock.unlock() }; return started }
}

/// Systemstimme als Rückfall (kein Phonon-Modell, Simulator, Fehler). Beendet jede Äußerung genau einmal.
@MainActor
private final class SystemSpeech: NSObject, AVSpeechSynthesizerDelegate {
    private let synth = AVSpeechSynthesizer()
    private var waiting: CheckedContinuation<Bool, Never>?
    /// Nur Rückmeldungen der laufenden Äußerung zählen (eine verspätete `didCancel` darf die nächste nicht beenden).
    private var current: ObjectIdentifier?

    override init() {
        super.init()
        synth.delegate = self
    }

    /// `true` nur, wenn die Äußerung zu Ende gesprochen wurde (nicht bei `stop()`, Ersetzen oder `didCancel`). `start` (optional):
    /// der tatsächliche Start (Übernahme der Äußerung und `synth.speak`) läuft als Body der atomaren Prüfung; `false` von
    /// `start` = nichts begann, Ergebnis `false`.
    func speak(_ text: String, language: BookTalkLanguage, voiceID: String? = nil, start: PhononStart? = nil) async -> Bool {
        let utterance = AVSpeechUtterance(string: text)
        utterance.voice = Self.voice(language: language, id: voiceID)
        return await withCheckedContinuation { (continuation: CheckedContinuation<Bool, Never>) in
            let begin = {
                self.waiting?.resume(returning: false)
                self.waiting = continuation
                self.current = ObjectIdentifier(utterance)
                self.synth.speak(utterance)
            }
            guard let start else { begin(); return }
            if !start(begin) { continuation.resume(returning: false) }
        }
    }

    /// Gewählte Systemstimme, wenn sie zur Sprache passt; sonst die Vorgabe wie bisher.
    private static func voice(language: BookTalkLanguage, id: String?) -> AVSpeechSynthesisVoice? {
        if let id, !id.isEmpty, PhononEngine.voiceName(id) == nil, let chosen = AVSpeechSynthesisVoice(identifier: id),
           chosen.language.lowercased().hasPrefix(language == .de ? "de" : "en") {
            return chosen
        }
        return language == .de ? LibraryPlayback.defaultVoice() : AVSpeechSynthesisVoice(language: "en-US")
    }

    func stop() {
        current = nil
        synth.stopSpeaking(at: .immediate)
        waiting?.resume(returning: false)
        waiting = nil
    }

    private func finished(_ id: ObjectIdentifier, completed: Bool) {
        guard current == id else { return }
        current = nil
        waiting?.resume(returning: completed)
        waiting = nil
    }

    nonisolated func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didFinish utterance: AVSpeechUtterance) {
        let id = ObjectIdentifier(utterance)
        Task { @MainActor in self.finished(id, completed: true) }
    }

    nonisolated func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didCancel utterance: AVSpeechUtterance) {
        let id = ObjectIdentifier(utterance)
        Task { @MainActor in self.finished(id, completed: false) }
    }
}
