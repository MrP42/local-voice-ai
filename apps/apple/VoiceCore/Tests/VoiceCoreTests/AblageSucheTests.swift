import XCTest
@testable import VoiceCore
#if canImport(CryptoKit)

// Suche in der Ablage auf dem iPhone (D5a-4, lva#441, Epic #395, Abnahme 6 und 7): der Index-Tor wirkt VOR dem Aufbau.
//
// Papierkorb (Datei und Ordner), entfernte und begrabene Dateien liefern 0 Treffer; nichts aus `Lokal` (nur Gerät, `personal`,
// `confidential`) oder `Halten` (unbekannte Klasse) und nichts aus einem angehaltenen Stand kommt in den Index oder in eine
// Fundstelle (Test mit Kontrolltext); ein Konflikt ohne gemeinsame Fassung wird in der Abdeckung gemeldet. Dazu: Stücke, Wörter,
// Phrasen und die Formel von BM25 an kleinen Beispielen; Zwischenspeicher und Fehler des Suchdienstes. Nur synthetische Inhalte.

private typealias W = AblageWelt

private let kontrolle = "Zebrafink"

private struct Aufbau {
    var z = W.neuerZustand()
    /// Texte, die auf die Platte kommen (Datei, Fassung, Bytes).
    var texte: [(dat: String, fas: AblageFassung, bytes: [UInt8])] = []
    var ordner: [AblageOrdner] = []
    var dateien: [AblageDatei] = []
    var graeber: [String] = []
}

final class AblageSucheTests: XCTestCase {
    private var tmp: URL!
    private var speicher: AblageSpeicher!
    private let fm = FileManager.default

    override func setUpWithError() throws {
        tmp = fm.temporaryDirectory.appendingPathComponent("ablage-suche-\(UUID().uuidString)", isDirectory: true)
        try fm.createDirectory(at: tmp, withIntermediateDirectories: true)
        speicher = AblageSpeicher(wurzel: tmp.appendingPathComponent("ablage", isDirectory: true))
    }

    override func tearDownWithError() throws { try? fm.removeItem(at: tmp) }

    // MARK: Hilfen

    /// Eine Fassung mit einem Text aus einem Abschnitt (`zeile=1`); der Inhalt des Originals ist je Fassung verschieden.
    private func fassung(_ n: Int, _ vorgaenger: Int? = nil, text: String, zeit: Int64 = 1_000) -> (AblageFassung, [UInt8]) {
        var f = W.fassung(n, vorgaenger, inhalt: "Original \(n) \(text)", typ: "txt", zeit: zeit)
        let t = W.text(zu: f, abschnitte: [("zeile=1", text)])
        f.texte = [t.eintrag]
        return (f, t.bytes)
    }

    private func datei(_ a: inout Aufbau, _ n: Int, ordner: Int = 1, name: String, fassungen: [(AblageFassung, [UInt8])], mitText: Bool = true,
                       korb: Int64? = nil, nurGeraetMin: Bool = false) {
        let d = W.datei(n, ordner: W.ordId(ordner), name: name, fassungen: fassungen.map(\.0), nurGeraetMin: nurGeraetMin, korb: korb)
        a.dateien.append(d)
        if mitText { for (f, b) in fassungen { a.texte.append((d.id, f, b)) } }
    }

    private func einspielen(_ a: Aufbau) throws -> (z: AblageZustand, v: AblageVorhanden) {
        var z = a.z
        let o = a.ordner.isEmpty ? [W.ordner(1, name: "Ablage", klasse: "internal")] : a.ordner
        W.einspielen(&z, ordner: o.map { W.remote(ordner: $0) }, dateien: a.dateien.map { W.remote(datei: $0) })
        // Texte zuerst auf die Platte; ein später begrabener Text bleibt dort als Altlast liegen (der Lauf räumt ihn sonst weg).
        for t in a.texte {
            guard let e = t.fas.texte.first else { continue }
            try speicher.textEinlegen(bestand: z.bestand, dat: t.dat, fas: t.fas.fas, textrev: e.textrev, bytes: t.bytes)
        }
        if !a.graeber.isEmpty {
            W.einspielen(&z, dateien: a.graeber.enumerated().map { W.remoteGrab(collection: "ablage_datei", id: $1, revision: Int64(100 + $0)) })
        }
        return (z, speicher.vorhanden(z.bestand))
    }

    private func suche(_ z: AblageZustand, _ frage: String = kontrolle) async -> (treffer: [AblageTreffer], dienst: AblageSuchdienst) {
        let d = AblageSuchdienst()
        await d.aktualisieren(zustand: z, speicher: speicher)
        return (await d.suche(frage), d)
    }

    private func dats(_ t: [AblageTreffer]) -> [String] { t.map(\.dat) }

    // MARK: Abnahme 6: Papierkorb, entfernt, begraben

    func testGefundenMitAnkerUndPfad() async throws {
        var a = Aufbau()
        datei(&a, 1, name: "gut.txt", fassungen: [fassung(1, text: "Das \(kontrolle) singt am Morgen.")])
        let (z, _) = try einspielen(a)
        let (t, _) = await suche(z)
        XCTAssertEqual(dats(t), [W.datId(1)])
        XCTAssertEqual(t[0].stellen.map(\.anker), ["zeile=1"])
        XCTAssertTrue(t[0].stellen[0].auszug.contains(kontrolle))
    }

