import Foundation

// Verknüpfte Kontexte (lva#557, Epic #554): Ein Kontext ist ein VERWEIS (Art, Kennung, Titel) auf ein Element des vorhandenen Index,
// nie eine Kopie des Inhalts. Dieser Wert sagt dem Wissen (`AssistantKnowledge.scope`), welche Elemente im Abruf zuerst kommen
// (`first`) oder allein zählen (`only`). Er ist unveränderlich wie `AssistantKnowledge.ceiling`, wird an das Wissen gehängt
// (`withScope`) und ändert weder Freigaben noch Lesegrenzen: Er filtert nur, was Abruf und Bestand ohnehin liefern würden. Ein
// Kontext öffnet damit nie einen nicht freigegebenen Bereich und nie etwas, das nicht im Index steht.
// Die Abbildung Profil-Kontexte → Scope und die Verfügbarkeit je Kontext sind reine Funktionen.

public struct AssistantScope: Equatable, Sendable {
    /// `first`: Treffer aus den Kontexten stehen vor allen anderen (die beste Stelle außerhalb bleibt erhalten).
    /// `only`: Abruf, Bestand und Auszüge ausschließlich aus den Kontexten.
    public enum Mode: Equatable, Sendable { case first, only }

    /// Ein Element des Index: Bereich und Kennung (`AssistantItem.id`).
    public struct Entry: Hashable, Sendable {
        public let area: AssistantArea
        public let itemId: String

        public init(area: AssistantArea, itemId: String) {
            self.area = area
            self.itemId = itemId
        }
    }

    public var mode: Mode
    /// Die einzelnen Elemente (Buch, Besprechung, Sprachnotiz, Datei).
    public var items: Set<Entry>
    /// Kennungen von Ablage-Ordnern: gemeint sind die Dateien in diesem Ordner und darunter (`AssistantItem.folders`).
    public var folders: Set<String>

    public init(mode: Mode, items: Set<Entry> = [], folders: Set<String> = []) {
        self.mode = mode
        self.items = items
        self.folders = folders
    }

    /// Nichts verknüpft, das auf diesem Gerät etwas bedeuten kann (alles unbekannte Arten). Bei `only` ist das Ergebnis dann leer.
    public var isEmpty: Bool { items.isEmpty && folders.isEmpty }

    /// Das Element gehört zu den Kontexten (einzeln oder über einen Ordner der Ablage).
    public func admits(_ item: AssistantItem) -> Bool {
        if items.contains(Entry(area: item.area, itemId: item.id)) { return true }
        guard item.area == .files, !folders.isEmpty else { return false }
        return item.folders.contains { folders.contains($0) }
    }

    // MARK: Abbildung

    /// Bereich einer Kontextart am iPhone; `nil`: Art ohne Abbildung (Projekt, Abschnitt; sie gibt es nur am PC).
    /// Der Ordner gehört zur Ablage.
    public static func area(of kind: AssistantProfile.Context.Kind) -> AssistantArea? {
        switch kind {
        case .book: .books
        case .meeting: .transcripts
        case .voiceNote: .notes
        case .file, .folder: .files
        case .project, .section: nil
        }
    }

    /// Kontextart eines Elements dieses Bereichs (Gegenstück zu `area(of:)`; die Ablage liefert Dateien, Ordner sind eigene Verweise).
    public static func kind(of area: AssistantArea) -> AssistantProfile.Context.Kind {
        switch area {
        case .books: .book
        case .transcripts: .meeting
        case .notes: .voiceNote
        case .files: .file
        }
    }

    /// Profil-Kontexte → Scope (reine Funktion). `alles` und eine leere Liste ergeben `nil` (kein Filter; wie
    /// `AssistantProfile.effectiveContextMode`). Arten ohne Abbildung tragen nichts bei, bleiben aber im Profil; sind alle ohne
    /// Abbildung, ist der Scope bei `nur` leer (nie ein stiller Rückfall auf alles).
    public static func make(mode: AssistantProfile.Knowledge.Mode, contexts: [AssistantProfile.Context]) -> AssistantScope? {
        switch mode {
        case .all: return nil
        case .first: return contexts.isEmpty ? nil : build(.first, contexts)
        case .only: return contexts.isEmpty ? nil : build(.only, contexts)
        }
    }

