import Foundation

// JSON-Wert der Dokumentenablage (D5a-1, lva#441).
//
// Warum ein eigener Wert statt `JSONSerialization`/`PassthroughJSON`:
// * Der Hash einer Nutzlast muss bitgleich zu `collect::canonical` am Desktop sein
//   (`apps/local-voice/src-tauri/src/sync/collect.rs`, serde_json). Ganzzahlen bis
//   `UInt64.max` müssen dafür exakt bleiben (`PassthroughJSON` rechnet über `Double`),
//   `null` darf nie gestrichen werden (`LibrarySyncFormat.canonicalHash` streicht es).
// * Schlüssel und Zeichenketten werden bytegenau (UTF-8) geordnet und verglichen. Swift-
//   `String` vergleicht nach kanonischer Äquivalenz ("é" == "e\u{301}"), Rust nach Bytes;
//   ein `Dictionary<String, _>` würde zwei verschiedene Schlüssel zu einem machen.
//
// Bewusst kein `Codable`: `JSONEncoder`/`JSONDecoder` schreiben 1.0 als `1`, -0.0 als `-0`, 1e20 als `1e+20` und
// lesen 1.0 als Ganzzahl. Ein Wert aus `extra` bekäme so beim Rundlauf einen anderen Hash als am Desktop
// (Ping-Pong). Einziger Weg ist `AblageJSON.parse` (Bytes → Wert) und `kanonisch()` (Wert → Bytes).
//
// Fehler tragen nie Inhalte, nur Codes und Feldnamen (Spec §10: kein Log mit Inhalten).

public enum AblageError: Error, Equatable, Sendable {
    /// Kein gültiges JSON (Syntax, ungültiges UTF-8, einzelne Surrogate, Zahl außerhalb des Bereichs).
    case ungueltigesJSON
    /// Schachtelungstiefe über der Grenze von serde_json.
    case zuTief
    /// Eingabe größer als die zugelassene Obergrenze.
    case zuGross
    /// Pflichtfeld fehlt oder hat den falschen Typ (nur der Feldname, nie der Wert).
    case feld(String)
    /// `v` > 1: neueres Schema, das Objekt wird nicht verstanden und darf nicht überschrieben werden.
    case neueresSchema
    /// Ein Schlüssel ist keine 32 Byte lang.
    case schluessel
    /// Versiegeln oder Öffnen scheiterte.
    case krypto
}

extension AblageError: LocalizedError {
    /// Nur feste Texte und Feldnamen, nie Inhalte (Spec §10).
    public var errorDescription: String? {
        switch self {
        case .ungueltigesJSON: return "Ablage-Objekt ist unlesbar."
        case .zuTief: return "Ablage-Objekt ist zu tief verschachtelt."
        case .zuGross: return "Ablage-Objekt ist zu groß."
        case .feld(let name): return "Ablage-Objekt: Feld „\(name)“ fehlt oder hat den falschen Typ."
        case .neueresSchema: return "Ablage-Objekt hat ein neueres Schema und wird von dieser App-Version nicht verstanden."
        case .schluessel: return "Ungültiger Schlüssel."
        case .krypto: return "Versiegeln oder Öffnen ist fehlgeschlagen."
        }
    }
}

/// Bytegenaue Ordnung und Gleichheit von Zeichenketten wie Rusts `String` (UTF-8, byteweise).
public enum AblageBytes {
    public static func less(_ a: String, _ b: String) -> Bool { a.utf8.lexicographicallyPrecedes(b.utf8) }
    public static func equal(_ a: String, _ b: String) -> Bool { a.utf8.elementsEqual(b.utf8) }
    /// -1, 0, 1 wie `Ord::cmp`.
    public static func compare(_ a: String, _ b: String) -> Int { equal(a, b) ? 0 : (less(a, b) ? -1 : 1) }

    /// Sortiert nach Bytes, stabil (gleiche Schlüssel behalten ihre Reihenfolge).
    public static func sorted<T>(_ xs: [T], by key: (T) -> String) -> [T] {
        xs.enumerated().sorted { l, r in
            let c = compare(key(l.element), key(r.element))
            return c != 0 ? c < 0 : l.offset < r.offset
        }.map(\.element)
    }

    /// Sortiert nach Bytes und entfernt Doppelte (wie `BTreeSet<String>`).
    public static func sortedUnique(_ xs: [String]) -> [String] {
        var out: [String] = []
        for s in sorted(xs, by: { $0 }) where out.last.map({ !equal($0, s) }) ?? true { out.append(s) }
        return out
    }
}

