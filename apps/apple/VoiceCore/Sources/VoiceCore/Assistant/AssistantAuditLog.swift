import Foundation

// Gesprächsmodus „Sprechen“ (Paket A, E4.2a): Protokoll ohne Inhalte, als fortgeschriebenes Journal.
// Je Ereignis wird eine JSON-Zeile angehängt (`protokoll.jsonl`):
// - `start` (VOR Modellaufruf und Ausgabe, awaitet): Anfrage-Id, inhaltsfreie Sitzungskennung, Zeit, Modellart, lokal/Cloud,
//   Modus (Opt-in), Einwilligungsversion (jetzt aus der Freigabe gelesen), Kontextzustand mit Grund, Rundenart, Generation;
// - `step`: Nummer, Einordnung oder Ablehnungsgrund, Generation (je Satz, je Ansage, je verworfenem Satz);
// - `commit`: Schrittnummer, Stufe (Kern, Anzeige, Audio, Verlauf), Ergebnis, Generation;
// - `end`: Ausgang, Stufe, Bereiche (auch die von Bestandsangaben);
// - `event`: Einwilligung und Widerruf (Opt-in, Cloud), nur noch von Aufrufern ohne Sitzung (ohne Freigabe gibt es keine
//   Generation); ältere Dateien enthalten sie für alle Fälle;
// - `release`: jede Erhöhung der Freigabe-Generation, vom Kern im Lock an der Stelle der Erhöhung geschrieben: neue Generation,
//   Grund (`cause`: optIn, areas, consent, resumeFree, other), bei Bereichen die NEUEN Bereichsnamen, dazu Opt-in-Zustand,
//   Einwilligungsversion, Modellart und lokal/Cloud. Eine Änderung erhöht genau EINMAL und schreibt genau EINE Zeile; die
//   Methode, die sie auslöst, kehrt erst zurück, wenn die Zeile in der Datei steht (Fehler werden gemeldet).
// - `state`: der Freigabezustand beim Öffnen des Protokolls (Opt-in, Einwilligungsversion, Bereichsnamen, Cloud-Zustimmung
//   ja/nein), vor der ersten Anfrage. Damit ist „freigegeben“ aus der Datei allein lesbar (Zustand + Zeilen `release`).
//
// Schreibfolge (ADR-0028 E4.2b): `start`, `round`, `areas` stehen dauerhaft in der Datei, bevor das Modell läuft oder etwas
// ausgegeben wird (Fehler: kein Modell, keine Ausgabe, Ausgang „Fehler“). Die Zeile `step` (Einordnung) JEDER Einheit, auch jedes
// verworfenen Kandidaten, steht dauerhaft in der Datei, bevor der erste Ausgabe-Commit der Einheit geprüft wird; sie behauptet
// nichts. Das Ergebnis je Commit (`commit`: Kern, Anzeige, Audio, Verlauf) wird im selben Lock eingereiht wie die Prüfung der
// Generation und die Aktion selbst: „ausgeführt“ (`+`) nur, wenn die Aktion ausgelöst wurde, „verworfen“ (`-`) bei veralteter
// Generation, „fehlgeschlagen“ (`!`), wenn sie sich nicht auslösen ließ. Ein Prozessende zwischen `step` und Ergebnis hinterlässt
// einen Schritt ohne Ergebnis: „Ausgabe möglich, Ergebnis unbekannt“ (`AssistantTraceStep.resultUnknown`); die Anfrage wird beim
// Öffnen als „unvollständig“ geschlossen. Ein Schreibfehler einer Ergebniszeile stoppt die laufende Ausgabe (keine weitere
// Einheit), Ausgang „Fehler“.
// Eine Antwort hat damit höchstens so viele Zeilen wie Schritte (ein Lauf endet nach 200 Schritten, Ausgang „begrenzt“); die
// Spur wird nicht gekappt und nicht im Speicher gesammelt. Ein Absturz mitten in der Antwort hinterlässt `start` und die bis
// dahin angefallenen Schritte; beim Öffnen bekommt jede Anfrage ohne `end` eines mit Ausgang „unvollständig“. Beim Öffnen
// wird außerdem auf die letzten 500 Anfragen verdichtet (Temp-Datei, dann atomar umbenennen: ein Abbruch oder ein voller
// Datenträger lässt die alte Datei unversehrt) und das alte Zeilenformat (Freitext-Modell, flache Zähler) ins neue
// übertragen. `read()` faltet die Zeilen zu `AssistantAuditEntry` (alte, schon gefaltete Zeilen bleiben lesbar).
// Ein Schreiber je Verzeichnis (Registry), eine serielle Warteschlange: Zeilen kommen in der Reihenfolge des Aufrufs, nie
// gleichzeitig. Ein Lesefehler wird gemeldet und überschreibt nichts. Keine Fragen, Antworten, Titel oder Auszüge: die Typen
// haben dafür gar kein Feld. Eigene Datei, damit „Neues Gespräch“ und „Verlauf löschen“ das Protokoll nicht mitlöschen. Dieses
// Modul loggt nichts, und es wird nie aus dem Audio-Callback gerufen.

/// Modellart als feste Kennung (kein Freitext aus der Anzeige des Modells).
public enum AssistantModelKind: String, Sendable, Codable, Equatable {
    case apple, gemma, other

    /// Ordnet eine Modellbezeichnung einer festen Art zu; die Bezeichnung selbst wird nicht gespeichert.
    public static func classify(label: String) -> AssistantModelKind {
        let l = label.lowercased()
        if l.contains("gemma") { return .gemma }
        if l.contains("apple") || l.contains("foundation") { return .apple }
        return .other
    }
}

/// Einstufung einer Antwort (ADR-0028 E4.3): S0 ist die geschlossene Liste der Vorlagen (Gruß, Bestätigung, feste Rückfrage,
/// Uhrzeit/Datum), alles andere ist mindestens S1. Eine Antwort mit Sätzen ohne Quelle ist nie S0.
public enum AssistantAuditLevel: String, Sendable, Codable, Equatable, Comparable {
    case s0, s1

    public static func < (a: Self, b: Self) -> Bool { a == .s0 && b == .s1 }

    public static func of(_ counts: AssistantSentenceCounts) -> AssistantAuditLevel {
        counts.grounded + counts.inventory + counts.unsourced > 0 ? .s1 : .s0
    }
}

/// Ausgabe-Commits, in der Reihenfolge ihres Auftretens je Satz: Ereignis aus dem Kern, Anzeige, Audiostart; dazu der Verlauf.
public enum AssistantCommitStage: String, Sendable, Codable, Equatable {
    case emit, display, audio, history

    var code: Character {
        switch self {
        case .emit: "e"
        case .display: "d"
        case .audio: "a"
        case .history: "h"
        }
    }

    init?(code: Character) {
        switch code {
        case "e": self = .emit
        case "d": self = .display
        case "a": self = .audio
        case "h": self = .history
        default: return nil
        }
    }
}

/// Ergebnis eines Ausgabe-Commits mit der dabei geprüften Generation. Im Protokoll als kurze feste Kennung („d+7“ = Anzeige
/// ausgeführt bei Generation 7, „a-8“ = Audiostart verworfen bei Generation 8, „a!8“ = Audiostart fehlgeschlagen: die
/// Generation galt, die Aktion ließ sich aber nicht auslösen). `ok` heißt „Aktion ausgelöst“.
public struct AssistantCommitMark: Sendable, Equatable, Codable {
    public let stage: AssistantCommitStage
    public let ok: Bool
    public let generation: Int
    /// Die Generation galt, die Aktion konnte aber nicht ausgelöst werden (`ok` ist dann `false`).
    public let failed: Bool

    public init(stage: AssistantCommitStage, ok: Bool, generation: Int, failed: Bool = false) {
        self.stage = stage
        self.ok = ok && !failed
        self.generation = generation
        self.failed = failed
    }

    public init(from decoder: Decoder) throws {
        let text = try decoder.singleValueContainer().decode(String.self)
        guard let first = text.first, let stage = AssistantCommitStage(code: first), text.count >= 3,
              let sign = text.dropFirst().first, sign == "+" || sign == "-" || sign == "!", let g = Int(text.dropFirst(2)) else {
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "Commit-Kennung"))
        }
        self.init(stage: stage, ok: sign == "+", generation: g, failed: sign == "!")
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        try c.encode("\(stage.code)\(failed ? "!" : (ok ? "+" : "-"))\(generation)")
    }
}

/// Einordnung eines Satzes in der Satzspur: die fünf Klassen (Zitat eines Quellblocks, Bestandsangabe, Satz ohne Quelle, Vorlage,
/// verworfen mit Grund) und die Ansage.
public enum AssistantTraceVerdict: String, Sendable, Codable, Equatable {
    case grounded, inventory, unsourced, template, announcement
    case droppedOwnData, droppedNoMode, droppedNotSnapped, droppedDuplicate, droppedTemplate, droppedRevoked, droppedFragment

    public init(status: AssistantSentenceStatus) {
        switch status {
        case .grounded: self = .grounded
        case .inventory: self = .inventory
        case .unsourced: self = .unsourced
        case .neutral: self = .template
        case .announcement: self = .announcement
        }
    }

    public init(dropped reason: AssistantDropReason) {
        switch reason {
        case .ownData: self = .droppedOwnData
        case .noMode: self = .droppedNoMode
        case .notSnapped: self = .droppedNotSnapped
        case .duplicate: self = .droppedDuplicate
        case .template: self = .droppedTemplate
        case .revoked: self = .droppedRevoked
        case .fragment: self = .droppedFragment
        }
    }
}

