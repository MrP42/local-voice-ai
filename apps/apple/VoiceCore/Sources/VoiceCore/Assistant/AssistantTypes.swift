import Foundation

// Gesprächsmodus „Sprechen“ (Paket K): gemeinsame Typen. Reines Foundation, unter Linux testbar.
// Namen und Signaturen sind mit Paket A abgestimmt (assistent-entwurf.md); Erweiterungen ja, Umbenennungen nein.

/// Bereiche, aus denen der Assistent Inhalte beziehen darf.
public enum AssistantArea: String, Sendable, Codable, CaseIterable {
    case books, transcripts, files, notes

    /// Reihenfolge für Bestand und Abruf (Bücher zuerst).
    var order: Int {
        switch self {
        case .books: 0
        case .transcripts: 1
        case .files: 2
        case .notes: 3
        }
    }

    /// Kopfzeile im Bestand (Mehrzahl).
    public var inventoryLabel: String {
        switch self {
        case .books: "Bücher"
        case .transcripts: "Transkripte"
        case .files: "Dokumente"
        case .notes: "Notizen"
        }
    }

    /// „keine Bücher“, „ein Buch“, „3 Bücher“ (für Bestandsangaben des Systems).
    func countPhrase(_ n: Int) -> String {
        switch (self, n) {
        case (.books, 0): "keine Bücher"
        case (.books, 1): "ein Buch"
        case (.transcripts, 0): "keine Transkripte"
        case (.transcripts, 1): "ein Transkript"
        case (.files, 0): "keine Dokumente"
        case (.files, 1): "ein Dokument"
        case (.notes, 0): "keine Notizen"
        case (.notes, 1): "eine Notiz"
        default: "\(n) \(inventoryLabel)"
        }
    }

    /// Kennzeichnung eines Auszugs im Prompt (Einzahl).
    public var singularLabel: String {
        switch self {
        case .books: "Buch"
        case .transcripts: "Transkript"
        case .files: "Dokument"
        case .notes: "Notiz"
        }
    }
}

/// Art der Quelle eines Blocks (E4.2b, Klasse B und Quelltextauszüge). Nur vier Arten haben einen festgelegten Quellblock und
/// sind zitierbar; für alles andere wird kein Quelltextauszug ausgegeben, bis ein Nachtrag die Einheit je Quellart festlegt.
public enum AssistantSourceKind: String, Sendable, Codable, CaseIterable {
    /// Buch: ein Absatz.
    case book
    /// Getippte Notiz (auch der gespeicherte Text einer Sprachnotiz): Text zwischen Leerzeilen.
    case typedNote
    /// Importiertes Dokument (Ablage): der Block, wie er beim Import gespeichert wurde.
    case importedDocument
    /// Transkript einer Besprechung: das Segment, wie es bei der Erkennung gespeichert wurde.
    case transcript
    /// Von der App erzeugter Text: Protokoll (Sprachnotiz, Besprechung), Antworten der App. Nie zitierbar.
    case generated
    /// Mail: kein Quellblock festgelegt, nie zitierbar.
    case mail
    /// Kalendereintrag: kein Quellblock festgelegt, nie zitierbar.
    case calendar

    /// Zitierbar: genau die vier Arten mit festgelegtem Quellblock.
    public var isQuotable: Bool {
        switch self {
        case .book, .typedNote, .importedDocument, .transcript: true
        case .generated, .mail, .calendar: false
        }
    }
}

public struct AssistantItem: Sendable, Equatable, Codable {
    public let area: AssistantArea
    public let id: String
    public let title: String
    public let detail: String?      // „Autor · 42 % gelesen“, „12.10.2026 · 48 min“
    public let updated: Date?

    public init(area: AssistantArea, id: String, title: String, detail: String? = nil, updated: Date? = nil) {
        self.area = area
        self.id = id
        self.title = title
        self.detail = detail
        self.updated = updated
    }
}

