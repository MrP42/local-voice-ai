import XCTest
@testable import VoiceCore
#if canImport(CryptoKit)
import CryptoKit

// Dateispeicher der Ablage (D5a-3, lva#441): Zustand atomar, Textfassung einlegen mit den drei Prüfungen und der
// Neuberechnung der `textrev`, Original einlegen, Vorhandensein aus dem Dateisystem, Aufräumen. Jeder Test arbeitet
// in einem eigenen temporären Ordner, nie in produktiven Daten. Alles synthetisch.

private typealias W = AblageWelt

final class AblageSpeicherTests: XCTestCase {
    private var tmp: URL!
    private var speicher: AblageSpeicher!
    private let fm = FileManager.default

    override func setUpWithError() throws {
        tmp = fm.temporaryDirectory.appendingPathComponent("ablage-speicher-\(UUID().uuidString)", isDirectory: true)
        try fm.createDirectory(at: tmp, withIntermediateDirectories: true)
        speicher = AblageSpeicher(wurzel: tmp.appendingPathComponent("ablage", isDirectory: true))
    }

    override func tearDownWithError() throws {
        // Schreibschutz aus Tests wieder aufheben, sonst bleibt der Ordner liegen.
        if let e = fm.enumerator(at: tmp, includingPropertiesForKeys: nil) {
            for case let u as URL in e { try? fm.setAttributes([.posixPermissions: 0o755], ofItemAtPath: u.path) }
        }
        try? fm.setAttributes([.posixPermissions: 0o755], ofItemAtPath: tmp.path)
        try? fm.removeItem(at: tmp)
    }

    private func list(_ rel: String) -> [String] {
        ((try? fm.contentsOfDirectory(atPath: speicher.wurzel.appendingPathComponent(rel).path)) ?? []).sorted()
    }

    private func bestand(_ d: AblageDatei) -> AblageBestand {
        var b = AblageBestand()
        b.dateien[d.id] = d
        return b
    }

    /// Datei mit einem Original (`Original 1`) und einer Textfassung.
    private func beispiel() -> (datei: AblageDatei, eintrag: AblageText, textBytes: [UInt8], original: [UInt8]) {
        var f = W.fassung(1, nil, inhalt: "Original 1")
        let t = W.text(zu: f)
        f.texte = [t.eintrag]
        return (W.datei(1, ordner: W.ordId(1), fassungen: [f]), t.eintrag, t.bytes, Array("Original 1".utf8))
    }

    // MARK: Zustand

    func testZustandRundlaufAtomarOhneTempRest() throws {
        let konto = W.konto.kennung
        var z = AblageZustand(konto: konto)
        z.ledger.cursor["ablage_datei"] = 42
        z.bestand.ordner[W.ordId(1)] = W.ordner(1, name: "Verträge")
        try speicher.speichern(z)
        XCTAssertEqual(list(""), ["ablage-zustand.json"], "keine temporären Reste neben dem Zustand")
        let r = speicher.laden(konto: konto)
        XCTAssertEqual(r.zustand, z)
        XCTAssertEqual(r.verlust, .keiner)
        // Zweites Speichern ersetzt, kein Anhäufen.
        z.ledger.cursor["ablage_datei"] = 43
        try speicher.speichern(z)
        XCTAssertEqual(speicher.laden(konto: konto).zustand.ledger.cursor["ablage_datei"], 43)
        XCTAssertEqual(list(""), ["ablage-zustand.json"])
    }

