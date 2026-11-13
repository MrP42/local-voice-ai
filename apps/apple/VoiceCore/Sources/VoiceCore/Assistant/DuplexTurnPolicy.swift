import Foundation

// Gesprächsmodus „Sprechen“ (Paket K): reine Entscheidungslogik für Vollduplex. Kein Audio, keine Uhr: Zeiten kommen als
// Millisekunden seit Gesprächsbeginn mit den Eingaben. Die Steuerung (Paket A) füttert Eingaben und führt Aktionen aus.
//
// Vertrag mit der Steuerung:
// - Nach `commit` ruft sie `answerStarted` (sobald das Modell läuft) und irgendwann `answerFinished` (alles gesprochen
//   oder abgebrochen). Nach `speculate` darf sie `answerStarted` für die Vorarbeit melden; das gilt noch nicht als Antwort.
// - Bei `stopCommand`, `commit` und `releaseAnswer` verbraucht sie die Äußerung im Erkenner (`consumeUtterance()`); bei `ignored`
//   NICHT (die Richtlinie zieht verbrauchte Wörter selbst ab). Eine Äußerung sind die erkannten Wörter seit dem letzten Verbrauch.
//   `partial.text` darf kumulativ sein: Wörter, die als Echo/Rückmeldelaut verbraucht wurden, stehen vorn und zählen über die
//   Wortposition nicht mehr (nach „ja“ zählt bei „ja morgen“ nur „morgen“; wird „ja“ zu „Jahr“ korrigiert, entsteht nichts Neues).
//   Das gilt bis zum Äußerungsende; hat der Text weniger Wörter als das verbrauchte Präfix, begann eine neue Erkennersitzung.
// - Eine Vorarbeit startete mit genau dem Text aus `speculate`; ändert sich ein Wort, kommt `discardSpeculation`. Ob sie
//   bei `commit` weiterverwendet wird, entscheidet die Steuerung am Text; war zwischen `speculate` und `commit` ein
//   `cancelAnswer(merge: false)` im Spiel, ist `afterInterruption` zu beachten (die Vorarbeit kannte das Kennzeichen nicht).
// - Das Äußerungsende hängt vom Inhalt ab (`UtteranceShape`): ein Satz, der unfertig klingt („… ob“, „… und“), bekommt mehr
//   Stille, ein fertiger („…?“) weniger. `Config.pauseScale` streckt oder kürzt alle Fristen; `adaptiveEnd = false` gilt die feste
//   Frist `endAfterMs`.
// - Sprecherwechsel bei aktiver Antwort (nach `commit`, bis `answerFinished`), ADR-0028 E4.2c Punkt 7 / E9.
//   Begriffe:
//   · EINHEIT: eine gesprochene Einheit der Antwort, auch Ansage/Überbrückung. Die Steuerung meldet Beginn und Ende JEDER Einheit
//     mit `unitStarted` / `unitEnded` (nach dem tatsächlichen Klang). `sentenceSpoken` liefert Text und Klangintervall für den
//     Echo-Abgleich und „erster Antwortsatz fertig“ (Ende gemeldet, nur Antwortsätze, nicht Ansagen).
//   · WORTKLASSEN, bei jedem Ergebnis über alle Wörter der laufenden Äußerung, erste zutreffende gilt: Stoppbefehl > Echo >
//     Rückmeldelaut > relevant. Normalisiert: klein, Umlaute/ß gefaltet, Satzzeichen weg, Trennung an Leerraum.
//     1. STOPPBEFEHL: höchstens 3 Wörter, darunter stopp/stop/halt/warte/moment (ohne Ausnahme, auch wenn die Einheit selbst das Wort
//        spricht). Verbindlich mit dem ersten Ergebnis, auf das es zutrifft (auch vorläufig); der Rest der Äußerung wird bis zu ihrem
//        Ende verworfen. Hat das ERSTE Ergebnis einer Äußerung mehr als 3 Wörter, ist sie nie ein Stoppbefehl.
//     2. ECHO: Echo-Segment = Wörter aller Einheiten, deren Klangintervall [Klangbeginn, Klangende + `echoTailAudioMs`] das Intervall der
//        Äußerung (Sprachbeginn bis `audioMs`) schneidet. Überdeckung (Wörter der Äußerung im Segment / Wörter der Äußerung)
//        ≥ `echoOverlap`. Ohne `audioMs`: Ankunftszeit `atMs`, Nachlauf `echoWindowMs` (bei aktiver Antwort; sonst 1,5 s).
//     3. RÜCKMELDELAUT: höchstens 3 Wörter, alle aus der Liste, nur bei aktiver Antwort.
//     4. RELEVANT: alles andere.
//     Echo und Rückmeldelaut zählen als 0 relevante Wörter und werden verbraucht (`ignored`); danach folgende Wörter sind eine
//     neue Äußerung. Eine Matrix-Entscheidung ist ab Erreichen der Schwelle verbindlich; spätere Korrekturen ändern sie nicht.
//   · FORTSETZUNG: `cancelAnswer(merge: true)` vor dem ersten fertig gesprochenen Antwortsatz (die alte Anfrage samt wartenden
//     Ergebnissen ist verworfen, Text „alte Frage + Fortsetzung“), danach `cancelAnswer(merge: false)` (Gesprochenes bleibt als
//     „unterbrochen“, neue Äußerung mit Unterbrechungshinweis). Die Wörter werden genau einmal übernommen; weitere `partial` derselben
//     Äußerung vervollständigen nur die neue Äußerung. Neue Ausgabe erst nach dem Äußerungsende (`commit`).
//   · NACHGEREICHTE ANTWORT (`answerStarted(…, detached: true)`): wie jede Antwort (Zurückhalten, Stopp, Echo, Handbedienung, Matrix), aber
//     jede FORTSETZUNG ist `cancelAnswer(merge: false)`, auch vor dem ersten fertigen Satz; das spätere `commit` enthält nur die neuen
//     Wörter (`merged: false`), `afterInterruption` nur, wenn mindestens eine Einheit dieser Antwort geklungen hat (vor dem ersten Ton
//     verdrängt: false). Mit `answerFinished` (oder Abbruch) endet das; die nächste `answerStarted` ohne `detached` gilt wie bisher.
//   Matrix (jede Zeile hat einen Ausgang):
//   1. Keine Einheit klingt (vor der ersten und in jeder Lücke), 0 relevante Wörter: `speechStarted` liefert `holdAnswer`, außer nach
//      `manualRelease` (Zeile 7). `holdAnswer` heißt für die Steuerung: nichts Neues starten UND übergebenes, noch nicht klingendes
//      Audio zurücknehmen (das Modell darf weiterlaufen). Gehalten wird, solange der Pegel aktiv ist ODER die Erkennung nicht
//      eingeholt ist. `releaseAnswer` kommt erst bei gemeldeter Stille ≥ `holdQuietMs` (nach dem Sprachbeginn begonnen) UND eingeholter
//      Erkennung (`recognition(caughtUp:…)`, Anfangswert `true`); oder bei `unitStarted` während des Haltens (Wettlauf); oder bei
//      `manualRelease`. Der Erkennertext wird dabei verbraucht. Zeitablauf gibt NIE frei. Stattdessen `holdStalled(reason:)`, je Grund
//      einmal je Halten: `.recognition`, wenn die Stille ≥ `recognitionGraceMs` dauert, die Erkennung aber nicht eingeholt ist, oder
//      nach `holdNoticeMs` (insgesamt ab Haltebeginn, unabhängig von Pegelwechseln) bei nicht folgender Erkennung; `.activity` nach
//      `holdNoticeMs` bei folgender Erkennung (`following`: bestätigter Stand höchstens 1,5 s hinter dem Mikrofon). Die Steuerung
//      zeigt dann die Handbedienung: bei `.activity` „Antwort jetzt hören“ (`manualRelease`) und „Verwerfen“ (`manualDiscard`), bei
//      `.recognition` nur „Verwerfen“. Gehalten wird weiter.
//   2. Keine Einheit klingt, ≥ 1 relevantes Wort: FORTSETZUNG sofort, auch ohne vorheriges `holdAnswer`.
//   3. Eine Einheit klingt, 0 oder 1 relevantes Wort: sie klingt weiter, das Wort bleibt erhalten. Bei `unitEnded` gilt nach der dann
//      tatsächlichen Wortzahl: ≥ 1 Wort sofort Zeile 2 (auch bei der letzten Einheit, solange `answerFinished` aussteht; endete die
//      Äußerung schon während des Klangs, folgt im selben Schritt das `commit`, vorher nicht); 0 Wörter und
//      Pegel aktiv (Sprache läuft, keine Stille gemeldet) oder Erkennung nicht eingeholt: `holdAnswer` ab Klangende.
//   4. Eine Einheit klingt, ≥ `bargeInMinWords` relevante Wörter: sofort FORTSETZUNG (Ausgabe stoppt).
//   5. Stoppbefehl in jedem Zustand (klingt, Lücke, Halten, auch ein einzelnes „Stopp“): `cancelAnswer(merge: false)` + `stopCommand`.
//      `manualDiscard` („Verwerfen“) beendet die Antwort ebenso.
//   6. Echo und Rückmeldelaut: `ignored`, nie Fortsetzung, nie Unterbrechung; beim Halten keine Freigabe vor der Stille.
//   7. `manualRelease` bestätigt Nichtsprachgeräusch und wirkt nur bei folgender Erkennung (sonst keine Aktion): `releaseAnswer` sofort,
//      und für den Rest DIESER Antwort löst Pegelaktivität ohne relevantes Wort kein Halten mehr aus. Relevante Wörter und
//      Stoppbefehle wirken weiter. Mit der nächsten Antwort gilt wieder die normale Regel.
//   8. Nach Stoppbefehl, `manualDiscard`, Fortsetzung oder `answerFinished` ist die Antwort beendet: `manualRelease`, `manualDiscard`,
//      `unitStarted`/`unitEnded` und späte `sentenceSpoken` zu ihr bewirken nichts mehr, bis die nächste `answerStarted` kommt. Das
//      erste beendende Ereignis gewinnt.

