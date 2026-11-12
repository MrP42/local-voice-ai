import Foundation

// Gesprächsmodus „Sprechen“, Stil `natural` (ADR-0028 E4.2c): reine Bausteine ohne Zustand der Sitzung.
// - `SpeakerGate`: trennt im Strom des Sprechers den Text (direkte Antwort oder ein Zwischensatz) von der Steuerzeile `SUCHE: …`.
// - `AssistantBridgeRotation`: Überbrückungssätze reihum, nie zweimal derselbe hintereinander.
// - `AssistantProtocolOrder`: Prüfer der Zeilenfolge einer Anfrage im Protokoll (Stil `verbatim`: wie bisher genau eine Rundenzeile;
//   Stil `natural`: nur Sprecher, Sprecher dann Daten, nur Daten).
// Ein Sprecher-Satz und ein Satz des Denkers gehen wie jede andere Einheit über `RunEmitter.emit` (Satzspur, Generationsprüfung);
// hier steht nur, was sich ohne Sitzung prüfen lässt.

public enum AssistantNatural {
    /// Beginn der einen Zeile, mit der der Sprecher eine Suche verlangt.
    public static let searchMarker = "SUCHE:"

    /// Überbrückungssätze, reihum. Nur gesprochen (`.announcement`): nicht im Antworttext, nicht im Verlauf. Kein Ansage- oder
    /// Überbrückungstext enthält ein Stoppwort (`stoppWords`): Das Mikrofon könnte die eigene Ausgabe sonst als Stoppbefehl hören.
    public static let bridges = [
        "Ich schaue kurz nach.",
        "Ich suche das heraus.",
        "Einen Augenblick, ich sehe nach.",
        "Das schlage ich kurz nach.",
    ]

    /// Ansage für eine langsame Antwort: `fillerFirstMs` nach dem Hinweis „Äußerung gilt“ (`AssistantSession.noteCommitted`), wenn bis
    /// dahin noch keine Einheit ausgegeben wurde (weder Überbrückung noch Satz). Nur gesprochen (`.announcement`).
    public static let fillerFirstSentence = "Einen Augenblick."
    /// Zweite Ansage: `fillerSecondMs` nach dem Hinweis, wenn bis dahin noch keine Antworteinheit (frei, zusammengefasst, feste
    /// Antwort, Bestandsangabe) ausgegeben wurde.
    public static let fillerSecondSentence = "Das dauert noch etwas."

    /// Wörter, die in keinem Ansage- und Überbrückungstext vorkommen dürfen (normalisiert verglichen): Die Sprachsteuerung deutet sie als
    /// Befehl zum Anhalten (`stopp`, `stop`, `halt`, `warte`, `moment`).
    public static let stoppWords: Set<String> = ["stopp", "stop", "halt", "warte", "moment"]

    /// Alle festen Ansage- und Überbrückungstexte dieses Stils (für die Prüfung auf Stoppwörter).
    static var announcementTexts: [String] { bridges + [fillerFirstSentence, fillerSecondSentence] }

    /// Feste Antwort (`.neutral`), wenn die Suche keine Auszüge fand. Der Denker läuft dann nicht.
    public static let nothingFoundSentence = "Dazu habe ich in deinen Inhalten nichts Passendes gefunden."
    /// Feste Antwort (`.neutral`), wenn der Sprecher nichts Verwertbares lieferte.
    public static let notUnderstoodSentence = "Das habe ich nicht verstanden. Sag es bitte noch einmal."

    /// Längste Suchwörter-Zeile (länger: gekürzt).
    static let maxSearchChars = 160

    /// Sätze, die nur das System spricht (feste Antworten, Überbrückung): Ein Satz des Sprechers mit diesem Wortlaut wird verworfen.
    private static let systemSentences: Set<String> = Set(
        ([nothingFoundSentence, notUnderstoodSentence, fillerFirstSentence, fillerSecondSentence] + bridges).map(AssistantGrounding.normalize))