public struct AssistantPassage: Sendable, Equatable, Codable {
    public let area: AssistantArea
    public let itemId: String
    public let title: String
    public let locator: String?     // „S. 12“, „00:14:03“, „Seite 2“
    public let text: String
    public let score: Double
    /// Der Text ist GENAU ein gespeicherter Quellblock (E4.2b): ganz, wortgetreu, nicht gekürzt, nicht geteilt, nicht mit einem
    /// anderen zusammengezogen und höchstens `AssistantGrounding.maxBlockChars` lang. Nur ein solcher Auszug ist zitierbar. Ohne
    /// Nachweis (Standard, auch bei Suchfenstern der Ablage) wird er nie ausgegeben. Wird nicht gespeichert.
    public let isBlock: Bool
    /// Die Quellart des Blocks lässt Quelltextauszüge zu (`AssistantSourceKind.isQuotable`: Buch, getippte Notiz, importiertes
    /// Dokument, Transkript). Erzeugter Text (Protokoll einer Sprachnotiz oder Besprechung, Antworten der App), Mails und
    /// Kalendereinträge sind nicht zitierbar. Fail-closed: Ohne ausdrückliche Angabe gilt der Auszug als nicht zitierbar. Wird
    /// nicht gespeichert.
    public let quotable: Bool

    public init(area: AssistantArea, itemId: String, title: String, locator: String? = nil, text: String, score: Double,
                isBlock: Bool = false, quotable: Bool = false) {
        self.area = area
        self.itemId = itemId
        self.title = title
        self.locator = locator
        self.text = text
        self.score = score
        self.isBlock = isBlock
        self.quotable = quotable
    }

    private enum CodingKeys: String, CodingKey { case area, itemId, title, locator, text, score }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        self.init(area: try c.decode(AssistantArea.self, forKey: .area), itemId: try c.decode(String.self, forKey: .itemId),
                  title: try c.decode(String.self, forKey: .title), locator: try c.decodeIfPresent(String.self, forKey: .locator),
                  text: try c.decode(String.self, forKey: .text), score: try c.decode(Double.self, forKey: .score))
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(area, forKey: .area)
        try c.encode(itemId, forKey: .itemId)
        try c.encode(title, forKey: .title)
        try c.encodeIfPresent(locator, forKey: .locator)
        try c.encode(text, forKey: .text)
        try c.encode(score, forKey: .score)
    }

    /// Dieselbe Stelle mit anderem Text und Punktwert. Ein veränderter (gekürzter) Text ist nie mehr ein ganzer Quellblock.
    func replacing(text newText: String, score newScore: Double? = nil) -> AssistantPassage {
        AssistantPassage(area: area, itemId: itemId, title: title, locator: locator, text: newText, score: newScore ?? score,
                         isBlock: isBlock && newText == text, quotable: quotable)
    }

    /// Auf höchstens `limit` Zeichen gekürzt (`BookTalkPrompt.clip`); ein gekürzter Text ist kein Block mehr.
    func clipped(to limit: Int) -> AssistantPassage { replacing(text: BookTalkPrompt.clip(text, to: limit)) }

    /// Anzeigezeile der Quelle: „Buch · Titel · S. 12“, „Transkript · Titel · 00:14:03“, „Ablage · Titel“, „Notiz · Datum“.
    public var sourceLabel: String {
        switch area {
        case .books: return ["Buch", title, locator ?? ""].filter { !$0.isEmpty }.joined(separator: " · ")
        case .transcripts: return ["Transkript", title, locator ?? ""].filter { !$0.isEmpty }.joined(separator: " · ")
        case .files: return "Ablage · " + title
        case .notes: return "Notiz · " + (locator ?? title)
        }
    }

    /// Beginn der Quellzeile jedes Zitats und Wortlaut der hörbaren Ansage („Wörtlicher Auszug aus ‚Titel‘:“).
    public static let excerptPrefix = "Wörtlicher Auszug aus "

    /// Die Quelle ohne Bereichswort, wo der Titel sie trägt: „Beispielbuch · S. 12“, „Besprechung Nordlicht · 00:14:03“,
    /// „Ablage · Name“, „Notiz · 11.11.2026, 14:03“.
    public var excerptSource: String {
        switch area {
        case .books, .transcripts:
            let parts = [title, locator ?? ""].filter { !$0.isEmpty }
            return parts.isEmpty ? sourceLabel : parts.joined(separator: " · ")
        case .files, .notes: return sourceLabel
        }
    }

    /// Quellzeile unter einem Zitat: „Wörtlicher Auszug aus Beispielbuch · S. 12“. Ein Block kann ein Fragment sein oder Zusammenhang
    /// auslassen; die Zeile sagt, dass es ein Auszug ist.
    public var excerptLine: String { Self.excerptPrefix + excerptSource }
}

