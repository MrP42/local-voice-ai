import XCTest
@testable import VoiceCore

// Eigenschaftstests des Ablage-Modells (D5a-1, lva#441): ohne Krypto, ohne Rust-Vektoren.
// Die Gegenprobe gegen feste Rust-Werte steht in AblageVectorTests, Siegel und Grabsteine in AblageKryptoTests.

private func parse(_ s: String) throws -> AblageJSON { try AblageJSON.parse(Array(s.utf8)) }

private let G = "g_01HZX4Q8E5K2V7M3N9P6R1S0T1"
private let ORD = "ord_01HZX4Q8E5K2V7M3N9P6R1S0TB"
private let DAT = "dat_01HZX4Q8E5K2V7M3N9P6R1S0TA"

private func reg(_ w: String, z: Int = 1) -> String { "{\"wert\":\(w),\"zaehler\":\(z),\"geraet\":\"\(G)\"}" }

private func ordnerJSON(extra: String = "", eltern: String = "null", elternRegister: String? = nil) -> String {
    "{\"id\":\"\(ORD)\",\"eltern\":\(elternRegister ?? reg(eltern)),\"name\":\(reg("\"Verträge\"")),\"datenklasse\":\(reg("\"erbt\"")),"
        + "\"nur_geraet\":\(reg("false")),\"zielordner_ki\":\(reg("false")),\"papierkorb\":\(reg("null"))\(extra)}"
}

private func dateiJSON(extra: String = "", fassungen: String = "") -> String {
    "{\"id\":\"\(DAT)\",\"ordner\":\(reg("\"\(ORD)\"")),\"name\":\(reg("\"a.txt\"")),\"klasse_min\":\(reg("\"public\"")),"
        + "\"papierkorb\":\(reg("null"))\(fassungen)\(extra)}"
}

final class AblageJSONTests: XCTestCase {
    func testSchluesselWerdenBytegenauGeordnetUndNieVerschmolzen() throws {
        // „é“ zusammengesetzt (C3 A9) und zerlegt (65 CC 81): Swift hält beide Zeichenketten für gleich,
        // Rust (und der Hash) unterscheidet sie. Ein Dictionary<String, _> würde zwei Schlüssel zu einem machen.
        let zerlegt = "e\u{301}", zusammen = "\u{e9}"
        XCTAssertEqual(zerlegt, zusammen, "Voraussetzung: Swift-String-Gleichheit ist kanonische Äquivalenz")
        XCTAssertFalse(AblageBytes.equal(zerlegt, zusammen))
        let j = try parse("{\"\(zusammen)\":1,\"\(zerlegt)\":2,\"B\":3,\"a\":4}")
        guard case .object(let m) = j else { return XCTFail("Objekt erwartet") }
        XCTAssertEqual(m.count, 4, "nicht zu drei Schlüsseln verschmolzen")
        // Großbuchstaben vor Kleinbuchstaben, Umlaute nach ASCII (Bytes, nicht Sprachregeln).
        XCTAssertEqual(j.kanonischerText, "{\"B\":3,\"a\":4,\"\(zerlegt)\":2,\"\(zusammen)\":1}")
    }

    func testGroßUndKleinUndUmlauteNachBytes() {
        let sortiert = AblageBytes.sorted(["äpfel", "zebra", "Äpfel", "Zebra", "apfel", "A\u{308}pfel"], by: { $0 })
        // A\u{308} (41 CC 88) < Zebra (5A) < apfel (61) < zebra (7A) < Äpfel (C3 84) < äpfel (C3 A4)
        XCTAssertEqual(sortiert, ["A\u{308}pfel", "Zebra", "apfel", "zebra", "Äpfel", "äpfel"])
        XCTAssertEqual(AblageBytes.sortedUnique(["b", "a", "b", "A", "a"]), ["A", "a", "b"])
        XCTAssertEqual(AblageBytes.compare("a", "b"), -1)
        XCTAssertEqual(AblageBytes.compare("b", "a"), 1)
        XCTAssertEqual(AblageBytes.compare("x", "x"), 0)
    }

    func testGanzzahlenBleibenExaktBisUInt64() throws {
        let text = "[18446744073709551615,9007199254740993,0,-9223372036854775808,-1]"
        let j = try parse(text)
        XCTAssertEqual(j.kanonischerText, text)
        XCTAssertEqual(j, .array([.uint(UInt64.max), .uint(9007199254740993), .uint(0), .int(Int64.min), .int(-1)]))
    }

