import Foundation
import XCTest
@testable import VoiceCore

// Gemeinsame Modellwahl (lva#801 T4, lva#804): Kundenprofil, Prüfstelle, Zustimmungsbindung, Auswahl. Nur Foundation.

private struct Resolver: ModelRouteResolver {
    var targets: [String: ModelResolvedTarget]
    func resolve(_ route: ModelRoute) async -> ModelResolvedTarget? { targets[route.key] }
}

private actor Counter {
    var sent = 0
    var stolen: ModelAuthorization?
    func hit() { sent += 1 }
    func keep(_ a: ModelAuthorization) { stolen = a }
}

private struct Echo: ModelProvider {
    let route: ModelRoute
    var pieces = ["Hal", "lo"]
    var delay: UInt64 = 0
    var counter: Counter? = nil
    func stream(_ request: ModelRequest, authorization: ModelAuthorization) -> AsyncThrowingStream<ModelEvent, Error> {
        let me = self
        return AsyncThrowingStream { c in
            let task = Task {
                do {
                    await me.counter?.keep(authorization)
                    try await ModelProviderCheck.confirm(me, request, authorization)
                    await me.counter?.hit()
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
    private var local: ModelChoice { ModelChoice(route: .local(modelId: "qwen3-4b"), label: "Qwen3 4B", contextTokens: 32_000) }
    private var openai: ModelChoice { ModelChoice(route: .endpoint(id: ep), label: "GPT", vendor: "OpenAI", contextTokens: 128_000, region: "USA") }
    private let claudeRoute = ModelRoute.pcRelay(provider: "claude-cli", model: "opus")
    private var resolver: Resolver {
        Resolver(targets: [
            ModelRoute.endpoint(id: ep).key: ModelResolvedTarget(vendor: "OpenAI", url: URL(string: "https://api.openai.com/v1/chat/completions")!),
            claudeRoute.key: ModelResolvedTarget(vendor: "Anthropic", url: nil),
        ])
    }
    private let cats = ["Transkript"]

    private func expect(_ failure: ModelFailure, _ body: () async throws -> Void, line: UInt = #line) async {
        do { try await body(); XCTFail("kein Fehler", line: line) }
        catch { XCTAssertEqual(error as? ModelFailure, failure, line: line) }
    }

    private func grant(_ store: InMemoryModelConsentStore, route: ModelRoute, subject: String = "m1", purpose: ModelPurpose = .meetingMinutes,
                       categories: [String]? = nil, resolver r: Resolver? = nil) async throws {
        let (scope, _) = try await ModelGate.consentScope(route: route, purpose: purpose, subjectId: subject, dataCategories: categories ?? cats,
                                                         policy: .open, resolver: r ?? resolver)!
        await store.record(ModelConsentRecord(scope: scope, grantedAt: Date()))
    }

    func testExternalDerivedFromRouteNotTrusted() {
        XCTAssertFalse(local.isExternal)
        XCTAssertTrue(ModelChoice(route: .endpoint(id: ep), label: "LAN", ownNetworkHint: true, contextTokens: 1).isExternal)
        XCTAssertTrue(ModelChoice(route: claudeRoute, label: "C", contextTokens: 1).isExternal)
    }

    func testUnknownPolicyIsClosedByDefault() async {
        XCTAssertFalse(ModelPolicy().externalAllowed)
        let store = InMemoryModelConsentStore()
        await expect(.blockedByPolicy) { _ = try await ModelGate.authorize(route: self.claudeRoute, purpose: .talk, subjectId: "g", dataCategories: self.cats, policy: .unknown, resolver: self.resolver, consents: store) }
    }

    func testPolicyVendorFromResolverNotDisplay() async throws {
        let store = InMemoryModelConsentStore()
        try await grant(store, route: .endpoint(id: ep))
        // Anzeige sagt „Anthropic“, aufgelöst ist OpenAI: das Profil prüft den aufgelösten Anbieter.
        let p = ModelPolicy(externalAllowed: true, allowedVendors: ["anthropic"])
        await expect(.blockedByPolicy) { _ = try await ModelGate.authorize(route: .endpoint(id: self.ep), purpose: .meetingMinutes, subjectId: "m1", dataCategories: self.cats, policy: p, resolver: self.resolver, consents: store) }
    }

    func testLocalOnlyCannotBeBypassed() async {
        let store = InMemoryModelConsentStore()
        await expect(.localOnly) { _ = try await ModelGate.authorize(route: self.claudeRoute, purpose: .liveState, subjectId: "m", dataCategories: self.cats, policy: .open, resolver: self.resolver, consents: store) }
        await expect(.localOnly) { _ = try await ModelGate.authorize(route: .endpoint(id: self.ep), purpose: .liveState, subjectId: "m", dataCategories: self.cats, policy: .open, resolver: self.resolver, consents: store) }
    }

    func testUnknownEndpointIsForbidden() async {
        let store = InMemoryModelConsentStore()
        await expect(.unavailable) { _ = try await ModelGate.authorize(route: .endpoint(id: UUID()), purpose: .talk, subjectId: "g", dataCategories: self.cats, policy: .open, resolver: self.resolver, consents: store) }
    }

    func testEndpointDestinationChecks() async {
        for bad in ["http://api.example.com/v1", "https://user:pw@api.example.com/v1", "ftp://api.example.com", "http://8.8.8.8/v1"] {
            let r = Resolver(targets: [ModelRoute.endpoint(id: ep).key: ModelResolvedTarget(vendor: "X", url: URL(string: bad)!)])
            await expect(.insecureDestination) { _ = try await ModelGate.consentScope(route: .endpoint(id: self.ep), purpose: .talk, subjectId: "g", dataCategories: self.cats, policy: .open, resolver: r) }
        }
        for ok in ["http://192.168.178.20:11434/v1", "http://localhost:1234/v1", "http://10.0.0.5/v1", "http://172.20.1.1/v1", "https://api.example.com/v1"] {
            let r = Resolver(targets: [ModelRoute.endpoint(id: ep).key: ModelResolvedTarget(vendor: "X", url: URL(string: ok)!)])
            let scope = try? await ModelGate.consentScope(route: .endpoint(id: ep), purpose: .talk, subjectId: "g", dataCategories: cats, policy: .open, resolver: r)
            XCTAssertNotNil(scope ?? nil, ok)
        }
        XCTAssertFalse(ModelGate.isPrivateHost("172.32.0.1"))
        XCTAssertTrue(ModelGate.isPrivateHost("[::1]"))
    }

    func testConsentBoundToScope() async throws {
        let store = InMemoryModelConsentStore()
        await expect(.consentMissing) { _ = try await ModelGate.authorize(route: .endpoint(id: self.ep), purpose: .meetingMinutes, subjectId: "m1", dataCategories: self.cats, policy: .open, resolver: self.resolver, consents: store) }
        try await grant(store, route: .endpoint(id: ep))
        let auth = try await ModelGate.authorize(route: .endpoint(id: ep), purpose: .meetingMinutes, subjectId: "m1", dataCategories: cats, policy: .open, resolver: resolver, consents: store)
        XCTAssertTrue(auth.permitsSending(to: URL(string: "https://api.openai.com/v1/chat/completions")!))
        XCTAssertFalse(auth.permitsSending(to: URL(string: "https://evil.example.com/v1/chat/completions")!))
        XCTAssertFalse(auth.permitsSending(to: URL(string: "https://api.openai.com:8443/v1/chat/completions")!))
        // Andere Besprechung, anderer Zweck, mehr Datenkategorien: keine Zustimmung.
        await expect(.consentMissing) { _ = try await ModelGate.authorize(route: .endpoint(id: self.ep), purpose: .meetingMinutes, subjectId: "m2", dataCategories: self.cats, policy: .open, resolver: self.resolver, consents: store) }
        await expect(.consentMissing) { _ = try await ModelGate.authorize(route: .endpoint(id: self.ep), purpose: .meetingSummary, subjectId: "m1", dataCategories: self.cats, policy: .open, resolver: self.resolver, consents: store) }
        await expect(.consentMissing) { _ = try await ModelGate.authorize(route: .endpoint(id: self.ep), purpose: .meetingMinutes, subjectId: "m1", dataCategories: ["Transkript", "Teilnehmernamen"], policy: .open, resolver: self.resolver, consents: store) }
        // Gleiche UUID, Anbieter bzw. Adresse geändert: alte Zustimmung gilt nicht.
        let moved = Resolver(targets: [ModelRoute.endpoint(id: ep).key: ModelResolvedTarget(vendor: "OpenAI", url: URL(string: "https://proxy.example.com/v1/chat/completions")!)])
        await expect(.consentMissing) { _ = try await ModelGate.authorize(route: .endpoint(id: self.ep), purpose: .meetingMinutes, subjectId: "m1", dataCategories: self.cats, policy: .open, resolver: moved, consents: store) }
        let rebranded = Resolver(targets: [ModelRoute.endpoint(id: ep).key: ModelResolvedTarget(vendor: "Mistral", url: URL(string: "https://api.openai.com/v1/chat/completions")!)])
        await expect(.consentMissing) { _ = try await ModelGate.authorize(route: .endpoint(id: self.ep), purpose: .meetingMinutes, subjectId: "m1", dataCategories: self.cats, policy: .open, resolver: rebranded, consents: store) }
    }

    func testRouteKeysAreUnambiguous() {
        XCTAssertNotEqual(ModelRoute.pcRelay(provider: "a/b", model: "c").key, ModelRoute.pcRelay(provider: "a", model: "b/c").key)
        XCTAssertEqual(claudeRoute.key, "relay:claude-cli/opus")
    }

    func testSessionEnforcesGateAndBinding() async throws {
        let store = InMemoryModelConsentStore()
        let relay = Echo(route: claudeRoute)
        await expect(.consentMissing) { _ = try await ModelGate.open(relay, purpose: .talk, subjectId: "g", dataCategories: ["Gesprochene Eingabe"], policy: .open, resolver: self.resolver, consents: store) }
        try await grant(store, route: claudeRoute, subject: "g", purpose: .talk, categories: ["Gesprochene Eingabe"])
        let session = try await ModelGate.open(relay, purpose: .talk, subjectId: "g", dataCategories: ["Gesprochene Eingabe"], policy: .open, resolver: resolver, consents: store)
        let text = try await session.complete(ModelRequest(purpose: .talk, messages: [ModelMessage(role: .user, text: "x")]))
        XCTAssertEqual(text, "Hallo")
        // Anderer Zweck über dieselbe Sitzung: abgelehnt.
        await expect(.consentMissing) { _ = try await session.complete(ModelRequest(purpose: .meetingMinutes, messages: [])) }
        // Eine abgegriffene Berechtigung ist verbraucht und passt nicht zu einem anderen Provider.
        let spy = Counter()
        let local = Echo(route: .local(modelId: "q"), counter: spy)
        let localSession = try await ModelGate.open(local, purpose: .talk, subjectId: "g", dataCategories: [], policy: ModelPolicy.unknown, resolver: resolver, consents: store)
        _ = try await localSession.complete(ModelRequest(purpose: .talk, messages: []))
        let stolen = await spy.stolen!
        for provider in [local, relay] {
            var failed = false
            do { for try await _ in provider.stream(ModelRequest(purpose: .talk, messages: []), authorization: stolen) {} } catch { failed = true }
            XCTAssertTrue(failed)
        }
    }

    func testCompleteChecksCancellationFirst() async throws {
        let store = InMemoryModelConsentStore()
        let session = try await ModelGate.open(Echo(route: .local(modelId: "q"), pieces: []), purpose: .talk, subjectId: "g", dataCategories: [], policy: ModelPolicy.unknown, resolver: resolver, consents: store)
        let task = Task { () -> String in
            withUnsafeCurrentTask { $0?.cancel() }
            return try await session.complete(ModelRequest(purpose: .talk, messages: []))
        }
        do { _ = try await task.value; XCTFail() } catch { XCTAssertTrue(error is CancellationError) }
    }

    func testHostCheckIsStrict() {
        for bad in ["10.0.0.1.attacker.example", "192.168.1.1.example.com", "010.0.0.1", "0x7f.0.0.1", "127.1", "10.0.0", "10.0.0.256",
                    "10.0.0.1.", "1.2.3.4.local", "evil.com", "fe80::1%en0", "::ffff:10.0.0.1", "2001:db8::1", "local", ".local", "a..local", "pc-patrick.local", "evil.local"] {
            XCTAssertFalse(ModelGate.isPrivateHost(bad), bad)
        }
        for ok in ["127.0.0.1", "10.0.0.1", "172.16.0.1", "192.168.178.20", "169.254.1.1", "localhost", "[::1]", "fd12:3456::1", "fe80::1"] {
            XCTAssertTrue(ModelGate.isPrivateHost(ok), ok)
        }
        XCTAssertNil(ModelGate.validatedOrigin(URL(string: "http://10.0.0.1.attacker.example/v1")!))
    }

    func testPolicyChangeAndRevocationAfterOpen() async throws {
        actor Profile: ModelPolicySource {
            var policy = ModelPolicy.open
            func set(_ p: ModelPolicy) { policy = p }
            func currentPolicy() -> ModelPolicy { policy }
        }
        let store = InMemoryModelConsentStore()
        let profile = Profile()
        try await grant(store, route: claudeRoute, subject: "g", purpose: .talk, categories: ["Gesprochene Eingabe"])
        let session = try await ModelGate.open(Echo(route: claudeRoute), purpose: .talk, subjectId: "g", dataCategories: ["Gesprochene Eingabe"], policy: profile, resolver: resolver, consents: store)
        let request = ModelRequest(purpose: .talk, messages: [])
        _ = try await session.complete(request)
        await profile.set(.localOnly)
        await expect(.blockedByPolicy) { _ = try await session.complete(request) }
        await profile.set(.open)
        _ = try await session.complete(request)
        let (scope, _) = try await ModelGate.consentScope(route: claudeRoute, purpose: .talk, subjectId: "g", dataCategories: ["Gesprochene Eingabe"], policy: .open, resolver: resolver)!
        await store.revoke(scope)
        await expect(.consentMissing) { _ = try await session.complete(request) }
        // Route verschwindet (PC meldet das Modell nicht mehr): verboten.
        try await grant(store, route: claudeRoute, subject: "g", purpose: .talk, categories: ["Gesprochene Eingabe"])
        let pc = Resolver(targets: resolver.targets)
        actor Switch: ModelRouteResolver {
            var inner: Resolver
            init(_ r: Resolver) { inner = r }
            func clear() { inner = Resolver(targets: [:]) }
            func resolve(_ route: ModelRoute) async -> ModelResolvedTarget? { await inner.resolve(route) }
        }
        let live = Switch(pc)
        let relay = try await ModelGate.open(Echo(route: claudeRoute), purpose: .talk, subjectId: "g", dataCategories: ["Gesprochene Eingabe"], policy: ModelPolicy.open, resolver: live, consents: store)
        _ = try await relay.complete(request)
        await live.clear()
        await expect(.unavailable) { _ = try await relay.complete(request) }
    }

    func testRevocationAfterGateIsCaughtBeforeSending() async throws {
        actor Profile: ModelPolicySource {
            var policy = ModelPolicy.open
            func set(_ p: ModelPolicy) { policy = p }
            func currentPolicy() -> ModelPolicy { policy }
        }
        // Das Profil wechselt, während die Route aufgelöst wird (nach dem ersten Lesen des Profils).
        actor Flipping: ModelRouteResolver {
            let inner: Resolver; let profile: Profile; var calls = 0
            init(_ r: Resolver, _ p: Profile) { inner = r; profile = p }
            func resolve(_ route: ModelRoute) async -> ModelResolvedTarget? {
                calls += 1
                if calls == 2 { await profile.set(.localOnly) }   // 1 = open, 2 = erster Aufruf von stream
                return await inner.resolve(route)
            }
        }
        let store = InMemoryModelConsentStore()
        try await grant(store, route: claudeRoute, subject: "g", purpose: .talk, categories: ["Gesprochene Eingabe"])
        let profile = Profile(), counter = Counter()
        let session = try await ModelGate.open(Echo(route: claudeRoute, counter: counter), purpose: .talk, subjectId: "g",
                                               dataCategories: ["Gesprochene Eingabe"], policy: profile, resolver: Flipping(resolver, profile), consents: store)
        await expect(.blockedByPolicy) { _ = try await session.complete(ModelRequest(purpose: .talk, messages: [])) }
        let sent = await counter.sent
        XCTAssertEqual(sent, 0)
    }

    func testCancelDuringAuthorizationAndStream() async throws {
        struct Slow: ModelRouteResolver {
            let inner: Resolver
            func resolve(_ route: ModelRoute) async -> ModelResolvedTarget? {
                try? await Task.sleep(nanoseconds: 300_000_000)
                return await inner.resolve(route)
            }
        }
        let store = InMemoryModelConsentStore()
        try await grant(store, route: claudeRoute, subject: "g", purpose: .talk, categories: ["Gesprochene Eingabe"])
        let counter = Counter()
        let open = try await ModelGate.open(Echo(route: claudeRoute, counter: counter), purpose: .talk, subjectId: "g",
                                            dataCategories: ["Gesprochene Eingabe"], policy: ModelPolicy.open, resolver: Slow(inner: resolver), consents: store)
        let during = Task { try await open.complete(ModelRequest(purpose: .talk, messages: [])) }
        try await Task.sleep(nanoseconds: 50_000_000)
        during.cancel()
        do { _ = try await during.value; XCTFail() } catch { XCTAssertTrue(error is CancellationError) }
        let sentAfterAuthCancel = await counter.sent
        XCTAssertEqual(sentAfterAuthCancel, 0)

        let slowStream = try await ModelGate.open(Echo(route: .local(modelId: "q"), pieces: ["a", "b", "c", "d"], delay: 100_000_000), purpose: .talk,
                                                  subjectId: "g", dataCategories: [], policy: ModelPolicy.unknown, resolver: resolver, consents: store)
        let running = Task { () -> String in
            var text = ""
            for try await e in slowStream.stream(ModelRequest(purpose: .talk, messages: [])) { if case .delta(let p) = e { text += p } }
            return text
        }
        try await Task.sleep(nanoseconds: 150_000_000)
        running.cancel()
        do { let t = try await running.value; XCTAssertLessThan(t.count, 4) } catch { XCTAssertTrue(error is CancellationError) }
    }

    func testStreamOnCancelledTaskSendsNothing() async throws {
        let store = InMemoryModelConsentStore()
        let session = try await ModelGate.open(Echo(route: .local(modelId: "q")), purpose: .talk, subjectId: "g", dataCategories: [], policy: ModelPolicy.unknown, resolver: resolver, consents: store)
        let task = Task { () -> Int in
            withUnsafeCurrentTask { $0?.cancel() }
            var n = 0
            for try await _ in session.stream(ModelRequest(purpose: .talk, messages: [])) { n += 1 }
            return n
        }
        do { let n = try await task.value; XCTFail("\(n) Ereignisse") } catch { XCTAssertTrue(error is CancellationError) }
    }

    func testSelectionFallsBackToLocalNeverSilentlyExternal() {
        let claude = ModelChoice(route: claudeRoute, label: "Claude (Abo)", vendor: "Anthropic", contextTokens: 200_000)
        let all = [openai, local, claude]
        XCTAssertEqual(ModelSelection.resolve(storedKey: nil, choices: all, purpose: .meetingMinutes, policy: .open), local)
        XCTAssertEqual(ModelSelection.resolve(storedKey: claude.id, choices: all, purpose: .meetingMinutes, policy: .open), claude)
        XCTAssertEqual(ModelSelection.resolve(storedKey: claude.id, choices: all, purpose: .meetingMinutes, policy: .unknown), local)
        XCTAssertEqual(ModelSelection.resolve(storedKey: claude.id, choices: all, purpose: .liveState, policy: .open), local)
        XCTAssertNil(ModelSelection.resolve(storedKey: nil, choices: [openai], purpose: .talk, policy: .open))
    }

    func testCodableAndTokenCaps() throws {
        let claude = ModelChoice(route: claudeRoute, label: "C", vendor: "Anthropic", contextTokens: 1)
        let data = try JSONEncoder().encode([local, openai, claude])
        XCTAssertEqual(try JSONDecoder().decode([ModelChoice].self, from: data), [local, openai, claude])
        XCTAssertEqual(ModelRequest(purpose: .talk, messages: [], maxTokens: 5000).maxTokens, 800)
        XCTAssertEqual(ModelRequest(purpose: .meetingMinutes, messages: []).maxTokens, 4000)
    }

    func testConsentSummary() async throws {
        let claude = ModelChoice(route: claudeRoute, label: "Claude (Abo)", vendor: "Anthropic", contextTokens: 1)
        let (scope, target) = try await ModelGate.consentScope(route: claudeRoute, purpose: .meetingMinutes, subjectId: "m", dataCategories: ["Transkript", "Teilnehmernamen"], policy: .open, resolver: resolver)!
        XCTAssertEqual(ModelConsentRecord.summary(choice: claude, scope: scope, vendor: target.vendor, region: target.region),
                       "Teilnehmernamen, Transkript gehen über deinen PC an Anthropic und werden dort mit Claude (Abo) verarbeitet.")
    }
}
