import XCTest
@testable import VoiceCore
#if canImport(CryptoKit)
import CryptoKit

// Verarbeitung einer Pull-Seite der Ablage (D5a-3, lva#441, Abnahme 3 und 4): Öffnen mit AAD, Prüfkette,
// Zusammenführen mit dem lokalen Stand, Grabsteine (nur mit Siegel), Ordnergräber nach dem Datei-Pull, angehaltene
// Stände. Reine Logik ohne Netz und Dateien. Alles synthetisch.

private typealias W = AblageWelt
private let ORD = "ablage_ordner"
private let DAT = "ablage_datei"

private func ersatz(_ n: Int, _ inhalt: String = "A") -> AblageDatei {
    W.datei(n, ordner: W.ordId(1), name: "vertrag\(n).pdf", fassungen: [W.fassung(n, nil, inhalt: inhalt)])
}

final class AblagePullTests: XCTestCase {
    // MARK: Übernehmen

    func testOrdnerUndDateienWerdenUebernommenMitStand() throws {
        var z = W.neuerZustand()
        let o = W.ordner(1, name: "Verträge", klasse: "internal")
        let d = ersatz(1)
        let b = W.einspielen(&z, ordner: [W.remote(ordner: o, revision: 4)], dateien: [W.remote(datei: d, revision: 9)])
        XCTAssertEqual(z.bestand.ordner[o.id]?.name.wert, "Verträge")
        XCTAssertEqual(z.bestand.ordner[o.id]?.datenklasse.wert, "internal")
        XCTAssertEqual(z.bestand.dateien[d.id]?.name.wert, "vertrag1.pdf")
        XCTAssertTrue(b.ordner.geaendert)
        XCTAssertTrue(b.dateien.geaendert)
        XCTAssertTrue(b.ordner.vollstaendig && b.dateien.vollstaendig)
        XCTAssertTrue(b.ordner.warnungen.isEmpty && b.dateien.warnungen.isEmpty)
        // Stand je Objekt: Revision, Hash des Hub-Stands, Hub-Kennungen.
        let so = try XCTUnwrap(z.ledger.objekt(ORD, o.id))
        XCTAssertEqual(so.revision, 4)
        XCTAssertEqual(so.hash, o.hash())
        XCTAssertEqual(so.refs, [])
        XCTAssertFalse(so.angehalten)
        let sd = try XCTUnwrap(z.ledger.objekt(DAT, d.id))
        XCTAssertEqual(sd.revision, 9)
        XCTAssertEqual(sd.hash, d.hash(ordnen: false))
        XCTAssertEqual(sd.refs, W.hubRefs(d))
        XCTAssertFalse(sd.angehalten)
        XCTAssertTrue(z.ledger.lebtAmHub(DAT, d.id))
    }

    func testBekannteRevisionWirdUebersprungenUndErneutesEinspielenAendertNichts() {
        var z = W.neuerZustand()
        let o = W.remote(ordner: W.ordner(1, name: "Alt"), revision: 3)
        W.einspielen(&z, ordner: [o])
        let vorher = z
        // Gleiche und ältere Revision: übersprungen, auch wenn die Nutzlast anders wäre.
        let neuerName = W.remote(ordner: W.ordner(1, name: "Anders", zaehler: 9), revision: 3)
        let aelter = W.remote(ordner: W.ordner(1, name: "Anders", zaehler: 9), revision: 2)
        let b = W.einspielen(&z, ordner: [o, neuerName, aelter])
        XCTAssertEqual(z, vorher)
        XCTAssertFalse(b.ordner.geaendert)
    }

    func testFremdeSammlungWirdIgnoriert() {
        var z = W.neuerZustand()
        let d = ersatz(1)
        // Ein Datei-Objekt auf einer Ordner-Seite: gehört nicht dazu.
        let b = W.einspielen(&z, ordner: [W.remote(datei: d)])
        XCTAssertTrue(z.bestand.dateien.isEmpty)
        XCTAssertNil(z.ledger.objekt(DAT, d.id))
        XCTAssertFalse(b.ordner.geaendert)
    }

