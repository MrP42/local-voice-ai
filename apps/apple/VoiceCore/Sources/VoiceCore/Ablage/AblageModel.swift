import Foundation

// Datenmodell der Dokumentenablage auf dem iPhone (D5a-1, lva#441, Epic #395).
//
// Bitgleich zum Desktop: `apps/local-voice/src-tauri/src/ablage/zusammenfuehren.rs` (Typen `Register`, `Text`,
// `Fassung`, `Wahl`, `Datei`, `Ordner`), serde-Namen unverändert (JSON-Schlüssel stehen in den Kommentaren;
// die Swift-Eigenschaften heißen camelCase). Die Typen lesen und schreiben über `AblageJSON`, damit
// Ganzzahlen bis `UInt64.max` exakt bleiben, `null` nie verschwindet und unbekannte Felder (`extra`) in
// `Datei` und `Ordner` erhalten bleiben und beim Schreiben wieder erscheinen.
//
// Bewusst kein `Codable` (siehe AblageJSON.swift): Lesen und Schreiben laufen nur über `init(nutzlast:)`,
// `init(ablageJSON:)` und `kanonischeNutzlast`, damit der JSON-Text Byte für Byte erhalten bleibt.
//
// Hinweis zu `==`: Die synthetische Gleichheit der Typen folgt Swift-`String` (kanonische Äquivalenz).
// Wo eine Entscheidung davon abhängt (Rang, Ordnung, Zusammenführen), gilt `AblageBytes`.

// MARK: - Brücke zu JSON

/// Ein Wert, der aus `AblageJSON` gelesen und dorthin geschrieben werden kann.
public protocol AblageWert: Equatable, Sendable {
    init(ablageJSON: AblageJSON) throws
    var ablageJSON: AblageJSON { get }
    /// Wert eines fehlenden Schlüssels (nur `Optional` kennt ihn; serde liest fehlendes `Option` als `None`).
    static func fehlend() throws -> Self
}

extension AblageWert {
    public static func fehlend() throws -> Self { throw AblageError.feld("fehlt") }
}

extension String: AblageWert {
    public init(ablageJSON: AblageJSON) throws {
        guard case .string(let s) = ablageJSON else { throw AblageError.feld("string") }
        self = s
    }
    public var ablageJSON: AblageJSON { .string(self) }
}

extension Bool: AblageWert {
    public init(ablageJSON: AblageJSON) throws {
        guard case .bool(let b) = ablageJSON else { throw AblageError.feld("bool") }
        self = b
    }
    public var ablageJSON: AblageJSON { .bool(self) }
}

extension UInt64: AblageWert {
    public init(ablageJSON: AblageJSON) throws {
        guard case .uint(let u) = ablageJSON else { throw AblageError.feld("u64") }
        self = u
    }
    public var ablageJSON: AblageJSON { .uint(self) }
}

extension UInt32: AblageWert {
    public init(ablageJSON: AblageJSON) throws {
        guard case .uint(let u) = ablageJSON, let v = UInt32(exactly: u) else { throw AblageError.feld("u32") }
        self = v
    }
    public var ablageJSON: AblageJSON { .uint(UInt64(self)) }
}

extension Int64: AblageWert {
    public init(ablageJSON: AblageJSON) throws {
        switch ablageJSON {
        case .uint(let u): guard let v = Int64(exactly: u) else { throw AblageError.feld("i64") }; self = v
        case .int(let n): self = n
        default: throw AblageError.feld("i64")
        }
    }
    public var ablageJSON: AblageJSON { .integer(self) }
}

extension Optional: AblageWert where Wrapped: AblageWert {
    public init(ablageJSON: AblageJSON) throws {
        if case .null = ablageJSON { self = nil } else { self = .some(try Wrapped(ablageJSON: ablageJSON)) }
    }
    /// `None` wird als `null` geschrieben, nie weggelassen.
    public var ablageJSON: AblageJSON { map { $0.ablageJSON } ?? .null }
    public static func fehlend() throws -> Wrapped? { nil }
}

extension Array: AblageWert where Element: AblageWert {
    public init(ablageJSON: AblageJSON) throws {
        guard case .array(let a) = ablageJSON else { throw AblageError.feld("liste") }
        self = try a.map { try Element(ablageJSON: $0) }
    }
    public var ablageJSON: AblageJSON { .array(map { $0.ablageJSON }) }
}

