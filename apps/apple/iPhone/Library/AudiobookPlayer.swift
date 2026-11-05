import SwiftUI

// Hörbuchplayer auf dem iPhone: Cover, Kapitel mit Fortschritt, Inhaltsverzeichnis
// mit Sprung, Info-Blatt mit den Angaben aus der Datei. Vorschau:
// `apps/local-voice/docs/vorschau/ios-hoerbuchplayer/`.

/// Formatierungen für Umfang und Fortschritt.
@MainActor
enum AudiobookFormat {
    static func pages(_ book: LibraryBook?, playback: LibraryPlayback) -> (pages: Int, estimated: Bool)? {
        let textLength = playback.paragraphs.last.map { $0.end }
        return AudiobookChapters.pageCount(known: book?.pages, textUTF16: textLength, totalMs: playback.totalMs)
    }

    static func pageLine(_ book: LibraryBook?, playback: LibraryPlayback) -> String? {
        guard let p = pages(book, playback: playback) else { return nil }
        let page = AudiobookChapters.estimatedPage(at: playback.positionMs, totalMs: playback.totalMs, pages: p.pages)
        return "\(p.estimated ? "etwa " : "")Seite \(page) von \(p.pages)"
    }

    static func language(_ code: String?) -> String? {
        guard let code, !code.isEmpty else { return nil }
        let short = code.count == 3 ? (["deu": "de", "ger": "de", "eng": "en", "fra": "fr", "fre": "fr", "spa": "es", "ita": "it"][code.lowercased()] ?? code) : code
        return Locale(identifier: "de_DE").localizedString(forLanguageCode: short) ?? code
    }
}

struct LibraryListenView: View {
    @ObservedObject var look = ReaderAppearance.shared
    @ObservedObject var playback = LibraryPlayback.shared
    @ObservedObject var library = PhoneLibrary.shared
    @State private var scrubbing: Double?
    /// Standard: Cover. „Text“ schaltet auf Mitlesen um.
    @State private var showText = false
    @State private var chaptersShown = false
    @State private var infoShown = false

    private var currentParagraph: Int {
        guard let map = playback.followMap else { return 0 }
        return LibraryText.paragraphIndex(playback.paragraphs, at: map.chars(atMs: playback.positionMs))
    }

    var body: some View {
        let chapters = playback.chapters
        let index = playback.chapterIndex
        VStack(spacing: 0) {
            PlayerHeader(title: playback.title,
                         subtitle: chapters.count > 1 ? "Kapitel \(index + 1) von \(chapters.count)" : "Hörbuch",
                         trailing: AnyView(HStack(spacing: 0) {
                             if showText { ReaderAppearanceButton() }
                             Button { infoShown = true } label: {
                                 Image(systemName: "info.circle").font(.title3).frame(width: 44, height: 44)
                             }.accessibilityLabel("Angaben zum Buch")
                         }))
            Divider()
            if showText, !playback.paragraphs.isEmpty { followText } else { coverArea }
            controls(chapters: chapters, index: index)
        }
        .background(VoicePalette.background)
        .sheet(isPresented: $chaptersShown) { AudiobookChapterSheet() }
        .sheet(isPresented: $infoShown) { AudiobookInfoSheet() }
    }

    // MARK: Cover

