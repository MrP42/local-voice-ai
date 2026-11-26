import XCTest
@testable import VoiceCore

/// Besprechungsliste (lva#837): Titel, Eckdaten, Status, Gruppen, Suche, Umbenennen und Löschen. Nur synthetische Daten.
final class MeetingListingTests: XCTestCase {
    private func doc(name: String = "aufnahme_2026.m4a", duration: Double = 2820, created: Date = Date(timeIntervalSince1970: 1_800_000_000)) -> MeetingDocument {
        MeetingDocument(id: UUID(), createdAt: created, originalName: name, originalExtension: "m4a", digest: "00", bytes: 1, duration: duration)
    }
    private func minutes(summary: String = "", scope: String = "", tasks: Int = 0, questions: Int = 0) -> MeetingMinutes {
        MeetingMinutes(summary: summary, scope: scope, decisions: [],
                       tasks: (0..<tasks).map { .init(text: "Aufgabe \($0)", assignee: nil, due: nil) },
                       next_steps: [], follow_ups: [], open_questions: (0..<questions).map { .init(text: "Frage \($0)") })
    }

    func testCleanTitleDropsGenericAndWhitespace() {
        XCTAssertNil(MeetingListing.cleanTitle(nil))
        XCTAssertNil(MeetingListing.cleanTitle("   "))
        XCTAssertNil(MeetingListing.cleanTitle("Besprechung"))
        XCTAssertNil(MeetingListing.cleanTitle(" neue  Aufnahme "))
        XCTAssertEqual(MeetingListing.cleanTitle("  Jour fixe\nTeam  Nord "), "Jour fixe Team Nord")
        XCTAssertEqual(MeetingListing.cleanTitle(String(repeating: "x", count: 300))?.count, 120)
    }

    func testTitlePrefersOwnThenMinutesThenFile() {
        var d = doc(name: "Besprechung.m4a")
        XCTAssertEqual(MeetingListing.title(d), "Besprechung")
        d = doc(name: "Projekt_Kickoff_Nord.m4a")
        XCTAssertEqual(MeetingListing.title(d), "Projekt Kickoff Nord")
        d.minutes = minutes(summary: "Abstimmung zum Haushalt 2027. Weitere Punkte folgen.")
        XCTAssertEqual(MeetingListing.title(d), "Abstimmung zum Haushalt 2027")
        d.title = "Jour fixe"
        XCTAssertEqual(MeetingListing.title(d), "Jour fixe")
    }

    func testAutoTitleShortensAtWordBoundary() {
        let long = minutes(scope: "Planung der gemeinsamen Beschaffung von Fahrzeugen für alle Standorte im kommenden Jahr")
        let title = MeetingListing.autoTitle(long)
        XCTAssertNotNil(title)
        XCTAssertLessThanOrEqual(title!.count, 60)
        XCTAssertTrue(title!.hasSuffix("…"))
        XCTAssertFalse(title!.dropLast().hasSuffix(" "))
        XCTAssertNil(MeetingListing.autoTitle(minutes(summary: "Ok")))
        XCTAssertEqual(MeetingListing.autoTitle(minutes(summary: "Kurz und klar")), "Kurz und klar")
    }

    func testDurationFormatting() {
        XCTAssertEqual(MeetingListing.duration(45), "45 s")
        XCTAssertEqual(MeetingListing.duration(2820), "47 Min")
        XCTAssertEqual(MeetingListing.duration(4320), "1 Std 12 Min")
        XCTAssertEqual(MeetingListing.duration(7200), "2 Std")
        XCTAssertEqual(MeetingListing.duration(.nan), "–")
        XCTAssertEqual(MeetingListing.duration(0), "–")
    }

