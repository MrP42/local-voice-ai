import XCTest
@testable import VoiceCore
#if canImport(CryptoKit)
import CryptoKit

// Zusätzliche Gegenprobe gegen den Desktop (D5a-1, lva#441, Epic #395) für Fälle, die die gemeinsame Fixture
// docs/superpowers/plans/ablage-testvektoren.json (lva#442; gelesen in AblageVectorTests) nicht enthält:
// unsortierte Listen (`ordnen`), Zahlenformate, Escapes, bytegenaue Schlüsselordnung, „+“-/Großbuchstaben-Hex,
// Lesen wie serde (45 Fälle), `id_gueltig`-Tabelle, Tiefenlimit, Schema-Marke.
//
// Alle Werte in `RustAblage` stammen aus der Ausgabe eines temporären Rust-Tests am Stand origin/main e92037ad
// (`sync/ablage_tests.rs`, nicht committet; Schlüssel = Bytes 00..1f, Benutzer 7; serde_json 1.0.149 laut
// Cargo.lock), aufgerufen über die echten Desktop-Funktionen `ablage::grab_nutzlast`, `extrakt::textrev`,
// `collect::canonical`, `hash_datei`, `hash_ordner`, `hub_blob`, `hub_refs`, `ablage::id_gueltig`, serde_json.
// Nur synthetische Daten. Temporär bis die Fälle in die Fixture wandern (aus Rust-Testausgabe, temporär bis
// Fixture tooling/fixtures/ablage).

private typealias R = RustAblage
private let key: [UInt8] = (0..<32).map { UInt8($0) }
private func hexString(_ b: [UInt8]) -> String { b.map { String(format: "%02x", $0) }.joined() }
private func bytes(_ s: String) -> [UInt8] { Array(s.utf8) }
private func text(_ b: [UInt8]) -> String { String(decoding: b, as: UTF8.self) }

// Dieselben Kennungen wie im Rust-Test.
private let G1 = "g_01HZX4Q8E5K2V7M3N9P6R1S0T1"
private let G2 = "g_01HZX4Q8E5K2V7M3N9P6R1S0T2"
private let DAT = "dat_01HZX4Q8E5K2V7M3N9P6R1S0TA"
private let ORD = "ord_01HZX4Q8E5K2V7M3N9P6R1S0TB"
private let ORD_E = "ord_01HZX4Q8E5K2V7M3N9P6R1S0TC"
private let FAS1 = "fas_01HZX4Q8E5K2V7M3N9P6R1S0TD"
private let FAS2 = "fas_01HZX4Q8E5K2V7M3N9P6R1S0TE"
private let WAHL1 = "wahl_01HZX4Q8E5K2V7M3N9P6R1S0TF"
private let WAHL2 = "wahl_01HZX4Q8E5K2V7M3N9P6R1S0TG"
private let FAS_X = "fas_01HZX4Q8E5K2V7M3N9P6R1S0TX"
private let FAS_Z = "fas_01HZX4Q8E5K2V7M3N9P6R1S0TZ"

/// Beispieldaten, spiegelbildlich zum Rust-Test (unabhängig aufgebaut, nicht aus den Rust-Strings gelesen).
private enum Beispiel {
    static var textA: AblageText { AblageText(textrev: R.textrev_1, blob: R.textfassung_sha, size: UInt64(R.textfassung_size)!, extraktor: "lva-text", extraktorVersion: "1", markenVersion: 1, stand: "ok") }
    static var textB: AblageText { AblageText(textrev: R.textrev_lva_text_2, blob: R.sha_tb, size: 17, extraktor: "lva-text", extraktorVersion: "2", markenVersion: 1, stand: "kein_text") }
    static var textF: AblageText { AblageText(textrev: R.textrev_2, blob: "", size: 0, extraktor: "pdf-extract", extraktorVersion: "0.12.0+lva1", markenVersion: 1, stand: "fehler", code: "extrakt.zeit", versuche: 2, versucheZeit: 1) }
    static var fassung1: AblageFassung {
        // Texte absteigend nach textrev = nicht in der Ablage-Ordnung (wie im Rust-Test).
        AblageFassung(fas: FAS1, vorgaenger: nil, blob: R.sha_a, size: 10, originalSha256: R.sha_a, typ: "txt", geraet: G1, zeit: 1730000000123,
                      texte: [textA, textB].sorted { $0.textrev > $1.textrev })
    }
    static var fassung2: AblageFassung {
        AblageFassung(fas: FAS2, vorgaenger: FAS1, blob: R.sha_b, size: UInt64.max, originalSha256: R.sha_b, typ: "pdf", geraet: G2, zeit: 1730000500000, texte: [textF])
    }
    static func reg<T: AblageWert>(_ w: T, _ z: UInt64, _ g: String) -> AblageRegister<T> { AblageRegister(wert: w, zaehler: z, geraet: g) }

    static var dateiUnsortiert: AblageDatei {
        AblageDatei(
            id: DAT, ordner: reg(ORD, 4, G1), name: reg("Mietvertrag Größe.pdf", 2, G1), klasseMin: reg("public", 1, G1), nurGeraetMin: false,
            fassungen: [fassung2, fassung1],
            wahlen: [AblageWahl(id: WAHL2, gewaehlt: FAS2, gegen: [FAS1], ersetzt: [WAHL1], zaehler: 9007199254740993, geraet: G2),
                     AblageWahl(id: WAHL1, gewaehlt: FAS1, zaehler: 5, geraet: G1)],
            entfernt: [FAS_Z, FAS_X, FAS_Z], blobRefs: ["stale"], papierkorb: reg(nil as Int64?, 3, G2),
            extra: [AblageJSON.Member(key: "zusatz", value: .objekt([("zeta", .null), ("alpha", .array([.uint(1), .double(2.5), .string("x")])), ("u", .string("ü"))]))])
    }

    static var dateiZweiRefs: AblageDatei {
        var f = fassung1
        f.texte = [textA]
        var d = dateiUnsortiert
        d.fassungen = [f]
        return d
    }

    static var ordner: AblageOrdner {
        let extra = try! AblageJSON.parse(bytes("{\"v\":1,\"zukunft\":{\"b\":[],\"a\":{}},\"zebra\":1,\"Zebra\":2,\"Äpfel\":3,\"A\\u0308pfel\":4,\"äpfel\":5,\"a\":6}"))
        return AblageOrdner(id: ORD, eltern: reg(nil as String?, 3, G1), name: reg("Verträge", 2, G1), datenklasse: reg("erbt", 1, G1),
                            nurGeraet: reg(false, 1, G1), zielordnerKi: reg(true, 1, G2), papierkorb: reg(nil as Int64?, 1, G1),
                            extra: extra.uebrige(ausser: []))
    }

    static var ordner2: AblageOrdner {
        AblageOrdner(id: ORD, eltern: reg(ORD_E as String?, 7, G2), name: reg("Rechnungen 2026", 6, G2), datenklasse: reg("internal", 5, G2),
                     nurGeraet: reg(true, 4, G1), zielordnerKi: reg(false, 1, G1), papierkorb: reg(1730000000000 as Int64?, 8, G2))
    }
}

final class AblageRustZusatzTests: XCTestCase {
    // MARK: Blob-Kennung (Hex → Rohbytes → Kennung)

