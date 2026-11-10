import Foundation

// Ledger und Zustand der Dokumentenablage je Konto (D5a-3, lva#441, Epic #395).
//
// Port von `apps/local-voice/src-tauri/src/sync/ablage.rs` (`ObjStand`, `Zustand`, `konto_kennung`,
// `Zustand::laden`). Das iPhone liest die Ablage nur (D5a, kein Push), deshalb fehlen Rückstellung, Upload-Menge und
// Löschkandidaten. Der Zustand eines Kontos besteht aus
// - dem `AblageLedger`: Konto, Cursor je Sammlung, Stand je Hub-Objekt, Menge begrabener Kennungen (nie wieder
//   aufnehmen) und die gemerkten Ordnergräber (erst nach dem Datei-Pull anzuwenden), und
// - dem `AblageBestand`: den zusammengeführten Ordnern und Dateien dieses Geräts.
// Beides liegt in einer Datei (`AblageSpeicher`), damit Cursor und Bestand nie auseinanderlaufen.
//
// Kanonisch persistiert über `AblageJSON` (nie `Codable`): ganze Zahlen bleiben exakt, `null` bleibt, `extra` bleibt
// erhalten, Schlüssel und Kennungen sind bytegenau (nie `Dictionary<String, _>`). Ein unlesbarer, neuerer oder
// fremder Zustand ergibt einen leeren Zustand (wie `Zustand::laden`), nie einen Absturz; alles im Bestand lässt sich
// vom Hub neu ziehen (D5a schreibt nichts zurück, eigene Importe gibt es erst in D5b).

/// Stand eines Hub-Objekts, wie ihn dieses Gerät kennt (`ObjStand`).
public struct AblageObjStand: Equatable, Sendable {
    public var revision: Int64
    /// Hash des Stands, den der Hub hat (leer beim Grabstein oder unlesbaren Objekt).
    public var hash: String
    public var deleted: Bool
    /// Hub-Kennungen, die dieses Objekt am Hub referenziert (Pull-Kopf `blob_refs`).
    public var refs: [String]
    /// Der entfernte Stand ist hier unlesbar (Siegel, JSON, neueres Schema, fremder Schlüssel, Grabstein ohne
    /// Siegel, Kennung ungleich `object_id`): nie überschreiben, nie indizieren, bis ein lesbarer Stand eintrifft.
    public var angehalten: Bool

    public init(revision: Int64 = 0, hash: String = "", deleted: Bool = false, refs: [String] = [], angehalten: Bool = false) {
        self.revision = revision; self.hash = hash; self.deleted = deleted; self.refs = refs; self.angehalten = angehalten
    }

    /// Grabstein vom Hub (`ObjStand::grab`): mit gültigem Siegel begraben; ohne ruht das Objekt (nicht als gelöscht
    /// vermerkt).
    public static func grab(revision: Int64, echt: Bool) -> AblageObjStand {
        AblageObjStand(revision: revision, deleted: echt, angehalten: !echt)
    }

    var ablageJSON: AblageJSON {
        .objekt([("revision", revision.ablageJSON), ("hash", .string(hash)), ("deleted", .bool(deleted)),
                 ("refs", .array(refs.map { .string($0) })), ("angehalten", .bool(angehalten))])
    }

    init(ablageJSON j: AblageJSON) throws {
        try j.istObjekt()
        revision = try j.pflicht("revision")
        hash = try j.vorgabe("hash", "")
        deleted = try j.vorgabe("deleted", false)
        refs = try j.vorgabe("refs", [String]())
        angehalten = try j.vorgabe("angehalten", false)
    }
}

/// Zustand des Abgleichs für ein Konto (`Zustand`, ohne die Teile für das Senden).
public struct AblageLedger: Equatable, Sendable {
    public static let version: UInt64 = 1
    public static let sammlungOrdner = "ablage_ordner"
    public static let sammlungDatei = "ablage_datei"
    /// Sammlungen in der Reihenfolge des Pulls: Ordner vor Dateien.
    public static let sammlungen = [sammlungOrdner, sammlungDatei]

