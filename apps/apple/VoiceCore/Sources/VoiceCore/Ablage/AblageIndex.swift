import Foundation
#if canImport(CryptoKit)

// Lexikalischer Index der Dokumentenablage auf dem iPhone (D5a-4, lva#441, Epic #395, Spec §4.3, §9 D5).
//
// Eigener BM25 in VoiceCore, kein FTS5 des System-SQLite (Entscheidung zu Übergabe §2.14, Frage 3: nicht auf jedem Ziel
// vorhanden, und die Tokenisierung muss bitgleich dem Desktop folgen). Parität zum Desktop (`ablage/index.rs` über das
// gevendorte `wai-retrieval`):
//
// - Stücke je Abschnitt: `AblageSuchtext.stuecke` (220 Zeichen, 40 Überlappung). Jedes Stück ist beim Desktop genau ein Chunk
//   (`chunk_page` fügt die Sätze eines höchstens 220 Zeichen langen Stücks zu einem Chunk zusammen); `seite` zählt die Stücke
//   einer Datei ab 1 über alle Abschnitte wie `ablage_stueck.seite`.
// - Faltung (lva#463): Der FTS-Text jedes Stücks UND jedes Anfragewort durchlaufen `AblageSuchtext.ftsNormalisieren` (ß/ẞ → ss, æ → ae,
//   œ → oe, ø → o, ł → l, đ/ð → d, þ → th, ħ → h, ı → i, ĳ → ij), bevor der Tokenizer sie zerlegt; so findet „Strasse“ „Straße“. Anzeigetext,
//   Stück und Auszug bleiben das Original. Anfrage: `content_tokens` (klein, Stoppwörter), dann Faltung, dann Tokenizer (wie `rank` am Desktop).
//   Kein persistierter Index: Dieser Index und die Vorbereitung (`AblageSuchdienst.vorbereitet`) liegen nur im Speicher und entstehen
//   bei jedem Start aus den Textfassungen neu, ein Altbestand mit ungefaltetem FTS-Text kann also nie existieren. Die einmalige Umstellung des
//   Desktops (`meta.fts_fmt`, `fts_umstellen`) entfällt hier deshalb; wer je etwas davon auf die Platte legt, braucht eine Fassungsmarke und
//   eine atomare Umstellung (Test `testIndexUndVorbereitungSindNurImSpeicher` erinnert daran).
// - Ranking wie `bm25(chunks_fts)` von FTS5: je Wort der Anfrage (`content_tokens`, auf 200 Zeichen gekürzt, Doppelte bleiben)
//   eine Phrase aus den Wörtern des Tokenizers (`unicode61 remove_diacritics 2`), Oder-Verknüpfung. Punkte nach der FTS5-Formel
//   `idf · f·(k1+1) / (f + k1·(1 − b + b·D/avgdl))` mit k1 = 1,2, b = 0,75, `idf = ln((N − n + 0,5)/(n + 0,5))` (≤ 0 wird 1e-6),
//   N = alle Stücke des Index (= die lesbaren, siehe unten), n = Stücke mit der Phrase, D = Wörter des Stücks, avgdl = Wörter aller Stücke / N. Sortierung: bm25
//   aufsteigend (also Punkte absteigend), dann Stück-Kennung; höchstens 3000 Zeilen (`LIMIT`), höchstens 3 Stellen je Datei,
//   höchstens 60 Kandidaten. Stück-Kennung = Reihenfolge im Index (Dateien nach Kennung (Bytes), Stücke nach `seite`).
//   Gleichstand im Rang (gleicher Rohrang): der Desktop bricht ihn nach `c.id` auf, der Reihenfolge, in der sein Index die Stücke
//   angelegt hat (er führt den Index nach: neue und geänderte Dateien kommen später, nicht nach Kennung geordnet); Swift baut den
//   Index jedes Mal aus dem Zustand neu und bricht nach Dateikennung (Bytes), dann `seite` auf. Reihenfolge und Rohrang sind also
//   nur OHNE Gleichstand gleich (der Normalfall bei echten Texten); bei gleichem Rohrang kann die Reihenfolge dieser Treffer vom
//   Desktop abweichen (die Vektoren in `ablage-index-vektoren.json` prüfen den Fall ohne Gleichstand und die Rohränge).
// - Gruppierung wie `index::suchen_mit`: je Datei die besten Stellen (höchstens 3, nie zwei mit gleichem Anker), Dateien in
//   der Reihenfolge ihrer besten Stelle.
//
// Abweichung, die bleibt: N, n und avgdl zählen hier nur die Stücke, die der Index-Tor zulässt. Der Desktop zählt seit lva#463 ebenfalls
// nur Stücke, die der Leser des Aufrufs lesen darf (Papierkorb und gesperrte Dateien zählen nie; Fixture `gesperrt`: gleicher Rohrang wie
// ohne sie, Test `testGesperrterBestandGleichRohrang`). Ein Unterschied der Rangzahlen bleibt daher nur bei Dateien, die nur der Desktop im
// Index hat: lebende Dateien, die der Tor hier ausschließt (nur Gerät, `personal`/`confidential`, `Halten`, angehalten) oder die der Hub nie
// liefert; der Desktop liest sie für seine Oberfläche mit. Bei gleichem Bestand sind Reihenfolge und Rohrang gleich (Test gegen
// `ablage-index-vektoren.json`).
//
// Der Index ist nach dem Aufbau unveränderlich (`Sendable`). Kein Logging; Namen und Inhalte verlassen den Prozess nie.

