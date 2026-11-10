import Foundation
#if canImport(CryptoKit)

// Suche in der Ablage: welche Dateien in den Index dürfen, Abdeckungsmeldung und der Suchdienst (D5a-4, lva#441, Epic #395).
//
// Der Index-Tor ist der aus D5a-3 (`AblageAnzeige.zugelasseneDateien`): nur lebende Dateien (nicht im Papierkorb, nicht
// begraben), Teilnahme `Sync` (nicht „nur Gerät“, nicht `personal`/`confidential`, nicht unbekannt, kein `Halten`), Hub-Stand
// lesbar (nie angehalten), Kette der Ordner lesbar, und eine brauchbare, hier geladene Textfassung. Der Filter wirkt VOR dem
// Aufbau des Index: was ihn nicht passiert, ist nie ein Stück, nie ein Treffer und nie ein Prompt (Spec §4.3, §5; Abnahme 6/7).
//
// Welche Textfassung: wie `speicher.rs::index_quellen` die der aktuellen Fassung (einziger Kopf, sonst die jüngste nach
// `(zeit, fas)`). Hat die aktuelle Fassung keinen hier geladenen Text mit `stand = ok`, ist die Datei „ohne Text“ (der Text
// einer älteren Fassung wird nie genommen). Gibt es mehrere `ok`-Texte (ältere Extraktorversion), gilt der letzte der Liste (wie
// der Rückfall des Desktops); das iPhone kennt die Extraktortabelle nicht und wählt nie selbst nach Version.

/// Abdeckungsmeldung wie Spec §5.3 (Dateien im Bereich, durchsucht, ohne Text, im Konflikt), erweitert um das, was das
/// iPhone zusätzlich nicht durchsucht (nur Gerät, angehalten).
public struct AblageAbdeckung: Equatable, Sendable {
    /// Lebende Dateien (kein Papierkorb, nicht begraben), auch solche, die der Tor nicht zulässt.
    public var imBereich = 0
    /// Davon im Index (Text vorhanden und geladen).
    public var durchsucht = 0
    /// Davon zugelassen, aber ohne brauchbaren Text hier: „Text wird am PC erstellt“, Format ohne Text, kein Text, Fehler der
    /// Extraktion, Text noch nicht geladen oder nicht lesbar.
    public var ohneText = 0
    /// Davon im Konflikt (mehr als ein Kopf): durchsucht wird die jüngste Fassung wie am Desktop, die Anzeige warnt.
    public var imKonflikt = 0
    /// Davon ohne gemeinsame Vorgängerfassung (die KI würde sie bis zur Wahl nicht nutzen).
    public var konfliktOhneGemeinsame = 0
    /// Davon nie im Index: nur dieses Gerät, `personal`/`confidential`, unbekannte Klasse, angehalten.
    public var nichtDurchsuchbar = 0

    public init() {}

    /// Eine Zeile für die Oberfläche (feste Texte, nie Namen).
    public var text: String {
        var teile = ["\(imBereich) \(imBereich == 1 ? "Datei" : "Dateien") im Bereich", "\(durchsucht) durchsucht", "\(ohneText) ohne Text"]
        if imKonflikt > 0 {
            teile.append("\(imKonflikt) im Konflikt" + (konfliktOhneGemeinsame > 0 ? " (\(konfliktOhneGemeinsame) ohne gemeinsame Fassung)" : ""))
        }
        if nichtDurchsuchbar > 0 { teile.append("\(nichtDurchsuchbar) nicht durchsuchbar") }
        return teile.joined(separator: ", ")
    }
}

public enum AblageSuche {
    /// Eine Textfassung, die in den Index geht.
    public struct Quelle: Equatable, Sendable {
        public let dat: String
        public let fas: String
        public let textrev: String
    }

    /// Der Text der aktuellen Fassung, der hier liegt und `ok` ist (der letzte der Liste, wenn es mehrere gibt).
    static func gewaehlterText(_ d: AblageDatei, vorhanden: AblageVorhanden) -> Quelle? {
        guard let f = AblageZusammenfuehren.aktuelleFassung(d) else { return nil }
        let t = f.texte.last { AblageBytes.equal($0.stand, "ok") && !$0.blob.isEmpty && vorhanden.text(d.id, $0.textrev) }
        return t.map { Quelle(dat: d.id, fas: f.fas, textrev: $0.textrev) }
    }

    /// Textfassungen aller zugelassenen Dateien (nach Kennung sortiert). Nichts aus Papierkorb, Lokal, Halten, angehalten,
    /// begraben und nichts ohne geladenen Text.
    public static func quellen(_ z: AblageZustand, vorhanden: AblageVorhanden) -> [Quelle] {
        AblageAnzeige.zugelasseneDateien(z).compactMap { id in z.bestand.dateien[id].flatMap { gewaehlterText($0, vorhanden: vorhanden) } }
    }

    /// Abdeckung. `durchsucht`: die Dateien, deren Text wirklich in den Index kam (Lesen kann scheitern).
    public static func abdeckung(_ z: AblageZustand, vorhanden: AblageVorhanden, durchsucht: Set<String>) -> AblageAbdeckung {
        var a = AblageAbdeckung()
        var zugelassen = AblageMenge(AblageAnzeige.zugelasseneDateien(z))
        for id in AblageAnzeige.lebendeDateien(z) {
            a.imBereich += 1
            guard zugelassen.contains(id), let d = z.bestand.dateien[id] else { a.nichtDurchsuchbar += 1; continue }
            zugelassen.remove(id)
            if durchsucht.contains(id) { a.durchsucht += 1 } else { a.ohneText += 1 }
            if AblageZusammenfuehren.konflikt(d) {
                a.imKonflikt += 1
                if AblageZusammenfuehren.gemeinsameVorgaengerin(d) == nil { a.konfliktOhneGemeinsame += 1 }
            }
        }
        return a
    }
}

