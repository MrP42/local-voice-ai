import Foundation

// Wörter, Stücke und Tokenizer des lexikalischen Index der Ablage (D5a-4, lva#441, Epic #395).
//
// Port der drei Bausteine, die am Desktop über das Ranking entscheiden (Spec §4.3, Übergabe §2.14):
//
// 1. `inhaltsWoerter`: `wai_retrieval::content_tokens` (`vendor/wai-retrieval/src/lib.rs`): am Nicht-Alphanumerischen
//    trennen (Rust `char::is_alphanumeric` = Alphabetic oder Nd/Nl/No), jedes Wort mit `str::to_lowercase`
//    (Sonderfall Schluss-Sigma) klein, Stoppwörter (`STOP`) entfernt. So entsteht die Anfrage; `content_tokens` selbst faltet nicht
//    (die Faltung nach Nr. 4 kommt danach, in `AblageIndex.phrasen`).
// 2. `fts5Woerter`: die Zerlegung des FTS5-Tokenizers `unicode61 remove_diacritics 2` (SQLite), mit der der Desktop Stücke
//    und Anfragewörter vergleicht. Die Zeichentabellen (welches Zeichen ein Wort bildet, was die Faltung daraus macht)
//    stammen aus `docs/superpowers/plans/ablage-index-vektoren.json` (`fts5`), die der Rust-Test
//    `ablage::tests_index_paritaet` aus dem SQLite des Index erzeugt; `AblageFts5Tabellen.swift` wird daraus von
//    `apps/apple/scripts/gen_ablage_fts5_tabellen.py` erzeugt und vom Test gegen die Datei geprüft.
// 3. `stuecke`: `ablage/index.rs::stuecke` (höchstens 220 Zeichen, 40 Überlappung, Schnitt am Leerraum in der zweiten
//    Hälfte, Zeichen = Unicode-Skalare, nicht Bytes; Leerraum = White_Space wie Rust `char::is_whitespace`).
// 4. `ftsNormalisieren` (lva#463): `wai_retrieval::fts_normalisieren`, die Faltung VOR dem Tokenizer für die Zeichen, die
//    `unicode61 remove_diacritics 2` nicht auf das Grundalphabet bringt (ß → ss, æ → ae, ø → o, …). Der Desktop wendet sie beim
//    Indexieren auf den Text jedes Stücks an und bei der Anfrage auf jedes Wort nach `content_tokens`; Anzeigetext, Stück und Zitat
//    bleiben das Original. Die Tabelle steht in `ablage-index-vektoren.json` (`normalisierung.zeichen`, Probe und Vergleich im Test).
//
// Reine Funktionen, kein I/O, kein Zustand. Zeichenketten werden nur über Unicode-Skalare verglichen (nie über Swifts
// kanonische Äquivalenz), weil Rust byteweise vergleicht.

public enum AblageSuchtext {
    // MARK: Stoppwörter (`STOP`, vendor/wai-retrieval/src/lib.rs)

    /// Genau die Liste des Desktops (die Konstante ist dort nicht öffentlich; `ablage-index-vektoren.json` trägt sie
    /// aus dem Quelltext, der Test vergleicht beide).
    public static let stoppwoerter: [String] = [
        "der", "die", "das", "den", "dem", "des", "ein", "eine", "einer", "einen", "und", "oder",
        "ist", "sind", "wie", "was", "wer", "wo", "wann", "im", "in", "am", "an", "auf", "zu", "zum",
        "zur", "von", "vom", "mit", "bei", "fuer", "für", "es", "sie", "er", "wir", "ihr", "nicht",
        "auch", "wird", "werden", "hat", "haben", "nach", "als", "ab",
    ]

    /// Byteweise Menge (Rust vergleicht `&str` byteweise: ein zerlegtes „für“ ist dort kein Stoppwort).
    private static let stopBytes: Set<[UInt8]> = Set(stoppwoerter.map { Array($0.utf8) })

    // MARK: content_tokens

