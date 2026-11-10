import XCTest
@testable import VoiceCore
#if canImport(CryptoKit)
import CryptoKit

// Lauf der Ablage gegen einen nachgebauten Hub (D5a-3, lva#441, Abnahme 3 und 4): Pull je Sammlung mit eigenem Cursor
// (Ordner vor Dateien), 410 = Neustart von 0 (genau einmal), Zustand je Seite atomar, Texte vorab, Originale nur auf
// Anforderung, Budget, Abbruch, Netz- und Schreibfehler, gleichzeitige Läufe. Der Nachbau ist ein Test-Double mit
// den Regeln des Hubs (Sammlungsfilter, `server_seq`, `more`, Blob-Speicher); der echte HTTP-Weg wird gegen
// `apps/apple/scripts/mock_voice_hub.py` im Simulator geprüft. Alles synthetisch.

private typealias W = AblageWelt

/// Nachbau des Hubs für den Lauf (`AblageHub`): Objekte je Sammlung nach `server_seq`, Blobs nach Hub-Kennung.
final class AblageHubNachbau: AblageHub, @unchecked Sendable {
    private let lock = NSLock()
    private var _objekte: [AblagePullObjekt] = []
    private var _seq: Int64 = 0
    private var _blobs: [String: [UInt8]] = [:]
    private var _protokoll: [String] = []
    private var _blobAufrufe: [(hubId: String, bulk: Bool)] = []
    var pullHook: (@Sendable (_ since: Int64, _ collection: String) async throws -> Void)?
    var blobHook: (@Sendable (_ hubId: String) throws -> Void)?
    /// Läuft in `blobLaden` nach dem Protokoll und vor dem Schreiben (kann warten, auch ohne auf Abbruch zu achten).
    var blobVorher: (@Sendable (_ hubId: String, _ ziel: URL) async throws -> Void)?

    /// Gesperrter Zugriff; die Sperre wird nie über ein `await` gehalten.
    private func mit<T>(_ body: () -> T) -> T { lock.withLock(body) }

    var protokoll: [String] { mit { _protokoll } }
    var blobAufrufe: [(hubId: String, bulk: Bool)] { mit { _blobAufrufe } }

    /// Legt ein Objekt an oder ersetzt es (neue `server_seq`).
    func stelle(_ o: AblagePullObjekt) {
        mit {
            _seq += 1
            var n = o
            n.serverSeq = _seq
            _objekte.removeAll { $0.collection == o.collection && $0.objectId == o.objectId }
            _objekte.append(n)
        }
    }

    func blob(_ bytes: [UInt8], key: [UInt8] = W.key) -> String {
        let id = AblageKrypto.hubBlobId(key: key, sha256Hex: AblageKrypto.sha256Hex(bytes))!
        mit { _blobs[id] = bytes }
        return id
    }

    func aendereBlob(_ id: String, _ bytes: [UInt8]) { mit { _blobs[id] = bytes } }

    func pull(since: Int64, limit: Int, collection: String) async throws -> AblagePullSeite {
        mit { _protokoll.append("pull \(collection) since=\(since) limit=\(limit)") }
        if let h = pullHook { try await h(since, collection) }
        return mit {
            let alle = _objekte.filter { $0.collection == collection && $0.serverSeq > since }.sorted { $0.serverSeq < $1.serverSeq }
            let seite = Array(alle.prefix(limit))
            return AblagePullSeite(objects: seite, cursor: seite.last?.serverSeq ?? since, more: alle.count > seite.count)
        }
    }

    func blobLaden(hubId: String, groesse: UInt64, ziel: URL, bulk: Bool) async throws {
        mit { _protokoll.append("blob \(hubId.prefix(8)) bulk=\(bulk)"); _blobAufrufe.append((hubId, bulk)) }
        if let h = blobHook { try h(hubId) }
        if let v = blobVorher { try await v(hubId, ziel) }
        guard let data = mit({ _blobs[hubId] }) else { throw AblageHubFehler.http(404, "not_found") }
        guard UInt64(data.count) == groesse else { throw AblageHubFehler.beschaedigt }
        try Data(data).write(to: ziel)
    }
}

/// Ein Tor, an dem ein Test-Hub wartet, bis der Test es öffnet. Das Warten achtet nicht auf Abbruch (wie ein Hub, der nicht
/// sofort auf den Abbruch reagiert).
final class AblageTor: @unchecked Sendable {
    private let lock = NSLock()
    private var offen = false
    private var wartende: [CheckedContinuation<Void, Never>] = []

    func warten() async {
        await withCheckedContinuation { (c: CheckedContinuation<Void, Never>) in
            let sofort = lock.withLock { () -> Bool in
                if offen { return true }
                wartende.append(c)
                return false
            }
            if sofort { c.resume() }
        }
    }

    func oeffnen() {
        let w = lock.withLock { () -> [CheckedContinuation<Void, Never>] in
            offen = true
            let w = wartende
            wartende = []
            return w
        }
        for c in w { c.resume() }
    }
}

/// Wartet (höchstens 10 s), bis `bedingung` gilt.
func ablageWarteBis(_ bedingung: @Sendable () -> Bool, file: StaticString = #filePath, line: UInt = #line) async {
    for _ in 0..<1000 {
        if bedingung() { return }
        try? await Task.sleep(nanoseconds: 10_000_000)
    }
    XCTFail("Bedingung nicht eingetreten", file: file, line: line)
}

/// Eine Datei mit Original und Textfassung samt Blobs am Hub.
private struct Welt {
    var datei: AblageDatei
    var text: AblageText
    var textBytes: [UInt8]
    var original: [UInt8]
}

private func welt(_ n: Int, ordner: String = W.ordId(1), extra: String = "", typ: String = "pdf") -> Welt {
    let inhalt = "Original \(n)"
    var f = W.fassung(n, nil, inhalt: inhalt, typ: typ)
    let t = W.text(zu: f, abschnitte: [("zeile=1", "Text \(n) \(extra)")])
    f.texte = [t.eintrag]
    return Welt(datei: W.datei(n, ordner: ordner, name: "d\(n).\(typ)", fassungen: [f]), text: t.eintrag, textBytes: t.bytes, original: Array(inhalt.utf8))
}

final class AblageLaufTests: XCTestCase {
    private var tmp: URL!
    private var speicher: AblageSpeicher!
    private let fm = FileManager.default
    private let konto = W.konto

    override func setUpWithError() throws {
        tmp = fm.temporaryDirectory.appendingPathComponent("ablage-lauf-\(UUID().uuidString)", isDirectory: true)
        try fm.createDirectory(at: tmp, withIntermediateDirectories: true)
        speicher = AblageSpeicher(wurzel: tmp.appendingPathComponent("ablage", isDirectory: true))
    }

    override func tearDownWithError() throws {
        if let e = fm.enumerator(at: tmp, includingPropertiesForKeys: nil) {
            for case let u as URL in e { try? fm.setAttributes([.posixPermissions: 0o755], ofItemAtPath: u.path) }
        }
        try? fm.setAttributes([.posixPermissions: 0o755], ofItemAtPath: tmp.path)
        try? fm.removeItem(at: tmp)
    }

    private func list(_ rel: String) -> [String] {
        ((try? fm.contentsOfDirectory(atPath: speicher.wurzel.appendingPathComponent(rel).path)) ?? []).sorted()
    }

    /// Ordner (`internal`) und die Dateien samt Blobs am Hub.
    @discardableResult
    private func bestuecke(_ hub: AblageHubNachbau, _ welten: [Welt], ordner: [AblageOrdner]? = nil) -> [Welt] {
        for o in ordner ?? [W.ordner(1, name: "Ablage", klasse: "internal")] { hub.stelle(W.remote(ordner: o)) }
        for w in welten {
            hub.stelle(W.remote(datei: w.datei))
            _ = hub.blob(w.textBytes)
            _ = hub.blob(w.original)
        }
        return welten
    }

    private func lauf(_ hub: AblageHubNachbau, _ o: AblageLaufOptionen = AblageLaufOptionen(), konto k: AblageKonto? = nil) async throws -> AblageBericht {
        try await AblageLauf.ausfuehren(hub: hub, speicher: speicher, konto: k ?? konto, optionen: o)
    }

    private func zustand() -> AblageZustand { speicher.laden(konto: konto.kennung).zustand }

    // MARK: Ablauf, Reihenfolge, Texte vor Originalen