public struct DuplexTurnPolicy: Sendable {
    public struct Config: Sendable {
        public var speculateAfterMs = 350
        public var endAfterMs = 600          // feste Frist, nur bei `adaptiveEnd == false`
        public var adaptiveEnd = true        // Ende nach Inhalt (`UtteranceShape`) statt fest `endAfterMs`
        public var endCompleteMs = 900       // Stille bis Ende bei fertig klingendem Satz
        public var endNeutralMs = 1_200      // ... bei neutralem Satz
        public var endOpenMs = 2_000         // ... bei unfertigem Satz
        public var pauseScale = 1.0          // App-Einstellung: kurz 0,8 / normal 1,0 / lang 1,5
        public var holdQuietMs = 700         // Stille nach Sprachbeginn, dann gibt `releaseAnswer` die Antwort frei
        public var holdNoticeMs = 6_000      // so lange gehalten (ab Haltebeginn) ohne Wort und Freigabe: einmal `holdStalled`
        public var recognitionGraceMs = 3_000   // Stille so lange da, Erkennung nicht eingeholt: einmal `holdStalled(.recognition)`
        public var bargeInMinWords = 2
        public var echoOverlap = 0.6
        public var echoWindowMs = 8_000      // Nachlauf ohne Audiozeit (`audioMs` nil)
        public var echoTailAudioMs = 300     // Nachlauf nach Klangende, wenn die Erkennung Audiozeit liefert
        public var speculate = true          // false bei Cloud-Modellen
        public var bargeIn = true            // false = Halbduplex (Schalter)
        public init() {}
    }

