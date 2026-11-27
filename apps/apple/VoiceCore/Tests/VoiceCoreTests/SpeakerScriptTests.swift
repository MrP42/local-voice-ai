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

    func testDisplayHidesMarkersAndTagsAndMapsRawRanges() {
        let text = "<German Father> Emilia und Mara.[break]\n\n<Erzähler> [calm] Hinter den Häusern.\n\n<Erzähler> Die Erwachsenen. <Anna> Hallo!"
        let script = SpeakerScript(text, speakers: speakers)
        let all = text as NSString
        func display(_ paragraph: String) -> SpeakerScriptDisplay {
            let range = all.range(of: paragraph)
            return SpeakerScriptDisplay(paragraph: paragraph, presentation: script.presentation(in: range))
        }
        let first = display("<German Father> Emilia und Mara.[break]")
        XCTAssertEqual(first.text, "Emilia und Mara.")
        XCTAssertEqual(first.leadName, "German Father")
        XCTAssertTrue(first.leadKnown)
        XCTAssertTrue(first.inlineLabels.isEmpty)

        let second = display("<Erzähler> [calm] Hinter den Häusern.")
        XCTAssertEqual(second.text, "Hinter den Häusern.")
        XCTAssertEqual(second.leadName, "Erzähler")

        let raw3 = "<Erzähler> Die Erwachsenen. <Anna> Hallo!"
        let third = display(raw3)
        XCTAssertEqual(third.text, "Die Erwachsenen. Anna\u{00A0}· Hallo!")
        XCTAssertEqual(third.leadName, "Erzähler")
        XCTAssertEqual(third.lastName, "Anna")
        XCTAssertEqual(third.inlineLabels.map(\.name), ["Anna"])
        XCTAssertEqual(third.accessibilityText, "Die Erwachsenen. Sprecher Anna: Hallo!")
        let shown = third.text as NSString
        XCTAssertEqual(third.inlineLabels.first?.range, shown.range(of: "Anna\u{00A0}· "))
        // Satz- und Wortmarkierung aus dem Rohtext landen auf demselben Text.
        let hallo = (raw3 as NSString).range(of: "Hallo!")
        XCTAssertEqual(third.map(hallo), shown.range(of: "Hallo!"))
        let erwachsene = (raw3 as NSString).range(of: "Die Erwachsenen.")
        XCTAssertEqual(third.map(erwachsene), shown.range(of: "Die Erwachsenen."))
        // Ein Bereich nur aus Marker verschwindet.
        XCTAssertNil(third.map((raw3 as NSString).range(of: "<Anna>")))
    }

    func testDisplayWithoutHiddenRangesIsIdentity() {
        let script = SpeakerScript("<Anna> Eins.\n\nZwei.", speakers: speakers)
        let range = ("<Anna> Eins.\n\nZwei." as NSString).range(of: "Zwei.")
        let display = SpeakerScriptDisplay(paragraph: "Zwei.", presentation: script.presentation(in: range))
        XCTAssertTrue(display.isIdentity)
        XCTAssertEqual(display.text, "Zwei.")
        XCTAssertEqual(display.leadName, "Anna")  // geerbt
        XCTAssertEqual(display.map(NSRange(location: 0, length: 5)), NSRange(location: 0, length: 5))
    }

    func testTranslationProtectsMarkersAndTags() async throws {
        let original = "<Anna:ruhig> Hallo [break] Welt. <Unbekannt> Ende."
        let result = try await SpeakerScript.translate([original]) { texts in texts.map { "EN:" + $0 } }
        XCTAssertEqual(result, ["<Anna:ruhig>EN: Hallo [break]EN: Welt. <Unbekannt>EN: Ende."])
        let plain = try await SpeakerScript.translate(["Ohne Marker"]) { $0.map { "EN:" + $0 } }
        XCTAssertEqual(plain, ["EN:Ohne Marker"])
    }

    func testTranslatedDisplayInheritsSourceSpeakerAndAllowsLocalChanges() throws {
        let source = "<German Father> Emilia und Mara.\n\nHinter den Häusern.\n\n<Fremd> Weiter."
        let script = SpeakerScript(source, speakers: speakers)
        let offset = (source as NSString).range(of: "Hinter").location
        let inherited = try XCTUnwrap(script.display(translated: "Behind the houses.", at: offset))
        XCTAssertEqual(inherited.leadName, "German Father")
        XCTAssertTrue(inherited.leadKnown)
        XCTAssertEqual(inherited.text, "Behind the houses.")
        let changed = try XCTUnwrap(script.display(translated: "Behind. <Anna> Hello!", at: offset))
        XCTAssertEqual(changed.leadName, "German Father")
        XCTAssertEqual(changed.lastName, "Anna")
        XCTAssertEqual(changed.text, "Behind. Anna\u{00A0}· Hello!")
        let reset = try XCTUnwrap(script.display(translated: "<Anna> Hello!", at: offset))
        XCTAssertEqual(reset.leadName, "Anna")
        let unknown = try XCTUnwrap(script.display(translated: "Continue.", at: source.utf16.count - 1))
        XCTAssertEqual(unknown.leadName, "Fremd")
        XCTAssertFalse(unknown.leadKnown)
        XCTAssertNil(SpeakerScript("Ordinary book [1].").display(translated: "Book [1].", at: 0))
    }

    func testDisplayUsesLastMarkerInMergedHiddenRange() {
        for tail in ["<Anna><German Father>", "<Anna> [calm] <German Father>", "[<Anna><German Father>]"] {
            let raw = "<Erzähler> Anfang. " + tail + " Weiter 😀."
            let script = SpeakerScript(raw, speakers: speakers)
            let display = SpeakerScriptDisplay(paragraph: raw,
                presentation: script.presentation(in: NSRange(location: 0, length: raw.utf16.count)))
            XCTAssertEqual(display.text, "Anfang. German Father · Weiter 😀.")
            XCTAssertEqual(display.inlineLabels.map(\.name), ["German Father"])
            XCTAssertEqual(display.lastName, "German Father")
            XCTAssertEqual(display.map((raw as NSString).range(of: "Weiter 😀.")),
                           (display.text as NSString).range(of: "Weiter 😀."))
        }
    }

    func testTranslatedDisplayWithEmptyText() {
        // Gegenprobe: ohne Prüfung der leeren Abschnitte greift display auf sections[0] zu.
        XCTAssertNil(SpeakerScript("<Anna> Hallo.").display(translated: "", at: 0))
    }

    func testTranslatedDisplayWithEmptyMarkdownHeading() throws {
        let source = "<Anna> Hallo.\n\n# "
        let heading = try XCTUnwrap(LibraryText.paragraphs(source).last)
        XCTAssertTrue(heading.heading)
        XCTAssertEqual(heading.text, "")
        // Gegenprobe: der leere Überschriftentext führte ebenfalls zu sections[0].
        XCTAssertNil(SpeakerScript(source, speakers: speakers).display(translated: heading.text, at: heading.textStart))
    }

    func testPresentationClipsPrecomputedSectionsAtUTF16Boundaries() {
        let raw = "😀 <Anna> Eins. <Fremd> Zwei. <German Father> Drei."
        let script = SpeakerScript(raw, speakers: speakers)
        let range = (raw as NSString).range(of: "Eins. <Fremd> Zwei.")
        let presentation = script.presentation(in: range)
        XCTAssertEqual(presentation.sections, [
            .init(range: NSRange(location: 0, length: 6), speakerName: "Anna", known: true),
            .init(range: NSRange(location: 6, length: 13), speakerName: "Fremd", known: false)])
        XCTAssertEqual(presentation.hiddenRanges, [NSRange(location: 6, length: 7)])
        XCTAssertTrue(script.presentation(in: NSRange(location: raw.utf16.count, length: 0)).sections.isEmpty)
        XCTAssertTrue(script.presentation(in: NSRange(location: raw.utf16.count, length: 4)).sections.isEmpty)
    }

    func testPresentationFindsOverlappingTagThatStartsBeforeParagraph() {
        let raw = "[<Anna><German Father> hidden] Ende."
        let range = (raw as NSString).range(of: "hidden")
        let script = SpeakerScript(raw, speakers: speakers)
        let presentation = script.presentation(in: range)
        XCTAssertEqual(presentation.hiddenRanges, [NSRange(location: 0, length: 6)])
        XCTAssertEqual(presentation.sections.first?.speakerName, "German Father")
    }

    func testRepeatedPresentationsDoNotRescanAllMarkers() {
        let paragraph = "<German Father> Synthetischer Text.\n\n"
        let script = SpeakerScript(String(repeating: paragraph, count: 8_000), speakers: speakers)
        let expected = SpeakerScript.Presentation(
            hiddenRanges: [NSRange(location: 0, length: 15)],
            sections: [.init(range: NSRange(location: 0, length: 34), speakerName: "German Father", known: true)])
        // Je höchstens 13 binäre Vergleiche für Abschnitte und verdeckte Bereiche, dazu lokale Treffer.
        let visitLimit = 32
        for i in 0..<8_000 {
            var visits = 0
            let presentation = script.presentation(in: NSRange(location: i * paragraph.utf16.count, length: 34),
                                                   visit: { visits += 1 })
            XCTAssertEqual(presentation, expected)
            XCTAssertGreaterThan(visits, 0)
            XCTAssertLessThanOrEqual(visits, visitLimit)
        }

        // Gegenprobe: vollständiger Scan wie vor dem Abschnittsindex, am letzten Absatz.
        let range = NSRange(location: 7_999 * paragraph.utf16.count, length: 34)
        var naiveVisits = 0
        func local(_ candidate: NSRange) -> NSRange? {
            naiveVisits += 1
            let overlap = NSIntersectionRange(candidate, range)
            return overlap.length > 0 ? NSRange(location: overlap.location - range.location, length: overlap.length) : nil
        }
        let starts = [0] + script.markers.map(\.range.location)
        let sections: [SpeakerScript.Section] = starts.enumerated().compactMap { i, start in
            let end = i + 1 < starts.count ? starts[i + 1] : script.text.utf16.count
            guard let clipped = local(NSRange(location: start, length: end - start)) else { return nil }
            let marker = i > 0 ? script.markers[i - 1] : nil
            return .init(range: clipped, speakerName: marker?.name, known: marker != nil)
        }
        XCTAssertEqual(SpeakerScript.Presentation(hiddenRanges: script.hiddenRanges.compactMap(local), sections: sections), expected)
        XCTAssertGreaterThan(naiveVisits, visitLimit)
    }

    func testTranslationKeepsMidLineLegacyNameButProtectsRealLineStart() async throws {
        let source = "Er sagte. Anna: kommt.\n  Anna: jetzt."
        let pieces = ["Anna: kommt.", "Anna: jetzt."]
        let ranges = pieces.map { (source as NSString).range(of: $0) }
        let units = LiveTranslation.units(paragraph: 0, text: source, sentences: ranges)
        XCTAssertEqual(units.map(\.startsAtLineStart), [false, true])
        let result = try await SpeakerScript.translate(pieces, speakers: speakers,
            startsAtLineStart: units.map(\.startsAtLineStart)) { $0.map { "EN:" + $0 } }
        XCTAssertEqual(result, ["EN:Anna: kommt.", "Anna:EN: jetzt."])
    }

    func testIdenticalSentenceTextsKeepTheirOwnSourceLineContext() async throws {
        let source = "Er sagte. Anna: kommt.\nAnna: kommt." as NSString
        let ranges = [source.range(of: "Anna: kommt."), source.range(of: "Anna: kommt.", options: .backwards)]
        let units = LiveTranslation.units(paragraph: 0, text: source as String, sentences: ranges)
        let result = try await SpeakerScript.translate(units.map(\.text), speakers: speakers,
            startsAtLineStart: units.map(\.startsAtLineStart)) { $0.map { "EN:" + $0 } }
        XCTAssertEqual(result, ["EN:Anna: kommt.", "Anna:EN: kommt."])
    }

    func testTranslationRejectsMismatchedLineContext() async {
        do {
            _ = try await SpeakerScript.translate(["Anna: kommt."], startsAtLineStart: []) { $0 }
            XCTFail("Fehlender Quellkontext angenommen")
        } catch { XCTAssertEqual(error as? LiveTranslationError, .countMismatch) }
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
