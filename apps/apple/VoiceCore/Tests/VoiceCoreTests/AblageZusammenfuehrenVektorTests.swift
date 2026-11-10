import XCTest
@testable import VoiceCore
#if canImport(CryptoKit)

// Zusammenführen gegen den Desktop (D5a-2, lva#441, Epic #395): die Gruppe `zusatz.zusammenfuehren` der gemeinsamen
// Fixture `docs/superpowers/plans/ablage-testvektoren.json`. Die Werte erzeugt der Desktop-Test
// `sync::ablage::vektoren_zusatz` aus `zusammenfuehren_datei`, `zusammenfuehren_ordner` und `hash_datei`; hier
// wird dieselbe Eingabe mit dem Swift-Zusammenführen verrechnet und muss Byte für Byte (kanonische Nutzlast,
// Hash, Hub-Kennungen) dasselbe liefern, in beiden Reihenfolgen und idempotent.

private func bytes(_ s: String) -> [UInt8] { Array(s.utf8) }
private func text(_ b: [UInt8]) -> String { String(decoding: b, as: UTF8.self) }

final class AblageZusammenfuehrenVektorTests: XCTestCase {
    private var gruppe: AblageJSON = .null
    private let key: [UInt8] = (0..<32).map { UInt8($0) }
    private typealias Z = AblageZusammenfuehren

    /// Die Fixture wird mit dem eigenen Leser gelesen (wie in AblageRustZusatzTests).
    override func setUpWithError() throws {
        let root = try AblageJSON.parse([UInt8](try Data(contentsOf: AblageVectorTests.fixtureURL())))
        let zusatz = try XCTUnwrap(root["zusatz"], "Schlüssel `zusatz` fehlt in der Fixture")
        gruppe = try XCTUnwrap(zusatz["zusammenfuehren"], "Gruppe `zusammenfuehren` fehlt")
    }

    private func abschnitt(_ name: String) throws -> AblageJSON { try XCTUnwrap(gruppe[name], name) }

    private func str(_ d: AblageJSON, _ k: String) throws -> String {
        guard case .string(let s)? = d[k] else { throw AblageError.feld(k) }
        return s
    }

    private func dat(_ d: AblageJSON, _ k: String) throws -> AblageDatei { try AblageDatei(nutzlast: bytes(try str(d, k))) }
    private func ord(_ d: AblageJSON, _ k: String) throws -> AblageOrdner { try AblageOrdner(nutzlast: bytes(try str(d, k))) }

    private func kanon(_ d: AblageDatei) -> String { text(d.kanonischeNutzlast(ordnen: false)) }
    private func kanon(_ o: AblageOrdner) -> String { text(o.kanonischeNutzlast()) }

    // MARK: Datei

    func testDateiZusammenfuehrenWieDesktop() throws {
        let d = try abschnitt("datei")
        let a = try dat(d, "a"), b = try dat(d, "b"), erwartet = try str(d, "ergebnis"), hash = try str(d, "ergebnis_hash")
        let ab = Z.zusammenfuehrenDatei(a, b), ba = Z.zusammenfuehrenDatei(b, a)
        XCTAssertEqual(kanon(ab), erwartet, "(a, b)")
        XCTAssertEqual(kanon(ba), erwartet, "(b, a)")
        XCTAssertEqual(ab.hash(ordnen: false), hash)
        XCTAssertEqual(ba.hash(ordnen: false), hash)
        XCTAssertEqual(AblageKrypto.hubRefs(datei: ab, key: key), try XCTUnwrap(d["ergebnis_hub_refs"]).stringListe())
        // Das Ergebnis ist schon in der Ablage-Ordnung und nimmt kein blob_refs aus der Eingabe mit.
        XCTAssertEqual(ab.geordnet(), ab)
        XCTAssertEqual(ab.blobRefs, Z.blobRefsAus(ab.fassungen))
        // Idempotent und mit den Eingaben absorbierend.
        let erg = try dat(d, "ergebnis")
        XCTAssertEqual(kanon(Z.zusammenfuehrenDatei(erg, erg)), erwartet, "idempotent")
        XCTAssertEqual(kanon(Z.zusammenfuehrenDatei(erg, a)), erwartet, "Ergebnis + a")
        XCTAssertEqual(kanon(Z.zusammenfuehrenDatei(a, erg)), erwartet, "a + Ergebnis")
        XCTAssertEqual(kanon(Z.zusammenfuehrenDatei(erg, b)), erwartet, "Ergebnis + b")
        XCTAssertEqual(kanon(Z.zusammenfuehrenDatei(b, erg)), erwartet, "b + Ergebnis")
        XCTAssertEqual(kanon(Z.zusammenfuehrenDatei(a, a)), kanon(Z.zusammenfuehrenDatei(a, a)))
        // Die inhaltlichen Zusagen des Falls: entfernte Fassung weg, Texte vereinigt, Zusatzfelder vereinigt.
        XCTAssertEqual(ab.entfernt, ["fas_01HZX3K9M2N4P5Q6R7S8T9V0W3"])
        XCTAssertFalse(ab.fassungen.contains { $0.fas == "fas_01HZX3K9M2N4P5Q6R7S8T9V0W3" })
        XCTAssertEqual(ab.extra.map(\.key), ["nur_a", "nur_b", "zukunft"])
    }

