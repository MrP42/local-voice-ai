import XCTest
@testable import VoiceCore

final class TextAufbereitenTests: XCTestCase {
    private func result(_ text: String, _ stage: TextAufbereiten.Stufe = .bereinigung) throws -> String {
        let draft = TextAufbereiten.vorschlaege(text, stufe: stage)
        return try draft.anwenden(angenommen: Set(draft.aenderungen.map(\.id)), auf: text)
    }

    // Eingabe/Ausgabe aus scripts/paste-cleanup.test.mjs und textCleanup.ts.
    func testDesktopBereinigung() throws {
        for (input, output) in [
            ("Hallo  Welt   12", "Hallo  Welt   12"),
            ("Erste Zeile des Textes.\r\nZweite Zeile des Textes.\r\n", "Erste Zeile des Textes.\nZweite Zeile des Textes."),
            ("Die Mit­arbeiter​ informieren﻿ sich hier.\nZweite Zeile folgt.", "Die Mitarbeiter informieren sich hier.\nZweite Zeile folgt."),
            ("Die Mit­\narbeiterinnen sind da.", "Die Mitarbeiterinnen sind da."),
            ("Ge-\ndanken", "Gedanken"),
            ("Wirtschafts-\nund Sozialpolitik", "Wirtschafts- und Sozialpolitik"),
            ("J.\nL.\nCollins hat begonnen.", "J. L. Collins hat begonnen."),
            ("Text.\n\n12\n\nEnde.", "Text.\n\nEnde."),
            ("```\n12\nGe-\ndanken  \u{200b}\n```", "```\n12\nGe-\ndanken  \u{200b}\n```"),
            ("A\n\n\n\nB", "A\n\nB"),
            ("", "")
        ] { XCTAssertEqual(try result(input), output, "Desktop-Vorlage") }
    }

    // Gleiche Paare wie format.rs (Regeln absichtlich getrennt von Paste-Regeln).
    func testDesktopFormat() throws {
        for (input, output) in [
            ("Cafe\u{301}", "Café"),
            ("\u{1100}\u{1161} A\u{301}", "가 Á"),
            ("ﬀ ﬁ ﬂ ﬃ ﬄ ﬅ ﬆ", "ff fi fl ffi ffl st st"),
            ("Sil\u{ad}be", "Silbe"),
            ("\"Hallo\" (\"Welt\")", "„Hallo“ („Welt“)"),
            ("Wort - Wort", "Wort – Wort"),
            ("Warte... .... .....", "Warte… .... ....."),
            ("A . B : C , D ; E ! F ?", "A. B: C, D; E! F?"),
            ("1) Eins\n2) Zwei", "1. Eins\n2. Zwei"),
            ("*\"Hallo\"* _\"Welt\"_", "*„Hallo“* _„Welt“_"),
            ("\"Offen\n\nEnde\"\n\n\"Gut\"", "\"Offen\n\nEnde\"\n\n„Gut“"),
            ("`x...y` Ende...", "`x...y` Ende…"),
            ("A\u{ad}\u{301}", "Á")
        ] { XCTAssertTrue(TextAufbereiten.identisch(try result(input, .format), output), "Desktop-Vorlage") }
        for input in ["A  \nB", "A   \r\nB", "10 km 5 Äpfel", "> Wort - Wort", "| A  B\t | C   |  \n", "Er sagte:\"Hallo\""] {
            XCTAssertEqual(try result(input, .format), input)
        }
    }

