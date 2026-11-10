import Foundation
#if canImport(CryptoKit)

// Koordination der Ablage-Läufe auf dem iPhone (D5a-3, Fix-Runde zu PR #453, lva#441).
//
// `AblageLauf` ist ohne Zustand: wer ihn aufruft, muss Läufe, Original-Downloads und Abmelden zusammenhalten. Das tut dieser
// Dienst (ein Akteur je Speicherordner, in der App einer für alle):
// - Abmelden (`verlassen`) bricht einen laufenden Abgleich und laufende Original-Downloads ab und WARTET, bis sie beendet
//   sind, bevor es den Speicherordner löscht. Sonst könnte ein Lauf nach dem Löschen noch den Zustand des alten Kontos
//   (samt Namen) neu schreiben. Solange das läuft, starten keine neuen Läufe.
// - Ein Aufruf von `laufen`, während schon ein Lauf aktiv ist, verliert seine angeforderten Originale nicht: sie werden
//   vorgemerkt. Der Folgelauf (mit den Optionen des zweiten Aufrufs) kommt nur, wenn der aktive Lauf ERFOLGREICH endet. Scheitert
//   er (Netz, Speicher, Abbruch), entfällt der Folgelauf; die Anforderungen bleiben vorgemerkt und der nächste Aufruf holt sie
//   nach. Das ist gewollt: nach einem Fehler hilft ein sofortiger zweiter Lauf nicht, und der Fehler geht an den aktiven Aufrufer.
// - Angeforderte Originale, die ein Lauf nicht erledigt (WLAN-Regel im Mobilfunk, Budget, Netz weg, kein Download erlaubt),
//   meldet er im Bericht (`offeneAnforderungen`); der Dienst merkt sie für den nächsten Lauf vor. Beim Abmelden verfallen sie.
// - Nach einem Abmelden kommt aus einem Lauf des alten Kontos nie ein Bericht oder Fehler an, nur `CancellationError`.
// - `verlassen` kehrt erst zurück, wenn kein Lauf und kein Original-Download mehr aktiv ist und der Speicherordner weg ist;
//   gleichzeitige Aufrufe teilen sich dieselbe Abmeldung. Bis dahin starten keine neuen Läufe.
// - Original-Downloads laufen in ihrer eigenen Aufgabe und sind für `verlassen` sichtbar. Dass ein gleichzeitiger Lauf ihren
//   `.part` nicht wegräumt, sichert `AblageSpeicher.eingangRaeumen` (laufende Zwischenpfade bleiben).
// Fehlerfälle, die dieser Dienst abdeckt: gleichzeitige Aufrufe und Abmeldungen (Akteur, Epoche, geteilte Abmeldung), Abbruch
// mitten im Vorgang (Abmelden wartet, löscht erst danach), voller Datenträger und fehlendes Netz (Anforderungen bleiben
// vorgemerkt). Kein Kindprozess, kein Echtzeitpfad.
// Kein Logging, keine Namen oder Inhalte.