    /// Ein Satz des Sprechers ist ein Satz des Systems (feste Fehl-Vorlage, Ansage, Überbrückung, feste Antwort dieses Stils). Der
    /// Sprecher hat nicht gesucht und darf nicht so tun, als hätte er es.
    static func isSystemOnly(_ sentence: String) -> Bool {
        AssistantGrounding.isSystemOnly(sentence) || systemSentences.contains(AssistantGrounding.normalize(sentence))
    }

    /// Die Zeile des Protokolls zwischen Sprecher und System (`SUCHE: …`) wird nie gesprochen, auch nicht, wenn sie nicht am
    /// Stromanfang steht (dann ist sie keine Suche, sondern ein Satz des Modells, den niemand hören soll).
    static func isProtocolLine(_ sentence: String) -> Bool {
        let edge = CharacterSet.whitespacesAndNewlines.union(CharacterSet(charactersIn: "\"'„“”‚‘’»«›‹*_`"))
        let trimmed = sentence.trimmingCharacters(in: edge).lowercased()
        guard trimmed.hasPrefix("suche") else { return false }
        let rest = trimmed.dropFirst(5).drop(while: { $0 == " " || $0 == "\t" || $0 == "\u{00A0}" })
        return rest.first == ":" || rest.first == "\u{FF1A}"
    }

    /// Die Titel der Auszüge für die Denk-Anzeige: in der Reihenfolge der Auszüge, ohne Dubletten und ohne leere Titel.
    static func titles(of passages: [AssistantPassage]) -> [String] {
        var seen = Set<String>()
        return passages.compactMap { p in
            let title = p.title.trimmingCharacters(in: .whitespacesAndNewlines)
            return !title.isEmpty && seen.insert(title).inserted ? title : nil
        }
    }

    /// Suchtext des Abrufs: Suchwörter des Sprechers + Leerzeichen + Frage (ohne Suchwörter: die Frage).
    static func searchText(words: String, question: String) -> String {
        words.isEmpty ? question : words + " " + question
    }

    /// Die Suchwörter aus dem Rest der ersten Zeile: ohne Anführungszeichen, Markdown-Zeichen und Satzzeichen an den Rändern,
    /// Leerraum zusammengezogen, auf `maxSearchChars` gekürzt. Leer, wenn nichts Brauchbares dasteht (dann gilt die Frage).
    static func searchWords(_ raw: String) -> String {
        let line = raw.split(omittingEmptySubsequences: false, whereSeparator: \.isNewline).first.map(String.init) ?? ""
        let flat = BookTalkPrompt.neutralize(line).split(whereSeparator: \.isWhitespace).joined(separator: " ")
        let edge = CharacterSet.whitespaces.union(CharacterSet(charactersIn: "\"'„“”‚‘’»«›‹*_`.,;:!?…<>"))
        let trimmed = flat.trimmingCharacters(in: edge)
        return BookTalkPrompt.clip(trimmed, to: maxSearchChars)
    }

    // MARK: Sprecher-Protokoll

    /// Trennt den Strom des Sprechers in Text und Steuerzeile. Der Sprecher antwortet
    /// - nur mit Text: direkte Antwort;
    /// - mit einem kurzen Zwischensatz und DANACH `SUCHE: <Suchwörter>`: der Satz geht sofort hinaus (nach der Prüfung
    ///   `isFillerInterim`), danach läuft die Suche;
    /// - nur mit `SUCHE: <Suchwörter>`: Suche ohne Satz.
    /// Die Steuerzeile beginnt (1) am Stromanfang oder nach einem Zeilenumbruch: führende Leerzeichen, Zeilenumbrüche, Anführungszeichen
    /// und Markdown-Zeichen werden übersprungen, Groß- und Kleinschreibung egal, auch „Suche :“; oder (2) im Zeileninneren NACH einem
    /// Satzende (. ! ? …) und Leerraum, dort NUR in Großbuchstaben („SUCHE:“), damit ein normales „suche:“ im Text keine Suche auslöst.
    /// Der Satz davor ist der Zwischensatz. „SUCHE:“ mitten im Satz ohne vorangehendes Satzende bleibt Text (der Satz mit diesem Anfang
    /// wird später als Protokollzeile verworfen, `AssistantNatural.isProtocolLine`). Ob ein Anfang eine Steuerzeile ist, steht erst nach
    /// einigen Zeichen fest; bis dahin werden diese wenigen Zeichen zurückgehalten (nur an solchen Stellen, und nur solange sie noch ein
    /// Anfang von „SUCHE:“ sein könnten). Ist es keine Steuerzeile, geht der Text unverändert hinaus (auch das Übersprungene). Die
    /// Steuerzeile selbst geht NIE als Text hinaus: Ihr Rest bis zum Zeilenende (oder einer Obergrenze) sind die Suchwörter.
    struct SpeakerGate: Sendable {
        struct Search: Equatable {
            /// Der Rest der Steuerzeile bisher (roh, noch nicht bereinigt: `AssistantNatural.searchWords`).
            let words: String
            /// Die Zeile ist abgeschlossen (Zeilenende oder Obergrenze): Der Strom kann abgebrochen werden.
            let complete: Bool
        }