/// Ein Eintrag der Satzspur. Kein Textfeld.
public struct AssistantTraceStep: Sendable, Equatable, Codable {
    /// Laufende Nummer der Einordnung (ab 1).
    public var n: Int
    public var verdict: AssistantTraceVerdict
    /// Freigabe-Generation bei der Einordnung.
    public var generation: Int
    public var commits: [AssistantCommitMark]
    /// Die Anfrage dieser Einheit ist regulär beendet (Zeile `end` mit einem Ausgang, der kein „unvollständig“ ist, und ohne den Vermerk,
    /// dass Zeilen der Anfrage nicht geschrieben werden konnten). Nur der Leser setzt es (aus dem Eintrag, nicht aus der Datei der
    /// Einheit); Vorgabe: beendet. Bestimmt, was eine fehlende Ergebniszeile bedeutet (`result(for:)`).
    public var requestClosed: Bool

    public init(n: Int, verdict: AssistantTraceVerdict, generation: Int, commits: [AssistantCommitMark] = [], requestClosed: Bool = true) {
        self.n = n
        self.verdict = verdict
        self.generation = generation
        self.commits = commits
        self.requestClosed = requestClosed
    }

    /// Ergebnis eines Commits einer Einheit, wie das Protokoll es zeigt.
    public enum UnitResult: String, Sendable, Equatable {
        /// Die Handlung wurde ausgelöst (`+`).
        case executed
        /// Die Generation galt nicht mehr, nichts wurde gestartet (`-`).
        case discarded
        /// Die Handlung ließ sich nicht auslösen (`!`).
        case failed
        /// Die Einordnung steht, das Ergebnis DIESER Art fehlt in einer nicht regulär beendeten Anfrage: Ausgabe möglich, Ergebnis
        /// unbekannt (Prozessende zwischen Handlung und dauerhaftem Schreiben der Zeile). Es wird nie nachgeholt und nie als
        /// ausgeführt geschrieben.
        case unknown
        /// Nicht ausgeführt: ein verworfener Kandidat (nichts vorgesehen), oder die Art hat in einer regulär beendeten Anfrage keine
        /// Ergebniszeile (der Schreiber schreibt für eine Handlung, die nicht begann, keine).
        case notPlanned
    }

    /// Die Commits einer ausgegebenen Einheit in der Reihenfolge ihres Auftretens. Auch der Verlauf hat sein Ergebnis JE EINHEIT
    /// (`h+g`: steht im Beitrag, `h-g`: steht nicht darin); die Marke der Anfrage insgesamt (n = 0) bleibt zusätzlich.
    public static let unitStages: [AssistantCommitStage] = [.emit, .display, .audio, .history]

    /// Ob die Einheit überhaupt ausgegeben werden sollte (verworfene Kandidaten behaupten nichts).
    private var isOutput: Bool {
        switch verdict {
        case .grounded, .inventory, .unsourced, .template, .announcement: true
        default: false
        }
    }

    /// Schreibfolge (6), je Commit-Art EINZELN, und nie aus einer anderen Art abgeleitet (Runde 8, R7): Steht die Zeile dieser Art, gilt
    /// ihr Ergebnis. Fehlt sie, entscheidet allein die Anfrage: In einer nicht regulär beendeten Anfrage ist das Ergebnis dieser Art
    /// UNBEKANNT (auch nach einem verworfenen oder fehlgeschlagenen früheren Commit: Was nach ihm geschah, steht nicht in der Datei), in
    /// einer regulär beendeten wurde die Handlung nicht ausgeführt.
    public func result(for stage: AssistantCommitStage) -> UnitResult {
        guard isOutput, Self.unitStages.contains(stage) else { return .notPlanned }
        if let mark = commits.last(where: { $0.stage == stage }) {
            return mark.failed ? .failed : (mark.ok ? .executed : .discarded)
        }
        return requestClosed ? .notPlanned : .unknown
    }

    /// Die Arten, deren Ergebnis unbekannt ist.
    public var unknownStages: [AssistantCommitStage] { Self.unitStages.filter { result(for: $0) == .unknown } }

    /// Mindestens eine Art ist unbekannt (Ausgabe möglich, Ergebnis dieser Art unbekannt).
    public var resultUnknown: Bool { !unknownStages.isEmpty }

    private enum CodingKeys: String, CodingKey {
        case n
        case verdict = "v"
        case generation = "g"
        case commits = "c"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        n = try c.decode(Int.self, forKey: .n)
        verdict = try c.decode(AssistantTraceVerdict.self, forKey: .verdict)
        generation = try c.decode(Int.self, forKey: .generation)
        commits = try c.decodeIfPresent([AssistantCommitMark].self, forKey: .commits) ?? []
        requestClosed = true
    }
}

public struct AssistantAuditEntry: Sendable, Equatable, Codable {
    public enum Kind: String, Sendable, Codable {
        /// `resumedFree`: der Nutzer hat mit „Ohne Quelle fortsetzen“ den Datenkontext aufgehoben. `release`: eine Erhöhung der
        /// Freigabe-Generation (vom Kern geschrieben; Grund in `cause`). `state`: Freigabezustand beim Öffnen des Protokolls.
        case answer, unsourcedOn, unsourcedOff, cloudGranted, cloudRevoked, resumedFree, release, state
        /// „Erweiterung beabsichtigt“ (Einwilligung, Bereich an, Zustimmung erteilt): hält die Handlung des Nutzers fest, nicht die
        /// Wirkung. Die Folgezeilen „wirksam“ und „in diesem Lauf nicht wirksam“ werden mit Lauf und Änderungsnummer zugeordnet
        /// (`effect`); fehlt beides, ist die Wirkung unbekannt.
        case intent
    }

    /// Wirkung einer beabsichtigten Erweiterung im Lauf, in dem sie angestoßen wurde.
    public enum Effect: String, Sendable, Codable, Equatable {
        /// Die Zeile „wirksam“ steht (mit der Generation in `effectiveGeneration`).
        case effective
        /// Die Zeile „Erweiterung in diesem Lauf nicht wirksam“ steht.
        case notEffective
        /// Weder noch: Die Wirkung ist UNBEKANNT (sie kann nach dem Austausch der Zustandsdatei eingetreten sein, bevor die Zeile
        /// geschrieben war), nicht „nicht wirksam“.
        case unknown
    }

    public enum Outcome: String, Sendable, Codable {
        /// Zu Ende gesprochen.
        case completed
        /// Vom Nutzer unterbrochen oder abgebrochen.
        case interrupted
        /// Wegen geänderter Freigabe (Opt-in, Bereich, Cloud-Zustimmung) abgebrochen oder verworfen.
        case revoked
        /// Modell- oder Verbindungsfehler.
        case failed
        /// Ohne Abschluss geblieben (Absturz oder Prozessende mitten in der Antwort): beim Öffnen nachgetragen.
        case incomplete
        /// Nach 200 Schritten beendet.
        case limited
    }

    public var at: Date
    public var kind: Kind
    /// Art des Modells (feste Kennung), kein Freitext.
    public var modelKind: AssistantModelKind
    public var cloud: Bool
    /// „Antworten ohne Quelle“ war bei Beginn der Antwort an.
    public var unsourcedMode: Bool
    public var consentVersion: Int
    public var round: AssistantRoundKind?
    public var level: AssistantAuditLevel?
    /// Anfrage-Id und inhaltsfreie Sitzungskennung (nur Antworten aus dem Journal).
    public var requestId: String?
    public var sessionId: String?
    /// Kontextzustand vor der Rundenwahl und der Grund der Wahl.
    public var contextMode: AssistantContext.Mode?
    public var contextReason: AssistantContext.Reason?
    /// Freigabe-Generation beim Start der Anfrage; bei Einwilligung, Widerruf und `release` die neue.
    public var generation: Int
    /// Grund der Erhöhung (nur `release`).
    public var cause: AssistantReleaseCause?
    /// Zahl der Sätze je Einordnung und Ablehnungsgrund.
    public var counts: AssistantSentenceCounts
    /// Bereiche der Quellen (Rohwerte `books`, `transcripts`, `files`, `notes`), sortiert.
    public var areas: [String]
    public var outcome: Outcome?
    public var trace: [AssistantTraceStep]
    /// Nur alte Zeilen (vor dem Journal) kannten eine Kappung der Spur.
    public var traceOverflow: Int
    /// Verlaufs-Commit der Antwort (nil: kein Beitrag vorgesehen).
    public var history: AssistantCommitMark?
    /// Zustimmung zum Cloud-Ziel (nur Zeilen `state`).
    public var cloudConsent: Bool?
    /// Laufnummer des Prozesslaufs (Schreibfolge 7). Das Paar aus Lauf und Generation wird nie wiederverwendet. `nil`: Zeile aus
    /// einer Fassung vor den Laufnummern.
    public var run: Int?
    /// Änderungsnummer je Lauf (Zeilen der Klasse A: `release`, `intent`).
    public var change: Int?
    /// Zeile `state`: Die Zustandsdatei war ungültig, der Lauf startete gesperrt („gesperrt“, Anzeige „Freigaben prüfen“).
    public var locked: Bool?
    /// Zeile `intent`: Wirkung der beabsichtigten Erweiterung.
    public var effect: Effect?
    /// Zeile `intent` mit Wirkung „wirksam“: die Generation, die im Schritt des Wirksamwerdens vergeben wurde.
    public var effectiveGeneration: Int?
    /// Zeile `state`: Der geltende Stand weicht vom zuletzt protokollierten ab (eine Zeile ging verloren, oder die Wirkung einer
    /// Erweiterung war unbekannt). Die Standzeile rekonstruiert nichts, sie macht die Abweichung nur erkennbar.
    public var deviatesFromLogged: Bool?

