import XCTest
@testable import VoiceCore

/// Dieselben Fälle wie `apps/local-voice/tests/sprachfassungen-abgleich.spec.ts`.
final class EditionSwitchTests: XCTestCase {
    private let de = ChapterOutline(starts: [0, 100, 200, 300], length: 400)
    private let en = ChapterOutline(starts: [0, 120, 240, 300, 360], length: 480)
    private let groups = [
        AlignGroup(a: [0], b: [0], confirmed: true),
        AlignGroup(a: [1], b: [1], confirmed: true),
        AlignGroup(a: [2], b: [2, 3], confirmed: true),
        AlignGroup(a: [3], b: [4], confirmed: false),
    ]

    func testConfirmedGroupMapsChapterExact() {
        let t = EditionSwitch.mapPosition(from: de, to: en, offset: 150, groups: groups)
        XCTAssertEqual(t.level, .chapter)
        XCTAssertEqual(t.chapters, [1])
        XCTAssertEqual(t.offset, 180, accuracy: 0.001)
    }

    func testOneToManyGroupSpreadsOverTargetChapters() {
        let t = EditionSwitch.mapPosition(from: de, to: en, offset: 250, groups: groups)
        XCTAssertEqual(t.level, .chapter)
        XCTAssertEqual(t.chapters, [2, 3])
        XCTAssertEqual(t.offset, 300, accuracy: 0.001)
    }

    func testUnconfirmedGroupFallsBackToApprox() {
        let t = EditionSwitch.mapPosition(from: de, to: en, offset: 350, groups: groups)
        XCTAssertEqual(t.level, .approx)
        XCTAssertEqual(t.fraction, 0.875, accuracy: 0.0001)
        XCTAssertEqual(t.offset, 420, accuracy: 0.001)
    }

    func testWithoutGroupsIsApprox() {
        XCTAssertEqual(EditionSwitch.mapPosition(from: de, to: en, offset: 150, groups: nil).level, .approx)
    }

    func testReturnPathViaFlippedGroups() {
        let there = EditionSwitch.mapPosition(from: de, to: en, offset: 150, groups: groups)
        let back = EditionSwitch.mapPosition(from: en, to: de, offset: there.offset, groups: EditionSwitch.flipGroups(groups))
        XCTAssertEqual(back.offset, 150, accuracy: 0.001)
    }

    func testValidGroupsRejectsGapsAndDoubleAssignment() {
        XCTAssertTrue(EditionSwitch.validGroups(groups))
        XCTAssertFalse(EditionSwitch.validGroups([AlignGroup(a: [0], b: [0], confirmed: true),
                                                   AlignGroup(a: [2], b: [1], confirmed: true)]))
        XCTAssertFalse(EditionSwitch.validGroups([AlignGroup(a: [0], b: [0], confirmed: true),
                                                   AlignGroup(a: [0], b: [1], confirmed: true)]))
    }

    func testProposeGroupsConfirmAll() {
        XCTAssertTrue(EditionSwitch.canConfirmAll(EditionSwitch.proposeGroups(countA: 4, countB: 4), countA: 4, countB: 4))
        XCTAssertFalse(EditionSwitch.canConfirmAll(EditionSwitch.proposeGroups(countA: 18, countB: 21), countA: 18, countB: 21))
    }

    func testProposedGroupsAreValidAndUnconfirmed() {
        for (a, b) in [(18, 21), (21, 18), (1, 5), (5, 1), (24, 24)] {
            let g = EditionSwitch.proposeGroups(countA: a, countB: b)
            XCTAssertTrue(EditionSwitch.validGroups(g), "\(a)/\(b)")
            XCTAssertEqual(g.flatMap(\.a).sorted(), Array(0..<a), "\(a)/\(b)")
            XCTAssertEqual(g.flatMap(\.b).sorted(), Array(0..<b), "\(a)/\(b)")
            XCTAssertFalse(g.contains { $0.confirmed }, "\(a)/\(b)")
        }
    }

    func testTextChaptersFindHeadings() {
        let text = "Titel\n\n# Eins\nText\n## Zwei\nMehr\n"
        let list = EditionSwitch.textChapters(text)
        XCTAssertEqual(list.titles, ["Eins", "Zwei"])
        XCTAssertEqual(list.starts.count, 2)
        XCTAssertEqual(list.starts[0], 9, accuracy: 0.001)  // hinter „# “: wie `textChapters` am PC (`at + line.indexOf(title)`)
    }
}