    func testZusammenfuehrenMitLokalemStandNieUeberschreiben() throws {
        var z = W.neuerZustand()
        let f1 = W.fassung(1, nil, inhalt: "A", zeit: 1_000)
        var lokal = W.datei(1, ordner: W.ordId(1), name: "a.pdf", fassungen: [f1], zaehler: 5)
        lokal.name = AblageRegister(wert: "lokal.pdf", zaehler: 5, geraet: W.geraet)
        W.einspielen(&z, dateien: [W.remote(datei: lokal, revision: 1)])
        // Entfernt: eine zweite Fassung auf der Vorgängerin (kein Konflikt), Name mit niedrigerem Zähler (verliert).
        let f2 = W.fassung(2, 1, inhalt: "B", zeit: 2_000, geraet: W.geraetB)
        var entfernt = W.datei(1, ordner: W.ordId(1), name: "x.pdf", fassungen: [f1, f2], zaehler: 2, geraet: W.geraetB)
        entfernt.name = AblageRegister(wert: "entfernt.pdf", zaehler: 2, geraet: W.geraetB)
        let b = W.einspielen(&z, dateien: [W.remote(datei: entfernt, revision: 2)])
        let d = try XCTUnwrap(z.bestand.dateien[lokal.id])
        XCTAssertEqual(d.fassungen.map(\.fas), [W.fasId(1), W.fasId(2)], "Vereinigung der Fassungen")
        XCTAssertEqual(d.name.wert, "lokal.pdf", "höherer Zähler gewinnt")
        XCTAssertEqual(d.blobRefs, AblageDatei.inhaltsRefs(d.fassungen))
        XCTAssertTrue(b.dateien.geaendert)
        XCTAssertEqual(AblageZusammenfuehren.koepfe(d), [W.fasId(2)])
        // Hash im Stand = Hash des Hub-Stands, nicht des zusammengeführten.
        XCTAssertEqual(z.ledger.objekt(DAT, lokal.id)?.hash, entfernt.hash(ordnen: false))
    }

    func testKonfliktZweiKoepfeWirdSichtbar() throws {
        var z = W.neuerZustand()
        let wurzel = W.fassung(1, nil, inhalt: "A", zeit: 1_000)
        let a = W.fassung(2, 1, inhalt: "B", zeit: 2_000)
        let b = W.fassung(3, 1, inhalt: "C", zeit: 3_000, geraet: W.geraetB)
        W.einspielen(&z, dateien: [W.remote(datei: W.datei(1, ordner: W.ordId(1), fassungen: [wurzel, a]), revision: 1)])
        W.einspielen(&z, dateien: [W.remote(datei: W.datei(1, ordner: W.ordId(1), fassungen: [wurzel, b], geraet: W.geraetB), revision: 2)])
        let d = try XCTUnwrap(z.bestand.dateien[W.datId(1)])
        XCTAssertEqual(AblageZusammenfuehren.koepfe(d), [W.fasId(2), W.fasId(3)])
        XCTAssertTrue(AblageZusammenfuehren.konflikt(d))
    }

    func testUngueltigeIdWirdVerworfenOhneInhalteInDerWarnung() throws {
        var z = W.neuerZustand()
        // Gültig versiegelt, aber die Kennung ist keine ULID (Pfad-Trenner): wird nie übernommen.
        var kaputt = ersatz(1)
        kaputt.id = "dat_../../geheim"
        kaputt.name = AblageRegister(wert: "GEHEIMER-NAME.pdf", zaehler: 1, geraet: W.geraet)
        let b = W.einspielen(&z, dateien: [W.remote(datei: kaputt, revision: 1)])
        XCTAssertTrue(z.bestand.dateien.isEmpty)
        XCTAssertEqual(b.dateien.verworfen.count, 1)
        XCTAssertTrue(b.dateien.warnungen.contains(.verworfen))
        // Wie der Desktop: lesbar, nur nicht eingespielt; der Stand merkt die Revision (kein ewiges Neuladen).
        XCTAssertEqual(z.ledger.objekt(DAT, kaputt.id)?.angehalten, false)
        for w in b.dateien.warnungen {
            XCTAssertFalse(w.text.contains("GEHEIMER"), "Warnung ohne Namen")
            XCTAssertFalse(w.text.contains("geheim"), "Warnung ohne Kennungen")
        }
    }

    func testNameUndTypPruefungVerwirft() {
        var z = W.neuerZustand()
        let leerName = W.datei(1, ordner: W.ordId(1), name: "", fassungen: [W.fassung(1, nil, inhalt: "A")])
        let steuer = W.datei(2, ordner: W.ordId(1), name: "a\u{0}b.pdf", fassungen: [W.fassung(2, nil, inhalt: "A")])
        let zuLang = W.datei(3, ordner: W.ordId(1), name: String(repeating: "x", count: 256), fassungen: [W.fassung(3, nil, inhalt: "A")])
        let typ = W.datei(4, ordner: W.ordId(1), fassungen: [W.fassung(4, nil, inhalt: "A", typ: "ab/../cdef")])
        let ok255 = W.datei(5, ordner: W.ordId(1), name: String(repeating: "ä", count: 255), fassungen: [W.fassung(5, nil, inhalt: "A")])
        let b = W.einspielen(&z, dateien: [leerName, steuer, zuLang, typ, ok255].map { W.remote(datei: $0) })
        XCTAssertEqual(b.dateien.verworfen.count, 4)
        XCTAssertEqual(z.bestand.dateien.keys, [W.datId(5)])
    }