    func testParticipantsAndLanguages() {
        var d = doc()
        XCTAssertNil(MeetingListing.participantCount(d))
        d.participantNames = ["A. Muster", " ", "B. Beispiel"]
        XCTAssertEqual(MeetingListing.participantCount(d), 2)
        d.liveSegments = [
            LiveSegment(id: "1", start: 0, end: 1, text: "a", language: "de-DE", refined: true),
            LiveSegment(id: "2", start: 1, end: 2, text: "b", language: "en", refined: true),
            LiveSegment(id: "3", start: 2, end: 3, text: "c", language: "de", refined: true),
            LiveSegment(id: "4", start: 3, end: 4, text: "d", language: "fr", refined: false),
            LiveSegment(id: "5", start: 4, end: 5, text: "e", language: "auto", refined: true),
        ]
        XCTAssertEqual(MeetingListing.languages(d), ["DE", "EN"])
    }

    func testStatusOrder() {
        var d = doc()
        XCTAssertEqual(MeetingListing.status(d, running: false, failed: false), .consentMissing, "Altbestand ohne Nachweis")
        d.consent = ConsentLog()
        XCTAssertEqual(MeetingListing.status(d, running: false, failed: false), .waiting)
        XCTAssertEqual(MeetingListing.status(d, running: true, failed: false), .running)
        XCTAssertEqual(MeetingListing.status(d, running: false, failed: true), .failed)
        d.minutes = minutes(tasks: 3, questions: 1)
        XCTAssertEqual(MeetingListing.status(d, running: false, failed: true), .ready(tasks: 3, openQuestions: 1))
    }

    func testProgressFromStatusLine() {
        XCTAssertEqual(MeetingListing.progress(fromStatus: "Lokal transkribieren · 73 %"), 0.73)
        XCTAssertEqual(MeetingListing.progress(fromStatus: "Qwen3 · Auswerten · Abschnitt 3 von 4"), 0.5)
        XCTAssertNil(MeetingListing.progress(fromStatus: "Audiospur vorbereiten"))
        XCTAssertNil(MeetingListing.progress(fromStatus: "Fehler 404 %"))
    }

    func testSectionsAndGrouping() {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "Europe/Berlin")!
        calendar.locale = Locale(identifier: "de_DE")
        calendar.firstWeekday = 2
        let now = calendar.date(from: DateComponents(year: 2026, month: 11, day: 26, hour: 15))!
        func at(_ month: Int, _ day: Int, _ year: Int = 2026) -> Date { calendar.date(from: DateComponents(year: year, month: month, day: day, hour: 10))! }
        XCTAssertEqual(MeetingListing.section(for: at(11, 26), now: now, calendar: calendar), "Heute")
        XCTAssertEqual(MeetingListing.section(for: at(11, 25), now: now, calendar: calendar), "Gestern")
        XCTAssertEqual(MeetingListing.section(for: at(11, 23), now: now, calendar: calendar), "Diese Woche")
        XCTAssertEqual(MeetingListing.section(for: at(10, 2), now: now, calendar: calendar), "Oktober")
        XCTAssertEqual(MeetingListing.section(for: at(12, 2, 2025), now: now, calendar: calendar), "Dezember 2025")

