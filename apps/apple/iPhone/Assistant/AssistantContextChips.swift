import SwiftUI

// Verknüpfte Inhalte im Gespräch „Sprechen“ (lva#557, Paket G3): Modelle und die Chip-Zeile. Ein Kontext ist ein Verweis (Art, Kennung,
// Titel) auf einen Inhalt im Index, nie eine Kopie. Die Chips zeigen die Kontexte des Assistenten (nicht entfernbar, „vom Assistenten“)
// und die des Gesprächs (entfernbar); ein Verweis, den es nicht mehr gibt, steht grau mit „nicht mehr verfügbar“, und das Gespräch läuft
// ohne ihn weiter. Ohne Kontexte gibt es keine Zeile.

/// Anzeigetexte und Symbole der Kontextarten.
enum AssistantContextText {
    static func kind(_ kind: AssistantProfile.Context.Kind) -> String {
        switch kind {
        case .book: "Buch"
        case .meeting: "Besprechung"
        case .voiceNote: "Sprachnotiz"
        case .file: "Datei"
        case .folder: "Ordner"
        case .project: "Projekt"
        case .section: "Abschnitt"
        }
    }

    static func symbol(_ kind: AssistantProfile.Context.Kind) -> String {
        switch kind {
        case .book: "book"
        case .meeting: "person.2"
        case .voiceNote: "mic"
        case .file: "doc.text"
        case .folder: "folder"
        case .project: "square.stack.3d.up"
        case .section: "text.alignleft"
        }
    }

    /// Hinweis zur Verfügbarkeit; `nil`, wenn verfügbar.
    static func note(_ availability: AssistantContextAvailability) -> String? {
        switch availability {
        case .available: nil
        case .gone: "nicht mehr verfügbar"
        case .unsupported: "auf diesem Gerät nicht verfügbar"
        }
    }
}

/// Ein Kontext als Chip der Chip-Zeile.
struct AssistantContextChip: Identifiable, Equatable {
    let kind: AssistantProfile.Context.Kind
    let contextId: String
    let title: String
    let fromAssistant: Bool
    let availability: AssistantContextAvailability

    init(context: AssistantProfile.Context, fromAssistant: Bool, availability: AssistantContextAvailability) {
        kind = context.kind
        contextId = context.id
        title = context.title
        self.fromAssistant = fromAssistant
        self.availability = availability
    }

    var id: String { (fromAssistant ? "assistent/" : "gespraech/") + Self.key(kind: kind, id: contextId) }
    var removable: Bool { !fromAssistant }
    var displayTitle: String { title.isEmpty ? AssistantContextText.kind(kind) : title }

    /// Zweite Zeile im Chip: „nicht mehr verfügbar“, sonst „vom Assistenten“ (nur bei Kontexten des Assistenten).
    var caption: String? {
        if let note = AssistantContextText.note(availability) { return note }
        return fromAssistant ? "vom Assistenten" : nil
    }

    /// Satz für VoiceOver: „Buch: Titel, vom Assistenten“ bzw. „… nicht mehr verfügbar“.
    var spokenLabel: String {
        var parts = ["\(AssistantContextText.kind(kind)): \(displayTitle)"]
        if let note = AssistantContextText.note(availability) { parts.append(note) }
        if fromAssistant { parts.append("vom Assistenten") }
        return parts.joined(separator: ", ")
    }

    static func key(kind: AssistantProfile.Context.Kind, id: String) -> String { kind.rawValue + "/" + id }
    static func key(_ context: AssistantProfile.Context) -> String { key(kind: context.kind, id: context.id) }
}

/// Ein Inhalt, den das Anhänge-Blatt anbietet.
struct AssistantContextCandidate: Identifiable, Equatable {
    let area: AssistantArea
    let kind: AssistantProfile.Context.Kind
    let itemId: String
    let title: String
    let detail: String?

    var id: String { key }
    var key: String { AssistantContextChip.key(kind: kind, id: itemId) }

    init(area: AssistantArea, itemId: String, title: String, detail: String? = nil) {
        self.area = area
        kind = AssistantScope.kind(of: area)
        self.itemId = itemId
        self.title = Self.cleaned(title, fallback: area.singularLabel)
        self.detail = detail
    }

    /// Nur Inhalte mit einer Kennung, die das Schema der Assistenten fasst (1 bis 200 Zeichen).
    init?(item: AssistantItem) {
        guard !item.id.isEmpty, item.id.count <= 200 else { return nil }
        self.init(area: item.area, itemId: item.id, title: item.title, detail: item.detail)
    }

    /// Ein Zeilenumbruch wird zum Leerzeichen, der Titel hat höchstens 200 Zeichen (Schema).
    private static func cleaned(_ title: String, fallback: String) -> String {
        let flat = title.split(whereSeparator: { $0.isNewline || $0 == "\t" }).joined(separator: " ")
            .trimmingCharacters(in: .whitespaces)
        return flat.isEmpty ? fallback : String(flat.prefix(200))
    }

