import Foundation

// Anzeigezustand je Eintrag und Index-Tor der Dokumentenablage (D5a-3, lva#441, Epic #395).
//
// Port von `apps/local-voice/src-tauri/src/sync/ablage.rs` (`zustaende`, `wartet_auf_text`), für das iPhone vereinfacht:
// D5a liest nur, also gibt es kein „ausstehend“ (das kommt mit dem Senden in D5b), und „wartet auf Text“ kennt keine
// erwartete `textrev` (die Extraktorversion gehört dem PC): ein Eintrag, der eine Textfassung haben müsste und keine
// brauchbare hier hat, wartet auf den PC. Reine Funktionen auf dem Zustand; Namen sind nur Felder der Übersicht,
// sie landen nie in Warnungen oder Protokollen (Spec §10).

public enum AblageSyncZustand: String, Equatable, Sendable {
    case synchronisiert
    /// Mehr als ein Kopf: nie per Zeit aufgelöst.
    case konflikt
    /// „Text wird am PC erstellt“.
    case wartetAufText = "wartet_auf_text"
    /// „Nur dieses Gerät“, `personal` oder `confidential`.
    case nurGeraet = "nur_geraet"
    /// Der Hub-Stand ist hier unlesbar, oder die Klasse ist unbekannt / die Kette unvollständig.
    case angehalten
}

public struct AblageOrdnerEintrag: Equatable, Sendable {
    public var id: String
    public var name: String
    public var eltern: AblageEltern
    public var klasse: AblageKlasse
    public var zustand: AblageSyncZustand
    public var imKorb: Bool
}

public struct AblageDateiEintrag: Equatable, Sendable {
    public var id: String
    public var name: String
    public var ordner: String
    public var klasse: AblageKlasse
    public var zustand: AblageSyncZustand
    public var imKorb: Bool
    /// Fassungs-Kennungen der offenen Köpfe (mehr als einer: Konflikt).
    public var koepfe: [String]
    public var aktuelleFassung: String?
    /// Endung der aktuellen Fassung.
    public var typ: String?
    public var groesse: UInt64?
    public var fassungen: Int
    /// Die aktuelle Fassung hat eine brauchbare Textfassung (`kein_text` oder eine geladene `ok`).
    public var textBereit: Bool
    public var originalVorhanden: Bool
}

public struct AblageUebersicht: Equatable, Sendable {
    public var ordner: [AblageOrdnerEintrag]
    public var dateien: [AblageDateiEintrag]
    /// Schlüssel (`<sammlung>/<id>`) der Objekte, deren Hub-Stand hier unlesbar ist (auch ohne lokalen Eintrag).
    public var angehalten: [String]
}

public enum AblageAnzeige {
    private static func ist(_ a: String, _ b: String) -> Bool { AblageBytes.equal(a, b) }

    // MARK: Zustand je Eintrag (`zustaende`)

    public static func zustaende(_ z: AblageZustand, vorhanden: AblageVorhanden)
        -> (ordner: AblageKarte<AblageSyncZustand>, dateien: AblageKarte<AblageSyncZustand>) {
        let t = AblageTeilnahme.alle(ordner: z.bestand.ordner, dateien: z.bestand.dateien)
        var ordner = AblageKarte<AblageSyncZustand>(), dateien = AblageKarte<AblageSyncZustand>()
        for o in z.bestand.ordner.values {
            ordner[o.id] = ordnerZustand(z, o, t[o.id])
        }
        for d in z.bestand.dateien.values {
            dateien[d.id] = dateiZustand(z, d, t[d.id], vorhanden)
        }
        return (ordner, dateien)
    }

    private static func ordnerZustand(_ z: AblageZustand, _ o: AblageOrdner, _ t: AblageTeilnahme?) -> AblageSyncZustand {
        if z.ledger.angehalten(AblageLedger.sammlungOrdner, o.id) { return .angehalten }
        switch t {
        case .sync?: return .synchronisiert
        case .halten?: return .angehalten
        default: return .nurGeraet
        }
    }