    func testNullBleibtUndLeereContainerBleiben() throws {
        let text = "{\"a\":null,\"b\":[],\"c\":{},\"d\":[null,{\"e\":null}]}"
        XCTAssertEqual(try parse(text).kanonischerText, text)
    }

    func testZeichenketteWieSerdeJson() throws {
        // `"`, `\` und Steuerzeichen < 0x20 werden escaped (Kurzformen \b \t \n \f \r, sonst \u00xx klein);
        // 0x7f, `/` und Nicht-ASCII bleiben roh.
        let j = AblageJSON.string("a\u{1}b\u{1f}\u{7f}/é\"\\\n\t\r\u{8}\u{c}😀")
        XCTAssertEqual(j.kanonischerText, "\"a\\u0001b\\u001f\u{7f}/é\\\"\\\\\\n\\t\\r\\b\\f😀\"")
        // Lesen kehrt es um, auch Ersatzpaare und \u-Großbuchstaben.
        XCTAssertEqual(try parse("\"\\u00E9\\ud83d\\ude00\\/\""), .string("é😀/"))
    }

    func testSchraegstrichWirdNichtEscaped() throws {
        // serde_json schreibt `/` roh (entspricht `.withoutEscapingSlashes`); gelesen wird auch die Form `\/`.
        XCTAssertEqual(AblageJSON.string("a/b").kanonischerText, "\"a/b\"")
        XCTAssertEqual(try parse("{\"p\\/q\":\"x\\/y\"}").kanonischerText, "{\"p/q\":\"x/y\"}")
        let f = AblageJSON.objekt([("pfad/name", .array([.string("§ 4/5")]))])
        XCTAssertEqual(f.kanonischerText, "{\"pfad/name\":[\"§ 4/5\"]}")
    }

    func testDoppelteSchluesselLetzterGewinnt() throws {
        XCTAssertEqual(try parse("{\"a\":1,\"a\":2,\"b\":{\"c\":1,\"c\":null}}").kanonischerText, "{\"a\":2,\"b\":{\"c\":null}}")
    }

    func testLeerraumUndSchachtelungWerdenKompakt() throws {
        XCTAssertEqual(try parse(" {\n \"b\" : [ 1 , 2 ] ,\t\"a\" : { } }\r\n").kanonischerText, "{\"a\":{},\"b\":[1,2]}")
    }

    func testUngueltigesJSONWirdAbgelehnt() {
        let schlecht = ["", "  ", "01", "1.", ".5", "+1", "-", "1e", "1e+", "[1,]", "{\"a\":1,}", "{\"a\"}", "{1:2}", "[", "{\"a\":}",
                        "nul", "tru", "'a'", "\"\\x\"", "\"\\u12\"", "\"\\ud800\"", "\"\\udc00\"", "\"\\ud800\\u0041\"", "[1] x", "1e999",
                        "\u{feff}1", "\"a\u{1}b\""]
        for s in schlecht {
            XCTAssertThrowsError(try parse(s), "sollte abgelehnt werden: \(s.debugDescription)") { XCTAssertEqual($0 as? AblageError, .ungueltigesJSON) }
        }
    }

    func testUngueltigesUTF8WirdAbgelehnt() {
        XCTAssertThrowsError(try AblageJSON.parse([0x22, 0xC3, 0x28, 0x22]))      // Fortsetzungsbyte fehlt
        XCTAssertThrowsError(try AblageJSON.parse([0x22, 0xFF, 0x22]))
        XCTAssertThrowsError(try AblageJSON.parse([0x22, 0xED, 0xA0, 0x80, 0x22])) // UTF-8-kodiertes Surrogat
        XCTAssertNoThrow(try AblageJSON.parse([0x22, 0xEF, 0xBF, 0xBD, 0x22]))     // ein echtes U+FFFD ist gültig
    }