public actor AblageDienst {
    public let speicher: AblageSpeicher

    private struct Auftrag {
        let hub: any AblageHub
        let konto: AblageKonto
        let optionen: AblageLaufOptionen
    }

    /// Der Abgleich selbst. In der App `AblageLauf.ausfuehren`; Tests setzen einen Ersatz ein (etwa einen, der den Abbruch
    /// nicht bemerkt und trotzdem einen Bericht liefert).
    typealias LaufFunktion = @Sendable (any AblageHub, AblageSpeicher, AblageKonto, AblageLaufOptionen) async throws -> AblageBericht

    private let laufFunktion: LaufFunktion
    private var laeuft = false
    private var verlassend = false
    /// Zählt jedes Abmelden: was vor einem Abmelden begann, gehört dem alten Konto und merkt sich nichts mehr vor.
    private var epoche = 0
    private var laufTask: Task<AblageBericht, Error>?
    private var downloads: [UUID: Task<URL, Error>] = [:]
    private var vorgemerkt: [AblageOriginalAnforderung] = []
    private var folge: Auftrag?
    /// Die laufende Abmeldung: weitere Aufrufe von `verlassen` warten auf sie, statt eine zweite zu beginnen.
    private var verlassenTask: Task<Void, Never>?
    /// Zählt die Aufrufe von `verlassen` (für Tests, die auf den Beginn des Abmeldens warten, statt zu schlafen).
    private(set) var verlassenAufrufe = 0
    /// Wartende `verlassen`, bis kein Lauf und kein Download mehr aktiv ist.
    private var ruheWartende: [CheckedContinuation<Void, Never>] = []

    public init(speicher: AblageSpeicher) {
        self.init(speicher: speicher, lauf: { hub, sp, konto, optionen in
            try await AblageLauf.ausfuehren(hub: hub, speicher: sp, konto: konto, optionen: optionen)
        })
    }

    init(speicher: AblageSpeicher, lauf: @escaping LaufFunktion) {
        self.speicher = speicher
        self.laufFunktion = lauf
    }

    /// Originale, die für den nächsten Lauf vorgemerkt sind (zum Prüfen und für die Oberfläche).
    public var vorgemerkteOriginale: [AblageOriginalAnforderung] { vorgemerkt }

    private func uebersprungen() -> AblageBericht {
        var b = AblageBericht()
        b.uebersprungen = true
        return b
    }

    private func merken(_ neu: [AblageOriginalAnforderung]) {
        for a in neu where !vorgemerkt.contains(where: { AblageBytes.equal($0.dat, a.dat) && AblageBytes.equal($0.fas, a.fas) }) {
            vorgemerkt.append(a)
        }
    }

    /// Ein Abgleich (`AblageLauf.ausfuehren`). Läuft schon einer, kehrt der Aufruf sofort mit `uebersprungen` zurück; seine
    /// `angefordert`en Originale werden vorgemerkt, und seine Optionen gelten für einen Folgelauf, den der aktive Aufruf nach
    /// seinem Lauf noch macht, WENN dieser erfolgreich endet (endet er mit einem Fehler, entfällt der Folgelauf, die
    /// Anforderungen bleiben vorgemerkt). Beim Abmelden kehrt er ebenfalls sofort zurück. Wird während des Laufs abgemeldet, wirft er
    /// `CancellationError` (nie einen Bericht oder Fehler des alten Kontos).
    public func laufen(hub: any AblageHub, konto: AblageKonto, optionen: AblageLaufOptionen) async throws -> AblageBericht {
        if verlassend { return uebersprungen() }
        if laeuft {
            merken(optionen.angefordert)
            var o = optionen
            o.angefordert = []
            folge = Auftrag(hub: hub, konto: konto, optionen: o)
            return uebersprungen()
        }
        laeuft = true
        let meine = epoche
        defer { laeuft = false; folge = nil; ruheMelden() }
        var auftrag = Auftrag(hub: hub, konto: konto, optionen: optionen)
        while true {
            var o = auftrag.optionen
            merken(o.angefordert)
            o.angefordert = vorgemerkt
            vorgemerkt = []
            let h = auftrag.hub, k = auftrag.konto, sp = speicher, opt = o, lauf = laufFunktion
            let task = Task { try await lauf(h, sp, k, opt) }
            laufTask = task
            let bericht: AblageBericht
            do {
                bericht = try await withTaskCancellationHandler { try await task.value } onCancel: { task.cancel() }
            } catch {
                laufTask = nil
                // Nach einem Abmelden gehört der Lauf dem alten Konto: nur der Abbruch kommt an, kein anderer Fehler.
                if epoche != meine { throw CancellationError() }
                // Nicht erledigte Anforderungen nicht verlieren: der nächste Lauf holt sie nach.
                merken(o.angefordert)
                throw error
            }
            laufTask = nil
            // Ein Lauf, der das Abmelden nicht mehr bemerkt hat und trotzdem fertig wurde: sein Bericht gehört dem alten Konto.
            if epoche != meine { throw CancellationError() }
            // Was der Lauf nicht erledigt hat (WLAN, Budget, Netz weg, keine Downloads), bleibt für den nächsten Lauf vorgemerkt.
            merken(bericht.offeneAnforderungen)
            guard !verlassend, let f = folge else { return bericht }
            folge = nil
            auftrag = f
        }
    }

    /// Ein Original auf Anforderung (`AblageLauf.originalLaden`) in einer eigenen, für `verlassen` sichtbaren Aufgabe.
    public func originalLaden(hub: any AblageHub, konto: AblageKonto, dat: String, fas: String, bulk: Bool,
                              freierPlatz: (@Sendable () -> UInt64?)? = nil) async throws -> URL {
        if verlassend { throw AblageLaufFehler.abgemeldet }
        let id = UUID()
        let sp = speicher
        let task = Task { try await AblageLauf.originalLaden(hub: hub, speicher: sp, konto: konto, dat: dat, fas: fas, bulk: bulk, freierPlatz: freierPlatz) }
        downloads[id] = task
        defer { downloads[id] = nil; ruheMelden() }
        return try await withTaskCancellationHandler { try await task.value } onCancel: { task.cancel() }
    }

    /// Abmelden: bricht Lauf und Original-Downloads ab, wartet, bis sie wirklich beendet sind (kein Lauf mehr aktiv, kein
    /// Download mehr offen), und löscht dann den ganzen Speicherordner (Zustand, Texte, Originale, Zwischendateien, beiseitegelegte
    /// Zustandsdatei). Danach schreibt nichts mehr im Namen des alten Kontos. Neue Läufe und Downloads werden bis dahin
    /// abgewiesen. Gleichzeitige Aufrufe teilen sich eine Abmeldung und kehren gemeinsam zurück: keiner setzt die Sperre zu
    /// früh zurück, keiner löscht, was ein neues Konto inzwischen geschrieben hat.
    public func verlassen() async {
        verlassenAufrufe += 1
        if let laufende = verlassenTask {
            await laufende.value
            return
        }
        verlassend = true
        epoche += 1
        vorgemerkt = []
        folge = nil
        laufTask?.cancel()
        for t in downloads.values { t.cancel() }
        let t = Task { await self.verlassenAbschliessen() }
        verlassenTask = t
        await t.value
    }

    /// Wartet, bis nichts mehr läuft (auch die Fortsetzung von `laufen` ist durch), löscht und gibt die Sperre erst dann frei.
    private func verlassenAbschliessen() async {
        while laeuft || !downloads.isEmpty {
            await withCheckedContinuation { (c: CheckedContinuation<Void, Never>) in ruheWartende.append(c) }
        }
        speicher.alleEntfernen()
        vorgemerkt = []
        folge = nil
        verlassend = false
        verlassenTask = nil
    }

    /// Weckt die wartenden Abmeldungen, sobald kein Lauf und kein Download mehr aktiv ist.
    private func ruheMelden() {
        guard !laeuft, downloads.isEmpty, !ruheWartende.isEmpty else { return }
        let wartende = ruheWartende
        ruheWartende = []
        for c in wartende { c.resume() }
    }
}
#endif