    func testDesktopPageNumbersAndFragmentsCounterprobes() throws {
        for s in ["12", "- 12 -", "– 12 –", "Seite 3", "Seite 3 von 30", "S. 3", "3 / 30", "Page 3 of 30"] {
            XCTAssertTrue(TextAufbereitenPaste.isPageNumber(s))
        }
        for s in ["2026", "1.", "2)", "Seite 3 ist leer", "12 Uhr", "30 / 3", "1234", ""] {
            XCTAssertFalse(TextAufbereitenPaste.isPageNumber(s))
        }
        let raw = "Die Abstimmung zwischen Bund-\nund Länderebene ist wichtig, weil die Zustän-\ndigkeiten sich überschneiden.\nDas gilt auch für die Ge-\nmeinden im Kreisgebiet."
        XCTAssertEqual(try result(raw), "Die Abstimmung zwischen Bund- und Länderebene ist wichtig, weil die Zuständigkeiten sich überschneiden.\nDas gilt auch für die Gemeinden im Kreisgebiet.")
        let wrapped = "Der Bericht wurde im vergangenen Jahr erstellt und\n- 14 -\nan alle Ämter im Kreisgebiet verschickt."
        XCTAssertEqual(try result(wrapped), "Der Bericht wurde im vergangenen Jahr erstellt und an alle Ämter im Kreisgebiet verschickt.")
        for unchanged in [
            "Aufgaben bis Freitag:\n1. Protokoll prüfen\n2. Raum buchen\n3. Einladung versenden",
            "(10:30) Begrüßung\n(10:45) Bericht der Leitung\n(11:15) Aussprache und Beschlüsse\n[12:00] Mittagspause"
        ] { XCTAssertEqual(try result(unchanged), unchanged) }
    }

    func testDesktopFormatProtectionAndLists() throws {
        for bullet in ["•", "·", "‣", "◦", "▪", "●"] {
            XCTAssertEqual(try result(bullet + " Punkt", .format), "- Punkt")
            XCTAssertEqual(try result("   " + bullet + "\t🦀", .format), "   - 🦀")
        }
        XCTAssertEqual(try result("18446744073709551615) Eins\n18446744073709551616) Zwei", .format), "18446744073709551615. Eins\n18446744073709551616. Zwei")
        XCTAssertEqual(try result("  9) Neun\r\n  10) Zehn\r\n11) Elf\r\n", .format), "  9. Neun\r\n  10. Zehn\r\n11. Elf\r\n")
        for unchanged in [
            "1) Punkt", "2) Zwei\n4) Vier", "1) Eins\n\n2) Zwei", "1)\tEins\n2)\tZwei",
            "```\n• x\n1) Eins\n2) Zwei\n```", "~~~\n• x\n~~~", "`• x`\n``1) Eins``",
            "| • |\n1) Eins | A\n2) Zwei | B", "> • x\n> 1) Eins\n> 2) Zwei",
            "<Anna> • x\n<Anna> 1) Eins\n<Anna> 2) Zwei", "[• x]\n<• x>\n{{S. 12}}",
            "    • Punkt\n    1) Eins\n    2) Zwei"
        ] { XCTAssertEqual(try result(unchanged, .format), unchanged) }
        XCTAssertEqual(try result("[offen\n\nEnde...]", .format), "[offen\n\nEnde…]")
        XCTAssertEqual(try result("[offen\rEnde...]", .format), "[offen\rEnde…]")
        let allowed = "[" + String(repeating: "ä", count: 77) + "...]"
        XCTAssertEqual(try result(allowed, .format), allowed)
        XCTAssertEqual(try result("[" + String(repeating: "ä", count: 78) + "...]", .format), "[" + String(repeating: "ä", count: 78) + "…]")
    }

