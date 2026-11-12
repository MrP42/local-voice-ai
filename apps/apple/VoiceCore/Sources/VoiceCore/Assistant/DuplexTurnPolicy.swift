import Foundation

// Gesprächsmodus „Sprechen“ (Paket K): reine Entscheidungslogik für Vollduplex. Kein Audio, keine Uhr: Zeiten kommen als
// Millisekunden seit Gesprächsbeginn mit den Eingaben. Die Steuerung (Paket A) füttert Eingaben und führt Aktionen aus.
//
// Vertrag mit der Steuerung:
// - Nach `commit` ruft sie `answerStarted` (sobald das Modell läuft) und irgendwann `answerFinished` (alles gesprochen
//   oder abgebrochen). Nach `speculate` darf sie `answerStarted` für die Vorarbeit melden; das gilt noch nicht als Antwort.
// - Bei `ignored`, `stopCommand` und `commit` verbraucht sie die Äußerung im Erkenner (`consumeUtterance()`), sonst steht
//   der ignorierte Text beim nächsten `partial` wieder davor.
// - Eine Vorarbeit startete mit genau dem Text aus `speculate`; ändert sich ein Wort, kommt `discardSpeculation`. Ob sie
//   bei `commit` weiterverwendet wird, entscheidet die Steuerung am Text; war zwischen `speculate` und `commit` ein
//   `cancelAnswer(merge: false)` im Spiel, ist `afterInterruption` zu beachten (die Vorarbeit kannte das Kennzeichen nicht).

public struct DuplexTurnPolicy: Sendable {
    public struct Config: Sendable {
        public var speculateAfterMs = 350
        public var endAfterMs = 600
        public var bargeInMinWords = 2
        public var echoOverlap = 0.6
        public var echoWindowMs = 8_000
        public var speculate = true          // false bei Cloud-Modellen
        public var bargeIn = true            // false = Halbduplex (Schalter)
        public init() {}
    }

    public enum Input: Sendable, Equatable {
        case speechStarted(atMs: Int)
        case partial(text: String, atMs: Int)        // gesamter Text der laufenden Äußerung (fest + vorläufig)
        case silence(sinceMs: Int, atMs: Int)        // Dauer der aktuellen Stille
        case answerStarted(requestId: String)        // Modell läuft
        case sentenceSpoken(text: String, startedAtMs: Int, endedAtMs: Int?)
        case answerFinished(requestId: String)       // alles gesprochen oder abgebrochen
    }

    public enum Action: Sendable, Equatable {
        case speculate(text: String)                 // Vorarbeit starten (verwerfbar)
        case discardSpeculation
        case commit(text: String, merged: Bool, afterInterruption: Bool)   // Äußerung gilt; Antwort (weiter)führen
        case cancelAnswer(merge: Bool)               // merge=true: Antwort verwerfen, Äußerung wird fortgesetzt
        case stopCommand                             // „stopp“: nur anhalten, keine neue Antwort
        case ignored(reason: String)                 // Echo, Rückmeldelaut (für Diagnose/Tests)
    }

    /// Gründe in `.ignored`.
    public static let reasonEcho = "echo"
    public static let reasonBackchannel = "backchannel"
    public static let reasonHalfDuplex = "halfduplex"

    private struct Spoken: Sendable {
        var text: String
        var words: Set<String>
        var startedAtMs: Int
        var endedAtMs: Int?
        var lastMs: Int { endedAtMs ?? startedAtMs }
    }

    private static let backchannels: Set<String> = ["mhm", "mmh", "mh", "hm", "hmm", "ja", "jaja", "okay", "ok", "aha", "genau"]
    private static let stopWords: Set<String> = ["stopp", "stop", "halt", "warte", "moment"]
    /// Nach dem letzten gesprochenen Satz zählt Nachhall noch so lange als möglicher Echotext.
    private static let echoGraceMs = 1_500
    /// Ein Satz ohne Ende (läuft noch oder wurde abgebrochen) zählt so lange.
    private static let unfinishedWindowMs = 30_000

    private var config: Config
    private var utterance = ""
    private var speculated: String?          // gefaltete Wörter der Vorarbeit
    private var answerActive = false
    private var answerUser = ""
    private var requestId: String?
    private var firstSentenceDone = false
    private var spoken: [Spoken] = []
    private var pendingMerge: String?
    private var interrupted = false
    private var swallow = false

    public init(config: Config = .init()) { self.config = config }

    public mutating func reset() {
        utterance = ""
        speculated = nil
        answerActive = false
        answerUser = ""
        requestId = nil
        firstSentenceDone = false
        spoken.removeAll()
        pendingMerge = nil
        interrupted = false
        swallow = false
    }

    public mutating func feed(_ input: Input) -> [Action] {
        switch input {
        case .speechStarted:
            // Der Nutzer redet (weiter): Vorarbeit ist gegenstandslos, bis die Stille wieder lang genug ist.
            return discard()
        case .partial(let text, let atMs):
            return partial(text, atMs: atMs)
        case .silence(let since, let atMs):
            return silence(since: since, atMs: atMs)
        case .answerStarted(let id):
            requestId = id
            firstSentenceDone = false
            if speculated == nil { answerActive = true }
            return []
        case .sentenceSpoken(let text, let start, let end):
            record(text, start, end)
            return []
        case .answerFinished(let id):
            if id == requestId {
                requestId = nil
                answerActive = false
            }
            return []
        }
    }

