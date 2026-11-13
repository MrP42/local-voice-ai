import Foundation

// Assistenten-Profil (lva#555, Paket G1): der Speicher der eigenen Assistenten, ein Verzeichnis je Gerät.
// Das Speicherformat gilt gemeinsam mit dem PC und steht in `apps/shared/assistenten/README.md` (Abschnitt „Speicherformat“):
// - je eigenem Assistenten eine Datei `<id>.json` mit exakt der Ausgabe von `encode` (UTF-8, ohne BOM);
// - der Dateiname muss zur `id` im Inhalt passen, sonst wird die Datei abgewiesen (`muster`, Pfad `id`);
// - die aktive Auswahl steht in `aktiv.json` als `{"id": "<id>"}`; fehlt sie, ist sie ungültig oder zeigt sie auf ein fehlendes
//   Profil, gilt die Vorlage `alles`;
// - Vorlagen liegen nie im Verzeichnis und lassen sich weder überschreiben noch löschen;
// - Obergrenzen: 200 Dateien, 256 KiB je Datei (größere werden abgewiesen und nicht gelesen);
// - andere Dateien im Verzeichnis werden ignoriert; geschrieben wird atomar (`Data.write(options: .atomic)`).
// Eine abgewiesene Datei wird nie verändert oder gelöscht.

/// Eine Datei, die nicht als Assistent gelesen werden konnte, mit der Fehlerliste.
public struct AssistantRejectedFile: Equatable, Sendable {
    public let fileName: String
    public let issues: [AssistantProfileIssue]
}

public struct AssistantProfileListing: Equatable, Sendable {
    /// Gültige eigene Profile, nach Dateiname sortiert.
    public let profiles: [AssistantProfile]
    public let rejected: [AssistantRejectedFile]
}

public enum AssistantProfileStoreError: Error, Equatable, Sendable {
    /// Das Profil ist ungültig (Fehlerliste der Prüfung).
    case invalid([AssistantProfileIssue])
    /// Vorlagen (und vorbehaltene Ids) lassen sich weder speichern noch löschen.
    case reservedId
    case notFound
    case tooLarge
    /// Schon 200 Dateien im Verzeichnis.
    case tooMany
    case writeFailed
}

/// Liefert Zufallsbytes für neue Ids (im Test ein fester Erzeuger).
public typealias AssistantIdSource = @Sendable () -> UInt8

public struct AssistantProfileStore: Sendable {
    public static let maxFiles = 200
    public static let maxFileBytes = 256 * 1024
    public static let activeFileName = "aktiv.json"
    static let idAlphabet = Array("0123456789abcdefghijklmnopqrstuvwxyz".utf8)

    public let directory: URL
    private let idSource: AssistantIdSource

    public init(directory: URL, idSource: @escaping AssistantIdSource = { UInt8.random(in: 0...255) }) {
        self.directory = directory
        self.idSource = idSource
    }

    // MARK: Lesen

    private func fileURL(_ id: String) -> URL { directory.appendingPathComponent(id + ".json") }

    /// Dateinamen der Profile (ohne `aktiv.json`, ohne Verzeichnisse), nach Bytes sortiert.
    private func profileFileNames() -> [String] {
        let fm = FileManager.default
        let names = (try? fm.contentsOfDirectory(atPath: directory.path)) ?? []
        return names
            .filter { name in
                guard name.utf8.count > 5, name.utf8.reversed().starts(with: ".json".utf8.reversed()) else { return false }
                if name.utf8.elementsEqual(AssistantProfileStore.activeFileName.utf8) { return false }
                var isDirectory: ObjCBool = false
                return fm.fileExists(atPath: directory.appendingPathComponent(name).path, isDirectory: &isDirectory) && !isDirectory.boolValue
            }
            .sorted { $0.utf8.lexicographicallyPrecedes($1.utf8) }
    }

    /// Liest eine Datei mit Größenprüfung vor dem Lesen.
    private func read(_ url: URL) -> Result<Data, AssistantProfileFailure> {
        let fail = { (code: AssistantProfileIssue.Code) in Result<Data, AssistantProfileFailure>.failure(AssistantProfileFailure([AssistantProfileIssue(path: "", code: code)])) }
        guard let attributes = try? FileManager.default.attributesOfItem(atPath: url.path),
              let size = (attributes[.size] as? NSNumber)?.intValue else { return fail(.json) }
        if size > AssistantProfileStore.maxFileBytes { return fail(.length) }
        guard let data = try? Data(contentsOf: url) else { return fail(.json) }
        if data.count > AssistantProfileStore.maxFileBytes { return fail(.length) }
        return .success(data)
    }

    private func load(fileName: String) -> Result<AssistantProfile, AssistantProfileFailure> {
        switch read(directory.appendingPathComponent(fileName)) {
        case .failure(let failure):
            return .failure(failure)
        case .success(let data):
            switch AssistantProfile.parse(data: data) {
            case .failure(let failure):
                return .failure(failure)
            case .success(let profile):
                if !(profile.id + ".json").utf8.elementsEqual(fileName.utf8) {
                    return .failure(AssistantProfileFailure([AssistantProfileIssue(path: "id", code: .pattern)]))
                }
                if AssistantProfileTemplates.isReservedId(profile.id) {
                    return .failure(AssistantProfileFailure([AssistantProfileIssue(path: "id", code: .value)]))
                }
                return .success(profile)
            }
        }
    }