    func testDesktopFormatControlsAndIdempotence() throws {
        for unchanged in [
            "Ein sauberer Absatz mit Satzzeichen, Umlauten und Schluss.",
            "{{S. 3}} <Anna> Hallo. [lacht]", "Rheinland-Pfalz\n- Punkt\n  - Punkt\n1 - 3 und 1-3",
            "Englische “Worte” und deutsche „Worte“.", "  Einzug\n\tEinzug\r\n",
            "Datei .env Pegel .5 2 : 1 :-) a ,b a ;b a !b a ?b a ./ a .)",
            "A\tB\t \nC", "A  B | C\t D  \n", "> - Punkt", "> > - Punkt", "| - Punkt |",
            "A - B | C - D", "- - -", "* * *", "---", "Wort\" Wort\""
        ] { XCTAssertTrue(TextAufbereiten.identisch(try result(unchanged, .format), unchanged)) }
        for input in [
            "\"Cafe\u{301}  ﬃ\u{ad} - Ende...\"  ", "{{S. 3}} <Anna> [lacht]  Hallo , Welt...",
            "\"Offen\n\n\"Gut\" - Ende...", "  A\t B\u{a0}C !\r\n- Punkt\n1 - 3",
            ".... ..... ... ﬀ ﬆ Sil\u{ad}be", "\u{e000}\u{e010}\u{e001} [ﬃ  ...] \"Ja\"",
            "A  -\t B und Ende . . .", "A\u{ad}\u{301} ﬃ\u{301}",
            "```rust\n\"a  b\" x...y\n```\nEnde...", "``x...y ` a  b`` und \"Ja\"",
            "> > - Punkt\n> Wort - Wort\n- - -", "| A  B - C | D\t E |\nA - B | C  D",
            "A  \nB   \r\nC", "•\t- x", "•  Punkt\n1) Eins\n2) Zwei", "◦\t\t🦀  Ende...",
            "•\t\u{a0}🦀", "1) \"Ja\"\n2) Wort - Wort"
        ] {
            let once = try result(input, .format)
            XCTAssertTrue(TextAufbereiten.identisch(once, try result(once, .format)))
        }
        let raw = "\"<A\u{301}  ﬃ\u{ad} - ...>\"  {{S. 12}} ...[\"A\u{301}\"  ﬃ\u{ad} - ...] ,"
        let expected = "„<A\u{301}  ﬃ\u{ad} - ...>“ {{S. 12}} …[\"A\u{301}\"  ﬃ\u{ad} - ...],"
        XCTAssertTrue(TextAufbereiten.identisch(try result(raw, .format), expected))
    }

    func testUnicodeRangesSubsetsAndStaleBasis() throws {
        let text = "😀 Cafe\u{301}  ﬃ Ende... 👩‍💻"
        let draft = TextAufbereiten.vorschlaege(text, stufe: .format)
        XCTAssertEqual(draft, TextAufbereiten.vorschlaege(text, stufe: .format))
        var end = 0
        for change in draft.aenderungen {
            XCTAssertGreaterThanOrEqual(change.bereich.location, end)
            XCTAssertNotNil(Range(change.bereich, in: text))
            end = NSMaxRange(change.bereich)
        }
        let chosen = Set(draft.aenderungen.filter { $0.regel == "auslassung" }.map(\.id))
        XCTAssertEqual(try draft.anwenden(angenommen: chosen, auf: text), "😀 Cafe\u{301}  ﬃ Ende… 👩‍💻")
        XCTAssertEqual(try draft.anwenden(angenommen: [], auf: text), text)
        XCTAssertThrowsError(try draft.anwenden(angenommen: chosen, auf: text + "!"))
        XCTAssertThrowsError(try draft.anwenden(angenommen: ["unknown"], auf: text))
        // Swift == setzt kanonisch gleiche Zeichen gleich; die Basisprüfung darf das nicht.
        let decomposed = TextAufbereiten.vorschlaege("e\u{301}", stufe: .format)
        XCTAssertThrowsError(try decomposed.anwenden(angenommen: [], auf: "é"))
    }

    func testQuotePairsAreAcceptedTogetherAndOtherRulesStayIndependent() throws {
        let text = "\"Warte...\"  "
        let draft = TextAufbereiten.vorschlaege(text, stufe: .format)
        let quotes = draft.aenderungen.filter { $0.regel == "anfuehrungszeichen" }
        XCTAssertEqual(quotes.count, 2)
        XCTAssertEqual(quotes.first?.gruppe, quotes.last?.gruppe)
        guard quotes.count == 2 else { return }
        XCTAssertThrowsError(try draft.anwenden(angenommen: [quotes[0].id], auf: text))
        let spaces = Set(draft.aenderungen.filter { $0.regel == "leerzeichen" }.map(\.id))
        XCTAssertEqual(try draft.anwenden(angenommen: spaces, auf: text), "\"Warte...\"")
        XCTAssertEqual(try draft.anwenden(angenommen: Set(quotes.map(\.id)), auf: text), "„Warte...“  ")
    }