    public enum Input: Sendable, Equatable {
        case speechStarted(atMs: Int)
        /// Gesamter Text der laufenden Äußerung (fest + vorläufig). `audioMs`: Mikrofon-Audiozeit (gleiche Uhr wie `atMs`) des Endes
        /// des erkannten Bereichs; nil, wenn die Erkennung keine liefert.
        case partial(text: String, atMs: Int, audioMs: Int? = nil)
        case silence(sinceMs: Int, atMs: Int)        // Dauer der aktuellen Stille
        /// Modell läuft. `detached`: nachgereichte Antwort, die nicht auf die letzte festgeschriebene Äußerung antwortet (z. B. ein fertiges
        /// Suchergebnis): Nutzersprache wird nie mit der letzten Äußerung zusammengeführt (siehe Kopfkommentar).
        case answerStarted(requestId: String, detached: Bool = false)
        case sentenceSpoken(text: String, startedAtMs: Int, endedAtMs: Int?)
        case unitStarted(atMs: Int)                  // eine Einheit der Antwort (auch Ansage/Überbrückung) klingt jetzt
        case unitEnded(atMs: Int)                    // sie klingt nicht mehr (fertig oder abgebrochen)
        case manualRelease(atMs: Int)                // „Antwort jetzt hören“: der Nutzer gibt die zurückgehaltene Antwort von Hand frei
        case manualDiscard(atMs: Int)                // „Verwerfen“: beendet die Antwort wie ein Stoppbefehl
        /// Stand der Erkennung: `caughtUp` = sie hat den Ton bis zum letzten lauten Puffer verarbeitet; `following` = der bestätigte Stand
        /// liegt höchstens 1,5 s hinter dem Mikrofon. Anfangswert beider true.
        case recognition(caughtUp: Bool, following: Bool = true, atMs: Int)
        case answerFinished(requestId: String)       // alles gesprochen oder abgebrochen
    }

    public enum Action: Sendable, Equatable {
        case speculate(text: String)                 // Vorarbeit starten (verwerfbar)
        case discardSpeculation
        case commit(text: String, merged: Bool, afterInterruption: Bool)   // Äußerung gilt; Antwort (weiter)führen
        case cancelAnswer(merge: Bool)               // merge=true: Antwort verwerfen, Äußerung wird fortgesetzt
        case stopCommand                             // „stopp“: nur anhalten, keine neue Antwort
        case ignored(reason: String)                 // Echo, Rückmeldelaut (für Diagnose/Tests)
        case holdAnswer                              // Nutzer spricht, keine Einheit klingt: nichts Neues starten, übergebenes Audio zurücknehmen
        case releaseAnswer                           // Zurückhalten vorbei, Antwort darf sprechen (Erkennertext verbrauchen)
        case holdStalled(reason: StallReason)        // lange gehalten ohne Wort und Freigabe (einmal je Grund und Halten): Handbedienung zeigen
    }

