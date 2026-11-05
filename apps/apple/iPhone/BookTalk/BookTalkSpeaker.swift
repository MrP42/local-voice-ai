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
            Self.activatePlayback()
            var announced = false
            while let self, g == self.generation, !self.queue.isEmpty {
                let text = self.queue.removeFirst()
                if !announced { announced = true; self.onAudible?() }
                await self.speak(text, generation: g)
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
        if language == .de { return voices.contains(Self.voiceName) ? "de:\(Self.voiceName)" : nil }
        return "en:\(voices[0])"
    }

    private func speak(_ text: String, generation g: Int) async {
        if let name = phononName(for: language) {
            do {
                try await PhononEngine.shared.speak(text, voice: name, rate: 1)
                return
            } catch is CancellationError {
                return
            } catch is PhononAudioStalled {
                return
            } catch {
                // Nicht verstummen: für diese Sitzung bei der Systemstimme bleiben.
                phononFailed = true
            }
        }
        guard g == generation else { return }
        await system.speak(text, language: language)
    }
}

/// Systemstimme als Rückfall (kein Phonon-Modell, Simulator, Fehler). Beendet jede Äußerung genau einmal.
@MainActor
private final class SystemSpeech: NSObject, AVSpeechSynthesizerDelegate {
    private let synth = AVSpeechSynthesizer()
    private var waiting: CheckedContinuation<Void, Never>?
    /// Nur Rückmeldungen der laufenden Äußerung zählen (eine verspätete `didCancel` darf die nächste nicht beenden).
    private var current: ObjectIdentifier?

    override init() {
        super.init()
        synth.delegate = self
    }

    func speak(_ text: String, language: BookTalkLanguage) async {
        let utterance = AVSpeechUtterance(string: text)
        utterance.voice = language == .de ? LibraryPlayback.defaultVoice() : AVSpeechSynthesisVoice(language: "en-US")
        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            waiting?.resume()
            waiting = continuation
            current = ObjectIdentifier(utterance)
            synth.speak(utterance)
        }
    }

    func stop() {
        current = nil
        synth.stopSpeaking(at: .immediate)
        waiting?.resume()
        waiting = nil
    }

    private func finished(_ id: ObjectIdentifier) {
        guard current == id else { return }
        current = nil
        waiting?.resume()
        waiting = nil
    }

    nonisolated func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didFinish utterance: AVSpeechUtterance) {
        let id = ObjectIdentifier(utterance)
        Task { @MainActor in self.finished(id) }
    }

    nonisolated func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didCancel utterance: AVSpeechUtterance) {
        let id = ObjectIdentifier(utterance)
        Task { @MainActor in self.finished(id) }
    }
}