extension AblageJSON {
    /// Pflichtfeld; `null` und Typfehler sind Fehler, ein fehlendes `Option`-Feld ist `nil`.
    func pflicht<T: AblageWert>(_ key: String, _ typ: T.Type = T.self) throws -> T {
        guard let v = self[key] else {
            do { return try T.fehlend() } catch { throw AblageError.feld(key) }
        }
        do { return try T(ablageJSON: v) } catch { throw AblageError.feld(key) }
    }

    /// Feld mit `#[serde(default)]`: fehlt es, gilt die Vorgabe; ist es da, muss es den Typ haben.
    func vorgabe<T: AblageWert>(_ key: String, _ standard: T) throws -> T {
        guard let v = self[key] else { return standard }
        do { return try T(ablageJSON: v) } catch { throw AblageError.feld(key) }
    }

    func istObjekt() throws {
        guard case .object = self else { throw AblageError.feld("objekt") }
    }

    /// Alle Wurzelschlüssel außer den bekannten (`#[serde(flatten)] extra`).
    func uebrige(ausser bekannt: Set<String>) -> [Member] {
        guard case .object(let m) = self else { return [] }
        return m.filter { mem in !bekannt.contains { AblageBytes.equal($0, mem.key) } }
    }

    /// Schema-Marke `v` (fehlt = 1). Wie der Desktop: nur eine nichtnegative Ganzzahl zählt, alles andere gilt
    /// als 1. `v` > 1: neueres Schema, das Objekt wird nicht verstanden.
    func pruefeSchema(unterstuetzt: UInt64 = 1) throws {
        if case .uint(let v)? = self["v"], v > unterstuetzt { throw AblageError.neueresSchema }
    }
}

// MARK: - Register

/// `{"wert": …, "zaehler": u64, "geraet": "g_…"}`. Das höhere `(zaehler, geraet)` gewinnt (`AblageZusammenfuehren.gewinner`).
public struct AblageRegister<T: AblageWert>: AblageWert {
    public var wert: T
    public var zaehler: UInt64
    public var geraet: String

    public init(wert: T, zaehler: UInt64, geraet: String) { self.wert = wert; self.zaehler = zaehler; self.geraet = geraet }

    public init(ablageJSON j: AblageJSON) throws {
        try j.istObjekt()
        wert = try j.pflicht("wert")
        zaehler = try j.pflicht("zaehler")
        geraet = try j.pflicht("geraet")
    }

    public var ablageJSON: AblageJSON {
        .objekt([("wert", wert.ablageJSON), ("zaehler", zaehler.ablageJSON), ("geraet", geraet.ablageJSON)])
    }
}

// MARK: - Text, Fassung, Wahl

/// `Text` (Rust): Eintrag einer Textfassung bei einer Fassung.
public struct AblageText: AblageWert {
    public var textrev: String
    /// Hex-SHA-256 der Bytes der Textfassungs-Datei; leer bei `fehler`.
    public var blob: String
    public var size: UInt64
    public var extraktor: String
    /// JSON `extraktor_version`.
    public var extraktorVersion: String
    /// JSON `marken_version`.
    public var markenVersion: UInt32
    /// `ok`, `kein_text` oder `fehler`.
    public var stand: String
    /// Fehlercode bei `fehler`; fehlt beim Schreiben, wenn `nil`.
    public var code: String?
    /// Fehlt beim Schreiben bei 0.
    public var versuche: UInt32
    /// JSON `versuche_zeit`; fehlt beim Schreiben bei 0.
    public var versucheZeit: UInt32

    public init(textrev: String, blob: String, size: UInt64, extraktor: String, extraktorVersion: String, markenVersion: UInt32,
                stand: String, code: String? = nil, versuche: UInt32 = 0, versucheZeit: UInt32 = 0) {
        self.textrev = textrev; self.blob = blob; self.size = size; self.extraktor = extraktor
        self.extraktorVersion = extraktorVersion; self.markenVersion = markenVersion; self.stand = stand
        self.code = code; self.versuche = versuche; self.versucheZeit = versucheZeit
    }