    func testPapierkorbDateiOrdnerEntferntUndBegrabenLiefernNullTreffer() async throws {
        var a = Aufbau()
        a.ordner = [W.ordner(1, name: "Ablage", klasse: "internal"), W.ordner(2, name: "Alt", eltern: nil, klasse: "internal", korb: 5_000)]
        datei(&a, 1, name: "gut.txt", fassungen: [fassung(1, text: "\(kontrolle) gut")])
        datei(&a, 2, name: "korb.txt", fassungen: [fassung(2, text: "\(kontrolle) im Papierkorb")], korb: 5_000)
        datei(&a, 3, ordner: 2, name: "imKorbOrdner.txt", fassungen: [fassung(3, text: "\(kontrolle) im Ordner im Papierkorb")])
        datei(&a, 4, name: "begraben.txt", fassungen: [fassung(4, text: "\(kontrolle) begraben")])
        a.graeber = [W.datId(4)]
        let (z, _) = try einspielen(a)
        XCTAssertTrue(z.ledger.begraben.contains(W.datId(4)))
        XCTAssertNotNil(speicher.textLesen(dat: W.datId(4), textrev: a.texte.first { $0.dat == W.datId(4) }!.fas.texte[0].textrev), "Altlast liegt noch auf der Platte")
        XCTAssertFalse(AblageAnzeige.darfIndiziert(z, dat: W.datId(4)), "begraben: nie zugelassen")
        let (t, d) = await suche(z)
        XCTAssertEqual(dats(t), [W.datId(1)], "nur die lebende Datei")
        let ix = await d.index
        for n in [2, 3, 4] { XCTAssertEqual(ix.stuecke(dat: W.datId(n)).count, 0, "Datei \(n) ist nie ein Stück") }
        XCTAssertEqual(ix.anzahlDateien, 1)
    }

    /// Papierkorb und Wiederherstellen: der Index folgt dem Zustand (Neuaufbau aus den zwischengespeicherten Stücken).
    func testPapierkorbUndWiederherstellenFolgtDemZustand() async throws {
        var a = Aufbau()
        datei(&a, 1, name: "a.txt", fassungen: [fassung(1, text: "\(kontrolle) a")])
        var (z, _) = try einspielen(a)
        let dienst = AblageSuchdienst()
        await dienst.aktualisieren(zustand: z, speicher: speicher)
        var treffer = await dienst.suche(kontrolle)
        XCTAssertEqual(dats(treffer), [W.datId(1)])
        // In den Papierkorb (höheres Register gewinnt).
        z.bestand.dateien[W.datId(1)]?.papierkorb = AblageRegister(wert: 9_000, zaehler: 5, geraet: W.geraet)
        await dienst.aktualisieren(zustand: z, speicher: speicher)
        treffer = await dienst.suche(kontrolle)
        XCTAssertEqual(treffer, [], "Papierkorb: 0 Treffer")
        // Wiederherstellen.
        z.bestand.dateien[W.datId(1)]?.papierkorb = AblageRegister(wert: nil, zaehler: 6, geraet: W.geraet)
        await dienst.aktualisieren(zustand: z, speicher: speicher)
        treffer = await dienst.suche(kontrolle)
        XCTAssertEqual(dats(treffer), [W.datId(1)])
    }

    /// Eine entfernte Fassung (`entfernt`) ist nicht mehr da; gesucht wird nur der Text der aktuellen Fassung.
    func testEntfernteUndAelterFassungenSindNichtImIndex() async throws {
        var a = Aufbau()
        let alt = fassung(1, text: "Altwort \(kontrolle) alt")
        let neu = fassung(2, 1, text: "Neuwort neu", zeit: 2_000)
        datei(&a, 1, name: "v.txt", fassungen: [alt, neu])
        var (z, _) = try einspielen(a)
        var (t, _) = await suche(z, "Altwort")
        XCTAssertEqual(t, [], "der Text der älteren Fassung wird nie gesucht")
        (t, _) = await suche(z, "Neuwort")
        XCTAssertEqual(dats(t), [W.datId(1)])
        // Die ältere Fassung wird wegen Aufbewahrung entfernt (`entfernt`, wächst nur): sie fehlt in `fassungen`.
        var d = try XCTUnwrap(z.bestand.dateien[W.datId(1)])
        d.entfernt = [alt.0.fas]
        d.fassungen = [neu.0]
        z.bestand.dateien[W.datId(1)] = d
        (t, _) = await suche(z, "Altwort")
        XCTAssertEqual(t, [])
        (t, _) = await suche(z, "Neuwort")
        XCTAssertEqual(dats(t), [W.datId(1)])
        // Die aktuelle Fassung ohne eigenen Text: die Datei ist „ohne Text“, der Text der älteren Fassung gilt nicht.
        var a2 = Aufbau()
        let altF = fassung(10, text: "Altwort")
        let ohne = fassung(11, 10, text: "Neuwort", zeit: 3_000)
        datei(&a2, 10, name: "o.txt", fassungen: [altF, ohne], mitText: false)
        a2.texte.append((W.datId(10), altF.0, altF.1))     // nur der Text der älteren Fassung liegt hier
        (z, _) = try einspielen(a2)
        (t, _) = await suche(z, "Altwort")
        XCTAssertEqual(t, [], "aktuelle Fassung ohne Text: nicht den der alten nehmen")
    }

    // MARK: Abnahme 7: nichts aus Lokal/Halten/angehalten im Index

