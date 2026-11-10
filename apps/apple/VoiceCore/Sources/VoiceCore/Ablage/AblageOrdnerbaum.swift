import Foundation

// Ordnerbaum, Klasse, Papierkorb und Teilnahme der Dokumentenablage (D5a-2, lva#441, Epic #395).
//
// Port von `apps/local-voice/src-tauri/src/ablage/zusammenfuehren.rs` (`Eltern`, `wirksame_eltern`, `kette`,
// `zusatzkette`, `strenge_kette`, `Klasse`, `wirksame_klasse`, `wirksame_klasse_datei`, `wirksam_nur_geraet`,
// `groesster_zaehler`), `speicher.rs` (`ordner_im_korb`, `datei_lebt`) und `sync/ablage.rs` (`Teilnahme`,
// `teilnahme_ordner`, `teilnahme_datei`, `teilnahmen`). Reine Funktionen ohne I/O und ohne Logging.
//
// Die Register werden nie verändert: die wirksame Elternzuordnung wird je Stand abgeleitet. Alle Kennungen werden
// bytegenau (UTF-8) verglichen, Ordner und Dateien liegen in `AblageKarte` (nie `Dictionary<String, _>`).

/// Ordnerbestand, geschlüsselt nach Ordner-Kennung (`BTreeMap<String, Ordner>`).
public typealias AblageOrdnerbestand = AblageKarte<AblageOrdner>
/// Dateibestand, geschlüsselt nach Datei-Kennung (`BTreeMap<String, Datei>`).
public typealias AblageDateibestand = AblageKarte<AblageDatei>

// MARK: - Eltern

/// Wirksame Elternzuordnung eines Ordners (`Eltern`).
public enum AblageEltern: Equatable, Sendable {
    case wurzel
    case ordner(String)
    /// Zyklus oder unbekannter Elternordner: vorläufig unter „Wiederhergestellt“.
    case wiederhergestellt

    public static func == (a: AblageEltern, b: AblageEltern) -> Bool {
        switch (a, b) {
        case (.wurzel, .wurzel), (.wiederhergestellt, .wiederhergestellt): return true
        case (.ordner(let x), .ordner(let y)): return AblageBytes.equal(x, y)
        default: return false
        }
    }
}

// MARK: - Klasse

/// Datenklasse eines Ordners oder einer Datei (`Klasse`). Die Reihenfolge ist die der Strenge: `max` ist die
/// strengere Klasse.
public enum AblageKlasse: Int, Comparable, Sendable {
    case `public`
    case `internal`
    case confidential
    case personal
    /// Unbekannter Wert nach dem Zusammenführen: wie gesperrt, bis der Nutzer ihn setzt.
    case unbekannt

    public static func < (a: AblageKlasse, b: AblageKlasse) -> Bool { a.rawValue < b.rawValue }

    /// Klasse, wenn nichts anderes gesetzt ist (`Klasse::VORGABE`).
    public static let vorgabe = AblageKlasse.internal

    /// Bekannter Klassenwert; `erbt` und alles Unbekannte ergibt `nil` (`Klasse::aus_text`).
    public init?(text: String) {
        if AblageBytes.equal(text, "public") { self = .public }
        else if AblageBytes.equal(text, "internal") { self = .internal }
        else if AblageBytes.equal(text, "confidential") { self = .confidential }
        else if AblageBytes.equal(text, "personal") { self = .personal }
        else { return nil }
    }

    public var text: String {
        switch self {
        case .public: return "public"
        case .internal: return "internal"
        case .confidential: return "confidential"
        case .personal: return "personal"
        case .unbekannt: return "unbekannt"
        }
    }
}

// MARK: - Teilnahme

/// Ob ein Eintrag am Abgleich teilnimmt (`Teilnahme`).
public enum AblageTeilnahme: Equatable, Sendable {
    case sync
    /// „Nur dieses Gerät“, `personal` oder `confidential`: verlässt das Gerät nie.
    case lokal
    /// Unbekannte Klasse oder unvollständige Kette: weder senden noch begraben, nicht indizieren.
    case halten
}

// MARK: - Ordnerbaum

