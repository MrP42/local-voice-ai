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
        guard let data = mit({ _blobs[hubId] }) else { throw AblageHubFehler.http(404, "not_found") }
        guard UInt64(data.count) == groesse else { throw AblageHubFehler.beschaedigt }
        try Data(data).write(to: ziel)
    }
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
        let ausfall = Counter()
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
        let gesehen = Counter()
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
            let aufrufe = Counter()
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
}

/// Zähler für Test-Hooks (Sendable, Sperre).
final class Counter: @unchecked Sendable {
    private let lock = NSLock()
    private var n = 0
    /// Erhöht und liefert den neuen Wert.
    @discardableResult func erhoehe() -> Int { lock.lock(); defer { lock.unlock() }; n += 1; return n }
    var wert: Int { lock.lock(); defer { lock.unlock() }; return n }
}
#endif
