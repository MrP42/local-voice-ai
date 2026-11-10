import Foundation

// Darstellung der Ablage für die Seite „Ablage“ (D5a-4, lva#441, Epic #395): flache Zeilen des Ordnerbaums, Detail einer Datei,
// lesbare Anker, Kontingent. Reine Funktionen auf der Übersicht bzw. dem Zustand; die Oberfläche (SwiftUI) zeichnet nur noch.
// Namen sind Felder dieser Werte, sie landen nie in Protokollen (Spec §10).

// MARK: - Ordnerbaum

public enum AblageBaumZeile: Equatable, Sendable, Identifiable {
    /// `tiefe` ab 0; `anzahl`: lebende Dateien im Ordner und darunter; `offen`: Inhalt sichtbar.
    case ordner(id: String, name: String, tiefe: Int, klasse: AblageKlasse, zustand: AblageSyncZustand, anzahl: Int, offen: Bool)
    case datei(AblageDateiEintrag, tiefe: Int)

    public var id: String {
        switch self {
        case .ordner(let id, _, _, _, _, _, _): return "o:" + id
        case .datei(let d, _): return "d:" + d.id
        }
    }
}

public enum AblageBaum {
    /// Platzhalter-Kennung für Dateien, deren Ordner dieses Gerät nicht kennt.
    public static let ohneOrdner = ""

    private static func name(_ a: String, _ b: String) -> Bool { a.localizedStandardCompare(b) == .orderedAscending }

    /// Flache Zeilen des Baums: Ordner (nach Name), darin zuerst Unterordner, dann Dateien (nach Name). Nur Lebendes (kein Papierkorb).
    /// Ein eingeklappter Ordner zeigt seinen Inhalt nicht. Ordner, deren Eltern unbekannt sind (Zyklus, fehlend), stehen an der Wurzel;
    /// Dateien ohne bekannten Ordner unter „Ohne Ordner“ (leere Kennung).
    public static func zeilen(_ u: AblageUebersicht, eingeklappt: Set<String> = []) -> [AblageBaumZeile] {
        let lebend = u.ordner.filter { !$0.imKorb }
        let ids = AblageMenge(lebend.map(\.id))
        var kinder: [String: [AblageOrdnerEintrag]] = [:]
        var wurzeln: [AblageOrdnerEintrag] = []
        for o in lebend {
            if case .ordner(let p) = o.eltern, ids.contains(p) { kinder[p, default: []].append(o) } else { wurzeln.append(o) }
        }
        var dateien: [String: [AblageDateiEintrag]] = [:]
        var verwaist: [AblageDateiEintrag] = []
        for d in u.dateien where !d.imKorb {
            if ids.contains(d.ordner) { dateien[d.ordner, default: []].append(d) } else { verwaist.append(d) }
        }
        func anzahl(_ o: AblageOrdnerEintrag, _ besucht: inout Set<String>) -> Int {
            guard besucht.insert(o.id).inserted else { return 0 }
            return (dateien[o.id]?.count ?? 0) + (kinder[o.id] ?? []).reduce(0) { $0 + anzahl($1, &besucht) }
        }
        var aus: [AblageBaumZeile] = []
        var besuchtGesamt = Set<String>()
        func ordner(_ o: AblageOrdnerEintrag, _ tiefe: Int) {
            guard besuchtGesamt.insert(o.id).inserted else { return }
            var b = Set<String>()
            let offen = !eingeklappt.contains(o.id)
            aus.append(.ordner(id: o.id, name: o.name, tiefe: tiefe, klasse: o.klasse, zustand: o.zustand, anzahl: anzahl(o, &b), offen: offen))
            guard offen else { return }
            for k in (kinder[o.id] ?? []).sorted(by: { name($0.name, $1.name) }) { ordner(k, tiefe + 1) }
            for d in (dateien[o.id] ?? []).sorted(by: { name($0.name, $1.name) }) { aus.append(.datei(d, tiefe: tiefe + 1)) }
        }
        for w in wurzeln.sorted(by: { name($0.name, $1.name) }) { ordner(w, 0) }
        if !verwaist.isEmpty {
            let offen = !eingeklappt.contains(ohneOrdner)
            aus.append(.ordner(id: ohneOrdner, name: "Ohne Ordner", tiefe: 0, klasse: .unbekannt, zustand: .angehalten, anzahl: verwaist.count, offen: offen))
            if offen { for d in verwaist.sorted(by: { name($0.name, $1.name) }) { aus.append(.datei(d, tiefe: 1)) } }
        }
        return aus
    }

