import Foundation

// Zusammenführen, Köpfe und Konflikt der Dokumentenablage (D5a-2, lva#441, Epic #395).
//
// Port von `apps/local-voice/src-tauri/src/ablage/zusammenfuehren.rs` (Register, `zusammenfuehren_datei`,
// `zusammenfuehren_ordner`, `extra_vereinigen`, `fassung_vereinigen`, `aktive_wahlen`, `abgeloest`, `koepfe`,
// `aktuell`, `gemeinsame_vorgaengerin`, `neue_wahl`) und `speicher.rs` (`aktuelle_fassung`). Reine Funktionen:
// kein I/O, keine Uhr, kein Zustand, kein Logging. Die Eingaben bleiben unverändert (Werttypen).
//
// Jede Ordnung und jeder Vergleich, der das Ergebnis bestimmt, läuft über UTF-8-Bytes (`AblageBytes`,
// `AblageOrdnung`, `AblageKarte`), nie über Swift-`String <` oder -`==`: "é" und "e\u{301}" sind zwei Kennungen.
//
// Das Ergebnis hängt nicht von der Reihenfolge der Zusammenführung ab (kommutativ, assoziativ, idempotent);
// die Tests führen dieselben Permutationen aus wie der Desktop und lesen die Zusatzvektoren der gemeinsamen
// Fixture (`zusatz.zusammenfuehren`).

// MARK: - Ordnung der Register-Werte

/// Werttypen von Registern mit Totalordnung wie Rusts `Ord` (Zeichenketten nach UTF-8-Bytes, `false` < `true`,
/// `None` < `Some`). Nur der Gleichstand von `(zaehler, geraet)` fragt den Wert.
public protocol AblageGeordnet: AblageWert {
    /// -1, 0 oder 1.
    static func ablageVergleich(_ a: Self, _ b: Self) -> Int
}

extension String: AblageGeordnet {
    public static func ablageVergleich(_ a: String, _ b: String) -> Int { AblageBytes.compare(a, b) }
}

extension Bool: AblageGeordnet {
    public static func ablageVergleich(_ a: Bool, _ b: Bool) -> Int { a == b ? 0 : (b ? -1 : 1) }
}

extension Int64: AblageGeordnet {
    public static func ablageVergleich(_ a: Int64, _ b: Int64) -> Int { a == b ? 0 : (a < b ? -1 : 1) }
}

extension Optional: AblageGeordnet where Wrapped: AblageGeordnet {
    public static func ablageVergleich(_ a: Wrapped?, _ b: Wrapped?) -> Int {
        switch (a, b) {
        case (nil, nil): return 0
        case (nil, _): return -1
        case (_, nil): return 1
        case (let x?, let y?): return Wrapped.ablageVergleich(x, y)
        }
    }
}

extension AblageRegister {
    /// Rang `(zaehler, geraet)`: erst der Zähler, dann das Gerät nach Bytes. -1, 0 oder 1.
    public func rangVergleich(_ other: AblageRegister<T>) -> Int {
        zaehler != other.zaehler ? (zaehler < other.zaehler ? -1 : 1) : AblageBytes.compare(geraet, other.geraet)
    }
}

extension AblageRegister where T: AblageGeordnet {
    /// Ordnung des Desktops (`impl Ord for Register`): erst der Rang, bei Gleichstand der Wert.
    public func vergleiche(_ other: AblageRegister<T>) -> Int {
        let r = rangVergleich(other)
        return r != 0 ? r : T.ablageVergleich(wert, other.wert)
    }
}

// MARK: - Zusammenführen

/// Namensraum wie das Rust-Modul `zusammenfuehren`.
public enum AblageZusammenfuehren {
    static func schluessel(_ s: String) -> [UInt8] { Array(s.utf8) }

    /// Werte eines byte-geschlüsselten Wörterbuchs in der Reihenfolge einer `BTreeMap` (nach Schlüssel-Bytes).
    static func nachBytes<T>(_ d: [[UInt8]: T]) -> [T] {
        d.sorted { $0.key.lexicographicallyPrecedes($1.key) }.map(\.value)
    }

    static func enthaelt(_ liste: [String], _ x: String) -> Bool { liste.contains { AblageBytes.equal($0, x) } }

    // MARK: Register