public indirect enum AblageJSON: Equatable, Sendable {
    case null
    case bool(Bool)
    /// Nichtnegative Ganzzahl (serde_json `PosInt`), exakt bis `UInt64.max`.
    case uint(UInt64)
    /// Negative Ganzzahl (serde_json `NegInt`); nichtnegative Werte sind immer `.uint`.
    case int(Int64)
    case double(Double)
    case string(String)
    case array([AblageJSON])
    /// Nach den UTF-8-Bytes der Schlüssel sortiert, Schlüssel eindeutig (siehe `objekt(_:)`).
    case object([Member])

    public struct Member: Equatable, Sendable {
        public var key: String
        public var value: AblageJSON
        public init(key: String, value: AblageJSON) { self.key = key; self.value = value }
        public static func == (a: Member, b: Member) -> Bool { AblageBytes.equal(a.key, b.key) && a.value == b.value }
    }

    // MARK: Gleichheit (Zeichenketten bytegenau)

    public static func == (a: AblageJSON, b: AblageJSON) -> Bool {
        switch (a, b) {
        case (.null, .null): return true
        case (.bool(let x), .bool(let y)): return x == y
        case (.uint(let x), .uint(let y)): return x == y
        case (.int(let x), .int(let y)): return x == y
        case (.double(let x), .double(let y)): return x == y
        case (.string(let x), .string(let y)): return AblageBytes.equal(x, y)
        case (.array(let x), .array(let y)): return x.count == y.count && zip(x, y).allSatisfy { $0 == $1 }
        case (.object(let x), .object(let y)): return x.count == y.count && zip(x, y).allSatisfy { $0 == $1 }
        default: return false
        }
    }

    // MARK: Aufbau

    /// Ganzzahl in der Form, die serde_json beim Lesen liefert (nichtnegativ = `.uint`).
    public static func integer(_ n: Int64) -> AblageJSON { n >= 0 ? .uint(UInt64(n)) : .int(n) }

    /// Objekt aus Paaren: nach Schlüssel-Bytes sortiert; bei gleichem Schlüssel gewinnt das letzte Paar
    /// (wie `serde_json::Map::insert`).
    public static func objekt(_ pairs: [(String, AblageJSON)]) -> AblageJSON {
        var letzte: [(key: String, value: AblageJSON, index: Int)] = []
        for (i, p) in pairs.enumerated() { letzte.append((p.0, p.1, i)) }
        let sortiert = letzte.sorted { l, r in
            let c = AblageBytes.compare(l.key, r.key)
            return c != 0 ? c < 0 : l.index < r.index
        }
        var out: [Member] = []
        for p in sortiert {
            if let l = out.last, AblageBytes.equal(l.key, p.key) { out[out.count - 1] = Member(key: p.key, value: p.value) }
            else { out.append(Member(key: p.key, value: p.value)) }
        }
        return .object(out)
    }

    /// Wert zum Schlüssel, wenn dies ein Objekt ist (Schlüssel bytegenau).
    public subscript(key: String) -> AblageJSON? {
        guard case .object(let m) = self else { return nil }
        return m.first { AblageBytes.equal($0.key, key) }?.value
    }
}

// MARK: - Lesen (wie serde_json::from_slice::<Value>)

extension AblageJSON {
    /// Größte Schachtelung, die serde_json noch liest (`recursion_limit` 128 ohne den obersten Wert).
    public static let maxDepth = 127
    /// Vorgabe-Obergrenze der Eingabe (Hub-Nutzlast höchstens ~4,3 MB; Textfassungen bis 32 MiB).
    public static let defaultMaxBytes = 64 * 1024 * 1024

    public static func parse(_ bytes: [UInt8], maxBytes: Int = defaultMaxBytes) throws -> AblageJSON {
        guard bytes.count <= maxBytes else { throw AblageError.zuGross }
        var p = Parser(bytes)
        p.skipSpace()
        let v = try p.value(depth: 0)
        p.skipSpace()
        guard p.i == bytes.count else { throw AblageError.ungueltigesJSON }
        return v
    }

    private struct Parser {
        let b: [UInt8]
        var i = 0
        init(_ b: [UInt8]) { self.b = b }

        mutating func skipSpace() { while i < b.count, b[i] == 0x20 || b[i] == 0x0A || b[i] == 0x0D || b[i] == 0x09 { i += 1 } }

