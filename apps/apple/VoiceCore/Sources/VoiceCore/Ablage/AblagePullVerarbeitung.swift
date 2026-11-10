import Foundation
#if canImport(CryptoKit)

// Verarbeitung einer Pull-Seite der Dokumentenablage (D5a-3, lva#441, Epic #395).
//
// Port von `apps/local-voice/src-tauri/src/sync/ablage.rs` (`seite_lesen`, `entschluesseln`, `grab_echt`-Anwendung,
// Ordnergräber nach dem Datei-Pull) und `ablage/speicher_sync.rs` (`sync_einspielen`, `ordner_gueltig`,
// `datei_gueltig`). Reine Logik auf Werttypen: kein I/O, keine Uhr, kein Netz, kein Logging. Nichts hier gibt
// Namen oder Inhalte aus: Berichte tragen nur Kennungen und feste Warncodes (Spec §10).
//
// Fail-closed: Was nicht eindeutig lesbar und gültig ist, wird nie eingespielt, nie überschrieben und nie gelöscht.
// Ein unlesbarer Hub-Stand hält das Objekt an (`AblageObjStand.angehalten`): der lokale Stand bleibt, bis ein
// lesbarer Stand eintrifft. Ein Grabstein gilt nur mit gültigem Siegel (`AblageKrypto.grabEcht`).

/// Konto und Schlüssel eines Laufs (`Konto`).
public struct AblageKonto: Equatable, Sendable {
    public let key: [UInt8]
    public let userId: Int64
    /// Adresse des Hubs (Teil der Kontobindung des Zustands).
    public let hubUrl: String

    public init(key: [UInt8], userId: Int64, hubUrl: String) {
        self.key = key; self.userId = userId; self.hubUrl = hubUrl
    }

    public var keyId: String { SyncCrypto.keyId(key) }
    public var kennung: String { AblageLedger.kontoKennung(userId: userId, keyId: keyId, hubUrl: hubUrl) }
}

/// Ein Objekt der Pull-Antwort (`RemoteObject`).
public struct AblagePullObjekt: Equatable, Sendable {
    public var collection: String
    public var objectId: String
    public var revision: Int64
    public var payload: String
    public var keyId: String
    public var deleted: Bool
    public var deviceId: String
    public var serverSeq: Int64
    /// Steuerfeld der Ablage (`ablage_datei`): Hub-Kennungen, wie der Hub sie führt.
    public var blobRefs: [String]?

    public init(collection: String, objectId: String, revision: Int64, payload: String, keyId: String, deleted: Bool,
                deviceId: String, serverSeq: Int64, blobRefs: [String]?) {
        self.collection = collection; self.objectId = objectId; self.revision = revision; self.payload = payload
        self.keyId = keyId; self.deleted = deleted; self.deviceId = deviceId; self.serverSeq = serverSeq; self.blobRefs = blobRefs
    }

    /// Wie serde: `collection`, `object_id`, `revision` sind Pflicht; `payload`, `key_id`, `deleted`, `device_id`,
    /// `server_seq` haben Vorgaben; `blob_refs` fehlt oder ist `null` oder eine Liste von Zeichenketten.
    init(ablageJSON j: AblageJSON) throws {
        try j.istObjekt()
        collection = try j.pflicht("collection")
        objectId = try j.pflicht("object_id")
        revision = try j.pflicht("revision")
        payload = try j.vorgabe("payload", "")
        keyId = try j.vorgabe("key_id", "")
        deleted = try j.vorgabe("deleted", false)
        deviceId = try j.vorgabe("device_id", "")
        serverSeq = try j.vorgabe("server_seq", 0 as Int64)
        blobRefs = try j.vorgabe("blob_refs", nil as [String]?)
    }
}

/// Antwort von `GET /api/v1/voice/sync/pull?since&limit&collection` (`PullResponse`).
public struct AblagePullSeite: Equatable, Sendable {
    public var objects: [AblagePullObjekt]
    public var cursor: Int64
    public var more: Bool

    public init(objects: [AblagePullObjekt], cursor: Int64, more: Bool) {
        self.objects = objects; self.cursor = cursor; self.more = more
    }

    /// Liest die Antwort über `AblageJSON` (ganze Zahlen exakt, nie über `Double`). Jede Abweichung macht die
    /// ganze Antwort unlesbar (wie `res.json::<PullResponse>()` am Desktop).
    public init(bytes: [UInt8]) throws {
        let j = try AblageJSON.parse(bytes)
        try j.istObjekt()
        guard case .array(let liste)? = j["objects"] else { throw AblageError.feld("objects") }
        objects = try liste.map { try AblagePullObjekt(ablageJSON: $0) }
        cursor = try j.pflicht("cursor")
        more = try j.pflicht("more")
    }
}

