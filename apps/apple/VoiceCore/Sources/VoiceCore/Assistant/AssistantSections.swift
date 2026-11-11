import Foundation

// Gesprächsmodus „Sprechen“ (Paket A, E4.2b): Quellblöcke für den Index. Die Einheit des Zitierens ist der gespeicherte Block
// zwischen STRUKTURELLEN Grenzen der Quelle, nie ein Satz und nie ein Teil eines Blocks. Die App erzeugt keine Grenze aus
// Satzzeichen, Abkürzungslisten oder Großbuchstaben, sie trennt und kürzt innerhalb eines Blocks nichts und zieht keine zwei
// Blöcke zusammen. Was ein Block ist, bestimmt je Quellart die Struktur der Quelle:
//
// - Buch (EPUB-Text): ein Absatz (Zeilen bis zur Leerzeile, Kapitelüberschrift oder Seitenmarke `{{S. n}}`), so wie ihn
//   `BookTalkIndex` aus dem gespeicherten Buchtext liest (ohne Sprecher- und Stilmarken, Leerraum gefaltet).
// - Notiz (Sprachnotiz): ein Textblock zwischen Leerzeilen bzw. dem Rand der Notiz; Zeilenumbrüche darin bleiben Teil des Blocks.
// - Besprechung (Transkript): ein Segment der Aufnahme (Zeile „Sprecher: Text“), wie der Rohling es führt.
// - Protokoll einer Besprechung (erzeugt): ein Teil des Protokolls (Zusammenfassung, Beschlüsse, Aufgaben, …).
// - Ablage: heute NICHT zitierbar (die Suche liefert Fenster von 220 Zeichen aus einem Abschnitt, keinen ganzen Block).
//
// Restgrenze (bewusst, durch die Kennzeichnung „Wörtlicher Auszug“ gedeckt): Ein Block aus einem PDF-Import oder einem
// Transkript kann mitten im Satz beginnen oder enden. Strukturelle Blockgrenzen garantieren keine Satzgrenzen.
// Reine Funktionen ohne Darwin-Abhängigkeit (Linux-Tests); die iPhone-Schicht liefert nur die Eingaben.

/// Ein Segment eines Transkripts in Zeitfolge. `usable == false`: nicht für die KI freigegeben oder leer; es wird kein Block.
struct AssistantSegment: Equatable {
    let start: Double
    /// Zeile für den Index (mit Sprecher).
    let line: String
    let usable: Bool
}

enum AssistantSections {
    /// Blöcke eines Textes (Notiz): Absätze zwischen Leerzeilen. Der Block bleibt unverändert, auch mit Zeilenumbrüchen darin; nur
    /// Leerraum an seinen Rändern fällt weg. Eine Zeile mit Abkürzung am Ende („Dr.“) trennt nichts.
    static func blocks(of text: String, locator: String?) -> [AssistantDocument.Section] {
        var out: [AssistantDocument.Section] = []
        var lines: [Substring] = []
        func flush() {
            defer { lines.removeAll() }
            let block = lines.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
            if !block.isEmpty { out.append(.init(locator: locator, text: block)) }
        }
        for line in text.split(omittingEmptySubsequences: false, whereSeparator: \.isNewline) {
            if line.trimmingCharacters(in: .whitespaces).isEmpty { flush() } else { lines.append(line) }
        }
        flush()
        return out
    }

    /// Erzeugter Text (Protokoll): jeder Teil ein Block.
    static func generated(_ parts: [String], locator: String?) -> [AssistantDocument.Section] {
        parts.compactMap { part in
            let text = part.trimmingCharacters(in: .whitespacesAndNewlines)
            return text.isEmpty ? nil : AssistantDocument.Section(locator: locator, text: text)
        }
    }

    /// Transkript: ein Block je benutzbarem Segment, in Zeitfolge. Nicht freigegebene oder leere Segmente werden kein Block.
    static func meeting(_ segments: [AssistantSegment], locator: (Double) -> String) -> [AssistantDocument.Section] {
        segments.compactMap { segment in
            guard segment.usable else { return nil }
            let text = segment.line.trimmingCharacters(in: .whitespacesAndNewlines)
            return text.isEmpty ? nil : AssistantDocument.Section(locator: locator(segment.start), text: text)
        }
    }

    /// Buch: ein Block je Absatz. `frontier` (UTF-16-Offset im Original): Lesegrenze; ein Absatz, der sie überragt, fällt GANZ
    /// weg (er wird nicht an der Grenze gekürzt: ein gekürzter Absatz wäre kein gespeicherter Block).
    static func book(_ paragraphs: [BookTalkBlock], frontier: Int?) -> [AssistantDocument.Section] {
        var out: [AssistantDocument.Section] = []
        for p in paragraphs {
            if let frontier, p.end > frontier { continue }
            guard !p.text.isEmpty else { continue }
            let title = p.chapterTitle.trimmingCharacters(in: .whitespacesAndNewlines)
            let locator = p.page.map { "S. \($0)" } ?? (title.isEmpty ? nil : title)
            out.append(.init(locator: locator, text: p.text))
        }
        return out
    }
}