    public init(at: Date = Date(), kind: Kind, model: String = "", cloud: Bool = false, unsourcedMode: Bool = false,
                consentVersion: Int = 0, counts: AssistantSentenceCounts = AssistantSentenceCounts(), areas: [AssistantArea] = [],
                outcome: Outcome? = nil, round: AssistantRoundKind? = nil, level: AssistantAuditLevel? = nil, generation: Int = 0,
                trace: [AssistantTraceStep] = [], traceOverflow: Int = 0, history: AssistantCommitMark? = nil,
                cause: AssistantReleaseCause? = nil, cloudConsent: Bool? = nil) {
        self.at = at
        self.kind = kind
        self.cause = cause
        self.cloudConsent = cloudConsent
        self.modelKind = AssistantModelKind.classify(label: model)
        self.cloud = cloud
        self.unsourcedMode = unsourcedMode
        self.consentVersion = consentVersion
        self.round = round
        self.generation = generation
        self.counts = counts
        self.areas = Set(areas.map(\.rawValue)).sorted()
        self.outcome = outcome
        self.trace = trace
        self.traceOverflow = traceOverflow
        self.history = history
        if kind == .answer {
            // Mit Sätzen ohne Quelle mindestens S1, nie S0.
            let derived = AssistantAuditLevel.of(counts)
            var l = level ?? derived
            if counts.unsourced > 0, l < .s1 { l = .s1 }
            self.level = max(l, derived)
        } else {
            self.level = level
        }
    }

    /// Alte Zeilen (vor Paket A, Runde E4.2a): `model` (Freitext, wird nur zur Art), flache Zähler `grounded`, `unsourced`,
    /// `neutral`, `dropped`. Neue Felder fehlen dort und bekommen Vorgabewerte.
    private enum CodingKeys: String, CodingKey {
        case at, kind, modelKind, cloud, unsourcedMode, consentVersion, round, level, generation, counts, areas, outcome, trace
        case traceOverflow, history, requestId, sessionId, contextMode, contextReason, cause, cloudConsent
        case run, change, locked, effect, effectiveGeneration, deviatesFromLogged
        case model, grounded, unsourced, neutral, dropped
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        at = try c.decode(Date.self, forKey: .at)
        kind = try c.decode(Kind.self, forKey: .kind)
        if let k = try c.decodeIfPresent(AssistantModelKind.self, forKey: .modelKind) {
            modelKind = k
        } else {
            modelKind = AssistantModelKind.classify(label: try c.decodeIfPresent(String.self, forKey: .model) ?? "")
        }
        cloud = try c.decodeIfPresent(Bool.self, forKey: .cloud) ?? false
        unsourcedMode = try c.decodeIfPresent(Bool.self, forKey: .unsourcedMode) ?? false
        consentVersion = try c.decodeIfPresent(Int.self, forKey: .consentVersion) ?? 0
        round = try c.decodeIfPresent(AssistantRoundKind.self, forKey: .round)
        generation = try c.decodeIfPresent(Int.self, forKey: .generation) ?? 0
        if let counts = try c.decodeIfPresent(AssistantSentenceCounts.self, forKey: .counts) {
            self.counts = counts
        } else {
            var legacy = AssistantSentenceCounts()
            legacy.grounded = try c.decodeIfPresent(Int.self, forKey: .grounded) ?? 0
            legacy.unsourced = try c.decodeIfPresent(Int.self, forKey: .unsourced) ?? 0
            legacy.neutral = try c.decodeIfPresent(Int.self, forKey: .neutral) ?? 0
            legacy.droppedNotSnapped = try c.decodeIfPresent(Int.self, forKey: .dropped) ?? 0
            counts = legacy
        }
        areas = try c.decodeIfPresent([String].self, forKey: .areas) ?? []
        outcome = try c.decodeIfPresent(Outcome.self, forKey: .outcome)
        trace = try c.decodeIfPresent([AssistantTraceStep].self, forKey: .trace) ?? []
        traceOverflow = try c.decodeIfPresent(Int.self, forKey: .traceOverflow) ?? 0
        history = try c.decodeIfPresent(AssistantCommitMark.self, forKey: .history)
        requestId = try c.decodeIfPresent(String.self, forKey: .requestId)
        sessionId = try c.decodeIfPresent(String.self, forKey: .sessionId)
        contextMode = try c.decodeIfPresent(AssistantContext.Mode.self, forKey: .contextMode)
        contextReason = try c.decodeIfPresent(AssistantContext.Reason.self, forKey: .contextReason)
        cause = try c.decodeIfPresent(AssistantReleaseCause.self, forKey: .cause)
        cloudConsent = try c.decodeIfPresent(Bool.self, forKey: .cloudConsent)
        run = try c.decodeIfPresent(Int.self, forKey: .run)
        change = try c.decodeIfPresent(Int.self, forKey: .change)
        locked = try c.decodeIfPresent(Bool.self, forKey: .locked)
        effect = try c.decodeIfPresent(Effect.self, forKey: .effect)
        effectiveGeneration = try c.decodeIfPresent(Int.self, forKey: .effectiveGeneration)
        deviatesFromLogged = try c.decodeIfPresent(Bool.self, forKey: .deviatesFromLogged)
        let derived = AssistantAuditLevel.of(counts)
        let stored = try c.decodeIfPresent(AssistantAuditLevel.self, forKey: .level)
        level = kind == .answer ? max(stored ?? derived, derived) : stored
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(at, forKey: .at)
        try c.encode(kind, forKey: .kind)
        try c.encode(modelKind, forKey: .modelKind)
        try c.encode(cloud, forKey: .cloud)
        try c.encode(unsourcedMode, forKey: .unsourcedMode)
        try c.encode(consentVersion, forKey: .consentVersion)
        try c.encodeIfPresent(round, forKey: .round)
        try c.encodeIfPresent(level, forKey: .level)
        try c.encode(generation, forKey: .generation)
        try c.encode(counts, forKey: .counts)
        try c.encode(areas, forKey: .areas)
        try c.encodeIfPresent(outcome, forKey: .outcome)
        if !trace.isEmpty { try c.encode(trace, forKey: .trace) }
        if traceOverflow > 0 { try c.encode(traceOverflow, forKey: .traceOverflow) }
        try c.encodeIfPresent(history, forKey: .history)
        try c.encodeIfPresent(requestId, forKey: .requestId)
        try c.encodeIfPresent(sessionId, forKey: .sessionId)
        try c.encodeIfPresent(contextMode, forKey: .contextMode)
        try c.encodeIfPresent(contextReason, forKey: .contextReason)
        try c.encodeIfPresent(cause, forKey: .cause)
        try c.encodeIfPresent(cloudConsent, forKey: .cloudConsent)
        try c.encodeIfPresent(run, forKey: .run)
        try c.encodeIfPresent(change, forKey: .change)
        try c.encodeIfPresent(locked, forKey: .locked)
        try c.encodeIfPresent(effect, forKey: .effect)
        try c.encodeIfPresent(effectiveGeneration, forKey: .effectiveGeneration)
        try c.encodeIfPresent(deviatesFromLogged, forKey: .deviatesFromLogged)
    }
}


public enum AssistantAuditError: Error, Sendable, Equatable {
    /// Die vorhandene Datei war nicht lesbar: nichts wurde geschrieben, die Datei bleibt, wie sie ist.
    case readFailed
    case encodeFailed
    case writeFailed
}

/// Ergebnis des Lesens: lesbare Einträge, Zahl der übersprungenen (beschädigten) Zeilen, Lesefehler.
public struct AssistantAuditRead: Sendable, Equatable {
    public var entries: [AssistantAuditEntry]
    public var skippedLines: Int
    public var error: AssistantAuditError?
}

/// Eine Zeile des Journals (`t` = Art). Felder, die nicht gelten, fehlen. Kein Textfeld.
struct AssistantJournalLine: Codable, Equatable {
    enum Kind: String, Codable, Equatable {
        /// `start`: erste Zeile einer Anfrage, steht in der Datei VOR dem Abruf (Zeit, Modell, lokal/Cloud, Modus, Fassung, Generation,
        /// Sitzung, Kontextzustand vorher). `round`: Rundenart und Grund, sobald gewählt. `areas`: Bereiche der Quellen, sobald sie
        /// feststehen (vor dem Modellaufruf bzw. bei Bestandsangaben), nicht erst im `end`. `round` und `areas` sind additiv:
        /// ältere Dateien (Rundenart im `start`, Bereiche nur im `end`) laden unverändert.
        case start, step, commit, end, event, round, areas
        /// `release`: Erhöhung der Freigabe-Generation (Kern, im Lock): `gen` neue Generation, `cause` Grund, `areas` bei
        /// Bereichen die neuen Namen, `unsourced`/`consent`/`model`/`cloud` Zustand danach. Additiv: ältere Fassungen kennen die
        /// Art nicht und überspringen die Zeile (sie bleibt Byte für Byte erhalten). Eine Einschränkung OHNE Sitzung (kein Modell)
        /// trägt keine Generation (`gen` fehlt, es gibt keine), dafür `run` und `chg`; `model` fehlt dann auch.
        case release
        /// `state`: Freigabezustand beim Öffnen des Protokolls (`run` = Laufnummer, `unsourced`, `consent`, `areas`, `granted` =
        /// Cloud-Zustimmung, `locked`, `model`, `cloud`, `gen`). Additiv wie `release`.
        case state
        /// Klasse A, Erweiterung (Schreibfolge 8): `intent` = „Erweiterung beabsichtigt“ (Lauf, Änderungsnummer, Ursache, KEINE
        /// Generation), `effective` = „Erweiterung wirksam“ (Lauf, Änderungsnummer, die im Schritt des Wirksamwerdens vergebene
        /// Generation), `ineffective` = „Erweiterung in diesem Lauf nicht wirksam“ (Lauf, Änderungsnummer). Additiv.
        case intent, effective, ineffective
    }