/// Feste Warncodes ohne Namen, Kennungen oder Inhalte (Spec §10).
public enum AblageWarnung: Hashable, Sendable {
    /// Objekte mit anderem Schlüssel im Konto: sie ruhen, der Lauf gilt als nicht vollständig.
    case fremderSchluessel
    case grabsteinOhneSiegel
    /// Ein Eintrag vom Hub ist hier nicht lesbar (Siegel, JSON, Schema, Kennung, Ableitung) und ruht.
    case unlesbar
    /// Ein Eintrag ist lesbar, aber ungültig (Kennung, Name, Typ, Hash-Felder) und wurde übersprungen.
    case verworfen
    /// Eine heruntergeladene Datei passte nicht zu ihrem Eintrag und wurde verworfen.
    case dateiPasstNicht
    /// Eine heruntergeladene Datei konnte nicht gespeichert werden (Speicher voll, Schreibschutz): Downloads ruhen.
    case dateiNichtSpeicherbar

    public var text: String {
        switch self {
        case .fremderSchluessel: return "Ablage: Objekte mit anderem Schlüssel im Konto"
        case .grabsteinOhneSiegel: return "Ablage: ein Grabstein ohne gültiges Siegel wurde ignoriert"
        case .unlesbar: return "Ablage: ein Eintrag vom Hub ist hier nicht lesbar und ruht"
        case .verworfen: return "Ablage: ein ungültiger Eintrag vom Hub wurde übersprungen"
        case .dateiPasstNicht: return "Ablage: eine Datei vom Hub passte nicht"
        case .dateiNichtSpeicherbar: return "Ablage: eine Datei konnte nicht gespeichert werden (Speicher voll?)"
        }
    }
}

/// Ergebnis einer verarbeiteten Seite: nur Kennungen und Zähler, nie Namen.
public struct AblageSeitenBericht: Equatable, Sendable {
    /// Ordner oder Dateien dieses Geräts haben sich geändert (oder wurden begraben).
    public var geaendert = false
    /// `false`, sobald ein fremder Schlüssel auftauchte (der Pull geht weiter, ist aber nicht vollständig).
    public var vollstaendig = true
    public var warnungen: [AblageWarnung] = []
    /// Kennungen ungültiger Datensätze (nicht eingespielt).
    public var verworfen: [String] = []
    /// Kennungen, die hier begraben wurden.
    public var begraben: [String] = []
    /// Grabsteine, die hier nicht gelten (nur Gerät, vertraulich/privat, unklar, oder ein Ordner mit lebendem Inhalt).
    public var gehalten: [String] = []
    /// Schlüssel (`<sammlung>/<id>`) der Objekte, die diese Seite angehalten hat.
    public var angehalten: [String] = []

    public init() {}

    mutating func warne(_ w: AblageWarnung) { if !warnungen.contains(w) { warnungen.append(w) } }

    /// Fasst Berichte mehrerer Seiten zusammen.
    public mutating func zusammenfuehren(_ b: AblageSeitenBericht) {
        geaendert = geaendert || b.geaendert
        vollstaendig = vollstaendig && b.vollstaendig
        for w in b.warnungen { warne(w) }
        verworfen += b.verworfen; begraben += b.begraben; gehalten += b.gehalten; angehalten += b.angehalten
    }
}

// MARK: - Gültigkeit entfernter Datensätze

/// Prüft entfernte Datensätze wie `ordner_gueltig` und `datei_gueltig` (`speicher_sync.rs`, `speicher.rs`).
public enum AblageValidierung {
    /// 64 Hex-Zeichen, klein.
    public static func hex64(_ s: String) -> Bool {
        let u = s.utf8
        return u.count == 64 && u.allSatisfy { ($0 >= 0x30 && $0 <= 0x39) || ($0 >= 0x61 && $0 <= 0x66) }
    }

    /// `name_lesbar`: nicht leer, höchstens 255 Unicode-Zeichen, kein Steuerzeichen (Kategorie Cc).
    public static func nameLesbar(_ s: String) -> Bool {
        let sc = s.unicodeScalars
        return !sc.isEmpty && sc.count <= 255 && !sc.contains { $0.properties.generalCategory == .control }
    }