/// Quelle mit eigenem Index (Ablage). Paket A liefert die Umsetzung.
public protocol AssistantSource: Sendable {
    var area: AssistantArea { get }
    func items() async -> [AssistantItem]
    func search(_ question: String, limit: Int) async -> [AssistantPassage]
}

/// Dokument für den eingebauten Index (Bücher, Transkripte, Notizen).
public struct AssistantDocument: Sendable {
    /// Ein Quellblock: der gespeicherte Text zwischen strukturellen Grenzen der Quelle (Absatz, Notizblock, Segment), ganz und
    /// wortgetreu. Die App erzeugt keine Grenze aus Satzzeichen. Zitiert wird nur ein ganzer Block, nie ein Teil davon.
    public struct Section: Sendable {
        public let locator: String?
        public let text: String
        /// Art der Quelle dieses Blocks. Ohne Vorgabewert: Jede Quelle legt sie ausdrücklich fest, eine vergessene Art kann nie
        /// stillschweigend zitierbar werden.
        public let kind: AssistantSourceKind

        /// Der Block darf wörtlich ausgegeben werden (nur Buch, getippte Notiz, importiertes Dokument, Transkript).
        public var quotable: Bool { kind.isQuotable }

        public init(locator: String?, text: String, kind: AssistantSourceKind) {
            self.locator = locator
            self.text = text
            self.kind = kind
        }
    }

    public let item: AssistantItem
    /// Ändert sich, wenn Text oder Lesegrenze sich ändern.
    public let version: String
    public let sections: [Section]

    public init(item: AssistantItem, version: String, sections: [Section]) {
        self.item = item
        self.version = version
        self.sections = sections
    }
}

/// Einordnung eines ausgegebenen Satzes (E4.2a): Zitat (ganzer Quellblock), Bestandsangabe, Satz ohne Quelle, Vorlage; verworfene Sätze
/// kommen nie als Ereignis. Dazu die hörbare Ansage.
public enum AssistantSentenceStatus: String, Sendable, Codable, Equatable {
    /// Wörtlicher Auszug: ein ganzer gespeicherter Quellblock, unverändert übernommen (Quelle = dieser Block).
    case grounded
    /// Bestandsangabe: vom System unverändert aus den Bestandsdaten erzeugt (Zahl je Bereich, Titel), nie vom Modell.
    case inventory
    /// Allgemeinwissen ohne Quelle; nur in einer Allgemeinrunde und nur mit Opt-in.
    case unsourced
    /// Vorlage: Gruß, Dank, feste Rückfrage, fester Satz, Uhrzeit und Datum aus der Systemuhr. Sagt nichts über eigene Inhalte.
    case neutral
    /// Hörbare Ansage („Ohne Quelle:“, „Wörtlicher Auszug aus ‚Titel‘:“, „Aus deinen Inhalten:“). Gehört nicht zum Antworttext.
    case announcement
}

/// Art einer Runde, vor dem Modellaufruf festgelegt (`AssistantContext`). Datenrunde ist der Standard:
/// Über eigene Daten gibt es nur wörtliche Auszüge (ganze Quellblöcke) und Bestandsangaben des Systems. Freier Modelltext kommt nur in der
/// Allgemeinrunde vor (mit Opt-in, ohne Auszüge und ohne Bestand im Prompt) und ist dort stets „ohne Quelle“.
public enum AssistantRoundKind: String, Sendable, Codable, Equatable {
    case data, general
}