    var t: Kind
    /// Anfrage-Id (nicht bei `event`).
    var req: String?
    var at: Date?
    /// Inhaltsfreie Sitzungskennung (`start`).
    var session: String?
    var model: AssistantModelKind?
    var cloud: Bool?
    /// Modus: Opt-in „Antworten ohne Quelle“ war an.
    var unsourced: Bool?
    var consent: Int?
    var ctx: AssistantContext.Mode?
    var reason: AssistantContext.Reason?
    var round: AssistantRoundKind?
    var gen: Int?
    /// Schrittnummer (`step`, `commit`; 0 = der Verlaufs-Commit der Antwort).
    var n: Int?
    var verdict: AssistantTraceVerdict?
    var mark: AssistantCommitMark?
    var outcome: AssistantAuditEntry.Outcome?
    var level: AssistantAuditLevel?
    var areas: [String]?
    /// Art des Ereignisses (`event`).
    var kind: AssistantAuditEntry.Kind?
    /// Grund der Erhöhung (`release`).
    var cause: AssistantReleaseCause?
    /// Zustimmung zum Cloud-Ziel (`state`, `release`, `intent`, `effective`).
    var granted: Bool?
    /// Laufnummer (Zeilen ohne Anfrage mit Generation: `state`, `release`, `intent`, `effective`, `ineffective`; dazu `start`).
    var run: Int?
    /// Änderungsnummer je Lauf (Klasse A).
    var chg: Int?
    /// Zustandsdatei ungültig, Lauf gesperrt (`state`).
    var locked: Bool?
    /// Die Bereiche des anderen Ziels (lokal/Cloud), wenn die Zustandsdatei beide führt (`state`): die Zeile trägt den ganzen
    /// Stand der Klasse A, auch wenn die erste Sitzung des Laufs nur ein Ziel nutzt.
    var otherAreas: [String]?
    /// Zeile `end`: Zeilen dieser Anfrage ließen sich nicht schreiben (Schreibfehler). Was in der Datei fehlt, heißt dann nicht
    /// „nicht ausgeführt“, sondern „unbekannt“ (Runde 8, R7): Der Leser leitet aus dem Fehlen nichts ab.
    var lost: Bool?

    init(_ t: Kind, req: String? = nil) {
        self.t = t
        self.req = req
    }
}

/// Faltet Journalzeilen zu Einträgen (die Ansicht für Prüfung und Tests). Zeilen ohne `t` sind alte, schon gefaltete Einträge.
enum AssistantJournalFold {
    static func decoder() -> JSONDecoder {
        let d = JSONDecoder()
        d.dateDecodingStrategy = .iso8601
        return d
    }

    static func journal(_ line: Data) -> AssistantJournalLine? { try? decoder().decode(AssistantJournalLine.self, from: line) }

    /// Zähler aus der Satzspur: ein Satz, dessen Ereignis nicht hinausging (`e-`), zählt als widerrufen.
    ///
    /// Runde 8, Suchlauf (e): Das ist die EINE Leserregel, die aus einer Abwesenheit etwas ableitet: Ohne Vermerk `e-` gilt der Satz
    /// als ausgegeben (auch wenn die Zeile des Ausgabe-Commits ganz fehlt, etwa beim Prozessende nach dem Schritt oder in Dateien aus
    /// der Zeit vor den Commit-Zeilen). Das ist die vorsichtige Richtung für eine Aussage „was aus eigenen Inhalten ausgegeben wurde“:
    /// Im Zweifel zählt ein Satz als möglicherweise ausgegeben (Stufe, Zähler), nie als sicher nicht ausgegeben. Der Zustand der
    /// einzelnen Art (`AssistantTraceStep.result(for:)`) sagt dazu „unbekannt“, wo die Zeile fehlt.
    static func counts(from trace: [AssistantTraceStep]) -> AssistantSentenceCounts {
        var c = AssistantSentenceCounts()
        for step in trace {
            let delivered = !step.commits.contains { $0.stage == .emit && !$0.ok }
            switch step.verdict {
            case .announcement: continue
            case .droppedOwnData: c.droppedOwnData += 1
            case .droppedNoMode: c.droppedNoMode += 1
            case .droppedNotSnapped: c.droppedNotSnapped += 1
            case .droppedDuplicate: c.droppedDuplicate += 1
            case .droppedTemplate: c.droppedTemplate += 1
            case .droppedRevoked: c.revoked += 1
            case .droppedFragment: c.droppedFragment += 1
            case .grounded: if delivered { c.grounded += 1 } else { c.revoked += 1 }
            case .inventory: if delivered { c.inventory += 1 } else { c.revoked += 1 }
            case .unsourced: if delivered { c.unsourced += 1 } else { c.revoked += 1 }
            case .template: if delivered { c.neutral += 1 } else { c.revoked += 1 }
            }
        }
        return c
    }

    /// Der zuletzt protokollierte Stand der Klasse A (für die Standzeile): aus `state`, `release` und `effective`, nie aus `intent`.
    private struct Logged: Equatable {
        var optIn = false
        var consent = 0
        var areas: [String] = []
        var granted = false
    }

    private static func logged(_ l: AssistantJournalLine, from old: Logged?) -> Logged {
        var out = old ?? Logged()
        if let u = l.unsourced { out.optIn = u }
        if let c = l.consent { out.consent = c }
        if let a = l.areas { out.areas = a.sorted() }
        if let g = l.granted { out.granted = g }
        return out
    }

    private static func entry(_ l: AssistantJournalLine, kind: AssistantAuditEntry.Kind, generation: Int = 0) -> AssistantAuditEntry {
        var e = AssistantAuditEntry(at: l.at ?? .distantPast, kind: kind, cloud: l.cloud ?? false, unsourcedMode: l.unsourced ?? false,
                                    consentVersion: l.consent ?? 0, generation: generation, cause: l.cause, cloudConsent: l.granted)
        e.modelKind = l.model ?? .other
        e.areas = l.areas ?? []
        e.run = l.run
        e.change = l.chg
        return e
    }

    static func fold(_ lines: [Data]) -> (entries: [AssistantAuditEntry], skipped: Int) {
        let plain = decoder()
        var lastLogged: Logged?
        /// Zeilen „beabsichtigt“ nach (Lauf, Änderungsnummer).
        var intents: [String: Int] = [:]
        var entries: [AssistantAuditEntry] = []
        var byRequest: [String: Int] = [:]
        var endLevel: [Int: AssistantAuditLevel] = [:]
        /// Bereiche aus den Zeilen `areas` (je Eintrag; die Vereinigung gilt, solange kein `end` sie ablöst).
        var offered: [Int: Set<String>] = [:]
        var journalEntries: [Int] = []
        /// Anfragen, deren Ende vermerkt, dass Zeilen der Anfrage nicht geschrieben werden konnten (`lost`).
        var lostLines = Set<Int>()
        var skipped = 0
        for data in lines {
            guard let l = journal(data) else {
                if let e = try? plain.decode(AssistantAuditEntry.self, from: data) { entries.append(e) } else { skipped += 1 }
                continue
            }
            switch l.t {
            case .start:
                guard let req = l.req else { skipped += 1; continue }
                var e = AssistantAuditEntry(at: l.at ?? .distantPast, kind: .answer, cloud: l.cloud ?? false,
                                            unsourcedMode: l.unsourced ?? false, consentVersion: l.consent ?? 0, round: l.round,
                                            generation: l.gen ?? 0)
                e.modelKind = l.model ?? .other
                e.run = l.run
                e.requestId = req
                e.sessionId = l.session
                e.contextMode = l.ctx
                e.contextReason = l.reason
                entries.append(e)
                byRequest[req] = entries.count - 1
                journalEntries.append(entries.count - 1)
            case .step:
                guard let req = l.req, let i = byRequest[req], let n = l.n, let v = l.verdict else { skipped += 1; continue }
                entries[i].trace.append(AssistantTraceStep(n: n, verdict: v, generation: l.gen ?? 0))
            case .commit:
                guard let req = l.req, let i = byRequest[req], let n = l.n, let mark = l.mark else { skipped += 1; continue }
                if n == 0 {
                    entries[i].history = mark
                } else if let k = entries[i].trace.firstIndex(where: { $0.n == n }) {
                    entries[i].trace[k].commits.append(mark)
                } else {
                    skipped += 1
                }
            case .round:
                guard let req = l.req, let i = byRequest[req], let round = l.round else { skipped += 1; continue }
                entries[i].round = round
                if let reason = l.reason { entries[i].contextReason = reason }
            case .areas:
                guard let req = l.req, let i = byRequest[req], let areas = l.areas else { skipped += 1; continue }
                offered[i, default: []].formUnion(areas)
            case .end:
                guard let req = l.req, let i = byRequest[req] else { skipped += 1; continue }
                entries[i].outcome = l.outcome
                entries[i].areas = l.areas ?? (offered[i] ?? []).sorted()
                if let level = l.level { endLevel[i] = level }
                if l.lost == true { lostLines.insert(i) }
            case .release:
                // Eine Zeile `release` ohne Generation gibt es nur für eine Änderung ohne Sitzung (es läuft kein Modell, also gibt es
                // keine Generation): Sie trägt dann Lauf und Änderungsnummer. Ohne beides ist sie nicht lesbar.
                guard l.cause != nil, l.gen != nil || (l.run != nil && l.chg != nil) else { skipped += 1; continue }
                entries.append(entry(l, kind: .release, generation: l.gen ?? 0))
                lastLogged = logged(l, from: lastLogged)
            case .state:
                var e = entry(l, kind: .state, generation: l.gen ?? 0)
                e.locked = l.locked
                let now = logged(l, from: nil)
                if let before = lastLogged { e.deviatesFromLogged = before != now }
                lastLogged = now
                entries.append(e)
            case .intent:
                guard let run = l.run, let chg = l.chg else { skipped += 1; continue }
                var e = entry(l, kind: .intent)
                e.effect = .unknown
                entries.append(e)
                intents["\(run)/\(chg)"] = entries.count - 1
            case .effective, .ineffective:
                guard let run = l.run, let chg = l.chg else { skipped += 1; continue }
                let key = "\(run)/\(chg)"
                if intents[key] == nil {
                    // „Erweiterung in diesem Lauf nicht wirksam“ ohne die Zeile „beabsichtigt“ (deren Schreiben scheiterte).
                    guard l.t == .ineffective else { skipped += 1; continue }
                    entries.append(entry(l, kind: .intent))
                    intents[key] = entries.count - 1
                }
                guard let i = intents[key] else { continue }
                if l.t == .effective {
                    entries[i].effect = .effective
                    entries[i].effectiveGeneration = l.gen
                    lastLogged = logged(l, from: lastLogged)
                } else if entries[i].effect != .effective {
                    entries[i].effect = .notEffective
                }
            case .event:
                var e = AssistantAuditEntry(at: l.at ?? .distantPast, kind: l.kind ?? .unsourcedOn, cloud: l.cloud ?? false,
                                            unsourcedMode: l.unsourced ?? false, consentVersion: l.consent ?? 0, generation: l.gen ?? 0)
                e.modelKind = l.model ?? .other
                entries.append(e)
            }
        }
        for i in journalEntries {
            entries[i].counts = counts(from: entries[i].trace)
            if entries[i].outcome == nil {
                entries[i].outcome = .incomplete
                entries[i].areas = (offered[i] ?? []).sorted()   // kein `end`: die Bereiche stammen aus den Zeilen `areas`
            }
            // Was eine fehlende Ergebniszeile bedeutet, entscheidet allein die Anfrage (`AssistantTraceStep.result(for:)`): beendet nur
            // mit regulärem Ende ohne Vermerk verlorener Zeilen.
            let closed = entries[i].outcome != .incomplete && !lostLines.contains(i)
            for k in entries[i].trace.indices { entries[i].trace[k].requestClosed = closed }
            entries[i].level = max(AssistantAuditLevel.of(entries[i].counts), endLevel[i] ?? .s0)
        }
        return (entries, skipped)
    }
}

