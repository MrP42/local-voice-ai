import Foundation

// Auswertung am Gesprächsende (lva#559, Paket G5-Kern): der Prompt für EIN Modell (Anweisung und Gespräch) und die
// Gesprächsfassung. Reiner Text; die Anweisung verlangt genau ein JSON-Objekt, nur für das, was das Profil einschaltet, dazu immer
// `offene_fragen`. Gegenstück am PC: `assistenten/auswertung.ts`; der Text ist in `auswertung-pruefstaende.json` bytegleich
// festgehalten.

public struct AssistantOutcomePrompt: Equatable, Sendable {
    public let text: String
    public let truncated: Bool
}

extension AssistantOutcome {
    private static let weekdaysDe = ["Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag", "Sonntag"]

    private static func typeDescription(_ f: AssistantProfile.Field) -> String {
        switch f.kind {
        case .text: return "Text"
        case .number: return "Zahl, mit Punkt als Dezimaltrenner, ohne Tausenderzeichen"
        case .date: return "Datum als JJJJ-MM-TT, relative Angaben anhand des heutigen Datums umgerechnet"
        case .choice:
            return "Auswahl, genau eine von: " + f.options.map { "\"" + AssistantOutcomeText.singleLine($0) + "\"" }.joined(separator: ", ")
        }
    }

    /// Die Anweisung an das Modell (alles bis einschließlich `Gespräch:`), nur für das, was das Profil einschaltet.
    private static func instruction(_ profile: AssistantProfile, _ today: AssistantOutcomeDate) -> [String] {
        let e = profile.evaluation
        let hasFields = !e.fields.isEmpty
        var lines = [
            "Werte das folgende beendete Gespräch aus. Antworte mit genau einem JSON-Objekt und sonst nichts: kein Text davor oder danach, keine Code-Zäune, keine Erklärung.",
            "",
        ]
        if today.isValid {
            lines.append("Heutiges Datum: \(today.formatted) (\(weekdaysDe[today.weekday - 1])).")
            lines.append("")
        }
        lines.append("Regeln:")
        lines.append("- Erfinde nichts. Nimm nur auf, was im Gespräch gesagt wurde.")
        if hasFields || e.tasks {
            lines.append("- Jeder Beleg ist ein wörtliches Zitat aus dem Gespräch, \"beitrag\" ist die Nummer des Beitrags, aus dem es stammt.")
        }
        if hasFields {
            lines.append("- Je Feld gilt ein Status: \"klar\" (eindeutig genannt), \"unklar\" (genannt, aber unsicher, widersprüchlich oder nur zum Teil) oder \"nicht_genannt\" (kommt im Gespräch nicht vor). Bei \"nicht_genannt\" sind \"wert\", \"beleg\" und \"beitrag\" null.")
            lines.append("- Widersprechen sich Angaben oder wurde etwas später korrigiert, gilt die letzte eindeutige Angabe, sonst ist der Status \"unklar\".")
        }
        if e.tasks {
            lines.append("- Fristen stehen als wörtlicher Ausdruck aus dem Gespräch in \"frist_text\" (zum Beispiel \"nächsten Dienstag\"), nicht als errechnetes Datum.")
        }
        lines.append("- Verwende nur Namen, die im Gespräch vorkommen; erfinde keinen Ersatznamen. Keine Floskeln.")
        lines.append("- Schreibe in der Sprache des Gesprächs.")
        lines.append("")
        lines.append("Das JSON-Objekt hat genau diese Schlüssel:")
        if hasFields {
            lines.append("- \"felder\": Liste mit genau einem Eintrag je Feld, in dieser Reihenfolge. Jeder Eintrag hat die Schlüssel \"name\", \"status\", \"wert\" (Text oder null), \"beleg\" (Zitat oder null) und \"beitrag\" (Zahl oder null). Die Felder:")
            for f in e.fields {
                let description = AssistantOutcomeText.singleLine(f.descriptionText)
                lines.append("  - \(f.name) (\(typeDescription(f)))" + (description.isEmpty ? "" : ": " + description))
            }
        }
        if e.summary {
            lines.append("- \"zusammenfassung\": Zusammenfassung des Gesprächs in zwei bis fünf Sätzen (Text).")
        }
        if e.tasks {
            lines.append("- \"aufgaben\": Liste der Aufgaben und Zusagen aus dem Gespräch, leer, wenn es keine gibt. Jeder Eintrag hat die Schlüssel \"text\" (kurz), \"frist_text\" (wörtlicher Ausdruck oder leerer Text), \"beleg\" (Zitat) und \"beitrag\" (Zahl).")
        }
        lines.append("- \"offene_fragen\": Liste von Fragen oder Punkten, die im Gespräch offen blieben (Texte), leer, wenn es keine gibt.")
        lines.append("")
        lines.append("Gespräch:")
        return lines
    }

    /// Die Gesprächsfassung: ein Beitrag je Zeile, `[n] Nutzer: …` oder `[n] Assistent: …`, Leerraum zu einem Leerzeichen
    /// zusammengefasst.
    public static func transcript(_ turns: [AssistantOutcomeTurn]) -> [String] {
        turns.enumerated().map { i, turn in
            let text = AssistantOutcomeText.singleLine(turn.text)
            return "[\(i + 1)] \(turn.role == .user ? "Nutzer" : "Assistent"):" + (text.isEmpty ? "" : " " + text)
        }
    }

    private static func omission(_ n: Int) -> String { "[… \(n) \(n == 1 ? "Beitrag" : "Beiträge") ausgelassen …]" }

    private static func count(_ s: String) -> Int { s.unicodeScalars.count }

    /// Der Prompt für EIN Modell: Anweisung und Gespräch. `budgetChars` zählt Codepunkte des ganzen Textes. Passt er nicht, bleiben
    /// die ersten zwei Beiträge und so viele der neuesten wie möglich; dazwischen steht `[… n Beiträge ausgelassen …]` (die
    /// Nummern bleiben die des ungekürzten Gesprächs), `truncated` ist dann wahr. Ein einzelner Beitrag wird nie geschnitten; das
    /// Mindeste (zwei erste Beiträge und die Auslassung) bleibt auch über dem Budget stehen. Mit höchstens zwei Beiträgen ist
    /// nichts auszulassen.
    public static func prompt(profile: AssistantProfile, turns: [AssistantOutcomeTurn], today: AssistantOutcomeDate, budgetChars: Int) -> AssistantOutcomePrompt {
        let head = instruction(profile, today).joined(separator: "\n")
        let lines = transcript(turns)
        let full = ([head] + lines).joined(separator: "\n")
        let budget = max(0, budgetChars)
        if count(full) <= budget || lines.count <= 2 { return AssistantOutcomePrompt(text: full, truncated: false) }

        let lengths = lines.map(count)
        let headLength = count(head)
        let n = lines.count
        // mit k neuesten Beiträgen: Kopf + 2 erste + Auslassung + k neueste, je durch einen Zeilenumbruch getrennt
        func length(_ k: Int, _ newestSum: Int) -> Int {
            headLength + 1 + lengths[0] + 1 + lengths[1] + 1 + count(omission(n - 2 - k)) + (k > 0 ? newestSum + k : 0)
        }
        var k = 0
        var sum = 0
        while k < n - 3 {
            let candidate = sum + lengths[n - 1 - k]
            if length(k + 1, candidate) > budget { break }
            sum = candidate
            k += 1
        }
        let kept = [lines[0], lines[1], omission(n - 2 - k)] + Array(lines[(n - k)...])
        return AssistantOutcomePrompt(text: ([head] + kept).joined(separator: "\n"), truncated: true)
    }
}
