import XCTest
@testable import VoiceCore
#if canImport(CryptoKit)
import CryptoKit

// Zusatzvektoren gegen den Desktop (D5a-1, lva#441, Epic #395): die Gruppen unter dem Schlüssel `zusatz` der
// gemeinsamen Fixture `docs/superpowers/plans/ablage-testvektoren.json` (Desktop-Test
// `sync::ablage::vektoren_zusatz`, alle Werte aus den echten Desktop-Funktionen). Der Grundsatz (AAD, Siegel,
// textrev, Textfassung, Nutzlast von Datei und Ordner) steht in AblageVectorTests.
//
// Gruppen: escapes, zahlen, umlaut_schluessel, serde_lesefaelle, id_gueltig, tiefe_127, hub_hex,
// grabstein_benutzer, zusammenfuehren. Das Zusammenführen selbst folgt in D5a-2; hier wird geprüft, dass Swift
// Eingaben und Ergebnis liest und byte-identisch zurückschreibt.

private func bytes(_ s: String) -> [UInt8] { Array(s.utf8) }
private func text(_ b: [UInt8]) -> String { String(decoding: b, as: UTF8.self) }

final class AblageRustZusatzTests: XCTestCase {
    private var zusatz: AblageJSON = .null
    private let key: [UInt8] = (0..<32).map { UInt8($0) }

    /// Die Fixture wird mit dem eigenen Leser (`AblageJSON.parse`) gelesen: `JSONSerialization` würde z. B. ein
    /// BOM am Anfang einer Zeichenkette verschlucken und damit einen Ablehnungsfall verfälschen.
    override func setUpWithError() throws {
        let root = try AblageJSON.parse([UInt8](try Data(contentsOf: AblageVectorTests.fixtureURL())))
        zusatz = try XCTUnwrap(root["zusatz"], "Schlüssel `zusatz` fehlt in der Fixture")
    }

    private func gruppe(_ name: String) throws -> AblageJSON { try XCTUnwrap(zusatz[name], name) }

    private func faelle(_ name: String, _ unter: String = "faelle") throws -> [AblageJSON] {
        guard case .array(let l)? = try gruppe(name)[unter] else { throw AblageError.feld("\(name).\(unter)") }
        XCTAssertFalse(l.isEmpty, "\(name).\(unter)")
        return l
    }

    private func str(_ d: AblageJSON, _ k: String) throws -> String {
        guard case .string(let s)? = d[k] else { throw AblageError.feld(k) }
        return s
    }

    /// `null` (z. B. abgelehnte Eingabe) ergibt `nil`.
    private func optStr(_ d: AblageJSON, _ k: String) throws -> String? {
        switch d[k] {
        case .string(let s)?: return s
        case .null?: return nil
        default: throw AblageError.feld(k)
        }
    }

    private func flag(_ d: AblageJSON, _ k: String) throws -> Bool {
        guard case .bool(let b)? = d[k] else { throw AblageError.feld(k) }
        return b
    }

    private func zahl(_ d: AblageJSON, _ k: String) throws -> Int64 {
        guard let v = d[k] else { throw AblageError.feld(k) }
        return try Int64(ablageJSON: v)
    }

    private func liste(_ d: AblageJSON, _ k: String) throws -> [String] {
        guard let v = d[k] else { throw AblageError.feld(k) }
        return try [String](ablageJSON: v)
    }

    // MARK: Roh → kanonisch (Escapes, Zahlen, Schlüsselordnung)

    private func pruefeRohFaelle(_ gruppenname: String) throws {
        var gueltig = 0, ungueltig = 0
        for z in try faelle(gruppenname) {
            let name = try str(z, "name"), ein = try str(z, "ein")
            if try flag(z, "gueltig") {
                gueltig += 1
                let aus = try str(z, "aus")
                XCTAssertEqual(try AblageJSON.parse(bytes(ein)).kanonischerText, aus, "\(gruppenname)/\(name)")
                // Idempotent: die kanonische Form noch einmal gelesen und geschrieben ändert nichts.
                XCTAssertEqual(try AblageJSON.parse(bytes(aus)).kanonischerText, aus, "\(gruppenname)/\(name) idempotent")
            } else {
                ungueltig += 1
                XCTAssertThrowsError(try AblageJSON.parse(bytes(ein)), "\(gruppenname)/\(name) muss abgelehnt werden: \(ein.debugDescription)")
                XCTAssertEqual(z["aus"], .null, "\(gruppenname)/\(name)")
            }
        }
        XCTAssertGreaterThan(gueltig, 2, gruppenname)
        if gruppenname != "umlaut_schluessel" { XCTAssertGreaterThan(ungueltig, 2, gruppenname) }
    }

