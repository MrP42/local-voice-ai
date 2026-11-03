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
    static func percent(_ f: Double) -> String { "\(Int((f * 100).rounded())) %" }
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
                        .lineLimit(5).minimumScaleFactor(0.6).padding(8)
                }
            }
            .clipped()
            .accessibilityHidden(true)
    }
}

enum ShelfFilter: String, CaseIterable, Identifiable {
    case all = "Alle", inProgress = "In Arbeit", audio = "Hörbuch", finished = "Fertig"
    var id: String { rawValue }
}

struct LibraryTab: View {
    @ObservedObject var library = PhoneLibrary.shared
    @ObservedObject var playback = LibraryPlayback.shared

    var body: some View {
        NavigationStack { LibraryShelf() }
            .safeAreaInset(edge: .bottom, spacing: 0) {
                if playback.book != nil && !playback.presented { LibraryMiniPlayer().padding(.horizontal, 8).padding(.bottom, 6) }
            }
            .fullScreenCover(isPresented: $playback.presented) { LibraryPlayerScreen() }
            #if DEBUG
            .task {
                // Diagnose-Startargumente für Simulator-Bilder, kein Produktpfad.
                let args = ProcessInfo.processInfo.arguments
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
                    playback.openAudiobook(book, autoplay: true); playback.presented = true
                }
                if args.contains("--library-read") { playback.openReadAloud(book, autoplay: true); playback.presented = true }
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
    @State private var importing = false
    @State private var showSync = false
    @ObservedObject var sync = LibrarySync.shared

    private func textFraction(_ b: LibraryBook) -> Double { LibraryProgress.fraction(library.record(library.textKey(b))) }
    private func audioFraction(_ b: LibraryBook) -> Double { LibraryProgress.fraction(library.record(library.audioKey(b))) }

    private var visible: [LibraryBook] {
        let q = query.trimmingCharacters(in: .whitespaces)
        return library.books.filter { b in
            let matches = q.isEmpty || b.title.localizedCaseInsensitiveContains(q) || b.authors.contains { $0.localizedCaseInsensitiveContains(q) }
            let done = b.finished || max(textFraction(b), audioFraction(b)) >= 1
            switch filter {
            case .all: return matches
            case .inProgress: return matches && !done && max(textFraction(b), audioFraction(b)) > 0
            case .audio: return matches && b.audio != nil
            case .finished: return matches && done
            }
        }
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
                    Text(isAudio ? "WEITERHÖREN" : "WEITERLESEN").font(.caption.weight(.semibold)).tracking(0.6).foregroundStyle(VoicePalette.secondaryText)
                    NavigationLink { LibraryBookDetail(bookId: book.id) } label: {
                        HStack(alignment: .top, spacing: 14) {
                            LibraryCover(title: book.title, image: library.coverThumbnail(book)).frame(width: 64, height: 96)
                            VStack(alignment: .leading, spacing: 3) {
                                Text(book.title).font(.headline)
                                if !book.authors.isEmpty { Text(book.authors.joined(separator: ", ")).font(.subheadline).foregroundStyle(VoicePalette.secondaryText) }
                                Text(isAudio ? "Hörbuch · noch \(LibraryFormat.remaining(max(0, rec.length - rec.offset)))" : "Text · \(LibraryFormat.percent(LibraryProgress.fraction(rec))) gelesen")
                                    .font(.footnote).foregroundStyle(VoicePalette.secondaryText)
                                ProgressView(value: LibraryProgress.fraction(rec)).tint(VoicePalette.accent)
                            }
                        }.voiceCard()
                    }.buttonStyle(.plain)
                }

                Text("ALLE BÜCHER · \(visible.count)").font(.caption.weight(.semibold)).tracking(0.6).foregroundStyle(VoicePalette.secondaryText)
                if library.books.isEmpty {
                    ContentUnavailableView {
                        Label("Noch keine Bücher", systemImage: "books.vertical")
                    } description: {
                        Text("Füge EPUB, PDF, TXT oder ein Hörbuch (MP3, M4A, M4B) aus der Dateien-App hinzu. Mehrere Audiodateien zusammen werden ein Hörbuch mit Teilen.")
                    } actions: { Button("Buch hinzufügen") { importing = true }.buttonStyle(.borderedProminent) }
                }
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 100), spacing: 12)], alignment: .leading, spacing: 16) {
                    ForEach(visible) { book in
                        NavigationLink { LibraryBookDetail(bookId: book.id) } label: { ShelfItem(book: book) }.buttonStyle(.plain)
                    }
                }
                if library.importing { ProgressView("Importiere …") }
                if let error = library.error { Text(error).font(.footnote).foregroundStyle(.red) }
            }.padding(16)
        }
        .background(VoicePalette.background)
        .navigationTitle("Bibliothek")
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button("Buch oder Hörbuch hinzufügen", systemImage: "plus") { importing = true }.accessibilityIdentifier("libraryAdd")
            }
            ToolbarItem(placement: .topBarLeading) {
                Button { showSync = true } label: {
                    Image(systemName: sync.account == nil ? "icloud.slash" : (sync.error != nil ? "exclamationmark.icloud" : "checkmark.icloud"))
                }.accessibilityLabel("Konto und Sync: \(sync.status)")
            }
        }
        .fileImporter(isPresented: $importing, allowedContentTypes: PhoneLibrary.importTypes, allowsMultipleSelection: true) { result in
            if case .success(let urls) = result { Task { await library.importFiles(urls); await sync.run() } }
        }
        .refreshable { await sync.run(); library.reload() }
        .task { await sync.run() }
        .sheet(isPresented: $showSync) { LibrarySyncSheet() }
    }
}

