import XCTest
@testable import VoiceCore
#if canImport(CryptoKit)

// Fixrunde zu PR #461 (D5a-4, lva#441): die Ansicht der Ablage (Übersicht, Abdeckung, Suchindex) wird nie von einem veralteten oder
// fremden Ergebnis überschrieben, und der Baum der Seite wird nicht bei jeder Auswertung neu berechnet. Nur synthetische Namen.
//
// Fehlerfälle dieses Pakets und ihre Tests:
// - Zwei Aktualisierungen gleichzeitig (Lauf und Laden eines Originals): der Akteur liest den Zustand erst, wenn der Aufruf dran ist,
//   und weist einen Aufruf mit kleinerer Nummer ab (`testVeralteterAufrufWirdAbgewiesen`, `testParalleleAufrufeEndenImNeuestenStand`).
//   Die Seite übernimmt nur das Ergebnis der neuesten Aktualisierung (`testStandsfolgeNurDieNeuesteSetzt`).
// - Abmelden und Neuanmelden, auch mit demselben Konto: Epoche und Kontokennung (`testStandsfolgeAbmeldenUndNeuanmeldenMitSelbemKonto`,
//   `testStandsfolgeAnderesKonto`, `testNachVerlassenNimmtDerSuchdienstNichtsAltesMehr`).
// - Kontingent hängt (60 s Zeitüberschreitung): es läuft in eigener Aufgabe und wird nur bei gleichem Konto und gleicher Epoche
//   übernommen (`testStandsfolgeKontoGiltAuchOhneNeueste`).
// - Abbruch/Fehler beim Lesen: ein fehlender oder unlesbarer Zustand ergibt eine leere Ansicht, nie einen Absturz
//   (`testFehlenderZustandIstLeereAnsicht`).

private typealias W = AblageWelt

final class AblageAnsichtTests: XCTestCase {
    private var tmp: URL!
    private var speicher: AblageSpeicher!
    private let fm = FileManager.default
    private let kennung = W.konto.kennung

    override func setUpWithError() throws {
        tmp = fm.temporaryDirectory.appendingPathComponent("ablage-ansicht-\(UUID().uuidString)", isDirectory: true)
        try fm.createDirectory(at: tmp, withIntermediateDirectories: true)
        speicher = AblageSpeicher(wurzel: tmp.appendingPathComponent("ablage", isDirectory: true))
    }

    override func tearDownWithError() throws { try? fm.removeItem(at: tmp) }

    // MARK: Hilfen

    private let kontrolle = "Zebrafink"

    /// Schreibt den Zustand mit einer Datei (Text liegt auf der Platte), im Papierkorb oder nicht, auf die Platte.
    @discardableResult
    private func zustandSchreiben(imKorb: Bool) throws -> AblageZustand {
        var f = W.fassung(1, nil, inhalt: "Original 1", typ: "txt")
        let t = W.text(zu: f, abschnitte: [("zeile=1", "Das \(kontrolle) singt.")])
        f.texte = [t.eintrag]
        let d = W.datei(1, ordner: W.ordId(1), name: "a.txt", fassungen: [f], korb: imKorb ? 9_000 : nil)
        var z = W.neuerZustand()
        W.einspielen(&z, ordner: [W.remote(ordner: W.ordner(1, name: "Ablage", klasse: "internal"))], dateien: [W.remote(datei: d)])
        try speicher.textEinlegen(bestand: z.bestand, dat: d.id, fas: f.fas, textrev: t.eintrag.textrev, bytes: t.bytes)
        try speicher.speichern(z)
        return z
    }

    private func dats(_ t: [AblageTreffer]) -> [String] { t.map(\.dat) }

    // MARK: Standsfolge (Nummer, Epoche, Konto)

    func testStandsfolgeNurDieNeuesteSetzt() {
        var s = AblageStandsfolge()
        let m1 = s.beginnen(konto: "A")
        XCTAssertTrue(s.gilt(m1, konto: "A"), "allein: gilt")
        let m2 = s.beginnen(konto: "A")
        XCTAssertFalse(s.gilt(m1, konto: "A"), "eine neuere Aktualisierung hat begonnen: die ältere setzt nichts mehr")
        XCTAssertTrue(s.gilt(m2, konto: "A"))
        XCTAssertNotEqual(m1.nummer, m2.nummer)
    }

