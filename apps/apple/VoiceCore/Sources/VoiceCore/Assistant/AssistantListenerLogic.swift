import Foundation

// Gesprächsmodus „Sprechen“ (Paket A): reine Logik hinter dem durchgehenden Zuhören. Nur Foundation, kein Audio, keine
// Uhr: Zeiten kommen als Sekunden der Erkennungs-Zeitachse (Summe der an den Analyzer gegebenen Frames) bzw. als
// Millisekunden je Puffer. Dadurch ohne Gerät testbar (`AssistantListenerLogicTests` im Kern, `swift test`).

enum AssistantWords {
    /// Ein Wort zum Vergleichen: klein, Umlaute gefaltet, nur Buchstaben und Ziffern („Wetter?“ == „wetter“).
    static func key(_ token: String) -> String {
        var out = ""
        for ch in token.lowercased() {
            switch ch {
            case "ä": out += "a"
            case "ö": out += "o"
            case "ü": out += "u"
            case "ß": out += "ss"
            default: if ch.isLetter || ch.isNumber { out.append(ch) }
            }
        }
        return out
    }

    /// Wörter eines Textes als Vergleichsschlüssel, leere (Satzzeichen) entfallen.
    static func keys(_ text: String) -> [String] {
        text.split(whereSeparator: { $0.isWhitespace }).map { key(String($0)) }.filter { !$0.isEmpty }
    }

    /// Gleicher Inhalt? (Vorarbeit mit identischem Text übernehmen.)
    static func same(_ a: String, _ b: String) -> Bool { keys(a) == keys(b) }
}

/// Text der laufenden Äußerung aus den Ergebnissen des Erkenners.
/// - Fest (`stable`) und vorläufig (`volatile`) getrennt, wie im Buch-Gespräch.
/// - `consume(at:)` beendet die Äußerung. Ergebnisse, die davor enden, gehören noch zur alten Äußerung (der Erkenner liefert
///   Endergebnisse verspätet) und werden verworfen; ein Ergebnis, das über die Grenze reicht, wird um den bereits
///   verbrauchten Text (`spent`) gekürzt.
/// - Stimmt die Zeitachse nicht (verschoben, Ergebnisse ohne Bereich), schaltet nach 1,5 s Sprache ohne angenommenes
///   Ergebnis der Bereichsfilter ab; dann gilt nur noch der Abgleich am Textanfang. So entsteht nie dauerhaft Stille.
struct AssistantUtterance: Equatable {
    /// Ergebnisse dürfen so viel (s) über die Grenze hinausreichen und trotzdem als alt gelten (Rundung der Zeitachse).
    static let tolerance = 0.05
    /// Sprache ohne angenommenes Ergebnis, nach der der Bereichsfilter abgeschaltet wird (ms).
    static let fallbackAfterMs = 1_500

    private(set) var stable = ""
    private(set) var volatile = ""
    /// Zeitachse (s), bis zu der der Erkenner Ergebnisse geliefert hat (auch über `consume` hinweg).
    private(set) var latestEnd = 0.0
    private(set) var cutoff = 0.0
    private var spent = ""
    private(set) var accepted = 0
    private(set) var staleDrops = 0
    private(set) var rangeFilter = true
    private var droppedSinceAccept = 0
    private var speechSinceAcceptMs = 0

    var text: String {
        let parts = [stable, volatile].filter { !$0.isEmpty }
        return parts.joined(separator: " ")
    }

    var isEmpty: Bool { text.isEmpty }