    // MARK: Prüfkette (Abnahme 4): unlesbar = angehalten, nichts überschrieben

    private func angehalten(_ z: AblageZustand, _ coll: String, _ id: String, file: StaticString = #filePath, line: UInt = #line) {
        let s = z.ledger.objekt(coll, id)
        XCTAssertEqual(s?.angehalten, true, "angehalten", file: file, line: line)
        XCTAssertEqual(s?.hash, "", "kein Hash eines unlesbaren Stands", file: file, line: line)
    }

    func testFremdeKeyIdHaeltAnUndMachtLaufUnvollstaendigAndereObjekteLaufenWeiter() throws {
        var z = W.neuerZustand()
        let fremd = ersatz(1)
        let gut = ersatz(2)
        // Wirklich fremder Schlüssel samt fremder key_id.
        let remoteFremd = W.remote(datei: fremd, revision: 5, keyId: SyncCrypto.keyId(W.andererKey), key: W.andererKey)
        let b = W.einspielen(&z, dateien: [remoteFremd, W.remote(datei: gut, revision: 1)])
        XCTAssertFalse(b.dateien.vollstaendig, "Lauf gilt nicht als vollständig")
        XCTAssertTrue(b.dateien.warnungen.contains(.fremderSchluessel))
        XCTAssertNil(z.bestand.dateien[fremd.id])
        angehalten(z, DAT, fremd.id)
        XCTAssertEqual(z.ledger.objekt(DAT, fremd.id)?.revision, 5)
        XCTAssertEqual(z.ledger.objekt(DAT, fremd.id)?.refs, remoteFremd.blobRefs, "Hub-Kennungen werden vermerkt")
        XCTAssertNotNil(z.bestand.dateien[gut.id], "der Pull geht weiter")
    }

    func testFremdeKeyIdImKopfObwohlEigenerSchluesselHaeltAn() {
        var z = W.neuerZustand()
        let d = ersatz(1)
        // Siegel mit eigener key_id in der AAD, aber der Kopf behauptet eine andere: fremd.
        var r = W.remote(datei: d, revision: 1)
        r.keyId = "deadbeef"
        let b = W.einspielen(&z, dateien: [r])
        XCTAssertFalse(b.dateien.vollstaendig)
        angehalten(z, DAT, d.id)
        XCTAssertTrue(z.bestand.dateien.isEmpty)
    }

    func testKaputtesSiegelHaeltAnUndLokalerStandBleibt() throws {
        var z = W.neuerZustand()
        let d = ersatz(1, "A")
        W.einspielen(&z, dateien: [W.remote(datei: d, revision: 1)])
        let vorher = try XCTUnwrap(z.bestand.dateien[d.id])
        // Neuere Revision mit zerstörtem Chiffrat (ein Bit gekippt) und mit abgeschnittenem Chiffrat.
        var r = W.remote(datei: ersatz(1, "B"), revision: 2)
        var roh = try XCTUnwrap(Data(base64Encoded: r.payload))
        roh[roh.count / 2] ^= 0x01
        r.payload = roh.base64EncodedString()
        var kurz = W.remote(datei: ersatz(1, "B"), revision: 3)
        kurz.payload = String(kurz.payload.prefix(20))
        var keinBase64 = W.remote(datei: ersatz(1, "B"), revision: 4)
        keinBase64.payload = "***nicht-base64***"
        for kaputt in [r, kurz, keinBase64] {
            let b = W.einspielen(&z, dateien: [kaputt])
            XCTAssertTrue(b.dateien.warnungen.contains(.unlesbar))
            angehalten(z, DAT, d.id)
            XCTAssertEqual(z.bestand.dateien[d.id], vorher, "lokaler Stand bleibt unverändert")
            XCTAssertTrue(b.dateien.vollstaendig, "kaputtes Siegel ist kein fremder Schlüssel")
        }
        XCTAssertEqual(z.ledger.objekt(DAT, d.id)?.revision, 4)
    }

