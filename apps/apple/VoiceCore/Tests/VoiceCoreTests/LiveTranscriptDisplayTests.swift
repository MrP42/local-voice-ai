import XCTest
@testable import VoiceCore

final class LiveTranscriptDisplayTests: XCTestCase {
    private func seg(_ id: String, _ start: Double, lang: String?, refined: Bool = true, final: Bool = true) -> LiveSegment {
        LiveSegment(id: id, start: start, end: start + 1, text: "Beispiel", isFinal: final, language: lang, refined: refined)
    }

    func testMainLanguageIsMostFrequentRefined() {
        let s = [seg("w1", 0, lang: "en"), seg("w2", 1, lang: "de"), seg("w3", 2, lang: "de"), seg("a1", 3, lang: "en", refined: false)]
        XCTAssertEqual(LiveTranscriptDisplay.mainLanguage(s), "de")
    }

    func testMainLanguageTieTakesEarliest() {
        let s = [seg("w1", 0, lang: "fr"), seg("w2", 1, lang: "de")]
        XCTAssertEqual(LiveTranscriptDisplay.mainLanguage(s), "fr")
    }

    func testMainLanguageNormalizesRegionAndIgnoresAuto() {
        let s = [seg("w1", 0, lang: "de-DE"), seg("w2", 1, lang: "de_AT"), seg("w3", 2, lang: "auto"), seg("w4", 3, lang: "EN")]
        XCTAssertEqual(LiveTranscriptDisplay.mainLanguage(s), "de")
        XCTAssertNil(LiveTranscriptDisplay.mainLanguage([seg("a1", 0, lang: nil)]))
    }

    func testBadgeOnlyForDeviatingLanguage() {
        XCTAssertNil(LiveTranscriptDisplay.badge(for: seg("w1", 0, lang: "de"), main: "de"))
        XCTAssertNil(LiveTranscriptDisplay.badge(for: seg("w1", 0, lang: "de-DE"), main: "de"))
        XCTAssertEqual(LiveTranscriptDisplay.badge(for: seg("w1", 0, lang: "en"), main: "de"), "EN")
        XCTAssertNil(LiveTranscriptDisplay.badge(for: seg("a1", 0, lang: nil), main: "de"))
        XCTAssertNil(LiveTranscriptDisplay.badge(for: seg("w1", 0, lang: "en"), main: nil))
    }

    func testRefinementStates() {
        XCTAssertEqual(LiveTranscriptDisplay.refinement(displayed: [], deferredRanges: 0), .idle)
        XCTAssertEqual(LiveTranscriptDisplay.refinement(displayed: [seg("w1", 0, lang: "de")], deferredRanges: 0), .idle)
        let mixed = [seg("w1", 0, lang: "de"), seg("a1", 1, lang: nil, refined: false), seg("v", 2, lang: nil, refined: false, final: false)]
        XCTAssertEqual(LiveTranscriptDisplay.refinement(displayed: mixed, deferredRanges: 0), .running(provisional: 1))
        XCTAssertEqual(LiveTranscriptDisplay.refinement(displayed: mixed, deferredRanges: 2), .deferred(provisional: 1))
        XCTAssertNil(LiveTranscriptDisplay.statusText(.idle))
        XCTAssertNotNil(LiveTranscriptDisplay.statusText(.running(provisional: 1)))
        XCTAssertNotNil(LiveTranscriptDisplay.statusText(.deferred(provisional: 0)))
    }

    func testLanguageNameGerman() {
        XCTAssertEqual(LiveTranscriptDisplay.languageName("en"), Locale(identifier: "de_DE").localizedString(forLanguageCode: "en"))
    }
}