    /// Ergebnis des Erkenners einspeisen. `true`, wenn sich der sichtbare Text geändert hat.
    @discardableResult
    mutating func apply(_ raw: String, start: Double?, end: Double?, isFinal: Bool) -> Bool {
        var text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        let before = self.text

        if rangeFilter, let end, end <= cutoff + Self.tolerance {
            drop()
            return false
        }
        if rangeFilter, let start, start >= cutoff - Self.tolerance {
            spent = ""                       // beginnt nach der Grenze: sicher neuer Abschnitt
        }
        if !spent.isEmpty, !text.isEmpty {
            let (rest, matched) = Self.dropping(spent, from: text)
            if matched {
                text = rest
                if text.isEmpty {            // nur der alte Text, noch einmal
                    drop()
                    return false
                }
            } else {
                spent = ""
            }
        }

        accepted += 1
        droppedSinceAccept = 0
        speechSinceAcceptMs = 0
        if let end { latestEnd = max(latestEnd, end) }
        if isFinal {
            if !text.isEmpty { stable = stable.isEmpty ? text : stable + " " + text }
            volatile = ""
        } else {
            volatile = text
        }
        return self.text != before
    }

    private mutating func drop() {
        staleDrops += 1
        droppedSinceAccept += 1
    }

    /// Sprache läuft (`ms` je Puffer): ohne angenommenes Ergebnis, obwohl welche verworfen wurden, ist die Zeitachse
    /// verdächtig; der Bereichsfilter geht dann aus.
    mutating func noteSpeech(ms: Int) {
        guard rangeFilter, droppedSinceAccept > 0 else { return }
        speechSinceAcceptMs += ms
        if speechSinceAcceptMs >= Self.fallbackAfterMs { rangeFilter = false }
    }

    /// Äußerung beenden und Text abholen. `time` ist die aktuelle Position der Zeitachse (s).
    mutating func consume(at time: Double) -> String {
        let consumed = text
        if !consumed.isEmpty { spent = consumed }
        stable = ""
        volatile = ""
        cutoff = max(cutoff, time)
        droppedSinceAccept = 0
        speechSinceAcceptMs = 0
        return consumed
    }

    /// Alles auf Anfang (neues Gespräch).
    mutating func reset() { self = AssistantUtterance() }

    /// Entfernt den bereits verbrauchten Text am Anfang von `text`. `matched`, wenn der Anfang zu mindestens 80 % mit
    /// `spent` übereinstimmt (Satzzeichen und Großschreibung egal); `rest` ist der Teil dahinter.
    static func dropping(_ spent: String, from text: String) -> (rest: String, matched: Bool) {
        let old = AssistantWords.keys(spent)
        let tokens = text.split(whereSeparator: { $0.isWhitespace }).map(String.init)
        let kept = tokens.filter { !AssistantWords.key($0).isEmpty }
        let count = min(old.count, kept.count)
        guard count > 0 else { return (text, false) }
        var hits = 0
        for i in 0..<count where AssistantWords.key(kept[i]) == old[i] { hits += 1 }
        guard hits * 5 >= count * 4 else { return (text, false) }
        let rest = kept.dropFirst(old.count).joined(separator: " ")
        return (rest, true)
    }
}

/// Sprachbeginn und Stille je Mikrofonpuffer, wiederholt über das ganze Gespräch (anders als `BookTalkEndpointer`, der
/// nach einer Äußerung endet). Rauschpegel wird nur in Stille nachgeführt; Sprache liegt deutlich darüber.
struct AssistantVAD {
    enum Event: Equatable {
        case none
        case speechStarted
        /// Dauer der aktuellen Stille (ms); 0 bei Sprache. Kommt je Puffer, solange eine Äußerung läuft.
        case silence(Int)
    }

    let minSpeechMs: Int
    let minThreshold: Float
    /// So lange Ruhe, bevor erneute Sprache wieder als neuer Beginn zählt.
    let rearmQuietMs: Int

    private(set) var floor: Float = 0.004
    private var voiced = 0
    private var quiet = 0
    private var armed = true
    private var spoken = false
    private var silence = 0
    /// Der zuletzt gefütterte Puffer zählte als Sprache.
    private(set) var heardSpeech = false

    init(minSpeechMs: Int = 200, minThreshold: Float = 0.012, rearmQuietMs: Int = 250) {
        self.minSpeechMs = minSpeechMs
        self.minThreshold = minThreshold
        self.rearmQuietMs = rearmQuietMs
    }