        mutating func value(depth: Int) throws -> AblageJSON {
            guard i < b.count else { throw AblageError.ungueltigesJSON }
            switch b[i] {
            case UInt8(ascii: "n"): try literal("null"); return .null
            case UInt8(ascii: "t"): try literal("true"); return .bool(true)
            case UInt8(ascii: "f"): try literal("false"); return .bool(false)
            case UInt8(ascii: "\""): return .string(try string())
            case UInt8(ascii: "["): return try array(depth: depth + 1)
            case UInt8(ascii: "{"): return try object(depth: depth + 1)
            case UInt8(ascii: "-"), UInt8(ascii: "0")...UInt8(ascii: "9"): return try number()
            default: throw AblageError.ungueltigesJSON
            }
        }

        mutating func literal(_ s: String) throws {
            let u = Array(s.utf8)
            guard i + u.count <= b.count, Array(b[i..<i + u.count]) == u else { throw AblageError.ungueltigesJSON }
            i += u.count
        }

        mutating func array(depth: Int) throws -> AblageJSON {
            guard depth <= AblageJSON.maxDepth else { throw AblageError.zuTief }
            i += 1
            skipSpace()
            var out: [AblageJSON] = []
            if i < b.count, b[i] == UInt8(ascii: "]") { i += 1; return .array(out) }
            while true {
                skipSpace()
                out.append(try value(depth: depth))
                skipSpace()
                guard i < b.count else { throw AblageError.ungueltigesJSON }
                if b[i] == UInt8(ascii: ",") { i += 1; continue }
                if b[i] == UInt8(ascii: "]") { i += 1; return .array(out) }
                throw AblageError.ungueltigesJSON
            }
        }

        mutating func object(depth: Int) throws -> AblageJSON {
            guard depth <= AblageJSON.maxDepth else { throw AblageError.zuTief }
            i += 1
            skipSpace()
            var pairs: [(String, AblageJSON)] = []
            if i < b.count, b[i] == UInt8(ascii: "}") { i += 1; return .object([]) }
            while true {
                skipSpace()
                guard i < b.count, b[i] == UInt8(ascii: "\"") else { throw AblageError.ungueltigesJSON }
                let k = try string()
                skipSpace()
                guard i < b.count, b[i] == UInt8(ascii: ":") else { throw AblageError.ungueltigesJSON }
                i += 1
                skipSpace()
                pairs.append((k, try value(depth: depth)))
                skipSpace()
                guard i < b.count else { throw AblageError.ungueltigesJSON }
                if b[i] == UInt8(ascii: ",") { i += 1; continue }
                if b[i] == UInt8(ascii: "}") { i += 1; return objekt(pairs) }
                throw AblageError.ungueltigesJSON
            }
        }

        func isDigit(_ c: UInt8) -> Bool { c >= UInt8(ascii: "0") && c <= UInt8(ascii: "9") }

        mutating func number() throws -> AblageJSON {
            let start = i
            var positive = true
            if b[i] == UInt8(ascii: "-") { positive = false; i += 1 }
            guard i < b.count else { throw AblageError.ungueltigesJSON }
            if b[i] == UInt8(ascii: "0") {
                i += 1
                if i < b.count, isDigit(b[i]) { throw AblageError.ungueltigesJSON }
            } else if b[i] >= UInt8(ascii: "1"), b[i] <= UInt8(ascii: "9") {
                while i < b.count, isDigit(b[i]) { i += 1 }
            } else { throw AblageError.ungueltigesJSON }
            var istFloat = false
            if i < b.count, b[i] == UInt8(ascii: ".") {
                i += 1
                guard i < b.count, isDigit(b[i]) else { throw AblageError.ungueltigesJSON }
                while i < b.count, isDigit(b[i]) { i += 1 }
                istFloat = true
            }
            if i < b.count, b[i] == UInt8(ascii: "e") || b[i] == UInt8(ascii: "E") {
                i += 1
                if i < b.count, b[i] == UInt8(ascii: "+") || b[i] == UInt8(ascii: "-") { i += 1 }
                guard i < b.count, isDigit(b[i]) else { throw AblageError.ungueltigesJSON }
                while i < b.count, isDigit(b[i]) { i += 1 }
                istFloat = true
            }
            let text = String(decoding: b[start..<i], as: UTF8.self)
            if !istFloat {
                // serde_json: PosInt bis u64, NegInt bis i64; "-0" und alles Größere wird zum Float.
                if positive, let u = UInt64(text) { return .uint(u) }
                if !positive, let n = Int64(text), n < 0 { return .int(n) }
            }
            guard let d = Double(text), d.isFinite else { throw AblageError.ungueltigesJSON }
            return .double(d)
        }