    func testSchachtelungstiefeWieSerdeJson() throws {
        let n = AblageJSON.maxDepth
        XCTAssertNoThrow(try parse(String(repeating: "[", count: n) + String(repeating: "]", count: n)))
        XCTAssertThrowsError(try parse(String(repeating: "[", count: n + 1) + String(repeating: "]", count: n + 1))) {
            XCTAssertEqual($0 as? AblageError, .zuTief)
        }
        // Sehr tiefe Eingabe stürzt nicht ab, sondern endet mit Fehler.
        XCTAssertThrowsError(try parse(String(repeating: "{\"a\":", count: 100_000)))
    }

    func testObergrenzeDerEingabe() {
        XCTAssertThrowsError(try AblageJSON.parse(Array("[1,2,3]".utf8), maxBytes: 3)) { XCTAssertEqual($0 as? AblageError, .zuGross) }
        XCTAssertNoThrow(try AblageJSON.parse(Array("[1,2,3]".utf8), maxBytes: 7))
    }

    func testFehlerTragenKeineInhalte() {
        let geheim = "GEHEIMER-INHALT"
        for json in ["{\"id\":\"\(geheim)\"}", "{\"v\":2,\"id\":\"\(geheim)\"}", "\(geheim)"] {
            do { _ = try AblageOrdner(nutzlast: Array(json.utf8)); XCTFail() }
            catch { XCTAssertFalse("\(error)".contains(geheim), "Fehlertext darf keinen Inhalt tragen") }
        }
    }
}

final class AblageModelTests: XCTestCase {
    // MARK: extra

    func testExtraBleibtImRundlaufErhalten() throws {
        let extra = ",\"zukunft\":{\"b\":[1,2],\"a\":null,\"c\":18446744073709551615},\"v\":1,\"Ärger\":\"ü\",\"x\":2.5"
        let o = try AblageOrdner(nutzlast: Array(ordnerJSON(extra: extra).utf8))
        XCTAssertEqual(Set(o.extra.map(\.key)), ["zukunft", "v", "Ärger", "x"])
        let aus = o.kanonischeNutzlast()
        let text = String(decoding: aus, as: UTF8.self)
        XCTAssertTrue(text.contains("\"zukunft\":{\"a\":null,\"b\":[1,2],\"c\":18446744073709551615}"), "null und UInt64 bleiben")
        XCTAssertTrue(text.contains("\"v\":1"))
        XCTAssertTrue(text.contains("\"Ärger\":\"ü\""))
        XCTAssertTrue(text.contains("\"x\":2.5"))
        // Zweiter Rundlauf ändert nichts mehr.
        XCTAssertEqual(try AblageOrdner(nutzlast: aus).kanonischeNutzlast(), aus)

        let d = try AblageDatei(nutzlast: Array(dateiJSON(extra: extra).utf8))
        XCTAssertEqual(Set(d.extra.map(\.key)), ["zukunft", "v", "Ärger", "x"])
        let da = d.kanonischeNutzlast(ordnen: false)
        XCTAssertTrue(String(decoding: da, as: UTF8.self).contains("\"zukunft\":{\"a\":null,\"b\":[1,2],\"c\":18446744073709551615}"))
        XCTAssertEqual(try AblageDatei(nutzlast: da).kanonischeNutzlast(ordnen: false), da)
    }

    func testBekannteFelderGewinnenUeberGleichnamigesExtra() {
        var o = try! AblageOrdner(nutzlast: Array(ordnerJSON().utf8))
        o.extra = [AblageJSON.Member(key: "name", value: .string("böse")), AblageJSON.Member(key: "id", value: .null)]
        let text = String(decoding: o.kanonischeNutzlast(), as: UTF8.self)
        XCTAssertFalse(text.contains("böse"))
        XCTAssertTrue(text.contains("\"id\":\"\(ORD)\""))
    }

    // MARK: null

