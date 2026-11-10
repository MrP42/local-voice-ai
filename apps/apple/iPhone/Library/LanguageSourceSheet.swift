import SwiftUI

// Sprachfassungen eines Buchs auf dem iPhone (lva#344), nach der abgenommenen Vorschau
// `apps/local-voice/docs/vorschau/sprachfassungen/` (sf-09 Blatt „Sprache und Quelle“, sf-10 Regal).
// Ein Blatt für beides: oben die Sprache (Fassung), darunter die Quelle (Hörbuch oder Vorlesen mit Stimme),
// das Blatt „Quelle“ aus lva#326 ist darin aufgegangen.

/// Was ein Tipp auf eine Sprache tut.
private enum LanguageAction {
    case current, switchTo, download, unavailable
}

struct LanguageSourceSheet: View {
    @ObservedObject var playback = LibraryPlayback.shared
    @ObservedObject var library = PhoneLibrary.shared
    @ObservedObject var downloads = AudioDownloads.shared
    @ObservedObject var sync = LibrarySync.shared
    @Environment(\.dismiss) private var dismiss

    private var raw: LibraryBook? { playback.book.flatMap { library.rawBook($0.id) } }
    private var editions: BookEditions? { raw.map { BookEditions(book: $0) } }
    private var showsLanguages: Bool { editions?.hasLanguages == true }
    private var showsSources: Bool { SourceMenu.sourcesOffered }
    private var currentEdition: String? { playback.editionId ?? editions?.readingEdition?.id }

    var body: some View {
        NavigationStack {
            List {
                if let raw, let editions, showsLanguages {
                    Section {
                        ForEach(editions.editions) { edition in row(raw, edition) }
                    } header: {
                        Text("Sprache")
                    } footer: {
                        Text("Beim Wechsel bleibt die Stelle: kapitelgenau, wenn der PC die Kapitel bestätigt zugeordnet hat, sonst ungefähr. „Rückgängig“ stellt Sprache und Stelle wieder her.")
                    }
                } else if let source = playback.liveSource {
                    // Nur eine Fassung: das Original als Rückweg aus der Live-Übersetzung (lva#358).
                    Section("Sprache") { originalRow(source) }
                }
                LiveLanguageSections(editionLanguages: editionLanguages)
                if showsSources {
                    Section("Quelle") { sourceRows }
                }
            }
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Fertig") { dismiss() } } }
        }
        .presentationDetents([.medium, .large])
    }

    /// Sprachen, die das Buch als eigene Fassung hat (dort wechseln statt live übersetzen).
    private var editionLanguages: Set<String> {
        guard showsLanguages, let editions else { return [] }
        return Set(editions.editions.map { LiveTranslation.base($0.lang) }.filter { !$0.isEmpty })
    }

    private var title: String {
        switch (showsLanguages || LiveLanguageSections.offered, showsSources) {
        case (true, true): return "Sprache und Quelle"
        case (true, false): return "Sprache"
        default: return "Quelle"
        }
    }

    // MARK: Sprache

    private func action(_ view: LibraryBook, _ edition: BookEdition) -> LanguageAction {
        if edition.id == currentEdition { return .current }
        let audioLocal = edition.audio != nil && library.hasAudio(view)
        let textLocal = edition.text != nil && library.hasText(view)
        let downloadable = edition.audio != nil && library.audioInAccount(view) && sync.account != nil
        if playback.source == .audiobook {
            // Hörbuch läuft: ohne Hörbuch der Zielsprache erst laden (Spielen geht bis dahin weiter); nur Text wechselt zum Vorlesen.
            if audioLocal { return .switchTo }
            if downloadable { return .download }
            return textLocal ? .switchTo : .unavailable
        }
        if textLocal || audioLocal { return .switchTo }
        return downloadable ? .download : .unavailable
    }

    private func media(_ edition: BookEdition) -> String {
        var parts: [String] = []
        if let audio = edition.audio, !audio.allTracks.isEmpty || !audio.file.isEmpty {
            parts.append("Hörbuch" + (audio.narrator.map { " · \($0)" } ?? ""))
        }
        if edition.text != nil { parts.append(parts.isEmpty ? "Text" : "und Text") }
        return parts.isEmpty ? "ohne Inhalt" : parts.joined(separator: " ")
    }

