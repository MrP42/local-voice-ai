import Foundation
#if canImport(CryptoKit)

// Ansicht der Ablage auf dem iPhone: wer ein Ergebnis übernehmen darf, und der Zwischenspeicher des Baums (Fixrunde zu PR #461,
// D5a-4, lva#441, Epic #395).
//
// Die Seite „Ablage“ liest Übersicht, Abdeckung und Suchindex abseits des Hauptthreads. Mehrere Aktualisierungen können gleichzeitig
// laufen (ein Lauf und das Laden eines Originals), und ein Abmelden mit Neuanmelden kann dazwischenkommen. Damit nie ein
// veraltetes oder fremdes Ergebnis das neuere überschreibt, trägt jede Aktualisierung eine Marke (Nummer, Epoche, Konto):
// - Nur die Aktualisierung mit der höchsten Nummer setzt ihr Ergebnis (`gilt`), eine ältere verwirft es.
// - Abmelden (`verwerfen`) erhöht die Epoche: nichts, was vorher begann, wird danach noch übernommen, auch nicht bei Neuanmeldung
//   mit demselben Konto. Die Kontokennung fängt den Wechsel auf ein anderes Konto ab.
// - Ergebnisse, die nur am Konto hängen (Kontingent am Hub), prüft `kontoGilt`: gleiches Konto und gleiche Epoche, die Nummer
//   spielt keine Rolle.
// Der Suchdienst (`AblageSuchdienst.ansichtAktualisieren`) nimmt dieselbe Nummer und weist ältere Aufrufe selbst ab, damit auch der
// Index nie auf einen älteren Stand zurückfällt.
// Kein I/O, kein Kindprozess, kein Echtzeitpfad; Wert-Typ ohne Sperre (nur vom Hauptthread benutzt).

/// Marken für Aktualisierungen der Ansicht.
public struct AblageStandsfolge: Sendable {
    /// Beginn einer Aktualisierung (oder Stand des Kontos).
    public struct Marke: Equatable, Sendable {
        public let nummer: Int
        public let epoche: Int
        public let konto: String
    }

    private var nummer = 0
    private var epoche = 0

    public init() {}

    /// Beginn einer Aktualisierung für dieses Konto; die Nummer steigt immer.
    public mutating func beginnen(konto: String) -> Marke {
        nummer += 1
        return Marke(nummer: nummer, epoche: epoche, konto: konto)
    }

    /// Stand von Konto und Epoche jetzt, ohne eine neue Nummer zu vergeben (für Ergebnisse, die nur am Konto hängen).
    public func kontoMarke(konto: String) -> Marke { Marke(nummer: nummer, epoche: epoche, konto: konto) }

    /// Abmelden (oder Ende der Ansicht): alles, was bisher begann, ist ungültig. Liefert die Schwelle für den Suchdienst
    /// (`leeren(nummer:)`): jede Nummer darunter gehört dem alten Konto.
    @discardableResult
    public mutating func verwerfen() -> Int {
        epoche += 1
        nummer += 1
        return nummer
    }

    /// Das Ergebnis dieser Aktualisierung darf übernommen werden: sie ist die neueste, die Epoche ist dieselbe, und das Konto ist
    /// noch das, für das sie begann (`konto`: das jetzt angemeldete, `nil` ohne Konto).
    public func gilt(_ marke: Marke, konto aktuell: String?) -> Bool {
        marke.nummer == nummer && kontoGilt(marke, konto: aktuell)
    }

    /// Gleiches Konto und gleiche Epoche, nicht notwendig die neueste Nummer.
    public func kontoGilt(_ marke: Marke, konto aktuell: String?) -> Bool {
        marke.epoche == epoche && aktuell == marke.konto
    }
}

/// Übersicht und Abdeckung aus einem Zustand, den der Suchdienst gelesen hat.
public struct AblageAnsicht: Equatable, Sendable {
    public let uebersicht: AblageUebersicht
    public let abdeckung: AblageAbdeckung
}

/// Hält Zeilen und Papierkorb des Baums für den Stand (Version der Übersicht, eingeklappte Ordner), für den sie berechnet wurden.
/// `AblageBaum.zeilen` und `papierkorb` sortieren die Namen mit `localizedStandardCompare`; die Seite wertet ihren Körper bei jedem
/// Tastendruck der Suche und jeder Änderung des Zustands neu aus, das Sortieren soll nur bei einer neuen Übersicht oder einem
/// neuen Klappstand passieren. Gehalten wird nur der letzte Stand. Nicht threadsicher: nur vom Hauptthread benutzen.
public final class AblageBaumZwischenspeicher {
    public struct Ansicht: Equatable {
        public let zeilen: [AblageBaumZeile]
        public let korb: [AblageDateiEintrag]
    }

    private var version: Int?
    private var eingeklappt: Set<String> = []
    private var stand: Ansicht?
    /// Wie oft wirklich berechnet wurde (für Tests).
    public private(set) var berechnungen = 0

    public init() {}

    /// Die Ansicht für diese Übersicht. `version` zählt jede neue Übersicht hoch (`AblageSync.indexVersion`); gleiche Version und
    /// gleicher Klappstand ergeben dieselbe Ansicht ohne neue Berechnung.
    public func ansicht(_ u: AblageUebersicht, version v: Int, eingeklappt e: Set<String>) -> Ansicht {
        if let s = stand, version == v, eingeklappt == e { return s }
        let neu = Ansicht(zeilen: AblageBaum.zeilen(u, eingeklappt: e), korb: AblageBaum.papierkorb(u))
        berechnungen += 1
        version = v
        eingeklappt = e
        stand = neu
        return neu
    }
}
#endif
