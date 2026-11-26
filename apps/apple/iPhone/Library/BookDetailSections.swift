import SwiftUI

// Buchseite (lva#795): alle Fassungen mit eigenem Fortschritt, Klappentext zum Vorlesen (lva#803),
// Inhaltsverzeichnis zum Springen (lva#802) und Verlauf. Die Abschnitte zeigen nur, was auf dem
// iPhone wirklich vorliegt; was fehlt, bleibt weg statt leer zu erscheinen.

/// Eine abspielbare Fassung des Buchs: eine Sprache × ein Medium (Text oder Hörbuch).
struct BookVersionRow: Identifiable {
    enum Medium { case text, audio }
    let id: String
    let view: LibraryBook
    let medium: Medium
    let language: String
    let record: ProgressRecord?
    let fraction: Double
    let detail: String
    let active: Bool
}

/// Öffnet eine bestimmte Fassung. Liegt dasselbe Buch schon im Player, bleibt `open` sonst in der laufenden
/// Fassung (`resolvedView`); deshalb zuerst die Sprache wechseln (gleiche Stelle, Hinweis mit Rückgängig), dann das Medium.
@MainActor
enum BookOpen {
    static func open(_ view: LibraryBook, prefer: PlayerSource?) {
        let playback = LibraryPlayback.shared
        if playback.book?.id == view.id, let target = view.editionId, playback.editionId != target {
            playback.switchLanguage(to: target)
        }
        playback.open(view, prefer: prefer)
    }

    /// Öffnet den Player und springt an eine Stelle des Texts (normalisierte UTF-16-Offsets wie im Player). Ein laufendes
    /// Hörbuch bleibt Hörbuch und springt mit; sonst öffnet der Text. Spielt nicht von selbst los.
    static func open(_ view: LibraryBook, atOffset offset: Int) {
        let playback = LibraryPlayback.shared
        let running = playback.book?.id == view.id
        open(view, prefer: running ? nil : .voice)
        guard playback.book?.id == view.id, playback.editionId == view.editionId else { return }
        playback.jump(toOffset: offset, autoplay: false)
    }
}

@MainActor
enum BookVersions {
    /// Alle Fassungen in der Reihenfolge des Buchs (Hauptfassung zuerst), je Sprache Text vor Hörbuch.
    static func rows(_ raw: LibraryBook, active: LibraryBook, library: PhoneLibrary) -> [BookVersionRow] {
        var views: [LibraryBook] = []
        let editions = BookEditions(book: raw)
        if (raw.editions?.arrayValue?.count ?? 0) > 1, editions.hasLanguages {
            var order = editions.editions
            if let primary = editions.primary, !order.contains(where: { $0.id == primary.id }) { order.insert(primary, at: 0) }
            views = order.map { library.editionView(raw, edition: $0) }
        } else {
            views = [active]
        }
        let multi = views.count > 1
        var out: [BookVersionRow] = []
        for v in views {
            let lang = BookEditions.languageName(v.language)
            let isActive = !multi || v.editionId == active.editionId
            if library.hasText(v) {
                let rec = library.record(library.textKey(v))
                let f = LibraryProgress.fraction(rec)
                out.append(BookVersionRow(id: "t:\(v.editionId ?? v.id)", view: v, medium: .text, language: lang,
                                          record: rec, fraction: f,
                                          detail: rec == nil ? "noch nicht gelesen" : "\(LibraryFormat.percent(f)) gelesen",
                                          active: isActive))
            }
            if library.hasAudio(v) {
                let rec = library.record(library.audioKey(v))
                let f = LibraryProgress.fraction(rec)
                let total = v.audio?.totalDurationMs ?? 0
                let detail = rec.map { "\(LibraryFormat.clock($0.offset)) von \(LibraryFormat.clock(total))" } ?? "noch nicht gehört"
                out.append(BookVersionRow(id: "a:\(v.editionId ?? v.id)", view: v, medium: .audio, language: lang,
                                          record: rec, fraction: f, detail: detail, active: isActive))
            }
        }
        return out
    }
}

