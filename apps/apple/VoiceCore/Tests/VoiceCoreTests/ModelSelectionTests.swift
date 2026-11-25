import Foundation
import XCTest
@testable import VoiceCore

// Gemeinsame Modellwahl (lva#801 T4, lva#804): Kundenprofil, versionierter Freigabestand, Zustimmungsbindung,
// einmalige Berechtigung, Auswahl. Nur Foundation.

private actor Spy {
    var sent = 0
    var stolen: ModelAuthorization?
    func hit() { sent += 1 }
    func keep(_ a: ModelAuthorization) { stolen = a }
}

private struct Echo: ModelProvider {
    let route: ModelRoute
    var pieces = ["Hal", "lo"]
    var delay: UInt64 = 0
    var spy: Spy? = nil
    /// Läuft zwischen Erhalt und Einlösen der Berechtigung (Zustandswechsel „mitten drin“).
    var beforeConfirm: (@Sendable () async -> Void)? = nil
    func stream(_ request: ModelRequest, authorization: ModelAuthorization) -> AsyncThrowingStream<ModelEvent, Error> {
        let me = self
        return AsyncThrowingStream { c in
            let task = Task {
                do {
                    await me.spy?.keep(authorization)
                    await me.beforeConfirm?()
                    try await ModelProviderCheck.confirm(me, request, authorization)
                    await me.spy?.hit()
                    for p in me.pieces {
                        if me.delay > 0 { try await Task.sleep(nanoseconds: me.delay) }
                        c.yield(.delta(p))
                    }
                    c.yield(.done(nil)); c.finish()
                } catch { c.finish(throwing: error) }
            }
            c.onTermination = { _ in task.cancel() }
        }
    }
}

final class ModelSelectionTests: XCTestCase {
    private let ep = UUID(uuidString: "00000000-0000-0000-0000-000000000002")!
    private let claude = ModelRoute.pcRelay(provider: "claude-cli", model: "opus")
    private let openaiURL = URL(string: "https://api.openai.com/v1/chat/completions")!
    private let talk = ["Gesprochene Eingabe"]
    private let cats = ["Transkript"]
    private var local: ModelChoice { ModelChoice(route: .local(modelId: "qwen3-4b"), label: "Qwen3 4B", contextTokens: 32_000) }
    private var openai: ModelChoice { ModelChoice(route: .endpoint(id: ep), label: "GPT", vendor: "OpenAI", contextTokens: 128_000, region: "USA") }

    private func state(_ policy: ModelPolicy = .open) -> ModelAccessState {
        ModelAccessState(policy: policy, targets: [.endpoint(id: ep): ModelResolvedTarget(vendor: "OpenAI", url: openaiURL),
                                                   claude: ModelResolvedTarget(vendor: "Anthropic", url: nil)])
    }

    private func grant(_ s: ModelAccessState, _ route: ModelRoute, subject: String = "m1", purpose: ModelPurpose = .meetingMinutes,
                       categories: [String]? = nil) async throws {
        let (scope, _) = try await s.consentScope(route: route, purpose: purpose, subjectId: subject, dataCategories: categories ?? cats)!
        await s.record(ModelConsentRecord(scope: scope, grantedAt: Date()))
    }

    private func expect(_ failure: ModelFailure, line: UInt = #line, _ body: () async throws -> Void) async {
        do { try await body(); XCTFail("kein Fehler", line: line) }
        catch { XCTAssertEqual(error as? ModelFailure, failure, line: line) }
    }

    private func run(_ s: ModelAccessState, _ provider: Echo, purpose: ModelPurpose = .meetingMinutes, subject: String = "m1",
                     categories: [String]? = nil) async throws -> String {
        let session = try await ModelGate.open(provider, purpose: purpose, subjectId: subject, dataCategories: categories ?? cats, state: s)
        return try await session.complete(ModelRequest(purpose: purpose, messages: []))
    }

    func testExternalDerivedFromRouteNotTrusted() {
        XCTAssertFalse(local.isExternal)
        XCTAssertTrue(ModelChoice(route: .endpoint(id: ep), label: "LAN", ownNetworkHint: true, contextTokens: 1).isExternal)
        XCTAssertTrue(ModelChoice(route: claude, label: "C", contextTokens: 1).isExternal)
    }