    func testNeuereSchemaVersionHaeltAnAuchBeiOrdner() {
        var z = W.neuerZustand()
        let o = W.ordner(1)
        var json = o.ablageJSON
        guard case .object(var m) = json else { return XCTFail() }
        m.append(AblageJSON.Member(key: "v", value: .uint(2)))
        json = .objekt(m.map { ($0.key, $0.value) })
        let r = AblagePullObjekt(collection: ORD, objectId: o.id, revision: 1,
                                 payload: W.siegel(json.kanonisch(), collection: ORD, id: o.id, refs: []), keyId: W.keyId, deleted: false,
                                 deviceId: "d", serverSeq: 1, blobRefs: nil)
        let b = W.einspielen(&z, ordner: [r])
        XCTAssertTrue(b.ordner.warnungen.contains(.unlesbar))
        angehalten(z, ORD, o.id)
        XCTAssertTrue(z.bestand.ordner.isEmpty)
        // v = 1 und fehlendes v sind lesbar.
        var ok = AblageJSON.object([])
        if case .object(var mm) = o.ablageJSON { mm.append(AblageJSON.Member(key: "v", value: .uint(1))); ok = .objekt(mm.map { ($0.key, $0.value) }) }
        let r1 = AblagePullObjekt(collection: ORD, objectId: o.id, revision: 2,
                                  payload: W.siegel(ok.kanonisch(), collection: ORD, id: o.id, refs: []), keyId: W.keyId, deleted: false,
                                  deviceId: "d", serverSeq: 2, blobRefs: nil)
        W.einspielen(&z, ordner: [r1])
        XCTAssertEqual(z.ledger.objekt(ORD, o.id)?.angehalten, false)
        XCTAssertNotNil(z.bestand.ordner[o.id])
    }

    func testIdImInhaltUngleichObjectIdHaeltAn() {
        var z = W.neuerZustand()
        let d = ersatz(1)
        // Die Nutzlast gehört zu dat_1, wird aber unter der Kennung dat_2 gesendet (AAD stimmt zur Kennung dat_2).
        let r = W.remote(datei: d, revision: 1, objectId: W.datId(2))
        let b = W.einspielen(&z, dateien: [r])
        XCTAssertTrue(b.dateien.warnungen.contains(.unlesbar))
        angehalten(z, DAT, W.datId(2))
        XCTAssertTrue(z.bestand.dateien.isEmpty)
        // Ordner genauso.
        let o = W.remote(ordner: W.ordner(1), revision: 1, objectId: W.ordId(2))
        W.einspielen(&z, ordner: [o])
        angehalten(z, ORD, W.ordId(2))
        XCTAssertTrue(z.bestand.ordner.isEmpty)
    }

    func testBlobRefsAnDieAADGebundenUndGegenAbleitungGeprueft() {
        let d = W.datei(1, ordner: W.ordId(1), fassungen: [W.fassung(1, nil, inhalt: "A"), W.fassung(2, 1, inhalt: "B", zeit: 2_000)])
        let richtig = W.hubRefs(d)
        XCTAssertEqual(richtig.count, 2)
        var z = W.neuerZustand()
        // (a) Der Hub liefert andere Refs im Kopf als im Siegel stehen: AAD passt nicht.
        W.einspielen(&z, dateien: [W.remote(datei: d, revision: 1, refsImKopf: [richtig[0]], refsImSiegel: richtig)])
        angehalten(z, DAT, d.id)
        // (b) Kopf und Siegel stimmen überein, aber nicht mit der Ableitung aus der Nutzlast.
        var z2 = W.neuerZustand()
        let falsch = [richtig[0], String(repeating: "a", count: 64)].sorted()
        W.einspielen(&z2, dateien: [W.remote(datei: d, revision: 1, refsImKopf: falsch)])
        angehalten(z2, DAT, d.id)
        // (c) Richtige Menge, aber falsche Reihenfolge: Listenvergleich in Reihenfolge, also unlesbar.
        var z3 = W.neuerZustand()
        W.einspielen(&z3, dateien: [W.remote(datei: d, revision: 1, refsImKopf: richtig.reversed())])
        angehalten(z3, DAT, d.id)
        // (d) Fehlende Refs (Hub ohne blob_refs) bei einer Datei mit Blobs: unlesbar.
        var z4 = W.neuerZustand()
        var ohne = W.remote(datei: d, revision: 1, refsImKopf: [])
        ohne.blobRefs = nil
        W.einspielen(&z4, dateien: [ohne])
        angehalten(z4, DAT, d.id)
        // (e) Gegenprobe: richtig gelesen.
        var z5 = W.neuerZustand()
        W.einspielen(&z5, dateien: [W.remote(datei: d, revision: 1)])
        XCTAssertEqual(z5.ledger.objekt(DAT, d.id)?.angehalten, false)
        XCTAssertNotNil(z5.bestand.dateien[d.id])
    }

    func testOrdnerMitBlobRefsIstUnlesbar() {
        var z = W.neuerZustand()
        let o = W.ordner(1)
        let r = W.remote(ordner: o, revision: 1, refs: ["ab"])
        var mit = r
        mit.blobRefs = ["ab"]
        W.einspielen(&z, ordner: [mit])
        angehalten(z, ORD, o.id)
        XCTAssertTrue(z.bestand.ordner.isEmpty)
    }