        struct Step: Equatable {
            /// Text, der in den Satztrenner geht (in dieser Reihenfolge, VOR der Steuerzeile).
            var speech = ""
            /// Gesetzt, sobald eine Steuerzeile erkannt ist; danach bei jedem Aufruf mit dem Stand der Suchwörter.
            var search: Search?
        }

        private enum Probe { case maybe, no, yes(rest: String) }

        private var probing = true       // an einer Stelle, an der die Steuerzeile beginnen könnte (Zeilenanfang oder nach einem Satzende)
        private var strict = false        // im Zeileninneren: nur Großbuchstaben
        private var afterEnd = false      // das letzte Zeichen des Textes war ein Satzende
        private var afterEndSpace = false // und danach kam Leerraum (kein Zeilenumbruch)
        private var searching = false
        private var held = ""
        private var words = ""

        /// Zeichen, die vor dem Wort „SUCHE“ stehen dürfen (zusätzlich zu Leerraum und Zeilenumbrüchen).
        private static let leading: Set<Character> = ["\"", "'", "„", "“", "”", "‚", "‘", "’", "»", "«", "›", "‹", "*", "_", "`"]
        private static let terminators: Set<Character> = [".", "!", "?", "…"]

        mutating func feed(_ piece: String) -> Step {
            var step = Step()
            for ch in piece {
                if searching {
                    words.append(ch)
                    continue
                }
                if !probing, afterEnd, afterEndSpace, !ch.isWhitespace {
                    probing = true
                    strict = true
                    held = ""
                }
                guard probing else {
                    step.speech.append(ch)
                    if ch.isNewline {
                        probing = true
                        strict = false
                        afterEnd = false
                        afterEndSpace = false
                    } else if Self.terminators.contains(ch) {
                        afterEnd = true
                        afterEndSpace = false
                    } else if ch.isWhitespace {
                        if afterEnd { afterEndSpace = true }
                    } else {
                        afterEnd = false
                        afterEndSpace = false
                    }
                    continue
                }
                held.append(ch)
                switch Self.probe(held, strict: strict) {
                case .maybe:
                    break
                case .no:
                    step.speech += held
                    held = ""
                    probing = ch.isNewline
                    strict = false
                    afterEnd = Self.terminators.contains(ch)
                    afterEndSpace = false
                case .yes(let rest):
                    searching = true
                    words = rest
                    held = ""
                    probing = false
                }
            }
            if searching { step.search = current() }
            return step
        }

        /// Stromende. Was noch zurückgehalten wird, war keine Steuerzeile (auch ein bloßes „Suche“ am Ende) und geht als Text hinaus.
        mutating func finish() -> Step {
            var step = Step()
            if searching {
                step.search = Search(words: words, complete: true)
                return step
            }
            step.speech = held
            held = ""
            return step
        }

        private func current() -> Search {
            Search(words: words, complete: words.contains(where: \.isNewline) || words.count > 2 * AssistantNatural.maxSearchChars)
        }