extension AblageZusammenfuehren {
    /// Wirksame Elternzuordnung (`wirksame_eltern`, die Register bleiben unverändert): Ordner absteigend nach
    /// dem Rang ihres `eltern`-Registers (bei Gleichstand nach Kennung aufsteigend, bytegenau); eine Kante gilt nur,
    /// wenn sie mit den bereits übernommenen keinen Zyklus bildet. Fehlender Elternordner (auch ein begrabener,
    /// den der Aufrufer nicht mehr im Bestand führt) und Zyklus ergeben `.wiederhergestellt`.
    public static func wirksameEltern(_ ordner: AblageOrdnerbestand) -> AblageKarte<AblageEltern> {
        let reihe = ordner.values.sorted { a, b in
            let c = b.eltern.rangVergleich(a.eltern)
            return c != 0 ? c < 0 : AblageBytes.less(a.id, b.id)
        }
        var aus = AblageKarte<AblageEltern>()
        for o in reihe {
            let e: AblageEltern
            if let p = o.eltern.wert {
                if !ordner.enthaelt(p) { e = .wiederhergestellt }
                else if fuehrtZu(aus, von: p, ziel: o.id) { e = .wiederhergestellt }
                else { e = .ordner(p) }
            } else {
                e = .wurzel
            }
            aus[o.id] = e
        }
        return aus
    }

    /// Führt die Kette der übernommenen Kanten von `von` aufwärts zu `ziel` (oder ist `von == ziel`)? Die
    /// Schrittzahl ist begrenzt (`len + 2`); wird sie erreicht, gilt das als Zyklus.
    private static func fuehrtZu(_ angenommen: AblageKarte<AblageEltern>, von: String, ziel: String) -> Bool {
        var aktuell = von
        for _ in 0...(angenommen.count + 1) {
            if AblageBytes.equal(aktuell, ziel) { return true }
            guard case .ordner(let p)? = angenommen[aktuell] else { return false }
            aktuell = p
        }
        return true
    }

    /// Kette eines Ordners von sich selbst bis zur Wurzel nach der wirksamen Zuordnung (`kette`).
    public static func kette(_ eltern: AblageKarte<AblageEltern>, _ id: String) -> [String] {
        var aus = [id]
        var aktuell = id
        while case .ordner(let p)? = eltern[aktuell] {
            if enthaelt(aus, p) { break }
            aus.append(p)
            aktuell = p
        }
        return aus
    }

    /// Zusätzliche Vorfahren nur für Einschränkungen (`zusatzkette`): Liegt ein Ordner wirksam unter
    /// „Wiederhergestellt“, bleibt die gespeicherte Elternkette für Sperre, Klasse und „nur dieses Gerät“
    /// maßgeblich (streng: Vereinigung beider Ketten, die strengste Regel gewinnt).
    public static func zusatzkette(_ ordner: AblageOrdnerbestand, _ eltern: AblageKarte<AblageEltern>, _ id: String) -> [String] {
        let wirksam = kette(eltern, id)
        var zusatz: [String] = []
        guard let letzter = wirksam.last, case .wiederhergestellt? = eltern[letzter] else { return zusatz }
        var aktuell = ordner[letzter]?.eltern.wert
        while let p = aktuell {
            if enthaelt(wirksam, p) || enthaelt(zusatz, p) { break }
            aktuell = ordner[p]?.eltern.wert
            zusatz.append(p)
        }
        return zusatz
    }

    /// Wirksame Kette plus `zusatzkette` (`strenge_kette`).
    public static func strengeKette(_ ordner: AblageOrdnerbestand, _ eltern: AblageKarte<AblageEltern>, _ id: String) -> [String] {
        kette(eltern, id) + zusatzkette(ordner, eltern, id)
    }

    /// Wirksame Klasse (`wirksame_klasse`): von der Wurzel abwärts; `erbt` übernimmt, ein bekannter Wert nur
    /// gleich oder höher (eine niedrigere gespeicherte Klasse wird auf die geerbte angehoben, nie umgekehrt),
    /// ein unbekannter Wert oder ein fehlender Ordner in der Kette macht das Ergebnis `.unbekannt`. Vorgabe `internal`.
    public static func wirksameKlasse(_ ordner: AblageOrdnerbestand, _ eltern: AblageKarte<AblageEltern>, _ id: String) -> AblageKlasse {
        var geerbt = AblageKlasse.vorgabe
        for glied in strengeKette(ordner, eltern, id).reversed() {
            guard let o = ordner[glied] else { return .unbekannt }
            let wert = o.datenklasse.wert
            if AblageBytes.equal(wert, "erbt") { continue }
            guard let k = AblageKlasse(text: wert) else { return .unbekannt }
            geerbt = max(geerbt, k)
        }
        return geerbt
    }