    private func status(_ view: LibraryBook, _ edition: BookEdition, _ action: LanguageAction) -> String {
        // Ein Auftrag je Buch: Er gehört zu dieser Sprache, wenn alle seine Dateien ihre Hörbuchteile sind.
        let files = Set(library.accountAudio(view).enumerated().map { LibrarySync.audioFileName($1.file, index: $0) })
        if let job = downloads.jobs[view.id], edition.audio != nil, !files.isEmpty, job.parts.allSatisfy({ files.contains($0.file) }) {
            let total = max(1, job.totalBytes), done = downloads.done[view.id] ?? 0
            let percent = LibraryFormat.percent(min(1, Double(done) / Double(total)))
            switch job.phase {
            case .failed: return "Laden fehlgeschlagen"
            case .paused: return "Angehalten · \(percent)"
            case .running: return downloads.waitsForWifi(view.id) ? "Wartet auf WLAN · \(percent)" : "Lädt · \(percent)"
            }
        }
        switch action {
        case .current: return playback.liveTarget.map { "Original · live in \(LiveLanguages.name($0))" } ?? ""
        case .download:
            let bytes = library.accountAudio(view).reduce(0) { $0 + $1.size }
            return "Laden, \(LibraryBytes.text(bytes))"
        case .unavailable:
            return edition.audio == nil && edition.text != nil ? "Text wird geladen" : "nicht auf dem iPhone"
        case .switchTo:
            if edition.audio != nil, library.hasAudio(view) { return "auf dem iPhone" }
            return edition.audio == nil ? "nur Text" : "Text auf dem iPhone"
        }
    }

    @ViewBuilder private func row(_ raw: LibraryBook, _ edition: BookEdition) -> some View {
        let view = library.editionView(raw, edition: edition)
        let act = action(view, edition)
        let state = status(view, edition, act)
        let name = edition.languageName.isEmpty ? "Sprache unbekannt" : edition.languageName
        Button {
            switch act {
            case .current: playback.setLive(nil); dismiss()  // zurück zum Original, falls live übersetzt wird
            case .switchTo: playback.switchLanguage(to: edition.id); dismiss()
            case .download: playback.downloadThenSwitch(view)
            case .unavailable: break
            }
        } label: {
            HStack(spacing: 12) {
                Image(systemName: "checkmark").font(.body.weight(.semibold)).frame(width: 22)
                    .opacity(act == .current ? 1 : 0).accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 2) {
                    Text(name).font(.body)
                    Text(media(edition)).font(.footnote).foregroundStyle(VoicePalette.secondaryText)
                }
                Spacer(minLength: 8)
                if !state.isEmpty {
                    HStack(spacing: 4) {
                        if act == .download { Image(systemName: "arrow.down.circle").accessibilityHidden(true) }
                        Text(state).lineLimit(2).multilineTextAlignment(.trailing)
                    }
                    .font(.footnote).foregroundStyle(VoicePalette.secondaryText)
                }
            }
            .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(act == .unavailable)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(name), \(media(edition))")
        .accessibilityValue(act == .current ? (playback.liveTarget == nil ? "Aktuell. \(state)" : state) : state)
        .accessibilityAddTraits(act == .current ? [.isButton, .isSelected] : .isButton)
        .accessibilityHint(hint(act))
    }

    private func hint(_ action: LanguageAction) -> String {
        switch action {
        case .current: return "Schließt das Blatt"
        case .switchTo: return "Wechselt die Sprache an derselben Stelle"
        case .download: return "Lädt das Hörbuch dieser Sprache, das aktuelle läuft weiter"
        case .unavailable: return "Diese Sprache liegt noch nicht auf dem iPhone"
        }
    }