    func testNullWirdGeschriebenNieGestrichen() throws {
        let o = try AblageOrdner(nutzlast: Array(ordnerJSON().utf8))
        XCTAssertNil(o.eltern.wert)
        XCTAssertNil(o.papierkorb.wert)
        let text = String(decoding: o.kanonischeNutzlast(), as: UTF8.self)
        XCTAssertTrue(text.contains("\"eltern\":{\"geraet\":\"\(G)\",\"wert\":null,\"zaehler\":1}"), text)
        XCTAssertTrue(text.contains("\"papierkorb\":{\"geraet\":\"\(G)\",\"wert\":null,\"zaehler\":1}"), text)

        let f = AblageFassung(fas: "fas_x", vorgaenger: nil, blob: "b", size: 1, originalSha256: "b", typ: "txt", geraet: G, zeit: 5)
        XCTAssertTrue(f.ablageJSON.kanonischerText.contains("\"vorgaenger\":null"))
        // Lesen: fehlendes und `null` ergeben dasselbe.
        let o2 = try AblageOrdner(nutzlast: Array(ordnerJSON(elternRegister: "{\"zaehler\":1,\"geraet\":\"\(G)\"}").utf8))
        XCTAssertNil(o2.eltern.wert, "fehlendes wert eines Option-Registers gilt wie bei serde als null")
    }

    func testTextLaesstOptionalesFehlenWieSerde() throws {
        var t = AblageText(textrev: "r", blob: "b", size: 1, extraktor: "e", extraktorVersion: "1", markenVersion: 1, stand: "ok")
        var text = t.ablageJSON.kanonischerText
        XCTAssertFalse(text.contains("code") || text.contains("versuche"), text)
        XCTAssertEqual(text, "{\"blob\":\"b\",\"extraktor\":\"e\",\"extraktor_version\":\"1\",\"marken_version\":1,\"size\":1,\"stand\":\"ok\",\"textrev\":\"r\"}")
        t.code = "extrakt.zeit"; t.versuche = 2; t.versucheZeit = 1
        text = t.ablageJSON.kanonischerText
        XCTAssertTrue(text.contains("\"code\":\"extrakt.zeit\"") && text.contains("\"versuche\":2") && text.contains("\"versuche_zeit\":1"), text)
        t.versuche = 0
        XCTAssertFalse(t.ablageJSON.kanonischerText.contains("\"versuche\":"))
        // Lesen: `code: null` ist keiner, fehlendes `versuche` ist 0.
        let gelesen = try AblageText(ablageJSON: parse("{\"textrev\":\"r\",\"blob\":\"\",\"size\":0,\"extraktor\":\"e\",\"extraktor_version\":\"1\",\"marken_version\":1,\"stand\":\"fehler\",\"code\":null}"))
        XCTAssertNil(gelesen.code)
        XCTAssertEqual(gelesen.versuche, 0)
    }

    func testStandardwerteBeimLesenUndSchreiben() throws {
        let d = try AblageDatei(nutzlast: Array(dateiJSON().utf8))
        XCTAssertFalse(d.nurGeraetMin)
        XCTAssertEqual(d.fassungen, [])
        XCTAssertEqual(d.wahlen, [])
        XCTAssertEqual(d.entfernt, [])
        XCTAssertEqual(d.blobRefs, [])
        let text = String(decoding: d.kanonischeNutzlast(ordnen: true), as: UTF8.self)
        for teil in ["\"nur_geraet_min\":false", "\"fassungen\":[]", "\"wahlen\":[]", "\"entfernt\":[]", "\"blob_refs\":[]"] {
            XCTAssertTrue(text.contains(teil), teil)
        }
    }

    func testPflichtfelderUndTypfehler() {
        XCTAssertThrowsError(try AblageDatei(nutzlast: Array(dateiJSON().replacingOccurrences(of: "\"id\":\"\(DAT)\",", with: "").utf8))) {
            XCTAssertEqual($0 as? AblageError, .feld("id"))
        }
        XCTAssertThrowsError(try AblageDatei(nutzlast: Array(dateiJSON(fassungen: ",\"fassungen\":null").utf8)))
        XCTAssertThrowsError(try AblageDatei(nutzlast: Array(dateiJSON(extra: ",\"nur_geraet_min\":null").utf8)))
        XCTAssertThrowsError(try AblageOrdner(nutzlast: Array("[1,2]".utf8)))
        XCTAssertThrowsError(try AblageOrdner(nutzlast: Array(ordnerJSON().replacingOccurrences(of: "\"zaehler\":1", with: "\"zaehler\":1.0").utf8)))
        XCTAssertThrowsError(try AblageOrdner(nutzlast: Array(ordnerJSON().replacingOccurrences(of: "\"zaehler\":1", with: "\"zaehler\":-1").utf8)))
        XCTAssertNoThrow(try AblageOrdner(nutzlast: Array(ordnerJSON().replacingOccurrences(of: "\"zaehler\":1", with: "\"zaehler\":18446744073709551615").utf8)))
        XCTAssertThrowsError(try AblageOrdner(nutzlast: Array(ordnerJSON().replacingOccurrences(of: "\"zaehler\":1", with: "\"zaehler\":18446744073709551616").utf8)))
    }