    /// `typ_ok`: höchstens 10 Bytes, nur ASCII-Buchstaben und -Ziffern (Endung ohne Punkt, geht in Dateinamen).
    public static func typOk(_ s: String) -> Bool {
        let u = s.utf8
        return u.count <= 10 && u.allSatisfy { ($0 >= 0x30 && $0 <= 0x39) || ($0 >= 0x41 && $0 <= 0x5A) || ($0 >= 0x61 && $0 <= 0x7A) }
    }

    public static func ordnerGueltig(_ o: AblageOrdner) -> Bool {
        AblageId.idGueltig(o.id, prefix: AblageId.praefixOrdner)
            && nameLesbar(o.name.wert)
            && (o.eltern.wert.map { AblageId.idGueltig($0, prefix: AblageId.praefixOrdner) && !AblageBytes.equal($0, o.id) } ?? true)
    }

    public static func dateiGueltig(_ d: AblageDatei) -> Bool {
        AblageId.idGueltig(d.id, prefix: AblageId.praefixDatei)
            && nameLesbar(d.name.wert)
            && AblageId.idGueltig(d.ordner.wert, prefix: AblageId.praefixOrdner)
            && d.fassungen.allSatisfy { f in
                AblageId.idGueltig(f.fas, prefix: AblageId.praefixFassung)
                    && typOk(f.typ) && hex64(f.blob) && hex64(f.originalSha256) && AblageBytes.equal(f.blob, f.originalSha256)
                    && f.texte.allSatisfy { hex64($0.textrev) && ($0.blob.isEmpty || hex64($0.blob)) }
            }
            && d.entfernt.allSatisfy { AblageId.idGueltig($0, prefix: AblageId.praefixFassung) }
            && d.wahlen.allSatisfy { w in
                AblageId.idGueltig(w.gewaehlt, prefix: AblageId.praefixFassung)
                    && w.gegen.allSatisfy { AblageId.idGueltig($0, prefix: AblageId.praefixFassung) }
            }
    }
}

// MARK: - Verarbeitung

public enum AblagePullVerarbeitung {
    /// Größte Nutzlast (base64-Zeichen), die geöffnet wird. Der Hub nimmt höchstens ~4,3 MB an; alles Größere ist
    /// unlesbar, nie ein Speicherfresser.
    public static let maxPayloadZeichen = 8 * 1024 * 1024

    private enum Eingang {
        case ordner(AblageOrdner)
        case datei(AblageDatei)
        case grab(String)
    }

    /// Verarbeitet die Objekte einer Seite der Sammlung `sammlung` (`seite_lesen` und `sync_einspielen`): Öffnen mit
    /// AAD (Hub-`blob_refs`, eigene `key_id`, `deleted=0`), Prüfkette, Zusammenführen mit dem lokalen Stand,
    /// Grabsteine nur mit Siegel. Ordnergräber werden nur gemerkt (`AblageLedger.ordnergraeber`) und erst von
    /// `ordnergraeber(zustand:)` nach dem Datei-Pull angewendet. Objekte anderer Sammlungen und bereits bekannte
    /// Revisionen werden übersprungen.
    @discardableResult
    public static func seite(sammlung: String, objekte: [AblagePullObjekt], konto: AblageKonto, zustand: inout AblageZustand,
                             lokal: [String] = []) -> AblageSeitenBericht {
        var bericht = AblageSeitenBericht()
        var eingaenge: [Eingang] = []
        var buch: [(String, AblageObjStand)] = []
        let keyId = konto.keyId
        for r in objekte {
            guard AblageBytes.equal(r.collection, sammlung) else { continue }
            let k = AblageLedger.schluessel(sammlung, r.objectId)
            if let bekannt = zustand.ledger.objekte[k], bekannt.revision >= r.revision { continue } // auch die eigene, bestätigte Änderung
            if r.deleted {
                // Nur ein vom Konto versiegelter Grabstein gilt; angewendet wird er außerdem nur auf Einträge, die am
                // Abgleich teilnehmen (siehe `einspielen`).
                let echt = AblageKrypto.grabEcht(key: konto.key, userId: konto.userId, collection: sammlung, id: r.objectId,
                                                 keyId: r.keyId, payload: r.payload)
                if echt {
                    eingaenge.append(.grab(r.objectId))
                } else {
                    bericht.warne(.grabsteinOhneSiegel)
                    bericht.angehalten.append(k)
                }
                buch.append((k, AblageObjStand.grab(revision: r.revision, echt: echt)))
                continue
            }
            let refs = r.blobRefs ?? []
            if !AblageBytes.equal(r.keyId, keyId) {
                // Fremder Schlüssel: ruht wie unlesbar (nie überschreiben), der Pull geht weiter.
                bericht.vollstaendig = false
                bericht.warne(.fremderSchluessel)
                bericht.angehalten.append(k)
                buch.append((k, AblageObjStand(revision: r.revision, refs: refs, angehalten: true)))
                continue
            }
            if let (e, hash) = entschluesseln(sammlung: sammlung, id: r.objectId, refs: refs, payload: r.payload, konto: konto, keyId: keyId) {
                eingaenge.append(e)
                buch.append((k, AblageObjStand(revision: r.revision, hash: hash, deleted: false, refs: refs, angehalten: false)))
            } else {
                bericht.warne(.unlesbar)
                bericht.angehalten.append(k)
                buch.append((k, AblageObjStand(revision: r.revision, hash: "", deleted: false, refs: refs, angehalten: true)))
            }
        }
        if AblageBytes.equal(sammlung, AblageLedger.sammlungOrdner) {
            // Ordnergräber erst nach dem Datei-Pull anwenden: sonst hielte eine gleichzeitig begrabene Datei den Ordner.
            eingaenge.removeAll { e in
                if case .grab(let id) = e { zustand.ledger.ordnergraeber.insert(id); return true }
                return false
            }
        }
        if !eingaenge.isEmpty { einspielen(eingaenge, zustand: &zustand, lokal: lokal, bericht: &bericht) }
        for (k, o) in buch { zustand.ledger.objekte[k] = o }
        return bericht
    }

