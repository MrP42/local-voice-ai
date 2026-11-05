import Foundation

// Buch-Gespräch (Epic #307, Paket G6): Chat-Rahmen für llama.cpp-Modelle und Bereinigung des Stroms.
// Rein Foundation. Die Rahmen-Strings folgen der kanonischen Vorlage `chat_template.jinja` von
// google/gemma-4-E2B-it (Stand 2026-07-09: `<bos><|turn>system\n…<turn|>\n<|turn>user\n…<turn|>\n<|turn>model\n`,
// Denken aus). Ein Rahmen statt `llama_chat_apply_template`: die alte Vorlagenerkennung der C-API kennt neue
// Vorlagen nicht zuverlässig, und Zähler und Erzeugung müssen exakt denselben Aufbau benutzen.

/// Rahmen um System- und Nutzertext. Die Rahmenstücke enthalten Sondertoken (werden als solche
/// tokenisiert), die Nachrichtentexte nie (kein Einschleusen von Sondertoken über Buchtext).
public struct BookTalkChatFrame: Sendable, Equatable {
    /// Vor dem Systemtext (inklusive BOS).
    public let head: String
    /// Zwischen Systemtext und Nutzertext.
    public let middle: String
    /// Nach dem Nutzertext: öffnet die Antwort des Modells.
    public let tail: String

    public init(head: String, middle: String, tail: String) {
        self.head = head
        self.middle = middle
        self.tail = tail
    }

    public static let gemma4 = BookTalkChatFrame(
        head: "<bos><|turn>system\n", middle: "<turn|>\n<|turn>user\n", tail: "<turn|>\n<|turn>model\n")

    /// Nachrichtentext für den Prompt: `<` vor Buchstaben, `|`, `_` und `/` wird zu „‹“, damit kein Text
    /// wie `<end_of_turn>` oder `<|turn>` als Sondertoken wirkt. Zähler und Erzeugung rufen dieselbe Funktion.
    public static func sanitize(_ text: String) -> String {
        var out = String.UnicodeScalarView()
        var iterator = text.unicodeScalars.makeIterator()
        var pending = iterator.next()
        while let current = pending {
            let next = iterator.next()
            if current == "<", let n = next, n == "|" || n == "_" || n == "/" || (n.properties.isAlphabetic && n.isASCII) {
                out.append("\u{2039}")
            } else {
                out.append(current)
            }
            pending = next
        }
        return String(out)
    }

    /// Systemtext wie in der Vorlage (`| trim`), bereinigt.
    public func systemText(_ instructions: String) -> String {
        Self.sanitize(instructions).trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// Nutzertext, bereinigt und beschnitten.
    public func userText(_ prompt: String) -> String {
        Self.sanitize(prompt).trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

/// Entfernt aus dem Modellstrom, was keine Antwort ist: Denkkanal (`<|channel>…<channel|>`), Steuerzeichen
/// (`<|turn>`, `<turn|>`, `<bos>`, `<eos>`, `<end_of_turn>`). Nach `<turn|>`/`<eos>`/`<end_of_turn>` ist der
/// Strom zu Ende. Angefangene Marker am Chunk-Ende werden zurückgehalten.
public struct BookTalkStreamCleaner: Sendable {
    private static let channelOpen = "<|channel>"
    private static let channelClose = "<channel|>"
    private static let endMarkers = ["<turn|>", "<eos>", "<end_of_turn>"]
    private static let dropMarkers = ["<|turn>", "<bos>", "<start_of_turn>"]
    private static var all: [String] { [channelOpen, channelClose] + endMarkers + dropMarkers }

    private var buffer = ""
    private var inChannel = false
    public private(set) var finished = false

    public init() {}

    public mutating func feed(_ piece: String) -> String {
        guard !finished else { return "" }
        buffer += piece
        return drain(final: false)
    }

    public mutating func finish() -> String {
        guard !finished else { return "" }
        let out = drain(final: true)
        finished = true
        return out
    }

    private mutating func drain(final: Bool) -> String {
        var out = ""
        while !buffer.isEmpty {
            guard let lt = buffer.firstIndex(of: "<") else {
                if !inChannel { out += buffer }
                buffer = ""
                break
            }
            if !inChannel { out += buffer[..<lt] }
            buffer = String(buffer[lt...])
            if let marker = Self.all.first(where: { buffer.hasPrefix($0) }) {
                buffer.removeFirst(marker.count)
                if marker == Self.channelOpen { inChannel = true }
                else if marker == Self.channelClose { inChannel = false }
                else if Self.endMarkers.contains(marker) {
                    buffer = ""
                    finished = true
                    return out
                }
                continue
            }
            if !final, Self.all.contains(where: { $0.hasPrefix(buffer) }) { break }
            if !inChannel { out.append("<") }
            buffer.removeFirst()
        }
        return out
    }
}