    /// Dateien im Papierkorb (die Datei selbst oder ein Ordner darüber), nach Name.
    public static func papierkorb(_ u: AblageUebersicht) -> [AblageDateiEintrag] {
        u.dateien.filter(\.imKorb).sorted { name($0.name, $1.name) }
    }

    /// Namen der Ordner von der Wurzel bis zu `ordner` (leer bei unbekanntem Ordner).
    public static func ordnerPfad(_ u: AblageUebersicht, ordner id: String) -> [String] {
        let je = Dictionary(u.ordner.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        var pfad: [String] = []
        var aktuell: String? = id
        var besucht = Set<String>()
        while let a = aktuell, let o = je[a], besucht.insert(a).inserted {
            pfad.append(o.name)
            if case .ordner(let p) = o.eltern { aktuell = p } else { aktuell = nil }
        }
        return pfad.reversed()
    }
}

// MARK: - Anker

/// Anker des Markenformats v1 (Spec §4.2) in lesbarer Form: `seite=3` → „Seite 3“.
public enum AblageAnker {
    public static func beschreibung(_ anker: String) -> String {
        if let (schluessel, wert) = zerlegen(anker) {
            switch schluessel {
            case "seite": return "Seite \(wert)"
            case "absatz": return "Absatz \(wert)"
            case "folie": return "Folie \(wert)"
            case "zeile": return "Zeile \(wert)"
            case "teil":
                switch wert {
                case "kopf": return "Kopfzeilen"
                case "text": return "Text"
                default: return "Teil \(wert)"
                }
            case "blatt":
                if let r = wert.range(of: "&zeilen=", options: .backwards) {
                    return "Blatt „\(wert[..<r.lowerBound])“, Zeilen \(wert[r.upperBound...].replacingOccurrences(of: "-", with: "–"))"
                }
                return "Blatt „\(wert)“"
            default: break
            }
        }
        return anker
    }

    private static func zerlegen(_ anker: String) -> (String, String)? {
        guard let i = anker.firstIndex(of: "=") else { return nil }
        return (String(anker[..<i]), String(anker[anker.index(after: i)...]))
    }
}

// MARK: - Detail einer Datei

public enum AblageTextStand: Equatable, Sendable {
    /// Text vom PC liegt hier.
    case bereit
    /// Der PC hat die Fassung geprüft: kein Text (z. B. Bild ohne Schrift).
    case keinText
    /// Die Extraktion am PC ist gescheitert (das iPhone wiederholt sie nie).
    case fehler
    /// Der Text ist eingetragen, aber noch nicht geladen.
    case nichtGeladen
    /// „Text wird am PC erstellt“.
    case wartetAufPC
    /// Format ohne Textfassung (z. B. PNG).
    case keinFormat
}

public struct AblageFassungSicht: Equatable, Sendable {
    public var fas: String
    public var zeit: Int64
    public var typ: String
    public var groesse: UInt64
    /// Offene Fassung ohne Nachfolger: bei mehr als einer liegt ein Konflikt vor.
    public var istKopf: Bool
    /// Die Fassung, die durchsucht und vorgeschaut wird (einziger Kopf, sonst die jüngste).
    public var istAktuell: Bool
    public var text: AblageTextStand
    public var originalVorhanden: Bool
}

public struct AblageDetail: Equatable, Sendable {
    public var dat: String
    public var name: String
    public var ordnerPfad: [String]
    public var klasse: AblageKlasse
    public var zustand: AblageSyncZustand
    public var imKorb: Bool
    /// Neueste zuerst.
    public var fassungen: [AblageFassungSicht]
    public var konflikt: Bool
    /// Konflikt ohne gemeinsame Vorgängerfassung: ohne Wahl am PC nicht auflösbar.
    public var konfliktOhneGemeinsame: Bool
}

extension AblageAnzeige {
    static func textStand(_ f: AblageFassung, dat: String, vorhanden: AblageVorhanden) -> AblageTextStand {
        guard AblageFormat.hatTextfassung(typ: f.typ) else { return .keinFormat }
        if f.texte.contains(where: { AblageBytes.equal($0.stand, "ok") && vorhanden.text(dat, $0.textrev) }) { return .bereit }
        if f.texte.contains(where: { AblageBytes.equal($0.stand, "kein_text") }) { return .keinText }
        if f.texte.contains(where: { AblageBytes.equal($0.stand, "ok") }) { return .nichtGeladen }
        if !f.texte.isEmpty { return .fehler }
        return .wartetAufPC
    }