    // MARK: Nutzerrede

    private mutating func partial(_ text: String, atMs: Int) -> [Action] {
        let words = BookTalkTokens.foldedWords(text)
        guard !words.isEmpty, !swallow else { return [] }

        guard answerActive else {
            if words.count >= 2, echoes(words, atMs: atMs, grace: true) { return [.ignored(reason: Self.reasonEcho)] }
            utterance = text
            guard let key = speculated, key != words.joined(separator: " ") else { return [] }
            return discard()
        }

        guard config.bargeIn else { return [.ignored(reason: Self.reasonHalfDuplex)] }
        if echoes(words, atMs: atMs, grace: false) { return [.ignored(reason: Self.reasonEcho)] }
        if words.count <= 3, words.allSatisfy({ Self.backchannels.contains($0) }) { return [.ignored(reason: Self.reasonBackchannel)] }
        if words.count <= 3, words.contains(where: { Self.stopWords.contains($0) }) {
            stopAnswer()
            swallow = true
            utterance = ""
            return [.cancelAnswer(merge: false), .stopCommand]
        }

        utterance = text
        // Ein einzelnes Wort zählt erst, wenn es bis zum Äußerungsende steht (siehe `silence`).
        guard words.count >= config.bargeInMinWords else { return [] }
        return [takeOverFromAnswer()]
    }

    /// Nutzerrede unterbricht die Antwort: vor dem ersten fertig gesprochenen Satz wird die Äußerung ergänzt (Antwort
    /// verworfen, beide Äußerungen werden eine), danach ist es eine Unterbrechung.
    private mutating func takeOverFromAnswer() -> Action {
        stopAnswer()
        if firstSentenceDone {
            interrupted = true
            return .cancelAnswer(merge: false)
        }
        pendingMerge = answerUser
        return .cancelAnswer(merge: true)
    }

    private mutating func stopAnswer() {
        answerActive = false
        requestId = nil
    }

    private mutating func silence(since: Int, atMs: Int) -> [Action] {
        if swallow {
            if since >= config.endAfterMs { swallow = false; utterance = "" }
            return []
        }
        let words = BookTalkTokens.foldedWords(utterance)
        guard !words.isEmpty else {
            if since >= config.endAfterMs { utterance = "" }
            return []
        }
        if since >= config.endAfterMs {
            var actions: [Action] = []
            if answerActive { actions.append(takeOverFromAnswer()) }
            let text = currentText()
            let merged = pendingMerge.map { !$0.isEmpty } ?? false
            actions.append(.commit(text: text, merged: merged, afterInterruption: interrupted))
            utterance = ""
            speculated = nil
            pendingMerge = nil
            interrupted = false
            answerActive = true
            answerUser = text
            firstSentenceDone = false
            return actions
        }
        if since >= config.speculateAfterMs, config.speculate, speculated == nil, !answerActive {
            speculated = words.joined(separator: " ")
            return [.speculate(text: currentText())]
        }
        return []
    }

    /// Text, der gilt: bei Ergänzung „<alte Äußerung> <neue Äußerung>“.
    private func currentText() -> String {
        let new = utterance.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let old = pendingMerge?.trimmingCharacters(in: .whitespacesAndNewlines), !old.isEmpty else { return new }
        return old + " " + new
    }

    private mutating func discard() -> [Action] {
        guard speculated != nil else { return [] }
        speculated = nil
        if !answerActive { requestId = nil }   // spätes `answerFinished` der Vorarbeit trifft dann nichts
        return [.discardSpeculation]
    }

    // MARK: Echo

    private mutating func record(_ text: String, _ start: Int, _ end: Int?) {
        let entry = Spoken(text: text, words: Set(BookTalkTokens.foldedWords(text)), startedAtMs: start, endedAtMs: end)
        if let i = spoken.lastIndex(where: { $0.text == text && $0.startedAtMs == start }) { spoken[i] = entry } else { spoken.append(entry) }
        if spoken.count > 40 { spoken.removeFirst(spoken.count - 40) }
        if end != nil { firstSentenceDone = true }
    }

    /// Stammen die Wörter zu `echoOverlap` aus den zuletzt gesprochenen Sätzen (Fenster `echoWindowMs`)?
    /// Mit `grace` gilt das auch kurz nach dem Ende der Antwort (Nachhall, verspätete Erkennung).
    private func echoes(_ words: [String], atMs: Int, grace: Bool) -> Bool {
        let recent = spoken.filter { s in
            guard let end = s.endedAtMs else { return atMs - s.startedAtMs <= Self.unfinishedWindowMs }
            return atMs - end <= (grace ? Self.echoGraceMs : config.echoWindowMs)
        }
        guard !recent.isEmpty else { return false }
        let vocabulary = recent.reduce(into: Set<String>()) { $0.formUnion($1.words) }
        let hits = words.filter { vocabulary.contains($0) }.count
        return Double(hits) / Double(words.count) >= config.echoOverlap
    }
}