    /// Wendet die gemerkten Ordnergräber an (nach dem Pull beider Sammlungen) und leert die Merkliste.
    @discardableResult
    public static func ordnergraeber(zustand: inout AblageZustand, lokal: [String] = []) -> AblageSeitenBericht {
        var bericht = AblageSeitenBericht()
        let graeber = zustand.ledger.ordnergraeber.sortiert
        guard !graeber.isEmpty else { return bericht }
        einspielen(graeber.map { .grab($0) }, zustand: &zustand, lokal: lokal, bericht: &bericht)
        zustand.ledger.ordnergraeber.removeAll()
        return bericht
    }

    /// Öffnet ein Objekt (AAD mit `blob_refs`, `key_id`, `deleted`) und prüft Kennung, Schema und Steuerfeld gegen den
    /// Inhalt (`entschluesseln`). `nil`: unlesbar, das Objekt ruht. Gibt den Hash des Hub-Stands mit zurück.
    private static func entschluesseln(sammlung: String, id: String, refs: [String], payload: String, konto: AblageKonto,
                                       keyId: String) -> (Eingang, String)? {
        guard payload.utf8.count <= maxPayloadZeichen else { return nil }
        let aad = AblageKrypto.aadAblage(userId: konto.userId, collection: sammlung, objectId: id, hubBlobRefs: refs, keyId: keyId, deleted: false)
        guard let plain = try? SyncCrypto.open(key: konto.key, aad: aad, sealedBase64: payload) else { return nil }
        if AblageBytes.equal(sammlung, AblageLedger.sammlungOrdner) {
            // Schema (`v` > 1) und Felder prüft der Leser; danach Kennung und leere Refs.
            guard let o = try? AblageOrdner(nutzlast: plain), AblageBytes.equal(o.id, id), refs.isEmpty else { return nil }
            return (.ordner(o), o.hash())
        }
        guard let d = try? AblageDatei(nutzlast: plain), AblageBytes.equal(d.id, id) else { return nil }
        // `hub_refs(d) == blob_refs`: Listenvergleich in Reihenfolge (die Hub-Kennungen sind sortiert und eindeutig).
        let abgeleitet = AblageKrypto.hubRefs(datei: d, key: konto.key)
        guard abgeleitet.count == refs.count, zip(abgeleitet, refs).allSatisfy({ AblageBytes.equal($0, $1) }) else { return nil }
        return (.datei(d), d.hash(ordnen: false))
    }

    // MARK: Einspielen (`sync_einspielen`)

    private static func gleich(_ a: AblageOrdner?, _ b: AblageOrdner) -> Bool {
        a.map { $0.kanonischeNutzlast() == b.kanonischeNutzlast() } ?? false
    }

    private static func gleich(_ a: AblageDatei?, _ b: AblageDatei) -> Bool {
        a.map { $0.kanonischeNutzlast(ordnen: false) == b.kanonischeNutzlast(ordnen: false) } ?? false
    }

