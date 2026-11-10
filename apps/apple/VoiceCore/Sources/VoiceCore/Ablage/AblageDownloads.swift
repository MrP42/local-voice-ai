import Foundation
#if canImport(CryptoKit)

// Download-Plan, Budget und Textformate der Dokumentenablage (D5a-3, lva#441, Epic #395).
//
// Port von `apps/local-voice/src-tauri/src/sync/ablage.rs` (`herunterladen`: Auswahl, Reihenfolge, Budget) und
// `ablage/extrakt/mod.rs` (`format_aus_typ`). Abweichung vom Desktop (Spec §6, iPhone): Texte werden vorab geladen,
// Originale nur auf ausdrückliche Anforderung (Vorschau, Öffnen, Teilen), nie von selbst. Reine Logik, kein I/O.

/// Welche Originale und Texte dieses Gerät schon hat (aus dem Dateisystem gelesen, nie gespeichert).
public struct AblageVorhanden: Equatable, Sendable {
    /// Schlüssel `<dat>/<fas>`.
    public var originale = AblageMenge()
    /// Schlüssel `<dat>/<textrev>`.
    public var texte = AblageMenge()

    public init() {}

    private static func schluessel(_ a: String, _ b: String) -> String { a + "/" + b }

    public func original(_ dat: String, _ fas: String) -> Bool { originale.contains(Self.schluessel(dat, fas)) }
    public func text(_ dat: String, _ textrev: String) -> Bool { texte.contains(Self.schluessel(dat, textrev)) }
    public mutating func setzeOriginal(dat: String, fas: String) { originale.insert(Self.schluessel(dat, fas)) }
    public mutating func setzeText(dat: String, textrev: String) { texte.insert(Self.schluessel(dat, textrev)) }
}

/// Ausdrückliche Anforderung eines Originals (Vorschau, Öffnen, Teilen).
public struct AblageOriginalAnforderung: Equatable, Sendable {
    public var dat: String
    public var fas: String
    public init(dat: String, fas: String) { self.dat = dat; self.fas = fas }
}

/// Ein Download: eine Textfassung oder ein Original.
public struct AblageAuftrag: Equatable, Sendable {
    public enum Art: Int, Sendable { case text = 0, original = 1 }

    public var art: Art
    /// Klartext-Größe in Bytes (`Text.size` bzw. `Fassung.size`).
    public var groesse: UInt64
    public var dat: String
    public var fas: String
    /// Nur bei Texten.
    public var textrev: String
    /// SHA-256 (hex) des Klartexts: `Text.blob` bzw. `Fassung.blob`. Der Download wird dagegen geprüft.
    public var sha256Hex: String
    /// Hub-Kennung des Blobs (`SHA256(key ‖ "lv-blob-v1" ‖ sha)`).
    public var hubId: String

    /// Reihenfolge wie am Desktop: `(art, size, dat, fas, textrev)`, Zeichenketten nach Bytes.
    static func kleiner(_ a: AblageAuftrag, _ b: AblageAuftrag) -> Bool {
        if a.art != b.art { return a.art.rawValue < b.art.rawValue }
        if a.groesse != b.groesse { return a.groesse < b.groesse }
        for (x, y) in [(a.dat, b.dat), (a.fas, b.fas), (a.textrev, b.textrev)] where !AblageBytes.equal(x, y) { return AblageBytes.less(x, y) }
        return false
    }
}

/// Budget je Lauf: Klartext-Bytes, mindestens eine Datei je Lauf (`DOWNLOAD_BUDGET`, 256 MiB).
public struct AblageDownloadBudget: Equatable, Sendable {
    public static let standard: UInt64 = 256 * 1024 * 1024
    /// Klartext-Bytes an Textfassungen je Lauf im Mobilfunk unter „nur im WLAN“ (5 MiB); mehr wartet auf WLAN. Anders als
    /// `standard` gilt hier die Grenze streng: auch die erste Datei muss hineinpassen.
    public static let mobilText: UInt64 = 5 * 1024 * 1024

    public private(set) var rest: UInt64
    private var erste = true

    public init(rest: UInt64 = AblageDownloadBudget.standard) { self.rest = rest }

    /// Die erste Datei läuft immer, jede weitere nur, wenn sie noch ins Budget passt.
    public func darfStarten(groesse: UInt64) -> Bool { erste || groesse <= rest }

    /// Nach einem erfolgreichen Download (ein Fehlschlag verbraucht nichts).
    public mutating func verbraucht(groesse: UInt64) {
        erste = false
        rest = rest > groesse ? rest - groesse : 0
    }
}

/// Formate, für die der PC eine Textfassung erstellt (`format_aus_typ`). Nur die Formatliste, nie die Extraktor-
/// oder Versionstabelle: sie ändert sich mit jeder Extraktorversion und gehört dem PC.
public enum AblageFormat {
    public static func hatTextfassung(typ: String) -> Bool {
        let klein = String(decoding: typ.utf8.map { $0 >= 0x41 && $0 <= 0x5A ? $0 + 32 : $0 }, as: UTF8.self)
        switch klein {
        case "pdf", "docx", "docm", "odt", "pptx", "pptm", "odp", "xlsx", "xlsm", "ods",
             "txt", "md", "markdown", "csv", "tsv", "log", "html", "htm", "xhtml", "eml":
            return true
        default:
            return false
        }
    }
}

public enum AblageDownloadPlan {
    /// Größte Stückzahl eines Blobs (`MAX_CHUNKS`); mehr ist ein ungültiger Dateiverweis.
    public static let maxStuecke = 4096

