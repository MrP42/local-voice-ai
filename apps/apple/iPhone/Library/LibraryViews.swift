import SwiftUI
import AVFoundation
import UniformTypeIdentifiers

// Bibliothek auf dem iPhone (Epic #210), nach der abgenommenen Vorschau
// `apps/local-voice/docs/vorschau/ios-bibliothek/`.

enum LibraryFormat {
    static func clock(_ ms: Double) -> String {
        let s = max(0, Int((ms / 1000).rounded(.down)))
        let h = s / 3600, m = (s % 3600) / 60, x = s % 60
        return h > 0 ? String(format: "%d:%02d:%02d", h, m, x) : String(format: "%d:%02d", m, x)
    }
    static func remaining(_ ms: Double) -> String {
        let minutes = max(0, Int((ms / 60_000).rounded()))
        return minutes >= 60 ? "\(minutes / 60) Std. \(minutes % 60) Min." : "\(minutes) Min."
    }
    /// Mit geschütztem Leerzeichen, damit „26 %“ nie umbricht.
    static func percent(_ f: Double) -> String { "\(Int((f * 100).rounded()))\u{00A0}%" }
    static func when(_ ms: Double) -> String {
        guard ms > 0 else { return "" }
        let f = RelativeDateTimeFormatter(); f.locale = Locale(identifier: "de_DE"); f.unitsStyle = .full
        return f.localizedString(for: Date(timeIntervalSince1970: ms / 1000), relativeTo: Date())
    }
}

/// Erzeugtes Cover aus Titel (wie am Desktop ohne Netz).
struct LibraryCover: View {
    let title: String
    var compact = false
    /// Echtes Cover (aus der Hörbuchdatei, EPUB oder vom Desktop); sonst erzeugt.
    var image: UIImage? = nil
    /// Abstand des Titels oben, wenn dort etwas aufliegt (Stern im Regal).
    var titleTopInset: CGFloat = 0
    private static let colors: [(Color, Color)] = [
        (Color(red: 0.18, green: 0.29, blue: 0.24), Color(red: 0.95, green: 0.91, blue: 0.82)),
        (Color(red: 0.54, green: 0.23, blue: 0.18), Color(red: 0.98, green: 0.91, blue: 0.86)),
        (Color(red: 0.13, green: 0.23, blue: 0.36), Color(red: 0.87, green: 0.91, blue: 0.96)),
        (Color(red: 0.79, green: 0.64, blue: 0.15), Color(red: 0.16, green: 0.13, blue: 0.03)),
        (Color(red: 0.36, green: 0.27, blue: 0.47), Color(red: 0.94, green: 0.91, blue: 0.97)),
    ]
    var body: some View {
        if let image { photo(image) } else { generated }
    }

    /// Quadratische Hörbuch-Cover passen ohne Beschnitt in jede Form:
    /// unscharfer Hintergrund aus demselben Bild, darüber das ganze Cover.
    private func photo(_ image: UIImage) -> some View {
        Image(uiImage: image).resizable().scaledToFit()
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background { if !compact { Image(uiImage: image).resizable().scaledToFill().blur(radius: 14).opacity(0.85) } }
            .background(Color.black)
        .clipShape(RoundedRectangle(cornerRadius: compact ? 4 : 6))
        .accessibilityHidden(true)
    }

    @ViewBuilder private var generated: some View {
        let pick = Self.colors[abs(title.unicodeScalars.reduce(0) { ($0 &* 31) &+ Int($1.value) }) % Self.colors.count]
        RoundedRectangle(cornerRadius: compact ? 4 : 6).fill(pick.0)
            .overlay(alignment: .topLeading) {
                if !compact {
                    Text(title).font(.system(.footnote, design: .serif).weight(.bold)).foregroundStyle(pick.1)
                        .lineLimit(5).minimumScaleFactor(0.6).padding(8).padding(.top, titleTopInset)
                }
            }
            .clipped()
            .accessibilityHidden(true)
    }
}

enum ShelfFilter: String, CaseIterable, Identifiable {
    case all = "Alle", favorites = "Favoriten", inProgress = "In Arbeit", audio = "Hörbuch", finished = "Fertig"
    var id: String { rawValue }
}

struct LibraryTab: View {
    @ObservedObject var library = PhoneLibrary.shared
    @ObservedObject var playback = LibraryPlayback.shared

    var body: some View {
        NavigationStack { LibraryShelf() }
            .onAppear { playback.warmUp() }
            .safeAreaInset(edge: .bottom, spacing: 0) {
                if playback.book != nil && !playback.presented { LibraryMiniPlayer().padding(.horizontal, 8).padding(.bottom, 6) }
            }
            .fullScreenCover(isPresented: $playback.presented) { LibraryPlayerScreen() }
            #if DEBUG
            .task {
                // Diagnose-Startargumente für Simulator-Bilder, kein Produktpfad.
                let args = ProcessInfo.processInfo.arguments
                if let i = args.firstIndex(of: "--pair-url"), i + 1 < args.count, let url = URL(string: args[i + 1]) { PairingFlow.shared.open(url) }
                if args.contains("--phonon-probe") {
                    // Geräteprobe ohne Bedienung: Ergebnis landet in Documents/phonon-probe.txt.
                    let out = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("phonon-probe.txt")
                    await PhononEngine.shared.refresh()
                    var log = "Stimmen: \(PhononEngine.shared.voices.mapValues(\.count))\n"
                    for (language, sentence) in [("de", "Guten Tag. Das ist Phonon auf dem iPhone."), ("en", "Hello. This is Phonon on the iPhone.")] {
                        guard let voice = PhononEngine.shared.voices[language]?.first else { continue }
                        let start = Date()
                        do {
                            try await PhononEngine.shared.speak(sentence, voice: "\(language):\(voice)")
                            log += "\(language) \(voice): \(Date().timeIntervalSince(start)) s\n"
                        } catch { log += "\(language) FEHLER \(error)\n" }
                    }
                    try? log.write(to: out, atomically: true, encoding: .utf8)
                }
                if let i = args.firstIndex(of: "--sync-test"), i + 1 < args.count, LibrarySync.shared.account == nil {
                    await LibrarySync.shared.login(email: "test@example.com", password: "geheim", url: args[i + 1], deviceName: "Simulator")
                }
                if args.contains("--sync-test") { await LibrarySync.shared.run() }
                guard var book = library.books.first else { return }
                if args.contains("--library-listen") {
                    if !library.hasAudio(book) {
                        guard await LibrarySync.shared.downloadAudio(book), let fresh = library.books.first(where: { $0.id == book.id }) else { return }
                        book = fresh
                    }
                    playback.open(book, autoplay: true)
                    if playback.source != .audiobook { playback.switchSource(to: .audiobook) }
                }
                if args.contains("--library-read") {
                    playback.open(book, autoplay: true)
                    if playback.source != .voice { playback.switchSource(to: .voice) }
                    playback.show(.readListen)
                }
                // Ansicht für Simulator-Bilder erzwingen: --player-surface listen|readListen|read (nach --library-read/-listen).
                if let i = args.firstIndex(of: "--player-surface"), i + 1 < args.count, let surface = PlayerSurface(rawValue: args[i + 1]) {
                    playback.show(surface)
                }
            }
            #endif
            .alert("Bibliothek", isPresented: Binding(get: { playback.error != nil }, set: { if !$0 { playback.error = nil } })) {
                Button("OK", role: .cancel) {}
            } message: { Text(playback.error ?? "") }
    }
}

struct LibraryShelf: View {
    @ObservedObject var library = PhoneLibrary.shared
    @ObservedObject var playback = LibraryPlayback.shared
    @State private var query = ""
    @State private var filter = ShelfFilter.all
    @Environment(\.dynamicTypeSize) private var typeSize
    @State private var importing = false
    @State private var showSync = false
    @ObservedObject var sync = LibrarySync.shared

    private func textFraction(_ b: LibraryBook) -> Double { LibraryProgress.fraction(library.record(library.textKey(b))) }
    private func audioFraction(_ b: LibraryBook) -> Double { LibraryProgress.fraction(library.record(library.audioKey(b))) }

    private var visible: [LibraryBook] {
        let q = query.trimmingCharacters(in: .whitespaces)
        return LibraryText.favoritesFirst(library.books.filter { b in
            let matches = q.isEmpty || b.title.localizedCaseInsensitiveContains(q) || b.authors.contains { $0.localizedCaseInsensitiveContains(q) }
            let done = b.finished || max(textFraction(b), audioFraction(b)) >= 1
            switch filter {
            case .all: return matches
            case .favorites: return matches && b.favorite
            case .inProgress: return matches && !done && max(textFraction(b), audioFraction(b)) > 0
            case .audio: return matches && b.audio != nil
            case .finished: return matches && done
            }
        })
    }