public struct AblageStueck: Equatable, Sendable {
    public let dat: String
    /// Nummer des Stücks in der Datei, ab 1 (wie `ablage_stueck.seite`).
    public let seite: Int
    /// Index des Abschnitts in der Textfassung.
    public let abschnitt: Int
    public let anker: String
    public let pfad: [String]
    public let text: String
}

/// Eine Stelle aus dem Ranking: das Stück samt Rohrang.
public struct AblageStelle: Equatable, Sendable {
    public let dat: String
    public let seite: Int
    public let anker: String
    public let pfad: [String]
    public let text: String
    /// `bm25(chunks_fts)` von FTS5 (negativ, kleiner = besser); nur zum Vergleichen mit dem Desktop.
    public let bm25: Double
    /// Rang 0…1 wie `AblageSuchtreffer.rang` am Desktop (aus der Platzziffer, nicht aus dem Rohrang).
    public let rang: Double
}

public struct AblageFundstelle: Equatable, Sendable {
    public let anker: String
    public let pfad: [String]
    public let auszug: String
}

/// Treffer in einer Datei, beste Datei zuerst.
public struct AblageTreffer: Equatable, Sendable {
    public let dat: String
    public let rang: Double
    public let stellen: [AblageFundstelle]
}

public final class AblageIndex: Sendable {
    public static let kandidaten = 60
    public static let stellenJeDatei = 3
    public static let anfrageMax = 200
    /// `LIMIT` der lexikalischen Abfrage: `max(k, 20) · 50` Zeilen vor der Kappung je Datei.
    static let sqlZeilen = 60 * 50
    static let k1 = 1.2
    static let b = 0.75
    /// `RRF_K` und `RRF_MAX` aus `wai-retrieval` (Rang aus der Platzziffer).
    private static let rrfK = 60.0
    private static let rrfMax = 2.0 / (61.0)

    // MARK: Vorbereitung (je Textfassung, lässt sich zwischenspeichern)

    /// Ein Stück mit seinen Wörtern, als Nummern in `Vorbereitet.begriffe` (nicht als Zeichenketten: der Zwischenspeicher hält die
    /// Stücke aller Dateien dauerhaft, und ein Wort kommt in einer Datei viele Male vor).
    public struct VorbereitetesStueck: Sendable {
        let abschnitt: Int
        let anker: String
        let pfad: [String]
        let text: String
        let woerter: [Int32]
    }

