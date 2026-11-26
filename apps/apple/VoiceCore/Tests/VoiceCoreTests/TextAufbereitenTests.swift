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

    // Gegenprobe: optionales Präfix und ICU-Faltung entfernen zusätzliche Inhaltszeilen.
    func testPageNumberPrefixesAndJavaScriptCaseFolding() throws {
        for line in ["3 von 30", "3 of 30", "ſeite 3", "Seİte 3", "Seıte 3", "ſ. 3"] {
            XCTAssertFalse(TextAufbereitenPaste.isPageNumber(line), line)
            let text = "Text.\n" + line + "\nEnde."
            XCTAssertEqual(try result(text), text)
        }
        for line in ["SEITE 3 VON 30", "s.3", "PAGE 3 OF 30", "P.3/30", "3 / 30"] {
            XCTAssertTrue(TextAufbereitenPaste.isPageNumber(line), line)
            XCTAssertEqual(try result("Text.\n" + line + "\nEnde."), "Text.\nEnde.")
        }
        XCTAssertEqual(try result("(00:01) J.\nL.\nCollins hat begonnen."), "(00:01) J. L. Collins hat begonnen.")
    }

    // Gegenprobe: Bereichsfilter lassen geschützte Zeichen in Kontextentscheidungen einfließen.
    func testMaskingPrecedesFormatRulesAndCRResetsDashContext() throws {
        for (input, output) in [
            ("<a...<b>", "<a...<b>"),
            ("<a|b> A  B", "<a|b> A B"),
            ("00:12 - 00:13", "00:12 – 00:13"),
            ("1) \tA\n2) \tB", "1. \tA\n2. \tB"),
            ("A\r - B", "A\r - B"),
            ("<a|b> Wort - Wort", "<a|b> Wort – Wort"),
            ("<a...b> A  B", "<a...b> A B"),
            ("1) \t\n2) \tB", "1) \t\n2) \tB"),
            ("A - B\rC - D", "A – B\rC – D"),
            ("\u{e000}\u{e010}\u{e001} <a|b> A  B", "\u{e000}\u{e010}\u{e001} <a|b> A B")
        ] { XCTAssertTrue(TextAufbereiten.identisch(try result(input, .format), output), input) }
    }

    // Gegenproben: Character-Maskierung verschluckt die Markengrenze; NFC darf sie nicht verschieben.
    func testFormatUnicodeAtProtectedBoundaries() throws {
        for mark in ["<Anna>", "[lacht]", "{{S. 12}}", "00:12", "`Code`", "[]"] {
            for suffix in ["\u{0340}", "\u{0301}", "👩\u{200d}💻", "\u{200d}👩\u{200d}💻", ""] {
                let protected = mark + suffix
                let input = "Cafe\u{301} " + protected
                let output = "Café " + protected
                let draft = TextAufbereiten.vorschlaege(input, stufe: .format)
                XCTAssertEqual(draft.aenderungen.count, 1)
                for change in draft.aenderungen {
                    XCTAssertNotNil(Range(change.bereich, in: input))
                    XCTAssertEqual(change.bereich, NSRange(location: 3, length: 2))
                }
                XCTAssertTrue(TextAufbereiten.identisch(try result(input, .format), output))
                XCTAssertTrue(TextAufbereiten.identisch(try result(protected, .format), protected))
                XCTAssertTrue(TextAufbereiten.vorschlaege(output, stufe: .format).aenderungen.isEmpty)
            }
        }
        // Ein Abschnitt mit unsicherer Grenze bleibt bei NFC vollständig unverändert.
        XCTAssertTrue(TextAufbereiten.identisch(try result("<Anna>\u{0340} Cafe\u{301}", .format), "<Anna>\u{0340} Cafe\u{301}"))
        XCTAssertTrue(TextAufbereiten.identisch(try result("<Anna>\u{0340} ﬃ Ende...", .format), "<Anna>\u{0340} ffi Ende…"))
        XCTAssertTrue(TextAufbereiten.identisch(try result("<>\u{0340}", .format), "<>\u{0300}"))
        XCTAssertEqual(try result("[] <> Ende...", .format), "[] <> Ende…")
    }

    // Gegenprobe: Nur die innere Winkelmarke zu maskieren lässt die äußere Auslassung durch.
    func testFormatUnionsNestedAngleMarksLikeDesktop() throws {
        for mark in ["<a...<b>", "<a...<b...<c>", "<a...<>", "<a[b>c...]", "[a<b]c...>", "<" + String(repeating: "ä", count: 35) + "...<b>"] {
            XCTAssertEqual(try result(mark + " Ende...", .format), mark + " Ende…")
        }
        let tooLong = "<" + String(repeating: "ä", count: 36) + "...<b>"
        XCTAssertEqual(try result(tooLong, .format), "<" + String(repeating: "ä", count: 36) + "…<b>")
        XCTAssertEqual(try result("<a...\n<b>", .format), "<a…\n<b>")
    }

    // Literale UTF-16-Grenzen aus aenderungen.rs; Byte-Grenzen werden am API-Rand übersetzt.
    func testDesktopProposalRangesRuleIDsGroupsAndPartialAcceptance() throws {
        let cases: [(String, [(Int, Int, String, String, String, String)])] = [
            ("🦀 ﬃ\nä ﬀ\nÖ...", [(3, 1, "ﬃ", "ffi", "ligaturen-1", "ligaturen#1"),
                                   (7, 1, "ﬀ", "ff", "ligaturen-2", "ligaturen#2"),
                                   (10, 3, "...", "…", "auslassung-1", "auslassung#1")]),
            ("ﬃ ... ﬀ", [(0, 1, "ﬃ", "ffi", "ligaturen-1", "ligaturen#1"),
                           (2, 3, "...", "…", "auslassung-1", "auslassung#1"),
                           (6, 1, "ﬀ", "ff", "ligaturen-2", "ligaturen#2")]),
            ("ä\u{ad}ö", [(1, 1, "\u{ad}", "", "weiche_trennstriche-1", "weiche_trennstriche#1")]),
            ("e\u{ad}\u{301}", [(0, 3, "e\u{ad}\u{301}", "é", "weiche_trennstriche-1", "weiche_trennstriche#1")]),
            ("A - B", [(2, 1, "-", "–", "format-1", "format#1")]),
            ("•  Eins\n◦\t\tZwei", [(0, 2, "• ", "-", "listen-1", "listen#1"),
                                       (8, 3, "◦\t\t", "- ", "listen-2", "listen#2")]),
            ("A  <Anna>  B", [(2, 1, " ", "", "leerzeichen-1", "leerzeichen#1"),
                                 (10, 1, " ", "", "leerzeichen-2", "leerzeichen#2")]),
            ("9) Neun\n10) Zehn\n11) Elf", [(1, 1, ")", ".", "listen-1", "listen#1"),
                                            (10, 1, ")", ".", "listen-2", "listen#2"),
                                            (19, 1, ")", ".", "listen-3", "listen#3")]),
            ("\"Warte...\"  ", [(0, 1, "\"", "„", "anfuehrungszeichen-1", "anfuehrungszeichen#1"),
                                  (6, 3, "...", "…", "auslassung-1", "auslassung#1"),
                                  (9, 1, "\"", "“", "anfuehrungszeichen-2", "anfuehrungszeichen#1"),
                                  (10, 2, "  ", "", "leerzeichen-1", "leerzeichen#1")])
        ]
        for (basis, expected) in cases {
            let draft = TextAufbereiten.vorschlaege(basis, stufe: .format)
            XCTAssertEqual(draft.aenderungen.count, expected.count)
            for (change, want) in zip(draft.aenderungen, expected) {
                XCTAssertEqual(change.bereich, NSRange(location: want.0, length: want.1))
                XCTAssertEqual(change.alt, want.2)
                XCTAssertEqual(change.neu, want.3)
                XCTAssertEqual(change.id, want.4)
                XCTAssertEqual(change.regel, String(want.4.dropLast(2)))
                XCTAssertEqual(change.gruppe, want.5)
            }
            let groups = Array(Set(expected.map { $0.5 })).sorted()
            for mask in 0..<(1 << groups.count) {
                let selected = Set(groups.enumerated().filter { mask & (1 << $0.offset) != 0 }.map(\.element))
                let ids = Set(expected.filter { selected.contains($0.5) }.map { $0.4 })
                var want = basis as NSString
                for item in expected.reversed() where ids.contains(item.4) {
                    want = want.replacingCharacters(in: NSRange(location: item.0, length: item.1), with: item.3) as NSString
                }
                XCTAssertEqual(try draft.anwenden(angenommen: ids, auf: basis), want as String)
            }
        }
    }

    // Beispielmatrix aus beispieltexte_mit_vollannahme_teilannahme_und_idempotenz.
    // Gegenproben: verlorene lokale Hunks, globale Zitatgruppen und instabile Regelzähler.
    func testDesktopChangeCorpusAndGroupSubsets() throws {
        let cases = [
            ("ﬀ ﬁ ﬂ ﬃ ﬄ ﬅ ﬆ", "ff fi fl ffi ffl st st"),
            ("Sil\u{ad}be", "Silbe"),
            ("\"Hallo\" (\"Welt\")", "„Hallo“ („Welt“)"),
            ("Wort - Wort\n1 - 3\n- Punkt", "Wort – Wort\n1 - 3\n- Punkt"),
            ("Warte... .... .....", "Warte… .... ....."),
            ("  A  B\tC , D\u{a0}E  \n", "  A B\tC, D E\n"),
            ("10\u{a0}km A\u{a0}\u{a0}B", "10\u{a0}km A B"),
            ("🦀 ä ﬃ\n界 Cafe\u{301}\n🦀  Öl", "🦀 ä ffi\n界 Café\n🦀 Öl"),
            ("Ä  B\r\nEnde...\r\n", "Ä B\r\nEnde…\r\n"),
            ("A  \n", "A\n"),
            ("", ""),
            ("Cafe\u{301}", "Café"),
            ("A\u{301} e\u{ad}\u{301}", "Á é"),
            ("`\"A\u{301}  ﬃ\u{ad} - x...y\"`", "`\"A\u{301}  ﬃ\u{ad} - x...y\"`"),
            ("`x...y` Ende...", "`x...y` Ende…"),
            ("`offen\nx...y", "`offen\nx…y"),
            ("Datei .env Pegel .5 2 : 1 :-) a ,b a ;b a !b a ?b a ./ a .)", "Datei .env Pegel .5 2 : 1 :-) a ,b a ;b a !b a ?b a ./ a .)"),
            ("A . B : C , D ; E ! F ?\nA .\nB :", "A. B: C, D; E! F?\nA.\nB:"),
            ("A  \nB\nA\tB\t \nC\n| A  B\t | C   |  \n", "A  \nB\nA\tB\t \nC\n| A  B\t | C   |  \n"),
            ("> > - Punkt\n> Wort - Wort\n- - -\n* * *\n---", "> > - Punkt\n> Wort - Wort\n- - -\n* * *\n---"),
            ("[offen\n\nEnde...]", "[offen\n\nEnde…]"),
            ("A\u{a0},", "A,"),
            ("Er sagte:\"Hallo\"", "Er sagte:\"Hallo\""),
            ("\u{e000}\u{e010}\u{e001} [ﬃ  ...] \"Ja\"", "\u{e000}\u{e010}\u{e001} [ﬃ  ...] „Ja“"),
            ("Wort - Wort", "Wort – Wort"),
            ("Ein sauberer Absatz mit Umlauten und Schluss.", "Ein sauberer Absatz mit Umlauten und Schluss."),
            ("\"<A\u{301}  ﬃ\u{ad} - ...>\"  {{S. 12}} ...[\"A\u{301}\"  ﬃ\u{ad} - ...] ,", "„<A\u{301}  ﬃ\u{ad} - ...>“ {{S. 12}} …[\"A\u{301}\"  ﬃ\u{ad} - ...],"),
            ("Wort\" Wort\"", "Wort\" Wort\""),
            ("Cafe\u{301}\nﬃ\nSil\u{ad}be\n\"Ja\"\nA - B\nEnde...\nA  B\n• Punkt", "Café\nffi\nSilbe\n„Ja“\nA – B\nEnde…\nA B\n- Punkt"),
            ("A\u{301} ﬃ Sil\u{ad}be Wort - Wort Ende... A  B", "Á ffi Silbe Wort – Wort Ende… A B"),
            ("A\u{301}\nﬃ\nSil\u{ad}be\n\"Wort\"\nA - B\nEnde...\nA  B", "Á\nffi\nSilbe\n„Wort“\nA – B\nEnde…\nA B"),
            ("A  -\t B und Ende . . .", "A -\t B und Ende…"),
            ("\"Cafe\u{301}  ﬃ\u{ad} - Ende...\"", "„Café ffi – Ende…“"),
            ("\"Anna  123\" - Sil\u{ad}be... {{S. 12}} <Berta> [lacht]", "„Anna 123“ – Silbe… {{S. 12}} <Berta> [lacht]"),
            ("[🦀\n...]\n``界`界``\n~~~\nä\n~~~\nä .", "[🦀\n…]\n``界`界``\n~~~\nä\n~~~\nä."),
            ("ﬃ <Anna> ﬀ", "ffi <Anna> ff"),
            ("A  <Anna>  B", "A <Anna> B"),
            ("ﬃ ... ﬀ", "ffi … ff"),
            ("\"A\" <Anna> \"B\nC\"", "„A“ <Anna> „B\nC“"),
            ("e\u{ad}\u{301}", "é"),
            ("A\u{a0}- B", "A – B"),
            ("a  -  b", "a – b"),
            ("ﬃ  {{S. 12}}  ﬀ", "ffi {{S. 12}} ff"),
            ("ﬃ  00:12  Ende...  \nB", "ffi 00:12 Ende…  \nB"),
            ("• Eins\n· Zwei\n‣ Drei\n◦ Vier\n▪ Fünf\n● Sechs", "- Eins\n- Zwei\n- Drei\n- Vier\n- Fünf\n- Sechs"),
            ("   • 🦀\n  ◦\tÖl", "   - 🦀\n  - Öl"),
            ("1) Eins\n2) Zwei\n3) Drei", "1. Eins\n2. Zwei\n3. Drei"),
            ("9) Neun\r\n10) Zehn\r\n", "9. Neun\r\n10. Zehn\r\n"),
            ("1) Punkt", "1) Punkt"),
            ("2) Zwei\n4) Vier", "2) Zwei\n4) Vier"),
            ("1) Eins\n\n2) Zwei", "1) Eins\n\n2) Zwei"),
            ("• {{S. 12}} <Anna> [lacht]\n1) `x`\n2) Öl", "- {{S. 12}} <Anna> [lacht]\n1. `x`\n2. Öl"),
            ("```\n• x\n1) Eins\n2) Zwei\n```\n`• x`", "```\n• x\n1) Eins\n2) Zwei\n```\n`• x`"),
            ("| • |\n> • x\n<Anna> • x\n– Wohin?", "| • |\n> • x\n<Anna> • x\n– Wohin?"),
            ("•  Eins\n◦\t\tZwei", "- Eins\n- Zwei"),
            // Rust (format.rs listen, dann gedankenstriche) liefert "- – x": der neue Listenstrich zählt als Vorgänger.
            ("•\t.\n•\t- x", "- .\n- – x"),
            ("\"A...\" und \"B...\"", "„A…“ und „B…“")
        ]
        for (basis, target) in cases {
            let draft = TextAufbereiten.vorschlaege(basis, stufe: .format)
            XCTAssertEqual(draft, TextAufbereiten.vorschlaege(basis, stufe: .format))
            XCTAssertTrue(TextAufbereiten.identisch(try draft.anwenden(angenommen: Set(draft.aenderungen.map(\.id)), auf: basis), target))
            XCTAssertTrue(TextAufbereiten.vorschlaege(target, stufe: .format).aenderungen.isEmpty)
            XCTAssertEqual(try draft.anwenden(angenommen: [], auf: basis), basis)
            var counts: [String: Int] = [:], end = 0
            for change in draft.aenderungen {
                counts[change.regel, default: 0] += 1
                XCTAssertEqual(change.id, "\(change.regel)-\(counts[change.regel]!)")
                XCTAssertNotNil(change.gruppe)
                XCTAssertGreaterThanOrEqual(change.bereich.location, end)
                XCTAssertNotNil(Range(change.bereich, in: basis))
                XCTAssertTrue(TextAufbereiten.identisch((basis as NSString).substring(with: change.bereich), change.alt))
                end = NSMaxRange(change.bereich)
            }
            let groups = Array(Set(draft.aenderungen.compactMap(\.gruppe))).sorted()
            let all = (1 << groups.count) - 1
            // Wie Rust: alle Kombinationen bis sechs Gruppen, sonst deterministische Stichprobe.
            var state: UInt32 = 658
            let masks: [Int] = groups.count <= 6 ? Array(0...all) : (0..<20).map { _ in
                state = state &* 1_664_525 &+ 1_013_904_223
                return Int(state) % all + 1
            }
            let originalMarks = TextAufbereitenFormat.protected(basis).map { (basis as NSString).substring(with: $0) }
            for mask in masks {
                let selected = Set(groups.enumerated().filter { mask & (1 << $0.offset) != 0 }.map(\.element))
                let ids = Set(draft.aenderungen.filter { selected.contains($0.gruppe!) }.map(\.id))
                let partial = try draft.anwenden(angenommen: ids, auf: basis)
                XCTAssertEqual(partial.filter { $0 == "„" }.count, partial.filter { $0 == "“" }.count)
                for mark in originalMarks { XCTAssertTrue(partial.contains(mark)) }
            }
            for group in groups {
                let members = draft.aenderungen.filter { $0.gruppe == group }
                if members.count > 1 {
                    XCTAssertThrowsError(try draft.anwenden(angenommen: [members[0].id], auf: basis))
                }
            }
        }
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

    @MainActor
    func testDetachedDraftRejectsTextChangedBeforeAwaitReturns() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let editor = ReadingPageEditor(store: try ReadingPageStore(root: root))
        let page = try editor.create(text: "Warte...", title: "Probe")
        let basis = page.text
        let task = Task.detached { TextAufbereiten.vorschlaege(basis, stufe: .format) }
        // Deterministisch vor der Übernahme ändern; keine Zeitannahme über den Worker.
        try editor.updateText(page.id, text: "Neue Eingabe")
        let draft = await task.value
        let prepared = try draft.anwenden(angenommen: Set(draft.aenderungen.map(\.id)), auf: basis)
        XCTAssertEqual(prepared, "Warte…")
        XCTAssertThrowsError(try editor.applyPreparedText(page.id, basis: basis, text: prepared))
        XCTAssertEqual(try editor.page(page.id)?.text, "Neue Eingabe")
        try editor.updateText(page.id, text: basis)
        try editor.applyPreparedText(page.id, basis: basis, text: prepared)
        XCTAssertEqual(try ReadingPageStore(root: root).page(page.id)?.text, "Warte…")
    }

    func testLongTextAndRepeatedHeaders() throws {
        let page = (0..<12).map { "Absatz \($0): Die Verwaltung informiert über den Stand der Umstellung und bittet um Rückmeldung bis Monatsende." }.joined(separator: "\n")
        let raw = (1...4).map { "Landratsamt Musterkreis – Mitarbeiterinfo\n\(page)\nSeite \($0) von 4" }.joined(separator: "\n")
        let cleaned = try result(raw)
        XCTAssertFalse(cleaned.contains("Landratsamt"))
        XCTAssertFalse(cleaned.contains("Seite 1 von 4"))
        XCTAssertTrue(try result("Refrain\n" + page + "\nRefrain\n" + page).contains("Refrain"))
        // 1× gegen 4×; quadratische Verarbeitung wächst auf etwa 16×.
        _ = try result(String(repeating: "😀 ﬃ Ende... Wort - Wort A  B\n", count: 100), .format)
        func duration(_ count: Int) throws -> TimeInterval {
            let long = String(repeating: "😀 ﬃ Ende... Wort - Wort A  B\n", count: count)
            let start = ProcessInfo.processInfo.systemUptime
            let prepared = try result(long, .format)
            let elapsed = ProcessInfo.processInfo.systemUptime - start
            XCTAssertEqual(prepared, String(repeating: "😀 ffi Ende… Wort – Wort A B\n", count: count))
            return elapsed
        }
        let small = try (0..<3).map { _ in try duration(2_500) }.sorted()[1]
        let large = try (0..<3).map { _ in try duration(10_000) }.sorted()[1]
        XCTAssertLessThan(large / max(small, 0.001), 10)
    }

    func testMaskeBleibtUnabhaengigVonPrivatenZeichenKurz() throws {
        let head = String(repeating: "\u{e000}", count: 20_000)
        XCTAssertEqual(try result(head + String(repeating: " <a>  x", count: 2_000), .format),
                       head + String(repeating: " <a> x", count: 2_000))
        // Auch das erste Wunschzeichen der Maske darf im Text stehen.
        XCTAssertEqual(try result("\u{e020} <a>  x", .format), "\u{e020} <a> x")
    }

    /// ICU lehnt manche Muster ab, die JS/Rust annehmen (z. B. `[[(]`). Jede Konstante einmal anlegen
    /// und die Regelstufen über Text mit allen Fallgruppen laufen lassen; kein Muster darf ungültig sein.
    func testAlleMusterSindGueltig() throws {
        _ = TextAufbereitenPaste.block
        _ = TextAufbereitenPaste.fence
        _ = TextAufbereitenPaste.bareNumber
        _ = TextAufbereitenPaste.wordPage
        _ = TextAufbereitenPaste.slashPage
        _ = TextAufbereitenPaste.dashedNumber
        _ = TextAufbereitenPaste.sentence
        _ = TextAufbereitenPaste.lower
        _ = TextAufbereitenPaste.hyphen
        _ = TextAufbereitenPaste.suspended
        _ = TextAufbereitenPaste.initials
        _ = TextAufbereitenPaste.spaceBlock
        _ = TextAufbereitenPaste.timestamp
        _ = TextAufbereitenFormat.marks
        let probe = "Kopf\n\n[12:30] Sprecher: Text mit Tren-\nnung und  Leerraum.\n\n– 3 –\n\nSeite 4 von 9\n\n„Zitat“ ... -- Ende\n"
        _ = TextAufbereiten.vorschlaege(probe, stufe: .bereinigung)
        _ = TextAufbereiten.vorschlaege(probe, stufe: .format)
        // Wiederholte Kopfzeilen greifen erst ab 1.500 Zeichen; repeatKey mit Seiten- und Kapitelzeile direkt.
        _ = TextAufbereitenPaste.repeatKey("Gartenratgeber – Seite 12")
        _ = TextAufbereitenPaste.repeatKey("Kapitel 3 Beete")
        let absatz = String(repeating: "Die Beete werden im Frühjahr vorbereitet und gelockert. ", count: 8)
        let lang = (1...6).map { "Gartenratgeber – Seite \($0) von 6\n\n\(absatz)\n\n" }.joined()
        XCTAssertGreaterThan(lang.utf16.count, 1500)
        _ = TextAufbereiten.vorschlaege(lang, stufe: .bereinigung)
        XCTAssertEqual(TextAufbereiten.ungueltigeMuster, [])
    }


    // MARK: lva#914 – Ursachen der roten Tests und der Laufzeit

    /// „\r\n“ ist in Swift EIN Character: die Prüfung auf einen Zeilenumbruch muss auf Skalaren laufen, sonst gilt
    /// ein kurzer mehrzeiliger Text als Einzeiler und wird nicht bereinigt (pasteCleanup.ts: /[\r\n\f]/).
    func testKurzerMehrzeiligerTextWirdBereinigt() throws {
        XCTAssertEqual(try result("Wort  Wort\nEnde."), "Wort Wort\nEnde.")
        XCTAssertEqual(try result("Wort  Wort\r\nEnde."), "Wort Wort\nEnde.")
        XCTAssertEqual(try result("Wort  Wort\rEnde."), "Wort Wort\nEnde.")
        // Ein einzelner kurzer Einzeiler bleibt, wie am Desktop (isShortLine).
        XCTAssertEqual(try result("Wort  Wort"), "Wort  Wort")
    }

    /// `identisch` ist bytegleich: kanonisch gleiche Texte mit anderen Bytes sind NICHT identisch (U+0340 ≡ U+0300).
    func testIdentischIstBytegleichheit() {
        XCTAssertTrue("\u{340}" == "\u{300}")
        XCTAssertFalse(TextAufbereiten.identisch("\u{340}", "\u{300}"))
        XCTAssertFalse(TextAufbereiten.identisch("é", "e\u{301}"))
        XCTAssertTrue(TextAufbereiten.identisch("é", "é"))
        XCTAssertTrue(TextAufbereiten.identisch("", ""))
        XCTAssertFalse(TextAufbereiten.identisch("a", ""))
        let long = String(repeating: "Zeile ä 😀\n", count: 500)
        XCTAssertTrue(TextAufbereiten.identisch(long, String(repeating: "Zeile ä 😀\n", count: 500)))
        XCTAssertFalse(TextAufbereiten.identisch(long, long + "x"))
        // Eine aus NSString gebrückte Zeichenkette hat keinen eigenen UTF-8-Speicher.
        let bridged = NSString(string: long) as String
        XCTAssertTrue(TextAufbereiten.identisch(long, bridged))
    }

    /// Die Mustertabelle ist durch eine Sperre geschützt und darf unter Nebenläufigkeit und beim Überlaufen
    /// (Grenze 512 Muster) keine Ergebnisse verändern.
    func testMusterTabelleIstThreadSicherUndErhaeltErgebnisse() throws {
        let texts = ["Wort - Wort  \"Ja\" ...\nﬃ • Punkt", "1) Eins\n2) Zwei\n[00:12] Text.", "A . B : C\n> Zitat - Strich\n| x  y |"]
        let expected = texts.map { TextAufbereiten.vorschlaege($0, stufe: .format) }
        let expectedPaste = texts.map { TextAufbereiten.vorschlaege($0, stufe: .bereinigung) }
        final class Wrong: @unchecked Sendable { let lock = NSLock(); var count = 0 }
        let wrong = Wrong()
        DispatchQueue.concurrentPerform(iterations: 96) { i in
            let k = i % texts.count
            if i % 16 == 5 { for n in 0..<600 { _ = TextAufbereiten.regex("zz\(i)_\(n)") } }
            let ok = TextAufbereiten.vorschlaege(texts[k], stufe: .format) == expected[k]
                && TextAufbereiten.vorschlaege(texts[k], stufe: .bereinigung) == expectedPaste[k]
            if !ok { wrong.lock.lock(); wrong.count += 1; wrong.lock.unlock() }
        }
        XCTAssertEqual(wrong.count, 0)
        XCTAssertEqual(TextAufbereiten.ungueltigeMuster, [])
    }

    /// Suchen im Text dürfen Zeichen mit folgendem kombinierendem Zeichen nicht übersehen (Foundation sucht sonst
    /// nur an Zeichengrenzen): das Anführungszeichen hier wird gesetzt, der Backtick schützt.
    func testSucheFindetZeichenMitKombinierendemNachfolger() throws {
        XCTAssertEqual(try result("\"\u{340}Ja\" und gut", .format), "„\u{300}Ja“ und gut")
        XCTAssertEqual(try result("`\u{301}x...y` Ende...", .format), "`\u{301}x...y` Ende…")
    }

    /// Ligaturen und entfernte Trennstriche können neue Zusammensetzungen freilegen: ffi + U+0301 wird zu „ffí“.
    func testNFCNachLigaturauflösung() throws {
        XCTAssertEqual(try result("\u{fb03}\u{301}", .format), "ff\u{ed}")
        XCTAssertEqual(try result("i\u{ad}\u{301}", .format), "\u{ed}")
    }

    /// Der Abstand nach einem Aufzählungszeichen kann ein geschütztes Leerzeichen sein; die Zeile bleibt eine
    /// reine Listenänderung (nicht „leerzeichen+listen“).
    func testListenzeichenMitGeschuetztemLeerzeichen() throws {
        let draft = TextAufbereiten.vorschlaege("•\u{a0}Wort", stufe: .format)
        XCTAssertEqual(draft.aenderungen.map(\.regel), ["listen"])
        XCTAssertEqual(try draft.anwenden(angenommen: Set(draft.aenderungen.map(\.id)), auf: "•\u{a0}Wort"), "- Wort")
    }

    /// Eine Tabellenzeile erkennt Rust auf dem Zeichen „|“ (format.rs `tabellenzeile`), auch wenn ein kombinierendes
    /// Zeichen folgt; der erste Swift-Port prüfte zeichenweise und ließ so Zeilen mit „|\u{301}“ durch.
    func testTabellenstrichMitKombinierendemZeichenSchuetztDieZeile() throws {
        for text in ["x  y|\u{301}  z - w", "x  y|\u{200d}  z - w"] { XCTAssertEqual(try result(text, .format), text) }
    }

    /// Laufzeit: Regeln dürfen weder je Zeile Muster neu übersetzen noch je Zeichen ein Stück führen (lva#914: 52 s).
    /// Die Grenzen liegen weit über der gemessenen Zeit (Debug-Bau, etwa 0,4 s je 10 000 Zeilen), unter der alten.
    func testLaufzeitBleibtBeiLangenTextenKlein() throws {
        let long = String(repeating: "😀 ﬃ Ende... Wort - Wort A  B\n", count: 10_000)
        var start = ProcessInfo.processInfo.systemUptime
        let format = try result(long, .format)
        XCTAssertLessThan(ProcessInfo.processInfo.systemUptime - start, 3)
        XCTAssertEqual(format, String(repeating: "😀 ffi Ende… Wort – Wort A B\n", count: 10_000))
        let page = (0..<40).map { "Absatz \($0): Die Verwal-\ntung informiert  über den Stand und bittet\num Rückmeldung bis Monats-\nende.\n" }.joined(separator: "\n")
        let paste = (1...16).map { "Landratsamt Musterkreis – Mitarbeiterinfo\n\(page)\n\($0)\n" }.joined()
        start = ProcessInfo.processInfo.systemUptime
        let cleaned = try result(paste)
        XCTAssertLessThan(ProcessInfo.processInfo.systemUptime - start, 3)
        XCTAssertFalse(cleaned.contains("Landratsamt"))
        XCTAssertTrue(cleaned.contains("Verwaltung informiert über"))
    }
}