    func testVollerLaufOrdnerVorDateienTexteVorabOriginaleNicht() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1), welt(2), welt(3, typ: "png")])
        let b = try await lauf(hub)
        XCTAssertTrue(b.vollstaendig)
        XCTAssertTrue(b.geaendert)
        XCTAssertEqual(b.warnungen, [])
        // Ordner vor Dateien, je Sammlung mit `collection=` und eigenem Cursor, beginnend bei 0.
        let pulls = hub.protokoll.filter { $0.hasPrefix("pull") }
        XCTAssertEqual(pulls, ["pull ablage_ordner since=0 limit=200", "pull ablage_datei since=0 limit=200"])
        let z = zustand()
        XCTAssertEqual(z.bestand.ordner.count, 1)
        XCTAssertEqual(z.bestand.dateien.count, 3)
        XCTAssertGreaterThan(z.ledger.cursor["ablage_ordner"] ?? 0, 0)
        XCTAssertGreaterThan(z.ledger.cursor["ablage_datei"] ?? 0, 0)
        // Texte liegen da (nur die der Dateien mit Text), Originale nicht.
        let v = speicher.vorhanden(z.bestand)
        XCTAssertTrue(v.text(ws[0].datei.id, ws[0].text.textrev))
        XCTAssertTrue(v.text(ws[1].datei.id, ws[1].text.textrev))
        XCTAssertFalse(v.original(ws[0].datei.id, W.fasId(1)))
        XCTAssertEqual(list("dateien"), [])
        XCTAssertEqual(b.texteGeladen, 3, "drei Texteinträge (das Bild trägt auch einen im Test)")
        XCTAssertEqual(b.originaleGeladen, 0)
        XCTAssertEqual(hub.blobAufrufe.count, 3)
        XCTAssertEqual(list("eingang"), [], "keine Zwischenreste")
        XCTAssertEqual(speicher.textLesen(dat: ws[0].datei.id, textrev: ws[0].text.textrev), ws[0].textBytes)
        // Zweiter Lauf: nichts Neues, keine Downloads, Cursor werden fortgeführt.
        let n = hub.blobAufrufe.count
        let b2 = try await lauf(hub)
        XCTAssertFalse(b2.geaendert)
        XCTAssertEqual(hub.blobAufrufe.count, n)
        XCTAssertTrue(hub.protokoll.contains { $0.hasPrefix("pull ablage_datei since=") && !$0.contains("since=0") })
    }

    func testOriginaleNurAufAnforderungNachTexten() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1), welt(2)])
        let o = AblageLaufOptionen(angefordert: [AblageOriginalAnforderung(dat: ws[0].datei.id, fas: W.fasId(1))])
        let b = try await lauf(hub, o)
        XCTAssertEqual(b.originaleGeladen, 1)
        XCTAssertEqual(b.texteGeladen, 2)
        let reihenfolge = hub.blobAufrufe.map(\.hubId)
        let textIds = ws.map { AblageKrypto.hubBlobId(key: W.key, sha256Hex: $0.text.blob)! }
        let origId = AblageKrypto.hubBlobId(key: W.key, sha256Hex: ws[0].datei.fassungen[0].blob)!
        XCTAssertEqual(Set(reihenfolge.prefix(2)), Set(textIds), "Texte vor dem Original")
        XCTAssertEqual(reihenfolge.last, origId)
        let v = speicher.vorhanden(zustand().bestand)
        XCTAssertTrue(v.original(ws[0].datei.id, W.fasId(1)))
        XCTAssertFalse(v.original(ws[1].datei.id, W.fasId(2)), "nicht angefordert, nicht geladen")
        XCTAssertEqual(try Data(contentsOf: try XCTUnwrap(speicher.originalURL(dat: ws[0].datei.id, fas: W.fasId(1), typ: "pdf"))), Data(ws[0].original))
    }

    func testPullSeitenweiseUndAbbruchNachErsterSeiteSetztFortUndDoppeltNichts() async throws {
        let hub = AblageHubNachbau()
        let ws = (1...5).map { welt($0) }
        bestuecke(hub, ws)
        // Seitengröße 2 über die Option; die zweite Datei-Seite scheitert (Verbindung weg).
        let ausfall = AblageZaehler()
        hub.pullHook = { _, coll in
            if coll == "ablage_datei", ausfall.erhoehe() == 2 { throw AblageHubFehler.netz }
        }
        var o = AblageLaufOptionen(); o.pullLimit = 2
        do {
            _ = try await lauf(hub, o)
            XCTFail("sollte scheitern")
        } catch let e as AblageLaufFehler {
            XCTAssertEqual(e, .hub(.netz))
        }
        // Erste Seite (zwei Dateien) und der Cursor sind gespeichert.
        let z1 = zustand()
        XCTAssertEqual(z1.bestand.dateien.count, 2)
        let cursor1 = try XCTUnwrap(z1.ledger.cursor["ablage_datei"])
        // Zweiter Lauf setzt beim gespeicherten Cursor fort, nicht bei 0.
        hub.pullHook = nil
        _ = try await lauf(hub, o)
        XCTAssertTrue(hub.protokoll.contains("pull ablage_datei since=\(cursor1) limit=2"))
        XCTAssertEqual(zustand().bestand.dateien.count, 5)
        // Ergebnis gleich einem ungestörten Lauf in einem frischen Speicher.
        let anders = AblageSpeicher(wurzel: tmp.appendingPathComponent("anders", isDirectory: true))
        _ = try await AblageLauf.ausfuehren(hub: hub, speicher: anders, konto: konto, optionen: o)
        let a = anders.laden(konto: konto.kennung).zustand, c = zustand()
        XCTAssertEqual(a.bestand, c.bestand)
        XCTAssertEqual(a.ledger, c.ledger)
        XCTAssertEqual(speicher.vorhanden(c.bestand), anders.vorhanden(a.bestand), "kein doppelter oder fehlender Text")
    }

    // MARK: 410, ohne Fortschritt, zu viele Seiten, abgemeldet

    func testCursorAbgelaufenStartetGenauEinmalVonNullNeu() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1), welt(2)])
        _ = try await lauf(hub)
        let cursor = try XCTUnwrap(zustand().ledger.cursor["ablage_datei"])
        // Der Hub lässt den gespeicherten Cursor ablaufen (410 für since == cursor), danach neue Objekte.
        hub.stelle(W.remote(datei: welt(3).datei))
        let gesehen = AblageZaehler()
        hub.pullHook = { since, coll in
            if coll == "ablage_datei", since == cursor, gesehen.erhoehe() == 1 { throw AblageHubFehler.cursorAbgelaufen }
        }
        let b = try await lauf(hub)
        XCTAssertEqual(b.neustarts, 1)
        let datei = hub.protokoll.filter { $0.hasPrefix("pull ablage_datei") }.suffix(2)
        XCTAssertTrue(datei.first?.contains("since=\(cursor) ") == true, "erst der abgelaufene Cursor: \(Array(datei))")
        XCTAssertTrue(datei.last?.contains("since=0 ") == true, "danach von 0: \(Array(datei))")
        let z = zustand()
        XCTAssertEqual(z.bestand.dateien.count, 3)
        XCTAssertGreaterThan(z.ledger.cursor["ablage_datei"] ?? 0, 0)
        XCTAssertNotNil(z.bestand.dateien[ws[0].datei.id], "bekannte Objekte bleiben (Revision schon gesehen)")
    }

    func testCursorAbgelaufenZweimalOderBeiNullIstFehler() async throws {
        let hub = AblageHubNachbau()
        bestuecke(hub, [welt(1), welt(2), welt(3)])
        var o = AblageLaufOptionen(); o.pullLimit = 1
        // 410 bei since == 0: kein Neustart möglich, Fehler.
        hub.pullHook = { since, _ in if since == 0 { throw AblageHubFehler.cursorAbgelaufen } }
        do { _ = try await lauf(hub, o); XCTFail() } catch let e as AblageLaufFehler { XCTAssertEqual(e, .hub(.cursorAbgelaufen)) }
        // Nach dem Neustart abermals 410 (bei since > 0): Fehler, nicht endlos.
        hub.pullHook = nil
        _ = try await lauf(hub, o)
        let cursor = try XCTUnwrap(zustand().ledger.cursor["ablage_datei"])
        hub.stelle(W.remote(datei: welt(4).datei))
        hub.pullHook = { since, coll in if coll == "ablage_datei", since > 0 { throw AblageHubFehler.cursorAbgelaufen } }
        do { _ = try await lauf(hub, o); XCTFail() } catch let e as AblageLaufFehler { XCTAssertEqual(e, .hub(.cursorAbgelaufen)) }
        XCTAssertGreaterThan(cursor, 0)
        let datei410 = hub.protokoll.filter { $0.hasPrefix("pull ablage_datei") }
        XCTAssertLessThan(datei410.count, 20, "kein Endlos-Neustart")
    }

    func testOhneFortschrittUndZuVieleSeitenBrechenAb() async throws {
        // Der Hub meldet `more` ohne Cursor-Fortschritt.
        final class Haengt: AblageHub, @unchecked Sendable {
            let aufrufe = AblageZaehler()
            let fortschritt: Bool
            init(fortschritt: Bool) { self.fortschritt = fortschritt }
            func pull(since: Int64, limit: Int, collection: String) async throws -> AblagePullSeite {
                aufrufe.erhoehe()
                return AblagePullSeite(objects: [], cursor: fortschritt ? since + 1 : since, more: true)
            }
            func blobLaden(hubId: String, groesse: UInt64, ziel: URL, bulk: Bool) async throws {}
        }
        let h1 = Haengt(fortschritt: false)
        do { _ = try await AblageLauf.ausfuehren(hub: h1, speicher: speicher, konto: konto); XCTFail() } catch let e as AblageLaufFehler { XCTAssertEqual(e, .ohneFortschritt) }
        XCTAssertEqual(h1.aufrufe.wert, 1, "bricht bei der ersten Seite ab")
        let h2 = Haengt(fortschritt: true)
        do { _ = try await AblageLauf.ausfuehren(hub: h2, speicher: speicher, konto: konto); XCTFail() } catch let e as AblageLaufFehler { XCTAssertEqual(e, .zuVieleSeiten) }
        XCTAssertEqual(h2.aufrufe.wert, 1000, "höchstens 1000 Seiten je Sammlung")
        // Nach einem Fehler läuft der nächste Lauf wieder (Sperre ist frei).
        let ok = AblageHubNachbau()
        let b = try await AblageLauf.ausfuehren(hub: ok, speicher: speicher, konto: konto)
        XCTAssertTrue(b.vollstaendig)
    }

    func testAbgemeldetBrichtAbBisherigesBleibt() async throws {
        let hub = AblageHubNachbau()
        bestuecke(hub, [welt(1)])
        _ = try await lauf(hub)
        hub.stelle(W.remote(datei: welt(2).datei))
        hub.pullHook = { _, coll in if coll == "ablage_datei" { throw AblageHubFehler.abgemeldet } }
        do { _ = try await lauf(hub); XCTFail() } catch let e as AblageLaufFehler { XCTAssertEqual(e, .abgemeldet) }
        XCTAssertEqual(zustand().bestand.dateien.count, 1, "Stand vor der Abmeldung bleibt")
    }

    func testSchreibfehlerBrichtAbOhneCursorVorzuruecken() async throws {
        if getuid() == 0 { throw XCTSkip("als root nicht prüfbar") }
        let hub = AblageHubNachbau()
        bestuecke(hub, [welt(1)])
        try fm.createDirectory(at: speicher.wurzel, withIntermediateDirectories: true)
        try fm.setAttributes([.posixPermissions: 0o500], ofItemAtPath: speicher.wurzel.path)
        do { _ = try await lauf(hub); XCTFail() } catch let e as AblageLaufFehler { XCTAssertEqual(e, .speicher) }
        try fm.setAttributes([.posixPermissions: 0o755], ofItemAtPath: speicher.wurzel.path)
        XCTAssertEqual(hub.protokoll.filter { $0.hasPrefix("pull") }, ["pull ablage_ordner since=0 limit=200"],
                       "nach dem ersten Schreibfehler wird nichts weiter gezogen")
        XCTAssertEqual(hub.blobAufrufe.count, 0)
        let z = zustand()
        XCTAssertEqual(z.ledger.cursor.count, 0, "Cursor nie vorgerückt, wenn der Zustand nicht gespeichert werden konnte")
        XCTAssertTrue(z.bestand.ordner.isEmpty)
        // Der nächste Lauf holt alles nach.
        _ = try await lauf(hub)
        XCTAssertEqual(zustand().bestand.dateien.count, 1)
    }

    // MARK: Nebenläufigkeit, Abbruch durch Task

    func testZweiLaeufeGleichzeitigNurEinerLaeuft() async throws {
        let hub = AblageHubNachbau()
        bestuecke(hub, [welt(1)])
        hub.pullHook = { _, _ in try await Task.sleep(nanoseconds: 80_000_000) }
        let s = speicher!, k = konto
        async let a = AblageLauf.ausfuehren(hub: hub, speicher: s, konto: k, optionen: AblageLaufOptionen())
        async let b = AblageLauf.ausfuehren(hub: hub, speicher: s, konto: k, optionen: AblageLaufOptionen())
        let (ra, rb) = try await (a, b)
        XCTAssertEqual([ra, rb].filter(\.uebersprungen).count, 1, "genau ein Lauf wird übersprungen")
        XCTAssertEqual([ra, rb].filter { !$0.uebersprungen }.count, 1)
        XCTAssertEqual(hub.protokoll.filter { $0.hasPrefix("pull ablage_ordner") }.count, 1, "nur ein Pull der Ordner")
        XCTAssertEqual(zustand().bestand.dateien.count, 1)
        // Danach ist die Sperre frei.
        hub.pullHook = nil
        let c = try await lauf(hub)
        XCTAssertFalse(c.uebersprungen)
    }

    func testTaskAbbruchWirftUndLaesstZustandHeil() async throws {
        let hub = AblageHubNachbau()
        bestuecke(hub, [welt(1), welt(2), welt(3)])
        hub.pullHook = { _, coll in if coll == "ablage_datei" { try await Task.sleep(nanoseconds: 5_000_000_000) } }
        let s = speicher!, k = konto
        let task = Task { try await AblageLauf.ausfuehren(hub: hub, speicher: s, konto: k, optionen: AblageLaufOptionen()) }
        try await Task.sleep(nanoseconds: 100_000_000)
        task.cancel()
        do { _ = try await task.value; XCTFail() } catch is CancellationError {} catch { XCTFail("\(error)") }
        // Ordner sind gespeichert, Dateien fehlen; der Zustand ist lesbar, der nächste Lauf vollendet.
        let z = zustand()
        XCTAssertEqual(z.bestand.ordner.count, 1)
        hub.pullHook = nil
        _ = try await lauf(hub)
        XCTAssertEqual(zustand().bestand.dateien.count, 3)
    }

    // MARK: Downloads: Budget, Netz, Prüfung, WLAN-Regel

    func testBudgetErsteDateiImmerRestImNaechstenLauf() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1), welt(2), welt(3)])
        var o = AblageLaufOptionen(); o.downloadBudget = 10   // kleiner als jeder Text
        let b1 = try await lauf(hub, o)
        XCTAssertEqual(b1.texteGeladen, 1, "mindestens eine Datei je Lauf, danach Schluss")
        let b2 = try await lauf(hub, o)
        XCTAssertEqual(b2.texteGeladen, 1)
        let b3 = try await lauf(hub, o)
        XCTAssertEqual(b3.texteGeladen, 1)
        let b4 = try await lauf(hub, o)
        XCTAssertEqual(b4.texteGeladen, 0)
        let v = speicher.vorhanden(zustand().bestand)
        for w in ws { XCTAssertTrue(v.text(w.datei.id, w.text.textrev)) }
    }

    func testNetzfehlerStopptDownloadsNichtDenLaufHttpFehlerNurDieDatei() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1), welt(2), welt(3)])
        let ids = ws.map { AblageKrypto.hubBlobId(key: W.key, sha256Hex: $0.text.blob)! }
        // Netz weg beim ersten Download: weitere Versuche entfallen, der Lauf endet ohne Fehler.
        hub.blobHook = { _ in throw AblageHubFehler.netz }
        let b = try await lauf(hub)
        XCTAssertTrue(b.downloadsUnterbrochen)
        XCTAssertEqual(b.texteGeladen, 0)
        XCTAssertEqual(hub.blobAufrufe.count, 1, "nach dem Netzfehler kein weiterer Versuch")
        XCTAssertEqual(zustand().bestand.dateien.count, 3, "der Pull ist trotzdem gespeichert")
        XCTAssertEqual(list("eingang"), [])
        // Ein Blob fehlt (404): nur diese Datei bleibt offen, die anderen kommen.
        let fehlend = ids[0]
        hub.blobHook = { id in if id == fehlend { throw AblageHubFehler.http(404, "not_found") } }
        let b2 = try await lauf(hub)
        XCTAssertFalse(b2.downloadsUnterbrochen)
        XCTAssertEqual(b2.texteGeladen, 2)
        let v = speicher.vorhanden(zustand().bestand)
        XCTAssertEqual(ws.filter { v.text($0.datei.id, $0.text.textrev) }.count, 2)
    }

    func testVollerDatentraegerStopptDownloadsSichtbarOhneZwischenreste() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1), welt(2)])
        // (a) Der Transport meldet, dass lokal nichts mehr geschrieben werden kann.
        hub.blobHook = { _ in throw AblageHubFehler.speicherVoll }
        let b = try await lauf(hub)
        XCTAssertTrue(b.downloadsUnterbrochen)
        XCTAssertTrue(b.warnungen.contains(.dateiNichtSpeicherbar))
        XCTAssertEqual(hub.blobAufrufe.count, 1, "kein weiterer Versuch")
        XCTAssertEqual(list("eingang"), [])
        XCTAssertEqual(zustand().bestand.dateien.count, 2, "der Pull ist trotzdem gespeichert")
        // (b) Das Ablegen scheitert (Textordner nicht beschreibbar): sichtbarer Stopp, nichts Halbes.
        if getuid() == 0 { return }
        hub.blobHook = nil
        let textOrdner = speicher.wurzel.appendingPathComponent("text")
        try fm.createDirectory(at: textOrdner, withIntermediateDirectories: true)
        try fm.setAttributes([.posixPermissions: 0o500], ofItemAtPath: textOrdner.path)
        let b2 = try await lauf(hub)
        try fm.setAttributes([.posixPermissions: 0o755], ofItemAtPath: textOrdner.path)
        XCTAssertTrue(b2.downloadsUnterbrochen)
        XCTAssertTrue(b2.warnungen.contains(.dateiNichtSpeicherbar))
        XCTAssertEqual(b2.texteGeladen, 0)
        XCTAssertEqual(list("eingang"), [])
        XCTAssertEqual(list("text"), [])
        // Danach (Platz wieder da) holt der nächste Lauf alles nach.
        let b3 = try await lauf(hub)
        XCTAssertEqual(b3.texteGeladen, 2)
        let v = speicher.vorhanden(zustand().bestand)
        for w in ws { XCTAssertTrue(v.text(w.datei.id, w.text.textrev)) }
    }

    func testFalscherInhaltDesDownloadsWirdVerworfenNichtAbgelegt() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1), welt(2)])
        // Der Blob der ersten Textfassung liefert andere Bytes gleicher Länge (Hub oder Übertragung beschädigt).
        let id1 = AblageKrypto.hubBlobId(key: W.key, sha256Hex: ws[0].text.blob)!
        var kaputt = ws[0].textBytes
        kaputt[kaputt.count - 4] ^= 0x01
        hub.aendereBlob(id1, kaputt)
        let b = try await lauf(hub)
        XCTAssertTrue(b.warnungen.contains(.dateiPasstNicht))
        XCTAssertEqual(b.texteGeladen, 1)
        let v = speicher.vorhanden(zustand().bestand)
        XCTAssertFalse(v.text(ws[0].datei.id, ws[0].text.textrev))
        XCTAssertTrue(v.text(ws[1].datei.id, ws[1].text.textrev))
        XCTAssertEqual(list("eingang"), [])
        XCTAssertEqual(list("text/\(ws[0].datei.id)"), [], "nichts von der beschädigten Datei")
        for w in b.warnungen { XCTAssertFalse(w.text.contains("d1.pdf")) }
    }

    func testWlanRegelKeineDownloadsAberPullUndBulkFlag() async throws {
        let hub = AblageHubNachbau()
        bestuecke(hub, [welt(1)])
        var o = AblageLaufOptionen(); o.downloadsErlaubt = false
        let b = try await lauf(hub, o)
        XCTAssertTrue(b.downloadsRuhen)
        XCTAssertEqual(hub.blobAufrufe.count, 0)
        XCTAssertEqual(zustand().bestand.dateien.count, 1, "Metadaten werden trotzdem gezogen")
        // Mit `bulk` wird die Regel an den Hub-Weg weitergereicht (kein Mobilfunk, kein Datensparmodus).
        o.downloadsErlaubt = true; o.bulk = true
        _ = try await lauf(hub, o)
        XCTAssertEqual(hub.blobAufrufe.map(\.bulk), [true])
    }

    func testEingangWirdBeiLaufstartGeraeumt() async throws {
        let hub = AblageHubNachbau()
        bestuecke(hub, [])
        try fm.createDirectory(at: speicher.wurzel.appendingPathComponent("eingang"), withIntermediateDirectories: true)
        try Data("x".utf8).write(to: speicher.wurzel.appendingPathComponent("eingang/.sync-alt"))
        _ = try await lauf(hub)
        XCTAssertEqual(list("eingang"), [])
    }

    // MARK: Fehlerpfade des Pulls (Abnahme 4)

    func testFremderSchluesselMachtLaufUnvollstaendigUndRaeumtNichtAuf() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1)])
        let fremd = welt(2)
        hub.stelle(W.remote(datei: fremd.datei, keyId: SyncCrypto.keyId(W.andererKey), key: W.andererKey))
        // Ein verwaister Rest im Speicher.
        let rest = speicher.wurzel.appendingPathComponent("text/\(W.datId(9))/\(String(repeating: "f", count: 64)).json")
        try fm.createDirectory(at: rest.deletingLastPathComponent(), withIntermediateDirectories: true)
        try Data("x".utf8).write(to: rest)
        let b = try await lauf(hub)
        XCTAssertFalse(b.vollstaendig)
        XCTAssertTrue(b.warnungen.contains(.fremderSchluessel))
        XCTAssertTrue(fm.fileExists(atPath: rest.path), "bei unvollständigem Pull wird nicht aufgeräumt")
        XCTAssertEqual(zustand().ledger.angehalteneKennungen, [AblageLedger.schluessel("ablage_datei", fremd.datei.id)])
        XCTAssertEqual(zustand().bestand.dateien.keys, [ws[0].datei.id], "das Fremde ist nicht im Bestand")
        XCTAssertEqual(hub.blobAufrufe.count, 1, "für das Fremde wird nichts geladen")
        // Ohne das fremde Objekt ist der nächste Lauf vollständig und räumt den Rest weg.
        hub.stelle(W.remoteGrab(collection: "ablage_datei", id: W.datId(2), revision: 9))
        let b2 = try await lauf(hub)
        XCTAssertTrue(b2.vollstaendig)
        XCTAssertFalse(fm.fileExists(atPath: rest.path), "vollständig: verwaiste Reste weg")
    }

    func testGrabsteinEntferntDateienUndBegrabenesKehrtNichtZurueck() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1), welt(2)])
        var anforderung = AblageLaufOptionen()
        anforderung.angefordert = [AblageOriginalAnforderung(dat: ws[0].datei.id, fas: W.fasId(1))]
        _ = try await lauf(hub, anforderung)
        XCTAssertEqual(list("text").count, 2)
        XCTAssertEqual(list("dateien"), [ws[0].datei.id])
        // Mit Siegel begraben: Original und Text weg, Kennung gemerkt.
        hub.stelle(W.remoteGrab(collection: "ablage_datei", id: ws[0].datei.id, revision: 2))
        let b = try await lauf(hub)
        XCTAssertEqual(b.begraben, 1)
        XCTAssertEqual(list("dateien"), [])
        XCTAssertEqual(list("text"), [ws[1].datei.id])
        XCTAssertTrue(zustand().ledger.begraben.contains(ws[0].datei.id))
        // Ein später mit höherer Revision erscheinender lesbarer Stand belebt sie nicht wieder.
        hub.stelle(W.remote(datei: ws[0].datei, revision: 3))
        _ = try await lauf(hub)
        XCTAssertNil(zustand().bestand.dateien[ws[0].datei.id])
        XCTAssertEqual(list("text"), [ws[1].datei.id])
    }

    func testGefaelschterGrabsteinLoeschtNichts() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1)])
        _ = try await lauf(hub)
        XCTAssertEqual(list("text").count, 1)
        hub.stelle(W.remoteGrab(collection: "ablage_datei", id: ws[0].datei.id, revision: 2, echt: false))
        let b = try await lauf(hub)
        XCTAssertTrue(b.warnungen.contains(.grabsteinOhneSiegel))
        XCTAssertEqual(b.begraben, 0)
        XCTAssertNotNil(zustand().bestand.dateien[ws[0].datei.id])
        XCTAssertEqual(list("text").count, 1, "Text bleibt")
        XCTAssertEqual(zustand().ledger.angehalteneKennungen, [AblageLedger.schluessel("ablage_datei", ws[0].datei.id)])
        // Kein Index-Eintrag mehr für das angehaltene Objekt.
        let z = zustand()
        XCTAssertEqual(AblageAnzeige.indexQuellen(z, vorhanden: speicher.vorhanden(z.bestand)), [])
    }

    func testOrdnergrabNachDemDateiPullImLauf() async throws {
        let hub = AblageHubNachbau()
        let a = W.ordner(1, name: "A", klasse: "internal")
        let w = welt(1, ordner: a.id)
        bestuecke(hub, [w], ordner: [a])
        _ = try await lauf(hub)
        // Ordner und Datei werden gemeinsam begraben: der Ordner folgt der Datei, nicht umgekehrt.
        hub.stelle(W.remoteGrab(collection: "ablage_ordner", id: a.id, revision: 2))
        hub.stelle(W.remoteGrab(collection: "ablage_datei", id: w.datei.id, revision: 2))
        _ = try await lauf(hub)
        let z = zustand()
        XCTAssertTrue(z.bestand.ordner.isEmpty && z.bestand.dateien.isEmpty)
        XCTAssertTrue(z.ledger.begraben.contains(a.id) && z.ledger.begraben.contains(w.datei.id))
        XCTAssertTrue(z.ledger.ordnergraeber.isEmpty)
    }

    // MARK: Konto

    func testKontoWechselLeertZustandUndDateien() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1)])
        _ = try await lauf(hub)
        XCTAssertEqual(list("text").count, 1)
        // Anderes Konto (anderer Hub): leerer Zustand, Dateien des alten Kontos weg, dann neu gezogen.
        let anderer = AblageKonto(key: W.key, userId: W.userId, hubUrl: "https://anderer-hub.example.test")
        let leererHub = AblageHubNachbau()
        let b = try await AblageLauf.ausfuehren(hub: leererHub, speicher: speicher, konto: anderer)
        XCTAssertTrue(b.kontoGewechselt)
        let z = speicher.laden(konto: anderer.kennung).zustand
        XCTAssertTrue(z.bestand.dateien.isEmpty)
        XCTAssertEqual(z.ledger.konto, anderer.kennung)
        XCTAssertEqual(list("text"), [], "Dateien des alten Kontos sind weg")
        XCTAssertEqual(leererHub.protokoll.first, "pull ablage_ordner since=0 limit=200", "beginnt bei 0")
        XCTAssertNil(z.bestand.dateien[ws[0].datei.id])
        // Zurück zum ersten Konto: wieder leer, es wird neu gezogen.
        let b2 = try await lauf(hub)
        XCTAssertTrue(b2.kontoGewechselt)
        XCTAssertEqual(zustand().bestand.dateien.count, 1)
    }

    // MARK: Ein Original auf Anforderung

    func testOriginalLadenAufAnforderung() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1)])
        _ = try await lauf(hub)
        let n = hub.blobAufrufe.count
        let url = try await AblageLauf.originalLaden(hub: hub, speicher: speicher, konto: konto, dat: ws[0].datei.id, fas: W.fasId(1), bulk: true)
        XCTAssertEqual(try Data(contentsOf: url), Data(ws[0].original))
        XCTAssertEqual(url, speicher.originalURL(dat: ws[0].datei.id, fas: W.fasId(1), typ: "pdf"))
        XCTAssertEqual(hub.blobAufrufe.count, n + 1)
        XCTAssertEqual(hub.blobAufrufe.last?.bulk, true, "WLAN-Regel wird an den Hub-Weg gereicht")
        // Schon da: kein erneuter Download.
        let url2 = try await AblageLauf.originalLaden(hub: hub, speicher: speicher, konto: konto, dat: ws[0].datei.id, fas: W.fasId(1), bulk: true)
        XCTAssertEqual(url2, url)
        XCTAssertEqual(hub.blobAufrufe.count, n + 1)
        XCTAssertEqual(list("eingang"), [])
    }

    func testOriginalLadenFehlerpfade() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1), welt(2)])
        _ = try await lauf(hub)
        func laden(_ dat: String, _ fas: String, platz: (@Sendable () -> UInt64?)? = nil) async throws -> URL {
            try await AblageLauf.originalLaden(hub: hub, speicher: speicher, konto: konto, dat: dat, fas: fas, bulk: false, freierPlatz: platz)
        }
        // Unbekannt, ungültig: nicht ladbar.
        for (d, f) in [(W.datId(9), W.fasId(1)), (ws[0].datei.id, W.fasId(9)), ("dat_../x", W.fasId(1))] {
            do { _ = try await laden(d, f); XCTFail() } catch let e as AblageLaufFehler { XCTAssertEqual(e, .nichtLadbar) }
        }
        // Voller Datenträger: nichts angefangen.
        let n = hub.blobAufrufe.count
        do { _ = try await laden(ws[0].datei.id, W.fasId(1), platz: { 5 }); XCTFail() } catch let e as AblageLaufFehler { XCTAssertEqual(e, .speicherVoll) }
        XCTAssertEqual(hub.blobAufrufe.count, n)
        // Hub liefert falschen Inhalt gleicher Länge: verworfen, nichts abgelegt, kein Zwischenrest.
        let orig = AblageKrypto.hubBlobId(key: W.key, sha256Hex: ws[0].datei.fassungen[0].blob)!
        var kaputt = ws[0].original
        kaputt[0] ^= 0x01
        hub.aendereBlob(orig, kaputt)
        do { _ = try await laden(ws[0].datei.id, W.fasId(1)); XCTFail() } catch let e as AblageLaufFehler { XCTAssertEqual(e, .passtNicht) }
        XCTAssertEqual(list("dateien"), [])
        XCTAssertEqual(list("eingang"), [])
        // Netz weg, Abmeldung, WLAN-Regel: sauber abgebildet, nichts liegen gelassen.
        let orig2 = AblageKrypto.hubBlobId(key: W.key, sha256Hex: ws[1].datei.fassungen[0].blob)!
        let faelle: [(AblageHubFehler, AblageLaufFehler)] = [(.netz, .hub(.netz)), (.abgemeldet, .abgemeldet), (.wlanRegel, .wlanRegel),
                                                             (.speicherVoll, .speicherVoll),
                                                             (.http(404, "not_found"), .hub(.http(404, "not_found")))]
        for (fehler, erwartet) in faelle {
            hub.blobHook = { id in if id == orig2 { throw fehler } }
            do { _ = try await laden(ws[1].datei.id, W.fasId(2)); XCTFail() } catch let e as AblageLaufFehler { XCTAssertEqual(e, erwartet) }
            XCTAssertEqual(list("eingang"), [])
            XCTAssertEqual(list("dateien"), [])
        }
        // Angehalten: nicht laden.
        var z = zustand()
        z.ledger.objekte[AblageLedger.schluessel("ablage_datei", ws[1].datei.id)]?.angehalten = true
        try speicher.speichern(z)
        hub.blobHook = nil
        do { _ = try await laden(ws[1].datei.id, W.fasId(2)); XCTFail() } catch let e as AblageLaufFehler { XCTAssertEqual(e, .nichtLadbar) }
    }

    func testOriginalLadenAbbruchRaeumtZwischendateien() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1)])
        _ = try await lauf(hub)
        hub.blobHook = { _ in throw CancellationError() }
        do { _ = try await AblageLauf.originalLaden(hub: hub, speicher: speicher, konto: konto, dat: ws[0].datei.id, fas: W.fasId(1), bulk: false); XCTFail() }
        catch is CancellationError {}
        XCTAssertEqual(list("eingang"), [])
        XCTAssertEqual(list("dateien"), [])
    }

    // MARK: Nachtrag zu D5a-3: offene Anforderungen im Bericht

    /// Der Bericht nennt die angeforderten Originale, die der Lauf nicht erledigt hat: Mobilfunk, Budget, Netz weg, voller
    /// Datenträger, keine Downloads, übersprungen. Unbekanntes und einzeln Gescheitertes nicht.
    func testBerichtNenntOffeneAnforderungen() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1), welt(2)])
        let a1 = AblageOriginalAnforderung(dat: ws[0].datei.id, fas: W.fasId(1)), a2 = AblageOriginalAnforderung(dat: ws[1].datei.id, fas: W.fasId(2))
        let unbekannt = AblageOriginalAnforderung(dat: W.datId(9), fas: W.fasId(9))
        let ids: Set<String> = [AblageKrypto.hubBlobId(key: W.key, sha256Hex: ws[0].datei.fassungen[0].blob)!,
                                AblageKrypto.hubBlobId(key: W.key, sha256Hex: ws[1].datei.fassungen[0].blob)!]
        _ = try await lauf(hub)                                                         // Texte vorab
        // Mobilfunk: beide warten.
        var o = AblageLaufOptionen(angefordert: [a2, unbekannt, a1, a2])               // Doppelte und Unbekanntes
        o.bulk = true; o.mobil = true
        var b = try await lauf(hub, o)
        XCTAssertEqual(b.offeneAnforderungen, [a2, a1], "Reihenfolge der Anforderung, ohne Doppelte und ohne Unbekanntes")
        // Budget: das erste läuft immer, das zweite bleibt offen (kleinere zuerst: a1 vor a2).
        o = AblageLaufOptionen(angefordert: [a1, a2])
        o.downloadBudget = 1
        b = try await lauf(hub, o)
        XCTAssertEqual(b.originaleGeladen, 1)
        XCTAssertEqual(b.offeneAnforderungen, [a2])
        // Das erledigte Original taucht nie wieder auf.
        b = try await lauf(hub, AblageLaufOptionen(angefordert: [a1, a2]))
        XCTAssertEqual(b.originaleGeladen, 1)
        XCTAssertEqual(b.offeneAnforderungen, [])
        // Netz weg und voller Datenträger (frischer Speicher): die abgebrochene und die folgenden bleiben offen.
        for fehler in [AblageHubFehler.netz, .speicherVoll] {
            speicher.alleEntfernen()
            _ = try await lauf(hub)
            hub.blobHook = { id in if ids.contains(id) { throw fehler } }
            b = try await lauf(hub, AblageLaufOptionen(angefordert: [a1, a2]))
            XCTAssertTrue(b.downloadsUnterbrochen, "\(fehler)")
            XCTAssertEqual(b.offeneAnforderungen, [a1, a2], "\(fehler)")
            hub.blobHook = nil
        }
        // Eine einzelne Datei, die scheitert (Blob fehlt), ist nicht „offen“.
        hub.blobHook = { id in if id == ids.sorted()[0] { throw AblageHubFehler.http(404, "not_found") } }
        b = try await lauf(hub, AblageLaufOptionen(angefordert: [a1, a2]))
        XCTAssertEqual(b.originaleGeladen, 1)
        XCTAssertEqual(b.offeneAnforderungen, [])
        hub.blobHook = nil
        // Keine Downloads erlaubt: nichts geladen, alle ladbaren Anforderungen offen.
        speicher.alleEntfernen()
        o = AblageLaufOptionen(angefordert: [a1, unbekannt, a2])
        o.downloadsErlaubt = false
        b = try await lauf(hub, o)
        XCTAssertTrue(b.downloadsRuhen)
        XCTAssertEqual(b.offeneAnforderungen, [a1, a2])
        // Übersprungen (ein anderer Lauf hält den Speicherordner): nichts angefasst, alles offen.
        XCTAssertTrue(AblageLaufSperre.shared.betreten(speicher.wurzel.path))
        defer { AblageLaufSperre.shared.verlassen(speicher.wurzel.path) }
        b = try await lauf(hub, AblageLaufOptionen(angefordert: [a1, a1, a2]))
        XCTAssertTrue(b.uebersprungen)
        XCTAssertEqual(b.offeneAnforderungen, [a1, a2])
    }

    // MARK: Fix-Runde zu PR #453

    /// Fix 2: Ein Original-Download läuft, während ein Lauf `eingangRaeumen` ausführt: sein `.part` bleibt heil.
    func testGleichzeitigerLaufRaeumtDenPartEinesOriginalDownloadsNichtWeg() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1)])
        _ = try await lauf(hub)
        let tor = AblageTor(), drin = AblageZaehler()
        let partPfad = AblageZaehlerPfad()
        hub.blobVorher = { _, ziel in
            let part = ziel.appendingPathExtension("part")
            partPfad.setze(part)
            try Data("teil".utf8).write(to: part)
            drin.erhoehe()
            await tor.warten()
            guard FileManager.default.fileExists(atPath: part.path) else { throw AblageHubFehler.beschaedigt }   // weggeräumt
            try? FileManager.default.removeItem(at: part)
        }
        let s = speicher!, k = konto
        let dat = ws[0].datei.id
        let download = Task { try await AblageLauf.originalLaden(hub: hub, speicher: s, konto: k, dat: dat, fas: W.fasId(1), bulk: false) }
        await ablageWarteBis { drin.wert == 1 }
        // Ein ganzer Lauf (räumt den Eingang) während der Download wartet.
        _ = try await lauf(hub)
        XCTAssertTrue(fm.fileExists(atPath: try XCTUnwrap(partPfad.wert).path), "der .part des laufenden Downloads ist noch da")
        // Reste früherer Läufe gehen dabei trotzdem weg.
        try Data("alt".utf8).write(to: speicher.wurzel.appendingPathComponent("eingang/.sync-alt"))
        _ = try await lauf(hub)
        XCTAssertFalse(fm.fileExists(atPath: speicher.wurzel.appendingPathComponent("eingang/.sync-alt").path))
        tor.oeffnen()
        let url = try await download.value
        XCTAssertEqual(try Data(contentsOf: url), Data(ws[0].original))
        XCTAssertEqual(list("eingang"), [])
    }

    /// Ein Abbruch während des Abrufs schreibt nichts mehr, auch wenn der Hub danach noch antwortet.
    func testAbbruchWaehrendAbrufSchreibtDieSeiteNicht() async throws {
        let hub = AblageHubNachbau()
        bestuecke(hub, [welt(1), welt(2)])
        let tor = AblageTor(), drin = AblageZaehler()
        hub.pullHook = { _, coll in
            if coll == "ablage_datei" { drin.erhoehe(); await tor.warten() }
        }
        let s = speicher!, k = konto
        let task = Task { try await AblageLauf.ausfuehren(hub: hub, speicher: s, konto: k, optionen: AblageLaufOptionen()) }
        await ablageWarteBis { drin.wert == 1 }
        task.cancel()
        tor.oeffnen()
        do { _ = try await task.value; XCTFail() } catch is CancellationError {}
        let z = zustand()
        XCTAssertEqual(z.bestand.ordner.count, 1, "die Ordner-Seite davor ist gespeichert")
        XCTAssertTrue(z.bestand.dateien.isEmpty, "die Datei-Seite nach dem Abbruch nicht")
        XCTAssertNil(z.ledger.cursor["ablage_datei"])
    }

    /// Fix 4: Im teuren Netz unter „nur im WLAN“ kommen Textfassungen bis zum Budget auch über Mobilfunk, Originale nie.
    func testMobilesNetzLaedtTexteBisZumBudgetOhneBulkKeineOriginale() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1), welt(2, extra: String(repeating: "m", count: 100)), welt(3, extra: String(repeating: "g", count: 400))])
        var o = AblageLaufOptionen(angefordert: [AblageOriginalAnforderung(dat: ws[0].datei.id, fas: W.fasId(1))])
        o.bulk = true                                   // „nur im WLAN“ ist an
        o.mobil = true                                  // und das Netz ist teuer
        o.mobilTextBudget = ws[0].text.size + ws[1].text.size + 5   // passt für zwei, nicht für drei
        let b = try await lauf(hub, o)
        XCTAssertEqual(b.texteGeladen, 2, "die zwei kleineren Texte")
        XCTAssertEqual(b.originaleGeladen, 0, "Originale bleiben unter der WLAN-Regel")
        XCTAssertEqual(b.wartetAufWLAN, 2, "der große Text und das angeforderte Original")
        XCTAssertFalse(b.downloadsRuhen)
        XCTAssertTrue(hub.blobAufrufe.allSatisfy { !$0.bulk }, "Texte dürfen über Mobilfunk (kein bulk)")
        let v = speicher.vorhanden(zustand().bestand)
        XCTAssertTrue(v.text(ws[0].datei.id, ws[0].text.textrev) && v.text(ws[1].datei.id, ws[1].text.textrev))
        XCTAssertFalse(v.text(ws[2].datei.id, ws[2].text.textrev))
        XCTAssertFalse(v.original(ws[0].datei.id, W.fasId(1)))
        // Auf WLAN (nicht mehr mobil): der Rest kommt, jetzt mit bulk (nur WLAN).
        o.mobil = false
        let n = hub.blobAufrufe.count
        let b2 = try await lauf(hub, o)
        XCTAssertEqual(b2.texteGeladen, 1)
        XCTAssertEqual(b2.originaleGeladen, 1)
        XCTAssertEqual(b2.wartetAufWLAN, 0)
        XCTAssertTrue(hub.blobAufrufe.dropFirst(n).allSatisfy { $0.bulk })
    }

    func testMobilesNetzOhneAusnahmeFuerDieErsteDatei() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1)])
        var o = AblageLaufOptionen()
        o.bulk = true; o.mobil = true; o.mobilTextBudget = ws[0].text.size - 1      // knapp zu groß
        let b = try await lauf(hub, o)
        XCTAssertEqual(b.texteGeladen, 0, "anders als das Budget je Lauf: auch die erste Datei muss passen")
        XCTAssertEqual(b.wartetAufWLAN, 1)
        XCTAssertEqual(hub.blobAufrufe.count, 0)
        XCTAssertEqual(zustand().bestand.dateien.count, 1, "der Pull läuft trotzdem")
    }

    /// Nit: eine beiseitegelegte unlesbare Zustandsdatei (kann Namen enthalten) bleibt nur, bis ein vollständiger Pull sie ersetzt hat.
    func testDefektDateiVerschwindetNachVollstaendigemLaufBeiUnvollstaendigemNicht() async throws {
        let hub = AblageHubNachbau()
        bestuecke(hub, [welt(1)])
        let fremd = welt(2)
        hub.stelle(W.remote(datei: fremd.datei, keyId: SyncCrypto.keyId(W.andererKey), key: W.andererKey))     // macht den Lauf unvollständig
        try fm.createDirectory(at: speicher.wurzel, withIntermediateDirectories: true)
        try Data("kaputt mit Namen".utf8).write(to: speicher.wurzel.appendingPathComponent("ablage-zustand.json"))
        let defekt = speicher.wurzel.appendingPathComponent("ablage-zustand.json.defekt")
        let b = try await lauf(hub)
        XCTAssertEqual(b.zustandVerlust, .unlesbar)
        XCTAssertFalse(b.vollstaendig)
        XCTAssertTrue(fm.fileExists(atPath: defekt.path), "unvollständig: bleibt zur Diagnose")
        hub.stelle(W.remoteGrab(collection: "ablage_datei", id: W.datId(2), revision: 9))
        let b2 = try await lauf(hub)
        XCTAssertTrue(b2.vollstaendig)
        XCTAssertFalse(fm.fileExists(atPath: defekt.path), "vollständiger Pull hat den Zustand neu aufgebaut")
    }

    func testDefektDateiGehtBeimKontoWechselWeg() async throws {
        let hub = AblageHubNachbau()
        bestuecke(hub, [welt(1)])
        _ = try await lauf(hub)
        let defekt = speicher.wurzel.appendingPathComponent("ablage-zustand.json.defekt")
        try Data("Namen".utf8).write(to: defekt)
        // Anderes Konto mit einem Hub, der unvollständig bleibt (nur ein Fremdschlüssel-Objekt): .defekt trotzdem weg.
        let anderer = AblageKonto(key: W.key, userId: W.userId, hubUrl: "https://anderer-hub.example.test")
        let leer = AblageHubNachbau()
        leer.stelle(W.remote(datei: welt(2).datei, keyId: SyncCrypto.keyId(W.andererKey), key: W.andererKey))
        let b = try await AblageLauf.ausfuehren(hub: leer, speicher: speicher, konto: anderer)
        XCTAssertTrue(b.kontoGewechselt)
        XCTAssertFalse(b.vollstaendig)
        XCTAssertFalse(fm.fileExists(atPath: defekt.path))
    }
}

