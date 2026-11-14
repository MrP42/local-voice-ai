import UIKit
import SwiftUI
import UniformTypeIdentifiers

/// Share-Extension „Local Voice“ (Epic #247, P4): nimmt genau eine Audio- oder
/// Videodatei (auch Sprachmemos, .m4a/.qta) entgegen, holt die Bestätigung ein
/// und kopiert die Datei in den Eingang der App-Gruppe. Sie transkribiert nicht
/// (Speichergrenze der Extension); das macht die App beim nächsten Öffnen.
///
/// Außerdem Text und Bilder mit Text zum Vorlesen: Sie gehen unverändert in den
/// Texteingang der App-Gruppe (`TextInbox`). Erkennung und Vorlesen macht die App
/// beim nächsten Öffnen; dort prüft man den Text vor dem Vorlesen.
final class ShareViewController: UIViewController {
    private static let accepted: [UTType] = [.audiovisualContent, .audio, .movie]

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground
        // Genau eine Datei: Mehrfachfreigaben werden abgelehnt, nicht still gekürzt.
        let all = (extensionContext?.inputItems as? [NSExtensionItem] ?? []).flatMap { $0.attachments ?? [] }
        let media = all.filter { p in Self.accepted.contains { p.hasItemConformingToTypeIdentifier($0.identifier) } }
        // Ohne Aufnahme: Text und Bilder zum Vorlesen.
        if media.isEmpty, all.contains(where: Self.isReadable) { return depositReadable(all.filter(Self.isReadable)) }
        guard all.count == 1, let provider = media.first else {
            return finish(error: media.isEmpty ? "Diese Freigabe enthält keine Audio- oder Videodatei." : "Bitte genau eine Aufnahme teilen, oder nur Text und Bilder zum Vorlesen.")
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

    // MARK: Text und Bilder zum Vorlesen

    private static func isReadable(_ p: NSItemProvider) -> Bool {
        p.hasItemConformingToTypeIdentifier(UTType.image.identifier) || p.hasItemConformingToTypeIdentifier(UTType.plainText.identifier)
    }

    private func depositReadable(_ providers: [NSItemProvider]) {
        show(ProgressView("Wird an Local Voice übergeben …").padding())
        Task { @MainActor in
            // Bilder als Dateikopien in einen eigenen Zwischenordner: Die Extension hat wenig Speicher,
            // deshalb wird kein Bild ganz geladen. Der Ordner verschwindet am Ende in jedem Fall.
            let temp = FileManager.default.temporaryDirectory.appendingPathComponent("vorlesen-\(UUID().uuidString)", isDirectory: true)
            defer { try? FileManager.default.removeItem(at: temp) }
            try? FileManager.default.createDirectory(at: temp, withIntermediateDirectories: true)
            var texts: [String] = []
            var images: [URL] = []
            var skipped = 0, total = 0
            for provider in providers {
                if provider.hasItemConformingToTypeIdentifier(UTType.image.identifier) {
                    let target = temp.appendingPathComponent(String(format: "bild-%03d", images.count + 1))
                    // Anzahl vorab, Größe je Bild und zusammen direkt nach dem Kopieren (Kopie, kein Laden).
                    if images.count < TextInbox.maxImages, await Self.copyImage(provider, to: target),
                       let size = TextInbox.fileSize(target), size <= TextInbox.maxImageBytes, total + size <= TextInbox.maxTotalImageBytes {
                        images.append(target); total += size
                    } else { skipped += 1; try? FileManager.default.removeItem(at: target) }
                } else if let text = await Self.loadText(provider) {
                    texts.append(text)
                }
            }
            let message: String?
            do {
                guard let root = TextInbox.sharedRoot() else { throw CocoaError(.fileNoSuchFile) }
                try TextInbox(root: root).deposit(text: texts.joined(separator: "\n\n"), imageFiles: images, source: "Teilen-Menü")
                message = nil
            } catch TextInbox.Failure.empty {
                message = "In dieser Freigabe wurde kein Text und kein lesbares Bild gefunden."
            } catch TextInbox.Failure.tooLarge {
                message = "Der Text oder ein Bild ist zu groß."
            } catch CocoaError.fileNoSuchFile {
                message = "Local Voice ist nicht vollständig eingerichtet. Bitte die App einmal öffnen."
            } catch {
                message = "Konnte nicht übergeben werden. Bitte freien Speicher prüfen."
            }
            let note = skipped > 0 ? " \(skipped) Bild(er) wurden ausgelassen (höchstens \(TextInbox.maxImages), je bis 40 MB, zusammen bis 200 MB)." : ""
            finish(error: message, success: "In Local Voice übernommen. Öffne Local Voice, um den Text zu prüfen und vorlesen zu lassen." + note)
        }
    }

    /// Bild als Datei kopieren; die URL des Anbieters gilt nur im Rückruf. Anbieter ohne Dateifassung
    /// (Bild nur im Speicher, z. B. aus einem Bildschirmfoto-Editor) liefern Daten, die direkt geschrieben werden.
    private static func copyImage(_ provider: NSItemProvider, to target: URL) async -> Bool {
        let copied: Bool = await withCheckedContinuation { continuation in
            _ = provider.loadFileRepresentation(for: .image, openInPlace: false) { url, _, _ in
                guard let url else { return continuation.resume(returning: false) }
                continuation.resume(returning: (try? FileManager.default.copyItem(at: url, to: target)) != nil)
            }
        }
        if copied { return true }
        guard let data = await loadData(provider, .image) else { return false }
        return (try? data.write(to: target)) != nil
    }

    private static func loadData(_ provider: NSItemProvider, _ type: UTType) async -> Data? {
        await withCheckedContinuation { continuation in
            _ = provider.loadDataRepresentation(for: type) { data, _ in continuation.resume(returning: data) }
        }
    }

    private static func loadText(_ provider: NSItemProvider) async -> String? {
        guard provider.canLoadObject(ofClass: NSString.self) else {
            return await loadData(provider, .plainText).flatMap { String(data: $0, encoding: .utf8) }
        }
        return await withCheckedContinuation { continuation in
            _ = provider.loadObject(ofClass: NSString.self) { object, _ in continuation.resume(returning: (object as? NSString).map { $0 as String }) }
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

    private func finish(error: String?, success: String = "In Local Voice übernommen. Die Transkription startet beim nächsten Öffnen der App.") {
        let text = error ?? success
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
