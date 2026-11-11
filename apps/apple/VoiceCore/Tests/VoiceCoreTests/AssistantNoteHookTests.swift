import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket A, E4.2b, Klasse B, B8): die Weiche zwischen dem Speicher der Notizen und der Meldestelle der
// Quellen (`NoteChangeHook`). Der Speicher (`DurableStore`) gehört auch zur Uhr; ohne eingehängte Meldestelle läuft jede Änderung
// unverändert. Die Tests des Speichers selbst brauchen CryptoKit und laufen nur dort, wo es das gibt (Mac). Nur synthetische Daten.

private struct QuietModel: BookTalkModel {
    func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
        AsyncThrowingStream { $0.finish() }
    }
}

private struct Boom: Error {}

private final class Events: @unchecked Sendable {
    private let lock = NSLock()
    private var items: [String] = []
    func add(_ s: String) { lock.withLock { items.append(s) } }
    var all: [String] { lock.withLock { items } }
}

final class AssistantNoteHookTests: XCTestCase {
    private let config = AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil, timeout: 5)

    override func tearDown() { NoteChangeHook.shared.install(nil) }

    func testTheBeginComesBeforeTheChangeAndTheFinishAfterIt() {
        let hook = NoteChangeHook()
        let events = Events()
        hook.install {
            events.add("begin")
            return { events.add("finish") }
        }
        let result = hook.around { () -> Int in
            events.add("change")
            return 42
        }
        XCTAssertEqual(result, 42)
        XCTAssertEqual(events.all, ["begin", "change", "finish"])
    }

    func testTheFinishComesAlsoWhenTheChangeThrowsAndTheErrorPasses() {
        let hook = NoteChangeHook()
        let events = Events()
        hook.install {
            events.add("begin")
            return { events.add("finish") }
        }
        XCTAssertThrowsError(try hook.around { () throws -> Void in
            events.add("change")
            throw Boom()
        }) { XCTAssertTrue($0 is Boom) }
        XCTAssertEqual(events.all, ["begin", "change", "finish"])
    }

    func testWithoutAnInstalledWatchTheChangeRunsUnchangedAsOnTheWatch() {
        let hook = NoteChangeHook()
        var ran = 0
        hook.around { ran += 1 }
        XCTAssertEqual(ran, 1)
        hook.install { { } }
        hook.install(nil)
        hook.around { ran += 1 }
        XCTAssertEqual(ran, 2, "ausgehängt: wieder unverändert")
    }

    /// Mit der echten Meldestelle: Beim Schreiben der Notiz ist die Generation schon gestiegen und der Bestand `stale`; nach der
    /// Änderung bleibt er es, bis ein Durchgang NACH ihr endet.
    func testTheRealWatchHasRaisedTheGenerationBeforeTheNoteIsWrittenAndTheStockStaysStaleUntilAPassEnds() {
        let watch = AssistantContentWatch()
        let hook = NoteChangeHook()
        hook.install {
            let ticket = watch.willChange()
            return { watch.changed(ticket) }
        }
        let s = AssistantSession(model: QuietModel(), knowledge: AssistantKnowledge(corpus: AssistantCorpus()), config: config, released: true)
        watch.attach(s)
        XCTAssertTrue(watch.passCompleted(mark: watch.mark))
        let g = s.releaseGeneration
        var duringTheWrite = (generation: 0, ready: true)
        hook.around { duringTheWrite = (s.releaseGeneration, s.isContentReady) }
        XCTAssertEqual(duringTheWrite.generation, g + 1, "VOR dem Schreiben gestiegen")
        XCTAssertFalse(duringTheWrite.ready)
        XCTAssertFalse(s.isContentReady, "die Änderung allein macht den Bestand nicht bereit")
        XCTAssertTrue(watch.passCompleted(mark: watch.mark))
        XCTAssertTrue(s.isContentReady)
    }

    func testAnOuterBracketKeepsTheMarkBackUntilTheViewIsUpdatedToo() {
        // So hält `VoiceModel.deleteNote` es: Klammer um Löschen und Neuaufbau der Sicht; die Meldung des Speichers liegt darin.
        let watch = AssistantContentWatch()
        let hook = NoteChangeHook()
        hook.install {
            let ticket = watch.willChange()
            return { watch.changed(ticket) }
        }
        let outer = watch.willChange()
        hook.around { }   // der Speicher hat die Notiz entfernt (Meldung davon beendet) ...
        let markWhileTheViewIsOld = watch.mark   // ... die Sicht führt sie noch: ein Durchgang jetzt sähe sie noch
        watch.changed(outer)
        XCTAssertEqual(markWhileTheViewIsOld, 0, "die äußere Klammer ist offen: die Marke deckt die Änderung nicht")
        XCTAssertGreaterThan(watch.mark, markWhileTheViewIsOld)
    }

    #if canImport(CryptoKit)
    // MARK: Speicher der Notizen (nur wo CryptoKit vorhanden ist)

    private func makeStore() throws -> (DurableStore, URL) {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("note-hook-\(UUID().uuidString)")
        return (try DurableStore(root: root), root)
    }

    func testDeletingANoteIsAnnouncedBeforeTheFolderMovesAndFinishedAfterwards() throws {
        let (store, root) = try makeStore()
        defer { try? FileManager.default.removeItem(at: root) }
        let capture = Packet.capture(audio: Data([1]))
        _ = try store.accept(capture)
        let folder = root.appendingPathComponent(capture.sessionId.uuidString)
        let events = Events()
        NoteChangeHook.shared.install {
            events.add("begin, Ordner da: \(FileManager.default.fileExists(atPath: folder.path))")
            return { events.add("finish, Ordner da: \(FileManager.default.fileExists(atPath: folder.path))") }
        }
        try store.deleteEntry(capture.sessionId)
        XCTAssertEqual(events.all, ["begin, Ordner da: true", "finish, Ordner da: false"])
    }

    func testOnlyTextChangesOfANoteAreAnnouncedNeverStateOrTiming() throws {
        let (store, root) = try makeStore()
        defer { try? FileManager.default.removeItem(at: root) }
        let capture = Packet.capture(audio: Data([1]))
        _ = try store.accept(capture)
        let events = Events()
        NoteChangeHook.shared.install {
            events.add("begin")
            return { events.add("finish") }
        }
        try store.update(capture.sessionId, state: .accepted, timing: ("x_ms", 1))
        XCTAssertEqual(events.all, [], "Stand und Zeiten betreffen den Bestand nicht")
        try store.update(capture.sessionId, transcript: "Ein dauerhaftes Transkript.", state: .deferred)
        XCTAssertEqual(events.all, ["begin", "finish"], "das Transkript ändert den Bestand")
        try store.update(capture.sessionId, reply: "Eine Antwort.", state: .answered)
        XCTAssertEqual(events.all, ["begin", "finish", "begin", "finish"], "ebenso die Antwort")
    }

    func testAnIncomingReplyIsAnnouncedBeforeItIsWritten() throws {
        let (store, root) = try makeStore()
        defer { try? FileManager.default.removeItem(at: root) }
        let capture = Packet.capture(audio: Data([1]))
        _ = try store.accept(capture)
        let events = Events()
        let entryFile = root.appendingPathComponent(capture.sessionId.uuidString).appendingPathComponent("entry.json")
        func stored() -> Bool { ((try? String(contentsOf: entryFile, encoding: .utf8)) ?? "").contains("Eine Antwort.") }
        NoteChangeHook.shared.install {
            events.add("begin, Antwort gespeichert: \(stored())")
            return { events.add("finish, Antwort gespeichert: \(stored())") }
        }
        _ = try store.acceptReply(VoiceEnvelope(sessionId: capture.sessionId, reply: "Eine Antwort."))
        XCTAssertEqual(events.all, ["begin, Antwort gespeichert: false", "finish, Antwort gespeichert: true"])
    }

    func testAFailedDeleteStillFinishesTheAnnouncement() throws {
        let (store, root) = try makeStore()
        defer { try? FileManager.default.removeItem(at: root) }
        let events = Events()
        NoteChangeHook.shared.install {
            events.add("begin")
            return { events.add("finish") }
        }
        XCTAssertThrowsError(try store.deleteEntry(UUID()))
        XCTAssertEqual(events.all, ["begin", "finish"], "der Abschluss kommt auch bei einem Fehler")
    }
    #endif
}