    /// `char::is_alphanumeric`: Alphabetic oder numerisch (Nd, Nl, No).
    static func istRustAlnum(_ s: Unicode.Scalar) -> Bool {
        let v = s.value
        if v < 128 { return (v >= 0x30 && v <= 0x39) || (v >= 0x41 && v <= 0x5A) || (v >= 0x61 && v <= 0x7A) }
        let p = s.properties
        if p.isAlphabetic { return true }
        switch p.generalCategory {
        case .decimalNumber, .letterNumber, .otherNumber: return true
        default: return false
        }
    }

    /// `str::to_lowercase` für ein Wort: Skalar für Skalar die volle Kleinschreibung (`İ` wird zwei Skalare), nur das große
    /// Sigma richtet sich nach seiner Umgebung (am Wortende `ς`, sonst `σ`; Unicode `Final_Sigma`).
    static func rustKlein(_ wort: [Unicode.Scalar]) -> String {
        var aus = String.UnicodeScalarView()
        for (i, s) in wort.enumerated() {
            if s.value == 0x03A3 {
                let davor = ignorierbarDannCased(wort[..<i].reversed())
                let danach = ignorierbarDannCased(wort[(i + 1)...])
                aus.append(davor && !danach ? Unicode.Scalar(0x03C2)! : Unicode.Scalar(0x03C3)!)
            } else if s.value < 128 {
                aus.append(s.value >= 0x41 && s.value <= 0x5A ? Unicode.Scalar(s.value + 32)! : s)
            } else {
                aus.append(contentsOf: s.properties.lowercaseMapping.unicodeScalars)
            }
        }
        return String(aus)
    }

    /// Wie Rusts `case_ignorable_then_cased`: Zeichen mit `Case_Ignorable` überspringen, dann muss ein `Cased` folgen.
    private static func ignorierbarDannCased<S: Sequence>(_ folge: S) -> Bool where S.Element == Unicode.Scalar {
        for s in folge {
            if s.properties.isCaseIgnorable { continue }
            return s.properties.isCased
        }
        return false
    }

    /// `wai_retrieval::content_tokens`: kleingeschriebene Wörter ohne Stoppwörter, Reihenfolge und Doppelte bleiben.
    public static func inhaltsWoerter(_ text: String) -> [String] {
        var aus: [String] = []
        var wort: [Unicode.Scalar] = []
        func abschliessen() {
            guard !wort.isEmpty else { return }
            let klein = rustKlein(wort)
            wort.removeAll(keepingCapacity: true)
            if !stopBytes.contains(Array(klein.utf8)) { aus.append(klein) }
        }
        for s in text.unicodeScalars {
            if istRustAlnum(s) { wort.append(s) } else { abschliessen() }
        }
        abschliessen()
        return aus
    }

    // MARK: Faltung vor FTS5 (lva#463)

    /// Zeichen, die `unicode61 remove_diacritics 2` nicht auf das Grundalphabet faltet, und ihr Ersatz: genau `FTS_FALTUNG` aus
    /// `vendor/wai-retrieval/src/lib.rs` (Reihenfolge wie dort). Der Ersatz ist immer klein geschrieben, auch für Großbuchstaben
    /// (Æ → „ae“, nicht „Ae“); höchstens zwei Skalare. Der Test gleicht sie mit `normalisierung.zeichen` der Fixture ab.
    static let ftsFaltung: [(von: Unicode.Scalar, nach: String)] = [
        ("\u{DF}", "ss"), ("\u{1E9E}", "ss"),   // ß ẞ
        ("\u{E6}", "ae"), ("\u{C6}", "ae"),     // æ Æ
        ("\u{153}", "oe"), ("\u{152}", "oe"),   // œ Œ
        ("\u{F8}", "o"), ("\u{D8}", "o"),       // ø Ø
        ("\u{142}", "l"), ("\u{141}", "l"),     // ł Ł
        ("\u{111}", "d"), ("\u{110}", "d"),     // đ Đ
        ("\u{F0}", "d"), ("\u{D0}", "d"),       // ð Ð
        ("\u{FE}", "th"), ("\u{DE}", "th"),     // þ Þ
        ("\u{127}", "h"), ("\u{126}", "h"),     // ħ Ħ
        ("\u{131}", "i"),                        // ı
        ("\u{133}", "ij"), ("\u{132}", "ij"),   // ĳ Ĳ
    ]