    // MARK: Schema

    func testSchemaNeuerAlsEinsWirdAbgelehnt() {
        for extra in [",\"v\":2", ",\"v\":99", ",\"v\":18446744073709551615"] {
            XCTAssertThrowsError(try AblageOrdner(nutzlast: Array(ordnerJSON(extra: extra).utf8))) { XCTAssertEqual($0 as? AblageError, .neueresSchema) }
            XCTAssertThrowsError(try AblageDatei(nutzlast: Array(dateiJSON(extra: extra).utf8))) { XCTAssertEqual($0 as? AblageError, .neueresSchema) }
        }
    }

    func testSchemaEinsFehlendOderKeineGanzzahlGiltAlsEins() throws {
        // Wie der Desktop (`as_u64().unwrap_or(1)`): nur eine nichtnegative Ganzzahl zählt.
        for extra in ["", ",\"v\":1", ",\"v\":0", ",\"v\":\"2\"", ",\"v\":2.0", ",\"v\":-1", ",\"v\":null", ",\"v\":true"] {
            XCTAssertNoThrow(try AblageOrdner(nutzlast: Array(ordnerJSON(extra: extra).utf8)), extra)
            XCTAssertNoThrow(try AblageDatei(nutzlast: Array(dateiJSON(extra: extra).utf8)), extra)
        }
    }

    // MARK: blob_refs

    private func fassung(_ fas: String, blob: String, texte: [String]) -> AblageFassung {
        AblageFassung(fas: fas, vorgaenger: nil, blob: blob, size: 1, originalSha256: blob, typ: "txt", geraet: G, zeit: 1,
                      texte: texte.enumerated().map { i, b in
                          AblageText(textrev: "rev\(i)", blob: b, size: 1, extraktor: "e", extraktorVersion: "1", markenVersion: 1,
                                     stand: b.isEmpty ? "fehler" : "ok")
                      })
    }

    func testBlobRefsWerdenAusDenFassungenAbgeleitet() {
        let fs = [fassung("fas_2", blob: "bb", texte: ["tt", "", "aa"]), fassung("fas_1", blob: "aa", texte: ["Ä", "tt"])]
        // Originale immer, Texte nur mit nicht leerem blob; bytegenau sortiert (Großbuchstaben/Umlaut), eindeutig.
        XCTAssertEqual(AblageDatei.inhaltsRefs(fs), ["aa", "bb", "tt", "Ä"])
        // Wie `blob_refs_aus`: auch ein leerer Original-blob zählt (nur Texte werden gefiltert).
        XCTAssertEqual(AblageDatei.inhaltsRefs([fassung("fas_1", blob: "", texte: [])]), [""])
        XCTAssertEqual(AblageDatei.inhaltsRefs([]), [])
    }

    func testVeraltetesBlobRefsWirdBeimSchreibenErsetzt() throws {
        var d = try AblageDatei(nutzlast: Array(dateiJSON().utf8))
        d.fassungen = [fassung("fas_1", blob: "aa", texte: ["tt"])]
        d.blobRefs = ["veraltet", "zz"]
        let text = String(decoding: d.kanonischeNutzlast(ordnen: false), as: UTF8.self)
        XCTAssertTrue(text.contains("\"blob_refs\":[\"aa\",\"tt\"]"), text)
        XCTAssertFalse(text.contains("veraltet"))
        // Auch beim Lesen bleibt der gespeicherte Wert unverändert sichtbar (Vergleich mit Hub-Refs macht D5a-2).
        let gelesen = try AblageDatei(nutzlast: Array(dateiJSON(extra: ",\"blob_refs\":[\"x\"]").utf8))
        XCTAssertEqual(gelesen.blobRefs, ["x"])
    }

    // MARK: Ordnung