    private var coverArea: some View {
        VStack(spacing: 14) {
            Spacer(minLength: 12)
            LibraryCover(title: playback.title, image: playback.book.flatMap { library.coverImage($0) })
                .aspectRatio(library.coverAspect(playback.book), contentMode: .fit)
                .frame(maxWidth: 310)
                .shadow(color: .black.opacity(0.35), radius: 18, y: 8)
                .padding(.horizontal, 32)
            VStack(spacing: 3) {
                Text(playback.title).font(.title3.bold()).multilineTextAlignment(.center).lineLimit(2)
                if let byline { Text(byline).font(.subheadline).foregroundStyle(VoicePalette.secondaryText).multilineTextAlignment(.center).lineLimit(2) }
            }.padding(.horizontal, 24)
            Spacer(minLength: 8)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityElement(children: .combine)
    }

    private var byline: String? {
        guard let book = playback.book else { return nil }
        let authors = book.authors.isEmpty ? (playback.info?.authors ?? []) : book.authors
        let narrators = book.audio?.narrator.map { [$0] } ?? (playback.info?.narrators ?? [])
        var parts: [String] = []
        if !authors.isEmpty { parts.append("von \(authors.joined(separator: ", "))") }
        if !narrators.isEmpty { parts.append("gelesen von \(narrators.joined(separator: ", "))") }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    // MARK: Mitlesen

    private var followText: some View {
        let current = currentParagraph
        return VStack(spacing: 0) {
            FollowText(paragraphs: playback.paragraphs, current: current) { i, p in
                Text(p.text).font(look.font(heading: p.heading)).foregroundStyle(look.text)
                    .lineSpacing(look.lineSpacing).fixedSize(horizontal: false, vertical: true)
                    .padding(10).frame(maxWidth: .infinity, alignment: .leading)
                    .background(i == current ? Color.yellow.opacity(0.28) : Color.clear, in: RoundedRectangle(cornerRadius: 10))
                    .contentShape(Rectangle()).onTapGesture { playback.jump(toParagraph: i) }
                    .accessibilityAddTraits(i == current ? [.isButton, .isSelected] : .isButton)
                    .accessibilityHint("Springt im Hörbuch zu diesem Absatz")
            }
            Text(playback.followMap?.approximate == true ? "Ohne Zeitachse vom PC: Markierung nur geschätzt" : "Absatz antippen, um im Hörbuch dorthin zu springen")
                .font(.caption).foregroundStyle(look.muted).padding(.vertical, 4)
        }
        .background(look.background)
    }

    // MARK: Bedienleiste

    @ViewBuilder private func controls(chapters: [AudiobookChapter], index: Int) -> some View {
        let chapter = chapters.isEmpty ? nil : chapters[min(index, chapters.count - 1)]
        let position = scrubbing ?? playback.positionMs
        VStack(spacing: 8) {
            Button { chaptersShown = true } label: {
                HStack(spacing: 10) {
                    Image(systemName: "list.bullet").font(.body.weight(.semibold)).foregroundStyle(VoicePalette.accent)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(chapters.count > 1 ? "Kapitel \(index + 1) von \(chapters.count)" : "Inhalt").font(.caption).foregroundStyle(VoicePalette.secondaryText)
                        Text(chapter?.title ?? playback.title).font(.subheadline.weight(.semibold)).lineLimit(1)
                    }
                    Spacer(minLength: 4)
                    Image(systemName: "chevron.up").font(.footnote.weight(.semibold)).foregroundStyle(VoicePalette.secondaryText)
                }
                .padding(.horizontal, 12).frame(minHeight: 48)
                .background(VoicePalette.border.opacity(0.5), in: RoundedRectangle(cornerRadius: 12))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Inhaltsverzeichnis, Kapitel \(index + 1) von \(max(1, chapters.count)): \(chapter?.title ?? "")")

            if let chapter {
                // Regler über das laufende Kapitel; die Gesamtposition steht darunter.
                Slider(value: Binding(get: { min(chapter.durationMs, max(0, position - chapter.startMs)) },
                                      set: { scrubbing = chapter.startMs + $0 }),
                       in: 0...max(1, chapter.durationMs)) { editing in
                    if !editing, let s = scrubbing { playback.seek(toMs: s); scrubbing = nil }
                }
                .tint(VoicePalette.accent)
                .accessibilityLabel("Position im Kapitel")
                .accessibilityValue("\(LibraryFormat.clock(position - chapter.startMs)) von \(LibraryFormat.clock(chapter.durationMs))")
                HStack {
                    Text(LibraryFormat.clock(position - chapter.startMs))
                    Spacer()
                    Text("noch \(LibraryFormat.clock(max(0, chapter.endMs - position))) im Kapitel")
                }.font(.caption.monospacedDigit()).foregroundStyle(VoicePalette.secondaryText)
            }

            overall(position: position)

            HStack {
                ChapterStepButton(forward: false) { playback.previousChapter() }
                Spacer(); ControlButton(title: "15 s zurück", systemImage: "gobackward.15") { playback.skip(seconds: -15) }
                Spacer(); PlayButton()
                Spacer(); ControlButton(title: "15 s vor", systemImage: "goforward.15") { playback.skip(seconds: 15) }
                Spacer(); ChapterStepButton(forward: true) { playback.nextChapter() }
                    .disabled(index + 1 >= chapters.count).opacity(index + 1 >= chapters.count ? 0.4 : 1)
            }
            HStack {
                RateMenu()
                Spacer(); ControlButton(title: "Inhalt", systemImage: "list.bullet.rectangle") { chaptersShown = true }
                if !playback.paragraphs.isEmpty {
                    Spacer(); ControlButton(title: showText ? "Cover" : "Text", systemImage: showText ? "photo" : "text.alignleft") { showText.toggle() }
                }
                Spacer(); SleepMenu()
            }
        }
        .padding(.horizontal, 20).padding(.vertical, 10).background(.bar)
    }

    private func overall(position: Double) -> some View {
        let total = playback.totalMs
        let fraction = total > 0 ? min(1, position / total) : 0
        return VStack(spacing: 4) {
            ProgressView(value: fraction).tint(VoicePalette.accent.opacity(0.75)).scaleEffect(x: 1, y: 0.6)
            let book = "Buch \(LibraryFormat.percent(fraction)) · noch \(LibraryFormat.remaining(max(0, total - position)))"
            let pages = AudiobookFormat.pageLine(playback.book, playback: playback)
            // Eine Zeile, bei großer Schrift zwei.
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 0) { Text(book + (pages.map { " · \($0)" } ?? "")).fixedSize(); Spacer(minLength: 0) }
                VStack(alignment: .leading, spacing: 2) {
                    Text(book)
                    if let pages { Text(pages) }
                }.frame(maxWidth: .infinity, alignment: .leading)
            }
            .font(.caption.monospacedDigit()).foregroundStyle(VoicePalette.secondaryText)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Fortschritt im Buch")
        .accessibilityValue("\(LibraryFormat.percent(fraction)), noch \(LibraryFormat.remaining(max(0, total - position)))")
    }
}

/// „Kapitel zurück/vor“: sichtbar kurz „Kapitel“, die Richtung zeigt das Symbol.
private struct ChapterStepButton: View {
    let forward: Bool
    let action: () -> Void
    var body: some View {
        Button(action: action) {
            VStack(spacing: 2) {
                Image(systemName: forward ? "forward.end.fill" : "backward.end.fill").font(.title2)
                Text("Kapitel").font(.caption2).lineLimit(1)
            }.frame(minWidth: 56, minHeight: 44)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(forward ? "Nächstes Kapitel" : "Kapitel zurück")
        .accessibilityHint(forward ? "" : "Kurz nach Kapitelbeginn ins vorige Kapitel, sonst an den Kapitelanfang")
    }
}

private struct SleepMenu: View {
    @ObservedObject var playback = LibraryPlayback.shared
    var body: some View {
        let active = playback.sleepEnds != nil || playback.sleepAtMs != nil
        Menu {
            Button("Aus") { playback.sleep(minutes: nil) }
            Button("Ende des Kapitels") { playback.sleepAtChapterEnd() }
            ForEach([15, 30, 45, 60], id: \.self) { m in Button("\(m) Minuten") { playback.sleep(minutes: m) } }
        } label: {
            VStack(spacing: 2) {
                Image(systemName: active ? "moon.fill" : "moon").font(.title2)
                Text(label).font(.caption2).lineLimit(1)
            }.frame(minWidth: 56, minHeight: 44)
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Schlaftimer")
        .accessibilityValue(label)
    }
    private var label: String {
        if playback.sleepAtMs != nil { return "Kapitelende" }
        if let end = playback.sleepEnds { return "bis \(end.formatted(date: .omitted, time: .shortened))" }
        return "Schlaftimer"
    }
}

// MARK: - Inhaltsverzeichnis

struct AudiobookChapterSheet: View {
    @ObservedObject var playback = LibraryPlayback.shared
    @ObservedObject var library = PhoneLibrary.shared
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        let chapters = playback.chapters
        let current = playback.chapterIndex
        let position = playback.positionMs
        NavigationStack {
            ScrollViewReader { proxy in
                List {
                    Section { summary(chapters: chapters, current: current) }
                    Section {
                        ForEach(Array(chapters.enumerated()), id: \.offset) { i, c in
                            Button {
                                playback.seek(toChapter: i)
                                if !playback.playing { playback.play() }
                                dismiss()
                            } label: { ChapterRow(number: i + 1, chapter: c, state: i < current ? .done : i == current ? .current : .upcoming, position: position) }
                            .buttonStyle(.plain)
                            .id(i)
                            .listRowBackground(i == current ? VoicePalette.brand.opacity(0.16) : nil)
                        }
                    } header: {
                        Text(playback.info?.chaptersFromFile == false && chapters.count > 1 ? "Teile" : "Kapitel")
                    } footer: {
                        if playback.info?.chaptersFromFile == false {
                            Text("Die Datei enthält keine Kapitelmarken; angezeigt werden die Teile des Hörbuchs.")
                        }
                    }
                }
                .onAppear { proxy.scrollTo(current, anchor: .center) }
            }
            .navigationTitle("Inhalt").navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Fertig") { dismiss() } } }
        }
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
    }

