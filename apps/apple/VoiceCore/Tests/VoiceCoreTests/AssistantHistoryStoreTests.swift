import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket K): Verlaufsdatei. Nur Verzeichnisse unter dem Temp-Ordner des Tests.

final class AssistantHistoryStoreTests: XCTestCase {
    private var dir: URL!

    override func setUp() {
        dir = FileManager.default.temporaryDirectory.appendingPathComponent("assistant-history-\(UUID().uuidString)", isDirectory: true)
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: dir)
    }

    private func turn(_ n: Int) -> AssistantTurn {
        AssistantTurn(
            user: "Frage \(n) mit Umlauten äöüß", assistant: "Antwort \(n)", interrupted: n % 2 == 0,
            sources: n % 3 == 0 ? [AssistantPassage(area: .books, itemId: "b\(n)", title: "Buch", locator: "S. \(n)", text: "Text", score: 0.5)] : [],
            general: n % 3 != 0, model: "Apple", at: Date(timeIntervalSince1970: 1_760_000_000 + Double(n)))
    }

    /// So, wie `save` sie schreibt: Quellen ohne Auszugstext und ohne Wertung.
    private func stored(_ n: Int) -> AssistantTurn { AssistantHistoryStore.minimized(turn(n)) }

    func testRoundTrip() throws {
        let store = AssistantHistoryStore(directory: dir)
        XCTAssertEqual(store.load(), [])
        XCTAssertFalse(store.hasAny)
        let turns = (0..<5).map(turn)
        try store.save(turns)
        XCTAssertTrue(store.hasAny)
        XCTAssertEqual(store.load(), turns.map(AssistantHistoryStore.minimized))
        XCTAssertEqual(AssistantHistoryStore(directory: dir).load(), turns.map(AssistantHistoryStore.minimized))
        // Alles außer dem Auszugstext der Quellen bleibt erhalten.
        let loaded = store.load()
        XCTAssertEqual(loaded.map(\.user), turns.map(\.user))
        XCTAssertEqual(loaded.map(\.assistant), turns.map(\.assistant))
        XCTAssertEqual(loaded[3].sources.first?.title, "Buch")
        XCTAssertEqual(loaded[3].sources.first?.locator, "S. 3")
        XCTAssertEqual(loaded[3].sources.first?.area, .books)
        XCTAssertEqual(loaded[3].sources.first?.itemId, "b3")
    }

    /// Auszugstexte (Buch, Transkript, Ablage, Notiz) stehen nie in der Verlaufsdatei, auch nicht als Rohtext.
    func testSourceTextNeverReachesTheFile() throws {
        let store = AssistantHistoryStore(directory: dir)
        let secret = "GEHEIMER-AUSZUG-aus-der-Ablage"
        let sources = AssistantArea.allCases.map {
            AssistantPassage(area: $0, itemId: "i-\($0.rawValue)", title: "Titel \($0.rawValue)", locator: "Stelle", text: secret, score: 0.9)
        }
        try store.save([AssistantTurn(user: "Frage", assistant: "Antwort", sources: sources, general: false, model: "Apple")])
        let raw = try String(contentsOf: store.url, encoding: .utf8)
        XCTAssertFalse(raw.contains(secret), raw)
        let loaded = store.load()
        XCTAssertEqual(loaded.first?.sources.count, 4)
        XCTAssertEqual(loaded.first?.sources.map(\.text), Array(repeating: "", count: 4))
        XCTAssertEqual(loaded.first?.sources.map(\.itemId), sources.map(\.itemId))
        XCTAssertEqual(loaded.first?.sources.map(\.area), sources.map(\.area))
    }

    /// Ein Verlauf mit gekürzten Quellen geht ohne Weiteres wieder in eine Sitzung und in die Anzeige.
    func testStoredTurnsWithoutSourceTextStayUsable() async throws {
        let store = AssistantHistoryStore(directory: dir)
        try store.save((0..<5).map(turn))
        let loaded = store.load()
        let chips = loaded.flatMap(\.sources).map { "\($0.area.rawValue)|\($0.title)|\($0.locator ?? "")" }
        XCTAssertEqual(chips, ["books|Buch|S. 0", "books|Buch|S. 3"])
        let built = AssistantPrompt.build(question: "Weiter?", afterInterruption: false, inventory: "", passages: [], history: loaded,
                                          config: AssistantConfig(modelLabel: "T", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil, timeout: 5))
        XCTAssertTrue(built.prompt.contains("Antwort 4"), built.prompt)
        XCTAssertTrue(built.used.isEmpty)
    }

    func testKeepsOnlyLast200() throws {
        let store = AssistantHistoryStore(directory: dir)
        try store.save((0..<260).map(turn))
        let loaded = store.load()
        XCTAssertEqual(loaded.count, 200)
        XCTAssertEqual(loaded.first, stored(60))
        XCTAssertEqual(loaded.last, stored(259))
    }

    func testEmptySaveAndClearDeleteFile() throws {
        let store = AssistantHistoryStore(directory: dir)
        try store.save([turn(1)])
        XCTAssertTrue(FileManager.default.fileExists(atPath: store.url.path))
        try store.save([])
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.url.path))
        try store.save([turn(1)])
        store.clear()
        XCTAssertEqual(store.load(), [])
        XCTAssertFalse(store.hasAny)
    }

    func testCorruptFileIsSetAsideAndNewerVersionIsLeftAlone() throws {
        let store = AssistantHistoryStore(directory: dir)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try Data("kaputt".utf8).write(to: store.url)
        XCTAssertEqual(store.load(), [])
        XCTAssertTrue(FileManager.default.fileExists(atPath: store.url.appendingPathExtension("defekt").path))
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.url.path))

        let future = #"{"version":99,"turns":[]}"#
        try Data(future.utf8).write(to: store.url)
        XCTAssertEqual(store.load(), [])
        XCTAssertEqual(try String(contentsOf: store.url, encoding: .utf8), future, "neuere Fassung bleibt unangetastet")
    }

    func testStandardDirectoryIsSeparateFromBookTalk() {
        XCTAssertEqual(AssistantHistoryStore.standardDirectory().lastPathComponent, "Assistant")
        XCTAssertNotEqual(AssistantHistoryStore.standardDirectory(), BookTalkHistoryStore.standardDirectory())
    }
}