    func testHubBlobIdentifikationAusHexWieRust() throws {
        XCTAssertEqual(SyncCrypto.keyId(key), R.key_id)
        XCTAssertEqual(AblageKrypto.sha256Hex(bytes("Hallo Buch")), R.sha_a)
        XCTAssertEqual(AblageKrypto.sha256Hex(bytes("Zweites Original")), R.sha_b)
        XCTAssertEqual(AblageKrypto.hubBlobId(key: key, sha256Hex: R.sha_a), R.hub_a)
        XCTAssertEqual(AblageKrypto.hubBlobId(key: key, sha256Hex: R.sha_b), R.hub_b)
        XCTAssertEqual(AblageKrypto.hubBlobId(key: key, sha256Hex: R.sha_tb), R.hub_tb)
        // Gleicher Weg wie die vorhandene Bibliotheks-Kennung aus Rohbytes (DesktopVectorTests: „Hallo Buch“).
        XCTAssertEqual(R.hub_a, "08d8fe6edc4415c512af3a369d59e5473fefa144043c3ca8d6ea8653210a7322")
        XCTAssertEqual(R.hub_a, LibrarySyncFormat.blobId(key: key, plaintextSha256: Data(SHA256.hash(data: Data(bytes("Hallo Buch"))))))
        // Rusts `hub_blob` nimmt Großbuchstaben und „+“-Paare an (from_str_radix), alles Übrige nicht.
        XCTAssertEqual(AblageKrypto.hubBlobId(key: key, sha256Hex: R.hub_a_gross_eingabe), R.hub_a_gross)
        XCTAssertEqual(R.hub_a_gross, R.hub_a)
        XCTAssertEqual(AblageKrypto.hubBlobId(key: key, sha256Hex: R.hub_a_plus_eingabe), R.hub_a_plus == "NONE" ? nil : R.hub_a_plus)
        XCTAssertNil(AblageKrypto.hubBlobId(key: key, sha256Hex: String(R.sha_a.dropLast())))
        XCTAssertNil(AblageKrypto.hubBlobId(key: key, sha256Hex: "zz" + R.sha_a.dropFirst(2)))
        XCTAssertNil(AblageKrypto.hubBlobId(key: key, sha256Hex: R.sha_a + "00"))
        XCTAssertNil(AblageKrypto.hubBlobId(key: key, sha256Hex: "-1" + R.sha_a.dropFirst(2)))
        XCTAssertNil(AblageKrypto.hubBlobId(key: key, sha256Hex: ""))
    }

    func testHubRefsWieRust() {
        XCTAssertEqual(AblageKrypto.hubRefs(datei: Beispiel.dateiUnsortiert, key: key).joined(separator: ","), R.hub_refs_4)
        XCTAssertEqual(AblageKrypto.hubRefs(datei: Beispiel.dateiUnsortiert, key: key).count, 4)
        XCTAssertEqual(AblageKrypto.hubRefs(datei: Beispiel.dateiZweiRefs, key: key).joined(separator: ","), R.hub_refs_2)
        XCTAssertEqual(AblageKrypto.hubRefs(datei: Beispiel.dateiZweiRefs, key: key).count, 2)
    }

    // MARK: AAD

    // MARK: Grabsteine

    func testGrabsteinSiegelWieRust() throws {
        XCTAssertEqual(text(AblageKrypto.grabKlartext(collection: "ablage_datei", id: DAT)), R.grab_klartext_datei)
        XCTAssertEqual(try AblageKrypto.grabNutzlast(key: key, userId: 7, collection: "ablage_datei", id: DAT), R.grab_nutzlast_datei)
        XCTAssertEqual(try AblageKrypto.grabNutzlast(key: key, userId: 7, collection: "ablage_ordner", id: ORD), R.grab_nutzlast_ordner)
        XCTAssertEqual(try AblageKrypto.grabNutzlast(key: key, userId: 8, collection: "ablage_datei", id: DAT), R.grab_nutzlast_datei_benutzer8)
        XCTAssertNotEqual(R.grab_nutzlast_datei, R.grab_nutzlast_datei_benutzer8)
        // Von Rust versiegelte Grabsteine sind echt, an anderem Ort nicht.
        func echt(_ p: String, coll: String = "ablage_datei", id: String = DAT, user: Int64 = 7, keyId: String = R.key_id) -> Bool {
            AblageKrypto.grabEcht(key: key, userId: user, collection: coll, id: id, keyId: keyId, payload: p)
        }
        XCTAssertTrue(echt(R.grab_nutzlast_datei))
        XCTAssertTrue(echt(R.grab_nutzlast_ordner, coll: "ablage_ordner", id: ORD))
        XCTAssertTrue(echt(R.grab_nutzlast_datei_benutzer8, user: 8))
        XCTAssertFalse(echt(R.grab_nutzlast_datei, user: 8))
        XCTAssertFalse(echt(R.grab_nutzlast_datei, coll: "ablage_ordner"))
        XCTAssertFalse(echt(R.grab_nutzlast_ordner, coll: "ablage_datei", id: DAT))
        XCTAssertFalse(echt(R.grab_nutzlast_datei, keyId: "deadbeef"))
        XCTAssertFalse(echt(""))
    }

    // MARK: textrev und Textfassung

    // MARK: Kanonische Nutzlast und Hash

    func testKanonischeNutzlastDateiWieRust() throws {
        let d = Beispiel.dateiUnsortiert
        // `ordnen: false` = Rusts `hash_datei` (blob_refs neu abgeleitet, Reihenfolge unverändert).
        XCTAssertEqual(text(d.kanonischeNutzlast(ordnen: false)), R.datei_treu_kanon)
        XCTAssertEqual(d.hash(ordnen: false), R.datei_treu_hash)
        // `ordnen: true` = Ordnung des Zusammenführens (Rusts `zusammenfuehren_datei(d, d)`).
        XCTAssertEqual(text(d.kanonischeNutzlast(ordnen: true)), R.datei_sortiert_kanon)
        XCTAssertEqual(d.hash(ordnen: true), R.datei_sortiert_hash)
        XCTAssertNotEqual(R.datei_treu_hash, R.datei_sortiert_hash, "Das Beispiel ist wirklich unsortiert")
        XCTAssertFalse(R.datei_treu_kanon.contains("stale"), "veraltetes blob_refs wird neu abgeleitet")
        XCTAssertEqual(R.texte_reihenfolge_unsortiert, Beispiel.fassung1.texte.map(\.textrev).joined(separator: ","))
    }

    func testDateiLesenAusRustAusgabeUndZurueckschreiben() throws {
        // serde-Rohform (Strukturreihenfolge, veraltetes blob_refs) lesen und kanonisch ausgeben.
        let roh = try AblageDatei(nutzlast: bytes(R.datei_serde_roh))
        XCTAssertEqual(text(roh.kanonischeNutzlast(ordnen: false)), R.datei_treu_kanon)
        XCTAssertEqual(text(roh.kanonischeNutzlast(ordnen: true)), R.datei_sortiert_kanon)
        XCTAssertEqual(roh.extra.map(\.key), ["zusatz"])
        XCTAssertEqual(roh.blobRefs, ["stale"])
        // Die kanonische Rust-Nutzlast selbst: lesen → schreiben ist ein Fixpunkt (extra, null, Zahlen bleiben).
        for kanon in [R.datei_treu_kanon, R.datei_sortiert_kanon] {
            let d = try AblageDatei(nutzlast: bytes(kanon))
            XCTAssertEqual(text(d.kanonischeNutzlast(ordnen: false)), kanon)
        }
        XCTAssertEqual(try AblageDatei(nutzlast: bytes(R.datei_sortiert_kanon)).hash(ordnen: false), R.datei_sortiert_hash)
        // Zahlen über 2^53 bleiben exakt.
        XCTAssertTrue(R.datei_sortiert_kanon.contains("\"zaehler\":9007199254740993"))
        XCTAssertTrue(R.datei_sortiert_kanon.contains("\"size\":18446744073709551615"))
        XCTAssertTrue(R.datei_sortiert_kanon.contains("\"vorgaenger\":null"))
    }