    func testLokalHaltenUndAngehaltenKommenNieInDenIndex() async throws {
        var a = Aufbau()
        a.ordner = [W.ordner(1, name: "Ablage", klasse: "internal"),
                    W.ordner(2, name: "Nur hier", klasse: "internal", nurGeraet: true),
                    W.ordner(3, name: "Privat", klasse: "personal"),
                    W.ordner(4, name: "Vertraulich", klasse: "confidential"),
                    W.ordner(5, name: "Neue Klasse", klasse: "geheim-neu")]
        datei(&a, 1, name: "gut.txt", fassungen: [fassung(1, text: "\(kontrolle) gut")])
        datei(&a, 2, ordner: 2, name: "nurGeraet.txt", fassungen: [fassung(2, text: "\(kontrolle) nur Gerät GEHEIM2")])
        datei(&a, 3, ordner: 3, name: "personal.txt", fassungen: [fassung(3, text: "\(kontrolle) personal GEHEIM3")])
        datei(&a, 4, ordner: 4, name: "confidential.txt", fassungen: [fassung(4, text: "\(kontrolle) vertraulich GEHEIM4")])
        datei(&a, 5, ordner: 5, name: "halten.txt", fassungen: [fassung(5, text: "\(kontrolle) unbekannt GEHEIM5")])
        datei(&a, 6, name: "dateiNurGeraet.txt", fassungen: [fassung(6, text: "\(kontrolle) Datei nur Gerät GEHEIM6")], nurGeraetMin: true)
        datei(&a, 7, name: "angehalten.txt", fassungen: [fassung(7, text: "\(kontrolle) angehalten GEHEIM7")])
        var (z, _) = try einspielen(a)
        z.ledger.objekte[AblageLedger.schluessel("ablage_datei", W.datId(7))]?.angehalten = true
        let (t, d) = await suche(z)
        XCTAssertEqual(dats(t), [W.datId(1)], "nur die zugelassene Datei")
        let ix = await d.index
        XCTAssertEqual(ix.anzahlDateien, 1)
        for n in 2...7 { XCTAssertEqual(ix.stuecke(dat: W.datId(n)), [], "Datei \(n) darf nie ein Stück sein") }
        // Kein Treffer enthält je den Kontrolltext der gesperrten Dateien, auch nicht über andere Wörter.
        for wort in ["GEHEIM2", "GEHEIM3", "GEHEIM4", "GEHEIM5", "GEHEIM6", "GEHEIM7", "nur Gerät", "personal", "vertraulich", "unbekannt", "angehalten"] {
            let r = await d.suche(wort)
            XCTAssertEqual(r, [], wort)
        }
        let ab = await d.abdeckung
        XCTAssertEqual(ab.imBereich, 7)
        XCTAssertEqual(ab.durchsucht, 1)
        XCTAssertEqual(ab.nichtDurchsuchbar, 6)
    }

    // MARK: Abdeckung

    func testAbdeckungNenntDateienImBereichDurchsuchtOhneTextImKonflikt() async throws {
        var a = Aufbau()
        a.ordner = [W.ordner(1, name: "Ablage", klasse: "internal"), W.ordner(2, name: "Nur hier", klasse: "internal", nurGeraet: true)]
        datei(&a, 1, name: "gut.txt", fassungen: [fassung(1, text: "\(kontrolle) gut")])
        datei(&a, 2, name: "korb.txt", fassungen: [fassung(2, text: "\(kontrolle) korb")], korb: 5_000)
        datei(&a, 3, ordner: 2, name: "lokal.txt", fassungen: [fassung(3, text: "\(kontrolle) lokal")])
        datei(&a, 6, name: "wartet.txt", fassungen: [fassung(6, text: "wartet")], mitText: false)
        // Konflikt mit gemeinsamer Vorgängerfassung: f21 -> (f22, f23).
        datei(&a, 7, name: "konfliktMit.txt", fassungen: [fassung(20, text: "Basis"), fassung(21, 20, text: "\(kontrolle) Konflikt eins", zeit: 2_000),
                                                           fassung(22, 20, text: "\(kontrolle) Konflikt zwei", zeit: 3_000)])
        // Konflikt ohne gemeinsame Fassung: zwei Wurzeln.
        datei(&a, 8, name: "konfliktOhne.txt", fassungen: [fassung(30, text: "\(kontrolle) Wurzel eins", zeit: 2_000), fassung(31, text: "\(kontrolle) Wurzel zwei", zeit: 3_000)])
        let (z, _) = try einspielen(a)
        let (t, d) = await suche(z)
        let ab = await d.abdeckung
        XCTAssertEqual(ab.imBereich, 5, "ohne die Datei im Papierkorb")
        XCTAssertEqual(ab.durchsucht, 3, "gut und beide Konfliktdateien")
        XCTAssertEqual(ab.ohneText, 1, "wartet auf den PC")
        XCTAssertEqual(ab.nichtDurchsuchbar, 1, "nur dieses Gerät")
        XCTAssertEqual(ab.imKonflikt, 2)
        XCTAssertEqual(ab.konfliktOhneGemeinsame, 1, "Konflikt ohne gemeinsame Fassung wird gemeldet")
        XCTAssertEqual(ab.text, "5 Dateien im Bereich, 3 durchsucht, 1 ohne Text, 2 im Konflikt (1 ohne gemeinsame Fassung), 1 nicht durchsuchbar")
        // Konfliktdateien werden wie am Desktop mit der jüngsten Fassung durchsucht.
        let konflikt = t.first { $0.dat == W.datId(7) }
        XCTAssertTrue(konflikt?.stellen.first?.auszug.contains("Konflikt zwei") ?? false)
        XCTAssertFalse(t.contains { $0.stellen.contains { $0.auszug.contains("Konflikt eins") } }, "nur die jüngste Fassung")
    }

    // MARK: Dienst

    func testDienstZwischenspeicherUndAbmelden() async throws {
        var a = Aufbau()
        datei(&a, 1, name: "a.txt", fassungen: [fassung(1, text: "Eins \(kontrolle)")])
        let (z, _) = try einspielen(a)
        let d = AblageSuchdienst()
        await d.aktualisieren(zustand: z, speicher: speicher)
        let i1 = await d.index
        await d.aktualisieren(zustand: z, speicher: speicher)
        let i2 = await d.index
        XCTAssertTrue(i1 === i2, "unverändert: kein Neuaufbau")
        await d.leeren()
        let leer = await d.index
        XCTAssertEqual(leer.anzahlStuecke, 0)
        let r = await d.suche(kontrolle)
        XCTAssertEqual(r, [])
        let ab = await d.abdeckung
        XCTAssertEqual(ab, AblageAbdeckung())
    }

    /// Ein Text auf der Platte, der nicht zu seiner `textrev` passt oder nicht lesbar ist: die Datei zählt „ohne Text“, kein Absturz.
    func testBeschaedigterTextIstOhneTextKeinAbsturz() async throws {
        var a = Aufbau()
        datei(&a, 1, name: "a.txt", fassungen: [fassung(1, text: "Eins \(kontrolle)")])
        datei(&a, 2, name: "b.txt", fassungen: [fassung(2, text: "Zwei \(kontrolle)")])
        let (z, _) = try einspielen(a)
        let b = try XCTUnwrap(z.bestand.dateien[W.datId(2)]?.fassungen[0].texte[0])
        let url = try XCTUnwrap(speicher.textURL(dat: W.datId(2), textrev: b.textrev))
        try Data("kein JSON".utf8).write(to: url)
        let (t, d) = await suche(z)
        XCTAssertEqual(dats(t), [W.datId(1)])
        let ab = await d.abdeckung
        XCTAssertEqual(ab.durchsucht, 1)
        XCTAssertEqual(ab.ohneText, 1)
    }