    /// Der Ersatz je Skalar als ASCII-Bytes (alle Ersetzungen sind ASCII).
    private static let ftsErsatz: [UInt32: [UInt8]] = Dictionary(uniqueKeysWithValues: ftsFaltung.map { ($0.von.value, Array($0.nach.utf8)) })

    /// `wai_retrieval::fts_normalisieren`: Skalar für Skalar die Zeichen aus `ftsFaltung` ersetzen, alles andere bleibt (Groß-/
    /// Kleinschreibung und Diakritika faltet der Tokenizer selbst). Codepunktgenau: keine kanonische Äquivalenz, keine Normalisierung
    /// (ein zerlegtes „é“ bleibt zerlegt; hinter einem ß stehende Diakritika bleiben stehen). Reine Funktion. Das Ergebnis ist nie länger
    /// als die Eingabe in UTF-8 (ß → ss, æ → ae, þ → th: gleich lang; ẞ → ss, ø → o: kürzer); es wird in Bytes aufgebaut, weil das
    /// Anhängen an eine `String.UnicodeScalarView` bei langen Texten merklich überlinear wird.
    public static func ftsNormalisieren(_ text: String) -> String {
        var aus: [UInt8] = []
        aus.reserveCapacity(text.utf8.count)
        for s in text.unicodeScalars {
            if s.value < 0x80 {
                aus.append(UInt8(truncatingIfNeeded: s.value))
            } else if let ersatz = ftsErsatz[s.value] {   // alle Zeichen der Tabelle liegen oberhalb von ASCII (wie `is_ascii` am Desktop)
                aus.append(contentsOf: ersatz)
            } else {
                UTF8.encode(s, into: { aus.append($0) })
            }
        }
        return String(decoding: aus, as: UTF8.self)
    }

    // MARK: FTS5 unicode61 remove_diacritics 2

    /// Wörter, wie der Tokenizer des Index sie sieht: Wortzeichen (Buchstaben, Ziffern, Privatgebrauch laut SQLite-Tabelle)
    /// bilden ein Wort, Diakritika setzen es nur fort, jedes Zeichen wird gefaltet (Kleinschreibung, Diakritika weg).
    public static func fts5Woerter(_ text: String) -> [String] {
        var aus: [String] = []
        var wort = String.UnicodeScalarView()
        var imWort = false
        let t = AblageFts5.tabelle
        for s in text.unicodeScalars {
            let c = s.value
            if imWort {
                if t.istWortzeichen(c) || t.istFolgezeichen(c) {
                    if let f = t.gefaltet(c) { wort.append(f) }
                } else {
                    aus.append(String(wort))
                    wort = String.UnicodeScalarView()
                    imWort = false
                }
            } else if t.istWortzeichen(c) {
                imWort = true
                if let f = t.gefaltet(c) { wort.append(f) }
            }
        }
        if imWort { aus.append(String(wort)) }
        return aus
    }

    // MARK: Stücke (`ablage/index.rs::stuecke`)

