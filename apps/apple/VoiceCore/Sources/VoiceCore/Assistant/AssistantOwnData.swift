import Foundation

// Gesprächsmodus „Sprechen“ (Paket A, E4.2a): Eigendatenbezug. Ein Typ für Frage und Satz, reine Funktion, kein Zustand.
// Zweite Schicht: Sie entscheidet, ob eine Frage eine Datenrunde ist (`AssistantTurnPolicy`) und ob ein Satz einer
// Allgemeinrunde verworfen wird. Sie ist KEINE Sicherheitsgrenze: Die Grenze ist, dass in Datenrunden nie Modelltext
// ausgegeben wird (nur Originalsätze und Bestandsangaben des Systems). Deshalb darf diese Schicht streng und grob sein; ein
// Fehlalarm kostet eine Antwort („Dazu finde ich nichts“), ein Fehlen nie die Treue.
//
// Gewertet wird nach NFKC und Faltung (klein, ohne Akzente, ß = ss, Formatzeichen wie Zero-Width-Space entfernt):
// - ein Wort mit Buchstaben außerhalb der lateinischen Schrift (kyrillisches „а“ im „Kаlender“) ist Bezug;
// - Stämme als Teilwort (Komposita): notiz, termin, kalender, mail, … ; „buch“ als Wort mit Formen oder als Wortende;
// - Anrede und Besitz: dein*, euer/eure*, großgeschriebenes Ihr*/Ihnen; „du“ mit Tätigkeitsverb oder hattest/warst/wolltest;
//   in Fragen zusätzlich mein*, unser*, „ich“/„wir“ mit Tätigkeitsverb;
// - Titel aus dem Bestand als Wortfolge;
// - „laut“, „gemäß“, „Seite“ mit Zahl.

public enum AssistantOwnData {
    public enum Mode: Sendable {
        case question
        case sentence
    }

    /// Stämme (gefaltet), die als Teilwort genügen: „Kalendertermine“, „Notizbuch“ (über „buch“), „Posteingangsregeln“.
    static let stems: [String] = [
        "notiz", "termin", "kalender", "mail", "korrespondenz", "nachricht", "posteingang", "postfach", "besprechung", "meeting",
        "protokoll", "transkript", "mitschrift", "aufnahme", "aufzeichnung", "ablage", "dokument", "datei", "unterlagen",
        "bibliothek", "kapitel", "horbuch", "diktat",
    ]

    private static let buchForms: [String] = ["buch", "buches", "buchs", "bucher", "buchern"]

    /// Tätigkeitsverben (gefaltet), mit denen dem Nutzer eigene Äußerungen oder Handlungen zugeschrieben werden.
    static let acts: Set<String> = [
        "gesagt", "notiert", "erwahnt", "aufgenommen", "gelesen", "geschrieben", "besprochen", "gespeichert", "diktiert",
        "hochgeladen", "importiert", "angelegt", "gefragt", "gesprochen", "erzahlt", "sagtest", "erwahntest", "notiertest",
        "schriebst", "fragtest", "markiert", "gemerkt", "vorgemerkt", "geloscht", "gesendet", "geschickt", "verschickt", "geplant",
        "vereinbart", "eingetragen", "gesucht", "gehort", "gesehen", "bestellt", "gebucht", "abgesagt", "angerufen", "gekauft",
        "geteilt", "kopiert", "gelernt", "erstellt", "bearbeitet", "geandert", "geoffnet", "gestartet", "gewollt", "gewunscht",
        "beschlossen", "entschieden", "versprochen", "vergessen", "gemacht", "getan", "gewusst", "sagst", "notierst", "liest",
        "schreibst", "besprichst", "speicherst", "diktierst", "fragst", "erwahnst", "willst", "sprichst", "erzahlst", "planst",
        "ladest", "lasst", "suchst", "kaufst", "buchst", "sagte", "notierte", "schrieb", "fragte", "erwahnte", "las", "wollte",
    ]

    private static let pastForms: Set<String> = ["hattest", "warst", "wolltest"]

