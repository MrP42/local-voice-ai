import Foundation

// Assistenten-Profil (lva#555, Paket G1): der Speicher der eigenen Assistenten, ein Verzeichnis je Gerät.
// Das Speicherformat gilt gemeinsam mit dem PC und steht in `apps/shared/assistenten/README.md` (Abschnitt „Speicherformat“):
// - je eigenem Assistenten eine Datei `<id>.json` mit exakt der Ausgabe von `encode` (UTF-8, ohne BOM);
// - der Dateiname muss zur `id` im Inhalt passen, sonst wird die Datei abgewiesen (`muster`, Pfad `id`);
// - die aktive Auswahl steht in `aktiv.json` als `{"id": "<id>"}`; fehlt sie, ist sie ungültig oder zeigt sie auf ein fehlendes
//   Profil, gilt die Vorlage `alles`;
// - Vorlagen liegen nie im Verzeichnis und lassen sich weder überschreiben noch löschen;
// - Obergrenzen: 200 Dateien beim Auflisten und bei der Neuanlage, 256 KiB je Datei (größere werden abgewiesen und nicht gelesen);
//   ein direkter Abruf per Id liest unabhängig von der 200er-Grenze;
// - gelesen werden nur reguläre Dateien (Typ über `attributesOfItem`, Symlinks werden nicht verfolgt); ein Symlink `x.json` oder
//   eine andere nicht reguläre Datei gilt als abgewiesen (`json`, Pfad ""), ein Verzeichnis `x.json` wird ignoriert;
// - andere Dateien im Verzeichnis werden ignoriert; geschrieben wird atomar (`Data.write(options: .atomic)`);
// - `aktiv` ist als Id vorbehalten (der Dateiname `aktiv.json` gehört der aktiven Auswahl);
// - ein Schreiber je Verzeichnis: die Kapazitätsprüfung und `newOwnId` sind nicht atomar gegen parallele Prozesse.
// Eine abgewiesene Datei wird nie verändert oder gelöscht: `save` und `delete` fassen eine vorhandene Datei nur an, wenn sie ein
// gültiges eigenes Profil mit genau dieser Id ist (sonst `occupied`, bei `delete` für Nicht-Reguläres `notFound`).

/// Eine Datei, die nicht als Assistent gelesen werden konnte, mit der Fehlerliste.
public struct AssistantRejectedFile: Equatable, Sendable {
    public let fileName: String
    public let issues: [AssistantProfileIssue]

    public static func == (a: AssistantRejectedFile, b: AssistantRejectedFile) -> Bool {
        AssistantExact.equal(a.fileName, b.fileName) && a.issues == b.issues
    }
}

public struct AssistantProfileListing: Equatable, Sendable {
    /// Gültige eigene Profile, nach Dateiname sortiert.
    public let profiles: [AssistantProfile]
    public let rejected: [AssistantRejectedFile]
}

public enum AssistantProfileStoreError: Error, Equatable, Sendable {
    /// Das Profil ist ungültig (Fehlerliste der Prüfung).
    case invalid([AssistantProfileIssue])
    /// Vorlagen (und vorbehaltene Ids, auch `aktiv`) lassen sich weder speichern, löschen noch auswählen.
    case reservedId
    case notFound
    case tooLarge
    /// Schon 200 Dateien im Verzeichnis.
    case tooMany
    case writeFailed
    /// Unter dieser Id liegt eine Datei, die kein gültiges eigenes Profil mit genau dieser Id ist (beschädigt, neueres Schema,
    /// falsche Id im Inhalt, zu groß, keine reguläre Datei): belegt. Sie wird weder überschrieben noch gelöscht.
    case occupied
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

    private enum EntryKind { case absent, regular, directory, other }

    /// Art des Eintrags, ohne Symlinks zu verfolgen (`attributesOfItem` meldet bei einem Symlink den Link selbst).
    private func kind(of url: URL) -> EntryKind {
        do {
            let attributes = try FileManager.default.attributesOfItem(atPath: url.path)
            switch attributes[.type] as? FileAttributeType {
            case .typeRegular?: return .regular
            case .typeDirectory?: return .directory
            default: return .other
            }
        } catch {
            return AssistantProfileStore.isMissing(error) ? .absent : .other
        }
    }

    private static func isMissing(_ error: Error) -> Bool {
        let nsError = error as NSError
        if nsError.domain == NSCocoaErrorDomain, nsError.code == NSFileNoSuchFileError || nsError.code == NSFileReadNoSuchFileError {
            return true
        }
        if let posix = nsError.userInfo[NSUnderlyingErrorKey] as? NSError, posix.domain == NSPOSIXErrorDomain {
            return posix.code == Int(ENOENT) || posix.code == Int(ENOTDIR)
        }
        return nsError.domain == NSPOSIXErrorDomain && (nsError.code == Int(ENOENT) || nsError.code == Int(ENOTDIR))
    }

