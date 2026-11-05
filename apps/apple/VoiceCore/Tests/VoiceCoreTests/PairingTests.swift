import CryptoKit
import XCTest
@testable import VoiceCore

private func vectors() throws -> [String: Any] {
    let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
    let file = root.appendingPathComponent("docs/superpowers/specs/2026-11-05-geraete-kopplung-qr-vektoren.json")
    return try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: file)) as? [String: Any])
}

private func uri(hub: String = "https%3A%2F%2Fportal.wolffappliedai.de", id: String = "AAECAwQFBgcICQoLDA0ODw", version: String = "2",
                 key: [UInt8] = [UInt8](repeating: 3, count: 32), q: [UInt8] = [UInt8](repeating: 2, count: 32)) -> String {
    let secret = PairingProto.b64u([UInt8](repeating: 1, count: 32))
    return "localvoice://pair?v=\(version)&h=\(hub)&p=\(id)&s=\(secret)&q=\(PairingProto.b64u(q))&k=\(PairingProto.b64u(key))"
}

/// Hub und Desktop in einem, nach Abschnitt 4.3. Standard ist der glatte Ablauf;
/// die Schalter stellen die Störungen aus Tabelle 5.3 her.
private actor FakeHub: PairingTransport {
    struct Fail: Error {}
    let desktop = Curve25519.KeyAgreement.PrivateKey()
    let q = [UInt8](repeating: 7, count: 32)
    let accountKey = [UInt8](repeating: 9, count: 32)
    let id = "AAECAwQFBgcICQoLDA0ODw"
    var nonce = [UInt8](repeating: 5, count: 32)
    var status = "claimed"
    var phonePublic: [UInt8] = []
    var phoneNonce: [UInt8] = []
    var commitment: [UInt8] = []
    var name = ""
    var reveals: [[String: String]] = []
    var confirms: [String] = []
    var calls: [String] = []
    var polls = 0

    // Störungen
    var claimFails = false
    var loseFirstRevealAnswer = false
    var failPolls = 0
    var changeNonceOnRepoll = false
    var rejectAfterReveal = false
    var expireAfterReveal = false
    var wrongQ = false
    var wrongKeyId = false
    var wrongUser = false
    var skipReveal = false
    var confirmFails = 0
    var confirmAnswerLost = false
    var hubLostRevealAnswer = false
    var claimStatus = 200
    var revealStatus = 200
    var fakeEmail: String?
    func set(_ change: @Sendable (isolated FakeHub) -> Void) { change(self) }

    var code: PairingCode {
        get throws {
            let hub = "https%3A%2F%2Fhub.example"
            return try PairingCode(uri: uri(hub: hub, id: id, key: [UInt8](desktop.publicKey.rawRepresentation), q: q))
        }
    }

    func post(_ path: String, body: [String: String], pollToken: String?) async throws -> (Int, Data) {
        calls.append(path)
        if path.hasSuffix("/claim") {
            if claimFails { throw Fail() }
            if claimStatus == 400 { return (400, Data(#"{"error":{"code":"invalid_device_name"}}"#.utf8)) }
            if claimStatus != 200 { return (claimStatus, Data()) }
            name = body["device_name"] ?? ""
            commitment = PairingProto.b64u32(body["phone_commit"] ?? "") ?? []
            let mac = PairingProto.claimMac(q: q, id: id, commitment: commitment)
            guard PairingProto.b64u(mac) == body["phone_mac"] else { return (400, Data()) }
            status = "challenged"
            return (200, Data(#"{"poll_token":"poll","expires_at":1}"#.utf8))
        }
        XCTAssertEqual(pollToken, "poll")
        if path.hasSuffix("/reveal") {
            reveals.append(body)
            if revealStatus != 200 { return (revealStatus, Data()) }
            phonePublic = PairingProto.b64u32(body["phone_pub"] ?? "") ?? []
            phoneNonce = PairingProto.b64u32(body["phone_nonce"] ?? "") ?? []
            let check = PairingProto.commitment(id: id, phonePublic: phonePublic, phoneNonce: phoneNonce, name: name)
            guard check == commitment else { return (409, Data()) }
            if loseFirstRevealAnswer, reveals.count == 1 { throw Fail() }  // bleibt am Hub „challenged“
            if hubLostRevealAnswer { status = "revealed"; throw Fail() }  // wie am Hub: erledigt, Antwort weg
            if loseFirstRevealAnswer { status = "revealed"; return (410, Data()) }  // Wiederholung: schon erledigt
            status = "revealed"
            return (200, Data(#"{"status":"revealed"}"#.utf8))
        }
        confirms.append(body["confirm"] ?? "")
        if confirmAnswerLost {
            if confirms.count == 1 { throw Fail() }
            return (410, Data())
        }
        if confirmFails > 0 { confirmFails -= 1; throw Fail() }
        return (200, Data(#"{"status":"confirmed"}"#.utf8))
    }

    func get(_ path: String, pollToken: String) async throws -> (Int, Data) {
        polls += 1
        if failPolls > 0 { failPolls -= 1; throw Fail() }
        switch status {
        case "challenged": return challenge()
        case "revealed":
            if changeNonceOnRepoll { nonce[0] ^= 1; return challenge() }
            if rejectAfterReveal { return (403, Data()) }
            if expireAfterReveal { return (410, Data()) }
            return result()
        default: return (202, Data(#"{"status":"waiting"}"#.utf8))
        }
    }

    private func challenge() -> (Int, Data) {
        if skipReveal { phonePublic = [UInt8](repeating: 1, count: 32); return result() }
        return (200, Data(#"{"status":"challenge","desktop_nonce":"\#(PairingProto.b64u(nonce))"}"#.utf8))
    }

    func sessionKey() throws -> [UInt8] {
        let peer = try Curve25519.KeyAgreement.PublicKey(rawRepresentation: phonePublic)
        let z = try desktop.sharedSecretFromKeyAgreement(with: peer).withUnsafeBytes { Array($0) }
        return PairingProto.sessionKey(z: z, q: wrongQ ? [UInt8](repeating: 8, count: 32) : q,
                                       desktopPublic: [UInt8](desktop.publicKey.rawRepresentation), phonePublic: phonePublic, id: id)
    }

    private func result() -> (Int, Data) {
        let s = (try? sessionKey()) ?? []
        let keyId = wrongKeyId ? "00000000" : SyncCrypto.keyId(accountKey)
        let plain = #"{"v":1,"k":"\#(PairingProto.b64u(accountKey))","email":"\#(fakeEmail ?? "p@example.org")","user_id":42,"key_id":"\#(keyId)"}"#
        let sealNonce = [UInt8](repeating: 4, count: 24)
        let aad = PairingProto.sealAAD(id: id, desktopPublic: [UInt8](desktop.publicKey.rawRepresentation), phonePublic: phonePublic)
        let box = (try? XChaCha20Poly1305.seal(key: s, nonce: sealNonce, plaintext: Array(plain.utf8), aad: aad)) ?? []
        let sealed = Data(sealNonce + box).base64EncodedString()
        let user = wrongUser ? 43 : 42
        return (200, Data(#"{"status":"ok","token":"wai_pat_x","sealed":"\#(sealed)","user":{"id":\#(user),"email":"p@example.org"}}"#.utf8))
    }
}

/// Jede Abfrage der Uhr rückt zehn Sekunden vor.
private final class Ticker: @unchecked Sendable {
    private let lock = NSLock()
    private var seconds = 0.0
    func next() -> TimeInterval { lock.withLock { seconds += 10; return seconds } }
}

private actor Recorder {
    var stages: [PairingStage] = []
    var adopted: [PairedAccount] = []
    func add(_ stage: PairingStage) { stages.append(stage) }
    func add(_ account: PairedAccount) { adopted.append(account) }
}

final class PairingTests: XCTestCase {
    private func run(_ hub: FakeHub, name: String = "iPhone von Patrick", _ recorder: Recorder = Recorder(),
                     adoptError: Error? = nil) async throws -> Bool {
        let session = PairingSession(code: try await hub.code, name: name, transport: hub, pause: { _ in })
        return try await session.run(stage: { await recorder.add($0) }, adopt: {
            if let adoptError { throw adoptError }
            await recorder.add($0)
        })
    }

    private func assertFails(_ hub: FakeHub, _ expected: PairingError, file: StaticString = #filePath, line: UInt = #line) async {
        let recorder = Recorder()
        do {
            _ = try await run(hub, recorder)
            XCTFail("hätte scheitern müssen", file: file, line: line)
        } catch {
            XCTAssertEqual(error as? PairingError, expected, file: file, line: line)
        }
        let adopted = await recorder.adopted
        XCTAssertTrue(adopted.isEmpty, "bei Fehlern darf kein Konto entstehen", file: file, line: line)
    }

    // MARK: Testvektoren

    func testVectorsMatchSpec() throws {
        let v = try vectors()
        let input = try XCTUnwrap(v["eingaben"] as? [String: String])
        let want = try XCTUnwrap(v["ergebnisse"] as? [String: String])
        func raw(_ value: String?) throws -> [UInt8] { try XCTUnwrap(value.flatMap(PairingProto.b64u32)) }
        let id = try XCTUnwrap(input["pair_id"])
        let name = try XCTUnwrap(input["device_name"])
        let q = try raw(input["q"]), nP = try raw(input["n_p"]), nD = try raw(input["n_d"])
        let phone = try Curve25519.KeyAgreement.PrivateKey(rawRepresentation: raw(input["p_sk"]))
        let desktop = try Curve25519.KeyAgreement.PrivateKey(rawRepresentation: raw(input["d_sk"]))
        let pPk = [UInt8](phone.publicKey.rawRepresentation), dPk = [UInt8](desktop.publicKey.rawRepresentation)
        XCTAssertEqual(PairingProto.b64u(pPk), want["p_pk"])
        XCTAssertEqual(PairingProto.b64u(dPk), want["d_pk"])

        let commit = PairingProto.commitment(id: id, phonePublic: pPk, phoneNonce: nP, name: name)
        XCTAssertEqual(PairingProto.b64u(commit), want["phone_commit"])
        XCTAssertEqual(PairingProto.b64u(PairingProto.claimMac(q: q, id: id, commitment: commit)), want["phone_mac"])
        XCTAssertEqual(PairingProto.sas(id: id, desktopPublic: dPk, phonePublic: pPk, phoneNonce: nP, desktopNonce: nD), want["sas"])

        let z = try PairingProto.shared(phone, desktopPublic: dPk)
        XCTAssertEqual(PairingProto.b64u(z), want["z"])
        let s = PairingProto.sessionKey(z: z, q: q, desktopPublic: dPk, phonePublic: pPk, id: id)
        XCTAssertEqual(PairingProto.b64u(s), want["s"])
        let aad = PairingProto.sealAAD(id: id, desktopPublic: dPk, phonePublic: pPk)
        XCTAssertEqual(PairingProto.b64u(aad), want["aad"])
        let plain = try PairingProto.open(sealed: try XCTUnwrap(want["sealed"]), s: s, aad: aad)
        XCTAssertEqual(String(decoding: plain, as: UTF8.self), want["plaintext"])
        XCTAssertEqual(SyncCrypto.keyId(try raw(input["K"])), want["key_id"])
        XCTAssertEqual(PairingProto.b64u(PairingProto.confirm(s: s, id: id)), want["confirm"])
    }

    func testVectorNameLengthCountsBytes() throws {
        let v = try vectors()
        let input = try XCTUnwrap(v["eingaben"] as? [String: String])
        let extra = try XCTUnwrap(v["zusatz_utf8"] as? [String: Any])
        let name = try XCTUnwrap(extra["device_name"] as? String)
        XCTAssertEqual(name.utf8.count, extra["laenge_bytes"] as? Int)
        let phone = try Curve25519.KeyAgreement.PrivateKey(rawRepresentation: try XCTUnwrap(PairingProto.b64u32(input["p_sk"] ?? "")))
        let commit = PairingProto.commitment(id: try XCTUnwrap(input["pair_id"]), phonePublic: [UInt8](phone.publicKey.rawRepresentation),
                                             phoneNonce: try XCTUnwrap(PairingProto.b64u32(input["n_p"] ?? "")), name: name)
        XCTAssertEqual(PairingProto.b64u(commit), extra["phone_commit"] as? String)
    }

    func testZeroPointIsRefused() throws {
        let key = Curve25519.KeyAgreement.PrivateKey()
        XCTAssertThrowsError(try PairingProto.shared(key, desktopPublic: [UInt8](repeating: 0, count: 32)))
    }

    func testOnlyCanonicalEncoding() {
        let good = PairingProto.b64u([UInt8](repeating: 0xAB, count: 32))
        XCTAssertNotNil(PairingProto.b64u32(good))
        XCTAssertNil(PairingProto.b64u32(good + "="))
        XCTAssertNil(PairingProto.b64u32(String(good.dropLast())))
        XCTAssertNil(PairingProto.b64u32(good.replacingOccurrences(of: "q", with: "+")))
        // Letztes Zeichen trägt zwei ungenutzte Bits: 43 Zeichen, aber nicht kanonisch.
        XCTAssertNil(PairingProto.b64u32("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAB"))
        XCTAssertTrue(PairingProto.validPairId("AAECAwQFBgcICQoLDA0ODw"))
        XCTAssertFalse(PairingProto.validPairId("AAECAwQFBgcICQoLDA0OD1"))
    }

    // MARK: QR-Code

    func testCodeParses() throws {
        let code = try PairingCode(uri: uri())
        XCTAssertEqual(code.hub, "https://portal.wolffappliedai.de")
        XCTAssertEqual(code.hubHost, "portal.wolffappliedai.de")
        XCTAssertEqual(code.id, "AAECAwQFBgcICQoLDA0ODw")
        XCTAssertEqual(code.q, [UInt8](repeating: 2, count: 32))
    }

    func testCodeOnlyAcceptsHttps() throws {
        XCTAssertThrowsError(try PairingCode(uri: uri(hub: "http%3A%2F%2Fportal.wolffappliedai.de"))) { XCTAssertEqual($0 as? PairingError, .insecureHub) }
        XCTAssertThrowsError(try PairingCode(uri: uri(hub: "https%3A%2F%2Fa%3Ab%40evil.example"))) { XCTAssertEqual($0 as? PairingError, .insecureHub) }
        XCTAssertThrowsError(try PairingCode(uri: uri(hub: "http%3A%2F%2F127.0.0.1%3A9"))) { XCTAssertEqual($0 as? PairingError, .insecureHub) }
        XCTAssertEqual(try PairingCode(uri: uri(hub: "http%3A%2F%2F127.0.0.1%3A9"), allowLocalHTTP: true).hub, "http://127.0.0.1:9")
        XCTAssertThrowsError(try PairingCode(uri: uri(hub: "http%3A%2F%2F10.0.0.1"), allowLocalHTTP: true))
    }

    func testCodeRefusesOtherInput() {
        XCTAssertThrowsError(try PairingCode(uri: uri(version: "1"))) { XCTAssertEqual($0 as? PairingError, .outdatedCode) }
        XCTAssertThrowsError(try PairingCode(uri: "https://example.org/?v=2")) { XCTAssertEqual($0 as? PairingError, .invalidCode) }
        XCTAssertThrowsError(try PairingCode(uri: uri(id: "kurz"))) { XCTAssertEqual($0 as? PairingError, .invalidCode) }
        XCTAssertThrowsError(try PairingCode(uri: uri() + "&v=2")) { XCTAssertEqual($0 as? PairingError, .invalidCode) }
        XCTAssertThrowsError(try PairingCode(uri: "localvoice://pair")) { XCTAssertEqual($0 as? PairingError, .invalidCode) }
    }

    func testNameIsCleaned() {
        XCTAssertEqual(PairingName.clean("  iPhone\u{200B} von\u{202E} Patrick\n"), "iPhone von Patrick")
        XCTAssertEqual(PairingName.clean("A\u{0308}"), "Ä")
        XCTAssertEqual(PairingName.clean("\u{200B}\n"), "iPhone")
        XCTAssertEqual(PairingName.clean(String(repeating: "ä", count: 80)).unicodeScalars.count, 60)
    }

    // MARK: Ablauf nach 5.3

    func testHappyPath() async throws {
        let hub = FakeHub(), recorder = Recorder()
        let confirmed = try await run(hub, recorder)
        XCTAssertTrue(confirmed)
        let adopted = await recorder.adopted
        XCTAssertEqual(adopted.count, 1)
        XCTAssertEqual(adopted.first?.email, "p@example.org")
        XCTAssertEqual(adopted.first?.userId, 42)
        XCTAssertEqual(adopted.first?.token, "wai_pat_x")
        XCTAssertEqual(adopted.first?.key, [UInt8](repeating: 9, count: 32))
        XCTAssertEqual(adopted.first?.hub, "https://hub.example")
        let reveals = await hub.reveals
        XCTAssertEqual(reveals.count, 1)
        let confirms = await hub.confirms
        let s = try await hub.sessionKey()
        XCTAssertEqual(confirms, [PairingProto.b64u(PairingProto.confirm(s: s, id: "AAECAwQFBgcICQoLDA0ODw"))])

        // Der gezeigte Code ist derselbe, den der PC berechnet.
        let stages = await recorder.stages
        let code = try await hub.code
        let phonePublic = await hub.phonePublic, phoneNonce = await hub.phoneNonce, nonce = await hub.nonce
        let sas = PairingProto.sas(id: code.id, desktopPublic: code.desktopPublic, phonePublic: phonePublic, phoneNonce: phoneNonce, desktopNonce: nonce)
        XCTAssertEqual(stages, [.claiming, .waitingForDesktop, .compare(sas), .opening])
    }

    func testLostRevealAnswerRepeatsIdenticalValuesAnd410CountsAsDone() async throws {
        let hub = FakeHub(), recorder = Recorder()
        await hub.set { $0.loseFirstRevealAnswer = true }
        let confirmed = try await run(hub, recorder)
        XCTAssertTrue(confirmed)
        let reveals = await hub.reveals
        XCTAssertEqual(reveals.count, 2)
        XCTAssertEqual(reveals[0], reveals[1])
        let compares = await recorder.stages.filter { if case .compare = $0 { return true } else { return false } }
        XCTAssertEqual(compares.count, 1, "der Vergleichscode wird nur einmal berechnet")
    }

    func testChangedNonceAborts() async {
        let hub = FakeHub()
        await hub.set { $0.changeNonceOnRepoll = true }
        await assertFails(hub, .nonceChanged)
    }

    func testRejectedAtDesktop() async {
        let hub = FakeHub()
        await hub.set { $0.rejectAfterReveal = true }
        await assertFails(hub, .rejected)
    }

    func testExpiredCode() async {
        let hub = FakeHub()
        await hub.set { $0.expireAfterReveal = true }
        await assertFails(hub, .expired)
    }

    func testPollErrorsAreRetried() async throws {
        let hub = FakeHub()
        await hub.set { $0.failPolls = 3 }
        let confirmed = try await run(hub)
        XCTAssertTrue(confirmed)
    }

    func testLostClaimIsNotRepeated() async {
        let hub = FakeHub()
        await hub.set { $0.claimFails = true }
        await assertFails(hub, .offline)
        let calls = await hub.calls
        XCTAssertEqual(calls.count, 1)
    }

    func testWrongSessionKeyStopsBeforeAdopt() async {
        let hub = FakeHub()
        await hub.set { $0.wrongQ = true }
        await assertFails(hub, .openFailed)
        let confirms = await hub.confirms
        XCTAssertTrue(confirms.isEmpty)
    }

    func testKeyIdAndUserAreChecked() async {
        let keyHub = FakeHub()
        await keyHub.set { $0.wrongKeyId = true }
        await assertFails(keyHub, .keyMismatch)
        let userHub = FakeHub()
        await userHub.set { $0.wrongUser = true }
        await assertFails(userHub, .keyMismatch)
    }

    func testEmailInSealedMustMatchHub() async {
        let hub = FakeHub()
        await hub.set { $0.fakeEmail = "opfer@example.org" }
        await assertFails(hub, .keyMismatch)
        let same = FakeHub(), recorder = Recorder()
        await same.set { $0.fakeEmail = "P@Example.org" }
        _ = try? await run(same, recorder)
        let adopted = await recorder.adopted
        XCTAssertEqual(adopted.first?.email, "p@example.org")
    }

    func testHubHostMustBePlainAscii() {
        XCTAssertThrowsError(try PairingCode(uri: uri(hub: "https%3A%2F%2Fp%D0%BErtal.wolffappliedai.de"))) { XCTAssertEqual($0 as? PairingError, .insecureHub) }
        XCTAssertThrowsError(try PairingCode(uri: uri(hub: "https%3A%2F%2Fa%2500b.example"))) { XCTAssertEqual($0 as? PairingError, .insecureHub) }
    }

    func testRevealAnswerLostAtHubContinuesByWaiting() async throws {
        let hub = FakeHub()
        await hub.set { $0.hubLostRevealAnswer = true }
        let confirmed = try await run(hub)
        XCTAssertTrue(confirmed)
        let reveals = await hub.reveals
        XCTAssertEqual(reveals.count, 1)
    }

    func testClaimAndRevealRefusals() async {
        for (status, expected) in [(400, PairingError.nameRejected), (429, .tooManyAttempts), (503, .serverFailed)] {
            let hub = FakeHub()
            await hub.set { $0.claimStatus = status }
            await assertFails(hub, expected)
        }
        let hub = FakeHub()
        await hub.set { $0.revealStatus = 409 }
        await assertFails(hub, .commitmentFailed)
    }

    func testLostConfirmAnswerThen410CountsAsConfirmed() async throws {
        let hub = FakeHub()
        await hub.set { $0.confirmAnswerLost = true }
        let confirmed = try await run(hub)
        XCTAssertTrue(confirmed)
    }

    func testDeadlineStopsWhenHubIsGone() async throws {
        let hub = FakeHub()
        await hub.set { $0.failPolls = 1000 }
        let start = Date()
        let tick = Ticker()
        let session = PairingSession(code: try await hub.code, name: "iPhone", transport: hub, pause: { _ in },
                                     now: { start.addingTimeInterval(tick.next()) })
        do {
            _ = try await session.run(stage: { _ in }, adopt: { _ in })
            XCTFail("hätte scheitern müssen")
        } catch { XCTAssertEqual(error as? PairingError, .offline) }
        let polls = await hub.polls
        XCTAssertLessThan(polls, 20)
    }

    func testResultWithoutRevealIsRefused() async {
        let hub = FakeHub()
        await hub.set { $0.skipReveal = true }
        await assertFails(hub, .badResponse)
    }

    func testNoConfirmWhenAccountCouldNotBeStored() async throws {
        let hub = FakeHub()
        do {
            _ = try await run(hub, adoptError: PairingError.adoptFailed("Schlüsselbund"))
            XCTFail("hätte scheitern müssen")
        } catch { XCTAssertEqual(error as? PairingError, .adoptFailed("Schlüsselbund")) }
        let confirms = await hub.confirms
        XCTAssertTrue(confirms.isEmpty)
    }

    func testLostConfirmIsRepeatedThenReportedButAccountStays() async throws {
        let once = FakeHub()
        await once.set { $0.confirmFails = 1 }
        let first = try await run(once)
        XCTAssertTrue(first)
        let always = FakeHub(), recorder = Recorder()
        await always.set { $0.confirmFails = 99 }
        let second = try await run(always, recorder)
        XCTAssertFalse(second)
        let adopted = await recorder.adopted
        XCTAssertEqual(adopted.count, 1)
    }
}
