import XCTest
@testable import VoiceCore

// Abschnitte mit Nachweis der Satzgrenzen (Paket A, lva#494, Befund H3): Nähte, Lücken, Kapitel, Lesegrenze.
// Ohne Nachweis (`false`) wird der Randsatz eines Auszugs nie als Originalsatz zitiert.

final class AssistantSectionsTests: XCTestCase {
    // MARK: Naht

    func testSeamIsProvenOnlyByARealSentenceBoundary() {
        XCTAssertTrue(AssistantSeam.proven("Anna übernimmt die Planung.", "Danach folgt die Leitung."))
        XCTAssertTrue(AssistantSeam.proven("Er sagte: „Nein.“", "Dann ging er."), "schließende Anführungszeichen")
        XCTAssertTrue(AssistantSeam.proven("  Das war gut.\n", "\nDann ging es weiter."), "Leerraum am Rand zählt nicht")
        XCTAssertFalse(AssistantSeam.proven("Anna übernimmt", "künftig die Planung."), "kein Satzzeichen")
        XCTAssertFalse(AssistantSeam.proven("Das war gut.", "dann ging es weiter."), "Kleinbuchstabe danach")
        XCTAssertFalse(AssistantSeam.proven("Das gilt z. B.", "Dann gilt es."), "Abkürzung davor")
        XCTAssertFalse(AssistantSeam.proven("", "Dann."), "leerer Text")
        XCTAssertFalse(AssistantSeam.proven("   ", nil), "nur Leerraum")
    }

    func testSeamWithoutSuccessorNeedsTheSentenceEndAtTheTextItself() {
        XCTAssertTrue(AssistantSeam.proven("Das ist gut.", nil))
        XCTAssertTrue(AssistantSeam.proven("Mal sehen…", nil))
        XCTAssertTrue(AssistantSeam.proven("Geht das?", nil))
        XCTAssertFalse(AssistantSeam.proven("Das ist gut", nil), "ohne Satzzeichen")
        XCTAssertFalse(AssistantSeam.proven("Das ist z. B.", nil), "Abkürzung am Ende beweist kein Satzende")
        XCTAssertFalse(AssistantSeam.proven("Anna für Projekt Nordlicht", nil))
        XCTAssertTrue(AssistantSeam.proven("Das ist gut.", ""), "leerer Nachfolger zählt wie keiner")
    }

    // MARK: Ganze Einheiten, erzeugter Text

    func testWholeUnitsCarryBothEdgesAndGeneratedTextNone() {
        let whole = AssistantSections.whole("Notiz ohne Punkt", locator: "01.02.2026, 10:00")
        XCTAssertTrue(whole.startsAtBoundary)
        XCTAssertTrue(whole.endsAtBoundary)
        XCTAssertEqual(whole.locator, "01.02.2026, 10:00")
        let generated = AssistantSections.generated("Zusammenfassung: Alles gut.", locator: "Protokoll")
        XCTAssertFalse(generated.startsAtBoundary)
        XCTAssertFalse(generated.endsAtBoundary)
    }

    // MARK: Transkript

    private func seg(_ start: Double, _ body: String, usable: Bool = true) -> AssistantSegment {
        AssistantSegment(start: start, line: body, body: body, usable: usable)
    }

    private func meeting(_ segments: [AssistantSegment], seconds: Double = 60, chars: Int = 900) -> [AssistantDocument.Section] {
        AssistantSections.meeting(segments, seconds: seconds, chars: chars) { "t\(Int($0))" }
    }

    func testMeetingStartAndEndOfTheRecordingAreProvenBySentences() {
        let sections = meeting([seg(0, "Anna übernimmt die Planung."), seg(10, "Danach folgt die Leitung.")])
        XCTAssertEqual(sections.count, 1)
        XCTAssertEqual(sections[0].text, "Anna übernimmt die Planung.\nDanach folgt die Leitung.")
        XCTAssertEqual(sections[0].locator, "t0")
        XCTAssertTrue(sections[0].startsAtBoundary, "Beginn der Aufnahme")
        XCTAssertTrue(sections[0].endsAtBoundary)
    }