    private static func dateiZustand(_ z: AblageZustand, _ d: AblageDatei, _ t: AblageTeilnahme?, _ v: AblageVorhanden) -> AblageSyncZustand {
        if z.ledger.angehalten(AblageLedger.sammlungDatei, d.id) { return .angehalten }
        switch t {
        case .sync?:
            if AblageZusammenfuehren.konflikt(d) { return .konflikt }
            if wartetAufText(d, vorhanden: v) { return .wartetAufText }
            return .synchronisiert
        case .halten?: return .angehalten
        default: return .nurGeraet
        }
    }

    // MARK: Wartet auf Text

    /// Hat die Fassung eine Textfassung, die hier brauchbar ist: `kein_text` (ein Ergebnis des PC) oder eine `ok`-Textfassung,
    /// die lokal liegt? Eine ältere Extraktorversion gilt, sobald sie hier liegt.
    static func textBereit(_ f: AblageFassung, dat: String, vorhanden: AblageVorhanden) -> Bool {
        f.texte.contains { ist($0.stand, "kein_text") || (ist($0.stand, "ok") && vorhanden.text(dat, $0.textrev)) }
    }

    /// „Text wird am PC erstellt“: die aktuelle Fassung (sonst die jüngste) hat ein Format mit Textfassung und hier keine
    /// brauchbare. Ein Fehler der Extraktion (nur `fehler`-Einträge) ist kein Warten (die Liste zeigt ihn). Ohne
    /// `texte` (eigener Import, der PC hat noch nicht gearbeitet) und mit einem eingetragenen, noch nicht geladenen
    /// Text wartet die Datei. Das iPhone extrahiert nie selbst (Spec §4.1).
    public static func wartetAufText(_ d: AblageDatei, vorhanden: AblageVorhanden) -> Bool {
        guard let f = AblageZusammenfuehren.aktuelleFassung(d), AblageFormat.hatTextfassung(typ: f.typ) else { return false }
        if textBereit(f, dat: d.id, vorhanden: vorhanden) { return false }
        if !f.texte.isEmpty && f.texte.allSatisfy({ ist($0.stand, "fehler") }) { return false }
        return true
    }

    // MARK: Index-Tor

    private struct Tor {
        let eltern: AblageKarte<AblageEltern>
        let teilnahme: AblageKarte<AblageTeilnahme>
    }

    private static func tor(_ z: AblageZustand) -> Tor {
        Tor(eltern: AblageZusammenfuehren.wirksameEltern(z.bestand.ordner),
            teilnahme: AblageTeilnahme.alle(ordner: z.bestand.ordner, dateien: z.bestand.dateien))
    }

    private static func offen(_ z: AblageZustand, _ tor: Tor, dat: String) -> Bool {
        guard let d = z.bestand.dateien[dat],
              !z.ledger.begraben.contains(dat),
              !z.ledger.angehalten(AblageLedger.sammlungDatei, dat),
              tor.teilnahme[dat] == .sync,
              AblageZusammenfuehren.dateiLebt(z.bestand.ordner, tor.eltern, d) else { return false }
        // Fail-closed: Ist der Hub-Stand eines Ordners der Kette unlesbar (auch ohne lokalen Eintrag), kennt dieses Gerät
        // dessen Klasse nicht verlässlich (der PC kann sie angehoben haben). Dann nichts aus diesem Baum indizieren.
        let kette = AblageZusammenfuehren.strengeKette(z.bestand.ordner, tor.eltern, d.ordner.wert)
        return !kette.contains { z.ledger.angehalten(AblageLedger.sammlungOrdner, $0) }
    }

    /// Darf diese Datei in den Index und an ein Modell? Nur lebend (kein Papierkorb), begraben nie, Teilnahme `sync`
    /// (nicht „nur Gerät“, nicht `personal`/`confidential`, nicht unbekannt), Hub-Stand lesbar (nie angehalten).
    public static func darfIndiziert(_ z: AblageZustand, dat: String) -> Bool { offen(z, tor(z), dat: dat) }