    func testStandsfolgeAbmeldenUndNeuanmeldenMitSelbemKonto() {
        var s = AblageStandsfolge()
        let alt = s.beginnen(konto: "A")
        let schwelle = s.verwerfen()
        XCTAssertFalse(s.gilt(alt, konto: "A"), "nach dem Abmelden gilt nichts, was vorher begann")
        XCTAssertFalse(s.gilt(alt, konto: nil))
        XCTAssertGreaterThan(schwelle, alt.nummer, "die Schwelle liegt über allen bisherigen Nummern")
        let neu = s.beginnen(konto: "A")
        XCTAssertGreaterThan(neu.nummer, schwelle)
        XCTAssertFalse(s.gilt(alt, konto: "A"), "auch nach dem Neuanmelden mit demselben Konto bleibt das alte Ergebnis ungültig")
        XCTAssertTrue(s.gilt(neu, konto: "A"))
    }

    func testStandsfolgeAnderesKonto() {
        var s = AblageStandsfolge()
        let m = s.beginnen(konto: "A")
        XCTAssertFalse(s.gilt(m, konto: "B"), "anderes Konto angemeldet: nichts übernehmen")
        XCTAssertFalse(s.gilt(m, konto: nil), "abgemeldet: nichts übernehmen")
        XCTAssertTrue(s.gilt(m, konto: "A"))
    }

    func testStandsfolgeKontoGiltAuchOhneNeueste() {
        var s = AblageStandsfolge()
        let m = s.kontoMarke(konto: "A")
        _ = s.beginnen(konto: "A")
        _ = s.beginnen(konto: "A")
        XCTAssertTrue(s.kontoGilt(m, konto: "A"), "Kontingent: es zählt nur Konto und Epoche, nicht die Nummer")
        XCTAssertFalse(s.kontoGilt(m, konto: "B"))
        XCTAssertFalse(s.kontoGilt(m, konto: nil))
        s.verwerfen()
        XCTAssertFalse(s.kontoGilt(m, konto: "A"), "nach Abmelden und Neuanmelden: nicht übernehmen")
        XCTAssertTrue(s.kontoGilt(s.kontoMarke(konto: "A"), konto: "A"))
    }

    // MARK: Suchdienst: Ansicht mit Nummer

    func testAnsichtLiefertUebersichtAbdeckungUndIndex() async throws {
        try zustandSchreiben(imKorb: false)
        let d = AblageSuchdienst()
        let a = await d.ansichtAktualisieren(speicher: speicher, konto: kennung, nummer: 1)
        let ansicht = try XCTUnwrap(a)
        XCTAssertEqual(ansicht.uebersicht.dateien.map(\.id), [W.datId(1)])
        XCTAssertEqual(ansicht.abdeckung.durchsucht, 1)
        let t = await d.suche(kontrolle)
        XCTAssertEqual(dats(t), [W.datId(1)])
    }

    /// Das Fehlerbild des Befunds: die ältere Aktualisierung (Stand vor dem Papierkorb) trifft nach der neueren ein. Sie darf weder
    /// die Übersicht noch den Index zurücksetzen (sonst bliebe die Datei im Papierkorb sichtbar und durchsuchbar).
    func testVeralteterAufrufWirdAbgewiesen() async throws {
        let d = AblageSuchdienst()
        try zustandSchreiben(imKorb: false)
        let erste = await d.ansichtAktualisieren(speicher: speicher, konto: kennung, nummer: 1)
        XCTAssertNotNil(erste)
        try zustandSchreiben(imKorb: true)
        let neu = await d.ansichtAktualisieren(speicher: speicher, konto: kennung, nummer: 3)
        XCTAssertEqual(neu?.uebersicht.dateien.first?.imKorb, true)
        // Nummer 2 kam früher an die Reihe der Seite, aber später im Akteur an.
        let alt = await d.ansichtAktualisieren(speicher: speicher, konto: kennung, nummer: 2)
        XCTAssertNil(alt, "überholt: nichts liefern")
        let t = await d.suche(kontrolle)
        XCTAssertEqual(t, [], "Papierkorb: bleibt aus dem Index")
        let ix = await d.index
        XCTAssertEqual(ix.anzahlDateien, 0)
        let ab = await d.abdeckung
        XCTAssertEqual(ab.durchsucht, 0)
    }

