import XCTest
@testable import VoiceCore
#if canImport(CryptoKit)
import CryptoKit

// Zählerobergrenze `ZAEHLER_MAX = UInt64.max / 2` (Swift-Gegenstück zu lva#452/#450, Fix-Runde zu PR #453):
// `naechsterZaehler` vergibt nie mehr als die Obergrenze (nil statt Umlauf oder Überschreiten, Fehlercode
// `ablage.zaehler_erschoepft`, nichts geschrieben); Datensätze vom Hub mit einem Register-Zähler über der Grenze sind
// ungültig und ruhen wie unlesbare (angehalten, nie überschreiben, nie indizieren, nie laden). Genau die Grenze gilt noch.
// Rust: `zusammenfuehren.rs` (`ZAEHLER_MAX`, `ordner_zaehler_ok`, `datei_zaehler_ok`, `naechster_zaehler`),
// `speicher_sync.rs` (`ordner_gueltig`, `datei_gueltig`), `sync/ablage.rs` (`entschluesseln`). Alles synthetisch.

private typealias W = AblageWelt
private typealias Z = AblageZusammenfuehren
private let MAX = UInt64.max / 2

private func ordner(_ n: Int, register: Int, zaehler: UInt64) -> AblageOrdner {
    var o = W.ordner(n, name: "O\(n)", klasse: "internal")
    switch register {
    case 0: o.eltern.zaehler = zaehler
    case 1: o.name.zaehler = zaehler
    case 2: o.datenklasse.zaehler = zaehler
    case 3: o.nurGeraet.zaehler = zaehler
    case 4: o.zielordnerKi.zaehler = zaehler
    default: o.papierkorb.zaehler = zaehler
    }
    return o
}

private let ordnerRegister = ["eltern", "name", "datenklasse", "nur_geraet", "zielordner_ki", "papierkorb"]
private let dateiRegister = ["ordner", "name", "klasse_min", "papierkorb", "wahlen[].zaehler"]

private func datei(_ n: Int, register: Int, zaehler: UInt64) -> AblageDatei {
    var d = W.datei(n, ordner: W.ordId(1), name: "d\(n).pdf", fassungen: [W.fassung(n, nil, inhalt: "A\(n)")], klasse: "public")
    d.wahlen = [AblageWahl(id: "wahl_" + String(format: "01HZX4Q8E5K2V7M3N9P6R1S0%02X", n), gewaehlt: W.fasId(n), gegen: [], ersetzt: [], zaehler: 1, geraet: W.geraet)]
    switch register {
    case 0: d.ordner.zaehler = zaehler
    case 1: d.name.zaehler = zaehler
    case 2: d.klasseMin.zaehler = zaehler
    case 3: d.papierkorb.zaehler = zaehler
    default: d.wahlen[0].zaehler = zaehler
    }
    return d
}

final class AblageZaehlerTests: XCTestCase {
    // MARK: Obergrenze und nächster Zähler

    func testObergrenzeIstHalberWertebereich() {
        XCTAssertEqual(Z.zaehlerMax, UInt64.max / 2)
        XCTAssertEqual(Z.zaehlerMax, 9_223_372_036_854_775_807)
    }

    func testNaechsterZaehlerNieUeberDerObergrenze() {
        func bestandMit(_ z: UInt64) -> (AblageOrdnerbestand, AblageDateibestand) {
            var o = W.ordner(1)
            o.name.zaehler = z
            var b = AblageOrdnerbestand()
            b[o.id] = o
            return (b, AblageDateibestand())
        }
        let leer = Z.naechsterZaehler(AblageOrdnerbestand(), AblageDateibestand())
        XCTAssertEqual(leer, 1)
        let (b1, d1) = bestandMit(MAX - 2)
        XCTAssertEqual(Z.naechsterZaehler(b1, d1), MAX - 1)
        let (b2, d2) = bestandMit(MAX - 1)
        XCTAssertEqual(Z.naechsterZaehler(b2, d2), MAX, "genau die Obergrenze wird noch vergeben")
        let (b3, d3) = bestandMit(MAX)
        XCTAssertNil(Z.naechsterZaehler(b3, d3), "der Zähler nach der Obergrenze würde sie überschreiten")
        let (b4, d4) = bestandMit(MAX + 1)
        XCTAssertNil(Z.naechsterZaehler(b4, d4))
        let (b5, d5) = bestandMit(UInt64.max - 1)
        XCTAssertNil(Z.naechsterZaehler(b5, d5))
        let (b6, d6) = bestandMit(UInt64.max)
        XCTAssertNil(Z.naechsterZaehler(b6, d6), "kein Umlauf auf 0")
    }

    func testSchreibenOhneNaechstenZaehlerWirftErschoepftUndSchreibtNichts() throws {
        var o = W.ordner(1)
        o.papierkorb.zaehler = MAX
        var b = AblageOrdnerbestand()
        b[o.id] = o
        XCTAssertThrowsError(try Z.naechsterZaehlerOderFehler(b, AblageDateibestand())) { e in
            XCTAssertEqual(e as? AblageError, .zaehlerErschoepft)
            XCTAssertEqual(AblageError.zaehlerErschoepft.code, "ablage.zaehler_erschoepft")
            XCTAssertFalse((e as? LocalizedError)?.errorDescription?.isEmpty ?? true)
        }
        o.papierkorb.zaehler = MAX - 1
        b[o.id] = o
        XCTAssertEqual(try Z.naechsterZaehlerOderFehler(b, AblageDateibestand()), MAX)
    }

