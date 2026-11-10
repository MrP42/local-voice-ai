import XCTest
@testable import VoiceCore

// Zusammenführen, Köpfe, Ordnerbaum, Klasse, Papierkorb und Teilnahme der Ablage (D5a-2, lva#441, Epic #395).
//
// Teil 1 sind die Permutationstests des Desktops (`ablage/zusammenfuehren.rs`, Modul `tests`) mit denselben Fällen
// und denselben erwarteten Werten, in Swift nachgebaut; die Hilfen heißen wie dort (`fas`, `datei`, `wahl`,
// `permutationen`, `falten`, `unabhaengig`, `ord`, `bestand`). Teil 2 prüft die Fehlerfälle, die ein Port
// besonders gefährden: Byte- statt Swift-Ordnung, Gleichstand im Rang, zerstörte Eingaben (Zyklen, Verweise ins
// Leere), unveränderte Eingaben, Nebenläufigkeit, Größe. Die Vektoren gegen Rust stehen in
// AblageZusammenfuehrenVektorTests. Alles synthetisch.
//
// Fehlerfälle und wo sie abgedeckt sind:
// - Nebenläufigkeit: testNebenlaeufigGleichesErgebnis (reine Funktionen auf Werttypen, kein geteilter Zustand).
// - Abbruch mitten im Vorgang: testEingabenBleibenUnveraendert (kein Teilzustand, kein I/O).
// - Voller Speicher/Datenträger: kein Zugriff darauf; Größe: testGrosseKetteTerminiert.
// - Zerstörte Eingaben: testVorgaengerZyklus…, testWahlMitUnbekanntenFassungen, testOrdnerZyklus…, testFehlender….
// - Unicode (Swift-String-Gleichheit/-Ordnung): testKarteIstByteGenau, testKennungenByteGenau, testRangOrdnungNachBytes.
// - Gleichstand im Rang: testRegisterGleichstandEntscheidetDerWert.

// MARK: - Hilfen wie im Rust-Modul `tests`

private func fas(_ id: String, _ vorgaenger: String?, _ zeit: Int64) -> AblageFassung {
    AblageFassung(fas: id, vorgaenger: vorgaenger, blob: "blob-\(id)", size: 10, originalSha256: "sha-\(id)", typ: "pdf",
                  geraet: "g_1", zeit: zeit, texte: [])
}

private func datei(_ fassungen: [AblageFassung], _ wahlen: [AblageWahl] = []) -> AblageDatei {
    AblageDatei(id: "dat_1",
                ordner: AblageRegister(wert: "ord_1", zaehler: 1, geraet: "g_1"),
                name: AblageRegister(wert: "x.pdf", zaehler: 1, geraet: "g_1"),
                klasseMin: AblageRegister(wert: "internal", zaehler: 1, geraet: "g_1"),
                nurGeraetMin: false, fassungen: fassungen, wahlen: wahlen, entfernt: [],
                blobRefs: AblageZusammenfuehren.blobRefsAus(fassungen),
                papierkorb: AblageRegister(wert: nil, zaehler: 0, geraet: "g_1"))
}

private func wahl(_ id: String, _ gewaehlt: String, _ gegen: [String], _ ersetzt: [String], _ z: UInt64, _ g: String) -> AblageWahl {
    AblageWahl(id: id, gewaehlt: gewaehlt, gegen: gegen, ersetzt: ersetzt, zaehler: z, geraet: g)
}

private func permutationen<T>(_ v: [T]) -> [[T]] {
    if v.count <= 1 { return [v] }
    var aus: [[T]] = []
    for i in v.indices {
        var rest = v
        let x = rest.remove(at: i)
        for var p in permutationen(rest) {
            p.insert(x, at: 0)
            aus.append(p)
        }
    }
    return aus
}

/// Bytegenauer Vergleichstext einer Datei (kanonische Nutzlast ohne Umordnen: Reihenfolge der Listen zählt mit).
private func kanon(_ d: AblageDatei) -> String { String(decoding: d.kanonischeNutzlast(ordnen: false), as: UTF8.self) }

private func kanon(_ b: AblageOrdnerbestand) -> [String] {
    b.eintraege.map { $0.key + "=" + String(decoding: $0.value.kanonischeNutzlast(), as: UTF8.self) }
}

private func falten(_ staende: [AblageDatei]) -> AblageDatei {
    staende.dropFirst().reduce(staende[0]) { AblageZusammenfuehren.zusammenfuehrenDatei($0, $1) }
}

/// Das Ergebnis darf nicht von der Reihenfolge abhängen (und nicht von der Gruppierung).
@discardableResult
private func unabhaengig(_ staende: [AblageDatei], file: StaticString = #filePath, line: UInt = #line) -> AblageDatei {
    let erste = falten(staende)
    for p in permutationen(staende) {
        XCTAssertEqual(kanon(falten(p)), kanon(erste), "Reihenfolge ändert das Ergebnis", file: file, line: line)
    }
    if staende.count == 3 {
        let rechts = AblageZusammenfuehren.zusammenfuehrenDatei(staende[0],
                                                                AblageZusammenfuehren.zusammenfuehrenDatei(staende[1], staende[2]))
        XCTAssertEqual(kanon(rechts), kanon(erste), "(a+b)+c ist nicht a+(b+c)", file: file, line: line)
    }
    return erste
}

private func ord(_ id: String, _ eltern: (String, UInt64, String)? = nil) -> AblageOrdner {
    AblageOrdner(
        id: id,
        eltern: eltern.map { AblageRegister<String?>(wert: $0.0, zaehler: $0.1, geraet: $0.2) } ?? AblageRegister(wert: nil, zaehler: 0, geraet: "g_0"),
        name: AblageRegister(wert: id, zaehler: 1, geraet: "g_0"),
        datenklasse: AblageRegister(wert: "erbt", zaehler: 1, geraet: "g_0"),
        nurGeraet: AblageRegister(wert: false, zaehler: 1, geraet: "g_0"),
        zielordnerKi: AblageRegister(wert: false, zaehler: 1, geraet: "g_0"),
        papierkorb: AblageRegister(wert: nil, zaehler: 0, geraet: "g_0"))
}

private func bestand(_ liste: [AblageOrdner]) -> AblageOrdnerbestand { AblageOrdnerbestand(liste.map { ($0.id, $0) }) }

private func faltenOrdner(_ staende: [AblageOrdnerbestand]) -> AblageOrdnerbestand {
    staende.dropFirst().reduce(staende[0]) { AblageZusammenfuehren.zusammenfuehrenOrdnerbestand($0, $1) }
}

private typealias Z = AblageZusammenfuehren