    // MARK: Stücke, Wörter, Phrasen, BM25

    func testStueckeWieDerDesktop() {
        let s = AblageSuchtext.stuecke(String(repeating: "Wort ", count: 100))
        XCTAssertGreaterThanOrEqual(s.count, 3)
        XCTAssertTrue(s.allSatisfy { $0.unicodeScalars.count <= 220 })
        XCTAssertTrue(s.allSatisfy { $0.hasPrefix("Wort") })
        XCTAssertGreaterThan(s.reduce(0) { $0 + $1.unicodeScalars.count }, 499, "Überlappung: Summe größer als der Text")
        XCTAssertEqual(AblageSuchtext.stuecke("kurz"), ["kurz"])
        let mehr = AblageSuchtext.stuecke(String(repeating: "äöü ßé€ ", count: 300))
        XCTAssertGreaterThanOrEqual(mehr.count, 3)
        XCTAssertTrue(mehr.allSatisfy { $0.unicodeScalars.count <= 220 && !$0.isEmpty })
        let dicht = AblageSuchtext.stuecke(String(repeating: "ä", count: 1000))
        XCTAssertTrue(dicht.allSatisfy { $0.unicodeScalars.allSatisfy { $0 == "ä" } })
        XCTAssertEqual(dicht[0].unicodeScalars.count, 220, "ohne Leerraum: harter Schnitt")
        XCTAssertEqual(AblageSuchtext.stuecke("   \n\t  "), [])
        XCTAssertEqual(AblageSuchtext.stuecke(""), [])
    }

    func testAnfrageWirdAuf200ZeichenGekuerzt() {
        let frage = String(repeating: "alpha ", count: 33) + "omega"    // 198 + 5: „omega“ liegt hinter Zeichen 200
        XCTAssertEqual(frage.unicodeScalars.count, 203)
        let w = AblageIndex.anfrageWoerter(frage)
        XCTAssertFalse(w.contains("omega"))
        XCTAssertTrue(w.contains("om"), "mitten im Wort abgeschnitten, wie am Desktop")
        // Gezählt wird in Unicode-Skalaren, nicht in Bytes (ein Skalar „ä“ zählt einmal).
        XCTAssertEqual(AblageIndex.anfrageWoerter(String(repeating: "ä", count: 200) + "ö").count, 1)
    }

    func testStoppwoerterUndGrossKleinUndDiakritika() {
        XCTAssertEqual(AblageSuchtext.inhaltsWoerter("Der Hund und die Katze"), ["hund", "katze"])
        XCTAssertEqual(AblageSuchtext.inhaltsWoerter("DER DIE DAS"), [])
        XCTAssertEqual(AblageSuchtext.inhaltsWoerter("fuer für"), [])
        XCTAssertEqual(AblageSuchtext.fts5Woerter("Über KÜNDIGUNG, Café"), ["uber", "kundigung", "cafe"])
        XCTAssertEqual(AblageSuchtext.fts5Woerter("Straße"), ["straße"], "ß bleibt im Tokenizer wie in FTS5; die Faltung ist `ftsNormalisieren` davor")
        XCTAssertEqual(AblageSuchtext.fts5Woerter(AblageSuchtext.ftsNormalisieren("Straße")), ["strasse"])
        // Schluss-Sigma wie Rust `to_lowercase`.
        XCTAssertEqual(AblageSuchtext.inhaltsWoerter("ΌΣΟΣ").map { Array($0.unicodeScalars.map(\.value)) }, [[0x3CC, 0x3C3, 0x3BF, 0x3C2]])
    }

    func testIndexMitDiakritikaGefaltetUndPhrasenSucheFindetBeideSchreibweisen() async throws {
        let ab = [AblageAbschnitt(anker: "zeile=1", pfad: [], text: "Die Kündigungsfrist beträgt drei Monate."),
                  AblageAbschnitt(anker: "zeile=3", pfad: [], text: "Das Café an der Straße.")]
        let ix = AblageIndex([AblageIndex.vorbereiten(dat: "dat_a", abschnitte: ab)])
        XCTAssertEqual(ix.suche("kundigungsfrist").map(\.dat), ["dat_a"])
        XCTAssertEqual(ix.suche("KÜNDIGUNGSFRIST").first?.stellen.map(\.anker), ["zeile=1"])
        XCTAssertEqual(ix.suche("cafe").first?.stellen.map(\.anker), ["zeile=3"])
        XCTAssertEqual(ix.suche("Strasse").first?.stellen.map(\.anker), ["zeile=3"], "ß wird wie am Desktop zu ss gefaltet (lva#463)")
        XCTAssertEqual(ix.suche("der die das"), [])
        XCTAssertEqual(ix.suche(""), [])
    }

