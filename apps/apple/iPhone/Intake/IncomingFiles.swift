import Foundation

/// Dateien, die per „Öffnen in“ an die App gingen und noch auf die Bestätigung warten.
/// iOS legt sie als Kopie unter Documents/Inbox ab. Die Warteliste wird beim Start aus
/// diesem Ordner neu aufgebaut (sie lebt nicht nur im Speicher). Eine Kopie verschwindet
/// erst, wenn sie dauerhaft im Eingang liegt oder ausdrücklich verworfen wurde.
@MainActor
final class IncomingFiles: ObservableObject {
    static let shared = IncomingFiles()
    @Published private(set) var pending: [URL] = []

    private var inboxPath: String {
        FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("Inbox").standardizedFileURL.path
    }

    func add(_ url: URL) { if !pending.contains(url) { pending.append(url) } }

    /// Reste aus früheren Starts: nur reguläre Dateien (keine Symlinks) direkt im Inbox-Ordner.
    func rescan() {
        let fm = FileManager.default
        let dir = URL(fileURLWithPath: inboxPath, isDirectory: true)
        for url in (try? fm.contentsOfDirectory(at: dir, includingPropertiesForKeys: nil)) ?? []
        where (try? fm.attributesOfItem(atPath: url.path)[.type] as? FileAttributeType) == .typeRegular { add(url) }
    }

    /// Erledigt: dauerhaft im Eingang oder ausdrücklich verworfen. Nur die iOS-Kopie im
    /// eigenen Inbox-Ordner wird gelöscht, nie eine Quelle in „Dateien“.
    func finish(_ url: URL, deleteCopy: Bool) {
        pending.removeAll { $0 == url }
        if deleteCopy, url.standardizedFileURL.path.hasPrefix(inboxPath + "/") { try? FileManager.default.removeItem(at: url) }
    }
}
