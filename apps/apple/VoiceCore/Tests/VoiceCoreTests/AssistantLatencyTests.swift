import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (lva#521): Latenzmessung. Nur synthetische Zahlen, keine Inhalte, keine echten Wartezeiten
// (die Zeitquelle des Messvorgangs ist ersetzt; die fsync-Tests verzögern nur das Dateisystem-Double).

private let baseDate = Date(timeIntervalSince1970: 1_760_000_000)

private func sample(visible: Int? = nil, audio: Int? = nil, unit: AssistantLatencyUnitClass = .template, cold: Bool = false,
                    local: Bool = true, model: AssistantModelKind = .apple, at: Date = baseDate) -> AssistantLatencySample {
    AssistantLatencySample(at: at, firstVisibleMs: visible, firstAudioMs: audio, unit: unit, cold: cold, local: local, model: model)
}

/// Eine Zeitquelle, die der Test von Hand vorrückt.
private final class FakeTime: @unchecked Sendable {
    private let lock = NSLock()
    private var value: TimeInterval = 100
    func set(_ v: TimeInterval) { lock.withLock { value = v } }
    var now: TimeInterval { lock.withLock { value } }
}

private struct Injected: Error {}

/// Dateisystem mit Fehlerinjektion und Verzögerung beim fsync der Datei.
private final class LatencyFS: AssistantStoreFileSystem, @unchecked Sendable {
    private let base = AssistantPosixStoreFileSystem()
    private let lock = NSLock()
    private var failures = 0
    private var delay: UInt32 = 0

    func failSync(times: Int) { lock.withLock { failures = times } }
    func slowSync(microseconds: UInt32) { lock.withLock { delay = microseconds } }

    func read(_ url: URL) throws -> Data? { try base.read(url) }
    func exists(_ url: URL) -> Bool { base.exists(url) }
    func createDirectory(_ url: URL) throws { try base.createDirectory(url) }
    func writeNew(_ url: URL, _ data: Data) throws { try base.writeNew(url, data) }
    func syncFile(_ url: URL) throws {
        let (fail, wait) = lock.withLock { () -> (Bool, UInt32) in
            let hit = failures > 0
            if hit { failures -= 1 }
            return (hit, delay)
        }
        if wait > 0 { usleep(wait) }
        if fail { throw Injected() }
        try base.syncFile(url)
    }
    func rename(_ from: URL, to: URL) throws { try base.rename(from, to: to) }
    func syncDirectory(_ url: URL) throws { try base.syncDirectory(url) }
    func remove(_ url: URL) { base.remove(url) }
}

final class AssistantLatencyTests: XCTestCase {
    private var dir: URL!

    override func setUp() {
        dir = FileManager.default.temporaryDirectory.appendingPathComponent("assistant-latency-\(UUID().uuidString)", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    }

    override func tearDown() { try? FileManager.default.removeItem(at: dir) }

    private func makeLog() -> AssistantLatencyLog { AssistantLatencyLog(directory: dir) }

    // MARK: Perzentile (Nearest-Rank)

    func testPercentileOfOneValueIsThatValue() {
        XCTAssertEqual(AssistantPercentile.nearestRank([42], percent: 50), 42)
        XCTAssertEqual(AssistantPercentile.nearestRank([42], percent: 95), 42)
        XCTAssertNil(AssistantPercentile.nearestRank([Int](), percent: 50))
    }

    func testPercentileOddAndEvenCountsUseNearestRankNotAnAverage() {
        XCTAssertEqual(AssistantPercentile.nearestRank([30, 10, 20], percent: 50), 20, "ungerade: der mittlere Wert, unsortiert gegeben")
        XCTAssertEqual(AssistantPercentile.nearestRank([10, 20, 30, 40], percent: 50), 20, "gerade: Rang 2, nie der Mittelwert 25")
        XCTAssertEqual(AssistantPercentile.nearestRank([10, 20, 30, 40], percent: 95), 40)
    }

    func testP95OfTwentyAndTwentyOneValues() {
        let twenty = Array(1...20)
        XCTAssertEqual(AssistantPercentile.nearestRank(twenty, percent: 95), 19, "ceil(0,95 · 20) = Rang 19")
        let twentyOne = Array(1...21)
        XCTAssertEqual(AssistantPercentile.nearestRank(twentyOne, percent: 95), 20, "ceil(0,95 · 21) = Rang 20")
        let hundred = Array(1...100)
        XCTAssertEqual(AssistantPercentile.nearestRank(hundred, percent: 50), 50)
        XCTAssertEqual(AssistantPercentile.nearestRank(hundred, percent: 95), 95)
    }

    // MARK: Gruppierung und Tore

    private func addAudio(_ log: AssistantLatencyLog, count: Int, audio: (Int) -> Int, unit: AssistantLatencyUnitClass = .template,
                          cold: Bool = false, local: Bool = true) {
        for i in 0..<count { log.add(sample(visible: 100 + i, audio: audio(i), unit: unit, cold: cold, local: local)) }
    }

    func testGateIsMetWhenP95OfFirstAudioIsWithinTheLimit() {
        let log = makeLog()
        addAudio(log, count: 20, audio: { 500 + $0 * 10 })   // p95 = Rang 19 = 680
        let group = try! XCTUnwrap(log.summary().groups.first)
        XCTAssertEqual(group.verdict, .met)
        XCTAssertEqual(group.count, 20)
        XCTAssertEqual(group.audio?.p95, 680)
        XCTAssertEqual(group.audio?.p50, 590)
        XCTAssertTrue(group.hasGate)
    }

    func testGateIsMissedWhenP95ExceedsTheLimitAndExactlyTheLimitStillMeets() {
        let missed = makeLog()
        addAudio(missed, count: 20, audio: { $0 == 19 || $0 == 18 ? 1_300 : 500 })   // p95 = Rang 19 = 1300 > 1200
        XCTAssertEqual(missed.summary().groups.first?.verdict, .missed)
        missed.reset()
        addAudio(missed, count: 20, audio: { _ in 1_200 })
        XCTAssertEqual(missed.summary().groups.first?.verdict, .met, "genau 1,2 s erfüllt das Tor (≤)")
        missed.reset()
        addAudio(missed, count: 20, audio: { _ in 1_201 })
        XCTAssertEqual(missed.summary().groups.first?.verdict, .missed)
    }

    func testContentGateIsFourSeconds() {
        let log = makeLog()
        addAudio(log, count: 20, audio: { _ in 4_000 }, unit: .content)
        XCTAssertEqual(log.summary().groups.first?.verdict, .met)
        log.reset()
        addAudio(log, count: 20, audio: { _ in 4_001 }, unit: .content)
        XCTAssertEqual(log.summary().groups.first?.verdict, .missed)
    }

    func testFewerThanTwentyAudioMeasurementsAreNotJudged() {
        let log = makeLog()
        addAudio(log, count: 19, audio: { _ in 300 })
        XCTAssertEqual(log.summary().groups.first?.verdict, .tooFew)
        log.add(sample(visible: 100, audio: 300))
        XCTAssertEqual(log.summary().groups.first?.verdict, .met, "mit der zwanzigsten Messung gilt ein Urteil")
    }

    func testOnlyAudioMeasurementsCountTowardsTheGate() {
        let log = makeLog()
        addAudio(log, count: 19, audio: { _ in 300 })
        for _ in 0..<5 { log.add(sample(visible: 100)) }   // 24 Messungen, aber nur 19 mit Ton
        let group = try! XCTUnwrap(log.summary().groups.first)
        XCTAssertEqual(group.count, 24)
        XCTAssertEqual(group.audio?.count, 19)
        XCTAssertEqual(group.visible?.count, 24)
        XCTAssertEqual(group.verdict, .tooFew)
    }

    func testColdAndNotLocalGroupsAreReportedWithoutAGate() {
        let log = makeLog()
        addAudio(log, count: 25, audio: { _ in 9_000 }, cold: true, local: true)
        addAudio(log, count: 25, audio: { _ in 9_000 }, cold: false, local: false)
        let groups = log.summary().groups
        XCTAssertEqual(groups.count, 2)
        XCTAssertTrue(groups.allSatisfy { $0.verdict == .noGate && !$0.hasGate })
    }

    func testGroupsAreSeparatedByClassWarmthAndLocalityInAFixedOrder() {
        let log = makeLog()
        log.add(sample(visible: 1, audio: 1, unit: .content, cold: true, local: false))
        log.add(sample(visible: 2, audio: 2, unit: .content, cold: false, local: true))
        log.add(sample(visible: 3, audio: 3, unit: .template, cold: true, local: true))
        log.add(sample(visible: 4, audio: 4, unit: .template, cold: false, local: false))
        log.add(sample(visible: 5, audio: 5, unit: .template, cold: false, local: true))
        log.add(sample(visible: 6, audio: 6, unit: .template, cold: false, local: true))
        let titles = log.summary().groups.map(\.title)
        XCTAssertEqual(titles, [
            "Vorlage/Ansage · warm · lokal", "Vorlage/Ansage · warm · nicht lokal", "Vorlage/Ansage · kalt · lokal",
            "Antwort mit Abruf · warm · lokal", "Antwort mit Abruf · kalt · nicht lokal",
        ])
        XCTAssertEqual(log.summary().groups.map(\.count), [2, 1, 1, 1, 1])
        XCTAssertEqual(log.summary().total, 6)
    }

    func testAudioOnlyAndVisibleOnlyMeasurementsFillTheirOwnColumn() {
        let log = makeLog()
        log.add(sample(visible: 100))
        log.add(sample(audio: 700))
        let group = try! XCTUnwrap(log.summary().groups.first)
        XCTAssertEqual(group.visible, AssistantLatencyPercentiles([100]))
        XCTAssertEqual(group.audio, AssistantLatencyPercentiles([700]))
    }

    // MARK: Ring bei 200

    func testRingKeepsTheLastTwoHundredSamples() {
        let log = makeLog()
        for i in 0..<205 { log.add(sample(visible: i, audio: i)) }
        let all = log.samples
        XCTAssertEqual(all.count, 200)
        XCTAssertEqual(all.first?.firstVisibleMs, 5, "die fünf ältesten fielen heraus")
        XCTAssertEqual(all.last?.firstVisibleMs, 204)
        XCTAssertEqual(makeLog().samples.count, 200, "auch die Datei hält höchstens 200")
    }

    // MARK: Persistenz

    func testRoundTripThroughTheFile() {
        let first = makeLog()
        let a = sample(visible: 120, audio: 840, unit: .content, cold: true, local: false, model: .other, at: baseDate)
        let b = sample(visible: nil, audio: 300, unit: .template, cold: false, local: true, model: .gemma, at: baseDate.addingTimeInterval(60))
        first.add(a)
        first.add(b)
        let second = makeLog()
        XCTAssertEqual(second.samples, [a, b])
        XCTAssertTrue(FileManager.default.fileExists(atPath: second.url.path))
    }

    func testBrokenFileStartsEmptyAndIsReplacedByTheNextWrite() throws {
        try Data("{ das ist kein JSON".utf8).write(to: dir.appendingPathComponent(AssistantLatencyLog.fileName))
        let log = makeLog()
        XCTAssertEqual(log.samples, [])
        XCTAssertEqual(log.summary().groups, [])
        log.add(sample(audio: 500))
        XCTAssertEqual(makeLog().samples.count, 1)
    }

    func testFileOfAnotherShapeOrNewerVersionStartsEmpty() throws {
        let file = dir.appendingPathComponent(AssistantLatencyLog.fileName)
        try Data("[1,2,3]".utf8).write(to: file)
        XCTAssertEqual(makeLog().samples, [])
        try Data(#"{"version":99,"samples":[]}"#.utf8).write(to: file)
        XCTAssertEqual(makeLog().samples, [])
    }

    func testSampleWithoutAnyMarkIsNotStored() {
        let log = makeLog()
        log.add(sample())
        XCTAssertEqual(log.samples, [])
        XCTAssertFalse(FileManager.default.fileExists(atPath: log.url.path), "nichts zu speichern, keine Datei")
    }

    func testNegativeValuesBecomeZero() {
        let log = makeLog()
        log.add(sample(visible: -5, audio: -1))
        XCTAssertEqual(log.samples.first?.firstVisibleMs, 0)
        XCTAssertEqual(log.samples.first?.firstAudioMs, 0)
    }

    func testResetEmptiesMemoryAndFile() {
        let log = makeLog()
        log.add(sample(audio: 500))
        log.reset()
        XCTAssertEqual(log.samples, [])
        XCTAssertFalse(FileManager.default.fileExists(atPath: log.url.path))
        XCTAssertEqual(makeLog().samples, [])
        log.reset()   // ohne Datei kein Fehler
    }

    func testUnwritableDirectoryKeepsTheSampleInMemory() throws {
        let blocker = dir.appendingPathComponent("datei")
        try Data("x".utf8).write(to: blocker)
        let log = AssistantLatencyLog(directory: blocker.appendingPathComponent("unter", isDirectory: true))
        log.add(sample(audio: 500))
        XCTAssertEqual(log.samples.count, 1, "ein Schreibfehler verliert die Messung nicht und wirft nicht")
    }

    func testConcurrentAddsLoseNothing() {
        let log = makeLog()
        DispatchQueue.concurrentPerform(iterations: 50) { i in log.add(sample(visible: i, audio: i)) }
        XCTAssertEqual(log.samples.count, 50)
        XCTAssertEqual(Set(log.samples.compactMap(\.firstVisibleMs)), Set(0..<50))
    }

    // MARK: Messvorgang

    func testClockMeasuresFromBeginToTheMarks() throws {
        let time = FakeTime()
        var clock = AssistantLatencyClock(now: { time.now })
        time.set(100)
        clock.begin()
        clock.markUnit(.content)
        time.set(100.4)
        clock.markVisible()
        time.set(101.25)
        clock.markAudio()
        let s = try XCTUnwrap(clock.finish(cold: true, local: false, model: .other, at: baseDate))
        XCTAssertEqual(s.firstVisibleMs, 400)
        XCTAssertEqual(s.firstAudioMs, 1_250)
        XCTAssertEqual(s.unit, .content)
        XCTAssertTrue(s.cold)
        XCTAssertFalse(s.local)
        XCTAssertEqual(s.model, .other)
        XCTAssertEqual(s.at, baseDate)
    }

    func testClockCountsOnlyTheFirstMark() throws {
        var clock = AssistantLatencyClock(now: { 0 })
        clock.begin(at: 10)
        clock.markUnit(.template)
        clock.markUnit(.content)
        clock.markVisible(at: 10.2)
        clock.markVisible(at: 10.9)
        clock.markAudio(at: 10.5)
        clock.markAudio(at: 12)
        let s = try XCTUnwrap(clock.finish(cold: false, local: true, model: .apple))
        XCTAssertEqual(s.firstVisibleMs, 200)
        XCTAssertEqual(s.firstAudioMs, 500)
        XCTAssertEqual(s.unit, .template)
    }

    func testANewBeginDiscardsAnUnfinishedMeasurement() throws {
        var clock = AssistantLatencyClock(now: { 0 })
        clock.begin(at: 10)
        clock.markUnit(.template)
        clock.markVisible(at: 10.1)
        clock.markAudio(at: 10.2)
        clock.begin(at: 20)
        XCTAssertNil(clock.finish(cold: false, local: true, model: .apple), "die alten Marken gehören nicht zur neuen Messung")
        clock.begin(at: 30)
        clock.markVisible(at: 30.3)
        let s = try XCTUnwrap(clock.finish(cold: false, local: true, model: .apple))
        XCTAssertEqual(s.firstVisibleMs, 300)
        XCTAssertNil(s.firstAudioMs)
        XCTAssertEqual(s.unit, .content, "ohne Klassenmarke gilt „Antwort mit Abruf“")
    }

    func testNoSampleWithoutMarksOrWithoutBegin() {
        var clock = AssistantLatencyClock(now: { 0 })
        XCTAssertNil(clock.finish(cold: false, local: true, model: .apple), "ohne Beginn")
        clock.markVisible(at: 5)
        clock.markAudio(at: 5)
        XCTAssertNil(clock.finish(cold: false, local: true, model: .apple), "Marken ohne Beginn zählen nicht")
        clock.begin(at: 1)
        clock.markUnit(.template)
        XCTAssertTrue(clock.isRunning)
        XCTAssertNil(clock.finish(cold: false, local: true, model: .apple), "begonnen, aber nie etwas sichtbar oder hörbar")
        XCTAssertFalse(clock.isRunning)
    }

    func testFinishEndsTheMeasurement() throws {
        var clock = AssistantLatencyClock(now: { 0 })
        clock.begin(at: 1)
        clock.markAudio(at: 1.5)
        XCTAssertNotNil(clock.finish(cold: false, local: true, model: .apple))
        clock.markVisible(at: 2)
        XCTAssertNil(clock.finish(cold: false, local: true, model: .apple), "nach dem Ende wird nichts mehr angenommen")
    }

    func testMarkBeforeTheBeginTimeBecomesZero() throws {
        var clock = AssistantLatencyClock(now: { 0 })
        clock.begin(at: 10)
        clock.markVisible(at: 9.9)
        XCTAssertEqual(clock.finish(cold: false, local: true, model: .apple)?.firstVisibleMs, 0)
    }

    // MARK: Texttabelle

    func testSummaryTextHasOnlyFixedLabelsAndNumbers() {
        let log = makeLog()
        for (v, a) in [(10, 100), (20, 200), (30, 300)] { log.add(sample(visible: v, audio: a)) }
        log.add(sample(visible: 500, unit: .content, cold: true, local: false))
        let expected = """
        Messwerte Gespräch (letzte 200 Runden, Zeiten ab erkanntem Sprachende)
        Gruppe | Anzahl | Sichtbar p50 | Sichtbar p95 | Ton p50 | Ton p95 | Tor erster Ton p95 | Stand
        Vorlage/Ansage · warm · lokal | 3 | 20 ms | 30 ms | 200 ms | 300 ms | ≤ 1,2 s | zu wenige Messungen
        Antwort mit Abruf · kalt · nicht lokal | 1 | 500 ms | 500 ms | – | – | – | kein Tor
        Gesamt: 4 Messungen. Tor nur warm und lokal, ab 20 Tonmessungen je Gruppe.
        """
        XCTAssertEqual(log.summaryText(), expected)
    }

    func testSummaryTextWithoutSamplesAndWithFsyncLine() {
        let log = makeLog()
        XCTAssertEqual(log.summaryText(), "Messwerte Gespräch (letzte 200 Runden, Zeiten ab erkanntem Sprachende)\nNoch keine Messungen.")
        let stats = AssistantDurableStats(total: 3, count: 3, p50Ms: 4.25, p95Ms: 12.4, maxMs: 30.2)
        XCTAssertTrue(log.summaryText(durable: stats).hasSuffix("Protokoll-fsync (letzte 3 Gruppen): p50 4,2 ms, p95 12 ms, Maximum 30 ms"))
        XCTAssertTrue(log.summaryText(durable: .empty).hasSuffix("Protokoll-fsync: noch keine Messung."))
    }

    // MARK: fsync des Protokollschreibers

    private func auditEntry() -> AssistantAuditEntry { AssistantAuditEntry(kind: .unsourcedOn, consentVersion: 2, generation: 7) }

    func testDurableStatsCountGroupsAndStayConsistentAfterAWriteFailure() async {
        let fs = LatencyFS()
        let audit = AssistantAuditLog(directory: dir, reader: AssistantAuditWriter.readFile, fileSystem: fs)
        XCTAssertEqual(audit.durableStats, .empty)
        for _ in 0..<3 {
            let ok = await audit.record(auditEntry())
            XCTAssertTrue(ok)
        }
        let three = audit.durableStats
        XCTAssertEqual(three.total, 3)
        XCTAssertEqual(three.count, 3)
        XCTAssertEqual(three.total, audit.syncCount, "ein fsync je Gruppe, eine Messung je fsync")
        let p50 = try! XCTUnwrap(three.p50Ms), p95 = try! XCTUnwrap(three.p95Ms), maximum = try! XCTUnwrap(three.maxMs)
        XCTAssertLessThanOrEqual(p50, p95)
        XCTAssertLessThanOrEqual(p95, maximum)
        XCTAssertGreaterThanOrEqual(p50, 0)

        fs.failSync(times: 1)
        let failed = await audit.record(auditEntry())
        XCTAssertFalse(failed, "das fsync scheiterte: die Zeile gilt als nicht geschrieben")
        XCTAssertEqual(audit.syncCount, 4)
        XCTAssertEqual(audit.durableStats, three, "ein gescheitertes fsync ändert die Statistik nicht")

        let healed = await audit.record(auditEntry())
        XCTAssertTrue(healed)
        XCTAssertEqual(audit.durableStats.total, 4, "der Schreiber arbeitet nach dem Fehler unverändert weiter")
        XCTAssertEqual(audit.durableStats.count, 4)
    }

    func testDurableStatsMeasureTheDurationOfTheFsync() async {
        let fs = LatencyFS()
        fs.slowSync(microseconds: 30_000)
        let audit = AssistantAuditLog(directory: dir, reader: AssistantAuditWriter.readFile, fileSystem: fs)
        for _ in 0..<2 { _ = await audit.record(auditEntry()) }
        let stats = audit.durableStats
        XCTAssertEqual(stats.count, 2)
        XCTAssertGreaterThanOrEqual(stats.p50Ms ?? 0, 29, "das verzögerte fsync (30 ms) steht in der Messung")
        XCTAssertGreaterThanOrEqual(stats.maxMs ?? 0, 29)
    }

    func testDurationRingKeepsTheLastTwoHundredAndCountsAll() {
        let ring = AssistantDurationRing()
        for i in 1...250 { ring.record(milliseconds: Double(i)) }
        let stats = ring.snapshot()
        XCTAssertEqual(stats.total, 250)
        XCTAssertEqual(stats.count, 200)
        XCTAssertEqual(stats.maxMs, 250)
        XCTAssertEqual(stats.p50Ms, 150, "Fenster 51...250: Rang 100 = 150")
        XCTAssertEqual(stats.p95Ms, 240, "Rang 190 = 240")
    }
}