    /// Viele Aufrufe gleichzeitig, der Zustand ändert sich zwischen den Aufrufen: der Index endet immer im Stand des letzten.
    func testParalleleAufrufeEndenImNeuestenStand() async throws {
        for runde in 0..<6 {
            let d = AblageSuchdienst()
            let sp = speicher!, k = kennung
            var tasks: [Task<AblageAnsicht?, Never>] = []
            var korb = false
            for nummer in 1...16 {
                korb = nummer % 2 == 0                         // 16 ist gerade: Endstand im Papierkorb
                try zustandSchreiben(imKorb: korb)
                tasks.append(Task.detached(priority: runde % 2 == 0 ? .utility : .userInitiated) {
                    await d.ansichtAktualisieren(speicher: sp, konto: k, nummer: nummer)
                })
            }
            var ergebnisse: [AblageAnsicht?] = []
            for t in tasks { ergebnisse.append(await t.value) }
            XCTAssertNotNil(ergebnisse[15], "der neueste Aufruf liefert immer")
            XCTAssertEqual(ergebnisse[15]?.uebersicht.dateien.first?.imKorb, true, "Runde \(runde)")
            let t = await d.suche(kontrolle)
            XCTAssertEqual(t, [], "Runde \(runde): der Endstand liegt im Papierkorb, der Index folgt ihm")
        }
    }

    /// Abmelden: `leeren(nummer:)` setzt die Schwelle. Was davor begann, kommt nicht mehr an; das Neue schon.
    func testNachVerlassenNimmtDerSuchdienstNichtsAltesMehr() async throws {
        try zustandSchreiben(imKorb: false)
        let d = AblageSuchdienst()
        _ = await d.ansichtAktualisieren(speicher: speicher, konto: kennung, nummer: 1)
        await d.leeren(nummer: 5)
        let leer = await d.index
        XCTAssertEqual(leer.anzahlStuecke, 0)
        let alt = await d.ansichtAktualisieren(speicher: speicher, konto: kennung, nummer: 4)
        XCTAssertNil(alt, "begann vor dem Abmelden")
        let nochLeer = await d.index
        XCTAssertEqual(nochLeer.anzahlStuecke, 0, "der alte Aufruf hat den Index nicht neu gefüllt")
        let neu = await d.ansichtAktualisieren(speicher: speicher, konto: kennung, nummer: 6)
        XCTAssertNotNil(neu, "das neue Konto kommt durch")
        let t = await d.suche(kontrolle)
        XCTAssertEqual(dats(t), [W.datId(1)])
    }

    /// Kommt das späte `leeren` des alten Kontos erst nach der ersten Aktualisierung des neuen an, räumt es deren Index nicht weg.
    func testSpaetesLeerenRaeumtDenIndexDesNeuenKontosNichtWeg() async throws {
        try zustandSchreiben(imKorb: false)
        let d = AblageSuchdienst()
        _ = await d.ansichtAktualisieren(speicher: speicher, konto: kennung, nummer: 7)
        await d.leeren(nummer: 5)
        let t = await d.suche(kontrolle)
        XCTAssertEqual(dats(t), [W.datId(1)], "der Index des neueren Aufrufs bleibt")
        let ab = await d.abdeckung
        XCTAssertEqual(ab.durchsucht, 1)
    }

    func testFehlenderZustandIstLeereAnsicht() async throws {
        let d = AblageSuchdienst()
        let a = await d.ansichtAktualisieren(speicher: speicher, konto: kennung, nummer: 1)
        let ansicht = try XCTUnwrap(a)
        XCTAssertEqual(ansicht.uebersicht.dateien, [])
        XCTAssertEqual(ansicht.abdeckung, AblageAbdeckung())
        // Zustand eines anderen Kontos: ebenfalls leer (nie Namen des anderen Kontos).
        try zustandSchreiben(imKorb: false)
        let b = await d.ansichtAktualisieren(speicher: speicher, konto: "anderes-konto", nummer: 2)
        XCTAssertEqual(b?.uebersicht.dateien, [])
        let t = await d.suche(kontrolle)
        XCTAssertEqual(t, [])
    }

    // MARK: Baum-Zwischenspeicher (Befund 3: nicht bei jeder Auswertung neu sortieren)

