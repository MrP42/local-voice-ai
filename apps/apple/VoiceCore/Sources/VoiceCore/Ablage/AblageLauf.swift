import Foundation
#if canImport(CryptoKit)

// Lesender Abgleich der Dokumentenablage auf dem iPhone (D5a-3, lva#441, Epic #395).
//
// Port der Lesehälfte von `apps/local-voice/src-tauri/src/sync/ablage.rs` (`lauf`, `pull`, `herunterladen`): Pull je
// Sammlung mit eigenem Cursor, Ordner vor Dateien, 410 = einmal von 0 neu, Zustand je Seite atomar, Ordnergräber nach
// dem Datei-Pull, danach Texte vorab; Originale nur auf ausdrückliche Anforderung. Kein Push, keine Blob-Löschung.
//
// Der Hub steckt hinter `AblageHub`: die App setzt HTTP ein (`AblageSync`), die Tests einen Nachbau. So läuft die ganze
// Ablauflogik ohne Netz unter `swift test`. Nichts hier protokolliert Namen oder Inhalte (Spec §10).
//
// Fehlerfälle: Abbruch mitten im Vorgang (Task abgebrochen, App beendet): der Zustand liegt je Seite atomar auf
// der Platte, der Cursor rückt erst nach dem Speichern vor, ein Wiederanlauf setzt dort fort; Downloads laufen über
// `eingang/` und werden erst nach Prüfung abgelegt. Schreibfehler/voller Datenträger: der Lauf bricht ab, ohne den
// Cursor vorzurücken (Pull) bzw. stoppt die Downloads sichtbar (Datei). Gleichzeitige Läufe: je Speicherordner läuft
// höchstens einer, ein zweiter kehrt sofort mit `uebersprungen` zurück.

public enum AblageHubFehler: Error, Equatable, Sendable {
    /// HTTP 410: der Cursor ist abgelaufen.
    case cursorAbgelaufen
    /// HTTP 401: abgemeldet.
    case abgemeldet
    /// Anderer HTTP-Status mit dem Fehlercode des Hubs (nie Inhalt).
    case http(Int, String)
    /// Verbindung, Zeitüberschreitung, kein Netz.
    case netz
    /// Antwort unlesbar oder Prüfsumme/Größe eines Stücks stimmt nicht.
    case beschaedigt
    /// „Nur im WLAN“ verbietet den Transfer.
    case wlanRegel
    /// Lokal nicht schreibbar (Datenträger voll, Schreibschutz) beim Ablegen eines Stücks.
    case speicherVoll
}

/// Die Routen, die der lesende Abgleich braucht (`AblageHub` am Desktop ohne Push, Reservieren und Löschen).
public protocol AblageHub: Sendable {
    /// `GET /api/v1/voice/sync/pull?since&limit&collection`; 410 = `AblageHubFehler.cursorAbgelaufen`, 401 = `.abgemeldet`.
    func pull(since: Int64, limit: Int, collection: String) async throws -> AblagePullSeite
    /// Lädt den Blob mit der Hub-Kennung in Stücken und legt den entschlüsselten Klartext unter `ziel` ab; prüft jedes
    /// Stück und die Gesamtgröße (`groesse`). Die Prüfung gegen den Klartext-Hash macht der Aufrufer. `bulk`: Dateiverkehr
    /// unter „nur im WLAN“ (kein Mobilfunk, kein Datensparmodus). Bei einem Fehler darf `ziel` (oder `ziel.part`) liegen
    /// bleiben, der Aufrufer räumt auf.
    func blobLaden(hubId: String, groesse: UInt64, ziel: URL, bulk: Bool) async throws
}

public enum AblageLaufFehler: Error, Equatable, Sendable {
    case abgemeldet
    case hub(AblageHubFehler)
    /// Der Zustand ließ sich nicht speichern (Datenträger voll, Schreibschutz).
    case speicher
    /// Eine Seite mit `more`, aber ohne Cursor-Fortschritt.
    case ohneFortschritt
    /// Mehr als 1000 Seiten je Sammlung.
    case zuVieleSeiten
    /// Datei oder Fassung unbekannt, angehalten, begraben oder der Verweis ungültig.
    case nichtLadbar
    /// Die geladene Datei passt nicht zu ihrem Eintrag (Größe oder Hash).
    case passtNicht
    /// Zu wenig freier Platz für das Original.
    case speicherVoll
    case wlanRegel
}

