import Foundation

/// Eine von der Texterkennung gefundene Zeile. Koordinaten normiert (0…1) wie bei Apple Vision:
/// Ursprung unten links, `maxY` ist also die Oberkante.
public struct OcrLine: Equatable, Sendable {
    public var text: String
    public var minX: Double, minY: Double, maxX: Double, maxY: Double
    public init(text: String, minX: Double, minY: Double, maxX: Double, maxY: Double) {
        self.text = text; self.minX = minX; self.minY = minY; self.maxX = maxX; self.maxY = maxY
    }
    var height: Double { maxY - minY }
}

/// Eingefügter oder aus Bildern erkannter Text, bevor er als Eintrag in die Bibliothek geht (iOS).
/// Reine Textlogik ohne Apple-Frameworks, damit sie überall testbar ist.
public enum TextIntake {
    /// Höchstlänge eines Eintrags. Darüber hinaus wird nicht still gekürzt, sondern abgelehnt.
    public static let maxCharacters = 2_000_000
    public static let maxTitleLength = 60

    /// Unbekannte Größen ablehnen; Subtraktion verhindert einen Überlauf der Gesamtsumme.
    public static func imageSizeAllowed(bytes: Int?, total: Int) -> Bool {
        guard let bytes, bytes >= 0, bytes <= TextInbox.maxImageBytes,
              total >= 0, total <= TextInbox.maxTotalImageBytes else { return false }
        return bytes <= TextInbox.maxTotalImageBytes - total
    }

    /// Ohne gültige Maße kein Dekodieren in Originalgröße.
    public static func thumbnailDimension(width: Double, height: Double) -> Int? {
        guard width.isFinite, height.isFinite, width > 0, height > 0 else { return nil }
        let limit = 40_000_000.0
        let scale = min(1, (limit / width / height).squareRoot())
        return Int(max(1, min(limit, (max(width, height) * scale).rounded(.down))))
    }

    public static func imagePixelsAllowed(width: Int, height: Int) -> Bool {
        width > 0 && height > 0 && width <= 40_000_000 / height
    }

    // MARK: Bereinigung

    /// Zeilenenden vereinheitlichen, unsichtbare Zeichen entfernen, harte Umbrüche mitten im Satz
    /// zusammenführen (Kopien aus PDFs und E-Mails), Silbentrennung am Zeilenende auflösen.
    /// Absätze (Leerzeile) bleiben erhalten.
    public static func clean(_ raw: String) -> String {
        var s = raw.replacingOccurrences(of: "\r\n", with: "\n").replacingOccurrences(of: "\r", with: "\n")
        s = s.replacingOccurrences(of: "\u{00A0}", with: " ").replacingOccurrences(of: "\u{2028}", with: "\n")
            .replacingOccurrences(of: "\u{2029}", with: "\n\n").replacingOccurrences(of: "\t", with: " ")
        s.unicodeScalars.removeAll { [0x00AD, 0x200B, 0x200C, 0x200D, 0xFEFF].contains($0.value) }
        let paragraphs = s.components(separatedBy: "\n")
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .split(separator: "", omittingEmptySubsequences: true)
            .map { joinLines(Array($0)) }
        return paragraphs.joined(separator: "\n\n")
    }

    /// Zeilen eines Absatzes: Eine Zeile wird mit der nächsten verbunden, wenn sie nicht mit einem
    /// Satz- oder Doppelpunkt endet und die nächste klein beginnt (eindeutig fortgesetzter Satz),
    /// oder wenn sie mit einem Trennstrich nach einem Buchstaben endet und die nächste klein beginnt.
    /// Sonst bleibt der Umbruch (Listen, Adressen, Verse, Überschriften).
    static func joinLines(_ lines: [String]) -> String {
        var out = ""
        for line in lines {
            let collapsed = line.split(separator: " ", omittingEmptySubsequences: true).joined(separator: " ")
            guard !out.isEmpty else { out = collapsed; continue }
            let nextLower = collapsed.first.map { $0.isLowercase } ?? false
            if nextLower, hyphenated(out) {
                out.removeLast(); out += collapsed
            } else if nextLower, let last = out.last, !".!?:;".contains(last) {
                out += " " + collapsed
            } else {
                out += "\n" + collapsed
            }
        }
        return out
    }