    /// Klasse einer Datei (`wirksame_klasse_datei`): Maximum aus der Klasse ihres Ordners und der eigenen
    /// Mindestklasse; eine unbekannte Mindestklasse ergibt `.unbekannt`, `erbt` übernimmt die Ordnerklasse.
    public static func wirksameKlasseDatei(_ ordner: AblageOrdnerbestand, _ eltern: AblageKarte<AblageEltern>, _ d: AblageDatei) -> AblageKlasse {
        let ordnerklasse = wirksameKlasse(ordner, eltern, d.ordner.wert)
        if ordnerklasse == .unbekannt { return .unbekannt }
        if let k = AblageKlasse(text: d.klasseMin.wert) { return max(ordnerklasse, k) }
        return AblageBytes.equal(d.klasseMin.wert, "erbt") ? ordnerklasse : .unbekannt
    }

    /// „Nur dieses Gerät“ ist eine Unterbaumregel (`wirksam_nur_geraet`): wirksam, sobald der Ordner oder ein
    /// Vorfahr (strenge Kette) es trägt.
    public static func wirksamNurGeraet(_ ordner: AblageOrdnerbestand, _ eltern: AblageKarte<AblageEltern>, _ id: String) -> Bool {
        strengeKette(ordner, eltern, id).contains { g in ordner[g]?.nurGeraet.wert ?? false }
    }

    /// Größter bekannter Zähler über alle Register (`groesster_zaehler`): alle sechs Ordner-Register, `ordner`,
    /// `name`, `klasse_min` und `papierkorb` jeder Datei und `wahlen[].zaehler`. Nicht gezählt: `nur_geraet_min`
    /// und die Zeit der Fassungen. Der nächste Schreibvorgang nimmt diesen Wert + 1; bei `UInt64.max` gibt es
    /// keinen nächsten Zähler (der Aufrufer muss das prüfen, siehe `naechsterZaehler`).
    public static func groessterZaehler(_ ordner: AblageOrdnerbestand, _ dateien: AblageDateibestand) -> UInt64 {
        var m: UInt64 = 0
        for o in ordner.values {
            m = max(m, o.eltern.zaehler, o.name.zaehler, o.datenklasse.zaehler, o.nurGeraet.zaehler, o.zielordnerKi.zaehler, o.papierkorb.zaehler)
        }
        for d in dateien.values {
            m = max(m, d.ordner.zaehler, d.name.zaehler, d.klasseMin.zaehler, d.papierkorb.zaehler)
            for w in d.wahlen { m = max(m, w.zaehler) }
        }
        return m
    }

    /// Nächster Lamport-Zähler (`zaehler`): größter bekannter Wert + 1; `nil`, wenn dafür `UInt64.max` überschritten
    /// würde (Rust rechnet dort ungeprüft; ein Umlauf auf 0 ließe jeden späteren Stand verlieren).
    public static func naechsterZaehler(_ ordner: AblageOrdnerbestand, _ dateien: AblageDateibestand) -> UInt64? {
        let g = groessterZaehler(ordner, dateien)
        return g == UInt64.max ? nil : g + 1
    }

    // MARK: Papierkorb

    /// Liegt der Ordner oder ein Vorfahr der wirksamen Kette im Papierkorb (`ordner_im_korb`)? Ein Glied, das im
    /// Bestand fehlt, zählt als im Korb.
    public static func ordnerImKorb(_ ordner: AblageOrdnerbestand, _ eltern: AblageKarte<AblageEltern>, _ id: String) -> Bool {
        kette(eltern, id).contains { g in ordner[g].map { $0.papierkorb.wert != nil } ?? true }
    }

    /// Lebt die Datei (`datei_lebt`)? Eigener Papierkorb leer und kein Ordner der wirksamen Kette im Korb. Was nicht
    /// lebt, wird weder angezeigt noch indiziert.
    public static func dateiLebt(_ ordner: AblageOrdnerbestand, _ eltern: AblageKarte<AblageEltern>, _ d: AblageDatei) -> Bool {
        d.papierkorb.wert == nil && !ordnerImKorb(ordner, eltern, d.ordner.wert)
    }
}

// MARK: - Teilnahme am Abgleich

extension AblageTeilnahme {
    private static func ausKlasse(_ k: AblageKlasse) -> AblageTeilnahme {
        switch k {
        case .public, .internal: return .sync
        case .confidential, .personal: return .lokal
        case .unbekannt: return .halten
        }
    }

