import Foundation
import SwiftUI

// Assistenten im Gespräch „Sprechen“ (lva#556, Paket G2): Verwaltung der Assistenten-Profile auf dem iPhone.
// Der Kern (`AssistantProfileStore`, G1) kennt Format, Prüfung und Speicher; diese Klasse macht daraus den Zustand für die Ansicht:
// Vorlagen und eigene Assistenten, die aktive Auswahl und die abgewiesenen Dateien (nur als Zahl in der Ansicht).
// Speicherort: `Application Support/Assistenten` (je Assistent eine Datei `<id>.json`, dazu `aktiv.json`). Dieselbe Schutzklasse wie die
// übrigen Dateien des Gesprächs (`completeUntilFirstUserAuthentication`, am Verzeichnis gesetzt: neue Dateien erben sie); die Dateien sind
// NICHT vom Backup ausgenommen (die Assistenten gehören dem Nutzer und sollen mit dem Gerät wiederkommen). Inhalte der Anweisungen
// stehen nie im Protokoll.

@MainActor
final class AssistantProfiles: ObservableObject {
    static let shared = AssistantProfiles()

    /// Eigene Assistenten, nach Name sortiert (ohne Beachtung der Groß-/Kleinschreibung).
    @Published private(set) var own: [AssistantProfile] = []
    /// Abgewiesene Dateien (ungültig, zu groß, zu viele). Die Ansicht zeigt nur die Zahl; die Dateien bleiben unverändert liegen.
    @Published private(set) var rejected: [AssistantRejectedFile] = []
    @Published private(set) var activeId = AssistantProfileTemplates.everything.id

    /// `nil`: nur im Speicher (DEBUG-Beispielzustände): nichts wird gelesen oder geschrieben.
    private let store: AssistantProfileStore?

    var templates: [AssistantProfile] { AssistantProfileTemplates.all }
    /// Vorlagen, dann eigene.
    var all: [AssistantProfile] { templates + own }
    var rejectedCount: Int { rejected.count }

    /// Der aktive Assistent; fehlt er (gelöscht, nie gewählt), gilt „Alles“.
    var active: AssistantProfile {
        all.first { $0.id == activeId } ?? AssistantProfileTemplates.everything
    }

    /// „Alles“ ist aktiv: Das Gespräch läuft wie bisher, nichts vom Profil wirkt (keine Rolle, keine Begrüßung, keine Vorgaben).
    var isEverything: Bool { active.id == AssistantProfileTemplates.everything.id }

    static func isTemplate(_ profile: AssistantProfile) -> Bool { AssistantProfileTemplates.isTemplate(profile.id) }

