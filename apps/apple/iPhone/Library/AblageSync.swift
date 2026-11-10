import Foundation
import Security

/// Lesender Abgleich der Dokumentenablage mit dem Hub (D5a-3, lva#441, Epic #395), nach
/// docs/superpowers/specs/2026-11-08-dokumentenablage.md §6 „iPhone (D5)“. Neben `LibrarySync`: dasselbe Konto, derselbe
/// Schlüssel, dieselbe HTTP-Infrastruktur (`LibrarySync.send`, `NoRedirects`, `fetchBlob` mit Stücken, Hash und
/// Wiederaufnahme, `wifiOnly`, `expensiveNetwork`). Die Ablauflogik steht in VoiceCore (`AblageLauf`, testbar ohne Netz);
/// diese Datei ist der HTTP-Weg dorthin und der Zustand für die Oberfläche.
///
/// - Die Ablage wird nur mit `collection=` und eigenem Cursor je Sammlung gezogen, nie über den globalen Pull.
/// - Texte werden vorab geladen, Originale nur auf ausdrückliche Anforderung (`originalLaden`) und nur nach der Regel
///   „nur im WLAN“ (`LibrarySync.wifiOnly`). Textfassungen kommen auch im teuren Netz (Mobilfunk, Datensparmodus) unter
///   „nur im WLAN“, aber höchstens 5 MiB je Lauf; was darüber liegt, wartet auf WLAN (`AblageLaufOptionen.mobil`).
/// - Läufe, Original-Downloads und Abmelden koordiniert `AblageDienst` (VoiceCore): Abmelden wartet auf laufende Arbeit, bevor
///   es löscht; Anforderungen während eines Laufs gehen nicht verloren.
/// - Die Ablage liegt im App-Container (Application Support), nie im Bibliothek-Ordner. Kein Logging und keine Fehlertexte
///   mit Namen oder Inhalten (Spec §10).
/// - Kein Push, keine Blob-Löschung: das iPhone liest in D5a nur.
/// Die Seite „Ablage“ und der automatische Lauf nach dem Bibliothek-Abgleich folgen mit D5a-4; bis dahin startet
/// niemand `run()` von selbst (Debug-Startargument `--ablage-test`).

/// Der HTTP-Weg des lesenden Abgleichs.
struct AblageHubHTTP: AblageHub {
    let account: LibrarySync.Account
    let sync: LibrarySync

