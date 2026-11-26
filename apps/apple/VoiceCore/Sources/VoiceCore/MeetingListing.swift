import Foundation

/// Anzeige-Regeln der Besprechungsliste (lva#837): Titel, Eckdaten, Status, Gruppen und Suche.
/// Rein und ohne UI, damit unter Linux testbar.
public enum MeetingListing {
    /// Titel, die nichts unterscheiden (Standard der Live-Aufnahme, generische Dateinamen).
    static let genericTitles: Set<String> = ["besprechung", "aufnahme", "neue aufnahme", "recording", "audio", "meeting", "new recording"]

    /// Eingegebenen Titel säubern; leer oder generisch → `nil` (dann gilt der automatische).
    public static func cleanTitle(_ raw: String?) -> String? {
        guard let raw else { return nil }
        let single = raw.components(separatedBy: .newlines).joined(separator: " ")
            .split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
        guard !single.isEmpty, !genericTitles.contains(single.lowercased()) else { return nil }
        return String(single.prefix(120))
    }

    /// Angezeigter Titel: eigener bzw. Sitzungstitel, sonst aus dem Protokoll, sonst sprechender Dateiname, sonst „Besprechung“.
    public static func title(_ doc: MeetingDocument) -> String {
        if let own = cleanTitle(doc.title) { return own }
        if let minutes = doc.minutes, let auto = autoTitle(minutes) { return auto }
        let stem = (doc.originalName as NSString).deletingPathExtension
        if let file = cleanTitle(stem.replacingOccurrences(of: "_", with: " ")) { return file }
        return "Besprechung"
    }

    /// Erster Satz des Kontexts bzw. der Zusammenfassung, an einer Wortgrenze auf höchstens 60 Zeichen gekürzt.
    public static func autoTitle(_ minutes: MeetingMinutes, limit: Int = 60) -> String? {
        for source in [minutes.scope, minutes.summary] {
            let trimmed = source.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.isEmpty else { continue }
            var sentence = trimmed.split(whereSeparator: { ".!?\n".contains($0) }).first.map(String.init) ?? trimmed
            sentence = sentence.trimmingCharacters(in: .whitespaces)
            guard sentence.count >= 3 else { continue }
            if sentence.count <= limit { return sentence }
            var out = ""
            for word in sentence.split(separator: " ") {
                if out.count + word.count + 1 > limit - 1 { break }
                out += (out.isEmpty ? "" : " ") + word
            }
            return (out.isEmpty ? String(sentence.prefix(limit - 1)) : out) + "…"
        }
        return nil
    }

    /// Zeitpunkt der Besprechung: Aufnahmebeginn, sonst Übernahme.
    public static func date(_ doc: MeetingDocument) -> Date { doc.recordedAt ?? doc.createdAt }

    /// „47 Min“, „1 Std 12 Min“, „45 s“.
    public static func duration(_ seconds: Double) -> String {
        guard seconds.isFinite, seconds > 0 else { return "–" }
        let total = Int(seconds.rounded())
        if total < 60 { return "\(total) s" }
        let minutes = (total + 30) / 60
        if minutes < 60 { return "\(minutes) Min" }
        let rest = minutes % 60
        return rest == 0 ? "\(minutes / 60) Std" : "\(minutes / 60) Std \(rest) Min"
    }

    public static func participantCount(_ doc: MeetingDocument) -> Int? {
        guard let names = doc.participantNames?.filter({ !$0.trimmingCharacters(in: .whitespaces).isEmpty }), !names.isEmpty else { return nil }
        return names.count
    }

    /// Erkannte Sprachen der verfeinerten Live-Zeilen, häufigste zuerst, als Kürzel („DE“, „EN“).
    public static func languages(_ doc: MeetingDocument) -> [String] {
        var counts: [String: Int] = [:]
        for segment in doc.liveSegments ?? [] where segment.refined {
            guard let raw = segment.language?.lowercased(), raw != "auto",
                  let code = raw.split(whereSeparator: { $0 == "-" || $0 == "_" }).first, !code.isEmpty else { continue }
            counts[String(code), default: 0] += 1
        }
        return counts.sorted { $0.value != $1.value ? $0.value > $1.value : $0.key < $1.key }.map { $0.key.uppercased() }
    }

    public enum Status: Equatable, Sendable {
        case consentMissing
        case running
        case waiting
        case failed
        case ready(tasks: Int, openQuestions: Int)
    }

    public static func status(_ doc: MeetingDocument, running: Bool, failed: Bool) -> Status {
        if doc.requiresMinutesConsent { return .consentMissing }
        if let minutes = doc.minutes { return .ready(tasks: minutes.tasks.count, openQuestions: minutes.open_questions.count) }
        if running { return .running }
        return failed ? .failed : .waiting
    }