    private static func einspielen(_ eingaenge: [Eingang], zustand z: inout AblageZustand, lokal: [String], bericht: inout AblageSeitenBericht) {
        let lokalMenge = AblageMenge(lokal)
        // Teilnahme vor dem Einspielen: ein Grabstein gilt nur für synchrone Einträge.
        let hatGrab = eingaenge.contains { if case .grab = $0 { return true } else { return false } }
        let teilnahmeVorher = hatGrab ? AblageTeilnahme.alle(ordner: z.bestand.ordner, dateien: z.bestand.dateien, lokal: lokal) : AblageKarte<AblageTeilnahme>()
        var graeberDat = AblageMenge(), graeberOrd = AblageMenge()
        for e in eingaenge {
            switch e {
            case .ordner(let o):
                guard AblageValidierung.ordnerGueltig(o) else { bericht.verworfen.append(o.id); bericht.warne(.verworfen); continue }
                if z.ledger.begraben.contains(o.id) { continue }
                let alt = z.bestand.ordner[o.id]
                let neu = alt.map { AblageZusammenfuehren.zusammenfuehrenOrdner($0, o) } ?? o
                if !gleich(alt, neu) {
                    z.bestand.ordner[o.id] = neu
                    bericht.geaendert = true
                }
            case .datei(let d):
                guard AblageValidierung.dateiGueltig(d) else { bericht.verworfen.append(d.id); bericht.warne(.verworfen); continue }
                if z.ledger.begraben.contains(d.id) { continue }
                let alt = z.bestand.dateien[d.id]
                var neu = alt.map { AblageZusammenfuehren.zusammenfuehrenDatei($0, d) } ?? AblageZusammenfuehren.zusammenfuehrenDatei(d, d)
                neu.blobRefs = AblageZusammenfuehren.blobRefsAus(neu.fassungen)
                if !gleich(alt, neu) {
                    z.bestand.dateien[d.id] = neu
                    bericht.geaendert = true
                }
            case .grab(let id):
                let gilt = !lokalMenge.contains(id) && (teilnahmeVorher[id].map { $0 == .sync } ?? true)
                if !gilt { bericht.gehalten.append(id) }
                else if AblageId.idGueltig(id, prefix: AblageId.praefixDatei) { graeberDat.insert(id) }
                else if AblageId.idGueltig(id, prefix: AblageId.praefixOrdner) { graeberOrd.insert(id) }
                else { bericht.verworfen.append(id); bericht.warne(.verworfen) }
            }
        }
        // Nur, was hier noch etwas hinterlässt oder noch nicht begraben ist, braucht eine Buchung.
        let datListe = graeberDat.sortiert.filter { z.bestand.dateien.enthaelt($0) || !z.ledger.begraben.contains($0) }
        let ordListe = graeberOrd.sortiert.filter { z.bestand.ordner.enthaelt($0) || !z.ledger.begraben.contains($0) }
        // Ein Ordnergrab, unter dem hier noch lebende, nicht begrabene Einträge liegen (z. B. eine Datei, die ein Gerät
        // vor dem Umschalten eines anderen hineinlegte), wird nicht angewendet: der Ordner bleibt als Behälter, seine
        // Einträge bleiben synchron und werden nie mitgelöscht.
        let datMenge = AblageMenge(datListe), ordMenge = AblageMenge(ordListe)
        let eltern = AblageZusammenfuehren.wirksameEltern(z.bestand.ordner)
        func bleibtUnter(_ o: String) -> Bool {
            z.bestand.dateien.values.contains { d in
                !datMenge.contains(d.id) && AblageZusammenfuehren.kette(eltern, d.ordner.wert).contains { AblageBytes.equal($0, o) }
            } || z.bestand.ordner.values.contains { k in
                !AblageBytes.equal(k.id, o) && !ordMenge.contains(k.id) && AblageZusammenfuehren.kette(eltern, k.id).contains { AblageBytes.equal($0, o) }
            }
        }
        var ordAnwenden: [String] = []
        for o in ordListe {
            if bleibtUnter(o) { bericht.gehalten.append(o) } else { ordAnwenden.append(o) }
        }
        for d in datListe {
            z.bestand.dateien[d] = nil
            z.ledger.begraben.insert(d)
            bericht.begraben.append(d)
        }
        for o in ordAnwenden {
            z.bestand.ordner[o] = nil
            z.ledger.begraben.insert(o)
            bericht.begraben.append(o)
        }
        if !datListe.isEmpty || !ordAnwenden.isEmpty { bericht.geaendert = true }
    }
}
#endif