    func pull(since: Int64, limit: Int, collection: String) async throws -> AblagePullSeite {
        guard let sammlung = collection.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed) else { throw AblageHubFehler.beschaedigt }
        let url = account.url + "/api/v1/voice/sync/pull?since=\(since)&limit=\(limit)&collection=\(sammlung)"
        let code: Int, data: Data
        do { (code, data) = try await LibrarySync.send(url, method: "GET", token: account.token) } catch { throw Self.uebersetzt(error) }
        switch code {
        case 200:
            do { return try AblagePullSeite(bytes: [UInt8](data)) } catch { throw AblageHubFehler.beschaedigt }
        case 401: throw AblageHubFehler.abgemeldet
        case 410: throw AblageHubFehler.cursorAbgelaufen
        default: throw AblageHubFehler.http(code, Self.fehlercode(data))
        }
    }

    func blobLaden(hubId: String, groesse: UInt64, ziel: URL, bulk: Bool) async throws {
        guard let size = Int(exactly: groesse) else { throw AblageHubFehler.beschaedigt }
        let ref = BlobRef(blob: hubId, size: size, chunks: LibrarySyncFormat.chunkCount(size: size))
        do { try await sync.fetchBlob(ref, account, to: ziel, bulk: bulk) { _ in } } catch { throw Self.uebersetzt(error) }
    }

    /// Fehlercode des Hubs (`{"error":{"code":…}}`), nur Kleinbuchstaben, Ziffern und Unterstrich, höchstens 64 Zeichen.
    static func fehlercode(_ data: Data) -> String {
        let code = ((try? JSONSerialization.jsonObject(with: data)) as? [String: Any])
            .flatMap { $0["error"] as? [String: Any] }.flatMap { $0["code"] as? String } ?? ""
        return String(code.unicodeScalars.filter { ($0.value >= 0x61 && $0.value <= 0x7A) || ($0.value >= 0x30 && $0.value <= 0x39) || $0 == "_" }.prefix(64))
    }

    /// Bildet jeden Fehler des Transports auf die Fehler des Kerns ab. Abbruch bleibt Abbruch.
    static func uebersetzt(_ error: Error) -> Error {
        if error is CancellationError || error is AblageHubFehler { return error }
        if let u = error as? URLError {
            if u.code == .cancelled { return CancellationError() }
            // „Nur im WLAN“: die Anfrage hatte Mobilfunk und Datensparmodus ausgeschlossen.
            if u.networkUnavailableReason != nil { return AblageHubFehler.wlanRegel }
            return AblageHubFehler.netz
        }
        if let s = error as? LibrarySync.SyncError {
            switch s {
            case .http(401, _), .notLoggedIn: return AblageHubFehler.abgemeldet
            case .http(410, _): return AblageHubFehler.cursorAbgelaufen
            case .http(let code, _): return AblageHubFehler.http(code, "")
            case .offlineRule: return AblageHubFehler.wlanRegel
            case .wrongKey, .corrupt: return AblageHubFehler.beschaedigt
            }
        }
        if error is SyncCryptoError { return AblageHubFehler.beschaedigt }
        let ns = error as NSError
        if ns.domain == NSCocoaErrorDomain, [NSFileWriteOutOfSpaceError, NSFileWriteVolumeReadOnlyError, NSFileWriteNoPermissionError].contains(ns.code) {
            return AblageHubFehler.speicherVoll
        }
        if ns.domain == NSPOSIXErrorDomain, ns.code == Int(ENOSPC) || ns.code == Int(EDQUOT) || ns.code == Int(EROFS) { return AblageHubFehler.speicherVoll }
        return AblageHubFehler.beschaedigt
    }
}

@MainActor
final class AblageSync: ObservableObject {
    static let shared = AblageSync()

    @Published private(set) var running = false
    /// Feste Texte, nie Namen oder Inhalte.
    @Published private(set) var status = "Ablage: noch nicht abgeglichen"
    @Published private(set) var lastRun: Date?
    @Published private(set) var bericht: AblageBericht?
    @Published private(set) var error: String?
    /// Ordner und Dateien mit Zustand für die Seite „Ablage“ (D5a-4).
    @Published private(set) var uebersicht: AblageUebersicht?

    let speicher: AblageSpeicher
    private let dienst: AblageDienst
    private let sync: LibrarySync

