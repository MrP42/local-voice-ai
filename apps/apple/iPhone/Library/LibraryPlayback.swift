import Foundation
import AVFoundation
import MediaPlayer

/// Was gerade in der Bibliothek läuft: Hörbuch oder Vorlesen. Es läuft immer
/// höchstens eines; der Mini-Player zeigt es auf Regal und Buchseite.
@MainActor
final class LibraryPlayback: NSObject, ObservableObject {
    static let shared = LibraryPlayback()

    enum Mode: Equatable { case audiobook, readAloud }

    @Published private(set) var book: LibraryBook?
    @Published private(set) var mode: Mode?
    @Published private(set) var playing = false
    @Published private(set) var positionMs: Double = 0
    @Published private(set) var totalMs: Double = 0
    @Published private(set) var trackIndex = 0
    @Published var rate: Float = 1.0 { didSet { applyRate() } }
    @Published private(set) var sleepEnds: Date?
    // Vorlesen
    @Published private(set) var paragraphs: [TextParagraph] = []
    @Published private(set) var paragraphIndex = 0
    @Published private(set) var wordRange: NSRange?
    @Published var voiceIdentifier: String? = UserDefaults.standard.string(forKey: "library.voice") {
        didSet { UserDefaults.standard.set(voiceIdentifier, forKey: "library.voice") }
    }
    @Published var error: String?
    /// Vollbild-Player sichtbar (Hören mit Mitlesen oder Vorlesen).
    @Published var presented = false

    private let library: PhoneLibrary
    private var player: AVPlayer?
    private var urls: [URL] = []
    private var tracks: [LibraryAudioTrack] = []
    private var timeObserver: Any?
    private var endObserver: NSObjectProtocol?
    private var lastSavedMs: Double = 0
    private var lastSaveAt = Date.distantPast
    private var sleepTimer: Timer?
    private let synth = AVSpeechSynthesizer()
    private var text = ""
    private var utteranceStart = 0
    private var remoteReady = false

    init(library: PhoneLibrary? = nil) {
        self.library = library ?? .shared
        super.init()
        synth.delegate = self
    }

    var title: String { book?.title ?? "" }
    var followMap: FollowMap? {
        guard let book, let audio = book.audio, !text.isEmpty else { return nil }
        return FollowMap(sync: audio.sync, textLength: Double(text.utf16.count), totalMs: totalMs)
    }

    // MARK: Hörbuch

    func openAudiobook(_ book: LibraryBook, autoplay: Bool = true) {
        if self.book?.id == book.id, mode == .audiobook { if autoplay && !playing { play() }; return }
        stop()
        let urls = library.audioURLs(book)
        guard !urls.isEmpty, let audio = book.audio else { error = "Das Hörbuch ist nicht auf dem iPhone."; return }
        self.book = book; mode = .audiobook
        self.urls = urls; tracks = Array(audio.allTracks.prefix(urls.count))
        totalMs = audio.totalDurationMs ?? tracks.reduce(0) { $0 + ($1.durationMs ?? 0) }
        text = library.text(book) ?? ""
        paragraphs = LibraryText.paragraphs(text)
        let start = library.record(library.audioKey(book))?.offset ?? 0
        lastSavedMs = start
        load(position: LibraryTracks.position(of: start, in: tracks))
        if autoplay { play() }
        setupRemote()
    }