struct ShelfItem: View {
    let book: LibraryBook
    @ObservedObject var library = PhoneLibrary.shared
    var body: some View {
        let audio = library.hasAudio(book), text = library.hasText(book)
        let rec = audio || library.audioInAccount(book) ? library.record(library.audioKey(book)) : library.record(library.textKey(book))
        VStack(alignment: .leading, spacing: 5) {
            LibraryCover(title: book.title, image: library.coverThumbnail(book)).aspectRatio(2/3, contentMode: .fit)
            ProgressView(value: book.finished ? 1 : LibraryProgress.fraction(rec)).tint(VoicePalette.accent)
            Text(audio || library.audioInAccount(book) ? (rec == nil ? "Hörbuch · neu" : "Hörbuch · \(LibraryFormat.percent(LibraryProgress.fraction(rec)))")
                       : "Text · \(LibraryFormat.percent(LibraryProgress.fraction(rec)))")
                .font(.caption).foregroundStyle(VoicePalette.secondaryText)
            let inAccount = library.audioInAccount(book)
            Text(inAccount ? (text ? "Hörbuch im Konto" : "Im Konto, nicht geladen") : (audio || text ? "Auf dem iPhone" : "Im Konto, nicht geladen"))
                .font(.caption.weight(.semibold)).foregroundStyle(!inAccount && (audio || text) ? Color.green : VoicePalette.secondaryText)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(book.title), \(book.authors.joined(separator: ", "))")
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
                        Label("Auf dem iPhone", systemImage: "checkmark").font(.footnote.weight(.semibold)).foregroundStyle(.green)
                    }
                }
                HStack(spacing: 10) {
                    if hasText {
                        Button { playback.openReadAloud(book, autoplay: true); playback.presented = playback.book != nil } label: {
                            Label("Text vorlesen", systemImage: "book").frame(maxWidth: .infinity, minHeight: 44)
                        }.buttonStyle(.bordered).accessibilityIdentifier("readAloud")
                    }
                    if hasAudio || inAccount {
                        Button {
                            Task {
                                if !library.hasAudio(book) { guard await sync.downloadAudio(book) else { return } }
                                guard let fresh = library.books.first(where: { $0.id == book.id }) else { return }
                                playback.openAudiobook(fresh); playback.presented = playback.book != nil
                            }
                        } label: {
                            Label("Hörbuch hören", systemImage: "headphones").frame(maxWidth: .infinity, minHeight: 44)
                        }.buttonStyle(.borderedProminent).tint(VoicePalette.ink).foregroundStyle(VoicePalette.brand)
                            .disabled(sync.downloads[book.id] != nil).accessibilityIdentifier("listen")
                    }
                }.font(.headline)

