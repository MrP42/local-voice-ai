import Foundation
#if canImport(CryptoKit)
import CryptoKit

// Textfassung im Markenformat v1 (Spec §4.2) und `textrev` (D5a-1, lva#441).
// Rust: `ablage/extrakt/mod.rs` (`Textfassung`, `Abschnitt`, `textrev`), Prüfung beim Einlegen
// `ablage/speicher_sync.rs::text_einlegen`. Das iPhone extrahiert nie selbst (Spec §4.1); es liest nur Texte,
// die der PC erstellt hat. Die Bytes bleiben unverändert (Blob-Hash, Hub-Kennung, Cache).

public struct AblageAbschnitt: Equatable, Sendable {
    /// z. B. `seite=7`, `absatz=2`, `folie=3`, `blatt=Tabelle1&zeilen=1-40`, `teil=text`, `zeile=12`.
    public var anker: String
    /// Überschriftenpfad; fehlt im JSON, wenn leer.
    public var pfad: [String]
    public var text: String
}

public struct AblageTextfassung: Equatable, Sendable {
    /// Lesegrenze für Textdateien (Desktop `index.rs`: 32 MiB).
    public static let maxBytes = 32 * 1024 * 1024

    /// Die Bytes der Datei, genau wie geladen.
    public let bytes: [UInt8]
    public let markenVersion: UInt32
    public let textrev: String
    public let originalSha256: String
    public let extraktor: String
    public let extraktorVersion: String
    public let abschnitte: [AblageAbschnitt]

    /// Liest `{"marken_version":1,"textrev":…,"original_sha256":…,"extraktor":…,"extraktor_version":…,"abschnitte":[…]}`.
    /// Unbekannte Felder werden überlesen (wie serde ohne `deny_unknown_fields`); fehlende Pflichtfelder sind Fehler.
    public init(bytes: [UInt8]) throws {
        let j = try AblageJSON.parse(bytes, maxBytes: Self.maxBytes)
        try j.istObjekt()
        self.bytes = bytes
        markenVersion = try j.pflicht("marken_version")
        textrev = try j.pflicht("textrev")
        originalSha256 = try j.pflicht("original_sha256")
        extraktor = try j.pflicht("extraktor")
        extraktorVersion = try j.pflicht("extraktor_version")
        guard case .array(let liste)? = j["abschnitte"] else { throw AblageError.feld("abschnitte") }
        abschnitte = try liste.map { a in
            try a.istObjekt()
            return AblageAbschnitt(anker: try a.pflicht("anker"), pfad: try a.vorgabe("pfad", [String]()), text: try a.pflicht("text"))
        }
    }

    /// SHA-256 (hex) der Bytes: der `blob` des Text-Eintrags (nicht der Hash des Originals).
    public var sha256Hex: String { AblageKrypto.sha256Hex(bytes) }

    /// Hub-Kennung dieser Datei.
    public func hubBlobId(key: [UInt8]) -> String? { AblageKrypto.hubBlobId(key: key, sha256Hex: sha256Hex) }

    /// Prüfung vor dem Ablegen wie `text_einlegen`: SHA-256 der Bytes = `Text.blob`, `textrev` im Kopf = Eintrag,
    /// `original_sha256` im Kopf = Fassung. Alles bytegenau; sonst verwerfen.
    public func passt(zu text: AblageText, fassung: AblageFassung) -> Bool {
        AblageBytes.equal(sha256Hex, text.blob)
            && AblageBytes.equal(textrev, text.textrev)
            && AblageBytes.equal(originalSha256, fassung.originalSha256)
    }

    /// Zusätzliche Probe (Empfehlung der Übergabe, der Desktop prüft nur Gleichheit): `textrev` lässt sich aus den
    /// Feldern des Eintrags und dem Original-Hash der Fassung neu berechnen.
    public static func textrevStimmt(text: AblageText, fassung: AblageFassung) -> Bool {
        AblageBytes.equal(
            textrev(originalSha256Hex: fassung.originalSha256, extraktor: text.extraktor,
                    extraktorVersion: text.extraktorVersion, markenVersion: text.markenVersion),
            text.textrev)
    }

    /// `hex(SHA-256(original_sha256_als_ASCII ‖ 0x00 ‖ extraktor ‖ 0x00 ‖ extraktor_version ‖ 0x00 ‖ dezimal(marken_version)))`
    /// wie `extrakt::textrev`. Eingabe ist der Hex-Text des Original-Hashes, nicht dessen Rohbytes. Extraktor und
    /// Version sind Eingaben; das iPhone schreibt die Tabelle des Desktops nie fest.
    public static func textrev(originalSha256Hex: String, extraktor: String, extraktorVersion: String, markenVersion: UInt32) -> String {
        var e = Array(originalSha256Hex.utf8)
        e.append(0)
        e += Array(extraktor.utf8)
        e.append(0)
        e += Array(extraktorVersion.utf8)
        e.append(0)
        e += Array(String(markenVersion).utf8)
        return AblageKrypto.sha256Hex(e)
    }
}
#endif
