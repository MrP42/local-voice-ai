import Foundation

// Gesprächsmodus „Sprechen“ (Paket A): Abschnitte für den Index mit Nachweis der Satzgrenzen. Ein Originalsatz wird nur
// zitiert, wenn er an beiden Enden nachweislich an einer Satzgrenze liegt. Die Kanten eines Abschnitts tragen deshalb
// `startsAtBoundary`/`endsAtBoundary`, bestimmt an den Nähten der Quelle (nie aus Vermutung) mit derselben Satzerkennung wie
// die Zerlegung des Indexes (`BookTalkSentences`, kennt Abkürzungen). Ohne Nachweis gilt `false`: der Randsatz wird nicht zitiert.
// Reine Funktionen ohne Darwin-Abhängigkeit (Linux-Tests); die iPhone-Schicht liefert nur die Eingaben.

/// EINE konservative Prüfung für jede Naht (außen an Abschnittsgrenzen, innen an Zeilenumbrüchen und für das Trennen innerhalb
/// einer Zeile). Fail-closed: im Zweifel nicht nachgewiesen, keine Heuristik erlaubt etwas. Eine Naht zwischen `davor` und
/// `danach` ist nur nachgewiesen, wenn
/// (a) `davor` (schließende Anführungszeichen und Klammern abgezogen) auf `!` oder `?` endet oder auf genau EINEN Punkt (keine
///     Auslassungspunkte), dessen letztes Wort T nur aus Buchstaben besteht (keine Ziffer: Ordnungszahlen und Daten wie „3.“
///     gelten nie), mindestens zwei Buchstaben hat (keine Initiale), keinen inneren Punkt trägt, nicht in der Abkürzungsliste
///     steht, nicht auf „str“ endet und entweder mindestens fünf Buchstaben hat oder kleingeschrieben in der geschlossenen Liste
///     von Kurzwörtern steht, die nie Abkürzungen sind; und
/// (b) `danach` (öffnende Anführungszeichen und Klammern abgezogen) mit einem GROSSBUCHSTABEN beginnt (nicht Ziffer, nicht klein).
/// Am nachgewiesenen Abschnittsende (`danach == nil`) entfällt (b). Restgrenze: eine nicht gelistete Abkürzung mit fünf oder mehr
/// Buchstaben vor einem Großbuchstaben gilt als Satzende.
enum AssistantSeam {
    /// Wort, das in der Probe auf einen Text ohne Nachfolger folgt (Großbuchstabe: ein Satzanfang).
    private static let probe = "Ende"

    private static let closers: Set<Character> = ["“", "”", "‘", "’", "\"", "'", "»", "«", ")", "›", "]"]
    private static let openers: Set<Character> = ["„", "“", "‚", "‘", "»", "«", "\"", "'", "(", "‹", "["]

    /// Abkürzungen (klein): die der Satzerkennung plus weitere. Nur für Wörter ab fünf Buchstaben von Belang.
    static let abbreviations: Set<String> = [
        "z", "b", "d", "h", "u", "a", "o", "bzw", "usw", "ca", "dr", "nr", "s", "vgl", "etc", "evtl", "ggf", "inkl",
        "prof", "str", "sog", "bspw", "e", "g", "i", "vs", "mr", "mrs", "ms", "st", "no", "p", "pp", "ff", "f",
        "hr", "fr", "dipl", "ing", "jr", "sen", "abs", "abb", "tel", "fax", "zzgl", "ggfs", "evt", "allg", "bes", "bzgl",
        "dgl", "engl", "frz", "ital", "max", "min", "mind", "rd", "zt", "zw", "urspr", "vorm", "nachm", "incl", "gem",
        "insb", "insbes", "sogen", "stellv", "gesch", "bearb", "jahrh", "jahrg", "abtlg", "hrsg", "anm", "geb", "gest",
        "verh", "mio", "mrd", "tsd", "univ", "fakult", "zeitschr", "vorzgl", "ergebn", "einw", "gebr", "lfg", "ztschr",
    ]