    func testKanonischeNutzlastOrdnerWieRust() throws {
        XCTAssertEqual(text(Beispiel.ordner.kanonischeNutzlast()), R.ordner_kanon)
        XCTAssertEqual(Beispiel.ordner.hash(), R.ordner_hash)
        XCTAssertEqual(text(Beispiel.ordner2.kanonischeNutzlast()), R.ordner2_kanon)
        XCTAssertEqual(Beispiel.ordner2.hash(), R.ordner2_hash)
        // Null-Felder und das Zusatzfeld sind in der Rust-Nutzlast enthalten.
        XCTAssertTrue(R.ordner_kanon.contains("\"eltern\":{\"geraet\":\"\(G1)\",\"wert\":null,\"zaehler\":3}"))
        XCTAssertTrue(R.ordner_kanon.contains("\"zukunft\":{\"a\":{},\"b\":[]}"))
        // Zerlegtes und zusammengesetztes „Äpfel“ sind zwei Schlüssel, nach Bytes geordnet.
        XCTAssertTrue(R.ordner_kanon.contains("\"A\u{308}pfel\":4"))
        XCTAssertTrue(R.ordner_kanon.contains("\"Äpfel\":3"))
        // Lesen aus Rusts serde-Rohform und aus der kanonischen Form.
        for quelle in [R.ordner_serde_roh, R.ordner_kanon] {
            let o = try AblageOrdner(nutzlast: bytes(quelle))
            XCTAssertEqual(text(o.kanonischeNutzlast()), R.ordner_kanon)
            XCTAssertEqual(o.hash(), R.ordner_hash)
            XCTAssertEqual(o.extra.count, 8)
            XCTAssertTrue(o.extra.contains { AblageBytes.equal($0.key, "v") }, "v bleibt in extra und wird wieder geschrieben")
        }
        XCTAssertEqual(try AblageOrdner(nutzlast: bytes(R.ordner2_kanon)).hash(), R.ordner2_hash)
    }

    func testKanonischesJSONRohWieRust() throws {
        for z in R.roh_tabelle {
            let j = try AblageJSON.parse(bytes(z.ein))
            XCTAssertEqual(j.kanonischerText, z.aus, "Fall \(z.name)")
            // Idempotent: die Rust-Ausgabe noch einmal gelesen und geschrieben ändert nichts.
            XCTAssertEqual(try AblageJSON.parse(bytes(z.aus)).kanonischerText, z.aus, "Fall \(z.name) idempotent")
        }
    }

    func testLesenWieSerde() {
        // Dieselben Dokumente nimmt Rust (serde_json::from_value) an oder lehnt sie ab.
        XCTAssertFalse(R.dec_tabelle.isEmpty)
        for z in R.dec_tabelle {
            let istDatei = z.name.hasPrefix("d_")
            let ok: Bool
            if istDatei { ok = (try? AblageDatei(nutzlast: bytes(z.json))) != nil } else { ok = (try? AblageOrdner(nutzlast: bytes(z.json))) != nil }
            XCTAssertEqual(ok, z.ok, "Fall \(z.name)")
        }
    }

    func testSchachtelungstiefeWieRust() {
        XCTAssertEqual(AblageJSON.maxDepth, Int(R.tiefe_max_ok))
        XCTAssertEqual(AblageJSON.maxDepth, Int(R.tiefe_max_ok_objekt))
        let n = AblageJSON.maxDepth
        XCTAssertNoThrow(try AblageJSON.parse(bytes(String(repeating: "{\"a\":", count: n) + "1" + String(repeating: "}", count: n))))
        XCTAssertThrowsError(try AblageJSON.parse(bytes(String(repeating: "{\"a\":", count: n + 1) + "1" + String(repeating: "}", count: n + 1))))
    }

    func testSchemaMarkeWieRust() {
        // `as_u64().unwrap_or(1)`: nur eine nichtnegative Ganzzahl zählt.
        XCTAssertEqual(R.schema_v2, "2")
        XCTAssertEqual(R.schema_vstr, "1")
        XCTAssertEqual(R.schema_vfloat, "1")
        XCTAssertEqual(R.schema_vneg, "1")
    }

    // MARK: Standardordner und Kennungen

    func testIdGueltigWieRust() {
        XCTAssertGreaterThan(R.id_tabelle.count, 20)
        for (i, z) in R.id_tabelle.enumerated() {
            XCTAssertEqual(AblageId.idGueltig(z.id, prefix: z.prefix), z.gueltig, "Fall \(i): \(z.id.debugDescription)")
        }
    }
}
#endif

// MARK: - Werte aus der Rust-Testausgabe (temporär bis Fixture tooling/fixtures/ablage)