    func testAngehaltenerStandWirdNieUeberschriebenUndLoestSichBeiLesbaremStand() throws {
        var z = W.neuerZustand()
        let d1 = ersatz(1, "A")
        W.einspielen(&z, dateien: [W.remote(datei: d1, revision: 1)])
        let lokal = try XCTUnwrap(z.bestand.dateien[d1.id])
        var kaputt = W.remote(datei: ersatz(1, "B"), revision: 2)
        kaputt.payload = "AAAA"
        W.einspielen(&z, dateien: [kaputt])
        angehalten(z, DAT, d1.id)
        XCTAssertEqual(z.bestand.dateien[d1.id], lokal)
        // Ein späterer lesbarer Stand hebt das Anhalten auf und wird zusammengeführt.
        let f2 = W.fassung(2, 1, inhalt: "B", zeit: 2_000)
        W.einspielen(&z, dateien: [W.remote(datei: W.datei(1, ordner: W.ordId(1), name: "vertrag1.pdf", fassungen: [W.fassung(1, nil, inhalt: "A"), f2]), revision: 3)])
        XCTAssertEqual(z.ledger.objekt(DAT, d1.id)?.angehalten, false)
        XCTAssertEqual(z.bestand.dateien[d1.id]?.fassungen.count, 2)
    }

    // MARK: Grabsteine

    func testGrabsteinMitSiegelBegraebtUndKennungKehrtNieZurueck() throws {
        var z = W.neuerZustand()
        let d = ersatz(1)
        W.einspielen(&z, dateien: [W.remote(datei: d, revision: 1)])
        XCTAssertNotNil(z.bestand.dateien[d.id])
        let b = W.einspielen(&z, dateien: [W.remoteGrab(collection: DAT, id: d.id, revision: 2)])
        XCTAssertNil(z.bestand.dateien[d.id])
        XCTAssertTrue(z.ledger.begraben.contains(d.id))
        XCTAssertEqual(b.dateien.begraben, [d.id])
        XCTAssertTrue(b.dateien.geaendert)
        let s = try XCTUnwrap(z.ledger.objekt(DAT, d.id))
        XCTAssertEqual(s.revision, 2)
        XCTAssertTrue(s.deleted)
        XCTAssertFalse(s.angehalten)
        XCTAssertFalse(z.ledger.lebtAmHub(DAT, d.id))
        // Ein später eintreffender (oder erneut gesendeter) lesbarer Stand belebt die Kennung nicht wieder.
        W.einspielen(&z, dateien: [W.remote(datei: d, revision: 3)])
        XCTAssertNil(z.bestand.dateien[d.id], "begrabene Kennungen werden nie wieder aufgenommen")
        XCTAssertTrue(z.ledger.begraben.contains(d.id))
    }

    func testGrabsteinOhneSiegelWirdNichtAngewendetUndHaeltAn() throws {
        var z = W.neuerZustand()
        let d = ersatz(1)
        W.einspielen(&z, dateien: [W.remote(datei: d, revision: 1)])
        let vorher = z.bestand
        // Leere Nutzlast (so sendet `deleteEverywhere` Bücher): für die Ablage ungültig.
        let b = W.einspielen(&z, dateien: [W.remoteGrab(collection: DAT, id: d.id, revision: 2, echt: false)])
        XCTAssertEqual(z.bestand, vorher, "nichts gelöscht")
        XCTAssertFalse(z.ledger.begraben.contains(d.id))
        XCTAssertTrue(b.dateien.begraben.isEmpty)
        XCTAssertTrue(b.dateien.warnungen.contains(.grabsteinOhneSiegel))
        let s = try XCTUnwrap(z.ledger.objekt(DAT, d.id))
        XCTAssertTrue(s.angehalten)
        XCTAssertFalse(s.deleted, "nicht als gelöscht vermerkt")
        XCTAssertEqual(s.revision, 2)
    }

