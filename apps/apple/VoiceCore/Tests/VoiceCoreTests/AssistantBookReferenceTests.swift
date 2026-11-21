import XCTest
@testable import VoiceCore

final class AssistantBookReferenceTests: XCTestCase {
    private let books = [
        AssistantItem(area: .books, id: "b1", title: "Der Zauberberg"),
        AssistantItem(area: .books, id: "b2", title: "Faust"),
    ]

    func testMissingReferenceAsksWithoutGuessingEvenWithOneBook() {
        for question in ["Um was geht es im Buch?", "What is this book about?"] {
            let result = AssistantBookReference.resolve(question: question, books: [books[0]])
            XCTAssertEqual(result, .ask)
            XCTAssertEqual(result.clarification(), "Welches Buch meinst du?")
            XCTAssertEqual(result.clarification(english: true), "Which book do you mean?")
        }
        XCTAssertEqual(AssistantBookReference.resolve(question: "Wie ist das Wetter?", books: books),
                       .none)
    }

    func testAnswerPrecedesOpenedBookAndExplicitTitleReplacesBoth() {
        for question in ["Und wie geht es darin weiter?", "What happens in this book?"] {
            XCTAssertEqual(AssistantBookReference.resolve(
                question: question, books: books, lastOpenedBookId: "b2",
                lastAnswerBookIds: ["b1"]
            ), .book("b1"))
        }
        XCTAssertEqual(AssistantBookReference.resolve(
            question: "Erzähl mir von ZAUBERBERG!", books: books, lastOpenedBookId: "b2"
        ), .book("b1"))
        XCTAssertEqual(AssistantBookReference.resolve(
            question: "Anderes Thema: Faust", books: books, lastAnswerBookIds: ["b1"]
        ), .book("b2"))
        XCTAssertEqual(AssistantBookReference.resolve(
            question: "Anderes Buch: worum geht es im Buch?", books: books,
            lastOpenedBookId: "b1", lastAnswerBookIds: ["b1"]
        ), .ask)
        XCTAssertEqual(AssistantBookReference.resolve(
            question: "Worum geht es im Buch?", books: books, lastOpenedBookId: "b2"
        ), .book("b2"))
    }

    func testAmbiguousTitlesAndStaleOrMultipleSourcesAsk() {
        let ambiguous = books + [AssistantItem(area: .books, id: "b3", title: "Faust II")]
        XCTAssertEqual(AssistantBookReference.resolve(
            question: "Was steht in Faust?", books: ambiguous, lastOpenedBookId: "b2"
        ), .book("b2"))
        XCTAssertEqual(AssistantBookReference.resolve(
            question: "Was steht in Faust II?", books: ambiguous, lastAnswerBookIds: ["b1"]
        ), .book("b3"))
        let duplicate = books + [AssistantItem(area: .books, id: "b4", title: "Faust")]
        XCTAssertEqual(AssistantBookReference.resolve(
            question: "Was steht in Faust?", books: duplicate, lastOpenedBookId: "b2"
        ), .ask)
        for ids in [Set(["b1", "b2"]), Set(["deleted"])] {
            XCTAssertEqual(AssistantBookReference.resolve(
                question: "Und was steht darin?", books: books, lastOpenedBookId: "b1",
                lastAnswerBookIds: ids
            ), .ask)
        }
    }

    func testTitlelessQuestionsKeepAnswerBookUnlessTheTopicExplicitlyChanges() {
        for question in [
            "Wie endet es?", "Erzähl mehr", "und dann?", "Tell me more", "How does it end?",
            "Erkläre bitte ausführlich die Entwicklung der Figuren und ihre Beziehungen zueinander",
        ] {
            XCTAssertEqual(AssistantBookReference.resolve(
                question: question, books: books, lastOpenedBookId: "b2",
                lastAnswerBookIds: ["b1"]
            ), .book("b1"), question)
        }
        for question in [
            "Neues Thema: Wetter", "Was steht in der Besprechung?",
            "Was steht in der Notiz?", "Was steht in der Ablage?",
            "Vergleiche das Buch mit der Notiz",
        ] {
            XCTAssertEqual(AssistantBookReference.resolve(
                question: question, books: books, lastAnswerBookIds: ["b1"]
            ), .none, question)
        }
    }

