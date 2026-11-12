import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket A, E4.2a): Einwilligung in „Antworten ohne Quelle“.

final class AssistantUnsourcedConsentTests: XCTestCase {
    private var suite: String!
    private var defaults: UserDefaults!

    override func setUp() {
        suite = "assistant-unsourced-\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suite)
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suite)
    }

    func testDefaultIsOff() {
        let consent = AssistantUnsourcedConsent(defaults: defaults)
        XCTAssertFalse(consent.isEnabled)
        XCTAssertEqual(consent.version, 0)
        XCTAssertNil(consent.grantedAt)
        XCTAssertFalse(AssistantConfig(modelLabel: "T", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil, timeout: 5).allowUnsourced,
                       "auch die Konfiguration ist ohne Angabe strikt")
    }

    func testGrantAndRevoke() {
        let consent = AssistantUnsourcedConsent(defaults: defaults)
        let when = Date(timeIntervalSince1970: 1_760_000_000)
        consent.grant(now: when)
        XCTAssertTrue(consent.isEnabled)
        XCTAssertEqual(consent.version, AssistantUnsourcedConsent.currentVersion)
        XCTAssertEqual(consent.grantedAt, when)
        consent.revoke()
        XCTAssertFalse(consent.isEnabled)
        XCTAssertEqual(consent.version, AssistantUnsourcedConsent.currentVersion, "der Eintrag bleibt als Nachweis")
    }

    func testAcknowledgementSurvivesSwitchingOffButNotATextChange() {
        let consent = AssistantUnsourcedConsent(defaults: defaults)
        XCTAssertFalse(consent.isAcknowledged, "nie bestätigt: das Menü fragt")
        consent.grant()
        XCTAssertTrue(consent.isAcknowledged)
        consent.revoke()
        XCTAssertFalse(consent.isEnabled)
        XCTAssertTrue(consent.isAcknowledged, "wieder einschalten ohne neue Rückfrage")
        defaults.set(AssistantUnsourcedConsent.currentVersion - 1, forKey: AssistantUnsourcedConsent.versionKey)
        XCTAssertFalse(AssistantUnsourcedConsent(defaults: defaults).isAcknowledged, "anderer Text: neu fragen")
        defaults.removeObject(forKey: AssistantUnsourcedConsent.versionKey)
        defaults.set(true, forKey: AssistantUnsourcedConsent.enabledKey)
        XCTAssertFalse(AssistantUnsourcedConsent(defaults: defaults).isAcknowledged, "Schalter ohne Version zählt nicht")
    }

    func testConsentToAnOlderTextVersionDoesNotCount() {
        defaults.set(true, forKey: AssistantUnsourcedConsent.enabledKey)
        defaults.set(AssistantUnsourcedConsent.currentVersion - 1, forKey: AssistantUnsourcedConsent.versionKey)
        XCTAssertFalse(AssistantUnsourcedConsent(defaults: defaults).isEnabled)
        defaults.set(true, forKey: AssistantUnsourcedConsent.enabledKey)
        defaults.removeObject(forKey: AssistantUnsourcedConsent.versionKey)
        XCTAssertFalse(AssistantUnsourcedConsent(defaults: defaults).isEnabled, "ein Schalter ohne Version (Migration, Fremdschreiber) schaltet nichts ein")
    }

    func testOtherAssistantSettingsDoNotEnableIt() {
        defaults.set(true, forKey: "assistant_barge_in")
        defaults.set(["relay|books"], forKey: "assistant_cloud_consent")
        XCTAssertFalse(AssistantUnsourcedConsent(defaults: defaults).isEnabled)
    }

    func testConfirmationTextIsTheAgreedOne() {
        XCTAssertEqual(AssistantUnsourcedConsent.confirmationText,
                       "Der Assistent darf dann auch aus dem Wissen des Sprachmodells antworten. Solche Sätze sind nicht durch deine Inhalte belegt und können falsch sein. Sie werden vor dem Sprechen mit ‚Ohne Quelle‘ angesagt und im Verlauf gekennzeichnet. Du kannst das jederzeit ausschalten.")
    }
}
