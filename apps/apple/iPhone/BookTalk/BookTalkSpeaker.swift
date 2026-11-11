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
    var onSentenceStarted: ((String) -> Void)?
    var onSentenceFinished: ((String) -> Void)?

    private(set) var isActive = false
    private var queue: [String] = []
    private var generation = 0
    private var inputFinished = true
    private var task: Task<Void, Never>?
    private var language = BookTalkLanguage.de
    private var phononFailed = false
    private let system = SystemSpeech()

    /// Neue Antwort: alles Alte verwerfen.
    func begin(language: BookTalkLanguage) {
        stop()
        self.language = language
        inputFinished = false
    }

    func enqueue(_ speech: String) {
        let text = speech.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, !inputFinished else { return }
        queue.append(text)
        pump()
    }

    /// Es kommen keine Sätze mehr; ist alles gesprochen, meldet `onIdle`.
    func endOfInput() {
        inputFinished = true
        if task == nil { finishIfDrained() }
    }

    /// Sofort still: Warteschlange leer, Ausgabe aus, Vorsynthetisiertes verworfen. `onIdle` kommt nicht.
    func stop() {
        generation += 1
        queue.removeAll()
        inputFinished = true
        task?.cancel()
        task = nil
        isActive = false
        PhononEngine.shared.silence()
        system.stop()
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
        let g = generation
        isActive = true
        task = Task { [weak self] in
            if self?.conversationMode != true { Self.activatePlayback() }
            var announced = false
            while let self, g == self.generation, !self.queue.isEmpty {
                let text = self.queue.removeFirst()
                if !announced { announced = true; self.onAudible?() }
                self.onSentenceStarted?(text)
                let played = await self.speak(text, generation: g)
                if played, g == self.generation { self.onSentenceFinished?(text) }
            }
            guard let self, g == self.generation else { return }
            self.task = nil
            self.finishIfDrained()
        }
    }

    /// Nach dem Zuhören steht die Sitzung auf `.playAndRecord`/`.voiceChat` oder ist aus: vor jeder Antwort
    /// wieder die Wiedergabe-Kategorie, wie sie auch das Vorlesen nutzt.
    private static func activatePlayback() {
        let session = AVAudioSession.sharedInstance()
        try? session.setCategory(.playback, mode: .spokenAudio)
        try? session.setActive(true)
    }

    private func finishIfDrained() {
        guard queue.isEmpty, inputFinished else { return }
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
    private func speak(_ text: String, generation g: Int) async -> Bool {
        if let name = phononName(for: language) {
            do {
                try await PhononEngine.shared.speak(text, voice: name, rate: 1)
                return true
            } catch is CancellationError {
                return false
            } catch is PhononAudioStalled {
                return false
            } catch {
                // Nicht verstummen: für diese Sitzung bei der Systemstimme bleiben.
                phononFailed = true
            }
        }
        guard g == generation else { return false }
        return await system.speak(text, language: language, voiceID: voiceID)
    }
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

    /// `true` nur, wenn die Äußerung zu Ende gesprochen wurde (nicht bei `stop()`, Ersetzen oder `didCancel`).
    func speak(_ text: String, language: BookTalkLanguage, voiceID: String? = nil) async -> Bool {
        let utterance = AVSpeechUtterance(string: text)
        utterance.voice = Self.voice(language: language, id: voiceID)
        return await withCheckedContinuation { (continuation: CheckedContinuation<Bool, Never>) in
            waiting?.resume(returning: false)
            waiting = continuation
            current = ObjectIdentifier(utterance)
            synth.speak(utterance)
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