        private static func probe(_ text: String, strict: Bool) -> Probe {
            let chars = Array(text)
            var i = 0
            while i < chars.count, leading.contains(chars[i]) || chars[i].isWhitespace { i += 1 }
            let marker: [String] = (strict ? "SUCHE" : "suche").map { String($0) }
            for (offset, expected) in marker.enumerated() {
                guard i + offset < chars.count else { return .maybe }
                let actual = strict ? String(chars[i + offset]) : chars[i + offset].lowercased()
                if actual != expected { return .no }
            }
            var k = i + 5
            while k < chars.count, chars[k] == " " || chars[k] == "\t" || chars[k] == "\u{00A0}" { k += 1 }
            guard k < chars.count else { return .maybe }
            guard chars[k] == ":" || chars[k] == "\u{FF1A}" else { return .no }
            return .yes(rest: String(chars[(k + 1)...]))
        }
    }

    // MARK: Zwischensatz

    /// Wörter, die in einem Zwischensatz nichts Inhaltliches sagen (Vergleichsschlüssel wie `AssistantWords.key`): Füllwörter, Pronomen,
    /// Partikel. Ein Satz „Ich schaue kurz nach.“ besteht nur aus ihnen und dem Suchverb mit „nach“.
    private static let functionWords: Set<String> = [
        "ich", "wir", "dir", "dich", "mir", "mich", "es", "das", "dies", "dazu", "da", "dort", "hier", "damit", "dafur", "jetzt", "gleich",
        "gern", "gerne", "schnell", "eben", "einmal", "mal", "kurz", "noch", "doch", "dann", "also", "und", "bitte", "ja", "ok", "okay",
        "klar", "gut", "na", "hm", "nur",
    ]
    private static let searchVerbs: Set<String> = ["schaue", "schau", "sehe", "seh", "gucke", "guck", "suche", "such"]
    /// Einzelne Wörter, die ein Satz allein zur Floskel machen („ü“ als „u“ gefaltet, dazu die Schreibung „ue“).
    private static let fillerWords: Set<String> = ["uberlege", "uberlegen", "ueberlege", "ueberlegen", "augenblick", "sekunde"]

    /// Der Zwischensatz des Sprechers (der Satz vor `SUCHE:`) ist eine Floskel und wird verworfen (`droppedTemplate`, die Suche läuft
    /// weiter, es gibt keinen Ersatzsatz). Direkte Antworten des Sprechers werden nicht geprüft. Verworfen wird ein Satz, wenn
    /// - er eines der Stoppwörter (`stoppWords`) als Wort enthält, oder
    /// - er weniger als 4 Wörter hat, oder
    /// - er auf ein Floskelmuster passt (Wörter wie `AssistantWords.keys`, ohne Rücksicht auf Groß-/Kleinschreibung): „denke“ mit
    ///   höchstens zwei Wörtern dazwischen vor „nach“; „überlege“, „überlegen“; „lass mich“; „augenblick“; „sekunde“; „gleich wieder“;
    ///   „bin gleich“; oder ein Suchverb (schaue, schau, sehe, seh, gucke, guck, suche, such) vor „nach“, WENN der Satz außer den beiden
    ///   kein Inhaltswort hat (alle anderen Wörter stehen in `functionWords`, darunter „mal“ und „kurz“). „Ich schaue in deinem
    ///   Humboldt-Buch nach.“ bleibt erlaubt, „Ich schaue kurz nach.“ nicht, „Ich denke, das steht in deinem Buch.“ bleibt erlaubt.
    static func isFillerInterim(_ sentence: String) -> Bool {
        let w = AssistantWords.keys(sentence)
        if w.count < 4 { return true }
        if w.contains(where: stoppWords.contains) { return true }
        if !fillerWords.isDisjoint(with: w) { return true }
        for (a, b) in [("lass", "mich"), ("gleich", "wieder"), ("bin", "gleich")] {
            for i in 0..<(w.count - 1) where w[i] == a && w[i + 1] == b { return true }
        }
        for i in w.indices where w[i] == "denke" {
            for j in (i + 1)..<min(w.count, i + 4) where w[j] == "nach" { return true }
        }
        for i in w.indices where searchVerbs.contains(w[i]) {
            guard let j = w[(i + 1)...].firstIndex(of: "nach") else { continue }
            if w.enumerated().allSatisfy({ $0.offset == i || $0.offset == j || functionWords.contains($0.element) }) { return true }
        }
        return false
    }
}

// MARK: Überbrückung