    /// Teilnahme eines Ordners (`teilnahme_ordner`): „nur dieses Gerät“ im Unterbaum ergibt `.lokal`, sonst
    /// entscheidet die wirksame Klasse.
    public static func ordner(_ ordner: AblageOrdnerbestand, _ eltern: AblageKarte<AblageEltern>, _ id: String) -> AblageTeilnahme {
        if AblageZusammenfuehren.wirksamNurGeraet(ordner, eltern, id) { return .lokal }
        return ausKlasse(AblageZusammenfuehren.wirksameKlasse(ordner, eltern, id))
    }

    /// Teilnahme einer Datei (`teilnahme_datei`): `nur_geraet_min` ergibt `.lokal`; sonst zählt zuerst der Ordner,
    /// bei `.sync` die Klasse der Datei.
    public static func datei(_ ordner: AblageOrdnerbestand, _ eltern: AblageKarte<AblageEltern>, _ d: AblageDatei) -> AblageTeilnahme {
        if d.nurGeraetMin { return .lokal }
        switch Self.ordner(ordner, eltern, d.ordner.wert) {
        case .sync: return ausKlasse(AblageZusammenfuehren.wirksameKlasseDatei(ordner, eltern, d))
        case let t: return t
        }
    }

    /// Ordner ohne eigene Regeln (`neutraler_ordner`): für fehlende Elternordner und, aus Sicht befreiter Einträge,
    /// festgehaltene Ordner; verhält sich wie „Wiederhergestellt“.
    static func neutralerOrdner(_ id: String) -> AblageOrdner {
        AblageOrdner(id: id,
                     eltern: AblageRegister(wert: nil, zaehler: 0, geraet: ""),
                     name: AblageRegister(wert: "", zaehler: 0, geraet: ""),
                     datenklasse: AblageRegister(wert: "erbt", zaehler: 0, geraet: ""),
                     nurGeraet: AblageRegister(wert: false, zaehler: 0, geraet: ""),
                     zielordnerKi: AblageRegister(wert: false, zaehler: 0, geraet: ""),
                     papierkorb: AblageRegister(wert: nil, zaehler: 0, geraet: ""))
    }

    /// Teilnahme je Kennung, Ordner und Dateien (`teilnahmen`), maßgeblich für Senden, Grabsteine und Index:
    /// - festgehalten (`lokal`) ergibt `.lokal`, für immer;
    /// - ein fehlender Elternordner zählt neutral (wie „Wiederhergestellt“), nicht als unbekannt;
    /// - befreite Einträge sehen festgehaltene Ordner ebenfalls neutral (sie bleiben synchron);
    /// - sonst Klasse und „nur dieses Gerät“ entlang der strengen Kette.
    /// Das iPhone führt weder `lokal` noch `befreit` selbst; beide Listen bleiben leer, solange der Desktop
    /// diese Zustände nicht überträgt (sie sind gerätelokal).
    public static func alle(ordner: AblageOrdnerbestand, dateien: AblageDateibestand,
                            lokal: [String] = [], befreit: [String] = []) -> AblageKarte<AblageTeilnahme> {
        var voll = ordner
        var fehlend = AblageMenge()
        for d in dateien.values where !ordner.enthaelt(d.ordner.wert) { fehlend.insert(d.ordner.wert) }
        for o in ordner.values { if let p = o.eltern.wert, !ordner.enthaelt(p) { fehlend.insert(p) } }
        for id in fehlend.sortiert { voll[id] = neutralerOrdner(id) }
        let eVoll = AblageZusammenfuehren.wirksameEltern(voll)
        let lokalMenge = AblageMenge(lokal), befreitMenge = AblageMenge(befreit)

        var frei: (AblageOrdnerbestand, AblageKarte<AblageEltern>)?
        if befreitMenge.count > 0 {
            var f = voll
            for id in lokalMenge.sortiert where f.enthaelt(id) { f[id] = neutralerOrdner(id) }
            frei = (f, AblageZusammenfuehren.wirksameEltern(f))
        }
        func sicht(_ id: String) -> (AblageOrdnerbestand, AblageKarte<AblageEltern>) {
            if let frei, befreitMenge.contains(id) { return frei }
            return (voll, eVoll)
        }

        var aus = AblageKarte<AblageTeilnahme>()
        for id in ordner.keys {
            if lokalMenge.contains(id) { aus[id] = .lokal; continue }
            let (o, e) = sicht(id)
            aus[id] = Self.ordner(o, e, id)
        }
        for d in dateien.values {
            if lokalMenge.contains(d.id) { aus[d.id] = .lokal; continue }
            let (o, e) = sicht(d.id)
            aus[d.id] = Self.datei(o, e, d)
        }
        return aus
    }
}