    // MARK: Register: genau die Grenze gilt, eine darüber nicht

    func testOrdnerRegisterGrenze() {
        for r in 0..<ordnerRegister.count {
            let ok = ordner(1, register: r, zaehler: MAX), zuHoch = ordner(1, register: r, zaehler: MAX + 1)
            XCTAssertEqual(Z.ordnerZaehlerMax(ok), MAX, ordnerRegister[r])
            XCTAssertTrue(Z.ordnerZaehlerOk(ok), "\(ordnerRegister[r]) == max gilt")
            XCTAssertFalse(Z.ordnerZaehlerOk(zuHoch), "\(ordnerRegister[r]) == max+1 ist ungültig")
            XCTAssertTrue(AblageValidierung.ordnerGueltig(ok), ordnerRegister[r])
            XCTAssertFalse(AblageValidierung.ordnerGueltig(zuHoch), ordnerRegister[r])
            XCTAssertFalse(AblageValidierung.ordnerGueltig(ordner(1, register: r, zaehler: UInt64.max)), "\(ordnerRegister[r]) == UInt64.max")
        }
    }

    func testDateiRegisterGrenze() {
        for r in 0..<dateiRegister.count {
            let ok = datei(1, register: r, zaehler: MAX), zuHoch = datei(1, register: r, zaehler: MAX + 1)
            XCTAssertEqual(Z.dateiZaehlerMax(ok), MAX, dateiRegister[r])
            XCTAssertTrue(Z.dateiZaehlerOk(ok), "\(dateiRegister[r]) == max gilt")
            XCTAssertFalse(Z.dateiZaehlerOk(zuHoch), "\(dateiRegister[r]) == max+1 ist ungültig")
            XCTAssertTrue(AblageValidierung.dateiGueltig(ok), dateiRegister[r])
            XCTAssertFalse(AblageValidierung.dateiGueltig(zuHoch), dateiRegister[r])
            XCTAssertFalse(AblageValidierung.dateiGueltig(datei(1, register: r, zaehler: UInt64.max)), "\(dateiRegister[r]) == UInt64.max")
        }
        // Eine von mehreren Wahlen genügt; `nur_geraet_min` hat keinen Zähler.
        var d = datei(1, register: 4, zaehler: 5)
        d.wahlen.append(AblageWahl(id: "wahl_01HZX4Q8E5K2V7M3N9P6R1S0FF", gewaehlt: W.fasId(1), gegen: [], ersetzt: [], zaehler: MAX + 1, geraet: W.geraet))
        XCTAssertFalse(AblageValidierung.dateiGueltig(d))
        d.nurGeraetMin = true
        d.wahlen[1].zaehler = MAX
        XCTAssertTrue(AblageValidierung.dateiGueltig(d))
    }

    func testGroessterZaehlerBenutztDieselbenHelfer() {
        let o = ordner(1, register: 3, zaehler: 777)
        let d = datei(2, register: 4, zaehler: 888)
        var b = AblageOrdnerbestand()
        b[o.id] = o
        XCTAssertEqual(Z.groessterZaehler(b, AblageDateibestand([(d.id, d)])), 888)
        XCTAssertEqual(Z.ordnerZaehlerMax(o), 777)
        XCTAssertEqual(Z.dateiZaehlerMax(d), 888)
    }

    // MARK: Pull: über der Grenze = unlesbar (angehalten)

    private func angehalten(_ z: AblageZustand, _ coll: String, _ id: String, _ name: String, file: StaticString = #filePath, line: UInt = #line) {
        let s = z.ledger.objekt(coll, id)
        XCTAssertEqual(s?.angehalten, true, "\(name): angehalten", file: file, line: line)
        XCTAssertEqual(s?.hash, "", "\(name): kein Hash eines unlesbaren Stands", file: file, line: line)
    }

    func testPullOrdnerUeberDerGrenzeRuhtGenauDieGrenzeWirdAufgenommen() {
        for r in 0..<ordnerRegister.count {
            var z = W.neuerZustand()
            let ok = ordner(1, register: r, zaehler: MAX)
            W.einspielen(&z, ordner: [W.remote(ordner: ok, revision: 1)])
            XCTAssertNotNil(z.bestand.ordner[ok.id], "\(ordnerRegister[r]) == max wird aufgenommen")
            XCTAssertEqual(z.ledger.objekt("ablage_ordner", ok.id)?.angehalten, false)
            // Eine neuere Revision über der Grenze: unlesbar, der lokale Stand bleibt, kein Hash.
            let vorher = z.bestand
            let zuHoch = ordner(1, register: r, zaehler: MAX + 1)
            let b = W.einspielen(&z, ordner: [W.remote(ordner: zuHoch, revision: 2)])
            XCTAssertEqual(z.bestand, vorher, "\(ordnerRegister[r]): nichts überschrieben")
            angehalten(z, "ablage_ordner", ok.id, ordnerRegister[r])
            XCTAssertTrue(b.ordner.warnungen.contains(.unlesbar), ordnerRegister[r])
            XCTAssertTrue(b.ordner.vollstaendig, "kein fremder Schlüssel")
            // Ein Stand ohne Vorgänger wird gar nicht erst aufgenommen.
            var z2 = W.neuerZustand()
            W.einspielen(&z2, ordner: [W.remote(ordner: zuHoch, revision: 1)])
            XCTAssertTrue(z2.bestand.ordner.isEmpty, ordnerRegister[r])
            angehalten(z2, "ablage_ordner", zuHoch.id, ordnerRegister[r])
        }
    }