    /// Ob `text` auf eigene Daten des Nutzers zeigt. `titles`: Titel aus dem Bestand der freigegebenen Bereiche.
    public static func refers(_ text: String, mode: Mode, titles: [String] = []) -> Bool {
        let prepared = Prepared(text)
        if prepared.hasNonLatinLetter { return true }
        let words = prepared.folded
        if words.isEmpty { return false }

        for word in words {
            if stems.contains(where: { word.contains($0) }) { return true }
            if buchForms.contains(where: { word == $0 || word.hasSuffix($0) }) { return true }
            if word.hasPrefix("dein") || word == "euer" || word.hasPrefix("eure") { return true }
            if word == "laut" || word == "gemass" { return true }
        }
        // Großgeschriebenes Ihr*/Ihnen (Anrede): das Wort beginnt im Original mit Großbuchstaben.
        for (original, folded) in zip(prepared.original, words) {
            guard let first = original.first, first.isUppercase else { continue }
            if folded.hasPrefix("ihr") || folded.hasPrefix("ihnen") { return true }
        }
        let set = Set(words)
        let acting = !set.isDisjoint(with: acts)
        if set.contains("du"), acting || !set.isDisjoint(with: pastForms) { return true }
        if mode == .question {
            if words.contains(where: { ["mein", "meine", "meinen", "meinem", "meiner", "meines"].contains($0) || $0.hasPrefix("unser") }) {
                return true
            }
            if (set.contains("ich") || set.contains("wir")), acting { return true }
        }
        if words.contains(where: { $0 == "seite" || $0 == "seiten" }), !BookTalkCitations.numbers(words).isEmpty { return true }
        return containsTitle(words, titles: titles)
    }

    // MARK: Titel

    private static func containsTitle(_ words: [String], titles: [String]) -> Bool {
        for title in titles {
            var sequence = Prepared(title).folded
            guard !sequence.isEmpty else { continue }
            var forms = [sequence]
            if let first = sequence.first, ["der", "die", "das", "ein", "eine", "the", "a", "an"].contains(first) {
                sequence.removeFirst()
                forms.append(sequence)
            }
            for form in forms where !form.isEmpty && form.joined(separator: " ").count >= 4
                && !form.allSatisfy({ BookTalkTokens.stop.contains($0) }) {
                if contains(words, sequence: form) { return true }
            }
        }
        return false
    }

    private static func contains(_ words: [String], sequence: [String]) -> Bool {
        guard !sequence.isEmpty, words.count >= sequence.count else { return false }
        for i in 0...(words.count - sequence.count) where Array(words[i..<(i + sequence.count)]) == sequence { return true }
        return false
    }

    // MARK: Aufbereitung

    /// Lateinische Buchstaben: ASCII, Latin-1, Latin Extended-A und das große ẞ. Alles andere (Kyrillisch, Griechisch, IPA,
    /// Latin Extended-B mit seinen Doppelgängern …) zählt als fremde Schrift.
    private static func isLatinLetter(_ v: UInt32) -> Bool {
        (0x41...0x5A).contains(v) || (0x61...0x7A).contains(v) || (0xC0...0xFF).contains(v) || (0x100...0x17F).contains(v) || v == 0x1E9E
    }

    private static func isLetter(_ s: Unicode.Scalar) -> Bool {
        switch s.properties.generalCategory {
        case .uppercaseLetter, .lowercaseLetter, .titlecaseLetter, .modifierLetter, .otherLetter: return true
        default: return false
        }
    }

    struct Prepared {
        /// Gefaltete Wörter (Buchstaben und Ziffern).
        let folded: [String]
        /// Dieselben Wörter in Originalschreibung (NFKC, ohne Formatzeichen).
        let original: [String]
        let hasNonLatinLetter: Bool

        init(_ text: String) {
            // NFKC: Vollbreite und mathematische Buchstaben werden zu gewöhnlichen; Formatzeichen (Zero-Width, weiches
            // Trennzeichen) fallen weg, damit sie kein Wort zerschneiden.
            let nfkc = text.precomposedStringWithCompatibilityMapping
            var scalars = String.UnicodeScalarView()
            var foreign = false
            for s in nfkc.unicodeScalars {
                if s.properties.generalCategory == .format { continue }
                if isLetter(s), !isLatinLetter(s.value) { foreign = true }
                scalars.append(s)
            }
            hasNonLatinLetter = foreign
            let clean = String(scalars)
            var original: [String] = []
            var current = ""
            for ch in clean {
                if ch.isLetter || ch.isNumber || ch.unicodeScalars.allSatisfy({ $0.properties.generalCategory == .nonspacingMark }) && !current.isEmpty {
                    current.append(ch)
                } else if !current.isEmpty {
                    original.append(current)
                    current = ""
                }
            }
            if !current.isEmpty { original.append(current) }
            var foldedWords: [String] = []
            var kept: [String] = []
            for word in original {
                let folded = Self.fold(word)
                guard !folded.isEmpty else { continue }
                foldedWords.append(folded)
                kept.append(word)
            }
            self.folded = foldedWords
            self.original = kept
        }

        static func fold(_ word: String) -> String {
            var out = ""
            for s in word.lowercased().decomposedStringWithCanonicalMapping.unicodeScalars {
                if s.properties.generalCategory == .nonspacingMark { continue }
                if s.value == 0xDF { out += "ss"; continue }
                out.unicodeScalars.append(s)
            }
            return out
        }
    }
}
