import Foundation

// Sprachwechsel mit Positionsübernahme (lva#344, Spezifikation §6). Reine Swift-Fassung von
// `apps/local-voice/src/lib/library/languageSwitch.ts` (Rechnung) und den Kapitelhilfen aus
// `editions.ts`/`switching.ts` (Gliederung, Plan). Gleiche Stufen, gleiche Ergebnisse; die Fälle der
// TypeScript-Prüfungen stehen in `EditionSwitchTests`.
//
// Eine Fassung wird als Gliederung beschrieben: Kapitelanfänge in ihrer Einheit (Zeichen beim Text,
// Millisekunden beim Hörbuch) und Gesamtlänge. Zwischen zwei Fassungen gilt:
// - kapitelgenau nur, wenn das Quellkapitel in einer bestätigten Gruppe der Zuordnung liegt;
// - sonst ungefähr: Anteil am ganzen Buch.
// „Genau“ (gleiche Fassung, Text ↔ Hörbuch) regelt die Zeitachse der Fassung und ist nicht Teil davon.

public struct ChapterOutline: Equatable, Sendable {
    /// Kapitelanfänge, aufsteigend, in der Einheit der Fassung.
    public var starts: [Double]
    /// Gesamtlänge in derselben Einheit.
    public var length: Double
    public init(starts: [Double], length: Double) { self.starts = starts; self.length = length }
}

public enum SwitchLevel: String, Equatable, Sendable { case chapter, approx }

public struct SwitchTarget: Equatable, Sendable {
    public var offset: Double
    public var level: SwitchLevel
    /// Zielkapitel (0-basiert) der Gruppe bzw. das Kapitel an der Zielstelle.
    public var chapters: [Int]
    /// Anteil am ganzen Buch (0…1), für „ungefähr bei 34 %“.
    public var fraction: Double
}

/// Kapitelliste einer Fassung für die Zuordnung (`ChapterList` in `editions.ts`).
public struct ChapterList: Equatable, Sendable {
    /// Kapitelüberschriften bzw. Teiltitel.
    public var titles: [String]
    /// Kapitelanfänge in der Einheit der Fassung; leer, wenn unbekannt.
    public var starts: [Double]
    public var length: Double
    public init(titles: [String], starts: [Double], length: Double) { self.titles = titles; self.starts = starts; self.length = length }
}

public enum SwitchMedium: String, Equatable, Sendable {
    case audio, text
    /// Einheit der Stelle in `reading.position` und im Fortschritt.
    public var unit: String { self == .audio ? "ms" : "chars" }
}

public struct SwitchPlan: Equatable, Sendable {
    public var from: String
    public var to: String
    public var fromMedium: SwitchMedium
    public var toMedium: SwitchMedium
    public var level: SwitchLevel
    /// Stelle in der Zielfassung, mit Schlüssel der Zielfassung.
    public var position: ReadingPositionRef
    /// Kapitel (0-basiert) der Gruppe in der Zielfassung.
    public var chapters: [Int]
    public var chapterTotal: Int
    /// Kapitel (0-basiert) der Quellstelle, sofern die Fassung Kapitel kennt.
    public var fromChapter: Int?
    /// Anteil am ganzen Buch (0…1).
    public var fraction: Double
    /// Ein Paar ist zugeordnet, aber diese Gruppe (oder das Paar) ist noch offen.
    public var hasAlignment: Bool

    /// Hinweis nach dem Wechsel („kapitelgenau“ nur bei bestätigter Zuordnung, sonst „ungefähr“).
    public func notice(languageName: String) -> String {
        let name = languageName.isEmpty ? "der anderen Sprache" : languageName
        switch level {
        case .chapter:
            guard let first = chapters.first, let last = chapters.last else { return "Weiter auf \(name), kapitelgenau." }
            let range = first == last ? "Kapitel \(first + 1), etwa gleiche Stelle im Kapitel" : "Kapitel \(first + 1)–\(last + 1), etwa gleiche Stelle"
            return "Weiter auf \(name), kapitelgenau: \(range)."
        case .approx:
            return "Weiter auf \(name), ungefähr bei \(Int((fraction * 100).rounded()))\u{00A0}%."
        }
    }
}

public enum EditionSwitch {
    private static func clamp(_ x: Double, _ lo: Double, _ hi: Double) -> Double { min(hi, max(lo, x)) }

    /// Kapitel (0-basiert), in dem `offset` liegt; vor dem ersten: 0.
    public static func chapterOf(_ outline: ChapterOutline, _ offset: Double) -> Int {
        var found = 0
        for (i, start) in outline.starts.enumerated() {
            if start <= offset { found = i } else { break }
        }
        return found
    }