    public init(ablageJSON j: AblageJSON) throws {
        try j.istObjekt()
        textrev = try j.pflicht("textrev")
        blob = try j.pflicht("blob")
        size = try j.pflicht("size")
        extraktor = try j.pflicht("extraktor")
        extraktorVersion = try j.pflicht("extraktor_version")
        markenVersion = try j.pflicht("marken_version")
        stand = try j.pflicht("stand")
        code = try j.vorgabe("code", nil as String?)
        versuche = try j.vorgabe("versuche", 0 as UInt32)
        versucheZeit = try j.vorgabe("versuche_zeit", 0 as UInt32)
    }

    public var ablageJSON: AblageJSON {
        var p: [(String, AblageJSON)] = [
            ("textrev", textrev.ablageJSON), ("blob", blob.ablageJSON), ("size", size.ablageJSON),
            ("extraktor", extraktor.ablageJSON), ("extraktor_version", extraktorVersion.ablageJSON),
            ("marken_version", markenVersion.ablageJSON), ("stand", stand.ablageJSON),
        ]
        if let code { p.append(("code", code.ablageJSON)) }                     // skip_serializing_if = Option::is_none
        if versuche != 0 { p.append(("versuche", versuche.ablageJSON)) }        // skip_serializing_if = ist_null
        if versucheZeit != 0 { p.append(("versuche_zeit", versucheZeit.ablageJSON)) }
        return .objekt(p)
    }
}

/// `Fassung` (Rust). Unbekannte Felder verwirft der Desktop beim Neuschreiben ebenfalls (kein `flatten`).
public struct AblageFassung: AblageWert {
    public var fas: String
    /// `null` oder Fassungs-ID der Vorgängerin.
    public var vorgaenger: String?
    /// Hex-SHA-256 des Originals.
    public var blob: String
    public var size: UInt64
    /// JSON `original_sha256`.
    public var originalSha256: String
    /// Endung ohne Punkt.
    public var typ: String
    public var geraet: String
    /// Millisekunden.
    public var zeit: Int64
    public var texte: [AblageText]

    public init(fas: String, vorgaenger: String?, blob: String, size: UInt64, originalSha256: String, typ: String,
                geraet: String, zeit: Int64, texte: [AblageText] = []) {
        self.fas = fas; self.vorgaenger = vorgaenger; self.blob = blob; self.size = size
        self.originalSha256 = originalSha256; self.typ = typ; self.geraet = geraet; self.zeit = zeit; self.texte = texte
    }

    public init(ablageJSON j: AblageJSON) throws {
        try j.istObjekt()
        fas = try j.pflicht("fas")
        vorgaenger = try j.pflicht("vorgaenger")
        blob = try j.pflicht("blob")
        size = try j.pflicht("size")
        originalSha256 = try j.pflicht("original_sha256")
        typ = try j.pflicht("typ")
        geraet = try j.pflicht("geraet")
        zeit = try j.pflicht("zeit")
        texte = try j.vorgabe("texte", [AblageText]())
    }

    public var ablageJSON: AblageJSON {
        .objekt([
            ("fas", fas.ablageJSON), ("vorgaenger", vorgaenger.ablageJSON), ("blob", blob.ablageJSON), ("size", size.ablageJSON),
            ("original_sha256", originalSha256.ablageJSON), ("typ", typ.ablageJSON), ("geraet", geraet.ablageJSON),
            ("zeit", zeit.ablageJSON), ("texte", texte.ablageJSON),
        ])
    }
}

/// `Wahl` (Rust): unveränderliches Ereignis einer Konfliktentscheidung.
public struct AblageWahl: AblageWert {
    public var id: String
    public var gewaehlt: String
    public var gegen: [String]
    public var ersetzt: [String]
    public var zaehler: UInt64
    public var geraet: String

    public init(id: String, gewaehlt: String, gegen: [String] = [], ersetzt: [String] = [], zaehler: UInt64, geraet: String) {
        self.id = id; self.gewaehlt = gewaehlt; self.gegen = gegen; self.ersetzt = ersetzt; self.zaehler = zaehler; self.geraet = geraet
    }

    public init(ablageJSON j: AblageJSON) throws {
        try j.istObjekt()
        id = try j.pflicht("id")
        gewaehlt = try j.pflicht("gewaehlt")
        gegen = try j.vorgabe("gegen", [String]())
        ersetzt = try j.vorgabe("ersetzt", [String]())
        zaehler = try j.pflicht("zaehler")
        geraet = try j.pflicht("geraet")
    }