final class AblageZusammenfuehrenTests: XCTestCase {

    // MARK: Teil 1: die Rust-Tests

    func testRegisterHoeheresPaarGewinnt() {
        let a = AblageRegister(wert: "a", zaehler: 3, geraet: "g_1")
        let b = AblageRegister(wert: "b", zaehler: 3, geraet: "g_2")
        let c = AblageRegister(wert: "c", zaehler: 2, geraet: "g_9")
        XCTAssertEqual(Z.gewinner(a, b).wert, "b")
        XCTAssertEqual(Z.gewinner(b, a).wert, "b")
        XCTAssertEqual(Z.gewinner(a, c).wert, "a")
    }

    func testFassungenNachFasVereinigtOhneEntfernte() {
        var a = datei([fas("f1", nil, 1), fas("f2", "f1", 2)])
        var b = datei([fas("f1", nil, 1), fas("f3", "f1", 3)])
        b.entfernt = ["f2"]
        a.entfernt = []
        let m = unabhaengig([a, b])
        XCTAssertEqual(m.fassungen.map(\.fas), ["f1", "f3"])
        XCTAssertEqual(m.entfernt, ["f2"])
        XCTAssertEqual(m.blobRefs, ["blob-f1", "blob-f3"])
    }

    func testGleicherInhaltZweierGeraeteBleibtZweiFassungen() {
        var f2 = fas("f2", nil, 1)
        f2.blob = "blob-f1"
        let m = Z.zusammenfuehrenDatei(datei([fas("f1", nil, 1)]), datei([f2]))
        XCTAssertEqual(m.fassungen.count, 2)
        XCTAssertEqual(m.blobRefs, ["blob-f1"])
        XCTAssertTrue(Z.konflikt(m))
    }

    func testTexteWerdenVereinigt() {
        func t(_ rev: String) -> AblageText {
            AblageText(textrev: rev, blob: "tb-\(rev)", size: 1, extraktor: "x", extraktorVersion: "1", markenVersion: 1, stand: "ok")
        }
        var fa = fas("f1", nil, 1)
        fa.texte = [t("r1")]
        var fb = fas("f1", nil, 1)
        fb.texte = [t("r2")]
        let m = Z.zusammenfuehrenDatei(datei([fa]), datei([fb]))
        XCTAssertEqual(m.fassungen[0].texte.count, 2)
        XCTAssertTrue(m.blobRefs.contains("tb-r1"))
        XCTAssertTrue(m.blobRefs.contains("tb-r2"))
    }

    func testZweiGeraeteZweiKoepfeAktuellNichtPerZeit() {
        let a = datei([fas("f1", nil, 1), fas("f2", "f1", 100)])
        let b = datei([fas("f1", nil, 1), fas("f3", "f1", 5)])
        let m = unabhaengig([a, b])
        XCTAssertEqual(Z.koepfe(m), ["f2", "f3"])
        XCTAssertTrue(Z.konflikt(m))
        XCTAssertNil(Z.aktuell(m))
        XCTAssertEqual(Z.gemeinsameVorgaengerin(m), "f1")
    }

    func testGemeinsameVorgaengerinUeberMehrereStufen() {
        // f1 <- f2 <- f3 (Kopf) und f1 <- f4 (Kopf): f2 gehört nur zu f3, die jüngste gemeinsame ist f1.
        let d = datei([fas("f1", nil, 1), fas("f2", "f1", 2), fas("f3", "f2", 3), fas("f4", "f1", 4)])
        XCTAssertEqual(Z.koepfe(d), ["f3", "f4"])
        XCTAssertEqual(Z.gemeinsameVorgaengerin(d), "f1")
        // Ein dritter Kopf auf f2 ändert daran nichts, ein Kopf auf f1 ebenfalls nicht.
        XCTAssertEqual(Z.gemeinsameVorgaengerin(datei(d.fassungen + [fas("f5", "f2", 5)])), "f1")
        // Liegt der zweite Kopf auf f2, ist f2 die jüngste gemeinsame.
        XCTAssertEqual(Z.gemeinsameVorgaengerin(datei([fas("f1", nil, 1), fas("f2", "f1", 2), fas("f3", "f2", 3), fas("f4", "f2", 4)])), "f2")
        // Ein dritter Kopf auf getrennter Wurzel: keine gemeinsame Vorgängerin mehr.
        XCTAssertNil(Z.gemeinsameVorgaengerin(datei(d.fassungen + [fas("w1", nil, 9)])))
    }

    func testOhneGemeinsameFassungKeineVorgaengerin() {
        let m = datei([fas("f1", nil, 1), fas("f2", nil, 2)])
        XCTAssertTrue(Z.konflikt(m))
        XCTAssertNil(Z.gemeinsameVorgaengerin(m))
    }

    func testEinfacheWahlDannWeiterbearbeitungEinKopf() {
        // A und B gleichzeitig; Wahl B gegen A; danach B -> C.
        let basis = datei([fas("A", nil, 1), fas("B", nil, 2)])
        let w = Z.neueWahl(basis, id: "w1", gewaehlt: "B", zaehler: 5, geraet: "g_1")
        XCTAssertEqual(w.gegen, ["A", "B"])
        var m = basis
        m.wahlen.append(w)
        XCTAssertEqual(Z.koepfe(m), ["B"])
        m.fassungen.append(fas("C", "B", 9))
        XCTAssertEqual(Z.koepfe(m), ["C"])
        XCTAssertEqual(Z.aktuell(m), "C")
    }

    func testOfflineANachDIstNeuerKonflikt() {
        let basis = datei([fas("A", nil, 1), fas("B", nil, 2)])
        var gewaehlt = basis
        gewaehlt.wahlen.append(Z.neueWahl(basis, id: "w1", gewaehlt: "B", zaehler: 5, geraet: "g_1"))
        gewaehlt.fassungen.append(fas("C", "B", 9))
        let offline = datei([fas("A", nil, 1), fas("B", nil, 2), fas("D", "A", 7)])
        let m = unabhaengig([gewaehlt, offline])
        XCTAssertEqual(Z.koepfe(m), ["C", "D"])
        XCTAssertTrue(Z.konflikt(m))
    }