/// Hält den Index der Ablage und baut ihn bei Änderungen neu (ein Akteur je Speicherordner). Textfassungen werden nur
/// gelesen und zerlegt, wenn ihre `textrev` neu ist; ein Wechsel der zugelassenen Dateien oder Texte baut den Index aus den
/// zwischengespeicherten Stücken neu auf. Fehlerfälle: ein Text, der nicht lesbar ist oder nicht zu seiner `textrev` passt,
/// bleibt draußen (die Datei zählt „ohne Text“, nie ein Absturz); beim Abmelden `leeren`. Kein I/O im Echtzeitpfad, kein
/// Kindprozess; Speicher: Stücke aller zugelassenen Texte und je Datei die Wörter einmal, die Stücke tragen nur Nummern (bei sehr
/// großen Ablagen Sekunden und einige zehn MB).
///
/// Gleichzeitige Aktualisierungen (`ansichtAktualisieren`): jeder Aufruf trägt eine Nummer der Seite. Der Akteur arbeitet die Aufrufe
/// nacheinander ab, liest den Zustand erst, wenn der Aufruf an der Reihe ist, und weist einen Aufruf ab, dessen Nummer unter der
/// des zuletzt bearbeiteten liegt: ein älterer Aufruf, der später eintrifft, setzt den Index nie auf einen älteren Stand zurück.
public actor AblageSuchdienst {
    private var vorbereitet: [String: AblageIndex.Vorbereitet] = [:]
    private var stand: [String] = []
    /// Höchste Nummer, die schon bearbeitet wurde (oder die Schwelle des letzten Abmeldens).
    private var hoechsteNummer = 0
    public private(set) var index = AblageIndex([])
    public private(set) var abdeckung = AblageAbdeckung()

    public init() {}

    private static func schluessel(_ q: AblageSuche.Quelle) -> String { q.dat + "\u{0}" + q.textrev }

    /// Bringt den Index auf den Stand des Zustands. Liefert die Abdeckung.
    @discardableResult
    public func aktualisieren(zustand z: AblageZustand, speicher: AblageSpeicher) -> AblageAbdeckung {
        let vorhanden = speicher.vorhanden(z.bestand)
        var docs: [AblageIndex.Vorbereitet] = []
        var schluessel: [String] = []
        var durchsucht = Set<String>()
        var behalten: [String: AblageIndex.Vorbereitet] = [:]
        for q in AblageSuche.quellen(z, vorhanden: vorhanden) {
            let k = Self.schluessel(q)
            let p: AblageIndex.Vorbereitet
            if let c = vorbereitet[k] {
                p = c
            } else if let bytes = speicher.textLesen(dat: q.dat, textrev: q.textrev), let tf = try? AblageTextfassung(bytes: bytes),
                      AblageBytes.equal(tf.textrev, q.textrev) {
                p = AblageIndex.vorbereiten(dat: q.dat, abschnitte: tf.abschnitte)
            } else {
                continue
            }
            behalten[k] = p
            guard p.anzahlStuecke > 0 else { continue }
            docs.append(p)
            schluessel.append(k)
            durchsucht.insert(q.dat)
        }
        vorbereitet = behalten
        if schluessel != stand {
            index = AblageIndex(docs)
            stand = schluessel
        }
        abdeckung = AblageSuche.abdeckung(z, vorhanden: vorhanden, durchsucht: durchsucht)
        return abdeckung
    }

    /// Liest den Zustand des Kontos, bringt den Index auf seinen Stand und baut die Übersicht für die Seite. `nummer` ist die der
    /// Aktualisierung (`AblageStandsfolge`). `nil`: ein neuerer Aufruf oder ein Abmelden war schon dran, nichts wurde geändert.
    /// Ein fehlender, unlesbarer oder fremder Zustand ergibt eine leere Ansicht (`AblageSpeicher.laden`), nie einen Absturz.
    public func ansichtAktualisieren(speicher: AblageSpeicher, konto kennung: String, nummer: Int) -> AblageAnsicht? {
        guard nummer >= hoechsteNummer else { return nil }
        hoechsteNummer = nummer
        let z = speicher.laden(konto: kennung).zustand
        let ab = aktualisieren(zustand: z, speicher: speicher)
        return AblageAnsicht(uebersicht: AblageAnzeige.uebersicht(z, vorhanden: speicher.vorhanden(z.bestand)), abdeckung: ab)
    }

    public func suche(_ frage: String) -> [AblageTreffer] { index.suche(frage) }

    /// Beim Abmelden: nichts behalten.
    public func leeren() {
        vorbereitet = [:]
        stand = []
        index = AblageIndex([])
        abdeckung = AblageAbdeckung()
    }

    /// Beim Abmelden mit Schwelle (`AblageStandsfolge.verwerfen`): leert und weist danach jeden Aufruf mit kleinerer Nummer ab, auch
    /// wenn er erst jetzt eintrifft. Hat schon ein Aufruf mit größerer Nummer gearbeitet (das neue Konto war schneller als dieses
    /// späte Abmelden), bleibt dessen Index stehen.
    public func leeren(nummer: Int) {
        guard nummer >= hoechsteNummer else { return }
        hoechsteNummer = nummer
        leeren()
    }
}
#endif