    public var ablageJSON: AblageJSON {
        .objekt([
            ("id", id.ablageJSON), ("gewaehlt", gewaehlt.ablageJSON), ("gegen", gegen.ablageJSON), ("ersetzt", ersetzt.ablageJSON),
            ("zaehler", zaehler.ablageJSON), ("geraet", geraet.ablageJSON),
        ])
    }
}

// MARK: - Ordner

/// `ablage_ordner` (Spec §2.2). `extra` = alle unbekannten Wurzelschlüssel (auch `v`), bleiben erhalten.
public struct AblageOrdner: AblageWert {
    public var id: String
    public var eltern: AblageRegister<String?>
    public var name: AblageRegister<String>
    /// `erbt | public | internal | confidential | personal`.
    public var datenklasse: AblageRegister<String>
    /// JSON `nur_geraet`.
    public var nurGeraet: AblageRegister<Bool>
    /// JSON `zielordner_ki`.
    public var zielordnerKi: AblageRegister<Bool>
    public var papierkorb: AblageRegister<Int64?>
    public var extra: [AblageJSON.Member]

    static let bekannt: Set<String> = ["id", "eltern", "name", "datenklasse", "nur_geraet", "zielordner_ki", "papierkorb"]

    public init(id: String, eltern: AblageRegister<String?>, name: AblageRegister<String>, datenklasse: AblageRegister<String>,
                nurGeraet: AblageRegister<Bool>, zielordnerKi: AblageRegister<Bool>, papierkorb: AblageRegister<Int64?>,
                extra: [AblageJSON.Member] = []) {
        self.id = id; self.eltern = eltern; self.name = name; self.datenklasse = datenklasse
        self.nurGeraet = nurGeraet; self.zielordnerKi = zielordnerKi; self.papierkorb = papierkorb; self.extra = extra
    }

    /// Prüft die Schema-Marke (`v` > 1 = `AblageError.neueresSchema`) und liest den Datensatz.
    public init(ablageJSON j: AblageJSON) throws {
        try j.istObjekt()
        try j.pruefeSchema()
        id = try j.pflicht("id")
        eltern = try j.pflicht("eltern")
        name = try j.pflicht("name")
        datenklasse = try j.pflicht("datenklasse")
        nurGeraet = try j.pflicht("nur_geraet")
        zielordnerKi = try j.pflicht("zielordner_ki")
        papierkorb = try j.pflicht("papierkorb")
        extra = j.uebrige(ausser: Self.bekannt)
    }

    /// Entschlüsselte Nutzlast (UTF-8-JSON) lesen.
    public init(nutzlast: [UInt8]) throws { try self.init(ablageJSON: try AblageJSON.parse(nutzlast)) }

    /// Bekannte Felder gewinnen über gleichnamige Einträge in `extra` (der Desktop kann das nicht erzeugen).
    public var ablageJSON: AblageJSON {
        .objekt(extra.map { ($0.key, $0.value) } + [
            ("id", id.ablageJSON), ("eltern", eltern.ablageJSON), ("name", name.ablageJSON), ("datenklasse", datenklasse.ablageJSON),
            ("nur_geraet", nurGeraet.ablageJSON), ("zielordner_ki", zielordnerKi.ablageJSON), ("papierkorb", papierkorb.ablageJSON),
        ])
    }
}

// MARK: - Datei

/// `ablage_datei` (Spec §2.1). `blobRefs` sind Inhalts-Hashes (hex), nicht Hub-Kennungen; beim Schreiben werden
/// sie immer neu aus den Fassungen abgeleitet.
public struct AblageDatei: AblageWert {
    public var id: String
    public var ordner: AblageRegister<String>
    public var name: AblageRegister<String>
    /// JSON `klasse_min`: `erbt` oder ein Klassenwert.
    public var klasseMin: AblageRegister<String>
    /// JSON `nur_geraet_min`, Vorgabe `false`.
    public var nurGeraetMin: Bool
    public var fassungen: [AblageFassung]
    public var wahlen: [AblageWahl]
    /// IDs entfernter Fassungen; wächst nur.
    public var entfernt: [String]
    /// JSON `blob_refs`: Inhalts-Hashes aller Originale und Texte (nur zum Lesen maßgeblich).
    public var blobRefs: [String]
    public var papierkorb: AblageRegister<Int64?>
    public var extra: [AblageJSON.Member]

