import XCTest
@testable import VoiceCore

final class LibraryTests: XCTestCase {
    func testDecodesDesktopBookJson() throws {
        let json = """
        {"id":"book_1730","title":"Die Verwandlung","subtitle":null,"authors":["Franz Kafka"],"language":"de",
         "isbn":null,"publisher":null,"publish_date":"1915","pages":null,"description":null,"metadata_source":"manual",
         "cover_source":"epub","text":{"page_id":"p_1","origin":"document","source_name":"kafka.epub"},
         "audio":{"file":"teil_001.mp3","duration_ms":1000,"narrator":null,"transcript_job":null,
           "tracks":[{"file":"teil_001.mp3","title":"Teil 1","duration_ms":1000,"transcript_job":null},
                     {"file":"teil_002.mp3","title":"Teil 2","duration_ms":2000,"transcript_job":null}],
           "sync":[{"chars":10,"ms":500}]},
         "added_ms":1730000000000,"finished":false,"cover_path":"/abs/cover.jpg","audio_path":null,"audio_paths":[]}
        """
        let book = try JSONDecoder().decode(LibraryBook.self, from: Data(json.utf8))
        XCTAssertEqual(book.authors, ["Franz Kafka"])
        XCTAssertEqual(book.text?.pageId, "p_1")
        XCTAssertEqual(book.audio?.allTracks.count, 2)
        XCTAssertEqual(book.audio?.totalDurationMs, 3000)
        XCTAssertEqual(book.audio?.sync, [LibrarySyncPoint(chars: 10, ms: 500)])
        // Rundreise behält die Schlüssel des Desktops.
        let back = try JSONSerialization.jsonObject(with: JSONEncoder().encode(book)) as! [String: Any]
        XCTAssertNotNil(back["added_ms"]); XCTAssertNotNil(back["cover_source"])
    }