    /// In sehr großer Schrift steht das Cover über dem Text.
    private var resumeLayout: AnyLayout {
        typeSize.isAccessibilitySize ? AnyLayout(VStackLayout(alignment: .leading, spacing: 10)) : AnyLayout(HStackLayout(alignment: .top, spacing: 14))
    }

    /// Angaben der Karte „Weiterhören/Weiterlesen“: Stand, Rest und Umfang.
    private func resumeFacts(_ book: LibraryBook, _ rec: ProgressRecord, isAudio: Bool) -> [String] {
        // Stelle und Rest aus demselben Wert: Gehörte Bereiche können weiter reichen als die aktuelle Stelle.
        let left = max(0, rec.length - rec.offset), percent = LibraryFormat.percent(rec.length > 0 ? min(1, rec.offset / rec.length) : 0)
        let load = library.hasAudio(book) ? "geladen" : "im Konto"
        guard isAudio else {
            var facts = ["Text · \(percent)", "Vorlesen: noch ca. \(LibraryFormat.remaining(LibraryText.listenMilliseconds(chars: left))) bei Normaltempo"]
            if book.audio != nil {
                let heard = LibraryFormat.percent(LibraryProgress.fraction(library.record(library.audioKey(book))))
                facts.append("Hörbuch · \(heard) · \(load)")
            }
            return facts
        }
        var facts = ["Hörbuch · \(percent) · noch \(LibraryFormat.remaining(left))"]
        if let chapters = library.cachedAudiobookInfo(book)?.chapters, chapters.count > 1 {
            facts.append("Kapitel \(AudiobookChapters.index(at: rec.offset, in: chapters) + 1) von \(chapters.count) · Gesamt \(LibraryFormat.remaining(rec.length))")
        }
        facts.append(load.prefix(1).uppercased() + load.dropFirst())
        return facts
    }

    /// Zuletzt gehörtes oder gelesenes, noch nicht fertiges Buch.
    private var resume: (LibraryBook, ProgressRecord, Bool)? {
        var best: (LibraryBook, ProgressRecord, Bool)?
        for b in library.books where !b.finished {
            for (key, isAudio) in [(library.audioKey(b), true), (library.textKey(b), false)] {
                guard let r = library.record(key), r.offset > 0, r.finished != true else { continue }
                if best == nil || r.updated > best!.1.updated { best = (b, r, isAudio) }
            }
        }
        return best
    }

    var body: some View {
        let shown = visible  // einmal je Durchlauf, das Regal zeichnet beim Hören laufend neu
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack(spacing: 10) {
                    Image(systemName: "magnifyingglass").foregroundStyle(VoicePalette.secondaryText)
                    TextField("Titel oder Autor suchen", text: $query).submitLabel(.search)
                }
                .padding(.horizontal, 12).frame(minHeight: 44)
                .background(VoicePalette.border.opacity(0.6), in: RoundedRectangle(cornerRadius: 12))

                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        ForEach(ShelfFilter.allCases) { f in
                            Button(f.rawValue) { filter = f }
                                .font(.subheadline.weight(filter == f ? .semibold : .regular))
                                .padding(.horizontal, 14).frame(minHeight: 44)
                                .foregroundStyle(filter == f ? Color.white : VoicePalette.text)
                                .background(filter == f ? VoicePalette.ink : VoicePalette.border.opacity(0.6), in: Capsule())
                                .accessibilityAddTraits(filter == f ? .isSelected : [])
                        }
                    }
                }

                if let (book, rec, isAudio) = resume, query.isEmpty, filter == .all {
                    let facts = resumeFacts(book, rec, isAudio: isAudio)
                    Text(isAudio ? "WEITERHÖREN" : "WEITERLESEN").font(.caption.weight(.semibold)).tracking(0.6).foregroundStyle(VoicePalette.secondaryText)
                    NavigationLink { LibraryBookDetail(bookId: book.id) } label: {
                        resumeLayout {
                            LibraryCover(title: book.title, image: library.coverThumbnail(book)).frame(width: 64, height: 96)
                            VStack(alignment: .leading, spacing: 3) {
                                Text(book.title).font(.headline).fixedSize(horizontal: false, vertical: true)
                                if !book.authors.isEmpty { Text(book.authors.joined(separator: ", ")).font(.subheadline).foregroundStyle(VoicePalette.secondaryText).fixedSize(horizontal: false, vertical: true) }
                                ForEach(facts, id: \.self) { fact in
                                    Text(fact).font(.footnote).foregroundStyle(VoicePalette.secondaryText).fixedSize(horizontal: false, vertical: true)
                                }
                                ProgressView(value: LibraryProgress.fraction(rec)).tint(VoicePalette.accent).accessibilityHidden(true)
                            }
                        }
                        .accessibilityElement(children: .ignore)
                        .accessibilityLabel(([book.title] + book.authors + facts).joined(separator: ", "))
                        .voiceCard()
                    }.buttonStyle(.plain)
                }

                Text("ALLE BÜCHER · \(shown.count)").font(.caption.weight(.semibold)).tracking(0.6).foregroundStyle(VoicePalette.secondaryText)
                if library.anyAudioDownloaded {
                    HStack(alignment: .top, spacing: 4) {
                        DownloadedMark()
                        Text("Hörbuch heruntergeladen, offline verfügbar").font(.caption).foregroundStyle(VoicePalette.secondaryText)
                            .fixedSize(horizontal: false, vertical: true)
                    }.accessibilityHidden(true)
                }
                if library.books.isEmpty {
                    ContentUnavailableView {
                        Label("Noch keine Bücher", systemImage: "books.vertical")
                    } description: {
                        Text("Füge EPUB, PDF, TXT oder ein Hörbuch (MP3, M4A, M4B) aus der Dateien-App hinzu. Mehrere Audiodateien zusammen werden ein Hörbuch mit Teilen.")
                    } actions: { Button("Buch hinzufügen") { importing = true }.buttonStyle(.borderedProminent) }
                }
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 100), spacing: 12)], alignment: .leading, spacing: 16) {
                    ForEach(shown) { book in
                        NavigationLink { LibraryBookDetail(bookId: book.id) } label: { ShelfItem(book: book) }.buttonStyle(.plain)
                            .overlay(alignment: .topLeading) { FavoriteStar(book: book) }
                            .overlay(alignment: .topTrailing) { ShelfPlayButton(book: book) }
                    }
                }
                if library.importing { ProgressView("Importiere …") }
                if let error = library.error { Text(error).font(.footnote).foregroundStyle(.red) }
            }.padding(16)
        }
        .modifier(MiniPlayerClearance())
        .background(VoicePalette.background)
        .navigationTitle("Bibliothek")
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button("Buch oder Hörbuch hinzufügen", systemImage: "plus") { importing = true }.accessibilityIdentifier("libraryAdd")
            }
            ToolbarItem(placement: .topBarTrailing) { SettingsButton() }
            ToolbarItem(placement: .topBarLeading) {
                Button { showSync = true } label: {
                    Image(systemName: sync.account == nil ? "icloud.slash" : (sync.error != nil ? "exclamationmark.icloud" : "checkmark.icloud"))
                }.accessibilityLabel("Konto und Sync: \(sync.status)")
            }
        }
        .fileImporter(isPresented: $importing, allowedContentTypes: PhoneLibrary.importTypes, allowsMultipleSelection: true) { result in
            if case .success(let urls) = result { Task { await library.importFiles(urls); await sync.run(); if await library.fillMissingCovers() { await sync.run() } } }
        }
        .refreshable { await sync.run(); library.reload(); if await library.fillMissingCovers() { await sync.run() } }
        .task { await sync.run(); if await library.fillMissingCovers() { await sync.run() } }
        .sheet(isPresented: $showSync) { LibrarySyncSheet() }
    }
}