    /// Stücke eines Abschnitts: höchstens `max` Zeichen, Überlappung `ueber` Zeichen, Schnitt möglichst an Leerraum.
    /// Leere Stücke entfallen. Arbeitet auf Unicode-Skalaren (Rust-`char`).
    public static func stuecke(_ text: String, max: Int = AblageSuchtext.stueckGroesse, ueber: Int = AblageSuchtext.ueberlappung) -> [String] {
        let t = Array(text.unicodeScalars)
        let n = t.count
        func leer(_ s: Unicode.Scalar) -> Bool { s.properties.isWhitespace }
        func getrimmt(_ von: Int, _ bis: Int) -> String {
            var a = von, b = bis
            while a < b, leer(t[a]) { a += 1 }
            while b > a, leer(t[b - 1]) { b -= 1 }
            var v = String.UnicodeScalarView()
            v.append(contentsOf: t[a..<b])
            return String(v)
        }
        var aus: [String] = []
        var start = 0
        while start < n {
            while start < n, leer(t[start]) { start += 1 }
            if start >= n { break }
            var ende = Swift.min(start + max, n)
            if ende < n {
                let halb = Swift.min(start + max / 2, n)
                if let i = (halb..<ende).last(where: { leer(t[$0]) }) { ende = i }
            }
            aus.append(getrimmt(start, ende))
            if ende >= n { break }
            var naechster = Swift.max(Swift.max(ende - ueber, 0), start + 1)
            // Am Wortanfang beginnen: der Skalar davor muss Leerraum sein (am Textanfang gibt es keinen).
            while naechster < ende, !(naechster > 0 && leer(t[naechster - 1])) { naechster += 1 }
            start = naechster
        }
        return aus.filter { !$0.isEmpty }
    }

    /// `index.rs`: `STUECK` und `UEBERLAPPUNG`.
    public static let stueckGroesse = 220
    public static let ueberlappung = 40
}

/// Tabellen des FTS5-Tokenizers (aus `AblageFts5Tabellen.swift`), aufgebaut für schnelle Abfragen.
struct AblageFts5Tabelle: Sendable {
    private let bitmap: [UInt64]
    private let folge: Set<UInt32>
    private let falten: [UInt32: UInt32]
    private let entfernen: Set<UInt32>

    init(wort: [UInt32], folge: [UInt32], falten: [Int32], entfernen: [UInt32]) {
        var bm = [UInt64](repeating: 0, count: 0x110000 / 64)
        func setze(_ z: UInt32) { bm[Int(z >> 6)] |= 1 << UInt64(z & 63) }
        var i = 0
        while i + 1 < wort.count { for z in wort[i]...wort[i + 1] { setze(z) }; i += 2 }
        bitmap = bm
        var f = Set<UInt32>()
        i = 0
        while i + 1 < folge.count { for z in folge[i]...folge[i + 1] { f.insert(z) }; i += 2 }
        self.folge = f
        var e = Set<UInt32>()
        i = 0
        while i + 1 < entfernen.count { for z in entfernen[i]...entfernen[i + 1] { e.insert(z) }; i += 2 }
        self.entfernen = e
        var d: [UInt32: UInt32] = [:]
        i = 0
        while i + 3 < falten.count {
            let start = Int64(falten[i]), anzahl = Int64(falten[i + 1]), schritt = Int64(falten[i + 2]), delta = Int64(falten[i + 3])
            for k in 0..<anzahl {
                let c = start + k * schritt
                d[UInt32(c)] = UInt32(c + delta)
            }
            i += 4
        }
        self.falten = d
    }

    func istWortzeichen(_ c: UInt32) -> Bool {
        c < 0x110000 && bitmap[Int(c >> 6)] & (1 << UInt64(c & 63)) != 0
    }

    func istFolgezeichen(_ c: UInt32) -> Bool { folge.contains(c) }

    /// Das gefaltete Zeichen; `nil`, wenn das Zeichen im Wort verschwindet (Diakritikum).
    func gefaltet(_ c: UInt32) -> Unicode.Scalar? {
        if entfernen.contains(c) { return nil }
        return Unicode.Scalar(falten[c] ?? c)
    }
}

enum AblageFts5 {
    /// Einmal aus den erzeugten Zahlenreihen aufgebaut.
    static let tabelle = AblageFts5Tabelle(wort: AblageFts5Tabellen.wortzeichen, folge: AblageFts5Tabellen.folgezeichen,
                                           falten: AblageFts5Tabellen.falten, entfernen: AblageFts5Tabellen.entfernen)
}