    func testLegacyAudioWithoutTracksHasOnePart() throws {
        let audio = try JSONDecoder().decode(LibraryAudioVersion.self, from: Data(#"{"file":"buch.m4b","duration_ms":42}"#.utf8))
        XCTAssertEqual(audio.allTracks, [LibraryAudioTrack(file: "buch.m4b", title: "buch", durationMs: 42)])
    }

    func testMergeRanges() {
        XCTAssertEqual(LibraryProgress.mergeRanges([[5, 8], [0, 3], [3, 4], [7, 10], [2, 2], [.nan, 4]]), [[0, 4], [5, 10]])
    }

    func testMergeKeepsNewerPositionAndUnionsRanges() {
        let pc = ProgressRecord(offset: 900, anchor: "PC", length: 1000, heard: [[0, 500]], updated: 200, unit: .ms)
        let phone = ProgressRecord(offset: 300, length: 0, heard: [[600, 700]], read: [[0, 50]], updated: 100, finished: true)
        let merged = LibraryProgress.merge(phone, pc)
        XCTAssertEqual(merged.offset, 900)
        XCTAssertEqual(merged.anchor, "PC")
        XCTAssertEqual(merged.heard, [[0, 500], [600, 700]])
        XCTAssertEqual(merged.read, [[0, 50]])
        XCTAssertEqual(merged.length, 1000)
        XCTAssertEqual(merged.finished, true)
        XCTAssertEqual(LibraryProgress.merge(pc, phone), merged, "Reihenfolge darf das Ergebnis nicht ändern")
    }

    func testMergeTakesAnchorFromOlderWhenNewerHasNone() {
        let older = ProgressRecord(offset: 1, anchor: "alt", length: 10, updated: 1, unit: .chars)
        let newer = ProgressRecord(offset: 2, length: 10, updated: 2)
        let merged = LibraryProgress.merge(older, newer)
        XCTAssertEqual(merged.anchor, "alt"); XCTAssertEqual(merged.unit, .chars); XCTAssertEqual(merged.offset, 2)
    }

    func testRecordPositionCountsSmallStepsOnly() {
        var rec = LibraryProgress.recordPosition(nil, to: 60_000, length: 1_000_000, unit: .ms, now: 1)
        XCTAssertEqual(rec.heard, [[0, 60_000]])
        rec = LibraryProgress.recordPosition(rec, to: 500_000, length: 0, unit: .ms, now: 2)
        XCTAssertEqual(rec.heard, [[0, 60_000]], "großer Sprung zählt nicht als gehört")
        XCTAssertEqual(rec.offset, 500_000); XCTAssertEqual(rec.length, 1_000_000)
    }

    func testAlmostEverythingHeardIsFinished() {
        let rec = LibraryProgress.recordPosition(ProgressRecord(offset: 0, length: 1000, heard: [[0, 0]], updated: 0),
                                                 to: 990, length: 1000, now: 1)
        XCTAssertEqual(rec.finished, true)
        XCTAssertEqual(LibraryProgress.fraction(rec), 1)
    }

    func testDecodesDesktopProgressText() throws {
        let raw = #"{"offset":12,"length":100,"heard":[[0,12]],"read":[],"updated":5,"unit":"chars"}"#
        let rec = try JSONDecoder().decode(ProgressRecord.self, from: Data(raw.utf8))
        XCTAssertEqual(LibraryProgress.fraction(rec), 0.12, accuracy: 1e-9)
    }

    func testParagraphsUseUtf16OffsetsLikeJavaScript() {
        // "ü" und das Emoji: 1 bzw. 2 UTF-16-Einheiten.
        let text = "# Kapitel\n\nÜber 😀 alles\n\n\nEnde"
        let ps = LibraryText.paragraphs(text)
        XCTAssertEqual(ps.map(\.text), ["Kapitel", "Über 😀 alles", "Ende"])
        XCTAssertEqual(ps.map(\.heading), [true, false, false])
        XCTAssertEqual(ps.map(\.start), [0, 11, 27])
        XCTAssertEqual(LibraryText.paragraphIndex(ps, at: 12), 1)
        XCTAssertEqual(LibraryText.paragraphIndex(ps, at: 26), 1)
    }

    func testFollowMapInterpolatesBothWays() {
        let map = FollowMap(sync: [LibrarySyncPoint(chars: 100, ms: 10_000), LibrarySyncPoint(chars: 50, ms: 20_000)],
                            textLength: 200, totalMs: 30_000)
        XCTAssertFalse(map.approximate)
        XCTAssertEqual(map.ms(atChars: 50), 20_000, "nicht monotone Zeit wird angehoben, nicht gesprungen")
        XCTAssertEqual(map.ms(atChars: 150), 25_000)
        XCTAssertEqual(map.chars(atMs: 25_000), 150)
        XCTAssertEqual(map.chars(atMs: 99_000), 200)
        XCTAssertTrue(FollowMap(sync: [], textLength: 10, totalMs: 10).approximate)
    }

    func testTrackPositionAcrossParts() {
        let tracks = [LibraryAudioTrack(file: "a", title: "a", durationMs: 1000), LibraryAudioTrack(file: "b", title: "b", durationMs: 2000)]
        XCTAssertEqual(LibraryTracks.position(of: 1500, in: tracks), TrackPosition(index: 1, offsetMs: 500))
        XCTAssertEqual(LibraryTracks.position(of: 9999, in: tracks), TrackPosition(index: 1, offsetMs: 2000))
        XCTAssertEqual(LibraryTracks.total(TrackPosition(index: 1, offsetMs: 500), in: tracks), 1500)
    }

    func testSentencesCoverParagraphWithoutEdges() {
        let text = "Es war spät. Dr. Meier ging z. B. nach Hause!  War das klug? Niemand wusste es."
        let parts = LibraryText.sentences(text).map { (text as NSString).substring(with: $0) }
        XCTAssertEqual(parts, ["Es war spät.", "Dr. Meier ging z. B. nach Hause!", "War das klug?", "Niemand wusste es."])
        let long = "Er traf gestern Abend Prof. Dr. Müller im Café. Dann ging er."
        XCTAssertEqual(LibraryText.sentences(long).count, 2)
        XCTAssertEqual(LibraryText.sentences(""), [])
        XCTAssertEqual(LibraryText.sentences("Überschrift ohne Punkt").count, 1)
    }

    func testSentenceIndexFindsSavedPosition() {
        let ranges = LibraryText.sentences("Erster Satz hier. Zweiter Satz hier. Dritter Satz hier.")
        XCTAssertEqual(LibraryText.sentenceIndex(ranges, at: 0), 0)
        XCTAssertEqual(LibraryText.sentenceIndex(ranges, at: ranges[1].location), 1)
        XCTAssertEqual(LibraryText.sentenceIndex(ranges, at: ranges[2].location + 3), 2)
        XCTAssertEqual(LibraryText.sentenceIndex([], at: 5), 0)
    }

    func testHeadingTextStartSkipsMarker() {
        let p = LibraryText.paragraphs("# Titel\n\nText")
        XCTAssertEqual(p[0].textStart, 2)
        XCTAssertEqual(p[1].textStart, p[1].start)
    }

    func testFavoritesComeFirstAndKeepOrder() {
        var books = ["a", "b", "c", "d"].map { LibraryBook(id: $0, title: $0, addedMs: 1) }
        books[1].favorite = true
        books[3].favorite = true
        XCTAssertEqual(LibraryText.favoritesFirst(books).map(\.id), ["b", "d", "a", "c"])
    }

    func testSortRecentlyAddedNewestFirstFavoritesBeforeAndLegacyLast() {
        var books = [
            LibraryBook(id: "a", title: "Effi Briest", authors: ["Theodor Fontane"], addedMs: 2),
            LibraryBook(id: "b", title: "Der Schimmelreiter", authors: ["Theodor Storm"], addedMs: 1),
            LibraryBook(id: "c", title: "Die Verwandlung", authors: ["Franz Kafka"], addedMs: 3),
            LibraryBook(id: "z", title: "Zauberberg", addedMs: 0),
            LibraryBook(id: "y", title: "Ärger", addedMs: 0),
        ]
        XCTAssertEqual(LibraryText.sorted(books, by: .added).map(\.id), ["c", "a", "b", "y", "z"],
                       "neueste zuerst, Altbestand ohne Datum am Ende nach Titel (Ä wie A)")
        books[1].favorite = true
        XCTAssertEqual(LibraryText.sorted(books, by: .added).map(\.id), ["b", "c", "a", "y", "z"])
    }

    func testSortRecentTitleAuthorMatchDesktop() {
        let books = [
            LibraryBook(id: "a", title: "Effi Briest", authors: ["Theodor Fontane"], addedMs: 2),
            LibraryBook(id: "b", title: "Der Schimmelreiter", authors: ["Storm, Theodor"], addedMs: 1),
            LibraryBook(id: "c", title: "Die Verwandlung", authors: ["Franz Kafka"], addedMs: 3),
        ]
        let used: [String: Double] = ["b": 50]
        XCTAssertEqual(LibraryText.sorted(books, by: .recent) { used[$0.id] ?? 0 }.map(\.id), ["b", "c", "a"],
                       "zuletzt benutzt, sonst Importdatum")
        XCTAssertEqual(LibraryText.sorted(books, by: .title).map(\.id), ["b", "c", "a"])
        XCTAssertEqual(LibraryText.sorted(books, by: .author).map(\.id), ["a", "c", "b"], "Fontane, Kafka, Storm")
    }

    func testStoredSortFallsBackToRecent() {
        XCTAssertEqual(LibrarySort.stored("added"), .added)
        XCTAssertEqual(LibrarySort.stored("unbekannt"), .recent)
        XCTAssertEqual(LibrarySort.allCases.map(\.rawValue), ["recent", "added", "title", "author"], "wie Desktop")
    }

    func testResumeEstimates() {
        XCTAssertEqual(LibraryText.listenMilliseconds(chars: 900), 60_000)
        XCTAssertEqual(LibraryText.listenMilliseconds(chars: -5), 0)
    }

    func testSentencePartsKeepEveryWord() {
        let sentence = "Dr. Meier ging wie immer zuerst zum Hafen, dann in die Stadt, wo ihn niemand erwartete und auch sonst keiner nach ihm fragte."
        let parts = LibraryText.sentenceParts(sentence, maxChars: 50)
        XCTAssertGreaterThan(parts.count, 1)
        XCTAssertEqual(parts.joined(separator: " "), sentence)
        XCTAssertTrue(parts.allSatisfy { $0.count <= 50 })
        XCTAssertEqual(LibraryText.sentenceParts("Kurz  und\ngut.", maxChars: 50), ["Kurz und gut."])
        XCTAssertEqual(LibraryText.sentenceParts("", maxChars: 50), [])
        let long = LibraryText.sentenceParts("Donaudampfschifffahrtsgesellschaft ja", maxChars: 10)
        XCTAssertTrue(long.allSatisfy { $0.count <= 10 }, "überlange Wörter werden geteilt")
        XCTAssertEqual(long.joined().replacingOccurrences(of: " ", with: ""), "Donaudampfschifffahrtsgesellschaftja")
        XCTAssertTrue(LibraryText.sentenceParts(String(repeating: "字", count: 30), maxChars: 8).allSatisfy { $0.count <= 8 }, "Schrift ohne Leerzeichen")
    }

    func testDashesInsideWordsAreBreakPoints() {
        for dash in ["—", "–"] {
            let word = "eins\(dash)zwei\(dash)drei"
            let parts = LibraryText.sentenceParts("Heute \(word) morgen", maxChars: 10)
            XCTAssertEqual(parts.joined().replacingOccurrences(of: " ", with: ""), "Heute" + word + "morgen", "kein Zeichen geht verloren")
            XCTAssertTrue(parts.contains { $0.hasSuffix(dash) }, "getrennt wird nach dem Strich, nicht mitten im Wort: \(parts)")
            XCTAssertTrue(parts.allSatisfy { !$0.contains("ein ") && $0.count <= 10 })
        }
        // Passt alles, bleibt das Wort samt Strich zusammen, ohne Leerzeichen.
        XCTAssertEqual(LibraryText.sentenceParts("eins—zwei", maxChars: 50), ["eins—zwei"])
    }

    func testPartsFollowMeasuredFit() {
        // Breite Zeichen zählen doppelt: Die Grenze kommt aus der Messung, nicht aus der Zeichenzahl.
        let fits: (String) -> Bool = { text in text.reduce(0) { $0 + ($1 == "W" || $1 == "字" ? 2 : 1) } <= 10 }
        for sentence in ["WWWWWWWWWWWW", String(repeating: "字", count: 12), "Wald Wiese WWW Weg"] {
            XCTAssertTrue(LibraryText.sentenceParts(sentence, fits: fits).allSatisfy(fits), sentence)
        }
    }

    func testSentencePartFollowsSpokenTime() {
        let parts = ["aaaa", "bbbb", "cc"]
        XCTAssertEqual(LibraryText.sentencePartIndex(parts, elapsed: 0, total: 10), 0)
        XCTAssertEqual(LibraryText.sentencePartIndex(parts, elapsed: 5, total: 10), 1)
        XCTAssertEqual(LibraryText.sentencePartIndex(parts, elapsed: 9, total: 10), 2)
        XCTAssertEqual(LibraryText.sentencePartIndex(parts, elapsed: 99, total: 10), 2, "Ton langsamer als geschätzt: bleibt am letzten Teil")
        XCTAssertEqual(LibraryText.sentencePartIndex(parts, elapsed: 5, total: 0), 0)
    }
}