struct ShelfItem: View {
    let book: LibraryBook
    @ObservedObject var library = PhoneLibrary.shared
    @ObservedObject var downloads = AudioDownloads.shared
    @Environment(\.dynamicTypeSize) private var typeSize
    var body: some View {
        let audio = library.hasAudio(book)
        let rec = audio || library.audioInAccount(book) ? library.record(library.audioKey(book)) : library.record(library.textKey(book))
        VStack(alignment: .leading, spacing: 5) {
            LibraryCover(title: book.title, image: library.coverThumbnail(book), titleTopInset: 24).aspectRatio(2/3, contentMode: .fit)
            if downloads.jobs[book.id] != nil {
                ShelfDownloadStatus(book: book)
            } else {
            ProgressView(value: book.finished ? 1 : LibraryProgress.fraction(rec)).tint(VoicePalette.accent)
            // Fester Platz für das Symbol: Lange Angaben brechen um und rücken nicht darunter.
            metaLayout {
                Text(audio || library.audioInAccount(book) ? (rec == nil ? "Hörbuch · neu" : "Hörbuch · \(LibraryFormat.percent(LibraryProgress.fraction(rec)))")
                           : "Text · \(LibraryFormat.percent(LibraryProgress.fraction(rec)))")
                    .font(.caption).foregroundStyle(VoicePalette.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: typeSize.isAccessibilitySize ? nil : .infinity, alignment: .leading)
                if audio || !typeSize.isAccessibilitySize { DownloadedMark(visible: audio) }
            }
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(([book.title] + book.authors + (audio ? ["Hörbuch heruntergeladen, offline verfügbar"] : [])).joined(separator: ", "))
        .accessibilityValue(downloads.jobs[book.id] != nil ? ShelfDownloadStatus.line(book, downloads) : "")
    }

    /// In sehr großer Schrift untereinander, sonst bliebe dem Text kaum Breite.
    private var metaLayout: AnyLayout {
        typeSize.isAccessibilitySize ? AnyLayout(VStackLayout(alignment: .leading, spacing: 2)) : AnyLayout(HStackLayout(alignment: .top, spacing: 4))
    }
}

/// Grüner Kreis mit Pfeil nach unten: Das Hörbuch liegt auf dem iPhone. Der Platz bleibt auch ohne Symbol frei.
struct DownloadedMark: View {
    var visible = true
    @ScaledMetric(relativeTo: .caption) private var size: CGFloat = 15

    var body: some View {
        Image(systemName: "arrow.down.circle.fill")
            .resizable().scaledToFit().frame(width: size, height: size)
            .foregroundStyle(VoicePalette.success)
            .opacity(visible ? 1 : 0)
            .accessibilityHidden(true)
    }
}

/// Kurzbuch auf der Buchseite. Es fasst das ganze Buch zusammen, deshalb einmal je Buch ein Hinweis.
struct KurzbuchRow: View {
    let book: LibraryBook
    let ref: KurzbuchRef
    let showLanguage: Bool
    @ObservedObject var library = PhoneLibrary.shared
    @ObservedObject var sync = LibrarySync.shared
    @ObservedObject var playback = LibraryPlayback.shared
    @State private var warning = false

    private var seenKey: String { "kurzbuch.gesehen." + book.id }
    private var title: String { "Kurzbuch" + (showLanguage ? " · \(ref.language ?? "")" : "") }
    private var origin: String {
        var parts: [String] = []
        if let ms = ref.createdMs { parts.append(Date(timeIntervalSince1970: ms / 1000).formatted(date: .numeric, time: .omitted)) }
        if let source = ref.source, !source.isEmpty { parts.append(source) }
        if let model = ref.model, !model.isEmpty { parts.append(model) }
        return parts.joined(separator: " · ")
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            if library.kurzbuchReady(book, ref) {
                Button {
                    if UserDefaults.standard.bool(forKey: seenKey) { open() } else { warning = true }
                } label: {
                    Label(title, systemImage: "text.book.closed").frame(maxWidth: .infinity, minHeight: 44)
                }.buttonStyle(.bordered)
            } else {
                HStack {
                    Label(sync.running ? "\(title) wird geladen" : "\(title) noch nicht geladen", systemImage: "text.book.closed")
                        .font(.subheadline).foregroundStyle(VoicePalette.secondaryText)
                    Spacer()
                    if sync.running { ProgressView() }
                    else { Button("Erneut laden") { Task { await sync.run() } }.frame(minHeight: 44) }
                }
            }
            if !origin.isEmpty { Text(origin).font(.caption).foregroundStyle(VoicePalette.secondaryText).fixedSize(horizontal: false, vertical: true) }
        }
        .confirmationDialog("Das Kurzbuch fasst das ganze Buch zusammen und kann das Ende verraten.", isPresented: $warning, titleVisibility: .visible) {
            Button("Kurzbuch öffnen") { UserDefaults.standard.set(true, forKey: seenKey); open() }
            Button("Abbrechen", role: .cancel) {}
        }
    }

    private func open() {
        if let entry = library.kurzbuchBook(book, ref) { playback.open(entry) }
    }
}

/// Solange ein Hörbuch lädt, ersetzt das die Fortschrittszeile unter dem Cover.
struct ShelfDownloadStatus: View {
    let book: LibraryBook
    @ObservedObject var downloads = AudioDownloads.shared
    @ObservedObject var sync = LibrarySync.shared  // Netzwechsel ändert die Zeile auch ohne neue Bytes

    var body: some View {
        Text(Self.line(book, downloads))
            .font(.caption).foregroundStyle(VoicePalette.secondaryText)
            .fixedSize(horizontal: false, vertical: true)
    }

    static func line(_ book: LibraryBook, _ downloads: AudioDownloads) -> String {
        let job = downloads.jobs[book.id]
        let total = max(1, job?.totalBytes ?? 1), bytes = downloads.done[book.id] ?? 0
        let percent = LibraryFormat.percent(min(1, Double(bytes) / Double(total)))
        switch job?.phase {
        case .failed: return "Laden fehlgeschlagen"
        case .paused: return "Angehalten · \(percent)"
        default:
            if downloads.waitsForWifi(book.id) { return "Wartet auf WLAN · \(percent)" }
            let rest = downloads.remainingSeconds(book.id).map { " · " + AudioDownload.remainingText($0) } ?? ""
            return "Lädt · \(percent)\(rest)"
        }
    }
}

/// Startet direkt aus dem Regal: Hörbuch, wenn es geladen ist, sonst Vorlesen. Der Mini-Player übernimmt.
struct ShelfPlayButton: View {
    let book: LibraryBook
    @ObservedObject var library = PhoneLibrary.shared

    var body: some View {
        if library.hasAudio(book) || library.hasText(book) { ShelfPlayControl(book: book) }
    }
}

/// Eigene Ansicht: Die Position ändert sich beim Hören laufend, das soll keine Dateien prüfen.
private struct ShelfPlayControl: View {
    let book: LibraryBook
    @ObservedObject var playback = LibraryPlayback.shared

    var body: some View {
        let current = playback.book?.id == book.id
        Button {
            if current { playback.toggle() } else { playback.open(book, autoplay: true); playback.presented = false }
        } label: {
            Group {
                if current && playback.preparing { ProgressView().controlSize(.small).tint(.white) }
                else { Image(systemName: current && playback.playing ? "pause.fill" : "play.fill").font(.system(size: 13, weight: .semibold)) }
            }
            .foregroundStyle(Color.white)
            .frame(width: 26, height: 26).background(.black.opacity(0.45), in: Circle())
            .padding(4).frame(width: 44, height: 44, alignment: .topTrailing).contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("\(current && playback.busy ? "Pause" : "Abspielen"): \(book.title)")
    }
}

/// Stern am Cover; wie am PC stehen Favoriten oben im Regal. Der Abgleich nimmt die Wahl mit.
struct FavoriteStar: View {
    let book: LibraryBook
    @ObservedObject var library = PhoneLibrary.shared

    var body: some View {
        Button {
            LibrarySync.shared.setFavorite(book, !book.favorite)
            Task { await LibrarySync.shared.run() }
        } label: {
            Image(systemName: book.favorite ? "star.fill" : "star")
                .font(.system(size: 13, weight: .semibold)).foregroundStyle(book.favorite ? Color.yellow : Color.white)
                .frame(width: 26, height: 26).background(.black.opacity(0.45), in: Circle())
                .padding(4).frame(width: 44, height: 44, alignment: .topLeading).contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(book.favorite ? "Aus Favoriten entfernen: \(book.title)" : "Zu Favoriten hinzufügen: \(book.title)")
    }
}

struct LibraryBookDetail: View {
    let bookId: String
    @ObservedObject var library = PhoneLibrary.shared
    @ObservedObject var playback = LibraryPlayback.shared
    @Environment(\.dismiss) private var dismiss
    @State private var confirmDelete = false
    @State private var confirmRemoveAudio = false
    /// Warum das Hörbuch (noch) nicht vom iPhone darf.
    @State private var offloadBlocked: AudioOffload.Verdict?
    @State private var removingAudio = false
    @State private var removeBytes = 0
    @ObservedObject var sync = LibrarySync.shared
    @ObservedObject var downloads = AudioDownloads.shared