        func hex4(_ at: Int) throws -> UInt32 {
            guard at + 4 <= b.count else { throw AblageError.ungueltigesJSON }
            var v: UInt32 = 0
            for k in 0..<4 {
                let c = b[at + k]
                let d: UInt32
                switch c {
                case UInt8(ascii: "0")...UInt8(ascii: "9"): d = UInt32(c - UInt8(ascii: "0"))
                case UInt8(ascii: "a")...UInt8(ascii: "f"): d = UInt32(c - UInt8(ascii: "a")) + 10
                case UInt8(ascii: "A")...UInt8(ascii: "F"): d = UInt32(c - UInt8(ascii: "A")) + 10
                default: throw AblageError.ungueltigesJSON
                }
                v = v << 4 | d
            }
            return v
        }

        mutating func string() throws -> String {
            i += 1 // öffnendes "
            var out: [UInt8] = []
            while true {
                guard i < b.count else { throw AblageError.ungueltigesJSON }
                let c = b[i]
                if c == UInt8(ascii: "\"") { i += 1; break }
                if c < 0x20 { throw AblageError.ungueltigesJSON }
                if c != UInt8(ascii: "\\") { out.append(c); i += 1; continue }
                i += 1
                guard i < b.count else { throw AblageError.ungueltigesJSON }
                let e = b[i]
                i += 1
                switch e {
                case UInt8(ascii: "\""): out.append(0x22)
                case UInt8(ascii: "\\"): out.append(0x5C)
                case UInt8(ascii: "/"): out.append(0x2F)
                case UInt8(ascii: "b"): out.append(0x08)
                case UInt8(ascii: "f"): out.append(0x0C)
                case UInt8(ascii: "n"): out.append(0x0A)
                case UInt8(ascii: "r"): out.append(0x0D)
                case UInt8(ascii: "t"): out.append(0x09)
                case UInt8(ascii: "u"):
                    var n = try hex4(i)
                    i += 4
                    if n >= 0xD800, n <= 0xDBFF {
                        // Führendes Surrogat braucht ein folgendes \uDC00-\uDFFF.
                        guard i + 1 < b.count, b[i] == UInt8(ascii: "\\"), b[i + 1] == UInt8(ascii: "u") else { throw AblageError.ungueltigesJSON }
                        let lo = try hex4(i + 2)
                        guard lo >= 0xDC00, lo <= 0xDFFF else { throw AblageError.ungueltigesJSON }
                        i += 6
                        n = 0x10000 + ((n - 0xD800) << 10) + (lo - 0xDC00)
                    } else if n >= 0xDC00, n <= 0xDFFF {
                        throw AblageError.ungueltigesJSON
                    }
                    guard let sc = Unicode.Scalar(n) else { throw AblageError.ungueltigesJSON }
                    out.append(contentsOf: Array(String(Character(sc)).utf8))
                default: throw AblageError.ungueltigesJSON
                }
            }
            // Ungültiges UTF-8 ergäbe beim Dekodieren U+FFFD: dann stimmen die Bytes nicht mehr überein.
            let s = String(decoding: out, as: UTF8.self)
            guard s.utf8.elementsEqual(out) else { throw AblageError.ungueltigesJSON }
            return s
        }
    }
}

// MARK: - Schreiben (wie collect::canonical)

extension AblageJSON {
    /// Kanonisches JSON: Schlüssel rekursiv nach UTF-8-Bytes sortiert (durch `objekt`), kompakt, `null` bleibt,
    /// Zahlen wie serde_json, Zeichenketten wie serde_json (nur `"`, `\` und Steuerzeichen < 0x20 werden
    /// escaped; Nicht-ASCII und `/` roh).
    public func kanonisch() -> [UInt8] {
        var out: [UInt8] = []
        schreibe(&out)
        return out
    }

    public var kanonischerText: String { String(decoding: kanonisch(), as: UTF8.self) }