    func testEscapesWieDesktop() throws { try pruefeRohFaelle("escapes") }

    func testZahlenWieDesktop() throws {
        try pruefeRohFaelle("zahlen")
        // JSON kennt weder NaN noch Unendlich: serde_json schreibt `null`.
        let ne = try faelle("zahlen", "nicht_endlich")
        XCTAssertEqual(ne.count, 3)
        let werte: [String: Double] = ["unendlich": .infinity, "minus_unendlich": -.infinity, "nan": .nan]
        for z in ne {
            let name = try str(z, "name")
            XCTAssertEqual(AblageJSON.double(try XCTUnwrap(werte[name], name)).kanonischerText, try str(z, "aus"), name)
        }
        // Gleitkommazahlen bleiben Gleitkommazahlen: 1.0 wird nicht zur Ganzzahl 1.
        XCTAssertEqual(try AblageJSON.parse(bytes("[1.0,-0.0,1e20,0.1]")),
                       .array([.double(1.0), .double(-0.0), .double(1e20), .double(0.1)]))
    }

    func testSchluesselOrdnungNachBytesWieDesktop() throws {
        try pruefeRohFaelle("umlaut_schluessel")
        // Zusammengesetzt und zerlegt sind zwei Schlüssel (kein Dictionary<String, _>).
        let f = try XCTUnwrap(try faelle("umlaut_schluessel").first { try str($0, "name") == "zusammengesetzt_und_zerlegt" })
        guard case .object(let m) = try AblageJSON.parse(bytes(try str(f, "ein"))) else { return XCTFail("Objekt erwartet") }
        XCTAssertEqual(m.count, 4)
    }

    func testOrdnerNutzlastenMitUmlautSchluesseln() throws {
        let l = try faelle("umlaut_schluessel", "nutzlasten")
        XCTAssertEqual(l.count, 2)
        for z in l {
            let name = try str(z, "name"), kanon = try str(z, "kanonisch"), hash = try str(z, "hash")
            // Aus der serde-Rohform (Strukturreihenfolge) und aus der kanonischen Form gelesen.
            for quelle in [try str(z, "serde_roh"), kanon] {
                let o = try AblageOrdner(nutzlast: bytes(quelle))
                XCTAssertEqual(text(o.kanonischeNutzlast()), kanon, name)
                XCTAssertEqual(o.hash(), hash, name)
            }
        }
        let mitV = try XCTUnwrap(l.first { (try? str($0, "name"))?.contains("v1") == true })
        let o = try AblageOrdner(nutzlast: bytes(try str(mitV, "kanonisch")))
        XCTAssertTrue(o.extra.contains { AblageBytes.equal($0.key, "v") }, "v bleibt in extra und wird wieder geschrieben")
        XCTAssertEqual(o.extra.count, 8)
        XCTAssertNil(o.eltern.wert)
    }

    // MARK: Lesen wie serde

    func testLesenWieSerde() throws {
        let f = try faelle("serde_lesefaelle")
        XCTAssertGreaterThanOrEqual(f.count, 40)
        var angenommen = 0
        for z in f {
            let name = try str(z, "name"), json = try str(z, "json"), art = try str(z, "art"), erwartet = try flag(z, "ok")
            let ok: Bool
            switch art {
            case "datei": ok = (try? AblageDatei(nutzlast: bytes(json))) != nil
            case "ordner": ok = (try? AblageOrdner(nutzlast: bytes(json))) != nil
            default: return XCTFail("unbekannte Art \(art)")
            }
            XCTAssertEqual(ok, erwartet, "Fall \(name)")
            if erwartet { angenommen += 1 }
        }
        XCTAssertGreaterThan(angenommen, 10)
        XCTAssertLessThan(angenommen, f.count)
    }

    func testSchemaMarkeWieDesktop() throws {
        // `as_u64().unwrap_or(1)`: nur eine nichtnegative Ganzzahl zählt, `v` > 1 lässt das Objekt ruhen.
        let l = try faelle("serde_lesefaelle", "schema_marke")
        XCTAssertGreaterThanOrEqual(l.count, 10)
        for z in l {
            let json = try str(z, "json")
            let j = try AblageJSON.parse(bytes(json))
            if try flag(z, "akzeptiert") {
                XCTAssertNoThrow(try j.pruefeSchema(), json)
            } else {
                XCTAssertThrowsError(try j.pruefeSchema(), json) { XCTAssertEqual($0 as? AblageError, .neueresSchema) }
            }
        }
        XCTAssertTrue(l.contains { (try? flag($0, "akzeptiert")) == false })
    }