// MARK: - Dienst: Abmelden, vorgemerkte Originale (Fix-Runde zu PR #453)

final class AblageDienstTests: XCTestCase {
    private var tmp: URL!
    private var speicher: AblageSpeicher!
    private var dienst: AblageDienst!
    private let fm = FileManager.default
    private let konto = W.konto

    override func setUpWithError() throws {
        tmp = fm.temporaryDirectory.appendingPathComponent("ablage-dienst-\(UUID().uuidString)", isDirectory: true)
        try fm.createDirectory(at: tmp, withIntermediateDirectories: true)
        speicher = AblageSpeicher(wurzel: tmp.appendingPathComponent("ablage", isDirectory: true))
        dienst = AblageDienst(speicher: speicher)
    }

    override func tearDownWithError() throws { try? fm.removeItem(at: tmp) }

    @discardableResult
    private func bestuecke(_ hub: AblageHubNachbau, _ welten: [Welt]) -> [Welt] {
        hub.stelle(W.remote(ordner: W.ordner(1, name: "Ablage", klasse: "internal")))
        for w in welten {
            hub.stelle(W.remote(datei: w.datei))
            _ = hub.blob(w.textBytes)
            _ = hub.blob(w.original)
        }
        return welten
    }

    private func zustand() -> AblageZustand { speicher.laden(konto: konto.kennung).zustand }