extension AblageLaufFehler: LocalizedError {
    /// Feste Texte, nie Namen oder Inhalte.
    public var errorDescription: String? {
        switch self {
        case .abgemeldet: return "Die Anmeldung ist abgelaufen oder wurde widerrufen."
        case .hub(.cursorAbgelaufen): return "Ablage: Abgleichsstand des Servers abgelaufen."
        case .hub(.netz): return "Ablage: Server nicht erreichbar."
        case .hub(.http(let code, _)): return "Ablage: Der Server antwortet mit Fehler \(code)."
        case .hub: return "Ablage: Antwort des Servers unlesbar."
        case .speicher: return "Ablage: Abgleichsstand nicht speicherbar."
        case .ohneFortschritt, .zuVieleSeiten: return "Ablage: Abgleich ohne Fortschritt."
        case .nichtLadbar: return "Ablage: Datei nicht verfügbar."
        case .passtNicht: return "Ablage: Datei passt nicht zu ihrem Eintrag."
        case .speicherVoll: return "Ablage: Nicht genug freier Speicher."
        case .wlanRegel: return "Originale werden nur im WLAN geladen (Einstellung in „Konto & Sync“)."
        }
    }
}

public struct AblageLaufOptionen: Sendable {
    /// `false`: nur Pull, keine Downloads (z. B. kein Netz).
    public var downloadsErlaubt = true
    /// Dateiverkehr unter „nur im WLAN“ (an den Hub-Weg gereicht: kein Mobilfunk, kein Datensparmodus).
    public var bulk = false
    /// Teures Netz unter „nur im WLAN“ (Mobilfunk oder Datensparmodus): Textfassungen kommen trotzdem, aber höchstens
    /// `mobilTextBudget` je Lauf (dafür ohne `bulk`, also auch im Mobilfunk); alles darüber und alle Originale warten auf WLAN.
    public var mobil = false
    public var mobilTextBudget = AblageDownloadBudget.mobilText
    /// Klartext-Bytes je Lauf zum Herunterladen (mindestens eine Datei).
    public var downloadBudget = AblageDownloadBudget.standard
    /// Originale, die in diesem Lauf (nach den Texten) geladen werden sollen.
    public var angefordert: [AblageOriginalAnforderung] = []
    /// Seitengröße des Pulls (`PULL_LIMIT`).
    public var pullLimit = 200
    /// Seiten je Sammlung, bevor der Lauf abbricht (`PULL_MAX_SEITEN`).
    public var maxSeiten = 1000

    public init(angefordert: [AblageOriginalAnforderung] = []) { self.angefordert = angefordert }
}

public struct AblageBericht: Equatable, Sendable {
    /// Ein anderer Lauf war aktiv; dieser hat nichts getan.
    public var uebersprungen = false
    /// Beide Sammlungen sind bis zum Ende gezogen und kein fremder Schlüssel ist aufgetaucht.
    public var vollstaendig = false
    public var geaendert = false
    public var kontoGewechselt = false
    public var zustandVerlust: AblageZustand.Verlust = .keiner
    public var warnungen: [AblageWarnung] = []
    public var seiten = 0
    /// Pull-Neustarts nach 410.
    public var neustarts = 0
    public var texteGeladen = 0
    public var originaleGeladen = 0
    public var begraben = 0
    /// Schlüssel (`<sammlung>/<id>`) der Objekte, die dieser Lauf angehalten hat.
    public var angehalten: [String] = []
    /// Downloads liefen nicht, weil `downloadsErlaubt` aus war.
    public var downloadsRuhen = false
    /// Aufträge (Texte über dem Mobilfunk-Budget, angeforderte Originale), die im teuren Netz auf WLAN warten.
    public var wartetAufWLAN = 0
    /// Downloads brachen ab (Netz weg, Speicher voll); der nächste Lauf versucht es erneut.
    public var downloadsUnterbrochen = false

    public init() {}

    mutating func warne(_ w: AblageWarnung) { if !warnungen.contains(w) { warnungen.append(w) } }
}

