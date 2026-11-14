import XCTest
@testable import VoiceCore

final class IdleReasonsTests: XCTestCase {
    func testReaderReasonTruthTable() {
        // Reihenfolge: Hören, Lesen & Hören, Lesen; jeweils pausiert und spielend.
        let surfaces: [PlayerSurface] = [.listen, .readListen, .read]
        let cases: [(Bool, Bool, [Bool], [Bool])] = [
            (false, false, [false, false, false], [false, false, false]),
            (false, true, [false, false, false], [false, false, false]),
            (true, false, [false, false, false], [false, false, false]),
            (true, true, [false, true, true], [true, true, true]),
        ]
        for (presented, sceneActive, paused, playing) in cases {
            for (index, surface) in surfaces.enumerated() {
                for isPlaying in [false, true] {
                    XCTAssertEqual(
                        readerReasonActive(
                            presented: presented,
                            sceneActive: sceneActive,
                            surface: surface,
                            playing: isPlaying
                        ),
                        (isPlaying ? playing : paused)[index],
                        "presented=\(presented), active=\(sceneActive), "
                            + "surface=\(surface), playing=\(isPlaying)"
                    )
                }
            }
        }
    }

    func testEmptyReasonsDoNotNeedAwake() {
        let reasons = IdleReasons()
        XCTAssertTrue(reasons.isEmpty)
        XCTAssertFalse(reasons.needsAwake)
    }

    func testRemovingEitherReasonPreservesTheOther() {
        for first in [IdleReasons.Reason.reader, .liveMeeting] {
            let other: IdleReasons.Reason = first == .reader ? .liveMeeting : .reader
            var reasons = IdleReasons()
            reasons.insert(first)
            reasons.insert(other)
            XCTAssertFalse(reasons.isEmpty)
            XCTAssertTrue(reasons.needsAwake)

            reasons.remove(first)
            XCTAssertFalse(reasons.isEmpty)
            XCTAssertTrue(reasons.needsAwake)

            reasons.remove(other)
            XCTAssertTrue(reasons.isEmpty)
            XCTAssertFalse(reasons.needsAwake)
        }
    }

    func testRepeatedInsertAndMissingRemoveAreHarmless() {
        var reasons = IdleReasons()
        reasons.insert(.reader)
        reasons.insert(.reader)
        reasons.remove(.liveMeeting)
        XCTAssertTrue(reasons.needsAwake)
        reasons.remove(.reader)
        reasons.remove(.reader)
        XCTAssertFalse(reasons.needsAwake)
        XCTAssertEqual(reasons, IdleReasons())
    }
}
