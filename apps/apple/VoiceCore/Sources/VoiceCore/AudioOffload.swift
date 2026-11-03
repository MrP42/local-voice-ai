import Foundation

/// Hörbuch vom Gerät entfernen, ohne etwas zu verlieren (Epic #210, P8 Paket 1).
///
/// Entfernt wird nur, wenn jedes Teil im Konto liegt und dort vollständig
/// bestätigt ist; sonst wäre das Hörbuch danach weg. Eintrag, Text und
/// Fortschritt bleiben immer erhalten. Plattformneutral (nur Foundation),
/// damit die Regeln auch unter Linux geprüft werden können.
public enum AudioOffload {
    /// Ein Teil im Konto, wie es der Abgleich kennt (`BlobRef`).
    public struct AccountFile: Equatable, Sendable {
        public var blob: String
        public var size: Int
        public init(blob: String, size: Int) { self.blob = blob; self.size = size }
    }

    public enum Verdict: Equatable, Sendable {
        case allowed
        /// Auf dem Gerät liegt nichts, was sich entfernen ließe.
        case nothingLocal
        /// Das Hörbuch wird gerade geladen.
        case downloading
        case notLoggedIn
        /// Das Konto kennt das Buch oder seine Audiodateien (noch) nicht.
        case notInAccount
        /// Anzahl oder Größe der Teile im Konto passt nicht zum Gerät.
        case differs
        /// Teile sind im Konto nicht vollständig bestätigt.
        case unconfirmed(missing: Int)
        /// Das Konto war nicht erreichbar und die Teile sind nicht schon bestätigt.
        case unreachable

        public var allowed: Bool { self == .allowed }

        /// Ein Abgleich kann den Zustand beheben.
        public var syncHelps: Bool {
            switch self {
            case .notInAccount, .differs, .unconfirmed: return true
            default: return false
            }
        }

        public var message: String {
            switch self {
            case .allowed: return "Das Hörbuch liegt vollständig im Konto und kann wieder geladen werden."
            case .nothingLocal: return "Auf diesem iPhone liegen keine Hörbuchdateien zu diesem Buch."
            case .downloading: return "Das Hörbuch wird gerade geladen. Entfernen geht, sobald der Download fertig oder abgebrochen ist."
            case .notLoggedIn: return "Ohne Konto gibt es keine Kopie außerhalb dieses iPhones. Melde dich unter „Konto & Sync“ an; nach dem Hochladen lässt sich das Hörbuch entfernen."
            case .notInAccount: return "Das Hörbuch liegt nur auf diesem iPhone. Es lässt sich entfernen, sobald der Abgleich es vollständig ins Konto hochgeladen hat."
            case .differs: return "Die Dateien im Konto passen nicht zu denen auf diesem iPhone. Erst nach dem nächsten Abgleich lässt sich das Hörbuch sicher entfernen."
            case .unconfirmed(let missing):
                return missing == 1
                    ? "Ein Teil ist im Konto noch nicht vollständig. Entfernen geht nach dem nächsten vollständigen Abgleich."
                    : "\(missing) Teile sind im Konto noch nicht vollständig. Entfernen geht nach dem nächsten vollständigen Abgleich."
            case .unreachable: return "Das Konto ist gerade nicht erreichbar. Vor dem Entfernen muss es bestätigen, dass alle Teile vollständig dort liegen. Bitte mit Verbindung erneut versuchen."
            }
        }
    }

    /// Darf das Hörbuch vom Gerät?
    /// - Parameters:
    ///   - localSizes: je Teil in Hörreihenfolge die Größe der lokalen Datei, `nil` wenn sie fehlt.
    ///   - hasLeftovers: unfertige Downloads (`.part`) oder Import-Reste liegen vor.
    ///   - account: Teile laut Konto in derselben Reihenfolge.
    ///   - confirmed: Blobs, die im Konto als vollständig bestätigt sind.
    public static func verdict(loggedIn: Bool, synced: Bool, downloading: Bool,
                               localSizes: [Int?], hasLeftovers: Bool,
                               account: [AccountFile], confirmed: Set<String>) -> Verdict {
        if downloading { return .downloading }
        let present = localSizes.compactMap { $0 }
        if present.isEmpty && !hasLeftovers { return .nothingLocal }
        guard loggedIn else { return .notLoggedIn }
        guard synced, !account.isEmpty else { return .notInAccount }
        guard account.count == localSizes.count else { return .differs }
        for (local, remote) in zip(localSizes, account) {
            if let local, local != remote.size { return .differs }
        }
        let missing = Set(account.map(\.blob)).subtracting(confirmed).count
        return missing == 0 ? .allowed : .unconfirmed(missing: missing)
    }

    /// Blobs, die vor dem Entfernen beim Konto zu prüfen sind (ohne Doppelte).
    public static func blobsToCheck(_ account: [AccountFile]) -> [String] {
        var seen = Set<String>()
        return account.map(\.blob).filter { seen.insert($0).inserted }
    }

    /// Ordnername des Hörbuchs im Buchordner.
    public static let audioFolder = "audio"
    /// Vorsilbe der Zwischenordner beim Import (`audio.neu-<uuid>`).
    public static let stagingPrefix = "audio.neu-"