    func testTopicWordDoesNotResolveTitleButSubstantialFragmentsDo() {
        let stock = books + [
            AssistantItem(area: .books, id: "b3", title: "Die Kunst des Krieges"),
        ]
        XCTAssertEqual(AssistantBookReference.resolve(
            question: "Was steht im Buch über Kunst?", books: stock
        ), .ask)
        XCTAssertEqual(AssistantBookReference.resolve(
            question: "Was steht im Buch über Kunst?", books: stock,
            lastAnswerBookIds: ["b1"]
        ), .book("b1"))
        for question in ["Erzähl von der Kunst des Krieges", "Was steht in des Krieges?"] {
            XCTAssertEqual(AssistantBookReference.resolve(
                question: question, books: stock
            ), .book("b3"), question)
        }
        // Ohne Volltitel bleiben gemeinsame lange Titelwörter mehrdeutig.
        let fragments = [
            AssistantItem(area: .books, id: "b4", title: "Zauberberg Chronik"),
            AssistantItem(area: .books, id: "b5", title: "Zauberberg Reise"),
        ]
        XCTAssertEqual(AssistantBookReference.resolve(
            question: "Erzähl von Zauberberg", books: fragments
        ), .ask)
    }

    func testUnresolvedBookMentionKeepsLegacyRetrieval() async {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc(
            "b1", "Der Zauberberg", area: .books,
            sections: [(nil, "Planung im Sanatorium in Davos.")]
        ))
        let knowledge = AssistantKnowledge(corpus: corpus)
        for question in ["Planung Davos?", "Was steht im Buch über Planung in Davos?"] {
            let reference = await knowledge.bookReference(question: question)
            XCTAssertEqual(reference, question.contains("Buch") ? .ask : .none)
            let legacy = await corpus.search(
                question, previous: nil, areas: Set(AssistantArea.allCases), limit: 16
            )
            let expected = AssistantKnowledge.merge(legacy, limit: 8, maxChars: 4_000)
            XCTAssertFalse(expected.isEmpty, question)
            let actual = await knowledge.retrieve(
                question: question, previous: nil, limit: 8, maxChars: 4_000
            )
            XCTAssertEqual(actual, expected, question)
        }
    }

    func testBudgetFollowUpKeepsTheAnswerBookWithoutABookMention() {
        XCTAssertEqual(AssistantBookReference.resolve(
            question: "Und was ist mit Budget und Planung?", books: books,
            lastOpenedBookId: "b2", lastAnswerBookIds: ["b1"]
        ), .book("b1"))
    }

    func testBookAndFollowUpExcludeLexicallyBetterOtherSources() async {
        let corpus = AssistantCorpus()
        for (id, title, area) in [
            ("b1", "Der Zauberberg", AssistantArea.books), ("b2", "Faust", .books),
            ("t1", "Besprechung", .transcripts), ("n1", "Notiz", .notes),
        ] {
            let text = id == "b1" ? "Planung im Sanatorium." : "Planung Budget Budget Aufnahme."
            await corpus.upsert(assistantDoc(id, title, area: area, sections: [(nil, text)]))
        }
        let knowledge = AssistantKnowledge(corpus: corpus)
        for question in ["Planung im Buch Zauberberg?", "Und was ist mit Budget und Planung?"] {
            let hits = await knowledge.retrieve(
                question: question, previous: "Faust Budget Aufnahme", limit: 8, maxChars: 4_000,
                lastOpenedBookId: "b2", lastAnswerBookIds: ["b1"]
            )
            XCTAssertFalse(hits.isEmpty)
            XCTAssertTrue(hits.allSatisfy { $0.area == .books && $0.itemId == "b1" })
        }
        for question in ["Wie endet es?", "Erzähl mehr", "und dann?"] {
            let hits = await knowledge.retrieve(
                question: question, previous: "Planung", limit: 8, maxChars: 4_000,
                lastOpenedBookId: "b2", lastAnswerBookIds: ["b1"]
            )
            XCTAssertFalse(hits.isEmpty, question)
            XCTAssertTrue(hits.allSatisfy { $0.area == .books && $0.itemId == "b1" }, question)
        }
        let profile = AssistantScope(mode: .only, items: [.init(area: .books, itemId: "b2")])
        let restricted = knowledge.withScope(profile)
        let excluded = await restricted.retrieve(
            question: "Planung im Buch Zauberberg?", previous: nil, limit: 8, maxChars: 4_000
        )
        XCTAssertTrue(excluded.isEmpty, "Anfrage erweitert den Profil-Scope nicht")
        let disabled = knowledge.withAreas([.books])
        let meeting = await disabled.retrieve(
            question: "Planung in der Besprechung?", previous: nil, limit: 8, maxChars: 4_000
        )
        XCTAssertTrue(meeting.isEmpty, "genannter gesperrter Bereich bleibt ausgeschlossen")
        let notes = await knowledge.retrieve(
            question: "Planung in meiner Notiz?", previous: nil, limit: 8, maxChars: 4_000
        )
        XCTAssertEqual(notes.map(\.itemId), ["n1"])
    }
}