    var body: some View {
        if let book = library.books.first(where: { $0.id == bookId }) { content(book) }
        else { ContentUnavailableView("Buch nicht mehr vorhanden", systemImage: "book.closed") }
    }

    @ViewBuilder private func content(_ book: LibraryBook) -> some View {
        let hasText = library.hasText(book), hasAudio = library.hasAudio(book), inAccount = library.audioInAccount(book)
        let audioRec = library.record(library.audioKey(book)), textRec = library.record(library.textKey(book))
        let tracks = book.audio?.allTracks ?? []
        let total = book.audio?.totalDurationMs ?? 0
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack(alignment: .top, spacing: 16) {
                    LibraryCover(title: book.title, image: library.coverImage(book)).frame(width: 96, height: 144)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(book.title).font(.title2.bold())
                        if !book.authors.isEmpty { Text(book.authors.joined(separator: ", ")).font(.body) }
                        if hasText { Text("Text: \(book.text?.sourceName ?? "vorhanden")").font(.footnote).foregroundStyle(VoicePalette.secondaryText) }
                        if hasAudio { Text("Hörbuch: \(LibraryFormat.clock(total)) · \(tracks.count) \(tracks.count == 1 ? "Teil" : "Teile")").font(.footnote).foregroundStyle(VoicePalette.secondaryText) }
                        if !inAccount, hasText || hasAudio {
                            Label("Auf dem iPhone", systemImage: "checkmark").font(.footnote.weight(.semibold)).foregroundStyle(VoicePalette.success)
                        }
                    }
                }
                // Ein Knopf für alles: Hören, Mitlesen und Lesen wählt man im Player.
                Button {
                    Task {
                        if !hasText, !library.hasAudio(book) { guard await sync.downloadAudio(book) else { return } }
                        guard let fresh = library.books.first(where: { $0.id == book.id }) else { return }
                        playback.open(fresh)
                    }
                } label: {
                    Label(audioRec != nil || textRec != nil ? "Weiter" : "Starten", systemImage: "play.fill")
                        .frame(maxWidth: .infinity, minHeight: 44)
                }
                .buttonStyle(.borderedProminent).tint(VoicePalette.ink).foregroundStyle(VoicePalette.brand).font(.headline)
                .disabled(!hasText && downloads.isActive(book.id)).accessibilityIdentifier("openBook")
                .accessibilityHint(hasText ? "Öffnet den Player. Hören, Lesen und Hören oder Lesen wählst du dort." : "Spielt das Hörbuch ab.")

                if inAccount || downloads.jobs[book.id] != nil || (hasAudio && !library.meta(book).files.audio.isEmpty) {
                    AudioDownloadCard(book: book, onRemove: { askRemoveAudio($0) })
                }

                if hasAudio || hasText {
                    VStack(alignment: .leading, spacing: 12) {
                        if hasAudio {
                            progressRow("Hörbuch", value: LibraryProgress.fraction(audioRec),
                                        detail: audioRec.map { r in
                                            let pos = LibraryTracks.position(of: r.offset, in: tracks)
                                            return "Teil \(pos.index + 1) · \(LibraryFormat.clock(r.offset)) von \(LibraryFormat.clock(total))"
                                        } ?? "noch nicht gehört", when: audioRec?.updated)
                        }
                        if hasText {
                            progressRow("Text", value: LibraryProgress.fraction(textRec),
                                        detail: "\(LibraryFormat.percent(LibraryProgress.fraction(textRec))) gelesen", when: textRec?.updated)
                        }
                    }.voiceCard()
                }

                ForEach(library.kurzbuecher(book), id: \.blob) { ref in
                    KurzbuchRow(book: book, ref: ref, showLanguage: library.kurzbuecher(book).count > 1)
                }

                if tracks.count > 1 {
                    Text("HÖRBUCH-TEILE").font(.caption.weight(.semibold)).tracking(0.6).foregroundStyle(VoicePalette.secondaryText)
                    let current = audioRec.map { LibraryTracks.position(of: $0.offset, in: tracks).index }
                    ForEach(Array(tracks.enumerated()), id: \.offset) { i, t in
                        Button {
                            playback.open(book, prefer: .audiobook)
                            guard playback.book?.id == book.id, playback.source == .audiobook else { return }
                            playback.seek(toMs: LibraryTracks.total(TrackPosition(index: i, offsetMs: 0), in: tracks))
                        } label: {
                            HStack(spacing: 12) {
                                Text("Teil \(i + 1) · \(t.title)").lineLimit(2)
                                Spacer()
                                Text(i == current ? "läuft zuletzt" : LibraryFormat.clock(t.durationMs ?? 0)).foregroundStyle(VoicePalette.secondaryText)
                            }.frame(minHeight: 44).fontWeight(i == current ? .semibold : .regular)
                        }.buttonStyle(.plain)
                        Divider()
                    }
                }
            }.padding(16)
        }
        .modifier(MiniPlayerClearance())
        .background(VoicePalette.background)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Menu("Mehr", systemImage: "ellipsis.circle") {
                    Button(book.finished ? "Als nicht fertig markieren" : "Als fertig markieren", systemImage: "checkmark.circle") { library.setFinished(book, !book.finished) }
                    if library.hasLocalAudio(book) {
                        Button("Hörbuch vom iPhone entfernen", systemImage: "icloud.and.arrow.down") { askRemoveAudio(book) }
                            .disabled(removingAudio)
                    }
                    if sync.isSynced(book) {
                        Button("Auf allen Geräten löschen", systemImage: "trash", role: .destructive) { confirmDelete = true }
                    } else {
                        Button("Vom iPhone entfernen", systemImage: "trash", role: .destructive) { confirmDelete = true }
                    }
                }
            }
        }
        .confirmationDialog("Hörbuch von diesem iPhone entfernen?", isPresented: $confirmRemoveAudio, titleVisibility: .visible) {
            Button("\(ByteCountFormatter.string(fromByteCount: Int64(removeBytes), countStyle: .file)) freigeben", role: .destructive) {
                removingAudio = true
                Task {
                    // Prüft vorher beim Konto, ob alle Teile vollständig dort liegen.
                    let result = await sync.removeDownloads(book)
                    removingAudio = false
                    if !result.verdict.allowed { offloadBlocked = result.verdict }
                }
            }
        } message: { Text("Es bleibt im Konto und auf dem PC und wird beim nächsten Hören wieder geladen. Text und Fortschritt bleiben erhalten.") }
        .alert("Hörbuch bleibt auf dem iPhone", isPresented: Binding(get: { offloadBlocked != nil }, set: { if !$0 { offloadBlocked = nil } }),
               presenting: offloadBlocked) { verdict in
            if verdict.syncHelps && sync.account != nil {
                Button("Jetzt abgleichen") { Task { await sync.run() } }
            }
            Button("OK", role: .cancel) {}
        } message: { verdict in Text(verdict.message) }
        .confirmationDialog(sync.isSynced(book) ? "„\(book.title)“ auf allen Geräten löschen?" : "„\(book.title)“ vom iPhone entfernen?",
                            isPresented: $confirmDelete, titleVisibility: .visible) {
            Button("Löschen", role: .destructive) {
                if playback.book?.id == book.id { playback.stop() }
                if sync.isSynced(book) { Task { await sync.deleteEverywhere(book); dismiss() } }
                else { library.delete(book); dismiss() }
            }
        } message: {
            Text(sync.isSynced(book)
                 ? "Das Buch verschwindet aus dem Konto, vom PC und von allen Geräten."
                 : "Text, Hörbuch und Fortschritt auf diesem iPhone werden gelöscht.")
        }
    }

    private func askRemoveAudio(_ book: LibraryBook) {
        let verdict = sync.offloadVerdict(book)
        switch verdict {
        case .allowed, .unconfirmed:  // wird beim Bestätigen frisch beim Konto geprüft
            removeBytes = library.localAudioBytes(book)
            confirmRemoveAudio = true
        default: offloadBlocked = verdict
        }
    }

    private func progressRow(_ title: String, value: Double, detail: String, when: Double?) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack { Text(title).fontWeight(.semibold); Spacer(); Text(detail).foregroundStyle(VoicePalette.secondaryText) }.font(.subheadline)
            ProgressView(value: value).tint(VoicePalette.accent)
            if let when, when > 0 { Text("Zuletzt auf diesem iPhone \(LibraryFormat.when(when))").font(.caption).foregroundStyle(VoicePalette.secondaryText) }
        }
    }
}

