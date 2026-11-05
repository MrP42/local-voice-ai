import Foundation
import AVFoundation
import MediaPlayer
import UIKit

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
    /// Schlaftimer „Ende des Kapitels“: Hörposition, an der angehalten wird.
    @Published private(set) var sleepAtMs: Double?
    /// Angaben und Kapitel aus den Hörbuchdateien (Cache `hoerbuch-info.json`).
    @Published private(set) var info: AudiobookInfo?
    // Vorlesen
    @Published private(set) var paragraphs: [TextParagraph] = []
    @Published private(set) var paragraphIndex = 0
    @Published private(set) var wordRange: NSRange?
    /// Dieselbe Stimme wie in den Einstellungen (`VoiceModel.selectedVoiceID`).
    @Published var voiceIdentifier: String? = UserDefaults.standard.string(forKey: "selectedVoiceID")
        ?? UserDefaults.standard.string(forKey: "library.voice") {
        didSet { if VoiceModel.shared.selectedVoiceID != (voiceIdentifier ?? "") { VoiceModel.shared.selectedVoiceID = voiceIdentifier ?? "" } }
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
    private var remoteReady = false
    /// Gerade gesprochene Äußerung; Rückmeldungen älterer werden verworfen.
    private var currentUtterance: AVSpeechUtterance?
    /// Absatz, in dem das Vorlesen pausiert wurde; nach einem Sprung neu sprechen.
    private var pausedParagraph: Int?
    private var phononTask: Task<Void, Never>?
    private var phononFailed = false
    private var phononPaused = false
    /// Erst nach abgeschlossenem Sprung an die gespeicherte Stelle sichern.
    private var seekSettled = true
    private var lastTextSave = Date.distantPast
    private var sessionObservers: [NSObjectProtocol] = []

    init(library: PhoneLibrary? = nil) {
        self.library = library ?? .shared
        super.init()
        synth.delegate = self
        let center = NotificationCenter.default
        sessionObservers.append(center.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: .main) { [weak self] note in
            let began = (note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt).flatMap(AVAudioSession.InterruptionType.init) == .began
            MainActor.assumeIsolated { if began, self?.playing == true { self?.pause() } }
        })
        sessionObservers.append(center.addObserver(forName: AVAudioSession.routeChangeNotification, object: nil, queue: .main) { [weak self] note in
            let reason = (note.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt).flatMap(AVAudioSession.RouteChangeReason.init)
            // Kopfhörer abgezogen: anhalten statt über den Lautsprecher weiterzuspielen.
            MainActor.assumeIsolated { if reason == .oldDeviceUnavailable, self?.playing == true { self?.pause() } }
        })
    }

    var title: String { book?.title ?? "" }

    /// Kapitel des Hörbuchs: aus der Datei, sonst die Teile.
    var chapters: [AudiobookChapter] {
        if let c = info?.chapters, !c.isEmpty { return c }
        return AudiobookChapters.merge(files: [], tracks: tracks)
    }
    var chapterIndex: Int { AudiobookChapters.index(at: positionMs, in: chapters) }
    var currentChapter: AudiobookChapter? { let c = chapters; return c.isEmpty ? nil : c[min(chapterIndex, c.count - 1)] }

    func seek(toChapter index: Int) {
        let c = chapters
        guard c.indices.contains(index) else { return }
        seek(toMs: c[index].startMs)
    }

    func previousChapter() { seek(toMs: AudiobookChapters.previousTarget(at: positionMs, in: chapters)) }
    func nextChapter() { if let t = AudiobookChapters.nextTarget(at: positionMs, in: chapters) { seek(toMs: t) } }

    private func loadInfo(_ book: LibraryBook) {
        info = library.cachedAudiobookInfo(book)
        guard info == nil else { return }
        let id = book.id
        Task { [weak self] in
            guard let self else { return }
            let found = await self.library.audiobookInfo(book)
            guard self.book?.id == id, self.mode == .audiobook else { return }
            self.info = found
            self.updateNowPlaying()
        }
    }
    var followMap: FollowMap? {
        guard let book, let audio = book.audio, !text.isEmpty else { return nil }
        return FollowMap(sync: audio.sync, textLength: Double(text.utf16.count), totalMs: totalMs)
    }

    // MARK: Hörbuch

    func openAudiobook(_ book: LibraryBook, autoplay: Bool = true) {
        if self.book?.id == book.id, mode == .audiobook { if autoplay && !playing { play() }; return }
        stop()
        guard let audio = book.audio else { error = "Das Hörbuch ist nicht auf dem iPhone."; return }
        let all = audio.allTracks
        let files = all.map { library.dir(book).appendingPathComponent("audio").appendingPathComponent($0.file) }
        guard !all.isEmpty, files.allSatisfy({ FileManager.default.fileExists(atPath: $0.path) }) else {
            error = "Teile des Hörbuchs fehlen auf dem iPhone. Bitte erneut importieren oder laden."; return
        }
        self.book = book; mode = .audiobook
        self.urls = files; tracks = all
        totalMs = audio.totalDurationMs ?? tracks.reduce(0) { $0 + ($1.durationMs ?? 0) }
        text = library.text(book) ?? ""
        paragraphs = LibraryText.paragraphs(text)
        loadInfo(book)
        let start = library.record(library.audioKey(book))?.offset ?? 0
        lastSavedMs = start
        lastSaveAt = Date()
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
        seekSettled = position.offsetMs <= 0
        player?.seek(to: CMTime(seconds: position.offsetMs / 1000, preferredTimescale: 600), toleranceBefore: .zero, toleranceAfter: .zero) { [weak self] _ in
            Task { @MainActor in self?.seekSettled = true }
        }
        positionMs = LibraryTracks.total(TrackPosition(index: index, offsetMs: position.offsetMs), in: tracks)
        timeObserver = player?.addPeriodicTimeObserver(forInterval: CMTime(seconds: 0.5, preferredTimescale: 600), queue: .main) { [weak self] t in
            MainActor.assumeIsolated { self?.tick(t.seconds) }
        }
        endObserver = NotificationCenter.default.addObserver(forName: .AVPlayerItemDidPlayToEndTime, object: item, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.trackEnded() }
        }
    }

    private func tick(_ seconds: Double) {
        guard mode == .audiobook, seconds.isFinite, seekSettled else { return }
        positionMs = LibraryTracks.total(TrackPosition(index: trackIndex, offsetMs: seconds * 1000), in: tracks)
        if let stopAt = sleepAtMs, positionMs >= stopAt - 250 { cancelSleep(); pause(); seek(toMs: stopAt) }
        if Date().timeIntervalSince(lastSaveAt) >= 10 { saveAudio() }
        updateNowPlaying()
    }

    private func trackEnded() {
        if trackIndex + 1 < urls.count {
            load(position: TrackPosition(index: trackIndex + 1, offsetMs: 0))
            if playing { player?.playImmediately(atRate: rate) }
        } else {
            playing = false
            cancelSleep()
            saveAudio()
        }
    }

    func seek(toMs ms: Double) {
        guard mode == .audiobook else { return }
        let target = max(0, totalMs > 0 ? min(ms, totalMs) : ms)
        // Sprung hinter das Schlaftimer-Ziel (z. B. „Kapitel vor“): Timer verfällt.
        if let stopAt = sleepAtMs, target >= stopAt - 250 { cancelSleep() }
        let pos = LibraryTracks.position(of: target, in: tracks)
        if pos.index != trackIndex { load(position: pos) }
        else {
            seekSettled = false
            player?.seek(to: CMTime(seconds: pos.offsetMs / 1000, preferredTimescale: 600), toleranceBefore: .zero, toleranceAfter: .zero) { [weak self] _ in
                Task { @MainActor in self?.seekSettled = true }
            }
        }
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
            phononFailed = false
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

    private var phononVoice: String? { phononFailed ? nil : PhononEngine.voiceName(voiceIdentifier) }

    private func stopPhonon() {
        phononPaused = false
        phononTask?.cancel(); phononTask = nil
        PhononEngine.shared.silence()
    }

    /// Phonon liefert keine Wortzeiten: markiert wird der Absatz, nicht das Wort.
    private func speakPhonon(_ p: TextParagraph, voice: String) {
        wordRange = nil
        pausedParagraph = nil
        playing = true
        updateNowPlaying()
        phononTask = Task { [weak self] in
            do {
                try await PhononEngine.shared.speak(p.text, voice: voice, rate: self?.rate ?? 1)
                guard !Task.isCancelled else { return }
                self?.paragraphFinished()
            } catch is CancellationError {
            } catch {
                guard !Task.isCancelled, let self else { return }
                // Nicht verstummen: mit der Systemstimme weiterlesen.
                self.error = "Phonon konnte nicht sprechen, die Systemstimme liest weiter. (\(error.localizedDescription))"
                self.phononFailed = true  // nur bis zum nächsten Öffnen; die Wahl bleibt
                self.speakCurrent()
            }
        }
    }

    private func speakCurrent() {
        guard paragraphs.indices.contains(paragraphIndex) else { playing = false; return }
        currentUtterance = nil
        stopPhonon()
        synth.stopSpeaking(at: .immediate)
        activateSession()
        let p = paragraphs[paragraphIndex]
        if let voice = phononVoice { speakPhonon(p, voice: voice); return }
        let u = AVSpeechUtterance(string: p.text)
        u.voice = voiceIdentifier.flatMap { AVSpeechSynthesisVoice(identifier: $0) } ?? Self.defaultVoice()
        u.rate = AVSpeechUtteranceDefaultSpeechRate * rate
        wordRange = nil
        pausedParagraph = nil
        playing = true
        currentUtterance = u
        synth.speak(u)
        updateNowPlaying()
    }

    func jump(toParagraph index: Int) {
        guard paragraphs.indices.contains(index) else { return }
        if mode == .audiobook, let map = followMap { seek(toMs: map.ms(atChars: Double(paragraphs[index].start))); return }
        paragraphIndex = index
        if playing { speakCurrent() }
        else {
            // Pausiert: alte Äußerung verwerfen, damit Weiter am neuen Absatz beginnt.
            if synth.isPaused { currentUtterance = nil; synth.stopSpeaking(at: .immediate) }
            stopPhonon()
            pausedParagraph = nil
            wordRange = nil
            saveText(at: paragraphs[index].start)
        }
    }

    func step(paragraphs delta: Int) { jump(toParagraph: min(max(0, paragraphIndex + delta), paragraphs.count - 1)) }

    private func saveText(at chars: Int, heard: Range<Int>? = nil) {
        guard let book else { return }
        var rec = LibraryProgress.recordPosition(library.record(library.textKey(book)), to: Double(chars), length: Double(text.utf16.count),
                                                 unit: .chars, mode: .heard, now: PhoneLibrary.nowMs())
        if let heard { rec = LibraryProgress.markHeard(rec, from: Double(heard.lowerBound), to: Double(heard.upperBound)) }
        library.put(library.textKey(book), rec)
        lastTextSave = Date()
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
            if phononPaused, PhononEngine.shared.resume() { phononPaused = false; playing = true }
            else if synth.isPaused, pausedParagraph == paragraphIndex { synth.continueSpeaking(); playing = true }
            else { speakCurrent() }
        case nil: return
        }
        updateNowPlaying()
    }

    func pause() {
        switch mode {
        case .audiobook: player?.pause(); saveAudio()
        case .readAloud:
            if phononTask != nil { PhononEngine.shared.pause(); phononPaused = true }
            else { synth.pauseSpeaking(at: .word); pausedParagraph = paragraphIndex }
        case nil: break
        }
        playing = false
        library.flush()
        updateNowPlaying()
    }

    func toggle() { playing ? pause() : play() }

    func stop() {
        if mode == .audiobook { player?.pause(); saveAudio() }
        if mode == .readAloud { currentUtterance = nil; stopPhonon(); synth.stopSpeaking(at: .immediate) }
        removeObservers()
        player?.replaceCurrentItem(with: nil)
        playing = false; book = nil; mode = nil; wordRange = nil; info = nil
        cancelSleep()
        library.flush()
        MPNowPlayingInfoCenter.default().nowPlayingInfo = nil
    }

    func scene(active: Bool) { if !active { if mode == .audiobook { saveAudio() }; library.flush() } }

    private func applyRate() {
        if mode == .audiobook, playing { player?.rate = rate }
        if mode == .readAloud {
            if phononTask != nil { PhononEngine.shared.setRate(rate) } else if playing { speakCurrent() }
        }
    }

    func sleep(minutes: Int?) {
        cancelSleep()
        guard let minutes else { return }
        sleepEnds = Date().addingTimeInterval(Double(minutes) * 60)
        sleepTimer = Timer.scheduledTimer(withTimeInterval: Double(minutes) * 60, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated { self?.pause(); self?.sleepEnds = nil }
        }
    }
    /// Am Ende des laufenden Kapitels anhalten.
    func sleepAtChapterEnd() {
        cancelSleep()
        guard mode == .audiobook, let c = currentChapter else { return }
        sleepAtMs = c.endMs
    }
    /// Artwork einmal je Cover bauen, nicht bei jedem Tick.
    private var artworkCache: (key: String, art: MPMediaItemArtwork?)?
    private func nowPlayingArtwork(_ book: LibraryBook) -> MPMediaItemArtwork? {
        let key = book.id + "#" + String(library.coverRevision)
        if let c = artworkCache, c.key == key { return c.art }
        let art = library.coverImage(book).map(Self.artwork)
        artworkCache = (key, art)
        return art
    }

    private func cancelSleep() { sleepTimer?.invalidate(); sleepTimer = nil; sleepEnds = nil; sleepAtMs = nil }

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
        // Kopfhörer/Sperrbildschirm „nächster/voriger Titel“ = Kapitel.
        c.nextTrackCommand.addTarget { [weak self] _ in
            MainActor.assumeIsolated { if self?.mode == .audiobook { self?.nextChapter() } else { self?.step(paragraphs: 1) } }; return .success }
        c.previousTrackCommand.addTarget { [weak self] _ in
            MainActor.assumeIsolated { if self?.mode == .audiobook { self?.previousChapter() } else { self?.step(paragraphs: -1) } }; return .success }
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
            info[MPMediaItemPropertyAlbumTitle] = book.title
            let c = chapters
            if c.count > 1, let current = currentChapter {
                info[MPMediaItemPropertyTitle] = current.title
                info[MPNowPlayingInfoPropertyChapterNumber] = chapterIndex
                info[MPNowPlayingInfoPropertyChapterCount] = c.count
            }
        }
        if let art = nowPlayingArtwork(book) { info[MPMediaItemPropertyArtwork] = art }
        MPNowPlayingInfoCenter.default().nowPlayingInfo = info
    }
}