    func testListenordnungBytegenau() {
        let zerlegt = "A\u{308}", gross = "B", klein = "a", umlaut = "\u{c4}"
        var d = try! AblageDatei(nutzlast: Array(dateiJSON().utf8))
        func f(_ fas: String, _ texte: [String]) -> AblageFassung {
            var x = fassung(fas, blob: "aa", texte: [])
            x.texte = texte.map { AblageText(textrev: $0, blob: "t", size: 1, extraktor: "e", extraktorVersion: "1", markenVersion: 1, stand: "ok") }
            return x
        }
        d.fassungen = [f(umlaut, [umlaut, klein, gross]), f(klein, []), f(zerlegt, [zerlegt, umlaut]), f(gross, [])]
        d.wahlen = [umlaut, klein, gross, zerlegt].map { AblageWahl(id: $0, gewaehlt: "x", zaehler: 1, geraet: G) }
        d.entfernt = [umlaut, klein, klein, gross, zerlegt]
        let g = d.geordnet()
        XCTAssertEqual(g.fassungen.map(\.fas), [zerlegt, gross, klein, umlaut])
        XCTAssertEqual(g.fassungen[0].texte.map(\.textrev), [zerlegt, umlaut])
        XCTAssertEqual(g.fassungen[3].texte.map(\.textrev), [gross, klein, umlaut])
        XCTAssertEqual(g.wahlen.map(\.id), [zerlegt, gross, klein, umlaut])
        XCTAssertEqual(g.entfernt, [zerlegt, gross, klein, umlaut], "sortiert und eindeutig")
        XCTAssertEqual(g.geordnet(), g, "idempotent")
        // `ordnen: false` lässt die Reihenfolge, `true` gibt sie in der Ordnung des Zusammenführens aus.
        XCTAssertNotEqual(d.kanonischeNutzlast(ordnen: false), d.kanonischeNutzlast(ordnen: true))
        XCTAssertEqual(g.kanonischeNutzlast(ordnen: false), d.kanonischeNutzlast(ordnen: true))
    }

    // MARK: Zahlentreue (kein Codable)

    func testGleitkommaInExtraBleibtByteGenauImRundlauf() throws {
        // `JSONEncoder` würde 1.0 als `1`, -0.0 als `-0` und 1e20 als `1e+20` schreiben und 1.0 als Ganzzahl lesen.
        // Hier läuft alles über AblageJSON: der Text bleibt wie bei serde_json, der Hash wie am Desktop.
        let extra = ",\"f\":[1.0,-0.0,1e20,0.1,2.5,100.0,18446744073709551615,-0],\"g\":{\"x\":1.0,\"y\":1}"
        let erwartet = "\"f\":[1.0,-0.0,1e+20,0.1,2.5,100.0,18446744073709551615,-0.0],\"g\":{\"x\":1.0,\"y\":1}"
        let o = try AblageOrdner(nutzlast: Array(ordnerJSON(extra: extra).utf8))
        let aus = o.kanonischeNutzlast()
        XCTAssertTrue(String(decoding: aus, as: UTF8.self).contains(erwartet), String(decoding: aus, as: UTF8.self))
        XCTAssertEqual(try AblageOrdner(nutzlast: aus).kanonischeNutzlast(), aus, "zweiter Rundlauf byte-identisch")
        XCTAssertEqual(try AblageOrdner(nutzlast: try AblageOrdner(nutzlast: aus).kanonischeNutzlast()).kanonischeNutzlast(), aus)
        // 1.0 bleibt eine Gleitkommazahl und wird nicht zur Ganzzahl 1.
        XCTAssertEqual(o.extra.first { AblageBytes.equal($0.key, "g") }?.value, .objekt([("x", .double(1.0)), ("y", .uint(1))]))

        let d = try AblageDatei(nutzlast: Array(dateiJSON(extra: extra).utf8))
        for ordnen in [false, true] {
            let da = d.kanonischeNutzlast(ordnen: ordnen)
            let t = String(decoding: da, as: UTF8.self)
            XCTAssertTrue(t.contains("\"f\":[1.0,-0.0,1e+20,0.1,2.5,100.0,18446744073709551615,-0.0]"), t)
            XCTAssertTrue(t.contains("\"g\":{\"x\":1.0,\"y\":1}"), t)
            XCTAssertEqual(try AblageDatei(nutzlast: da).kanonischeNutzlast(ordnen: ordnen), da)
        }
        // Ganze Zahlen über 2^53 und `v` bleiben ebenfalls exakt.
        let g = try AblageOrdner(nutzlast: Array(ordnerJSON(extra: ",\"n\":9007199254740993,\"v\":1").utf8))
        XCTAssertTrue(String(decoding: g.kanonischeNutzlast(), as: UTF8.self).contains("\"n\":9007199254740993"))
    }