    func schreibe(_ out: inout [UInt8]) {
        switch self {
        case .null: out.append(contentsOf: Array("null".utf8))
        case .bool(let b): out.append(contentsOf: Array((b ? "true" : "false").utf8))
        case .uint(let u): out.append(contentsOf: Array(String(u).utf8))
        case .int(let n): out.append(contentsOf: Array(String(n).utf8))
        case .double(let d):
            // serde_json schreibt NaN und ±Unendlich als `null` (JSON kennt sie nicht).
            out.append(contentsOf: Array((d.isFinite ? Self.gleitkomma(d) : "null").utf8))
        case .string(let s): Self.schreibeString(s, &out)
        case .array(let a):
            out.append(UInt8(ascii: "["))
            for (k, v) in a.enumerated() { if k > 0 { out.append(UInt8(ascii: ",")) }; v.schreibe(&out) }
            out.append(UInt8(ascii: "]"))
        case .object(let m):
            // Auch ein von Hand gebauter Wert wird hier nach Bytes geordnet und bereinigt.
            let pairs = AblageJSON.objekt(m.map { ($0.key, $0.value) })
            guard case .object(let sorted) = pairs else { return }
            out.append(UInt8(ascii: "{"))
            for (k, mem) in sorted.enumerated() {
                if k > 0 { out.append(UInt8(ascii: ",")) }
                Self.schreibeString(mem.key, &out)
                out.append(UInt8(ascii: ":"))
                mem.value.schreibe(&out)
            }
            out.append(UInt8(ascii: "}"))
        }
    }

    private static let hexKlein = Array("0123456789abcdef".utf8)

    static func schreibeString(_ s: String, _ out: inout [UInt8]) {
        out.append(0x22)
        for c in s.utf8 {
            switch c {
            case 0x22: out.append(0x5C); out.append(0x22)
            case 0x5C: out.append(0x5C); out.append(0x5C)
            case 0x08: out.append(0x5C); out.append(UInt8(ascii: "b"))
            case 0x09: out.append(0x5C); out.append(UInt8(ascii: "t"))
            case 0x0A: out.append(0x5C); out.append(UInt8(ascii: "n"))
            case 0x0C: out.append(0x5C); out.append(UInt8(ascii: "f"))
            case 0x0D: out.append(0x5C); out.append(UInt8(ascii: "r"))
            case 0x00..<0x20:
                out.append(contentsOf: [0x5C, UInt8(ascii: "u"), UInt8(ascii: "0"), UInt8(ascii: "0"), hexKlein[Int(c >> 4)], hexKlein[Int(c & 0xF)]])
            default: out.append(c)
            }
        }
        out.append(0x22)
    }

    /// Gleitkommazahl wie serde_json 1.0.149 (Formatierer `zmij`, Stand des Desktop-Cargo.lock): kürzeste Ziffern,
    /// dezimal für Beträge ab 1e-5 bis unter 1e16 (ganze Werte mit `.0`), sonst `1e+21`, `1.5e-7`, `1.5e+300`
    /// (Vorzeichen immer, Exponent ohne Auffüllung). Zusatzfelder künftiger Schemas können Gleitkommazahlen tragen;
    /// der Desktop schreibt selbst nur Ganzzahlen. Das Format ändert sich mit der serde_json-Version (ältere
    /// Versionen schrieben `1e21`); die Vektoren in AblageVectorTests schlagen dann an.
    static func gleitkomma(_ d: Double) -> String {
        if d == 0 { return d.sign == .minus ? "-0.0" : "0.0" }
        var text = "\(abs(d))"                     // kürzeste Ziffern, z. B. "1.5", "1e-07", "1.7976931348623157e+308"
        var exp10 = 0
        if let e = text.firstIndex(where: { $0 == "e" || $0 == "E" }) {
            exp10 = Int(text[text.index(after: e)...]) ?? 0
            text = String(text[..<e])
        }
        let teile = text.split(separator: ".", omittingEmptySubsequences: false)
        var ziffern = Array(String(teile[0]))
        var punkt = ziffern.count
        if teile.count > 1 { ziffern += Array(String(teile[1])) }
        while let f = ziffern.first, f == "0" { ziffern.removeFirst(); punkt -= 1 }
        while let l = ziffern.last, l == "0" { ziffern.removeLast() }
        let n = ziffern.count
        let kk = punkt + exp10          // Wert = 0.ziffern × 10^kk
        let k = kk - n
        let vorzeichen = d < 0 ? "-" : ""
        let z = String(ziffern)
        if k >= 0 && kk <= 16 { return vorzeichen + z + String(repeating: "0", count: k) + ".0" }
        if kk > 0 && kk <= 16 { return vorzeichen + String(z.prefix(kk)) + "." + String(z.dropFirst(kk)) }
        if kk > -5 && kk <= 0 { return vorzeichen + "0." + String(repeating: "0", count: -kk) + z }
        let e = (kk - 1 >= 0 ? "e+" : "e") + String(kk - 1)
        if n == 1 { return vorzeichen + z + e }
        return vorzeichen + String(z.prefix(1)) + "." + String(z.dropFirst(1)) + e
    }
}