/// Die Überbrückungssätze reihum, je Sitzung: nie zweimal derselbe hintereinander (die Liste hat mehrere Einträge). Ein kurzer Lock,
/// kein Warten, kein I/O.
final class AssistantBridgeRotation: @unchecked Sendable {
    private let lock = NSLock()
    private var next = 0

    func take() -> String {
        lock.withLock {
            let text = AssistantNatural.bridges[next % AssistantNatural.bridges.count]
            next = (next + 1) % AssistantNatural.bridges.count
            return text
        }
    }
}

// MARK: Prüfer der Zeilenfolge

/// Prüft die Zeilen EINER Anfrage (in Dateireihenfolge, ohne `state`) auf die zulässige Folge. Reine Funktion; sie liest nichts und
/// ändert nichts. Leer = in Ordnung.
///
/// Stil `verbatim` (Standard, `start` ohne Stil): höchstens eine Rundenzeile, keine Gründe und Einordnungen des Stils `natural`.
///
/// Stil `natural` hat bis zu zwei PHASEN, die an ihren Rundenzeilen zuordenbar sind; beide hängen am selben Freigabestand und an
/// derselben Generation (jede Zeile `round`, `areas`, `step` trägt die Generation der Zeile `start`):
/// - nur Sprecher: `round` (`.general`, `naturalTalk`), Bereichszeile (die Bereiche der Quellen der Verlaufsbeiträge im Prompt, leer
///   möglich), dann nur Einheiten des Sprechers (`conversational`, feste Sätze, verworfene);
/// - Sprecher dann Daten: wie oben (die Einheit des Sprechers ist hier der Zwischensatz vor der Steuerzeile, höchstens einer), danach
///   `round` (`.data`, `naturalSearch`) mit den zulässigen Abrufbereichen in der Zeile selbst, der Überbrückungssatz (nur hier, nur mit
///   `bridge`, danach der Abruf), die Bereichszeile der verwendeten Auszüge, dann `summarized` (oder ein fester Satz);
/// - nur Daten (direkter Inhaltsbezug): nur die zweite Phase. Eine Bestandsfrage ist eine Datenphase mit Grund `inventory`.
/// Ansagen für eine langsame Antwort (`fillerFirstSentence`, `fillerSecondSentence`, nur mit `fillers`) können in jeder Phase vor der
/// ersten Antworteinheit dieser Phase stehen. Eine Anfrage, die vor der Rundenwahl endet, hat keine Rundenzeile und keine Einheit.
enum AssistantProtocolOrder {
    enum Violation: String, Equatable {
        case startMissing
        case lineAfterEnd
        case generationMismatch
        /// Stil `verbatim`: mehr als eine Rundenzeile.
        case roundCount
        /// Eine Rundenzeile mit Rundenart oder Grund, die an dieser Stelle der Folge nicht vorkommen darf.
        case roundNotAllowed
        /// Stil `verbatim`: Grund oder Einordnung des Stils `natural`.
        case naturalInVerbatim
        case stepBeforeRound
        case areasBeforeRound
        /// Vor dem Sprecher fehlt die Bereichszeile des Verlaufs.
        case speakerAreasMissing
        /// Die Rundenzeile der Daten-Phase trägt die zulässigen Abrufbereiche nicht.
        case retrievalAreasMissing
        /// Vor dem Denker (verwendete Auszüge) fehlt die Bereichszeile.
        case dataAreasMissing
        /// Die Einordnung gehört nicht in diese Phase.
        case verdictNotAllowed
        /// Eine Ansage (Überbrückung, Ansage für eine langsame Antwort) steht nach der ersten Antworteinheit.
        case announcementAfterAnswer
        /// Mehr als drei Ansagen in einer Anfrage (Überbrückung, zwei Ansagen für eine langsame Antwort; Bestandsvorspann).
        case tooManyAnnouncements
    }

    private enum Phase { case none, speaker, data }

    /// Höchstens so viele Ansagen je Anfrage im Stil `natural`: Überbrückung (Anfang der Daten-Phase) und die zwei Ansagen für eine
    /// langsame Antwort (je höchstens einmal, jeweils vor der ersten Antworteinheit). Sie teilen die Einordnung `announcement`; der
    /// Prüfer kann sie nicht unterscheiden, er prüft Zahl und Lage (nie nach einer Antworteinheit).
    static let maxAnnouncements = 3

