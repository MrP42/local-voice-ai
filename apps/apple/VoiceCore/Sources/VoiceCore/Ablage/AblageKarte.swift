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
        guard a.nachBytes.count == b.nachBytes.count else { return false }
        for (k, links) in a.nachBytes {
            guard let rechts = b.nachBytes[k], rechts.value == links.value else { return false }
        }
        return true
    }
}

/// Menge von Zeichenketten wie `BTreeSet<String>` (bytegenau, sortiert nach Bytes).
public struct AblageMenge: Sendable, Equatable {
    private var bytes: Set<[UInt8]> = []

    public init() {}
    public init<S: Sequence>(_ xs: S) where S.Element == String { formUnion(xs) }

    public mutating func insert(_ x: String) { bytes.insert(Array(x.utf8)) }
    public mutating func remove(_ x: String) { bytes.remove(Array(x.utf8)) }
    public mutating func formUnion<S: Sequence>(_ xs: S) where S.Element == String { for x in xs { insert(x) } }
    public func contains(_ x: String) -> Bool { bytes.contains(Array(x.utf8)) }
    public var count: Int { bytes.count }
    public var isEmpty: Bool { bytes.isEmpty }
    public mutating func removeAll() { bytes.removeAll() }

    /// Aufsteigend nach Bytes, eindeutig.
    public var sortiert: [String] {
        bytes.sorted { $0.lexicographicallyPrecedes($1) }.map { String(decoding: $0, as: UTF8.self) }
    }
}
