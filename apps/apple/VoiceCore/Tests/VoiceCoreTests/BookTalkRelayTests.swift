import Foundation
import XCTest
@testable import VoiceCore

// Buch-Gespräch (lva#320, Spec §11): Relay-Typen, Authentifizierung, Fristen, Negativtests (R13), Budgetprofile,
// Überlauf-Wiederholung, recap ohne Modell. Linux-tauglich, nur Foundation.

private let key = [UInt8](repeating: 7, count: 32)
private let device = "dev0123456789abcdef0123456789abcd"

private func enrollment(expires: Int = Int.max / 2, models: [String] = []) -> BookTalkRelayEnrollment {
    BookTalkRelayEnrollment(keyId: "k1", key: key, desktopDeviceId: "desk", desktopName: "Büro-PC", deviceId: device, modelIds: models, expiresAt: expires)
}

private let sonnet = BookTalkRelayModelInfo(id: "sonnet", label: "Claude Sonnet", provider: .claudeCLI, contextTokens: 200_000)

/// Fake-Hub: speichert Objekte; `script` spielt den PC und liefert bei jedem Abruf von `resp_…` den aktuellen Stand.
private actor FakeChannel: BookTalkRelayChannel {
    var objects: [String: Data] = [:]
    var puts: [String] = []
    var removed: [String] = []
    var script: (@Sendable (_ request: BookTalkRelayRequest?, _ poll: Int) -> BookTalkRelayResponse?)?
    private var polls = 0

    func setScript(_ s: @escaping @Sendable (BookTalkRelayRequest?, Int) -> BookTalkRelayResponse?) { script = s }

    func put(objectId: String, json: Data) async throws { objects[objectId] = json; puts.append(objectId) }

    func fetch(objectId: String) async throws -> Data? {
        guard objectId.hasPrefix("resp_") else { return objects[objectId] }
        let reqData = objects["req_" + objectId.dropFirst(5)]
        let request = reqData.flatMap { try? JSONDecoder().decode(BookTalkRelayRequest.self, from: $0) }
        polls += 1
        guard request != nil, let r = script?(request, polls) else { return nil }
        return try? JSONEncoder().encode(r)
    }

    func remove(objectId: String) async throws { objects[objectId] = nil; removed.append(objectId) }
}

/// Uhr, die nur durch `sleep` vorrückt.
private final class FakeClock: @unchecked Sendable {
    private let lock = NSLock()
    private var t = 1_800_000_000.0
    var date: Date { lock.lock(); defer { lock.unlock() }; return Date(timeIntervalSince1970: t) }
    func advance(_ s: Double) { lock.lock(); t += s; lock.unlock() }
}

private func model(_ channel: FakeChannel, clock: FakeClock, authorized: @escaping @Sendable () -> Bool = { true },
                   enroll: BookTalkRelayEnrollment? = nil) -> BookTalkRelayModel {
    BookTalkRelayModel(
        info: sonnet, enrollment: enroll ?? enrollment(), channel: channel, authorized: authorized,
        now: { clock.date }, sleep: { clock.advance($0); try Task.checkCancellation() }, makeId: { "00112233445566778899aabbccddeeff" })
}

private func collect(_ m: BookTalkRelayModel) async -> (pieces: [String], error: Error?) {
    var pieces: [String] = []
    do {
        for try await p in m.stream(instructions: "sys", prompt: "frage", maxTokens: 400) { pieces.append(p) }
        return (pieces, nil)
    } catch { return (pieces, error) }
}

private let rid = "00112233445566778899aabbccddeeff"

private func reply(seq: Int, _ text: String, done: Bool = false, error: BookTalkRelayError? = nil, tuple: String = "claude-sonnet-5-5",
                   device: String = device, request: String = rid, key: [UInt8] = key) -> BookTalkRelayResponse {
    BookTalkRelayResponse.signed(key: key, requestId: request, deviceId: device, seq: seq, modelTuple: tuple, text: text, done: done, error: error)
}

final class BookTalkRelayTests: XCTestCase {
    // MARK: Hilfen