    func testGegenwahlErgibtGenauEinenKopfInAllenReihenfolgen() {
        let basis = datei([fas("A", nil, 1), fas("B", nil, 2)])
        var g1 = basis
        g1.wahlen.append(wahl("w1", "A", ["A", "B"], [], 3, "g_1"))
        var g2 = basis
        g2.wahlen.append(wahl("w2", "B", ["A", "B"], [], 3, "g_2"))
        let m = unabhaengig([g1, g2, basis])
        XCTAssertEqual(m.wahlen.count, 2)
        XCTAssertEqual(Z.koepfe(m), ["B"], "höherer Rang (g_2) gilt")
        // Zähler entscheidet vor Gerät
        var g3 = basis
        g3.wahlen.append(wahl("w3", "A", ["A", "B"], [], 4, "g_0"))
        let m2 = unabhaengig([g3, g2])
        XCTAssertEqual(Z.koepfe(m2), ["A"])
    }

    func testErneuteWahlNachGegenwahlEinKopfUndWeiterbearbeitung() {
        let basis = datei([fas("A", nil, 1), fas("B", nil, 2)])
        var m = basis
        m.wahlen.append(wahl("w1", "A", ["A", "B"], [], 3, "g_1"))
        m.wahlen.append(wahl("w2", "B", ["A", "B"], [], 3, "g_2"))
        XCTAssertEqual(Z.koepfe(m), ["B"])
        // Gerät 1 wählt erneut A: ersetzt beide, gegen = Vereinigung
        let neu = Z.neueWahl(m, id: "w3", gewaehlt: "A", zaehler: 4, geraet: "g_1")
        XCTAssertEqual(neu.ersetzt, ["w1", "w2"])
        XCTAssertEqual(neu.gegen, ["A", "B"])
        m.wahlen.append(neu)
        XCTAssertEqual(Z.koepfe(m), ["A"])
        m.fassungen.append(fas("E", "A", 20))
        XCTAssertEqual(Z.koepfe(m), ["E"], "Weiterbearbeitung konfliktfrei")
    }

    func testW1AW2HoeherBW3BErsetztBeideEinKopfB() {
        let basis = datei([fas("A", nil, 1), fas("B", nil, 2)])
        let w1 = wahl("w1", "A", ["A", "B"], [], 3, "g_1")
        let w2 = wahl("w2", "B", ["A", "B"], [], 3, "g_2")
        var zwischen = basis
        zwischen.wahlen = [w1, w2]
        XCTAssertEqual(Z.koepfe(zwischen), ["B"])
        let w3 = wahl("w3", "B", ["A", "B"], ["w1", "w2"], 4, "g_3")
        var a = basis
        a.wahlen = [w1]
        var b = basis
        b.wahlen = [w2]
        var c = basis
        c.wahlen = [w3]
        let m = unabhaengig([a, b, c])
        XCTAssertEqual(Z.aktiveWahlen(m).count, 1)
        XCTAssertEqual(Z.koepfe(m), ["B"])
        XCTAssertEqual(Z.abgeloest(m), ["A"])
    }

    func testNeueWahlUebernimmtGegenErsetzterWahlen() {
        // Wahl B gegen A; später entsteht C auf B. Neue Wahl C: A bleibt verdrängt.
        var d = datei([fas("A", nil, 1), fas("B", nil, 2)])
        d.wahlen.append(wahl("w1", "B", ["A", "B"], [], 3, "g_1"))
        d.fassungen.append(fas("C", "B", 5))
        let w = Z.neueWahl(d, id: "w2", gewaehlt: "C", zaehler: 6, geraet: "g_1")
        XCTAssertTrue(w.gegen.contains("A"), "verdrängte Fassung bleibt verdrängt")
        XCTAssertEqual(w.ersetzt, ["w1"])
        d.wahlen.append(w)
        XCTAssertEqual(Z.koepfe(d), ["C"])
    }

    // MARK: Ordner

    func testOrdnerABBABCInAllenReihenfolgen() {
        // Gemeinsamer Ausgangsstand: A, B, C an der Wurzel (Zähler 0).
        func basis() -> [AblageOrdner] { [ord("A"), ord("B"), ord("C")] }
        func mit(_ aendere: AblageOrdner) -> AblageOrdnerbestand {
            var b = bestand(basis())
            b[aendere.id] = aendere
            return b
        }
        let g1 = mit(ord("A", ("B", 1, "g_1")))
        let g2 = mit(ord("B", ("A", 2, "g_2")))
        let g3 = mit(ord("B", ("C", 3, "g_3")))
        let alle = [g1, g2, g3]
        let erwartet = faltenOrdner(alle)
        for p in permutationen(alle) {
            let m = faltenOrdner(p)
            XCTAssertEqual(kanon(m), kanon(erwartet), "Register hängen von der Reihenfolge ab")
            let e = Z.wirksameEltern(m)
            XCTAssertEqual(e["A"], .ordner("B"))
            XCTAssertEqual(e["B"], .ordner("C"))
            XCTAssertEqual(e["C"], .wurzel)
            XCTAssertEqual(Z.kette(e, "A"), ["A", "B", "C"])
        }
        // Register unverändert: A.eltern=B (1), B.eltern=C (3)
        XCTAssertEqual(erwartet["A"]?.eltern, AblageRegister<String?>(wert: "B", zaehler: 1, geraet: "g_1"))
        XCTAssertEqual(erwartet["B"]?.eltern, AblageRegister<String?>(wert: "C", zaehler: 3, geraet: "g_3"))
    }

    func testOrdnerZyklusZunaechstWiederhergestelltDannAufgeloest() {
        func basis() -> [AblageOrdner] { [ord("A"), ord("B"), ord("C")] }
        func mit(_ aendere: AblageOrdner) -> AblageOrdnerbestand {
            var b = bestand(basis())
            b[aendere.id] = aendere
            return b
        }
        let g1 = mit(ord("A", ("B", 1, "g_1")))
        let g2 = mit(ord("B", ("A", 2, "g_2")))
        for (x, y) in [(g1, g2), (g2, g1)] {
            let m = Z.zusammenfuehrenOrdnerbestand(x, y)
            let e = Z.wirksameEltern(m)
            XCTAssertEqual(e["B"], .ordner("A"), "B unter A")
            XCTAssertEqual(e["A"], .wiederhergestellt, "A vorläufig wiederhergestellt")
        }
    }

    func testOrdnerOhneBekanntenElternIstWiederhergestellt() {
        let b = bestand([ord("A", ("X", 1, "g_1"))])
        XCTAssertEqual(Z.wirksameEltern(b)["A"], .wiederhergestellt)
    }

