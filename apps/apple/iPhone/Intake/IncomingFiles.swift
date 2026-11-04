import Foundation

/// Dateien, die per „Öffnen in“ an die App gingen und noch auf die Bestätigung warten.
/// iOS legt sie als Kopie unter Documents/Inbox ab; nach der Übernahme in den Eingang
/// wird diese Kopie entfernt.
@MainActor
final class IncomingFiles: ObservableObject {
    static let shared = IncomingFiles()
    @Published private(set) var pending: [URL] = []

    func add(_ url: URL) { if !pending.contains(url) { pending.append(url) } }

    /// Nächste Datei zur Bestätigung.
    var next: URL? { pending.first }

    /// Aus der Warteliste nehmen (Bestätigung offen oder erledigt).
    func take(_ url: URL) { pending.removeAll { $0 == url } }

    /// Die iOS-Kopie entfernen, sobald sie im Eingang gesichert oder verworfen ist.
    func discardCopy(_ url: URL) {
        pending.removeAll { $0 == url }
        // Nur die iOS-Kopie im eigenen Inbox-Ordner löschen, nie die Quelle in Dateien.
        let inbox = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("Inbox").standardizedFileURL.path
        if url.standardizedFileURL.path.hasPrefix(inbox + "/") { try? FileManager.default.removeItem(at: url) }
    }
}