    /// Warum das Halten nicht endet: Erkennung folgend, aber Pegel ohne Wort (`activity`; Handfreigabe und Verwerfen) oder Erkennung
    /// hinkt hinterher (`recognition`; nur Verwerfen).
    public enum StallReason: Sendable, Hashable { case activity, recognition }

    /// Gründe in `.ignored`.
    public static let reasonEcho = "echo"
    public static let reasonBackchannel = "backchannel"
    public static let reasonHalfDuplex = "halfduplex"

    private struct Spoken: Sendable {
        var text: String
        var words: Set<String>
        var startedAtMs: Int
        var endedAtMs: Int?
    }

    private static let backchannels: Set<String> = ["mhm", "mmh", "mh", "hm", "hmm", "ja", "jaja", "okay", "ok", "aha", "genau"]
    private static let stopWords: Set<String> = ["stopp", "stop", "halt", "warte", "moment"]
    /// Ein Stoppbefehl hat höchstens so viele Wörter; ein erstes Ergebnis mit mehr Wörtern macht die Äußerung nie zum Stoppbefehl.
    private static let stopMaxWords = 3
    private static let backchannelMaxWords = 3
    /// Nach dem Klangende einer Einheit zählt Nachhall noch so lange als möglicher Echotext.
    private static let echoGraceMs = 1_500
    /// Ein Satz ohne Ende (läuft noch oder wurde abgebrochen) zählt so lange.
    private static let unfinishedWindowMs = 30_000
    /// Eine Stille gilt nur als Stille nach dem Sprachbeginn, wenn sie nicht früher als dieser (abzüglich Zeitachsen-Ungenauigkeit) begann.
    private static let holdStaleToleranceMs = 100

    private var config: Config
    private var utterance = ""
    private var utteranceOpen = false        // das erste Ergebnis der laufenden Äußerung wurde schon eingestuft
    private var firstResultTooLong = false   // das erste Ergebnis hatte mehr als `stopMaxWords` Wörter: nie ein Stoppbefehl
    private var utteranceStartMs: Int?       // Sprachbeginn der laufenden Äußerung (Audiozeit), für das Echo-Segment
    private var consumedPrefixWords = 0     // Wörter am Anfang des kumulativen Erkennertexts, die als Echo/Rückmeldelaut verbraucht wurden
    private var continuing = false           // die Äußerung setzt eine verworfene Antwort fort: kein Echo-Aussortieren mehr
    private var speculated: String?          // gefaltete Wörter der Vorarbeit
    private var answerActive = false
    private var detached = false             // nachgereichte Antwort: nie mit der letzten Äußerung zusammenführen
    private var unitHeard = false            // mindestens eine Einheit dieser Antwort hat geklungen
    private var awaitingNewAnswer = false    // die Antwort ist beendet; Meldungen zu ihr wirken nicht mehr, bis `answerStarted` kommt
    private var answerUser = ""
    private var requestId: String?
    private var firstSentenceDone = false
    private var spoken: [Spoken] = []
    private var pendingMerge: String?
    private var interrupted = false
    private var swallow = false
    private var unitSounding = false         // gerade klingt eine Einheit (auch Ansage)
    private var holding = false
    private var holdSinceMs = 0
    private var stallReported: Set<StallReason> = []
    private var holdSuppressed = false       // Handfreigabe: für den Rest dieser Antwort kein Halten mehr
    private var recognitionCaughtUp = true
    private var recognitionFollowing = true
    private var lastSpeechStartMs: Int?      // letzter Sprachbeginn laut Pegel
    private var levelActive = false          // seit dem letzten Sprachbeginn wurde keine Stille gemeldet
    private var quietStartMs: Int?           // Beginn der gemeldeten Stille nach dem letzten Sprachbeginn

    public init(config: Config = .init()) { self.config = config }

    public mutating func reset() {
        utterance = ""
        utteranceOpen = false
        firstResultTooLong = false
        utteranceStartMs = nil
        consumedPrefixWords = 0
        continuing = false
        speculated = nil
        answerActive = false
        detached = false
        unitHeard = false
        awaitingNewAnswer = false
        answerUser = ""
        requestId = nil
        firstSentenceDone = false
        spoken.removeAll()
        pendingMerge = nil
        interrupted = false
        swallow = false
        unitSounding = false
        holding = false
        stallReported = []
        holdSuppressed = false
        recognitionCaughtUp = true
        recognitionFollowing = true
        lastSpeechStartMs = nil
        levelActive = false
        quietStartMs = nil
    }