    /// Alle gültigen eigenen Profile und die abgewiesenen Dateien. Über 200 Dateien hinaus wird nichts gelesen.
    public func list() -> AssistantProfileListing {
        var profiles: [AssistantProfile] = []
        var rejected: [AssistantRejectedFile] = []
        for (index, name) in profileFileNames().enumerated() {
            if index >= AssistantProfileStore.maxFiles {
                rejected.append(AssistantRejectedFile(fileName: name, issues: [AssistantProfileIssue(path: "", code: .tooMany)]))
                continue
            }
            switch load(fileName: name) {
            case .success(let profile): profiles.append(profile)
            case .failure(let failure): rejected.append(AssistantRejectedFile(fileName: name, issues: failure.issues))
            }
        }
        return AssistantProfileListing(profiles: profiles, rejected: rejected)
    }

    /// Die Vorlagen, dann die eigenen Profile.
    public func allProfiles() -> [AssistantProfile] { AssistantProfileTemplates.all + list().profiles }

    /// Ein Profil nach Id: Vorlage oder gültige eigene Datei.
    public func profile(id: String) -> AssistantProfile? {
        if let template = AssistantProfileTemplates.template(id: id) { return template }
        guard AssistantProfilePatterns.id(id), case .success(let profile) = load(fileName: id + ".json") else { return nil }
        return profile
    }

    // MARK: Schreiben

    /// Prüft und speichert ein Profil atomar (Name getrimmt) und liefert die gespeicherte Fassung.
    @discardableResult
    public func save(_ profile: AssistantProfile) throws -> AssistantProfile {
        let checked: AssistantProfile
        switch AssistantProfile.parse(tree: profile.tree()) {
        case .failure(let failure): throw AssistantProfileStoreError.invalid(failure.issues)
        case .success(let value): checked = value
        }
        if AssistantProfileTemplates.isReservedId(checked.id) { throw AssistantProfileStoreError.reservedId }
        let data = Data(checked.encode().utf8)
        if data.count > AssistantProfileStore.maxFileBytes { throw AssistantProfileStoreError.tooLarge }
        let fm = FileManager.default
        let target = fileURL(checked.id)
        if !fm.fileExists(atPath: target.path), profileFileNames().count >= AssistantProfileStore.maxFiles {
            throw AssistantProfileStoreError.tooMany
        }
        try write(data, to: target)
        return checked
    }

    /// Löscht ein eigenes Profil. Die aktive Auswahl bleibt, wie sie ist (der Rückfall greift beim Lesen).
    public func delete(id: String) throws {
        if AssistantProfileTemplates.isReservedId(id) { throw AssistantProfileStoreError.reservedId }
        guard AssistantProfilePatterns.id(id), FileManager.default.fileExists(atPath: fileURL(id).path) else {
            throw AssistantProfileStoreError.notFound
        }
        do { try FileManager.default.removeItem(at: fileURL(id)) } catch { throw AssistantProfileStoreError.writeFailed }
    }

    /// Eine neue freie eigene Id: `eigen-` und 12 Zeichen aus dem Erzeuger.
    public func newOwnId() throws -> String {
        for _ in 0..<32 {
            let tail = (0..<12).map { _ in AssistantProfileStore.idAlphabet[Int(idSource()) % AssistantProfileStore.idAlphabet.count] }
            let id = "eigen-" + String(decoding: tail, as: UTF8.self)
            if !FileManager.default.fileExists(atPath: fileURL(id).path) { return id }
        }
        throw AssistantProfileStoreError.writeFailed
    }

    private func write(_ data: Data, to url: URL) throws {
        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            try data.write(to: url, options: .atomic)
        } catch {
            throw AssistantProfileStoreError.writeFailed
        }
    }

    // MARK: Aktive Auswahl

    private var activeURL: URL { directory.appendingPathComponent(AssistantProfileStore.activeFileName) }

    /// Die Id der aktiven Auswahl; `alles`, wenn die Datei fehlt, ungültig ist oder auf ein fehlendes Profil zeigt.
    public func activeId() -> String {
        let fallback = AssistantProfileTemplates.everything.id
        guard case .success(let data) = read(activeURL),
              let text = String(data: data, encoding: .utf8),
              case .string(let id)? = AssistantJSON.parse(text)?.member("id"),
              profile(id: id) != nil else { return fallback }
        return id
    }

    /// Das aktive Profil (nie `nil`: im Zweifel `alles`).
    public func active() -> AssistantProfile {
        profile(id: activeId()) ?? AssistantProfileTemplates.everything
    }

    /// Wählt ein vorhandenes Profil (Vorlage oder gültige eigene Datei) als aktiv.
    public func setActive(_ id: String) throws {
        guard profile(id: id) != nil else { throw AssistantProfileStoreError.notFound }
        let text = AssistantJSON.object([AssistantJSONMember("id", .string(id))]).encoded()
        try write(Data(text.utf8), to: activeURL)
    }
}