/// „Fassungen“: jede Sprache und jedes Medium mit eigenem Fortschritt; Tippen öffnet genau diese Fassung an ihrer Stelle.
struct BookVersionsSection: View {
    let rows: [BookVersionRow]
    let multiLanguage: Bool
    @ObservedObject var playback = LibraryPlayback.shared
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        if !rows.isEmpty {
            VStack(alignment: .leading, spacing: 8) {
                BookSectionHeader("Fassungen und Fortschritt")
                VStack(spacing: 0) {
                    ForEach(Array(rows.enumerated()), id: \.element.id) { i, row in
                        if i > 0 { Divider().padding(.leading, 44) }
                        Button { open(row) } label: { label(row) }
                            .buttonStyle(.plain)
                            .accessibilityIdentifier("bookVersion-\(row.id)")
                    }
                }.voiceCard()
                Text("Jede Fassung behält ihre eigene Stelle. Beim Wechsel der Sprache springt die App an die passende Stelle der anderen Fassung; die Live-Übersetzung liest im Originaltext mit und zählt dort.")
                    .font(.caption).foregroundStyle(VoicePalette.secondaryText)
            }
        }
    }

    private func label(_ row: BookVersionRow) -> some View {
        HStack(alignment: .center, spacing: 12) {
            Image(systemName: row.medium == .text ? "text.book.closed" : "headphones")
                .font(.body).frame(width: 32, height: 32)
                .foregroundStyle(row.active ? VoicePalette.accent : VoicePalette.secondaryText)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 4) {
                // Ab großer Schrift (xLarge) untereinander und umbrechend statt abgeschnitten.
                let stacked = typeSize >= .xLarge
                let lineLayout = stacked
                    ? AnyLayout(VStackLayout(alignment: .leading, spacing: 2))
                    : AnyLayout(HStackLayout(spacing: 6))
                lineLayout {
                    Text(title(row)).fontWeight(.semibold).lineLimit(stacked ? nil : 1)
                    if !stacked { Spacer(minLength: 4) }
                    Text(row.detail).font(.footnote).foregroundStyle(VoicePalette.secondaryText)
                        .lineLimit(stacked ? nil : 1)
                }
                ProgressView(value: row.fraction).tint(VoicePalette.accent)
                if let when = row.record?.updated, when > 0 {
                    Text(row.active && multiLanguage ? "Zuletzt aktiv · \(LibraryFormat.when(when))" : "Zuletzt \(LibraryFormat.when(when))")
                        .font(.caption)
                        .foregroundStyle(row.active && multiLanguage ? VoicePalette.accent : VoicePalette.secondaryText)
                }
            }
        }
        .padding(.vertical, 10)
        .frame(minHeight: 44)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        .accessibilityHint("Öffnet diese Fassung an ihrer Stelle.")
    }

    private func title(_ row: BookVersionRow) -> String {
        let medium = row.medium == .text ? "Text" : "Hörbuch"
        return multiLanguage ? "\(medium) · \(row.language)" : medium
    }

    private func open(_ row: BookVersionRow) {
        BookOpen.open(row.view, prefer: row.medium == .text ? .voice : .audiobook)
    }
}

/// Klappentext (lva#803): gekürzt mit „Mehr“, vorlesbar mit der Gesprächsstimme.
struct BookBlurbSection: View {
    let text: String
    let language: String?
    @State private var expanded = false
    @StateObject private var reader = BlurbReader()
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            // Ab großer Schrift untereinander: sonst trennt die Überschrift („KLAP-“) und der Knopf schrumpft.
            let headLayout = typeSize >= .xxLarge
                ? AnyLayout(VStackLayout(alignment: .leading, spacing: 8))
                : AnyLayout(HStackLayout())
            headLayout {
                BookSectionHeader("Klappentext")
                if typeSize < .xxLarge { Spacer() }
                Button {
                    reader.toggle(text, language: language)
                } label: {
                    Label(reader.speaking ? "Stopp" : "Vorlesen", systemImage: reader.speaking ? "stop.fill" : "speaker.wave.2.fill")
                        .font(.footnote.weight(.semibold))
                }
                .buttonStyle(.bordered).tint(VoicePalette.accent)
                .fixedSize()
                .accessibilityIdentifier("blurbSpeak")
            }
            Text(text)
                .font(.callout)
                .lineLimit(expanded ? nil : 5)
                .frame(maxWidth: .infinity, alignment: .leading)
            Button(expanded ? "Weniger" : "Mehr") { withAnimation(.easeOut(duration: 0.2)) { expanded.toggle() } }
                .font(.footnote.weight(.semibold)).foregroundStyle(VoicePalette.accent)
        }
        .voiceCard()
        .onDisappear { reader.stop() }
    }
}

/// Liest den Klappentext satzweise mit der Gesprächsstimme (Phonon „Anton“, sonst Systemstimme).
@MainActor
final class BlurbReader: ObservableObject {
    @Published private(set) var speaking = false
    private var speaker: BookTalkSpeaker?

    func toggle(_ text: String, language: String?) {
        if speaking { stop(); return }
        if LibraryPlayback.shared.busy { LibraryPlayback.shared.pause() }
        let s = BookTalkSpeaker()
        s.onIdle = { [weak self] in Task { @MainActor in self?.speaking = false; self?.speaker = nil } }
        s.begin(language: BookEditions.languageCode(language) == "en" ? .en : .de)
        for sentence in Self.sentences(text) { s.enqueue(sentence) }
        s.endOfInput()
        speaker = s
        speaking = true
    }