    /// Die BM25-Formel von FTS5 an einem Beispiel zum Nachrechnen: zwei Stücke, ein Wort in beiden (idf wird zu 1e-6), ein seltenes.
    func testBm25FormelUndIdfUntergrenze() {
        let ix = AblageIndex([
            AblageIndex.vorbereiten(dat: "dat_a", abschnitte: [AblageAbschnitt(anker: "a", pfad: [], text: "alpha beta")]),
            AblageIndex.vorbereiten(dat: "dat_b", abschnitte: [AblageAbschnitt(anker: "b", pfad: [], text: "alpha gamma gamma gamma")]),
        ])
        XCTAssertEqual(ix.anzahlWoerter, 6)
        // alpha steckt in beiden Stücken: idf = ln((2-2+0.5)/(2+0.5)) < 0, also 1e-6.
        let alpha = ix.rohzeilen("alpha")
        let avg = 6.0 / 2.0
        let erwartetA = -1e-6 * (1.0 * 2.2) / (1.0 + 1.2 * (1 - 0.75 + 0.75 * 2.0 / avg))
        XCTAssertEqual(alpha.map(\.dat), ["dat_a", "dat_b"], "das kürzere Stück gewinnt bei gleicher Häufigkeit")
        XCTAssertEqual(alpha.first { $0.dat == "dat_a" }!.bm25, erwartetA, accuracy: 1e-15)
        // gamma: n = 1, N = 2, idf = ln(1.5/1.5) = 0 -> 1e-6; f = 3, D = 4.
        let gamma = ix.rohzeilen("gamma")
        XCTAssertEqual(gamma.count, 1)
        let erwartetG = -1e-6 * (3.0 * 2.2) / (3.0 + 1.2 * (1 - 0.75 + 0.75 * 4.0 / avg))
        XCTAssertEqual(gamma[0].bm25, erwartetG, accuracy: 1e-15)
        // beta: n = 1 von N = 2 ergibt idf = ln(1) = 0 -> 1e-6 ebenfalls.
        XCTAssertEqual(ix.rohzeilen("beta").count, 1)
        // Mit drei Stücken wird idf positiv: ln((3-1+0.5)/(1+0.5)).
        let drei = AblageIndex([
            AblageIndex.vorbereiten(dat: "dat_a", abschnitte: [AblageAbschnitt(anker: "a", pfad: [], text: "selten")]),
            AblageIndex.vorbereiten(dat: "dat_b", abschnitte: [AblageAbschnitt(anker: "b", pfad: [], text: "haeufig")]),
            AblageIndex.vorbereiten(dat: "dat_c", abschnitte: [AblageAbschnitt(anker: "c", pfad: [], text: "haeufig")]),
        ])
        let z = drei.rohzeilen("selten")
        let idf = log((3.0 - 1.0 + 0.5) / (1.0 + 0.5))
        XCTAssertEqual(z[0].bm25, -idf * (1.0 * 2.2) / (1.0 + 1.2 * (1 - 0.75 + 0.75 * 1.0 / 1.0)), accuracy: 1e-12)
    }

    /// Doppelte Wörter der Anfrage zählen doppelt (FTS5: je Wort eine Phrase); Stoppwörter nicht.
    func testDoppelteAnfragewoerterZaehlenDoppelt() {
        let ix = AblageIndex([
            AblageIndex.vorbereiten(dat: "dat_a", abschnitte: [AblageAbschnitt(anker: "a", pfad: [], text: "rot grün blau")]),
            AblageIndex.vorbereiten(dat: "dat_b", abschnitte: [AblageAbschnitt(anker: "b", pfad: [], text: "gelb")]),
            AblageIndex.vorbereiten(dat: "dat_c", abschnitte: [AblageAbschnitt(anker: "c", pfad: [], text: "gelb schwarz")]),
            AblageIndex.vorbereiten(dat: "dat_d", abschnitte: [AblageAbschnitt(anker: "d", pfad: [], text: "weiß")]),
        ])
        let einfach = ix.rohzeilen("rot")[0].bm25
        XCTAssertEqual(ix.rohzeilen("rot rot")[0].bm25, 2 * einfach, accuracy: 1e-12)
        XCTAssertEqual(ix.rohzeilen("rot der rot")[0].bm25, 2 * einfach, accuracy: 1e-12)
    }

    /// Je Datei höchstens drei Stellen; 60 Kandidaten; Dateien nach ihrer besten Stelle.
    func testKappungJeDateiUndKandidaten() {
        let viele = (1...10).map { AblageAbschnitt(anker: "zeile=\($0)", pfad: [], text: "Stichwort Nummer \($0)") }
        let ix = AblageIndex([AblageIndex.vorbereiten(dat: "dat_a", abschnitte: viele),
                              AblageIndex.vorbereiten(dat: "dat_b", abschnitte: [AblageAbschnitt(anker: "x", pfad: [], text: "Stichwort einzeln")])])
        let st = ix.kandidatenstellen("Stichwort")
        XCTAssertEqual(st.filter { $0.dat == "dat_a" }.count, 3, "höchstens drei Stellen je Datei")
        XCTAssertEqual(st.count, 4)
        let t = ix.suche("Stichwort")
        XCTAssertEqual(t.count, 2)
        XCTAssertLessThanOrEqual(t.first { $0.dat == "dat_a" }!.stellen.count, 3)
        // Kandidatenzahl: 100 Dateien mit je einem Stück liefern höchstens 60.
        let hundert = (1...100).map { AblageIndex.vorbereiten(dat: String(format: "dat_%03d", $0), abschnitte: [AblageAbschnitt(anker: "a", pfad: [], text: "Wort \($0)")]) }
        XCTAssertEqual(AblageIndex(hundert).kandidatenstellen("Wort").count, 60)
        // Rang: erste Stelle 0,5 (wie am Desktop: 61/(2·61)).
        XCTAssertEqual(AblageIndex(hundert).kandidatenstellen("Wort")[0].rang, 0.5, accuracy: 1e-12)
    }

    /// Ein Anfragewort, das der Tokenizer in mehrere Wörter zerlegt (hier: Hindi mit Vokalzeichen), wird als Phrase gesucht.
    func testPhraseAusMehrerenTokenizerWoertern() {
        // „हिंदी“: für Rust ein Wort (alle Zeichen sind alphabetisch), für den Tokenizer zerfällt es an den Vokalzeichen.
        let wort = "\u{939}\u{93F}\u{902}\u{926}\u{940}"
        XCTAssertEqual(AblageSuchtext.inhaltsWoerter(wort).count, 1)
        let teile = AblageSuchtext.fts5Woerter(wort)
        XCTAssertEqual(teile.count, 2, "der Tokenizer zerlegt das Wort")
        let ix = AblageIndex([
            AblageIndex.vorbereiten(dat: "dat_a", abschnitte: [AblageAbschnitt(anker: "a", pfad: [], text: "Das Wort \(wort) steht hier.")]),
            AblageIndex.vorbereiten(dat: "dat_b", abschnitte: [AblageAbschnitt(anker: "b", pfad: [], text: teile.reversed().joined(separator: " "))]),
        ])
        XCTAssertEqual(ix.suche(wort).map(\.dat), ["dat_a"], "nur die Folge in der Reihenfolge ist ein Treffer")
    }