    static let bekannt: Set<String> = ["id", "ordner", "name", "klasse_min", "nur_geraet_min", "fassungen", "wahlen", "entfernt", "blob_refs", "papierkorb"]

    public init(id: String, ordner: AblageRegister<String>, name: AblageRegister<String>, klasseMin: AblageRegister<String>,
                nurGeraetMin: Bool = false, fassungen: [AblageFassung] = [], wahlen: [AblageWahl] = [], entfernt: [String] = [],
                blobRefs: [String] = [], papierkorb: AblageRegister<Int64?>, extra: [AblageJSON.Member] = []) {
        self.id = id; self.ordner = ordner; self.name = name; self.klasseMin = klasseMin; self.nurGeraetMin = nurGeraetMin
        self.fassungen = fassungen; self.wahlen = wahlen; self.entfernt = entfernt; self.blobRefs = blobRefs
        self.papierkorb = papierkorb; self.extra = extra
    }

    /// Prüft die Schema-Marke (`v` > 1 = `AblageError.neueresSchema`) und liest den Datensatz.
    public init(ablageJSON j: AblageJSON) throws {
        try j.istObjekt()
        try j.pruefeSchema()
        id = try j.pflicht("id")
        ordner = try j.pflicht("ordner")
        name = try j.pflicht("name")
        klasseMin = try j.pflicht("klasse_min")
        nurGeraetMin = try j.vorgabe("nur_geraet_min", false)
        fassungen = try j.vorgabe("fassungen", [AblageFassung]())
        wahlen = try j.vorgabe("wahlen", [AblageWahl]())
        entfernt = try j.vorgabe("entfernt", [String]())
        blobRefs = try j.vorgabe("blob_refs", [String]())
        papierkorb = try j.pflicht("papierkorb")
        extra = j.uebrige(ausser: Self.bekannt)
    }

    /// Entschlüsselte Nutzlast (UTF-8-JSON) lesen.
    public init(nutzlast: [UInt8]) throws { try self.init(ablageJSON: try AblageJSON.parse(nutzlast)) }

    /// Inhalts-Hashes aller Originale (`blob` immer, wie `blob_refs_aus`) und aller Texte mit nicht leerem `blob`,
    /// bytegenau sortiert und eindeutig.
    public static func inhaltsRefs(_ fassungen: [AblageFassung]) -> [String] {
        var alle: [String] = []
        for f in fassungen {
            alle.append(f.blob)
            alle.append(contentsOf: f.texte.map(\.blob).filter { !$0.isEmpty })
        }
        return AblageBytes.sortedUnique(alle)
    }

    /// Dieselbe Datei in der Ablage-Ordnung des Zusammenführens: `fassungen` und `wahlen` nach der abgeleiteten
    /// Rust-Ordnung (`#[derive(Ord)]`: alle Felder in Reihenfolge der Struktur, siehe `AblageOrdnung`), `texte` je
    /// Fassung ebenso, `entfernt` sortiert und eindeutig, `blob_refs` neu abgeleitet; alles bytegenau. Genau gleiche
    /// Einträge fallen wie in einer Menge zusammen. Einträge mit gleicher `fas`, `id` oder `textrev`, aber anderem
    /// Inhalt bleiben beide stehen (deren Auflösung, das Größere gewinnt, ist Sache des Zusammenführens); ihre
    /// Reihenfolge ist trotzdem von der Eingabe unabhängig. Entfernt keine Fassung aus `entfernt`.
    public func geordnet() -> AblageDatei {
        var d = self
        d.fassungen = AblageOrdnung.sortiertEindeutig(fassungen.map { f in
            var f = f
            f.texte = AblageOrdnung.sortiertEindeutig(f.texte, AblageOrdnung.vergleiche)
            return f
        }, AblageOrdnung.vergleiche)
        d.wahlen = AblageOrdnung.sortiertEindeutig(wahlen, AblageOrdnung.vergleiche)
        d.entfernt = AblageBytes.sortedUnique(entfernt)
        d.blobRefs = Self.inhaltsRefs(d.fassungen)
        return d
    }

