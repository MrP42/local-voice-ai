import XCTest
@testable import VoiceCore
#if canImport(CryptoKit)
import CryptoKit

// Eigenschaftstests von Siegel, Grabsteinen, AAD und Textfassung (D5a-1, lva#441). Fälle, in denen eine
// Fälschung nichts bewirken darf: „gefälschter“ oder ungesiegelter Grabstein, fremde key_id, fremdes Objekt.

private let key: [UInt8] = (0..<32).map { UInt8($0) }
private let anderer: [UInt8] = (0..<32).map { UInt8($0 == 0 ? 99 : $0) }
private let kid = SyncCrypto.keyId(key)
private let DAT = "dat_01HZX4Q8E5K2V7M3N9P6R1S0TA"
private let DAT2 = "dat_01HZX4Q8E5K2V7M3N9P6R1S0TB"
private let COLL = "ablage_datei"

private func seal(_ klar: String, key k: [UInt8] = key, collection: String = COLL, id: String = DAT, keyId: String = kid,
                  deleted: Bool = true, refs: [String] = []) throws -> String {
    try SyncCrypto.seal(key: k, aad: AblageKrypto.aadAblage(userId: 7, collection: collection, objectId: id, hubBlobRefs: refs, keyId: keyId, deleted: deleted),
                        plaintext: Array(klar.utf8))
}

private func echt(_ payload: String, key k: [UInt8] = key, collection: String = COLL, id: String = DAT, keyId: String = kid, user: Int64 = 7) -> Bool {
    AblageKrypto.grabEcht(key: k, userId: user, collection: collection, id: id, keyId: keyId, payload: payload)
}

final class AblageKryptoTests: XCTestCase {
    // MARK: AAD

    func testAADAufbau() {
        let a = AblageKrypto.aadAblage(userId: 7, collection: "ablage_datei", objectId: DAT, hubBlobRefs: ["aa", "bb"], keyId: "0a0b0c0d", deleted: false)
        XCTAssertEqual(String(decoding: a, as: UTF8.self),
                       "7\u{1f}ablage_datei\u{1f}\(DAT)\u{1f}blob_refs\u{1f}aa,bb\u{1f}key_id\u{1f}0a0b0c0d\u{1f}deleted\u{1f}0")
        // Leere Refs: leerer Text zwischen den Trennern; Grabstein: deleted = 1.
        let g = AblageKrypto.aadAblage(userId: -7, collection: "ablage_ordner", objectId: "ord_x", hubBlobRefs: [], keyId: "k", deleted: true)
        XCTAssertEqual(String(decoding: g, as: UTF8.self), "-7\u{1f}ablage_ordner\u{1f}ord_x\u{1f}blob_refs\u{1f}\u{1f}key_id\u{1f}k\u{1f}deleted\u{1f}1")
        XCTAssertTrue(g.starts(with: SyncCrypto.aad(userId: -7, collection: "ablage_ordner", objectId: "ord_x")))
    }

    func testAADBindetJedesFeld() throws {
        let sealed = try seal("{}", deleted: false, refs: ["aa"])
        func offen(collection: String = COLL, id: String = DAT, refs: [String] = ["aa"], keyId: String = kid, deleted: Bool = false, user: Int64 = 7) -> Bool {
            (try? SyncCrypto.open(key: key, aad: AblageKrypto.aadAblage(userId: user, collection: collection, objectId: id, hubBlobRefs: refs, keyId: keyId, deleted: deleted),
                                  sealedBase64: sealed)) != nil
        }
        XCTAssertTrue(offen())
        XCTAssertFalse(offen(collection: "ablage_ordner"))
        XCTAssertFalse(offen(id: DAT2))
        XCTAssertFalse(offen(refs: ["ab"]), "vertauschte Referenz")
        XCTAssertFalse(offen(refs: ["aa", "bb"]))
        XCTAssertFalse(offen(refs: []))
        XCTAssertFalse(offen(keyId: "deadbeef"), "umgeschriebene Schlüsselkennung")
        XCTAssertFalse(offen(deleted: true), "zum Grabstein erklärt")
        XCTAssertFalse(offen(user: 8))
    }

    // MARK: Deterministisches Siegel