    nonisolated static func standardDirectory() -> URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? FileManager.default.temporaryDirectory
        return base.appendingPathComponent("Assistenten", isDirectory: true)
    }

    init(directory: URL = AssistantProfiles.standardDirectory()) {
        #if DEBUG
        if AssistantProfiles.demo { store = nil; loadDemo(); return }
        #endif
        store = AssistantProfileStore(directory: directory)
        Self.prepareDirectory(directory)
        reload()
    }

    // MARK: Lesen

    /// Liest das Verzeichnis neu (Start, nach jedem Schreiben). Eine fehlende oder unlesbare Datei ändert nichts am Rest.
    func reload() {
        guard let store else { return }
        let listing = store.list()
        own = listing.profiles.sorted { Self.order($0, $1) }
        rejected = listing.rejected
        activeId = store.activeId()
    }

    private static func order(_ a: AssistantProfile, _ b: AssistantProfile) -> Bool {
        let order = a.name.localizedCaseInsensitiveCompare(b.name)
        return order == .orderedSame ? a.id < b.id : order == .orderedAscending
    }

    func profile(id: String) -> AssistantProfile? { all.first { $0.id == id } }

    // MARK: Aktionen

    /// Wählt den Assistenten. `false`, wenn es ihn nicht gibt oder die Auswahl nicht gesichert werden konnte (dann bleibt die bisherige).
    @discardableResult
    func select(_ id: String) -> Bool {
        guard profile(id: id) != nil else { return false }
        guard let store else { activeId = id; return true }
        do { try store.setActive(id) } catch { return false }
        activeId = id
        return true
    }

    /// Ein neuer, noch nicht gesicherter Assistent: eigene Kennung, Standardwerte. `nil`, wenn keine Kennung vergeben werden kann.
    func blank() -> AssistantProfile? {
        guard let id = newId() else { return nil }
        return AssistantProfile(id: id, name: "")
    }

    /// Eine Kopie als eigener Assistent (aus einer Vorlage oder einem eigenen Assistenten), sofort gesichert. Der Name bekommt „ (Kopie)“,
    /// gekürzt auf die erlaubten 60 Zeichen. Die Auswahl ändert sich nicht.
    func copy(_ source: AssistantProfile) -> Result<AssistantProfile, AssistantProfileStoreError> {
        guard let id = newId() else { return .failure(.writeFailed) }
        let suffix = " (Kopie)"
        let base = String(source.name.unicodeScalars.prefix(60 - suffix.unicodeScalars.count).map { Character($0) })
        let name = base + suffix
        let made: AssistantProfile
        if Self.isTemplate(source) {
            switch AssistantProfileTemplates.copyAsOwn(source, newId: id, newName: name) {
            case .success(let profile): made = profile
            case .failure(let failure): return .failure(.invalid(failure.issues))
            }
        } else {
            var duplicate = source
            duplicate.id = id
            duplicate.name = name
            made = duplicate
        }
        return save(made)
    }

    /// Sichert den Assistenten (neu oder geändert). Das Ergebnis ist das geprüfte Profil, wie es auf dem Gerät liegt.
    @discardableResult
    func save(_ profile: AssistantProfile) -> Result<AssistantProfile, AssistantProfileStoreError> {
        guard let store else {
            if let index = own.firstIndex(where: { $0.id == profile.id }) { own[index] = profile } else { own.append(profile) }
            own.sort { Self.order($0, $1) }
            return .success(profile)
        }
        do {
            let saved = try store.save(profile)
            reload()
            return .success(saved)
        } catch let error as AssistantProfileStoreError {
            return .failure(error)
        } catch {
            return .failure(.writeFailed)
        }
    }

    /// Löscht einen eigenen Assistenten. War er aktiv, gilt danach „Alles“ (die Auswahl wird mit gesichert). `false`, wenn das Löschen
    /// scheiterte; Vorlagen lassen sich nie löschen.
    @discardableResult
    func delete(_ id: String) -> Bool {
        guard !AssistantProfileTemplates.isReservedId(id) else { return false }
        let wasActive = activeId == id
        if let store {
            do { try store.delete(id: id) } catch { return false }
            if wasActive { try? store.setActive(AssistantProfileTemplates.everything.id) }
            reload()
            return true
        }
        own.removeAll { $0.id == id }
        if wasActive { activeId = AssistantProfileTemplates.everything.id }
        return true
    }

    private func newId() -> String? {
        if let store { return try? store.newOwnId() }
        return "eigen-" + String(UUID().uuidString.lowercased().replacingOccurrences(of: "-", with: "").prefix(12))
    }

    // MARK: Verzeichnis

    /// Legt das Verzeichnis mit der Schutzklasse an (neue Dateien erben sie). Ein Fehler bleibt folgenlos: der Speicher meldet das Schreiben.
    nonisolated static func prepareDirectory(_ directory: URL) {
        let fm = FileManager.default
        let attributes: [FileAttributeKey: Any] = [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication]
        try? fm.createDirectory(at: directory, withIntermediateDirectories: true, attributes: attributes)
        try? fm.setAttributes(attributes, ofItemAtPath: directory.path)
    }

    #if DEBUG
    // MARK: Beispielzustand (nur DEBUG)

    /// `--assistant-demo-profiles`, `--assistant-demo-profile-editor`, `--assistant-demo-greeting`: zwei eigene, erfundene Assistenten
    /// neben den Vorlagen, einer davon aktiv. Nichts wird gelesen oder geschrieben (`store == nil`).
    static let demo: Bool = {
        let arguments = ProcessInfo.processInfo.arguments
        return arguments.contains("--assistant-demo-profiles") || arguments.contains("--assistant-demo-profile-editor")
            || arguments.contains("--assistant-demo-greeting")
    }()

    /// Erster eigener Assistent der Beispielzustände (aktiv): mit Anweisung, Begrüßung mit Platzhaltern, Wissen und Verhalten.
    static var demoFront: AssistantProfile {
        var profile = AssistantProfile(id: "eigen-beispielempfang", name: "Empfang Beispielbüro")
        profile.descriptionText = "Nimmt Anrufe für das Beispielbüro entgegen."
        profile.instructions = "Du bist die Empfangsstelle des Beispielbüros. Antworte kurz, freundlich und sachlich. "
            + "Öffnungszeiten: Montag bis Freitag, 9 bis 16 Uhr."
        profile.greeting = "Guten {{tageszeit}}, hier ist der Empfang des Beispielbüros. Wie kann ich helfen?"
        profile.knowledge.allAreas = false
        profile.knowledge.areas = ["besprechungen", "ablage"]
        profile.behavior.style = .natural
        profile.behavior.pause = .long
        profile.behavior.interruptible = true
        return profile
    }

    static var demoNotes: AssistantProfile {
        var profile = AssistantProfile(id: "eigen-beispielnotizen", name: "Notizen vorlesen")
        profile.descriptionText = "Liest wörtliche Auszüge aus Büchern und Sprachnotizen."
        profile.instructions = "Antworte nur mit dem, was in den Auszügen steht."
        profile.knowledge.allAreas = false
        profile.knowledge.areas = ["buecher", "sprachnotizen"]
        profile.behavior.style = .literal
        profile.behavior.pause = .short
        profile.behavior.interruptible = false
        return profile
    }

    private func loadDemo() {
        own = [Self.demoFront, Self.demoNotes].sorted { Self.order($0, $1) }
        rejected = []
        activeId = Self.demoFront.id
    }
    #endif
}