    /// Alles, was die Detailseite einer Datei zeigt; `nil`, wenn es die Datei nicht gibt.
    public static func detail(_ z: AblageZustand, dat: String, vorhanden: AblageVorhanden) -> AblageDetail? {
        guard let d = z.bestand.dateien[dat] else { return nil }
        let u = uebersicht(z, vorhanden: vorhanden)
        guard let e = u.dateien.first(where: { AblageBytes.equal($0.id, dat) }) else { return nil }
        let koepfe = AblageMenge(AblageZusammenfuehren.koepfe(d))
        let fassungen = d.fassungen.sorted { a, b in
            a.zeit != b.zeit ? a.zeit > b.zeit : AblageBytes.less(b.fas, a.fas)
        }.map { f in
            AblageFassungSicht(fas: f.fas, zeit: f.zeit, typ: f.typ, groesse: f.size, istKopf: koepfe.contains(f.fas),
                               istAktuell: e.aktuelleFassung.map { AblageBytes.equal($0, f.fas) } ?? false,
                               text: textStand(f, dat: dat, vorhanden: vorhanden), originalVorhanden: vorhanden.original(dat, f.fas))
        }
        let konflikt = AblageZusammenfuehren.konflikt(d)
        return AblageDetail(dat: dat, name: e.name, ordnerPfad: AblageBaum.ordnerPfad(u, ordner: e.ordner), klasse: e.klasse, zustand: e.zustand,
                            imKorb: e.imKorb, fassungen: fassungen, konflikt: konflikt,
                            konfliktOhneGemeinsame: konflikt && AblageZusammenfuehren.gemeinsameVorgaengerin(d) == nil)
    }
}

// MARK: - Kontingent

/// Verbrauch und Kontingent der Dateiablage am Hub (`GET /api/v1/voice/blobs` → `{used, quota, blobs}`). Nur zur Anzeige im Fuß der
/// Seite; ohne Antwort oder ohne Kontingent (`quota == 0`) fehlt die Zeile.
public struct AblageKontingent: Equatable, Sendable {
    public var belegt: UInt64
    public var kontingent: UInt64

    public init(belegt: UInt64, kontingent: UInt64) { self.belegt = belegt; self.kontingent = kontingent }

    /// Liest die Antwort; `nil` bei unlesbarer Antwort oder `quota == 0` (kein Kontingent bekannt).
    public init?(antwort: Data) {
        guard let j = (try? JSONSerialization.jsonObject(with: antwort)) as? [String: Any],
              let q = (j["quota"] as? NSNumber)?.uint64Value, q > 0 else { return nil }
        self.init(belegt: (j["used"] as? NSNumber)?.uint64Value ?? 0, kontingent: q)
    }

    /// Voll wie `kontingent_voll` am Desktop (`quota > 0 && used >= quota`).
    public var voll: Bool { kontingent > 0 && belegt >= kontingent }

    public var anteil: Double { kontingent == 0 ? 0 : min(1, Double(belegt) / Double(kontingent)) }

    /// „1,2 MB von 32 GB belegt“ (deutsche Zahlenschreibweise, 1000er-Einheiten wie die Systemanzeige).
    public var text: String {
        let f = ByteCountFormatter()
        f.countStyle = .file
        f.zeroPadsFractionDigits = false
        let basis = "\(f.string(fromByteCount: Int64(clamping: belegt))) von \(f.string(fromByteCount: Int64(clamping: kontingent))) belegt"
        return voll ? basis + " (voll)" : basis
    }
}
