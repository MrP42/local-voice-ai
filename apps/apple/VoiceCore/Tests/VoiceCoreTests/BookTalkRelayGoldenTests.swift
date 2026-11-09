import Foundation
import XCTest
@testable import VoiceCore

// Buch-Gespräch (lva#320, Spec §11.7 R2/R3, Befund 13): gemeinsame Golden-Datei `tooling/fixtures/buch-gespraech/relay/golden.json`
// (erzeugt vom PC-Zweig, unabhängig von Rust und Swift gerechnet). Der Test läuft die Datei GENERISCH durch: jedes Objekt mit
// `canonical` und `mac` wird gegen die Swift-Kanonisierung und den HMAC geprüft, jeder Einrichtungscode gegen sein JSON.
// Erweitert der PC die Datei (Sonderzeichen-Fälle, weitere Antworten, Modelllisten), läuft das ohne Änderung hier mit.

final class BookTalkRelayGoldenTests: XCTestCase {
    private static func fileURL() -> URL {
        if let override = ProcessInfo.processInfo.environment["BOOKTALK_RELAY_GOLDEN"] { return URL(fileURLWithPath: override) }
        var url = URL(fileURLWithPath: #filePath).resolvingSymlinksInPath()
        for _ in 0..<6 { url.deleteLastPathComponent() }
        return url.appendingPathComponent("tooling/fixtures/buch-gespraech/relay/golden.json")
    }

    private func load() throws -> [String: Any] {
        let data = try Data(contentsOf: Self.fileURL())
        guard let root = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            XCTFail("golden.json ist kein Objekt"); return [:]
        }
        return root
    }

    private func key(_ root: [String: Any]) throws -> [UInt8] {
        let hex = try XCTUnwrap(root["key_hex"] as? String)
        let bytes = stride(from: 0, to: hex.count, by: 2).map { i -> UInt8 in
            let a = hex.index(hex.startIndex, offsetBy: i)
            return UInt8(hex[a..<hex.index(a, offsetBy: 2)], radix: 16) ?? 0
        }
        XCTAssertEqual(bytes.count, 32)
        return bytes
    }

    private struct Counts { var requests = 0, responses = 0, lists = 0 }

    /// Alle Objekte mit `canonical` + `mac` und einer bekannten Nutzlast (`request`, `response`, `list`) rekursiv.
    private func walk(_ node: Any, key: [UInt8], counts: inout Counts, path: String = "$") throws {
        if let array = node as? [Any] {
            for (i, item) in array.enumerated() { try walk(item, key: key, counts: &counts, path: "\(path)[\(i)]") }
            return
        }
        guard let dict = node as? [String: Any] else { return }
        if let canonical = dict["canonical"] as? String, let mac = dict["mac"] as? String {
            if let payload = dict["request"] as? [String: Any] {
                try checkRequest(payload, canonical: canonical, mac: mac, wrapper: dict, key: key, path: path)
                counts.requests += 1
            } else if let payload = dict["response"] as? [String: Any] {
                try checkResponse(payload, canonical: canonical, mac: mac, key: key, path: path)
                counts.responses += 1
            } else if let payload = dict["list"] as? [String: Any] {
                try checkList(payload, canonical: canonical, mac: mac, key: key, path: path)
                counts.lists += 1
            }
        }
        for (name, value) in dict where name != "canonical" { try walk(value, key: key, counts: &counts, path: "\(path).\(name)") }
    }

    private func decode<T: Decodable>(_ type: T.Type, _ object: [String: Any], _ path: String) throws -> T {
        try JSONDecoder().decode(type, from: JSONSerialization.data(withJSONObject: object))
    }

    private func hmacHex(_ key: [UInt8], _ text: String) -> String { BookTalkHash.hex(BookTalkHash.hmacSHA256(key: key, message: Array(text.utf8))) }

