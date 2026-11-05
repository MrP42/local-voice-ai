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
    /// Gerade gelesener Satz im aktuellen Absatz.
    @Published private(set) var sentenceRange: NSRange?
    /// Zwischen Abspielen und dem ersten Ton (Modell laden, ersten Satz erzeugen).
    @Published private(set) var preparing = false
    /// Dieselbe Stimme wie in den Einstellungen (`VoiceModel.selectedVoiceID`).
    @Published var voiceIdentifier: String? = UserDefaults.standard.string(forKey: "selectedVoiceID")
        ?? UserDefaults.standard.string(forKey: "library.voice") {
        didSet {
            if VoiceModel.shared.selectedVoiceID != (voiceIdentifier ?? "") { VoiceModel.shared.selectedVoiceID = voiceIdentifier ?? "" }
            if voiceIdentifier != oldValue { voiceChanged() }
        }
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
    /// Jede neue Lesung zählt hoch; Rückmeldungen älterer werden verworfen.
    private var readGeneration = 0
    /// Nächster Satz, den Phonon erzeugt; liegt vor dem hörbaren Satz.
    private var nextToSynthesize: Cursor?
    private var sentences: [NSRange] = []
    private var sentenceIndex = 0
    /// Beginn der Äußerung der Systemstimme im Absatz.
    private var utteranceBase = 0
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
            MainActor.assumeIsolated { if began, self?.busy == true { self?.pause() } }
        })
        sessionObservers.append(center.addObserver(forName: AVAudioSession.routeChangeNotification, object: nil, queue: .main) { [weak self] note in
            let reason = (note.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt).flatMap(AVAudioSession.RouteChangeReason.init)
            // Kopfhörer abgezogen: anhalten statt über den Lautsprecher weiterzuspielen.
            MainActor.assumeIsolated { if reason == .oldDeviceUnavailable, self?.busy == true { self?.pause() } }
        })
    }

    var title: String { book?.title ?? "" }
    /// Läuft oder ist auf dem Weg dorthin.
    var busy: Bool { playing || preparing }

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

    func openAudiobook(_ book: LibraryBook, autoplay: Bool = false) {
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

    func openReadAloud(_ book: LibraryBook, autoplay: Bool = false, fromParagraph: Int? = nil) {
        if self.book?.id != book.id || mode != .readAloud {
            stop()
            guard let text = library.text(book) else { error = "Der Text ist nicht auf dem iPhone."; return }
            phononFailed = false
            self.book = book; mode = .readAloud
            self.text = text
            paragraphs = LibraryText.paragraphs(text)
            // Ohne gemerkte Stelle beim ersten Inhalt beginnen, nicht bei Titelei,
            // Inhaltsverzeichnis oder Impressum (wie am Desktop, lva#295).
            let readPreface = UserDefaults.standard.object(forKey: BookStart.readPrefaceKey) as? Bool ?? true
            let offset = library.record(library.textKey(book))?.offset
                ?? Double(BookStart.start(text, readPreface: readPreface).offset)
            let index = LibraryText.paragraphIndex(paragraphs, at: offset)
            move(to: Cursor(paragraph: index, sentence: 0))
            if paragraphs.indices.contains(index) {
                // Gespeichert wird satzgenau: Weiterlesen wiederholt höchstens diesen Satz.
                let local = Int(offset) - paragraphs[index].textStart
                move(to: Cursor(paragraph: index, sentence: LibraryText.sentenceIndex(sentences, at: local)))
            }
            totalMs = 0
            setupRemote()
            prepareVoice()
        }
        if let fromParagraph { move(to: Cursor(paragraph: fromParagraph, sentence: 0)); if busy || autoplay { speakCurrent() }; return }
        if autoplay { play() }
    }

    private var phononVoice: String? { phononFailed ? nil : PhononEngine.voiceName(voiceIdentifier) }

    private func stopPhonon() {
        readGeneration += 1
        phononPaused = false
        preparing = false
        phononTask?.cancel(); phononTask = nil
        PhononEngine.shared.silence()
    }

    // MARK: Stelle im Text

    private struct Cursor: Equatable { var paragraph: Int; var sentence: Int }

    private func sentenceRanges(_ paragraph: Int) -> [NSRange] {
        if paragraph == paragraphIndex, !sentences.isEmpty { return sentences }
        return paragraphs.indices.contains(paragraph) ? LibraryText.sentences(paragraphs[paragraph].text) : []
    }

    private func move(to cursor: Cursor) {
        let ranges = sentenceRanges(cursor.paragraph)
        paragraphIndex = cursor.paragraph
        sentences = ranges
        sentenceIndex = ranges.isEmpty ? 0 : min(max(0, cursor.sentence), ranges.count - 1)
        sentenceRange = ranges.isEmpty ? nil : ranges[sentenceIndex]
    }

    /// Nächster Satz; leere Absätze werden übersprungen.
    private func cursor(after c: Cursor) -> Cursor? {
        if c.sentence + 1 < sentenceRanges(c.paragraph).count { return Cursor(paragraph: c.paragraph, sentence: c.sentence + 1) }
        var next = c.paragraph + 1
        while next < paragraphs.count {
            if !sentenceRanges(next).isEmpty { return Cursor(paragraph: next, sentence: 0) }
            next += 1
        }
        return nil
    }

    /// Stelle im Gesamttext, an der der Satz beginnt.
    private func chars(of c: Cursor) -> Int {
        let ranges = sentenceRanges(c.paragraph)
        return paragraphs[c.paragraph].textStart + (ranges.indices.contains(c.sentence) ? ranges[c.sentence].location : 0)
    }

    /// Phonon liest Satz für Satz: Das Ende eines Satzes rückt Markierung und Stand weiter.
    private func speakPhonon(voice: String) {
        pausedParagraph = nil
        playing = false
        preparing = true
        updateNowPlaying()
        let generation = readGeneration
        nextToSynthesize = sentences.isEmpty ? cursor(after: Cursor(paragraph: paragraphIndex, sentence: 0))
                                             : Cursor(paragraph: paragraphIndex, sentence: sentenceIndex)
        if let start = nextToSynthesize { move(to: start) }
        phononTask = Task { [weak self] in
            do {
                try await PhononEngine.shared.read(voice: voice, rate: self?.rate ?? 1, next: { self?.nextSentence(generation) },
                                                   audible: { self?.audioStarted(generation) }, played: { self?.sentencePlayed(generation) })
                guard !Task.isCancelled else { return }
                self?.readingEnded(generation)
            } catch is CancellationError {
                // Eigene Abbrüche zählen die Generation hoch; sonst wurde von außen beendet.
                guard let self, generation == self.readGeneration else { return }
                self.readingInterrupted()
            } catch is PhononAudioStalled {
                guard let self, generation == self.readGeneration else { return }
                let resume = self.busy && !self.phononPaused
                self.readingInterrupted()
                if resume { self.speakCurrent() }  // am hörbaren Satz neu ansetzen
            } catch {
                guard !Task.isCancelled, let self, generation == self.readGeneration else { return }
                self.phononFailed = true  // nur bis zum nächsten Öffnen; die Wahl bleibt
                if self.phononPaused { self.readingInterrupted(); return }  // Pause bleibt Pause
                // Nicht verstummen: mit der Systemstimme weiterlesen.
                self.error = "Phonon konnte nicht sprechen, die Systemstimme liest weiter. (\(error.localizedDescription))"
                self.speakCurrent()
            }
        }
    }

    private func nextSentence(_ generation: Int) -> String? {
        guard generation == readGeneration, let c = nextToSynthesize else { return nil }
        nextToSynthesize = cursor(after: c)
        let ranges = sentenceRanges(c.paragraph)
        guard ranges.indices.contains(c.sentence) else { return nil }
        return (paragraphs[c.paragraph].text as NSString).substring(with: ranges[c.sentence])
    }

    private func audioStarted(_ generation: Int) {
        guard generation == readGeneration else { return }
        preparing = false
        playing = true
        updateNowPlaying()
    }

    private func sentencePlayed(_ generation: Int) {
        guard generation == readGeneration, mode == .readAloud, paragraphs.indices.contains(paragraphIndex) else { return }
        let here = Cursor(paragraph: paragraphIndex, sentence: sentenceIndex)
        let p = paragraphs[here.paragraph]
        let from = here.sentence == 0 ? p.start : chars(of: here)
        let next = cursor(after: here)
        if let next, next.paragraph == here.paragraph {
            saveText(at: chars(of: next), heard: from..<chars(of: next))
        } else {
            // Stand = Beginn des nächsten Absatzes, damit Weiterlesen nicht wiederholt.
            saveText(at: next.map { paragraphs[$0.paragraph].start } ?? p.end, heard: from..<p.end)
        }
        if let next { move(to: next) }
    }

    /// Lesung ohne eigenes Zutun zu Ende: Stand bleibt am hörbaren Satz, Weiter setzt dort an.
    private func readingInterrupted() {
        readGeneration += 1
        phononTask = nil
        phononPaused = false
        preparing = false
        playing = false
        updateNowPlaying()
    }

    private func readingEnded(_ generation: Int) {
        guard generation == readGeneration else { return }
        playing = false
        preparing = false
        sentenceRange = nil
        phononTask = nil
        library.flush()
        updateNowPlaying()
    }

    private func voiceChanged() {
        guard mode == .readAloud else { return }
        phononFailed = false
        if busy { speakCurrent() } else { discardPausedSpeech() }
        prepareVoice()
    }

    /// Modell laden und die nächsten Sätze vorab erzeugen.
    private func prepareVoice() {
        guard let voice = phononVoice else { return }
        var upcoming: [String] = []
        var c: Cursor? = Cursor(paragraph: paragraphIndex, sentence: sentenceIndex)
        while let here = c, upcoming.count < 3 {
            let ranges = sentenceRanges(here.paragraph)
            if ranges.indices.contains(here.sentence) { upcoming.append((paragraphs[here.paragraph].text as NSString).substring(with: ranges[here.sentence])) }
            c = cursor(after: here)
        }
        PhononEngine.shared.prepare(voice: voice, sentences: upcoming)
    }

    /// Pausierte Äußerung verwerfen: Weiter beginnt am aktuellen Satz.
    private func discardPausedSpeech() {
        if synth.isPaused { currentUtterance = nil; synth.stopSpeaking(at: .immediate) }
        stopPhonon()
        pausedParagraph = nil
    }

    private func speakCurrent() {
        guard paragraphs.indices.contains(paragraphIndex) else { playing = false; return }
        currentUtterance = nil
        stopPhonon()
        synth.stopSpeaking(at: .immediate)
        activateSession()
        if let voice = phononVoice { speakPhonon(voice: voice); return }
        let p = paragraphs[paragraphIndex]
        utteranceBase = sentences.indices.contains(sentenceIndex) ? sentences[sentenceIndex].location : 0
        let u = AVSpeechUtterance(string: (p.text as NSString).substring(from: utteranceBase))
        u.voice = voiceIdentifier.flatMap { AVSpeechSynthesisVoice(identifier: $0) } ?? Self.defaultVoice()
        u.rate = AVSpeechUtteranceDefaultSpeechRate * rate
        pausedParagraph = nil
        playing = true
        currentUtterance = u
        synth.speak(u)
        updateNowPlaying()
    }

    func jump(toParagraph index: Int) {
        guard paragraphs.indices.contains(index) else { return }
        if mode == .audiobook, let map = followMap { seek(toMs: map.ms(atChars: Double(paragraphs[index].start))); return }
        move(to: Cursor(paragraph: index, sentence: 0))
        saveText(at: paragraphs[index].start, jumped: true)
        if busy { speakCurrent() } else { discardPausedSpeech(); prepareVoice() }
    }

    /// Satz vor oder zurück. Zuletzt gelesene Sätze liegen noch im Speicher und klingen sofort.
    func step(sentences delta: Int) {
        guard mode == .readAloud else { return }
        var here = Cursor(paragraph: paragraphIndex, sentence: sentenceIndex)
        if delta > 0 {
            guard let next = cursor(after: here) else { return }
            here = next
        } else if here.sentence > 0 {
            here.sentence -= 1
        } else {
            var previous = here.paragraph - 1
            while previous >= 0, sentenceRanges(previous).isEmpty { previous -= 1 }
            guard previous >= 0 else { return }
            here = Cursor(paragraph: previous, sentence: sentenceRanges(previous).count - 1)
        }
        move(to: here)
        saveText(at: chars(of: here), jumped: true)
        if busy { speakCurrent() } else { discardPausedSpeech(); prepareVoice() }
    }

    func step(paragraphs delta: Int) { jump(toParagraph: min(max(0, paragraphIndex + delta), paragraphs.count - 1)) }

    /// `jumped`: Die Stelle wurde angesprungen, das Dazwischen zählt nicht als gehört.
    private func saveText(at chars: Int, heard: Range<Int>? = nil, jumped: Bool = false) {
        guard let book else { return }
        let previous = library.record(library.textKey(book))
        var rec = LibraryProgress.recordPosition(previous, to: Double(chars), length: Double(text.utf16.count),
                                                 unit: .chars, mode: .heard, now: PhoneLibrary.nowMs())
        if jumped { rec.heard = previous?.heard ?? []; rec.finished = previous?.finished }
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
            if busy { return }
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
            if preparing { stopPhonon() }  // noch kein Ton: nichts anzuhalten, Weiter beginnt am Satz
            else if phononTask != nil { PhononEngine.shared.pause(); phononPaused = true }
            else { synth.pauseSpeaking(at: .word); pausedParagraph = paragraphIndex }
        case nil: break
        }
        playing = false
        library.flush()
        updateNowPlaying()
    }

    func toggle() { busy ? pause() : play() }

    func stop() {
        if mode == .audiobook { player?.pause(); saveAudio() }
        if mode == .readAloud { currentUtterance = nil; stopPhonon(); synth.stopSpeaking(at: .immediate) }
        removeObservers()
        player?.replaceCurrentItem(with: nil)
        playing = false; book = nil; mode = nil; sentenceRange = nil; sentences = []; info = nil
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
            MainActor.assumeIsolated { if self?.mode == .readAloud { self?.step(sentences: -1) } else { self?.skip(seconds: -15) } }; return .success }
        c.skipForwardCommand.addTarget { [weak self] _ in
            MainActor.assumeIsolated { if self?.mode == .readAloud { self?.step(sentences: 1) } else { self?.skip(seconds: 15) } }; return .success }
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
            self.systemVoiceReached(range.location)
        }
    }
    nonisolated func speechSynthesizer(_ s: AVSpeechSynthesizer, didFinish utterance: AVSpeechUtterance) {
        let id = ObjectIdentifier(utterance)
        Task { @MainActor in
            guard let current = self.currentUtterance, ObjectIdentifier(current) == id else { return }
            self.paragraphFinished()
        }
    }

    /// Die Systemstimme spricht den Rest des Absatzes am Stück; hier rückt der Satz nach.
    fileprivate func systemVoiceReached(_ location: Int) {
        let index = LibraryText.sentenceIndex(sentences, at: utteranceBase + location)
        guard index != sentenceIndex, paragraphs.indices.contains(paragraphIndex) else { return }
        let here = Cursor(paragraph: paragraphIndex, sentence: index)
        move(to: here)
        saveText(at: chars(of: here))
    }

    /// Absatz von der Systemstimme zu Ende gesprochen.
    fileprivate func paragraphFinished() {
        guard mode == .readAloud, playing else { return }
        let finished = paragraphIndex
        guard paragraphs.indices.contains(finished) else { return }
        let p = paragraphs[finished]
        let next = finished + 1 < paragraphs.count ? paragraphs[finished + 1].start : p.end
        // Stand = Beginn des nächsten Absatzes, damit Weiterlesen nicht wiederholt.
        saveText(at: next, heard: (utteranceBase == 0 ? p.start : p.textStart + utteranceBase)..<p.end)
        if finished + 1 < paragraphs.count { move(to: Cursor(paragraph: finished + 1, sentence: 0)); speakCurrent() }
        else { playing = false; sentenceRange = nil; currentUtterance = nil; library.flush() }
    }
}