        var live = doc(created: at(11, 26))
        live.recordedAt = at(11, 25)
        let older = doc(created: at(10, 2)), today = doc(created: at(11, 26))
        let groups = MeetingListing.grouped([older, live, today], now: now, calendar: calendar)
        XCTAssertEqual(groups.map(\.title), ["Heute", "Gestern", "Oktober"])
        XCTAssertEqual(groups[1].items.map(\.id), [live.id], "Aufnahmebeginn zählt, nicht die Übernahme")
    }

    func testFilters() {
        var ready = doc(); ready.minutes = minutes()
        var recorded = doc(); recorded.recordedAt = Date(); recorded.liveSegments = []
        let imported = doc()
        XCTAssertTrue(MeetingListing.matches(ready, filter: .ready))
        XCTAssertFalse(MeetingListing.matches(imported, filter: .ready))
        XCTAssertTrue(MeetingListing.matches(imported, filter: .open))
        XCTAssertTrue(MeetingListing.matches(imported, filter: .imported))
        XCTAssertFalse(MeetingListing.matches(recorded, filter: .imported))
        XCTAssertTrue(MeetingListing.Filter.allCases.allSatisfy { MeetingListing.matches(ready, filter: .all) || $0 != .all })
    }

    func testSearchIsCaseAndAccentInsensitiveAndNeedsAllWords() {
        var d = doc(name: "Sitzung.m4a")
        d.participantNames = ["Jürgen Beispiel"]
        d.minutes = minutes(summary: "Beschluss zur Gebührenordnung.")
        d.segments = [MeetingSegment(index: 0, start: 0, end: 1, text: "Wir sprechen über den Fuhrpark.")]
        XCTAssertTrue(MeetingListing.matches(d, query: ""))
        XCTAssertTrue(MeetingListing.matches(d, query: "juergen") == false, "keine Umschreibung, nur Akzente")
        XCTAssertTrue(MeetingListing.matches(d, query: "jurgen"))
        XCTAssertTrue(MeetingListing.matches(d, query: "GEBÜHREN fuhrpark"))
        XCTAssertFalse(MeetingListing.matches(d, query: "gebühren haushalt"))
    }

    func testLiveImportCarriesTitleParticipantsAndStart() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let source = root.appendingPathComponent("synthetic.wav")
        try Data([1, 2, 3]).write(to: source)
        let archive = try MeetingArchive(root: root.appendingPathComponent("archive"), quota: 1000)
        let started = Date(timeIntervalSince1970: 1_790_000_000)
        let consent = ConsentLog(records: [ConsentRecord(confirmedAt: Date(), channel: .app, reason: .start,
            scopes: [.recording, .transcription], purpose: "Test", controller: "Test", retentionDays: 1)])
        let live = MeetingArchive.LiveImport(consent: consent, live: LiveState(), segments: [], notes: nil,
                                             title: " Jour  fixe ", participantNames: ["A", "B"], startedAt: started)
        let saved = try await archive.importMedia(source, duration: 1, live: live)
        XCTAssertEqual(saved.title, "Jour fixe")
        XCTAssertEqual(saved.participantNames, ["A", "B"])
        XCTAssertEqual(saved.recordedAt, started)
        let generic = MeetingArchive.LiveImport(consent: consent, live: LiveState(), segments: [], notes: nil, title: "Besprechung")
        let other = try await archive.importMedia(source, duration: 1, live: generic, id: UUID())
        XCTAssertNil(other.title, "Standardtitel der Aufnahme gilt als automatisch")
    }

    func testRenameAndDelete() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let source = root.appendingPathComponent("synthetic.wav")
        try Data([1, 2, 3]).write(to: source)
        let folder = root.appendingPathComponent("archive")
        let archive = try MeetingArchive(root: folder, quota: 1000)
        let a = try await archive.importMedia(source, duration: 1)
        let b = try await archive.importMedia(source, duration: 1, id: UUID())

        let renamed = try await archive.rename(a.id, title: "  Haushalt  ")
        XCTAssertEqual(renamed.title, "Haushalt")
        XCTAssertEqual(renamed.titleIsUser, true)
        let reset = try await archive.rename(a.id, title: "")
        XCTAssertNil(reset.title)
        XCTAssertNil(reset.titleIsUser)

        try await archive.delete(a.id)
        let remaining = try await archive.list().map(\.id)
        XCTAssertEqual(remaining, [b.id])
        do { try await archive.delete(a.id); XCTFail("zweites Löschen muss scheitern") } catch VoiceError.missing {}
        let leftovers = try FileManager.default.contentsOfDirectory(atPath: folder.path).filter { $0.hasPrefix(".delete-") }
        XCTAssertTrue(leftovers.isEmpty)

        // Rest eines abgebrochenen Löschvorgangs: unsichtbar und beim nächsten Öffnen entfernt.
        try FileManager.default.createDirectory(at: folder.appendingPathComponent(".delete-\(UUID().uuidString)"), withIntermediateDirectories: true)
        let reopened = try MeetingArchive(root: folder, quota: 1000)
        let listed = try await reopened.list().map(\.id)
        XCTAssertEqual(listed, [b.id])
        XCTAssertFalse(try FileManager.default.contentsOfDirectory(atPath: folder.path).contains { $0.hasPrefix(".delete-") })
    }
}