    /// Das gewinnende Register (`gewinner`): bei gleichem Rang entscheidet der Wert (nur für die Totalordnung;
    /// ein Gerät vergibt jeden Zähler höchstens einmal). Kommutativ und idempotent.
    public static func gewinner<T: AblageGeordnet>(_ a: AblageRegister<T>, _ b: AblageRegister<T>) -> AblageRegister<T> {
        a.vergleiche(b) >= 0 ? a : b
    }

    // MARK: Unbekannte Felder

    /// Unbekannte Felder vereinigen (`extra_vereinigen`). Bei gleichem Schlüssel gewinnt der als kompaktes JSON
    /// bytegenau größere Wert, damit das Ergebnis nicht von der Reihenfolge abhängt. Ergebnis nach Schlüsseln sortiert.
    public static func extraVereinigen(_ a: [AblageJSON.Member], _ b: [AblageJSON.Member]) -> [AblageJSON.Member] {
        var aus: [[UInt8]: AblageJSON.Member] = [:]
        for m in a { aus[schluessel(m.key)] = m }
        for m in b {
            let k = schluessel(m.key)
            if let alt = aus[k], !alt.value.kanonisch().lexicographicallyPrecedes(m.value.kanonisch()) { continue }
            aus[k] = m
        }
        return nachBytes(aus)
    }

    // MARK: Datei

    /// Blob-Kennungen aller lebenden Fassungen und Texte (`blob_refs_aus`), sortiert und eindeutig.
    public static func blobRefsAus(_ fassungen: [AblageFassung]) -> [String] { AblageDatei.inhaltsRefs(fassungen) }

    /// Fassung gleicher `fas`: unveränderliche Felder bleiben (die nach der abgeleiteten Ordnung größere Fassung
    /// gewinnt), `texte` werden nach `textrev` vereinigt (der größere Text gewinnt je `textrev`).
    static func fassungVereinigen(_ a: AblageFassung, _ b: AblageFassung) -> AblageFassung {
        var basis = AblageOrdnung.vergleiche(a, b) >= 0 ? a : b
        var jeRev: [[UInt8]: AblageText] = [:]
        for t in a.texte + b.texte {
            let k = schluessel(t.textrev)
            if let e = jeRev[k] {
                if AblageOrdnung.vergleiche(t, e) > 0 { jeRev[k] = t }
            } else {
                jeRev[k] = t
            }
        }
        basis.texte = nachBytes(jeRev)
        return basis
    }

    /// Führt zwei Stände derselben Datei zusammen (`zusammenfuehren_datei`; kommutativ, assoziativ, idempotent).
    /// `id` stammt von `a` (der Aufrufer führt nur Stände derselben Datei zusammen). Listen im Ergebnis: `fassungen`
    /// nach `fas`, `wahlen` nach `id`, `entfernt` sortiert und eindeutig, `texte` je Fassung nach `textrev`,
    /// `blob_refs` neu abgeleitet, alles nach UTF-8-Bytes. Gleiche `fas`, `id` oder `textrev` werden verdichtet,
    /// das Größere nach der abgeleiteten Ordnung gewinnt.
    public static func zusammenfuehrenDatei(_ a: AblageDatei, _ b: AblageDatei) -> AblageDatei {
        assert(AblageBytes.equal(a.id, b.id), "nur Stände derselben Datei zusammenführen")
        let entfernt = AblageBytes.sortedUnique(a.entfernt + b.entfernt)
        var weg = AblageMenge()
        weg.formUnion(entfernt)
        var jeFas: [[UInt8]: AblageFassung] = [:]
        for f in a.fassungen + b.fassungen {
            if weg.contains(f.fas) { continue }
            let k = schluessel(f.fas)
            jeFas[k] = jeFas[k].map { fassungVereinigen($0, f) } ?? f
        }
        let fassungen = nachBytes(jeFas)
        var jeWahl: [[UInt8]: AblageWahl] = [:]
        for w in a.wahlen + b.wahlen {
            let k = schluessel(w.id)
            if let e = jeWahl[k] {
                if AblageOrdnung.vergleiche(w, e) > 0 { jeWahl[k] = w }
            } else {
                jeWahl[k] = w
            }
        }
        return AblageDatei(
            id: a.id,
            ordner: gewinner(a.ordner, b.ordner),
            name: gewinner(a.name, b.name),
            klasseMin: gewinner(a.klasseMin, b.klasseMin),
            nurGeraetMin: a.nurGeraetMin || b.nurGeraetMin,
            fassungen: fassungen,
            wahlen: nachBytes(jeWahl),
            entfernt: entfernt,
            blobRefs: blobRefsAus(fassungen),
            papierkorb: gewinner(a.papierkorb, b.papierkorb),
            extra: extraVereinigen(a.extra, b.extra))
    }