/// Grund, aus dem ein Satz nicht ausgegeben wurde (Zähler im Protokoll, keine Inhalte).
public enum AssistantDropReason: String, Sendable, Codable, Equatable, CaseIterable {
    /// Behauptung über eigene Daten (Eigendatenbezug) ohne Deckung.
    case ownData
    /// Satz ohne Quelle, aber der Modus ist (nicht mehr) an.
    case noMode
    /// In der Datenrunde auf keinen Quellblock eingerastet (freier Modelltext).
    case notSnapped
    /// Dasselbe Zitat zum zweiten Mal.
    case duplicate
    /// Modelltext mit dem Wortlaut einer Fehl-Vorlage (die spricht nur das System) oder eine Satzfolge ohne Ende.
    case template
    /// Durch Widerruf (Freigabe-Generation) veraltet.
    case revoked
    /// Rest ohne Satzzeichen am Stromende (abgeschnittener Satz): wird nie ausgegeben.
    case fragment
}

/// Abschnitt eines Beitrags mit gleicher Kennzeichnung (gespeichert, wenn der Beitrag Zitate oder Sätze mit
/// Kennzeichnung enthält oder Sätze ohne Quelle mit anderen gemischt sind).
///
/// Marke je Satz: In der Datenrunde ist jeder angezeigte Satz entweder ein Zitat mit genau EINER Quellzeile (`quote`, `source`)
/// oder trägt eine eigene Kennzeichnung (`note`: feste Antwort des Systems, Bestandsangabe), weil er strukturell keine Quelle hat.
public struct AssistantTurnPart: Sendable, Equatable, Codable {
    public var text: String
    public var unsourced: Bool
    /// Wörtlicher Auszug (ganzer Quellblock): wird mit Anführungszeichen angezeigt.
    public var quote: Bool
    /// Anzeigezeile der Quelle eines Zitats (`AssistantPassage.excerptLine`: „Wörtlicher Auszug aus … · S. 12“), unter dem Zitat angezeigt. Nur die Zeile,
    /// nicht der Auszug. Alte Beiträge und alles, was kein Zitat ist: `nil`.
    public var source: String?
    /// Kennzeichnung eines Satzes OHNE Quellzeile, der nicht aus einer Quelle stammt: `templateNote` (feste Antwort des Systems:
    /// Gruß, feste Rückfrage, „dazu finde ich nichts“, Uhrzeit) oder `inventoryNote` (Bestandsangabe aus den Zahlen und Titeln
    /// des Bestands). Nie bei Zitaten; alte Beiträge: `nil`.
    public var note: String?

    public static let templateNote = "Feste Antwort"
    public static let inventoryNote = "Aus deinem Bestand"

    public init(text: String, unsourced: Bool, quote: Bool = false, source: String? = nil, note: String? = nil) {
        self.text = text
        self.unsourced = unsourced
        self.quote = quote
        self.source = source
        self.note = note
    }

    private enum CodingKeys: String, CodingKey { case text, unsourced, quote, source, note }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        text = try c.decode(String.self, forKey: .text)
        unsourced = try c.decode(Bool.self, forKey: .unsourced)
        quote = try c.decodeIfPresent(Bool.self, forKey: .quote) ?? false
        source = try c.decodeIfPresent(String.self, forKey: .source)
        note = try c.decodeIfPresent(String.self, forKey: .note)
    }

    /// Kennzeichnung eines Satzes nach seiner Einordnung; `nil`, wenn er eine Quellzeile oder die Kennzeichnung „Ohne Quelle“
    /// (ältere Beiträge) trägt.
    public static func note(for status: AssistantSentenceStatus, quote: Bool) -> String? {
        guard !quote else { return nil }
        switch status {
        case .neutral: return templateNote
        case .inventory: return inventoryNote
        case .grounded, .unsourced, .announcement: return nil
        }
    }

    /// Hängt einen ausgegebenen Satz an die Abschnitte an (dieselbe Zuordnung für die Anzeige der laufenden Antwort und den
    /// Beitrag im Verlauf): Ein Zitat bekommt einen eigenen Abschnitt mit seiner EINEN Quellzeile; Sätze gleicher Art und gleicher
    /// Kennzeichnung (ohne Zitat) werden zu einem Abschnitt.
    public static func append(to list: inout [AssistantTurnPart], text: String, status: AssistantSentenceStatus, quote: Bool,
                              sources: [AssistantPassage]) {
        let unsourced = status == .unsourced
        let note = note(for: status, quote: quote)
        if !quote, let last = list.last, !last.quote, last.unsourced == unsourced, last.note == note {
            list[list.count - 1].text += " " + text
        } else {
            list.append(AssistantTurnPart(text: text, unsourced: unsourced, quote: quote, source: quote ? sources.first?.excerptLine : nil,
                                          note: note))
        }
    }
}