    var threshold: Float { max(minThreshold, floor * 3) }
    /// Seit dem letzten `rearm()` wurde gesprochen.
    var inUtterance: Bool { spoken }

    mutating func feed(rms: Float, ms: Int) -> Event {
        heardSpeech = false
        guard ms > 0 else { return .none }
        let level = rms.isFinite ? max(0, rms) : 0
        let loud = level > threshold
        if !loud { floor = level < floor ? level : floor + (level - floor) * 0.02 }

        guard loud else {
            quiet += ms
            voiced = 0
            if quiet >= rearmQuietMs { armed = true }
            guard spoken else { return .none }
            silence += ms
            return .silence(silence)
        }

        voiced += ms
        quiet = 0
        guard voiced >= minSpeechMs else {
            // Kurzes Geräusch: zählt weder als Sprache noch beendet es die Stille.
            guard spoken else { return .none }
            silence += ms
            return .silence(silence)
        }
        heardSpeech = true
        silence = 0
        if armed {
            armed = false
            spoken = true
            return .speechStarted
        }
        return spoken ? .silence(0) : .none
    }

    /// Der Erkenner hat Text geliefert, obwohl hier keine Sprache erkannt wurde (leise Stimme): als Sprache behandeln,
    /// damit die Stille gezählt wird und die Äußerung enden kann.
    mutating func assumeSpeech() {
        guard !spoken else { return }
        spoken = true
        armed = false
        silence = 0
    }

    /// Nach Commit oder Verwerfen einer Äußerung: der nächste Sprachbeginn zählt wieder.
    mutating func rearm() {
        armed = true
        spoken = false
        voiced = 0
        silence = 0
        heardSpeech = false
    }
}

/// Hält die gemeldete Stille kurz, solange der Erkenner dem Sprachschluss noch hinterherhinkt: sonst fehlen beim Commit
/// die letzten Wörter, die erst nach der Stille erkannt werden.
enum AssistantSilenceGate {
    /// Bis hierher wird Stille immer gemeldet (unterhalb von Vorarbeit und Ende der Policy).
    static let holdMs = 300
    /// Ab hier wird sie ohne Rücksicht auf den Erkenner voll gemeldet.
    static let releaseMs = 1_300
    /// Der Erkenner gilt als eingeholt, wenn er höchstens so viel (s) vor dem letzten lauten Puffer endet.
    static let tolerance = 0.35

    static func reported(silenceMs: Int, lastLoudAt: Double, recognizedUntil: Double) -> Int {
        if silenceMs <= holdMs || silenceMs >= releaseMs { return silenceMs }
        return recognizedUntil >= lastLoudAt - tolerance ? silenceMs : holdMs
    }
}

/// Einmalige Umstellung auf „Freisprechen an“ (Merker `assistant_handsfree_migrated`), als reine Logik über `UserDefaults`
/// (testbar mit einer eigenen Suite). Erst den Wert schreiben, dann den Merker: bricht der Prozess dazwischen ab, läuft die
/// Umstellung beim nächsten Start noch einmal (gleiches Ergebnis). Ist der Merker gesetzt, gilt die Wahl des Nutzers.
enum AssistantHandsFreeMigration {
    static let marker = "assistant_handsfree_migrated"
    static let key = "handsFreeEnabled"

    /// - Parameter forceOff: nur für UI-Tests (`--no-handsfree`): aus, und der Merker gilt als gesetzt.
    static func resolve(_ defaults: UserDefaults, forceOff: Bool = false) -> Bool {
        if forceOff {
            defaults.set(false, forKey: key)
            defaults.set(true, forKey: marker)
            return false
        }
        guard defaults.object(forKey: marker) == nil else { return defaults.bool(forKey: key) }
        defaults.set(true, forKey: key)
        defaults.set(true, forKey: marker)
        return true
    }
}