    /// Kurzwörter, die nie Abkürzungen sind (klein geschrieben): nur sie beweisen mit weniger als fünf Buchstaben ein Satzende.
    static let shortWords: Set<String> = [
        "an", "auf", "aus", "ein", "mit", "vor", "zu", "ab", "bei", "da", "es", "ist", "sind", "war", "hat", "gut", "neu", "nun",
        "hier", "dort", "dann", "noch", "auch", "mehr", "wir", "ihr", "sie", "er", "ich", "du", "uns", "euch", "ihn", "ihm", "sich",
        "mich", "dich", "mir", "dir", "nach", "fest", "frei", "los", "weg", "hin", "her", "um", "nie", "oft", "viel", "klar",
        "statt", "teil", "nein", "ja",
    ]

    /// Teil (a): Schließt `text` nachweislich einen Satz ab?
    static func endsSentence(_ text: String) -> Bool {
        var tail = Substring(text.trimmingCharacters(in: .whitespacesAndNewlines))
        while let last = tail.last, closers.contains(last) { tail = tail.dropLast() }
        guard let end = tail.last else { return false }
        if end == "!" || end == "?" { return true }
        guard end == "." else { return false }
        let body = tail.dropLast()
        guard let previous = body.last, !previous.isWhitespace, previous != ".", previous != "…" else { return false }
        guard var word = body.split(whereSeparator: { $0.isWhitespace }).last else { return false }
        while let first = word.first, openers.contains(first) { word = word.dropFirst() }
        guard word.count >= 2, word.allSatisfy(\.isLetter) else { return false }
        let lower = word.lowercased()
        guard !abbreviations.contains(lower), !lower.hasSuffix("str") else { return false }
        return word.count >= 5 || (String(word) == lower && shortWords.contains(lower))
    }

    /// Teil (b): Beginnt `text` (öffnende Anführungszeichen und Klammern abgezogen) mit einem Großbuchstaben?
    static func startsSentence(_ text: String) -> Bool {
        var head = Substring(text.trimmingCharacters(in: .whitespacesAndNewlines))
        while let first = head.first, openers.contains(first) { head = head.dropFirst() }
        guard let first = head.first else { return false }
        return first.isLetter && first.isUppercase
    }

    /// Liegt zwischen `before` und `after` nachweislich eine Satzgrenze? (a) und (b) müssen gelten, und die Satzerkennung muss
    /// über die Naht an dieser Stelle schneiden. `after == nil`: nichts folgt (Ende der Quelle oder Lücke); dann zählt nur (a),
    /// die Satzerkennung wird mit einem hypothetischen Satzanfang geprüft.
    static func proven(_ before: String, _ after: String?) -> Bool {
        let a = before.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !a.isEmpty, endsSentence(a) else { return false }
        var b = probe
        if let after {
            let trimmed = after.trimmingCharacters(in: .whitespacesAndNewlines)
            if !trimmed.isEmpty {
                guard startsSentence(trimmed) else { return false }
                b = trimmed
            }
        }
        let cut = a.utf16.count
        return BookTalkSentences.split(a + " " + b).contains { $0.end == cut }
    }
}

/// Ein Segment eines Transkripts in Zeitfolge. `usable == false`: nicht für die KI freigegeben oder leer; es unterbricht die Folge.
struct AssistantSegment: Equatable {
    let start: Double
    /// Zeile für den Index (mit Sprecher).
    let line: String
    /// Text des Segments ohne Sprecher, getrimmt (für die Naht).
    let body: String
    let usable: Bool
}

enum AssistantSections {
    /// Eine ganze Einheit (Sprachnotiz, Antwort der App): sie beginnt und endet an der Einheit selbst. Ein Schlusssatz ohne
    /// Satzzeichen wird trotzdem nicht zitiert (nur Sätze mit Satzzeichen am Ende sind zitierbar).
    static func whole(_ text: String, locator: String?) -> AssistantDocument.Section {
        AssistantDocument.Section(locator: locator, text: text, startsAtBoundary: true, endsAtBoundary: true)
    }

    /// Ein erzeugter Text (Protokoll) ohne Nachweis der Randsätze: Zeilen ohne Satzzeichen würden sonst zu einem langen
    /// „Satz“ verschmelzen, der nie ein Originalsatz war.
    static func generated(_ text: String, locator: String?) -> AssistantDocument.Section {
        AssistantDocument.Section(locator: locator, text: text)
    }