    func testKlasseWirdAngehobenNieGesenktUndUnbekanntSperrt() {
        var b = bestand([ord("A"), ord("B", ("A", 1, "g_1")), ord("C", ("B", 1, "g_1"))])
        b["A"]!.datenklasse.wert = "confidential"
        b["B"]!.datenklasse.wert = "public"
        let e = Z.wirksameEltern(b)
        XCTAssertEqual(Z.wirksameKlasse(b, e, "A"), .confidential)
        XCTAssertEqual(Z.wirksameKlasse(b, e, "B"), .confidential, "angehoben")
        XCTAssertEqual(Z.wirksameKlasse(b, e, "C"), .confidential, "erbt")
        b["C"]!.datenklasse.wert = "personal"
        XCTAssertEqual(Z.wirksameKlasse(b, e, "C"), .personal)
        b["C"]!.datenklasse.wert = "geheimer"
        XCTAssertEqual(Z.wirksameKlasse(b, e, "C"), .unbekannt)
    }

    func testNurGeraetGiltImUnterbaum() {
        var b = bestand([ord("A"), ord("B", ("A", 1, "g_1"))])
        b["A"]!.nurGeraet.wert = true
        let e = Z.wirksameEltern(b)
        XCTAssertTrue(Z.wirksamNurGeraet(b, e, "B"))
    }

    func testDateiKlasseIstMaximum() {
        var b = bestand([ord("A")])
        b["A"]!.datenklasse.wert = "internal"
        let e = Z.wirksameEltern(b)
        var d = datei([])
        d.ordner.wert = "A"
        d.klasseMin.wert = "confidential"
        XCTAssertEqual(Z.wirksameKlasseDatei(b, e, d), .confidential)
        d.klasseMin.wert = "public"
        XCTAssertEqual(Z.wirksameKlasseDatei(b, e, d), .internal)
    }

    // MARK: Teil 2: Byte- statt Swift-Ordnung

    func testKarteIstByteGenau() {
        let zusammen = "\u{e9}", zerlegt = "e\u{301}"
        XCTAssertEqual(zusammen, zerlegt, "Voraussetzung: Swift hält beide für gleich")
        var k = AblageKarte<Int>()
        k[zusammen] = 1
        k[zerlegt] = 2
        XCTAssertEqual(k.count, 2, "zwei verschiedene Schlüssel")
        XCTAssertEqual(k[zusammen], 1)
        XCTAssertEqual(k[zerlegt], 2)
        XCTAssertEqual(k.keys.map { Array($0.utf8) }, [Array(zerlegt.utf8), Array(zusammen.utf8)], "Reihenfolge nach Bytes (e < é)")
        k[zerlegt] = nil
        XCTAssertEqual(k.count, 1)
        XCTAssertEqual(AblageKarte<Int>([(zusammen, 1)]), AblageKarte<Int>([(zusammen, 1)]))
        XCTAssertNotEqual(AblageKarte<Int>([(zusammen, 1)]), AblageKarte<Int>([(zerlegt, 1)]), "gleich in Swift, verschieden in Bytes")
        // Spätere Paare ersetzen frühere.
        XCTAssertEqual(AblageKarte<Int>([("a", 1), ("a", 2)])["a"], 2)
    }

    func testKennungenByteGenau() {
        let zusammen = "fas_\u{e9}", zerlegt = "fas_e\u{301}"
        // Zwei Fassungen, die Swift nicht unterscheidet, bleiben zwei.
        let m = Z.zusammenfuehrenDatei(datei([fas(zusammen, nil, 1)]), datei([fas(zerlegt, nil, 2)]))
        XCTAssertEqual(m.fassungen.count, 2)
        XCTAssertEqual(m.fassungen.map { Array($0.fas.utf8) }, [Array(zerlegt.utf8), Array(zusammen.utf8)])
        // `entfernt` trifft nur die byte-gleiche Kennung.
        var a = datei([fas(zusammen, nil, 1), fas(zerlegt, nil, 2)])
        a.entfernt = [zusammen]
        let b = Z.zusammenfuehrenDatei(a, a)
        XCTAssertEqual(b.fassungen.map { Array($0.fas.utf8) }, [Array(zerlegt.utf8)])
        XCTAssertEqual(Z.koepfe(m).map { Array($0.utf8) }, [Array(zerlegt.utf8), Array(zusammen.utf8)])
        // Vorgängerin zählt nur bei Byte-Gleichheit.
        let d = datei([fas(zusammen, nil, 1), fas("fas_x", zerlegt, 2)])
        XCTAssertEqual(Z.koepfe(d).count, 2, "der Verweis auf die zerlegte Kennung macht die zusammengesetzte nicht zur Vorgängerin")
    }

    func testRangOrdnungNachBytes() {
        // Swift ordnet `e\u{301}` (NFC: é, U+E9) hinter "f"; Rust nach Bytes: 'e' (0x65) vor 'f' (0x66).
        let a = AblageRegister(wert: "a", zaehler: 1, geraet: "e\u{301}")
        let b = AblageRegister(wert: "b", zaehler: 1, geraet: "f")
        XCTAssertGreaterThan("e\u{301}", "f", "Voraussetzung: Swift-Ordnung weicht ab")
        XCTAssertEqual(Z.gewinner(a, b).wert, "b", "f gewinnt nach Bytes")
        XCTAssertEqual(Z.gewinner(b, a).wert, "b")
        // Zeichen über dem Ersatzbereich: UTF-8 (EF BF A0) vor U+1F600 (F0 9F 98 80), UTF-16 umgekehrt.
        let c = AblageRegister(wert: "c", zaehler: 1, geraet: "g_\u{FFE0}")
        let d = AblageRegister(wert: "d", zaehler: 1, geraet: "g_\u{1F600}")
        XCTAssertEqual(Z.gewinner(c, d).wert, "d")
        XCTAssertEqual(Z.gewinner(d, c).wert, "d")
        // Wahlen: Rang der Gerätekennung nach Bytes bestimmt, welche Wahl überstimmt.
        var m = datei([fas("A", nil, 1), fas("B", nil, 2)])
        m.wahlen = [wahl("w1", "A", ["A", "B"], [], 3, "e\u{301}"), wahl("w2", "B", ["A", "B"], [], 3, "f")]
        XCTAssertEqual(Z.koepfe(m), ["B"], "w2 (g = f) hat den höheren Rang")
    }

