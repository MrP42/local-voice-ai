import Foundation

// Testprogramm für die Logik des Gesprächsmodus (Paket A), ohne Xcode und ohne Gerät:
//   cd apps/apple/iPhone/Assistant && swiftc AssistantListenerLogic.swift Tests/AssistantLogicTests.swift -o /tmp/assistant-logic && /tmp/assistant-logic
// Gibt je Prüfung PASS/FAIL aus und endet mit Status 1, wenn etwas fehlschlägt. Nur synthetische Texte.
// NICHT in das App-Target aufnehmen (eigenes `@main`).

@main
struct AssistantLogicTests {
    nonisolated(unsafe) static var failures = 0

    static func check(_ ok: Bool, _ name: String, line: Int = #line) {
        if ok {
            print("PASS \(name)")
        } else {
            failures += 1
            print("FAIL \(name) (Zeile \(line))")
        }
    }

    static func main() {
        words()
        utterance()
        utteranceFallback()
        vad()
        gate()
        migration()
        print(failures == 0 ? "ALLE BESTANDEN" : "\(failures) FEHLGESCHLAGEN")
        exit(failures == 0 ? 0 : 1)
    }

    // MARK: Wörter

    static func words() {
        check(AssistantWords.keys("Wie wird das Wetter?") == ["wie", "wird", "das", "wetter"], "keys: Satzzeichen und Großschreibung")
        check(AssistantWords.same("Über Väter, bitte!", "uber vater bitte"), "same: Umlaute gefaltet")
        check(!AssistantWords.same("a b", "a b c"), "same: Länge zählt")
        check(AssistantWords.keys(" — ").isEmpty, "keys: nur Satzzeichen")
    }

    // MARK: Äußerung

    static func utterance() {
        var u = AssistantUtterance()
        check(u.apply("wie wird", start: 0, end: 0.8, isFinal: false), "Äußerung: vorläufiger Text")
        check(u.text == "wie wird" && u.stable.isEmpty, "Äußerung: nur vorläufig")
        u.apply("Wie wird das Wetter", start: 0, end: 1.6, isFinal: false)
        u.apply("Wie wird das Wetter morgen?", start: 0, end: 2.2, isFinal: true)
        check(u.stable == "Wie wird das Wetter morgen?" && u.volatile.isEmpty, "Äußerung: Endergebnis wird fest")
        check(abs(u.latestEnd - 2.2) < 1e-9, "Äußerung: latestEnd")
        u.apply("Und", start: 2.4, end: 2.6, isFinal: false)
        check(u.text == "Wie wird das Wetter morgen? Und", "Äußerung: fest plus vorläufig")

        let consumed = u.consume(at: 3.0)
        check(consumed == "Wie wird das Wetter morgen? Und", "consume: liefert den Text")
        check(u.isEmpty, "consume: setzt zurück")

        check(!u.apply("Und dann", start: 2.4, end: 2.9, isFinal: true), "alt: Endergebnis vor der Grenze wird verworfen")
        check(u.isEmpty && u.staleDrops == 1, "alt: Zähler und Text")

        u.apply("Wie wird das Wetter morgen? Und übermorgen", start: 0, end: 4.2, isFinal: false)
        check(u.text == "übermorgen", "Überlappung: verbrauchter Anfang wird abgezogen")

        var v = AssistantUtterance()
        v.apply("Hallo zusammen", start: 0, end: 1, isFinal: true)
        _ = v.consume(at: 2.0)
        v.apply("Danke schön", start: 2.5, end: 3.2, isFinal: true)
        check(v.text == "Danke schön", "neuer Abschnitt: nach der Grenze, kein Abzug")
        // Ein weiteres Ergebnis darf nicht mehr gekürzt werden, auch wenn es zufällig gleich beginnt.
        v.apply("Hallo wieder", start: 3.3, end: 3.9, isFinal: true)
        check(v.text == "Danke schön Hallo wieder", "neuer Abschnitt: spent wurde zurückgesetzt")

        var w = AssistantUtterance()
        w.apply("eins zwei drei vier fünf", start: 0, end: 2, isFinal: false)
        _ = w.consume(at: 3.0)
        w.apply("Eins, zwei, drei, vier, fünf sechs", start: 0, end: 4, isFinal: false)
        check(w.text == "sechs", "Überlappung: Satzzeichen und Großschreibung egal")
        w.apply("eins zwei drei vier fünf", start: 0, end: 3.01, isFinal: true)
        check(w.text == "sechs", "Überlappung: reines Wiederholen ändert nichts")

        var x = AssistantUtterance()
        x.apply("hallo", start: 0, end: 1, isFinal: false)
        x.apply("", start: 0, end: 1, isFinal: true)
        check(x.isEmpty, "leeres Endergebnis löscht den vorläufigen Teil")

        let (rest, matched) = AssistantUtterance.dropping("a b c d e", from: "a b c x e f")
        check(matched && rest == "f", "dropping: 80 % reichen")
        let (rest2, matched2) = AssistantUtterance.dropping("a b c d e", from: "x y z")
        check(!matched2 && rest2 == "x y z", "dropping: anderer Text bleibt")

        var y = AssistantUtterance()
        y.apply("ohne bereich", start: nil, end: nil, isFinal: false)
        check(y.text == "ohne bereich", "ohne Zeitangaben: wird angenommen")
    }