    public mutating func feed(_ input: Input) -> [Action] {
        switch input {
        case .speechStarted(let atMs):
            // Der Nutzer redet (weiter): Vorarbeit ist gegenstandslos, bis die Stille wieder lang genug ist.
            var actions = discard()
            lastSpeechStartMs = atMs
            levelActive = true
            quietStartMs = nil
            if utteranceStartMs == nil { utteranceStartMs = atMs }
            if answerActive, !unitSounding, !holding, !holdSuppressed { actions.append(beginHold(atMs: atMs)) }
            return actions + holdExpiry(atMs: atMs)
        case .partial(let text, let atMs, let audioMs):
            return partial(text, atMs: atMs, audioMs: audioMs) + holdExpiry(atMs: atMs)
        case .silence(let since, let atMs):
            noteSilence(since: since, atMs: atMs)
            return silence(since: since, atMs: atMs) + holdExpiry(atMs: atMs)
        case .answerStarted(let id, let isDetached):
            requestId = id
            firstSentenceDone = false
            awaitingNewAnswer = false
            detached = isDetached
            unitHeard = false
            if speculated == nil { answerActive = true }
            return []
        case .sentenceSpoken(let text, let start, let end):
            record(text, start, end)
            return []
        case .unitStarted:
            guard answerActive, !awaitingNewAnswer else { return [] }
            unitSounding = true
            unitHeard = true
            return holdEndedBySound()
        case .unitEnded(let atMs):
            guard answerActive, !awaitingNewAnswer else { return [] }
            unitSounding = false
            return afterUnitEnded(atMs: atMs)
        case .manualRelease:
            guard holding, recognitionFollowing, !awaitingNewAnswer else { return [] }
            holdSuppressed = true
            return releaseHold()
        case .manualDiscard:
            guard answerActive, !awaitingNewAnswer else { return [] }
            stopAnswer()
            consumeUtterance()
            consumedPrefixWords = 0
            return [.cancelAnswer(merge: false), .stopCommand]
        case .recognition(let caughtUp, let following, let atMs):
            recognitionCaughtUp = caughtUp
            recognitionFollowing = following
            return holdExpiry(atMs: atMs)
        case .answerFinished(let id):
            if id == requestId {
                requestId = nil
                answerActive = false
                detached = false
                unitHeard = false
                awaitingNewAnswer = true
                unitSounding = false
                holding = false
                holdSuppressed = false
            }
            return []
        }
    }

    // MARK: Zurückhalten

    private mutating func beginHold(atMs: Int) -> Action {
        holding = true
        holdSinceMs = atMs
        stallReported = []
        return .holdAnswer
    }

    /// Klang einer Einheit während des Zurückhaltens: die Steuerung und die Richtlinie bleiben im Gleichschritt.
    private mutating func holdEndedBySound() -> [Action] {
        guard holding else { return [] }
        holding = false
        return [.releaseAnswer]
    }

    /// Zurückhalten beenden; der Erkennertext wird verbraucht.
    private mutating func releaseHold() -> [Action] {
        guard holding else { return [] }
        holding = false
        consumeUtterance()
        consumedPrefixWords = 0
        return [.releaseAnswer]
    }

    /// Klangende einer Einheit (Matrix Zeile 3): nach der dann tatsächlichen Wortzahl Fortsetzung oder Halten ab Klangende.
    private mutating func afterUnitEnded(atMs: Int) -> [Action] {
        guard !swallow else { return [] }
        if !Self.words(utterance).isEmpty {
            let shape = UtteranceShape.classify(utterance)
            var actions = [takeOverFromAnswer()]
            // Endete die Äußerung schon während des Klangs, gilt sie jetzt: die Fortsetzung wird übernommen.
            if let start = quietStartMs, atMs - start >= endMs(shape) { actions.append(commitUtterance()) }
            return actions
        }
        guard levelActive || !recognitionCaughtUp, !holding, !holdSuppressed else { return [] }
        return [beginHold(atMs: atMs)]
    }

    /// Eine gemeldete Stille zählt nur, wenn sie nach dem letzten Sprachbeginn begann; eine ältere gehört zur Zeit vor der Rede.
    private mutating func noteSilence(since: Int, atMs: Int) {
        if let start = lastSpeechStartMs, atMs - since < start - Self.holdStaleToleranceMs { return }
        // Der Hörer meldet je lautem Mikrofonpuffer `silence(sinceMs: 0)`: Das ist keine Stille, sondern laufender Pegel.
        guard since > 0 else {
            levelActive = true
            quietStartMs = nil
            return
        }
        quietStartMs = atMs - since
        levelActive = false
    }