    /// Die Stücke einer Textfassung samt Wörtern. Hängt nur von der Textfassung ab (Kennung, `textrev`), nicht vom Rest des Index.
    /// Die Wörter der Stücke sind Nummern in `begriffe` (die verschiedenen Wörter der Datei, in der Reihenfolge ihres ersten
    /// Vorkommens); der Index bildet sie beim Aufbau auf seine eigenen Nummern ab.
    public struct Vorbereitet: Sendable {
        public let dat: String
        let stuecke: [VorbereitetesStueck]
        let begriffe: [String]
        public var anzahlStuecke: Int { stuecke.count }
    }

    public static func vorbereiten(dat: String, abschnitte: [AblageAbschnitt]) -> Vorbereitet {
        var aus: [VorbereitetesStueck] = []
        var nummern: [String: Int32] = [:]
        var begriffe: [String] = []
        for (i, ab) in abschnitte.enumerated() {
            for s in AblageSuchtext.stuecke(ab.text) {
                // lva#463: der FTS-Text ist gefaltet (ß → ss, …); `text` (Anzeige, Auszug) bleibt das Original.
                let woerter = AblageSuchtext.fts5Woerter(AblageSuchtext.ftsNormalisieren(s))
                var ids: [Int32] = []
                ids.reserveCapacity(woerter.count)
                for w in woerter {
                    if let id = nummern[w] { ids.append(id) } else {
                        // Höchstens so viele verschiedene Wörter, wie der Text Zeichen hat (Textfassung höchstens 32 MiB): passt in Int32.
                        let id = Int32(begriffe.count)
                        nummern[w] = id
                        begriffe.append(w)
                        ids.append(id)
                    }
                }
                aus.append(VorbereitetesStueck(abschnitt: i, anker: ab.anker, pfad: ab.pfad, text: s, woerter: ids))
            }
        }
        return Vorbereitet(dat: dat, stuecke: aus, begriffe: begriffe)
    }

    // MARK: Aufbau

    private struct Chunk: Sendable {
        let dok: Int32
        let seite: Int32
        let abschnitt: Int32
        let anker: String
        let pfad: [String]
        let text: String
        /// Wörter als Kennungen der Begriffe, in Reihenfolge (für Phrasen und die Länge).
        let woerter: [Int32]
    }

    private struct Posting: Sendable {
        let chunk: Int32
        let tf: Int32
    }

    private let dateien: [String]
    private let chunks: [Chunk]
    private let begriffe: [String: Int32]
    private let postings: [[Posting]]
    private let gesamtWoerter: Int

    /// Baut den Index. Die Dateien werden nach Kennung (Bytes) geordnet (wie der Desktop sie bei einem Neuaufbau aus seinem
    /// Bestand liest); die Reihenfolge entscheidet nur bei Gleichstand im Rang.
    public init(_ dokumente: [Vorbereitet]) {
        let geordnet = AblageBytes.sorted(dokumente, by: \.dat)
        var dateien: [String] = []
        var chunks: [Chunk] = []
        var begriffe: [String: Int32] = [:]
        var woerterGesamt = 0
        for (d, dok) in geordnet.enumerated() {
            dateien.append(dok.dat)
            // Nummern der Datei auf Nummern des Index abbilden (ein Nachschlagen je verschiedenem Wort der Datei, nicht je Vorkommen).
            var abbildung: [Int32] = []
            abbildung.reserveCapacity(dok.begriffe.count)
            for w in dok.begriffe {
                if let id = begriffe[w] { abbildung.append(id) } else {
                    let id = Int32(begriffe.count)
                    begriffe[w] = id
                    abbildung.append(id)
                }
            }
            var seite = 0
            for st in dok.stuecke {
                seite += 1
                let ids = st.woerter.map { abbildung[Int($0)] }
                woerterGesamt += ids.count
                chunks.append(Chunk(dok: Int32(d), seite: Int32(seite), abschnitt: Int32(st.abschnitt), anker: st.anker, pfad: st.pfad,
                                    text: st.text, woerter: ids))
            }
        }
        var postings = [[Posting]](repeating: [], count: begriffe.count)
        for (i, c) in chunks.enumerated() {
            var tf: [Int32: Int32] = [:]
            for w in c.woerter { tf[w, default: 0] += 1 }
            for (w, n) in tf { postings[Int(w)].append(Posting(chunk: Int32(i), tf: n)) }
        }
        // Je Begriff nach Stück geordnet (für Phrasen und gleichmäßige Summen).
        for i in postings.indices { postings[i].sort { $0.chunk < $1.chunk } }
        self.dateien = dateien
        self.chunks = chunks
        self.begriffe = begriffe
        self.postings = postings
        self.gesamtWoerter = woerterGesamt
    }