    func stop() {
        speaker?.stop()
        speaker = nil
        speaking = false
    }

    static func sentences(_ text: String) -> [String] {
        var out: [String] = []
        text.enumerateSubstrings(in: text.startIndex..., options: .bySentences) { sub, _, _, _ in
            if let s = sub?.trimmingCharacters(in: .whitespacesAndNewlines), !s.isEmpty { out.append(s) }
        }
        return out.isEmpty ? [text] : out
    }
}

/// Inhaltsverzeichnis (lva#802): Kapitel aus den Überschriften des Texts, aktuelles Kapitel markiert,
/// gelesene mit Haken. Tippen öffnet den Text am Kapitelanfang.
struct BookContentsSection: View {
    let view: LibraryBook
    let record: ProgressRecord?
    @ObservedObject var playback = LibraryPlayback.shared
    @State private var entries: [BookContentsEntry] = []
    @State private var paragraphs: [TextParagraph] = []
    @State private var showAll = false
    private let preview = 8

    var body: some View {
        // Der Container ist immer da (auch ohne Kapitel), sonst startet `.task` nie und der Abschnitt bliebe leer.
        VStack(alignment: .leading, spacing: 0) {
            Color.clear.frame(height: 0).accessibilityHidden(true)
            if !entries.isEmpty {
                VStack(alignment: .leading, spacing: 8) {
                    HStack {
                        BookSectionHeader("Inhalt")
                        Spacer()
                        Text("\(entries.count) \(entries.count == 1 ? "Kapitel" : "Kapitel")").font(.caption).foregroundStyle(VoicePalette.secondaryText)
                    }
                    let current = currentChapter
                    let shown = showAll ? Array(entries.enumerated()) : Array(entries.enumerated().prefix(max(preview, (current ?? 0) + 2)))
                    VStack(spacing: 0) {
                        ForEach(shown, id: \.offset) { i, entry in
                            if i > 0 { Divider() }
                            Button { BookOpen.open(view, atOffset: paragraphs[entry.paragraph].textStart) } label: {
                                HStack(spacing: 10) {
                                    Image(systemName: i == current ? "play.circle.fill" : (done(i) ? "checkmark.circle" : "circle"))
                                        .foregroundStyle(i == current ? VoicePalette.accent : VoicePalette.secondaryText)
                                        .accessibilityHidden(true)
                                    Text(entry.title).lineLimit(2).multilineTextAlignment(.leading)
                                        .fontWeight(i == current ? .semibold : .regular)
                                    Spacer(minLength: 0)
                                }
                                .frame(minHeight: 44).contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .accessibilityValue(i == current ? "Hier bist du" : (done(i) ? "gelesen" : ""))
                        }
                    }.voiceCard()
                    if entries.count > shown.count || showAll {
                        Button(showAll ? "Weniger anzeigen" : "Alle \(entries.count) Kapitel") { showAll.toggle() }
                            .font(.footnote.weight(.semibold)).foregroundStyle(VoicePalette.accent)
                    }
                }
            }
        }
        .task(id: view.editionId ?? view.id) { await load() }
    }

    private var currentChapter: Int? {
        guard let record else { return nil }
        let offset = Int(record.offset)
        guard let para = paragraphs.lastIndex(where: { $0.start <= offset }) else { return nil }
        return BookContents.chapter(of: para, in: entries)
    }

    /// Kapitel gilt als gelesen, wenn der weiteste Stand hinter seinem Ende liegt.
    private func done(_ i: Int) -> Bool {
        guard let record else { return false }
        let reach = max(record.frontier ?? 0, record.offset)
        let endPara = i + 1 < entries.count ? entries[i + 1].paragraph : paragraphs.count
        guard endPara > 0, endPara - 1 < paragraphs.count else { return false }
        return reach >= Double(paragraphs[endPara - 1].end)
    }

    /// Liest und zerlegt den Text abseits des MainActors, im selben normalisierten Text wie der Player
    /// (`LiveTranslation.hashInput`), damit Offsets und Absätze übereinstimmen. `.task(id:)` bricht einen alten Lauf
    /// beim Fassungswechsel ab; ein abgebrochener oder überholter Lauf schreibt nichts.
    private func load() async {
        let library = PhoneLibrary.shared
        let key = view.editionId ?? view.id
        guard library.hasText(view) else { entries = []; paragraphs = []; return }
        let url = library.textURL(view)
        let work = Task.detached(priority: .utility) { () -> ([TextParagraph], [BookContentsEntry])? in
            guard let raw = try? String(contentsOf: url, encoding: .utf8), !Task.isCancelled else { return nil }
            let paras = LibraryText.paragraphs(LiveTranslation.hashInput(raw))
            return Task.isCancelled ? nil : (paras, BookContents.entries(paras))
        }
        let result = await withTaskCancellationHandler { await work.value } onCancel: { work.cancel() }
        guard !Task.isCancelled, key == (view.editionId ?? view.id), let result else { return }
        paragraphs = result.0
        entries = result.1
    }
}

/// Verlauf: Lesesitzungen aus dem Leseprotokoll (`reading:<buch>`, #795), je Tag gruppiert. Ohne Protokoll
/// (ältere Stände) bleibt die bisherige Anzeige: welche Fassung zuletzt gelesen oder gehört wurde und bis wohin.
struct BookHistorySection: View {
    let rows: [BookVersionRow]
    var log: [ReadingLogEntry] = []
    var device: String = ""