    func testOriginTapeHandlesInsertionsDeletionsAndCombiningCharacters() throws {
        for (old, new) in [("A\u{c}\nB", "A\n\nB"), ("A😀B", "😀B!"), ("e\u{301}", "é"), ("", "Probe")] {
            var tape = TextAufbereiten.Tape(old)
            tape.apply(TextAufbereitenPaste.differences(old, new, offset: 0), rule: "Probe")
            XCTAssertTrue(TextAufbereiten.identisch(tape.text, new))
            var end = 0
            for piece in tape.pieces {
                XCTAssertGreaterThanOrEqual(piece.range.location, end)
                XCTAssertNotNil(Range(piece.range, in: old))
                end = NSMaxRange(piece.range)
            }
        }
    }

    @MainActor
    func testPersistenceAndPendingTextCounterprobe() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let e = ReadingPageEditor(store: try ReadingPageStore(root: root))
        var page = try e.create(text: "Warte...", title: "Probe")
        page.setStateField("einfuegungen", .array([.string("synthetische Rohfassung")]))
        page.setStateField("kiDoc", .objekt([("unbekannt", .string("erhalten"))]))
        try e.store.save(page)
        let draft = TextAufbereiten.vorschlaege(page.text, stufe: .format)
        let prepared = try draft.anwenden(angenommen: Set(draft.aenderungen.map(\.id)), auf: page.text)
        let pending = PendingTextBuffer()
        _ = pending.set(page.id, text: "Neue Eingabe")
        try pending.flush(page.id) { try e.updateText($0, text: $1) }
        let current = try e.page(page.id)!.json()
        XCTAssertThrowsError(try e.applyPreparedText(page.id, basis: draft.basis, text: prepared))
        XCTAssertEqual(try e.page(page.id)!.json(), current)
        try e.updateText(page.id, text: draft.basis)
        try e.applyPreparedText(page.id, basis: draft.basis, text: prepared)
        let restored = try ReadingPageStore(root: root).page(page.id)!
        XCTAssertEqual(restored.text, "Warte…")
        XCTAssertEqual(restored.state["einfuegungen"], page.state["einfuegungen"])
        XCTAssertEqual(restored.state["kiDoc"], page.state["kiDoc"])
        try e.store.delete(page.id)
        XCTAssertThrowsError(try e.applyPreparedText(page.id, basis: prepared, text: "anderer Text"))
        XCTAssertNil(try e.page(page.id))
    }

    func testLongTextAndRepeatedHeaders() throws {
        let page = (0..<12).map { "Absatz \($0): Die Verwaltung informiert über den Stand der Umstellung und bittet um Rückmeldung bis Monatsende." }.joined(separator: "\n")
        let raw = (1...4).map { "Landratsamt Musterkreis – Mitarbeiterinfo\n\(page)\nSeite \($0) von 4" }.joined(separator: "\n")
        let cleaned = try result(raw)
        XCTAssertFalse(cleaned.contains("Landratsamt"))
        XCTAssertFalse(cleaned.contains("Seite 1 von 4"))
        XCTAssertTrue(try result("Refrain\n" + page + "\nRefrain\n" + page).contains("Refrain"))
        let long = String(repeating: "😀 ﬃ Ende... Wort - Wort A  B\n", count: 10_000)
        let start = Date()
        XCTAssertEqual(try result(long, .format), String(repeating: "😀 ffi Ende… Wort – Wort A B\n", count: 10_000))
        XCTAssertLessThan(Date().timeIntervalSince(start), 15)
    }
}