    /// Benutzer, Schlüsselkennung und Hub (`konto_kennung`): ein anderes Konto beginnt mit leerem Zustand.
    public var konto: String
    /// Pull-Cursor je Sammlung.
    public var cursor: AblageKarte<Int64>
    /// Stand je Hub-Objekt, Schlüssel `<sammlung>/<id>`.
    public var objekte: AblageKarte<AblageObjStand>
    /// Endgültig gelöschte Kennungen (Dateien und Ordner): werden nie wieder aufgenommen.
    public var begraben: AblageMenge
    /// Geprüfte Ordnergräber, die erst nach dem Pull der Dateien angewendet werden (sonst läge eine gleichzeitig
    /// begrabene Datei noch darunter und hielte den Ordner).
    public var ordnergraeber: AblageMenge

    public init(konto: String) {
        self.konto = konto
        cursor = AblageKarte()
        objekte = AblageKarte()
        begraben = AblageMenge()
        ordnergraeber = AblageMenge()
    }

    /// `"<user_id>:<key_id>:<hub_url ohne / am Ende>"` (`trim_end_matches('/')` entfernt alle).
    public static func kontoKennung(userId: Int64, keyId: String, hubUrl: String) -> String {
        var url = Substring(hubUrl)
        while url.last == "/" { url = url.dropLast() }
        return "\(userId):\(keyId):\(url)"
    }

    public static func schluessel(_ collection: String, _ id: String) -> String { "\(collection)/\(id)" }

    public func objekt(_ collection: String, _ id: String) -> AblageObjStand? { objekte[Self.schluessel(collection, id)] }

    /// Lebt das Objekt (nach dem Wissen dieses Geräts) am Hub?
    public func lebtAmHub(_ collection: String, _ id: String) -> Bool {
        objekt(collection, id).map { $0.revision > 0 && !$0.deleted } ?? false
    }

    public func angehalten(_ collection: String, _ id: String) -> Bool { objekt(collection, id)?.angehalten ?? false }

    /// Schlüssel (`<sammlung>/<id>`) aller Objekte, deren Hub-Stand hier unlesbar ist.
    public var angehalteneKennungen: [String] {
        objekte.eintraege.filter { $0.value.angehalten }.map(\.key)
    }

    // MARK: JSON

    var ablageJSON: AblageJSON {
        .objekt([
            ("konto", .string(konto)),
            ("cursor", .objekt(cursor.eintraege.map { ($0.key, $0.value.ablageJSON) })),
            ("objekte", .objekt(objekte.eintraege.map { ($0.key, $0.value.ablageJSON) })),
            ("begraben", .array(begraben.sortiert.map { .string($0) })),
            ("ordnergraeber", .array(ordnergraeber.sortiert.map { .string($0) })),
        ])
    }

    init(ablageJSON j: AblageJSON) throws {
        try j.istObjekt()
        konto = try j.pflicht("konto")
        cursor = AblageKarte()
        objekte = AblageKarte()
        if let c = j["cursor"] {
            guard case .object(let m) = c else { throw AblageError.feld("cursor") }
            for mem in m { cursor[mem.key] = try Int64(ablageJSON: mem.value) }
        }
        if let o = j["objekte"] {
            guard case .object(let m) = o else { throw AblageError.feld("objekte") }
            for mem in m { objekte[mem.key] = try AblageObjStand(ablageJSON: mem.value) }
        }
        begraben = AblageMenge(try j.vorgabe("begraben", [String]()))
        ordnergraeber = AblageMenge(try j.vorgabe("ordnergraeber", [String]()))
    }
}

/// Ordner und Dateien dieses Geräts, wie der Abgleich sie zusammengeführt hat.
public struct AblageBestand: Equatable, Sendable {
    public var ordner: AblageOrdnerbestand
    public var dateien: AblageDateibestand

    public init(ordner: AblageOrdnerbestand = AblageOrdnerbestand(), dateien: AblageDateibestand = AblageDateibestand()) {
        self.ordner = ordner; self.dateien = dateien
    }
}