/// Die Laufnummer eines Prozesslaufs (Schreibfolge 7). Sie wird beim ersten Schreiben einer Zeile dieses Laufs vergeben: höchste
/// Laufnummer im Protokoll plus eins, nie dieselbe für zwei Läufe desselben Schreibers. Fehlschläge vergeben nichts Neues: Dieselbe
/// Nummer gilt für den nächsten Versuch.
final class AssistantRunToken: @unchecked Sendable {
    private let lock = NSLock()
    private var value: Int?

    /// Die Nummer, sobald vergeben.
    var number: Int? {
        lock.lock()
        defer { lock.unlock() }
        return value
    }

    /// Vergibt die Nummer einmal (nur auf der Warteschlange des Schreibers).
    fileprivate func resolve(_ make: () -> Int) -> Int {
        lock.lock()
        defer { lock.unlock() }
        if let value { return value }
        let fresh = make()
        value = fresh
        return fresh
    }
}

/// Der Anspruch auf den Ausgang der Zeilen unter einem Schlüssel (Runde 8, R6). Der Fehler einer Zeile hängt am Anspruch dessen, der auf
/// sie wartet (die Anfrage, die Änderung), nicht an einer Tabelle des Schreibers mit Obergrenze: Er geht nie verloren, solange der
/// Anspruch besteht, gleich wie viele andere Schlüssel in der Zwischenzeit scheitern. Alle Entscheidungen (Abschluss der Anfrage,
/// Stopp der Ausgabe, Ergebnis einer Änderung) lesen diesen einen Ort.
final class AssistantAuditClaim: @unchecked Sendable {
    private let lock = NSLock()
    private var value: AssistantAuditError?

    func record(_ error: AssistantAuditError) { lock.withLock { value = error } }
    var error: AssistantAuditError? { lock.withLock { value } }
    func take() -> AssistantAuditError? { lock.withLock { defer { value = nil }; return value } }
}

/// Der eine Schreiber eines Verzeichnisses. Alle Handles auf dasselbe Verzeichnis teilen ihn. Alle Zugriffe auf die Datei laufen
/// auf einer seriellen Warteschlange: `enqueue` kehrt sofort zurück, die Zeilen werden in dieser Reihenfolge angehängt.
final class AssistantAuditWriter: @unchecked Sendable {
    let directory: URL
    var url: URL { directory.appendingPathComponent(AssistantAuditLog.fileName) }

    private let reader: @Sendable (URL) throws -> Data?
    private let appender: @Sendable (URL, Data) throws -> Void
    /// Eigenes Neuschreiben (Tests); ohne: das dauerhafte Verdichten über `fileSystem` (Temp-Datei, fsync, rename, fsync Verzeichnis).
    private let rewriter: (@Sendable (URL, Data) throws -> Void)?
    /// Dauerhaftigkeit: fsync der Datei, fsync des Verzeichnisses beim Anlegen, Teilschritte des Verdichtens. Tests ersetzen es, um
    /// jeden Schritt scheitern zu lassen (wie beim Speicher der Zustandsdatei).
    private let fileSystem: any AssistantStoreFileSystem
    private let queue = DispatchQueue(label: "de.wolffappliedai.localvoice.assistant.journal")

    /// Eingereihte, noch nicht geschriebene Zeilen (kurzer Lock, nie I/O darunter). Die Warteschlange schreibt alles, was bis zu
    /// ihrem Lauf eingereiht ist, in einem Durchgang und sichert es mit EINEM fsync (Gruppen-Commit): `flush` und alle Wartepunkte
    /// kehren erst zurück, wenn dieses fsync die Zeile gedeckt hat.
    private struct Pending {
        let line: AssistantJournalLine
        let run: AssistantRunToken?
        let key: String
        /// Zeilen, auf die niemand wartet (Klasse B, Blockade): Scheitert das Schreiben, geht der Fehler an diese Stelle (Register der
        /// Zeilen zum Nachholen), nie in eine Tabelle, aus der er verdrängt werden kann.
        let onFailure: (@Sendable (AssistantAuditError) -> Void)?
        /// Runde 10, T3: Die Zeile wird nur geschrieben, wenn die Zeile DAVOR (dieselbe Gruppe, sie stehen hintereinander) angehängt
        /// werden konnte: „beide oder keine“ (eine Schlusszeile ohne ihre Startzeile kann der Leser keiner Anfrage zuordnen).
        var needsPrevious = false
    }
    private let pendingLock = NSLock()
    private var pending: [Pending] = []
    /// Nur auf `queue`: Die Datei wurde neu angelegt, das Verzeichnis ist noch nicht dauerhaft bestätigt.
    private var directoryUnsynced = false
    /// Zahl der fsync der Datei (Tests, Messung der Wartepunkte).
    private let syncLock = NSLock()
    private var syncCountValue = 0
    var syncCount: Int { syncLock.withLock { syncCountValue } }

    // Nur auf `queue`:
    private var opened = false
    /// Das Ende der Datei ist unsicher (letzter Schreibvorgang abgebrochen): die nächste Zeile beginnt mit einem Zeilenumbruch.
    private var torn = false
    private var openError: AssistantAuditError = .readFailed
    /// Höchste Laufnummer in der Datei und unter den vergebenen (Schreibfolge 7).
    private var maxRun = 0
    /// Hinter einem eigenen Lock (lesbar ohne Warteschlange): die Ansprüche auf Schlüssel (der Fehler einer Anfrage, einer Änderung
    /// hängt an seinem Anspruch, `AssistantAuditClaim`) und die Fehler unter Schlüsseln OHNE Anspruch (Waisen: niemand wartet auf sie,
    /// keine Entscheidung liest sie; nur diese sind begrenzt und werden verdrängt, der älteste zuerst).
    private let failedLock = NSLock()
    private var claims: [String: AssistantAuditClaim] = [:]
    private var orphans: [String: AssistantAuditError] = [:]
    private var orphanOrder: [String] = []
    /// Wird gerufen, sobald eine Zeile einen Schreibfehler unter einem Schlüssel meldet (hinter `failedLock`; der Aufruf selbst
    /// geschieht außerhalb jedes Locks, auf der Schreibwarteschlange). Der Controller stoppt damit die Ausgabe SOFORT, auch mitten
    /// in der Wiedergabe, statt erst beim nächsten Start einer Einheit.
    private var failureObserver: (@Sendable (String) -> Void)?
    static let maxErrors = 256

    /// Anfragen, über die verdichtet wird (die letzten).
    static let maxUnits = AssistantAuditLog.maxEntries

    init(directory: URL, reader: @escaping @Sendable (URL) throws -> Data? = AssistantAuditWriter.readFile,
         appender: @escaping @Sendable (URL, Data) throws -> Void = AssistantAuditWriter.appendFile,
         rewriter: (@Sendable (URL, Data) throws -> Void)? = nil,
         fileSystem: any AssistantStoreFileSystem = AssistantPosixStoreFileSystem()) {
        self.directory = directory
        self.reader = reader
        self.appender = appender
        self.rewriter = rewriter
        self.fileSystem = fileSystem
    }

    static let compactName = "protokoll.neu"

