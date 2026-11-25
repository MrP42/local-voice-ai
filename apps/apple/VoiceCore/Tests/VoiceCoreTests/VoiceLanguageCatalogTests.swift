import XCTest
@testable import VoiceCore

// Gegenproben zu lva#779: synthetische Stimmen, keine Geräte- oder Buchinhalte.
final class VoiceLanguageCatalogTests: XCTestCase {
    func testAndereSystemsprachenWerdenGruppiertUndLesespracheBleibtOben() {
        let groups = VoiceLanguageGrouping.group(
            voices: ["fr-FR", "de-DE", "en-US", "fr-CA"],
            readingLanguage: "de", locale: Locale(identifier: "de"), language: { $0 }
        )
        XCTAssertEqual(groups.map(\.code), ["en", "fr"])
        XCTAssertEqual(groups.map(\.name), ["Englisch", "Französisch"])
        XCTAssertEqual(groups.last?.voices, ["fr-FR", "fr-CA"])
        let english = VoiceLanguageGrouping.group(
            voices: ["de-DE"], readingLanguage: nil,
            locale: Locale(identifier: "en"), language: { $0 }
        )
        XCTAssertEqual(english.first?.name, "German")
    }

    func testGeraetBestimmtAngebotMitHaeufigenZuerstUndRestNachName() {
        let list = LiveLanguageCatalog.offered(
            supportedCodes: ["sv-SE", "fi-FI", "fr-CA", "en-US", "fr-FR", "eng"],
            locale: Locale(identifier: "de")
        )
        XCTAssertEqual(list.map(\.code), ["en", "fr", "fi", "sv"])
        XCTAssertEqual(Array(list.suffix(2)).map(\.name), ["Finnisch", "Schwedisch"])
        XCTAssertFalse(list.contains { $0.code == "de" }, "kein festes Zusatzangebot")
        XCTAssertNotEqual(LiveLanguages.name("sv"), "SV", "auch neue Sprachen haben Anzeigenamen")
    }

    func testRueckfallNurBeiLeererGeraetelisteUndSchriftenBleibenGetrennt() {
        XCTAssertEqual(
            LiveLanguageCatalog.offered(supportedCodes: [], locale: Locale(identifier: "de"))
                .map(\.code), LiveLanguages.offered.map(\.code)
        )
        let list = LiveLanguageCatalog.offered(
            supportedCodes: ["zh_Hant_TW", "zh-Hans-CN", "zh-Hans-SG"],
            locale: Locale(identifier: "de")
        )
        XCTAssertEqual(list.map(\.code), ["zh-hans", "zh-hant"])
    }

    func testChinesischOhneSchriftangabeWirdNachRegionGetrennt() {
        let list = LiveLanguageCatalog.offered(supportedCodes: ["zh", "zh-TW", "en"], locale: Locale(identifier: "de"))
        XCTAssertEqual(list.map(\.code), ["en", "zh-hans", "zh-hant"])
        XCTAssertEqual(list.filter { $0.code.hasPrefix("zh") }.map(\.name),
                       ["Chinesisch (vereinfacht)", "Chinesisch (traditionell)"])
        XCTAssertEqual(LiveLanguageCatalog.liveCode("zh_HK"), "zh-hant")
        XCTAssertEqual(LiveLanguageCatalog.liveCode("zh-CN"), "zh-hans")
        XCTAssertEqual(LiveLanguageCatalog.liveCode("zh-Hant"), "zh-hant")
        XCTAssertEqual(LiveLanguageCatalog.liveCode("fr-CA"), "fr")
    }

    func testRueckkehrknopfZeigtZurStelle() {
        XCTAssertEqual(FollowDirection.of(current: 3, visible: [10, 11, 12]), .up)
        XCTAssertEqual(FollowDirection.of(current: 20, visible: [10, 11, 12]), .down)
        XCTAssertEqual(FollowDirection.of(current: 11, visible: [12, 10, 11]), .down)
        XCTAssertEqual(FollowDirection.of(current: 0, visible: []), .down)
    }

    func testFesteBeispieleFolgenDerStimmenspracheMitEnglischemRueckfall() {
        XCTAssertEqual(SampleSentences.sentence(for: "de-DE"),
                       "Guten Tag. So klingt diese Stimme beim Vorlesen.")
        XCTAssertEqual(SampleSentences.sentence(for: "fra"),
                       "Bonjour. Voici ma voix pour la lecture.")
        XCTAssertEqual(SampleSentences.sentence(for: "xx"),
                       SampleSentences.sentence(for: "en-GB"))
        XCTAssertNotEqual(SampleSentences.sentence(for: "ja"),
                          SampleSentences.sentence(for: "en"))
    }

    func testNeueProbeErsetztAlteUndSpaetesEndeWirdVerworfen() throws {
        var state = VoicePreviewState()
        state.handle(.toggle("erste"))
        let old = try XCTUnwrap(state.active)
        state.handle(.toggle("zweite"))
        let new = try XCTUnwrap(state.active)
        XCTAssertEqual(new.identifier, "zweite")
        state.handle(.finished(old.token))
        XCTAssertEqual(state.active, new)
        state.handle(.finished(new.token))
        XCTAssertNil(state.active)
    }

    func testStoppWahlUndSchliessenBeendenProbeAuchBeiErneutGleicherStimme() throws {
        var state = VoicePreviewState()
        state.handle(.toggle("stimme"))
        let old = try XCTUnwrap(state.active)
        state.handle(.toggle("stimme"))
        XCTAssertNil(state.active, "zweiter Tipp stoppt")
        state.handle(.toggle("stimme"))
        state.handle(.finished(old.token))
        XCTAssertNotNil(state.active, "alte Marke gilt auch bei gleicher Stimme nicht")
        state.handle(.selected)
        XCTAssertNil(state.active)
        state.handle(.toggle("stimme"))
        state.handle(.dismissed)
        XCTAssertNil(state.active)
    }
}