    private static func build(_ mode: Mode, _ contexts: [AssistantProfile.Context]) -> AssistantScope {
        var scope = AssistantScope(mode: mode)
        for context in contexts {
            guard let area = area(of: context.kind) else { continue }
            if context.kind == .folder { scope.folders.insert(context.id) } else { scope.items.insert(Entry(area: area, itemId: context.id)) }
        }
        return scope
    }

    /// Der wirksame Scope eines Gesprächs: die Kontexte des Assistenten (`profile`; `nil` bei „Alles“) und die des Gesprächs zusammen.
    /// Modus: der des Assistenten (`effectiveContextMode`); ist er `alles`, wirken die Kontexte des Gesprächs als `zuerst`. Der Schalter
    /// „Nur aus diesen Inhalten antworten“ setzt für das Gespräch `nur`. Ohne irgendeinen Kontext: `nil`.
    public static func effective(profile: AssistantProfile?, conversation: AssistantConversationContexts) -> AssistantScope? {
        let base = profile?.effectiveContextMode ?? .all
        // Im Modus `alles` haben die Kontexte des Assistenten keine Wirkung; nur die des Gesprächs zählen (als `zuerst`).
        let own = base == .all ? [] : (profile?.knowledge.contexts ?? [])
        let all = own + conversation.contexts.compactMap { $0.context }
        guard !all.isEmpty else { return nil }
        let mode: AssistantProfile.Knowledge.Mode
        if conversation.onlyFromThese { mode = .only } else if base == .all { mode = .first } else { mode = base }
        return make(mode: mode, contexts: all)
    }

    // MARK: Ordner der Ablage

    /// Kennungen von `folder` bis zur Wurzel (der Ordner selbst zuerst), zyklenfest; leer bei leerer oder unbekannter Kennung.
    /// `parents`: Elternordner je Ordner (`nil`: Wurzel oder unbekannter Elternordner); alle bekannten Ordner stehen als Schlüssel darin.
    public static func folderChain(_ folder: String, parents: [String: String?]) -> [String] {
        guard !folder.isEmpty else { return [] }
        var chain: [String] = []
        var seen = Set<String>()
        var current: String? = folder
        while let id = current, let parent = parents[id], seen.insert(id).inserted {
            chain.append(id)
            current = parent
        }
        return chain
    }
}

// MARK: - Verfügbarkeit

/// Wie ein Kontext gerade dasteht.
public enum AssistantContextAvailability: Equatable, Sendable {
    /// Der Inhalt steht im Index der wirksamen Bereiche.
    case available
    /// „nicht mehr verfügbar“: der Inhalt steht nicht (mehr) im Index der wirksamen Bereiche (gelöscht, nicht freigegeben, hinter
    /// der Lesegrenze ohne Text, Bereich nicht gewollt). Der Verweis bleibt, das Gespräch läuft ohne ihn weiter.
    case gone
    /// „auf diesem Gerät nicht verfügbar“: Art ohne Abbildung (Projekt, Abschnitt). Bleibt im Profil erhalten.
    case unsupported
}

extension AssistantScope {
    /// Verfügbarkeit eines Kontexts gegen die Elemente des Index (`AssistantKnowledge.indexedItems`, höchstens die wirksamen Bereiche).
    public static func availability(of context: AssistantProfile.Context, in items: [AssistantItem]) -> AssistantContextAvailability {
        guard let area = area(of: context.kind) else { return .unsupported }
        if context.kind == .folder {
            return items.contains { $0.area == area && $0.folders.contains(context.id) } ? .available : .gone
        }
        return items.contains { $0.area == area && $0.id == context.id } ? .available : .gone
    }

    public static func availability(of contexts: [AssistantProfile.Context], in items: [AssistantItem]) -> [AssistantContextAvailability] {
        contexts.map { availability(of: $0, in: items) }
    }
}