    func testRegisterGleichstandEntscheidetDerWert() {
        // Gleicher Rang, anderer Wert: der Wert bestimmt, in beiden Reihenfolgen dasselbe Ergebnis.
        func paar<T: AblageGeordnet>(_ a: T, _ b: T, gewinnt: T, _ name: String, file: StaticString = #filePath, line: UInt = #line) {
            let ra = AblageRegister(wert: a, zaehler: 4, geraet: "g_1"), rb = AblageRegister(wert: b, zaehler: 4, geraet: "g_1")
            XCTAssertEqual(Z.gewinner(ra, rb).wert, gewinnt, name, file: file, line: line)
            XCTAssertEqual(Z.gewinner(rb, ra).wert, gewinnt, name, file: file, line: line)
            XCTAssertEqual(Z.gewinner(ra, ra).wert, a, name, file: file, line: line)
        }
        paar("a.pdf", "b.pdf", gewinnt: "b.pdf", "String")
        paar("e\u{301}", "f", gewinnt: "f", "String nach Bytes")
        paar(false, true, gewinnt: true, "Bool")
        paar(nil as Int64?, 5 as Int64?, gewinnt: 5, "None vor Some")
        paar(-3 as Int64?, 2 as Int64?, gewinnt: 2, "Int64 mit Vorzeichen")
        paar(nil as String?, "" as String?, gewinnt: "", "None vor leerem Some")
        // Der Rang geht vor dem Wert.
        XCTAssertEqual(Z.gewinner(AblageRegister(wert: "zzz", zaehler: 1, geraet: "g_9"), AblageRegister(wert: "a", zaehler: 2, geraet: "g_1")).wert, "a")
    }

