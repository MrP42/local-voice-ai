import XCTest
@testable import VoiceCore

final class ModelPolicyProfileTests: XCTestCase {
    func testWithoutManagedUsesUserSettingUnlocked() {
        XCTAssertEqual(ModelPolicyProfile.resolve(managed: nil, userAllowsExternal: false),
                       ModelPolicy(externalAllowed: false, locked: false))
        XCTAssertEqual(ModelPolicyProfile.resolve(managed: [:], userAllowsExternal: true), .localOnly)
    }

    func testManagedWithoutModelPolicyIsLocalOnly() {
        XCTAssertEqual(ModelPolicyProfile.resolve(managed: ["other": 1], userAllowsExternal: true), .localOnly)
    }

    func testManagedProfileIsLockedAndWinsOverUser() {
        let p = ModelPolicyProfile.resolve(managed: ["modelPolicy": ["externalAllowed": false]], userAllowsExternal: true)
        XCTAssertEqual(p, ModelPolicy(externalAllowed: false, allowedVendors: nil, locked: true))
        let q = ModelPolicyProfile.resolve(managed: ["modelPolicy": ["externalAllowed": true, "allowedVendors": [" Anthropic ", ""]]],
                                           userAllowsExternal: false)
        XCTAssertEqual(q, ModelPolicy(externalAllowed: true, allowedVendors: ["Anthropic"], locked: true))
        XCTAssertTrue(q.permits(route: .endpoint(id: UUID()), vendor: "anthropic"))
        XCTAssertFalse(q.permits(route: .endpoint(id: UUID()), vendor: "openai"))
    }

    func testMalformedManagedProfileFailsClosed() {
        for raw: Any in ["ja", ["externalAllowed": "true"], ["externalAllowed": true, "allowedVendors": "OpenAI"]] {
            XCTAssertEqual(ModelPolicyProfile.resolve(managed: ["modelPolicy": raw], userAllowsExternal: true), .localOnly)
        }
    }

    func testExplanationNamesVendorsSorted() {
        let text = ModelPolicyProfile.explanation(ModelPolicy(externalAllowed: true, allowedVendors: ["OpenAI", "Anthropic"], locked: true))
        XCTAssertTrue(text.contains("Anthropic, OpenAI"))
        XCTAssertEqual(ModelPolicyProfile.explanation(.localOnly), "Deine Organisation erlaubt nur Modelle auf dem Gerät.")
    }

    func testUpdatePolicyOnlyOnChange() async {
        let state = ModelAccessState()
        let changed = await state.updatePolicy(.localOnly)
        XCTAssertFalse(changed, "gleich wie .unknown: keine neue Version")
        let opened = await state.updatePolicy(.open)
        XCTAssertTrue(opened)
        let again = await state.updatePolicy(.open)
        XCTAssertFalse(again)
        let current = await state.currentPolicy()
        XCTAssertEqual(current, .open)
    }
}
