import Foundation
import XCTest
import VoiceCore

final class AssistantVocabularyTests: XCTestCase {
    func testCurrentAreasRemoveRevokedTitlesAndAuthors() {
        let items = [
            AssistantItem(area: .books, id: "1", title: "Buch", detail: "Ada Lovelace"),
            AssistantItem(area: .notes, id: "2", title: "Notiz")
        ]
        XCTAssertEqual(
            AssistantVocabulary.contextualStrings(items: items, areas: [.notes], limit: 1),
            ["Notiz"])
        XCTAssertEqual(AssistantVocabulary.contextualStrings(items: items, areas: []), [])
    }

    func testAddedAreaIncludesItsTitlesAndAuthors() {
        let items = [
            AssistantItem(area: .books, id: "1", title: "Buch", detail: "Ada Lovelace"),
            AssistantItem(area: .notes, id: "2", title: "Notiz")
        ]
        XCTAssertEqual(
            AssistantVocabulary.contextualStrings(items: items, areas: [.notes]), ["Notiz"])
        XCTAssertEqual(
            AssistantVocabulary.contextualStrings(items: items, areas: [.books, .notes]),
            ["Buch", "Ada Lovelace", "Notiz"])
    }

    func testAreaChangeInvalidatesPreparationEvenAfterAnotherChange() {
        var generation = AssistantVocabulary.PreparationGeneration()
        let original = generation.value
        XCTAssertTrue(generation.accepts(original))
        generation.invalidate()
        let changed = generation.value
        XCTAssertFalse(generation.accepts(original))
        XCTAssertTrue(generation.accepts(changed))
        generation.invalidate()
        XCTAssertFalse(generation.accepts(original))
        XCTAssertFalse(generation.accepts(changed))
        XCTAssertTrue(generation.accepts(generation.value))
    }

    func testAuthorsWithoutProgressOrOtherDetails() {
        let items = [
            AssistantItem(area: .books, id: "1", title: "Buch",
                          detail: "Ada Lovelace, Grace Hopper · 42 % gelesen"),
            AssistantItem(area: .books, id: "2", title: "Anderes", detail: "12 % gelesen"),
            AssistantItem(area: .notes, id: "3", title: "Notiz", detail: "Geheimer Text")
        ]
        XCTAssertEqual(AssistantVocabulary.contextualStrings(items: items),
                       ["Buch", "Ada Lovelace", "Grace Hopper", "Anderes", "Notiz"])
    }

    func testBooksThenNewestWithStableTiesAndLimit() {
        let items = [
            AssistantItem(area: .notes, id: "1", title: "Neu", updated: Date(timeIntervalSince1970: 9)),
            AssistantItem(area: .books, id: "2", title: "Alt"),
            AssistantItem(area: .books, id: "3", title: "Buch", updated: Date(timeIntervalSince1970: 1)),
            AssistantItem(area: .files, id: "4", title: "Ablage"),
            AssistantItem(area: .transcripts, id: "5", title: "Besprechung")
        ]
        XCTAssertEqual(AssistantVocabulary.contextualStrings(items: items),
                       ["Buch", "Alt", "Neu", "Ablage", "Besprechung"])
        XCTAssertEqual(AssistantVocabulary.contextualStrings(items: items, limit: 2), ["Buch", "Alt"])
        XCTAssertEqual(AssistantVocabulary.contextualStrings(items: items, limit: 0), [])
        XCTAssertEqual(AssistantVocabulary.contextualStrings(items: items, limit: -1), [])
    }

    func testCleaningDuplicatesAndBounds() {
        let titles = [" \n\u{0000}", " Ada\tLovelace\n", "ada lovelace",
                      String(repeating: "Ä", count: 70), String(repeating: "Ä", count: 64)]
        let items = (titles + (0..<110).map { "Titel \($0)" }).enumerated().map {
            AssistantItem(area: .books, id: String($0.offset), title: $0.element)
        }
        let words = AssistantVocabulary.contextualStrings(items: items, limit: 200)
        XCTAssertEqual(Array(words.prefix(3)),
                       ["Ada Lovelace", String(repeating: "Ä", count: 64), "Titel 0"])
        XCTAssertEqual(words.count, 100)
        XCTAssertTrue(words.allSatisfy { $0.count <= 64 })
        XCTAssertEqual(AssistantVocabulary.contextualStrings(items: []), [])
    }
}
