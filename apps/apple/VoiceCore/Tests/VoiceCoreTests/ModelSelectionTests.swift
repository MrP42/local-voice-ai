import Foundation
import XCTest
@testable import VoiceCore

// Gemeinsame Modellwahl (lva#801 T4, lva#804): Kundenprofil, Zustimmung, Auswahl. Nur Foundation.

final class ModelSelectionTests: XCTestCase {
    private let local = ModelChoice(route: .local(modelId: "qwen3-4b"), label: "Qwen3 4B", isExternal: false, contextTokens: 32_000)
    private let lan = ModelChoice(route: .endpoint(id: UUID(uuidString: "00000000-0000-0000-0000-000000000001")!), label: "Ollama", isExternal: false, contextTokens: 16_000)
    private let openai = ModelChoice(route: .endpoint(id: UUID(uuidString: "00000000-0000-0000-0000-000000000002")!), label: "GPT", vendor: "OpenAI", isExternal: true, contextTokens: 128_000, region: "USA")
    private let claude = ModelChoice(route: .pcRelay(provider: "claude-cli", model: "opus"), label: "Claude (Abo)", vendor: "Anthropic", isExternal: true, contextTokens: 200_000)

    func testPolicyLocalAlwaysAllowed() {
        XCTAssertTrue(ModelPolicy.localOnly.permits(local))
        XCTAssertTrue(ModelPolicy.localOnly.permits(lan))
        XCTAssertFalse(ModelPolicy.localOnly.permits(openai))
    }

    func testPolicyVendorList() {
        let p = ModelPolicy(allowedVendors: ["anthropic"])
        XCTAssertTrue(p.permits(claude))
        XCTAssertFalse(p.permits(openai))
        let nameless = ModelChoice(route: .endpoint(id: UUID()), label: "X", isExternal: true, contextTokens: 1)
        XCTAssertFalse(p.permits(nameless))
    }

    func testGateNeedsConsentForExternal() async throws {
        let store = InMemoryModelConsentStore()
        do { try await ModelGate.check(openai, purpose: .meetingMinutes, subjectId: "m1", policy: ModelPolicy(), consents: store); XCTFail() }
        catch { XCTAssertEqual(error as? ModelFailure, .consentMissing) }
        await store.record(ModelConsentRecord(subjectId: "m1", purpose: .meetingMinutes, route: openai.route, vendor: "OpenAI", dataCategories: ["Transkript"], grantedAt: Date()))
        try await ModelGate.check(openai, purpose: .meetingMinutes, subjectId: "m1", policy: ModelPolicy(), consents: store)
        // Zustimmung gilt nur für diese Besprechung, diesen Zweck und diese Route.
        for (s, p, c) in [("m2", ModelPurpose.meetingMinutes, openai), ("m1", .meetingSummary, openai), ("m1", .meetingMinutes, claude)] {
            do { try await ModelGate.check(c, purpose: p, subjectId: s, policy: ModelPolicy(), consents: store); XCTFail() }
            catch { XCTAssertEqual(error as? ModelFailure, .consentMissing) }
        }
    }

    func testGatePolicyAndLocalOnly() async throws {
        let store = InMemoryModelConsentStore()
        try await ModelGate.check(local, purpose: .liveState, subjectId: "m", policy: .localOnly, consents: store)
        do { try await ModelGate.check(lan, purpose: .liveState, subjectId: "m", policy: ModelPolicy(), consents: store); XCTFail() }
        catch { XCTAssertEqual(error as? ModelFailure, .localOnly) }
        do { try await ModelGate.check(claude, purpose: .talk, subjectId: "g", policy: .localOnly, consents: store); XCTFail() }
        catch { XCTAssertEqual(error as? ModelFailure, .blockedByPolicy) }
    }

    func testSelectionFallsBackToLocalNeverSilentlyExternal() {
        let all = [openai, local, claude]
        XCTAssertEqual(ModelSelection.resolve(storedKey: nil, choices: all, purpose: .meetingMinutes, policy: ModelPolicy()), local)
        XCTAssertEqual(ModelSelection.resolve(storedKey: claude.id, choices: all, purpose: .meetingMinutes, policy: ModelPolicy()), claude)
        XCTAssertEqual(ModelSelection.resolve(storedKey: claude.id, choices: all, purpose: .meetingMinutes, policy: .localOnly), local)
        XCTAssertEqual(ModelSelection.resolve(storedKey: claude.id, choices: all, purpose: .liveState, policy: ModelPolicy()), local)
        XCTAssertNil(ModelSelection.resolve(storedKey: nil, choices: [openai], purpose: .talk, policy: ModelPolicy()))
    }

    func testRouteKeysAndCodable() throws {
        XCTAssertEqual(claude.route.key, "relay:claude-cli/opus")
        XCTAssertEqual(openai.id, "endpoint:00000000-0000-0000-0000-000000000002")
        let data = try JSONEncoder().encode([local, openai, claude])
        XCTAssertEqual(try JSONDecoder().decode([ModelChoice].self, from: data), [local, openai, claude])
    }

    func testRequestCapsTokensByPurpose() {
        XCTAssertEqual(ModelRequest(purpose: .talk, messages: [], maxTokens: 5000).maxTokens, 800)
        XCTAssertEqual(ModelRequest(purpose: .meetingMinutes, messages: []).maxTokens, 4000)
    }

    func testConsentSummaryNamesRelayAndRegion() {
        XCTAssertEqual(ModelConsentRecord.summary(choice: claude, dataCategories: ["Transkript", "Teilnehmernamen"]),
                       "Transkript, Teilnehmernamen gehen über deinen PC an Anthropic und werden dort mit Claude (Abo) verarbeitet.")
        XCTAssertTrue(ModelConsentRecord.summary(choice: openai, dataCategories: ["Transkript"]).contains("(Verarbeitung: USA)"))
    }

    func testCompleteCollectsStream() async throws {
        struct Fake: ModelProvider {
            let choice: ModelChoice
            func stream(_ request: ModelRequest) -> AsyncThrowingStream<ModelEvent, Error> {
                AsyncThrowingStream { c in c.yield(.delta("Hal")); c.yield(.delta("lo")); c.yield(.done(nil)); c.finish() }
            }
        }
        let f = Fake(choice: local)
        let text = try await f.complete(ModelRequest(purpose: .talk, messages: [ModelMessage(role: .user, text: "x")]))
        XCTAssertEqual(text, "Hallo")
        let n = await f.boundedTokens("äb")
        XCTAssertEqual(n, 3)
    }
}