    func testGefaelschteGrabsteineLoeschenNichts() throws {
        let d = ersatz(1)
        let id = d.id
        let gueltigerKlartext = AblageKrypto.grabKlartext(collection: DAT, id: id)
        func versiegelt(klar: [UInt8], coll: String = DAT, objectId: String = id, deleted: Bool = true, kid: String? = nil, key k: [UInt8]? = nil,
                        refs: [String] = []) -> AblagePullObjekt {
            AblagePullObjekt(collection: DAT, objectId: id, revision: 2,
                             payload: W.siegel(klar, collection: coll, id: objectId, refs: refs, keyId: kid, key: k, deleted: deleted),
                             keyId: kid ?? W.keyId, deleted: true, deviceId: "d", serverSeq: 2, blobRefs: nil)
        }
        func json(_ pairs: [(String, AblageJSON)]) -> [UInt8] { AblageJSON.objekt(pairs).kanonisch() }
        let faelle: [(String, AblagePullObjekt)] = [
            ("anderer Schlüssel", versiegelt(klar: gueltigerKlartext, kid: SyncCrypto.keyId(W.andererKey), key: W.andererKey)),
            ("Siegel mit deleted=0 in der AAD", versiegelt(klar: gueltigerKlartext, deleted: false)),
            ("AAD einer anderen Kennung", versiegelt(klar: gueltigerKlartext, objectId: W.datId(9))),
            ("AAD mit Refs", versiegelt(klar: gueltigerKlartext, refs: ["ab"])),
            ("falsche Sammlung im JSON", versiegelt(klar: AblageKrypto.grabKlartext(collection: ORD, id: id))),
            ("falsche Kennung im JSON", versiegelt(klar: AblageKrypto.grabKlartext(collection: DAT, id: W.datId(9)))),
            ("deleted=false im JSON", versiegelt(klar: json([("collection", .string(DAT)), ("deleted", .bool(false)), ("id", .string(id))]))),
            ("deleted fehlt im JSON", versiegelt(klar: json([("collection", .string(DAT)), ("id", .string(id))]))),
            ("kein JSON", versiegelt(klar: Array("gelöscht".utf8))),
            ("key_id im Kopf umgeschrieben", { var r = versiegelt(klar: gueltigerKlartext); r.keyId = "deadbeef"; return r }()),
            ("zerstörtes Chiffrat", { var r = versiegelt(klar: gueltigerKlartext); r.payload = "AAAA" + r.payload.dropFirst(4); return r }()),
            ("ungesiegelt", { var r = versiegelt(klar: gueltigerKlartext); r.payload = ""; return r }()),
        ]
        for (name, grab) in faelle {
            var z = W.neuerZustand()
            W.einspielen(&z, dateien: [W.remote(datei: d, revision: 1)])
            let vorher = z.bestand
            let b = W.einspielen(&z, dateien: [grab])
            XCTAssertEqual(z.bestand, vorher, "\(name): nichts gelöscht")
            XCTAssertFalse(z.ledger.begraben.contains(id), "\(name): nicht begraben")
            XCTAssertEqual(z.ledger.objekt(DAT, id)?.angehalten, true, "\(name): angehalten")
            XCTAssertEqual(z.ledger.objekt(DAT, id)?.deleted, false, "\(name): nicht als gelöscht vermerkt")
            XCTAssertTrue(b.dateien.warnungen.contains(.grabsteinOhneSiegel), "\(name): Warnung")
            XCTAssertTrue(z.ledger.angehalteneKennungen.contains(AblageLedger.schluessel(DAT, id)), name)
            for w in b.dateien.warnungen { XCTAssertFalse(w.text.contains(id), "\(name): Warnung ohne Kennung") }
        }
    }

    func testGrabsteinFuerUnbekannteKennungMerktSieAlsBegraben() {
        var z = W.neuerZustand()
        let id = W.datId(7)
        W.einspielen(&z, dateien: [W.remoteGrab(collection: DAT, id: id, revision: 5)])
        XCTAssertTrue(z.ledger.begraben.contains(id))
        W.einspielen(&z, dateien: [W.remote(datei: ersatz(7), revision: 6)])
        XCTAssertNil(z.bestand.dateien[id])
    }

    func testGrabsteinNurFuerSyncTeilnahme() {
        // Eine Datei, die hier „nur Gerät“ oder `confidential` ist, nimmt nicht am Abgleich teil: ein Grabstein gilt nicht.
        var z = W.neuerZustand()
        let geheim = W.datei(1, ordner: W.ordId(1), name: "g.pdf", fassungen: [W.fassung(1, nil, inhalt: "A")], klasse: "confidential")
        let lokal = W.datei(2, ordner: W.ordId(1), name: "l.pdf", fassungen: [W.fassung(2, nil, inhalt: "A")], nurGeraetMin: true)
        let gut = W.datei(3, ordner: W.ordId(1), name: "p.pdf", fassungen: [W.fassung(3, nil, inhalt: "A")])
        W.einspielen(&z, ordner: [W.remote(ordner: W.ordner(1, klasse: "internal"))],
                     dateien: [geheim, lokal, gut].map { W.remote(datei: $0) })
        let b = W.einspielen(&z, dateien: [geheim, lokal, gut].map { W.remoteGrab(collection: DAT, id: $0.id, revision: 2) })
        XCTAssertNotNil(z.bestand.dateien[geheim.id])
        XCTAssertNotNil(z.bestand.dateien[lokal.id])
        XCTAssertNil(z.bestand.dateien[gut.id])
        XCTAssertEqual(Set(b.dateien.gehalten), [geheim.id, lokal.id])
        XCTAssertEqual(b.dateien.begraben, [gut.id])
    }