    static func utteranceFallback() {
        var u = AssistantUtterance()
        u.apply("hallo welt", start: 0, end: 1, isFinal: true)
        _ = u.consume(at: 5.0)
        // Zeitachse verschoben: neue Sprache trägt alte Zeiten und würde dauerhaft verworfen.
        check(!u.apply("was ist das", start: 2, end: 3, isFinal: false), "Rückfall: zunächst verworfen")
        check(u.rangeFilter, "Rückfall: Filter noch an")
        u.noteSpeech(ms: 1_000)
        check(u.rangeFilter, "Rückfall: nach 1,0 s noch an")
        u.noteSpeech(ms: 600)
        check(!u.rangeFilter, "Rückfall: nach 1,6 s Sprache ohne Ergebnis aus")
        check(u.apply("was ist das", start: 2, end: 3, isFinal: false), "Rückfall: jetzt angenommen")
        check(u.text == "was ist das", "Rückfall: Text da")

        _ = u.consume(at: 6.0)
        check(!u.apply("was ist das", start: 2, end: 3, isFinal: true), "Rückfall: alter Text wird über den Anfangsabgleich verworfen")

        var v = AssistantUtterance()
        v.noteSpeech(ms: 5_000)
        check(v.rangeFilter, "Rückfall: ohne verworfene Ergebnisse bleibt der Filter an (Erkenner nur langsam)")
    }

    // MARK: VAD

    static func vad() {
        var v = AssistantVAD()
        let loud: Float = 0.1, quiet: Float = 0.001
        check(v.feed(rms: quiet, ms: 100) == .none, "VAD: Ruhe vor Sprache")
        check(v.feed(rms: loud, ms: 100) == .none, "VAD: erster lauter Puffer reicht nicht")
        check(v.feed(rms: loud, ms: 100) == .speechStarted, "VAD: Sprachbeginn nach 200 ms")
        check(v.heardSpeech, "VAD: heardSpeech")
        check(v.feed(rms: loud, ms: 100) == .silence(0), "VAD: Sprache meldet Stille 0")
        check(v.feed(rms: quiet, ms: 100) == .silence(100), "VAD: Stille zählt")
        check(v.feed(rms: quiet, ms: 100) == .silence(200), "VAD: Stille wächst")
        check(!v.heardSpeech, "VAD: kein heardSpeech in Stille")

        // Kurze Lücke (100 ms < 250 ms), dann wieder Sprache: kein neuer Beginn.
        v.feed(rms: loud, ms: 100)
        check(v.feed(rms: loud, ms: 100) == .silence(0), "VAD: Weitersprechen nach kurzer Lücke ist kein neuer Beginn")

        // Längere Lücke (300 ms): neuer Beginn.
        _ = v.feed(rms: quiet, ms: 150)
        _ = v.feed(rms: quiet, ms: 150)
        _ = v.feed(rms: loud, ms: 100)
        check(v.feed(rms: loud, ms: 100) == .speechStarted, "VAD: nach 300 ms Ruhe zählt Sprache wieder als Beginn")

        // Kurzes Geräusch in Stille beendet die Stille nicht.
        _ = v.feed(rms: quiet, ms: 100)
        let before = v.feed(rms: quiet, ms: 100)
        let click = v.feed(rms: loud, ms: 100)
        if case .silence(let a) = before, case .silence(let b) = click {
            check(b == a + 100, "VAD: Klick zählt als Stille")
        } else {
            check(false, "VAD: Klick zählt als Stille")
        }
        check(v.feed(rms: quiet, ms: 100) != .silence(0), "VAD: nach dem Klick weiter Stille")

        v.rearm()
        check(!v.inUtterance, "VAD: rearm beendet die Äußerung")
        check(v.feed(rms: quiet, ms: 100) == .none, "VAD: nach rearm keine Stille-Ereignisse")
        _ = v.feed(rms: loud, ms: 100)
        check(v.feed(rms: loud, ms: 100) == .speechStarted, "VAD: nach rearm wieder Beginn")

        var w = AssistantVAD()
        w.assumeSpeech()
        check(w.feed(rms: quiet, ms: 100) == .silence(100), "VAD: assumeSpeech startet die Stille-Zählung")

        var n = AssistantVAD()
        for _ in 0..<200 { _ = n.feed(rms: 0.02, ms: 100) }   // Dauerrauschen über der Mindestschwelle, ohne Beginn davor
        check(n.threshold > 0.012 || n.inUtterance, "VAD: Schwelle passt sich an oder Sprache erkannt")
        check(n.floor >= 0.004, "VAD: Rauschpegel nicht unter Anfangswert gefallen")
        var z = AssistantVAD()
        _ = z.feed(rms: .nan, ms: 100)
        _ = z.feed(rms: .nan, ms: 100)
        check(!z.inUtterance, "VAD: NaN ist keine Sprache")
    }