    func testUnknownPolicyIsClosedByDefault() async {
        XCTAssertFalse(ModelPolicy().externalAllowed)
        let s = ModelAccessState(targets: [claude: ModelResolvedTarget(vendor: "Anthropic", url: nil)])
        await expect(.blockedByPolicy) { _ = try await self.run(s, Echo(route: self.claude)) }
        let text = try? await run(s, Echo(route: .local(modelId: "q")))
        XCTAssertEqual(text, "Hallo")
    }

    func testPolicyUsesResolvedVendor() async throws {
        let s = state(ModelPolicy(externalAllowed: true, allowedVendors: ["anthropic"]))
        await expect(.blockedByPolicy) { _ = try await s.consentScope(route: .endpoint(id: self.ep), purpose: .meetingMinutes, subjectId: "m1", dataCategories: self.cats) }
    }

    func testLocalOnlyCannotBeBypassed() async {
        let s = state()
        await expect(.localOnly) { _ = try await self.run(s, Echo(route: self.claude), purpose: .liveState) }
        await expect(.localOnly) { _ = try await self.run(s, Echo(route: .endpoint(id: self.ep)), purpose: .liveState) }
    }

    func testUnknownEndpointIsForbidden() async {
        await expect(.unavailable) { _ = try await self.run(self.state(), Echo(route: .endpoint(id: UUID()))) }
    }

    func testEndpointDestinationChecks() async {
        for bad in ["http://api.example.com/v1", "https://user:pw@api.example.com/v1", "ftp://api.example.com", "http://8.8.8.8/v1",
                    "http://10.0.0.1.attacker.example/v1", "http://pc.local/v1"] {
            let s = ModelAccessState(policy: .open, targets: [.endpoint(id: ep): ModelResolvedTarget(vendor: "X", url: URL(string: bad)!)])
            await expect(.insecureDestination) { _ = try await s.consentScope(route: .endpoint(id: self.ep), purpose: .talk, subjectId: "g", dataCategories: self.cats) }
        }
        for ok in ["http://192.168.178.20:11434/v1", "http://localhost:1234/v1", "http://10.0.0.5/v1", "http://172.20.1.1/v1", "https://api.example.com/v1"] {
            let s = ModelAccessState(policy: .open, targets: [.endpoint(id: ep): ModelResolvedTarget(vendor: "X", url: URL(string: ok)!)])
            let r = try? await s.consentScope(route: .endpoint(id: ep), purpose: .talk, subjectId: "g", dataCategories: cats)
            XCTAssertNotNil(r ?? nil, ok)
        }
    }

    func testHostCheckIsStrict() {
        for bad in ["10.0.0.1.attacker.example", "192.168.1.1.example.com", "010.0.0.1", "0x7f.0.0.1", "127.1", "10.0.0", "10.0.0.256",
                    "10.0.0.1.", "1.2.3.4.local", "evil.com", "fe80::1%en0", "::ffff:10.0.0.1", "2001:db8::1", "local", ".local",
                    "pc-patrick.local", "evil.local", "172.32.0.1"] {
            XCTAssertFalse(ModelGate.isPrivateHost(bad), bad)
        }
        for ok in ["127.0.0.1", "10.0.0.1", "172.16.0.1", "192.168.178.20", "169.254.1.1", "localhost", "[::1]", "fd12:3456::1", "fe80::1"] {
            XCTAssertTrue(ModelGate.isPrivateHost(ok), ok)
        }
    }

    func testConsentBoundToScope() async throws {
        let s = state()
        await expect(.consentMissing) { _ = try await self.run(s, Echo(route: .endpoint(id: self.ep))) }
        try await grant(s, .endpoint(id: ep))
        _ = try await run(s, Echo(route: .endpoint(id: ep)))
        await expect(.consentMissing) { _ = try await self.run(s, Echo(route: .endpoint(id: self.ep)), subject: "m2") }
        await expect(.consentMissing) { _ = try await self.run(s, Echo(route: .endpoint(id: self.ep)), purpose: .meetingSummary) }
        await expect(.consentMissing) { _ = try await self.run(s, Echo(route: .endpoint(id: self.ep)), categories: ["Transkript", "Teilnehmernamen"]) }
        // Gleiche UUID, Adresse bzw. Anbieter geändert: alte Zustimmung gilt nicht.
        await s.setTarget(ModelResolvedTarget(vendor: "OpenAI", url: URL(string: "https://proxy.example.com/v1/chat/completions")!), for: .endpoint(id: ep))
        await expect(.consentMissing) { _ = try await self.run(s, Echo(route: .endpoint(id: self.ep))) }
        await s.setTarget(ModelResolvedTarget(vendor: "Mistral", url: openaiURL), for: .endpoint(id: ep))
        await expect(.consentMissing) { _ = try await self.run(s, Echo(route: .endpoint(id: self.ep))) }
    }