    /// Freigabe nur bei gemeldeter Stille ≥ `holdQuietMs` UND eingeholter Erkennung; eine Freigabe nach Zeit gibt es nie. Sonst je
    /// Grund einmal `holdStalled`. Echte Wörter beenden das Halten über `takeOverFromAnswer`.
    private mutating func holdExpiry(atMs: Int) -> [Action] {
        guard holding else { return [] }
        let quiet = quietStartMs.map { max(0, atMs - $0) }
        if let quiet, quiet >= config.holdQuietMs, recognitionCaughtUp { return releaseHold() }
        let reason: StallReason?
        if let quiet, quiet >= config.recognitionGraceMs, !recognitionCaughtUp {
            reason = .recognition
        } else if atMs - holdSinceMs >= config.holdNoticeMs {
            reason = recognitionFollowing ? .activity : .recognition
        } else {
            reason = nil
        }
        guard let reason, stallReported.insert(reason).inserted else { return [] }
        return [.holdStalled(reason: reason)]
    }

    // MARK: Nutzerrede

    /// Wörter einer Äußerung: klein, Umlaute/ß gefaltet, Satzzeichen entfernt, Trennung an Leerraum.
    private static func words(_ text: String) -> [String] { AssistantWords.keys(text) }

    /// Text ohne die ersten `count` Wörter (Zeichen wie Satzzeichen bleiben an den Wörtern).
    private static func dropWords(_ count: Int, from text: String) -> String {
        var kept: [Substring] = []
        var seen = 0
        for token in text.split(whereSeparator: { $0.isWhitespace }) {
            if AssistantWords.key(String(token)).isEmpty { if seen >= count { kept.append(token) }; continue }
            seen += 1
            if seen > count { kept.append(token) }
        }
        return kept.joined(separator: " ")
    }

    /// Äußerung als Echo/Rückmeldelaut verbraucht: die Wörter stehen im kumulativen Text weiter vorn.
    private mutating func consumeIgnored(_ words: [String]) {
        consumeUtterance()
        consumedPrefixWords += words.count
    }

    /// Äußerung verbraucht (Erkenner und Richtlinie beginnen eine neue).
    private mutating func consumeUtterance() {
        utterance = ""
        utteranceOpen = false
        firstResultTooLong = false
        utteranceStartMs = nil
    }

    /// Der Text ist der kumulative Text der Erkennersitzung. Als Echo/Rückmeldelaut verbrauchte Wörter stehen vorn und zählen nicht
    /// mehr (Wortposition): „ja“ verbraucht, dann „ja morgen“ → neue Äußerung „morgen“; korrigiert der Erkenner „ja“ zu „Jahr“
    /// (gleiche Position, nichts dahinter), entsteht keine neue Äußerung. Hat der Text weniger Wörter als das Präfix, begann eine neue
    /// Erkennersitzung. Das Präfix gilt bis zum Äußerungsende (Stille, Commit, Stopp, Freigabe): danach beginnt die nächste Äußerung vorn.
    private mutating func partial(_ fullText: String, atMs: Int, audioMs: Int?) -> [Action] {
        var text = fullText
        var words = Self.words(fullText)
        if consumedPrefixWords > 0 {
            if words.count < consumedPrefixWords {
                consumedPrefixWords = 0
            } else {
                words = Array(words.dropFirst(consumedPrefixWords))
                text = Self.dropWords(consumedPrefixWords, from: fullText)
            }
        }
        guard !words.isEmpty, !swallow else { return [] }
        if !utteranceOpen {
            utteranceOpen = true
            firstResultTooLong = words.count > Self.stopMaxWords
        }
        let segment = echoSegment(atMs: atMs, audioMs: audioMs)

        guard answerActive else {
            if !continuing, echoes(words, segment) {
                consumeIgnored(words)
                return [.ignored(reason: Self.reasonEcho)]
            }
            utterance = text
            guard let key = speculated, key != words.joined(separator: " ") else { return [] }
            return discard()
        }

        guard config.bargeIn else {
            consumeUtterance()
            return [.ignored(reason: Self.reasonHalfDuplex)]
        }
        // Rangfolge: Stoppbefehl > Echo > Rückmeldelaut > relevant.
        if !firstResultTooLong, words.count <= Self.stopMaxWords, words.contains(where: { Self.stopWords.contains($0) }) {
            stopAnswer()
            swallow = true
            consumeUtterance()
            consumedPrefixWords = 0
            return [.cancelAnswer(merge: false), .stopCommand]
        }
        if echoes(words, segment) {
            consumeIgnored(words)
            return [.ignored(reason: Self.reasonEcho)]
        }
        if words.count <= Self.backchannelMaxWords, words.allSatisfy({ Self.backchannels.contains($0) }) {
            consumeIgnored(words)
            return [.ignored(reason: Self.reasonBackchannel)]
        }

        utterance = text
        // Klingt eine Einheit, braucht die Unterbrechung `bargeInMinWords` Wörter (Echo-Risiko); ein einzelnes Wort bleibt dann stehen
        // und zählt bei `unitEnded` oder am Äußerungsende. Klingt nichts (Zeile 1/2), ist schon ein relevantes Wort eine Fortsetzung.
        guard words.count >= (unitSounding ? config.bargeInMinWords : 1) else { return [] }
        return [takeOverFromAnswer()]
    }