    func testNichtEndlicheZahlenWerdenNull() {
        // Wie serde_json: NaN und ±Unendlich ergeben `null`.
        XCTAssertEqual(AblageJSON.array([.double(.infinity), .double(-.infinity), .double(.nan), .double(1.5)]).kanonischerText, "[null,null,null,1.5]")
    }

    func testNeueresSchemaAuchOhneCodableAbgelehnt() {
        XCTAssertThrowsError(try AblageOrdner(nutzlast: Array(ordnerJSON(extra: ",\"v\":2").utf8))) { XCTAssertEqual($0 as? AblageError, .neueresSchema) }
    }

    // MARK: Ordnung der Rust-Typen

    private func wahl(_ id: String, _ gew: String, gegen: [String] = [], ersetzt: [String] = [], z: UInt64, g: String = "g") -> AblageWahl {
        AblageWahl(id: id, gewaehlt: gew, gegen: gegen, ersetzt: ersetzt, zaehler: z, geraet: g)
    }

    private func permutationen<T>(_ xs: [T]) -> [[T]] {
        guard xs.count > 1 else { return [xs] }
        return xs.indices.flatMap { i -> [[T]] in
            var rest = xs
            let x = rest.remove(at: i)
            return permutationen(rest).map { [x] + $0 }
        }
    }

    func testWahlenMitGleicherIdUndAnderemInhaltHabenFesteReihenfolge() throws {
        let w1 = wahl("wahl_b", "f1", z: 2), w2 = wahl("wahl_b", "f1", z: 1), w3 = wahl("wahl_b", "f0", z: 9)
        let w4 = wahl("wahl_a", "f9", z: 7), w5 = wahl("wahl_b", "f1", gegen: ["x"], z: 0)
        // Abgeleitete Ordnung: id, gewaehlt, gegen (Liste lexikographisch, kürzere zuerst), ersetzt, zaehler, geraet.
        let erwartet = [w4, w3, w2, w1, w5]
        var d = try AblageDatei(nutzlast: Array(dateiJSON().utf8))
        for p in permutationen([w1, w2, w3, w4, w5, w1, w2]) {
            d.wahlen = p
            let g = d.geordnet()
            XCTAssertEqual(g.wahlen, erwartet, "Reihenfolge unabhängig von der Eingabe, genau gleiche Einträge nur einmal")
        }
        // Die Ausgabe selbst ist von der Eingabereihenfolge unabhängig (gleicher Hash).
        d.wahlen = [w1, w2, w3, w4, w5]
        let a = d.kanonischeNutzlast(ordnen: true)
        d.wahlen = [w5, w4, w3, w2, w1, w1]
        XCTAssertEqual(d.kanonischeNutzlast(ordnen: true), a)
        XCTAssertEqual(AblageOrdnung.vergleiche(w4, w3), -1)
        XCTAssertEqual(AblageOrdnung.vergleiche(w2, w5), -1, "gegen [] vor [\"x\"], obwohl der Zähler von w5 kleiner ist")
        XCTAssertEqual(AblageOrdnung.vergleiche(w1, w1), 0)
    }