                if inAccount {
                    VStack(alignment: .leading, spacing: 6) {
                        if let f = sync.downloads[book.id] {
                            Text("Hörbuch lädt … \(LibraryFormat.percent(f))").font(.subheadline.weight(.semibold))
                            ProgressView(value: f).tint(VoicePalette.accent)
                            Text(sync.wifiOnly ? "Nur im WLAN" : "Über WLAN oder Mobilfunk").font(.caption).foregroundStyle(VoicePalette.secondaryText)
                        } else {
                            Text("Hörbuch im Konto · \(ByteCountFormatter.string(fromByteCount: Int64(library.meta(book).files.audioBytes), countStyle: .file))")
                                .font(.subheadline.weight(.semibold))
                            Text(sync.wifiOnly && sync.expensiveNetwork ? "Wartet auf WLAN („Nur im WLAN laden“ ist an)." : "Wird beim ersten Hören geladen und ist danach offline verfügbar.")
                                .font(.caption).foregroundStyle(VoicePalette.secondaryText)
                            Button("Jetzt laden") { Task { _ = await sync.downloadAudio(book) } }.frame(minHeight: 44)
                        }
                        if let e = sync.error { Text(e).font(.caption).foregroundStyle(.red) }
                    }.voiceCard()
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

                if tracks.count > 1 {
                    Text("HÖRBUCH-TEILE").font(.caption.weight(.semibold)).tracking(0.6).foregroundStyle(VoicePalette.secondaryText)
                    let current = audioRec.map { LibraryTracks.position(of: $0.offset, in: tracks).index }
                    ForEach(Array(tracks.enumerated()), id: \.offset) { i, t in
                        Button {
                            playback.openAudiobook(book, autoplay: false)
                            guard playback.book?.id == book.id else { return }
                            playback.seek(toMs: LibraryTracks.total(TrackPosition(index: i, offsetMs: 0), in: tracks))
                            playback.play(); playback.presented = true
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
        .background(VoicePalette.background)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Menu("Mehr", systemImage: "ellipsis.circle") {
                    Button(book.finished ? "Als nicht fertig markieren" : "Als fertig markieren", systemImage: "checkmark.circle") { library.setFinished(book, !book.finished) }
                    if library.hasLocalAudio(book) {
                        Button("Hörbuch vom iPhone entfernen", systemImage: "icloud.and.arrow.down") {
                            let verdict = sync.offloadVerdict(book)
                            switch verdict {
                            case .allowed, .unconfirmed:  // wird beim Bestätigen frisch beim Konto geprüft
                                removeBytes = library.localAudioBytes(book)
                                confirmRemoveAudio = true
                            default: offloadBlocked = verdict
                            }
                        }.disabled(removingAudio)
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

    private func progressRow(_ title: String, value: Double, detail: String, when: Double?) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack { Text(title).fontWeight(.semibold); Spacer(); Text(detail).foregroundStyle(VoicePalette.secondaryText) }.font(.subheadline)
            ProgressView(value: value).tint(VoicePalette.accent)
            if let when, when > 0 { Text("Zuletzt auf diesem iPhone \(LibraryFormat.when(when))").font(.caption).foregroundStyle(VoicePalette.secondaryText) }
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
                        Text(subtitle).font(.caption).lineLimit(1).foregroundStyle(.white.opacity(0.75))
                    }
                    Spacer(minLength: 0)
                }.frame(minHeight: 44).contentShape(Rectangle())
            }.buttonStyle(.plain).accessibilityLabel("\(playback.title), zum Player")
            Button { playback.toggle() } label: {
                Image(systemName: playback.playing ? "pause.fill" : "play.fill").font(.headline)
                    .foregroundStyle(VoicePalette.ink).frame(width: 44, height: 44).background(VoicePalette.brand, in: Circle())
            }.accessibilityLabel(playback.playing ? "Pause" : "Abspielen")
        }
        .padding(.horizontal, 8).frame(minHeight: 60)
        .background(VoicePalette.ink, in: RoundedRectangle(cornerRadius: 14))
    }
    private var subtitle: String {
        playback.mode == .audiobook ? "\(playback.currentChapter?.title ?? "Teil \(playback.trackIndex + 1)") · \(LibraryFormat.clock(playback.positionMs)) · zum Player"
                                    : "Text wird vorgelesen · zum Player"
    }
}

struct LibraryPlayerScreen: View {
    @ObservedObject var playback = LibraryPlayback.shared
    var body: some View {
        if playback.mode == .readAloud { LibraryReadAloudView() } else { LibraryListenView() }
    }
}

/// Text mit markiertem Absatz; bei eigenem Scrollen 5 s kein Mitscrollen.
struct FollowText<Row: View>: View {
    let paragraphs: [TextParagraph]
    let current: Int
    let row: (Int, TextParagraph) -> Row
    @State private var pausedUntil = Date.distantPast
    @State private var showReturn = false