    func testOrdnergrabErstNachDemDateiPullUndNieUnterLebenderDatei() throws {
        var z = W.neuerZustand()
        let a = W.ordner(1, name: "A", klasse: "internal")
        let b = W.ordner(2, name: "B", eltern: a.id)
        let d1 = W.datei(1, ordner: b.id, name: "x.pdf", fassungen: [W.fassung(1, nil, inhalt: "A")])
        let d2 = W.datei(2, ordner: a.id, name: "y.pdf", fassungen: [W.fassung(2, nil, inhalt: "B")])
        W.einspielen(&z, ordner: [a, b].map { W.remote(ordner: $0) }, dateien: [d1, d2].map { W.remote(datei: $0) })
        XCTAssertEqual(z.bestand.ordner.count, 2)

        // Seite 1 der Ordner trägt zwei Ordnergräber: sie werden nur gemerkt, noch nicht angewendet.
        let o = AblagePullVerarbeitung.seite(sammlung: ORD, objekte: [W.remoteGrab(collection: ORD, id: a.id, revision: 2),
                                                                     W.remoteGrab(collection: ORD, id: b.id, revision: 2)],
                                             konto: W.konto, zustand: &z)
        XCTAssertEqual(z.bestand.ordner.count, 2, "Ordnergräber erst nach dem Datei-Pull")
        XCTAssertEqual(z.ledger.ordnergraeber.sortiert, [a.id, b.id].sorted())
        XCTAssertTrue(o.begraben.isEmpty)
        // Die Datei-Seite begräbt d1 (unter B) – d2 (unter A) lebt weiter.
        AblagePullVerarbeitung.seite(sammlung: DAT, objekte: [W.remoteGrab(collection: DAT, id: d1.id, revision: 2)], konto: W.konto, zustand: &z)
        let g = AblagePullVerarbeitung.ordnergraeber(zustand: &z)
        // B ist leer: begraben. A trägt noch d2: bleibt als Behälter, wird nie mitgelöscht.
        XCTAssertNil(z.bestand.ordner[b.id])
        XCTAssertNotNil(z.bestand.ordner[a.id])
        XCTAssertNotNil(z.bestand.dateien[d2.id])
        XCTAssertEqual(g.begraben, [b.id])
        XCTAssertEqual(g.gehalten, [a.id])
        XCTAssertTrue(z.ledger.ordnergraeber.isEmpty, "Merkliste ist abgearbeitet")
        XCTAssertTrue(z.ledger.begraben.contains(b.id))
        XCTAssertFalse(z.ledger.begraben.contains(a.id))
    }

    func testOrdnergrabNachBegrabenerDateiNimmtDenOrdnerMit() {
        var z = W.neuerZustand()
        let a = W.ordner(1, name: "A", klasse: "internal")
        let d = W.datei(1, ordner: a.id, fassungen: [W.fassung(1, nil, inhalt: "A")])
        W.einspielen(&z, ordner: [W.remote(ordner: a)], dateien: [W.remote(datei: d)])
        // Gleichzeitig begraben: die Reihenfolge Ordner vor Dateien darf den Ordner nicht halten.
        let b = W.einspielen(&z, ordner: [W.remoteGrab(collection: ORD, id: a.id, revision: 2)],
                             dateien: [W.remoteGrab(collection: DAT, id: d.id, revision: 2)])
        XCTAssertTrue(z.bestand.ordner.isEmpty)
        XCTAssertTrue(z.bestand.dateien.isEmpty)
        XCTAssertEqual(b.graeber.begraben, [a.id])
        XCTAssertTrue(z.ledger.begraben.contains(a.id) && z.ledger.begraben.contains(d.id))
    }

    func testOrdnergrabOhneSiegelWirdNichtGemerkt() {
        var z = W.neuerZustand()
        let a = W.ordner(1, klasse: "internal")
        W.einspielen(&z, ordner: [W.remote(ordner: a)])
        let b = W.einspielen(&z, ordner: [W.remoteGrab(collection: ORD, id: a.id, revision: 2, echt: false)])
        XCTAssertNotNil(z.bestand.ordner[a.id])
        XCTAssertTrue(z.ledger.ordnergraeber.isEmpty)
        XCTAssertEqual(z.ledger.objekt(ORD, a.id)?.angehalten, true)
        XCTAssertTrue(b.ordner.warnungen.contains(.grabsteinOhneSiegel))
        XCTAssertTrue(b.graeber.begraben.isEmpty)
    }

    func testBegrabenerOrdnerWirdNichtWiederAufgenommen() {
        var z = W.neuerZustand()
        let a = W.ordner(1, klasse: "internal")
        W.einspielen(&z, ordner: [W.remote(ordner: a)])
        W.einspielen(&z, ordner: [W.remoteGrab(collection: ORD, id: a.id, revision: 2)])
        XCTAssertNil(z.bestand.ordner[a.id])
        W.einspielen(&z, ordner: [W.remote(ordner: a, revision: 3)])
        XCTAssertNil(z.bestand.ordner[a.id])
    }

