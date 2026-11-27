import Foundation
import XCTest
@testable import VoiceCore

final class SpeakerScriptTests: XCTestCase {
    private let speakers = [SpeakerScript.Speaker(id: "anna", name: "Anna"),
                            SpeakerScript.Speaker(id: "father", name: "German Father"),
                            SpeakerScript.Speaker(id: "narrator", name: "Erzähler")]

    func testRustGolden() throws {
        // Eine Datei für beide Sprachen; kein zweites Swift-Soll, das unbemerkt abweichen könnte.
        let url = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
            .appendingPathComponent("../../../tests/speaker-script-golden.json").standardizedFileURL
        let root = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: Any])
        for row in try XCTUnwrap(root["cases"] as? [[String: Any]]) {
            let text = try XCTUnwrap(row["text"] as? String)
            let expected = try XCTUnwrap(row["expected"] as? [String: Any])
            let actual: [String: Any] = [
                "tags": SpeakerScript.tagSpans(text).map { [$0.location, NSMaxRange($0)] },
                "markers": SpeakerScript.scanMarkerCandidates(text).map {
                    ["start": $0.range.location, "end": NSMaxRange($0.range), "name": $0.name,
                     "style": $0.style as Any? ?? NSNull()] as [String: Any]
                },
                "segments": SpeakerScript.splitSpeakerSegments(text, speakers: speakers).map {
                    ["text": $0.text, "voice": $0.voice as Any? ?? NSNull(),
                     "style": $0.style as Any? ?? NSNull()] as [String: Any]
                }
            ]
            XCTAssertEqual(try canonical(actual), try canonical(expected))
        }
        for row in try XCTUnwrap(root["names"] as? [[String: String]]) {
            XCTAssertEqual(SpeakerScript.foldSpeakerName(try XCTUnwrap(row["input"])), row["folded"])
        }
        for row in try XCTUnwrap(root["pauses"] as? [[String: Any]]) {
            let actual = SpeakerScript.pauseMillis(try XCTUnwrap(row["input"] as? String))
            XCTAssertEqual(actual, row["millis"] as? Int)
        }
    }

    private func canonical(_ value: [String: Any]) throws -> Data {
        try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
    }

    func testRawOffsetsAndInheritedSpeaker() {
        let raw = "😀 <German Father>[calm] Hallo.[break]\n\nWeiter. <Fremd> Ende."
        let script = SpeakerScript(raw, speakers: speakers)
        let paragraphs = LibraryText.paragraphs(raw)
        let second = script.presentation(in: NSRange(location: paragraphs[1].textStart, length: paragraphs[1].textLength))
        XCTAssertEqual(second.sections.first?.speakerName, "German Father")
        XCTAssertEqual(second.sections.last?.speakerName, "Fremd")
        XCTAssertEqual(second.sections.last?.known, false)
        XCTAssertEqual(second.hiddenRanges.map { (paragraphs[1].text as NSString).substring(with: $0) }, ["<Fremd>"])
        XCTAssertEqual(paragraphs[1].textStart, (raw as NSString).range(of: "Weiter.").location)
        XCTAssertEqual(script.segments.last?.voice, nil)
    }

    func testRosterAliasesDuplicatesAndOwnVoices() {
        let roster = SpeakerScript.roster([
            .init(id: "default", label: "Standard"), .init(id: "a", label: "Anna – ruhig"),
            .init(id: "b", label: "Anna – hell"), .init(id: "Meine Stimme", label: "Meine Stimme")])
        XCTAssertEqual(roster.map(\.name), ["Anna", "Anna (b)", "Meine Stimme"])
        XCTAssertEqual(SpeakerScript.splitSpeakerSegments("<a> Eins. <Anna (b)> Zwei. <Meine Stimme> Drei.", speakers: roster).map(\.voice), ["a", "b", "Meine Stimme"])
    }

    func testTagsDoNotEndSentencesAndPausesAreOrdered() {
        let raw = "<Anna>[tired. very tired.] Hallo [break][pause] Welt. <Unbekannt> Ende."
        let ranges = SpeakerScript.sentenceRanges(raw, speakers: speakers)
        XCTAssertFalse(ranges.contains { NSMaxRange($0) < (raw as NSString).range(of: "Hallo").location })
        let parts = SpeakerScript(raw, speakers: speakers).speechParts(fallbackVoice: "book")
        XCTAssertEqual(parts.map(\.text), ["Hallo", "", "", "Welt.", "Ende."])
        XCTAssertEqual(parts.map(\.pauseMillis), [0, 700, 500, 0, 0])
        XCTAssertEqual(parts.map(\.voice), ["anna", "anna", "anna", "anna", "book"])
        XCTAssertEqual(SpeakerScript.sentenceRanges("Ein normaler Satz. Danach weiter.", speakers: speakers),
                       LibraryText.sentences("Ein normaler Satz. Danach weiter."))
    }

    func testRawWordOffsetsSurviveRemovedTagsAndSurrogatePairs() {
        let raw = "<Anna>😀 Hallo [calm] schöne Welt.[break]"
        let part = SpeakerScript(raw, speakers: speakers).speechParts(fallbackVoice: "book")[0]
        let word = (part.text as NSString).range(of: "schöne")
        let start = part.sourceOffsets[word.location]
        let end = part.sourceOffsets[NSMaxRange(word) - 1] + 1
        XCTAssertEqual(NSRange(location: start, length: end - start), (raw as NSString).range(of: "schöne"))
        XCTAssertEqual(part.sourceOffsets.count, part.text.utf16.count)
        XCTAssertFalse(part.text.contains("calm"))
        XCTAssertTrue(SpeakerScript("[<Anna>]", speakers: speakers).speechParts(fallbackVoice: "book").isEmpty)
        let nested = "[<Anna>] Danach."
        let ranges = SpeakerScript.sentenceRanges(nested, speakers: speakers)
        XCTAssertFalse(ranges.contains { $0.location > 0 && $0.location < 8 })
        let spoken = ranges.flatMap { range in
            SpeakerScript((nested as NSString).substring(with: range), speakers: speakers).speechParts(fallbackVoice: "book")
        }
        XCTAssertEqual(spoken.map(\.text), ["Danach."])
    }

    func testLegacyMarkersOnlyConsumeKnownNamesAndUnknownResetsStyle() {
        let parts = SpeakerScript("Anna: Eins.\nAchtung: bleibt.\n<Anna:ruhig> Zwei. <Fremd:laut> Drei.", speakers: speakers).segments
        XCTAssertEqual(parts.map(\.text), ["Eins.\nAchtung: bleibt.", "Zwei.", "Drei."])
        XCTAssertEqual(parts.map(\.voice), ["anna", "anna", nil])
        XCTAssertEqual(parts.map(\.style), [nil, "ruhig", nil])
        XCTAssertEqual(SpeakerScript.scanMarkerCandidates("<" + String(repeating: "😀", count: 60) + ">").count, 1)
        XCTAssertTrue(SpeakerScript.scanMarkerCandidates("<" + String(repeating: "😀", count: 61) + ">").isEmpty)
    }

    func testSentenceCutMidLineIgnoresLegacyMarkerInFirstLine() {
        // Satz aus „Er sagte. Anna: kommt.“: mitten in der Zeile kein Sprecherwechsel (Codex-Befund #954).
        let cut = SpeakerScript("Anna: kommt.", speakers: speakers, startsAtLineStart: false)
        XCTAssertTrue(cut.markers.isEmpty)
        XCTAssertEqual(cut.speechParts(fallbackVoice: "buch", initialVoice: "father").map(\.voice), ["father"])
        // Folgezeilen und Spitzklammer-Marker gelten weiter.
        let next = SpeakerScript("Anna: kommt.\nAnna: jetzt. <Erzähler> Ende.", speakers: speakers, startsAtLineStart: false)
        XCTAssertEqual(next.segments.map(\.voice), [nil, "anna", "narrator"])
        // Am Zeilenanfang wie bisher.
        XCTAssertEqual(SpeakerScript("Anna: kommt.", speakers: speakers).segments.map(\.voice), ["anna"])
    }

    func testTranslationProtectsMarkersAndTags() async throws {
        let original = "<Anna:ruhig> Hallo [break] Welt. <Unbekannt> Ende."
        let result = try await SpeakerScript.translate([original]) { texts in texts.map { "EN:" + $0 } }
        XCTAssertEqual(result, ["<Anna:ruhig>EN: Hallo [break]EN: Welt. <Unbekannt>EN: Ende."])
        let plain = try await SpeakerScript.translate(["Ohne Marker"]) { $0.map { "EN:" + $0 } }
        XCTAssertEqual(plain, ["EN:Ohne Marker"])
    }

    func testTranslationUsesTargetVoicesAndFallsBackWithoutTargets() {
        let script = SpeakerScript("<German Father> Eins. <Anna> Zwei. <Fremd> Drei.", speakers: speakers)
        let translated = script.translatedSpeakers(targets: [.init(id: "english", name: "English Voice"), speakers[0]], fallbackVoice: "book")
        let parts = SpeakerScript(script.text, speakers: translated).speechParts(fallbackVoice: "book")
        XCTAssertEqual(parts.map(\.voice), ["english", "anna", "book"])
        let missing = script.translatedSpeakers(targets: [], fallbackVoice: "book")
        XCTAssertEqual(SpeakerScript(script.text, speakers: missing).speechParts(fallbackVoice: "book").map(\.voice), ["book", "book", "book"])
    }

    func testTranslationRejectsMissingAndEmptyPieces() async {
        for empty in [false, true] {
            do {
                _ = try await SpeakerScript.translate(["<Anna> Hallo."]) { _ in empty ? [""] : [] }
                XCTFail("Unbrauchbare Übersetzung angenommen")
            } catch { XCTAssertEqual(error as? LiveTranslationError, empty ? .unusable : .countMismatch) }
        }
    }
}