    // MARK: Ordner

    /// Führt zwei Stände desselben Ordners zusammen (`zusammenfuehren_ordner`); `id` stammt von `a`.
    public static func zusammenfuehrenOrdner(_ a: AblageOrdner, _ b: AblageOrdner) -> AblageOrdner {
        assert(AblageBytes.equal(a.id, b.id), "nur Stände desselben Ordners zusammenführen")
        return AblageOrdner(
            id: a.id,
            eltern: gewinner(a.eltern, b.eltern),
            name: gewinner(a.name, b.name),
            datenklasse: gewinner(a.datenklasse, b.datenklasse),
            nurGeraet: gewinner(a.nurGeraet, b.nurGeraet),
            zielordnerKi: gewinner(a.zielordnerKi, b.zielordnerKi),
            papierkorb: gewinner(a.papierkorb, b.papierkorb),
            extra: extraVereinigen(a.extra, b.extra))
    }

    /// Führt zwei Ordnerbestände zusammen (`zusammenfuehren_ordnerbestand`): Vereinigung nach Kennung, gleiche
    /// Kennungen per Register.
    public static func zusammenfuehrenOrdnerbestand(_ a: AblageKarte<AblageOrdner>, _ b: AblageKarte<AblageOrdner>) -> AblageKarte<AblageOrdner> {
        var aus = a
        for (id, o) in b.eintraege {
            aus[id] = aus[id].map { zusammenfuehrenOrdner($0, o) } ?? o
        }
        return aus
    }
}

// MARK: - Wahlen, Köpfe, Konflikt

extension AblageZusammenfuehren {
    /// Rang `(zaehler, geraet)` zweier Wahlen: -1, 0 oder 1 (Gerät nach Bytes).
    private static func rangVergleich(_ a: AblageWahl, _ b: AblageWahl) -> Int {
        a.zaehler != b.zaehler ? (a.zaehler < b.zaehler ? -1 : 1) : AblageBytes.compare(a.geraet, b.geraet)
    }

    /// Aktive Wahlen (`aktive_wahlen`): in keinem `ersetzt` einer Wahl genannt, in der Reihenfolge von `wahlen`.
    public static func aktiveWahlen(_ d: AblageDatei) -> [AblageWahl] {
        var ersetzt = AblageMenge()
        for w in d.wahlen { ersetzt.formUnion(w.ersetzt) }
        return d.wahlen.filter { !ersetzt.contains($0.id) }
    }

    /// Abgelöste Fassungen (`abgeloest`, Vorrangregel ADR-0029 E4), sortiert und eindeutig. F ist abgelöst, wenn
    /// eine aktive Wahl W (F in `gegen`, F nicht gewählt) existiert und keine aktive Wahl mit höherem Rang als W
    /// die Fassung F gewählt hat.
    public static func abgeloest(_ d: AblageDatei) -> [String] {
        let aktiv = aktiveWahlen(d)
        var aus = AblageMenge()
        for w in aktiv {
            for f in w.gegen {
                if AblageBytes.equal(f, w.gewaehlt) { continue }
                let ueberstimmt = aktiv.contains { h in rangVergleich(h, w) > 0 && AblageBytes.equal(h.gewaehlt, f) }
                if !ueberstimmt { aus.insert(f) }
            }
        }
        return aus.sortiert
    }

    /// Köpfe (`koepfe`): lebende Fassungen ohne Nachfolger (kein `vorgaenger`-Verweis auf sie), die nicht
    /// abgelöst sind, nach `fas` sortiert (bytegenau, ohne Verdichtung).
    public static func koepfe(_ d: AblageDatei) -> [String] {
        var mitNachfolger = AblageMenge()
        for f in d.fassungen { if let v = f.vorgaenger { mitNachfolger.insert(v) } }
        var abgel = AblageMenge()
        abgel.formUnion(abgeloest(d))
        let k = d.fassungen.filter { !mitNachfolger.contains($0.fas) && !abgel.contains($0.fas) }.map(\.fas)
        return AblageBytes.sorted(k, by: { $0 })
    }

    /// Mehr als ein Kopf.
    public static func konflikt(_ d: AblageDatei) -> Bool { koepfe(d).count > 1 }

