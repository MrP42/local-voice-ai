import XCTest
@testable import VoiceCore
#if canImport(CryptoKit)

// Parität des lexikalischen Index der Ablage zum Desktop (D5a-4, lva#441, Epic #395, Abnahme 5).
//
// Die Fixture `docs/superpowers/plans/ablage-index-vektoren.json` erzeugt der Rust-Test `ablage::tests_index_paritaet` aus den echten
// Desktop-Wegen (Import, Textwarteschlange, `index.db`, FTS5-`bm25`, `index::suchen_mit`) und prüft sie auf Übereinstimmung. Dieser
// Test liest dieselbe Datei und verlangt vom Swift-Index dasselbe: gleiche Wörter (Anfrage und Tokenizer), gleiche Stücke und
// Längen, gleiche Stoppwörter, gleiche Zeichentabellen, gleiche Rohränge (`bm25`) und gleiche erste fünf Dateien samt Ankern in
// gleicher Reihenfolge. Nur synthetische Daten.

private func bytes(_ s: String) -> [UInt8] { Array(s.utf8) }
private func gleich(_ a: [String], _ b: [String]) -> Bool { a.map(bytes) == b.map(bytes) }

final class AblageIndexParitaetTests: XCTestCase {
    static func fixtureURL() -> URL {
        if let o = ProcessInfo.processInfo.environment["ABLAGE_INDEX_VEKTOREN"] { return URL(fileURLWithPath: o) }
        var url = URL(fileURLWithPath: #filePath).resolvingSymlinksInPath()
        for _ in 0..<6 { url.deleteLastPathComponent() }   // Datei, VoiceCoreTests, Tests, VoiceCore, apple, apps
        return url.appendingPathComponent("docs/superpowers/plans/ablage-index-vektoren.json")
    }

    private var root: [String: Any] = [:]

    override func setUpWithError() throws {
        let data = try Data(contentsOf: Self.fixtureURL())
        root = try XCTUnwrap(try JSONSerialization.jsonObject(with: data) as? [String: Any], "Fixture ist kein Objekt")
    }

    private func liste(_ k: String, in d: [String: Any]? = nil) throws -> [[String: Any]] {
        try XCTUnwrap((d ?? root)[k] as? [[String: Any]], k)
    }

    private func strings(_ k: String, in d: [String: Any]) throws -> [String] { try XCTUnwrap(d[k] as? [String], k) }

    // MARK: Wörter

    func testStoppwoerterSindDieDesDesktops() throws {
        let stop = try strings("stop", in: root)
        XCTAssertGreaterThan(stop.count, 40)
        XCTAssertTrue(gleich(AblageSuchtext.stoppwoerter, stop), "Stoppliste weicht vom Quelltext von wai-retrieval ab")
    }

    /// `content_tokens` und die FTS5-Zerlegung je Probe, Byte für Byte.
    func testProbenInhaltsWoerterUndFts5Woerter() throws {
        let proben = try liste("proben")
        XCTAssertGreaterThan(proben.count, 100)
        for p in proben {
            let text = try XCTUnwrap(p["text"] as? String)
            let inhalt = try strings("inhalt", in: p), fts5 = try strings("fts5", in: p)
            XCTAssertTrue(gleich(AblageSuchtext.inhaltsWoerter(text), inhalt), "content_tokens(\(text.debugDescription)): \(AblageSuchtext.inhaltsWoerter(text)) statt \(inhalt)")
            XCTAssertTrue(gleich(AblageSuchtext.fts5Woerter(text), fts5), "FTS5 \(text.debugDescription): \(AblageSuchtext.fts5Woerter(text)) statt \(fts5)")
        }
    }

    /// Die Zahlenreihen in `AblageFts5Tabellen.swift` sind die der Datei (sonst ist die Tabelle veraltet: Skript neu laufen lassen).
    func testZeichentabellenSindDieDerFixture() throws {
        let f = try XCTUnwrap(root["fts5"] as? [String: Any])
        func bereiche(_ k: String) throws -> [UInt32] {
            var aus: [UInt32] = []
            for t in try XCTUnwrap(f[k] as? String).split(separator: ",") {
                let p = t.split(separator: "-").map { UInt32($0, radix: 16)! }
                aus += [p[0], p.count > 1 ? p[1] : p[0]]
            }
            return aus
        }
        XCTAssertEqual(AblageFts5Tabellen.wortzeichen, try bereiche("token_zeichen"))
        XCTAssertEqual(AblageFts5Tabellen.folgezeichen, try bereiche("folge_zeichen"))
        XCTAssertEqual(AblageFts5Tabellen.entfernen, try bereiche("entfernen"))
        let falten = try XCTUnwrap(f["falten"] as? String).split(separator: ",").flatMap { $0.split(separator: ":").map { Int32($0)! } }
        XCTAssertEqual(AblageFts5Tabellen.falten, falten)
        // Stichproben der Faltung, wie FTS5 sie wirklich macht (die Proben oben prüfen weit mehr): nur Diakritika (Akzente, Umlautpunkte)
        // fallen weg; Striche (ø, ł), „ß“ und die Akzente außerhalb des Lateinischen bleiben, nur klein geschrieben.
        for (zeichen, soll) in [("Ä", "a"), ("ö", "o"), ("ü", "u"), ("é", "e"), ("ß", "ß"), ("ẞ", "ß"), ("Ø", "ø"), ("ł", "ł"), ("Ё", "ё"), ("ς", "σ"), ("ό", "ό")] {
            XCTAssertEqual(AblageSuchtext.fts5Woerter(zeichen).first.map(bytes), bytes(soll), zeichen)
        }
    }

    // MARK: Stücke und Index

    private struct Datei {
        let name: String
        let abschnitte: [AblageAbschnitt]
        let stuecke: [[String: Any]]
    }

    private func korpus() throws -> [Datei] {
        try liste("korpus").map { k in
            let name = try XCTUnwrap(k["name"] as? String)
            let ab = try liste("abschnitte", in: k).map {
                AblageAbschnitt(anker: $0["anker"] as! String, pfad: ($0["pfad"] as? [String]) ?? [], text: $0["text"] as! String)
            }
            return Datei(name: name, abschnitte: ab, stuecke: try liste("stuecke", in: k))
        }
    }

    func testStueckeUndLaengenGleichDesktop() throws {
        let k = try korpus()
        XCTAssertGreaterThanOrEqual(k.count, 10)
        var vielfach = 0
        for d in k {
            var erzeugt: [(abschnitt: Int, text: String)] = []
            for (i, a) in d.abschnitte.enumerated() { for s in AblageSuchtext.stuecke(a.text) { erzeugt.append((i, s)) } }
            XCTAssertEqual(erzeugt.count, d.stuecke.count, d.name)
            for (i, s) in d.stuecke.enumerated() where i < erzeugt.count {
                XCTAssertEqual(bytes(erzeugt[i].text), bytes(s["text"] as! String), "\(d.name) Stück \(i + 1)")
                XCTAssertEqual(erzeugt[i].abschnitt, s["abschnitt"] as? Int, d.name)
                XCTAssertEqual(s["seite"] as? Int, i + 1, d.name)
                XCTAssertEqual(AblageSuchtext.fts5Woerter(erzeugt[i].text).count, s["laenge"] as? Int, "Länge \(d.name) Stück \(i + 1)")
            }
            if d.stuecke.count > d.abschnitte.count { vielfach += 1 }
        }
        XCTAssertGreaterThan(vielfach, 0, "mindestens eine Datei hat Abschnitte, die in mehrere Stücke zerfallen (Überlappung)")
    }

    /// `index::stuecke` auf Eingaben mit Leerraum aller Art, Mehrbyte-Zeichen und Wörtern ohne Trenner, mit den Werten der App und
    /// kleinen Werten (Schnitt, Überlappung und Wortanfang werden dort am meisten gefordert).
    func testStueckeProbenGleichDesktop() throws {
        let proben = try liste("stuecke_proben")
        XCTAssertGreaterThan(proben.count, 20)
        var mehrere = 0
        for p in proben {
            let text = try XCTUnwrap(p["text"] as? String)
            let max = try XCTUnwrap(p["max"] as? Int), ueber = try XCTUnwrap(p["ueber"] as? Int)
            let soll = try strings("stuecke", in: p)
            let ist = AblageSuchtext.stuecke(text, max: max, ueber: ueber)
            XCTAssertTrue(gleich(ist, soll), "stuecke(\(text.debugDescription), \(max), \(ueber)): \(ist) statt \(soll)")
            if soll.count > 1 { mehrere += 1 }
        }
        XCTAssertGreaterThan(mehrere, 10, "die Proben zerfallen oft in mehrere Stücke")
    }

    private func index(_ k: [Datei]) -> AblageIndex {
        AblageIndex(k.map { AblageIndex.vorbereiten(dat: $0.name, abschnitte: $0.abschnitte) })
    }

    func testIndexZahlenUndStueckeGleichDesktop() throws {
        let k = try korpus()
        let ix = index(k)
        let stat = try XCTUnwrap(root["statistik"] as? [String: Any])
        XCTAssertEqual(ix.anzahlStuecke, stat["stuecke"] as? Int)
        XCTAssertEqual(ix.anzahlWoerter, stat["tokens"] as? Int)
        XCTAssertEqual(ix.anzahlDateien, k.count)
        for d in k {
            let st = ix.stuecke(dat: d.name)
            XCTAssertEqual(st.count, d.stuecke.count, d.name)
            for (s, soll) in zip(st, d.stuecke) {
                XCTAssertEqual(s.seite, soll["seite"] as? Int)
                XCTAssertEqual(s.anker, soll["anker"] as? String, d.name)
                XCTAssertEqual(s.pfad, (soll["pfad"] as? [String]) ?? [], d.name)
                XCTAssertEqual(bytes(s.text), bytes(soll["text"] as! String), d.name)
                XCTAssertEqual(ix.laenge(dat: d.name, seite: s.seite), soll["laenge"] as? Int, d.name)
            }
        }
    }

    // MARK: Fragen

    private func gleichGenau(_ a: Double, _ b: Double, _ was: String, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertLessThanOrEqual(abs(a - b), 1e-9 * max(abs(a), abs(b), 1e-300), "\(was): \(a) statt \(b)", file: file, line: line)
    }

    /// Rohrang, Kandidaten und die ersten fünf Dateien mit Ankern: wie der Desktop.
    func testFragenGleichDesktop() throws {
        let ix = index(try korpus())
        let fragen = try liste("fragen")
        XCTAssertGreaterThanOrEqual(fragen.count, 20)
        var mitTreffern = 0, ohneTreffer = 0
        for f in fragen {
            let frage = try XCTUnwrap(f["frage"] as? String)
            XCTAssertTrue(gleich(AblageIndex.anfrageWoerter(frage), try strings("tokens", in: f)), "Anfragewörter \(frage)")

            // Rohrang je Stück (bm25) in der Reihenfolge der SQL-Abfrage.
            let zeilen = ix.zeilen(frage)
            XCTAssertEqual(zeilen.count, f["bm25_anzahl"] as? Int, "Zahl der Zeilen \(frage)")
            let bm25 = try liste("bm25", in: f)
            let roh = ix.rohzeilen(frage)
            for (i, soll) in bm25.enumerated() where i < roh.count {
                XCTAssertEqual(roh[i].dat, soll["name"] as? String, "Reihenfolge \(frage) #\(i)")
                XCTAssertEqual(roh[i].seite, soll["seite"] as? Int, "Reihenfolge \(frage) #\(i)")
                gleichGenau(roh[i].bm25, soll["wert"] as! Double, "bm25 \(frage) #\(i)")
            }
            XCTAssertGreaterThanOrEqual(zeilen.count, bm25.count, frage)

            // Kandidaten nach Kappung (die flache Liste von `ki_suchen`).
            let stellen = ix.kandidatenstellen(frage)
            for (i, soll) in try liste("stellen", in: f).enumerated() {
                XCTAssertLessThan(i, stellen.count, frage)
                guard i < stellen.count else { break }
                XCTAssertEqual(stellen[i].dat, soll["name"] as? String, "Stelle \(frage) #\(i)")
                XCTAssertEqual(stellen[i].seite, soll["seite"] as? Int, "Stelle \(frage) #\(i)")
                XCTAssertEqual(stellen[i].anker, soll["anker"] as? String, "Anker \(frage) #\(i)")
                gleichGenau(stellen[i].rang, soll["rang"] as! Double, "Rang \(frage) #\(i)")
            }

            // Treffer je Datei: erste fünf, Reihenfolge, Anker, Auszug.
            let treffer = ix.suche(frage)
            XCTAssertEqual(treffer.count, f["treffer_gesamt"] as? Int, "Zahl der Dateien \(frage)")
            let dateien = try liste("dateien", in: f)
            XCTAssertEqual(Array(treffer.prefix(5)).map(\.dat), dateien.map { $0["name"] as! String }, "erste fünf Dateien \(frage)")
            for (t, soll) in zip(treffer, dateien) {
                gleichGenau(t.rang, soll["rang"] as! Double, "Dateirang \(frage)")
                let ss = try liste("stellen", in: soll)
                XCTAssertEqual(t.stellen.map(\.anker), ss.map { $0["anker"] as! String }, "Anker \(frage) \(t.dat)")
                XCTAssertEqual(t.stellen.map(\.pfad), ss.map { ($0["pfad"] as? [String]) ?? [] }, "Pfad \(frage) \(t.dat)")
                XCTAssertEqual(t.stellen.map { bytes($0.auszug) }, ss.map { bytes($0["auszug"] as! String) }, "Auszug \(frage) \(t.dat)")
            }
            if treffer.isEmpty { ohneTreffer += 1 } else { mitTreffern += 1 }
        }
        XCTAssertGreaterThan(mitTreffern, 12)
        XCTAssertGreaterThan(ohneTreffer, 1, "mindestens die Stoppwortfrage und eine Frage ohne Fundstelle")
    }

    /// Der Papierkorb des Desktops: Kontrolltext erst gefunden, dann 0 Treffer. (Die Gegenprobe am iPhone steht in `AblageSucheTests`.)
    func testDesktopPapierkorbVektor() throws {
        let p = try XCTUnwrap(root["papierkorb"] as? [String: Any])
        XCTAssertEqual(p["treffer_vorher"] as? Int, 1)
        XCTAssertEqual(p["treffer_nachher"] as? Int, 0)
    }
}
#endif