    /// Dateien (Kennungen, nach Bytes sortiert), die in den Index dürfen und eine brauchbare, geladene Textfassung haben
    /// (`ok` und hier vorhanden). Nichts aus Papierkorb, `Lokal`, `Halten`, angehalten oder begraben.
    public static func indexQuellen(_ z: AblageZustand, vorhanden: AblageVorhanden) -> [String] {
        let t = tor(z)
        var aus: [String] = []
        for d in z.bestand.dateien.values where offen(z, t, dat: d.id) {
            if d.fassungen.contains(where: { f in f.texte.contains { ist($0.stand, "ok") && !$0.blob.isEmpty && vorhanden.text(d.id, $0.textrev) } }) {
                aus.append(d.id)
            }
        }
        return AblageBytes.sorted(aus, by: { $0 })
    }

    // MARK: Übersicht

    /// Alles, was eine Liste braucht: Namen, Klassen, Zustand, Papierkorb, Köpfe. Nach Kennung (Bytes) sortiert.
    public static func uebersicht(_ z: AblageZustand, vorhanden: AblageVorhanden) -> AblageUebersicht {
        let eltern = AblageZusammenfuehren.wirksameEltern(z.bestand.ordner)
        let t = AblageTeilnahme.alle(ordner: z.bestand.ordner, dateien: z.bestand.dateien)
        // Klassen wie die Teilnahme: ein fehlender Elternordner zählt neutral (wie „Wiederhergestellt“).
        var voll = z.bestand.ordner
        var fehlend = AblageMenge()
        for d in z.bestand.dateien.values where !voll.enthaelt(d.ordner.wert) { fehlend.insert(d.ordner.wert) }
        for o in z.bestand.ordner.values { if let p = o.eltern.wert, !voll.enthaelt(p) { fehlend.insert(p) } }
        for id in fehlend.sortiert { voll[id] = AblageTeilnahme.neutralerOrdner(id) }
        let eVoll = AblageZusammenfuehren.wirksameEltern(voll)

        var ordner: [AblageOrdnerEintrag] = []
        for o in z.bestand.ordner.values {
            ordner.append(AblageOrdnerEintrag(
                id: o.id, name: o.name.wert, eltern: eltern[o.id] ?? .wiederhergestellt,
                klasse: AblageZusammenfuehren.wirksameKlasse(voll, eVoll, o.id), zustand: ordnerZustand(z, o, t[o.id]),
                imKorb: AblageZusammenfuehren.ordnerImKorb(z.bestand.ordner, eltern, o.id)))
        }
        var dateien: [AblageDateiEintrag] = []
        for d in z.bestand.dateien.values {
            let f = AblageZusammenfuehren.aktuelleFassung(d)
            dateien.append(AblageDateiEintrag(
                id: d.id, name: d.name.wert, ordner: d.ordner.wert,
                klasse: AblageZusammenfuehren.wirksameKlasseDatei(voll, eVoll, d), zustand: dateiZustand(z, d, t[d.id], vorhanden),
                imKorb: !AblageZusammenfuehren.dateiLebt(z.bestand.ordner, eltern, d),
                koepfe: AblageZusammenfuehren.koepfe(d), aktuelleFassung: f?.fas, typ: f?.typ, groesse: f?.size,
                fassungen: d.fassungen.count,
                textBereit: f.map { textBereit($0, dat: d.id, vorhanden: vorhanden) } ?? false,
                originalVorhanden: f.map { vorhanden.original(d.id, $0.fas) } ?? false))
        }
        return AblageUebersicht(ordner: AblageBytes.sorted(ordner, by: \.id), dateien: AblageBytes.sorted(dateien, by: \.id),
                                angehalten: z.ledger.angehalteneKennungen)
    }
}