/// Höchstens ein Lauf je Speicherordner im Prozess. Wird nie über ein `await` gehalten (nur Mitgliedschaft).
final class AblageLaufSperre: @unchecked Sendable {
    static let shared = AblageLaufSperre()
    private let lock = NSLock()
    private var aktiv = Set<String>()

    func betreten(_ pfad: String) -> Bool {
        lock.lock(); defer { lock.unlock() }
        return aktiv.insert(pfad).inserted
    }

    func verlassen(_ pfad: String) {
        lock.lock(); defer { lock.unlock() }
        aktiv.remove(pfad)
    }
}

public enum AblageLauf {
    /// Reserve, die beim Prüfen des freien Platzes vor einem Original zusätzlich frei bleiben muss.
    public static let platzReserve: UInt64 = 64 * 1024 * 1024

    private static func fehler(_ e: AblageHubFehler) -> AblageLaufFehler {
        switch e {
        case .abgemeldet: return .abgemeldet
        case .wlanRegel: return .wlanRegel
        case .speicherVoll: return .speicherVoll
        default: return .hub(e)
        }
    }

    // MARK: Ein Lauf

    /// Pull beider Sammlungen (Ordner vor Dateien), Ordnergräber, Aufräumen (nur nach vollständigem Pull), Texte vorab
    /// und die angeforderten Originale. Wirft bei Abmeldung, Hub- und Speicherfehlern sowie bei Abbruch des Tasks; der
    /// bis dahin gespeicherte Zustand bleibt gültig.
    public static func ausfuehren(hub: any AblageHub, speicher: AblageSpeicher, konto: AblageKonto,
                                  optionen: AblageLaufOptionen = AblageLaufOptionen()) async throws -> AblageBericht {
        let pfad = speicher.wurzel.path
        guard AblageLaufSperre.shared.betreten(pfad) else {
            var b = AblageBericht()
            b.uebersprungen = true
            return b
        }
        defer { AblageLaufSperre.shared.verlassen(pfad) }

        var bericht = AblageBericht()
        let geladen = speicher.laden(konto: konto.kennung)
        var z = geladen.zustand
        bericht.zustandVerlust = geladen.verlust
        bericht.kontoGewechselt = geladen.verlust == .anderesKonto
        if bericht.kontoGewechselt {
            // Dateien eines anderen Kontos bleiben nicht liegen; D5a hat nichts, was sich nicht neu ziehen ließe.
            speicher.alleDateienEntfernen()
            do { try speicher.speichern(z) } catch { throw AblageLaufFehler.speicher }
            speicher.defektEntfernen()      // Namen eines Kontos, das hier nichts mehr verloren hat
        }
        speicher.eingangRaeumen()

        var seitenBericht = AblageSeitenBericht()
        for coll in AblageLedger.sammlungen {
            try await sammlungZiehen(coll, hub: hub, speicher: speicher, konto: konto, optionen: optionen, z: &z,
                                     bericht: &bericht, seitenBericht: &seitenBericht)
        }
        // Ordnergräber erst jetzt: eine gleichzeitig begrabene Datei darf den Ordner nicht halten.
        try Task.checkCancellation()
        let g = AblagePullVerarbeitung.ordnergraeber(zustand: &z)
        seitenBericht.zusammenfuehren(g)
        do { try speicher.speichern(z) } catch { throw AblageLaufFehler.speicher }

        bericht.vollstaendig = seitenBericht.vollstaendig
        bericht.geaendert = bericht.geaendert || seitenBericht.geaendert
        bericht.begraben = seitenBericht.begraben.count
        bericht.angehalten = seitenBericht.angehalten
        for w in seitenBericht.warnungen { bericht.warne(w) }
        // Aufräumen nur bei vollständigem Pull: sonst fehlen dem Bestand Einträge, deren Dateien noch gebraucht werden.
        // Ein vollständiger Pull hat den Zustand neu aufgebaut: eine beiseitegelegte unlesbare Datei wird nicht mehr gebraucht.
        if bericht.vollstaendig {
            speicher.bereinigen(bestand: z.bestand)
            speicher.defektEntfernen()
        }
        try Task.checkCancellation()

        if optionen.downloadsErlaubt {
            try await herunterladen(hub: hub, speicher: speicher, konto: konto, optionen: optionen, z: z, bericht: &bericht)
        } else {
            bericht.downloadsRuhen = true
        }
        return bericht
    }