/// Ladezustand des Hörbuchs auf der Buchseite (Epic #210, P8 Paket 3), wie in
/// der Vorschau `ios-bibliothek/06-ladezustaende`.
struct AudioDownloadCard: View {
    let book: LibraryBook
    var onRemove: (LibraryBook) -> Void
    @ObservedObject var library = PhoneLibrary.shared
    @ObservedObject var sync = LibrarySync.shared
    @ObservedObject var downloads = AudioDownloads.shared
    @State private var confirmCancel = false

    var body: some View {
        let job = downloads.jobs[book.id]
        let total = job?.totalBytes ?? library.meta(book).files.audioBytes
        let done = downloads.done[book.id] ?? 0
        let amount = "\(LibraryBytes.text(done)) von \(LibraryBytes.text(total))"
        VStack(alignment: .leading, spacing: 8) {
            if let job, job.phase == .failed {
                Label("Laden fehlgeschlagen", systemImage: "exclamationmark.circle").font(.headline).foregroundStyle(VoicePalette.danger)
                Text(job.message ?? "Bereits geladene Teile bleiben erhalten, es geht an derselben Stelle weiter.")
                    .font(.subheadline).foregroundStyle(VoicePalette.text)
                actions(action("Erneut versuchen", "arrow.clockwise") { downloads.start(book) }, cancel)
            } else if let job, job.phase == .paused {
                header("Pausiert", amount)
                ProgressView(value: fraction(done, total)).tint(VoicePalette.secondaryText)
                Text("Bereits geladene Teile bleiben erhalten. „Fortsetzen“ lädt nur den Rest.")
                    .font(.caption).foregroundStyle(VoicePalette.secondaryText)
                actions(action("Fortsetzen", "play") { downloads.start(book) }, cancel)
            } else if job != nil, downloads.waitsForWifi(book.id) {
                header("Wartet auf WLAN", amount)
                Text("„Nur im WLAN laden“ ist an. Das Laden startet, sobald du im WLAN bist.")
                    .font(.subheadline).foregroundStyle(VoicePalette.secondaryText)
                actions(action("Jetzt mobil laden", "antenna.radiowaves.left.and.right") { downloads.useCellular(book) }, cancel)
            } else if job != nil {
                header("Hörbuch lädt … \(LibraryFormat.percent(fraction(done, total)))", amount)
                ProgressView(value: fraction(done, total)).tint(VoicePalette.accent)
                Text("Lädt auch weiter, wenn die App im Hintergrund ist.").font(.caption).foregroundStyle(VoicePalette.secondaryText)
                actions(action("Pausieren", "pause") { downloads.pause(book.id) }, cancel)
            } else if library.hasAudio(book) {
                Label("Auf dem iPhone · \(LibraryBytes.text(library.localAudioBytes(book)))", systemImage: "checkmark")
                    .font(.headline).foregroundStyle(VoicePalette.success)
                action("Vom iPhone entfernen", "icloud.and.arrow.down") { onRemove(book) }
                Text("Bleibt im Konto und auf dem PC, Fortschritt bleibt erhalten.").font(.caption).foregroundStyle(VoicePalette.secondaryText)
            } else {
                header("Hörbuch im Konto", LibraryBytes.text(total))
                Text(sync.wifiOnly && sync.expensiveNetwork ? "Wartet auf WLAN („Nur im WLAN laden“ ist an)." : "Wird beim ersten Hören geladen und ist danach offline verfügbar.")
                    .font(.caption).foregroundStyle(VoicePalette.secondaryText)
                action("Jetzt laden", "arrow.down.circle") { downloads.start(book) }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .voiceCard()
        .overlay {
            if job?.phase == .failed { RoundedRectangle(cornerRadius: 8).stroke(VoicePalette.danger, lineWidth: 1.5) }
        }
        .accessibilityElement(children: .contain)
        .confirmationDialog("Laden abbrechen?", isPresented: $confirmCancel, titleVisibility: .visible) {
            Button("Laden abbrechen", role: .destructive) { downloads.cancel(book) }
            Button("Weiter laden", role: .cancel) {}
        } message: {
            Text(done > 0
                 ? "Bereits geladene \(LibraryBytes.text(done)) werden vom iPhone gelöscht. Das Hörbuch bleibt im Konto und kann jederzeit neu geladen werden."
                 : "Das Hörbuch bleibt im Konto und kann jederzeit neu geladen werden.")
        }
    }

    private var cancel: some View {
        Button(role: .destructive) { confirmCancel = true } label: {
            Label("Abbrechen", systemImage: "xmark").frame(minHeight: 44).padding(.horizontal, 6)
        }
        .buttonStyle(.bordered).tint(VoicePalette.danger)
        .accessibilityHint("Bricht das Laden ab und löscht bereits geladene Teile vom iPhone.")
    }

    private func actions(_ primary: some View, _ secondary: some View) -> some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 8) { primary; secondary }
            VStack(alignment: .leading, spacing: 8) { primary; secondary }
        }
    }

    private func header(_ title: String, _ detail: String) -> some View {
        HStack(alignment: .firstTextBaseline) {
            Text(title).font(.headline)
            Spacer(minLength: 8)
            Text(detail).font(.subheadline.monospacedDigit()).foregroundStyle(VoicePalette.secondaryText)
        }
    }

    private func action(_ title: String, _ symbol: String, _ run: @escaping () -> Void) -> some View {
        Button(action: run) { Label(title, systemImage: symbol).frame(minHeight: 44).padding(.horizontal, 6) }
            .buttonStyle(.bordered).tint(VoicePalette.accent)
    }

    private func fraction(_ done: Int, _ total: Int) -> Double { total > 0 ? min(1, Double(done) / Double(total)) : 0 }
}

/// Scrollflächen reichen unter den Mini-Player. Der Rand sorgt dafür, dass das Letzte frei gescrollt werden kann.
struct MiniPlayerClearance: ViewModifier {
    @ObservedObject var playback = LibraryPlayback.shared
    @ScaledMetric(relativeTo: .subheadline) private var height: CGFloat = 76

    func body(content: Content) -> some View {
        content.contentMargins(.bottom, playback.book != nil && !playback.presented ? height : 0, for: .scrollContent)
    }
}

/// Laufschrift für den Satz im Mini-Player: ein Durchlauf je Satz, nur während die Wiedergabe läuft und die App vorne ist.
/// Passt er in die Zeile, steht er still; „Bewegung reduzieren“ zeigt ihn zweizeilig.
struct MarqueeText: View {
    let text: String
    let moving: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.scenePhase) private var scenePhase
    @State private var textWidth: CGFloat = 0
    @State private var boxWidth: CGFloat = 0
    @State private var offset: CGFloat = 0

    private var overflow: CGFloat { max(0, textWidth - boxWidth) }
    private var runs: Bool { moving && scenePhase == .active }

    var body: some View {
        if reduceMotion {
            Text(text).font(.caption).lineLimit(2).foregroundStyle(.white.opacity(0.85))
        } else {
            Text(text).font(.caption).lineLimit(1).fixedSize().foregroundStyle(.white.opacity(0.85))
                .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { textWidth = $0 }
                .offset(x: offset)
                .id("\(text)|\(runs)")  // Neue Ansicht beendet einen laufenden Durchlauf wirklich.
                // `minWidth: 0` hält den Rahmen schmal; sonst meldet er die Breite des Textes und sprengt den Mini-Player.
                .frame(minWidth: 0, maxWidth: .infinity, alignment: .leading)
                .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { boxWidth = $0 }
                .clipped()
                .task(id: "\(text)|\(overflow)|\(runs)") {
                    offset = 0
                    guard runs, overflow > 0 else { return }
                    try? await Task.sleep(nanoseconds: 700_000_000)
                    guard !Task.isCancelled else { return }
                    // So lange wie der Satz gesprochen wird, damit die Anzeige mithält.
                    let spoken = LibraryText.listenMilliseconds(chars: Double(text.utf16.count)) / 1000
                    withAnimation(.linear(duration: max(1.5, spoken - 0.7))) { offset = -overflow }
                }
        }
    }
}