    private func summary(chapters: [AudiobookChapter], current: Int) -> some View {
        let total = playback.totalMs
        let fraction = total > 0 ? min(1, playback.positionMs / total) : 0
        let remainingChapters = max(0, chapters.count - current - 1)
        return HStack(alignment: .top, spacing: 14) {
            LibraryCover(title: playback.title, compact: true, image: playback.book.flatMap { library.coverThumbnail($0) })
                .frame(width: 64, height: 64)
            VStack(alignment: .leading, spacing: 4) {
                Text(playback.title).font(.headline).lineLimit(2)
                Text("\(chapters.count) Kapitel · \(LibraryFormat.clock(total))")
                    .font(.subheadline).foregroundStyle(VoicePalette.secondaryText)
                ProgressView(value: fraction).tint(VoicePalette.accent)
                Text("\(LibraryFormat.percent(fraction)) gehört · noch \(LibraryFormat.remaining(max(0, total - playback.positionMs)))")
                    .font(.caption).foregroundStyle(VoicePalette.secondaryText)
                Text((remainingChapters == 0 ? "Letztes Kapitel" : "Es folgen noch \(remainingChapters) Kapitel")
                     + (AudiobookFormat.pageLine(playback.book, playback: playback).map { " · \($0)" } ?? ""))
                    .font(.caption).foregroundStyle(VoicePalette.secondaryText)
            }
        }
        .padding(.vertical, 4)
        .accessibilityElement(children: .combine)
    }
}

private struct ChapterRow: View {
    enum Phase { case done, current, upcoming }
    let number: Int
    let chapter: AudiobookChapter
    let state: Phase
    let position: Double

