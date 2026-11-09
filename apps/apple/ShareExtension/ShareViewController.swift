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
        // Genau eine Datei: Mehrfachfreigaben werden abgelehnt, nicht still gekürzt.
        let all = (extensionContext?.inputItems as? [NSExtensionItem] ?? []).flatMap { $0.attachments ?? [] }
        let media = all.filter { p in Self.accepted.contains { p.hasItemConformingToTypeIdentifier($0.identifier) } }
        guard all.count == 1, let provider = media.first else {
            return finish(error: media.isEmpty ? "Diese Freigabe enthält keine Audio- oder Videodatei." : "Bitte genau eine Aufnahme teilen.")
        }
        let name = provider.suggestedName ?? "Aufnahme"
        show(ImportConsentView(fileName: name, channel: .import,
                               onConfirm: { [weak self] record in self?.deposit(provider, consent: record) },
                               onCancel: { [weak self] in self?.cancel() }))
    }

    private func deposit(_ provider: NSItemProvider, consent: ConsentRecord) {
        guard let type = Self.accepted.first(where: { provider.hasItemConformingToTypeIdentifier($0.identifier) }) else { return finish(error: "Dateityp wird nicht unterstützt.") }
        show(ProgressView("Aufnahme wird gesichert …").padding())
        // Die URL gilt nur innerhalb des Callbacks. `deposit` prüft Größe und Dateiart vor
        // dem Kopieren, kopiert ins Staging (APFS-Klon, kein Puffern im Speicher), prüft die
        // Prüfsumme, synchronisiert und räumt bei Fehlern auf. Der Name ist nur ein Metadatum.
        _ = provider.loadFileRepresentation(for: type, openInPlace: false) { [weak self] url, _, _ in
            let message: String?
            if let url {
                do {
                    guard let root = MeetingInbox.sharedRoot() else { throw IntakeError.invalid }
                    _ = try MeetingInbox(root: root, digest: SHA256Digest()).deposit(url, from: .shareSheet, consent: consent, name: provider.suggestedName)
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