    private func uebersicht(_ namen: [String], korb: [String] = []) -> AblageUebersicht {
        var z = W.neuerZustand()
        var dateien: [AblageDatei] = []
        for (i, n) in (namen + korb).enumerated() {
            dateien.append(W.datei(i + 1, ordner: W.ordId(1), name: n, fassungen: [W.fassung(i + 1, nil, inhalt: "x\(i)")], korb: i >= namen.count ? 5_000 : nil))
        }
        W.einspielen(&z, ordner: [W.remote(ordner: W.ordner(1, name: "Ablage", klasse: "internal")), W.remote(ordner: W.ordner(2, name: "Zweiter", klasse: "internal"))],
                     dateien: dateien.map { W.remote(datei: $0) })
        return AblageAnzeige.uebersicht(z, vorhanden: AblageVorhanden())
    }

    func testBaumZwischenspeicherRechnetNurBeiAenderungVonVersionOderEinklappen() {
        let u = uebersicht(["b.txt", "a.txt", "c10.txt", "c9.txt"], korb: ["z.txt", "y.txt"])
        let z = AblageBaumZwischenspeicher()
        XCTAssertEqual(z.berechnungen, 0)
        let a = z.ansicht(u, version: 1, eingeklappt: [])
        XCTAssertEqual(a.zeilen, AblageBaum.zeilen(u))
        XCTAssertEqual(a.korb, AblageBaum.papierkorb(u))
        XCTAssertEqual(a.korb.map(\.name), ["y.txt", "z.txt"])
        XCTAssertEqual(z.berechnungen, 1)
        for _ in 0..<20 { _ = z.ansicht(u, version: 1, eingeklappt: []) }
        XCTAssertEqual(z.berechnungen, 1, "gleiche Version und gleicher Klappstand: aus dem Zwischenspeicher")
        // Einklappen: neu berechnen, und das Ergebnis ist das des Baums.
        let zu = z.ansicht(u, version: 1, eingeklappt: [W.ordId(1)])
        XCTAssertEqual(zu.zeilen, AblageBaum.zeilen(u, eingeklappt: [W.ordId(1)]))
        XCTAssertEqual(z.berechnungen, 2)
        for _ in 0..<20 { _ = z.ansicht(u, version: 1, eingeklappt: [W.ordId(1)]) }
        XCTAssertEqual(z.berechnungen, 2)
        // Neue Übersicht (Version zählt hoch): neu berechnen.
        let u2 = uebersicht(["neu.txt"])
        let b = z.ansicht(u2, version: 2, eingeklappt: [W.ordId(1)])
        XCTAssertEqual(b.zeilen, AblageBaum.zeilen(u2, eingeklappt: [W.ordId(1)]))
        XCTAssertEqual(b.korb, [])
        XCTAssertEqual(z.berechnungen, 3)
        // Zurück zum alten Klappstand mit der neuen Version: wieder neu (es wird nur der letzte Stand gehalten).
        _ = z.ansicht(u2, version: 2, eingeklappt: [])
        XCTAssertEqual(z.berechnungen, 4)
    }

    // MARK: Index: Zwischenspeicher hält Nummern statt Wörter (Befund 4)

    func testVorbereitetHaeltJeBegriffNurEinmalUndNummernStattWoerter() {
        let text = String(repeating: "alpha beta gamma beta alpha. ", count: 40)
        let v = AblageIndex.vorbereiten(dat: "dat_a", abschnitte: [AblageAbschnitt(anker: "zeile=1", pfad: [], text: text),
                                                                    AblageAbschnitt(anker: "zeile=2", pfad: [], text: "gamma delta")])
        XCTAssertEqual(Set(v.begriffe), ["alpha", "beta", "gamma", "delta"])
        XCTAssertEqual(v.begriffe.count, 4, "jeder Begriff einmal je Datei, nicht je Vorkommen")
        let vorkommen = v.stuecke.reduce(0) { $0 + $1.woerter.count }
        XCTAssertEqual(vorkommen, v.stuecke.reduce(0) { $0 + AblageSuchtext.fts5Woerter($1.text).count }, "die Stücke tragen alle Vorkommen als Nummern")
        XCTAssertGreaterThan(vorkommen, 200, "viele Vorkommen, aber nur vier Begriffe")
        XCTAssertTrue(v.stuecke.allSatisfy { $0.woerter.allSatisfy { Int($0) >= 0 && Int($0) < v.begriffe.count } })
        // Die Wörter eines Stücks, über die Nummern zurückgelesen, sind die des Tokenizers.
        for s in v.stuecke { XCTAssertEqual(s.woerter.map { v.begriffe[Int($0)] }, AblageSuchtext.fts5Woerter(s.text)) }
        // Der Index daraus ist derselbe wie vorher: Zahlen und Ranking.
        let ix = AblageIndex([v])
        XCTAssertEqual(ix.anzahlWoerter, vorkommen)
        XCTAssertEqual(ix.suche("delta").first?.stellen.map(\.anker), ["zeile=2"])
        XCTAssertEqual(ix.laenge(dat: "dat_a", seite: ix.stuecke(dat: "dat_a").count), 2)
    }