/// Ledger und Bestand eines Kontos, zusammen in einer Datei.
public struct AblageZustand: Equatable, Sendable {
    public static let version: UInt64 = 1

    /// Warum ein geladener Zustand leer ist.
    public enum Verlust: Equatable, Sendable {
        case keiner
        /// Keine Datei (erster Start).
        case fehlt
        /// Datei unlesbar (JSON, Typen, Pflichtfelder).
        case unlesbar
        /// Neuere Version dieses Formats.
        case neuereVersion
        /// Der Zustand gehört zu einem anderen Konto (Benutzer, Schlüssel oder Hub).
        case anderesKonto
    }

    public var ledger: AblageLedger
    public var bestand: AblageBestand

    public init(konto: String) {
        ledger = AblageLedger(konto: konto)
        bestand = AblageBestand()
    }

    /// Kanonische Bytes (Schlüssel sortiert, kompakt, Zahlen exakt).
    public func serialisieren() -> [UInt8] {
        var ordner: [(String, AblageJSON)] = []
        for (id, o) in bestand.ordner.eintraege { ordner.append((id, o.ablageJSON)) }
        var dateien: [(String, AblageJSON)] = []
        for (id, d) in bestand.dateien.eintraege { dateien.append((id, d.ablageJSON)) }
        guard case .object(let l) = ledger.ablageJSON else { return [] }
        return AblageJSON.objekt(l.map { ($0.key, $0.value) } + [
            ("v", .uint(Self.version)), ("ordner", .objekt(ordner)), ("dateien", .objekt(dateien)),
        ]).kanonisch()
    }

    /// Liest einen Zustand; wirft bei jeder Abweichung (auch bei anderem Konto, siehe `laden`).
    public init(bytes: [UInt8], konto: String) throws {
        let j = try AblageJSON.parse(bytes)
        try j.istObjekt()
        let version: UInt64 = try j.pflicht("v")
        guard version <= Self.version else { throw AblageError.neueresSchema }
        let l = try AblageLedger(ablageJSON: j)
        guard AblageBytes.equal(l.konto, konto) else { throw AblageError.feld("konto") }
        var b = AblageBestand()
        if let o = j["ordner"] {
            guard case .object(let m) = o else { throw AblageError.feld("ordner") }
            for mem in m {
                let ordner = try AblageOrdner(ablageJSON: mem.value)
                guard AblageBytes.equal(ordner.id, mem.key) else { throw AblageError.feld("ordner") }
                b.ordner[mem.key] = ordner
            }
        }
        if let d = j["dateien"] {
            guard case .object(let m) = d else { throw AblageError.feld("dateien") }
            for mem in m {
                let datei = try AblageDatei(ablageJSON: mem.value)
                guard AblageBytes.equal(datei.id, mem.key) else { throw AblageError.feld("dateien") }
                b.dateien[mem.key] = datei
            }
        }
        ledger = l
        bestand = b
    }

    /// Lädt den Zustand; fehlt er, ist er unlesbar, neuer oder von einem anderen Konto, beginnt das Gerät mit einem
    /// leeren Zustand dieses Kontos (`Zustand::laden` und die Kontoprüfung in `lauf`). Nie ein Absturz.
    public static func laden(bytes: [UInt8]?, konto: String) -> (zustand: AblageZustand, verlust: Verlust) {
        let leer = AblageZustand(konto: konto)
        guard let bytes else { return (leer, .fehlt) }
        do {
            return (try AblageZustand(bytes: bytes, konto: konto), .keiner)
        } catch AblageError.neueresSchema {
            return (leer, .neuereVersion)
        } catch {
            // Konto prüfen, ohne dem Rest zu trauen: nur ein lesbarer Zustand eines anderen Kontos heißt so.
            if let j = try? AblageJSON.parse(bytes), case .string(let k)? = j["konto"], !AblageBytes.equal(k, konto),
               (try? AblageLedger(ablageJSON: j)) != nil {
                return (leer, .anderesKonto)
            }
            return (leer, .unlesbar)
        }
    }
}