    var body: some View {
        HStack(spacing: 12) {
            ZStack {
                switch state {
                case .done: Image(systemName: "checkmark.circle.fill").foregroundStyle(VoicePalette.secondaryText)
                case .current: Image(systemName: "speaker.wave.2.circle.fill").foregroundStyle(VoicePalette.accent)
                case .upcoming: Text("\(number)").font(.subheadline.monospacedDigit()).foregroundStyle(VoicePalette.secondaryText)
                }
            }
            .font(.title3).frame(width: 30)
            VStack(alignment: .leading, spacing: 3) {
                Text(chapter.title).font(.body.weight(state == .current ? .semibold : .regular))
                    .foregroundStyle(state == .done ? VoicePalette.secondaryText : VoicePalette.text).lineLimit(2)
                if state == .current {
                    ProgressView(value: AudiobookChapters.fraction(at: position, in: chapter)).tint(VoicePalette.accent)
                    Text("läuft · noch \(LibraryFormat.clock(max(0, chapter.endMs - position)))").font(.caption).foregroundStyle(VoicePalette.secondaryText)
                } else {
                    Text(state == .done ? "gehört" : "ab \(LibraryFormat.clock(chapter.startMs))").font(.caption).foregroundStyle(VoicePalette.secondaryText)
                }
            }
            Spacer(minLength: 4)
            Text(LibraryFormat.clock(chapter.durationMs)).font(.footnote.monospacedDigit()).foregroundStyle(VoicePalette.secondaryText)
        }
        .frame(minHeight: 48)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Kapitel \(number): \(chapter.title)")
        .accessibilityValue(state == .done ? "gehört" : state == .current ? "läuft, noch \(LibraryFormat.remaining(max(0, chapter.endMs - position)))"
                            : "Dauer \(LibraryFormat.remaining(chapter.durationMs))")
        .accessibilityHint("Springt an den Anfang des Kapitels")
        .accessibilityAddTraits(state == .current ? [.isButton, .isSelected] : .isButton)
    }
}

// MARK: - Angaben zum Buch

struct AudiobookInfoSheet: View {
    @ObservedObject var playback = LibraryPlayback.shared
    @ObservedObject var library = PhoneLibrary.shared
    @Environment(\.dismiss) private var dismiss
    @State private var fullSummary = false