    func testSiegelIstDeterministischUndEingabeabhaengig() throws {
        let aad = AblageKrypto.aadAblage(userId: 7, collection: COLL, objectId: DAT, hubBlobRefs: [], keyId: kid, deleted: true)
        let a = try AblageKrypto.sealDeterministisch(key: key, aad: aad, plaintext: Array("x".utf8))
        XCTAssertEqual(a, try AblageKrypto.sealDeterministisch(key: key, aad: aad, plaintext: Array("x".utf8)), "Wiederholung trägt dasselbe Chiffrat")
        XCTAssertNotEqual(a, try AblageKrypto.sealDeterministisch(key: key, aad: aad, plaintext: Array("y".utf8)))
        XCTAssertNotEqual(a, try AblageKrypto.sealDeterministisch(key: key, aad: aad + [0], plaintext: Array("x".utf8)))
        XCTAssertNotEqual(a, try AblageKrypto.sealDeterministisch(key: anderer, aad: aad, plaintext: Array("x".utf8)))
        // Öffnet wie jedes andere Siegel; Nonce = die ersten 24 Bytes von SHA-256(…).
        XCTAssertEqual(try SyncCrypto.open(key: key, aad: aad, sealedBase64: a), Array("x".utf8))
        XCTAssertLessThan(aad.count, 256)
        var eingabe: [UInt8] = Array("lv-ablage-siegel-nonce".utf8)
        eingabe.append(0x1F)
        eingabe += key
        eingabe += [0, 0, 0, 0, 0, 0, 0, UInt8(aad.count)]
        eingabe += aad
        eingabe += Array("x".utf8)
        XCTAssertEqual(Array(Data(base64Encoded: a)!.prefix(24)), Array(SHA256.hash(data: Data(eingabe)).prefix(24)))
    }

    func testSchluesselFalscherLaengeStuerztNichtAb() {
        let kurz = [UInt8](repeating: 1, count: 31), lang = [UInt8](repeating: 1, count: 33)
        for k in [kurz, lang, []] {
            XCTAssertThrowsError(try AblageKrypto.sealDeterministisch(key: k, aad: [], plaintext: [])) { XCTAssertEqual($0 as? AblageError, .schluessel) }
            XCTAssertThrowsError(try AblageKrypto.grabNutzlast(key: k, userId: 7, collection: COLL, id: DAT))
            XCTAssertFalse(echt("AAAA", key: k))
            XCTAssertNil(AblageKrypto.hubBlobId(key: k, sha256Hex: String(repeating: "a", count: 64)))
            XCTAssertNil(AblageKrypto.standardOrdnerId(key: k))
        }
    }

    // MARK: Grabsteine

    func testEchterGrabsteinWirdAkzeptiert() throws {
        let n = try AblageKrypto.grabNutzlast(key: key, userId: 7, collection: COLL, id: DAT)
        XCTAssertTrue(echt(n))
        XCTAssertEqual(String(decoding: AblageKrypto.grabKlartext(collection: COLL, id: DAT), as: UTF8.self),
                       "{\"collection\":\"ablage_datei\",\"deleted\":true,\"id\":\"\(DAT)\"}", "Schlüssel in dieser Reihenfolge")
        // Auch ein mit zufälliger Nonce versiegelter, sonst echter Grabstein ist echt (der Desktop prüft nur Inhalt und AAD).
        XCTAssertTrue(echt(try seal(String(decoding: AblageKrypto.grabKlartext(collection: COLL, id: DAT), as: UTF8.self))))
        // Zusätzliche Felder im Klartext schaden nicht.
        XCTAssertTrue(echt(try seal("{\"collection\":\"\(COLL)\",\"deleted\":true,\"id\":\"\(DAT)\",\"neu\":1}")))
    }

    func testUngesiegelterGrabsteinWirdAbgelehnt() {
        XCTAssertFalse(echt(""), "leere Nutzlast = ohne Siegel (so sendet LibrarySync.deleteEverywhere für Bücher)")
        XCTAssertFalse(echt("   "))
        XCTAssertFalse(echt("AAAA"), "zu kurz")
        XCTAssertFalse(echt("nicht base64!"))
        XCTAssertFalse(echt(Data("{\"collection\":\"ablage_datei\",\"deleted\":true,\"id\":\"\(DAT)\"}".utf8).base64EncodedString()), "Klartext als Nutzlast")
        XCTAssertFalse(echt(Data((0..<80).map { UInt8($0) }).base64EncodedString()), "zufällige Bytes")
    }