    // MARK: Zahlen

    public var anzahlDateien: Int { dateien.count }
    public var anzahlStuecke: Int { chunks.count }
    /// Summe der Wörter aller Stücke (Zähler von `avgdl`).
    public var anzahlWoerter: Int { gesamtWoerter }

    /// Die Stücke einer Datei in der Reihenfolge ihrer Nummer (zum Prüfen gegen den Desktop).
    public func stuecke(dat: String) -> [AblageStueck] {
        guard let d = dateien.firstIndex(where: { AblageBytes.equal($0, dat) }) else { return [] }
        return chunks.filter { Int($0.dok) == d }.map { stueck($0) }
    }

    /// Länge (Wörter) des Stücks `seite` der Datei, wie `chunks_fts_docsize`.
    public func laenge(dat: String, seite: Int) -> Int? {
        guard let d = dateien.firstIndex(where: { AblageBytes.equal($0, dat) }) else { return nil }
        return chunks.first { Int($0.dok) == d && Int($0.seite) == seite }?.woerter.count
    }

    private func stueck(_ c: Chunk) -> AblageStueck {
        return AblageStueck(dat: dateien[Int(c.dok)], seite: Int(c.seite), abschnitt: Int(c.abschnitt), anker: c.anker, pfad: c.pfad, text: c.text)
    }

    // MARK: Anfrage

    /// Die Wörter der Anfrage wie der Desktop: auf 200 Zeichen (Unicode-Skalare) gekürzt, dann `content_tokens`.
    public static func anfrageWoerter(_ frage: String) -> [String] {
        var v = String.UnicodeScalarView()
        v.append(contentsOf: frage.unicodeScalars.prefix(anfrageMax))
        return AblageSuchtext.inhaltsWoerter(String(v))
    }

    /// Eine Phrase der Anfrage: die Wörter, in die der Tokenizer ein Anfragewort zerlegt (meist eins). Reihenfolge wie `wai-retrieval`
    /// (`rank`, `phrasen_zerlegen`): `anfrageWoerter` (`content_tokens`: Kleinschreibung, Stoppwörter), dann die Faltung (lva#463:
    /// „Strasse“ trifft „Straße“), dann der Tokenizer.
    private func phrasen(_ woerter: [String]) -> [[String]] {
        woerter.map { AblageSuchtext.fts5Woerter(AblageSuchtext.ftsNormalisieren($0)) }
    }

    /// Alle Zeilen der lexikalischen Abfrage, beste zuerst (bm25 aufsteigend, dann Stück-Kennung), höchstens `sqlZeilen`.
    /// Entspricht `ORDER BY bm25(chunks_fts), c.id LIMIT 3000`.
    func zeilen(_ frage: String) -> [(chunk: Int, bm25: Double)] {
        let n = chunks.count
        guard n > 0 else { return [] }
        let ph = phrasen(AblageIndex.anfrageWoerter(frage))
        let avgdl = Double(gesamtWoerter) / Double(n)
        var punkte: [Int32: Double] = [:]
        for phrase in ph {
            guard !phrase.isEmpty else { continue }
            let treffer = phrasentreffer(phrase)
            guard !treffer.isEmpty else { continue }
            let nHit = Double(treffer.count)
            var idf = log((Double(n) - nHit + 0.5) / (nHit + 0.5))
            if idf <= 0.0 { idf = 1e-6 }
            for (chunk, f) in treffer {
                let d = Double(chunks[Int(chunk)].woerter.count)
                punkte[chunk, default: 0.0] += idf * ((f * (AblageIndex.k1 + 1.0)) / (f + AblageIndex.k1 * (1 - AblageIndex.b + AblageIndex.b * d / avgdl)))
            }
        }
        var zeilen = punkte.map { (chunk: Int($0.key), bm25: -1.0 * $0.value) }
        zeilen.sort { $0.bm25 != $1.bm25 ? $0.bm25 < $1.bm25 : $0.chunk < $1.chunk }
        return zeilen.count > AblageIndex.sqlZeilen ? Array(zeilen.prefix(AblageIndex.sqlZeilen)) : zeilen
    }