    func testExtraVereinigenNachKompaktemJSON() throws {
        func extra(_ s: String) throws -> [AblageJSON.Member] {
            guard case .object(let m) = try AblageJSON.parse(Array(s.utf8)) else { throw AblageError.feld("objekt") }
            return m
        }
        let a = try extra(#"{"nur_a":1,"gemeinsam":{"a":[1]},"zahl":10,"text":"b"}"#)
        let b = try extra(#"{"nur_b":null,"gemeinsam":{"a":[2]},"zahl":9,"text":"a"}"#)
        let ab = Z.extraVereinigen(a, b), ba = Z.extraVereinigen(b, a)
        XCTAssertEqual(ab, ba, "kommutativ")
        XCTAssertEqual(ab.map(\.key), ["gemeinsam", "nur_a", "nur_b", "text", "zahl"], "nach Schlüsseln sortiert, null bleibt")
        func wert(_ k: String) -> String { String(decoding: ab.first { $0.key == k }!.value.kanonisch(), as: UTF8.self) }
        XCTAssertEqual(wert("gemeinsam"), #"{"a":[2]}"#)
        XCTAssertEqual(wert("text"), #""b""#)
        // Der Text entscheidet, nicht die Zahl: "9" ist als Text größer als "10".
        XCTAssertEqual(wert("zahl"), "9")
        XCTAssertEqual(Z.extraVereinigen(ab, ab), ab, "idempotent")
        XCTAssertEqual(Z.extraVereinigen([], []), [])
    }

    // MARK: Teil 2: zerstörte Eingaben

    func testVorgaengerZyklusTerminiert() {
        // f1 -> f2 -> f1 (Vorgänger-Kreis), f3 hängt an f1, f4 an f2.
        let d = datei([fas("f1", "f2", 1), fas("f2", "f1", 2), fas("f3", "f1", 3), fas("f4", "f2", 4)])
        XCTAssertEqual(Z.koepfe(d), ["f3", "f4"])
        XCTAssertNil(Z.aktuell(d))
        XCTAssertEqual(Z.gemeinsameVorgaengerin(d), "f1", "Rust-Ablauf: Ahnenliste von f3 beginnt mit f1, f1 steht auch in der von f4")
        XCTAssertLessThanOrEqual(Z.ahnen(d, "f3").count, d.fassungen.count + 1, "begrenzt durch die Schrittzahl")
        // Reiner Kreis ohne Kopf: kein Kopf, kein Absturz.
        let kreis = datei([fas("f1", "f2", 1), fas("f2", "f1", 2)])
        XCTAssertEqual(Z.koepfe(kreis), [])
        XCTAssertNil(Z.aktuell(kreis))
        XCTAssertNil(Z.gemeinsameVorgaengerin(kreis))
        XCTAssertEqual(Z.aktuelleFassung(kreis)?.fas, "f2", "ohne Kopf die jüngste")
        // Selbstverweis.
        let selbst = datei([fas("f1", "f1", 1), fas("f2", nil, 2)])
        XCTAssertEqual(Z.koepfe(selbst), ["f2"])
    }

    func testWahlMitUnbekanntenFassungen() {
        var d = datei([fas("A", nil, 1), fas("B", nil, 2)])
        d.wahlen = [wahl("w1", "ZZ", ["A", "B", "ZZ", "YY"], ["w_unbekannt"], 3, "g_1")]
        // A und B sind gegen die unbekannte Wahl abgelöst; ZZ gibt es nicht.
        XCTAssertEqual(Z.koepfe(d), [])
        XCTAssertEqual(Z.abgeloest(d), ["A", "B", "YY"])
        XCTAssertNil(Z.aktuell(d))
        XCTAssertNil(Z.gemeinsameVorgaengerin(d))
        // `neueWahl` auf diesem Stand terminiert und nimmt die alte Wahl mit.
        let w = Z.neueWahl(d, id: "w2", gewaehlt: "A", zaehler: 9, geraet: "g_1")
        XCTAssertEqual(w.ersetzt, ["w1"])
        XCTAssertEqual(w.gegen, ["A", "B", "YY", "ZZ"])
    }

    func testOrdnerZyklusSelbstUndDreier() {
        // Selbst-Elternschaft.
        XCTAssertEqual(Z.wirksameEltern(bestand([ord("A", ("A", 1, "g_1"))]))["A"], .wiederhergestellt)
        // Dreierkreis A -> B -> C -> A: der Ordner mit dem niedrigsten Rang wird wiederhergestellt, kein Absturz.
        let b = bestand([ord("A", ("B", 3, "g_1")), ord("B", ("C", 2, "g_1")), ord("C", ("A", 1, "g_1"))])
        let e = Z.wirksameEltern(b)
        XCTAssertEqual(e["A"], .ordner("B"))
        XCTAssertEqual(e["B"], .ordner("C"))
        XCTAssertEqual(e["C"], .wiederhergestellt)
        XCTAssertEqual(Z.kette(e, "A"), ["A", "B", "C"])
        for id in ["A", "B", "C"] {
            XCTAssertEqual(Z.strengeKette(b, e, id).count, Z.kette(e, id).count + Z.zusatzkette(b, e, id).count)
            XCTAssertFalse(Z.strengeKette(b, e, id).isEmpty)
        }
        // Die Klasse im Kreis bleibt berechenbar (alle erben: Vorgabe).
        XCTAssertEqual(Z.wirksameKlasse(b, e, "A"), .internal)
    }

    func testRangGleichstandImOrdnerbaumIstDeterministisch() {
        // Gleicher Rang bei A und B: die Kennung entscheidet (aufsteigend), in jeder Einfügereihenfolge gleich.
        let a = ord("A", ("B", 5, "g_1")), b = ord("B", ("A", 5, "g_1"))
        let e1 = Z.wirksameEltern(bestand([a, b])), e2 = Z.wirksameEltern(bestand([b, a]))
        XCTAssertEqual(e1, e2)
        XCTAssertEqual(e1["A"], .ordner("B"), "A kommt zuerst")
        XCTAssertEqual(e1["B"], .wiederhergestellt)
    }

    func testFehlenderOrdnerInDerKette() {
        // A liegt unter X, das nicht im Bestand ist: wiederhergestellt; Sperre und Klasse folgen der gespeicherten Kette.
        var b = bestand([ord("A", ("X", 1, "g_1")), ord("B", ("A", 1, "g_1"))])
        let e = Z.wirksameEltern(b)
        XCTAssertEqual(e["A"], .wiederhergestellt)
        XCTAssertEqual(e["B"], .ordner("A"))
        XCTAssertEqual(Z.zusatzkette(b, e, "B"), ["X"])
        XCTAssertEqual(Z.strengeKette(b, e, "B"), ["B", "A", "X"])
        XCTAssertEqual(Z.wirksameKlasse(b, e, "B"), .unbekannt, "fehlendes Glied in der strengen Kette")
        XCTAssertFalse(Z.ordnerImKorb(b, e, "B"), "die wirksame Kette endet bei A; das fehlende X gehört nicht dazu")
        // Ein Glied, das im Bestand fehlt, zählt als im Korb: eine Datei in einem Ordner, den es nicht gibt, lebt nicht.
        XCTAssertTrue(Z.ordnerImKorb(b, e, "fehlt"))
        var d = datei([])
        d.ordner.wert = "fehlt"
        XCTAssertFalse(Z.dateiLebt(b, e, d))
        XCTAssertFalse(Z.wirksamNurGeraet(b, e, "B"))
        b["A"]!.nurGeraet.wert = true
        XCTAssertTrue(Z.wirksamNurGeraet(b, e, "B"), "nur Gerät am Vorfahr gilt trotz fehlendem Elternordner")
    }

    // MARK: Teil 2: Papierkorb

    func testPapierkorbLebendigkeit() {
        var b = bestand([ord("A"), ord("B", ("A", 1, "g_1")), ord("C", ("B", 1, "g_1"))])
        let e = Z.wirksameEltern(b)
        var d = datei([])
        d.ordner.wert = "C"
        XCTAssertTrue(Z.dateiLebt(b, e, d))
        XCTAssertFalse(Z.ordnerImKorb(b, e, "C"))
        // Der Korb eines Vorfahren erfasst alle darunter.
        b["B"]!.papierkorb.wert = 1_762_800_000_000
        XCTAssertTrue(Z.ordnerImKorb(b, e, "C"))
        XCTAssertTrue(Z.ordnerImKorb(b, e, "B"))
        XCTAssertFalse(Z.ordnerImKorb(b, e, "A"))
        XCTAssertFalse(Z.dateiLebt(b, e, d))
        // Datei im Korb, Ordner nicht.
        b["B"]!.papierkorb.wert = nil
        d.papierkorb.wert = 5
        XCTAssertFalse(Z.dateiLebt(b, e, d))
        d.papierkorb.wert = nil
        XCTAssertTrue(Z.dateiLebt(b, e, d))
        // Wert 0 ist ein Zeitstempel, kein „leer“.
        d.papierkorb.wert = 0
        XCTAssertFalse(Z.dateiLebt(b, e, d))
    }

    // MARK: Teil 2: Teilnahme

    private func ordnerMit(_ klasse: String, nurGeraet: Bool = false, eltern: (String, UInt64, String)? = nil, id: String = "A") -> AblageOrdner {
        var o = ord(id, eltern)
        o.datenklasse.wert = klasse
        o.nurGeraet.wert = nurGeraet
        return o
    }

    func testTeilnahmeNachKlasseUndNurGeraet() {
        let b = bestand([
            ordnerMit("public", id: "P"), ordnerMit("internal", id: "I"), ordnerMit("confidential", id: "C"),
            ordnerMit("personal", id: "S"), ordnerMit("geheimer", id: "U"), ordnerMit("erbt", id: "E"),
            ordnerMit("public", nurGeraet: true, id: "N"), ordnerMit("public", eltern: ("N", 1, "g_1"), id: "NK"),
            ordnerMit("erbt", eltern: ("C", 1, "g_1"), id: "CK"), ordnerMit("public", eltern: ("C", 1, "g_1"), id: "CP"),
        ])
        let t = AblageTeilnahme.alle(ordner: b, dateien: [:])
        XCTAssertEqual(t["P"], .sync)
        XCTAssertEqual(t["I"], .sync)
        XCTAssertEqual(t["C"], .lokal)
        XCTAssertEqual(t["S"], .lokal)
        XCTAssertEqual(t["U"], .halten, "unbekannte Klasse: weder senden noch begraben")
        XCTAssertEqual(t["E"], .sync, "erbt ohne Elternklasse: Vorgabe internal")
        XCTAssertEqual(t["N"], .lokal)
        XCTAssertEqual(t["NK"], .lokal, "nur Gerät gilt im Unterbaum")
        XCTAssertEqual(t["CK"], .lokal, "erbt confidential")
        XCTAssertEqual(t["CP"], .lokal, "public unter confidential wird angehoben")
        XCTAssertEqual(t.count, b.count)
    }

    func testTeilnahmeDatei() {
        let b = bestand([ordnerMit("internal", id: "I"), ordnerMit("confidential", id: "C"), ordnerMit("geheimer", id: "U")])
        func d(_ id: String, ordner: String, klasseMin: String = "public", nurGeraetMin: Bool = false) -> AblageDatei {
            var x = datei([])
            x.id = id
            x.ordner.wert = ordner
            x.klasseMin.wert = klasseMin
            x.nurGeraetMin = nurGeraetMin
            return x
        }
        let dateien = AblageDateibestand([
            ("d1", d("d1", ordner: "I")), ("d2", d("d2", ordner: "I", klasseMin: "confidential")),
            ("d3", d("d3", ordner: "I", nurGeraetMin: true)), ("d4", d("d4", ordner: "C")),
            ("d5", d("d5", ordner: "U")), ("d6", d("d6", ordner: "I", klasseMin: "erbt")),
            ("d7", d("d7", ordner: "I", klasseMin: "seltsam")), ("d8", d("d8", ordner: "fehlt")),
            ("d9", d("d9", ordner: "I", klasseMin: "personal")),
        ])
        let t = AblageTeilnahme.alle(ordner: b, dateien: dateien)
        XCTAssertEqual(t["d1"], .sync)
        XCTAssertEqual(t["d2"], .lokal, "klasse_min confidential hebt die Datei an")
        XCTAssertEqual(t["d3"], .lokal, "nur_geraet_min")
        XCTAssertEqual(t["d4"], .lokal)
        XCTAssertEqual(t["d5"], .halten, "Ordner mit unbekannter Klasse")
        XCTAssertEqual(t["d6"], .sync)
        XCTAssertEqual(t["d7"], .halten, "unbekannte Mindestklasse")
        XCTAssertEqual(t["d8"], .sync, "fehlender Ordner zählt neutral, nicht als unbekannt")
        XCTAssertEqual(t["d9"], .lokal)
        XCTAssertEqual(t.count, 3 + 9)
    }

    func testTeilnahmeFehlenderElternordnerZaehltNeutral() {
        // Direkt gerechnet ist die Klasse unbekannt (fehlendes Glied), `alle` ergänzt den neutralen Ordner.
        let b = bestand([ord("A", ("X", 1, "g_1"))])
        XCTAssertEqual(Z.wirksameKlasse(b, Z.wirksameEltern(b), "A"), .unbekannt)
        XCTAssertEqual(AblageTeilnahme.alle(ordner: b, dateien: [:])["A"], .sync)
    }

    func testTeilnahmeFestgehaltenUndBefreit() {
        let b = bestand([ordnerMit("confidential", id: "C"), ordnerMit("erbt", id: "F")])
        var d = datei([])
        d.id = "d1"
        d.ordner.wert = "C"
        d.klasseMin.wert = "public"
        let dateien = AblageDateibestand([("d1", d)])
        // Festgehalten: für immer lokal, auch wenn Klasse und Kette es nicht verlangen.
        let t1 = AblageTeilnahme.alle(ordner: b, dateien: [:], lokal: ["F"])
        XCTAssertEqual(t1["F"], .lokal)
        XCTAssertEqual(AblageTeilnahme.alle(ordner: b, dateien: [:])["F"], .sync)
        // Datei im confidential-Ordner: lokal; ist der Ordner festgehalten und die Datei befreit, sieht sie ihn neutral.
        XCTAssertEqual(AblageTeilnahme.alle(ordner: b, dateien: dateien)["d1"], .lokal)
        XCTAssertEqual(AblageTeilnahme.alle(ordner: b, dateien: dateien, lokal: ["C"], befreit: ["d1"])["d1"], .sync)
        XCTAssertEqual(AblageTeilnahme.alle(ordner: b, dateien: dateien, lokal: ["C"], befreit: ["d1"])["C"], .lokal)
        XCTAssertEqual(AblageTeilnahme.alle(ordner: b, dateien: dateien, lokal: ["C"])["d1"], .lokal, "ohne Befreiung bleibt sie lokal")
        XCTAssertEqual(AblageTeilnahme.alle(ordner: b, dateien: dateien, lokal: ["d1"], befreit: ["d1"])["d1"], .lokal, "festgehalten schlägt befreit")
    }

    // MARK: Teil 2: Zähler, aktuelle Fassung

    func testGroessterZaehlerUndNaechster() {
        var o = ord("A")
        o.eltern.zaehler = 3; o.name.zaehler = 4; o.datenklasse.zaehler = 5; o.nurGeraet.zaehler = 6
        o.zielordnerKi.zaehler = 7; o.papierkorb.zaehler = 8
        var d = datei([fas("f1", nil, 99_999)], [wahl("w1", "f1", [], [], 21, "g_1")])
        d.ordner.zaehler = 10; d.name.zaehler = 11; d.klasseMin.zaehler = 12; d.papierkorb.zaehler = 13
        let b = bestand([o])
        XCTAssertEqual(Z.groessterZaehler(b, [:]), 8)
        // Jedes einzelne Register zählt.
        for kaputt in 0..<10 {
            var x = o; var y = d
            switch kaputt {
            case 0: x.eltern.zaehler = 1000
            case 1: x.name.zaehler = 1000
            case 2: x.datenklasse.zaehler = 1000
            case 3: x.nurGeraet.zaehler = 1000
            case 4: x.zielordnerKi.zaehler = 1000
            case 5: x.papierkorb.zaehler = 1000
            case 6: y.ordner.zaehler = 1000
            case 7: y.name.zaehler = 1000
            case 8: y.klasseMin.zaehler = 1000
            default: y.papierkorb.zaehler = 1000
            }
            XCTAssertEqual(Z.groessterZaehler(bestand([x]), AblageDateibestand([("dat_1", y)])), 1000, "Register \(kaputt)")
        }
        XCTAssertEqual(Z.groessterZaehler(b, AblageDateibestand([("dat_1", d)])), 21, "Wahl-Zähler zählt, Fassungs-Zeit nicht")
        // `nur_geraet_min` hat keinen Zähler; Zähler über 2^53 bleiben exakt.
        d.nurGeraetMin = true
        d.wahlen[0].zaehler = 9_007_199_254_740_993
        XCTAssertEqual(Z.groessterZaehler(b, AblageDateibestand([("dat_1", d)])), 9_007_199_254_740_993)
        XCTAssertEqual(Z.naechsterZaehler(b, AblageDateibestand([("dat_1", d)])), 9_007_199_254_740_994)
        XCTAssertEqual(Z.groessterZaehler([:], [:]), 0)
        XCTAssertEqual(Z.naechsterZaehler([:], [:]), 1)
        d.wahlen[0].zaehler = UInt64.max
        XCTAssertEqual(Z.groessterZaehler(b, AblageDateibestand([("dat_1", d)])), UInt64.max)
        XCTAssertNil(Z.naechsterZaehler(b, AblageDateibestand([("dat_1", d)])), "kein Umlauf auf 0")
    }

    func testAktuelleFassung() {
        // Einziger Kopf: er, auch wenn eine andere Fassung jünger ist.
        let a = datei([fas("f1", nil, 100), fas("f2", "f1", 5)])
        XCTAssertEqual(Z.aktuelleFassung(a)?.fas, "f2")
        // Konflikt: die jüngste nach (zeit, fas), nicht „aktuell“.
        let b = datei([fas("f1", nil, 1), fas("f2", "f1", 100), fas("f3", "f1", 5)])
        XCTAssertNil(Z.aktuell(b))
        XCTAssertEqual(Z.aktuelleFassung(b)?.fas, "f2")
        // Gleiche Zeit: die größere Kennung nach Bytes.
        let c = datei([fas("fa", nil, 7), fas("fb", nil, 7)])
        XCTAssertEqual(Z.aktuelleFassung(c)?.fas, "fb")
        XCTAssertEqual(Z.aktuelleFassung(datei([fas("fb", nil, 7), fas("fa", nil, 7)]))?.fas, "fb", "unabhängig von der Reihenfolge")
        let e = datei([fas("f\u{e9}", nil, 7), fas("fe\u{301}", nil, 7)])
        XCTAssertEqual(Array((Z.aktuelleFassung(e)?.fas ?? "").utf8), Array("f\u{e9}".utf8), "Bytes: é (0xC3) kommt nach e (0x65)")
        // Exakter Gleichstand (dieselbe `fas` und Zeit in einem nicht zusammengeführten Stand): Rusts `max_by` nimmt das
        // letzte Maximum, in beiden Reihenfolgen also das hintere.
        var erste = fas("fx", nil, 9), zweite = fas("fx", nil, 9)
        erste.blob = "blob-erste"
        zweite.blob = "blob-zweite"
        XCTAssertEqual(Z.aktuelleFassung(datei([erste, zweite]))?.blob, "blob-zweite")
        XCTAssertEqual(Z.aktuelleFassung(datei([zweite, erste]))?.blob, "blob-erste")
        // Negative Zeit ist älter als positive.
        XCTAssertEqual(Z.aktuelleFassung(datei([fas("fa", nil, -5), fas("fb", nil, 3)]))?.fas, "fb")
        XCTAssertNil(Z.aktuelleFassung(datei([])))
        // Durch `entfernt` verschwundene Fassungen zählen nicht.
        var mitEntfernt = datei([fas("f1", nil, 1), fas("f2", nil, 2)])
        mitEntfernt.entfernt = ["f2"]
        XCTAssertEqual(Z.aktuelleFassung(Z.zusammenfuehrenDatei(mitEntfernt, mitEntfernt))?.fas, "f1")
    }

    // MARK: Teil 2: Eingaben, Nebenläufigkeit, Größe

    func testEingabenBleibenUnveraendert() {
        var a = datei([fas("f1", nil, 1), fas("f2", "f1", 100)], [wahl("w1", "f2", ["f1", "f2"], [], 3, "g_1")])
        a.entfernt = ["x", "a"]
        var b = datei([fas("f1", nil, 1), fas("f3", "f1", 5)])
        b.fassungen[0].texte = [AblageText(textrev: "r", blob: "tb", size: 1, extraktor: "x", extraktorVersion: "1", markenVersion: 1, stand: "ok")]
        let vorherA = kanon(a), vorherB = kanon(b)
        _ = Z.zusammenfuehrenDatei(a, b)
        _ = Z.zusammenfuehrenDatei(b, a)
        _ = Z.neueWahl(a, id: "w9", gewaehlt: "f2", zaehler: 9, geraet: "g_1")
        _ = Z.koepfe(a); _ = Z.abgeloest(a); _ = Z.gemeinsameVorgaengerin(b)
        XCTAssertEqual(kanon(a), vorherA, "Eingabe a verändert")
        XCTAssertEqual(kanon(b), vorherB, "Eingabe b verändert")

        let oa = bestand([ord("A"), ord("B", ("A", 1, "g_1"))]), ob = bestand([ord("A", ("B", 2, "g_2"))])
        let vorherO = (kanon(oa), kanon(ob))
        let e = Z.wirksameEltern(oa)
        _ = Z.zusammenfuehrenOrdnerbestand(oa, ob)
        _ = AblageTeilnahme.alle(ordner: oa, dateien: [:], lokal: ["A"], befreit: ["B"])
        _ = Z.wirksameKlasse(oa, e, "B")
        XCTAssertEqual(kanon(oa), vorherO.0)
        XCTAssertEqual(kanon(ob), vorherO.1)
        XCTAssertEqual(oa.count, 2, "neutrale Ordner aus der Teilnahme landen nicht im Bestand")
    }

    func testNebenlaeufigGleichesErgebnis() {
        final class Sammler: @unchecked Sendable {
            private let lock = NSLock()
            private var werte: [String] = []
            func add(_ s: String) { lock.lock(); werte.append(s); lock.unlock() }
            var alle: [String] { lock.lock(); defer { lock.unlock() }; return werte }
        }
        let a = datei([fas("f1", nil, 1), fas("f2", "f1", 100), fas("f4", "f2", 7)], [wahl("w1", "f2", ["f1", "f2"], [], 3, "g_1")])
        let b = datei([fas("f1", nil, 1), fas("f3", "f1", 5)], [wahl("w2", "f3", ["f1", "f3"], [], 3, "g_2")])
        let erwartet = kanon(Z.zusammenfuehrenDatei(a, b))
        let sammler = Sammler()
        DispatchQueue.concurrentPerform(iterations: 200) { i in
            let m = i % 2 == 0 ? Z.zusammenfuehrenDatei(a, b) : Z.zusammenfuehrenDatei(b, a)
            sammler.add(kanon(m) + "|" + Z.koepfe(m).joined(separator: ","))
        }
        let koepfe = Z.koepfe(Z.zusammenfuehrenDatei(a, b)).joined(separator: ",")
        XCTAssertEqual(sammler.alle.count, 200)
        XCTAssertEqual(Set(sammler.alle), [erwartet + "|" + koepfe], "alle Läufe bytegleich")
    }

    func testGrosseKetteTerminiert() {
        let n = 1500
        var kette: [AblageFassung] = []
        for i in 0..<n { kette.append(fas(String(format: "f%05d", i), i == 0 ? nil : String(format: "f%05d", i - 1), Int64(i))) }
        let a = datei(kette)
        let b = datei(Array(kette.prefix(n / 2)) + [fas("g00000", String(format: "f%05d", n / 2 - 1), 1)])
        let m = Z.zusammenfuehrenDatei(a, b)
        XCTAssertEqual(m.fassungen.count, n + 1)  // die Kette plus der Seitenzweig g00000
        XCTAssertEqual(Z.koepfe(m), [String(format: "f%05d", n - 1), "g00000"])
        XCTAssertEqual(Z.gemeinsameVorgaengerin(m), String(format: "f%05d", n / 2 - 1))
        // Tiefe Ordnerkette.
        var liste = [ord("o0")]
        for i in 1..<400 { liste.append(ord("o\(i)", ("o\(i - 1)", UInt64(i), "g_1"))) }
        let bst = bestand(liste)
        let e = Z.wirksameEltern(bst)
        XCTAssertEqual(Z.kette(e, "o399").count, 400)
        XCTAssertEqual(Z.wirksameKlasse(bst, e, "o399"), .internal)
    }
}