struct LibraryMiniPlayer: View {
    @ObservedObject var playback = LibraryPlayback.shared
    var body: some View {
        HStack(spacing: 10) {
            Button { playback.presented = true } label: {
                HStack(spacing: 10) {
                    LibraryCover(title: playback.title, compact: true, image: playback.book.flatMap { PhoneLibrary.shared.coverThumbnail($0) })
                        .frame(width: 32, height: 44)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(playback.title).font(.subheadline.weight(.semibold)).lineLimit(1).foregroundStyle(.white)
                        if playback.playing, let sentence = playback.currentSentence {
                            MarqueeText(text: sentence, moving: playback.playing).accessibilityHidden(true)
                        } else {
                            Text(subtitle).font(.caption).lineLimit(1).foregroundStyle(.white.opacity(0.75))
                        }
                    }
                    Spacer(minLength: 0)
                }.frame(minHeight: 44).contentShape(Rectangle())
            }.buttonStyle(.plain).accessibilityLabel("\(playback.title), zum Player")
            Button { playback.toggle() } label: {
                Group {
                    if playback.preparing { ProgressView().tint(VoicePalette.ink) }
                    else { Image(systemName: playback.playing ? "pause.fill" : "play.fill").font(.headline) }
                }
                .foregroundStyle(VoicePalette.ink).frame(width: 44, height: 44).background(VoicePalette.brand, in: Circle())
            }.accessibilityLabel(playback.preparing ? "Wird vorbereitet, abbrechen" : playback.playing ? "Pause" : "Abspielen")
        }
        .padding(.horizontal, 8).frame(minHeight: 60)
        .background(VoicePalette.ink, in: RoundedRectangle(cornerRadius: 14))
    }
    private var subtitle: String {
        playback.mode == .audiobook ? "\(playback.currentChapter?.title ?? "Teil \(playback.trackIndex + 1)") · \(LibraryFormat.clock(playback.positionMs)) · zum Player"
                                    : "\(playback.preparing ? "Wird vorbereitet" : playback.playing ? "Text wird vorgelesen" : "Text pausiert") · zum Player"
    }
}

struct LibraryPlayerScreen: View {
    @ObservedObject var playback = LibraryPlayback.shared
    var body: some View {
        BookPlayerScreen()
    }
}

/// Text mit markiertem Absatz. Scrollt der Nutzer selbst, folgt der Text nicht mehr, bis er
/// „Zur aktuellen Stelle“ tippt (wie Audible): kein Zurückspringen mitten im Lesen.
struct FollowText<Row: View>: View {
    let paragraphs: [TextParagraph]
    let current: Int
    /// Aus: kein Mitscrollen (Ansicht „Lesen“), die Stelle bleibt, wo der Nutzer liest.
    var follow = true
    let row: (Int, TextParagraph) -> Row
    /// Nutzer hat selbst gescrollt: kein Mitscrollen, Knopf „Zur aktuellen Stelle“ sichtbar.
    @State private var detached = false

    init(paragraphs: [TextParagraph], current: Int, follow: Bool = true, @ViewBuilder row: @escaping (Int, TextParagraph) -> Row) {
        self.paragraphs = paragraphs; self.current = current; self.follow = follow; self.row = row
    }

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 4) {
                    ForEach(paragraphs.indices, id: \.self) { i in row(i, paragraphs[i]).id(i) }
                }.padding(.horizontal, 12).padding(.top, 20).padding(.bottom, follow ? 76 : 56)  // Platz unter dem Rückkehrknopf
            }
            // Weiche Kanten oben und unten: keine halb abgeschnittene Zeile an der Kopfzeile oder am Hinweis.
            // Die Innenabstände machen jede Zeile vollständig erreichbar.
            .mask {
                VStack(spacing: 0) {
                    LinearGradient(colors: [.clear, .black], startPoint: .top, endPoint: .bottom).frame(height: 18)
                    Color.black
                    LinearGradient(colors: [.black, .clear], startPoint: .top, endPoint: .bottom).frame(height: 28)
                }
            }
            // Scrollphase statt DragGesture: eine zusätzliche Geste auf der ScrollView konkurriert bei
            // häufigem Neuzeichnen (Wortmarkierung der Stimme) mit dem Wischen und verliert Gesten.
            .onScrollPhaseChange { _, phase in
                if follow, phase == .interacting { detached = true }
            }
            .overlay(alignment: .bottomTrailing) {
                if follow && detached {
                    // Nur Symbol, feste Größe: wächst bei großer Schrift nicht über den Text.
                    Button {
                        detached = false
                        withAnimation { proxy.scrollTo(current, anchor: .center) }
                    } label: {
                        Image(systemName: "arrow.down.to.line").font(.system(size: 18, weight: .semibold))
                            .foregroundStyle(VoicePalette.brand)
                            .frame(width: 48, height: 48).background(VoicePalette.ink, in: Circle())
                            .shadow(color: .black.opacity(0.25), radius: 4, y: 2)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Zur aktuellen Stelle")
                    .accessibilityHint("Der Text folgt wieder der Wiedergabe")
                    .accessibilityShowsLargeContentViewer { Label("Zur aktuellen Stelle", systemImage: "arrow.down.to.line") }
                    .padding(12)
                }
            }
            .onAppear { proxy.scrollTo(current, anchor: .center) }  // ohne Animation: sonst baut die Liste jede Zeile bis dorthin
            .onChange(of: current) { old, value in
                guard follow, !detached else { return }
                if abs(value - old) > 3 { proxy.scrollTo(value, anchor: .center) }
                else { withAnimation { proxy.scrollTo(value, anchor: .center) } }
            }
        }
    }
}

struct ControlButton: View {
    /// `title` ist der VoiceOver-Name; `caption` die kurze sichtbare Beschriftung (Standard: `title`).
    let title: String; let systemImage: String; var caption: String? = nil; let action: () -> Void
    var body: some View {
        Button(action: action) {
            VStack(spacing: 2) {
                Image(systemName: systemImage).font(.title2)
                // Große Schrift: Beschriftung schrumpft statt mit dem Nachbarn zu kollidieren.
                Text(caption ?? title).font(.caption2).lineLimit(1).minimumScaleFactor(0.7)
            }.frame(minWidth: 56, minHeight: 44).contentShape(Rectangle())
        }.buttonStyle(.plain).accessibilityLabel(title).accessibilityShowsLargeContentViewer()
    }
}

struct PlayerHeader: View {
    let title: String; let subtitle: String; var trailing: AnyView? = nil
    @ObservedObject var playback = LibraryPlayback.shared
    var body: some View {
        HStack {
            Button { playback.presented = false } label: { Image(systemName: "chevron.down").font(.title3.weight(.semibold)).frame(width: 44, height: 44) }
                .accessibilityLabel("Player schließen")
            Spacer()
            VStack(spacing: 1) { Text(title).font(.subheadline.weight(.semibold)).lineLimit(1); Text(subtitle).font(.caption).foregroundStyle(VoicePalette.secondaryText).lineLimit(1) }
            Spacer()
            trailing ?? AnyView(Color.clear.frame(width: 44, height: 44))
        }.padding(.horizontal, 8)
    }
}

struct RateMenu: View {
    @ObservedObject var playback = LibraryPlayback.shared
    @State private var flowShown = false
    var body: some View {
        Menu {
            ForEach([0.8, 1.0, 1.2, 1.5, 2.0], id: \.self) { r in Button(String(format: "%.1f×", r).replacingOccurrences(of: ".", with: ",")) { playback.rate = Float(r) } }
            if playback.mode == .readAloud {
                Section("Sprachfluss") {
                    ForEach(SpeechFlowSheet.presets, id: \.value) { preset in
                        Button { playback.flow = preset.value } label: {
                            if abs(playback.flow - preset.value) < 0.01 { Label(preset.title, systemImage: "checkmark") } else { Text(preset.title) }
                        }
                    }
                    Button("Feineinstellung …") { flowShown = true }
                }
            }
        } label: {
            VStack(spacing: 2) { Text(String(format: "%.1f×", playback.rate).replacingOccurrences(of: ".", with: ",")).font(.headline); Text("Tempo").font(.caption2) }
                .frame(minWidth: 56, minHeight: 44)
        }.buttonStyle(.plain).accessibilityLabel("Tempo und Sprachfluss")
        .sheet(isPresented: $flowShown) { SpeechFlowSheet().presentationDetents([.medium]) }
    }
}