    // MARK: Teilnahme und Klasse bleiben aus dem Bestand ableitbar

    func testPapierkorbUndKlasseKommenAusDemBestand() throws {
        var z = W.neuerZustand()
        let wurzel = W.ordner(1, name: "Privat", klasse: "confidential")
        let kind = W.ordner(2, name: "Kind", eltern: wurzel.id)
        let d = W.datei(1, ordner: kind.id, fassungen: [W.fassung(1, nil, inhalt: "A")], klasse: "public", korb: 1_700_000_000_000)
        W.einspielen(&z, ordner: [wurzel, kind].map { W.remote(ordner: $0) }, dateien: [W.remote(datei: d)])
        let eltern = AblageZusammenfuehren.wirksameEltern(z.bestand.ordner)
        let datei = try XCTUnwrap(z.bestand.dateien[d.id])
        XCTAssertEqual(AblageZusammenfuehren.wirksameKlasseDatei(z.bestand.ordner, eltern, datei), .confidential, "Klasse der Kette wird gehoben")
        XCTAssertFalse(AblageZusammenfuehren.dateiLebt(z.bestand.ordner, eltern, datei), "im Papierkorb")
        XCTAssertEqual(datei.papierkorb.wert, 1_700_000_000_000)
    }

    // MARK: Seitenformat

    func testSeiteLesenStrengUndZahlenExakt() throws {
        let json = #"{"objects":[{"collection":"ablage_datei","object_id":"dat_x","revision":9007199254740993,"payload":"AAAA","key_id":"abcd","deleted":false,"device_id":"d","server_seq":9007199254740995,"blob_refs":["r1","r2"]},{"collection":"ablage_ordner","object_id":"ord_x","revision":2,"payload":"","key_id":"","deleted":true,"blob_refs":null,"zusatz":1.5}],"cursor":9007199254740999,"more":true}"#
        let s = try AblagePullSeite(bytes: Array(json.utf8))
        XCTAssertEqual(s.cursor, 9_007_199_254_740_999)
        XCTAssertTrue(s.more)
        XCTAssertEqual(s.objects.count, 2)
        XCTAssertEqual(s.objects[0].revision, 9_007_199_254_740_993, "kein Verlust über Double")
        XCTAssertEqual(s.objects[0].serverSeq, 9_007_199_254_740_995)
        XCTAssertEqual(s.objects[0].blobRefs, ["r1", "r2"])
        XCTAssertNil(s.objects[1].blobRefs)
        XCTAssertTrue(s.objects[1].deleted)
        XCTAssertEqual(s.objects[1].deviceId, "", "fehlendes device_id = leer (serde default)")
        // Fehlende oder falsch typisierte Pflichtfelder machen die ganze Antwort unlesbar (fail-closed).
        for kaputt in [#"{"objects":[],"cursor":1}"#, #"{"objects":[],"more":false}"#, #"{"cursor":1,"more":false}"#,
                       #"{"objects":[{"collection":"c","revision":1}],"cursor":1,"more":false}"#,
                       #"{"objects":[{"collection":"c","object_id":"i"}],"cursor":1,"more":false}"#,
                       #"{"objects":[{"collection":"c","object_id":"i","revision":1.5}],"cursor":1,"more":false}"#,
                       #"{"objects":[{"collection":"c","object_id":"i","revision":"1"}],"cursor":1,"more":false}"#,
                       #"{"objects":[{"collection":"c","object_id":"i","revision":1,"blob_refs":[1]}],"cursor":1,"more":false}"#,
                       #"{"objects":{},"cursor":1,"more":false}"#, "[]", "", "nicht json"] {
            XCTAssertThrowsError(try AblagePullSeite(bytes: Array(kaputt.utf8)), kaputt)
        }
    }

    func testZuGrossePayloadWirdUnlesbarStattSpeicherfresser() {
        var z = W.neuerZustand()
        var r = W.remote(ordner: W.ordner(1), revision: 1)
        r.payload = String(repeating: "A", count: AblagePullVerarbeitung.maxPayloadZeichen + 4)
        let b = W.einspielen(&z, ordner: [r])
        XCTAssertTrue(b.ordner.warnungen.contains(.unlesbar))
        angehalten(z, ORD, W.ordId(1))
    }

    func testWarnTexteTragenKeineKennungen() {
        for w in [AblageWarnung.fremderSchluessel, .grabsteinOhneSiegel, .unlesbar, .verworfen] {
            XCTAssertTrue(w.text.hasPrefix("Ablage:"))
            XCTAssertFalse(w.text.contains("dat_") || w.text.contains("ord_"))
        }
    }
}
#endif