    func testSha256AndHmacKnownAnswers() {
        XCTAssertEqual(BookTalkHash.sha256Hex("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        XCTAssertEqual(BookTalkHash.sha256Hex(""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        let mac = BookTalkHash.hmacSHA256(key: Array("Jefe".utf8), message: Array("what do ya want for nothing?".utf8))
        XCTAssertEqual(BookTalkHash.hex(mac), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843")
        XCTAssertTrue(BookTalkHash.constantTimeEqual("ab", "ab"))
        XCTAssertFalse(BookTalkHash.constantTimeEqual("ab", "ac"))
    }

    func testObjectIdsFitHubPattern() {
        let id = BookTalkRelay.objectId(.req, rid)
        XCTAssertEqual(id, "req_" + rid)
        XCTAssertNotNil(id.range(of: "^[A-Za-z0-9_-]{1,64}$", options: .regularExpression))
    }

    // MARK: Typen

    func testRequestEncodesSpecFieldsAndRoundTrips() throws {
        let r = BookTalkRelayRequest.make(key: key, keyId: "k1", deviceId: device, requestId: rid, modelId: "sonnet", system: "S", prompt: "P",
                                          maxTokens: 5000, now: 1_800_000_000)
        XCTAssertEqual(r.maxTokens, 800)
        XCTAssertEqual(r.expiresAt, r.createdAt + 90)
        let json = try JSONSerialization.jsonObject(with: JSONEncoder().encode(r)) as! [String: Any]
        for k in ["v", "request_id", "device_id", "key_id", "function", "model_id", "system", "prompt", "max_tokens", "created_at", "expires_at", "mac"] {
            XCTAssertNotNil(json[k], k)
        }
        XCTAssertEqual(json["function"] as? String, "book_talk")
        XCTAssertEqual(try JSONDecoder().decode(BookTalkRelayRequest.self, from: JSONEncoder().encode(r)), r)
    }

    func testCanonicalMetadataIsStable() {
        let c = BookTalkRelayRequest.canonical(requestId: "r", deviceId: "d", keyId: "k", function: "book_talk", modelId: "m", system: "", prompt: "",
                                               maxTokens: 10, createdAt: 100, expiresAt: 190)
        let empty = BookTalkHash.sha256Hex("")
        XCTAssertEqual(c, "{\"created_at\":100,\"device_id\":\"d\",\"expires_at\":190,\"function\":\"book_talk\",\"key_id\":\"k\",\"max_tokens\":10,"
            + "\"model_id\":\"m\",\"prompt_sha256\":\"\(empty)\",\"request_id\":\"r\",\"system_sha256\":\"\(empty)\",\"v\":1}")
    }

    func testResponseDecodesSpecFieldsWithOptionalUsage() throws {
        let r = reply(seq: 3, "Text", done: true)
        let data = try JSONEncoder().encode(r)
        let json = try JSONSerialization.jsonObject(with: data) as! [String: Any]
        XCTAssertEqual(json["model_tuple"] as? String, "claude-sonnet-5-5")
        XCTAssertEqual(try JSONDecoder().decode(BookTalkRelayResponse.self, from: data), r)
        let raw = Data("""
        {"v":1,"request_id":"a","device_id":"b","seq":1,"model_tuple":"x","text":"t","done":false,"error":"llm_failed","usage":{"input_tokens":5},"mac":"00"}
        """.utf8)
        let d = try JSONDecoder().decode(BookTalkRelayResponse.self, from: raw)
        XCTAssertEqual(d.error, .llmFailed)
        XCTAssertEqual(d.usage?.inputTokens, 5)
    }

    func testModelListAndEnrollmentDecode() throws {
        let raw = Data("""
        {"v":1,"models":[{"id":"sonnet","label":"Claude Sonnet","provider":"claude_cli","context_tokens":200000},
        {"id":"gpt-5","label":"Codex","provider":"codex_cli","context_tokens":128000}],"updated_at":1800000000}
        """.utf8)
        let list = try JSONDecoder().decode(BookTalkRelayModelList.self, from: raw)
        XCTAssertEqual(list.models.map(\.provider), [.claudeCLI, .codexCLI])
        XCTAssertEqual(list.models[0].contextTokens, 200_000)
        let e = enrollment(models: ["sonnet"])
        let text = "lvrelay1:" + String(decoding: try JSONEncoder().encode(e), as: UTF8.self)
        XCTAssertEqual(BookTalkRelayEnrollment.parse(text), e)
        XCTAssertEqual(e.allowed(list.models).map(\.id), ["sonnet"])
        XCTAssertNil(BookTalkRelayEnrollment.parse("{\"v\":1}"))
    }

    // MARK: Negativtests R13

    func testPcSideVerifyRejectsWrongMacDeviceExpiryAndTamper() {
        let now = 1_800_000_000
        let r = BookTalkRelayRequest.make(key: key, keyId: "k1", deviceId: device, requestId: rid, modelId: "sonnet", system: "S", prompt: "P",
                                          maxTokens: 400, now: now)
        XCTAssertTrue(r.verify(key: key, deviceId: device, now: now))
        XCTAssertTrue(r.verify(key: key, deviceId: device, now: now + 60))
        XCTAssertFalse(r.verify(key: [UInt8](repeating: 8, count: 32), deviceId: device, now: now), "falscher Schlüssel")
        XCTAssertFalse(r.verify(key: key, deviceId: "fremd", now: now), "fremdes Gerät")
        XCTAssertFalse(r.verify(key: key, deviceId: device, now: now + 90), "abgelaufen")
        XCTAssertFalse(r.verify(key: key, deviceId: device, now: now - 200), "Uhr weicht zu weit ab")
        let tampered = BookTalkRelayRequest(v: 1, requestId: rid, deviceId: device, keyId: "k1", function: "book_talk", modelId: "sonnet",
                                            system: "S", prompt: "ANDERS", maxTokens: 400, createdAt: r.createdAt, expiresAt: r.expiresAt, mac: r.mac)
        XCTAssertFalse(tampered.verify(key: key, deviceId: device, now: now), "Prompt verändert")
        let other = BookTalkRelayRequest(v: 1, requestId: rid, deviceId: device, keyId: "k1", function: "book_talk", modelId: "opus",
                                         system: "S", prompt: "P", maxTokens: 400, createdAt: r.createdAt, expiresAt: r.expiresAt, mac: r.mac)
        XCTAssertFalse(other.verify(key: key, deviceId: device, now: now), "anderes Modell")
    }

    func testStreamsOnlyNewTextAndCleansUp() async {
        let channel = FakeChannel(), clock = FakeClock()
        await channel.setScript { _, n in
            switch n {
            case 1: reply(seq: 0, "Hallo ")
            case 2: reply(seq: 1, "Hallo Welt")
            default: reply(seq: 2, "Hallo Welt!", done: true)
            }
        }
        let m = model(channel, clock: clock)
        let r = await collect(m)
        XCTAssertNil(r.error)
        XCTAssertEqual(r.pieces.joined(), "Hallo Welt!")
        XCTAssertEqual(r.pieces, ["Hallo ", "Welt", "!"])
        XCTAssertEqual(m.actualModel, "claude-sonnet-5-5")
        let removed = await channel.removed
        XCTAssertTrue(removed.contains("req_" + rid) && removed.contains("resp_" + rid))
        let puts = await channel.puts
        XCTAssertEqual(puts, ["req_" + rid])
    }

    func testRewrittenTextDeliversFromCommonPrefix() async {
        let channel = FakeChannel(), clock = FakeClock()
        await channel.setScript { _, n in n == 1 ? reply(seq: 0, "Der Hund") : reply(seq: 1, "Der Hase springt", done: true) }
        let r = await collect(model(channel, clock: clock))
        XCTAssertEqual(r.pieces, ["Der Hund", "ase springt"])
    }

    func testWrongMacIsIgnoredAndEndsOffline() async {
        let channel = FakeChannel(), clock = FakeClock()
        await channel.setScript { _, _ in reply(seq: 0, "Gefälscht", done: true, key: [UInt8](repeating: 9, count: 32)) }
        let r = await collect(model(channel, clock: clock))
        XCTAssertTrue(r.pieces.isEmpty)
        XCTAssertEqual(r.error as? BookTalkModelError, .offline)
        let removed = await channel.removed
        XCTAssertTrue(removed.contains("req_" + rid))
    }

    func testForeignDeviceOrRequestIsIgnored() async {
        let channel = FakeChannel(), clock = FakeClock()
        await channel.setScript { _, n in n % 2 == 0 ? reply(seq: n, "x", device: "fremd") : reply(seq: n, "y", request: "ffffffffffffffffffffffffffffffff") }
        let r = await collect(model(channel, clock: clock))
        XCTAssertTrue(r.pieces.isEmpty)
        XCTAssertEqual(r.error as? BookTalkModelError, .offline)
    }

    func testReplayedOrNonIncreasingSeqIsIgnored() async {
        let channel = FakeChannel(), clock = FakeClock()
        await channel.setScript { _, n in
            switch n {
            case 1: reply(seq: 5, "Eins")
            case 2: reply(seq: 5, "Eins Zwei")
            case 3: reply(seq: 4, "Eins Drei")
            default: reply(seq: 6, "Eins fertig", done: true)
            }
        }
        let r = await collect(model(channel, clock: clock))
        XCTAssertNil(r.error)
        XCTAssertEqual(r.pieces, ["Eins", "Eins fertig".dropFirst(4).description])
    }

    func testWrongModelTupleIsRejected() async {
        let channel = FakeChannel(), clock = FakeClock()
        await channel.setScript { _, _ in reply(seq: 0, "Antwort", done: true, tuple: "gpt-4o") }
        let r = await collect(model(channel, clock: clock))
        XCTAssertTrue(r.pieces.isEmpty)
        XCTAssertEqual(r.error as? BookTalkModelError, .unavailable)
    }

    func testSwitchingModelTupleMidStreamIsRejected() async {
        let channel = FakeChannel(), clock = FakeClock()
        await channel.setScript { _, n in
            n == 1 ? reply(seq: 0, "A") : reply(seq: 1, "AB", tuple: "claude-sonnet-4")
        }
        let r = await collect(model(channel, clock: clock))
        XCTAssertEqual(r.pieces, ["A"])
        XCTAssertEqual(r.error as? BookTalkModelError, .unavailable, "R7: anderes Modell nach dem Pinning verwirft die Antwort")
    }

    func testNoFirstResponseWithin15SecondsIsOffline() async {
        let channel = FakeChannel(), clock = FakeClock()
        let r = await collect(model(channel, clock: clock))
        XCTAssertEqual(r.error as? BookTalkModelError, .offline)
        XCTAssertGreaterThanOrEqual(clock.date.timeIntervalSince1970 - 1_800_000_000, 15)
        XCTAssertLessThan(clock.date.timeIntervalSince1970 - 1_800_000_000, 17)
        let removed = await channel.removed
        XCTAssertTrue(removed.contains("req_" + rid))
    }

    func testTotalWaitIsCappedAt75Seconds() async {
        let channel = FakeChannel(), clock = FakeClock()
        await channel.setScript { _, n in reply(seq: n, "Anfang") }
        let r = await collect(model(channel, clock: clock))
        XCTAssertEqual(r.error as? BookTalkModelError, .timeout)
        let spent = clock.date.timeIntervalSince1970 - 1_800_000_000
        XCTAssertGreaterThanOrEqual(spent, 75)
        XCTAssertLessThan(spent, 77)
    }

    func testRevocationDuringRunCancelsAndDeletesRequest() async {
        let channel = FakeChannel(), clock = FakeClock()
        let flag = Flag()
        await channel.setScript { _, n in
            if n == 2 { flag.set(false) }
            return reply(seq: n, "Text \(n)")
        }
        let r = await collect(model(channel, clock: clock, authorized: { flag.value }))
        XCTAssertTrue(r.error is CancellationError, "\(String(describing: r.error))")
        let removed = await channel.removed
        XCTAssertTrue(removed.contains("req_" + rid))
    }

    func testNoRequestIsSentWithoutConsentOrWithExpiredEnrollment() async {
        let channel = FakeChannel(), clock = FakeClock()
        let a = await collect(model(channel, clock: clock, authorized: { false }))
        XCTAssertEqual(a.error as? BookTalkModelError, .notAllowed)
        let b = await collect(model(channel, clock: clock, enroll: enrollment(expires: 1_700_000_000)))
        XCTAssertEqual(b.error as? BookTalkModelError, .notAllowed)
        let puts = await channel.puts
        XCTAssertTrue(puts.isEmpty)
    }

    func testPcErrorsMapToModelErrors() async {
        for (error, expected) in [(BookTalkRelayError.notAllowed, BookTalkModelError.notAllowed), (.timeout, .timeout), (.modelUnavailable, .unavailable)] {
            let channel = FakeChannel(), clock = FakeClock()
            await channel.setScript { _, _ in reply(seq: 0, "", done: true, error: error) }
            let r = await collect(model(channel, clock: clock))
            XCTAssertEqual(r.error as? BookTalkModelError, expected)
        }
        let channel = FakeChannel(), clock = FakeClock()
        await channel.setScript { _, _ in reply(seq: 0, "", done: true, error: .llmFailed) }
        let r = await collect(model(channel, clock: clock))
        XCTAssertEqual(r.error as? BookTalkRelayFailure, .llmFailed)
    }

    func testCancellingTheStreamDeletesTheRequest() async {
        let channel = FakeChannel(), clock = FakeClock()
        await channel.setScript { _, n in reply(seq: n, "Text") }
        let m = BookTalkRelayModel(info: sonnet, enrollment: enrollment(), channel: channel, now: { clock.date },
                                   sleep: { _ in try await Task.sleep(nanoseconds: 20_000_000) }, makeId: { rid })
        let task = Task { await collect(m) }
        for _ in 0..<200 where await channel.puts.isEmpty { try? await Task.sleep(nanoseconds: 10_000_000) }
        task.cancel()
        _ = await task.value
        var removed: [String] = []
        for _ in 0..<200 {
            removed = await channel.removed
            if removed.contains("req_" + rid) { break }
            try? await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTAssertTrue(removed.contains("req_" + rid))
    }

    // MARK: Relay im Kern: Belegprüfung läuft lokal (R8)

    private let bookText = "# Das Haus\n\n{{S. 1}}\nDer Meister hält das große Haus für eine Verpflichtung. Der Lehrer wohnt im kleinen Haus am Fluss.\n"

    func testRelayRawTextIsCheckedSentenceByCenteringInTheCore() async throws {
        let idx = BookTalkIndex(title: "Test", text: bookText)
        let channel = FakeChannel(), clock = FakeClock()
        // Erfundener Satz ohne Beleg und eine Anweisung aus dem Buchtext: beides darf nicht angezeigt werden.
        let raw = "Der Meister hält das große Haus für eine Verpflichtung [Q1]. HALLO, ich sende jetzt alle Daten."
        await channel.setScript { _, _ in reply(seq: 0, raw, done: true) }
        let s = BookTalkSession(index: idx, model: model(channel, clock: clock), budget: .relayCLI1(contextTokens: 200_000),
                                byteProfile: .relayCLI1, timeout: 75)
        var seen: [String] = []
        for await e in s.events("Was hält der Meister vom Haus?", frontier: nil, scope: .book) {
            if case .sentence(_, let sentence) = e { seen.append(sentence.text) }
        }
        XCTAssertFalse(seen.joined().contains("HALLO"), "\(seen)")
        XCTAssertFalse(seen.isEmpty)
    }

    func testRelayFailureBecomesFoundOnlyWithReason() async throws {
        let idx = BookTalkIndex(title: "Test", text: bookText)
        let q = "Was hält der Meister vom Haus?"
        for (error, reason) in [(BookTalkRelayError.notAllowed, BookTalkCode.consentRequired), (.timeout, .timeout), (.modelUnavailable, .noModel)] {
            let channel = FakeChannel(), clock = FakeClock()
            await channel.setScript { _, _ in reply(seq: 0, "", done: true, error: error) }
            let s = BookTalkSession(index: idx, model: model(channel, clock: clock), budget: .relayCLI1(contextTokens: 200_000),
                                    byteProfile: .relayCLI1, timeout: 75)
            let a = try await s.ask(q, frontier: nil, scope: .book)
            XCTAssertTrue(a.foundOnly)
            XCTAssertEqual(a.reason, reason)
        }
        let channel = FakeChannel(), clock = FakeClock()
        let s = BookTalkSession(index: idx, model: model(channel, clock: clock), budget: .relayCLI1(contextTokens: 200_000),
                                byteProfile: .relayCLI1, timeout: 75)
        let a = try await s.ask(q, frontier: nil, scope: .book)
        XCTAssertEqual(a.reason, .offline)
    }

    // MARK: Eval-Status

    func testEvalStatusIsThreeValuedAndAppleIsAlwaysUnchecked() {
        let relay = BookTalkEvalTuple(provider: "claude_cli", model: "claude-sonnet-5-5", budget: "relay-cli@1")
        let apple = BookTalkEvalTuple(provider: "apple", model: "system", budget: "apple-fm@1")
        let empty = BookTalkEvalTable()
        XCTAssertEqual(empty.status(for: relay), .unchecked)
        let table = BookTalkEvalTable(entries: [
            .init(tuple: relay, status: .passed, reportSha256: "a"), .init(tuple: apple, status: .passed, reportSha256: "b"),
        ])
        XCTAssertEqual(table.status(for: relay), .passed)
        XCTAssertEqual(table.status(for: apple), .unchecked, "R12")
        XCTAssertEqual(table.status(for: BookTalkEvalTuple(provider: "claude_cli", model: "claude-sonnet-5-6", budget: "relay-cli@1")), .unchecked)
        let both = BookTalkEvalTable(entries: [.init(tuple: relay, status: .passed, reportSha256: "a"), .init(tuple: relay, status: .failed, reportSha256: "c")])
        XCTAssertEqual(both.status(for: relay), .failed)
        XCTAssertEqual(BookTalkEvalTable.load(nil), BookTalkEvalTable())
        XCTAssertEqual(BookTalkEvalTable.load(Data("kaputt".utf8)), BookTalkEvalTable())
    }

    // MARK: Budgetprofile (§11.1, §11.3)

    func testAppleProfileBudgetArithmetic() {
        let b = BookTalkBudget.appleFM1, p = BookTalkByteProfile.appleFM1
        XCTAssertEqual(b.contextTokens, 4_096)
        XCTAssertEqual(b.answerTokens, 400)
        XCTAssertEqual(BookTalkBudget.promptTokens(instructions: String(repeating: "a", count: 100), prompt: String(repeating: "b", count: 200),
                                                   counter: nil, profile: p), 900)
        XCTAssertTrue(b.fits(instructions: String(repeating: "a", count: 1_000), prompt: String(repeating: "b", count: 2_096), counter: nil, profile: p))
        XCTAssertFalse(b.fits(instructions: String(repeating: "a", count: 1_000), prompt: String(repeating: "b", count: 2_097), counter: nil, profile: p))
    }

    func testRelayProfileUsesReportedContext() {
        let b = BookTalkBudget.relayCLI1(contextTokens: 10_000), p = BookTalkByteProfile.relayCLI1
        XCTAssertEqual(b.name, "relay-cli@1")
        XCTAssertEqual(b.excerptChars, 20_000)
        XCTAssertTrue(b.fits(instructions: "", prompt: String(repeating: "x", count: 7_000), counter: nil, profile: p))
        XCTAssertFalse(b.fits(instructions: "", prompt: String(repeating: "x", count: 7_300), counter: nil, profile: p))
    }

    // MARK: Überlauf-Wiederholung (§11.1)

    private func longBook() -> String {
        var s = "# Das Haus\n\n"
        for n in 1...12 {
            s += "{{S. \(n)}}\nDer Meister spricht über das große Haus im Abschnitt \(n). " + String(repeating: "Das Haus des Lehrers steht am Fluss und ist alt. ", count: 14) + "\n\n"
        }
        return s
    }

    private final class Overflowing: BookTalkModel, @unchecked Sendable {
        private let lock = NSLock()
        private(set) var prompts: [String] = []
        let alwaysOverflow: Bool
        init(always: Bool) { alwaysOverflow = always }
        func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
            lock.lock(); prompts.append(prompt); let n = prompts.count; lock.unlock()
            return AsyncThrowingStream { c in
                if alwaysOverflow || n == 1 { c.finish(throwing: BookTalkModelError.contextOverflow) } else {
                    c.yield("Der Meister spricht über das große Haus [Q1].")
                    c.finish()
                }
            }
        }
    }

    func testOverflowRetriesOnceWithHalfTheExcerptBudget() async throws {
        let idx = BookTalkIndex(title: "T", text: longBook())
        let budget = BookTalkBudget(name: "t", instructionChars: 1_200, historyChars: 1_400, questionChars: 520, excerptChars: 7_000,
                                    answerTokens: 400, contextTokens: 200_000)
        let m = Overflowing(always: false)
        let s = BookTalkSession(index: idx, model: m, budget: budget, byteProfile: .appleFM1)
        let a = try await s.ask("Was sagt der Meister über das große Haus?", frontier: nil, scope: .book)
        XCTAssertEqual(m.prompts.count, 2)
        XCTAssertLessThan(m.prompts[1].utf16.count, m.prompts[0].utf16.count * 3 / 4, "zweiter Prompt hat deutlich weniger Auszüge")
        XCTAssertNotEqual(a.reason, .budget)
    }

    func testSecondOverflowEndsAsFoundOnlyBudgetAfterExactlyOneRetry() async throws {
        let idx = BookTalkIndex(title: "T", text: longBook())
        let budget = BookTalkBudget(name: "t", instructionChars: 1_200, historyChars: 1_400, questionChars: 520, excerptChars: 7_000,
                                    answerTokens: 400, contextTokens: 200_000)
        let m = Overflowing(always: true)
        let s = BookTalkSession(index: idx, model: m, budget: budget, byteProfile: .appleFM1)
        let a = try await s.ask("Was sagt der Meister über das große Haus?", frontier: nil, scope: .book)
        XCTAssertEqual(m.prompts.count, 2)
        XCTAssertTrue(a.foundOnly)
        XCTAssertEqual(a.reason, .budget)
    }

    // MARK: recap ohne Modell (§11.6)

    func testRecapWithoutModelListsLatestPassagesWithChapterTitles() async throws {
        var text = ""
        for (i, title) in ["Anfang", "Reise", "Ankunft"].enumerated() {
            text += "# \(title)\n\n{{S. \(i * 2 + 1)}}\nIm Kapitel \(title) geschieht etwas Wichtiges mit dem Meister. Danach folgt ein weiterer Satz zum Kapitel \(title).\n\n"
        }
        let idx = BookTalkIndex(title: "T", text: text)
        let s = BookTalkSession(index: idx, model: nil, budget: .appleFM1)
        let a = try await s.ask("Um was ging es bisher?", frontier: nil, scope: .book)
        XCTAssertTrue(a.foundOnly)
        XCTAssertEqual(a.intent, .recap)
        XCTAssertTrue(a.text.contains("Kapitel „"), a.text)
        XCTAssertTrue(a.violations(index: idx).isEmpty)
        XCTAssertGreaterThanOrEqual(a.sentences.count, 1)
    }
}

// MARK: - Fixrunde: Antwortsemantik, Modellabgleich, Liste, Einrichtungscode, Aufräumen

private func evalModel(_ channel: FakeChannel, clock: FakeClock, alias: BookTalkRelayModelInfo = sonnet,
                       status: @escaping @Sendable (String) -> BookTalkEvalStatus) -> BookTalkRelayModel {
    BookTalkRelayModel(
        info: alias, enrollment: enrollment(), channel: channel, evalStatus: status,
        now: { clock.date }, sleep: { clock.advance($0); try Task.checkCancellation() }, makeId: { rid })
}

private final class Seen: @unchecked Sendable {
    private let lock = NSLock()
    private var items: [String] = []
    var value: [String] { lock.lock(); defer { lock.unlock() }; return items }
    func add(_ x: String) { lock.lock(); items.append(x); lock.unlock() }
}

final class BookTalkRelayFixTests: XCTestCase {
    private let bookText = "# Das Haus\n\n{{S. 1}}\nDer Meister hält das große Haus für eine Verpflichtung. Der Lehrer wohnt im kleinen Haus am Fluss.\n"

    // MARK: Antwortsemantik (Befund 3)

    func testPendingIsLifelineThatResetsTheFifteenSecondFrameWithoutPinning() async {
        let channel = FakeChannel(), clock = FakeClock()
        await channel.setScript { _, n in
            n <= 24 ? reply(seq: n, n == 3 ? "Vorab" : "", tuple: "pending") : reply(seq: n, "Antwort", done: true, tuple: "claude-sonnet-4-5-20250929")
        }
        let m = model(channel, clock: clock)
        let r = await collect(m)
        XCTAssertNil(r.error, "\(String(describing: r.error))")
        XCTAssertEqual(r.pieces, ["Antwort"], "Text vor dem Tupel wird nicht angezeigt")
        XCTAssertEqual(m.actualModel, "claude-sonnet-4-5-20250929", "gepinnt wird erst das echte Tupel")
        let spent = clock.date.timeIntervalSince1970 - 1_800_000_000
        XCTAssertGreaterThan(spent, 15, "länger als die erste 15-s-Frist")
        XCTAssertLessThan(spent, 75)
    }

    func testPendingForeverStillEndsAtTheTotalFrame() async {
        let channel = FakeChannel(), clock = FakeClock()
        await channel.setScript { _, n in reply(seq: n, "", tuple: "pending") }
        let r = await collect(model(channel, clock: clock))
        XCTAssertEqual(r.error as? BookTalkModelError, .timeout)
        let spent = clock.date.timeIntervalSince1970 - 1_800_000_000
        XCTAssertGreaterThanOrEqual(spent, 75)
        XCTAssertLessThan(spent, 77)
    }

    func testPendingThatStopsEndsOfflineFifteenSecondsAfterTheLastSign() async {
        let channel = FakeChannel(), clock = FakeClock()
        await channel.setScript { _, n in n <= 5 ? reply(seq: n, "", tuple: "pending") : nil }
        let r = await collect(model(channel, clock: clock))
        XCTAssertEqual(r.error as? BookTalkModelError, .offline)
        let spent = clock.date.timeIntervalSince1970 - 1_800_000_000
        XCTAssertGreaterThanOrEqual(spent, 19)
        XCTAssertLessThan(spent, 21)
    }

    func testPendingWithInvalidMacOrOldSeqIsNoSign() async {
        let channel = FakeChannel(), clock = FakeClock()
        await channel.setScript { _, n in
            n % 2 == 0 ? reply(seq: n, "", tuple: "pending", key: [UInt8](repeating: 9, count: 32)) : reply(seq: 0, "", tuple: "pending")
        }
        let r = await collect(model(channel, clock: clock))
        XCTAssertEqual(r.error as? BookTalkModelError, .offline)
    }

    func testPcErrorCodesBecomeOwnFailCodes() async throws {
        let idx = BookTalkIndex(title: "Test", text: bookText)
        let q = "Was hält der Meister vom Haus?"
        let cases: [(BookTalkRelayError, BookTalkCode)] = [
            (.notAllowed, .consentRequired), (.limit, .llmFailed), (.modelUnavailable, .noModel), (.clock, .timeout),
            (.timeout, .timeout), (.llmFailed, .llmFailed), (BookTalkRelayError(rawValue: "neu_vom_pc"), .llmFailed),
        ]
        for (error, code) in cases {
            let channel = FakeChannel(), clock = FakeClock()
            await channel.setScript { _, _ in reply(seq: 1, "", done: true, error: error, tuple: "unknown") }
            let m = model(channel, clock: clock)
            let s = BookTalkSession(index: idx, model: m, budget: .relayCLI1(contextTokens: 200_000), byteProfile: .relayCLI1, timeout: 75)
            let a = try await s.ask(q, frontier: nil, scope: .book)
            XCTAssertTrue(a.foundOnly, error.rawValue)
            XCTAssertEqual(a.reason, code, error.rawValue)
            XCTAssertNil(m.actualModel, "Fehler mit unknown pinnt nichts")
        }
    }

    func testUnknownErrorCodeKeepsTheMacCheckable() throws {
        let r = BookTalkRelayResponse.signed(key: key, requestId: rid, deviceId: device, seq: 1, modelTuple: "unknown", text: "", done: true,
                                             error: BookTalkRelayError(rawValue: "quota_x"))
        let back = try JSONDecoder().decode(BookTalkRelayResponse.self, from: JSONEncoder().encode(r))
        XCTAssertEqual(back.error?.rawValue, "quota_x")
        XCTAssertTrue(back.macIsValid(key: key))
    }

    func testUnknownTupleIsAllowedOnlyAsFinalAnswerAndStaysUnchecked() async {
        let channel = FakeChannel(), clock = FakeClock()
        await channel.setScript { _, n in n == 1 ? reply(seq: 0, "Zwischen", tuple: "pending") : reply(seq: 1, "Fertig", done: true, tuple: "unknown") }
        let m = evalModel(channel, clock: clock) { _ in .passed }
        let r = await collect(m)
        XCTAssertNil(r.error)
        XCTAssertEqual(r.pieces, ["Fertig"])
        XCTAssertEqual(m.actualModel, "unknown")
        XCTAssertEqual(m.actualEvalStatus, .unchecked, "unknown gilt als ungeprüft, auch wenn die Tabelle alles bestehen ließe")

        let channel2 = FakeChannel(), clock2 = FakeClock()
        await channel2.setScript { _, _ in reply(seq: 0, "Teil", done: false, tuple: "unknown") }
        let r2 = await collect(model(channel2, clock: clock2))
        XCTAssertTrue(r2.pieces.isEmpty)
        XCTAssertEqual(r2.error as? BookTalkModelError, .unavailable, "unknown nur als finale Antwort")

        let channel3 = FakeChannel(), clock3 = FakeClock()
        await channel3.setScript { _, n in n == 1 ? reply(seq: 0, "A") : reply(seq: 1, "AB", done: true, tuple: "unknown") }
        let r3 = await collect(model(channel3, clock: clock3))
        XCTAssertEqual(r3.error as? BookTalkModelError, .unavailable, "nach dem Pinning kein Wechsel auf unknown")
    }

    // MARK: Modellabgleich und Eval nach dem tatsächlichen Modell (Befund 6)

    func testEvalStatusComesFromTheActualModelNotTheAlias() async {
        let seen = Seen()
        let channel = FakeChannel(), clock = FakeClock()
        await channel.setScript { _, _ in reply(seq: 0, "Antwort", done: true, tuple: "claude-sonnet-5-5") }
        let m = evalModel(channel, clock: clock) { tuple in seen.add(tuple); return tuple == "claude-sonnet-5-5" ? .passed : .unchecked }
        XCTAssertEqual(m.actualEvalStatus, .unchecked, "vor der Antwort ungeprüft")
        let r = await collect(m)
        XCTAssertNil(r.error)
        XCTAssertEqual(seen.value, ["claude-sonnet-5-5"], "Abfrage mit dem Tupel der Antwort, nie mit `sonnet`")
        XCTAssertEqual(m.actualEvalStatus, .passed)
        XCTAssertFalse(m.rejectedByEval)
    }

    func testFailedActualModelDiscardsTheAnswerAsFoundOnly() async throws {
        let channel = FakeChannel(), clock = FakeClock()
        await channel.setScript { _, _ in reply(seq: 0, "Der Meister hält das große Haus für eine Verpflichtung [Q1].", done: true, tuple: "claude-sonnet-5-5") }
        let m = evalModel(channel, clock: clock) { $0 == "claude-sonnet-5-5" ? .failed : .passed }
        let idx = BookTalkIndex(title: "Test", text: bookText)
        let s = BookTalkSession(index: idx, model: m, budget: .relayCLI1(contextTokens: 200_000), byteProfile: .relayCLI1, timeout: 75)
        let a = try await s.ask("Was hält der Meister vom Haus?", frontier: nil, scope: .book)
        XCTAssertTrue(a.foundOnly)
        XCTAssertEqual(a.reason, .noModel)
        XCTAssertTrue(m.rejectedByEval)
        XCTAssertEqual(m.actualEvalStatus, .failed)
        let removed = await channel.removed
        XCTAssertTrue(removed.contains("req_" + rid))
    }

    func testAliasFamilyMatching() {
        let yes: [(String, String)] = [
            ("sonnet", "sonnet"), ("sonnet", "claude-sonnet-5-5"), ("sonnet", "claude-sonnet-4-5-20250929"), ("sonnet", "claude-sonnet-5"),
            ("sonnet", "Claude-Sonnet-5-5"), ("opus", "claude-opus-4-1"), ("haiku", "claude-haiku-4-5-20251001"),
            ("gpt-6.1-sol", "gpt-6.1-sol"), ("gpt-6.1-sol", "gpt-6.1-sol-2026-11-01"), ("gpt-6.1-sol", "gpt-6.1-sol-20261101"),
            ("claude-sonnet-4-5", "claude-sonnet-4-5-20250929"),
        ]
        let no: [(String, String)] = [
            ("sonnet", "claude-opus-4-1"), ("sonnet", "claude-sonnet-5-5-mini"), ("sonnet", "claude-sonnet-x"), ("sonnet", "claude-sonnet-"),
            ("sonnet", "claude-sonnet-5-"), ("sonnet", "my-sonnet-5"), ("sonnet", "xsonnet"), ("sonnet", "claude-sonnet-5-5 mini"),
            ("sonnet", "unknown"), ("sonnet", "pending"), ("sonnet", ""), ("opus", "claude-sonnet-5-5"), ("haiku", "claude-haiku"),
            ("gpt-6.1-sol", "gpt-6.1-sol-mini"), ("gpt-6.1-sol", "gpt-6.1-solar"), ("gpt-6.1-sol", "gpt-6.1"),
            ("gpt-6.1-sol", "gpt-6.1-sol-mini-2026"), ("gpt-6.1-sol", "gpt-6.1-sol-2026-mini"), ("gpt-6.1-sol", "x-gpt-6.1-sol"),
            ("claude-sonnet-4-5", "claude-sonnet-4-5-latest"),
        ]
        for (alias, tuple) in yes { XCTAssertTrue(BookTalkRelayModelMatch.accepts(alias: alias, tuple: tuple), "\(alias) ↔ \(tuple)") }
        for (alias, tuple) in no { XCTAssertFalse(BookTalkRelayModelMatch.accepts(alias: alias, tuple: tuple), "\(alias) ↔ \(tuple)") }
    }

    func testSubstringOfAnotherFamilyNoLongerPassesTheModelCheck() async {
        let channel = FakeChannel(), clock = FakeClock()
        await channel.setScript { _, _ in reply(seq: 0, "x", done: true, tuple: "claude-sonnet-5-5-mini-opus-sonnet") }
        let r = await collect(model(channel, clock: clock))
        XCTAssertEqual(r.error as? BookTalkModelError, .unavailable)
    }

    // MARK: Signierte Modellliste (Befund 12)

    private let models = [
        BookTalkRelayModelInfo(id: "sonnet", label: "Claude Sonnet", provider: .claudeCLI, contextTokens: 200_000),
        BookTalkRelayModelInfo(id: "gpt-6.1-sol", label: "Codex „Sol“", provider: .codexCLI, contextTokens: 128_000),
    ]

    func testModelListCanonicalSortsEntriesAndFixesTheFieldOrder() {
        let c = BookTalkRelayModelList.canonical(models: models, desktopDeviceId: "desk", deviceId: device, issuedAt: 1_800_000_000)
        XCTAssertEqual(c, "{\"desktop_device_id\":\"desk\",\"device_id\":\"\(device)\",\"issued_at\":1800000000,\"models\":["
            + "{\"context_tokens\":128000,\"id\":\"gpt-6.1-sol\",\"label\":\"Codex „Sol“\",\"provider\":\"codex_cli\"},"
            + "{\"context_tokens\":200000,\"id\":\"sonnet\",\"label\":\"Claude Sonnet\",\"provider\":\"claude_cli\"}],\"v\":1}")
        XCTAssertEqual(c, BookTalkRelayModelList.canonical(models: models.reversed(), desktopDeviceId: "desk", deviceId: device, issuedAt: 1_800_000_000))
    }

    func testSignedModelListIsAuthenticOnlyForThisPcDeviceAndKey() throws {
        let list = BookTalkRelayModelList.signed(key: key, models: models, desktopDeviceId: "desk", deviceId: device, issuedAt: 1_800_000_000)
        XCTAssertTrue(list.isAuthentic(key: key, desktopDeviceId: "desk", deviceId: device))
        let back = try JSONDecoder().decode(BookTalkRelayModelList.self, from: JSONEncoder().encode(list))
        XCTAssertTrue(back.isAuthentic(key: key, desktopDeviceId: "desk", deviceId: device))
        XCTAssertFalse(list.isAuthentic(key: [UInt8](repeating: 1, count: 32), desktopDeviceId: "desk", deviceId: device), "falscher Schlüssel")
        XCTAssertFalse(list.isAuthentic(key: key, desktopDeviceId: "anderer-pc", deviceId: device), "anderer PC")
        XCTAssertFalse(list.isAuthentic(key: key, desktopDeviceId: "desk", deviceId: "anderes-iphone"), "anderes Gerät")
    }

    func testUnsignedOrTamperedModelListIsIgnored() throws {
        let raw = Data("""
        {"v":1,"models":[{"id":"sonnet","label":"Claude Sonnet","provider":"claude_cli","context_tokens":200000}],"updated_at":1800000000}
        """.utf8)
        let unsigned = try JSONDecoder().decode(BookTalkRelayModelList.self, from: raw)
        XCTAssertFalse(unsigned.isAuthentic(key: key, desktopDeviceId: "desk", deviceId: device), "unsigniert")
        let good = BookTalkRelayModelList.signed(key: key, models: models, desktopDeviceId: "desk", deviceId: device, issuedAt: 1_800_000_000)
        var bigger = models
        bigger[0] = BookTalkRelayModelInfo(id: "sonnet", label: "Claude Sonnet", provider: .claudeCLI, contextTokens: 999_999)
        let tamperedModels = BookTalkRelayModelList(models: bigger, desktopDeviceId: "desk", deviceId: device, issuedAt: 1_800_000_000, mac: good.mac)
        XCTAssertFalse(tamperedModels.isAuthentic(key: key, desktopDeviceId: "desk", deviceId: device), "Eintrag verändert")
        let extra = BookTalkRelayModelList(models: models + [sonnet], desktopDeviceId: "desk", deviceId: device, issuedAt: 1_800_000_000, mac: good.mac)
        XCTAssertFalse(extra.isAuthentic(key: key, desktopDeviceId: "desk", deviceId: device), "Eintrag hinzugefügt")
        let later = BookTalkRelayModelList(models: models, desktopDeviceId: "desk", deviceId: device, issuedAt: 1_800_000_001, mac: good.mac)
        XCTAssertFalse(later.isAuthentic(key: key, desktopDeviceId: "desk", deviceId: device), "issued_at verändert")
        let noMac = BookTalkRelayModelList(models: models, desktopDeviceId: "desk", deviceId: device, issuedAt: 1_800_000_000, mac: nil)
        XCTAssertFalse(noMac.isAuthentic(key: key, desktopDeviceId: "desk", deviceId: device))
    }

    // MARK: Einrichtungscode (grant_expires_at)

    func testEnrollmentCodeAndGrantHaveSeparateExpiry() throws {
        let e = BookTalkRelayEnrollment(keyId: "k1", key: key, desktopDeviceId: "desk", desktopName: "PC", deviceId: device, expiresAt: 1_000, grantExpiresAt: 9_000)
        XCTAssertFalse(e.codeExpired(now: 999))
        XCTAssertTrue(e.codeExpired(now: 1_000), "Code abgelaufen")
        XCTAssertFalse(e.isExpired(now: 1_000), "Freigabe läuft weiter")
        XCTAssertFalse(e.isExpired(now: 8_999))
        XCTAssertTrue(e.isExpired(now: 9_000))
        let text = "lvrelay1:" + String(decoding: try JSONEncoder().encode(e), as: UTF8.self)
        XCTAssertTrue(text.contains("\"grant_expires_at\":9000"))
        XCTAssertEqual(BookTalkRelayEnrollment.parse(text), e)
        let legacy = BookTalkRelayEnrollment(keyId: "k1", key: key, desktopDeviceId: "desk", desktopName: "PC", deviceId: device, expiresAt: 1_000)
        XCTAssertNil(legacy.grantExpiresAt)
        XCTAssertTrue(legacy.isExpired(now: 1_000), "ohne grant_expires_at gilt expires_at auch für die Freigabe")
        XCTAssertFalse(String(decoding: try JSONEncoder().encode(legacy), as: UTF8.self).contains("grant_expires_at"))
    }

    func testRelayModelRefusesAfterTheGrantExpiredButNotAfterTheCodeExpired() async {
        let now = 1_800_000_000
        let channel = FakeChannel(), clock = FakeClock()
        let codeOnlyExpired = BookTalkRelayEnrollment(keyId: "k1", key: key, desktopDeviceId: "desk", desktopName: "PC", deviceId: device,
                                                      expiresAt: now - 10, grantExpiresAt: now + 86_400)
        await channel.setScript { _, _ in reply(seq: 0, "Ok", done: true) }
        let ok = await collect(model(channel, clock: clock, enroll: codeOnlyExpired))
        XCTAssertNil(ok.error)
        let grantExpired = BookTalkRelayEnrollment(keyId: "k1", key: key, desktopDeviceId: "desk", desktopName: "PC", deviceId: device,
                                                   expiresAt: now - 1_000, grantExpiresAt: now - 10)
        let refused = await collect(model(channel, clock: clock, enroll: grantExpired))
        XCTAssertEqual(refused.error as? BookTalkModelError, .notAllowed)
    }

    // MARK: Aufräumen (Befund 10)

    private func reqObject(_ id: String, device d: String = device, created: Int) throws -> BookTalkRelayCleanup.Object {
        let r = BookTalkRelayRequest.make(key: key, keyId: "k1", deviceId: d, requestId: id, modelId: "sonnet", system: "S", prompt: "P", maxTokens: 10, now: created)
        return .init(id: "req_" + id, json: try JSONEncoder().encode(r), firstSeen: created)
    }

    private func respObject(_ id: String, device d: String = device, seen: Int) throws -> BookTalkRelayCleanup.Object {
        let r = BookTalkRelayResponse.signed(key: key, requestId: id, deviceId: d, seq: 1, modelTuple: "claude-sonnet-5-5", text: "x", done: false)
        return .init(id: "resp_" + id, json: try JSONEncoder().encode(r), firstSeen: seen)
    }

    func testCleanupOnlyRemovesOwnOldObjectsOfClosedRequests() throws {
        let now = 1_800_000_000
        let old = now - 700, young = now - 30
        let objects = [
            try reqObject("a", created: old),                                  // eigen, alt: weg
            try reqObject("b", created: young),                                // eigen, jung: bleibt
            try reqObject("c", device: "fremd", created: old),                 // fremd: bleibt
            try reqObject("d", created: old),                                  // eigen, alt, aber offen: bleibt
            try respObject("e", seen: old),                                    // eigen, alt, geschlossen: weg
            try respObject("f", seen: young),                                  // eigen, jung gesehen: bleibt
            try respObject("g", seen: old),                                    // offene Anfrage: bleibt
            try respObject("h", device: "fremd", seen: old),                   // fremd: bleibt
            try respObject("b", seen: old),                                    // gehört zu jungem req: bleibt
        ]
        let stale = BookTalkRelayCleanup.stale(objects: objects, deviceId: device, openRequestIds: ["d", "g"], now: now)
        XCTAssertEqual(Set(stale), ["req_a", "resp_e"])
        XCTAssertEqual(stale.count, 2)
    }

    func testCleanupKeepsAnOpenRequestsResponseEvenWhenItsReqIsGone() throws {
        let now = 1_800_000_000
        let objects = [try respObject("x", seen: now - 5_000)]
        XCTAssertEqual(BookTalkRelayCleanup.stale(objects: objects, deviceId: device, openRequestIds: ["x"], now: now), [])
        XCTAssertEqual(BookTalkRelayCleanup.stale(objects: objects, deviceId: device, openRequestIds: [], now: now), ["resp_x"])
    }
}


private final class Flag: @unchecked Sendable {
    private let lock = NSLock()
    private var v = true
    var value: Bool { lock.lock(); defer { lock.unlock() }; return v }
    func set(_ x: Bool) { lock.lock(); v = x; lock.unlock() }
}