    func testLadenFehltUnlesbarNeuerAnderesKonto() throws {
        let konto = W.konto.kennung
        XCTAssertEqual(speicher.laden(konto: konto).verlust, .fehlt)
        XCTAssertEqual(speicher.laden(konto: konto).zustand, AblageZustand(konto: konto))
        try fm.createDirectory(at: speicher.wurzel, withIntermediateDirectories: true)
        let datei = speicher.wurzel.appendingPathComponent("ablage-zustand.json")
        // Unlesbar: leerer Zustand, die kaputte Datei bleibt als .defekt erhalten (nichts geht verloren).
        try Data("kein json".utf8).write(to: datei)
        XCTAssertEqual(speicher.laden(konto: konto).verlust, .unlesbar)
        XCTAssertEqual(try String(contentsOf: speicher.wurzel.appendingPathComponent("ablage-zustand.json.defekt"), encoding: .utf8), "kein json")
        // Neuere Version: ebenso beiseitegelegt.
        try Data(#"{"v":2,"konto":"x"}"#.utf8).write(to: datei)
        XCTAssertEqual(speicher.laden(konto: konto).verlust, .neuereVersion)
        XCTAssertEqual(try String(contentsOf: speicher.wurzel.appendingPathComponent("ablage-zustand.json.defekt"), encoding: .utf8), #"{"v":2,"konto":"x"}"#)
        // Anderes Konto: leer, ohne Kopie (die Daten des alten Kontos sollen nicht liegen bleiben).
        try? fm.removeItem(at: speicher.wurzel.appendingPathComponent("ablage-zustand.json.defekt"))
        var fremd = AblageZustand(konto: "8:abcd:https://anderer.example.test")
        fremd.bestand.ordner[W.ordId(1)] = W.ordner(1, name: "FremdesKonto")
        try speicher.speichern(fremd)
        let r = speicher.laden(konto: konto)
        XCTAssertEqual(r.verlust, .anderesKonto)
        XCTAssertEqual(r.zustand, AblageZustand(konto: konto))
        XCTAssertFalse(fm.fileExists(atPath: speicher.wurzel.appendingPathComponent("ablage-zustand.json.defekt").path))
    }

    func testSpeichernFehlerLaesstAltenZustandIntakt() throws {
        let konto = W.konto.kennung
        var z = AblageZustand(konto: konto)
        z.ledger.cursor["ablage_datei"] = 1
        try speicher.speichern(z)
        // Nicht beschreibbarer Ordner (wie voller Datenträger oder Schreibschutz): Fehler, alter Stand bleibt.
        try fm.setAttributes([.posixPermissions: 0o500], ofItemAtPath: speicher.wurzel.path)
        defer { try? fm.setAttributes([.posixPermissions: 0o755], ofItemAtPath: speicher.wurzel.path) }
        if getuid() == 0 { throw XCTSkip("als root nicht prüfbar") }
        z.ledger.cursor["ablage_datei"] = 2
        XCTAssertThrowsError(try speicher.speichern(z)) { XCTAssertEqual($0 as? AblageSpeicherFehler, .schreiben) }
        try fm.setAttributes([.posixPermissions: 0o755], ofItemAtPath: speicher.wurzel.path)
        XCTAssertEqual(speicher.laden(konto: konto).zustand.ledger.cursor["ablage_datei"], 1, "alter Zustand intakt")
        XCTAssertEqual(list(""), ["ablage-zustand.json"], "kein halbes Temp-Ergebnis")
    }

    // MARK: Textfassung einlegen

    func testTextEinlegenMitAllenPruefungen() throws {
        let b = beispiel()
        let bs = bestand(b.datei)
        XCTAssertTrue(try speicher.textEinlegen(bestand: bs, dat: b.datei.id, fas: W.fasId(1), textrev: b.eintrag.textrev, bytes: b.textBytes))
        XCTAssertEqual(speicher.textLesen(dat: b.datei.id, textrev: b.eintrag.textrev), b.textBytes, "Bytes bleiben unverändert")
        XCTAssertTrue(speicher.vorhanden(bs).text(b.datei.id, b.eintrag.textrev))
        XCTAssertEqual(list("text/\(b.datei.id)"), ["\(b.eintrag.textrev).json"])
        // Erneut einlegen ersetzt atomar.
        XCTAssertTrue(try speicher.textEinlegen(bestand: bs, dat: b.datei.id, fas: W.fasId(1), textrev: b.eintrag.textrev, bytes: b.textBytes))
    }

    func testTextEinlegenVerwirftAlleAbweichungenUndSchreibtNichts() throws {
        let b = beispiel()
        let bs = bestand(b.datei)
        let f = b.datei.fassungen[0]
        func einlegen(_ bytes: [UInt8], _ bestand: AblageBestand? = nil, _ rev: String? = nil) throws -> Bool {
            try speicher.textEinlegen(bestand: bestand ?? bs, dat: b.datei.id, fas: W.fasId(1), textrev: rev ?? b.eintrag.textrev, bytes: bytes)
        }
        func abgelehnt(_ name: String, _ bytes: [UInt8], _ bestand: AblageBestand? = nil, file: StaticString = #filePath, line: UInt = #line) {
            XCTAssertThrowsError(try einlegen(bytes, bestand), name, file: file, line: line) {
                XCTAssertEqual($0 as? AblageSpeicherFehler, .passtNicht, name, file: file, line: line)
            }
            XCTAssertNil(speicher.textLesen(dat: b.datei.id, textrev: b.eintrag.textrev), "\(name): nichts abgelegt", file: file, line: line)
        }
        // 1. SHA-256 der Bytes passt nicht zu `Text.blob` (ein Byte verändert, Längen gleich).
        var kaputt = b.textBytes
        kaputt[kaputt.count - 3] ^= 0x01
        abgelehnt("Hash der Bytes", kaputt)
        // 2. `textrev` im Kopf ist nicht die des Eintrags (Blob des Eintrags passend zu den fremden Bytes gesetzt).
        let andereRev = W.textBytes(textrev: String(repeating: "a", count: 64), original: f.originalSha256, extraktor: "lva-text", version: "1",
                                    abschnitte: [("zeile=1", "x")])
        var mitFremderRev = b.datei
        mitFremderRev.fassungen[0].texte[0].blob = AblageKrypto.sha256Hex(andereRev)
        abgelehnt("textrev im Kopf", andereRev, bestand(mitFremderRev))
        // 3. `original_sha256` im Kopf ist nicht die der Fassung.
        let anderesOriginal = W.textBytes(textrev: b.eintrag.textrev, original: String(repeating: "b", count: 64), extraktor: "lva-text", version: "1",
                                          abschnitte: [("zeile=1", "x")])
        var mitFremdemOriginal = b.datei
        mitFremdemOriginal.fassungen[0].texte[0].blob = AblageKrypto.sha256Hex(anderesOriginal)
        abgelehnt("original_sha256 im Kopf", anderesOriginal, bestand(mitFremdemOriginal))
        // 4. Zusätzlich: `textrev` des Eintrags stimmt nicht mit der Neuberechnung aus den Feldern des Eintrags überein.
        let fremdeRev = String(repeating: "c", count: 64)
        let bytes4 = W.textBytes(textrev: fremdeRev, original: f.originalSha256, extraktor: "lva-text", version: "1", abschnitte: [("zeile=1", "x")])
        var neu = b.datei
        neu.fassungen[0].texte[0].textrev = fremdeRev
        neu.fassungen[0].texte[0].blob = AblageKrypto.sha256Hex(bytes4)
        XCTAssertThrowsError(try speicher.textEinlegen(bestand: bestand(neu), dat: b.datei.id, fas: W.fasId(1), textrev: fremdeRev, bytes: bytes4)) {
            XCTAssertEqual($0 as? AblageSpeicherFehler, .passtNicht, "textrev nicht aus den Feldern berechenbar")
        }
        XCTAssertNil(speicher.textLesen(dat: b.datei.id, textrev: fremdeRev))
        // 5. Kein JSON / fehlende Pflichtfelder (Hash passt, Inhalt nicht).
        let kein = Array("kein json".utf8)
        var mitKein = b.datei
        mitKein.fassungen[0].texte[0].blob = AblageKrypto.sha256Hex(kein)
        abgelehnt("kein JSON", kein, bestand(mitKein))
        // 6. Über der Lesegrenze (32 MiB).
        var riesig = b.textBytes
        riesig += [UInt8](repeating: 0x20, count: AblageTextfassung.maxBytes)
        var mitRiesig = b.datei
        mitRiesig.fassungen[0].texte[0].blob = AblageKrypto.sha256Hex(riesig)
        abgelehnt("zu groß", riesig, bestand(mitRiesig))
        XCTAssertEqual(list("text"), [], "nach allen Ablehnungen liegt nichts im Textordner")
    }

    func testTextEinlegenOhneEintragIstFalseUngueltigeKennungWirft() throws {
        let b = beispiel()
        let bs = bestand(b.datei)
        XCTAssertFalse(try speicher.textEinlegen(bestand: bs, dat: W.datId(9), fas: W.fasId(1), textrev: b.eintrag.textrev, bytes: b.textBytes), "Datei gibt es nicht (mehr)")
        XCTAssertFalse(try speicher.textEinlegen(bestand: bs, dat: b.datei.id, fas: W.fasId(9), textrev: b.eintrag.textrev, bytes: b.textBytes), "Fassung gibt es nicht (mehr)")
        XCTAssertFalse(try speicher.textEinlegen(bestand: bs, dat: b.datei.id, fas: W.fasId(1), textrev: String(repeating: "d", count: 64), bytes: b.textBytes), "Eintrag gibt es nicht")
        for (dat, rev) in [("dat_../../x", b.eintrag.textrev), (b.datei.id, "../../etc/passwd"), (b.datei.id, b.eintrag.textrev.uppercased())] {
            XCTAssertThrowsError(try speicher.textEinlegen(bestand: bs, dat: dat, fas: W.fasId(1), textrev: rev, bytes: b.textBytes)) {
                XCTAssertEqual($0 as? AblageSpeicherFehler, .ungueltigeKennung)
            }
        }
        XCTAssertEqual(list("text"), [])
        XCTAssertNil(speicher.textURL(dat: "dat_../x", textrev: b.eintrag.textrev))
        XCTAssertNil(speicher.originalURL(dat: b.datei.id, fas: "fas_x/y", typ: "pdf"))
        XCTAssertNil(speicher.originalURL(dat: b.datei.id, fas: W.fasId(1), typ: "pd/f"))
    }

    // MARK: Original einlegen

    private func zwischen(_ inhalt: [UInt8]) throws -> URL {
        let z = speicher.zwischenpfad()
        try Data(inhalt).write(to: z)
        return z
    }

    func testOriginalEinlegenPrueftGroesseUndHashUndLegtOhneErsetzenAb() throws {
        let b = beispiel()
        let bs = bestand(b.datei)
        let z1 = try zwischen(b.original)
        XCTAssertTrue(try speicher.originalEinlegen(bestand: bs, dat: b.datei.id, fas: W.fasId(1), zwischen: z1))
        XCTAssertFalse(fm.fileExists(atPath: z1.path), "Zwischendatei ist verschoben")
        let ziel = try XCTUnwrap(speicher.originalURL(dat: b.datei.id, fas: W.fasId(1), typ: "pdf"))
        XCTAssertEqual(try Data(contentsOf: ziel), Data(b.original))
        XCTAssertEqual(ziel.lastPathComponent, "\(W.fasId(1)).pdf")
        XCTAssertTrue(speicher.vorhanden(bs).original(b.datei.id, W.fasId(1)))
        // Liegt es schon, wird nicht ersetzt; die Zwischendatei verschwindet.
        let z2 = try zwischen(b.original)
        XCTAssertTrue(try speicher.originalEinlegen(bestand: bs, dat: b.datei.id, fas: W.fasId(1), zwischen: z2))
        XCTAssertFalse(fm.fileExists(atPath: z2.path))
        XCTAssertEqual(list("eingang"), [])
    }

    func testOriginalEinlegenVerwirftFalscheGroesseHashUndUnbekanntes() throws {
        let b = beispiel()
        let bs = bestand(b.datei)
        // Falsche Größe.
        let z1 = try zwischen(Array("Original 12".utf8))
        XCTAssertThrowsError(try speicher.originalEinlegen(bestand: bs, dat: b.datei.id, fas: W.fasId(1), zwischen: z1)) {
            XCTAssertEqual($0 as? AblageSpeicherFehler, .passtNicht)
        }
        XCTAssertFalse(fm.fileExists(atPath: z1.path), "Zwischendatei weg")
        // Gleiche Größe, anderer Inhalt: nur der Hash erkennt es.
        let z2 = try zwischen(Array("Original 2".utf8))
        XCTAssertThrowsError(try speicher.originalEinlegen(bestand: bs, dat: b.datei.id, fas: W.fasId(1), zwischen: z2)) {
            XCTAssertEqual($0 as? AblageSpeicherFehler, .passtNicht)
        }
        XCTAssertFalse(fm.fileExists(atPath: z2.path))
        // Datei oder Fassung gibt es nicht (mehr): false, Zwischendatei weg.
        let z3 = try zwischen(b.original)
        XCTAssertFalse(try speicher.originalEinlegen(bestand: bs, dat: b.datei.id, fas: W.fasId(9), zwischen: z3))
        XCTAssertFalse(fm.fileExists(atPath: z3.path))
        let z4 = try zwischen(b.original)
        XCTAssertFalse(try speicher.originalEinlegen(bestand: AblageBestand(), dat: b.datei.id, fas: W.fasId(1), zwischen: z4))
        XCTAssertFalse(fm.fileExists(atPath: z4.path))
        XCTAssertEqual(list("dateien"), [], "nichts abgelegt")
        XCTAssertFalse(speicher.vorhanden(bs).original(b.datei.id, W.fasId(1)))
    }

    func testOriginalEinlegenGehtNieDurchEineVerknuepfung() throws {
        let b = beispiel()
        let bs = bestand(b.datei)
        // `dateien/<dat>` ist eine Verknüpfung nach außen: nicht hineinschreiben.
        let aussen = tmp.appendingPathComponent("aussen", isDirectory: true)
        try fm.createDirectory(at: aussen, withIntermediateDirectories: true)
        try fm.createDirectory(at: speicher.wurzel.appendingPathComponent("dateien"), withIntermediateDirectories: true)
        try fm.createSymbolicLink(at: speicher.wurzel.appendingPathComponent("dateien/\(b.datei.id)"), withDestinationURL: aussen)
        let z = try zwischen(b.original)
        XCTAssertThrowsError(try speicher.originalEinlegen(bestand: bs, dat: b.datei.id, fas: W.fasId(1), zwischen: z)) {
            XCTAssertEqual($0 as? AblageSpeicherFehler, .schreiben)
        }
        XCTAssertEqual((try? fm.contentsOfDirectory(atPath: aussen.path)) ?? ["x"], [], "außerhalb bleibt unberührt")
        XCTAssertFalse(fm.fileExists(atPath: z.path))
        // Dasselbe für Texte.
        try fm.createDirectory(at: speicher.wurzel.appendingPathComponent("text"), withIntermediateDirectories: true)
        try fm.createSymbolicLink(at: speicher.wurzel.appendingPathComponent("text/\(b.datei.id)"), withDestinationURL: aussen)
        XCTAssertThrowsError(try speicher.textEinlegen(bestand: bs, dat: b.datei.id, fas: W.fasId(1), textrev: b.eintrag.textrev, bytes: b.textBytes)) {
            XCTAssertEqual($0 as? AblageSpeicherFehler, .schreiben)
        }
        XCTAssertEqual((try? fm.contentsOfDirectory(atPath: aussen.path)) ?? ["x"], [])
        // Eine Datei hinter einer Verknüpfung zählt nicht als vorhanden, und Lesen geht nicht hindurch.
        try Data(b.textBytes).write(to: aussen.appendingPathComponent("\(b.eintrag.textrev).json"))
        XCTAssertFalse(speicher.vorhanden(bs).text(b.datei.id, b.eintrag.textrev))
        XCTAssertNil(speicher.textLesen(dat: b.datei.id, textrev: b.eintrag.textrev))
    }

    // MARK: Vorhanden, Aufräumen

    func testBereinigenEntferntNurWasKeinBestandMehrBraucht() throws {
        let b = beispiel()
        var bs = bestand(b.datei)
        XCTAssertTrue(try speicher.textEinlegen(bestand: bs, dat: b.datei.id, fas: W.fasId(1), textrev: b.eintrag.textrev, bytes: b.textBytes))
        XCTAssertTrue(try speicher.originalEinlegen(bestand: bs, dat: b.datei.id, fas: W.fasId(1), zwischen: try zwischen(b.original)))
        // Verwaiste Reste: begrabene Datei, entfernte Fassung, fremde Dateinamen, nicht mehr eingetragene Textfassung.
        let tot = W.datId(7)
        for rel in ["dateien/\(tot)/\(W.fasId(1)).pdf", "text/\(tot)/\(b.eintrag.textrev).json",
                    "dateien/\(b.datei.id)/\(W.fasId(2)).pdf", "dateien/\(b.datei.id)/\(W.fasId(1)).pdf.part", "dateien/\(b.datei.id)/notiz.txt",
                    "text/\(b.datei.id)/\(String(repeating: "e", count: 64)).json", "text/\(b.datei.id)/\(b.eintrag.textrev).json.tmp"] {
            let u = speicher.wurzel.appendingPathComponent(rel)
            try fm.createDirectory(at: u.deletingLastPathComponent(), withIntermediateDirectories: true)
            try Data("x".utf8).write(to: u)
        }
        let n = speicher.bereinigen(bestand: bs)
        XCTAssertEqual(n, 7)
        XCTAssertEqual(list("dateien"), [b.datei.id])
        XCTAssertEqual(list("dateien/\(b.datei.id)"), ["\(W.fasId(1)).pdf"])
        XCTAssertEqual(list("text"), [b.datei.id])
        XCTAssertEqual(list("text/\(b.datei.id)"), ["\(b.eintrag.textrev).json"])
        XCTAssertEqual(speicher.bereinigen(bestand: bs), 0, "idempotent")
        // Fassung per `entfernt` aus dem Bestand: ihre Dateien gehen beim nächsten Bereinigen.
        bs.dateien[b.datei.id]!.fassungen = []
        XCTAssertEqual(speicher.bereinigen(bestand: bs), 2)
        XCTAssertEqual(list("dateien/\(b.datei.id)"), [])
        // Leerer Bestand: alles weg (Zustand verloren oder alle Dateien begraben).
        XCTAssertGreaterThanOrEqual(speicher.bereinigen(bestand: AblageBestand()), 0)
        XCTAssertEqual(list("dateien"), [])
        XCTAssertEqual(list("text"), [])
    }

    func testBereinigenFolgtKeinerVerknuepfung() throws {
        let aussen = tmp.appendingPathComponent("aussen", isDirectory: true)
        try fm.createDirectory(at: aussen, withIntermediateDirectories: true)
        try Data("wichtig".utf8).write(to: aussen.appendingPathComponent("a.txt"))
        try fm.createDirectory(at: speicher.wurzel.appendingPathComponent("dateien"), withIntermediateDirectories: true)
        try fm.createSymbolicLink(at: speicher.wurzel.appendingPathComponent("dateien/\(W.datId(5))"), withDestinationURL: aussen)
        _ = speicher.bereinigen(bestand: AblageBestand())
        XCTAssertEqual(try String(contentsOf: aussen.appendingPathComponent("a.txt"), encoding: .utf8), "wichtig", "Ziel der Verknüpfung bleibt")
        XCTAssertEqual(list("dateien"), [], "die Verknüpfung selbst ist weg")
    }

    func testEntfernenBegrabenerDateien() throws {
        let b = beispiel()
        let bs = bestand(b.datei)
        XCTAssertTrue(try speicher.textEinlegen(bestand: bs, dat: b.datei.id, fas: W.fasId(1), textrev: b.eintrag.textrev, bytes: b.textBytes))
        XCTAssertTrue(try speicher.originalEinlegen(bestand: bs, dat: b.datei.id, fas: W.fasId(1), zwischen: try zwischen(b.original)))
        speicher.entfernen(dateien: [b.datei.id, "dat_../x", W.datId(9)])
        XCTAssertEqual(list("dateien"), [])
        XCTAssertEqual(list("text"), [])
        XCTAssertFalse(speicher.vorhanden(bs).original(b.datei.id, W.fasId(1)))
    }

    func testZwischenpfadEingangUndRaeumen() throws {
        let a = speicher.zwischenpfad(), b = speicher.zwischenpfad()
        XCTAssertNotEqual(a, b)
        XCTAssertEqual(a.deletingLastPathComponent().lastPathComponent, "eingang")
        try Data("x".utf8).write(to: a)
        try Data("y".utf8).write(to: b.appendingPathExtension("part"))
        // Reste früherer Läufe (nie angemeldet) gehen weg, laufende Downloads (a, b samt .part) bleiben.
        try Data("alt".utf8).write(to: speicher.wurzel.appendingPathComponent("eingang/.sync-alt"))
        try Data("alt".utf8).write(to: speicher.wurzel.appendingPathComponent("eingang/.sync-alt.part"))
        XCTAssertEqual(list("eingang").count, 4)
        speicher.eingangRaeumen()
        XCTAssertEqual(Set(list("eingang")), [a.lastPathComponent, b.lastPathComponent + ".part"], "nur Reste weg, laufende bleiben")
        // Beendet (zwischenEntfernen): beim nächsten Räumen ist nichts mehr da, auch nicht der .part.
        speicher.zwischenEntfernen(a)
        speicher.zwischenEntfernen(b)
        XCTAssertEqual(list("eingang"), [])
        // Ein Rest mit dem Namen eines beendeten Downloads wird wieder geräumt.
        try Data("x".utf8).write(to: a)
        speicher.eingangRaeumen()
        XCTAssertEqual(list("eingang"), [])
    }

    func testDefektEntfernenUndAlleEntfernen() throws {
        try fm.createDirectory(at: speicher.wurzel, withIntermediateDirectories: true)
        let defekt = speicher.wurzel.appendingPathComponent("ablage-zustand.json.defekt")
        try Data("Namen".utf8).write(to: defekt)
        speicher.defektEntfernen()
        XCTAssertFalse(fm.fileExists(atPath: defekt.path))
        // Abmelden: der ganze Ordner ist weg (Zustand, Texte, Originale, Eingang, .defekt).
        let b = beispiel()
        try speicher.speichern(AblageZustand(konto: "k"))
        XCTAssertTrue(try speicher.textEinlegen(bestand: bestand(b.datei), dat: b.datei.id, fas: W.fasId(1), textrev: b.eintrag.textrev, bytes: b.textBytes))
        try Data("Namen".utf8).write(to: defekt)
        _ = speicher.zwischenpfad()
        speicher.alleEntfernen()
        XCTAssertFalse(fm.fileExists(atPath: speicher.wurzel.path))
        speicher.alleEntfernen()   // ohne Ordner kein Fehler
    }

    func testAlleDateienEntfernenLaesstDenZustand() throws {
        let b = beispiel()
        let bs = bestand(b.datei)
        try speicher.speichern(AblageZustand(konto: "k"))
        XCTAssertTrue(try speicher.textEinlegen(bestand: bs, dat: b.datei.id, fas: W.fasId(1), textrev: b.eintrag.textrev, bytes: b.textBytes))
        speicher.alleDateienEntfernen()
        XCTAssertEqual(list(""), ["ablage-zustand.json"])
    }
}
#endif