    private func load(position: TrackPosition) {
        removeObservers()
        let index = min(max(0, position.index), urls.count - 1)
        trackIndex = index
        let item = AVPlayerItem(url: urls[index])
        if player == nil { player = AVPlayer() }
        player?.replaceCurrentItem(with: item)
        player?.seek(to: CMTime(seconds: position.offsetMs / 1000, preferredTimescale: 600), toleranceBefore: .zero, toleranceAfter: .zero)
        positionMs = LibraryTracks.total(TrackPosition(index: index, offsetMs: position.offsetMs), in: tracks)
        timeObserver = player?.addPeriodicTimeObserver(forInterval: CMTime(seconds: 0.5, preferredTimescale: 600), queue: .main) { [weak self] t in
            MainActor.assumeIsolated { self?.tick(t.seconds) }
        }
        endObserver = NotificationCenter.default.addObserver(forName: .AVPlayerItemDidPlayToEndTime, object: item, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.trackEnded() }
        }
    }

    private func tick(_ seconds: Double) {
        guard mode == .audiobook, seconds.isFinite else { return }
        positionMs = LibraryTracks.total(TrackPosition(index: trackIndex, offsetMs: seconds * 1000), in: tracks)
        if Date().timeIntervalSince(lastSaveAt) >= 10 { saveAudio() }
        updateNowPlaying()
    }

    private func trackEnded() {
        if trackIndex + 1 < urls.count {
            load(position: TrackPosition(index: trackIndex + 1, offsetMs: 0))
            if playing { player?.playImmediately(atRate: rate) }
        } else {
            playing = false
            saveAudio()
        }
    }

    func seek(toMs ms: Double) {
        guard mode == .audiobook else { return }
        let target = max(0, totalMs > 0 ? min(ms, totalMs) : ms)
        let pos = LibraryTracks.position(of: target, in: tracks)
        if pos.index != trackIndex { load(position: pos) }
        else { player?.seek(to: CMTime(seconds: pos.offsetMs / 1000, preferredTimescale: 600), toleranceBefore: .zero, toleranceAfter: .zero) }
        positionMs = target
        if playing { player?.playImmediately(atRate: rate) }
        saveAudio()
    }

    func skip(seconds: Double) { seek(toMs: positionMs + seconds * 1000) }

    private func saveAudio() {
        guard let book, mode == .audiobook else { return }
        let rec = LibraryProgress.recordPosition(library.record(library.audioKey(book)), to: positionMs, length: totalMs,
                                                 unit: .ms, mode: .heard, now: PhoneLibrary.nowMs())
        library.put(library.audioKey(book), rec)
        lastSavedMs = positionMs; lastSaveAt = Date()
    }

    // MARK: Vorlesen

    func openReadAloud(_ book: LibraryBook, autoplay: Bool = true, fromParagraph: Int? = nil) {
        if self.book?.id != book.id || mode != .readAloud {
            stop()
            guard let text = library.text(book) else { error = "Der Text ist nicht auf dem iPhone."; return }
            self.book = book; mode = .readAloud
            self.text = text
            paragraphs = LibraryText.paragraphs(text)
            let offset = library.record(library.textKey(book))?.offset ?? 0
            paragraphIndex = LibraryText.paragraphIndex(paragraphs, at: offset)
            totalMs = 0
            setupRemote()
        }
        if let fromParagraph { paragraphIndex = fromParagraph; if playing || autoplay { speakCurrent() }; return }
        if autoplay { play() }
    }

    private func speakCurrent() {
        guard paragraphs.indices.contains(paragraphIndex) else { playing = false; return }
        synth.stopSpeaking(at: .immediate)
        activateSession()
        let p = paragraphs[paragraphIndex]
        let u = AVSpeechUtterance(string: p.text)
        u.voice = voiceIdentifier.flatMap { AVSpeechSynthesisVoice(identifier: $0) } ?? Self.defaultVoice()
        u.rate = AVSpeechUtteranceDefaultSpeechRate * rate
        utteranceStart = p.start + (p.heading ? 2 : 0)
        wordRange = nil
        playing = true
        synth.speak(u)
        updateNowPlaying()
    }

    func jump(toParagraph index: Int) {
        guard paragraphs.indices.contains(index) else { return }
        if mode == .audiobook, let map = followMap { seek(toMs: map.ms(atChars: Double(paragraphs[index].start))); return }
        paragraphIndex = index
        if playing { speakCurrent() } else { saveText(at: paragraphs[index].start) }
    }

    func step(paragraphs delta: Int) { jump(toParagraph: min(max(0, paragraphIndex + delta), paragraphs.count - 1)) }

    private func saveText(at chars: Int) {
        guard let book else { return }
        let rec = LibraryProgress.recordPosition(library.record(library.textKey(book)), to: Double(chars), length: Double(text.utf16.count),
                                                 unit: .chars, mode: .heard, now: PhoneLibrary.nowMs())
        library.put(library.textKey(book), rec)
    }

    static func defaultVoice() -> AVSpeechSynthesisVoice? {
        let german = AVSpeechSynthesisVoice.speechVoices().filter { $0.language.hasPrefix("de") }
        return german.max { $0.quality.rawValue < $1.quality.rawValue } ?? AVSpeechSynthesisVoice(language: "de-DE")
    }

    static var germanVoices: [AVSpeechSynthesisVoice] {
        AVSpeechSynthesisVoice.speechVoices().filter { $0.language.hasPrefix("de") }
            .sorted { ($0.quality.rawValue, $0.name) > ($1.quality.rawValue, $1.name) }
    }

    // MARK: Gemeinsam

    func play() {
        activateSession()
        switch mode {
        case .audiobook: player?.playImmediately(atRate: rate); playing = true
        case .readAloud:
            if synth.isPaused { synth.continueSpeaking(); playing = true } else { speakCurrent() }
        case nil: return
        }
        updateNowPlaying()
    }

    func pause() {
        switch mode {
        case .audiobook: player?.pause(); saveAudio()
        case .readAloud: synth.pauseSpeaking(at: .word)
        case nil: break
        }
        playing = false
        library.flush()
        updateNowPlaying()
    }

    func toggle() { playing ? pause() : play() }

    func stop() {
        if mode == .audiobook { player?.pause(); saveAudio() }
        if mode == .readAloud { synth.stopSpeaking(at: .immediate) }
        removeObservers()
        player?.replaceCurrentItem(with: nil)
        playing = false; book = nil; mode = nil; wordRange = nil
        cancelSleep()
        library.flush()
        MPNowPlayingInfoCenter.default().nowPlayingInfo = nil
    }

    func scene(active: Bool) { if !active { if mode == .audiobook { saveAudio() }; library.flush() } }

    private func applyRate() {
        if mode == .audiobook, playing { player?.rate = rate }
        if mode == .readAloud, playing { speakCurrent() }
    }

    func sleep(minutes: Int?) {
        cancelSleep()
        guard let minutes else { return }
        sleepEnds = Date().addingTimeInterval(Double(minutes) * 60)
        sleepTimer = Timer.scheduledTimer(withTimeInterval: Double(minutes) * 60, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated { self?.pause(); self?.sleepEnds = nil }
        }
    }
    private func cancelSleep() { sleepTimer?.invalidate(); sleepTimer = nil; sleepEnds = nil }

    private func removeObservers() {
        if let timeObserver { player?.removeTimeObserver(timeObserver) }
        timeObserver = nil
        if let endObserver { NotificationCenter.default.removeObserver(endObserver) }
        endObserver = nil
    }

    private func activateSession() {
        do {
            try AVAudioSession.sharedInstance().setCategory(.playback, mode: .spokenAudio)
            try AVAudioSession.sharedInstance().setActive(true)
        } catch { self.error = "Wiedergabe konnte nicht starten." }
    }

    private func setupRemote() {
        guard !remoteReady else { return }
        remoteReady = true
        let c = MPRemoteCommandCenter.shared()
        c.playCommand.addTarget { [weak self] _ in MainActor.assumeIsolated { self?.play() }; return .success }
        c.pauseCommand.addTarget { [weak self] _ in MainActor.assumeIsolated { self?.pause() }; return .success }
        c.togglePlayPauseCommand.addTarget { [weak self] _ in MainActor.assumeIsolated { self?.toggle() }; return .success }
        c.skipBackwardCommand.preferredIntervals = [15]
        c.skipForwardCommand.preferredIntervals = [15]
        c.skipBackwardCommand.addTarget { [weak self] _ in
            MainActor.assumeIsolated { if self?.mode == .readAloud { self?.step(paragraphs: -1) } else { self?.skip(seconds: -15) } }; return .success }
        c.skipForwardCommand.addTarget { [weak self] _ in
            MainActor.assumeIsolated { if self?.mode == .readAloud { self?.step(paragraphs: 1) } else { self?.skip(seconds: 15) } }; return .success }
        c.changePlaybackPositionCommand.addTarget { [weak self] event in
            guard let e = event as? MPChangePlaybackPositionCommandEvent else { return .commandFailed }
            MainActor.assumeIsolated { self?.seek(toMs: e.positionTime * 1000) }; return .success
        }
    }

    private func updateNowPlaying() {
        guard let book else { return }
        var info: [String: Any] = [MPMediaItemPropertyTitle: book.title,
                                   MPMediaItemPropertyArtist: book.authors.joined(separator: ", "),
                                   MPNowPlayingInfoPropertyPlaybackRate: playing ? Double(rate) : 0]
        if mode == .audiobook {
            info[MPMediaItemPropertyPlaybackDuration] = totalMs / 1000
            info[MPNowPlayingInfoPropertyElapsedPlaybackTime] = positionMs / 1000
        }
        MPNowPlayingInfoCenter.default().nowPlayingInfo = info
    }
}

extension LibraryPlayback: AVSpeechSynthesizerDelegate {
    nonisolated func speechSynthesizer(_ s: AVSpeechSynthesizer, willSpeakRangeOfSpeechString range: NSRange, utterance: AVSpeechUtterance) {
        Task { @MainActor in self.wordRange = range }
    }
    nonisolated func speechSynthesizer(_ s: AVSpeechSynthesizer, didFinish utterance: AVSpeechUtterance) {
        Task { @MainActor in
            guard self.mode == .readAloud, self.playing else { return }
            let finished = self.paragraphIndex
            if self.paragraphs.indices.contains(finished) { self.saveText(at: self.paragraphs[finished].end) }
            if finished + 1 < self.paragraphs.count { self.paragraphIndex = finished + 1; self.speakCurrent() }
            else { self.playing = false; self.wordRange = nil; self.library.flush() }
        }
    }
}