    /// Bereich [von, bis) der Kapitel `first`…`last`.
    private static func span(_ outline: ChapterOutline, _ first: Int, _ last: Int) -> (Double, Double) {
        let from = outline.starts.indices.contains(first) ? outline.starts[first] : 0
        let to = outline.starts.indices.contains(last + 1) ? outline.starts[last + 1] : outline.length
        return (from, max(from, to))
    }

    /// Gruppen lückenlos, zusammenhängend, jedes Kapitel genau einmal (wie Rust).
    public static func validGroups(_ groups: [AlignGroup]) -> Bool {
        var na = 0, nb = 0
        for g in groups {
            if g.a.isEmpty || g.b.isEmpty { return false }
            if g.a.enumerated().contains(where: { $0.element != na + $0.offset }) { return false }
            if g.b.enumerated().contains(where: { $0.element != nb + $0.offset }) { return false }
            na += g.a.count
            nb += g.b.count
        }
        return true
    }

    /// „Alle bestätigen“ ist nur bei gleicher Kapitelzahl und reinen 1:1-Gruppen erlaubt.
    public static func canConfirmAll(_ groups: [AlignGroup], countA: Int, countB: Int) -> Bool {
        countA == countB && groups.count == countA && groups.allSatisfy { $0.a.count == 1 && $0.b.count == 1 }
    }

    /// Vorschlag: gleiche Kapitelzahl → 1:1, sonst eine Gruppe je Kapitel von `a` mit proportional
    /// verteilten Kapiteln von `b`. Nichts ist bestätigt.
    public static func proposeGroups(countA: Int, countB: Int) -> [AlignGroup] {
        if countA <= 0 || countB <= 0 { return [] }
        var groups: [AlignGroup] = []
        var nb = 0
        for i in 0..<countA {
            let end = i == countA - 1 ? countB : max(nb + 1, Int((Double((i + 1) * countB) / Double(countA)).rounded()))
            let room = countB - (countA - 1 - i)  // je folgendem a-Kapitel mindestens ein b
            let last = min(end, max(nb + 1, room))
            if nb >= countB {
                // Mehr a- als b-Kapitel: an die vorige Gruppe anhängen (n:1).
                groups[groups.count - 1].a.append(i)
                continue
            }
            groups.append(AlignGroup(a: [i], b: Array(nb..<last), confirmed: false))
            nb = last
        }
        return groups
    }

    public static func flipGroups(_ groups: [AlignGroup]) -> [AlignGroup] {
        groups.map { AlignGroup(a: $0.b, b: $0.a, confirmed: $0.confirmed) }
    }

    /// Gehört die Gruppenliste zu genau diesen Kapitelzahlen?
    public static func groupsFit(_ groups: [AlignGroup]?, countA: Int, countB: Int) -> Bool {
        guard let groups, !groups.isEmpty else { return false }
        return groups.reduce(0) { $0 + $1.a.count } == countA && groups.reduce(0) { $0 + $1.b.count } == countB
    }

    /// Zielstelle in Fassung `to` für Stelle `offset` in Fassung `from`. `groups` beschreibt die Zuordnung
    /// von `from` (a) nach `to` (b); ist sie andersherum gespeichert, vorher mit `flipGroups` drehen.
    public static func mapPosition(from: ChapterOutline, to: ChapterOutline, offset: Double, groups: [AlignGroup]?) -> SwitchTarget {
        let fraction = from.length > 0 ? clamp(offset / from.length, 0, 1) : 0
        func approx() -> SwitchTarget {
            let target = (fraction * to.length).rounded()
            return SwitchTarget(offset: target, level: .approx,
                                chapters: to.starts.isEmpty ? [] : [chapterOf(to, target)], fraction: fraction)
        }
        guard let groups, validGroups(groups), !from.starts.isEmpty, !to.starts.isEmpty else { return approx() }
        let ch = chapterOf(from, offset)
        guard let g = groups.first(where: { $0.a.contains(ch) }), g.confirmed, let lastB = g.b.last,
              lastB < to.starts.count, let firstA = g.a.first, let lastA = g.a.last, let firstB = g.b.first else { return approx() }
        let (fa, ta) = span(from, firstA, lastA)
        let (fb, tb) = span(to, firstB, lastB)
        let within = ta > fa ? clamp((offset - fa) / (ta - fa), 0, 1) : 0
        let target = (fb + within * (tb - fb)).rounded()
        return SwitchTarget(offset: target, level: .chapter, chapters: g.b,
                            fraction: to.length > 0 ? clamp(target / to.length, 0, 1) : fraction)
    }

    // MARK: Gliederung einer Fassung