    func testPullDateiUeberDerGrenzeRuhtGenauDieGrenzeWirdAufgenommen() {
        for r in 0..<dateiRegister.count {
            var z = W.neuerZustand()
            let ok = datei(1, register: r, zaehler: MAX)
            W.einspielen(&z, dateien: [W.remote(datei: ok, revision: 1)])
            XCTAssertNotNil(z.bestand.dateien[ok.id], "\(dateiRegister[r]) == max wird aufgenommen")
            XCTAssertEqual(z.ledger.objekt("ablage_datei", ok.id)?.angehalten, false)
            let vorher = z.bestand
            let zuHoch = datei(1, register: r, zaehler: MAX + 1)
            let b = W.einspielen(&z, dateien: [W.remote(datei: zuHoch, revision: 2)])
            XCTAssertEqual(z.bestand, vorher, "\(dateiRegister[r]): nichts überschrieben")
            angehalten(z, "ablage_datei", ok.id, dateiRegister[r])
            XCTAssertTrue(b.dateien.warnungen.contains(.unlesbar), dateiRegister[r])
            var z2 = W.neuerZustand()
            W.einspielen(&z2, dateien: [W.remote(datei: zuHoch, revision: 1)])
            XCTAssertTrue(z2.bestand.dateien.isEmpty, dateiRegister[r])
        }
    }

    func testPullUeberDerGrenzeKeinIndexKeinDownloadKeinUeberschreibenDurchLesbaren() throws {
        // Datei mit Text, lokal lesbar, dann ein neuerer Stand mit Zähler über der Grenze.
        var f = W.fassung(1, nil, inhalt: "Original 1")
        let t = W.text(zu: f)
        f.texte = [t.eintrag]
        let gut = W.datei(1, ordner: W.ordId(1), name: "gut.pdf", fassungen: [f])
        var z = W.neuerZustand()
        W.einspielen(&z, ordner: [W.remote(ordner: W.ordner(1, klasse: "internal"))], dateien: [W.remote(datei: gut, revision: 1)])
        var da = AblageVorhanden()
        da.setzeText(dat: gut.id, textrev: t.eintrag.textrev)
        XCTAssertEqual(AblageAnzeige.indexQuellen(z, vorhanden: da), [gut.id])
        var hoch = gut
        hoch.name = AblageRegister(wert: "gut.pdf", zaehler: MAX + 1, geraet: W.geraet)
        W.einspielen(&z, dateien: [W.remote(datei: hoch, revision: 2)])
        XCTAssertFalse(AblageAnzeige.darfIndiziert(z, dat: gut.id), "angehalten: nie in den Index")
        XCTAssertEqual(AblageAnzeige.indexQuellen(z, vorhanden: da), [])
        XCTAssertEqual(AblageAnzeige.zustaende(z, vorhanden: da).dateien[gut.id], .angehalten)
        XCTAssertTrue(AblageDownloadPlan.auftraege(zustand: z, vorhanden: AblageVorhanden(), key: W.key).isEmpty, "für Angehaltenes wird nichts geladen")
        XCTAssertNil(AblageDownloadPlan.original(zustand: z, dat: gut.id, fas: W.fasId(1), key: W.key))
        // Ein späterer lesbarer Stand (Zähler im Rahmen) hebt das Anhalten wieder auf.
        var wieder = gut
        wieder.name = AblageRegister(wert: "gut.pdf", zaehler: 5, geraet: W.geraet)
        W.einspielen(&z, dateien: [W.remote(datei: wieder, revision: 3)])
        XCTAssertEqual(z.ledger.objekt("ablage_datei", gut.id)?.angehalten, false)
    }

    func testPullWahlUeberDerGrenzeMachtDieDateiUnlesbar() {
        // Eine Wahl über der Grenze macht die ganze Datei unlesbar, auch wenn alle Register im Rahmen sind.
        var d = datei(1, register: 4, zaehler: MAX + 1)
        d.name.zaehler = 3
        var z = W.neuerZustand()
        let b = W.einspielen(&z, dateien: [W.remote(datei: d, revision: 1)])
        XCTAssertTrue(z.bestand.dateien.isEmpty)
        XCTAssertTrue(b.dateien.warnungen.contains(.unlesbar))
        XCTAssertEqual(b.dateien.angehalten, [AblageLedger.schluessel("ablage_datei", d.id)])
    }
}
#endif