    init(paragraphs: [TextParagraph], current: Int, @ViewBuilder row: @escaping (Int, TextParagraph) -> Row) {
        self.paragraphs = paragraphs; self.current = current; self.row = row
    }

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 4) {
                    ForEach(Array(paragraphs.enumerated()), id: \.offset) { i, p in row(i, p).id(i) }
                }.padding(.horizontal, 12).padding(.vertical, 12)
            }
            .simultaneousGesture(DragGesture().onChanged { _ in pausedUntil = Date().addingTimeInterval(5); showReturn = true })
            .overlay(alignment: .bottomTrailing) {
                if showReturn {
                    Button("Zur aktuellen Stelle", systemImage: "arrow.down.to.line") {
                        pausedUntil = .distantPast; showReturn = false
                        withAnimation { proxy.scrollTo(current, anchor: .center) }
                    }.buttonStyle(.borderedProminent).tint(VoicePalette.ink).foregroundStyle(VoicePalette.brand).padding(12)
                }
            }
            .onChange(of: current, initial: true) { _, value in
                guard Date() >= pausedUntil else { return }
                showReturn = false
                withAnimation { proxy.scrollTo(value, anchor: .center) }
            }
        }
    }
}

struct ControlButton: View {
    let title: String; let systemImage: String; let action: () -> Void
    var body: some View {
        Button(action: action) {
            VStack(spacing: 2) { Image(systemName: systemImage).font(.title2); Text(title).font(.caption2) }.frame(minWidth: 56, minHeight: 44)
        }.buttonStyle(.plain).accessibilityLabel(title)
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
    var body: some View {
        Menu {
            ForEach([0.8, 1.0, 1.2, 1.5, 2.0], id: \.self) { r in Button(String(format: "%.1f×", r).replacingOccurrences(of: ".", with: ",")) { playback.rate = Float(r) } }
        } label: {
            VStack(spacing: 2) { Text(String(format: "%.1f×", playback.rate).replacingOccurrences(of: ".", with: ",")).font(.headline); Text("Tempo").font(.caption2) }
                .frame(minWidth: 56, minHeight: 44)
        }.buttonStyle(.plain).accessibilityLabel("Tempo")
    }
}

struct PlayButton: View {
    @ObservedObject var playback = LibraryPlayback.shared
    var body: some View {
        Button { playback.toggle() } label: {
            Image(systemName: playback.playing ? "pause.fill" : "play.fill").font(.title).foregroundStyle(VoicePalette.brand)
                .frame(width: 64, height: 64).background(VoicePalette.ink, in: Circle())
        }.accessibilityLabel(playback.playing ? "Pause" : "Abspielen")
    }
}

// LibraryListenView: siehe AudiobookPlayer.swift

struct LibraryReadAloudView: View {
    @ObservedObject var playback = LibraryPlayback.shared
    @State private var voices = false

    private func highlighted(_ p: TextParagraph) -> AttributedString {
        var s = AttributedString(p.text)
        guard let r = playback.wordRange, let range = Range(r, in: p.text), let a = AttributedString.Index(range.lowerBound, within: s),
              let b = AttributedString.Index(range.upperBound, within: s) else { return s }
        s[a..<b].backgroundColor = VoicePalette.ink
        s[a..<b].foregroundColor = VoicePalette.brand
        return s
    }