    /// Original der einzigen Fassung; Haken, solange nicht live übersetzt wird.
    private func originalRow(_ source: String) -> some View {
        let selected = playback.liveTarget == nil
        return Button { playback.setLive(nil); dismiss() } label: {
            HStack(spacing: 12) {
                Image(systemName: "checkmark").font(.body.weight(.semibold)).frame(width: 22)
                    .opacity(selected ? 1 : 0).accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 2) {
                    Text(LiveLanguages.name(source)).font(.body)
                    Text("Original").font(.footnote).foregroundStyle(VoicePalette.secondaryText)
                }
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
    }

    // MARK: Quelle

    @ViewBuilder private var sourceRows: some View {
        let audioLocal = playback.book.map { library.hasAudio($0) } ?? false
        sourceRow("Hörbuch", detail: audioLocal ? narrator : "erst im Buch laden", selected: playback.source == .audiobook, enabled: audioLocal) {
            playback.switchSource(to: .audiobook); dismiss()
        }
        sourceRow("Vorlesen", detail: "Text mit Stimme „\(playback.voiceName)“", selected: playback.source == .voice, enabled: true) {
            playback.switchSource(to: .voice); dismiss()
        }
    }

    private var narrator: String {
        playback.book?.audio?.narrator.map { "Sprecher: \($0)" } ?? "Sprecher der Hörbuchdatei"
    }

    private func sourceRow(_ title: String, detail: String, selected: Bool, enabled: Bool, _ run: @escaping () -> Void) -> some View {
        Button(action: run) {
            HStack(spacing: 12) {
                Image(systemName: "checkmark").font(.body.weight(.semibold)).frame(width: 22)
                    .opacity(selected ? 1 : 0).accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 2) {
                    Text(title).font(.body)
                    Text(detail).font(.footnote).foregroundStyle(VoicePalette.secondaryText)
                }
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(title), \(detail)")
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
        .accessibilityHint("Die Stelle im Buch bleibt")
    }
}

/// „DE ▾“ im Kopf des Players: öffnet dasselbe Blatt wie „Quelle“. Bei Büchern mit mehreren Sprachen oder mit Text
/// (Live-Übersetzung, lva#358); live zeigt es die Zielsprache mit Übersetzen-Symbol.
struct LanguageButton: View {
    @ObservedObject var playback = LibraryPlayback.shared
    @ObservedObject var library = PhoneLibrary.shared
    @State private var shown = false

    var body: some View {
        if let book = playback.book, let raw = library.rawBook(book.id) {
            let editions = BookEditions(book: raw)
            if editions.hasLanguages || LiveLanguageSections.offered {
                let current = editions.hasLanguages ? (editions.edition(playback.editionId) ?? editions.readingEdition) : nil
                let short = playback.liveTarget.map(LiveLanguages.short)
                    ?? current?.languageShort ?? playback.liveSource.map(LiveLanguages.short) ?? "–"
                let name = playback.liveTarget.map { "live übersetzt in \(LiveLanguages.name($0))" }
                    ?? current?.languageName ?? playback.liveSource.map(LiveLanguages.name) ?? "unbekannt"
                Button { shown = true } label: {
                    HStack(spacing: 2) {
                        if playback.liveTarget != nil {
                            Image(systemName: "translate").font(.caption.weight(.bold)).accessibilityHidden(true)
                        }
                        Text(short).font(.subheadline.weight(.bold))
                        Image(systemName: "chevron.down").font(.caption2.weight(.bold))
                    }
                    .foregroundStyle(VoicePalette.text)
                    .frame(minWidth: 44, minHeight: 44).contentShape(Rectangle())
                }
                .accessibilityLabel("Sprache: \(name)")
                .accessibilityHint("Öffnet Sprache und Quelle")
                .accessibilityIdentifier("languageButton")
                .sheet(isPresented: $shown) { LanguageSourceSheet() }
            }
        }
    }
}

/// Sprachkürzel im Regal (sf-10): die zuletzt aktive Sprache dunkel, die übrigen hell. Nur Anzeige.
struct LanguageBadges: View {
    let book: LibraryBook
    /// Klein für die Statuszeile der Bibliothekskachel (lva#421).
    var compact = false
    @ScaledMetric(relativeTo: .caption2) var compactSize: CGFloat = 9

    var body: some View {
        let editions = BookEditions(book: book)
        if editions.hasLanguages {
            let current = editions.readingEdition?.id
            HStack(spacing: compact ? 2 : 4) {
                ForEach(editions.editions) { e in
                    let on = e.id == current
                    Text(e.languageShort).font(compact ? .system(size: compactSize, weight: .bold) : .caption2.weight(.bold))
                        .foregroundStyle(on ? Color.white : VoicePalette.secondaryText)
                        .padding(.horizontal, compact ? 3 : 6).padding(.vertical, compact ? 1 : 2)
                        .background(on ? VoicePalette.ink : VoicePalette.border, in: RoundedRectangle(cornerRadius: 4))
                }
            }
            .accessibilityHidden(true)  // steht gesprochen im Etikett des Buchs
        }
    }

    /// Für VoiceOver: „Sprachen: Deutsch (aktuell), English“; leer ohne Sprachfassungen.
    static func spoken(_ book: LibraryBook) -> String? {
        let editions = BookEditions(book: book)
        guard editions.hasLanguages else { return nil }
        let current = editions.readingEdition?.id
        let names = editions.editions.map { ($0.languageName.isEmpty ? $0.languageShort : $0.languageName) + ($0.id == current ? " (aktuell)" : "") }
        return "Sprachen: " + names.joined(separator: ", ")
    }
}

/// Hinweis nach dem Sprachwechsel mit „Rückgängig“ (zehn Sekunden), sf-10 unten.
struct LanguageNoticeBar: View {
    @ObservedObject var playback = LibraryPlayback.shared
    @AccessibilityFocusState private var focused: Bool

    var body: some View {
        if let notice = playback.languageNotice {
            HStack(spacing: 12) {
                Text(notice.text).font(.subheadline).foregroundStyle(Color.white)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 8)
                Button("Rückgängig") { withAnimation { playback.undoLanguageSwitch() } }
                    .accessibilityFocused($focused)
                    .onChange(of: focused) { _, held in playback.noticeHeld = held }
                    .font(.subheadline.weight(.bold)).foregroundStyle(VoicePalette.brand)
                    .frame(minWidth: 44, minHeight: 44)
                    .accessibilityHint("Stellt Sprache und Stelle wie vor dem Wechsel wieder her")
            }
            .padding(.horizontal, 16).padding(.vertical, 4)
            .background(VoicePalette.ink, in: RoundedRectangle(cornerRadius: 14))
            .padding(.horizontal, 12).padding(.bottom, 8)
            .transition(.move(edge: .bottom).combined(with: .opacity))
            .accessibilityElement(children: .contain)
        }
    }
}