    func testMeetingEndWithoutSentencePunctuationIsNotProven() {
        let sections = meeting([seg(0, "Anna übernimmt die Planung."), seg(10, "Danach folgt die Leitung")])
        XCTAssertTrue(sections[0].startsAtBoundary)
        XCTAssertFalse(sections[0].endsAtBoundary, "das Ende der Aufnahme ohne Satzzeichen")
    }

    func testMeetingAfterAGapNeverStartsProven() {
        let sections = meeting([seg(0, "Erster Satz."), seg(5, "geheim", usable: false), seg(10, "Dritter Satz.")])
        XCTAssertEqual(sections.map(\.text), ["Erster Satz.", "Dritter Satz."])
        XCTAssertTrue(sections[0].startsAtBoundary)
        XCTAssertTrue(sections[0].endsAtBoundary, "vor der Lücke endet der Satz am Satzzeichen")
        XCTAssertFalse(sections[1].startsAtBoundary, "was vor der Lücke stand, ist unbekannt")
        XCTAssertTrue(sections[1].endsAtBoundary)
    }

    func testMeetingGapAfterAnUnfinishedSentenceDoesNotProveTheEnd() {
        let sections = meeting([seg(0, "Anna übernimmt"), seg(5, "x", usable: false), seg(10, "Dritter Satz.")])
        XCTAssertFalse(sections[0].endsAtBoundary)
        XCTAssertFalse(sections[1].startsAtBoundary)
    }

    func testMeetingFirstUsableSegmentAfterAnUnusableOneIsNotTheStart() {
        let sections = meeting([seg(0, "Rauschen", usable: false), seg(5, "Anna übernimmt die Planung.")])
        XCTAssertEqual(sections.count, 1)
        XCTAssertFalse(sections[0].startsAtBoundary, "davor lag ein nicht freigegebenes Segment")
        XCTAssertTrue(sections[0].endsAtBoundary)
    }

    func testMeetingLengthSplitAtASentenceSeamProvesBothSides() {
        let sections = meeting([seg(0, "Erster Satz."), seg(70, "Zweiter Satz.")], seconds: 60)
        XCTAssertEqual(sections.count, 2)
        XCTAssertTrue(sections[0].endsAtBoundary)
        XCTAssertTrue(sections[1].startsAtBoundary, "dieselbe Naht beweist beide Seiten")
    }

    func testMeetingLengthSplitInsideASentenceProvesNeitherSide() {
        let sections = meeting([seg(0, "Anna übernimmt"), seg(5, "künftig die Planung.")], chars: 30)
        XCTAssertEqual(sections.count, 2)
        XCTAssertFalse(sections[0].endsAtBoundary, "„Anna übernimmt“ ist kein ganzer Satz")
        XCTAssertFalse(sections[1].startsAtBoundary, "„künftig die Planung.“ ist ein Rest")
    }

    func testMeetingSeamAtAnAbbreviationProvesNothing() {
        let sections = meeting([seg(0, "Das gilt z. B."), seg(5, "Dann gilt es.")], chars: 20)
        XCTAssertEqual(sections.count, 2)
        XCTAssertFalse(sections[0].endsAtBoundary)
        XCTAssertFalse(sections[1].startsAtBoundary)
    }

    func testMeetingWithoutUsableSegmentsHasNoSections() {
        XCTAssertTrue(meeting([]).isEmpty)
        XCTAssertTrue(meeting([seg(0, "a", usable: false)]).isEmpty)
    }

    // MARK: Buch

    private func paragraph(_ tag: String, terminated: Bool = true) -> String {
        let sentences = (1...12).map { "Satz \(tag) \($0) handelt von der Stadt Beispielburg am Fluss." }
        let text = sentences.joined(separator: " ")
        return terminated ? text : String(text.dropLast())
    }

    /// Ein Stück je Absatz (Absätze ~650 Zeichen).
    private func index(_ blocks: [String]) -> BookTalkIndex {
        BookTalkIndex(title: "Beispielbuch", text: blocks.joined(separator: "\n\n"))
    }