    func testDateiZusammenfuehrenAnDerFixtureAllePermutationen() throws {
        // Dritter Stand dazu (das Ergebnis selbst): jede Reihenfolge und jede Gruppierung liefert dasselbe.
        let d = try abschnitt("datei")
        let a = try dat(d, "a"), b = try dat(d, "b"), erg = try dat(d, "ergebnis")
        let stand = [a, b, erg]
        let erwartet = try str(d, "ergebnis")
        for p in [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] {
            let links = Z.zusammenfuehrenDatei(Z.zusammenfuehrenDatei(stand[p[0]], stand[p[1]]), stand[p[2]])
            let rechts = Z.zusammenfuehrenDatei(stand[p[0]], Z.zusammenfuehrenDatei(stand[p[1]], stand[p[2]]))
            XCTAssertEqual(kanon(links), erwartet, "links \(p)")
            XCTAssertEqual(kanon(rechts), erwartet, "rechts \(p)")
        }
    }

    // MARK: Ordner

    func testOrdnerZusammenfuehrenWieDesktop() throws {
        let o = try abschnitt("ordner")
        let a = try ord(o, "a"), b = try ord(o, "b"), erwartet = try str(o, "ergebnis"), hash = try str(o, "ergebnis_hash")
        let ab = Z.zusammenfuehrenOrdner(a, b), ba = Z.zusammenfuehrenOrdner(b, a)
        XCTAssertEqual(kanon(ab), erwartet, "(a, b)")
        XCTAssertEqual(kanon(ba), erwartet, "(b, a)")
        XCTAssertEqual(ab.hash(), hash)
        XCTAssertEqual(ba.hash(), hash)
        let erg = try ord(o, "ergebnis")
        XCTAssertEqual(kanon(Z.zusammenfuehrenOrdner(erg, erg)), erwartet, "idempotent")
        XCTAssertEqual(kanon(Z.zusammenfuehrenOrdner(erg, a)), erwartet)
        XCTAssertEqual(kanon(Z.zusammenfuehrenOrdner(b, erg)), erwartet)
        // Je Register gewinnt das höhere (zaehler, geraet): name aus a (Gleichstand 2, g…W1: Wert "Verträge alt" > "Verträge"),
        // datenklasse und eltern aus b, nur_geraet aus b (a hat denselben Zähler, aber b … Rang aus der Fixture).
        XCTAssertEqual(ab.zielordnerKi.zaehler, 1)
        XCTAssertEqual(ab.extra.map(\.key), ["neu", "zukunft"])
        // Als Bestand: Vereinigung nach Kennung, gleiche Kennung per Register.
        let bestandA = AblageOrdnerbestand([(a.id, a)]), bestandB = AblageOrdnerbestand([(b.id, b)])
        let bestand = Z.zusammenfuehrenOrdnerbestand(bestandA, bestandB)
        XCTAssertEqual(bestand.count, 1)
        XCTAssertEqual(bestand[a.id].map(kanon), erwartet)
        XCTAssertEqual(Z.zusammenfuehrenOrdnerbestand(bestandB, bestandA)[a.id].map(kanon), erwartet)
    }

    // MARK: Unsortiert und gleiche Kennung