    /// Wie `wert_datei` am Desktop: `blob_refs` wird aus den Fassungen neu abgeleitet. Bekannte Felder gewinnen
    /// über gleichnamige Einträge in `extra`. `ordnen` ist Pflicht: `false` = Reihenfolge unverändert (Rusts
    /// `hash_datei`), `true` = Ergebnis von `zusammenfuehren_datei` (siehe `geordnet()`).
    public func nutzlastJSON(ordnen: Bool) -> AblageJSON {
        let d = ordnen ? geordnet() : self
        return .objekt(d.extra.map { ($0.key, $0.value) } + [
            ("id", d.id.ablageJSON), ("ordner", d.ordner.ablageJSON), ("name", d.name.ablageJSON),
            ("klasse_min", d.klasseMin.ablageJSON), ("nur_geraet_min", d.nurGeraetMin.ablageJSON),
            ("fassungen", d.fassungen.ablageJSON), ("wahlen", d.wahlen.ablageJSON), ("entfernt", d.entfernt.ablageJSON),
            ("blob_refs", Self.inhaltsRefs(d.fassungen).ablageJSON), ("papierkorb", d.papierkorb.ablageJSON),
        ])
    }

    public var ablageJSON: AblageJSON { nutzlastJSON(ordnen: false) }
}

// MARK: - Ordnung der Rust-Typen

/// Die von Rust abgeleitete Ordnung (`#[derive(PartialOrd, Ord)]`) von `Text`, `Fassung` und `Wahl`: Felder in der
/// Reihenfolge der Struktur, Zeichenketten nach UTF-8-Bytes, `Option` mit `None` vor `Some`, `Vec` lexikographisch
/// (kürzere Liste bei gleichem Anfang zuerst). Ergebnis -1, 0 oder 1; 0 heißt in allen Feldern bytegleich.
public enum AblageOrdnung {
    private static func zahl<T: Comparable>(_ a: T, _ b: T) -> Int { a == b ? 0 : (a < b ? -1 : 1) }

    private static func kette(_ schritte: (() -> Int)...) -> Int {
        for s in schritte { let r = s(); if r != 0 { return r } }
        return 0
    }

    public static func vergleiche(_ a: String?, _ b: String?) -> Int {
        switch (a, b) {
        case (nil, nil): return 0
        case (nil, _): return -1
        case (_, nil): return 1
        case (let x?, let y?): return AblageBytes.compare(x, y)
        }
    }

    public static func vergleiche(_ a: [String], _ b: [String]) -> Int {
        for (x, y) in zip(a, b) { let r = AblageBytes.compare(x, y); if r != 0 { return r } }
        return zahl(a.count, b.count)
    }

    public static func vergleiche(_ a: AblageText, _ b: AblageText) -> Int {
        kette({ AblageBytes.compare(a.textrev, b.textrev) }, { AblageBytes.compare(a.blob, b.blob) }, { zahl(a.size, b.size) },
              { AblageBytes.compare(a.extraktor, b.extraktor) }, { AblageBytes.compare(a.extraktorVersion, b.extraktorVersion) },
              { zahl(a.markenVersion, b.markenVersion) }, { AblageBytes.compare(a.stand, b.stand) }, { vergleiche(a.code, b.code) },
              { zahl(a.versuche, b.versuche) }, { zahl(a.versucheZeit, b.versucheZeit) })
    }

    public static func vergleiche(_ a: [AblageText], _ b: [AblageText]) -> Int {
        for (x, y) in zip(a, b) { let r = vergleiche(x, y); if r != 0 { return r } }
        return zahl(a.count, b.count)
    }

    public static func vergleiche(_ a: AblageFassung, _ b: AblageFassung) -> Int {
        kette({ AblageBytes.compare(a.fas, b.fas) }, { vergleiche(a.vorgaenger, b.vorgaenger) }, { AblageBytes.compare(a.blob, b.blob) },
              { zahl(a.size, b.size) }, { AblageBytes.compare(a.originalSha256, b.originalSha256) }, { AblageBytes.compare(a.typ, b.typ) },
              { AblageBytes.compare(a.geraet, b.geraet) }, { zahl(a.zeit, b.zeit) }, { vergleiche(a.texte, b.texte) })
    }