    func testIdGueltigWieDesktop() throws {
        let l = try faelle("id_gueltig")
        XCTAssertGreaterThan(l.count, 20)
        for (i, z) in l.enumerated() {
            let id = try str(z, "id")
            XCTAssertEqual(AblageId.idGueltig(id, prefix: try str(z, "praefix")), try flag(z, "gueltig"), "Fall \(i): \(id.debugDescription)")
        }
    }

    func testSchachtelungstiefeWieDesktop() throws {
        let t = try gruppe("tiefe_127")
        let arr = Int(try zahl(t, "max_tiefe_array")), obj = Int(try zahl(t, "max_tiefe_objekt"))
        XCTAssertEqual(AblageJSON.maxDepth, arr)
        XCTAssertEqual(AblageJSON.maxDepth, obj)
        let n = AblageJSON.maxDepth
        XCTAssertNoThrow(try AblageJSON.parse(bytes(String(repeating: "{\"a\":", count: n) + "1" + String(repeating: "}", count: n))))
        XCTAssertThrowsError(try AblageJSON.parse(bytes(String(repeating: "{\"a\":", count: n + 1) + "1" + String(repeating: "}", count: n + 1))))
        XCTAssertNoThrow(try AblageJSON.parse(bytes(String(repeating: "[", count: n) + String(repeating: "]", count: n))))
        XCTAssertThrowsError(try AblageJSON.parse(bytes(String(repeating: "[", count: n + 1) + String(repeating: "]", count: n + 1))))
    }

    // MARK: Hub-Kennung und Grabsteine

    func testHubKennungHexEingabenWieDesktop() throws {
        let l = try faelle("hub_hex")
        XCTAssertGreaterThanOrEqual(l.count, 8)
        var treffer = 0
        for z in l {
            let name = try str(z, "name"), eingabe = try str(z, "eingabe")
            let erwartet = try optStr(z, "hub_kennung") // null = abgelehnt
            XCTAssertEqual(AblageKrypto.hubBlobId(key: key, sha256Hex: eingabe), erwartet, "Fall \(name)")
            if erwartet != nil { treffer += 1 }
        }
        // Großbuchstaben und „+x“-Paare nimmt der Desktop an (`from_str_radix`), alles Übrige nicht.
        let nach = Dictionary(uniqueKeysWithValues: try l.map { (try str($0, "name"), try optStr($0, "hub_kennung")) })
        XCTAssertEqual(nach["gross"], nach["klein"])
        XCTAssertNotNil(nach["plus_paar"] ?? nil)
        XCTAssertNotEqual(nach["plus_paar"] ?? nil, nach["klein"] ?? nil)
        XCTAssertEqual(treffer, 3)
    }

    func testGrabsteineAndererBenutzerWieDesktop() throws {
        let l = try faelle("grabstein_benutzer")
        XCTAssertGreaterThanOrEqual(l.count, 3)
        for z in l {
            let user = try zahl(z, "user_id"), coll = try str(z, "collection"), id = try str(z, "object_id")
            let kid = try str(z, "key_id"), payload = try str(z, "payload_base64")
            XCTAssertEqual(kid, SyncCrypto.keyId(key))
            XCTAssertEqual(try AblageKrypto.grabNutzlast(key: key, userId: user, collection: coll, id: id), payload)
            XCTAssertTrue(AblageKrypto.grabEcht(key: key, userId: user, collection: coll, id: id, keyId: kid, payload: payload))
            XCTAssertFalse(AblageKrypto.grabEcht(key: key, userId: 7, collection: coll, id: id, keyId: kid, payload: payload), "anderer Benutzer")
        }
        // Verschiedene Benutzer ergeben verschiedene Siegel für dasselbe Objekt.
        XCTAssertEqual(Set(try l.map { try str($0, "payload_base64") }).count, l.count)
    }

    // MARK: Zusammenführen

    func testZusammenfuehrenEingabenUndErgebnisSindKanonischLesbar() throws {
        let z = try gruppe("zusammenfuehren")
        let d = try XCTUnwrap(z["datei"])
        for k in ["a", "b", "ergebnis"] {
            let kanon = try str(d, k)
            let datei = try AblageDatei(nutzlast: bytes(kanon))
            XCTAssertEqual(text(datei.kanonischeNutzlast(ordnen: false)), kanon, "datei.\(k)")
        }
        let ergebnis = try AblageDatei(nutzlast: bytes(try str(d, "ergebnis")))
        XCTAssertEqual(ergebnis.hash(ordnen: false), try str(d, "ergebnis_hash"))
        XCTAssertEqual(AblageKrypto.hubRefs(datei: ergebnis, key: key), try liste(d, "ergebnis_hub_refs"))
        XCTAssertEqual(ergebnis.geordnet(), ergebnis, "das Ergebnis des Zusammenführens ist schon in der Ablage-Ordnung")
        XCTAssertEqual(ergebnis.extra.map(\.key), ["nur_a", "nur_b", "zukunft"], "extra vereinigt")

        let o = try XCTUnwrap(z["ordner"])
        for k in ["a", "b", "ergebnis"] {
            let kanon = try str(o, k)
            XCTAssertEqual(text(try AblageOrdner(nutzlast: bytes(kanon)).kanonischeNutzlast()), kanon, "ordner.\(k)")
        }
        XCTAssertEqual(try AblageOrdner(nutzlast: bytes(try str(o, "ergebnis"))).hash(), try str(o, "ergebnis_hash"))
        // Das Zusammenführen selbst (a, b -> ergebnis, kommutativ und idempotent) folgt in D5a-2.
    }