    func testFassungenMitGleicherFasHabenFesteReihenfolgeUndGleicheFallenZusammen() throws {
        func text(_ rev: String, _ blob: String) -> AblageText { AblageText(textrev: rev, blob: blob, size: 1, extraktor: "e", extraktorVersion: "1", markenVersion: 1, stand: "ok") }
        func fas(_ f: String, vorg: String? = nil, zeit: Int64, texte: [AblageText] = []) -> AblageFassung {
            AblageFassung(fas: f, vorgaenger: vorg, blob: "aa", size: 1, originalSha256: "aa", typ: "txt", geraet: "g", zeit: zeit, texte: texte)
        }
        let a = fas("fas_x", zeit: 5), b = fas("fas_x", vorg: "fas_0", zeit: 1), c = fas("fas_x", zeit: 9)
        let d1 = fas("fas_x", zeit: 5, texte: [text("r1", "b1")]), d2 = fas("fas_w", zeit: 5)
        // fas, dann vorgaenger (None vor Some), dann blob … zeit, dann texte (leere Liste vor gefüllter).
        let erwartet = [d2, a, d1, c, b]
        var d = try AblageDatei(nutzlast: Array(dateiJSON().utf8))
        for p in permutationen([a, b, c, d1, d2, a]) {
            d.fassungen = p
            XCTAssertEqual(d.geordnet().fassungen, erwartet)
        }
        // Texte je Fassung: Ordnung über alle Felder, genau gleiche nur einmal.
        let t1 = text("r1", "b1"), t2 = text("r1", "b0"), t3 = text("r0", "zz")
        var f = fas("fas_y", zeit: 1)
        f.texte = [t1, t2, t3, t1]
        d.fassungen = [f]
        XCTAssertEqual(d.geordnet().fassungen[0].texte, [t3, t2, t1])
        XCTAssertEqual(AblageOrdnung.vergleiche([AblageText](), [t1]), -1)
        XCTAssertEqual(AblageOrdnung.vergleiche(nil as String?, "a"), -1)
        XCTAssertEqual(AblageOrdnung.vergleiche(["a"], ["a", "b"]), -1)
        XCTAssertEqual(AblageOrdnung.vergleiche(["b"], ["a", "b"]), 1)
    }

    // MARK: Kennungen

    func testIdGueltig() {
        XCTAssertTrue(AblageId.idGueltig(DAT, prefix: "dat_"))
        XCTAssertTrue(AblageId.idGueltig(DAT.lowercased(), prefix: "dat_".lowercased()), "Kleinbuchstaben gelten")
        XCTAssertFalse(AblageId.idGueltig(DAT, prefix: "ord_"))
        XCTAssertFalse(AblageId.idGueltig(String(DAT.dropLast()), prefix: "dat_"))
        XCTAssertFalse(AblageId.idGueltig(DAT + "A", prefix: "dat_"))
        for schlecht in ["I", "L", "O", "U", "/", ".", "\u{e4}", " ", "_", "-"] {
            XCTAssertFalse(AblageId.idGueltig(String(DAT.dropLast()) + schlecht, prefix: "dat_"), schlecht)
        }
        XCTAssertFalse(AblageId.idGueltig("dat_../../../../etc/passwd_______", prefix: "dat_"))
        XCTAssertFalse(AblageId.idGueltig("", prefix: "dat_"))
    }

    func testUlidKodierungGegenDieTestsDerRustBibliothek() {
        // Werte aus den Tests des `ulid`-Crates (base32.rs `test_valid`).
        XCTAssertEqual(AblageId.ulid([UInt8](repeating: 0x41, count: 16)), "21850M2GA1850M2GA1850M2GA1")
        XCTAssertEqual(AblageId.ulid([0x4d, 0x4e, 0x38, 0x50, 0x51, 0x44, 0x4a, 0x59, 0x45, 0x42, 0x34, 0x33, 0x5a, 0x41, 0x37, 0x56]), "2D9RW50MA499CMAGHM6DD42DTP")
        XCTAssertEqual(AblageId.ulid([UInt8](repeating: 0, count: 16)), "00000000000000000000000000")
        XCTAssertEqual(AblageId.ulid([UInt8](repeating: 0xFF, count: 16)), "7ZZZZZZZZZZZZZZZZZZZZZZZZZ")
        XCTAssertNil(AblageId.ulid([1, 2, 3]))
    }

    func testGleitkommaSchreibweiseWieSerdeJson() {
        let f: [(Double, String)] = [(1.5, "1.5"), (100, "100.0"), (0.1, "0.1"), (1e21, "1e+21"), (1e-7, "1e-7"), (0.00001, "0.00001"),
                                     (1e16, "1e+16"), (1e15, "1000000000000000.0"), (-2.5e-10, "-2.5e-10"), (1.5e300, "1.5e+300"),
                                     (0, "0.0"), (-0.0, "-0.0"), (123456789.125, "123456789.125"), (Double.leastNonzeroMagnitude, "5e-324")]
        for (d, s) in f { XCTAssertEqual(AblageJSON.double(d).kanonischerText, s, "\(d)") }
    }
}