    static func check(_ lines: [AssistantJournalLine]) -> [Violation] {
        guard let start = lines.first, start.t == .start else { return [.startMissing] }
        let natural = start.style == .natural
        let generation = start.gen
        var found: [Violation] = []
        func add(_ v: Violation) { if !found.contains(v) { found.append(v) } }

        var rounds: [(kind: AssistantRoundKind, reason: AssistantContext.Reason?)] = []
        var phase = Phase.none
        var speakerAreas = false
        var dataAreas = false
        var announcements = 0
        var answered = false
        var ended = false

        for l in lines.dropFirst() {
            if ended { add(.lineAfterEnd); continue }
            switch l.t {
            case .end:
                ended = true
            case .round:
                if let g = l.gen, let generation, g != generation { add(.generationMismatch) }
                guard let kind = l.round else { add(.roundNotAllowed); continue }
                if !natural {
                    if !rounds.isEmpty { add(.roundCount) }
                    if l.reason == .naturalTalk || l.reason == .naturalSearch { add(.naturalInVerbatim) }
                    rounds.append((kind, l.reason))
                    continue
                }
                switch (rounds.count, kind, l.reason) {
                case (0, .general, .naturalTalk?):
                    phase = .speaker
                case (0, .data, .naturalSearch?):
                    phase = .data
                    if l.areas == nil { add(.retrievalAreasMissing) }
                case (0, .data, .inventory?):
                    phase = .data
                case (1, .data, .naturalSearch?) where rounds[0].kind == .general && rounds[0].reason == .naturalTalk:
                    if !speakerAreas { add(.speakerAreasMissing) }
                    phase = .data
                    // Ein Zwischensatz des Sprechers (`conversational`) steht vor der Datenphase; die Überbrückung am Anfang der
                    // Datenphase darf ihm folgen. „Nach einer Antworteinheit“ zählt ab hier nur noch die Datenphase.
                    answered = false
                    if l.areas == nil { add(.retrievalAreasMissing) }
                default:
                    add(.roundNotAllowed)
                }
                rounds.append((kind, l.reason))
            case .areas:
                if let g = l.gen, let generation, g != generation { add(.generationMismatch) }
                if rounds.isEmpty { add(.areasBeforeRound) }
                switch phase {
                case .speaker: speakerAreas = true
                case .data: dataAreas = true
                case .none: break
                }
            case .step:
                if let g = l.gen, let generation, g != generation { add(.generationMismatch) }
                guard !rounds.isEmpty else { add(.stepBeforeRound); continue }
                guard let verdict = l.verdict else { continue }
                if !natural {
                    if verdict == .conversational || verdict == .summarized { add(.naturalInVerbatim) }
                    continue
                }
                let dropped = isDropped(verdict)
                if verdict == .announcement {
                    announcements += 1
                    if announcements > maxAnnouncements { add(.tooManyAnnouncements) }
                    if answered { add(.announcementAfterAnswer) }
                    continue
                }
                switch phase {
                case .none:
                    break
                case .speaker:
                    if !speakerAreas { add(.speakerAreasMissing) }
                    if !(verdict == .conversational || verdict == .template || dropped) { add(.verdictNotAllowed) }
                case .data:
                    switch verdict {
                    case .template:
                        break
                    case .summarized, .inventory:
                        if !dataAreas { add(.dataAreasMissing) }
                    case .conversational, .grounded, .unsourced:
                        add(.verdictNotAllowed)
                    default:
                        if dropped, !dataAreas { add(.dataAreasMissing) }
                    }
                }
                if !dropped { answered = true }
            default:
                break
            }
        }
        return found
    }

    private static func isDropped(_ verdict: AssistantTraceVerdict) -> Bool {
        switch verdict {
        case .droppedOwnData, .droppedNoMode, .droppedNotSnapped, .droppedDuplicate, .droppedTemplate, .droppedRevoked, .droppedFragment:
            true
        default:
            false
        }
    }
}