/// Ein Beitrag im Verlauf: Äußerung des Nutzers und (gesprochene) Antwort.
public struct AssistantTurn: Sendable, Equatable, Codable {
    public var user: String
    public var assistant: String
    public var interrupted: Bool
    /// Nur Quellen, die mindestens einen gesprochenen Satz decken.
    public var sources: [AssistantPassage]
    public var general: Bool          // ohne deckenden Auszug beantwortet
    /// Mindestens ein unbelegter Satz wurde gesprochen. Alte Beiträge ohne das Feld: wie `general`.
    public var unsourced: Bool
    /// Gemischte Antwort: Abschnitte in Sprechreihenfolge mit Kennzeichnung; sonst leer (dann gilt `unsourced` für alles).
    public var parts: [AssistantTurnPart]
    public var model: String
    public var at: Date
    /// Rundenart des Beitrags. Alte Beiträge ohne das Feld: `nil` (gelten für die Prompt-Auswahl als Datenrunde).
    public var round: AssistantRoundKind?

    public init(user: String, assistant: String, interrupted: Bool = false, sources: [AssistantPassage] = [],
                general: Bool = false, unsourced: Bool = false, parts: [AssistantTurnPart] = [], model: String = "", at: Date = Date(),
                round: AssistantRoundKind? = nil) {
        self.user = user
        self.assistant = assistant
        self.interrupted = interrupted
        self.sources = sources
        self.general = general
        self.unsourced = unsourced
        self.parts = parts
        self.model = model
        self.at = at
        self.round = round
    }

    private enum CodingKeys: String, CodingKey {
        case user, assistant, interrupted, sources, general, unsourced, parts, model, at, round
    }

    /// Beitrag aus einer Runde ohne Quelle: Rundenart `.general`, ein Satz ohne Quelle oder ein Abschnitt ohne Quelle. Alte
    /// Beiträge ohne das Feld `unsourced` gelten über `general` ebenso (vorsichtig: im Zweifel ohne Quelle).
    public var isUnsourced: Bool { round == .general || unsourced || parts.contains { $0.unsourced } }

    /// Die Beiträge, die ein Gespräch ANZEIGEN und in einen Prompt geben darf. Ist die Funktion „Antworten ohne Quelle“ nicht
    /// freigeschaltet (`released == false`), fallen alle Beiträge aus Runden ohne Quelle heraus, bevor sie angezeigt werden oder
    /// in einen Prompt gelangen. Reine Funktion; die Datei bleibt, bis der nächste Verlauf gespeichert wird, dann fehlen sie.
    public static func admitted(_ turns: [AssistantTurn], released: Bool) -> [AssistantTurn] {
        released ? turns : turns.filter { !$0.isUnsourced }
    }