    /// Eine Sammlung bis zum Ende ziehen (`pull`): Cursor aus dem Zustand, 410 genau einmal von 0 neu, kein `more` ohne
    /// Fortschritt, höchstens `maxSeiten` Seiten; je Seite verarbeiten, Dateien begrabener Dateien entfernen, Cursor setzen
    /// und den Zustand atomar speichern.
    private static func sammlungZiehen(_ coll: String, hub: any AblageHub, speicher: AblageSpeicher, konto: AblageKonto,
                                       optionen: AblageLaufOptionen, z: inout AblageZustand, bericht: inout AblageBericht,
                                       seitenBericht: inout AblageSeitenBericht) async throws {
        var since = z.ledger.cursor[coll] ?? 0
        var zurueckgesetzt = false
        var seiten = 0
        while true {
            try Task.checkCancellation()
            seiten += 1
            if seiten > optionen.maxSeiten { throw AblageLaufFehler.zuVieleSeiten }
            let seite: AblagePullSeite
            do {
                seite = try await hub.pull(since: since, limit: optionen.pullLimit, collection: coll)
            } catch AblageHubFehler.cursorAbgelaufen where since > 0 && !zurueckgesetzt {
                zurueckgesetzt = true
                since = 0
                bericht.neustarts += 1
                continue
            } catch let e as AblageHubFehler {
                throw fehler(e)
            }
            // Wurde der Lauf während des Abrufs abgebrochen (z. B. Abmelden), schreibt er nichts mehr, auch wenn der Hub noch antwortete.
            try Task.checkCancellation()
            if seite.more && seite.cursor <= since { throw AblageLaufFehler.ohneFortschritt }
            let b = AblagePullVerarbeitung.seite(sammlung: coll, objekte: seite.objects, konto: konto, zustand: &z)
            // Dateien begrabener Dateien gehen vor dem Speichern des Zustands weg: bricht es dazwischen ab, läuft die Seite
            // erneut (Cursor nicht vorgerückt) und der Grabstein wirkt noch einmal, ohne Schaden.
            speicher.entfernen(dateien: b.begraben)
            since = max(seite.cursor, since)
            z.ledger.cursor[coll] = since
            do { try speicher.speichern(z) } catch { throw AblageLaufFehler.speicher }
            bericht.seiten += 1
            seitenBericht.zusammenfuehren(b)
            if !seite.more { break }
        }
    }

    // MARK: Downloads

    /// Texte vorab, danach die angeforderten Originale; kleinere zuerst, mit Budget. Ein Netz- oder Speicherfehler beendet
    /// die Downloads dieses Laufs (der nächste versucht es wieder), ein Fehler einer einzelnen Datei überspringt nur sie.
    private static func herunterladen(hub: any AblageHub, speicher: AblageSpeicher, konto: AblageKonto, optionen: AblageLaufOptionen,
                                      z: AblageZustand, bericht: inout AblageBericht) async throws {
        var plan = AblageDownloadPlan.auftraege(zustand: z, vorhanden: speicher.vorhanden(z.bestand), key: konto.key,
                                                angefordert: optionen.angefordert)
        if optionen.mobil {
            let teil = AblageDownloadPlan.mobilAufteilen(plan, textBudget: optionen.mobilTextBudget)
            plan = teil.jetzt
            bericht.wartetAufWLAN = teil.wartet.count
        }
        var budget = AblageDownloadBudget(rest: optionen.downloadBudget)
        for a in plan {
            try Task.checkCancellation()
            guard budget.darfStarten(groesse: a.groesse) else { break }
            let ziel = speicher.zwischenpfad()
            defer { speicher.zwischenEntfernen(ziel) }
            do {
                // Im teuren Netz dürfen die (kleinen) Textfassungen auch über Mobilfunk, Originale nie.
                try await hub.blobLaden(hubId: a.hubId, groesse: a.groesse, ziel: ziel, bulk: optionen.mobil && a.art == .text ? false : optionen.bulk)
            } catch AblageHubFehler.abgemeldet {
                throw AblageLaufFehler.abgemeldet
            } catch AblageHubFehler.netz, AblageHubFehler.wlanRegel {
                bericht.downloadsUnterbrochen = true
                return
            } catch AblageHubFehler.speicherVoll {
                bericht.warne(.dateiNichtSpeicherbar)
                bericht.downloadsUnterbrochen = true
                return
            } catch is AblageHubFehler {
                continue                          // Blob fehlt oder beschädigt: nur diese Datei bleibt offen
            }
            try Task.checkCancellation()        // abgebrochen (z. B. Abmelden): nichts mehr ablegen
            budget.verbraucht(groesse: a.groesse)
            do {
                switch a.art {
                case .text:
                    guard let data = try? Data(contentsOf: ziel) else { bericht.warne(.dateiPasstNicht); continue }
                    speicher.zwischenEntfernen(ziel)
                    if try speicher.textEinlegen(bestand: z.bestand, dat: a.dat, fas: a.fas, textrev: a.textrev, bytes: [UInt8](data)) {
                        bericht.texteGeladen += 1
                        bericht.geaendert = true
                    }
                case .original:
                    if try speicher.originalEinlegen(bestand: z.bestand, dat: a.dat, fas: a.fas, zwischen: ziel) {
                        bericht.originaleGeladen += 1
                        bericht.geaendert = true
                    }
                }
            } catch AblageSpeicherFehler.schreiben {
                bericht.warne(.dateiNichtSpeicherbar)
                bericht.downloadsUnterbrochen = true
                return
            } catch is AblageSpeicherFehler {
                bericht.warne(.dateiPasstNicht)
            }
        }
    }

