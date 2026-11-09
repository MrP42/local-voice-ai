import XCTest
@testable import VoiceCore
#if canImport(CryptoKit)
import CryptoKit

// Gegenprobe der Ablage-Krypto und -Kanonik gegen den Desktop (D5a-1, lva#441, Epic #395).
//
// Die Werte kommen aus der gemeinsamen Fixture `docs/superpowers/plans/ablage-testvektoren.json`, die der Desktop-
// Test `sync::ablage::vektoren` aus den echten Funktionen erzeugt und auf Übereinstimmung prüft (lva#442). Dieser
// Test liest dieselbe Datei und rechnet jeden Vektor nach: AAD, versiegelter Grabstein samt Nonce, Hub-Kennungen,
// `textrev`, Textfassung (Bytes, SHA-256, Kennung), kanonische Nutzlast und Hash von Datei und Ordner,
// `standard_id`. Datei und Ordner werden zweifach geprüft: aus der kanonischen Rust-Nutzlast gelesen und
// unabhängig in Swift aufgebaut. Nur synthetische Daten.
//
// Weitere Fälle (unsortierte Listen, Zahlenformate, Lesen wie serde, ...) stehen in AblageRustZusatzTests.

private func hexBytes(_ s: String) -> [UInt8] {
    let u = Array(s.utf8)
    return stride(from: 0, to: u.count - 1, by: 2).map { UInt8(String(decoding: u[$0..<$0 + 2], as: UTF8.self), radix: 16) ?? 0 }
}
private func hexString(_ b: [UInt8]) -> String { b.map { String(format: "%02x", $0) }.joined() }
private func bytes(_ s: String) -> [UInt8] { Array(s.utf8) }
private func text(_ b: [UInt8]) -> String { String(decoding: b, as: UTF8.self) }