    var body: some View {
        let book = playback.book
        let info = playback.info
        NavigationStack {
            List {
                Section {
                    VStack(spacing: 10) {
                        let aspect = library.coverAspect(book)
                        let h = min(200, 180 / max(0.1, aspect))
                        LibraryCover(title: playback.title, image: book.flatMap { library.coverImage($0) })
                            .frame(width: h * aspect, height: h)
                        Text(playback.title).font(.title3.bold()).multilineTextAlignment(.center)
                        if let sub = book?.subtitle ?? info?.subtitle { Text(sub).font(.subheadline).foregroundStyle(VoicePalette.secondaryText).multilineTextAlignment(.center) }
                    }
                    .frame(maxWidth: .infinity).padding(.vertical, 6)
                    .listRowBackground(Color.clear)
                    .accessibilityElement(children: .combine)
                }
                Section("Angaben") {
                    row("Autor", people(book?.authors, info?.authors))
                    row("Gelesen von", book?.audio?.narrator ?? people(nil, info?.narrators))
                    row("Reihe", info?.series)
                    row("Verlag", book?.publisher ?? info?.publisher)
                    row("Erschienen", AudiobookInfo.displayDate(book?.publishDate ?? info?.releaseDate))
                    row("Genre", info?.genre)
                    row("Sprache", AudiobookFormat.language(book?.language ?? info?.language))
                    row("ISBN", book?.isbn)
                }
                Section("Umfang") {
                    row("Dauer", playback.totalMs > 0 ? LibraryFormat.remaining(playback.totalMs) : nil)
                    row("Kapitel", playback.chapters.count > 0 ? "\(playback.chapters.count)" : nil)
                    if let p = AudiobookFormat.pages(book, playback: playback) {
                        row("Seiten", p.estimated ? "etwa \(p.pages) (geschätzt)" : "\(p.pages)")
                    }
                    if let tracks = book?.audio?.allTracks, tracks.count > 1 { row("Dateien", "\(tracks.count) Teile") }
                    row("Format", format(info))
                    row("Größe", info?.fileBytes.map { ByteCountFormatter.string(fromByteCount: $0, countStyle: .file) })
                }
                if let summary = book?.description ?? info?.summary, !summary.isEmpty {
                    Section("Beschreibung") {
                        Text(summary).font(.body).lineLimit(fullSummary ? nil : 6)
                        if summary.count > 280 {
                            Button(fullSummary ? "Weniger anzeigen" : "Mehr anzeigen") { fullSummary.toggle() }.frame(minHeight: 44)
                        }
                    }
                }
                Section {
                    row("Copyright", info?.copyright)
                    row("Erstellt mit", info?.encoder)
                } footer: {
                    Text(info == nil ? "Angaben werden aus der Hörbuchdatei gelesen …"
                         : "Angaben aus der Hörbuchdatei\(book?.metadataSource == nil ? "" : " und der Buchsuche am PC"). Kapitel \(info?.chaptersFromFile == true ? "aus den Kapitelmarken der Datei" : "nach den Teilen des Hörbuchs").")
                }
            }
            .navigationTitle("Info").navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Fertig") { dismiss() } } }
        }
    }

    @ViewBuilder private func row(_ label: String, _ value: String?) -> some View {
        if let value, !value.isEmpty { LabeledContent(label) { Text(value).multilineTextAlignment(.trailing) } }
    }

    private func people(_ primary: [String]?, _ fallback: [String]?) -> String? {
        let list = (primary?.isEmpty == false ? primary : fallback) ?? []
        return list.isEmpty ? nil : list.joined(separator: ", ")
    }

    private func format(_ info: AudiobookInfo?) -> String? {
        guard let info else { return nil }
        let parts = [info.format, info.codec, info.bitrateKbps.map { "\($0) kbit/s" }].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }
}

extension PhoneLibrary {
    /// Seitenverhältnis des Covers: echtes Bild wie geliefert (Hörbücher meist
    /// quadratisch), erzeugtes Cover im Buchformat 2:3.
    func coverAspect(_ book: LibraryBook?) -> CGFloat {
        guard let book, let image = coverImage(book), image.size.height > 0 else { return 2 / 3 }
        return image.size.width / image.size.height
    }
}