    func testGefaelschterGrabsteinWirdAbgelehnt() throws {
        let klar = String(decoding: AblageKrypto.grabKlartext(collection: COLL, id: DAT), as: UTF8.self)
        // Falscher Schlüssel (Hub oder Dritter ohne den Schlüssel kann nichts siegeln).
        XCTAssertFalse(echt(try seal(klar, key: anderer)))
        // AAD ohne deleted=1: ein gewöhnliches Objekt kann nicht zum Grabstein umgedeutet werden.
        XCTAssertFalse(echt(try seal(klar, deleted: false)))
        // Richtige AAD, aber Klartext bestätigt nichts oder Falsches.
        XCTAssertFalse(echt(try seal("{\"collection\":\"\(COLL)\",\"deleted\":false,\"id\":\"\(DAT)\"}")))
        XCTAssertFalse(echt(try seal("{\"collection\":\"\(COLL)\",\"id\":\"\(DAT)\"}")))
        XCTAssertFalse(echt(try seal("{\"collection\":\"\(COLL)\",\"deleted\":\"true\",\"id\":\"\(DAT)\"}")))
        XCTAssertFalse(echt(try seal("{\"collection\":\"\(COLL)\",\"deleted\":1,\"id\":\"\(DAT)\"}")))
        XCTAssertFalse(echt(try seal("{\"collection\":\"\(COLL)\",\"deleted\":true,\"id\":\"\(DAT2)\"}")), "Klartext nennt anderes Objekt")
        XCTAssertFalse(echt(try seal("{\"collection\":\"ablage_ordner\",\"deleted\":true,\"id\":\"\(DAT)\"}")), "Klartext nennt andere Sammlung")
        XCTAssertFalse(echt(try seal("kein json")))
        XCTAssertFalse(echt(try seal("[true]")))
        XCTAssertFalse(echt(try seal("")))
        // Echter Grabstein, an anderem Ort vorgelegt (Hub verschiebt das Siegel).
        let fuerDat = try AblageKrypto.grabNutzlast(key: key, userId: 7, collection: COLL, id: DAT)
        XCTAssertFalse(echt(fuerDat, id: DAT2), "anderes Objekt")
        XCTAssertFalse(echt(fuerDat, collection: "ablage_ordner"), "andere Sammlung")
        XCTAssertFalse(echt(fuerDat, user: 8), "anderer Benutzer")
        XCTAssertFalse(echt(fuerDat, key: anderer, keyId: SyncCrypto.keyId(anderer)), "anderer Schlüssel")
        // Eine gewöhnliche Nutzlast (Datei mit Refs) ist kein Grabstein.
        XCTAssertFalse(echt(try seal(klar, deleted: false, refs: ["aa"])))
    }

    func testFremdeKeyIdWirdAbgelehnt() throws {
        let n = try AblageKrypto.grabNutzlast(key: key, userId: 7, collection: COLL, id: DAT)
        XCTAssertFalse(echt(n, keyId: "deadbeef"), "fremde key_id des Objekts")
        XCTAssertFalse(echt(n, keyId: kid.uppercased()), "Schreibweise zählt (Bytevergleich)")
        // Mit fremder key_id in der AAD versiegelt: auch dann nicht echt, weil die key_id nicht die eigene ist.
        let klar = String(decoding: AblageKrypto.grabKlartext(collection: COLL, id: DAT), as: UTF8.self)
        XCTAssertFalse(echt(try seal(klar, keyId: "deadbeef"), keyId: "deadbeef"))
    }

    // MARK: Hub-Kennung

