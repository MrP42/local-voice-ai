import Foundation
#if canImport(CryptoKit)

// Koordination der Ablage-Läufe auf dem iPhone (D5a-3, Fix-Runde zu PR #453, lva#441).
//
// `AblageLauf` ist ohne Zustand: wer ihn aufruft, muss Läufe, Original-Downloads und Abmelden zusammenhalten. Das tut dieser
// Dienst (ein Akteur je Speicherordner, in der App einer für alle):
// - Abmelden (`verlassen`) bricht einen laufenden Abgleich und laufende Original-Downloads ab und WARTET, bis sie beendet
//   sind, bevor es den Speicherordner löscht. Sonst könnte ein Lauf nach dem Löschen noch den Zustand des alten Kontos
//   (samt Namen) neu schreiben. Solange das läuft, starten keine neuen Läufe.
// - Ein Aufruf von `laufen`, während schon ein Lauf aktiv ist, geht nicht verloren: seine angeforderten Originale werden
//   vorgemerkt und in einem Folgelauf nachgeholt (auch nach einem fehlgeschlagenen Lauf bleiben sie vorgemerkt).
// - Original-Downloads laufen in ihrer eigenen Aufgabe und sind für `verlassen` sichtbar. Dass ein gleichzeitiger Lauf ihren
//   `.part` nicht wegräumt, sichert `AblageSpeicher.eingangRaeumen` (laufende Zwischenpfade bleiben).
// Kein Logging, keine Namen oder Inhalte.

public actor AblageDienst {
    public let speicher: AblageSpeicher

    private struct Auftrag {
        let hub: any AblageHub
        let konto: AblageKonto
        let optionen: AblageLaufOptionen
    }

    private var laeuft = false
    private var verlassend = false
    /// Zählt jedes Abmelden: was vor einem Abmelden begann, gehört dem alten Konto und merkt sich nichts mehr vor.
    private var epoche = 0
    private var laufTask: Task<AblageBericht, Error>?
    private var downloads: [UUID: Task<URL, Error>] = [:]
    private var vorgemerkt: [AblageOriginalAnforderung] = []
    private var folge: Auftrag?

    public init(speicher: AblageSpeicher) { self.speicher = speicher }

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
    /// `angefordert`en Originale und seine Optionen werden für einen Folgelauf vorgemerkt, den der aktive Aufruf nach seinem
    /// Lauf noch macht. Beim Abmelden kehrt er ebenfalls sofort zurück.
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
        defer { laeuft = false; folge = nil }
        var auftrag = Auftrag(hub: hub, konto: konto, optionen: optionen)
        while true {
            var o = auftrag.optionen
            merken(o.angefordert)
            o.angefordert = vorgemerkt
            vorgemerkt = []
            let h = auftrag.hub, k = auftrag.konto, sp = speicher, opt = o
            let task = Task { try await AblageLauf.ausfuehren(hub: h, speicher: sp, konto: k, optionen: opt) }
            laufTask = task
            let bericht: AblageBericht
            do {
                bericht = try await withTaskCancellationHandler { try await task.value } onCancel: { task.cancel() }
            } catch {
                laufTask = nil
                // Nicht erledigte Anforderungen nicht verlieren: der nächste Lauf holt sie nach (außer beim Abmelden).
                if epoche == meine { merken(o.angefordert) }
                throw error
            }
            laufTask = nil
            guard epoche == meine, !verlassend, let f = folge else { return bericht }
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
        defer { downloads[id] = nil }
        return try await withTaskCancellationHandler { try await task.value } onCancel: { task.cancel() }
    }

    /// Abmelden: bricht Lauf und Original-Downloads ab, wartet, bis sie wirklich beendet sind, und löscht dann den ganzen
    /// Speicherordner (Zustand, Texte, Originale, Zwischendateien, beiseitegelegte Zustandsdatei). Danach schreibt nichts mehr
    /// im Namen des alten Kontos. Neue Läufe und Downloads werden bis dahin abgewiesen.
    public func verlassen() async {
        verlassend = true
        epoche += 1
        vorgemerkt = []
        folge = nil
        let lauf = laufTask
        let laufende = Array(downloads.values)
        lauf?.cancel()
        for t in laufende { t.cancel() }
        _ = try? await lauf?.value
        for t in laufende { _ = try? await t.value }
        speicher.alleEntfernen()
        verlassend = false
    }
}
#endif