    /// Dateinamen der Profile (ohne `aktiv.json`, ohne Verzeichnisse), nach Bytes sortiert. Symlinks und andere nicht reguläre
    /// Einträge zählen mit (sie erscheinen als abgewiesen), Verzeichnisse nicht.
    private func profileFileNames() -> [String] {
        let names = (try? FileManager.default.contentsOfDirectory(atPath: directory.path)) ?? []
        return names
            .filter { name in
                guard name.utf8.count > 5, name.utf8.reversed().starts(with: ".json".utf8.reversed()) else { return false }
                if name.utf8.elementsEqual(AssistantProfileStore.activeFileName.utf8) { return false }
                switch kind(of: directory.appendingPathComponent(name)) {
                case .regular, .other: return true
                case .absent, .directory: return false
                }
            }
            .sorted { $0.utf8.lexicographicallyPrecedes($1.utf8) }
    }

    /// Liest eine reguläre Datei mit Größenprüfung vor dem Lesen. Alles andere (Symlink, Gerät, FIFO) ist `json`; es wird nicht geöffnet.
    private func read(_ url: URL) -> Result<Data, AssistantProfileFailure> {
        let fail = { (code: AssistantProfileIssue.Code) in Result<Data, AssistantProfileFailure>.failure(AssistantProfileFailure([AssistantProfileIssue(path: "", code: code)])) }
        guard kind(of: url) == .regular,
              let attributes = try? FileManager.default.attributesOfItem(atPath: url.path),
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
        guard !AssistantProfileTemplates.isReservedId(id), AssistantProfilePatterns.id(id), case .success(let profile) = load(fileName: id + ".json") else { return nil }
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
        let target = fileURL(checked.id)
        switch kind(of: target) {
        case .absent:
            if profileFileNames().count >= AssistantProfileStore.maxFiles { throw AssistantProfileStoreError.tooMany }
        case .regular:
            guard case .success = load(fileName: checked.id + ".json") else { throw AssistantProfileStoreError.occupied }
        case .directory, .other:
            throw AssistantProfileStoreError.occupied
        }
        try write(data, to: target)
        return checked
    }

    /// Löscht ein eigenes, gültiges Profil. Eine abgewiesene Datei (beschädigt, neueres Schema, falsche Id im Inhalt, zu groß) wird
    /// nicht gelöscht (`occupied`); ein Verzeichnis, Symlink oder anderes Nicht-Reguläres ist `notFound`, ebenso eine fehlende Datei.
    /// Die aktive Auswahl bleibt, wie sie ist (der Rückfall greift beim Lesen).
    public func delete(id: String) throws {
        if AssistantProfileTemplates.isReservedId(id) { throw AssistantProfileStoreError.reservedId }
        guard AssistantProfilePatterns.id(id), kind(of: fileURL(id)) == .regular else { throw AssistantProfileStoreError.notFound }
        guard case .success = load(fileName: id + ".json") else { throw AssistantProfileStoreError.occupied }
        do { try FileManager.default.removeItem(at: fileURL(id)) } catch { throw AssistantProfileStoreError.writeFailed }
    }

    /// Eine neue freie eigene Id: `eigen-` und 12 Zeichen aus dem Erzeuger.
    public func newOwnId() throws -> String {
        for _ in 0..<32 {
            let tail = (0..<12).map { _ in AssistantProfileStore.idAlphabet[Int(idSource()) % AssistantProfileStore.idAlphabet.count] }
            let id = "eigen-" + String(decoding: tail, as: UTF8.self)
            if kind(of: fileURL(id)) == .absent { return id }
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

    /// Wählt ein vorhandenes Profil (Vorlage oder gültige eigene Datei) als aktiv. Vorbehaltene Ids, die keine Vorlage sind
    /// (`aktiv`, `vorlage-…`), lassen sich nicht wählen (`reservedId`); geschrieben wird nur `aktiv.json`, nie ein Profil.
    public func setActive(_ id: String) throws {
        if AssistantProfileTemplates.isReservedId(id), !AssistantProfileTemplates.isTemplate(id) { throw AssistantProfileStoreError.reservedId }
        guard profile(id: id) != nil else { throw AssistantProfileStoreError.notFound }
        let text = AssistantJSON.object([AssistantJSONMember("id", .string(id))]).encoded()
        try write(Data(text.utf8), to: activeURL)
    }
}