    /// `Application Support/Ablage` im App-Container.
    static var standardWurzel: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("Ablage", isDirectory: true)
    }

    init(sync: LibrarySync? = nil, wurzel: URL? = nil) {
        self.sync = sync ?? .shared
        let sp = AblageSpeicher(wurzel: wurzel ?? Self.standardWurzel)
        speicher = sp
        dienst = AblageDienst(speicher: sp)
    }

    /// Ordner anlegen und aus Sicherungen ausnehmen: alles hier ist aus dem Konto neu ladbar (D5a). D5b (eigene, noch nicht
    /// gesendete Importe) muss das neu bewerten.
    private func wurzelVorbereiten() {
        try? FileManager.default.createDirectory(at: speicher.wurzel, withIntermediateDirectories: true)
        var werte = URLResourceValues()
        werte.isExcludedFromBackup = true
        var url = speicher.wurzel
        try? url.setResourceValues(werte)
    }

    private func konto(_ acc: LibrarySync.Account) -> AblageKonto {
        AblageKonto(key: [UInt8](acc.key), userId: acc.userId, hubUrl: acc.url)
    }

    // MARK: Abgleich

    /// Ein Lauf: Pull je Sammlung (Ordner vor Dateien), Texte vorab, `angefordert` (Originale) danach. Ohne Konto passiert
    /// nichts. Unter „nur im WLAN“ und im teuren Netz (Mobilfunk) kommen nur Textfassungen bis 5 MiB, der Rest wartet auf WLAN;
    /// der Pull läuft immer (er ist klein). Ein Aufruf während eines Laufs kehrt sofort zurück: seine Anforderungen werden
    /// vorgemerkt und in einem Folgelauf nachgeholt, und der laufende Aufruf meldet das Ergebnis.
    func run(angefordert: [AblageOriginalAnforderung] = []) async {
        guard let acc = sync.account else { status = "Ablage: nicht angemeldet"; return }
        let erster = !running
        let vorher = status
        if erster {
            running = true; error = nil; status = "Ablage: Abgleich …"
            wurzelVorbereiten()
        }
        defer { if erster { running = false } }
        var optionen = AblageLaufOptionen(angefordert: angefordert)
        optionen.bulk = sync.wifiOnly
        optionen.mobil = sync.wifiOnly && sync.expensiveNetwork
        do {
            let b = try await dienst.laufen(hub: AblageHubHTTP(account: acc, sync: sync), konto: konto(acc), optionen: optionen)
            if b.uebersprungen {
                // Ein anderer Lauf ist aktiv und meldet das Ergebnis, oder die Ablage wird gerade geleert: nichts melden,
                // der Status bleibt, wie er war (nicht auf „Abgleich …“ hängen).
                if erster { status = vorher }
                return
            }
            bericht = b
            lastRun = Date()
            status = b.downloadsRuhen ? "Ablage: abgeglichen, Dateien warten"
                : b.downloadsUnterbrochen ? "Ablage: abgeglichen, Dateien unvollständig"
                : b.wartetAufWLAN > 0 ? "Ablage: abgeglichen, \(b.wartetAufWLAN) Dateien warten auf WLAN" : "Ablage: abgeglichen"
            await aktualisiere()
        } catch let e as AblageLaufFehler {
            error = e.localizedDescription
            status = e == .abgemeldet ? "Ablage: abgemeldet" : "Ablage: Abgleich unterbrochen"
            await aktualisiere()
        } catch is CancellationError {
            if sync.account != nil { status = "Ablage: Abgleich abgebrochen" }     // beim Abmelden setzt `kontoVerlassen` den Status
        } catch {
            self.error = "Ablage: Abgleich fehlgeschlagen."
            status = "Ablage: Abgleich unterbrochen"
        }
    }

    /// Lädt ein Original auf ausdrückliche Anforderung (Vorschau, Öffnen, Teilen) und gibt den Pfad zurück. Gilt die Regel
    /// „nur im WLAN“ und das Gerät ist im teuren Netz (Mobilfunk), wird nichts geladen (`AblageLaufFehler.wlanRegel`).
    func originalLaden(dat: String, fas: String) async throws -> URL {
        guard let acc = sync.account else { throw AblageLaufFehler.abgemeldet }
        if sync.wifiOnly && sync.expensiveNetwork { throw AblageLaufFehler.wlanRegel }
        let url = try await dienst.originalLaden(hub: AblageHubHTTP(account: acc, sync: sync), konto: konto(acc), dat: dat, fas: fas,
                                                 bulk: sync.wifiOnly, freierPlatz: { Self.freierPlatz() })
        await aktualisiere()
        return url
    }

    /// Freier Platz für wichtige Nutzung in Bytes (`nil`: unbekannt).
    nonisolated static func freierPlatz() -> UInt64? {
        let v = try? FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey])
        return v?.volumeAvailableCapacityForImportantUsage.flatMap { $0 >= 0 ? UInt64($0) : nil }
    }

    /// Liest die Übersicht neu (abseits des Haupt-Threads).
    func aktualisiere() async {
        guard let acc = sync.account else { uebersicht = nil; return }
        let speicher = self.speicher, kennung = konto(acc).kennung
        uebersicht = await Task.detached(priority: .utility) { () -> AblageUebersicht in
            let z = speicher.laden(konto: kennung).zustand
            return AblageAnzeige.uebersicht(z, vorhanden: speicher.vorhanden(z.bestand))
        }.value
    }

    /// Beim Abmelden: alles, was dieses Konto geladen hat, vom Gerät nehmen (Texte, Originale, Zustand). In D5a ist alles
    /// aus dem Konto neu ladbar. D5b (eigene Importe) muss noch nicht gesendete Importe hier ausnehmen.
    /// Bricht laufende Läufe und Downloads ab und WARTET, bis sie beendet sind, bevor es löscht (`AblageDienst.verlassen`):
    /// sonst könnte ein Lauf nach dem Löschen den Zustand des alten Kontos neu schreiben.
    func kontoVerlassen() async {
        await dienst.verlassen()
        bericht = nil; uebersicht = nil; lastRun = nil; error = nil
        status = "Ablage: noch nicht abgeglichen"
    }

    // MARK: Diagnose für Simulator-Proben

    #if DEBUG
    /// `--ablage-test`: wartet auf das Konto (Kopplung oder `--sync-test`), gleicht ab und schreibt
    /// `Documents/ablage-testbericht.json`. Mit `--ablage-test-original` wird zusätzlich das Original der ersten lebenden Datei
    /// geladen. Nur Debug-Bau und nur mit synthetischen Testdaten; nie ein Produktpfad (der Bericht enthält Namen).
    func debugLauf(arguments args: [String]) async {
        for _ in 0..<240 where sync.account == nil { try? await Task.sleep(nanoseconds: 500_000_000) }
        guard sync.account != nil else { schreibeDebug(["fehler": "kein Konto"]); return }
        if args.contains("--ablage-test-neu") { await kontoVerlassen() }
        await run()
        var geladen: [String: Any] = [:]
        if args.contains("--ablage-test-original"), let d = uebersicht?.dateien.first(where: { !$0.imKorb && $0.zustand != .angehalten && $0.aktuelleFassung != nil }),
           let fas = d.aktuelleFassung {
            do {
                let url = try await originalLaden(dat: d.id, fas: fas)
                geladen = ["datei": d.id, "pfad": url.lastPathComponent, "bytes": (try? Data(contentsOf: url).count) ?? -1]
            } catch let e as AblageLaufFehler { geladen = ["datei": d.id, "fehler": "\(e)"] } catch { geladen = ["datei": d.id, "fehler": "unbekannt"] }
        }
        await aktualisiere()
        schreibeDebug(debugBericht(original: geladen))
    }

    private func debugBericht(original: [String: Any]) -> [String: Any] {
        var out: [String: Any] = ["status": status, "fehler": error ?? ""]
        if let b = bericht {
            out["bericht"] = ["vollstaendig": b.vollstaendig, "geaendert": b.geaendert, "seiten": b.seiten, "neustarts": b.neustarts,
                              "texteGeladen": b.texteGeladen, "originaleGeladen": b.originaleGeladen, "begraben": b.begraben,
                              "angehalten": b.angehalten, "downloadsRuhen": b.downloadsRuhen, "downloadsUnterbrochen": b.downloadsUnterbrochen,
                              "warnungen": b.warnungen.map { "\($0)" }, "kontoGewechselt": b.kontoGewechselt] as [String: Any]
        }
        if let u = uebersicht {
            out["ordner"] = u.ordner.map { ["id": $0.id, "name": $0.name, "klasse": $0.klasse.text, "zustand": $0.zustand.rawValue, "imKorb": $0.imKorb] as [String: Any] }
            out["dateien"] = u.dateien.map { ["id": $0.id, "name": $0.name, "ordner": $0.ordner, "klasse": $0.klasse.text, "zustand": $0.zustand.rawValue,
                                              "imKorb": $0.imKorb, "koepfe": $0.koepfe, "textBereit": $0.textBereit, "originalVorhanden": $0.originalVorhanden,
                                              "typ": $0.typ ?? ""] as [String: Any] }
            out["angehalten"] = u.angehalten
        }
        if let acc = sync.account {
            let z = speicher.laden(konto: konto(acc).kennung).zustand
            out["indexQuellen"] = AblageAnzeige.indexQuellen(z, vorhanden: speicher.vorhanden(z.bestand))
            out["begrabeneKennungen"] = z.ledger.begraben.sortiert
            out["cursor"] = ["ordner": z.ledger.cursor["ablage_ordner"] ?? 0, "datei": z.ledger.cursor["ablage_datei"] ?? 0]
        }
        out["original"] = original
        return out
    }

    private func schreibeDebug(_ obj: [String: Any]) {
        let url = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("ablage-testbericht.json")
        if let data = try? JSONSerialization.data(withJSONObject: obj, options: [.sortedKeys, .prettyPrinted]) { try? data.write(to: url, options: .atomic) }
    }
    #endif
}