    /// Fortschritt aus der Statuszeile („… · 73 %“ oder „Abschnitt 2 von 5“), sonst `nil`.
    public static func progress(fromStatus text: String) -> Double? {
        if let part = text.range(of: #"Abschnitt (\d+) von (\d+)"#, options: .regularExpression) {
            let numbers = text[part].split(whereSeparator: { !$0.isNumber }).compactMap { Double($0) }
            if numbers.count == 2, numbers[1] > 0, numbers[0] <= numbers[1] { return max(0, numbers[0] - 1) / numbers[1] }
        }
        guard let percent = text.range(of: #"(\d{1,3})\s?%"#, options: .regularExpression) else { return nil }
        let digits = text[percent].filter(\.isNumber)
        guard let value = Double(digits), value <= 100 else { return nil }
        return value / 100
    }

    /// Abschnitt der Liste: Heute, Gestern, Diese Woche, sonst Monat und Jahr.
    public static func section(for date: Date, now: Date, calendar: Calendar) -> String {
        Sections(now: now, calendar: calendar).title(for: date)
    }

    /// Abschnittsbildung für einen Durchlauf: Formatierer und Wochengrenzen nur einmal anlegen.
    struct Sections {
        let now: Date, calendar: Calendar
        let yesterday: Date?, week: DateInterval?
        let month: DateFormatter, monthYear: DateFormatter
        init(now: Date, calendar: Calendar) {
            self.now = now; self.calendar = calendar
            yesterday = calendar.date(byAdding: .day, value: -1, to: now)
            week = calendar.dateInterval(of: .weekOfYear, for: now)
            func formatter(_ template: String) -> DateFormatter {
                let f = DateFormatter()
                f.calendar = calendar; f.timeZone = calendar.timeZone; f.locale = Locale(identifier: "de_DE")
                f.setLocalizedDateFormatFromTemplate(template)
                return f
            }
            month = formatter("MMMM"); monthYear = formatter("MMMM yyyy")
        }
        func title(for date: Date) -> String {
            if calendar.isDate(date, inSameDayAs: now) { return "Heute" }
            if let yesterday, calendar.isDate(date, inSameDayAs: yesterday) { return "Gestern" }
            if let week, week.contains(date) { return "Diese Woche" }
            return (calendar.isDate(date, equalTo: now, toGranularity: .year) ? month : monthYear).string(from: date)
        }
    }

    /// Gruppen in Reihenfolge der Liste (neueste zuerst), innerhalb neueste zuerst.
    public static func grouped(_ docs: [MeetingDocument], now: Date, calendar: Calendar) -> [(title: String, items: [MeetingDocument])] {
        var order: [String] = []
        var groups: [String: [MeetingDocument]] = [:]
        let sections = Sections(now: now, calendar: calendar)
        for doc in docs.sorted(by: { date($0) > date($1) }) {
            let key = sections.title(for: date(doc))
            if groups[key] == nil { order.append(key) }
            groups[key, default: []].append(doc)
        }
        return order.map { ($0, groups[$0] ?? []) }
    }

    public enum Filter: String, CaseIterable, Sendable {
        case all = "Alle", ready = "Mit Protokoll", open = "In Arbeit", imported = "Importiert"
    }

    public static func matches(_ doc: MeetingDocument, filter: Filter) -> Bool {
        switch filter {
        case .all: return true
        case .ready: return doc.minutes != nil
        case .open: return doc.minutes == nil
        case .imported: return !isRecording(doc)
        }
    }

    /// In der App aufgenommen (Aufnahmebeginn oder Live-Zeilen). Importe tragen höchstens einen leeren Live-Stand
    /// mit dem Einwilligungsnachweis der Übernahme.
    public static func isRecording(_ doc: MeetingDocument) -> Bool {
        doc.recordedAt != nil || !(doc.liveSegments ?? []).isEmpty
    }

    /// Suche ohne Groß-/Kleinschreibung und Akzente über Titel, Dateiname, Teilnehmende, Protokoll und Transkript.
    /// Alle Wörter der Anfrage müssen vorkommen.
    public static func matches(_ doc: MeetingDocument, query: String) -> Bool {
        matches(searchText: searchText(doc), query: query)
    }

    /// Gefalteter Suchtext eines Dokuments; einmal je Stand berechnen, nicht je Tastendruck.
    public static func searchText(_ doc: MeetingDocument) -> String {
        var haystack = [title(doc), doc.originalName] + (doc.participantNames ?? [])
        if let minutes = doc.minutes {
            haystack += [minutes.summary, minutes.scope] + minutes.tasks.map(\.text) + minutes.decisions.map(\.text)
        }
        haystack += doc.segments.map(\.text)
        haystack += (doc.liveSegments ?? []).map(\.text)
        return fold(haystack.joined(separator: " "))
    }

    public static func matches(searchText text: String, query: String) -> Bool {
        let words = query.split(whereSeparator: { $0.isWhitespace }).map { fold(String($0)) }
        return words.allSatisfy { text.contains($0) }
    }

    static func fold(_ s: String) -> String {
        s.folding(options: [.caseInsensitive, .diacriticInsensitive], locale: Locale(identifier: "de_DE"))
    }
}
