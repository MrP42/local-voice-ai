import XCTest
@testable import VoiceCore

final class CoverLookupTests: XCTestCase {
    private let json = Data(#"""
    {"docs":[
      {"title":"Die Verwandlung und andere Erzählungen","author_name":["Franz Kafka"],"cover_i":1},
      {"title":"Verwandlung","author_name":["Jemand Anders"],"cover_i":2},
      {"title":"Die Verwandlung","author_name":["Franz Kafka"]},
      {"title":"Die Verwandlung: Roman","author_name":["Kafka, Franz","Franz Kafka"],"cover_i":3}
    ]}
    """#.utf8)

    func testPicksSameTitleAndAuthorWithCover() {
        XCTAssertEqual(CoverLookup.bestCoverId(json, title: "Die Verwandlung", authors: ["Franz Kafka"]), 3)
    }

    func testUnknownAuthorNeedsLongTitle() {
        XCTAssertNil(CoverLookup.bestCoverId(json, title: "Verwandlung", authors: []), "kurzer Titel ohne Autor ist zu unsicher")
        let long = Data(#"{"docs":[{"title":"Schnelles Denken","author_name":["D. K."],"cover_i":9}]}"#.utf8)
        XCTAssertEqual(CoverLookup.bestCoverId(long, title: "Schnelles Denken", authors: []), 9)
    }

    func testUmlautsFoldLikeDesktop() {
        XCTAssertEqual(CoverLookup.coreTitle("Müller"), CoverLookup.coreTitle("Mueller"))
        XCTAssertNotEqual(CoverLookup.coreTitle("Bär"), CoverLookup.coreTitle("Bar"))
    }

    func testCoverSizeGuard() {
        XCTAssertFalse(CoverLookup.plausibleCover(bytes: 43))
        XCTAssertTrue(CoverLookup.plausibleCover(bytes: 90_000))
        XCTAssertFalse(CoverLookup.plausibleCover(bytes: 9 * 1024 * 1024))
    }

    func testNoMatchAndBrokenAnswerGiveNil() {
        XCTAssertNil(CoverLookup.bestCoverId(json, title: "Der Process", authors: ["Franz Kafka"]))
        XCTAssertNil(CoverLookup.bestCoverId(Data("kein json".utf8), title: "x", authors: []))
    }

    func testCoreTitleFoldsLikeDesktop() {
        XCTAssertEqual(CoverLookup.coreTitle("Die Straße: Ein Roman"), CoverLookup.coreTitle("straße"))
        XCTAssertEqual(CoverLookup.coreTitle("Dieb"), "dieb")
        XCTAssertEqual(CoverLookup.coreTitle("Alles Idioten! - Hörbuch"), "allesidioten")
    }

    func testSearchURLCarriesOnlyTitleAndAuthor() throws {
        let url = try XCTUnwrap(CoverLookup.searchURL(title: "Größe & Maß", author: "Ä B"))
        let items = try XCTUnwrap(URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems)
        XCTAssertEqual(items.first { $0.name == "title" }?.value, "Größe & Maß")
        XCTAssertEqual(Set(items.map(\.name)), ["title", "author", "fields", "limit"])
    }
}