    private func checkRequest(_ payload: [String: Any], canonical: String, mac: String, wrapper: [String: Any], key: [UInt8], path: String) throws {
        let r = try decode(BookTalkRelayRequest.self, payload, path)
        XCTAssertEqual(r.canonical, canonical, "\(path): Kanonisierung der Anfrage")
        XCTAssertEqual(hmacHex(key, canonical), mac, "\(path): MAC über die Kanonisierung der Datei")
        XCTAssertEqual(r.mac, mac, "\(path): mac der Anfrage")
        XCTAssertTrue(r.macIsValid(key: key), "\(path): macIsValid")
        if let h = wrapper["system_sha256"] as? String { XCTAssertEqual(BookTalkHash.sha256Hex(r.system), h, "\(path): system_sha256") }
        if let h = wrapper["prompt_sha256"] as? String { XCTAssertEqual(BookTalkHash.sha256Hex(r.prompt), h, "\(path): prompt_sha256") }
        // Neu gebaut aus denselben Eingaben, mit derselben Uhr: gleiche Anfrage, gleicher MAC (Fristen- und Token-Kappung gelten mit).
        let rebuilt = BookTalkRelayRequest.make(key: key, keyId: r.keyId, deviceId: r.deviceId, requestId: r.requestId, modelId: r.modelId,
                                                system: r.system, prompt: r.prompt, maxTokens: r.maxTokens, now: r.createdAt)
        XCTAssertEqual(rebuilt, r, "\(path): `make` reproduziert die Anfrage der Datei")
        XCTAssertTrue(r.verify(key: key, deviceId: r.deviceId, now: r.createdAt + 5), "\(path): PC-Prüfung")
    }

    private func checkResponse(_ payload: [String: Any], canonical: String, mac: String, key: [UInt8], path: String) throws {
        let r = try decode(BookTalkRelayResponse.self, payload, path)
        XCTAssertEqual(r.canonical, canonical, "\(path): Kanonisierung der Antwort")
        XCTAssertEqual(hmacHex(key, canonical), mac, "\(path): MAC über die Kanonisierung der Datei")
        XCTAssertEqual(r.mac, mac)
        XCTAssertTrue(r.macIsValid(key: key), "\(path): macIsValid")
        let signed = BookTalkRelayResponse.signed(key: key, requestId: r.requestId, deviceId: r.deviceId, seq: r.seq, modelTuple: r.modelTuple,
                                                  text: r.text, done: r.done, error: r.error)
        XCTAssertEqual(signed.mac, mac, "\(path): `signed` reproduziert den MAC")
        var tampered = payload
        tampered["text"] = (payload["text"] as? String ?? "") + "!"
        XCTAssertFalse(try decode(BookTalkRelayResponse.self, tampered, path).macIsValid(key: key), "\(path): veränderter Text bricht den MAC")
    }

    private func checkList(_ payload: [String: Any], canonical: String, mac: String, key: [UInt8], path: String) throws {
        let list = try decode(BookTalkRelayModelList.self, payload, path)
        let desktop = try XCTUnwrap(list.desktopDeviceId, path), device = try XCTUnwrap(list.deviceId, path), issued = try XCTUnwrap(list.issuedAt, path)
        XCTAssertEqual(BookTalkRelayModelList.canonical(models: list.models, desktopDeviceId: desktop, deviceId: device, issuedAt: issued, version: list.v),
                       canonical, "\(path): Kanonisierung der Modellliste")
        XCTAssertEqual(hmacHex(key, canonical), mac, "\(path): MAC über die Kanonisierung der Datei")
        XCTAssertTrue(list.isAuthentic(key: key, desktopDeviceId: desktop, deviceId: device), "\(path): isAuthentic")
    }

    func testFileExistsAndKeyMaterialIsConsistent() throws {
        let root = try load()
        let k = try key(root)
        XCTAssertEqual(BookTalkBase64URL.decode(try XCTUnwrap(root["key_b64url"] as? String)), k)
        XCTAssertEqual(BookTalkBase64URL.encode(k), root["key_b64url"] as? String)
        let keyId = BookTalkHash.hex(Array(BookTalkHash.sha256(Array("lv-booktalk-relay-key-id-v1".utf8) + k).prefix(8)))
        XCTAssertEqual(keyId, root["key_id"] as? String, "key_id = erste 8 Byte von SHA-256(Präfix + Schlüssel)")
    }

    func testCanonicalizationAndMacsOfAllRequestsResponsesAndLists() throws {
        let root = try load()
        var counts = Counts()
        try walk(root, key: try key(root), counts: &counts)
        XCTAssertGreaterThanOrEqual(counts.requests, 1, "mindestens eine Anfrage in der Datei")
        XCTAssertGreaterThanOrEqual(counts.responses, 3, "pending, final und Fehlerantwort")
    }