    #if DEBUG
    /// Erfundener Bestand der Beispielzustände (`--assistant-demo-contexts`, `--assistant-demo-context-picker`).
    static var demoItems: [AssistantItem] {
        let now = Date()
        func day(_ days: Double) -> Date { now.addingTimeInterval(-days * 86_400) }
        return [
            AssistantItem(area: .books, id: "demo-buch", title: "Beispielbuch", detail: "Erika Muster · 42 % gelesen", updated: day(2)),
            AssistantItem(area: .books, id: "demo-reise", title: "Reisetagebuch Nordküste", detail: "Max Beispiel", updated: day(9)),
            AssistantItem(area: .transcripts, id: "demo-besprechung", title: "Besprechung Nordlicht", detail: "12.10.2026 · 48 min", updated: day(5)),
            AssistantItem(area: .transcripts, id: "demo-jourfixe", title: "Jour fixe Beispielteam", detail: "05.10.2026 · 25 min", updated: day(12)),
            AssistantItem(area: .notes, id: "demo-notiz", title: "Sprachnotiz Gartenplanung", detail: "11.11.2026", updated: day(1)),
            AssistantItem(area: .notes, id: "demo-einkauf", title: "Sprachnotiz Einkauf", detail: "08.11.2026", updated: day(4)),
            AssistantItem(area: .files, id: "demo-datei", title: "Angebot Beispielfirma", detail: "PDF", updated: nil),
            AssistantItem(area: .files, id: "demo-hausversammlung", title: "Protokoll Hausversammlung", detail: "DOCX", updated: nil),
        ]
    }

    static var demo: [AssistantContextCandidate] { demoItems.compactMap { AssistantContextCandidate(item: $0) } }
    #endif
}

/// Ergebnis des Anhänge-Blatts: die gewählten Inhalte, alle angebotenen (Schlüssel) und der Schalter „Nur aus diesen Inhalten antworten“.
struct AssistantContextSelection {
    var chosen: [AssistantContextCandidate]
    var offered: Set<String>
    var onlyFromThese: Bool
}

// MARK: - Chip

private struct AssistantContextHeightKey: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = max(value, nextValue()) }
}

/// Ein Chip: Symbol, Titel, Beschriftung („vom Assistenten“ bzw. „nicht mehr verfügbar“) und, bei Kontexten des Gesprächs, „Entfernen“.
/// Ein verwaister Chip ist grau; der Zustand steht auch als Text da, nicht nur in der Farbe.
struct AssistantContextChipView: View {
    let chip: AssistantContextChip
    let remove: () -> Void

    var body: some View {
        if chip.removable {
            content
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier("assistantContextChip")
        } else {
            content
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(chip.spokenLabel)
                .accessibilityIdentifier("assistantContextChip")
        }
    }

    private var content: some View {
        HStack(spacing: 6) {
            Image(systemName: AssistantContextText.symbol(chip.kind)).font(.footnote)
            VStack(alignment: .leading, spacing: 0) {
                Text(chip.displayTitle)
                    .font(.footnote)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
                if let caption = chip.caption {
                    Text(caption).font(.caption2).foregroundStyle(VoicePalette.secondaryText)
                }
            }
            if chip.removable {
                Button(action: remove) {
                    Image(systemName: "xmark.circle.fill").font(.body).frame(width: 44, height: 44).contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("\(chip.displayTitle) entfernen")
                .accessibilityIdentifier("assistantContextRemove")
            }
        }
        .foregroundStyle(chip.availability == .available ? VoicePalette.text : VoicePalette.secondaryText)
        .padding(.leading, 10).padding(.trailing, chip.removable ? 0 : 10)
        .frame(minHeight: 44)
        .overlay(RoundedRectangle(cornerRadius: 22).stroke(VoicePalette.border, lineWidth: 1))
    }
}

/// Die Chip-Zeile unter der Kopfzeile: ein Satz zur Wirkung, darunter die Chips. Wächst mit dem Inhalt, bis 132 pt; darüber scrollt sie.
struct AssistantContextChipsView: View {
    let chips: [AssistantContextChip]
    let modeLine: String?
    let remove: (AssistantContextChip) -> Void
    @State private var contentHeight: CGFloat = 44

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if let modeLine {
                Text(modeLine)
                    .font(.caption)
                    .foregroundStyle(VoicePalette.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("assistantContextMode")
            }
            ScrollView(.vertical, showsIndicators: false) {
                AssistantFlowLayout(spacing: 8) {
                    ForEach(chips) { chip in
                        AssistantContextChipView(chip: chip, remove: { remove(chip) })
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(GeometryReader { proxy in Color.clear.preference(key: AssistantContextHeightKey.self, value: proxy.size.height) })
            }
            .frame(height: min(max(contentHeight, 44), 132))
            .onPreferenceChange(AssistantContextHeightKey.self) { contentHeight = $0 }
        }
        .padding(.horizontal, 16).padding(.vertical, 8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("assistantContextChips")
    }
}