final class AblageVectorTests: XCTestCase {
    /// Fixture im Repo; `ABLAGE_TESTVEKTOREN` überschreibt den Pfad (wie `BOOKTALK_RELAY_GOLDEN`).
    static func fixtureURL() -> URL {
        if let o = ProcessInfo.processInfo.environment["ABLAGE_TESTVEKTOREN"] { return URL(fileURLWithPath: o) }
        var url = URL(fileURLWithPath: #filePath).resolvingSymlinksInPath()
        for _ in 0..<6 { url.deleteLastPathComponent() }   // Datei, VoiceCoreTests, Tests, VoiceCore, apple, apps
        return url.appendingPathComponent("docs/superpowers/plans/ablage-testvektoren.json")
    }

    private var root: [String: Any] = [:]
    private var key: [UInt8] = []
    private var userId: Int64 = 0

    override func setUpWithError() throws {
        let data = try Data(contentsOf: Self.fixtureURL())
        root = try XCTUnwrap(try JSONSerialization.jsonObject(with: data) as? [String: Any], "Fixture ist kein Objekt")
        XCTAssertEqual(root["version"] as? Int, 1)
        key = hexBytes(try XCTUnwrap(root["schluessel_hex"] as? String))
        userId = Int64(try XCTUnwrap(root["user_id"] as? Int))
        XCTAssertEqual(key, (0..<32).map { UInt8($0) }, "Schlüssel = Bytes 00..1f")
        XCTAssertEqual(userId, 7)
    }

    private func liste(_ name: String) throws -> [[String: Any]] {
        let l = try XCTUnwrap(root[name] as? [[String: Any]], name)
        XCTAssertFalse(l.isEmpty, name)
        return l
    }

    private func str(_ d: [String: Any], _ k: String) throws -> String { try XCTUnwrap(d[k] as? String, k) }

    // MARK: Schlüssel, Kennungen

    func testSchluesselkennungUndStandardOrdner() throws {
        XCTAssertEqual(SyncCrypto.keyId(key), root["key_id"] as? String)
        let std = try XCTUnwrap(root["standard_id"] as? String)
        XCTAssertEqual(AblageKrypto.standardOrdnerId(key: key), std)
        XCTAssertTrue(AblageId.idGueltig(std, prefix: AblageId.praefixOrdner))
        XCTAssertNotEqual(AblageKrypto.standardOrdnerId(key: Array(key.reversed())), std)
    }

    func testHubKennungenAusHexHash() throws {
        for z in try liste("hub_kennungen") {
            let sha = try str(z, "sha256"), erwartet = try str(z, "hub_kennung")
            XCTAssertEqual(AblageKrypto.hubBlobId(key: key, sha256Hex: sha), erwartet)
            // Gleicher Weg wie die vorhandene Bibliotheks-Kennung aus Rohbytes.
            XCTAssertEqual(LibrarySyncFormat.blobId(key: key, plaintextSha256: Data(hexBytes(sha))), erwartet)
            XCTAssertNil(AblageKrypto.hubBlobId(key: key, sha256Hex: String(sha.dropLast())))
        }
    }

    // MARK: AAD

    func testAADWieDesktop() throws {
        let faelle = try liste("aad")
        XCTAssertGreaterThanOrEqual(faelle.count, 6)
        var gesehen = Set<String>()
        for z in faelle {
            let name = try str(z, "name")
            let refs = try XCTUnwrap(z["blob_refs"] as? [String])
            XCTAssertEqual(refs, AblageBytes.sortedUnique(refs), "\(name): Hub-Kennungen sortiert und eindeutig")
            let a = AblageKrypto.aadAblage(userId: Int64(try XCTUnwrap(z["user_id"] as? Int)), collection: try str(z, "collection"),
                                           objectId: try str(z, "object_id"), hubBlobRefs: refs, keyId: try str(z, "key_id"),
                                           deleted: try XCTUnwrap(z["deleted"] as? Bool))
            XCTAssertEqual(hexString(a), try str(z, "aad_hex"), name)
            XCTAssertEqual(text(a), try str(z, "aad_utf8"), name)
            gesehen.insert(name)
        }
        XCTAssertTrue(gesehen.isSuperset(of: ["ordner", "ordner_grabstein", "datei_ohne_refs", "datei_mit_refs", "datei_ein_ref", "datei_grabstein"]))
    }

    // MARK: Grabsteine

    func testVersiegelteGrabsteineWieDesktop() throws {
        for z in try liste("grabstein") {
            let coll = try str(z, "collection"), id = try str(z, "object_id"), kid = SyncCrypto.keyId(key)
            XCTAssertEqual(text(AblageKrypto.grabKlartext(collection: coll, id: id)), try str(z, "klartext"))
            let aad = AblageKrypto.aadAblage(userId: userId, collection: coll, objectId: id, hubBlobRefs: [], keyId: kid, deleted: true)
            XCTAssertEqual(hexString(aad), try str(z, "aad_hex"))
            // Deterministisches Siegel: dieselbe Nutzlast Zeichen für Zeichen (Base64 STANDARD mit Auffüllung).
            let nutzlast = try AblageKrypto.grabNutzlast(key: key, userId: userId, collection: coll, id: id)
            XCTAssertEqual(nutzlast, try str(z, "payload_base64"))
            XCTAssertTrue(nutzlast.hasSuffix("=") || nutzlast.count % 4 == 0)
            let roh = [UInt8](try XCTUnwrap(Data(base64Encoded: nutzlast)))
            XCTAssertEqual(hexString(roh), try str(z, "payload_hex"))
            // Nonce = erste 24 Bytes von SHA-256("lv-ablage-siegel-nonce" ‖ 0x1F ‖ key ‖ u64be(aad.len) ‖ aad ‖ klartext).
            XCTAssertEqual(hexString(Array(roh.prefix(24))), try str(z, "nonce_hex"))
            XCTAssertEqual(try AblageKrypto.grabNutzlast(key: key, userId: userId, collection: coll, id: id), nutzlast, "Wiederholung")
            XCTAssertEqual(text(try SyncCrypto.open(key: key, aad: aad, sealedBase64: nutzlast)), try str(z, "klartext"))
            // Echt – und nur am eigenen Ort.
            func echt(_ p: String = nutzlast, coll c: String = coll, id i: String = id, user: Int64 = userId, keyId: String = kid) -> Bool {
                AblageKrypto.grabEcht(key: key, userId: user, collection: c, id: i, keyId: keyId, payload: p)
            }
            XCTAssertTrue(echt())
            XCTAssertFalse(echt(id: id + "X"))
            XCTAssertFalse(echt(coll: coll == "ablage_datei" ? "ablage_ordner" : "ablage_datei"))
            XCTAssertFalse(echt(user: userId + 1))
            XCTAssertFalse(echt(keyId: "deadbeef"))
            XCTAssertFalse(echt(""), "ohne Siegel")
            var kaputt = roh; kaputt[30] ^= 1
            XCTAssertFalse(echt(Data(kaputt).base64EncodedString()), "verändertes Chiffrat")
            var nonce = roh; nonce[0] ^= 1
            XCTAssertFalse(echt(Data(nonce).base64EncodedString()), "veränderte Nonce")
        }
    }

    // MARK: textrev und Textfassung

    func testTextrevWieDesktop() throws {
        let faelle = try liste("textrev")
        for z in faelle {
            XCTAssertEqual(AblageTextfassung.textrev(originalSha256Hex: try str(z, "original_sha256"), extraktor: try str(z, "extraktor"),
                                                     extraktorVersion: try str(z, "extraktor_version"),
                                                     markenVersion: UInt32(try XCTUnwrap(z["marken_version"] as? Int))),
                           try str(z, "textrev"))
        }
        XCTAssertEqual(Set(try faelle.map { try str($0, "textrev") }).count, faelle.count)
    }

    func testTextfassungWieDesktop() throws {
        let z = try XCTUnwrap(root["textfassung"] as? [String: Any])
        let b = bytes(try str(z, "json_utf8"))
        XCTAssertEqual(AblageKrypto.sha256Hex(b), try str(z, "sha256"))
        XCTAssertEqual(AblageKrypto.hubBlobId(key: key, sha256Hex: try str(z, "sha256")), try str(z, "hub_kennung"))
        let tf = try AblageTextfassung(bytes: b)
        XCTAssertEqual(tf.bytes, b, "Bytes unverändert")
        XCTAssertEqual(tf.sha256Hex, try str(z, "sha256"))
        XCTAssertEqual(tf.hubBlobId(key: key), try str(z, "hub_kennung"))
        XCTAssertEqual(tf.markenVersion, 1)
        XCTAssertEqual(tf.textrev, try str(try XCTUnwrap(try liste("textrev").first), "textrev"))
        XCTAssertEqual(tf.abschnitte, [AblageAbschnitt(anker: "zeile=1", pfad: [], text: "Die Miete beträgt 850 Euro."),
                                       AblageAbschnitt(anker: "absatz=2", pfad: ["§ 4 Laufzeit"], text: "Kündigung \"zum Quartal\" – ohne Frist.")])
        // Prüfung wie `text_einlegen` gegen Eintrag und Fassung, die sich daraus ergeben.
        let fassung = AblageFassung(fas: "fas_x", vorgaenger: nil, blob: tf.originalSha256, size: 1, originalSha256: tf.originalSha256, typ: "txt", geraet: "g_x", zeit: 1)
        let eintrag = AblageText(textrev: tf.textrev, blob: tf.sha256Hex, size: UInt64(b.count), extraktor: tf.extraktor,
                                 extraktorVersion: tf.extraktorVersion, markenVersion: tf.markenVersion, stand: "ok")
        XCTAssertTrue(tf.passt(zu: eintrag, fassung: fassung))
        XCTAssertTrue(AblageTextfassung.textrevStimmt(text: eintrag, fassung: fassung))
        var fremd = eintrag; fremd.textrev = String(repeating: "0", count: 64)
        XCTAssertFalse(tf.passt(zu: fremd, fassung: fassung))
        var andere = fassung; andere.originalSha256 = String(repeating: "2", count: 64)
        XCTAssertFalse(tf.passt(zu: eintrag, fassung: andere))
        var verschoben = b; verschoben[verschoben.count - 5] ^= 1
        if let t2 = try? AblageTextfassung(bytes: verschoben) { XCTAssertFalse(t2.passt(zu: eintrag, fassung: fassung)) }
        // Die Datei steht in Strukturreihenfolge, nicht nach Schlüsseln sortiert: kanonisch neu geschrieben wären es
        // andere Bytes (anderer Hash). Textfassungen werden deshalb nie neu serialisiert.
        XCTAssertNotEqual(try AblageJSON.parse(b).kanonischerText, text(b))
    }

    // MARK: Kanonische Nutzlast und Hash

    private let g = "g_01HZX3K9M2N4P5Q6R7S8T9V0W1"
    private let ORD = "ord_01HZX3K9M2N4P5Q6R7S8T9V0WX"
    private let DAT = "dat_01HZX3K9M2N4P5Q6R7S8T9V0WY"
    private let FAS2 = "fas_01HZX3K9M2N4P5Q6R7S8T9V0W2"
    private let FAS3 = "fas_01HZX3K9M2N4P5Q6R7S8T9V0W3"
    private let SHA1 = String(repeating: "1", count: 64)
    private let SHA2 = String(repeating: "2", count: 64)
    private let SHA3 = String(repeating: "3", count: 64)

    private func reg<T: AblageWert>(_ w: T, _ z: UInt64) -> AblageRegister<T> { AblageRegister(wert: w, zaehler: z, geraet: g) }

    /// Wie `datei()` im Rust-Test, unabhängig in Swift aufgebaut.
    private func dateiAusSwift() -> AblageDatei {
        func text(_ rev: String, _ blob: String, _ stand: String) -> AblageText {
            AblageText(textrev: rev, blob: blob, size: 420, extraktor: "lva-text", extraktorVersion: "1", markenVersion: 1, stand: stand)
        }
        var fehler = text("ff", "", "fehler")
        fehler.code = "extrakt.zeit"; fehler.versuche = 2; fehler.versucheZeit = 1
        return AblageDatei(
            id: DAT, ordner: reg(ORD, 3), name: reg("Mietvertrag Köln.txt", 2), klasseMin: reg("public", 1), nurGeraetMin: false,
            fassungen: [
                AblageFassung(fas: FAS2, vorgaenger: nil, blob: SHA1, size: 1234, originalSha256: SHA1, typ: "txt", geraet: g, zeit: 1_762_700_000_000,
                              texte: [text("aa", SHA3, "ok"), fehler]),
                AblageFassung(fas: FAS3, vorgaenger: FAS2, blob: SHA2, size: 99, originalSha256: SHA2, typ: "txt", geraet: g, zeit: 1_762_700_100_000),
            ],
            wahlen: [AblageWahl(id: "wahl_01HZX3K9M2N4P5Q6R7S8T9V0W4", gewaehlt: FAS3, gegen: [FAS2], ersetzt: [], zaehler: 4, geraet: g)],
            papierkorb: reg(nil as Int64?, 1),
            extra: [AblageJSON.Member(key: "zukunft", value: .objekt([("b", .null), ("a", .uint(1))]))])
    }

    private func ordnerAusSwift() -> AblageOrdner {
        AblageOrdner(id: ORD, eltern: reg(nil as String?, 1), name: reg("Verträge – Übersicht", 2), datenklasse: reg("erbt", 1),
                     nurGeraet: reg(false, 1), zielordnerKi: reg(false, 1), papierkorb: reg(nil as Int64?, 1),
                     extra: [AblageJSON.Member(key: "zukunft", value: .objekt([("z", .null), ("a", .array([.uint(3), .string("ü"), .null])), ("m", .bool(true))]))])
    }

    func testNutzlastDateiWieDesktop() throws {
        let z = try XCTUnwrap((try XCTUnwrap(root["nutzlast"] as? [String: Any]))["datei"] as? [String: Any])
        let kanon = try str(z, "kanonisch"), hash = try str(z, "hash")
        let inhalt = try XCTUnwrap(z["blob_refs_inhalt"] as? [String]), hub = try XCTUnwrap(z["blob_refs_hub"] as? [String])
        XCTAssertEqual(inhalt, AblageBytes.sortedUnique(inhalt), "blob_refs sortiert und eindeutig")
        XCTAssertEqual(hub, AblageBytes.sortedUnique(hub))

        // (1) Aus Swift aufgebaut: Bytes und Hash wie Rust, in beiden Ordnungen (das Beispiel ist schon geordnet).
        let d = dateiAusSwift()
        XCTAssertEqual(text(d.kanonischeNutzlast(ordnen: false)), kanon)
        XCTAssertEqual(text(d.kanonischeNutzlast(ordnen: true)), kanon)
        XCTAssertEqual(d.hash(ordnen: false), hash)
        XCTAssertEqual(d.hash(), hash)
        XCTAssertEqual(AblageDatei.inhaltsRefs(d.fassungen), inhalt)
        XCTAssertEqual(AblageKrypto.hubRefs(datei: d, key: key), hub)

        // (2) Aus der kanonischen Rust-Nutzlast gelesen: Fixpunkt; `extra`, null, Listen und Zahlen bleiben.
        let gelesen = try AblageDatei(nutzlast: bytes(kanon))
        XCTAssertEqual(text(gelesen.kanonischeNutzlast(ordnen: false)), kanon)
        XCTAssertEqual(gelesen.hash(ordnen: false), hash)
        XCTAssertEqual(gelesen.blobRefs, inhalt)
        XCTAssertEqual(gelesen.extra.map(\.key), ["zukunft"])
        XCTAssertEqual(AblageKrypto.hubRefs(datei: gelesen, key: key), hub)
        XCTAssertEqual(gelesen.fassungen.count, 2)
        XCTAssertNil(gelesen.fassungen[0].vorgaenger)
        XCTAssertEqual(gelesen.fassungen[0].texte[1].code, "extrakt.zeit")
        XCTAssertTrue(kanon.contains("\"vorgaenger\":null") && kanon.contains("\"wert\":null") && kanon.contains("\"b\":null"))
        XCTAssertFalse(kanon.contains("\"code\":null"), "Optionales Feld fehlt, statt null zu sein")
    }

    func testNutzlastOrdnerWieDesktop() throws {
        let z = try XCTUnwrap((try XCTUnwrap(root["nutzlast"] as? [String: Any]))["ordner"] as? [String: Any])
        let kanon = try str(z, "kanonisch"), hash = try str(z, "hash")
        let o = ordnerAusSwift()
        XCTAssertEqual(text(o.kanonischeNutzlast()), kanon)
        XCTAssertEqual(o.hash(), hash)
        let gelesen = try AblageOrdner(nutzlast: bytes(kanon))
        XCTAssertEqual(text(gelesen.kanonischeNutzlast()), kanon)
        XCTAssertEqual(gelesen.hash(), hash)
        XCTAssertEqual(gelesen.extra.map(\.key), ["zukunft"])
        XCTAssertNil(gelesen.eltern.wert)
        XCTAssertNil(gelesen.papierkorb.wert)
        XCTAssertTrue(kanon.contains("\"wert\":null"))
        XCTAssertTrue(kanon.contains("Verträge – Übersicht"), "Nicht-ASCII bleibt roh")
        // Grabstein-AAD und Sammlung: Ordner tragen nie Hub-Kennungen.
        XCTAssertEqual(AblageKrypto.hubRefs(datei: dateiAusSwift(), key: key).count, 3)
    }
}
#endif