    // MARK: Ein Original auf Anforderung

    /// Lädt ein Original auf ausdrückliche Anforderung (Vorschau, Öffnen, Teilen): prüft, ob es geladen werden darf, ob
    /// genug Platz da ist (`freierPlatz`: freie Bytes, `nil` = unbekannt), lädt, prüft Größe und SHA-256 gegen die
    /// Fassung und legt es ab. Liegt es schon da, kommt sofort der Pfad zurück. Die Regel „nur im WLAN“ setzt der
    /// Aufrufer über `bulk` (und prüft sie vorab). Hinterlässt nie eine Zwischendatei.
    public static func originalLaden(hub: any AblageHub, speicher: AblageSpeicher, konto: AblageKonto, dat: String, fas: String,
                                     bulk: Bool, freierPlatz: (@Sendable () -> UInt64?)? = nil) async throws -> URL {
        let z = speicher.laden(konto: konto.kennung).zustand
        guard let a = AblageDownloadPlan.original(zustand: z, dat: dat, fas: fas, key: konto.key),
              let typ = z.bestand.dateien[dat]?.fassungen.first(where: { AblageBytes.equal($0.fas, fas) })?.typ else {
            throw AblageLaufFehler.nichtLadbar
        }
        if let url = speicher.originalLiegt(dat: dat, fas: fas, typ: typ) { return url }
        if let platz = freierPlatz?(), platz < a.groesse + platzReserve { throw AblageLaufFehler.speicherVoll }
        try Task.checkCancellation()
        let ziel = speicher.zwischenpfad()
        defer { speicher.zwischenEntfernen(ziel) }
        do {
            try await hub.blobLaden(hubId: a.hubId, groesse: a.groesse, ziel: ziel, bulk: bulk)
        } catch let e as AblageHubFehler {
            throw fehler(e)
        }
        try Task.checkCancellation()            // abgebrochen (z. B. Abmelden): nichts mehr ablegen
        // Mit frischem Stand ablegen: wurde die Datei inzwischen begraben, wird nichts abgelegt.
        let frisch = speicher.laden(konto: konto.kennung).zustand
        do {
            guard try speicher.originalEinlegen(bestand: frisch.bestand, dat: dat, fas: fas, zwischen: ziel) else {
                throw AblageLaufFehler.nichtLadbar
            }
        } catch AblageSpeicherFehler.passtNicht {
            throw AblageLaufFehler.passtNicht
        } catch is AblageSpeicherFehler {
            throw AblageLaufFehler.speicher
        }
        guard let url = speicher.originalLiegt(dat: dat, fas: fas, typ: typ) else { throw AblageLaufFehler.speicher }
        return url
    }
}
#endif