    private static func hyphenated(_ s: String) -> Bool {
        guard s.hasSuffix("-"), s.count >= 2 else { return false }
        return s[s.index(s.endIndex, offsetBy: -2)].isLetter
    }

    // MARK: Titel

    /// Titel aus der ersten Zeile mit Inhalt; zu lange Zeilen enden an einer Wortgrenze mit „…“.
    public static func title(for text: String, fallback: String) -> String {
        let first = text.split(whereSeparator: \.isNewline).lazy
            .map { $0.trimmingCharacters(in: CharacterSet(charactersIn: "#*>-•· ").union(.whitespaces)) }
            .first { !$0.isEmpty } ?? ""
        let line = first.split(separator: " ", omittingEmptySubsequences: true).joined(separator: " ")
        guard line.count >= 2 else { return fallback }
        guard line.count > maxTitleLength else { return line }
        let cut = String(line.prefix(maxTitleLength))
        let atWord = cut.range(of: " ", options: .backwards).map { String(cut[..<$0.lowerBound]) } ?? cut
        return (atWord.count >= maxTitleLength / 2 ? atWord : cut).trimmingCharacters(in: CharacterSet(charactersIn: ",;:-–— ")) + "…"
    }

    // MARK: Texterkennung

    /// Zeilen der Texterkennung in Lesereihenfolge: zuerst Spalten (links, dann rechts), darin von
    /// oben nach unten, Zeilen auf gleicher Höhe von links nach rechts. Absätze entstehen an größeren
    /// Zeilenabständen und nach kurzen Zeilen, die einen Satz beenden.
    public static func assemble(_ raw: [OcrLine]) -> String {
        let lines = raw.map { var l = $0; l.text = l.text.trimmingCharacters(in: .whitespaces); return l }.filter { !$0.text.isEmpty }
        guard !lines.isEmpty else { return "" }
        let text = readingBlocks(lines).map(paragraphs).flatMap { $0 }.joined(separator: "\n\n")
        return clean(text)
    }

    /// Lesereihenfolge als Blöcke. Gibt es eine Spaltenlücke, teilen Zeilen, die sie überspannen
    /// (Überschrift, Bildunterschrift, Fußzeile), die Seite in Bänder von oben nach unten; in jedem Band
    /// erst die linke, dann die rechte Spalte. Ohne klare Lücke eine Spalte (häufigster Fall:
    /// Bildschirmfoto, Brief, Zettel).
    static func readingBlocks(_ lines: [OcrLine]) -> [[OcrLine]] {
        guard lines.count >= 6, let gutter = columnGutter(lines) else { return [lines] }
        let crosses = { (l: OcrLine) in l.minX < gutter && l.maxX > gutter }
        var blocks: [[OcrLine]] = []
        var band: [OcrLine] = []
        func flush() {
            let left = band.filter { $0.maxX <= gutter }, right = band.filter { $0.minX >= gutter }
            if !left.isEmpty { blocks.append(left) }
            if !right.isEmpty { blocks.append(right) }
            band = []
        }
        for line in lines.sorted(by: { $0.maxY > $1.maxY }) {
            if crosses(line) { flush(); blocks.append([line]) } else { band.append(line) }
        }
        flush()
        return blocks
    }