    // MARK: Faltung ß → ss und weitere Zeichen (lva#463)

    private func b(_ s: String) -> [UInt8] { Array(s.utf8) }

    /// Index UND Anfrage falten; Anzeigetext und Stück bleiben das Original (ß, Æ, Ł wie abgelegt).
    func testFaltungWirktBeiIndexUndAnfrageStueckBleibtOriginal() {
        let text = "Die Straße der Pariser Kommune. Gäste aus Łódź und Ærø: Ørsted-Allee, Fuß-Ball."
        let ix = AblageIndex([AblageIndex.vorbereiten(dat: "dat_a", abschnitte: [AblageAbschnitt(anker: "zeile=1", pfad: [], text: text)])])
        for frage in ["Strasse", "STRASSE", "straße", "Straße", "StraSSe"] {
            let t = ix.suche(frage)
            XCTAssertEqual(t.map(\.dat), ["dat_a"], frage)
            XCTAssertEqual(t.first?.stellen.map { b($0.auszug) }, [b(text)], "Auszug bleibt das Original (\(frage))")
        }
        for frage in ["Lodz", "lódź", "Aero", "Orsted", "Fuss", "Fuß", "Fuss Ball", "Ørsted"] { XCTAssertFalse(ix.suche(frage).isEmpty, frage) }
        // Das Stück selbst trägt den Originaltext; die Länge zählt Wörter des gefalteten Texts (gleich: Faltung ändert keine Wortgrenze).
        let st = ix.stuecke(dat: "dat_a")
        XCTAssertEqual(st.map { b($0.text) }, [b(text)])
        XCTAssertEqual(ix.laenge(dat: "dat_a", seite: 1), AblageSuchtext.fts5Woerter(AblageSuchtext.ftsNormalisieren(text)).count)
        XCTAssertEqual(ix.laenge(dat: "dat_a", seite: 1), AblageSuchtext.fts5Woerter(text).count, "ß → ss, Æ → ae usw. ändern keine Wortgrenze")
    }

    /// Reihenfolge der Anfrage wie `rank`: `content_tokens` (klein, Stoppwörter), danach Faltung, danach Tokenizer. `anfrageWoerter` selbst faltet
    /// nicht (die Wörter der Fixture bleiben „straße“); Großbuchstaben wie ẞ werden erst klein (ß), dann zu „ss“.
    func testAnfrageFaltetNachKleinschreibungUndStoppwoertern() {
        XCTAssertEqual(AblageIndex.anfrageWoerter("Straße Der GROẞE"), ["straße", "große"], "Wörter der Anfrage sind nicht gefaltet")
        let ix = AblageIndex([AblageIndex.vorbereiten(dat: "dat_a", abschnitte: [AblageAbschnitt(anker: "a", pfad: [], text: "Grosse Strasse")])])
        XCTAssertEqual(ix.suche("GROẞE").map(\.dat), ["dat_a"])
        XCTAssertEqual(ix.suche("Große Straße").map(\.dat), ["dat_a"])
        XCTAssertEqual(ix.suche("Große Straße").first?.stellen.count, 1)
        // Zwei Wörter der Anfrage = zwei Phrasen: genau die doppelte Punktzahl eines Worts.
        XCTAssertEqual(ix.rohzeilen("Strasse Strasse")[0].bm25, 2 * ix.rohzeilen("Straße")[0].bm25, accuracy: 1e-12)
        // Stoppwörter fallen vor der Faltung heraus (wie `content_tokens` vor `fts_normalisieren`).
        XCTAssertEqual(ix.suche("der die das"), [])
    }

    /// Codepunktgenau: kein NFC/NFD, keine Sonderbehandlung von Zeichenfolgen; nur die 21 Zeichen der Tabelle ändern sich.
    func testFtsNormalisierenCodepunktgenauUndGrenzfaelle() {
        XCTAssertEqual(AblageSuchtext.ftsNormalisieren(""), "")
        XCTAssertEqual(b(AblageSuchtext.ftsNormalisieren("ASCII only 123, ÄÖÜ")), b("ASCII only 123, ÄÖÜ"))
        // Zerlegt bleibt zerlegt, vorzusammengesetzt bleibt vorzusammengesetzt.
        let zerlegt = "e\u{301} a\u{308}", fertig = "\u{E9} \u{E4}"
        XCTAssertEqual(AblageSuchtext.ftsNormalisieren(zerlegt).unicodeScalars.map(\.value), zerlegt.unicodeScalars.map(\.value))
        XCTAssertEqual(AblageSuchtext.ftsNormalisieren(fertig).unicodeScalars.map(\.value), fertig.unicodeScalars.map(\.value))
        // ß mit angehängtem Diakritikum: nur das ß wird ersetzt, das Zeichen dahinter bleibt (der Tokenizer entfernt es).
        XCTAssertEqual(AblageSuchtext.ftsNormalisieren("\u{DF}\u{308}").unicodeScalars.map(\.value), [0x73, 0x73, 0x308])
        XCTAssertEqual(AblageSuchtext.fts5Woerter(AblageSuchtext.ftsNormalisieren("Stra\u{DF}\u{308}e")), ["strasse"])
        // Nicht in der Tabelle: ſ, İ, ǅ, ﬁ, ŉ, ŋ, ŧ, ŀ, ĸ bleiben (FTS5 faltet sie selbst oder gar nicht).
        for z in ["\u{17F}", "\u{130}", "\u{1C5}", "\u{FB01}", "\u{149}", "\u{14B}", "\u{167}", "\u{140}", "\u{138}"] {
            XCTAssertEqual(AblageSuchtext.ftsNormalisieren(z).unicodeScalars.map(\.value), z.unicodeScalars.map(\.value), z)
        }
        // Alle 21 einzeln und in Folge.
        var alle = "", soll = ""
        for (von, nach) in AblageSuchtext.ftsFaltung {
            XCTAssertEqual(AblageSuchtext.ftsNormalisieren(String(Character(von))), nach)
            alle.unicodeScalars.append(von); soll += nach
        }
        XCTAssertEqual(b(AblageSuchtext.ftsNormalisieren(alle)), b(soll))
        // Großbuchstaben ergeben einen kleinen Ersatz (wie `FTS_FALTUNG`: Æ → ae, Þ → th, Ĳ → ij).
        XCTAssertEqual(AblageSuchtext.ftsNormalisieren("\u{C6}\u{DE}\u{132}\u{1E9E}"), "aethijss")
    }

