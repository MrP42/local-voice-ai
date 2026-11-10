import XCTest
@testable import VoiceCore
#if canImport(CryptoKit)

// Darstellung der Ablage (D5a-4, lva#441): Ordnerbaum als flache Zeilen, Papierkorb, Anker in lesbarer Form, Detail einer Datei
// (Fassungen, Konflikt, Textstand), Kontingent. Nur synthetische Namen.

private typealias W = AblageWelt

final class AblageDarstellungTests: XCTestCase {
    private func fassung(_ n: Int, _ vorg: Int? = nil, typ: String = "pdf", zeit: Int64 = 1_000, mitText: Bool = true) -> AblageFassung {
        var f = W.fassung(n, vorg, inhalt: "Original \(n)", typ: typ, zeit: zeit)
        if mitText { f.texte = [W.text(zu: f).eintrag] }
        return f
    }

    private func zustand(ordner: [AblageOrdner], dateien: [AblageDatei]) -> AblageZustand {
        var z = W.neuerZustand()
        W.einspielen(&z, ordner: ordner.map { W.remote(ordner: $0) }, dateien: dateien.map { W.remote(datei: $0) })
        return z
    }

    // MARK: Baum

    func testBaumZeilenOrdnerUnterordnerDateienSortiertUndEinklappbar() {
        let z = zustand(
            ordner: [W.ordner(1, name: "Verträge", klasse: "internal"), W.ordner(2, name: "Mietverträge", eltern: W.ordId(1)),
                     W.ordner(3, name: "Allgemein", klasse: "internal"), W.ordner(4, name: "Altes", klasse: "internal", korb: 5)],
            dateien: [W.datei(1, ordner: W.ordId(2), name: "Mietvertrag.pdf", fassungen: [fassung(1)]),
                      W.datei(2, ordner: W.ordId(1), name: "Angebot.pdf", fassungen: [fassung(2)]),
                      W.datei(3, ordner: W.ordId(3), name: "Notiz.txt", fassungen: [fassung(3, typ: "txt")]),
                      W.datei(4, ordner: W.ordId(3), name: "Im Korb.txt", fassungen: [fassung(4, typ: "txt")], korb: 5),
                      W.datei(5, ordner: W.ordId(4), name: "Im Korbordner.txt", fassungen: [fassung(5, typ: "txt")])])
        let u = AblageAnzeige.uebersicht(z, vorhanden: AblageVorhanden())
        let zeilen = AblageBaum.zeilen(u)
        func kurz(_ z: AblageBaumZeile) -> String {
            switch z {
            case .ordner(_, let n, let t, _, _, let a, let o): return "\(t):O \(n) (\(a))\(o ? "" : " zu")"
            case .datei(let d, let t): return "\(t):D \(d.name)"
            }
        }
        XCTAssertEqual(zeilen.map(kurz), ["0:O Allgemein (1)", "1:D Notiz.txt", "0:O Verträge (2)", "1:O Mietverträge (1)", "2:D Mietvertrag.pdf", "1:D Angebot.pdf"],
                       "Papierkorb (Datei und Ordner) fehlt; Unterordner vor Dateien; nach Namen")
        // Einklappen.
        let zu = AblageBaum.zeilen(u, eingeklappt: [W.ordId(1)])
        XCTAssertEqual(zu.map(kurz), ["0:O Allgemein (1)", "1:D Notiz.txt", "0:O Verträge (2) zu"])
        // Papierkorb.
        XCTAssertEqual(AblageBaum.papierkorb(u).map(\.name), ["Im Korb.txt", "Im Korbordner.txt"])
        XCTAssertEqual(AblageBaum.ordnerPfad(u, ordner: W.ordId(2)), ["Verträge", "Mietverträge"])
        XCTAssertEqual(AblageBaum.ordnerPfad(u, ordner: W.ordId(99)), [])
    }

    /// Eine Datei, deren Ordner dieses Gerät nicht kennt, zählt als „im Papierkorb“ (wie am Desktop) und steht nicht im Baum; kommt
    /// dennoch eine lebende Datei ohne lebenden Ordner vor, erscheint sie unter „Ohne Ordner“, statt zu verschwinden.
    func testDateienOhneBekanntenOrdnerSindImPapierkorbOderUnterOhneOrdner() {
        let z = zustand(ordner: [W.ordner(1, name: "Ablage", klasse: "internal")],
                        dateien: [W.datei(1, ordner: W.ordId(77), name: "Waise.pdf", fassungen: [fassung(1)])])
        let u = AblageAnzeige.uebersicht(z, vorhanden: AblageVorhanden())
        XCTAssertEqual(AblageBaum.zeilen(u).count, 1, "nur der Ordner „Ablage“")
        XCTAssertEqual(AblageBaum.papierkorb(u).map(\.name), ["Waise.pdf"])
        var lebend = u
        lebend.dateien[0].imKorb = false
        let zeilen = AblageBaum.zeilen(lebend)
        XCTAssertEqual(zeilen.count, 3)
        if case .ordner(_, let name, _, _, _, let anzahl, _) = zeilen[1] { XCTAssertEqual(name, "Ohne Ordner"); XCTAssertEqual(anzahl, 1) } else { XCTFail() }
    }

    // MARK: Anker