    var body: some View {
        let days = ReadingHistory.days(log, calendar: .current)
        if !days.isEmpty {
            sessions(days)
        } else {
            fallback
        }
    }

    private func sessions(_ days: [ReadingHistory.Day]) -> some View {
        let now = Date()
        return VStack(alignment: .leading, spacing: 8) {
            BookSectionHeader("Verlauf")
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(days.enumerated()), id: \.element.start) { d, day in
                    if d > 0 { Divider() }
                    Text(ReadingHistory.dayLabel(day.start, now: now, calendar: .current))
                        .font(.subheadline.weight(.semibold))
                        .padding(.top, 10).padding(.bottom, 2)
                        .accessibilityAddTraits(.isHeader)
                    ForEach(Array(day.entries.enumerated()), id: \.offset) { _, entry in
                        HStack(alignment: .firstTextBaseline, spacing: 10) {
                            Image(systemName: Self.icon(entry.medium)).foregroundStyle(VoicePalette.secondaryText)
                                .frame(width: 20).accessibilityHidden(true)
                            VStack(alignment: .leading, spacing: 2) {
                                Text("\(ReadingHistory.mediumLabel(entry.medium)) · \(ReadingHistory.duration(entry))")
                                    .font(.subheadline)
                                Text(detail(entry)).font(.caption).foregroundStyle(VoicePalette.secondaryText)
                            }
                            Spacer(minLength: 0)
                        }
                        .padding(.vertical, 6)
                        .accessibilityElement(children: .combine)
                    }
                }
            }
            .padding(.bottom, 4)
            .voiceCard()
        }
    }

    private func detail(_ entry: ReadingLogEntry) -> String {
        var parts = [ReadingHistory.timeSpan(entry, calendar: .current)]
        if !device.isEmpty, entry.device != device { parts.append("anderes Gerät") }
        return parts.joined(separator: " · ")
    }

    private static func icon(_ medium: ReadingMedium) -> String {
        switch medium {
        case .text: return "eye"
        case .audio: return "ear"
        case .live: return "globe"
        case .kurzbuch: return "text.badge.star"
        }
    }

    @ViewBuilder private var fallback: some View {
        let used = rows.filter { ($0.record?.updated ?? 0) > 0 }.sorted { ($0.record?.updated ?? 0) > ($1.record?.updated ?? 0) }
        if !used.isEmpty {
            VStack(alignment: .leading, spacing: 8) {
                BookSectionHeader("Verlauf")
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(used.enumerated()), id: \.element.id) { i, row in
                        if i > 0 { Divider() }
                        HStack(alignment: .firstTextBaseline, spacing: 10) {
                            Image(systemName: row.medium == .text ? "eye" : "ear").foregroundStyle(VoicePalette.secondaryText)
                                .accessibilityHidden(true)
                            VStack(alignment: .leading, spacing: 2) {
                                Text("\(row.medium == .text ? "Gelesen" : "Gehört") · \(row.language)").font(.subheadline.weight(.semibold))
                                Text("\(LibraryFormat.when(row.record?.updated ?? 0)) · bis \(LibraryFormat.percent(row.fraction))")
                                    .font(.caption).foregroundStyle(VoicePalette.secondaryText)
                            }
                            Spacer(minLength: 0)
                        }
                        .padding(.vertical, 8)
                        .accessibilityElement(children: .combine)
                    }
                }.voiceCard()
            }
        }
    }
}

/// Kleine Abschnittsüberschrift wie „HÖRBUCH-TEILE“.
struct BookSectionHeader: View {
    let title: String
    init(_ title: String) { self.title = title }
    var body: some View {
        Text(title.uppercased()).font(.caption.weight(.semibold)).tracking(0.6)
            .foregroundStyle(VoicePalette.secondaryText)
            .accessibilityAddTraits(.isHeader)
    }
}
