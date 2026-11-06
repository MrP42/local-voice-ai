import Foundation
import AVFoundation
import MediaPlayer
import Combine
import UIKit

/// Was gerade in der Bibliothek läuft: Hörbuch oder Vorlesen. Es läuft immer
/// höchstens eines; der Mini-Player zeigt es auf Regal und Buchseite.
@MainActor
final class LibraryPlayback: NSObject, ObservableObject {
    static let shared = LibraryPlayback()

    enum Mode: Equatable { case audiobook, readAloud }

    @Published private(set) var book: LibraryBook?
    @Published private(set) var mode: Mode?
    @Published private(set) var playing = false {
        didSet { if playing != oldValue { noteSpokenClock(rate: rate) } }
    }
    @Published private(set) var positionMs: Double = 0
    @Published private(set) var totalMs: Double = 0
    @Published private(set) var trackIndex = 0
    @Published var rate: Float = 1.0 {
        didSet { noteSpokenClock(rate: oldValue); applyRate() }
    }
    /// Sprachfluss beim Vorlesen mit Phonon: 0 zügig, 0,5 Hörbuch, 1 ruhig (`SpeechFlow`).
    @Published var flow: Double = SpeechFlow.stored().value {
        didSet {
            UserDefaults.standard.set(flow, forKey: SpeechFlow.storageKey)
            guard mode == .readAloud, oldValue != flow else { return }
            // Während Phonon noch lädt nichts tun: read() liest Tempo und Fluss erst danach.
            if phononTask != nil, phononPaused { discardPausedSpeech() } else if playing, phononTask != nil { speakCurrent() }
        }
    }
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
            if voiceIdentifier != oldValue, !applyingVoicePref { voiceChanged() }
        }
    }
    /// Sprache des offenen Buchs (`de`, `en` …) für die Stimmenwahl; `nil`: unbekannt.
    @Published private(set) var voiceLanguage: String?
    /// Woher die Stimme kommt: nur dieses Buch oder alle Bücher der Sprache.
    @Published private(set) var voiceScope: VoicePrefs.Source?
    /// Das Buch hat eine eigene Stimme, die es auf diesem iPhone nicht gibt: es liest der Sprachstandard.
    @Published private(set) var bookVoiceMissing = false
    private var applyingVoicePref = false
    private var catalogWatch: AnyCancellable?
    @Published var error: String?
    /// Vollbild-Player sichtbar (Hören mit Mitlesen oder Vorlesen).
    @Published var presented = false
    /// Ansicht im Einheits-Player (Hören · Lesen & Hören · Lesen), je Buch gemerkt.
    @Published private(set) var surface: PlayerSurface = .listen
    /// Wort, das die Systemstimme gerade spricht (relativ zum Absatztext). Phonon liest satzweise: dort `nil`.
    @Published private(set) var wordRange: NSRange?

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
    private var spokenBefore: Double = 0
    private var spokenSince: Date?
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
    /// Satzgrenzen je Absatz fürs Mitlesen im Hörbuch (NLTokenizer ist zu teuer für jeden Takt).
    private var sentenceCache: [Int: [NSRange]] = [:]
    private var followCache: FollowMap?
    /// Buch-Gespräch (G5): Solange das Blatt offen ist, hält die Pause-Taste (AirPods, Sperrbildschirm) die Antwort
    /// an statt die Wiedergabe. Gibt `true` zurück, wenn der Befehl verbraucht wurde.
    enum RemoteCommand { case play, pause, toggle }
    var remoteIntercept: ((RemoteCommand) -> Bool)?

    init(library: PhoneLibrary? = nil) {
        self.library = library ?? .shared
        super.init()
        synth.delegate = self
        // Phonon-Katalog geladen oder geändert: gemerkte Stimme des offenen Buchs neu prüfen (lva#352).
        catalogWatch = PhononEngine.shared.$voices.dropFirst().receive(on: RunLoop.main).sink { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self, self.mode == .readAloud, let book = self.book else { return }
                self.applyVoicePref(book, live: true)
            }
        }
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
    /// Im aktuellen Satz gesprochene Zeit bei Normaltempo, ohne Pausen. Die Anzeige rechnet daraus den Satzteil,
    /// damit sie Pausen und das Öffnen des Players übersteht.
    var sentenceSpokenSeconds: Double { spokenBefore + (spokenSince.map { Date().timeIntervalSince($0) } ?? 0) * Double(rate) }

    private func noteSpokenClock(rate used: Float) {
        if let since = spokenSince { spokenBefore += Date().timeIntervalSince(since) * Double(used) }
        spokenSince = playing ? Date() : nil
    }

    /// Gerade gelesener Satz, für das Mitlesen im Mini-Player.
    var currentSentence: String? {
        guard mode == .readAloud, let range = sentenceRange, paragraphs.indices.contains(paragraphIndex) else { return nil }
        let text = paragraphs[paragraphIndex].text as NSString
        return NSMaxRange(range) <= text.length ? text.substring(with: range) : nil
    }
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
        if mode == .audiobook, let followCache { return followCache }
        return FollowMap(sync: audio.sync, textLength: Double(text.utf16.count), totalMs: totalMs)
    }

    // MARK: Einheits-Player

    /// Name der Vorlesestimme für die Anzeige.
    var voiceName: String {
        if let phonon = PhononEngine.voiceName(voiceIdentifier) { return PhononEngine.displayName(phonon) }
        return (voiceIdentifier.flatMap { AVSpeechSynthesisVoice(identifier: $0) } ?? Self.defaultVoice())?.name ?? "System"
    }

    /// Laufende Tonquelle.
    var source: PlayerSource? { mode == .audiobook ? .audiobook : mode == .readAloud ? .voice : nil }

    /// Satzbereiche eines Absatzes, zwischengespeichert.
    func sentences(inParagraph index: Int) -> [NSRange] {
        if let cached = sentenceCache[index] { return cached }
        guard paragraphs.indices.contains(index) else { return [] }
        let ranges = LibraryText.sentences(paragraphs[index].text)
        sentenceCache[index] = ranges
        return ranges
    }

    /// Was beim Mitlesen markiert ist: Absatz, Satz und Wort.
    var readingMark: ReadingMark? {
        switch mode {
        case .audiobook:
            guard let map = followMap else { return nil }
            return ReadAlong.mark(atMs: positionMs, map: map, paragraphs: paragraphs) { self.sentences(inParagraph: $0) }
        case .readAloud:
            guard paragraphs.indices.contains(paragraphIndex) else { return nil }
            return ReadingMark(paragraph: paragraphIndex, sentence: sentenceRange, word: wordRange, precise: true)
        case nil: return nil
        }
    }

    private static func surfaceKey(_ id: String) -> String { "lva.player.surface.\(id)" }
    private static func sourceKey(_ id: String) -> String { "lva.player.source.\(id)" }

    /// Ein Einstieg für alles: öffnet das Buch mit der gemerkten Quelle und Ansicht.
    /// Liegt das Hörbuch nicht auf dem iPhone, liest die Stimme; ohne Text spielt nur das Hörbuch.
    /// `prefer`: Quelle für diesen Aufruf erzwingen (z. B. Hörbuch-Teil im Buchdetail) und merken.
    func open(_ book: LibraryBook, autoplay: Bool = false, prefer: PlayerSource? = nil) {
        let d = UserDefaults.standard
        let book = resolvedView(book)
        // Läuft das Buch schon, bleibt die Quelle; nur den Buchstand auffrischen (z. B. Hörbuch inzwischen geladen).
        if self.book?.id == book.id, let current = source, prefer == nil || prefer == current {
            self.book = book
            if autoplay && !busy { play() }
            presented = true
            return
        }
        if let prefer, self.book?.id == book.id, source != nil {
            self.book = book
            switchSource(to: prefer)
            if autoplay && !busy { play() }
            presented = true
            return
        }
        let hasText = library.hasText(book)
        let stored = prefer?.rawValue ?? d.string(forKey: Self.sourceKey(book.id))
        let source = PlayerSource.initial(stored: stored, audioLocal: library.hasAudio(book) && audioFilesPresent(book), hasText: hasText)
        // Vorlesen als Ersatz, weil das Hörbuch noch nicht geladen ist, ist keine Wahl: Nach dem Laden gilt das Hörbuch.
        if let source, prefer != nil || library.hasAudio(book) { d.set(source.rawValue, forKey: Self.sourceKey(book.id)) }
        switch source {
        case .audiobook: openAudiobook(book, autoplay: false)
        case .voice: openReadAloud(book, autoplay: false)
        case nil: error = "Für dieses Buch liegt weder Text noch Hörbuch auf dem iPhone."; return
        }
        guard self.book?.id == book.id else { return }
        // Erst die Ansicht dieses Buchs, dann abspielen: `play` richtet sich nach ihr.
        surface = PlayerSurface.initial(stored: d.string(forKey: Self.surfaceKey(book.id)), hasText: !paragraphs.isEmpty)
        if autoplay { play() }
        presented = true
    }

    /// Ansicht wechseln, ein Tipp. „Lesen“ hält an; zurück zu „Lesen & Hören“ spielt weiter, wenn vorher lief.
    func show(_ next: PlayerSurface) {
        guard let book, next != surface, PlayerSurface.available(hasText: !paragraphs.isEmpty).contains(next) else { return }
        if !next.keepsPlaying {
            resumeAfterReading = busy
            if busy { pause() }
        } else if !surface.keepsPlaying {
            if resumeAfterReading { play() }
            resumeAfterReading = false
        }
        surface = next
        UserDefaults.standard.set(next.rawValue, forKey: Self.surfaceKey(book.id))
    }
    private var resumeAfterReading = false

    /// Hörbuch ↔ Vorlesestimme an derselben Textstelle. Läuft die Wiedergabe, läuft sie an der neuen Quelle weiter.
    func switchSource(to target: PlayerSource) {
        guard let book, let current = source, target != current else { return }
        let wasPlaying = busy
        let offset: Int
        switch current {
        case .audiobook:
            guard let map = followMap else { return }
            offset = ReadAlong.handoverChars(atMs: positionMs, map: map, paragraphs: paragraphs) { self.sentences(inParagraph: $0) }
        case .voice:
            offset = paragraphs.indices.contains(paragraphIndex) ? chars(of: Cursor(paragraph: paragraphIndex, sentence: sentenceIndex)) : 0
        }
        let keep = surface
        // Schlaftimer über den Wechsel retten (stop() setzt ihn zurück); „Kapitelende“ gilt nur fürs Hörbuch.
        let sleepUntil = sleepEnds
        switch target {
        case .audiobook:
            // Vor stop() prüfen: ein gescheiterter Wechsel darf den laufenden Player nicht leeren.
            guard library.hasAudio(book), audioFilesPresent(book) else { error = "Das Hörbuch ist nicht vollständig auf dem iPhone. Bitte zuerst im Buch laden."; return }
            openAudiobook(book)
        case .voice:
            guard library.hasText(book) else { return }
            openReadAloud(book)
        }
        guard self.book?.id == book.id, source == target else { return }
        surface = keep
        UserDefaults.standard.set(target.rawValue, forKey: Self.sourceKey(book.id))
        jump(toOffset: offset, autoplay: wasPlaying)
        if let sleepUntil, sleepUntil > Date() { armSleep(until: sleepUntil) }
    }

    // MARK: Sprachfassungen (lva#344)

    /// Hinweis nach einem Sprachwechsel mit „Rückgängig“ (zehn Sekunden sichtbar).
    struct LanguageNotice: Identifiable {
        let id = UUID()
        let text: String
        let undo: PhoneLibrary.SwitchUndo
    }
    @Published private(set) var languageNotice: LanguageNotice?
    private var noticeTask: Task<Void, Never>?

    /// Kennung der Sprachfassung, die gerade läuft (nur bei Büchern mit mehreren Sprachen).
    var editionId: String? { book?.editionId }

    /// Das Buch als Sicht auf die Fassung, die gespielt wird: laufendes Buch bleibt in seiner Fassung, sonst gilt die
    /// zuletzt aktive, die auf diesem iPhone liegt. Kurzbücher und Bücher ohne Fassungen bleiben unverändert.
    private func resolvedView(_ book: LibraryBook) -> LibraryBook {
        guard let raw = library.rawBook(book.id) else { return book }
        if self.book?.id == book.id { return library.view(raw, editionId: self.book?.editionId) }
        let wanted = book.editionId.map { library.view(raw, editionId: $0) } ?? library.activeView(raw)
        return library.isPlayable(wanted) ? wanted : library.playableView(raw)
    }

    /// Trägt die Wiedergabe in der Zielfassung: das gerade benutzte Medium, sonst das andere. `nil`: nichts davon liegt hier.
    func medium(of view: LibraryBook) -> SwitchMedium? {
        let audio = library.hasAudio(view) && audioFilesPresent(view)
        let hasText = library.hasText(view)
        switch mode {
        case .audiobook: return audio ? .audio : (hasText ? .text : nil)
        case .readAloud: return hasText ? .text : (audio ? .audio : nil)
        case nil: return nil
        }
    }

    /// Sprache wechseln: gleiche Stelle (kapitelgenau, sonst ungefähr), Hinweis mit „Rückgängig“. Fortschritt und Auswahl
    /// gehen beim nächsten Abgleich zum PC.
    func switchLanguage(to editionId: String) {
        guard let current = book, let mode, let raw = library.rawBook(current.id) else { return }
        let editions = BookEditions(book: raw)
        guard let target = editions.edition(editionId), let fromId = current.editionId ?? editions.readingEdition?.id, fromId != editionId else { return }
        let view = library.editionView(raw, edition: target)
        guard let toMedium = medium(of: view) else {
            error = "Von dieser Sprache liegt weder Text noch Hörbuch auf dem iPhone. Bitte zuerst laden."
            return
        }
        let fromMedium: SwitchMedium = mode == .audiobook ? .audio : .text
        let wasPlaying = busy
        let offset: Double, length: Double
        if fromMedium == .audio {
            saveAudio()
            offset = positionMs; length = totalMs
        } else {
            offset = paragraphs.indices.contains(paragraphIndex) ? Double(chars(of: Cursor(paragraph: paragraphIndex, sentence: sentenceIndex))) : 0
            length = Double(text.utf16.count)
        }
        let fromChapters = fromMedium == .audio ? EditionSwitch.audioChapters(current.audio) : EditionSwitch.textChapters(text)
        let toChapters = toMedium == .audio ? EditionSwitch.audioChapters(view.audio) : EditionSwitch.textChapters(library.text(view) ?? "")
        let toKey = toMedium == .audio ? library.audioKey(view) : library.textKey(view)
        let plan = EditionSwitch.plan(editions: editions, from: fromId, to: editionId, fromMedium: fromMedium, toMedium: toMedium,
                                      fromChapters: fromChapters, toChapters: toChapters, toKey: toKey, offset: offset, length: length)
        let sourceKey = fromMedium == .audio ? library.audioKey(current) : library.textKey(current)
        let undo = PhoneLibrary.SwitchUndo(bookId: raw.id, fromEdition: fromId,
                                           fromPosition: ReadingPositionRef(key: sourceKey, unit: fromMedium.unit, offset: offset),
                                           targetKey: plan.position.key, targetRecord: library.record(plan.position.key), wasPlaying: wasPlaying)
        guard let changed = library.writeSwitch(bookId: raw.id, plan: plan, length: toChapters.length) else {
            error = "Der Sprachwechsel konnte nicht gespeichert werden."
            return
        }
        reopen(library.view(changed, editionId: editionId), medium: toMedium, autoplay: wasPlaying)
        guard book?.id == raw.id else { return }
        showNotice(LanguageNotice(text: plan.notice(languageName: target.languageName), undo: undo))
        Task { await LibrarySync.shared.run() }
    }

    /// „Rückgängig“ im Hinweis: Fassung und Stelle wie vor dem Wechsel.
    func undoLanguageSwitch() {
        guard let notice = languageNotice else { return }
        hideNotice()
        guard let changed = library.undoSwitch(notice.undo) else { error = "Das Rückgängigmachen ist nicht möglich."; return }
        let medium: SwitchMedium = notice.undo.fromPosition?.unit == "ms" ? .audio : .text
        reopen(library.view(changed, editionId: notice.undo.fromEdition), medium: medium, autoplay: notice.undo.wasPlaying)
        Task { await LibrarySync.shared.run() }
    }

    func hideNotice() {
        noticeTask?.cancel(); noticeTask = nil
        languageNotice = nil
    }

    private func showNotice(_ notice: LanguageNotice) {
        noticeTask?.cancel()
        languageNotice = notice
        UIAccessibility.post(notification: .announcement, argument: notice.text + " Rückgängig möglich.")
        noticeTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 10_000_000_000)
            guard !Task.isCancelled else { return }
            self?.languageNotice = nil
        }
    }

    /// Dieselbe Sicht neu öffnen, ohne den Player zu schließen: Ansicht, Schlaftimer und Quelle bleiben.
    private func reopen(_ view: LibraryBook, medium: SwitchMedium, autoplay: Bool) {
        let keep = surface
        let sleepUntil = sleepEnds
        stop()
        switch medium {
        case .audio: openAudiobook(view)
        case .text: openReadAloud(view)
        }
        guard book?.id == view.id else { return }
        surface = PlayerSurface.initial(stored: keep.rawValue, hasText: !paragraphs.isEmpty)
        UserDefaults.standard.set((medium == .audio ? PlayerSource.audiobook : PlayerSource.voice).rawValue, forKey: Self.sourceKey(view.id))
        if autoplay { play() }
        if let sleepUntil, sleepUntil > Date() { armSleep(until: sleepUntil) }
    }

    /// Liegen alle Teile des Hörbuchs als Datei vor?
    private func audioFilesPresent(_ book: LibraryBook) -> Bool {
        guard let audio = book.audio, !audio.allTracks.isEmpty else { return false }
        let dir = library.dir(book).appendingPathComponent("audio")
        return audio.allTracks.allSatisfy { FileManager.default.fileExists(atPath: dir.appendingPathComponent($0.file).path) }
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
        sentenceCache = [:]
        followCache = text.isEmpty ? nil : FollowMap(sync: audio.sync, textLength: Double(text.utf16.count), totalMs: totalMs)
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
        // Höchste Qualität beim Tempo (iOS-Standard ist .timeDomain, #314).
        item.audioTimePitchAlgorithm = .spectral
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
            sentenceCache = [:]
            followCache = nil
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
            // Stimme der Buchsprache erst jetzt: die Absätze des neuen Buchs stehen.
            applyVoicePref(book)
            prepareVoice()
        }
        if let fromParagraph { move(to: Cursor(paragraph: fromParagraph, sentence: 0)); if busy || autoplay { speakCurrent() }; return }
        if autoplay { play() }
    }

    private var phononVoice: String? { phononFailed ? nil : PhononEngine.voiceName(voiceIdentifier) }

    // MARK: Stimme je Buchsprache (lva#352)

    /// Gemerkte Stimme als Kennung dieses iPhones: eigene Kennung, sonst dieselbe Phonon-Stimme.
    private func voiceId(_ pref: VoicePref) -> String? {
        let catalog = PhononEngine.shared.voices
        if let id = pref.ios {
            if let phonon = PhononEngine.voiceName(id) {
                if let ok = VoicePrefs.phononIdentifier(phonon, catalog: catalog) { return ok }
            } else if AVSpeechSynthesisVoice(identifier: id) != nil { return id }
        }
        return VoicePrefs.phononIdentifier(pref.phonon, catalog: catalog)
    }

    /// `live`: wie eine eigene Wahl umschalten (laufende Lesung nimmt die neue Stimme);
    /// beim Öffnen nicht, dort bereitet `openReadAloud` die Stimme selbst vor.
    private func applyVoicePref(_ book: LibraryBook, live: Bool = false) {
        let lang = VoicePrefs.language(book.language)
        voiceLanguage = lang
        let owner = VoicePrefs.owner(book.id)
        let hit = VoicePrefs.resolve(owner: owner, language: lang, read: { self.library.voicePrefRaw($0) }, toId: voiceId)
        let hasBook = lang.flatMap { VoicePrefs.bookKey(owner, $0) }.map { VoicePrefs.isSet(library.voicePrefRaw($0)) } ?? false
        bookVoiceMissing = hasBook && hit?.source != .book
        // Fehlt die Buchstimme, bleibt die Ausnahme bestehen (Schalter an), es liest aber der Sprachstandard.
        voiceScope = hasBook ? .book : (lang == nil ? nil : .language)
        // Nichts gemerkt: eine Stimme der Buchsprache statt der Stimme des vorigen Buchs.
        guard let wanted = hit?.id ?? fallbackVoice(lang), wanted != voiceIdentifier else { return }
        if live { voiceIdentifier = wanted; return }
        applyingVoicePref = true
        voiceIdentifier = wanted
        applyingVoicePref = false
    }

    /// Passt die aktuelle Stimme nicht zur Sprache: Phonon dieser Sprache, sonst die beste Systemstimme.
    private func fallbackVoice(_ lang: String?) -> String? {
        guard let lang else { return nil }
        if let id = voiceIdentifier {
            // Phonon nur, wenn der Katalog die Stimme kennt; sonst bliebe eine ungültige Kennung aktiv.
            if let phonon = PhononEngine.voiceName(id) {
                if phonon.hasPrefix(lang + ":"), VoicePrefs.phononIdentifier(phonon, catalog: PhononEngine.shared.voices) != nil { return nil }
            } else if let v = AVSpeechSynthesisVoice(identifier: id), v.language.hasPrefix(lang) { return nil }
        }
        if let name = PhononEngine.shared.voices[lang]?.first { return PhononEngine.identifier(language: lang, voice: name) }
        return AVSpeechSynthesisVoice.speechVoices().filter { $0.language.hasPrefix(lang) }
            .max { $0.quality.rawValue < $1.quality.rawValue }?.identifier
    }

    /// Wahl in der Stimmenliste: gilt für die Sprache des Buchs (lva#352) bzw. nur das Buch.
    func chooseVoice(_ id: String) {
        voiceIdentifier = id
        rememberVoice()
        // Die gewählte Stimme gibt es: kein Ersatz-Hinweis mehr.
        bookVoiceMissing = false
    }

    private var voicePrefKey: String? {
        guard let book, let lang = voiceLanguage else { return nil }
        return voiceScope == .book ? VoicePrefs.bookKey(VoicePrefs.owner(book.id), lang) : VoicePrefs.languageKey(lang)
    }

    private func rememberVoice() {
        guard mode == .readAloud, let key = voicePrefKey, let id = voiceIdentifier else { return }
        let pref = VoicePrefs.iosPref(id: id, phonon: PhononEngine.voiceName(id), old: VoicePrefs.decode(library.voicePrefRaw(key)),
                                      now: PhoneLibrary.nowMs())
        if let raw = VoicePrefs.encode(pref) { library.setVoicePrefRaw(key, raw) }
    }

    /// „Nur dieses Buch“: eigene Stimme fürs Buch ein- oder ausschalten.
    func setBookOnlyVoice(_ on: Bool) {
        guard let book, let lang = voiceLanguage, let key = VoicePrefs.bookKey(VoicePrefs.owner(book.id), lang) else { return }
        if on {
            voiceScope = .book
            bookVoiceMissing = false
            rememberVoice()
        } else {
            // Leerer Eintrag statt Löschen: reist im Abgleich wie jede Wahl (neuerer Stand gewinnt).
            if let raw = VoicePrefs.encode(VoicePref(updated: PhoneLibrary.nowMs())) { library.setVoicePrefRaw(key, raw) }
            voiceScope = .language
            applyVoicePref(book, live: true)
        }
    }

    /// Systemstimmen in der Sprache des Buchs (sonst Deutsch), beste zuerst.
    var systemVoices: [AVSpeechSynthesisVoice] {
        let lang = voiceLanguage ?? "de"
        let list = AVSpeechSynthesisVoice.speechVoices().filter { $0.language.hasPrefix(lang) }
            .sorted { ($0.quality.rawValue, $0.name) > ($1.quality.rawValue, $1.name) }
        return list.isEmpty ? Self.germanVoices : list
    }

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
        spokenBefore = 0
        spokenSince = playing ? Date() : nil
        let ranges = sentenceRanges(cursor.paragraph)
        paragraphIndex = cursor.paragraph
        sentences = ranges
        sentenceIndex = ranges.isEmpty ? 0 : min(max(0, cursor.sentence), ranges.count - 1)
        sentenceRange = ranges.isEmpty ? nil : ranges[sentenceIndex]
        wordRange = nil
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

    private func nextSentence(_ generation: Int) -> PhononSentence? {
        guard generation == readGeneration, let c = nextToSynthesize else { return nil }
        nextToSynthesize = cursor(after: c)
        let ranges = sentenceRanges(c.paragraph)
        guard ranges.indices.contains(c.sentence) else { return nil }
        let text = (paragraphs[c.paragraph].text as NSString).substring(with: ranges[c.sentence])
        return PhononSentence(text: text, paragraphEnd: c.sentence == ranges.count - 1)
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

    /// Bibliothek geöffnet: Modell und Stimme schon laden, bevor ein Buch gewählt ist.
    /// Das erste Laden (Core ML) dauert einige Sekunden; danach beginnt Vorlesen sofort.
    func warmUp() {
        // Mit offenem Buch läuft schon die Vorschau der nächsten Sätze; nicht abbrechen.
        guard book == nil, !busy, phononTask == nil, let voice = phononVoice else { return }
        PhononEngine.shared.prepare(voice: voice)
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

    /// Belegkarte des Buch-Gesprächs: an eine Stelle im Originaltext springen (UTF-16-Offset). Die Stelle zählt
    /// nicht als gelesen. `autoplay`: dort vorlesen bzw. das Hörbuch abspielen („Ab hier vorlesen“).
    func jump(toOffset offset: Int, autoplay: Bool) {
        guard book != nil, let mode, !paragraphs.isEmpty else { return }
        if autoplay { leaveReadingForPlayback() }
        let target = max(0, offset)
        if mode == .audiobook {
            if let map = followMap { seek(toMs: map.ms(atChars: Double(target))) }
            if autoplay { play() }
            return
        }
        let index = LibraryText.paragraphIndex(paragraphs, at: Double(target))
        let local = max(0, target - paragraphs[index].textStart)
        let ranges = sentenceRanges(index)
        let here = Cursor(paragraph: index, sentence: LibraryText.sentenceIndex(ranges, at: local))
        move(to: here)
        saveText(at: chars(of: here), jumped: true)
        if autoplay { speakCurrent() } else if busy { speakCurrent() } else { discardPausedSpeech(); prepareVoice() }
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
        // Ein Sprung ist nicht gelesen: weder Bereiche noch Lesegrenze (Buch-Gespräch) rücken vor.
        if jumped { rec.heard = previous?.heard ?? []; rec.frontier = previous?.frontier; rec.finished = previous?.finished }
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
        leaveReadingForPlayback()
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
        playing = false; book = nil; mode = nil; sentenceRange = nil; wordRange = nil; sentences = []; info = nil
        sentenceCache = [:]; followCache = nil; resumeAfterReading = false
        cancelSleep()
        library.flush()
        MPNowPlayingInfoCenter.default().nowPlayingInfo = nil
    }

    func scene(active: Bool) { if !active { if mode == .audiobook { saveAudio() }; library.flush() } }

    private func applyRate() {
        if mode == .audiobook, playing { player?.rate = rate }
        if mode == .readAloud {
            // Phonon: Das Tempo steckt im erzeugten Ton, also am aktuellen Satz neu ansetzen
            // (aus dem Satzspeicher sofort). Pausiert gilt es beim Weiterlesen.
            PhononEngine.shared.setRate(rate)
            if phononTask != nil, phononPaused { discardPausedSpeech() } else if playing { speakCurrent() }
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

    /// Wiedergabe in „Lesen“ (Mini-Player, Sperrbildschirm, Beleg im Buch-Gespräch): sichtbar mitlesen.
    private func leaveReadingForPlayback() {
        guard surface == .read, let book else { return }
        surface = .readListen; resumeAfterReading = false
        UserDefaults.standard.set(PlayerSurface.readListen.rawValue, forKey: Self.surfaceKey(book.id))
    }

    private func armSleep(until end: Date) {
        sleepEnds = end
        sleepTimer = Timer.scheduledTimer(withTimeInterval: max(1, end.timeIntervalSinceNow), repeats: false) { [weak self] _ in
            MainActor.assumeIsolated { self?.pause(); self?.sleepEnds = nil }
        }
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
        c.playCommand.addTarget { [weak self] _ in
            MainActor.assumeIsolated { if self?.remoteIntercept?(.play) != true { self?.play() } }; return .success }
        c.pauseCommand.addTarget { [weak self] _ in
            MainActor.assumeIsolated { if self?.remoteIntercept?(.pause) != true { self?.pause() } }; return .success }
        c.togglePlayPauseCommand.addTarget { [weak self] _ in
            MainActor.assumeIsolated { if self?.remoteIntercept?(.toggle) != true { self?.toggle() } }; return .success }
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
            self.systemVoiceReached(range)
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
    fileprivate func systemVoiceReached(_ range: NSRange) {
        let location = range.location
        let word = NSRange(location: utteranceBase + location, length: range.length)
        let index = LibraryText.sentenceIndex(sentences, at: utteranceBase + location)
        guard paragraphs.indices.contains(paragraphIndex) else { return }
        if index != sentenceIndex {
            let here = Cursor(paragraph: paragraphIndex, sentence: index)
            move(to: here)
            saveText(at: chars(of: here))
        }
        wordRange = word
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