    public static func vergleiche(_ a: AblageWahl, _ b: AblageWahl) -> Int {
        kette({ AblageBytes.compare(a.id, b.id) }, { AblageBytes.compare(a.gewaehlt, b.gewaehlt) }, { vergleiche(a.gegen, b.gegen) },
              { vergleiche(a.ersetzt, b.ersetzt) }, { zahl(a.zaehler, b.zaehler) }, { AblageBytes.compare(a.geraet, b.geraet) })
    }

    /// Aufsteigend sortiert, genau gleiche Elemente (Vergleich 0) nur einmal, wie `BTreeSet`.
    public static func sortiertEindeutig<T>(_ xs: [T], _ vergleich: (T, T) -> Int) -> [T] {
        var out: [T] = []
        for x in xs.sorted(by: { vergleich($0, $1) < 0 }) where out.last.map({ vergleich($0, x) != 0 }) ?? true { out.append(x) }
        return out
    }
}

// MARK: - Kanonische Nutzlast

extension AblageOrdner {
    /// Klartext der Nutzlast: `collect::canonical(wert_ordner)`.
    public func kanonischeNutzlast() -> [UInt8] { ablageJSON.kanonisch() }
}

extension AblageDatei {
    /// Klartext der Nutzlast: `collect::canonical(wert_datei)`. Die Wahl von `ordnen` trifft jede Aufrufstelle selbst:
    /// - `false`: Reihenfolge der Listen unverändert, genau `hash_datei` am Desktop (der nicht sortiert).
    /// - `true`: Listen in der Ordnung des Ergebnisses von `zusammenfuehren_datei` (siehe `geordnet()`), damit zwei
    ///   Geräte denselben Hash erzeugen; richtig für Stände, die gesendet werden.
    public func kanonischeNutzlast(ordnen: Bool) -> [UInt8] { nutzlastJSON(ordnen: ordnen).kanonisch() }
}

// MARK: - Kennungen

public enum AblageId {
    public static let praefixOrdner = "ord_"
    public static let praefixDatei = "dat_"
    public static let praefixFassung = "fas_"
    public static let praefixGeraet = "g_"
    public static let praefixWahl = "wahl_"

    /// Crockford-Base32 ohne I, L, O, U; Groß- und Kleinbuchstaben gelten (wie `ulid::Ulid::from_string`).
    private static let alphabet = Array("0123456789ABCDEFGHJKMNPQRSTVWXYZ".utf8)

    /// Ist `id` eine Kennung `<präfix><ULID>`? Wie `ablage::id_gueltig`: Präfix, dann genau 26 Bytes aus dem
    /// ULID-Alphabet (die `ulid`-Bibliothek prüft keinen Überlauf des ersten Zeichens). Pfadtrenner und `..`
    /// fallen damit heraus.
    public static func idGueltig(_ id: String, prefix: String) -> Bool {
        let u = Array(id.utf8), p = Array(prefix.utf8)
        guard u.count == p.count + 26, Array(u[0..<p.count]) == p else { return false }
        return u[p.count...].allSatisfy { c in
            switch c {
            case UInt8(ascii: "0")...UInt8(ascii: "9"): return true
            case UInt8(ascii: "A")...UInt8(ascii: "Z"), UInt8(ascii: "a")...UInt8(ascii: "z"):
                return alphabet.contains(c & 0xDF)
            default: return false
            }
        }
    }

    /// 16 Bytes als ULID-Text (26 Zeichen, Großbuchstaben, erstes Zeichen trägt nur 3 Bit).
    public static func ulid(_ bytes: [UInt8]) -> String? {
        guard bytes.count == 16 else { return nil }
        var out: [UInt8] = []
        var acc: UInt32 = 0
        var bits = 2                       // 130 = 26 × 5 Bit: zwei führende Nullbits
        for byte in bytes {
            acc = acc << 8 | UInt32(byte)
            bits += 8
            while bits >= 5 { bits -= 5; out.append(alphabet[Int((acc >> UInt32(bits)) & 31)]) }
            acc &= (1 << UInt32(bits)) - 1
        }
        return String(decoding: out, as: UTF8.self)
    }
}