    func testUnsortierteDateiTreuUndGeordnetWieDesktop() throws {
        let u = try XCTUnwrap(try gruppe("zusammenfuehren")["unsortiert"])
        let d = try AblageDatei(nutzlast: bytes(try str(u, "serde_roh")))
        // `ordnen: false` = `hash_datei` (sortiert nicht, leitet nur blob_refs neu ab) ...
        XCTAssertEqual(text(d.kanonischeNutzlast(ordnen: false)), try str(u, "treu_kanonisch"))
        XCTAssertEqual(d.hash(ordnen: false), try str(u, "treu_hash"))
        // ... `ordnen: true` = Ergebnis von `zusammenfuehren_datei(d, d)` (Ordnung der Listen).
        XCTAssertEqual(text(d.kanonischeNutzlast(ordnen: true)), try str(u, "sortiert_kanonisch"))
        XCTAssertEqual(d.hash(ordnen: true), try str(u, "sortiert_hash"))
        XCTAssertNotEqual(try str(u, "treu_hash"), try str(u, "sortiert_hash"))
        XCTAssertEqual(d.blobRefs, ["veraltet"], "gespeichertes blob_refs bleibt beim Lesen sichtbar, beim Schreiben wird neu abgeleitet")
        XCTAssertFalse(try str(u, "treu_kanonisch").contains("veraltet"))
        XCTAssertEqual(AblageKrypto.hubRefs(datei: d, key: key), try liste(u, "hub_refs"))
        // Zahlen über 2^53, null und das Zusatzfeld bleiben.
        let treu = try str(u, "treu_kanonisch")
        XCTAssertTrue(treu.contains("\"zaehler\":9007199254740993") && treu.contains("\"size\":18446744073709551615"))
        XCTAssertTrue(treu.contains("\"vorgaenger\":null") && treu.contains("\"zeta\":null"))
        XCTAssertEqual(d.extra.map(\.key), ["zusatz"])
        // Fixpunkt beider Formen.
        for k in ["treu_kanonisch", "sortiert_kanonisch"] {
            let kanon = try str(u, k)
            XCTAssertEqual(text(try AblageDatei(nutzlast: bytes(kanon)).kanonischeNutzlast(ordnen: false)), kanon, k)
        }
    }

    func testGleicheKennungVerdichtetErstDasZusammenfuehren() throws {
        // Zwei Einträge mit gleicher `fas` / Wahl-`id`, aber anderem Inhalt: `zusammenfuehren_datei` behält je
        // Kennung das Größere (`verdichtet_*`), `hash_datei` ohne Zusammenführen verdichtet nichts (`treu_*`).
        // `geordnet()` ordnet und fasst nur genau gleiche Einträge zusammen; die Verdichtung folgt in D5a-2.
        let g = try XCTUnwrap(try gruppe("zusammenfuehren")["gleiche_kennung"])
        let d = try AblageDatei(nutzlast: bytes(try str(g, "serde_roh")))
        XCTAssertEqual(text(d.kanonischeNutzlast(ordnen: false)), try str(g, "treu_kanonisch"))
        XCTAssertEqual(d.hash(ordnen: false), try str(g, "treu_hash"))
        XCTAssertEqual(d.fassungen.count, 2)
        XCTAssertEqual(d.wahlen.count, 3)
        let v = try AblageDatei(nutzlast: bytes(try str(g, "verdichtet_kanonisch")))
        XCTAssertEqual(v.hash(ordnen: false), try str(g, "verdichtet_hash"))
        XCTAssertEqual(v.fassungen.count, 1)
        XCTAssertEqual(v.wahlen.count, 1)
        // Die Reihenfolge der Einträge mit gleicher Kennung ist von der Eingabe unabhängig.
        var umgekehrt = d
        umgekehrt.fassungen.reverse(); umgekehrt.wahlen.reverse()
        XCTAssertEqual(umgekehrt.kanonischeNutzlast(ordnen: true), d.kanonischeNutzlast(ordnen: true))
    }
}
#endif