    func testUnsortierteDateiErgibtDieSortierteNutzlast() throws {
        let u = try abschnitt("unsortiert")
        let d = try dat(u, "serde_roh")
        // `zusammenfuehren_datei(d, d)` ordnet die Listen (Desktop: `sortiert_*`) ...
        let m = Z.zusammenfuehrenDatei(d, d)
        XCTAssertEqual(kanon(m), try str(u, "sortiert_kanonisch"))
        XCTAssertEqual(m.hash(ordnen: false), try str(u, "sortiert_hash"))
        // ... und stimmt mit der Ordnung von `geordnet()` überein, die D5a-1 schon geprüft hat.
        XCTAssertEqual(text(d.kanonischeNutzlast(ordnen: true)), kanon(m))
        // Die Eingabe selbst bleibt unsortiert (der Hash ohne Zusammenführen ist ein anderer).
        XCTAssertEqual(d.hash(ordnen: false), try str(u, "treu_hash"))
        XCTAssertNotEqual(d.hash(ordnen: false), m.hash(ordnen: false))
        XCTAssertEqual(AblageKrypto.hubRefs(datei: m, key: key), try XCTUnwrap(u["hub_refs"]).stringListe())
        // Auch gegen eine umgekehrte Eingabe: dieselbe Nutzlast.
        var umgekehrt = d
        umgekehrt.fassungen.reverse(); umgekehrt.wahlen.reverse(); umgekehrt.entfernt.reverse()
        XCTAssertEqual(kanon(Z.zusammenfuehrenDatei(umgekehrt, d)), kanon(m))
        XCTAssertEqual(kanon(Z.zusammenfuehrenDatei(d, umgekehrt)), kanon(m))
        // Fixpunkt.
        XCTAssertEqual(kanon(Z.zusammenfuehrenDatei(m, m)), kanon(m))
        // Zahlen über 2^53 und `null` überleben das Zusammenführen.
        let k = kanon(m)
        XCTAssertTrue(k.contains("\"size\":18446744073709551615") && k.contains("\"zaehler\":9007199254740993") && k.contains("\"vorgaenger\":null"))
    }

    func testGleicheKennungWirdVerdichtetWieDesktop() throws {
        let g = try abschnitt("gleiche_kennung")
        let d = try dat(g, "serde_roh")
        XCTAssertEqual(d.fassungen.count, 2)
        XCTAssertEqual(d.wahlen.count, 3)
        let erwartet = try str(g, "verdichtet_kanonisch")
        let m = Z.zusammenfuehrenDatei(d, d)
        XCTAssertEqual(kanon(m), erwartet)
        XCTAssertEqual(m.hash(ordnen: false), try str(g, "verdichtet_hash"))
        XCTAssertEqual(m.fassungen.count, 1)
        XCTAssertEqual(m.wahlen.count, 1)
        // Vorher ohne Zusammenführen nichts verdichtet (`treu_*`).
        XCTAssertEqual(kanon(d), try str(g, "treu_kanonisch"))
        XCTAssertEqual(d.hash(ordnen: false), try str(g, "treu_hash"))
        // Die Verdichtung gewinnt das Größere nach der abgeleiteten Ordnung, unabhängig von der Reihenfolge der Einträge.
        var umgekehrt = d
        umgekehrt.fassungen.reverse(); umgekehrt.wahlen.reverse()
        for (x, y) in [(d, umgekehrt), (umgekehrt, d), (umgekehrt, umgekehrt)] {
            XCTAssertEqual(kanon(Z.zusammenfuehrenDatei(x, y)), erwartet)
        }
        // Idempotent.
        XCTAssertEqual(kanon(Z.zusammenfuehrenDatei(m, m)), erwartet)
        XCTAssertEqual(kanon(Z.zusammenfuehrenDatei(m, d)), erwartet)
    }

    func testDieFixtureLiefertAlleVierFaelle() throws {
        guard case .object(let m) = gruppe else { return XCTFail("Gruppe ist kein Objekt") }
        XCTAssertEqual(m.map(\.key), ["datei", "gleiche_kennung", "ordner", "unsortiert"])
    }
}

extension AblageJSON {
    /// Liste von Zeichenketten (Test-Hilfe).
    fileprivate func stringListe() throws -> [String] { try [String](ablageJSON: self) }
}
#endif