    func testResponsesCoverPendingFinalAndErrorSemantics() throws {
        let root = try load()
        let k = try key(root)
        var found: [BookTalkRelayResponse] = []
        var counts = Counts()
        func collect(_ node: Any) {
            if let a = node as? [Any] { a.forEach(collect) }
            guard let d = node as? [String: Any] else { return }
            if let p = d["response"] as? [String: Any], let r = try? decode(BookTalkRelayResponse.self, p, "") { found.append(r) }
            d.values.forEach(collect)
        }
        collect(root)
        try walk(root, key: k, counts: &counts)
        XCTAssertTrue(found.contains { $0.modelTuple == BookTalkRelayModelMatch.pending && !$0.done })
        XCTAssertTrue(found.contains { $0.done && $0.error == nil && BookTalkRelayModelMatch.accepts(alias: "sonnet", tuple: $0.modelTuple) })
        XCTAssertTrue(found.contains { $0.modelTuple == BookTalkRelayModelMatch.unknown && $0.error != nil })
    }

    func testEnrollmentCodesMatchTheirJsonAndCanonicalForm() throws {
        let root = try load()
        var checked = 0
        func visit(_ node: Any, _ path: String) throws {
            if let a = node as? [Any] { for (i, x) in a.enumerated() { try visit(x, "\(path)[\(i)]") } }
            guard let d = node as? [String: Any] else { return }
            if let json = d["json"] as? [String: Any], let code = d["code"] as? String {
                try checkEnrollment(json: json, code: code, path: path)
                checked += 1
            }
            for (n, v) in d { try visit(v, "\(path).\(n)") }
        }
        try visit(root, "$")
        XCTAssertGreaterThanOrEqual(checked, 1, "mindestens ein Einrichtungscode")
    }

    private func checkEnrollment(json: [String: Any], code: String, path: String) throws {
        XCTAssertTrue(code.hasPrefix("lvrelay1:"), path)
        let parsed = try XCTUnwrap(BookTalkRelayEnrollment.parse(code), "\(path): Code lässt sich lesen")
        let fromJson = try decode(BookTalkRelayEnrollment.self, json, path)
        XCTAssertEqual(parsed, fromJson, "\(path): Code und JSON ergeben dieselbe Einrichtung")
        XCTAssertEqual(parsed.keyId, json["key_id"] as? String)
        XCTAssertEqual(parsed.desktopDeviceId, json["desktop_device_id"] as? String)
        XCTAssertEqual(parsed.desktopName, json["desktop_name"] as? String)
        XCTAssertEqual(parsed.deviceId, json["device_id"] as? String)
        XCTAssertEqual(parsed.modelIds, json["model_ids"] as? [String] ?? [])
        XCTAssertEqual(parsed.expiresAt, json["expires_at"] as? Int)
        XCTAssertEqual(parsed.grantExpiresAt, json["grant_expires_at"] as? Int)
        XCTAssertEqual(BookTalkBase64URL.encode(parsed.key), json["k"] as? String)
        XCTAssertEqual(parsed.key.count, 32)
        // Freigabe-Ablauf: ohne `grant_expires_at` gilt `expires_at`, sonst getrennt vom Code-Ablauf.
        let grant = parsed.grantExpiresAt ?? parsed.expiresAt
        XCTAssertTrue(parsed.codeExpired(now: parsed.expiresAt))
        XCTAssertFalse(parsed.codeExpired(now: parsed.expiresAt - 1))
        XCTAssertTrue(parsed.isExpired(now: grant))
        XCTAssertFalse(parsed.isExpired(now: grant - 1))
        // Der Code ist kanonisch (Schlüssel alphabetisch, ohne Leerraum, UTF-8 unmaskiert), soweit sich die Werte so darstellen lassen.
        if let canonical = canonicalJSON(json) { XCTAssertEqual(String(code.dropFirst("lvrelay1:".count)), canonical, "\(path): kanonische Form des Codes") }
    }

    private func canonicalJSON(_ json: [String: Any]) -> String? {
        var parts: [String] = []
        for name in json.keys.sorted(by: { Array($0.utf8).lexicographicallyPrecedes(Array($1.utf8)) }) {
            let value = json[name]!
            let text: String
            if let s = value as? String { text = BookTalkCanonical.quote(s) }
            else if let list = value as? [String] { text = "[" + list.map(BookTalkCanonical.quote).joined(separator: ",") + "]" }
            else if let n = value as? Int { text = String(n) }
            else { return nil }
            parts.append(BookTalkCanonical.quote(name) + ":" + text)
        }
        return "{" + parts.joined(separator: ",") + "}"
    }
}
