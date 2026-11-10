import SwiftUI

// Inhalt und Suche eines Textbuchs (lva#476, Patrick 10.11.: „Wie soll man da eine Geschichte
// finden? Und es gibt auch keine Schlagwortsuche in Büchern“). Ohne Suchbegriff das
// Inhaltsverzeichnis aus den Überschriften, mit Suchbegriff die Treffer im ganzen Buch
// (gefaltet wie die Ablage: „strasse“ findet „Straße“). Ein Tipp schlägt die Stelle auf.

/// Ziel eines Sprungs aus Inhalt oder Suche.
struct BookFindTarget: Equatable {
    var paragraph: Int
    /// Treffer im Absatz (UTF-16); `nil` beim Sprung zu einer Überschrift.
    var range: NSRange?
}

struct BookFindSheet: View {
    let paragraphs: [TextParagraph]
    /// Aktueller Absatz, für das Hervorheben des Kapitels.
    let current: Int
    let onGo: (BookFindTarget) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var query = ""
    @State private var index: BookSearchIndex?
    @State private var result = BookSearchResult.empty
    @State private var building = false

    private var contents: [BookContentsEntry] { BookContents.entries(paragraphs) }
    private var searching: Bool { BookSearch.fold(text: query).count >= BookSearch.minQuery }

    var body: some View {
        let entries = contents
        NavigationStack {
            ScrollViewReader { proxy in
                List {
                    if searching { results(entries) } else { toc(entries) }
                }
                .onAppear {
                    if let c = BookContents.chapter(of: current, in: entries) { proxy.scrollTo("toc-\(c)", anchor: .center) }
                }
            }
            .searchable(text: $query, placement: .navigationBarDrawer(displayMode: .always), prompt: "Im Buch suchen")
            .autocorrectionDisabled()
            .textInputAutocapitalization(.never)
            .task(id: query) { await run() }
            .navigationTitle("Inhalt und Suche").navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Fertig") { dismiss() } } }
        }
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
    }

    @ViewBuilder private func toc(_ entries: [BookContentsEntry]) -> some View {
        let here = BookContents.chapter(of: current, in: entries)
        Section {
            if entries.isEmpty {
                Text("Das Buch hat keine Überschriften. Suchen Sie oben nach einem Wort.")
                    .font(.callout).foregroundStyle(VoicePalette.secondaryText)
            }
            ForEach(Array(entries.enumerated()), id: \.offset) { i, e in
                Button { go(BookFindTarget(paragraph: e.paragraph, range: nil)) } label: {
                    Text(e.title).foregroundStyle(VoicePalette.text).frame(maxWidth: .infinity, minHeight: 32, alignment: .leading)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .id("toc-\(i)")
                .listRowBackground(i == here ? VoicePalette.brand.opacity(0.16) : nil)
                .accessibilityAddTraits(i == here ? [.isSelected] : [])
            }
        } header: {
            Text(entries.isEmpty ? "Inhalt" : "Inhalt · \(entries.count) Kapitel")
        }
    }

    @ViewBuilder private func results(_ entries: [BookContentsEntry]) -> some View {
        Section {
            if building {
                HStack { ProgressView(); Text("Buch wird durchsucht …").foregroundStyle(VoicePalette.secondaryText) }
            } else if result.total == 0 {
                Text("Keine Treffer").foregroundStyle(VoicePalette.secondaryText)
            }
            ForEach(Array(result.hits.enumerated()), id: \.offset) { _, hit in
                Button { go(BookFindTarget(paragraph: hit.paragraph, range: hit.range)) } label: {
                    VStack(alignment: .leading, spacing: 2) {
                        if let c = BookContents.chapter(of: hit.paragraph, in: entries) {
                            Text(entries[c].title).font(.caption.weight(.semibold)).foregroundStyle(VoicePalette.secondaryText).lineLimit(1)
                        }
                        Text(snippet(hit)).font(.callout).foregroundStyle(VoicePalette.text).lineLimit(3)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("\(hit.before)\(hit.match)\(hit.after)")
                .accessibilityHint("Schlägt die Fundstelle auf")
            }
        } header: {
            if !building, result.total > 0 { Text(summary) }
        }
    }

    private var summary: String {
        if result.allWords { return result.total == 1 ? "1 Absatz mit allen Wörtern" : "\(result.total) Absätze mit allen Wörtern" }
        if result.total > result.hits.count { return "\(result.total) Treffer, die ersten \(result.hits.count) angezeigt" }
        return "\(result.total) Treffer"
    }

    private func snippet(_ hit: BookSearchHit) -> AttributedString {
        var match = AttributedString(hit.match)
        match.font = .callout.weight(.semibold)
        match.backgroundColor = VoicePalette.brand.opacity(0.35)
        return AttributedString(hit.before) + match + AttributedString(hit.after)
    }

    /// Suchen, kurz entprellt; der Index entsteht einmal, abseits des Haupt-Threads.
    private func run() async {
        guard searching else { result = .empty; return }
        try? await Task.sleep(for: .milliseconds(150))
        guard !Task.isCancelled else { return }
        let built: BookSearchIndex
        if let index { built = index } else {
            building = true
            let source = paragraphs
            built = await Task.detached(priority: .userInitiated) { BookSearchIndex(paragraphs: source) }.value
            index = built
            building = false
        }
        guard !Task.isCancelled else { return }
        let q = query
        result = await Task.detached(priority: .userInitiated) { BookSearch.search(built, query: q) }.value
    }

    private func go(_ target: BookFindTarget) {
        onGo(target)
        dismiss()
    }
}