    /// Zwei Dateien mit denselben Wörtern in verschiedener Reihenfolge (verschiedene Nummern je Datei): der Index ordnet sie richtig zu.
    func testNummernJeDateiWerdenImIndexRichtigZugeordnet() {
        let a = AblageIndex.vorbereiten(dat: "dat_a", abschnitte: [AblageAbschnitt(anker: "a", pfad: [], text: "rot gruen blau")])
        let b = AblageIndex.vorbereiten(dat: "dat_b", abschnitte: [AblageAbschnitt(anker: "b", pfad: [], text: "blau rot gelb")])
        XCTAssertEqual(a.begriffe, ["rot", "gruen", "blau"])
        XCTAssertEqual(b.begriffe, ["blau", "rot", "gelb"])
        let ix = AblageIndex([a, b])
        XCTAssertEqual(Set(ix.suche("rot").map(\.dat)), ["dat_a", "dat_b"])
        XCTAssertEqual(ix.suche("gruen").map(\.dat), ["dat_a"])
        XCTAssertEqual(ix.suche("gelb").map(\.dat), ["dat_b"])
        XCTAssertEqual(Set(ix.suche("blau").map(\.dat)), ["dat_a", "dat_b"])
        XCTAssertEqual(ix.suche("gruen gelb").count, 2, "Oder-Verknüpfung: je ein Wort in einer der Dateien")
        XCTAssertEqual(ix.stuecke(dat: "dat_b").map(\.text), ["blau rot gelb"])
    }

    // MARK: Speicher: Vorprüfsumme mit frisch gelesener Größe (Nit)

    /// Die Größe der Vorprüfsumme muss zur frisch gelesenen Größe der Zwischendatei passen; sonst gilt sie als veraltet und der Hash
    /// wird neu berechnet (nie ein Hash von einer anderen Datei übernehmen).
    func testVorgepruefteSummeMitAbweichenderGroesseWirdNichtUebernommen() throws {
        let original = "Originalinhalt 0123456789"
        let f = W.fassung(1, nil, inhalt: original, typ: "txt")
        let d = W.datei(1, ordner: W.ordId(1), name: "a.txt", fassungen: [f])
        var z = W.neuerZustand()
        W.einspielen(&z, ordner: [W.remote(ordner: W.ordner(1, name: "Ablage", klasse: "internal"))], dateien: [W.remote(datei: d)])
        func zwischen(_ bytes: [UInt8]) throws -> URL {
            let u = speicher.zwischenpfad()
            try Data(bytes).write(to: u)
            return u
        }
        let richtigeBytes = Array(original.utf8)
        var anders = richtigeBytes
        anders[0] ^= 0x01                                         // gleiche Größe, anderer Inhalt
        // Vorprüfsumme einer anderen Größe (veraltet) mit dem Hash des richtigen Inhalts: gilt nicht, die falsche Datei wird abgelehnt.
        let veraltet = AblageOriginalPruefsumme(groesse: UInt64(richtigeBytes.count) + 5, sha256Hex: f.blob)
        XCTAssertThrowsError(try speicher.originalEinlegen(bestand: z.bestand, dat: d.id, fas: f.fas, zwischen: try zwischen(anders), vorgeprueft: veraltet)) {
            XCTAssertEqual($0 as? AblageSpeicherFehler, .passtNicht)
        }
        XCTAssertNil(speicher.originalLiegt(dat: d.id, fas: f.fas, typ: "txt"))
        // Veraltete Vorprüfsumme mit falschem Hash, aber richtiger Inhalt: der Hash wird frisch berechnet, die Datei passt.
        let veraltetFalsch = AblageOriginalPruefsumme(groesse: UInt64(richtigeBytes.count) + 5, sha256Hex: String(repeating: "0", count: 64))
        XCTAssertTrue(try speicher.originalEinlegen(bestand: z.bestand, dat: d.id, fas: f.fas, zwischen: try zwischen(richtigeBytes), vorgeprueft: veraltetFalsch))
        XCTAssertNotNil(speicher.originalLiegt(dat: d.id, fas: f.fas, typ: "txt"))
    }
}
#endif