enum RustAblage {
    static let key_id = "630dcd29"
    static let sha_a = "1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb"
    static let sha_b = "37f648cfa94ffe1c8479b756f2aa94cb9901f907d29e02bce0feafd00e786c49"
    static let sha_tb = "d807a7b92c2ad48493bd673c55cef95f4674d253c5084599e4a3d87c0c05cb41"
    static let hub_a = "08d8fe6edc4415c512af3a369d59e5473fefa144043c3ca8d6ea8653210a7322"
    static let hub_b = "a5bad8d974f50b2bebebb43f3f9593252cbf6386ea3dfb2d065ed23599660e2d"
    static let hub_tb = "1a1720595a79721c028e362dc1cfb0c4b70c6f1a3ba895317fb73295ddaefae8"
    static let hub_a_gross_eingabe = "1FBBB75CD3895BE571826B41FEE492A3CD94509465C7BB9606285675037F24CB"
    static let hub_a_gross = "08d8fe6edc4415c512af3a369d59e5473fefa144043c3ca8d6ea8653210a7322"
    static let hub_a_plus_eingabe = "+1bbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb"
    static let hub_a_plus = "b5095597d43b50feb3edf9bedbddf5dc19e7db165a2f60c198ea8cacca0885de"
    static let textrev_1 = "7da2e2bed4982376d8b8c7cc36ec8c3a1e514209bcf9fab9e56fc4d12acf4293"
    static let textrev_2 = "b6f283a1250098d3a87e08559f14a12331705765aa91d42a8e16943387ace145"
    static let textrev_lva_text_2 = "6606cb21c0ca82428c3c42f4d29f1b44ee63819ba0534e1cc767a3d48b88860c"
    static let textfassung_sha = "fb33111c8c62a25694468653584d83e22de2e0aedf49c0e7d0895b4330478246"
    static let textfassung_size = "373"
    static let texte_reihenfolge_unsortiert = "7da2e2bed4982376d8b8c7cc36ec8c3a1e514209bcf9fab9e56fc4d12acf4293,6606cb21c0ca82428c3c42f4d29f1b44ee63819ba0534e1cc767a3d48b88860c"
    static let datei_treu_kanon = "{\"blob_refs\":[\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"37f648cfa94ffe1c8479b756f2aa94cb9901f907d29e02bce0feafd00e786c49\",\"d807a7b92c2ad48493bd673c55cef95f4674d253c5084599e4a3d87c0c05cb41\",\"fb33111c8c62a25694468653584d83e22de2e0aedf49c0e7d0895b4330478246\"],\"entfernt\":[\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TZ\",\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TX\",\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TZ\"],\"fassungen\":[{\"blob\":\"37f648cfa94ffe1c8479b756f2aa94cb9901f907d29e02bce0feafd00e786c49\",\"fas\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TE\",\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T2\",\"original_sha256\":\"37f648cfa94ffe1c8479b756f2aa94cb9901f907d29e02bce0feafd00e786c49\",\"size\":18446744073709551615,\"texte\":[{\"blob\":\"\",\"code\":\"extrakt.zeit\",\"extraktor\":\"pdf-extract\",\"extraktor_version\":\"0.12.0+lva1\",\"marken_version\":1,\"size\":0,\"stand\":\"fehler\",\"textrev\":\"b6f283a1250098d3a87e08559f14a12331705765aa91d42a8e16943387ace145\",\"versuche\":2,\"versuche_zeit\":1}],\"typ\":\"pdf\",\"vorgaenger\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\",\"zeit\":1730000500000},{\"blob\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"fas\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\",\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T1\",\"original_sha256\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"size\":10,\"texte\":[{\"blob\":\"fb33111c8c62a25694468653584d83e22de2e0aedf49c0e7d0895b4330478246\",\"extraktor\":\"lva-text\",\"extraktor_version\":\"1\",\"marken_version\":1,\"size\":373,\"stand\":\"ok\",\"textrev\":\"7da2e2bed4982376d8b8c7cc36ec8c3a1e514209bcf9fab9e56fc4d12acf4293\"},{\"blob\":\"d807a7b92c2ad48493bd673c55cef95f4674d253c5084599e4a3d87c0c05cb41\",\"extraktor\":\"lva-text\",\"extraktor_version\":\"2\",\"marken_version\":1,\"size\":17,\"stand\":\"kein_text\",\"textrev\":\"6606cb21c0ca82428c3c42f4d29f1b44ee63819ba0534e1cc767a3d48b88860c\"}],\"typ\":\"txt\",\"vorgaenger\":null,\"zeit\":1730000000123}],\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"klasse_min\":{\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T1\",\"wert\":\"public\",\"zaehler\":1},\"name\":{\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T1\",\"wert\":\"Mietvertrag Gr\u{f6}\u{df}e.pdf\",\"zaehler\":2},\"nur_geraet_min\":false,\"ordner\":{\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T1\",\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":4},\"papierkorb\":{\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T2\",\"wert\":null,\"zaehler\":3},\"wahlen\":[{\"ersetzt\":[\"wahl_01HZX4Q8E5K2V7M3N9P6R1S0TF\"],\"gegen\":[\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\"],\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T2\",\"gewaehlt\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TE\",\"id\":\"wahl_01HZX4Q8E5K2V7M3N9P6R1S0TG\",\"zaehler\":9007199254740993},{\"ersetzt\":[],\"gegen\":[],\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T1\",\"gewaehlt\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\",\"id\":\"wahl_01HZX4Q8E5K2V7M3N9P6R1S0TF\",\"zaehler\":5}],\"zusatz\":{\"alpha\":[1,2.5,\"x\"],\"u\":\"\u{fc}\",\"zeta\":null}}"
    static let datei_treu_hash = "12d493568d3469beb30d64a3bb47b5f45bd214aa641257be5f73324c4b5fb069"
    static let datei_sortiert_kanon = "{\"blob_refs\":[\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"37f648cfa94ffe1c8479b756f2aa94cb9901f907d29e02bce0feafd00e786c49\",\"d807a7b92c2ad48493bd673c55cef95f4674d253c5084599e4a3d87c0c05cb41\",\"fb33111c8c62a25694468653584d83e22de2e0aedf49c0e7d0895b4330478246\"],\"entfernt\":[\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TX\",\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TZ\"],\"fassungen\":[{\"blob\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"fas\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\",\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T1\",\"original_sha256\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"size\":10,\"texte\":[{\"blob\":\"d807a7b92c2ad48493bd673c55cef95f4674d253c5084599e4a3d87c0c05cb41\",\"extraktor\":\"lva-text\",\"extraktor_version\":\"2\",\"marken_version\":1,\"size\":17,\"stand\":\"kein_text\",\"textrev\":\"6606cb21c0ca82428c3c42f4d29f1b44ee63819ba0534e1cc767a3d48b88860c\"},{\"blob\":\"fb33111c8c62a25694468653584d83e22de2e0aedf49c0e7d0895b4330478246\",\"extraktor\":\"lva-text\",\"extraktor_version\":\"1\",\"marken_version\":1,\"size\":373,\"stand\":\"ok\",\"textrev\":\"7da2e2bed4982376d8b8c7cc36ec8c3a1e514209bcf9fab9e56fc4d12acf4293\"}],\"typ\":\"txt\",\"vorgaenger\":null,\"zeit\":1730000000123},{\"blob\":\"37f648cfa94ffe1c8479b756f2aa94cb9901f907d29e02bce0feafd00e786c49\",\"fas\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TE\",\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T2\",\"original_sha256\":\"37f648cfa94ffe1c8479b756f2aa94cb9901f907d29e02bce0feafd00e786c49\",\"size\":18446744073709551615,\"texte\":[{\"blob\":\"\",\"code\":\"extrakt.zeit\",\"extraktor\":\"pdf-extract\",\"extraktor_version\":\"0.12.0+lva1\",\"marken_version\":1,\"size\":0,\"stand\":\"fehler\",\"textrev\":\"b6f283a1250098d3a87e08559f14a12331705765aa91d42a8e16943387ace145\",\"versuche\":2,\"versuche_zeit\":1}],\"typ\":\"pdf\",\"vorgaenger\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\",\"zeit\":1730000500000}],\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"klasse_min\":{\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T1\",\"wert\":\"public\",\"zaehler\":1},\"name\":{\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T1\",\"wert\":\"Mietvertrag Gr\u{f6}\u{df}e.pdf\",\"zaehler\":2},\"nur_geraet_min\":false,\"ordner\":{\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T1\",\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":4},\"papierkorb\":{\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T2\",\"wert\":null,\"zaehler\":3},\"wahlen\":[{\"ersetzt\":[],\"gegen\":[],\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T1\",\"gewaehlt\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\",\"id\":\"wahl_01HZX4Q8E5K2V7M3N9P6R1S0TF\",\"zaehler\":5},{\"ersetzt\":[\"wahl_01HZX4Q8E5K2V7M3N9P6R1S0TF\"],\"gegen\":[\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\"],\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T2\",\"gewaehlt\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TE\",\"id\":\"wahl_01HZX4Q8E5K2V7M3N9P6R1S0TG\",\"zaehler\":9007199254740993}],\"zusatz\":{\"alpha\":[1,2.5,\"x\"],\"u\":\"\u{fc}\",\"zeta\":null}}"
    static let datei_sortiert_hash = "9d1eb6724dadecde8ad93dc5305a7f4adcb62dc2b05a3ee16bcf7dca17539729"
    static let hub_refs_4 = "08d8fe6edc4415c512af3a369d59e5473fefa144043c3ca8d6ea8653210a7322,1a1720595a79721c028e362dc1cfb0c4b70c6f1a3ba895317fb73295ddaefae8,9bcc4bdc8ff8581833176b5a6c046338df59cabecaebec42164fbc6e1755acf6,a5bad8d974f50b2bebebb43f3f9593252cbf6386ea3dfb2d065ed23599660e2d"
    static let hub_refs_2 = "08d8fe6edc4415c512af3a369d59e5473fefa144043c3ca8d6ea8653210a7322,9bcc4bdc8ff8581833176b5a6c046338df59cabecaebec42164fbc6e1755acf6"
    static let grab_klartext_datei = "{\"collection\":\"ablage_datei\",\"deleted\":true,\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\"}"
    static let grab_nutzlast_datei = "uKOA1oH2zVWOuM/uK25lbfFN3bikAVJOL2OtUADCJccWaVVImooOiVRpPxD/71CzYsUGQg80Br3+UAAEwvQM9KLUqn+Da1gEY/M3GWh+zQTtwhMMkdLy6uiQmGA4y7bYO48/wKfovddbGuS+F3PajNYhb9MafjThvFs="
    static let grab_nutzlast_ordner = "BN+y6FPaX3rjkZze0F9b6txpMMSJ+a3ebq8DhTh4BmToFbow48XKZkQFkk1p3WtR49pqICyvwFcYlXOB8jIYyII9zoml75H1TFfuJZcz1nV93Ik2Xf3esGzoXNUPwpKIkKxxgHcBAXvw64FbuGrnJJBaVpWdzz+xWvAi"
    static let grab_nutzlast_datei_benutzer8 = "tbA7jJql81/61EFWXPrNiD7le9hEVs4p3EQiBBOvBNI7MG8ZY8pTMGTgm27lHPimRnzdk6xUDE8xMq8gLqBGc+EoluUkq7SL9X2ps5hKY9d6njyG6k/SfgR4KT6r5y5vW3AWD6ckaYAOEgaYTzm2q1QYZrQt2634HGU="
    static let ordner_kanon = "{\"A\u{308}pfel\":4,\"Zebra\":2,\"a\":6,\"datenklasse\":{\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T1\",\"wert\":\"erbt\",\"zaehler\":1},\"eltern\":{\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T1\",\"wert\":null,\"zaehler\":3},\"id\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"name\":{\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T1\",\"wert\":\"Vertr\u{e4}ge\",\"zaehler\":2},\"nur_geraet\":{\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T1\",\"wert\":false,\"zaehler\":1},\"papierkorb\":{\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T1\",\"wert\":null,\"zaehler\":1},\"v\":1,\"zebra\":1,\"zielordner_ki\":{\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T2\",\"wert\":true,\"zaehler\":1},\"zukunft\":{\"a\":{},\"b\":[]},\"\u{c4}pfel\":3,\"\u{e4}pfel\":5}"
    static let ordner_hash = "91b1bd1479e0ded9ba4134a1363b795ac254fdbb4046c136cfd4dc6c51f38377"
    static let ordner2_kanon = "{\"datenklasse\":{\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T2\",\"wert\":\"internal\",\"zaehler\":5},\"eltern\":{\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T2\",\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TC\",\"zaehler\":7},\"id\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"name\":{\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T2\",\"wert\":\"Rechnungen 2026\",\"zaehler\":6},\"nur_geraet\":{\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T1\",\"wert\":true,\"zaehler\":4},\"papierkorb\":{\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T2\",\"wert\":1730000000000,\"zaehler\":8},\"zielordner_ki\":{\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T1\",\"wert\":false,\"zaehler\":1}}"
    static let ordner2_hash = "edbd2ea01405c275e340d2d60c9892a7e059a9d29aa72a9fc00734ee338c5957"
    static let tiefe_max_ok = "127"
    static let tiefe_max_ok_objekt = "127"
    static let datei_serde_roh = "{\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"ordner\":{\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":4,\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T1\"},\"name\":{\"wert\":\"Mietvertrag Gr\u{f6}\u{df}e.pdf\",\"zaehler\":2,\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T1\"},\"klasse_min\":{\"wert\":\"public\",\"zaehler\":1,\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T1\"},\"nur_geraet_min\":false,\"fassungen\":[{\"fas\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TE\",\"vorgaenger\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\",\"blob\":\"37f648cfa94ffe1c8479b756f2aa94cb9901f907d29e02bce0feafd00e786c49\",\"size\":18446744073709551615,\"original_sha256\":\"37f648cfa94ffe1c8479b756f2aa94cb9901f907d29e02bce0feafd00e786c49\",\"typ\":\"pdf\",\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T2\",\"zeit\":1730000500000,\"texte\":[{\"textrev\":\"b6f283a1250098d3a87e08559f14a12331705765aa91d42a8e16943387ace145\",\"blob\":\"\",\"size\":0,\"extraktor\":\"pdf-extract\",\"extraktor_version\":\"0.12.0+lva1\",\"marken_version\":1,\"stand\":\"fehler\",\"code\":\"extrakt.zeit\",\"versuche\":2,\"versuche_zeit\":1}]},{\"fas\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\",\"vorgaenger\":null,\"blob\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"size\":10,\"original_sha256\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"typ\":\"txt\",\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T1\",\"zeit\":1730000000123,\"texte\":[{\"textrev\":\"7da2e2bed4982376d8b8c7cc36ec8c3a1e514209bcf9fab9e56fc4d12acf4293\",\"blob\":\"fb33111c8c62a25694468653584d83e22de2e0aedf49c0e7d0895b4330478246\",\"size\":373,\"extraktor\":\"lva-text\",\"extraktor_version\":\"1\",\"marken_version\":1,\"stand\":\"ok\"},{\"textrev\":\"6606cb21c0ca82428c3c42f4d29f1b44ee63819ba0534e1cc767a3d48b88860c\",\"blob\":\"d807a7b92c2ad48493bd673c55cef95f4674d253c5084599e4a3d87c0c05cb41\",\"size\":17,\"extraktor\":\"lva-text\",\"extraktor_version\":\"2\",\"marken_version\":1,\"stand\":\"kein_text\"}]}],\"wahlen\":[{\"id\":\"wahl_01HZX4Q8E5K2V7M3N9P6R1S0TG\",\"gewaehlt\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TE\",\"gegen\":[\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\"],\"ersetzt\":[\"wahl_01HZX4Q8E5K2V7M3N9P6R1S0TF\"],\"zaehler\":9007199254740993,\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T2\"},{\"id\":\"wahl_01HZX4Q8E5K2V7M3N9P6R1S0TF\",\"gewaehlt\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\",\"gegen\":[],\"ersetzt\":[],\"zaehler\":5,\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T1\"}],\"entfernt\":[\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TZ\",\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TX\",\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TZ\"],\"blob_refs\":[\"stale\"],\"papierkorb\":{\"wert\":null,\"zaehler\":3,\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T2\"},\"zusatz\":{\"alpha\":[1,2.5,\"x\"],\"u\":\"\u{fc}\",\"zeta\":null}}"
    static let ordner_serde_roh = "{\"id\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"eltern\":{\"wert\":null,\"zaehler\":3,\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T1\"},\"name\":{\"wert\":\"Vertr\u{e4}ge\",\"zaehler\":2,\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T1\"},\"datenklasse\":{\"wert\":\"erbt\",\"zaehler\":1,\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T1\"},\"nur_geraet\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T1\"},\"zielordner_ki\":{\"wert\":true,\"zaehler\":1,\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T2\"},\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_01HZX4Q8E5K2V7M3N9P6R1S0T1\"},\"A\u{308}pfel\":4,\"Zebra\":2,\"a\":6,\"v\":1,\"zebra\":1,\"zukunft\":{\"a\":{},\"b\":[]},\"\u{c4}pfel\":3,\"\u{e4}pfel\":5}"
    static let schema_v2 = "2"
    static let schema_vstr = "1"
    static let schema_vfloat = "1"
    static let schema_vneg = "1"

    static let roh_tabelle: [(name: String, ein: String, aus: String)] = [
        ("zahlen", "{\"b\":[1.5,0.1,1e21,1e-7,0.00001,123456789.125,1e16,12345678901234567890123,-2.5e-10,5e-324,1.7976931348623157e308,100.0,1E2,-0,-0.0,0.5,1e15,123456.789e3,1e-4,9007199254740993,18446744073709551615,18446744073709551616,-9223372036854775808,-9223372036854775809,0,-1,1e0,123456789012345678.0,0.000123,1.5e300,2.5e-5],\"a\":null}", "{\"a\":null,\"b\":[1.5,0.1,1e+21,1e-7,0.00001,123456789.125,1e+16,1.2345678901234568e+22,-2.5e-10,5e-324,1.7976931348623157e+308,100.0,100.0,-0.0,-0.0,0.5,1000000000000000.0,123456789.0,0.0001,9007199254740993,18446744073709551615,1.8446744073709552e+19,-9223372036854775808,-9.223372036854776e+18,0,-1,1.0,1.2345678901234568e+17,0.000123,1.5e+300,0.000025]}"),
        ("escape", "{\"s\":\"a\\u0001b\\u001f\\u007f\\\"\\\\/\\n\\t\\r\\b\\f\\u00e9\\ud83d\\ude00 \\u2028 end\",\"k\\u00e9\":\"v\"}", "{\"k\u{e9}\":\"v\",\"s\":\"a\\u0001b\\u001f\u{7f}\\\"\\\\/\\n\\t\\r\\b\\f\u{e9}\u{1f600} \u{2028} end\"}"),
        ("umlaut", "{\"zebra\":1,\"Zebra\":2,\"\u{c4}pfel\":3,\"A\\u0308pfel\":4,\"\u{e4}pfel\":5,\"a\":6}", "{\"A\u{308}pfel\":4,\"Zebra\":2,\"a\":6,\"zebra\":1,\"\u{c4}pfel\":3,\"\u{e4}pfel\":5}"),
        ("verschachtelt", "[ {\"b\" : {\"d\":null,\"c\":[true, false,null,{\"z\":1,\"y\":[]}]},\u{a}\"a\":{}} , \"x\" ]", "[{\"a\":{},\"b\":{\"c\":[true,false,null,{\"y\":[],\"z\":1}],\"d\":null}},\"x\"]"),
        ("doppelt", "{\"a\":1,\"a\":2,\"b\":{\"c\":1,\"c\":null}}", "{\"a\":2,\"b\":{\"c\":null}}"),
    ]

    static let dec_tabelle: [(name: String, json: String, ok: Bool)] = [
        ("o_basis", "{\"id\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"eltern\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"n\",\"zaehler\":1,\"geraet\":\"g_x\"},\"datenklasse\":{\"wert\":\"erbt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"nur_geraet\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"zielordner_ki\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", true),
        ("o_eltern_wert_fehlt", "{\"id\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"eltern\":{\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"n\",\"zaehler\":1,\"geraet\":\"g_x\"},\"datenklasse\":{\"wert\":\"erbt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"nur_geraet\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"zielordner_ki\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", true),
        ("o_eltern_wert_string", "{\"id\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"eltern\":{\"wert\":\"ord_x\",\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"n\",\"zaehler\":1,\"geraet\":\"g_x\"},\"datenklasse\":{\"wert\":\"erbt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"nur_geraet\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"zielordner_ki\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", true),
        ("o_name_wert_fehlt", "{\"id\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"eltern\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"zaehler\":1,\"geraet\":\"g_x\"},\"datenklasse\":{\"wert\":\"erbt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"nur_geraet\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"zielordner_ki\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", false),
        ("o_name_wert_null", "{\"id\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"eltern\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"},\"datenklasse\":{\"wert\":\"erbt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"nur_geraet\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"zielordner_ki\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", false),
        ("o_nur_geraet_string", "{\"id\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"eltern\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"n\",\"zaehler\":1,\"geraet\":\"g_x\"},\"datenklasse\":{\"wert\":\"erbt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"nur_geraet\":{\"wert\":\"false\",\"zaehler\":1,\"geraet\":\"g_x\"},\"zielordner_ki\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", false),
        ("o_papierkorb_float", "{\"id\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"eltern\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"n\",\"zaehler\":1,\"geraet\":\"g_x\"},\"datenklasse\":{\"wert\":\"erbt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"nur_geraet\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"zielordner_ki\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"papierkorb\":{\"wert\":1.5,\"zaehler\":1,\"geraet\":\"g_x\"}}", false),
        ("o_papierkorb_negativ", "{\"id\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"eltern\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"n\",\"zaehler\":1,\"geraet\":\"g_x\"},\"datenklasse\":{\"wert\":\"erbt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"nur_geraet\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"zielordner_ki\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"papierkorb\":{\"wert\":-5,\"zaehler\":1,\"geraet\":\"g_x\"}}", true),
        ("o_papierkorb_zu_gross", "{\"id\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"eltern\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"n\",\"zaehler\":1,\"geraet\":\"g_x\"},\"datenklasse\":{\"wert\":\"erbt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"nur_geraet\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"zielordner_ki\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"papierkorb\":{\"wert\":9223372036854775808,\"zaehler\":1,\"geraet\":\"g_x\"}}", false),
        ("o_papierkorb_max", "{\"id\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"eltern\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"n\",\"zaehler\":1,\"geraet\":\"g_x\"},\"datenklasse\":{\"wert\":\"erbt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"nur_geraet\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"zielordner_ki\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"papierkorb\":{\"wert\":9223372036854775807,\"zaehler\":1,\"geraet\":\"g_x\"}}", true),
        ("o_zaehler_float", "{\"id\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"eltern\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"n\",\"zaehler\":1.0,\"geraet\":\"g_x\"},\"datenklasse\":{\"wert\":\"erbt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"nur_geraet\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"zielordner_ki\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", false),
        ("o_zaehler_negativ", "{\"id\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"eltern\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"n\",\"zaehler\":-1,\"geraet\":\"g_x\"},\"datenklasse\":{\"wert\":\"erbt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"nur_geraet\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"zielordner_ki\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", false),
        ("o_zaehler_max", "{\"id\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"eltern\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"n\",\"zaehler\":18446744073709551615,\"geraet\":\"g_x\"},\"datenklasse\":{\"wert\":\"erbt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"nur_geraet\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"zielordner_ki\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", true),
        ("o_zaehler_zu_gross", "{\"id\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"eltern\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"n\",\"zaehler\":18446744073709551616,\"geraet\":\"g_x\"},\"datenklasse\":{\"wert\":\"erbt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"nur_geraet\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"zielordner_ki\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", false),
        ("o_register_zusatzfeld", "{\"id\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"eltern\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"n\",\"zaehler\":1,\"neu\":7,\"geraet\":\"g_x\"},\"datenklasse\":{\"wert\":\"erbt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"nur_geraet\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"zielordner_ki\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", true),
        ("o_ohne_id", "{\"eltern\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"n\",\"zaehler\":1,\"geraet\":\"g_x\"},\"datenklasse\":{\"wert\":\"erbt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"nur_geraet\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"zielordner_ki\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", false),
        ("o_id_zahl", "{\"id\":5,\"eltern\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"n\",\"zaehler\":1,\"geraet\":\"g_x\"},\"datenklasse\":{\"wert\":\"erbt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"nur_geraet\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"zielordner_ki\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", false),
        ("o_extra_v1", "{\"id\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"eltern\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"n\",\"zaehler\":1,\"geraet\":\"g_x\"},\"datenklasse\":{\"wert\":\"erbt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"nur_geraet\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"zielordner_ki\":{\"wert\":false,\"zaehler\":1,\"geraet\":\"g_x\"},\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"},\"v\":1,\"x\":null}", true),
        ("o_kein_objekt", "[1,2]", false),
        ("d_minimal", "{\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"ordner\":{\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"a.txt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"klasse_min\":{\"wert\":\"public\",\"zaehler\":1,\"geraet\":\"g_x\"},\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", true),
        ("d_nur_geraet_null", "{\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"ordner\":{\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"a.txt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"klasse_min\":{\"wert\":\"public\",\"zaehler\":1,\"geraet\":\"g_x\"},\"nur_geraet_min\":null,\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", false),
        ("d_nur_geraet_true", "{\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"ordner\":{\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"a.txt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"klasse_min\":{\"wert\":\"public\",\"zaehler\":1,\"geraet\":\"g_x\"},\"nur_geraet_min\":true,\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", true),
        ("d_fassungen_null", "{\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"ordner\":{\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"a.txt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"klasse_min\":{\"wert\":\"public\",\"zaehler\":1,\"geraet\":\"g_x\"},\"fassungen\":null,\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", false),
        ("d_fassung_ok", "{\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"ordner\":{\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"a.txt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"klasse_min\":{\"wert\":\"public\",\"zaehler\":1,\"geraet\":\"g_x\"},\"fassungen\":[{\"fas\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\",\"vorgaenger\":null,\"blob\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"size\":10,\"original_sha256\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"typ\":\"txt\",\"geraet\":\"g_x\",\"zeit\":1730000000123}],\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", true),
        ("d_fassung_vorgaenger_fehlt", "{\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"ordner\":{\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"a.txt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"klasse_min\":{\"wert\":\"public\",\"zaehler\":1,\"geraet\":\"g_x\"},\"fassungen\":[{\"fas\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\",\"blob\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"size\":10,\"original_sha256\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"typ\":\"txt\",\"geraet\":\"g_x\",\"zeit\":1}],\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", true),
        ("d_fassung_zeit_float", "{\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"ordner\":{\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"a.txt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"klasse_min\":{\"wert\":\"public\",\"zaehler\":1,\"geraet\":\"g_x\"},\"fassungen\":[{\"fas\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\",\"blob\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"size\":10,\"original_sha256\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"typ\":\"txt\",\"geraet\":\"g_x\",\"zeit\":1.5}],\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", false),
        ("d_fassung_zeit_negativ", "{\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"ordner\":{\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"a.txt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"klasse_min\":{\"wert\":\"public\",\"zaehler\":1,\"geraet\":\"g_x\"},\"fassungen\":[{\"fas\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\",\"blob\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"size\":10,\"original_sha256\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"typ\":\"txt\",\"geraet\":\"g_x\",\"zeit\":-1}],\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", true),
        ("d_fassung_size_negativ", "{\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"ordner\":{\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"a.txt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"klasse_min\":{\"wert\":\"public\",\"zaehler\":1,\"geraet\":\"g_x\"},\"fassungen\":[{\"fas\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\",\"blob\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"size\":-1,\"original_sha256\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"typ\":\"txt\",\"geraet\":\"g_x\",\"zeit\":1}],\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", false),
        ("d_fassung_size_max", "{\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"ordner\":{\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"a.txt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"klasse_min\":{\"wert\":\"public\",\"zaehler\":1,\"geraet\":\"g_x\"},\"fassungen\":[{\"fas\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\",\"blob\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"size\":18446744073709551615,\"original_sha256\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"typ\":\"txt\",\"geraet\":\"g_x\",\"zeit\":1}],\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", true),
        ("d_fassung_texte_null", "{\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"ordner\":{\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"a.txt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"klasse_min\":{\"wert\":\"public\",\"zaehler\":1,\"geraet\":\"g_x\"},\"fassungen\":[{\"fas\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\",\"blob\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"size\":1,\"original_sha256\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"typ\":\"txt\",\"geraet\":\"g_x\",\"zeit\":1,\"texte\":null}],\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", false),
        ("d_text_ok", "{\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"ordner\":{\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"a.txt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"klasse_min\":{\"wert\":\"public\",\"zaehler\":1,\"geraet\":\"g_x\"},\"fassungen\":[{\"fas\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\",\"blob\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"size\":1,\"original_sha256\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"typ\":\"txt\",\"geraet\":\"g_x\",\"zeit\":1,\"texte\":[{\"textrev\":\"7da2e2bed4982376d8b8c7cc36ec8c3a1e514209bcf9fab9e56fc4d12acf4293\",\"blob\":\"fb33111c8c62a25694468653584d83e22de2e0aedf49c0e7d0895b4330478246\",\"size\":5,\"extraktor\":\"lva-text\",\"extraktor_version\":\"1\",\"marken_version\":1,\"stand\":\"ok\"}]}],\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", true),
        ("d_text_versuche_null", "{\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"ordner\":{\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"a.txt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"klasse_min\":{\"wert\":\"public\",\"zaehler\":1,\"geraet\":\"g_x\"},\"fassungen\":[{\"fas\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\",\"blob\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"size\":1,\"original_sha256\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"typ\":\"txt\",\"geraet\":\"g_x\",\"zeit\":1,\"texte\":[{\"textrev\":\"7da2e2bed4982376d8b8c7cc36ec8c3a1e514209bcf9fab9e56fc4d12acf4293\",\"blob\":\"fb33111c8c62a25694468653584d83e22de2e0aedf49c0e7d0895b4330478246\",\"size\":5,\"extraktor\":\"lva-text\",\"extraktor_version\":\"1\",\"marken_version\":1,\"stand\":\"ok\",\"versuche\":null}]}],\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", false),
        ("d_text_code_null", "{\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"ordner\":{\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"a.txt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"klasse_min\":{\"wert\":\"public\",\"zaehler\":1,\"geraet\":\"g_x\"},\"fassungen\":[{\"fas\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\",\"blob\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"size\":1,\"original_sha256\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"typ\":\"txt\",\"geraet\":\"g_x\",\"zeit\":1,\"texte\":[{\"textrev\":\"7da2e2bed4982376d8b8c7cc36ec8c3a1e514209bcf9fab9e56fc4d12acf4293\",\"blob\":\"fb33111c8c62a25694468653584d83e22de2e0aedf49c0e7d0895b4330478246\",\"size\":5,\"extraktor\":\"lva-text\",\"extraktor_version\":\"1\",\"marken_version\":1,\"stand\":\"ok\",\"code\":null}]}],\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", true),
        ("d_text_code_zahl", "{\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"ordner\":{\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"a.txt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"klasse_min\":{\"wert\":\"public\",\"zaehler\":1,\"geraet\":\"g_x\"},\"fassungen\":[{\"fas\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\",\"blob\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"size\":1,\"original_sha256\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"typ\":\"txt\",\"geraet\":\"g_x\",\"zeit\":1,\"texte\":[{\"textrev\":\"7da2e2bed4982376d8b8c7cc36ec8c3a1e514209bcf9fab9e56fc4d12acf4293\",\"blob\":\"fb33111c8c62a25694468653584d83e22de2e0aedf49c0e7d0895b4330478246\",\"size\":5,\"extraktor\":\"lva-text\",\"extraktor_version\":\"1\",\"marken_version\":1,\"stand\":\"ok\",\"code\":5}]}],\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", false),
        ("d_text_marken_zu_gross", "{\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"ordner\":{\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"a.txt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"klasse_min\":{\"wert\":\"public\",\"zaehler\":1,\"geraet\":\"g_x\"},\"fassungen\":[{\"fas\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\",\"blob\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"size\":1,\"original_sha256\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"typ\":\"txt\",\"geraet\":\"g_x\",\"zeit\":1,\"texte\":[{\"textrev\":\"7da2e2bed4982376d8b8c7cc36ec8c3a1e514209bcf9fab9e56fc4d12acf4293\",\"blob\":\"fb33111c8c62a25694468653584d83e22de2e0aedf49c0e7d0895b4330478246\",\"size\":5,\"extraktor\":\"lva-text\",\"extraktor_version\":\"1\",\"marken_version\":4294967296,\"stand\":\"ok\"}]}],\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", false),
        ("d_text_versuche_max", "{\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"ordner\":{\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"a.txt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"klasse_min\":{\"wert\":\"public\",\"zaehler\":1,\"geraet\":\"g_x\"},\"fassungen\":[{\"fas\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\",\"blob\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"size\":1,\"original_sha256\":\"1fbbb75cd3895be571826b41fee492a3cd94509465c7bb9606285675037f24cb\",\"typ\":\"txt\",\"geraet\":\"g_x\",\"zeit\":1,\"texte\":[{\"textrev\":\"7da2e2bed4982376d8b8c7cc36ec8c3a1e514209bcf9fab9e56fc4d12acf4293\",\"blob\":\"fb33111c8c62a25694468653584d83e22de2e0aedf49c0e7d0895b4330478246\",\"size\":5,\"extraktor\":\"lva-text\",\"extraktor_version\":\"1\",\"marken_version\":4294967295,\"stand\":\"ok\",\"versuche\":4294967295}]}],\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", true),
        ("d_wahl_gegen_fehlt", "{\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"ordner\":{\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"a.txt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"klasse_min\":{\"wert\":\"public\",\"zaehler\":1,\"geraet\":\"g_x\"},\"wahlen\":[{\"id\":\"wahl_01HZX4Q8E5K2V7M3N9P6R1S0TF\",\"gewaehlt\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\",\"zaehler\":3,\"geraet\":\"g_x\"}],\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", true),
        ("d_wahl_gegen_null", "{\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"ordner\":{\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"a.txt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"klasse_min\":{\"wert\":\"public\",\"zaehler\":1,\"geraet\":\"g_x\"},\"wahlen\":[{\"id\":\"wahl_01HZX4Q8E5K2V7M3N9P6R1S0TF\",\"gewaehlt\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\",\"gegen\":null,\"zaehler\":3,\"geraet\":\"g_x\"}],\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", false),
        ("d_wahl_ohne_zaehler", "{\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"ordner\":{\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"a.txt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"klasse_min\":{\"wert\":\"public\",\"zaehler\":1,\"geraet\":\"g_x\"},\"wahlen\":[{\"id\":\"wahl_01HZX4Q8E5K2V7M3N9P6R1S0TF\",\"gewaehlt\":\"fas_01HZX4Q8E5K2V7M3N9P6R1S0TD\",\"geraet\":\"g_x\"}],\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", false),
        ("d_blob_refs_string", "{\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"ordner\":{\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"a.txt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"klasse_min\":{\"wert\":\"public\",\"zaehler\":1,\"geraet\":\"g_x\"},\"blob_refs\":\"x\",\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", false),
        ("d_blob_refs_liste", "{\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"ordner\":{\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"a.txt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"klasse_min\":{\"wert\":\"public\",\"zaehler\":1,\"geraet\":\"g_x\"},\"blob_refs\":[\"a\",\"b\"],\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", true),
        ("d_entfernt_zahl", "{\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"ordner\":{\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"a.txt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"klasse_min\":{\"wert\":\"public\",\"zaehler\":1,\"geraet\":\"g_x\"},\"entfernt\":[1],\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", false),
        ("d_ohne_klasse_min", "{\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"ordner\":{\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"a.txt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"}}", false),
        ("d_ohne_papierkorb", "{\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"ordner\":{\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"a.txt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"klasse_min\":{\"wert\":\"public\",\"zaehler\":1,\"geraet\":\"g_x\"}}", false),
        ("d_extra_zahlen", "{\"id\":\"dat_01HZX4Q8E5K2V7M3N9P6R1S0TA\",\"ordner\":{\"wert\":\"ord_01HZX4Q8E5K2V7M3N9P6R1S0TB\",\"zaehler\":1,\"geraet\":\"g_x\"},\"name\":{\"wert\":\"a.txt\",\"zaehler\":1,\"geraet\":\"g_x\"},\"klasse_min\":{\"wert\":\"public\",\"zaehler\":1,\"geraet\":\"g_x\"},\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g_x\"},\"x\":1.0,\"y\":18446744073709551615,\"z\":[null,{\"q\":-0}]}", true),
    ]

    static let id_tabelle: [(prefix: String, id: String, gueltig: Bool)] = [
        ("dat_", "dat_01HZX4Q8E5K2V7M3N9P6R1S0TA", true),
        ("dat_", "dat_01hzx4q8e5k2v7m3n9p6r1s0ta", true),
        ("dat_", "dat_8ZZZZZZZZZZZZZZZZZZZZZZZZZ", true),
        ("dat_", "dat_ZZZZZZZZZZZZZZZZZZZZZZZZZZ", true),
        ("dat_", "dat_00000000000000000000000000", true),
        ("dat_", "dat_01HZX4Q8E5K2V7M3N9P6R1S0T", false),
        ("dat_", "dat_01HZX4Q8E5K2V7M3N9P6R1S0TAA", false),
        ("dat_", "dat_01HZX4Q8E5K2V7M3N9P6R1S0TI", false),
        ("dat_", "dat_01HZX4Q8E5K2V7M3N9P6R1S0TL", false),
        ("dat_", "dat_01HZX4Q8E5K2V7M3N9P6R1S0TO", false),
        ("dat_", "dat_01HZX4Q8E5K2V7M3N9P6R1S0TU", false),
        ("dat_", "dat_01hzx4q8e5k2v7m3n9p6r1s0ti", false),
        ("dat_", "dat_01HZX4Q8E5K2V7M3N9P6R1S0T/", false),
        ("dat_", "dat_01HZX4Q8E5K2V7M3N9P6R1S0T.", false),
        ("dat_", "dat_../../../../etc/passwd_______", false),
        ("dat_", "dat_01HZX4Q8E5K2V7M3N9P6R1S0T\u{e4}", false),
        ("dat_", "ord_01HZX4Q8E5K2V7M3N9P6R1S0TA", false),
        ("dat_", "DAT_01HZX4Q8E5K2V7M3N9P6R1S0TA", false),
        ("dat_", "01HZX4Q8E5K2V7M3N9P6R1S0TA", false),
        ("dat_", "", false),
        ("ord_", "ord_01HZX4Q8E5K2V7M3N9P6R1S0TB", true),
        ("g_", "g_01HZX4Q8E5K2V7M3N9P6R1S0T1", true),
        ("fas_", "fas_01HZX4Q8E5K2V7M3N9P6R1S0TD", true),
        ("wahl_", "wahl_01HZX4Q8E5K2V7M3N9P6R1S0TF", true),
    ]
}