    func testAuthorizationBindsDestination() async throws {
        let s = state()
        try await grant(s, .endpoint(id: ep))
        let spy = Spy()
        _ = try await run(s, Echo(route: .endpoint(id: ep), spy: spy))
        let auth = await spy.stolen!
        XCTAssertTrue(auth.permitsSending(to: openaiURL))
        XCTAssertFalse(auth.permitsSending(to: URL(string: "https://evil.example.com/v1/chat/completions")!))
        XCTAssertFalse(auth.permitsSending(to: URL(string: "https://api.openai.com:8443/v1/chat/completions")!))
    }

    func testRouteKeysAreUnambiguous() {
        XCTAssertNotEqual(ModelRoute.pcRelay(provider: "a/b", model: "c").key, ModelRoute.pcRelay(provider: "a", model: "b/c").key)
        XCTAssertEqual(claude.key, "relay:claude-cli/opus")
    }

    func testAuthorizationIsSingleUseAndBound() async throws {
        let s = state()
        try await grant(s, claude, subject: "g", purpose: .talk, categories: talk)
        let spy = Spy()
        let relay = Echo(route: claude, spy: spy)
        let first = try await run(s, relay, purpose: .talk, subject: "g", categories: talk)
        XCTAssertEqual(first, "Hallo")
        let stolen = await spy.stolen!
        // Wiederverwendung und fremder Provider: abgelehnt, nichts gesendet.
        for provider in [relay, Echo(route: .local(modelId: "q"))] {
            var failed = false
            do { for try await _ in provider.stream(ModelRequest(purpose: .talk, messages: []), authorization: stolen) {} } catch { failed = true }
            XCTAssertTrue(failed)
        }
        let sent1 = await spy.sent; XCTAssertEqual(sent1, 1)
        // Anderer Zweck über dieselbe Sitzung: abgelehnt.
        let session = try await ModelGate.open(relay, purpose: .talk, subjectId: "g", dataCategories: talk, state: s)
        await expect(.consentMissing) { _ = try await session.complete(ModelRequest(purpose: .meetingMinutes, messages: [])) }
    }

    // Codex Runde 4, HOCH 3: Profil wechselt zwischen Ausstellen und Einlösen.
    func testPolicyChangeBetweenIssueAndConfirm() async throws {
        let s = state()
        try await grant(s, claude, subject: "g", purpose: .talk, categories: talk)
        let spy = Spy()
        let relay = Echo(route: claude, spy: spy, beforeConfirm: { await s.setPolicy(.localOnly) })
        await expect(.blockedByPolicy) { _ = try await self.run(s, relay, purpose: .talk, subject: "g", categories: self.talk) }
        let sent0 = await spy.sent; XCTAssertEqual(sent0, 0)
    }

    // Codex Runde 4, HOCH 2: Ziel wechselt und wird neu freigegeben, alte Berechtigung zeigt auf das alte Ziel.
    func testDestinationSwitchBetweenIssueAndConfirm() async throws {
        let s = state()
        try await grant(s, .endpoint(id: ep))
        let other = URL(string: "https://proxy.example.com/v1/chat/completions")!
        let spy = Spy()
        let route = ModelRoute.endpoint(id: ep), categories = cats
        let provider = Echo(route: route, spy: spy, beforeConfirm: {
            await s.setTarget(ModelResolvedTarget(vendor: "OpenAI", url: other), for: route)
            if let (scope, _) = try? await s.consentScope(route: route, purpose: .meetingMinutes, subjectId: "m1", dataCategories: categories) {
                await s.record(ModelConsentRecord(scope: scope, grantedAt: Date()))
            }
        })
        await expect(.consentMissing) { _ = try await self.run(s, provider) }
        let sent0 = await spy.sent; XCTAssertEqual(sent0, 0)
    }

    func testRevocationAndRemovedRouteAfterOpen() async throws {
        let s = state()
        try await grant(s, claude, subject: "g", purpose: .talk, categories: talk)
        let session = try await ModelGate.open(Echo(route: claude), purpose: .talk, subjectId: "g", dataCategories: talk, state: s)
        let request = ModelRequest(purpose: .talk, messages: [])
        _ = try await session.complete(request)
        await s.setPolicy(.localOnly)
        await expect(.blockedByPolicy) { _ = try await session.complete(request) }
        await s.setPolicy(.open)
        _ = try await session.complete(request)
        await s.revokeAll(subjectId: "g")
        await expect(.consentMissing) { _ = try await session.complete(request) }
        try await grant(s, claude, subject: "g", purpose: .talk, categories: talk)
        await s.setTarget(nil, for: claude)
        await expect(.unavailable) { _ = try await session.complete(request) }
    }