    /// Darf zu dieser Datei überhaupt etwas geladen werden? Nur Objekte, die der Hub kennt und die nicht begraben
    /// sind; nicht bei unbekannter Klasse oder unvollständiger Kette (`Halten`); nie bei einem angehaltenen Stand
    /// (nicht lesbar, also nichts, wofür das Gerät einstehen kann).
    private static func darfLaden(_ z: AblageZustand, _ d: AblageDatei, _ teilnahme: AblageKarte<AblageTeilnahme>) -> Bool {
        !z.ledger.begraben.contains(d.id)
            && z.ledger.lebtAmHub(AblageLedger.sammlungDatei, d.id)
            && !z.ledger.angehalten(AblageLedger.sammlungDatei, d.id)
            && teilnahme[d.id] != .halten
    }

    private static func auftrag(_ art: AblageAuftrag.Art, groesse: UInt64, dat: String, fas: String, textrev: String, sha: String,
                                key: [UInt8]) -> AblageAuftrag? {
        guard let hub = AblageKrypto.hubBlobId(key: key, sha256Hex: sha),
              let n = Int(exactly: groesse), LibrarySyncFormat.chunkCount(size: n) <= maxStuecke else { return nil }
        return AblageAuftrag(art: art, groesse: groesse, dat: dat, fas: fas, textrev: textrev, sha256Hex: sha, hubId: hub)
    }

    /// Texte (`stand == ok`, `blob` nicht leer, noch nicht hier) zuerst, dann die angeforderten Originale; je Art
    /// kleinere zuerst, bei gleicher Größe nach Kennung (Bytes). Texte über der Lesegrenze (32 MiB) werden nie geladen.
    public static func auftraege(zustand z: AblageZustand, vorhanden: AblageVorhanden, key: [UInt8],
                                 angefordert: [AblageOriginalAnforderung] = []) -> [AblageAuftrag] {
        let teilnahme = AblageTeilnahme.alle(ordner: z.bestand.ordner, dateien: z.bestand.dateien)
        var aus: [AblageAuftrag] = []
        for d in z.bestand.dateien.values where darfLaden(z, d, teilnahme) {
            for f in d.fassungen {
                for t in f.texte where AblageBytes.equal(t.stand, "ok") && !t.blob.isEmpty && !vorhanden.text(d.id, t.textrev) {
                    guard t.size <= UInt64(AblageTextfassung.maxBytes),
                          let a = auftrag(.text, groesse: t.size, dat: d.id, fas: f.fas, textrev: t.textrev, sha: t.blob, key: key) else { continue }
                    aus.append(a)
                }
            }
        }
        for anf in angefordert where !vorhanden.original(anf.dat, anf.fas) {
            guard let a = original(zustand: z, dat: anf.dat, fas: anf.fas, key: key, teilnahme: teilnahme),
                  !aus.contains(where: { $0.art == .original && AblageBytes.equal($0.dat, a.dat) && AblageBytes.equal($0.fas, a.fas) }) else { continue }
            aus.append(a)
        }
        return aus.sorted(by: AblageAuftrag.kleiner)
    }

    /// Teilt einen Plan für ein teures Netz (Mobilfunk oder Datensparmodus unter „nur im WLAN“): Textfassungen in der
    /// Reihenfolge des Plans (kleinere zuerst), solange ihre Summe in `textBudget` passt; alles, was nicht mehr hineinpasst,
    /// und alle Originale warten auf WLAN. Ohne Ausnahme für die erste Datei (anders als `AblageDownloadBudget.darfStarten`).
    public static func mobilAufteilen(_ plan: [AblageAuftrag], textBudget: UInt64 = AblageDownloadBudget.mobilText)
        -> (jetzt: [AblageAuftrag], wartet: [AblageAuftrag]) {
        var rest = textBudget
        var jetzt: [AblageAuftrag] = [], wartet: [AblageAuftrag] = []
        for a in plan {
            if a.art == .text, a.groesse <= rest {
                jetzt.append(a)
                rest -= a.groesse
            } else {
                wartet.append(a)
            }
        }
        return (jetzt, wartet)
    }

    /// Auftrag für ein ausdrücklich angefordertes Original; `nil`, wenn die Datei oder Fassung unbekannt ist, nicht
    /// geladen werden darf (siehe oben) oder der Verweis ungültig ist (Kennung, Größe, mehr als 4096 Stücke).
    public static func original(zustand z: AblageZustand, dat: String, fas: String, key: [UInt8]) -> AblageAuftrag? {
        original(zustand: z, dat: dat, fas: fas, key: key,
                 teilnahme: AblageTeilnahme.alle(ordner: z.bestand.ordner, dateien: z.bestand.dateien))
    }

    private static func original(zustand z: AblageZustand, dat: String, fas: String, key: [UInt8],
                                 teilnahme: AblageKarte<AblageTeilnahme>) -> AblageAuftrag? {
        guard AblageId.idGueltig(dat, prefix: AblageId.praefixDatei), AblageId.idGueltig(fas, prefix: AblageId.praefixFassung),
              let d = z.bestand.dateien[dat], darfLaden(z, d, teilnahme),
              let f = d.fassungen.first(where: { AblageBytes.equal($0.fas, fas) }), AblageValidierung.typOk(f.typ) else { return nil }
        return auftrag(.original, groesse: f.size, dat: d.id, fas: f.fas, textrev: "", sha: f.blob, key: key)
    }
}
#endif