    // Fix 1: Abmelden während eines Abgleichs

    func testAbmeldenWaehrendLaufWartetAufDenLaufUndHinterlaesstNichts() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1), welt(2)])
        let tor = AblageTor(), drin = AblageZaehler(), fertig = AblageZaehler()
        hub.pullHook = { _, coll in
            if coll == "ablage_datei" { drin.erhoehe(); await tor.warten() }     // reagiert nicht auf Abbruch
        }
        let d = dienst!, k = konto
        let anf = AblageOriginalAnforderung(dat: ws[0].datei.id, fas: W.fasId(1))
        let lauf = Task { try await d.laufen(hub: hub, konto: k, optionen: AblageLaufOptionen(angefordert: [anf])) }
        await ablageWarteBis { drin.wert == 1 }
        XCTAssertEqual(zustand().bestand.ordner.count, 1, "der Zustand des Kontos liegt schon auf der Platte")
        let abmelden = Task { await d.verlassen(); fertig.erhoehe() }
        try await Task.sleep(nanoseconds: 150_000_000)
        XCTAssertEqual(fertig.wert, 0, "Abmelden wartet, bis der Lauf wirklich beendet ist")
        XCTAssertTrue(fm.fileExists(atPath: speicher.wurzel.path), "nichts gelöscht, solange der Lauf noch schreiben könnte")
        // Ein neuer Lauf während des Abmeldens wird abgewiesen.
        let neu = try await d.laufen(hub: hub, konto: k, optionen: AblageLaufOptionen())
        XCTAssertTrue(neu.uebersprungen)
        tor.oeffnen()
        await abmelden.value
        do { _ = try await lauf.value; XCTFail("der abgebrochene Lauf wirft") } catch is CancellationError {}
        XCTAssertFalse(fm.fileExists(atPath: speicher.wurzel.path), "vom alten Konto bleibt nichts liegen (kein Zustand, keine Namen, keine Texte)")
        let vorgemerkt = await d.vorgemerkteOriginale
        XCTAssertEqual(vorgemerkt, [], "Anforderungen des alten Kontos werden nicht auf das nächste übertragen")
        XCTAssertEqual(hub.protokoll.filter { $0.hasPrefix("pull ablage_datei") }.count, 1, "nach dem Abbruch kein weiterer Abruf")
        // Mit dem nächsten Konto läuft alles wieder.
        hub.pullHook = nil
        let b = try await d.laufen(hub: hub, konto: k, optionen: AblageLaufOptionen())
        XCTAssertFalse(b.uebersprungen)
        XCTAssertEqual(zustand().bestand.dateien.count, 2)
    }

    func testAbmeldenWaehrendOriginalDownloadLegtNichtsAb() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1)])
        _ = try await AblageLauf.ausfuehren(hub: hub, speicher: speicher, konto: konto)
        let tor = AblageTor(), drin = AblageZaehler()
        hub.blobVorher = { _, _ in drin.erhoehe(); await tor.warten() }
        let d = dienst!, k = konto, dat = ws[0].datei.id
        let download = Task { try await d.originalLaden(hub: hub, konto: k, dat: dat, fas: W.fasId(1), bulk: false) }
        await ablageWarteBis { drin.wert == 1 }
        let abmelden = Task { await d.verlassen() }
        try await Task.sleep(nanoseconds: 100_000_000)
        XCTAssertTrue(fm.fileExists(atPath: speicher.wurzel.path))
        tor.oeffnen()
        await abmelden.value
        do { _ = try await download.value; XCTFail() } catch is CancellationError {}
        XCTAssertFalse(fm.fileExists(atPath: speicher.wurzel.path), "auch das Original wurde nicht mehr abgelegt")
        // Nach dem Abmelden werden Original-Anforderungen des alten Kontos nicht mehr angenommen, nach dem Ende geht es wieder.
        do { _ = try await dienst.originalLaden(hub: hub, konto: konto, dat: dat, fas: W.fasId(1), bulk: false); XCTFail() }
        catch let e as AblageLaufFehler { XCTAssertEqual(e, .nichtLadbar) }
    }

    func testAbmeldenOhneLaufRaeumtAllesUndIstWiederholbar() async throws {
        let hub = AblageHubNachbau()
        bestuecke(hub, [welt(1)])
        _ = try await dienst.laufen(hub: hub, konto: konto, optionen: AblageLaufOptionen())
        try Data("Namen".utf8).write(to: speicher.wurzel.appendingPathComponent("ablage-zustand.json.defekt"))
        XCTAssertTrue(fm.fileExists(atPath: speicher.wurzel.path))
        await dienst.verlassen()
        XCTAssertFalse(fm.fileExists(atPath: speicher.wurzel.path))
        await dienst.verlassen()
        let leer = await dienst.vorgemerkteOriginale
        XCTAssertEqual(leer.count, 0)
    }

    // Fix 3: Anforderungen gehen nicht verloren

    func testAnforderungWaehrendLaufWirdVorgemerktUndImFolgelaufNachgeholt() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1), welt(2)])
        let tor = AblageTor(), drin = AblageZaehler()
        hub.pullHook = { _, coll in
            if coll == "ablage_datei", drin.erhoehe() == 1 { await tor.warten() }    // nur im ersten Lauf
        }
        let d = dienst!, k = konto
        let erster = Task { try await d.laufen(hub: hub, konto: k, optionen: AblageLaufOptionen()) }
        await ablageWarteBis { drin.wert == 1 }
        let anf = AblageOriginalAnforderung(dat: ws[1].datei.id, fas: W.fasId(2))
        let zweiter = try await d.laufen(hub: hub, konto: k, optionen: AblageLaufOptionen(angefordert: [anf]))
        XCTAssertTrue(zweiter.uebersprungen, "ein Lauf ist aktiv: sofort zurück")
        let vorgemerkt = await d.vorgemerkteOriginale
        XCTAssertEqual(vorgemerkt, [anf], "die Anforderung ist nicht verloren")
        // Doppelte Anforderung wird nicht doppelt vorgemerkt.
        _ = try await d.laufen(hub: hub, konto: k, optionen: AblageLaufOptionen(angefordert: [anf]))
        let nochmal = await d.vorgemerkteOriginale
        XCTAssertEqual(nochmal.count, 1)
        tor.oeffnen()
        let b = try await erster.value
        XCTAssertFalse(b.uebersprungen)
        XCTAssertEqual(b.originaleGeladen, 1, "der Folgelauf hat das Original geholt")
        XCTAssertTrue(speicher.vorhanden(zustand().bestand).original(ws[1].datei.id, W.fasId(2)))
        XCTAssertFalse(speicher.vorhanden(zustand().bestand).original(ws[0].datei.id, W.fasId(1)))
        let rest = await d.vorgemerkteOriginale
        XCTAssertEqual(rest, [])
        XCTAssertEqual(hub.protokoll.filter { $0.hasPrefix("pull ablage_ordner") }.count, 2, "genau ein Folgelauf")
    }

    func testAnforderungBleibtNachFehlgeschlagenemLaufVorgemerkt() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1)])
        let anf = AblageOriginalAnforderung(dat: ws[0].datei.id, fas: W.fasId(1))
        hub.pullHook = { _, _ in throw AblageHubFehler.netz }
        do { _ = try await dienst.laufen(hub: hub, konto: konto, optionen: AblageLaufOptionen(angefordert: [anf])); XCTFail() }
        catch let e as AblageLaufFehler { XCTAssertEqual(e, .hub(.netz)) }
        let vorgemerkt = await dienst.vorgemerkteOriginale
        XCTAssertEqual(vorgemerkt, [anf], "bleibt für den nächsten Lauf")
        hub.pullHook = nil
        let b = try await dienst.laufen(hub: hub, konto: konto, optionen: AblageLaufOptionen())
        XCTAssertEqual(b.originaleGeladen, 1)
        let rest = await dienst.vorgemerkteOriginale
        XCTAssertEqual(rest, [])
    }

    // MARK: Nachtrag zu D5a-3, Punkt 1: Anforderungen, die ein Lauf nicht erledigt, gehen nicht verloren

    private func anforderung(_ w: Welt) -> AblageOriginalAnforderung { AblageOriginalAnforderung(dat: w.datei.id, fas: w.datei.fassungen[0].fas) }
    private func originalHubId(_ w: Welt) -> String { AblageKrypto.hubBlobId(key: W.key, sha256Hex: w.datei.fassungen[0].blob)! }

    /// Mobilfunk unter „nur im WLAN“: Originale warten, ihre Anforderung auch.
    func testMobilWartendeOriginaleBleibenVorgemerktUndKommenImWlan() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1)])
        let a = anforderung(ws[0])
        var o = AblageLaufOptionen(angefordert: [a])
        o.bulk = true; o.mobil = true
        let b = try await dienst.laufen(hub: hub, konto: konto, optionen: o)
        XCTAssertEqual(b.originaleGeladen, 0)
        XCTAssertEqual(b.wartetAufWLAN, 1)
        let vorgemerkt = await dienst.vorgemerkteOriginale
        XCTAssertEqual(vorgemerkt, [a], "die Anforderung wartet mit dem Original auf WLAN")
        // Später im WLAN (ohne neue Anforderung): der nächste Lauf holt es.
        let b2 = try await dienst.laufen(hub: hub, konto: konto, optionen: AblageLaufOptionen())
        XCTAssertEqual(b2.originaleGeladen, 1)
        let rest = await dienst.vorgemerkteOriginale
        XCTAssertEqual(rest, [])
        XCTAssertTrue(speicher.vorhanden(zustand().bestand).original(ws[0].datei.id, W.fasId(1)))
    }

    /// Budget je Lauf erschöpft (die erste Datei läuft immer): der Rest bleibt vorgemerkt.
    func testBudgetErschoepftRestDerAnforderungenBleibtVorgemerkt() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1), welt(2)])
        _ = try await dienst.laufen(hub: hub, konto: konto, optionen: AblageLaufOptionen())      // Texte vorab
        let a1 = anforderung(ws[0]), a2 = anforderung(ws[1])
        var o = AblageLaufOptionen(angefordert: [a1, a2])
        o.downloadBudget = 1
        let b = try await dienst.laufen(hub: hub, konto: konto, optionen: o)
        XCTAssertEqual(b.originaleGeladen, 1, "die erste Datei läuft immer")
        let vorgemerkt = await dienst.vorgemerkteOriginale
        XCTAssertEqual(vorgemerkt, [a2], "was das Budget nicht mehr schaffte, bleibt vorgemerkt")
        let b2 = try await dienst.laufen(hub: hub, konto: konto, optionen: AblageLaufOptionen())
        XCTAssertEqual(b2.originaleGeladen, 1)
        let rest = await dienst.vorgemerkteOriginale
        XCTAssertEqual(rest, [])
        let v = speicher.vorhanden(zustand().bestand)
        XCTAssertTrue(v.original(ws[0].datei.id, W.fasId(1)) && v.original(ws[1].datei.id, W.fasId(2)))
    }

    /// Netz weg, voller Datenträger, WLAN-Regel: die Downloads brechen ab, alle noch offenen Anforderungen bleiben vorgemerkt.
    func testDownloadsUnterbrochenAnforderungenBleibenVorgemerkt() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1), welt(2)])
        let a1 = anforderung(ws[0]), a2 = anforderung(ws[1])
        let ids: Set<String> = [originalHubId(ws[0]), originalHubId(ws[1])]
        for fehler in [AblageHubFehler.netz, .speicherVoll, .wlanRegel] {
            _ = try await dienst.laufen(hub: hub, konto: konto, optionen: AblageLaufOptionen())      // Texte vorab
            hub.blobHook = { id in if ids.contains(id) { throw fehler } }
            let b = try await dienst.laufen(hub: hub, konto: konto, optionen: AblageLaufOptionen(angefordert: [a1, a2]))
            XCTAssertTrue(b.downloadsUnterbrochen, "\(fehler)")
            let vorgemerkt = await dienst.vorgemerkteOriginale
            XCTAssertEqual(vorgemerkt, [a1, a2], "\(fehler): die abgebrochene und die noch nicht begonnene Anforderung bleiben")
            hub.blobHook = nil
            let b2 = try await dienst.laufen(hub: hub, konto: konto, optionen: AblageLaufOptionen())
            XCTAssertEqual(b2.originaleGeladen, 2, "\(fehler)")
            let rest = await dienst.vorgemerkteOriginale
            XCTAssertEqual(rest, [], "\(fehler)")
            await dienst.verlassen()                                    // frischer Stand für den nächsten Fall
        }
    }

    /// Kein Netz (`downloadsErlaubt` aus): der Lauf lädt nichts, die Anforderung wartet.
    func testDownloadsRuhenAnforderungBleibtVorgemerkt() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1)])
        let a = anforderung(ws[0])
        var o = AblageLaufOptionen(angefordert: [a])
        o.downloadsErlaubt = false
        let b = try await dienst.laufen(hub: hub, konto: konto, optionen: o)
        XCTAssertTrue(b.downloadsRuhen)
        let vorgemerkt = await dienst.vorgemerkteOriginale
        XCTAssertEqual(vorgemerkt, [a])
        let b2 = try await dienst.laufen(hub: hub, konto: konto, optionen: AblageLaufOptionen())
        XCTAssertEqual(b2.originaleGeladen, 1)
        let rest = await dienst.vorgemerkteOriginale
        XCTAssertEqual(rest, [])
    }

    /// Was nie ladbar wird (unbekannt) oder an einer einzelnen Datei scheitert (Blob fehlt), wird nicht ewig vorgemerkt.
    func testNichtLadbareAnforderungenWerdenNichtVorgemerkt() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1), welt(2)])
        let fehlt = originalHubId(ws[1])
        hub.blobHook = { id in if id == fehlt { throw AblageHubFehler.http(404, "not_found") } }
        let unbekannt = AblageOriginalAnforderung(dat: W.datId(9), fas: W.fasId(9))
        let b = try await dienst.laufen(hub: hub, konto: konto,
                                        optionen: AblageLaufOptionen(angefordert: [unbekannt, anforderung(ws[0]), anforderung(ws[1])]))
        XCTAssertEqual(b.originaleGeladen, 1)
        let vorgemerkt = await dienst.vorgemerkteOriginale
        XCTAssertEqual(vorgemerkt, [], "unbekannt und Blob-fehlt (nur diese Datei offen) kehren nicht in jedem Lauf wieder")
    }

    /// Abmelden verwirft auch Anforderungen, die ein beendeter Lauf noch offen hatte (Epoche des alten Kontos).
    func testAbmeldenVerwirftVorgemerkteAnforderungen() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1)])
        var o = AblageLaufOptionen(angefordert: [anforderung(ws[0])])
        o.bulk = true; o.mobil = true
        _ = try await dienst.laufen(hub: hub, konto: konto, optionen: o)
        let davor = await dienst.vorgemerkteOriginale
        XCTAssertEqual(davor.count, 1)
        await dienst.verlassen()
        let danach = await dienst.vorgemerkteOriginale
        XCTAssertEqual(danach, [])
        let b = try await dienst.laufen(hub: hub, konto: konto, optionen: AblageLaufOptionen())
        XCTAssertEqual(b.originaleGeladen, 0, "das neue Konto erbt keine Anforderung")
    }

    // MARK: Nachtrag, Punkt 2: nach dem Abmelden kein Bericht des alten Kontos

    /// Endet der abgebrochene Lauf mit einem anderen Fehler (hier: Netz), kommt trotzdem nur der Abbruch an: die Oberfläche
    /// zeigt nach dem Abmelden keinen Fehler des alten Kontos.
    func testAbmeldenMeldetNurAbbruchAuchWennDerLaufMitAnderemFehlerEndet() async throws {
        let hub = AblageHubNachbau()
        bestuecke(hub, [welt(1)])
        let tor = AblageTor(), drin = AblageZaehler()
        hub.pullHook = { _, coll in
            if coll == "ablage_datei" { drin.erhoehe(); await tor.warten(); throw AblageHubFehler.netz }
        }
        let d = dienst!, k = konto
        let lauf = Task { try await d.laufen(hub: hub, konto: k, optionen: AblageLaufOptionen()) }
        await ablageWarteBis { drin.wert == 1 }
        let abmelden = Task { await d.verlassen() }
        try await Task.sleep(nanoseconds: 100_000_000)
        tor.oeffnen()
        await abmelden.value
        do { _ = try await lauf.value; XCTFail("kein Ergebnis des alten Kontos") }
        catch is CancellationError {}
        catch { XCTFail("nur der Abbruch darf ankommen, nicht \(error)") }
    }

    // MARK: Nachtrag, Punkt 3: Abmelden wartet, bis wirklich nichts mehr läuft

    /// Zwei gleichzeitige Abmeldungen und ein Aufruf direkt nach dem Abmelden: nichts geht verloren, nichts löscht das neue Konto.
    func testZweiAbmeldungenGleichzeitigUndAufrufDanachVerlierenNichts() async throws {
        let d = dienst!, k = konto
        for runde in 0..<25 {
            let hub = AblageHubNachbau()
            bestuecke(hub, [welt(1)])
            let tor = AblageTor(), drin = AblageZaehler()
            hub.pullHook = { _, coll in if coll == "ablage_datei", drin.erhoehe() == 1 { await tor.warten() } }
            // Der Lauf mit niedriger Priorität: seine Fortsetzung auf dem Akteur kommt nach der des Abmeldens (Fenster).
            let lauf = Task(priority: .background) { try await d.laufen(hub: hub, konto: k, optionen: AblageLaufOptionen()) }
            await ablageWarteBis { drin.wert == 1 }
            let a = Task(priority: .userInitiated) { await d.verlassen() }
            let b = Task(priority: .userInitiated) { await d.verlassen() }
            try await Task.sleep(nanoseconds: 20_000_000)
            let waehrend = try await d.laufen(hub: hub, konto: k, optionen: AblageLaufOptionen())
            XCTAssertTrue(waehrend.uebersprungen, "Runde \(runde): beim Abmelden startet kein Lauf")
            XCTAssertTrue(fm.fileExists(atPath: speicher.wurzel.path), "Runde \(runde): erst löschen, wenn nichts mehr läuft")
            tor.oeffnen()
            await a.value
            // Der Aufruf nach dem Abmelden findet keinen aktiven Lauf mehr: er läuft selbst (kein „übersprungen“, kein verlorenes Folgen).
            let neu = try await d.laufen(hub: hub, konto: k, optionen: AblageLaufOptionen())
            XCTAssertFalse(neu.uebersprungen, "Runde \(runde): nach dem Abmelden ist niemand mehr aktiv")
            await b.value
            XCTAssertEqual(zustand().bestand.dateien.count, 1, "Runde \(runde): das zweite Abmelden löscht nicht das neue Konto")
            do { _ = try await lauf.value; XCTFail("Runde \(runde)") } catch is CancellationError {}
            await d.verlassen()
        }
    }

    // MARK: Nachtrag, Punkt 4: kein Original einer begrabenen oder angehaltenen Datei bleibt liegen

    /// Während des Downloads wird die Datei angehalten: das Original wird nicht abgelegt.
    func testOriginalEinerWaehrendDesDownloadsAngehaltenenDateiBleibtNichtLiegen() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1)])
        _ = try await dienst.laufen(hub: hub, konto: konto, optionen: AblageLaufOptionen())
        let tor = AblageTor(), drin = AblageZaehler()
        hub.blobVorher = { _, _ in drin.erhoehe(); await tor.warten() }
        let d = dienst!, k = konto, dat = ws[0].datei.id
        let download = Task { try await d.originalLaden(hub: hub, konto: k, dat: dat, fas: W.fasId(1), bulk: false) }
        await ablageWarteBis { drin.wert == 1 }
        var z = zustand()
        z.ledger.objekte[AblageLedger.schluessel("ablage_datei", dat)]?.angehalten = true
        try speicher.speichern(z)
        tor.oeffnen()
        do { _ = try await download.value; XCTFail("angehalten: nicht ladbar") } catch let e as AblageLaufFehler { XCTAssertEqual(e, .nichtLadbar) }
        XCTAssertFalse(speicher.vorhanden(zustand().bestand).original(dat, W.fasId(1)))
        XCTAssertEqual((try? fm.contentsOfDirectory(atPath: speicher.wurzel.appendingPathComponent("dateien").path)) ?? [], [])
        XCTAssertEqual((try? fm.contentsOfDirectory(atPath: speicher.wurzel.appendingPathComponent("eingang").path)) ?? [], [])
    }

    /// Während des Downloads wird die Datei begraben (ein Lauf verarbeitet den Grabstein): nichts bleibt liegen.
    func testOriginalEinerWaehrendDesDownloadsBegrabenenDateiBleibtNichtLiegen() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1)])
        _ = try await dienst.laufen(hub: hub, konto: konto, optionen: AblageLaufOptionen())
        let tor = AblageTor(), drin = AblageZaehler()
        hub.blobVorher = { _, _ in drin.erhoehe(); await tor.warten() }
        let d = dienst!, k = konto, dat = ws[0].datei.id
        let download = Task { try await d.originalLaden(hub: hub, konto: k, dat: dat, fas: W.fasId(1), bulk: false) }
        await ablageWarteBis { drin.wert == 1 }
        hub.stelle(W.remoteGrab(collection: "ablage_datei", id: dat, revision: 99))
        let b = try await dienst.laufen(hub: hub, konto: konto, optionen: AblageLaufOptionen())
        XCTAssertEqual(b.begraben, 1)
        tor.oeffnen()
        do { _ = try await download.value; XCTFail("begraben: nicht ladbar") } catch let e as AblageLaufFehler { XCTAssertEqual(e, .nichtLadbar) }
        XCTAssertEqual((try? fm.contentsOfDirectory(atPath: speicher.wurzel.appendingPathComponent("dateien").path)) ?? [], [])
        XCTAssertEqual((try? fm.contentsOfDirectory(atPath: speicher.wurzel.appendingPathComponent("eingang").path)) ?? [], [])
    }

    /// Die Lücke im Lauf: Dateien der begrabenen Datei sind weg, der Zustand auf der Platte ist noch der alte. Ein Download,
    /// der genau dann den Zustand liest, darf nichts ablegen: er wartet an der Sperre des Speichers, bis der Zustand
    /// gespeichert ist, und sieht dann den Grabstein.
    func testOriginalImFensterZwischenEntfernenUndSpeichernWartetUndBleibtNichtLiegen() async throws {
        let hub = AblageHubNachbau()
        let ws = bestuecke(hub, [welt(1)])
        _ = try await dienst.laufen(hub: hub, konto: konto, optionen: AblageLaufOptionen())
        let dat = ws[0].datei.id
        var z = zustand()
        _ = AblagePullVerarbeitung.seite(sammlung: "ablage_datei", objekte: [W.remoteGrab(collection: "ablage_datei", id: dat, revision: 99)],
                                         konto: konto, zustand: &z)
        let begraben = z, s = speicher!
        let drin = AblageZaehler(), frei = DispatchSemaphore(value: 0)
        // Wie `sammlungZiehen`: unter der Sperre erst die Dateien entfernen, dann den Zustand speichern; dazwischen ist Pause.
        DispatchQueue.global().async {
            try? s.exklusiv {
                s.entfernen(dateien: [dat])
                drin.erhoehe()
                frei.wait()
                try s.speichern(begraben)
            }
        }
        await ablageWarteBis { drin.wert == 1 }
        let fertig = AblageZaehler()
        let d = dienst!, k = konto
        let download = Task { defer { fertig.erhoehe() }; return try await d.originalLaden(hub: hub, konto: k, dat: dat, fas: W.fasId(1), bulk: false) }
        try await Task.sleep(nanoseconds: 300_000_000)
        XCTAssertEqual(fertig.wert, 0, "der Download liest den Zustand nicht in der Lücke, er wartet an der Sperre")
        frei.signal()
        do { _ = try await download.value; XCTFail("begraben: nicht ladbar") } catch let e as AblageLaufFehler { XCTAssertEqual(e, .nichtLadbar) }
        XCTAssertEqual((try? fm.contentsOfDirectory(atPath: speicher.wurzel.appendingPathComponent("dateien").path)) ?? [], [], "kein verwaistes Original")
        XCTAssertEqual((try? fm.contentsOfDirectory(atPath: speicher.wurzel.appendingPathComponent("eingang").path)) ?? [], [])
    }

    // MARK: Ersatz für den Lauf (Punkt 2, Lauf bemerkt den Abbruch nicht)

    /// Ein Lauf, der den Abbruch nicht bemerkt und trotzdem mit einem Bericht endet, liefert nach dem Abmelden keinen Bericht
    /// des alten Kontos (die Oberfläche setzte sonst „abgeglichen“) und merkt sich nichts vor.
    func testLaufDerNachDemAbmeldenFertigWirdMeldetKeinenBericht() async throws {
        let tor = AblageTor(), drin = AblageZaehler()
        let anf = AblageOriginalAnforderung(dat: W.datId(1), fas: W.fasId(1))
        let d = AblageDienst(speicher: speicher, lauf: { _, _, _, o in
            drin.erhoehe()
            await tor.warten()                                            // bemerkt den Abbruch nicht
            var b = AblageBericht()
            b.vollstaendig = true
            b.offeneAnforderungen = o.angefordert
            return b
        })
        let hub = AblageHubNachbau(), k = konto
        let lauf = Task { try await d.laufen(hub: hub, konto: k, optionen: AblageLaufOptionen(angefordert: [anf])) }
        await ablageWarteBis { drin.wert == 1 }
        let abmelden = Task { await d.verlassen() }
        try await Task.sleep(nanoseconds: 100_000_000)
        tor.oeffnen()
        await abmelden.value
        do { _ = try await lauf.value; XCTFail("kein Bericht des alten Kontos") }
        catch is CancellationError {}
        catch { XCTFail("nur der Abbruch darf ankommen, nicht \(error)") }
        let vorgemerkt = await d.vorgemerkteOriginale
        XCTAssertEqual(vorgemerkt, [], "auch die offene Anforderung des alten Kontos verfällt")
        // Ohne Abmelden kommt derselbe Bericht an und seine offenen Anforderungen bleiben vorgemerkt.
        let b = try await d.laufen(hub: hub, konto: k, optionen: AblageLaufOptionen(angefordert: [anf]))
        XCTAssertTrue(b.vollstaendig)
        let bleibt = await d.vorgemerkteOriginale
        XCTAssertEqual(bleibt, [anf])
    }
}

/// Ein Pfad, den ein Test-Hook setzt (Sendable, Sperre).
final class AblageZaehlerPfad: @unchecked Sendable {
    private let lock = NSLock()
    private var _wert: URL?
    func setze(_ u: URL) { lock.withLock { _wert = u } }
    var wert: URL? { lock.withLock { _wert } }
}

/// Zähler für Test-Hooks (Sendable, Sperre).
final class AblageZaehler: @unchecked Sendable {
    private let lock = NSLock()
    private var n = 0
    /// Erhöht und liefert den neuen Wert.
    @discardableResult func erhoehe() -> Int { lock.withLock { n += 1; return n } }
    var wert: Int { lock.withLock { n } }
}
#endif