    /// Nutzerrede unterbricht die Antwort: vor dem ersten fertig gesprochenen Satz wird die Äußerung ergänzt (Antwort
    /// verworfen, beide Äußerungen werden eine), danach ist es eine Unterbrechung.
    private mutating func takeOverFromAnswer() -> Action {
        let wasDetached = detached
        let heard = unitHeard
        stopAnswer()
        continuing = true
        if wasDetached {
            // Nachgereichte Antwort: nie zusammenführen; Unterbrechung nur, wenn schon etwas geklungen hat.
            interrupted = heard
            return .cancelAnswer(merge: false)
        }
        if firstSentenceDone {
            interrupted = true
            return .cancelAnswer(merge: false)
        }
        pendingMerge = answerUser
        return .cancelAnswer(merge: true)
    }

    /// Die Antwort ist beendet (Stopp, Verwerfen, Fortsetzung): Meldungen zu ihr wirken nicht mehr, bis `answerStarted` kommt.
    private mutating func stopAnswer() {
        answerActive = false
        detached = false
        unitHeard = false
        awaitingNewAnswer = true
        requestId = nil
        unitSounding = false
        holding = false
        holdSuppressed = false
    }

    private mutating func silence(since: Int, atMs: Int) -> [Action] {
        // Kürzeste Frist: ab hier ist eine leere oder geschluckte Äußerung abgeschlossen.
        let resetMs = endMs(.complete)
        if swallow {
            if since >= resetMs { swallow = false; consumeUtterance(); consumedPrefixWords = 0 }
            return []
        }
        let words = Self.words(utterance)
        guard !words.isEmpty else {
            if since >= resetMs { consumeUtterance(); consumedPrefixWords = 0 }
            return []
        }
        // Ein einzelnes Wort bei klingender Einheit bleibt stehen, auch wenn die Äußerung vor dem Klangende endet: Es wird erst mit der
        // Matrixentscheidung am Klangende verbraucht (`afterUnitEnded`).
        if answerActive, unitSounding { return [] }
        let shape = UtteranceShape.classify(utterance)
        if since >= endMs(shape) {
            var actions: [Action] = []
            if answerActive { actions.append(takeOverFromAnswer()) }
            actions.append(commitUtterance())
            return actions
        }
        // Ein Satz, der unfertig klingt, bekommt keine Vorarbeit: die Fortsetzung kommt gleich.
        let unfinished = config.adaptiveEnd && shape == .open
        if since >= config.speculateAfterMs, config.speculate, speculated == nil, !answerActive, !unfinished {
            speculated = words.joined(separator: " ")
            return [.speculate(text: currentText())]
        }
        return []
    }

    /// Die Äußerung gilt: `commit` und Zustand der neuen Antwort.
    private mutating func commitUtterance() -> Action {
        let text = currentText()
        let merged = pendingMerge.map { !$0.isEmpty } ?? false
        let action = Action.commit(text: text, merged: merged, afterInterruption: interrupted)
        consumeUtterance()
        consumedPrefixWords = 0
        continuing = false
        speculated = nil
        pendingMerge = nil
        interrupted = false
        detached = false
        unitHeard = false
        answerActive = true
        answerUser = text
        firstSentenceDone = false
        unitSounding = false
        holding = false
        holdSuppressed = false
        return action
    }

    /// Stille bis zum Äußerungsende für diese Form, mit `pauseScale`; ohne `adaptiveEnd` die feste Frist.
    private func endMs(_ shape: UtteranceShape.Shape) -> Int {
        guard config.adaptiveEnd else { return config.endAfterMs }
        let base: Int
        switch shape {
        case .complete: base = config.endCompleteMs
        case .neutral: base = config.endNeutralMs
        case .open: base = config.endOpenMs
        }
        return Int((Double(base) * config.pauseScale).rounded())
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
        let entry = Spoken(text: text, words: Set(Self.words(text)), startedAtMs: start, endedAtMs: end)
        if let i = spoken.lastIndex(where: { $0.text == text && $0.startedAtMs == start }) { spoken[i] = entry } else { spoken.append(entry) }
        if spoken.count > 40 { spoken.removeFirst(spoken.count - 40) }
        if end != nil, !awaitingNewAnswer { firstSentenceDone = true }
    }