extension LibraryPlayback {
    /// Außerhalb des Main Actors gebaut: MediaPlayer ruft den Block auf einer Hintergrundwarteschlange.
    nonisolated static func artwork(_ image: UIImage) -> MPMediaItemArtwork {
        MPMediaItemArtwork(boundsSize: image.size) { _ in image }
    }
}

extension LibraryPlayback: AVSpeechSynthesizerDelegate {
    nonisolated func speechSynthesizer(_ s: AVSpeechSynthesizer, willSpeakRangeOfSpeechString range: NSRange, utterance: AVSpeechUtterance) {
        let id = ObjectIdentifier(utterance)
        Task { @MainActor in
            guard let current = self.currentUtterance, ObjectIdentifier(current) == id else { return }
            self.wordRange = range
            // Lange Absätze (ganze PDF-Seiten): Stelle im Absatz alle 10 s sichern.
            if Date().timeIntervalSince(self.lastTextSave) >= 10, self.paragraphs.indices.contains(self.paragraphIndex) {
                let p = self.paragraphs[self.paragraphIndex]
                let offset = (p.heading ? p.end - p.text.utf16.count : p.start) + range.location
                self.saveText(at: min(offset, p.end))
            }
        }
    }
    nonisolated func speechSynthesizer(_ s: AVSpeechSynthesizer, didFinish utterance: AVSpeechUtterance) {
        let id = ObjectIdentifier(utterance)
        Task { @MainActor in
            guard let current = self.currentUtterance, ObjectIdentifier(current) == id else { return }
            self.paragraphFinished()
        }
    }

    /// Absatz zu Ende gesprochen, von der Systemstimme oder von Phonon.
    fileprivate func paragraphFinished() {
        guard mode == .readAloud, playing else { return }
        let finished = paragraphIndex
        guard paragraphs.indices.contains(finished) else { return }
        let p = paragraphs[finished]
        let next = finished + 1 < paragraphs.count ? paragraphs[finished + 1].start : p.end
        // Stand = Beginn des nächsten Absatzes, damit Weiterlesen nicht wiederholt.
        saveText(at: next, heard: p.start..<p.end)
        if finished + 1 < paragraphs.count { paragraphIndex = finished + 1; speakCurrent() }
        else { playing = false; wordRange = nil; currentUtterance = nil; phononTask = nil; library.flush() }
    }
}
