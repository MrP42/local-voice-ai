import UIKit
import SwiftUI
import UniformTypeIdentifiers

/// Share-Extension „Local Voice“ (Epic #247, P4): nimmt genau eine Audio- oder
/// Videodatei (auch Sprachmemos, .m4a/.qta) entgegen, holt die Bestätigung ein
/// und kopiert die Datei in den Eingang der App-Gruppe. Sie transkribiert nicht
/// (Speichergrenze der Extension); das macht die App beim nächsten Öffnen.
final class ShareViewController: UIViewController {
    private static let accepted: [UTType] = [.audiovisualContent, .audio, .movie]

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground
        guard let provider = firstProvider() else { return finish(error: "Diese Freigabe enthält keine Audio- oder Videodatei.") }
        let name = provider.suggestedName ?? "Aufnahme"
        show(ImportConsentView(fileName: name, channel: .import,
                               onConfirm: { [weak self] record in self?.deposit(provider, consent: record) },
                               onCancel: { [weak self] in self?.cancel() }))
    }

    private func firstProvider() -> NSItemProvider? {
        let items = (extensionContext?.inputItems as? [NSExtensionItem]) ?? []
        return items.flatMap { $0.attachments ?? [] }.first { p in Self.accepted.contains { p.hasItemConformingToTypeIdentifier($0.identifier) } }
    }

    private func deposit(_ provider: NSItemProvider, consent: ConsentRecord) {
        guard let type = Self.accepted.first(where: { provider.hasItemConformingToTypeIdentifier($0.identifier) }) else { return finish(error: "Dateityp wird nicht unterstützt.") }
        show(ProgressView("Aufnahme wird gesichert …").padding())
        // Die URL gilt nur innerhalb des Callbacks: dort vollständig und geprüft kopieren.
        _ = provider.loadFileRepresentation(for: type, openInPlace: false) { [weak self] url, _, error in
            let message: String?
            _ = error
            if let url {
                do {
                    guard let root = MeetingInbox.sharedRoot() else { throw IntakeError.invalid }
                    let named = try Self.rename(url, to: provider.suggestedName)
                    defer { if named != url { try? FileManager.default.removeItem(at: named.deletingLastPathComponent()) } }
                    _ = try MeetingInbox(root: root, digest: SHA256Digest()).deposit(named, from: .shareSheet, consent: consent)
                    message = nil
                } catch IntakeError.tooLarge {
                    message = "Die Datei ist größer als 2 GB."
                } catch {
                    message = "Die Aufnahme konnte nicht gesichert werden. Bitte freien Speicher prüfen."
                }
            } else {
                message = "Die Datei konnte nicht gelesen werden."
            }
            DispatchQueue.main.async { self?.finish(error: message) }
        }
    }

    /// Ursprünglichen Namen erhalten (die Extension bekommt oft nur eine Zufallsdatei).
    private static func rename(_ url: URL, to name: String?) throws -> URL {
        guard let name, !name.isEmpty else { return url }
        let ext = url.pathExtension
        let clean = name.components(separatedBy: CharacterSet(charactersIn: "/\\:?*\"<>|").union(.newlines)).joined(separator: " ").prefix(120)
        let target = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: target, withIntermediateDirectories: true)
        let file = target.appendingPathComponent(String(clean) + (ext.isEmpty || clean.hasSuffix("." + ext) ? "" : "." + ext))
        try FileManager.default.copyItem(at: url, to: file)
        return file
    }

    private func show<V: View>(_ content: V) {
        children.forEach { $0.willMove(toParent: nil); $0.view.removeFromSuperview(); $0.removeFromParent() }
        let host = UIHostingController(rootView: content)
        addChild(host)
        host.view.frame = view.bounds
        host.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.addSubview(host.view)
        host.didMove(toParent: self)
    }

    private func finish(error: String?) {
        let text = error ?? "In Local Voice übernommen. Die Transkription startet beim nächsten Öffnen der App."
        show(VStack(spacing: 16) {
            Image(systemName: error == nil ? "checkmark.circle" : "exclamationmark.triangle").font(.largeTitle)
            Text(text).multilineTextAlignment(.center)
            Button("Fertig") { [weak self] in
                if error == nil { self?.extensionContext?.completeRequest(returningItems: nil) } else { self?.cancel() }
            }.frame(minWidth: 44, minHeight: 44)
        }.padding(24))
    }

    private func cancel() {
        extensionContext?.cancelRequest(withError: NSError(domain: NSCocoaErrorDomain, code: NSUserCancelledError))
    }
}