    /// Spaltenlücke zwischen 30 % und 70 % der Breite: höchstens ein Fünftel der Zeilen überspannt
    /// sie, und in mindestens einem Band dazwischen stehen links und rechts je mindestens drei Zeilen.
    static func columnGutter(_ lines: [OcrLine]) -> Double? {
        let sorted = lines.sorted { $0.maxY > $1.maxY }
        for step in 0...40 {
            let gutter = 0.30 + Double(step) * 0.01
            let crossing = sorted.filter { $0.minX < gutter && $0.maxX > gutter }.count
            guard crossing * 5 <= lines.count else { continue }
            var left = 0, right = 0
            for line in sorted {
                if line.minX < gutter && line.maxX > gutter { left = 0; right = 0; continue }
                if line.maxX <= gutter { left += 1 } else if line.minX >= gutter { right += 1 }
                if left >= 3 && right >= 3 { return gutter }
            }
        }
        return nil
    }

    /// Eine Spalte in Absätze. Rückgabe: Absätze, Zeilen darin mit `\n` (für `clean`).
    static func paragraphs(_ column: [OcrLine]) -> [String] {
        let rows = rowsTopDown(column)
        let heights = column.map(\.height).sorted()
        let lineHeight = max(heights[heights.count / 2], 0.001)
        let rightEdge = column.map(\.maxX).max() ?? 1, leftEdge = column.map(\.minX).min() ?? 0
        var result: [[String]] = []
        var previous: (top: Double, bottom: Double, maxX: Double, text: String)?
        for row in rows {
            let text = row.map(\.text).joined(separator: " ")
            let top = row.map(\.maxY).max()!, bottom = row.map(\.minY).min()!, maxX = row.map(\.maxX).max()!
            var newParagraph = previous == nil
            if let p = previous {
                let gap = p.bottom - top
                let shortEnded = p.maxX < rightEdge - 0.15 * (rightEdge - leftEdge) && p.text.last.map { ".!?:".contains($0) } == true
                newParagraph = gap > 0.75 * lineHeight || shortEnded
            }
            if newParagraph { result.append([text]) } else { result[result.count - 1].append(text) }
            previous = (top, bottom, maxX, text)
        }
        return result.map { $0.joined(separator: "\n") }
    }

    /// Zeilen, die sich vertikal mehr als zur Hälfte überdecken, sind eine Reihe (z. B. Tabellenzelle
    /// neben Beschriftung). Reihen von oben nach unten, innerhalb einer Reihe von links nach rechts.
    static func rowsTopDown(_ lines: [OcrLine]) -> [[OcrLine]] {
        let sorted = lines.sorted { $0.maxY != $1.maxY ? $0.maxY > $1.maxY : $0.minX < $1.minX }
        var rows: [[OcrLine]] = []
        for line in sorted {
            if let last = rows.last?.last {
                let overlap = min(last.maxY, line.maxY) - max(last.minY, line.minY)
                if overlap > 0.5 * min(last.height, line.height) { rows[rows.count - 1].append(line); continue }
            }
            rows.append([line])
        }
        return rows.map { $0.sorted { $0.minX < $1.minX } }
    }
}

// MARK: - Texteingang der App-Gruppe

/// Text und Bilder, die über das Teilen-Menü an Local Voice gingen. Die Share-Extension legt jeden
/// Eingang vollständig in einem Staging-Ordner an und benennt ihn erst danach um; die App sieht also
/// nur fertige Eingänge. Die App liest sie beim Aktivieren, zeigt sie zur Bestätigung und entfernt
/// sie erst, wenn der Text in der Bibliothek liegt oder ausdrücklich verworfen wurde.
public struct TextInbox: Sendable {
    public static let folder = "Texteingang"
    public static let maxImages = 20
    public static let maxImageBytes = 40 * 1024 * 1024
    /// Alle Bilder eines Eingangs zusammen; die App liest sie einzeln, nie alle auf einmal.
    public static let maxTotalImageBytes = 200 * 1024 * 1024
    public static let maxTextBytes = 8 * 1024 * 1024

    public struct Item: Equatable, Sendable {
        public let id: String
        public let created: Date
        public let text: String?
        public let images: [URL]
        public let source: String?
    }