    /// Große Eingabe (voller Speicher): das Ergebnis ist in UTF-8 nie länger als die Eingabe (ß → ss, þ → th gleich lang, ẞ → ss und ø → o
    /// kürzer), höchstens zwei Skalare je Ersatz. Die Zeit wird nur großzügig begrenzt (20 s; gemessen etwa 0,3 s im Debug-Bau): der Test soll
    /// ein überlineares Anhängen an eine `UnicodeScalarView` fangen (das brauchte hier 110 s), kein Tempo prüfen.
    func testFtsNormalisierenGrosseEingabeNieLaenger() {
        let n = 400_000
        let gross = String(repeating: "\u{DF}\u{FE}x", count: n)
        let t0 = ContinuousClock.now
        let aus = AblageSuchtext.ftsNormalisieren(gross)
        let dauer = ContinuousClock.now - t0
        XCTAssertLessThan(dauer, .seconds(20), "1,2 Millionen Skalare dürfen nicht überlinear werden: \(dauer)")
        XCTAssertEqual(aus.unicodeScalars.count, n * 5)
        XCTAssertEqual(aus.utf8.count, n * 5)
        XCTAssertLessThanOrEqual(aus.utf8.count, gross.utf8.count)
        XCTAssertTrue(AblageSuchtext.ftsFaltung.allSatisfy { $0.nach.unicodeScalars.count <= 2 && !$0.nach.isEmpty && $0.nach.utf8.count <= String($0.von).utf8.count })
        let kuerzer = AblageSuchtext.ftsNormalisieren(String(repeating: "\u{1E9E}\u{F8}", count: 1000))
        XCTAssertEqual(kuerzer, String(repeating: "sso", count: 1000))
    }

    /// Nebenläufigkeit: reine Funktion über eine einmal aufgebaute Tabelle; viele Threads gleichzeitig liefern dasselbe.
    func testFtsNormalisierenNebenlaeufigGleichesErgebnis() async {
        let text = "Straße Ærø Łódź Þór Đorđe Ħal ıstanbul Ĳssel Œuvre ẞ"
        let soll = b(AblageSuchtext.ftsNormalisieren(text))
        XCTAssertEqual(String(decoding: soll, as: UTF8.self), "Strasse aero lódź thór dorde hal istanbul ijssel oeuvre ss")
        let ergebnisse = await withTaskGroup(of: [UInt8].self) { gruppe -> [[UInt8]] in
            for _ in 0..<64 {
                gruppe.addTask {
                    var letzte: [UInt8] = []
                    for _ in 0..<200 { letzte = Array(AblageSuchtext.ftsNormalisieren(text).utf8) }
                    return letzte
                }
            }
            var aus: [[UInt8]] = []
            for await e in gruppe { aus.append(e) }
            return aus
        }
        XCTAssertEqual(ergebnisse.count, 64)
        XCTAssertTrue(ergebnisse.allSatisfy { $0 == soll })
    }

    /// Ende zu Ende über den Suchdienst (Index-Tor, Zwischenspeicher): „Strasse“ findet „Straße“ und umgekehrt; der Auszug bleibt das Original.
    func testDienstSucheFaltetSsFindetBeideSchreibweisen() async throws {
        var a = Aufbau()
        datei(&a, 1, name: "a.txt", fassungen: [fassung(1, text: "Die Straße ist lang.")])
        datei(&a, 2, name: "b.txt", fassungen: [fassung(2, text: "Die Strasse ist kurz.")])
        let (z, _) = try einspielen(a)
        for frage in ["Strasse", "Straße", "STRASSE"] {
            let (t, d) = await suche(z, frage)
            XCTAssertEqual(Set(dats(t)), [W.datId(1), W.datId(2)], frage)
            let eins = try XCTUnwrap(t.first { $0.dat == W.datId(1) })
            XCTAssertEqual(eins.stellen.map { b($0.auszug) }, [b("Die Straße ist lang.")], "Auszug bleibt das Original")
            // Zweiter Aufruf aus dem Zwischenspeicher (gleiche Vorbereitung): dasselbe Ergebnis.
            await d.aktualisieren(zustand: z, speicher: speicher)
            let wieder = await d.suche(frage)
            XCTAssertEqual(wieder, t)
        }
    }

    /// Migration (lva#463): Der iPhone-Index liegt nur im Speicher (D5a-4: Neuaufbau bei jedem Start aus den Textfassungen), es gibt keinen
    /// persistierten FTS-Text und keinen Zwischenspeicher auf der Platte, also keine Umstellung wie `meta.fts_fmt` am Desktop. Dieser Test
    /// hält das über die Typen fest: Index, Vorbereitung und Suchdienst (der Zwischenspeicher) sind weder `Encodable` noch `Decodable`. Wer
    /// sie je auf die Platte legt (meist beginnt das mit `Codable`), stolpert hier und muss eine Fassungsmarke (Faltung v2) einführen und den
    /// Altbestand atomar umstellen (erst schreiben, dann umbenennen), sonst findet ein alter Cache „Strasse“ nie. Grenze: Wer ohne
    /// `Codable` von Hand schreibt, wird hier nicht erkannt; der Kommentar an `AblageSuchdienst` und `AblageIndex` nennt die Pflicht ebenfalls.
    func testIndexUndVorbereitungSindNichtSerialisierbar() {
        func codierbar<T>(_ typ: T.Type) -> Bool { typ is any Encodable.Type || typ is any Decodable.Type }
        XCTAssertTrue(codierbar(String.self), "Kontrolle: die Prüfung erkennt Codable")
        XCTAssertFalse(codierbar(AblageIndex.self), "AblageIndex ist codierbar: Index persistiert? Dann Fassungsmarke und Umstellung nötig (lva#463)")
        XCTAssertFalse(codierbar(AblageIndex.Vorbereitet.self), "Vorbereitet ist codierbar: Zwischenspeicher persistiert? Dann Fassungsmarke und Umstellung nötig (lva#463)")
        XCTAssertFalse(codierbar(AblageIndex.VorbereitetesStueck.self), "VorbereitetesStueck ist codierbar: Zwischenspeicher persistiert? Fassungsmarke nötig (lva#463)")
        XCTAssertFalse(codierbar(AblageSuchdienst.self), "AblageSuchdienst ist codierbar: Zwischenspeicher persistiert? Fassungsmarke nötig (lva#463)")
    }