    /// Transkript: Abschnitte aus aufeinanderfolgenden benutzbaren Segmenten, höchstens `seconds` Sekunden und `chars` Zeichen
    /// lang. Der Anfang gilt nur am Beginn der Aufnahme oder nach einem Satzende des Vorgängers, das Ende nur nach einem
    /// Satzende (an der Naht zum Nachfolger oder, vor einer Lücke und am Ende, am Segment selbst). Nach einer Lücke (nicht
    /// freigegebenes oder leeres Segment) ist der Anfang nie nachgewiesen.
    static func meeting(_ segments: [AssistantSegment], seconds: Double, chars: Int,
                        locator: (Double) -> String) -> [AssistantDocument.Section] {
        var out: [AssistantDocument.Section] = []
        var text = ""
        var first: AssistantSegment?
        var last: AssistantSegment?
        var startProven = false

        func flush(next: AssistantSegment?) {
            defer { text = ""; first = nil; last = nil }
            guard let first, let last, !text.isEmpty else { return }
            out.append(AssistantDocument.Section(locator: locator(first.start), text: text, startsAtBoundary: startProven,
                                                 endsAtBoundary: AssistantSeam.proven(last.body, next?.body)))
        }

        for (i, segment) in segments.enumerated() {
            guard segment.usable else {
                flush(next: nil)               // Lücke: nie darüber hinweg verbinden, nichts über die Naht behaupten
                continue
            }
            let tooLong = first.map { segment.start - $0.start >= seconds } ?? false
            if tooLong || text.count + segment.line.count > chars { flush(next: segment) }
            if first == nil {
                first = segment
                startProven = i == 0 || (segments[i - 1].usable && AssistantSeam.proven(segments[i - 1].body, segment.body))
            }
            last = segment
            text += (text.isEmpty ? "" : "\n") + segment.line
        }
        flush(next: nil)
        return out
    }

    /// Buch: ein Abschnitt je Stück des Buchindexes. `frontier` (UTF-16-Offset im Original): Lesegrenze; nichts dahinter steht
    /// im Ergebnis, ein Stück an der Grenze endet an der Satzgrenze davor (dann ist das Ende nachgewiesen). Der Anfang gilt am
    /// Buch- und Kapitelanfang oder nach einem Satzende des Vorgängers; das Ende am Buch- und Kapitelende oder vor einem
    /// Nachfolger, an dessen Naht die Satzerkennung schneidet. Ein Nachfolger hinter der Lesegrenze zählt nicht (kein Text
    /// dahinter); dann entscheidet das Stück allein.
    static func book(_ chunks: [BookTalkChunk], frontier: Int?) -> [AssistantDocument.Section] {
        var out: [AssistantDocument.Section] = []
        for (i, chunk) in chunks.enumerated() {
            let part: BookTalkChunk?
            if let frontier { part = chunk.truncated(atFrontier: frontier) } else { part = chunk }
            guard let part, !part.text.isEmpty else { continue }

            let starts = i == 0 || chunks[i - 1].chapterIndex != chunk.chapterIndex || AssistantSeam.proven(chunks[i - 1].text, chunk.text)
            let ends: Bool
            if part.text != chunk.text {
                ends = true                     // an der Lesegrenze an einer Satzgrenze gekürzt
            } else if i + 1 < chunks.count, frontier.map({ chunks[i + 1].start < $0 }) ?? true {
                let next = chunks[i + 1]
                ends = next.chapterIndex != chunk.chapterIndex || AssistantSeam.proven(chunk.text, next.text)
            } else {
                ends = AssistantSeam.proven(chunk.text, nil)
            }

            let title = part.chapterTitle.trimmingCharacters(in: .whitespacesAndNewlines)
            let locator = part.page.map { "S. \($0)" } ?? (title.isEmpty ? nil : title)
            out.append(AssistantDocument.Section(locator: locator, text: part.text, startsAtBoundary: starts, endsAtBoundary: ends))
        }
        return out
    }
}
