import Foundation

public enum ReadingPageError: Error, LocalizedError {
    case invalid, tooLarge, persistence, unavailable
    public var errorDescription: String? {
        switch self {
        case .invalid: return "Vorlese-Seite ist unlesbar."
        case .tooLarge: return "Vorlese-Seite ist zu groß für den Abgleich."
        case .persistence: return "Vorlese-Seite konnte nicht gespeichert werden."
        case .unavailable: return "Der gemeinsame Seitenspeicher ist nicht verfügbar."
        }
    }
}

/// `page` am Desktop: `{title, state}`. Auch unbekannte Felder, null und Fassungen
/// bleiben erhalten. AblageJSON erhält Ganzzahlen und Schlüssel bytegenau;
/// PassthroughJSON/LibrarySyncFormat.canonicalHash wären hier verlustbehaftet.
public struct ReadingPage: Equatable, Sendable, Identifiable {
    public static let collection = "page"
    public static let maxPlaintext = 3 * 1024 * 1024
    public static let hubMaxPayload = 4_300_000
    public let id: String
    private var value: AblageJSON

    public init(id: String = "page_" + UUID().uuidString.replacingOccurrences(of: "-", with: ""), title: String) throws {
        guard Self.validID(id) else { throw ReadingPageError.invalid }
        self.id = id
        value = .objekt([("title", .string(title)), ("state", .objekt([("text", .string(""))]))])
    }

    public init(id: String, json: Data, maxBytes: Int = ReadingPage.maxPlaintext) throws {
        guard Self.validID(id) else { throw ReadingPageError.invalid }
        guard json.count <= maxBytes else { throw ReadingPageError.tooLarge }
        let parsed = try AblageJSON.parse([UInt8](json), maxBytes: maxBytes)
        guard case .object = parsed else { throw ReadingPageError.invalid }
        self.id = id; value = parsed
    }

    /// Schnittmenge aus Desktop `library_layout::gueltige_seite` und Hub (64 Bytes).
    public static func validID(_ id: String) -> Bool {
        id.hasPrefix("page_") && id.utf8.count > 5 && id.utf8.count <= 64
            && id.utf8.dropFirst(5).allSatisfy { (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0) }
    }

    public var title: String {
        get { if case .string(let s) = value["title"] { return s }; return "Seite" }
        set { value = Self.setting(value, "title", .string(newValue)) }
    }
    public var state: AblageJSON { value["state"] ?? .null }
    public var text: String { get { string("text") } set { setStateField("text", .string(newValue)) } }
    public var summary: String { get { string("summary") } set { setStateField("summary", .string(newValue)) } }
    public var source: String { get { string("source") } set { setStateField("source", .string(newValue)) } }
    public var translation: String { get { string("translation") } set { setStateField("translation", .string(newValue)) } }

    /// Fassungen (`einfuegungen`, `kiDoc` usw.) und künftige Felder haben absichtlich
    /// kein engeres Swift-Schema. Änderungen ersetzen genau ein Feld.
    public mutating func setStateField(_ key: String, _ field: AblageJSON) {
        value = Self.setting(value, "state", Self.setting(state, key, field))
    }
    public func json() -> Data { Data(value.kanonisch()) }

    /// Wie collect::pages_differ: fehlend und null sind gleich; URL/Reiter zählen nicht.
    public func differs(from other: ReadingPage) -> Bool {
        (value["title"] ?? .null) != (other.value["title"] ?? .null)
            || ["text", "summary", "translation"].contains { (state[$0] ?? .null) != (other.state[$0] ?? .null) }
    }

    private func string(_ key: String) -> String {
        if case .string(let s) = state[key] { return s }; return ""
    }
    private static func setting(_ object: AblageJSON, _ key: String, _ value: AblageJSON) -> AblageJSON {
        var pairs: [(String, AblageJSON)] = []
        if case .object(let members) = object { pairs = members.map { ($0.key, $0.value) } }
        pairs.append((key, value))
        return .objekt(pairs)
    }
}

/// Gerätelokal: Weder Bildverweise noch Aufnahme-Metadaten gehören in den Sync.
public struct ReadingCapture: Codable, Equatable, Sendable, Identifiable {
    public var id: String
    /// Relativer Bildname im lokalen Bilderordner; nie eine URL aus dem Konto.
    public var image: String
    public var recognizedText: String
    public var source: String
    public var createdMs: Double
    public var textEdited: Bool
    /// S2 kann ein neues OCR-Ergebnis vormerken, bis Ersetzen/Anhängen gewählt wurde.
    public var pendingRecognition: String?

    public init(id: String = UUID().uuidString, image: String, recognizedText: String, source: String,
                createdMs: Double, textEdited: Bool = false, pendingRecognition: String? = nil) {
        self.id = id; self.image = image; self.recognizedText = recognizedText; self.source = source
        self.createdMs = createdMs; self.textEdited = textEdited; self.pendingRecognition = pendingRecognition
    }

    public static func compose(_ captures: [ReadingCapture]) -> String {
        captures.map(\.recognizedText).filter { !$0.isEmpty }.joined(separator: "\n\n")
    }
}