    /// Eine Fassung und alle ihre Vorgängerinnen (nur lebende), nächste zuerst (`ahnen`). Bricht bei einem
    /// Zyklus oder einer unbekannten Vorgängerin ab.
    static func ahnen(_ d: AblageDatei, _ fas: String) -> [String] {
        var je: [[UInt8]: AblageFassung] = [:]
        for f in d.fassungen { je[schluessel(f.fas)] = f }
        var aus: [String] = []
        var aktuell = je[schluessel(fas)]?.vorgaenger
        var schritte = 0
        while let id = aktuell {
            guard let f = je[schluessel(id)] else { break }
            aus.append(f.fas)
            aktuell = f.vorgaenger
            schritte += 1
            if schritte > d.fassungen.count { break }
        }
        return aus
    }

    /// Jüngste gemeinsame Vorgängerfassung aller offenen Köpfe (`gemeinsame_vorgaengerin`). `nil` bei weniger als
    /// zwei Köpfen oder getrennten Wurzeln (dann nutzt die KI die Datei bis zur Wahl nicht).
    public static func gemeinsameVorgaengerin(_ d: AblageDatei) -> String? {
        let k = koepfe(d)
        guard k.count >= 2 else { return nil }
        let listen = k.map { ahnen(d, $0) }
        guard let erste = listen.first else { return nil }
        let rest = listen.dropFirst()
        return erste.first { kandidat in rest.allSatisfy { enthaelt($0, kandidat) } }
    }

    /// Die Fassung, die „aktuell“ ist (`aktuell`): der einzige Kopf, sonst `nil`. Nie per Zeit gewählt.
    public static func aktuell(_ d: AblageDatei) -> String? {
        let k = koepfe(d)
        return k.count == 1 ? k[0] : nil
    }

    /// Die Fassung, die als aktuell gilt (`aktuelle_fassung`, `speicher.rs`): der einzige Kopf, sonst die jüngste
    /// nach `(zeit, fas)`. Nur für Anzeige und Suche; die KI nimmt bei Konflikt `gemeinsameVorgaengerin`.
    public static func aktuelleFassung(_ d: AblageDatei) -> AblageFassung? {
        if let id = aktuell(d) { return d.fassungen.first { AblageBytes.equal($0.fas, id) } }
        // Rusts `max_by` liefert bei Gleichstand das letzte Maximum.
        var beste: AblageFassung?
        for f in d.fassungen {
            guard let b = beste else { beste = f; continue }
            let c = f.zeit != b.zeit ? (f.zeit < b.zeit ? -1 : 1) : AblageBytes.compare(f.fas, b.fas)
            if c >= 0 { beste = f }
        }
        return beste
    }

    /// Neue Wahl (`neue_wahl`): `gegen` = offene Köpfe ∪ gewählte Fassung ∪ `gegen` und `gewaehlt` aller berührten
    /// aktiven Wahlen; `ersetzt` = alle berührten aktiven Wahlen. Berührt ist eine aktive Wahl, deren `gegen` oder
    /// `gewaehlt` eine betroffene Fassung (Köpfe, gewählte Fassung, ihre Vorgängerinnen, dazu die Fassungen
    /// bereits berührter Wahlen) nennt. Listen sortiert und eindeutig.
    public static func neueWahl(_ d: AblageDatei, id: String, gewaehlt: String, zaehler: UInt64, geraet: String) -> AblageWahl {
        var anfang = AblageMenge(koepfe(d))
        anfang.insert(gewaehlt)
        for f in anfang.sortiert { anfang.formUnion(ahnen(d, f)) }
        var betroffen = anfang
        var gegen = AblageMenge(koepfe(d))
        gegen.insert(gewaehlt)
        var ersetzt = AblageMenge()
        let aktive = aktiveWahlen(d)
        while true {
            let vorher = (betroffen.count, ersetzt.count)
            for w in aktive {
                let beruehrt = w.gegen.contains { betroffen.contains($0) } || betroffen.contains(w.gewaehlt)
                if beruehrt {
                    ersetzt.insert(w.id)
                    gegen.formUnion(w.gegen)
                    gegen.insert(w.gewaehlt)
                    betroffen.formUnion(w.gegen)
                    betroffen.insert(w.gewaehlt)
                }
            }
            if betroffen.count == vorher.0 && ersetzt.count == vorher.1 { break }
        }
        return AblageWahl(id: id, gewaehlt: gewaehlt, gegen: gegen.sortiert, ersetzt: ersetzt.sortiert, zaehler: zaehler, geraet: geraet)
    }
}