    /// Die synthetisierte Fassung ignoriert Standardwerte: Beiträge aus Verlaufsdateien vor dem Feld `unsourced` müssen laden.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        user = try c.decode(String.self, forKey: .user)
        assistant = try c.decode(String.self, forKey: .assistant)
        interrupted = try c.decode(Bool.self, forKey: .interrupted)
        sources = try c.decode([AssistantPassage].self, forKey: .sources)
        general = try c.decode(Bool.self, forKey: .general)
        unsourced = try c.decodeIfPresent(Bool.self, forKey: .unsourced) ?? general
        parts = try c.decodeIfPresent([AssistantTurnPart].self, forKey: .parts) ?? []
        model = try c.decode(String.self, forKey: .model)
        at = try c.decode(Date.self, forKey: .at)
        round = try c.decodeIfPresent(AssistantRoundKind.self, forKey: .round)
    }
}

public struct AssistantConfig: Sendable {
    public var modelLabel: String
    public var isCloud: Bool
    public var contextTokens: Int      // Apple 4096, Gemma laut Budget, Cloud ≥ 16000
    public var maxAnswerTokens: Int    // Apple 400, sonst 700
    public var userName: String?
    public var timeout: TimeInterval
    /// Antworten ohne Quelle (Allgemeinwissen) erlaubt. Standard: aus. Die Sitzung kann es zur Laufzeit ändern.
    public var allowUnsourced: Bool
    /// Version des Einwilligungstextes zu `allowUnsourced` (nur für das Protokoll).
    public var consentVersion: Int

    public init(modelLabel: String, isCloud: Bool, contextTokens: Int, maxAnswerTokens: Int, userName: String?, timeout: TimeInterval,
                allowUnsourced: Bool = false, consentVersion: Int = 0) {
        self.modelLabel = modelLabel
        self.isCloud = isCloud
        self.contextTokens = contextTokens
        self.maxAnswerTokens = maxAnswerTokens
        self.userName = userName
        self.timeout = timeout
        self.allowUnsourced = allowUnsourced
        self.consentVersion = consentVersion
    }
}

public struct AssistantRequest: Sendable, Equatable {
    public let requestId: String
    public let text: String
    public let afterInterruption: Bool

    public init(requestId: String, text: String, afterInterruption: Bool = false) {
        self.requestId = requestId
        self.text = text
        self.afterInterruption = afterInterruption
    }
}

/// Zahl der Sätze je Einordnung in einer Antwort (für Protokoll und Anzeige; keine Inhalte). Fünf Klassen: Zitat
/// (`grounded`), Bestandsangabe (`inventory`), Satz ohne Quelle (`unsourced`), Vorlage (`neutral`), verworfen (je Grund).
public struct AssistantSentenceCounts: Sendable, Equatable, Codable {
    public var grounded = 0
    public var inventory = 0
    public var unsourced = 0
    public var neutral = 0
    public var droppedOwnData = 0
    public var droppedNoMode = 0
    public var droppedNotSnapped = 0
    public var droppedDuplicate = 0
    public var droppedTemplate = 0
    /// Verworfen, weil die Freigabe-Generation veraltet war (Widerruf).
    public var revoked = 0
    /// Verworfen: Rest ohne Satzzeichen am Stromende.
    public var droppedFragment = 0

    public init(grounded: Int = 0, inventory: Int = 0, unsourced: Int = 0, neutral: Int = 0, droppedOwnData: Int = 0,
                droppedNoMode: Int = 0, droppedNotSnapped: Int = 0, droppedDuplicate: Int = 0, droppedTemplate: Int = 0,
                revoked: Int = 0, droppedFragment: Int = 0) {
        self.grounded = grounded
        self.inventory = inventory
        self.unsourced = unsourced
        self.neutral = neutral
        self.droppedOwnData = droppedOwnData
        self.droppedNoMode = droppedNoMode
        self.droppedNotSnapped = droppedNotSnapped
        self.droppedDuplicate = droppedDuplicate
        self.droppedTemplate = droppedTemplate
        self.revoked = revoked
        self.droppedFragment = droppedFragment
    }

    private enum CodingKeys: String, CodingKey {
        case grounded, inventory, unsourced, neutral, droppedOwnData, droppedNoMode, droppedNotSnapped, droppedDuplicate, droppedTemplate
        case revoked, droppedFragment
    }

