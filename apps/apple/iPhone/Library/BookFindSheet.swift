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

/// Suchzustand des Sheets. Der Index entsteht genau einmal (ein gemeinsamer Task für alle Anfragen),
/// jede Anfrage trägt eine Generation: ein Ergebnis, das nach einer neueren Eingabe fertig wird,
/// überschreibt nichts. Abbruch läuft bis in die Index- und Suchschleifen.
@MainActor
final class BookFindModel: ObservableObject {
    @Published private(set) var result = BookSearchResult.empty
    @Published private(set) var building = false
    private var index: Task<BookSearchIndex?, Never>?
    private var generation = 0

    func search(query: String, paragraphs: [TextParagraph]) async {
        generation += 1
        let mine = generation
        guard BookSearch.fold(text: query).count >= BookSearch.minQuery else {
            result = .empty
            building = false
            return
        }
        try? await Task.sleep(for: .milliseconds(150))
        guard mine == generation, !Task.isCancelled else { return }
        if index == nil { building = true }
        let build = indexTask(paragraphs)
        guard let built = await build.value else { return }
        guard mine == generation, !Task.isCancelled else { return }
        let work = Task.detached(priority: .userInitiated) {
            BookSearch.search(built, query: query, limit: 300, isCancelled: { Task.isCancelled })
        }
        let found = await withTaskCancellationHandler { await work.value } onCancel: { work.cancel() }
        guard mine == generation, !Task.isCancelled, let found else { return }
        result = found
        building = false
    }

    /// Beim Schließen: laufenden Indexaufbau abbrechen und späte Ergebnisse verwerfen.
    func stop() {
        generation += 1
        index?.cancel()
        index = nil
        building = false
    }

    private func indexTask(_ paragraphs: [TextParagraph]) -> Task<BookSearchIndex?, Never> {
        if let index { return index }
        let task = Task.detached(priority: .userInitiated) {
            BookSearchIndex(paragraphs: paragraphs, isCancelled: { Task.isCancelled })
        }
        index = task
        return task
    }
}

struct BookFindSheet: View {
    let paragraphs: [TextParagraph]
    /// Aktueller Absatz, für das Hervorheben des Kapitels.
    let current: Int
    let onGo: (BookFindTarget) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var query = ""
    @StateObject private var model = BookFindModel()

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
            .task(id: query) { await model.search(query: query, paragraphs: paragraphs) }
            .onDisappear { model.stop() }
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
            if model.building {
                HStack { ProgressView(); Text("Buch wird durchsucht …").foregroundStyle(VoicePalette.secondaryText) }
            } else if model.result.total == 0 {
                Text("Keine Treffer").foregroundStyle(VoicePalette.secondaryText)
            }
            ForEach(Array(model.result.hits.enumerated()), id: \.offset) { _, hit in
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
            if !model.building, model.result.total > 0 { Text(summary) }
        }
    }

    private var summary: String {
        let result = model.result
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

    private func go(_ target: BookFindTarget) {
        onGo(target)
        dismiss()
    }
}