    /// Dauerhaftes Verdichten: die ganze neue Fassung in eine EIGENE Datei, fsync, atomar umbenennen, fsync des Verzeichnisses. Ein
    /// Fehler in einem Schritt lässt die alte Datei unversehrt (nur nach dem Umbenennen liegt schon die neue Fassung).
    private func compact(_ url: URL, _ data: Data) throws {
        if let rewriter { return try rewriter(url, data) }
        let temp = directory.appendingPathComponent(Self.compactName)
        var renamed = false
        do {
            try fileSystem.createDirectory(directory)
            try fileSystem.writeNew(temp, data)
            try fileSystem.syncFile(temp)
            try fileSystem.rename(temp, to: url)
            renamed = true
            try fileSystem.syncDirectory(directory)
        } catch {
            guard renamed else {
                fileSystem.remove(temp)
                throw error
            }
            // Runde 8, R2: Die neue Fassung liegt schon unter dem Namen der Datei, der Verzeichniseintrag ist aber nicht bestätigt
            // (nach einem Absturz könnte die alte Fassung wieder dastehen, und alles, was danach angehängt wird, fehlte). Das ist kein
            // Erfolg und wird nicht verschluckt: Das Verzeichnis gilt als unbestätigt, bis ein fsync es bestätigt hat. Jeder Wartepunkt
            // (`makeDurable`) versucht es zuerst erneut; solange es scheitert, ist jede Zeile ein Schreibfehler („nicht gesichert“).
            directoryUnsynced = true
        }
        BookTalkHistoryStore.excludeFromBackup(url)
    }

    /// Sichert, was geschrieben wurde: fsync der Datei, bei neu angelegter Datei auch das Verzeichnis (und dessen Eltern).
    private func makeDurable() throws {
        syncLock.withLock { syncCountValue += 1 }
        try fileSystem.syncFile(url)
        if directoryUnsynced {
            try fileSystem.syncDirectory(directory)
            try fileSystem.syncDirectory(directory.deletingLastPathComponent())
            directoryUnsynced = false
        }
    }

    // MARK: Dateizugriff

    /// `nil`: die Datei gibt es (noch) nicht. Jeder andere Fehler wird geworfen.
    static func readFile(_ url: URL) throws -> Data? {
        guard FileManager.default.fileExists(atPath: url.path) else { return nil }
        return try Data(contentsOf: url)
    }