    /// Alles im Buchordner, was zum Hörbuch gehört: `audio/` (mit `.part`) und Import-Reste.
    public static func audioItems(in bookDir: URL) -> [URL] {
        let fm = FileManager.default
        let names = (try? fm.contentsOfDirectory(atPath: bookDir.path)) ?? []
        return names.filter { $0 == audioFolder || $0.hasPrefix(stagingPrefix) }
            .sorted()
            .map { bookDir.appendingPathComponent($0, isDirectory: true) }
    }

    /// Liegen unfertige Downloads oder Import-Reste im Buchordner?
    public static func hasLeftovers(in bookDir: URL) -> Bool {
        let fm = FileManager.default
        for item in audioItems(in: bookDir) {
            if item.lastPathComponent.hasPrefix(stagingPrefix) { return true }
            let names = (try? fm.contentsOfDirectory(atPath: item.path)) ?? []
            if names.contains(where: { $0.hasSuffix(".part") }) { return true }
        }
        return false
    }

    /// Belegter Platz einer Datei oder eines Ordners (rekursiv), in Bytes.
    public static func bytes(at url: URL) -> Int {
        let fm = FileManager.default
        var isDir: ObjCBool = false
        guard fm.fileExists(atPath: url.path, isDirectory: &isDir) else { return 0 }
        if !isDir.boolValue { return fileSize(url.path) }
        var total = 0
        if let walker = fm.enumerator(atPath: url.path) {
            while let rel = walker.nextObject() as? String {
                total += fileSize(url.appendingPathComponent(rel).path)
            }
        }
        return total
    }

    /// Hörbuchdateien des Buchs löschen und den frei gewordenen Platz melden.
    /// Prüft selbst nichts: vorher `verdict` fragen. Löscht den ganzen Ordner
    /// `audio/`; Dateien darin, die zu keinem Teil gehören, spielt die App ohnehin nicht ab.
    @discardableResult
    public static func remove(bookDir: URL) throws -> Int {
        var freed = 0
        for item in audioItems(in: bookDir) {
            let size = bytes(at: item)
            try FileManager.default.removeItem(at: item)
            freed += size
        }
        return freed
    }

    private static func fileSize(_ path: String) -> Int {
        guard let attrs = try? FileManager.default.attributesOfItem(atPath: path),
              (attrs[.type] as? FileAttributeType) == .typeRegular else { return 0 }
        return (attrs[.size] as? NSNumber)?.intValue ?? 0
    }
}

/// Speicherübersicht der Hörbücher auf dem Gerät (Epic #210, P8 Paket 2).
public enum AudioStorage {
    public struct Item: Equatable, Sendable, Identifiable {
        public var id: String
        public var title: String
        /// Belegter Platz auf dem Gerät, mit unfertigen Downloads.
        public var bytes: Int
        /// Gehörter Anteil 0…1.
        public var fraction: Double
        public var finished: Bool
        /// Letzte Nutzung (ms seit 1970), `nil` wenn nie gehört.
        public var lastUsedMs: Double?
        /// Nicht alle Teile liegen vollständig auf dem Gerät.
        public var partial: Bool
        /// Darf (nach Prüfung beim Konto) entfernt werden.
        public var removable: Bool
        /// Liegt nur auf diesem Gerät (nicht im Konto oder ohne Konto).
        public var localOnly: Bool
        public init(id: String, title: String, bytes: Int, fraction: Double, finished: Bool,
                    lastUsedMs: Double?, partial: Bool, removable: Bool, localOnly: Bool = false) {
            self.id = id; self.title = title; self.bytes = bytes; self.fraction = fraction; self.finished = finished
            self.lastUsedMs = lastUsedMs; self.partial = partial; self.removable = removable; self.localOnly = localOnly
        }

        /// Stand in Worten, z. B. „45 % gehört“.
        public var stand: String {
            if finished { return "Fertig gehört" }
            if partial && fraction <= 0 { return "Download unvollständig" }
            let percent = Int((min(max(fraction, 0), 1) * 100).rounded())
            let heard = percent == 0 ? "Noch nicht gehört" : "\(percent) % gehört"
            return partial ? heard + " · Download unvollständig" : heard
        }
    }

    /// Größte zuerst; bei gleicher Größe nach Titel.
    public static func sorted(_ items: [Item]) -> [Item] {
        items.sorted { $0.bytes != $1.bytes ? $0.bytes > $1.bytes
            : $0.title.localizedStandardCompare($1.title) == .orderedAscending }
    }

    public static func total(_ items: [Item]) -> Int { items.reduce(0) { $0 + $1.bytes } }

    /// Kandidaten für „Fertig gehörte entfernen“.
    public static func finishedRemovable(_ items: [Item]) -> [Item] { sorted(items.filter { $0.finished && $0.removable }) }

    /// Ergebnis einer Sammelentfernung in Worten.
    public static func summary(removed: Int, freedText: String, kept: Int) -> String {
        let head: String
        switch removed {
        case 0: head = "Nichts entfernt."
        case 1: head = "1 Hörbuch entfernt, \(freedText) frei."
        default: head = "\(removed) Hörbücher entfernt, \(freedText) frei."
        }
        guard kept > 0 else { return head }
        return head + (kept == 1 ? " 1 Hörbuch bleibt, weil das Konto es nicht als vollständig bestätigt hat."
                                 : " \(kept) Hörbücher bleiben, weil das Konto sie nicht als vollständig bestätigt hat.")
    }
}