/// Sprachfluss beim Vorlesen: Pausen an Komma, Satz- und Absatzende.
struct SpeechFlowSheet: View {
    struct Preset: Hashable { let title: String; let value: Double }
    static let presets = [Preset(title: "Zügig", value: SpeechFlow.brisk), Preset(title: "Hörbuch", value: SpeechFlow.audiobook),
                          Preset(title: "Ruhig", value: SpeechFlow.calm)]
    @ObservedObject var playback = LibraryPlayback.shared
    @Environment(\.dismiss) private var dismiss
    @State private var value = LibraryPlayback.shared.flow

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Picker("Vorgabe", selection: Binding(get: { Self.presets.first { abs($0.value - value) < 0.01 }?.value ?? -1 },
                                                          set: { if $0 >= 0 { value = $0; playback.flow = $0 } })) {
                        ForEach(Self.presets, id: \.value) { Text($0.title).tag($0.value) }
                        if !Self.presets.contains(where: { abs($0.value - value) < 0.01 }) { Text("Eigene").tag(-1.0) }
                    }.pickerStyle(.segmented).labelsHidden()
                } footer: { Text("Hörbuch entspricht dem üblichen Vortrag von Hörbüchern.") }
                Section("Feineinstellung") {
                    HStack {
                        Text("Zügig").font(.caption)
                        Slider(value: $value, in: 0...1) { editing in if !editing { playback.flow = value } }
                            .accessibilityLabel("Sprachfluss")
                            .accessibilityValue(SpeechFlow(value).title)
                        Text("Ruhig").font(.caption)
                    }.frame(minHeight: 44)
                    let f = SpeechFlow(value)
                    LabeledContent("Pause am Komma", value: "\(Int(f.commaMs)) ms")
                    LabeledContent("Pause zwischen Sätzen", value: "\(Int(f.sentenceMs)) ms")
                    LabeledContent("Pause am Absatzende", value: "\(Int(f.paragraphMs)) ms")
                }
            }
            .navigationTitle("Sprachfluss").navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Fertig") { playback.flow = value; dismiss() } } }
        }
    }
}

struct PlayButton: View {
    @ObservedObject var playback = LibraryPlayback.shared
    var body: some View {
        Button { playback.toggle() } label: {
            Group {
                if playback.preparing { ProgressView().tint(VoicePalette.brand) }
                else { Image(systemName: playback.playing ? "pause.fill" : "play.fill").font(.title) }
            }
            .foregroundStyle(VoicePalette.brand).frame(width: 64, height: 64).background(VoicePalette.ink, in: Circle())
        }.accessibilityLabel(playback.preparing ? "Wird vorbereitet, abbrechen" : playback.playing ? "Pause" : "Abspielen")
    }
}

struct VoiceList: View {
    @ObservedObject var playback = LibraryPlayback.shared
    @ObservedObject var phonon = PhononEngine.shared
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        NavigationStack {
            List {
                // Wofür die Wahl gilt (lva#352): alle Bücher der Sprache oder nur dieses Buch.
                if let lang = playback.voiceLanguage {
                    let name = Locale.current.localizedString(forLanguageCode: lang) ?? lang
                    Section {
                        Toggle("Nur für dieses Buch", isOn: Binding(
                            get: { playback.voiceScope == .book },
                            set: { playback.setBookOnlyVoice($0) }))
                        .frame(minHeight: 44)
                    } footer: {
                        Text(playback.voiceScope != .book ? "Standardstimme für Bücher auf \(name)."
                             : playback.bookVoiceMissing
                             ? "Die eigene Stimme dieses Buchs fehlt auf dem iPhone. Es liest die Standardstimme für \(name)."
                             : "Eigene Stimme für dieses Buch (\(name)).")
                    }
                }
                ForEach(PhononEngine.languages, id: \.self) { language in
                    if let names = phonon.voices[language] {
                        Section {
                            ForEach(names, id: \.self) { name in
                                let id = PhononEngine.identifier(language: language, voice: name)
                                Button { playback.chooseVoice(id); dismiss() } label: {
                                    HStack {
                                        Text(name)
                                        Spacer()
                                        if id == playback.voiceIdentifier { Image(systemName: "checkmark") }
                                    }.frame(minHeight: 44).contentShape(Rectangle())
                                }.buttonStyle(.plain)
                            }
                        } header: { Text(PhononEngine.sectionTitle(language)) } footer: {
                            Text("Läuft auf dem iPhone. Markiert wird der Satz statt des Worts. Die erste Nutzung einer Sprache dauert einige Sekunden.")
                        }
                    }
                }
                Section {
                    ForEach(playback.systemVoices, id: \.identifier) { v in
                        Button {
                            playback.chooseVoice(v.identifier); dismiss()
                        } label: {
                            HStack {
                                VStack(alignment: .leading) {
                                    Text(v.name)
                                    Text(v.quality == .premium ? "Premium" : v.quality == .enhanced ? "Erweitert" : "Standard").font(.caption).foregroundStyle(.secondary)
                                }
                                Spacer()
                                if v.identifier == playback.voiceIdentifier { Image(systemName: "checkmark") }
                            }.frame(minHeight: 44)
                        }.buttonStyle(.plain)
                    }
                } footer: {
                    Text("Bessere Stimmen (Premium) lädst du kostenlos unter Einstellungen › Bedienungshilfen › Gesprochene Inhalte › Stimmen.")
                }
            }
            .navigationTitle("Stimme").navigationBarTitleDisplayMode(.inline)
            .task { await phonon.refresh() }
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Fertig") { dismiss() } } }
        }
    }
}


struct LibrarySyncSheet: View {
    @ObservedObject var sync = LibrarySync.shared
    @Environment(\.dismiss) private var dismiss
    @State private var email = ""
    @State private var password = ""
    @State private var url = LibrarySync.defaultHub
    @State private var device = "iPhone"

    var body: some View {
        NavigationStack {
            Form {
                if let acc = sync.account {
                    Section {
                        LabeledContent("Konto", value: acc.email)
                        LabeledContent("Stand", value: sync.status)
                        if let last = sync.lastSync { LabeledContent("Zuletzt", value: last.formatted(date: .omitted, time: .shortened)) }
                        if let e = sync.error { Text(e).foregroundStyle(.red).font(.footnote) }
                        Button("Jetzt abgleichen") { Task { await sync.run() } }.disabled(sync.running)
                    } footer: { Text("Bibliothek und Fortschritt werden mit der Desktop-App abgeglichen, Ende-zu-Ende verschlüsselt. Der Server sieht nur Chiffrat.") }
                    Section {
                        Toggle("Hörbücher nur im WLAN laden", isOn: $sync.wifiOnly)
                    }
                    LibraryStorageSummary()
                    Section { Button("Abmelden", role: .destructive) { Task { await sync.logout() } } }
                } else {
                    Section {
                        Button { PairingFlow.shared.startScan() } label: { Label("Mit Desktop-App verbinden", systemImage: "qrcode.viewfinder") }
                    } footer: { Text("Am PC unter Einstellungen › Konto & Geräte auf „Handy verbinden“ gehen und den Code scannen. Ein Passwort ist dann nicht nötig.") }
                    Section {
                        TextField("E-Mail", text: $email).textContentType(.username).keyboardType(.emailAddress).textInputAutocapitalization(.never)
                        SecureField("Passwort", text: $password).textContentType(.password)
                        TextField("Gerätename", text: $device)
                    } footer: { Text("Dasselbe Konto wie in der Desktop-App unter Einstellungen › Konto & Geräte. Das Passwort verlässt das iPhone nicht; daraus wird nur hier der Schlüssel berechnet.") }
                    LibraryStorageSummary()
                    Section("Server") { TextField("Server", text: $url).keyboardType(.URL).textInputAutocapitalization(.never) }
                    Section {
                        Link("Passwort vergessen?", destination: URL(string: "\(url.trimmingCharacters(in: .whitespaces))/admin/password/forgot") ?? URL(string: LibrarySync.defaultHub + "/admin/password/forgot")!)
                            .frame(minHeight: 44)
                    } footer: {
                        Text("Nach einer Änderung des Passworts musst du dich auch am PC neu anmelden. Aus dem Passwort entsteht der Schlüssel; beide Geräte brauchen denselben.")
                    }
                    Section {
                        Button {
                            Task { await sync.login(email: email, password: password, url: url, deviceName: device); if sync.account != nil { password = "" } }
                        } label: { HStack { Text("Anmelden"); if sync.running { Spacer(); ProgressView() } } }
                        .disabled(sync.running || email.isEmpty || password.isEmpty)
                        if sync.running { Text(sync.status).font(.footnote).foregroundStyle(.secondary) }
                        if let e = sync.error { Text(e).foregroundStyle(.red).font(.footnote) }
                    }
                }
            }
            .navigationTitle("Konto & Sync").navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Fertig") { dismiss() } } }
        }
    }
}

// MARK: Speicher (Epic #210, P8 Paket 2)