    func testCancelDuringIssueAndStream() async throws {
        let s = state()
        try await grant(s, claude, subject: "g", purpose: .talk, categories: talk)
        let spy = Spy()
        let slowConfirm = Echo(route: claude, spy: spy, beforeConfirm: { try? await Task.sleep(nanoseconds: 300_000_000) })
        let session = try await ModelGate.open(slowConfirm, purpose: .talk, subjectId: "g", dataCategories: talk, state: s)
        let during = Task { try await session.complete(ModelRequest(purpose: .talk, messages: [])) }
        try await Task.sleep(nanoseconds: 50_000_000)
        during.cancel()
        do { _ = try await during.value; XCTFail() } catch { XCTAssertTrue(error is CancellationError) }
        let sent0 = await spy.sent; XCTAssertEqual(sent0, 0)

        let slow = try await ModelGate.open(Echo(route: .local(modelId: "q"), pieces: ["a", "b", "c", "d"], delay: 100_000_000),
                                            purpose: .talk, subjectId: "g", dataCategories: [], state: s)
        let running = Task { () -> String in
            var text = ""
            for try await e in slow.stream(ModelRequest(purpose: .talk, messages: [])) { if case .delta(let p) = e { text += p } }
            return text
        }
        try await Task.sleep(nanoseconds: 150_000_000)
        running.cancel()
        do { let t = try await running.value; XCTAssertLessThan(t.count, 4) } catch { XCTAssertTrue(error is CancellationError) }

        let already = Task { () -> Int in
            withUnsafeCurrentTask { $0?.cancel() }
            var n = 0
            for try await _ in slow.stream(ModelRequest(purpose: .talk, messages: [])) { n += 1 }
            return n
        }
        do { _ = try await already.value; XCTFail() } catch { XCTAssertTrue(error is CancellationError) }
    }

    func testSelectionFallsBackToLocalNeverSilentlyExternal() {
        let c = ModelChoice(route: claude, label: "Claude (Abo)", vendor: "Anthropic", contextTokens: 200_000)
        let all = [openai, local, c]
        XCTAssertEqual(ModelSelection.resolve(storedKey: nil, choices: all, purpose: .meetingMinutes, policy: .open), local)
        XCTAssertEqual(ModelSelection.resolve(storedKey: c.id, choices: all, purpose: .meetingMinutes, policy: .open), c)
        XCTAssertEqual(ModelSelection.resolve(storedKey: c.id, choices: all, purpose: .meetingMinutes, policy: .unknown), local)
        XCTAssertEqual(ModelSelection.resolve(storedKey: c.id, choices: all, purpose: .liveState, policy: .open), local)
        XCTAssertNil(ModelSelection.resolve(storedKey: nil, choices: [openai], purpose: .talk, policy: .open))
    }

    func testCodableTokenCapsAndSummary() async throws {
        let c = ModelChoice(route: claude, label: "Claude (Abo)", vendor: "Anthropic", contextTokens: 1)
        let data = try JSONEncoder().encode([local, openai, c])
        XCTAssertEqual(try JSONDecoder().decode([ModelChoice].self, from: data), [local, openai, c])
        XCTAssertEqual(ModelRequest(purpose: .talk, messages: [], maxTokens: 5000).maxTokens, 800)
        XCTAssertEqual(ModelRequest(purpose: .meetingMinutes, messages: []).maxTokens, 4000)
        let (scope, target) = try await state().consentScope(route: claude, purpose: .meetingMinutes, subjectId: "m", dataCategories: ["Transkript", "Teilnehmernamen"])!
        XCTAssertEqual(ModelConsentRecord.summary(choice: c, scope: scope, vendor: target.vendor, region: target.region),
                       "Teilnehmernamen, Transkript gehen über deinen PC an Anthropic und werden dort mit Claude (Abo) verarbeitet.")
        let st = state()
        try await grant(st, claude)
        let records = try JSONEncoder().encode(await st.consentRecords())
        XCTAssertEqual(try JSONDecoder().decode([ModelConsentRecord].self, from: records).count, 1)
    }
}