    /// Kapitel aus den Überschriften (# und ##) des Seitentexts (`textChapters` in `editions.ts`).
    /// Stellen und Länge in UTF-16-Einheiten wie JavaScript.
    public static func textChapters(_ text: String) -> ChapterList {
        var titles: [String] = []
        var starts: [Double] = []
        var at = 0
        var first = true
        let heading = try? NSRegularExpression(pattern: #"^(#{1,2})[ \t]+(.+?)[ \t]*#*[ \t]*$"#)
        // NSString trennt nach UTF-16-Einheiten (auch „\r\n“), wie `split("\n")` in JavaScript.
        let lines = (text as NSString).components(separatedBy: "\n")
        for (i, line) in lines.enumerated() {
            let ns = line as NSString
            if let m = heading?.firstMatch(in: line, range: NSRange(location: 0, length: ns.length)),
               NSMaxRange(m.range) == ns.length {  // `$` ohne Mehrzeilenmodus: nur am ganzen Zeilenende (JavaScript)
                let marks = ns.substring(with: m.range(at: 1))
                let title = ns.substring(with: m.range(at: 2))
                let next = lines[(i + 1)...].first { !$0.trimmingCharacters(in: .whitespaces).isEmpty } ?? ""
                // Titelseite des EPUB-Imports: „# Titel“ und „von Autor“ ist kein Kapitel.
                let lowered = next.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
                let titlePage = first && marks == "#" && (lowered.hasPrefix("von ") || lowered.hasPrefix("von\t") || lowered.hasPrefix("by ") || lowered.hasPrefix("by\t"))
                first = false
                if !titlePage {
                    titles.append(title.replacingOccurrences(of: "*", with: "").replacingOccurrences(of: "_", with: "")
                        .trimmingCharacters(in: .whitespacesAndNewlines))
                    starts.append(Double(at + ns.range(of: title, options: .literal).location))
                }
            }
            at += ns.length + 1
        }
        return ChapterList(titles: titles, starts: starts, length: Double(text.utf16.count))
    }

    /// Kapitel aus den Teilen des Hörbuchs. Fehlt eine Dauer, bleiben `starts` leer; die Länge ist dann die
    /// Gesamtdauer der Datei, falls bekannt (der PC liest fehlende Dauern aus den Dateien, das iPhone rechnet
    /// bei unbekannter Gliederung ungefähr).
    public static func audioChapters(_ audio: LibraryAudioVersion?) -> ChapterList {
        let tracks = audio?.allTracks ?? []
        let titles = tracks.enumerated().map { i, t in
            let trimmed = t.title.trimmingCharacters(in: .whitespacesAndNewlines)
            return trimmed.isEmpty ? "Teil \(i + 1)" : trimmed
        }
        guard !tracks.isEmpty, tracks.allSatisfy({ ($0.durationMs ?? 0) > 0 }) else {
            return ChapterList(titles: titles, starts: [], length: max(0, audio?.durationMs ?? 0))
        }
        var starts: [Double] = []
        var sum = 0.0
        for t in tracks { starts.append(sum); sum += t.durationMs ?? 0 }
        return ChapterList(titles: titles, starts: starts, length: sum)
    }

    // MARK: Plan

    /// Zielstelle für den Wechsel von `from` nach `to` (`planSwitch` in `switching.ts`, ohne Ein-/Ausgabe).
    /// `offset` und `length` sind Stelle und Gesamtlänge der Quellfassung in der Einheit ihres Mediums
    /// (ms beim Hörbuch, Zeichen beim Text). Eine Zuordnung gilt nur im selben Medium und nur, wenn ihre
    /// Kapitelzahlen zu beiden Gliederungen passen; sonst bleibt es ehrlich bei „ungefähr“.
    public static func plan(editions: BookEditions, from: String, to: String, fromMedium: SwitchMedium, toMedium: SwitchMedium,
                            fromChapters: ChapterList?, toChapters: ChapterList, toKey: String,
                            offset: Double, length: Double) -> SwitchPlan {
        let source = ChapterOutline(starts: fromChapters?.starts ?? [], length: max(fromChapters?.length ?? 0, length))
        let target = ChapterOutline(starts: toChapters.starts, length: toChapters.length)
        let found = editions.alignment(from: from, to: to)
        var usable: [AlignGroup]?
        if fromMedium == toMedium, let fromChapters,
           groupsFit(found.groups, countA: fromChapters.titles.count, countB: toChapters.titles.count) {
            usable = found.groups
        }
        let mapped = mapPosition(from: source, to: target, offset: offset, groups: usable)
        return SwitchPlan(from: from, to: to, fromMedium: fromMedium, toMedium: toMedium, level: mapped.level,
                          position: ReadingPositionRef(key: toKey, unit: toMedium.unit, offset: mapped.offset),
                          chapters: mapped.chapters, chapterTotal: toChapters.titles.count,
                          fromChapter: source.starts.isEmpty ? nil : chapterOf(source, offset),
                          fraction: mapped.fraction, hasAlignment: found.alignment != nil)
    }
}