enum LibraryBytes {
    static func text(_ bytes: Int) -> String { text(Int64(bytes)) }
    static func text(_ bytes: Int64) -> String {
        let f = ByteCountFormatter(); f.countStyle = .file; f.allowsNonnumericFormatting = false  // „0 Byte“ statt „Null KB“
        return f.string(fromByteCount: bytes)
    }
}

/// Abschnitt in „Konto & Sync“: belegter und freier Platz, Weg zur Übersicht.
struct LibraryStorageSummary: View {
    @ObservedObject var library = PhoneLibrary.shared
    @ObservedObject var sync = LibrarySync.shared
    @State private var items: [AudioStorage.Item] = []
    @State private var free: Int64?

    var body: some View {
        Section {
            // Nur an EINER Zeile: Modifier an einer Section gelten sonst je Zeile.
            LabeledContent("Hörbücher auf diesem iPhone", value: LibraryBytes.text(AudioStorage.total(items)))
                .onAppear(perform: refresh)
                .onChange(of: library.books) { _, _ in refresh() }
            if let free { LabeledContent("Frei auf dem iPhone", value: LibraryBytes.text(free)) }
            NavigationLink {
                LibraryStorageView()
            } label: {
                LabeledContent("Speicher verwalten", value: items.isEmpty ? "" : "\(items.count) geladen")
            }.accessibilityIdentifier("storage")
        } header: { Text("Speicher") } footer: {
            Text("Hörbücher liegen nur bei Bedarf auf dem iPhone und zählen nicht zum iCloud-Backup.")
        }
    }

    private func refresh() { items = sync.storageItems(); free = library.freeBytes() }
}

/// Geladene Hörbücher nach Größe; Entfernen per Wischen oder gesammelt.
struct LibraryStorageView: View {
    @ObservedObject var library = PhoneLibrary.shared
    @ObservedObject var sync = LibrarySync.shared
    @ObservedObject var playback = LibraryPlayback.shared
    @State private var items: [AudioStorage.Item] = []
    @State private var free: Int64?
    @State private var working: Set<String> = []
    @State private var confirmFinished = false
    @State private var blocked: AudioOffload.Verdict?
    @State private var notice: String?
    /// Zum Entfernen gewählt (per Wischen); Rückfrage nennt das Buch.
    @State private var pending: AudioStorage.Item?

    var body: some View {
        // Das gerade laufende Buch nicht nebenbei stoppen.
        let finished = AudioStorage.finishedRemovable(items).filter { $0.id != playback.book?.id }
        List {
            Section {
                LabeledContent("Belegt durch Hörbücher", value: LibraryBytes.text(AudioStorage.total(items)))
                if let free { LabeledContent("Frei auf dem iPhone", value: LibraryBytes.text(free)) }
                if !finished.isEmpty {
                    Button(role: .destructive) { confirmFinished = true } label: {
                        Label("Fertig gehörte entfernen (\(finished.count))", systemImage: "checkmark.circle")
                            .frame(minHeight: 44)
                    }.disabled(!working.isEmpty).accessibilityIdentifier("removeFinished")
                }
                if let notice { Text(notice).font(.footnote).foregroundStyle(VoicePalette.secondaryText) }
            } footer: {
                Text("Entfernte Hörbücher bleiben im Konto und in der Bibliothek. Sie werden beim nächsten Hören wieder geladen, der Fortschritt bleibt.")
            }
            if items.isEmpty {
                ContentUnavailableView("Keine Hörbücher geladen", systemImage: "headphones",
                                       description: Text("Hörbücher aus dem Konto werden beim ersten Hören auf dieses iPhone geladen."))
            } else {
                Section("Geladene Hörbücher") {
                    ForEach(items) { item in
                        row(item)
                            .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                                if item.removable && !working.contains(item.id) {
                                    // Rückfrage mit Titel: beim Wischen rutscht die Zeile aus dem Bild.
                                    Button(role: .destructive) { pending = item } label: { Label("Entfernen", systemImage: "icloud.and.arrow.down") }
                                }
                            }
                    }
                }
            }
        }
        .navigationTitle("Speicher").navigationBarTitleDisplayMode(.inline)
        .onAppear(perform: refresh)
        // Während einer Sammelentfernung nicht je Buch neu messen; am Ende einmal.
        .onChange(of: library.books) { _, _ in if working.isEmpty { refresh() } }
        .refreshable { refresh() }
        .confirmationDialog("Fertig gehörte Hörbücher entfernen?", isPresented: $confirmFinished, titleVisibility: .visible) {
            Button("\(finished.count) entfernen · \(LibraryBytes.text(AudioStorage.total(finished))) frei", role: .destructive) {
                let ids = finished.map(\.id)
                working.formUnion(ids)
                Task {
                    let result = await sync.removeDownloads(ids: ids)
                    working.subtract(ids)
                    announce(AudioStorage.summary(removed: result.removed, freedText: LibraryBytes.text(result.freed), kept: result.kept))
                    refresh()
                }
            }
        } message: { Text("Sie bleiben im Konto und lassen sich jederzeit wieder laden.") }
        .confirmationDialog(pending.map { "„\($0.title)“ vom iPhone entfernen?" } ?? "",
                            isPresented: Binding(get: { pending != nil }, set: { if !$0 { pending = nil } }),
                            titleVisibility: .visible, presenting: pending) { item in
            Button("\(LibraryBytes.text(item.bytes)) freigeben", role: .destructive) { remove(item) }
        } message: { item in Text("\(item.stand). Es bleibt im Konto und wird beim nächsten Hören wieder geladen.") }
        .alert("Hörbuch bleibt auf dem iPhone", isPresented: Binding(get: { blocked != nil }, set: { if !$0 { blocked = nil } }),
               presenting: blocked) { verdict in
            if verdict.syncHelps && sync.account != nil { Button("Jetzt abgleichen") { Task { await sync.run(); refresh() } } }
            Button("OK", role: .cancel) {}
        } message: { verdict in Text(verdict.message) }
    }

    @ViewBuilder private func row(_ item: AudioStorage.Item) -> some View {
        let book = library.books.first { $0.id == item.id }
        let busy = working.contains(item.id)
        let content = HStack(spacing: 12) {
            LibraryCover(title: item.title, compact: true, image: book.flatMap { library.coverImage($0) }).frame(width: 40, height: 60)
            VStack(alignment: .leading, spacing: 3) {
                Text(item.title).font(.body.weight(.semibold)).lineLimit(2)
                Text(detail(item)).font(.footnote).foregroundStyle(VoicePalette.secondaryText).lineLimit(2)
                if item.localOnly {
                    Label("Nur auf diesem iPhone", systemImage: "iphone").font(.caption.weight(.semibold)).foregroundStyle(VoicePalette.text)
                }
            }
            Spacer(minLength: 8)
            if busy { ProgressView().accessibilityLabel("Wird entfernt") }
            else { Text(LibraryBytes.text(item.bytes)).font(.subheadline.monospacedDigit()).foregroundStyle(VoicePalette.secondaryText) }
        }
        .frame(minHeight: 60)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        if item.removable {
            content  // Entfernen steht als Wischaktion auch im VoiceOver-Rotor.
        } else {
            // Nicht entfernbar: Antippen erklärt warum (sonst sähe man es nur mit VoiceOver).
            content
                .onTapGesture { if let book { blocked = sync.offloadVerdict(book) } }
                .accessibilityAddTraits(.isButton)
                .accessibilityHint("Zeigt, warum sich das Hörbuch noch nicht entfernen lässt.")
        }
    }

    private func detail(_ item: AudioStorage.Item) -> String {
        guard let used = item.lastUsedMs else { return item.stand }
        let recent = Date().timeIntervalSince1970 * 1000 - used < 60_000
        return "\(item.stand) · \(recent ? "gerade eben" : "zuletzt " + LibraryFormat.when(used))"
    }

    private func remove(_ item: AudioStorage.Item) {
        guard !working.contains(item.id), let book = library.books.first(where: { $0.id == item.id }) else { return }
        working.insert(item.id)
        Task {
            let result = await sync.removeDownloads(book)
            working.remove(item.id)
            if result.verdict.allowed { announce("„\(item.title)“ entfernt, \(LibraryBytes.text(result.freed)) frei.") }
            else { blocked = result.verdict }
            refresh()
        }
    }

    private func announce(_ text: String) {
        notice = text
        UIAccessibility.post(notification: .announcement, argument: text)
    }

    private func refresh() { items = sync.storageItems(); free = library.freeBytes() }
}