    var body: some View {
        VStack(spacing: 0) {
            PlayerHeader(title: "\(playback.title) · Text", subtitle: playback.playing ? "wird vorgelesen" : "pausiert")
            Divider()
            FollowText(paragraphs: playback.paragraphs, current: playback.paragraphIndex) { i, p in
                Text(i == playback.paragraphIndex ? highlighted(p) : AttributedString(p.text))
                    .font(p.heading ? .custom("Georgia-Bold", size: 21, relativeTo: .title3) : .custom("Georgia", size: 18, relativeTo: .body))
                    .foregroundStyle(i == playback.paragraphIndex ? VoicePalette.text : VoicePalette.secondaryText)
                    .lineSpacing(5).fixedSize(horizontal: false, vertical: true)
                    .padding(10).frame(maxWidth: .infinity, alignment: .leading)
                    .background(i == playback.paragraphIndex ? Color.yellow.opacity(0.22) : Color.clear, in: RoundedRectangle(cornerRadius: 10))
                    .contentShape(Rectangle()).onTapGesture { playback.jump(toParagraph: i) }
                    .accessibilityAddTraits(.isButton).accessibilityHint("Liest ab diesem Absatz vor")
            }
            VStack(spacing: 8) {
                Text("Absatz antippen, um ab dort vorzulesen").font(.caption).foregroundStyle(VoicePalette.secondaryText)
                HStack {
                    let rec = playback.book.map { PhoneLibrary.shared.record(PhoneLibrary.shared.textKey($0)) } ?? nil
                    Text("Text: \(LibraryFormat.percent(LibraryProgress.fraction(rec))) gelesen")
                    Spacer()
                    Text("Stimme: \(currentVoiceName)")
                }.font(.caption).foregroundStyle(VoicePalette.secondaryText)
                HStack {
                    RateMenu()
                    Spacer(); ControlButton(title: "Absatz zurück", systemImage: "backward.fill") { playback.step(paragraphs: -1) }
                    Spacer(); PlayButton()
                    Spacer(); ControlButton(title: "Absatz vor", systemImage: "forward.fill") { playback.step(paragraphs: 1) }
                    Spacer(); ControlButton(title: "Stimme", systemImage: "person.wave.2") { voices = true }
                }
            }.padding(.horizontal, 20).padding(.vertical, 10).background(.bar)
        }
        .background(VoicePalette.background)
        .sheet(isPresented: $voices) { VoiceList() }
    }

    private var currentVoiceName: String {
        (playback.voiceIdentifier.flatMap { AVSpeechSynthesisVoice(identifier: $0) } ?? LibraryPlayback.defaultVoice())?.name ?? "System"
    }
}

private struct VoiceList: View {
    @ObservedObject var playback = LibraryPlayback.shared
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        NavigationStack {
            List {
                Section {
                    ForEach(LibraryPlayback.germanVoices, id: \.identifier) { v in
                        Button {
                            playback.voiceIdentifier = v.identifier; dismiss()
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
                    Text("Bessere Stimmen (Premium) lädst du kostenlos unter Einstellungen › Bedienungshilfen › Gesprochene Inhalte › Stimmen › Deutsch.")
                }
            }
            .navigationTitle("Stimme").navigationBarTitleDisplayMode(.inline)
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
                        TextField("E-Mail", text: $email).textContentType(.username).keyboardType(.emailAddress).textInputAutocapitalization(.never)
                        SecureField("Passwort", text: $password).textContentType(.password)
                        TextField("Gerätename", text: $device)
                    } footer: { Text("Dasselbe Konto wie in der Desktop-App unter Einstellungen › Konto & Geräte. Das Passwort verlässt das iPhone nicht; daraus wird nur hier der Schlüssel berechnet.") }
                    LibraryStorageSummary()
                    Section("Server") { TextField("Server", text: $url).keyboardType(.URL).textInputAutocapitalization(.never) }
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
            LabeledContent("Hörbücher auf diesem iPhone", value: LibraryBytes.text(AudioStorage.total(items)))
            if let free { LabeledContent("Frei auf dem iPhone", value: LibraryBytes.text(free)) }
            NavigationLink {
                LibraryStorageView()
            } label: {
                LabeledContent("Speicher verwalten", value: items.isEmpty ? "" : "\(items.count) geladen")
            }.accessibilityIdentifier("storage")
        } header: { Text("Speicher") } footer: {
            Text("Hörbücher liegen nur bei Bedarf auf dem iPhone und zählen nicht zum iCloud-Backup.")
        }
        .onAppear(perform: refresh)
        .onChange(of: library.books) { _, _ in refresh() }
    }