    /// Stücke, die die Phrase enthalten, mit der Zahl ihrer Vorkommen (`f`).
    private func phrasentreffer(_ phrase: [String]) -> [(Int32, Double)] {
        var ids: [Int32] = []
        for w in phrase {
            guard let id = begriffe[w] else { return [] }
            ids.append(id)
        }
        if ids.count == 1 { return postings[Int(ids[0])].map { ($0.chunk, Double($0.tf)) } }
        // Mehrere Wörter: Stücke aus dem seltensten Wort, dann die Folge in der Reihenfolge der Wörter zählen.
        let seltenstes = ids.min { postings[Int($0)].count < postings[Int($1)].count }!
        var aus: [(Int32, Double)] = []
        for p in postings[Int(seltenstes)] {
            let w = chunks[Int(p.chunk)].woerter
            var f = 0
            if w.count >= ids.count {
                for i in 0...(w.count - ids.count) where w[i] == ids[0] {
                    var passt = true
                    for k in 1..<ids.count where w[i + k] != ids[k] { passt = false; break }
                    if passt { f += 1 }
                }
            }
            if f > 0 { aus.append((p.chunk, Double(f))) }
        }
        return aus
    }

    /// Die Zeilen der Abfrage mit Datei und Nummer des Stücks (zum Prüfen gegen den Desktop).
    func rohzeilen(_ frage: String) -> [(dat: String, seite: Int, bm25: Double)] {
        zeilen(frage).map { (dateien[Int(chunks[$0.chunk].dok)], Int(chunks[$0.chunk].seite), $0.bm25) }
    }

    /// Die Kandidaten nach Kappung je Datei und Kandidatenzahl (die Liste, die `wai-retrieval::rank` bewertet), beste zuerst.
    public func kandidatenstellen(_ frage: String) -> [AblageStelle] {
        var je: [Int32: Int] = [:]
        var aus: [AblageStelle] = []
        for z in zeilen(frage) {
            let c = chunks[z.chunk]
            let n = je[c.dok, default: 0]
            if n >= AblageIndex.stellenJeDatei { continue }
            je[c.dok] = n + 1
            if aus.count >= AblageIndex.kandidaten { break }
            let s = stueck(c)
            // Rang wie `rank`: RRF-Platz `1 / (K + 1 + r)`, durch `RRF_MAX` auf 0…1.
            let rang = min((1.0 / (AblageIndex.rrfK + 1.0 + Double(aus.count))) / AblageIndex.rrfMax, 1.0)
            aus.append(AblageStelle(dat: s.dat, seite: s.seite, anker: s.anker, pfad: s.pfad, text: s.text, bm25: z.bm25, rang: rang))
        }
        return aus
    }

    /// Volltextsuche: Treffer je Datei mit höchstens 3 Fundstellen, beste Datei zuerst (`index::suchen_mit`). Eine Frage nur
    /// aus Stoppwörtern oder Satzzeichen liefert nichts.
    public func suche(_ frage: String) -> [AblageTreffer] {
        var je: [String: (rang: Double, stellen: [AblageFundstelle])] = [:]
        var reihenfolge: [String] = []
        for s in kandidatenstellen(frage) {
            if je[s.dat] == nil { je[s.dat] = (s.rang, []); reihenfolge.append(s.dat) }
            guard var t = je[s.dat], t.stellen.count < AblageIndex.stellenJeDatei,
                  !t.stellen.contains(where: { AblageBytes.equal($0.anker, s.anker) }) else { continue }
            t.stellen.append(AblageFundstelle(anker: s.anker, pfad: s.pfad, auszug: s.text))
            je[s.dat] = t
        }
        return reihenfolge.compactMap { d in je[d].map { AblageTreffer(dat: d, rang: $0.rang, stellen: $0.stellen) } }
    }
}
#endif