    /// Echo-Segment: Wörter aller Einheiten, deren Klang die Zeit der Äußerung einschließt.
    /// Mit `audioMs`: Klangintervall [Beginn, Ende + `echoTailAudioMs`] gegen das Intervall der Äußerung (Sprachbeginn bis `audioMs`); ein Satz ohne
    /// Ende zählt `unfinishedWindowMs` lang. Ohne `audioMs`: Ankunftszeit und Nachlauf `echoWindowMs` (bei aktiver Antwort, sonst 1,5 s).
    private func echoSegment(atMs: Int, audioMs: Int?) -> Set<String> {
        let units: [Spoken]
        if let audio = audioMs {
            let start = min(utteranceStartMs ?? audio, audio)
            units = spoken.filter { s in
                let end = s.endedAtMs.map { $0 + config.echoTailAudioMs } ?? (s.startedAtMs + Self.unfinishedWindowMs)
                return s.startedAtMs <= audio && end >= start
            }
        } else {
            let tail = answerActive ? config.echoWindowMs : Self.echoGraceMs
            units = spoken.filter { s in
                guard let end = s.endedAtMs else { return atMs - s.startedAtMs <= Self.unfinishedWindowMs }
                return atMs - end <= tail
            }
        }
        return units.reduce(into: Set<String>()) { $0.formUnion($1.words) }
    }

    /// Echo: Überdeckung (Wörter der Äußerung im Segment / Wörter der Äußerung) ≥ `echoOverlap`.
    private func echoes(_ words: [String], _ segment: Set<String>) -> Bool {
        guard !segment.isEmpty, !words.isEmpty else { return false }
        let hits = words.filter { segment.contains($0) }.count
        return Double(hits) / Double(words.count) >= config.echoOverlap
    }
}

/// Form einer Äußerung für das Ende nach Inhalt (reine Funktion, nur Text).
/// - `open`: klingt unfertig (endet auf Konjunktion, Präposition, Artikel, Possessiv, Füllwort oder Zweitstellungsverb, auf
///   „,“ „…“ „-“ „:“, oder ist zu kurz: ohne Satzzeichen unter drei Wörtern). Hat Vorrang vor `complete`: „Das reicht und.“ ist offen.
/// - `complete`: endet auf „.“ „?“ „!“ mit mindestens zwei Wörtern (ohne Satzzeichen müssen es drei sein, sonst `open`), oder ist ein
///   Einwort-Beitrag („ja“, „danke“, „stopp“).
/// - `neutral`: alles andere, auch ein einzelnes Wort mit Satzzeichen außerhalb der Einwortliste.
/// Wörter, die am Satzende oft fertig sind (abtrennbare Partikeln wie „Lies mir das vor.“, Hilfsverben wie „… wie spät es ist?“,
/// „Was ist das?“), gelten nur ohne Satzzeichen am Ende als offen; mit Satzzeichen ist der Satz fertig.
public enum UtteranceShape {
    public enum Shape: Sendable, Equatable { case open, complete, neutral }

    /// Vergleichsschlüssel (`AssistantWords.key`: klein, Umlaute gefaltet).
    private static func keys(_ words: [String]) -> Set<String> { Set(words.map(AssistantWords.key)) }

    /// Offen, auch wenn ein Satzzeichen folgt.
    private static let strongOpen = keys([
        "und", "oder", "aber", "dass", "weil", "wenn", "ob", "also", "denn", "sondern", "mit", "von", "für", "zum", "zur", "in",
        "im", "am", "bei", "über", "unter", "der", "den", "dem", "des", "ein", "eine", "einen", "einem", "einer", "mein", "meine",
        "dein", "deine", "ähm", "äh", "hm", "hmm", "ehm", "möchte", "will", "soll",
        "and", "or", "but", "that", "because", "if", "the", "a", "to", "of", "with", "for", "is", "are",
    ])
    /// Offen nur ohne Satzzeichen am Ende.
    private static let weakOpen = keys(["an", "auf", "aus", "vor", "nach", "zu", "die", "das", "ist", "sind", "war", "hat", "habe", "wird", "kann"])
    private static let oneWordReplies = keys(["ja", "nein", "doch", "danke", "hallo", "stopp", "stop", "weiter", "genau", "okay", "ok", "richtig", "falsch"])
    private static let openPunctuation = [",", "…", "...", "-", "–", "—", ":", ";"]
    private static let closingPunctuation: Set<Character> = [".", "?", "!"]
    private static let minimumWords = 3
    private static let minimumWordsClosed = 2

    public static func classify(_ text: String) -> Shape {
        let raw = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if openPunctuation.contains(where: { raw.hasSuffix($0) }) { return .open }
        let words = AssistantWords.keys(raw)
        let closed = raw.last.map { closingPunctuation.contains($0) } ?? false
        if let last = words.last {
            if strongOpen.contains(last) { return .open }
            if !closed, weakOpen.contains(last) { return .open }
            if words.count == 1, oneWordReplies.contains(last) { return .complete }
        }
        // Mit Satzzeichen genügen zwei Wörter („Wie spät?“); ein einzelnes Wort außerhalb der Einwortliste ist nur neutral.
        if words.count < (closed ? minimumWordsClosed : minimumWords) { return closed && words.count == 1 ? .neutral : .open }
        return closed ? .complete : .neutral
    }
}