    /// Gemischter Bestand wie in der Fixture (`gesperrt`), durch den echten Index-Tor: die zwölf Korpusdateien leben, die drei Dateien des
    /// Desktop-Papierkorbs liegen mit ihrem Text auf der Platte, sind aber als Papierkorb markiert. Der Tor lässt sie nie in den Index; N, n
    /// und avgdl, damit jeder Rohrang, sind die des Korpus allein und gleich dem, was der Desktop für seinen Leser rechnet.
    func testGesperrterBestandDurchDenTorGleichRohrangWieDesktop() async throws {
        let data = try Data(contentsOf: AblageIndexParitaetTests.fixtureURL())
        let root = try XCTUnwrap(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        let korpus = try XCTUnwrap(root["korpus"] as? [[String: Any]])
        let stat = try XCTUnwrap(root["statistik"] as? [String: Any])
        let gesperrt = try XCTUnwrap(root["gesperrt"] as? [String: Any])
        let gDateien = try XCTUnwrap(gesperrt["dateien"] as? [[String: Any]])
        let gFragen = try XCTUnwrap(gesperrt["fragen"] as? [[String: Any]])
        XCTAssertEqual(korpus.count, 12)
        XCTAssertEqual(gDateien.count, 3)

        var a = Aufbau()
        var namen: [String: String] = [:]
        // Nummern in der Reihenfolge der Dateinamen (Bytes), wie der Parität-Test: die Kennung entscheidet nur bei Gleichstand im Rang.
        for (i, k) in korpus.sorted(by: { Array(($0["name"] as! String).utf8).lexicographicallyPrecedes(Array(($1["name"] as! String).utf8)) }).enumerated() {
            let n = i + 1, name = k["name"] as! String
            let ab = (k["abschnitte"] as! [[String: Any]]).map { (anker: $0["anker"] as! String, text: $0["text"] as! String) }
            datei(&a, n, name: name, fassungen: [fassungMitAbschnitten(n, ab)])
            namen[W.datId(n)] = name
        }
        for (i, g) in gDateien.enumerated() {
            datei(&a, 20 + i, name: g["name"] as! String, fassungen: [fassungMitAbschnitten(20 + i, [("zeile=1", g["text"] as! String)])], korb: 5_000)
        }
        let (z, v) = try einspielen(a)
        for i in gDateien.indices {   // Kontrolle: der Text der gesperrten Dateien liegt wirklich auf der Platte (Altlast), der Tor hält ihn fern.
            let t = try XCTUnwrap(z.bestand.dateien[W.datId(20 + i)]?.fassungen[0].texte[0])
            XCTAssertTrue(v.text(W.datId(20 + i), t.textrev), "Text von Gesperrt \(i) liegt auf der Platte")
            XCTAssertFalse(AblageAnzeige.darfIndiziert(z, dat: W.datId(20 + i)), "Papierkorb: nie zugelassen")
        }

        let d = AblageSuchdienst()
        let abdeckung = await d.aktualisieren(zustand: z, speicher: speicher)
        let ix = await d.index
        XCTAssertEqual(abdeckung.imBereich, korpus.count, "der Papierkorb ist nicht im Bereich")
        XCTAssertEqual(abdeckung.durchsucht, korpus.count)
        XCTAssertEqual(ix.anzahlDateien, korpus.count)
        for i in gDateien.indices { XCTAssertEqual(ix.stuecke(dat: W.datId(20 + i)).count, 0, "gesperrte Datei \(i) ist nie ein Stück") }
        // Dieselben Zahlen wie der Index des Desktops über den Korpus (N und Wörter, Grundlage von idf und avgdl).
        XCTAssertEqual(ix.anzahlStuecke, stat["stuecke"] as? Int)
        XCTAssertEqual(ix.anzahlWoerter, stat["tokens"] as? Int)

        XCTAssertGreaterThanOrEqual(gFragen.count, 27)
        for f in gFragen {
            let frage = try XCTUnwrap(f["frage"] as? String)
            let soll = try XCTUnwrap(f["bm25"] as? [[String: Any]])
            let roh = ix.rohzeilen(frage)
            XCTAssertEqual(roh.count, f["bm25_anzahl"] as? Int, "Zeilen \(frage)")
            for (i, s) in soll.enumerated() where i < roh.count {
                XCTAssertEqual(namen[roh[i].dat], s["name"] as? String, "Reihenfolge \(frage) #\(i)")
                XCTAssertEqual(roh[i].seite, s["seite"] as? Int, "Seite \(frage) #\(i)")
                let w = s["wert"] as! Double
                XCTAssertLessThanOrEqual(abs(roh[i].bm25 - w), 1e-9 * max(abs(roh[i].bm25), abs(w), 1e-300), "bm25 \(frage) #\(i): \(roh[i].bm25) statt \(w)")
            }
        }
    }

    /// Eine Fassung mit mehreren Abschnitten (Anker wie in der Fixture), für den gemischten Bestand.
    private func fassungMitAbschnitten(_ n: Int, _ abschnitte: [(anker: String, text: String)]) -> (AblageFassung, [UInt8]) {
        var f = W.fassung(n, nil, inhalt: "Original \(n)", typ: "txt", zeit: 1_000)
        let t = W.text(zu: f, abschnitte: abschnitte)
        f.texte = [t.eintrag]
        return (f, t.bytes)
    }
}
#endif