    /// Fehlende Zähler gelten als 0: Zeilen älterer und neuerer Fassungen bleiben lesbar.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        func count(_ key: CodingKeys) throws -> Int { try c.decodeIfPresent(Int.self, forKey: key) ?? 0 }
        self.init(grounded: try count(.grounded), inventory: try count(.inventory), unsourced: try count(.unsourced),
                  neutral: try count(.neutral), droppedOwnData: try count(.droppedOwnData), droppedNoMode: try count(.droppedNoMode),
                  droppedNotSnapped: try count(.droppedNotSnapped), droppedDuplicate: try count(.droppedDuplicate),
                  droppedTemplate: try count(.droppedTemplate), revoked: try count(.revoked),
                  droppedFragment: try count(.droppedFragment))
    }

    /// Alle verworfenen Sätze (nicht gesprochen, nicht angezeigt), unabhängig vom Grund.
    public var dropped: Int {
        droppedOwnData + droppedNoMode + droppedNotSnapped + droppedDuplicate + droppedTemplate + revoked + droppedFragment
    }

    public mutating func drop(_ reason: AssistantDropReason) {
        switch reason {
        case .ownData: droppedOwnData += 1
        case .noMode: droppedNoMode += 1
        case .notSnapped: droppedNotSnapped += 1
        case .duplicate: droppedDuplicate += 1
        case .template: droppedTemplate += 1
        case .revoked: revoked += 1
        case .fragment: droppedFragment += 1
        }
    }

    /// Ausgegebene Sätze (ohne Ansagen).
    public var emitted: Int { grounded + inventory + unsourced + neutral }
}

public struct AssistantAnswer: Sendable, Equatable {
    /// Die gesprochenen und angezeigten Sätze (ohne Ansagen, ohne verworfene).
    public let text: String
    /// Nur Quellen, die mindestens einen ausgegebenen Satz decken.
    public let sources: [AssistantPassage]
    public let general: Bool
    public let firstSentenceMs: Int?
    public let totalMs: Int
    /// Mindestens ein Satz ohne Quelle wurde ausgegeben.
    public let unsourced: Bool
    public let counts: AssistantSentenceCounts
    public let round: AssistantRoundKind
    /// Freigabe-Generation, unter der die Antwort lief.
    public let generation: Int

    public init(text: String, sources: [AssistantPassage], general: Bool, firstSentenceMs: Int?, totalMs: Int,
                unsourced: Bool = false, counts: AssistantSentenceCounts = AssistantSentenceCounts(),
                round: AssistantRoundKind = .data, generation: Int = 0) {
        self.text = text
        self.sources = sources
        self.general = general
        self.firstSentenceMs = firstSentenceMs
        self.totalMs = totalMs
        self.unsourced = unsourced
        self.counts = counts
        self.round = round
        self.generation = generation
    }
}

public enum AssistantFailure: String, Sendable, Equatable {
    case cancelled, timeout, noModel, modelFailed, offline, consentRequired, revoked
    /// Das Protokoll ließ sich nicht schreiben (Pflichtzeile `start`, `round` oder `areas`): kein Modellaufruf, keine Ausgabe.
    case auditUnavailable
}

public enum AssistantEvent: Sendable, Equatable {
    /// Abgerufene Auszüge dieser Anfrage (Vorab-Anzeige; in einer Allgemeinrunde leer). Gedeckt wird erst je Satz.
    case sources(requestId: String, generation: Int, [AssistantPassage])
    /// Ein Satz zur Ausgabe. `speech` = ohne Markdown, sprechbar. `sources`: bei einem Zitat sein Block, sonst leer.
    /// `quote`: wörtlicher Auszug, mit Anführungszeichen anzeigen. `generation`: Freigabe-Generation des Laufs; wer sie nicht mehr
    /// kennt (Widerruf), zeigt und spricht nichts. Verworfene Sätze kommen nie.
    case sentence(requestId: String, generation: Int, text: String, speech: String, status: AssistantSentenceStatus,
                  sources: [AssistantPassage], quote: Bool)
    case done(requestId: String, AssistantAnswer)
    case failed(requestId: String, AssistantFailure)
}
