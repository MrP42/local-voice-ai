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
    /// Der ROHE gespeicherte Block: genau der Teil des gespeicherten Quelltextes zwischen den Grenzen des Blocks, Zeichen für
    /// Zeichen (Zeilentrenner, Leerraum, Unicode-Form unverändert; Bytefolge UTF-8 gleich). Nur DIESER Text wird bei einem Zitat
    /// ausgegeben. `text` ist der aufbereitete Text für Suche, Abruf und Prompt und wird nie ausgegeben. `nil`: es gibt keinen
    /// eindeutigen Rohtext (nie die aufbereitete Fassung als Ersatz): der Auszug ist nicht zitierbar. Wird nicht gespeichert.
    public let raw: String?
    /// Die Quellart des Blocks lässt Quelltextauszüge zu (`AssistantSourceKind.isQuotable`: Buch, getippte Notiz, importiertes
    /// Dokument, Transkript). Erzeugter Text (Protokoll einer Sprachnotiz oder Besprechung, Antworten der App), Mails und
    /// Kalendereinträge sind nicht zitierbar. Fail-closed: Ohne ausdrückliche Angabe gilt der Auszug als nicht zitierbar. Wird
    /// nicht gespeichert.
    public let quotable: Bool

    public init(area: AssistantArea, itemId: String, title: String, locator: String? = nil, text: String, score: Double,
                isBlock: Bool = false, quotable: Bool = false, raw: String? = nil) {
        self.area = area
        self.itemId = itemId
        self.title = title
        self.locator = locator
        self.text = text
        self.score = score
        self.isBlock = isBlock
        self.quotable = quotable
        self.raw = raw
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

    /// Dieselbe Stelle mit anderem Text und Punktwert. Ein veränderter (gekürzter) Text ist nie mehr ein ganzer Quellblock und
    /// verliert den Rohtext (Vergleich auf der Bytefolge, nicht auf kanonischer Gleichheit).
    func replacing(text newText: String, score newScore: Double? = nil) -> AssistantPassage {
        let same = newText.hasSameBytes(as: text)
        return AssistantPassage(area: area, itemId: itemId, title: title, locator: locator, text: newText, score: newScore ?? score,
                                isBlock: isBlock && same, quotable: quotable, raw: same ? raw : nil)
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
        /// Der aufbereitete Text (Suche, Abruf, Prompt, Erkennung eigener Inhalte). Wird nie als Zitat ausgegeben.
        public let text: String
        /// Art der Quelle dieses Blocks. Ohne Vorgabewert: Jede Quelle legt sie ausdrücklich fest, eine vergessene Art kann nie
        /// stillschweigend zitierbar werden.
        public let kind: AssistantSourceKind
        /// Der rohe gespeicherte Block (siehe `AssistantPassage.raw`); `nil`, wo es keinen eindeutigen Rohtext gibt. Ohne ihn ist
        /// der Block nicht zitierbar, auch nicht in der aufbereiteten Fassung.
        public let raw: String?

        /// Der Block darf wörtlich ausgegeben werden: eine Quellart mit festgelegtem Quellblock UND ein eindeutiger Rohtext.
        public var quotable: Bool { kind.isQuotable && raw != nil }

        public init(locator: String?, text: String, kind: AssistantSourceKind, raw: String? = nil) {
            self.locator = locator
            self.text = text
            self.kind = kind
            self.raw = raw
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
    /// Hörbare Ansage („Ohne Quelle:“, „Wörtlicher Auszug aus ‚Titel‘:“, „Aus deinen Inhalten:“) oder, im Stil `natural`, der
    /// Überbrückungssatz („Ich schaue kurz nach.“). Gehört nicht zum Antworttext und nicht in den Verlauf.
    case announcement
    /// Stil `natural`: freier Satz des Sprechers ohne Inhalte des Nutzers (keine Auszüge im Prompt, keine Quelle).
    case conversational
    /// Stil `natural`: freier Satz des Denkers auf Grundlage von Auszügen. `sources` = genau die Auszüge im Prompt des Denkers;
    /// die Quellen sind Daten am Satz, nie Teil des gesprochenen Textes.
    case summarized

    /// Die Einordnung deckt Quellen: wörtlicher Auszug, im Stil `natural` die freie Zusammenfassung von Auszügen. Nur solche Sätze
    /// machen ihre Quellen zu Quellen der Antwort, des Beitrags und der bedeckten Bereiche der Schlusszeile.
    var coversSources: Bool { self == .grounded || self == .summarized }
}

/// Antwortstil des Gesprächsmodus (ADR-0028 E4.2c). `verbatim` ist der bisherige Pfad (nur wörtliche Quellblöcke, Bestandsangaben
/// und feste Sätze des Systems) und bleibt der Standard. `natural`: ein kurzer Sprecher antwortet direkt oder verlangt eine Suche;
/// dann ein Denker, der aus Auszügen frei in ein bis drei Sätzen antwortet.
public enum AssistantStyle: String, Sendable, Codable, Equatable {
    case verbatim, natural
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
/// Marke je Ausgabeeinheit (ein ganzer Quellblock oder ein Satz des Systems): In der Datenrunde ist jede angezeigte Einheit entweder ein
/// Zitat mit genau EINER Quellzeile (`quote`, `source`) oder trägt eine eigene Kennzeichnung (`note`: feste Antwort des Systems,
/// Bestandsangabe), weil sie strukturell keine Quelle hat.
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
    /// Stil `natural`: freier Satz des Sprechers (keine Inhalte des Nutzers) bzw. freie Zusammenfassung von Auszügen (Quellen als
    /// Daten am Beitrag, `AssistantTurn.sources`). Die Marke macht den Abschnitt anzeigbar (`isMarked`); wie die App sie zeigt, ist
    /// ihre Sache (`isFreeform`).
    public static let conversationNote = "Frei formuliert"
    public static let summaryNote = "Zusammengefasst aus deinen Inhalten"

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

    /// Der Abschnitt trägt seine Marke vollständig (Runde 8, R1/U2): ein Zitat nur mit der Quellzeile „Wörtlicher Auszug aus …“
    /// (identifizierbare Quelle), sonst die Kennzeichnung eines Satzes ohne Quelle (`note`) oder „Ohne Quelle“ (`unsourced`). Ein
    /// Abschnitt ohne Marke wird nie angezeigt, auch nicht als einfacher Text: Eine Ansicht zeigt nur, was `isMarked` ist.
    public var isMarked: Bool {
        if quote { return source.map { $0.hasPrefix(AssistantPassage.excerptPrefix) && $0.count > AssistantPassage.excerptPrefix.count } ?? false }
        return note != nil || unsourced
    }

    /// Der Abschnitt stammt aus dem Stil `natural` (frei formuliert oder frei zusammengefasst): Er ist kein Zitat und keine feste
    /// Antwort des Systems; die Ansicht braucht dafür keine auffällige Kennzeichnung.
    public var isFreeform: Bool { !quote && (note == Self.conversationNote || note == Self.summaryNote) }

    /// Die Einheiten der laufenden Antwort, die die Ansicht zeigen darf (Runde 8, U2): nur die mit vollständiger Marke, jede ganz.
    /// Eine Einheit ohne Marke erscheint nicht, auch nicht als einfacher Text.
    public static func displayed(_ list: [AssistantTurnPart]) -> [AssistantTurnPart] { list.filter(\.isMarked) }

    /// Kennzeichnung eines Satzes nach seiner Einordnung; `nil`, wenn er eine Quellzeile oder die Kennzeichnung „Ohne Quelle“
    /// (ältere Beiträge) trägt.
    public static func note(for status: AssistantSentenceStatus, quote: Bool) -> String? {
        guard !quote else { return nil }
        switch status {
        case .neutral: return templateNote
        case .inventory: return inventoryNote
        case .conversational: return conversationNote
        case .summarized: return summaryNote
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

/// Welche ausgegebenen Einheiten einer Anfrage in den Beitrag des Verlaufs kommen (Runde 8, R1): Die Steuerung übergibt Einheiten,
/// keinen Text. Eine Einheit kommt ganz in den Verlauf (Art, Text unverändert, Quelle) oder gar nicht; ein Quellblock wird nie
/// gekürzt, getrimmt oder neu zerlegt.
public enum AssistantSpokenUnits: Sendable, Equatable {
    /// Keine Einheit (nichts gesprochen, „Ergänzen“, Fehler vor der Ausgabe): kein Beitrag.
    case nothing
    /// Alle ausgegebenen Einheiten der Anfrage (der Lauf endete regulär).
    case all
    /// Nur diese Einheiten, ganz gesprochen: Nummern der Ereignisse der Anfrage (dieselben wie bei `noteCommit`, Ansagen eingeschlossen,
    /// sie zählen nicht). Eine unbekannte Nummer zählt nicht; die Reihenfolge ist die der Ausgabe, nicht die der Liste.
    case events([Int])
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
    /// Rundenart des Beitrags. Alte Beiträge ohne das Feld: `nil` (gelten für die Prompt-Auswahl als Datenrunde). Im Stil `natural`
    /// die letzte Rundenart der Anfrage (Sprecher: `.general`, mit Suche: `.data`).
    public var round: AssistantRoundKind?
    /// Stil, in dem der Beitrag entstand. `nil` = `verbatim` (alte Beiträge und der Standard bleiben unverändert gespeichert).
    public var style: AssistantStyle?

    public init(user: String, assistant: String, interrupted: Bool = false, sources: [AssistantPassage] = [],
                general: Bool = false, unsourced: Bool = false, parts: [AssistantTurnPart] = [], model: String = "", at: Date = Date(),
                round: AssistantRoundKind? = nil, style: AssistantStyle? = nil) {
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
        self.style = style
    }

    private enum CodingKeys: String, CodingKey {
        case user, assistant, interrupted, sources, general, unsourced, parts, model, at, round, style
    }

    /// Beitrag aus einer Runde ohne Quelle: Rundenart `.general`, ein Satz ohne Quelle oder ein Abschnitt ohne Quelle. Alte
    /// Beiträge ohne das Feld `unsourced` gelten über `general` ebenso (vorsichtig: im Zweifel ohne Quelle).
    /// Im Stil `natural` ist die Rundenart `.general` die des Sprechers (freies Gespräch), nicht die Funktion „Antworten ohne Quelle“:
    /// Dort zählt nur, was ausdrücklich als Satz ohne Quelle (`unsourced`) ausgegeben wurde.
    public var isUnsourced: Bool {
        if style == .natural { return unsourced || parts.contains { $0.unsourced } }
        return round == .general || unsourced || parts.contains { $0.unsourced }
    }

    /// Die Beiträge, die ein Gespräch ANZEIGEN und in einen Prompt geben darf. Ist die Funktion „Antworten ohne Quelle“ nicht
    /// freigeschaltet (`released == false`), fallen alle Beiträge aus Runden ohne Quelle heraus, bevor sie angezeigt werden oder
    /// in einen Prompt gelangen. Reine Funktion; die Datei bleibt, bis der nächste Verlauf gespeichert wird, dann fehlen sie.
    ///
    /// Ein Beitrag ohne gespeicherte Einheiten (`parts` leer) oder mit einem Abschnitt ohne vollständige Marke (`AssistantTurnPart.isMarked`:
    /// ein Zitat ohne die Quellzeile „Wörtlicher Auszug aus …“, ein Satz ohne Kennzeichnung) wird nie angezeigt und nie in einen Prompt
    /// gegeben (Runde 8, R1/U2): Aus dem bloßen Text einer Antwort lässt sich nicht mehr sagen, ob er ein Auszug war. Solche Beiträge
    /// (nur aus der Zeit vor den gespeicherten Einheiten) fallen beim Laden heraus; die Datei bleibt, bis der nächste Verlauf gespeichert wird.
    public static func admitted(_ turns: [AssistantTurn], released: Bool) -> [AssistantTurn] {
        turns.filter { $0.hasMarkedUnits && (released || !$0.isUnsourced) }
    }

    /// Die Einheiten des Beitrags sind gespeichert und jede trägt ihre vollständige Marke.
    public var hasMarkedUnits: Bool { !parts.isEmpty && parts.allSatisfy(\.isMarked) }

    /// Was die Ansicht von einem Beitrag zeigen darf (Runde 8, U2): alle Einheiten, wenn jede ihre vollständige Marke trägt, sonst
    /// nichts. Es gibt keinen Rückfall auf den bloßen Text (`assistant`) und keine Einheit ohne Marke als einfachen Text. Die
    /// Ansicht ruft nur diese Eigenschaft auf; der Kern prüft sie (die Ansicht selbst ist nur auf dem Mac prüfbar).
    public var displayedParts: [AssistantTurnPart] { hasMarkedUnits ? parts : [] }

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
        style = try c.decodeIfPresent(AssistantStyle.self, forKey: .style)
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
    /// Antwortstil (E4.2c). Standard `.verbatim`: Alle bisherigen Aufrufer laufen unverändert im bisherigen Pfad.
    public var style: AssistantStyle
    /// Stil `natural`: einen festen Überbrückungssatz sprechen, wenn die Suche beginnt. Standard AUS (Vorgabe: keine festen Floskeln im
    /// Gespräch; das Warten zeigt die Denk-Anzeige, `AssistantProgress`). Der Überbrückungssatz ist eine Ansage (`.announcement`).
    public var bridge: Bool
    /// Stil `natural`: höchstens so viele Auszüge und Zeichen für den Denker (zusätzlich zum Budget des Kontexts).
    public var naturalPassageLimit: Int
    public var naturalPassageChars: Int
    /// Stil `natural`: höchstens so viele freie Sätze je Phase (Sprecher oder Denker); danach wird das Modell abgebrochen.
    public var naturalMaxSentences: Int
    /// Stil `natural`: feste Ansagen für langsame Antworten. Standard AUS (wie `bridge`: keine festen Floskeln). Eingeschaltet gilt:
    /// gemessen ab dem Hinweis „Äußerung gilt“ (`AssistantSession.noteCommitted`) kommt die erste Ansage („Einen Augenblick.“) nach
    /// `fillerFirstMs`, wenn bis dahin noch keine Einheit ausgegeben wurde, die zweite („Das dauert noch etwas.“) nach `fillerSecondMs`,
    /// wenn bis dahin noch keine Antworteinheit ausgegeben wurde. Ohne den Hinweis gibt es keine dieser Ansagen. Mit `bridge` und
    /// `fillers` beide aus gibt der Kern in keinem Fall einen festen Überbrückungs- oder Ansagesatz aus.
    public var fillers: Bool
    public var fillerFirstMs: Int
    public var fillerSecondMs: Int

    public init(modelLabel: String, isCloud: Bool, contextTokens: Int, maxAnswerTokens: Int, userName: String?, timeout: TimeInterval,
                allowUnsourced: Bool = false, consentVersion: Int = 0, style: AssistantStyle = .verbatim, bridge: Bool = false,
                naturalPassageLimit: Int = 4, naturalPassageChars: Int = 2400, naturalMaxSentences: Int = 4,
                fillers: Bool = false, fillerFirstMs: Int = 1_200, fillerSecondMs: Int = 8_000) {
        self.fillers = fillers
        self.fillerFirstMs = fillerFirstMs
        self.fillerSecondMs = fillerSecondMs
        self.modelLabel = modelLabel
        self.isCloud = isCloud
        self.contextTokens = contextTokens
        self.maxAnswerTokens = maxAnswerTokens
        self.userName = userName
        self.timeout = timeout
        self.allowUnsourced = allowUnsourced
        self.consentVersion = consentVersion
        self.style = style
        self.bridge = bridge
        self.naturalPassageLimit = naturalPassageLimit
        self.naturalPassageChars = naturalPassageChars
        self.naturalMaxSentences = naturalMaxSentences
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
    /// Stil `natural`: freie Sätze des Sprechers und freie Zusammenfassungen des Denkers (im Stil `verbatim` immer 0).
    public var conversational = 0
    public var summarized = 0

    public init(grounded: Int = 0, inventory: Int = 0, unsourced: Int = 0, neutral: Int = 0, droppedOwnData: Int = 0,
                droppedNoMode: Int = 0, droppedNotSnapped: Int = 0, droppedDuplicate: Int = 0, droppedTemplate: Int = 0,
                revoked: Int = 0, droppedFragment: Int = 0, conversational: Int = 0, summarized: Int = 0) {
        self.conversational = conversational
        self.summarized = summarized
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
        case revoked, droppedFragment, conversational, summarized
    }

    /// Fehlende Zähler gelten als 0: Zeilen älterer und neuerer Fassungen bleiben lesbar.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        func count(_ key: CodingKeys) throws -> Int { try c.decodeIfPresent(Int.self, forKey: key) ?? 0 }
        self.init(grounded: try count(.grounded), inventory: try count(.inventory), unsourced: try count(.unsourced),
                  neutral: try count(.neutral), droppedOwnData: try count(.droppedOwnData), droppedNoMode: try count(.droppedNoMode),
                  droppedNotSnapped: try count(.droppedNotSnapped), droppedDuplicate: try count(.droppedDuplicate),
                  droppedTemplate: try count(.droppedTemplate), revoked: try count(.revoked),
                  droppedFragment: try count(.droppedFragment), conversational: try count(.conversational),
                  summarized: try count(.summarized))
    }

    /// Schreibt die neuen Zähler nur, wenn sie gesetzt sind: Zähler des Stils `verbatim` bleiben Byte für Byte, wie sie waren.
    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(grounded, forKey: .grounded)
        try c.encode(inventory, forKey: .inventory)
        try c.encode(unsourced, forKey: .unsourced)
        try c.encode(neutral, forKey: .neutral)
        try c.encode(droppedOwnData, forKey: .droppedOwnData)
        try c.encode(droppedNoMode, forKey: .droppedNoMode)
        try c.encode(droppedNotSnapped, forKey: .droppedNotSnapped)
        try c.encode(droppedDuplicate, forKey: .droppedDuplicate)
        try c.encode(droppedTemplate, forKey: .droppedTemplate)
        try c.encode(revoked, forKey: .revoked)
        try c.encode(droppedFragment, forKey: .droppedFragment)
        if conversational > 0 { try c.encode(conversational, forKey: .conversational) }
        if summarized > 0 { try c.encode(summarized, forKey: .summarized) }
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
    public var emitted: Int { grounded + inventory + unsourced + neutral + conversational + summarized }
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

/// Fortschritt eines Laufs im Stil `natural` für die Denk-Anzeige. NUR Anzeige: Er steht nicht im Protokoll und nicht im Verlauf (Titel
/// und Suchwörter sind Inhalte), und gesprochen und veröffentlicht wird weiter ausschließlich über `AssistantEvent.sentence`.
/// Reihenfolge je Anfrage: `speaker`, dann nur bei einer Suche `searching`, `found`, `composing` und `draft` (mehrfach). Eine Runde
/// ohne Suche hat nur `speaker`; bei direktem Inhaltsbezug (der Sprecher läuft nicht) beginnt die Folge mit `searching`, eine
/// Bestandsfrage hat keinen Fortschritt (kein Modell).
public enum AssistantProgress: Sendable, Equatable {
    /// Der Sprecher-Aufruf läuft.
    case speaker
    /// Die Daten-Phase beginnt, der Abruf läuft mit diesen Suchwörtern (ohne Suchwörter des Sprechers: der Fragetext).
    case searching(query: String)
    /// Titel der Auszüge im Prompt des Denkers, in der Reihenfolge der Auszüge und ohne Dubletten. Leer: nichts gefunden (dann kommt
    /// weder `composing` noch `draft`).
    case found(titles: [String])
    /// Der Denker-Aufruf läuft.
    case composing
    /// Der Rohtext des Denkers bisher (kumulativ), höchstens etwa alle 150 ms und am Ende einmal vollständig.
    case draft(String)
}

public enum AssistantEvent: Sendable, Equatable {
    /// Fortschritt für die Denk-Anzeige (nur Stil `natural`, nur Anzeige). `generation` wie bei `sentence`: Wer sie nicht mehr kennt
    /// (Widerruf), zeigt nichts. Nach `cancel` und nach einem Widerruf kommt keiner mehr.
    case progress(requestId: String, generation: Int, AssistantProgress)
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

extension String {
    /// Gleiche UTF-8-Bytefolge. `==` auf `String` gilt für kanonisch gleiche Zeichenfolgen (NFC = NFD) und ist für „zeichengleich
    /// mit dem Gespeicherten“ zu schwach.
    func hasSameBytes(as other: String) -> Bool { utf8.elementsEqual(other.utf8) }
}