    func testHubKennungHexEingaben() {
        let sha = SHA256.hash(data: Data("Hallo Buch".utf8)).map { String(format: "%02x", $0) }.joined()
        let id = AblageKrypto.hubBlobId(key: key, sha256Hex: sha)
        XCTAssertEqual(id, LibrarySyncFormat.blobId(key: key, plaintextSha256: Data(SHA256.hash(data: Data("Hallo Buch".utf8)))))
        XCTAssertEqual(AblageKrypto.hubBlobId(key: key, sha256Hex: sha.uppercased()), id, "Großbuchstaben nimmt der Desktop an")
        for schlecht in [String(sha.dropLast()), sha + "0", "", String(repeating: "g", count: 64), "-1" + sha.dropFirst(2), "é" + sha.dropFirst(2), " " + sha.dropFirst()] {
            XCTAssertNil(AblageKrypto.hubBlobId(key: key, sha256Hex: schlecht), schlecht)
        }
    }

    func testHubRefsSortiertEindeutigUndUebergehtUngueltiges() {
        let h = { (c: Character) in String(repeating: c, count: 64) }
        func text(_ blob: String, rev: String) -> AblageText { AblageText(textrev: rev, blob: blob, size: 1, extraktor: "e", extraktorVersion: "1", markenVersion: 1, stand: "ok") }
        let f1 = AblageFassung(fas: "fas_1", vorgaenger: nil, blob: h("b"), size: 1, originalSha256: h("b"), typ: "txt", geraet: "g", zeit: 1,
                               texte: [text(h("a"), rev: "1"), text("", rev: "2"), text("kaputt", rev: "3"), text(h("b"), rev: "4")])
        let f2 = AblageFassung(fas: "fas_2", vorgaenger: nil, blob: "kein-hex", size: 1, originalSha256: h("c"), typ: "txt", geraet: "g", zeit: 1, texte: [])
        let d = AblageDatei(id: DAT, ordner: AblageRegister(wert: "o", zaehler: 1, geraet: "g"), name: AblageRegister(wert: "n", zaehler: 1, geraet: "g"),
                            klasseMin: AblageRegister(wert: "public", zaehler: 1, geraet: "g"), fassungen: [f1, f2],
                            papierkorb: AblageRegister(wert: nil, zaehler: 1, geraet: "g"))
        let erwartet = [h("a"), h("b")].map { AblageKrypto.hubBlobId(key: key, sha256Hex: $0)! }.sorted()
        XCTAssertEqual(AblageKrypto.hubRefs(datei: d, key: key), erwartet)
    }

    // MARK: Hash

    func testHashEinerDateiIstUnterNebenlaeufigkeitStabil() {
        let json = "{\"id\":\"\(DAT)\",\"ordner\":{\"wert\":\"o\",\"zaehler\":1,\"geraet\":\"g\"},\"name\":{\"wert\":\"n\",\"zaehler\":1,\"geraet\":\"g\"},"
            + "\"klasse_min\":{\"wert\":\"public\",\"zaehler\":1,\"geraet\":\"g\"},\"papierkorb\":{\"wert\":null,\"zaehler\":1,\"geraet\":\"g\"},\"x\":{\"b\":1,\"a\":[null]}}"
        let d = try! AblageDatei(nutzlast: Array(json.utf8))
        let erwartet = d.hash(ordnen: true)
        let lock = NSLock()
        nonisolated(unsafe) var abweichungen = 0
        DispatchQueue.concurrentPerform(iterations: 200) { _ in
            let h = (try? AblageDatei(nutzlast: Array(json.utf8)))?.hash(ordnen: true)
            if h != erwartet { lock.lock(); abweichungen += 1; lock.unlock() }
        }
        XCTAssertEqual(abweichungen, 0)
        XCTAssertEqual(erwartet.count, 64)
    }

    // MARK: Textfassung

    private func textfassungBytes(textrev: String = "T", original: String = "O") -> [UInt8] {
        // Bewusst „unschön“ formatiert, mit unbekanntem Feld: die Bytes dürfen nie neu serialisiert werden.
        Array("{ \"marken_version\": 1,\n \"textrev\": \"\(textrev)\", \"original_sha256\": \"\(original)\", \"extraktor\": \"lva-text\",\n \"extraktor_version\": \"1\", \"zusatz\": [1,2], \"abschnitte\": [ {\"anker\":\"zeile=1\",\"text\":\"Größe\"}, {\"anker\":\"absatz=2\",\"pfad\":[\"§ 4\"],\"text\":\"x\"} ] }".utf8)
    }