    /// Hängt Bytes an (legt Verzeichnis und Datei bei Bedarf an). Ein Fehler lässt Bestehendes unberührt.
    static func appendFile(_ url: URL, _ data: Data) throws {
        let fm = FileManager.default
        if !fm.fileExists(atPath: url.path) {
            let dir = url.deletingLastPathComponent()
            try fm.createDirectory(at: dir, withIntermediateDirectories: true)
            BookTalkHistoryStore.excludeFromBackup(dir)
            #if os(iOS) || os(watchOS) || os(tvOS) || os(visionOS)
            let created = fm.createFile(atPath: url.path, contents: nil,
                                        attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication])
            #else
            let created = fm.createFile(atPath: url.path, contents: nil)
            #endif
            guard created else { throw AssistantAuditError.writeFailed }
            BookTalkHistoryStore.excludeFromBackup(url)
        }
        let handle = try FileHandle(forWritingTo: url)
        defer { try? handle.close() }
        try handle.seekToEnd()
        try handle.write(contentsOf: data)
    }

    private static func encoder() -> JSONEncoder {
        let e = JSONEncoder()
        e.dateEncodingStrategy = .iso8601
        e.outputFormatting = [.sortedKeys]
        return e
    }

    private static func splitLines(_ data: Data) -> [Data] {
        data.split(separator: UInt8(ascii: "\n"), omittingEmptySubsequences: true).map { Data($0) }
    }

    /// Zeilen im alten Format (Freitext `model`, flache Zähler) gehen beim Verdichten ins neue über: Art statt Freitext, Zähler in
    /// `counts`. Alles andere, auch Unlesbares oder von einer neueren Fassung, bleibt Byte für Byte.
    static func migrated(_ line: Data) -> Data {
        guard let object = (try? JSONSerialization.jsonObject(with: line)) as? [String: Any], object["t"] == nil,
              object["model"] != nil || object["grounded"] != nil || object["dropped"] != nil,
              let entry = try? AssistantJournalFold.decoder().decode(AssistantAuditEntry.self, from: line),
              let data = try? encoder().encode(entry) else { return line }
        return data
    }

    // MARK: Schreiben

    /// Hängt eine Zeile an (asynchron, in der Reihenfolge der Aufrufe). Kehrt sofort zurück; ein Fehler wird unter `key` gemerkt
    /// (Standard: die Anfrage der Zeile) und von `flush` gemeldet. Der Fehler hängt am Anspruch auf den Schlüssel
    /// (`claim`): Wer einen Schlüssel angibt, wartet auf ihn und hat damit den Anspruch (er endet mit `waitForLine`); die Anfrage
    /// legt ihren Anspruch beim Start an (`claim`). Ein Fehler geht nie verloren, und `hasFailed` zeigt ihn ohne Warten (Controller:
    /// laufende Ausgabe stoppen). `onFailure`: für Zeilen, auf die niemand wartet; ohne Anspruch, der Fehler geht nur an diese Stelle.
    /// `run`: die Zeile trägt die Laufnummer des Tokens, die beim ersten Schreiben vergeben wird (nach dem Öffnen der Datei).
    func enqueue(_ line: AssistantJournalLine, key: String? = nil, run: AssistantRunToken? = nil,
                 onFailure: (@Sendable (AssistantAuditError) -> Void)? = nil) {
        let k = key ?? line.req ?? ""
        if key != nil, onFailure == nil { _ = claim(k) }
        guard run != nil || (try? Self.encoder().encode(line)) != nil else {
            queue.async {
                self.note(k, .encodeFailed)
                onFailure?(.encodeFailed)
            }
            return
        }
        pendingLock.withLock { pending.append(Pending(line: line, run: run, key: k, onFailure: onFailure)) }
        queue.async { self.drain() }
    }

    /// Reiht mehrere Zeilen als EINE Gruppe ein (unter einem Lock, ein Anstoß der Warteschlange): Sie stehen hintereinander in der
    /// Reihenfolge der Liste und werden mit einem fsync gesichert, nie mit einem je Zeile. Jeder Schlüssel hat danach seinen Anspruch
    /// (der Fehler dieser Zeile hängt an ihm und wird mit `settle(keys:)` abgeholt).
    /// `dependent`: jede Zeile außer der ersten wird nur geschrieben, wenn die davor angehängt werden konnte (beide oder keine).
    func enqueue(group: [(line: AssistantJournalLine, key: String)], run: AssistantRunToken?, dependent: Bool = false) {
        guard !group.isEmpty else { return }
        for item in group { _ = claim(item.key) }
        pendingLock.withLock {
            for (offset, item) in group.enumerated() {
                pending.append(Pending(line: item.line, run: run, key: item.key, onFailure: nil, needsPrevious: dependent && offset > 0))
            }
        }
        queue.async { self.drain() }
    }

    /// Wartet, bis alles bis hierher Eingereihte geschrieben ist, holt den Fehler jedes Schlüssels ab (`nil`: steht dauerhaft) und
    /// beendet die Ansprüche: ein Warten für die ganze Gruppe.
    func settle(keys: [String]) async -> [String: AssistantAuditError] {
        await withCheckedContinuation { c in
            queue.async {
                var errors: [String: AssistantAuditError] = [:]
                for key in keys {
                    if let error = self.outcome(of: key, consume: true) { errors[key] = error }
                    self.release(key)
                }
                c.resume(returning: errors)
            }
        }
    }

    /// Schreibt alles Eingereihte in der Reihenfolge des Einreihens und sichert es mit EINEM fsync. Scheitert das fsync, gelten alle
    /// Zeilen dieser Gruppe als nicht geschrieben (Fehler unter ihrem Schlüssel): Eine Pflichtzeile, die nicht dauerhaft steht, gibt es nicht.
    private func drain() {
        let batch = pendingLock.withLock { () -> [Pending] in
            let all = pending
            pending.removeAll()
            return all
        }
        guard !batch.isEmpty else { return }
        var written: [Pending] = []
        // Die Zeilen einer Gruppe stehen im Stapel hintereinander (sie werden unter EINEM Lock angehängt): Die Zeile davor ist die
        // Vorgängerin in der Gruppe.
        var previousStands = true
        for item in batch {
            if item.needsPrevious, !previousStands {
                note(item.key, .writeFailed)
                item.onFailure?(.writeFailed)
                continue
            }
            previousStands = write(item)
            if previousStands { written.append(item) }
        }
        guard !written.isEmpty else { return }
        do {
            try makeDurable()
        } catch {
            torn = true   // das Ende der Datei ist unsicher: die nächste Zeile beginnt mit einem Zeilenumbruch
            for key in Set(written.map(\.key)) { note(key, .writeFailed) }
            for item in written { item.onFailure?(.writeFailed) }
            return   // eine gescheiterte Gruppe ist keine Gelegenheit (kein Nachholen im Kreis)
        }
        notifyDurable()
    }

    /// Wartet, bis alles bis hierher Eingereihte geschrieben ist. Gibt den Fehler zurück, der für `request` (oder ohne Anfrage:
    /// für Ereignisse) aufgetreten ist. Ein Anspruch behält ihn (`consume: false`) oder gibt ihn ab (`consume: true`, dann ist er
    /// gemeldet); ein Schlüssel ohne Anspruch vergisst ihn beim Abholen.
    func flush(request: String?, consume: Bool = true) async -> AssistantAuditError? {
        await withCheckedContinuation { c in
            queue.async {
                c.resume(returning: self.outcome(of: request ?? "", consume: consume))
            }
        }
    }

    /// Hat eine Zeile dieses Schlüssels einen Fehler gemeldet, der noch nicht abgeholt wurde? Ohne Warten und ohne Warteschlange
    /// (ein Lock): auch aus dem MainActor aufrufbar. Zeigt nur schon geschehene Fehler; für den aktuellen Stand `flush`. Dieselbe
    /// Quelle wie `flush` (der Anspruch).
    func hasFailed(_ key: String) -> Bool {
        failedLock.lock()
        defer { failedLock.unlock() }
        if let c = claims[key] { return c.error != nil }
        return orphans[key] != nil
    }

    /// Meldet den Anspruch auf einen Schlüssel an (Runde 8, R6; mehrfach gerufen: derselbe Anspruch). Von da an hängt der Fehler jeder
    /// Zeile unter diesem Schlüssel an ihm, bis `release` ihn beendet; er wird nie verdrängt.
    @discardableResult
    func claim(_ key: String) -> AssistantAuditClaim {
        failedLock.lock()
        defer { failedLock.unlock() }
        if let c = claims[key] { return c }
        let c = AssistantAuditClaim()
        claims[key] = c
        return c
    }

    /// Beendet den Anspruch (die Anfrage ist abgeschlossen, die Änderung gemeldet).
    func release(_ key: String) {
        failedLock.lock()
        claims[key] = nil
        failedLock.unlock()
    }

    /// Zahl der bestehenden Ansprüche (Tests: Ansprüche werden beendet).
    var claimCount: Int { failedLock.withLock { claims.count } }

    // Nur auf `queue`:
    private func outcome(of key: String, consume: Bool) -> AssistantAuditError? {
        failedLock.lock()
        defer { failedLock.unlock() }
        if let c = claims[key] { return consume ? c.take() : c.error }
        guard consume else { return orphans[key] }
        orphanOrder.removeAll { $0 == key }
        return orphans.removeValue(forKey: key)
    }

    private func note(_ key: String, _ error: AssistantAuditError) {
        failedLock.lock()
        if let c = claims[key] {
            c.record(error)
        } else {
            // Ein Schlüssel ohne Anspruch: Niemand wartet auf ihn, keine Entscheidung liest ihn. Die Obergrenze verdrängt nur solche
            // Fehler (der älteste zuerst); den Fehler einer Anfrage oder einer Änderung verdrängt sie nie.
            if orphans[key] == nil { orphanOrder.append(key) }
            orphans[key] = error
            while orphanOrder.count > Self.maxErrors { orphans[orphanOrder.removeFirst()] = nil }
        }
        let observer = failureObserver
        failedLock.unlock()
        observer?(key)
    }

    func setFailureObserver(_ observer: (@Sendable (String) -> Void)?) {
        failedLock.lock()
        failureObserver = observer
        failedLock.unlock()
    }

    /// Hinter einem eigenen, kurzen Lock (nie zusammen mit einem anderen gehalten): Wer wissen will, dass wieder eine Zeilengruppe
    /// DAUERHAFT steht (Runde 9, S3: jede solche Gruppe ist eine Gelegenheit, Fehlendes nachzuholen), je Besitzer ein Rückruf.
    private let durableLock = NSLock()
    private var durableObservers: [String: @Sendable () -> Void] = [:]

    /// Meldet (`observer`) oder entfernt (`nil`) den Rückruf eines Besitzers. Der Rückruf kommt von der Schreibwarteschlange, NACH dem
    /// fsync und außerhalb jedes Locks dieses Schreibers: kurz, ohne Warten, ohne I/O, nie in ein Lock, das beim Einreihen gehalten wird.
    func setDurableObserver(owner: String, _ observer: (@Sendable () -> Void)?) {
        durableLock.withLock { durableObservers[owner] = observer }
    }

    private func notifyDurable() {
        let observers = durableLock.withLock { Array(durableObservers.values) }
        for observer in observers { observer() }
    }

    /// `true`: die Zeile wurde angehängt (noch nicht dauerhaft: das deckt das fsync der Gruppe).
    private func write(_ item: Pending) -> Bool {
        let req = item.key
        func fail(_ error: AssistantAuditError) -> Bool {
            note(req, error)
            item.onFailure?(error)
            return false
        }
        guard openIfNeeded() else { return fail(openError) }
        var journal = item.line
        if let run = item.run { journal.run = run.resolve { maxRun += 1; return maxRun } }
        if let r = journal.run, r > maxRun { maxRun = r }
        guard let line = try? Self.encoder().encode(journal) else { return fail(.encodeFailed) }
        var data = Data()
        if torn { data.append(10) }
        data.append(line)
        data.append(10)
        do {
            try appender(url, data)
            torn = false
            return true
        } catch {
            torn = true
            return fail(.writeFailed)
        }
    }

    /// Einmal je Schreiber vor der ersten Zeile: offene Anfragen schließen, verdichten, altes Format übertragen. Ein Lesefehler
    /// ändert nichts und wird beim nächsten Mal erneut versucht.
    private func openIfNeeded() -> Bool {
        if opened { return true }
        let data: Data?
        do { data = try reader(url) } catch {
            openError = .readFailed
            return false
        }
        guard let data, !data.isEmpty else {
            // Die Datei gibt es nicht (oder sie ist leer): wird sie angelegt, bestätigt der Schreiber auch das Verzeichnis.
            if data == nil { directoryUnsynced = true }
            opened = true
            return true
        }
        torn = data.last != UInt8(ascii: "\n")
        var lines = Self.splitLines(data)
        let runs = lines.map { AssistantJournalFold.journal($0)?.run }
        maxRun = max(maxRun, runs.compactMap { $0 }.max() ?? 0)

        var keys: [String] = []
        var started: [String] = []
        var ended = Set<String>()
        var offered: [String: Set<String>] = [:]
        for (i, l) in lines.enumerated() {
            if let j = AssistantJournalFold.journal(l), let req = j.req, j.t != .event {
                keys.append("r:" + req)
                if j.t == .start, !started.contains(req) { started.append(req) }
                if j.t == .end { ended.insert(req) }
                if j.t == .areas, let a = j.areas { offered[req, default: []].formUnion(a) }
            } else {
                keys.append("l:\(i)")
            }
        }
        // Das Wiederherstellungs-Ende faltet die Bereiche aus den vorhandenen Zeilen `areas`; die Rundenart steht in ihrer Zeile
        // `round` und braucht nichts. Was das Prozessende überlebt hat, geht so nicht verloren.
        var closing: [Data] = []
        for req in started where !ended.contains(req) {
            var end = AssistantJournalLine(.end, req: req)
            end.at = Date()
            end.outcome = .incomplete
            end.areas = (offered[req] ?? []).sorted()
            // Runde 8 (Suchlauf d): Ein Ende, das sich nicht kodieren lässt, wird nicht übergangen (die Anfrage bliebe offen, ohne dass
            // jemand es erfährt), sondern ist ein Fehler beim Öffnen; beim nächsten Mal wird erneut versucht. Nicht einspielbar (der
            // Aufbau der Zeile ist fest), darum ohne Test.
            guard let bytes = try? Self.encoder().encode(end) else {
                openError = .encodeFailed
                return false
            }
            closing.append(bytes)
        }

        var order: [String] = []
        var seen = Set<String>()
        for k in keys where seen.insert(k).inserted { order.append(k) }
        var drop = Set(order.prefix(max(0, order.count - Self.maxUnits)))
        // Das Kürzen behält die höchste Laufnummer (Schreibfolge 7): Die Einheit der letzten Zeile mit ihr bleibt, damit die Zählung
        // nie zurückfällt und das Paar aus Lauf und Generation nie wiederverwendet wird.
        if let top = runs.compactMap({ $0 }).max(), let at = runs.lastIndex(where: { $0 == top }) { drop.remove(keys[at]) }
        let migrated = lines.map(Self.migrated)
        let changedFormat = zip(lines, migrated).contains { $0 != $1 }
        if !drop.isEmpty || changedFormat {
            lines = zip(migrated, keys).filter { !drop.contains($0.1) }.map(\.0) + closing
            var out = Data()
            for l in lines {
                out.append(l)
                out.append(10)
            }
            do {
                try compact(url, out)
                torn = false
                opened = true
                return true
            } catch {
                // Die alte Datei ist unversehrt; ohne Verdichtung weiterschreiben, beim nächsten Öffnen erneut versuchen.
            }
        }
        for l in closing {
            var bytes = Data()
            if torn { bytes.append(10) }
            bytes.append(l)
            bytes.append(10)
            do {
                try appender(url, bytes)
                torn = false
            } catch {
                torn = true
                openError = .writeFailed
                return false
            }
        }
        if !closing.isEmpty {
            do { try makeDurable() } catch {
                torn = true
                openError = .writeFailed
                return false
            }
        }
        opened = true
        return true
    }

    // MARK: Lesen und Löschen

    func read() async -> AssistantAuditRead {
        await withCheckedContinuation { c in queue.async { c.resume(returning: self.readNow()) } }
    }

    private func readNow() -> AssistantAuditRead {
        let data: Data?
        do { data = try reader(url) } catch { return AssistantAuditRead(entries: [], skippedLines: 0, error: .readFailed) }
        guard let data else { return AssistantAuditRead(entries: [], skippedLines: 0, error: nil) }
        let folded = AssistantJournalFold.fold(Self.splitLines(data))
        return AssistantAuditRead(entries: folded.entries, skippedLines: folded.skipped, error: nil)
    }

    /// Löscht die Protokolldatei (nur auf ausdrücklichen Wunsch). `false`, wenn sie sich nicht löschen ließ (die Datei bleibt dann, wie
    /// sie ist: Nichts geht verloren, und das Protokoll gilt weiter). Eine danach neu angelegte Datei hat einen unbestätigten
    /// Verzeichniseintrag: Der Schreiber öffnet sie wie beim ersten Mal neu (`opened = false`), das Verzeichnis gilt als unbestätigt.
    @discardableResult
    func clear() async -> Bool {
        await withCheckedContinuation { c in
            queue.async {
                do {
                    if FileManager.default.fileExists(atPath: self.url.path) { try FileManager.default.removeItem(at: self.url) }
                } catch {
                    c.resume(returning: false)
                    return
                }
                self.torn = false
                self.opened = false
                c.resume(returning: true)
            }
        }
    }
}