    func testBookChunksAreProvenAcrossParagraphsChaptersAndTheBookEnd() {
        let idx = index(["# Kapitel Eins", paragraph("A"), paragraph("B"), "# Kapitel Zwei", paragraph("C"), paragraph("D", terminated: false)])
        XCTAssertEqual(idx.chunks.count, 4)
        let sections = AssistantSections.book(idx.chunks, frontier: nil)
        XCTAssertEqual(sections.count, 4)
        XCTAssertEqual(sections.map(\.startsAtBoundary), [true, true, true, true])
        XCTAssertEqual(sections.map(\.endsAtBoundary), [true, true, true, false], "das Buch endet ohne Satzzeichen")
        XCTAssertEqual(sections[0].locator, "Kapitel Eins")
        XCTAssertEqual(sections[2].locator, "Kapitel Zwei")
    }

    func testBookParagraphWithoutPunctuationInsideAChapterProvesNeitherSide() {
        let idx = index(["# Kapitel Eins", paragraph("A"), paragraph("B", terminated: false), paragraph("C")])
        let sections = AssistantSections.book(idx.chunks, frontier: nil)
        XCTAssertEqual(sections.count, 3)
        XCTAssertTrue(sections[0].endsAtBoundary)
        XCTAssertFalse(sections[1].endsAtBoundary, "Absatz ohne Satzzeichen mitten im Kapitel")
        XCTAssertFalse(sections[2].startsAtBoundary, "der Vorgänger endet ohne Satzzeichen")
        XCTAssertTrue(sections[2].endsAtBoundary)
    }

    func testBookChapterChangeProvesTheEdgeEvenWithoutPunctuation() {
        let idx = index(["# Kapitel Eins", paragraph("A", terminated: false), "# Kapitel Zwei", paragraph("B")])
        let sections = AssistantSections.book(idx.chunks, frontier: nil)
        XCTAssertEqual(sections.count, 2)
        XCTAssertTrue(sections[0].endsAtBoundary, "ein Kapitel endet mit seinem Absatz")
        XCTAssertTrue(sections[1].startsAtBoundary)
    }

    func testBookFrontierInsideAChunkCutsAtASentenceAndProvesTheEnd() {
        let idx = index(["# Kapitel Eins", paragraph("A"), paragraph("B"), paragraph("C")])
        let frontier = idx.chunks[1].start + 300
        let sections = AssistantSections.book(idx.chunks, frontier: frontier)
        XCTAssertEqual(sections.count, 2, "nichts hinter der Lesegrenze")
        XCTAssertTrue(sections[0].endsAtBoundary)
        XCTAssertTrue(sections[1].startsAtBoundary)
        XCTAssertTrue(sections[1].endsAtBoundary, "an der Satzgrenze vor der Lesegrenze gekürzt")
        XCTAssertLessThan(sections[1].text.count, idx.chunks[1].text.count)
        XCTAssertTrue(sections[1].text.hasSuffix("Fluss."))
    }

    func testBookSuccessorBehindTheFrontierIsNotConsultedForTheEdge() {
        let idx = index(["# Kapitel Eins", paragraph("A", terminated: false), paragraph("B")])
        // Lesegrenze genau am Beginn des zweiten Stücks: nur das erste ist sichtbar.
        let sections = AssistantSections.book(idx.chunks, frontier: idx.chunks[1].start)
        XCTAssertEqual(sections.count, 1)
        XCTAssertTrue(sections[0].startsAtBoundary)
        XCTAssertFalse(sections[0].endsAtBoundary, "das Stück entscheidet allein: kein Satzzeichen am Ende")
        let terminated = index(["# Kapitel Eins", paragraph("A"), paragraph("B")])
        let ok = AssistantSections.book(terminated.chunks, frontier: terminated.chunks[1].start)
        XCTAssertEqual(ok.count, 1)
        XCTAssertTrue(ok[0].endsAtBoundary)
    }

    func testBookFrontierBeforeEverythingGivesNoSections() {
        let idx = index(["# Kapitel Eins", paragraph("A")])
        XCTAssertTrue(AssistantSections.book(idx.chunks, frontier: 0).isEmpty)
    }
}
