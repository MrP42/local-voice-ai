import Foundation

// Byte-geordnete Karte und Menge für die Ablage (D5a-2, lva#441).
//
// Rust hält Ordner, Dateien und Kennungsmengen in `BTreeMap<String, _>` und `BTreeSet<String>`: Schlüssel gleich
// heißt byte-gleich (UTF-8), die Reihenfolge ist die der Bytes. Swift-`Dictionary<String, _>` und `Set<String>`
// würden "é" und "e\u{301}" zu einem Schlüssel machen (kanonische Äquivalenz) und zwei verschiedene Kennungen
// verschmelzen. Beide Typen hier schlüsseln deshalb über die UTF-8-Bytes.

/// Karte `String → Wert` wie `BTreeMap<String, _>`: Schlüssel bytegenau, Durchlauf nach Schlüssel-Bytes.
public struct AblageKarte<Wert: Sendable>: Sendable, ExpressibleByDictionaryLiteral {
    private struct Eintrag: Sendable {
        var key: String
        var value: Wert
    }

    private var nachBytes: [[UInt8]: Eintrag] = [:]

    public init() {}

    /// Spätere Paare ersetzen frühere mit demselben Schlüssel (wie `collect::<BTreeMap>`).
    public init<S: Sequence>(_ paare: S) where S.Element == (String, Wert) {
        for (k, v) in paare { self[k] = v }
    }

    public init(dictionaryLiteral elements: (String, Wert)...) { self.init(elements) }

    public subscript(key: String) -> Wert? {
        get { nachBytes[Array(key.utf8)]?.value }
        set {
            let b = Array(key.utf8)
            if let n = newValue { nachBytes[b] = Eintrag(key: key, value: n) } else { nachBytes[b] = nil }
        }
    }

    public func enthaelt(_ key: String) -> Bool { nachBytes[Array(key.utf8)] != nil }

    public var count: Int { nachBytes.count }
    public var isEmpty: Bool { nachBytes.isEmpty }

    /// Nach Schlüssel-Bytes aufsteigend (Durchlauf einer `BTreeMap`).
    public var eintraege: [(key: String, value: Wert)] {
        nachBytes.sorted { $0.key.lexicographicallyPrecedes($1.key) }.map { ($0.value.key, $0.value.value) }
    }

    public var keys: [String] { eintraege.map(\.key) }
    public var values: [Wert] { eintraege.map(\.value) }
}

extension AblageKarte: Equatable where Wert: Equatable {
    public static func == (a: Self, b: Self) -> Bool {
        a.nachBytes.count == b.nachBytes.count
            && a.nachBytes.allSatisfy { b.nachBytes[$0.key].map { e in e.value == $0.value.value } ?? false }
    }
}

/// Menge von Zeichenketten wie `BTreeSet<String>` (bytegenau, sortiert nach Bytes).
struct AblageMenge: Sendable {
    private var bytes: Set<[UInt8]> = []

    init() {}
    init<S: Sequence>(_ xs: S) where S.Element == String { formUnion(xs) }

    mutating func insert(_ x: String) { bytes.insert(Array(x.utf8)) }
    mutating func formUnion<S: Sequence>(_ xs: S) where S.Element == String { for x in xs { insert(x) } }
    func contains(_ x: String) -> Bool { bytes.contains(Array(x.utf8)) }
    var count: Int { bytes.count }

    /// Aufsteigend nach Bytes, eindeutig.
    var sortiert: [String] {
        bytes.sorted { $0.lexicographicallyPrecedes($1) }.map { String(decoding: $0, as: UTF8.self) }
    }
}
