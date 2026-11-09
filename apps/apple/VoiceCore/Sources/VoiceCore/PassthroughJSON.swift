import Foundation

/// Beliebiger JSON-Wert, den das iPhone nicht auswertet, aber unverändert
/// weiterreicht (Sprachfassungen lva#344: `editions`, `primary`, `reading` …).
/// So verliert ein Abgleich vom iPhone keine Felder, die nur der Desktop kennt.
public indirect enum PassthroughJSON: Codable, Equatable, Sendable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([PassthroughJSON])
    case object([String: PassthroughJSON])

    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() { self = .null }
        else if let b = try? c.decode(Bool.self) { self = .bool(b) }
        else if let n = try? c.decode(Double.self) { self = .number(n) }
        else if let s = try? c.decode(String.self) { self = .string(s) }
        else if let a = try? c.decode([PassthroughJSON].self) { self = .array(a) }
        else { self = .object(try c.decode([String: PassthroughJSON].self)) }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .null: try c.encodeNil()
        case .bool(let b): try c.encode(b)
        case .number(let n):
            // Ganze Zahlen (Zeitstempel, Kapitelnummern) ohne „.0“ schreiben.
            if n.rounded() == n, abs(n) < 9.0e15 { try c.encode(Int64(n)) } else { try c.encode(n) }
        case .string(let s): try c.encode(s)
        case .array(let a): try c.encode(a)
        case .object(let o): try c.encode(o)
        }
    }
}
