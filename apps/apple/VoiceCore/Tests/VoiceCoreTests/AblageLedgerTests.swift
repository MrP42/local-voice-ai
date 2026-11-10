import XCTest
@testable import VoiceCore
#if canImport(CryptoKit)

// Ledger und Zustand der Ablage je Konto (D5a-3, lva#441, Epic #395).
//
// Fehlerfälle dieses Pakets (D5a-3) und wo sie abgedeckt sind:
// - Nebenläufigkeit: zwei gleichzeitige Läufe -> AblageLaufTests.testZweiLaeufeGleichzeitigNurEinerLaeuft; der Kern
//   (Ledger, Verarbeitung, Plan) sind Werttypen ohne geteilten Zustand.
// - Abbruch mitten im Vorgang: Zustand wird je Seite atomar geschrieben (temporäre Datei, dann Umbenennen) ->
//   AblageSpeicherTests.testZustandRundlauf…, AblageLaufTests.testAbbruchNachErsterSeiteSetztFortUndDoppeltNichts;
//   Downloads laufen über `eingang/` und werden erst nach Prüfung abgelegt (AblageSpeicherTests, AblageLaufTests).
// - Voller Datenträger/Schreibfehler: AblageSpeicherTests.testSpeichernFehlerLaesstAltenZustandIntakt,
//   AblageLaufTests.testSchreibfehlerBrichtAbOhneCursorVorzuruecken, …testOriginalOhneFreienPlatz….
// - Fehlendes Konto/Netz/WLAN: AblageLaufTests (abgemeldet, Netzfehler stoppt Downloads, WLAN-Regel).
// - Absturz eines Kindprozesses: nicht zutreffend, das iPhone startet keinen (keine Extraktion, Spec §4.1).
// - Feindliche Hub-Daten: AblagePullTests (fremde key_id, Siegel, Schema, Grabstein ohne/gefälschtes Siegel).
// - Echtzeit-Audiopfad: nicht berührt (kein Audio-Callback, keine Allokation im Callback).
// - Migration: neue Datei `ablage-zustand.json` (Version 1), keine Altdaten; unlesbar/neuer/anderes Konto = leer,
//   nie Absturz (dieser Test).

final class AblageLedgerTests: XCTestCase {
    func testKontoKennungFormat() {
        XCTAssertEqual(AblageLedger.kontoKennung(userId: 7, keyId: "0a0b0c0d", hubUrl: "https://hub.example.test"), "7:0a0b0c0d:https://hub.example.test")
        // `trim_end_matches('/')`: alle Schrägstriche am Ende fallen weg.
        XCTAssertEqual(AblageLedger.kontoKennung(userId: 7, keyId: "k", hubUrl: "https://hub.example.test///"), "7:k:https://hub.example.test")
        XCTAssertEqual(AblageLedger.kontoKennung(userId: -3, keyId: "k", hubUrl: "http://127.0.0.1:8765/"), "-3:k:http://127.0.0.1:8765")
    }

    func testSchluesselUndAbfragen() {
        var l = AblageLedger(konto: "k")
        XCTAssertEqual(AblageLedger.schluessel("ablage_datei", "dat_x"), "ablage_datei/dat_x")
        XCTAssertNil(l.objekt("ablage_datei", "dat_x"))
        XCTAssertFalse(l.lebtAmHub("ablage_datei", "dat_x"))
        l.objekte[AblageLedger.schluessel("ablage_datei", "dat_x")] = AblageObjStand(revision: 2, hash: "h", refs: ["a"])
        XCTAssertTrue(l.lebtAmHub("ablage_datei", "dat_x"))
        l.objekte[AblageLedger.schluessel("ablage_datei", "dat_g")] = AblageObjStand.grab(revision: 3, echt: true)
        XCTAssertFalse(l.lebtAmHub("ablage_datei", "dat_g"), "begraben")
        l.objekte[AblageLedger.schluessel("ablage_datei", "dat_u")] = AblageObjStand.grab(revision: 4, echt: false)
        XCTAssertEqual(l.objekt("ablage_datei", "dat_u")?.angehalten, true)
        XCTAssertEqual(l.objekt("ablage_datei", "dat_u")?.deleted, false, "ein Grabstein ohne Siegel gilt nicht als gelöscht")
        XCTAssertEqual(l.angehalteneKennungen, ["ablage_datei/dat_u"])
    }

    func testRundlaufBleibtByteGleichUndKanonisch() throws {
        var l = AblageLedger(konto: "7:abcd:https://hub.example.test")
        l.cursor["ablage_ordner"] = 12
        l.cursor["ablage_datei"] = 345
        l.objekte["ablage_datei/dat_b"] = AblageObjStand(revision: 3, hash: "bb", deleted: false, refs: ["r2", "r1"], angehalten: true)
        l.objekte["ablage_datei/dat_a"] = AblageObjStand(revision: 1, hash: "aa")
        l.begraben.formUnion(["dat_z", "dat_y", "ord_q"])
        l.ordnergraeber.insert("ord_q")
        let bytes = l.ablageJSON.kanonisch()
        let zurueck = try AblageLedger(ablageJSON: try AblageJSON.parse(bytes))
        XCTAssertEqual(zurueck, l)
        XCTAssertEqual(zurueck.ablageJSON.kanonisch(), bytes, "zweimal serialisieren ergibt dieselben Bytes")
        // Reihenfolge der Refs ist Teil des Stands (Listenvergleich in Reihenfolge), sie bleibt erhalten.
        XCTAssertEqual(zurueck.objekte["ablage_datei/dat_b"]?.refs, ["r2", "r1"])
        // Schlüssel stehen sortiert, nicht in Einfügereihenfolge.
        let text = String(decoding: bytes, as: UTF8.self)
        XCTAssertLessThan(try XCTUnwrap(text.range(of: "dat_a")).lowerBound, try XCTUnwrap(text.range(of: "dat_b")).lowerBound)
    }

