import XCTest
@testable import VoiceCore

final class ReadingHistoryTests: XCTestCase {
    private var berlin: Calendar {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = TimeZone(identifier: "Europe/Berlin")!
        return c
    }

    /// Millisekunden für eine Berliner Ortszeit.
    private func ms(_ y: Int, _ mo: Int, _ d: Int, _ h: Int, _ mi: Int) -> Double {
        let date = berlin.date(from: DateComponents(year: y, month: mo, day: d, hour: h, minute: mi))!
        return date.timeIntervalSince1970 * 1000
    }

    private func entry(_ start: Double, minutes: Double, medium: ReadingMedium = .audio, device: String = "a") -> ReadingLogEntry {
        ReadingLogEntry(startMs: start, endMs: start + minutes * 60_000, edition: "de", medium: medium,
                        from: 0, to: 100, unit: .ms, device: device)
    }

    func testGruppiertNachOrtstagNeuesteZuerstUndFiltertKurzes() {
        let log = [
            entry(ms(2026, 11, 25, 21, 10), minutes: 30),
            entry(ms(2026, 11, 26, 0, 30), minutes: 10, medium: .text),   // nach Mitternacht Ortszeit: 26.
            entry(ms(2026, 11, 26, 8, 0), minutes: 0.2),                  // 12 s: Antippen
            entry(ms(2026, 11, 26, 7, 0), minutes: 5, device: "b"),
        ]
        let days = ReadingHistory.days(log, calendar: berlin)
        XCTAssertEqual(days.count, 2)
        XCTAssertEqual(days[0].entries.map(\.startMs), [ms(2026, 11, 26, 7, 0), ms(2026, 11, 26, 0, 30)])
        XCTAssertEqual(days[1].entries.map(\.startMs), [ms(2026, 11, 25, 21, 10)])
    }

    func testObergrenzeGiltUeberAlleTage() {
        let log = (0..<50).map { entry(ms(2026, 11, 1, 8, 0) + Double($0) * 86_400_000, minutes: 5) }
        let days = ReadingHistory.days(log, calendar: berlin, maxEntries: 40)
        XCTAssertEqual(days.flatMap(\.entries).count, 40)
        XCTAssertEqual(days.first?.entries.first?.startMs, log.last?.startMs)
    }

    func testBeschriftungen() {
        let now = Date(timeIntervalSince1970: ms(2026, 11, 26, 12, 0) / 1000)
        let today = berlin.startOfDay(for: now)
        XCTAssertEqual(ReadingHistory.dayLabel(today, now: now, calendar: berlin), "Heute")
        XCTAssertEqual(ReadingHistory.dayLabel(berlin.date(byAdding: .day, value: -1, to: today)!, now: now, calendar: berlin), "Gestern")
        let older = ReadingHistory.dayLabel(berlin.date(byAdding: .day, value: -3, to: today)!, now: now, calendar: berlin)
        XCTAssertTrue(older.contains("23"), older)
        XCTAssertTrue(older.contains("November"), older)
        let e = entry(ms(2026, 11, 25, 21, 10), minutes: 30)
        XCTAssertEqual(ReadingHistory.timeSpan(e, calendar: berlin), "21:10–21:40")
        XCTAssertEqual(ReadingHistory.duration(e), "30 Min.")
        XCTAssertEqual(ReadingHistory.duration(entry(0, minutes: 65)), "1 Std. 5 Min.")
        XCTAssertEqual(ReadingHistory.mediumLabel(.live), "Live übersetzt")
    }
}