    func testTextfassungBleibtUnveraendertUndWirdGeprueft() throws {
        let bytes = textfassungBytes()
        let tf = try AblageTextfassung(bytes: bytes)
        XCTAssertEqual(tf.bytes, bytes, "Bytes unverändert")
        XCTAssertEqual(tf.sha256Hex, AblageKrypto.sha256Hex(bytes))
        XCTAssertEqual(tf.abschnitte, [AblageAbschnitt(anker: "zeile=1", pfad: [], text: "Größe"), AblageAbschnitt(anker: "absatz=2", pfad: ["§ 4"], text: "x")])
        let fassung = AblageFassung(fas: "fas_1", vorgaenger: nil, blob: "O", size: 1, originalSha256: "O", typ: "txt", geraet: "g", zeit: 1)
        let text = AblageText(textrev: "T", blob: tf.sha256Hex, size: UInt64(bytes.count), extraktor: "lva-text", extraktorVersion: "1", markenVersion: 1, stand: "ok")
        XCTAssertTrue(tf.passt(zu: text, fassung: fassung))
        var falsch = text; falsch.blob = String(repeating: "0", count: 64)
        XCTAssertFalse(tf.passt(zu: falsch, fassung: fassung), "SHA-256 der Bytes ≠ Text.blob")
        falsch = text; falsch.textrev = "X"
        XCTAssertFalse(tf.passt(zu: falsch, fassung: fassung), "textrev im Kopf ≠ Eintrag")
        var fremd = fassung; fremd.originalSha256 = "P"
        XCTAssertFalse(tf.passt(zu: text, fassung: fremd), "original_sha256 im Kopf ≠ Fassung")
        falsch = text; falsch.blob = tf.sha256Hex.uppercased()
        XCTAssertFalse(tf.passt(zu: falsch, fassung: fassung), "Schreibweise zählt")
        // Verändertes Byte: Hash passt nicht mehr.
        var kaputt = bytes; kaputt[kaputt.count - 3] ^= 1
        if let t2 = try? AblageTextfassung(bytes: kaputt) { XCTAssertFalse(t2.passt(zu: text, fassung: fassung)) }
    }

    func testTextfassungPflichtfelderUndGrenzen() {
        XCTAssertThrowsError(try AblageTextfassung(bytes: Array("{\"textrev\":\"a\"}".utf8)))
        XCTAssertThrowsError(try AblageTextfassung(bytes: Array("[]".utf8)))
        XCTAssertThrowsError(try AblageTextfassung(bytes: Array("{".utf8)))
        XCTAssertThrowsError(try AblageTextfassung(bytes: textfassungBytes().map { $0 == UInt8(ascii: "[") ? UInt8(ascii: "{") : $0 }))
    }

    func testTextrevAbleitungBeiEintragUndFassung() {
        let f = AblageFassung(fas: "fas_1", vorgaenger: nil, blob: "O", size: 1, originalSha256: "ab" + String(repeating: "0", count: 62), typ: "txt", geraet: "g", zeit: 1)
        let rev = AblageTextfassung.textrev(originalSha256Hex: f.originalSha256, extraktor: "lva-text", extraktorVersion: "1", markenVersion: 1)
        var t = AblageText(textrev: rev, blob: "", size: 0, extraktor: "lva-text", extraktorVersion: "1", markenVersion: 1, stand: "ok")
        XCTAssertTrue(AblageTextfassung.textrevStimmt(text: t, fassung: f))
        t.extraktorVersion = "2"
        XCTAssertFalse(AblageTextfassung.textrevStimmt(text: t, fassung: f), "andere Extraktorversion = andere textrev")
        XCTAssertNotEqual(rev, AblageTextfassung.textrev(originalSha256Hex: f.originalSha256, extraktor: "lva-text", extraktorVersion: "1", markenVersion: 2))
        // Felder sind durch Nullbytes getrennt: Verschiebungen zwischen Feldern ergeben andere Werte.
        XCTAssertNotEqual(AblageTextfassung.textrev(originalSha256Hex: "a", extraktor: "bc", extraktorVersion: "d", markenVersion: 1),
                          AblageTextfassung.textrev(originalSha256Hex: "ab", extraktor: "c", extraktorVersion: "d", markenVersion: 1))
    }
}
#endif