    private func refresh() { items = sync.storageItems(); free = library.freeBytes() }
}

/// Geladene Hörbücher nach Größe; Entfernen per Wischen oder gesammelt.
struct LibraryStorageView: View {
    @ObservedObject var library = PhoneLibrary.shared
    @ObservedObject var sync = LibrarySync.shared
    @State private var items: [AudioStorage.Item] = []
    @State private var free: Int64?
    @State private var working: Set<String> = []
    @State private var confirmFinished = false
    @State private var blocked: AudioOffload.Verdict?
    @State private var notice: String?

    var body: some View {
        let finished = AudioStorage.finishedRemovable(items)
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
                            .swipeActions(edge: .trailing, allowsFullSwipe: true) {
                                if item.removable {
                                    Button(role: .destructive) { remove(item) } label: { Label("Entfernen", systemImage: "icloud.and.arrow.down") }
                                }
                            }
                    }
                }
            }
        }
        .navigationTitle("Speicher").navigationBarTitleDisplayMode(.inline)
        .onAppear(perform: refresh)
        .onChange(of: library.books) { _, _ in refresh() }
        .refreshable { refresh() }
        .confirmationDialog("Fertig gehörte Hörbücher entfernen?", isPresented: $confirmFinished, titleVisibility: .visible) {
            Button("\(finished.count) entfernen · \(LibraryBytes.text(AudioStorage.total(finished))) frei", role: .destructive) {
                let ids = finished.map(\.id)
                working.formUnion(ids)
                Task {
                    let result = await sync.removeDownloads(ids: ids)
                    working.subtract(ids)
                    notice = AudioStorage.summary(removed: result.removed, freedText: LibraryBytes.text(result.freed), kept: result.kept)
                    refresh()
                }
            }
        } message: { Text("Sie bleiben im Konto und lassen sich jederzeit wieder laden.") }
        .alert("Hörbuch bleibt auf dem iPhone", isPresented: Binding(get: { blocked != nil }, set: { if !$0 { blocked = nil } }),
               presenting: blocked) { verdict in
            if verdict.syncHelps && sync.account != nil { Button("Jetzt abgleichen") { Task { await sync.run(); refresh() } } }
            Button("OK", role: .cancel) {}
        } message: { verdict in Text(verdict.message) }
    }

    private func row(_ item: AudioStorage.Item) -> some View {
        let book = library.books.first { $0.id == item.id }
        return HStack(spacing: 12) {
            LibraryCover(title: item.title, compact: true, image: book.flatMap { library.coverImage($0) }).frame(width: 40, height: 60)
            VStack(alignment: .leading, spacing: 3) {
                Text(item.title).font(.body.weight(.semibold)).lineLimit(2)
                Text(detail(item)).font(.footnote).foregroundStyle(VoicePalette.secondaryText).lineLimit(2)
                if !item.removable {
                    Label("Nur auf diesem iPhone", systemImage: "iphone").font(.caption.weight(.semibold)).foregroundStyle(.orange)
                }
            }
            Spacer(minLength: 8)
            if working.contains(item.id) { ProgressView() }
            else { Text(LibraryBytes.text(item.bytes)).font(.subheadline.monospacedDigit()).foregroundStyle(VoicePalette.secondaryText) }
        }
        .frame(minHeight: 60)
        .accessibilityElement(children: .combine)
        .accessibilityHint(item.removable ? "Zum Entfernen nach links wischen." : "Liegt nur auf diesem iPhone und lässt sich noch nicht entfernen.")
        .accessibilityAction(named: "Entfernen") { item.removable ? remove(item) : (blocked = book.map { sync.offloadVerdict($0) }) }
    }

    private func detail(_ item: AudioStorage.Item) -> String {
        guard let used = item.lastUsedMs else { return item.stand }
        return "\(item.stand) · zuletzt \(LibraryFormat.when(used))"
    }

    private func remove(_ item: AudioStorage.Item) {
        guard let book = library.books.first(where: { $0.id == item.id }) else { return }
        working.insert(item.id)
        Task {
            let result = await sync.removeDownloads(book)
            working.remove(item.id)
            if result.verdict.allowed { notice = "„\(item.title)“ entfernt, \(LibraryBytes.text(result.freed)) frei." }
            else { blocked = result.verdict }
            refresh()
        }
    }

    private func refresh() { items = sync.storageItems(); free = library.freeBytes() }
}