    // MARK: Stille-Tor

    static func gate() {
        check(AssistantSilenceGate.reported(silenceMs: 200, lastLoudAt: 5, recognizedUntil: 0) == 200, "Tor: kurze Stille unverändert")
        check(AssistantSilenceGate.reported(silenceMs: 500, lastLoudAt: 5, recognizedUntil: 3) == 300, "Tor: Erkenner hinkt, Stille gehalten")
        check(AssistantSilenceGate.reported(silenceMs: 500, lastLoudAt: 5, recognizedUntil: 4.8) == 500, "Tor: Erkenner eingeholt")
        check(AssistantSilenceGate.reported(silenceMs: 1_299, lastLoudAt: 5, recognizedUntil: 0) == 300, "Tor: kurz vor der Freigabe noch gehalten")
        check(AssistantSilenceGate.reported(silenceMs: 1_300, lastLoudAt: 5, recognizedUntil: 0) == 1_300, "Tor: spätestens nach 1300 ms voll")
    }

    // MARK: Umstellung „Freisprechen an“

    static func migration() {
        let suite = "assistant-logic-test-\(UUID().uuidString)"
        guard let defaults = UserDefaults(suiteName: suite) else { check(false, "Umstellung: Suite"); return }
        defer { defaults.removePersistentDomain(forName: suite) }
        let key = AssistantHandsFreeMigration.key, marker = AssistantHandsFreeMigration.marker

        // Bestandsgerät: Freisprechen war aus, Merker fehlt -> einmal an, Merker gesetzt.
        defaults.set(false, forKey: key)
        check(AssistantHandsFreeMigration.resolve(defaults), "Umstellung: Altwert aus wird einmal auf an gesetzt")
        check(defaults.bool(forKey: marker) && defaults.bool(forKey: key), "Umstellung: Wert und Merker geschrieben")

        // Nutzer schaltet danach aus: bleibt aus (Umstellung läuft nicht noch einmal).
        defaults.set(false, forKey: key)
        check(!AssistantHandsFreeMigration.resolve(defaults), "Umstellung: spätere Wahl aus bleibt")
        defaults.set(true, forKey: key)
        check(AssistantHandsFreeMigration.resolve(defaults), "Umstellung: spätere Wahl an bleibt")

        // Neues Gerät ohne jeden Wert.
        defaults.removeObject(forKey: key); defaults.removeObject(forKey: marker)
        check(AssistantHandsFreeMigration.resolve(defaults), "Umstellung: frisches Gerät startet mit an")

        // Abbruch zwischen Wert und Merker: beim nächsten Start gleiches Ergebnis, nichts geht verloren.
        defaults.removeObject(forKey: marker); defaults.set(true, forKey: key)
        check(AssistantHandsFreeMigration.resolve(defaults) && defaults.bool(forKey: marker), "Umstellung: Abbruch vor dem Merker wird nachgeholt")

        // UI-Test: erzwungen aus, auch wenn der Merker fehlt, und der Merker gilt danach als gesetzt.
        defaults.removeObject(forKey: marker); defaults.set(true, forKey: key)
        check(!AssistantHandsFreeMigration.resolve(defaults, forceOff: true), "Umstellung: --no-handsfree erzwingt aus")
        check(defaults.bool(forKey: marker) && !defaults.bool(forKey: key), "Umstellung: --no-handsfree setzt Merker und Wert")
        check(!AssistantHandsFreeMigration.resolve(defaults), "Umstellung: danach gilt der gespeicherte Wert")
    }
}