    public enum Failure: Error, Equatable { case empty, tooManyImages, tooLarge }

    struct Manifest: Codable { var version = 1; var created: Double; var text: String?; var images: [String]; var source: String? }

    public let root: URL
    public init(root: URL) { self.root = root }

    public static func sharedRoot() -> URL? {
        #if canImport(Darwin)
        return FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: MeetingInbox.appGroup)?
            .appendingPathComponent(folder, isDirectory: true)
        #else
        return nil
        #endif
    }

    /// Legt einen Eingang an. Bilder als Dateien, die kopiert werden (die Share-Extension hat wenig
    /// Speicher; nichts wird ganz in den Arbeitsspeicher geladen). `source`: Anzeigename der Herkunft
    /// (z. B. „Teilen-Menü“), kein Inhalt.
    @discardableResult
    public func deposit(text: String?, imageFiles images: [URL], source: String?, now: Date = Date()) throws -> String {
        let text = text?.trimmingCharacters(in: .whitespacesAndNewlines)
        let hasText = !(text ?? "").isEmpty
        guard hasText || !images.isEmpty else { throw Failure.empty }
        guard images.count <= Self.maxImages else { throw Failure.tooManyImages }
        let sizes = images.map { Self.fileSize($0) }
        guard sizes.allSatisfy({ $0.map { $0 <= Self.maxImageBytes } ?? false }),
              sizes.reduce(0, { $0 + ($1 ?? 0) }) <= Self.maxTotalImageBytes,
              (text?.utf8.count ?? 0) <= Self.maxTextBytes else { throw Failure.tooLarge }
        let fm = FileManager.default
        try fm.createDirectory(at: root, withIntermediateDirectories: true)
        let id = "\(Int((now.timeIntervalSince1970 * 1000).rounded()))-\(UUID().uuidString.prefix(8).lowercased())"
        let staging = root.appendingPathComponent(".neu-\(id)", isDirectory: true)
        try fm.createDirectory(at: staging, withIntermediateDirectories: true)
        do {
            var names: [String] = []
            for (i, file) in images.enumerated() {
                let name = String(format: "bild-%03d", i + 1)
                try fm.copyItem(at: file, to: staging.appendingPathComponent(name))
                names.append(name)
            }
            if hasText, let text { try Data(text.utf8).write(to: staging.appendingPathComponent("text.txt"), options: .atomic) }
            let manifest = Manifest(created: now.timeIntervalSince1970, text: hasText ? "text.txt" : nil, images: names, source: source)
            try JSONEncoder().encode(manifest).write(to: staging.appendingPathComponent("eingang.json"), options: .atomic)
            try fm.moveItem(at: staging, to: root.appendingPathComponent(id, isDirectory: true))
        } catch {
            try? fm.removeItem(at: staging)
            throw error
        }
        return id
    }

    /// Größe einer regulären Datei; `nil` für Ordner, Verknüpfungen und Unlesbares.
    public static func fileSize(_ url: URL) -> Int? {
        guard let attrs = try? FileManager.default.attributesOfItem(atPath: url.path),
              attrs[.type] as? FileAttributeType == .typeRegular else { return nil }
        return (attrs[.size] as? NSNumber)?.intValue
    }

    /// Fertige Eingänge, älteste zuerst. Unlesbare Eingänge und Staging-Reste werden übergangen.
    public func pending() -> [Item] {
        let fm = FileManager.default
        let dirs = (try? fm.contentsOfDirectory(at: root, includingPropertiesForKeys: nil)) ?? []
        return dirs.filter { !$0.lastPathComponent.hasPrefix(".") }.compactMap(read).sorted { $0.created == $1.created ? $0.id < $1.id : $0.created < $1.created }
    }

    private func read(_ dir: URL) -> Item? {
        guard let data = try? Data(contentsOf: dir.appendingPathComponent("eingang.json")),
              let m = try? JSONDecoder().decode(Manifest.self, from: data) else { return nil }
        // Nur einfache Namen im eigenen Ordner, nie Pfade.
        let safe = { (name: String) in !name.isEmpty && !name.contains("/") && !name.hasPrefix(".") }
        let text = m.text.flatMap { safe($0) ? try? String(contentsOf: dir.appendingPathComponent($0), encoding: .utf8) : nil }
        let images = m.images.filter(safe).map { dir.appendingPathComponent($0) }.filter { FileManager.default.fileExists(atPath: $0.path) }
        guard !(text ?? "").isEmpty || !images.isEmpty else { return nil }
        return Item(id: dir.lastPathComponent, created: Date(timeIntervalSince1970: m.created), text: text, images: images, source: m.source)
    }

    public func remove(_ id: String) {
        guard !id.isEmpty, !id.contains("/"), !id.hasPrefix(".") else { return }
        try? FileManager.default.removeItem(at: root.appendingPathComponent(id, isDirectory: true))
    }

    /// Aufräumen, älter als eine Stunde: Staging-Reste abgebrochener Freigaben und Eingänge, die
    /// nicht lesbar sind (beschädigtes Manifest, Bilder fehlen). Fertige Eingänge entstehen nur durch
    /// Umbenennen, ein unlesbarer ist also beschädigt und würde sonst für immer liegen bleiben.
    /// Rückgabe: Zahl der entfernten unlesbaren Eingänge (für einen Hinweis).
    @discardableResult
    public func sweep(now: Date = Date()) -> Int {
        let fm = FileManager.default
        var removed = 0
        for url in (try? fm.contentsOfDirectory(at: root, includingPropertiesForKeys: [.contentModificationDateKey])) ?? [] {
            let date = (try? url.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? .distantPast
            guard now.timeIntervalSince(date) > 3600 else { continue }
            let name = url.lastPathComponent
            guard name.hasPrefix(".neu-") || !name.hasPrefix(".") else { continue }
            let readable = name.hasPrefix(".neu-") ? false : read(url) != nil
            guard Self.shouldRemove(name: name, modified: date, now: now,
                                    readable: readable) else { continue }
            do {
                try fm.removeItem(at: url)
                if !name.hasPrefix(".") { removed += 1 }
            } catch {}
        }
        return removed
    }

    /// Lesbare Eingänge und fremde versteckte Dateien bleiben unabhängig vom Alter erhalten.
    public static func shouldRemove(
        name: String, modified: Date, now: Date, readable: Bool
    ) -> Bool {
        guard now.timeIntervalSince(modified) > 3600 else { return false }
        return name.hasPrefix(".neu-") || (!name.hasPrefix(".") && !readable)
    }
}

// MARK: - Entwurfswächter

/// Verhindert, dass ein langsamer Vorgang (Fotos laden, Texterkennung, Einfügen) sein Ergebnis in einen
/// Entwurf schreibt, den es beim Start nicht gab: Vor dem ersten Warten wird ein Ticket gezogen; nach
/// dem Warten gilt es nur, wenn derselbe Entwurf noch offen ist (bzw. weiterhin keiner) und kein
/// Entwurf inzwischen verworfen, vorgelesen oder neu begonnen wurde.
public struct DraftGuard: Sendable {
    public struct Ticket: Equatable, Sendable {
        public let draft: UUID?
        public let generation: Int
    }
    public private(set) var draft: UUID?
    public private(set) var generation = 0
    public init() {}

    public func ticket() -> Ticket { Ticket(draft: draft, generation: generation) }
    public func isValid(_ ticket: Ticket) -> Bool { ticket.draft == draft && ticket.generation == generation }

    /// Neuer Entwurf geöffnet, oder offener Entwurf beendet (`nil`): Alle älteren Tickets verfallen.
    public mutating func switchTo(_ draft: UUID?) {
        self.draft = draft
        generation += 1
    }
}