    func testGrosseZahlenGehenNichtVerloren() throws {
        // Über 2^53: ein Rundlauf über Double würde sie runden.
        let gross = Int64.max - 1
        var l = AblageLedger(konto: "k")
        l.cursor["ablage_datei"] = gross
        l.objekte["ablage_datei/dat_x"] = AblageObjStand(revision: 9_007_199_254_740_993, hash: "h")
        let zurueck = try AblageLedger(ablageJSON: try AblageJSON.parse(l.ablageJSON.kanonisch()))
        XCTAssertEqual(zurueck.cursor["ablage_datei"], gross)
        XCTAssertEqual(zurueck.objekte["ablage_datei/dat_x"]?.revision, 9_007_199_254_740_993)
        XCTAssertTrue(String(decoding: l.ablageJSON.kanonisch(), as: UTF8.self).contains("9007199254740993"))
    }

    func testUnicodeSchluesselBleibenGetrennt() throws {
        // "é" (U+00E9) und "e\u{301}" sind zwei Kennungen (Bytes), nie eine.
        var l = AblageLedger(konto: "k")
        l.objekte["ablage_datei/dat_\u{e9}"] = AblageObjStand(revision: 1, hash: "a")
        l.objekte["ablage_datei/dat_e\u{301}"] = AblageObjStand(revision: 2, hash: "b")
        XCTAssertEqual(l.objekte.count, 2)
        let zurueck = try AblageLedger(ablageJSON: try AblageJSON.parse(l.ablageJSON.kanonisch()))
        XCTAssertEqual(zurueck.objekte.count, 2)
        XCTAssertEqual(zurueck.objekte["ablage_datei/dat_\u{e9}"]?.revision, 1)
        XCTAssertEqual(zurueck.objekte["ablage_datei/dat_e\u{301}"]?.revision, 2)
    }

    // MARK: Zustand (Ledger + Bestand) laden

    func testZustandLadenAnderesKontoIstLeer() throws {
        var z = AblageZustand(konto: "7:aaaa:https://a.example.test")
        z.ledger.cursor["ablage_datei"] = 5
        z.ledger.begraben.insert("dat_x")
        let bytes = z.serialisieren()
        XCTAssertEqual(try AblageZustand(bytes: bytes, konto: "7:aaaa:https://a.example.test"), z)
        // Anderer Hub, anderer Benutzer, anderer Schlüssel: leer, mit der neuen Kennung.
        for andere in ["7:aaaa:https://b.example.test", "8:aaaa:https://a.example.test", "7:bbbb:https://a.example.test"] {
            let leer = AblageZustand.laden(bytes: bytes, konto: andere)
            XCTAssertEqual(leer.zustand, AblageZustand(konto: andere), andere)
            XCTAssertEqual(leer.verlust, .anderesKonto)
        }
    }

    func testZustandLadenUnlesbarNeuerOderLeerIstLeerNieAbsturz() {
        let konto = "7:aaaa:https://a.example.test"
        let kaputt: [[UInt8]] = [
            [], Array("nicht json".utf8), Array("[]".utf8), Array("{}".utf8),
            Array(#"{"v":2,"konto":"7:aaaa:https://a.example.test"}"#.utf8),                       // neuere Version
            Array(#"{"v":1,"konto":"7:aaaa:https://a.example.test","cursor":{"x":"nein"}}"#.utf8), // falscher Typ
            Array(#"{"v":1,"konto":"7:aaaa:https://a.example.test","objekte":{"k":{"hash":"x"}}}"#.utf8), // Pflichtfeld revision fehlt
            Array(#"{"v":1,"konto":"7:aaaa:https://a.example.test","dateien":{"dat_x":{"id":"dat_x"}}}"#.utf8),
        ]
        for b in kaputt {
            let r = AblageZustand.laden(bytes: b, konto: konto)
            XCTAssertEqual(r.zustand, AblageZustand(konto: konto))
            XCTAssertNotEqual(r.verlust, AblageZustand.Verlust.keiner)
        }
        XCTAssertEqual(AblageZustand.laden(bytes: nil, konto: konto).verlust, .fehlt)
    }

    func testZustandFassungUndExtraUeberlebenRundlauf() throws {
        let o = AblageWelt.ordner(1, name: "Verträge")
        var d = AblageWelt.datei(1, ordner: o.id, name: "mietvertrag.pdf", fassungen: [AblageWelt.fassung(1, nil, inhalt: "A")])
        d.extra = [AblageJSON.Member(key: "zusatz", value: .objekt([("n", .uint(UInt64.max)), ("leer", .null)]))]
        var z = AblageZustand(konto: "k")
        z.bestand.ordner[o.id] = o
        z.bestand.dateien[d.id] = d
        let zurueck = try AblageZustand(bytes: z.serialisieren(), konto: "k")
        XCTAssertEqual(zurueck.bestand.dateien[d.id]?.kanonischeNutzlast(ordnen: false), d.kanonischeNutzlast(ordnen: false),
                       "null und UInt64.max im Zusatzfeld bleiben bitgleich")
        XCTAssertEqual(zurueck.bestand.ordner[o.id]?.kanonischeNutzlast(), o.kanonischeNutzlast())
    }
}
#endif