/// Ein Schreiber je Verzeichnis, prozessweit.
final class AssistantAuditRegistry: @unchecked Sendable {
    static let shared = AssistantAuditRegistry()
    private let lock = NSLock()
    private var writers: [String: AssistantAuditWriter] = [:]

    static func key(_ directory: URL) -> String { directory.standardizedFileURL.resolvingSymlinksInPath().path }

    static func writer(for directory: URL) -> AssistantAuditWriter { shared.writer(for: directory) }

    private func writer(for directory: URL) -> AssistantAuditWriter {
        lock.lock()
        defer { lock.unlock() }
        let k = Self.key(directory)
        if let w = writers[k] { return w }
        let w = AssistantAuditWriter(directory: directory)
        writers[k] = w
        return w
    }
}

/// Handle auf das Protokoll eines Verzeichnisses. Beliebig viele Handles teilen einen Schreiber (Registry).
public struct AssistantAuditLog: Sendable {
    /// Anfragen (und Ereignisse), auf die beim Öffnen verdichtet wird.
    public static let maxEntries = 500
    public static let fileName = "protokoll.jsonl"
    /// Nach so vielen Schritten endet eine Antwort (Ausgang „begrenzt“).
    public static let maxSteps = 200

    public let directory: URL
    private let writer: AssistantAuditWriter

    public init(directory: URL) {
        self.directory = directory
        self.writer = AssistantAuditRegistry.writer(for: directory)
    }

    /// Nur für Tests: eigener Schreiber (nicht in der Registry) mit austauschbarem Lesen und Schreiben.
    init(directory: URL, reader: @escaping @Sendable (URL) throws -> Data?,
         appender: @escaping @Sendable (URL, Data) throws -> Void = AssistantAuditWriter.appendFile,
         rewriter: (@Sendable (URL, Data) throws -> Void)? = nil,
         fileSystem: any AssistantStoreFileSystem = AssistantPosixStoreFileSystem()) {
        self.directory = directory
        self.writer = AssistantAuditWriter(directory: directory, reader: reader, appender: appender, rewriter: rewriter,
                                           fileSystem: fileSystem)
    }

    /// Zahl der fsync der Datei bisher (Tests, Messung der Wartepunkte vor dem ersten Ton).
    var syncCount: Int { writer.syncCount }

    /// Standardort der App: neben dem Verlauf in `Application Support/Assistant`.
    public static func standardDirectory() -> URL { AssistantHistoryStore.standardDirectory() }

    public var url: URL { directory.appendingPathComponent(Self.fileName) }

    // MARK: Journal (Kern)

    /// Reiht eine Zeile ein; kehrt sofort zurück. `key`: unter ihm wird ein Schreibfehler gemerkt (Standard: die Anfrage).
    func enqueue(_ line: AssistantJournalLine, key: String? = nil, run: AssistantRunToken? = nil,
                 onFailure: (@Sendable (AssistantAuditError) -> Void)? = nil) {
        writer.enqueue(line, key: key, run: run, onFailure: onFailure)
    }

    /// Der Anspruch der Anfrage (Runde 8, R6): ab hier hängt der Fehler jeder Zeile der Anfrage an ihrem Anspruch (nie verdrängt, auch
    /// nicht von Fehlern anderer Schlüssel). Er endet mit `release`.
    @discardableResult
    func claim(request: String) -> AssistantAuditClaim { writer.claim(request) }

    /// Beendet den Anspruch der Anfrage (abgeschlossen).
    func release(request: String) { writer.release(request) }

    /// Anzahl der bestehenden Ansprüche (Tests).
    var claimCount: Int { writer.claimCount }

    /// Schreibt die erste Zeile einer Anfrage und wartet, bis sie steht. `false` bei einem Fehler; der Fehler bleibt für den
    /// Abschluss der Anfrage (`flush`) gemerkt, damit er nie verloren geht.
    func begin(_ line: AssistantJournalLine, run: AssistantRunToken? = nil) async -> Bool {
        writer.enqueue(line, key: nil, run: run)
        return await writer.flush(request: line.req, consume: false) == nil
    }

    /// Wartet, bis alles bis hierher Eingereihte geschrieben ist, und gibt den Fehler der Anfrage zurück (`nil`: alles steht in
    /// der Datei). Der Fehler bleibt für den Abschluss der Anfrage gemerkt und ist klebrig: Er geht nie verloren.
    @discardableResult
    func waitUntilWritten(request: String) async -> AssistantAuditError? { await writer.flush(request: request, consume: false) }

    /// Hat eine Zeile dieser Anfrage schon einen Schreibfehler gemeldet? Ohne Warten, aus jedem Thread.
    public func hasFailed(request: String) -> Bool { writer.hasFailed(request) }

    /// Meldet jeden Schreibfehler einer Zeile mit dem Schlüssel der Anfrage (bzw. der Zeile), sobald er geschieht. Der Aufruf kommt
    /// von der Schreibwarteschlange: kurz, ohne Warten, ohne Rückruf in die Ordnung (Controller: auf den Hauptthread wechseln und die
    /// Ausgabe stoppen). Ein Fehler vor dem Anmelden wird nicht nachgemeldet (`hasFailed` zeigt ihn).
    public func setFailureObserver(_ observer: (@Sendable (String) -> Void)?) { writer.setFailureObserver(observer) }

    /// Schreibt Zeilen als EINE Gruppe (ein fsync) und wartet einmal. Gibt je Schlüssel den Fehler zurück; ein Schlüssel ohne Eintrag
    /// steht dauerhaft in der Datei. `dependent`: beide oder keine (eine Zeile wird nur geschrieben, wenn die davor angehängt wurde).
    func writeGroup(_ group: [(line: AssistantJournalLine, key: String)], run: AssistantRunToken?,
                    dependent: Bool = false) async -> [String: AssistantAuditError] {
        writer.enqueue(group: group, run: run, dependent: dependent)
        return await writer.settle(keys: group.map(\.key))
    }

    /// Meldet den Rückruf eines Besitzers für jede dauerhaft gesicherte Zeilengruppe (siehe `AssistantAuditWriter`).
    func setDurableObserver(owner: String, _ observer: (@Sendable () -> Void)?) { writer.setDurableObserver(owner: owner, observer) }

    /// Beide Handles schreiben über denselben Schreiber.
    func sharesWriter(with other: AssistantAuditLog) -> Bool { writer === other.writer }

    /// Wartet auf die Zeile mit diesem Schlüssel (`release:<Generation>`, `state`) und holt ihren Fehler ab.
    func waitForLine(key: String) async -> AssistantAuditError? {
        let error = await writer.flush(request: key)
        writer.release(key)
        return error
    }

    /// Wartet, bis alle bisher eingereihten Zeilen ohne Anfrage (Ereignisse) geschrieben sind. Der Fehler dabei
    /// (Speicher voll, Datei nicht lesbar), sonst `nil`; er wird damit gemeldet und vergessen.
    public func flushReleaseLines() async -> AssistantAuditError? { await writer.flush(request: nil) }

    /// Wartet, bis alle Zeilen der Anfrage geschrieben sind. Der Fehler dieser Anfrage, sonst `nil`; er wird damit abgeholt.
    func flush(request: String) async -> AssistantAuditError? { await writer.flush(request: request) }

    /// Wartet, bis alles Eingereihte steht (die Warteschlange ist leer), ohne einen Fehler abzuholen (Sperre vor einem Lesen).
    func barrier() async { _ = await writer.flush(request: nil, consume: false) }

    // MARK: Ereignisse und Lesen

    /// Hängt ein Ereignis an (Einwilligung, Widerruf). Antworten schreibt der Kern als Journal (`start`, `step`, `commit`,
    /// `end`); ein Eintrag der Art `answer` wird hier nicht angenommen. Ein Fehler (Speicher voll, Datei nicht lesbar) lässt die
    /// bisherige Datei unverändert.
    public func append(_ entry: AssistantAuditEntry) async -> AssistantAuditError? {
        guard entry.kind != .answer else { return .encodeFailed }
        var line = AssistantJournalLine(.event)
        line.at = entry.at
        line.kind = entry.kind
        line.model = entry.modelKind
        line.cloud = entry.cloud
        line.unsourced = entry.unsourcedMode
        line.consent = entry.consentVersion
        line.gen = entry.generation
        // Ein eigener Schlüssel je Ereignis (Anspruch): Der Fehler dieses Ereignisses hängt an seinem Anspruch, nicht an einem
        // gemeinsamen Schlüssel, den die Fehler anderer Zeilen verdrängen könnten.
        let key = "event:" + UUID().uuidString
        writer.enqueue(line, key: key)
        return await waitForLine(key: key)
    }

    /// Wie `append`, mit `false` bei einem Fehler.
    @discardableResult
    public func record(_ entry: AssistantAuditEntry) async -> Bool { await append(entry) == nil }

    public func read() async -> AssistantAuditRead { await writer.read() }

    /// Alle lesbaren Einträge, älteste zuerst; beschädigte Zeilen (abgebrochener Schreibvorgang) werden übersprungen.
    public func entries() async -> [AssistantAuditEntry] { await writer.read().entries }

    /// Nur auf ausdrücklichen Wunsch (z. B. Prüfung); „Neues Gespräch“ löscht das Protokoll nicht.
    @discardableResult
    public func clear() async -> Bool { await writer.clear() }
}