    func testAnkerLesbar() {
        XCTAssertEqual(AblageAnker.beschreibung("seite=3"), "Seite 3")
        XCTAssertEqual(AblageAnker.beschreibung("absatz=12"), "Absatz 12")
        XCTAssertEqual(AblageAnker.beschreibung("folie=2"), "Folie 2")
        XCTAssertEqual(AblageAnker.beschreibung("zeile=7"), "Zeile 7")
        XCTAssertEqual(AblageAnker.beschreibung("teil=kopf"), "Kopfzeilen")
        XCTAssertEqual(AblageAnker.beschreibung("teil=text"), "Text")
        XCTAssertEqual(AblageAnker.beschreibung("teil=2"), "Teil 2")
        XCTAssertEqual(AblageAnker.beschreibung("blatt=Kosten 2026&zeilen=2-21"), "Blatt „Kosten 2026“, Zeilen 2–21")
        XCTAssertEqual(AblageAnker.beschreibung("blatt=Tabelle1"), "Blatt „Tabelle1“")
        XCTAssertEqual(AblageAnker.beschreibung("irgendwas"), "irgendwas", "Unbekanntes bleibt, wie es ist")
    }

    // MARK: Detail

    func testDetailFassungenKonfliktUndTextstand() throws {
        let basis = fassung(10, zeit: 1_000)
        let a = fassung(11, 10, zeit: 2_000)
        let b = fassung(12, 10, zeit: 3_000, mitText: false)
        let z = zustand(ordner: [W.ordner(1, name: "Verträge", klasse: "internal")],
                        dateien: [W.datei(1, ordner: W.ordId(1), name: "Angebot.pdf", fassungen: [basis, a, b]),
                                  W.datei(2, ordner: W.ordId(1), name: "Bild.png", fassungen: [fassung(20, typ: "png", mitText: false)]),
                                  W.datei(3, ordner: W.ordId(1), name: "Wurzeln.pdf", fassungen: [fassung(30, zeit: 1), fassung(31, zeit: 2)])])
        let da = AblageSpeicherLeer.vorhanden(z, text: [(W.datId(1), a)])
        let d = try XCTUnwrap(AblageAnzeige.detail(z, dat: W.datId(1), vorhanden: da))
        XCTAssertEqual(d.name, "Angebot.pdf")
        XCTAssertEqual(d.ordnerPfad, ["Verträge"])
        XCTAssertEqual(d.zustand, .konflikt)
        XCTAssertTrue(d.konflikt)
        XCTAssertFalse(d.konfliktOhneGemeinsame)
        XCTAssertEqual(d.fassungen.map(\.fas), [W.fasId(12), W.fasId(11), W.fasId(10)], "neueste zuerst")
        XCTAssertEqual(d.fassungen.map(\.istKopf), [true, true, false])
        XCTAssertEqual(d.fassungen.map(\.istAktuell), [true, false, false], "bei Konflikt die jüngste")
        XCTAssertEqual(d.fassungen.map(\.text), [.wartetAufPC, .bereit, .nichtGeladen])
        // Bild: Format ohne Textfassung; Wurzeln: Konflikt ohne gemeinsame Fassung.
        XCTAssertEqual(try XCTUnwrap(AblageAnzeige.detail(z, dat: W.datId(2), vorhanden: AblageVorhanden())).fassungen[0].text, .keinFormat)
        XCTAssertTrue(try XCTUnwrap(AblageAnzeige.detail(z, dat: W.datId(3), vorhanden: AblageVorhanden())).konfliktOhneGemeinsame)
        XCTAssertNil(AblageAnzeige.detail(z, dat: W.datId(99), vorhanden: AblageVorhanden()))
    }

    // MARK: Kontingent

    func testKontingentLesenUndText() throws {
        let k = try XCTUnwrap(AblageKontingent(antwort: Data(#"{"used": 1200000, "quota": 32000000000, "blobs": 4}"#.utf8)))
        XCTAssertEqual(k.belegt, 1_200_000)
        XCTAssertEqual(k.kontingent, 32_000_000_000)
        XCTAssertFalse(k.voll)
        XCTAssertEqual(k.anteil, 1_200_000.0 / 32_000_000_000.0, accuracy: 1e-12)
        XCTAssertTrue(k.text.hasSuffix("belegt"), k.text)
        let voll = try XCTUnwrap(AblageKontingent(antwort: Data(#"{"used": 100, "quota": 100}"#.utf8)))
        XCTAssertTrue(voll.voll)
        XCTAssertTrue(voll.text.hasSuffix("(voll)"), voll.text)
        XCTAssertNil(AblageKontingent(antwort: Data(#"{"used": 5, "quota": 0}"#.utf8)), "kein Kontingent bekannt: keine Zeile")
        XCTAssertNil(AblageKontingent(antwort: Data("kaputt".utf8)))
        XCTAssertNil(AblageKontingent(antwort: Data(#"{"used": 5}"#.utf8)))
    }
}

/// Ein `AblageVorhanden` mit den Texten, die ein Test als geladen angibt (ohne Dateien).
private enum AblageSpeicherLeer {
    static func vorhanden(_ z: AblageZustand, text: [(String, AblageFassung)]) -> AblageVorhanden {
        var v = AblageVorhanden()
        for (dat, f) in text { for t in f.texte { v.setzeText(dat: dat, textrev: t.textrev) } }
        return v
    }
}
#endif
