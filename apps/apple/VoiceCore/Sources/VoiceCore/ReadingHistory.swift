import Foundation

/// Verlauf der Buchseite aus dem Leseprotokoll (`reading:<buch>`, #795): Sitzungen je Kalendertag,
/// neueste zuerst, mit verständlichen Beschriftungen („Heute“, „21:10–21:40“, „30 Min.“).
public enum ReadingHistory {
    /// Sitzungen eines Kalendertags (Beginn des Tags in `calendar`), neueste zuerst.
    public struct Day: Equatable, Sendable {
        public let start: Date
        public let entries: [ReadingLogEntry]
    }

    /// Kürzere Sitzungen sind Antippen, kein Lesen (das Protokoll speichert ab 30 s; ältere Stände können kürzer sein).
    public static let minimumMs: Double = 30_000

    /// Gruppiert nach Kalendertag des Sitzungsbeginns; höchstens `maxEntries` Sitzungen insgesamt.
    public static func days(_ log: [ReadingLogEntry], calendar: Calendar, maxEntries: Int = 40) -> [Day] {
        let kept = log
            .filter { $0.startMs.isFinite && $0.endMs.isFinite && $0.endMs - $0.startMs >= minimumMs }
            .sorted { $0.startMs != $1.startMs ? $0.startMs > $1.startMs : $0.device < $1.device }
            .prefix(maxEntries)
        var out: [Day] = []
        for entry in kept {
            let day = calendar.startOfDay(for: date(entry.startMs))
            if let last = out.last, last.start == day {
                out[out.count - 1] = Day(start: day, entries: last.entries + [entry])
            } else {
                out.append(Day(start: day, entries: [entry]))
            }
        }
        return out
    }

    /// „Heute“, „Gestern“, sonst „Mittwoch, 25. November“ (mit Jahr, wenn es nicht das laufende ist).
    public static func dayLabel(_ day: Date, now: Date, calendar: Calendar) -> String {
        if calendar.isDate(day, inSameDayAs: now) { return "Heute" }
        if let yesterday = calendar.date(byAdding: .day, value: -1, to: now),
           calendar.isDate(day, inSameDayAs: yesterday) { return "Gestern" }
        let f = DateFormatter()
        f.calendar = calendar; f.timeZone = calendar.timeZone; f.locale = Locale(identifier: "de_DE")
        let sameYear = calendar.component(.year, from: day) == calendar.component(.year, from: now)
        f.setLocalizedDateFormatFromTemplate(sameYear ? "EEEEdMMMM" : "EEEEdMMMMyyyy")
        return f.string(from: day)
    }

    /// „21:10–21:40“ (über Mitternacht ebenso, nur Uhrzeiten).
    public static func timeSpan(_ entry: ReadingLogEntry, calendar: Calendar) -> String {
        let f = DateFormatter()
        f.calendar = calendar; f.timeZone = calendar.timeZone; f.locale = Locale(identifier: "de_DE")
        f.dateFormat = "HH:mm"
        return "\(f.string(from: date(entry.startMs)))–\(f.string(from: date(entry.endMs)))"
    }

    /// Dauer auf volle Minuten, mindestens „1 Min.“; ab einer Stunde „1 Std. 5 Min.“.
    public static func duration(_ entry: ReadingLogEntry) -> String {
        let minutes = max(1, Int(((entry.endMs - entry.startMs) / 60_000).rounded()))
        return minutes >= 60 ? "\(minutes / 60) Std. \(minutes % 60) Min." : "\(minutes) Min."
    }

    /// „Gelesen“, „Gehört“, „Live übersetzt“, „Kurzbuch“.
    public static func mediumLabel(_ medium: ReadingMedium) -> String {
        switch medium {
        case .text: return "Gelesen"
        case .audio: return "Gehört"
        case .live: return "Live übersetzt"
        case .kurzbuch: return "Kurzbuch"
        }
    }

    private static func date(_ ms: Double) -> Date { Date(timeIntervalSince1970: ms / 1000) }
}
