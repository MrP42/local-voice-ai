import SwiftUI
import UIKit
import PDFKit
import PhotosUI
import Vision
import VisionKit
import ImageIO
import NaturalLanguage
import UniformTypeIdentifiers

// MARK: - Texterkennung (lokal, Apple Vision)

/// Erkennt Text in Bildern und Bild-PDFs auf dem Gerät. Kein Netz, kein Cloud-Dienst.
enum ImageTextReader {
    enum Failure: Error { case unreadableImage }

    /// Obergrenze der Pixel je Bild (Speicher). Große Fotos werden verkleinert, lange Bildschirmfotos nicht.
    static let maxPixels: Double = 40_000_000
    /// Sprachen der genauen Erkennung, Deutsch zuerst.
    static let languages = ["de-DE", "en-US", "fr-FR", "it-IT", "es-ES"]

    /// Text mehrerer Bilder in ihrer Reihenfolge, je Bild ein Abschnitt. Läuft abseits des Haupt-Threads.
    /// Ein unlesbares Bild bricht ab (Fehler), ein Bild ohne Text nicht.
    static func text(fromImages images: [Data]) async throws -> String {
        try await Task.detached(priority: .userInitiated) {
            try images.compactMap { data -> String? in
                try autoreleasepool {
                    guard let image = cgImage(data) else { throw Failure.unreadableImage }
                    let text = try recognize(image)
                    return text.isEmpty ? nil : text
                }
            }.joined(separator: "\n\n")
        }.value
    }

    /// Wie oben, die Bilder werden erst abseits des Haupt-Threads gelesen.
    static func text(fromFiles files: [URL]) async throws -> String {
        let images = try await Task.detached(priority: .userInitiated) { try files.map { try Data(contentsOf: $0) } }.value
        return try await text(fromImages: images)
    }

    /// Seite eines PDFs ohne Textschicht lesen (Scan). Rendering mit Faktor 1, damit die Pixelzahl
    /// nicht mit dem Bildschirmfaktor wächst; je Seite ein eigener Speicherpool.
    static func text(ofScannedPage page: PDFPage) throws -> String {
        try autoreleasepool {
            let box = page.bounds(for: .mediaBox)
            guard box.width > 0, box.height > 0 else { return "" }
            let scale = 2200 / max(box.width, box.height)
            let size = CGSize(width: (box.width * scale).rounded(), height: (box.height * scale).rounded())
            let format = UIGraphicsImageRendererFormat()
            format.scale = 1
            format.opaque = true
            let rendered = UIGraphicsImageRenderer(size: size, format: format).image { ctx in
                UIColor.white.setFill()
                ctx.fill(CGRect(origin: .zero, size: size))
                ctx.cgContext.translateBy(x: 0, y: size.height)
                ctx.cgContext.scaleBy(x: scale, y: -scale)
                page.draw(with: .mediaBox, to: ctx.cgContext)
            }
            guard let image = rendered.cgImage else { return "" }
            return try recognize(image)
        }
    }

    /// Bild laden, Ausrichtung aus den EXIF-Angaben anwenden, sehr große Bilder verkleinern.
    static func cgImage(_ data: Data) -> CGImage? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
        let props = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any]
        let w = (props?[kCGImagePropertyPixelWidth] as? NSNumber)?.doubleValue ?? 0
        let h = (props?[kCGImagePropertyPixelHeight] as? NSNumber)?.doubleValue ?? 0
        var options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
        ]
        // Ohne Größenangabe nicht raten (sonst entstünde ein Winzbild): volle Größe.
        if w > 0, h > 0 {
            let scale = w * h > maxPixels ? (maxPixels / (w * h)).squareRoot() : 1
            options[kCGImageSourceThumbnailMaxPixelSize] = Int((max(w, h) * scale).rounded())
        }
        return CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
            ?? CGImageSourceCreateImageAtIndex(source, 0, nil)
    }

    /// Eine Seite: genaue Erkennung mit Sprachkorrektur.
    static func recognize(_ image: CGImage) throws -> String {
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        request.usesLanguageCorrection = true
        request.recognitionLanguages = languages
        try VNImageRequestHandler(cgImage: image, orientation: .up, options: [:]).perform([request])
        let lines: [OcrLine] = (request.results ?? []).compactMap { observation in
            guard let best = observation.topCandidates(1).first else { return nil }
            let b = observation.boundingBox
            return OcrLine(text: best.string, minX: b.minX, minY: b.minY, maxX: b.maxX, maxY: b.maxY)
        }
        return TextIntake.assemble(lines)
    }
}

// MARK: - Dokumentkamera

struct DocumentScanner: UIViewControllerRepresentable {
    static var isSupported: Bool { VNDocumentCameraViewController.isSupported }
    /// Gescannte Seiten als JPEG; leer bei Abbruch oder Fehler.
    let onDone: ([Data]) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(onDone: onDone) }
    func makeUIViewController(context: Context) -> VNDocumentCameraViewController {
        let controller = VNDocumentCameraViewController()
        controller.delegate = context.coordinator
        return controller
    }
    func updateUIViewController(_ controller: VNDocumentCameraViewController, context: Context) {}

    final class Coordinator: NSObject, VNDocumentCameraViewControllerDelegate {
        let onDone: ([Data]) -> Void
        init(onDone: @escaping ([Data]) -> Void) { self.onDone = onDone }
        func documentCameraViewController(_ controller: VNDocumentCameraViewController, didFinishWith scan: VNDocumentCameraScan) {
            let pages = (0..<scan.pageCount).compactMap { i in autoreleasepool { scan.imageOfPage(at: i).jpegData(compressionQuality: 0.9) } }
            onDone(pages)
        }
        func documentCameraViewControllerDidCancel(_ controller: VNDocumentCameraViewController) { onDone([]) }
        func documentCameraViewController(_ controller: VNDocumentCameraViewController, didFailWithError error: Error) { onDone([]) }
    }
}

// MARK: - Ablauf: Einfügen → prüfen → Vorlesen

/// Text oder Bilder mit Text werden ein Entwurf, den man vor dem Vorlesen prüfen und berichtigen kann.
/// „Vorlesen“ legt ihn als Eintrag in der Bibliothek an (wie ein importiertes Dokument, also auch
/// mit Sync und Lesestand) und startet den Player mit der gewählten Stimme.
@MainActor
final class TextIntakeModel: ObservableObject {
    static let shared = TextIntakeModel()

    struct Draft: Identifiable, Equatable {
        let id = UUID()
        var title: String
        var text: String
        /// Herkunft, nur zur Anzeige (z. B. „Fotos (Texterkennung)“).
        var source: String
        /// Eingang aus dem Teilen-Menü; wird erst nach Vorlesen oder Verwerfen entfernt.
        var inboxId: String?
    }

    @Published var draft: Draft?
    @Published private(set) var recognizing = false
    @Published var error: String?

    /// Buch, das nach dem Schließen des Blatts in den Player geht. Erst danach öffnen: Ein Vollbild
    /// lässt sich nicht zeigen, solange das Blatt noch verschwindet.
    private var pendingOpen: LibraryBook?

    private var inbox: TextInbox? { TextInbox.sharedRoot().map { TextInbox(root: $0) } }

    func startEmpty() { error = nil; draft = Draft(title: "", text: "", source: "Eingefügter Text") }

    /// Bilder erkennen und als neuen Entwurf öffnen, oder an den beim Start offenen Entwurf anhängen.
    /// Wurde dieser Entwurf inzwischen verworfen oder ersetzt, fällt das Ergebnis weg (kein Wiederöffnen).
    func recognize(_ images: [Data], source: String, requested: Int? = nil) async {
        if let requested, images.count < requested { error = "\(requested - images.count) Bild(er) konnten nicht geladen werden." }
        guard !images.isEmpty else {
            if requested != nil, error == nil { error = "Die Bilder konnten nicht geladen werden." }
            return
        }
        let target = draft?.id
        recognizing = true
        if requested == nil { error = nil }
        defer { recognizing = false }
        do {
            let text = try await ImageTextReader.text(fromImages: Array(images.prefix(TextInbox.maxImages)))
            guard draft?.id == target else { return }
            guard !text.isEmpty else { error = "Auf den Bildern wurde kein Text erkannt."; return }
            if draft != nil { append(text) } else { draft = Draft(title: "", text: text, source: source) }
            if images.count > TextInbox.maxImages { error = "Es wurden nur die ersten \(TextInbox.maxImages) Bilder gelesen." }
        } catch {
            self.error = "Die Bilder konnten nicht gelesen werden."
        }
    }

    func append(_ text: String) {
        let cleaned = TextIntake.clean(text)
        guard !cleaned.isEmpty, var d = draft else { return }
        d.text = d.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? cleaned : d.text + "\n\n" + cleaned
        draft = d
    }

    /// Ältester Eingang aus dem Teilen-Menü. Ein offener Entwurf wird nie überschrieben.
    /// Gelöscht wird ein Eingang nur nach „Vorlesen“, „Verwerfen“ oder wenn er nachweislich keinen Text enthält;
    /// scheitert die Erkennung technisch, bleibt er liegen und öffnet als Entwurf mit Hinweis.
    @discardableResult
    func takeFromInbox() async -> Bool {
        guard draft == nil, !recognizing, let inbox else { return false }
        inbox.sweepStaging()
        guard let item = inbox.pending().first else { return false }
        var parts: [String] = []
        if let text = item.text { parts.append(TextIntake.clean(text)) }
        var failed = false
        if !item.images.isEmpty {
            recognizing = true
            defer { recognizing = false }
            do {
                let text = try await ImageTextReader.text(fromFiles: item.images)
                if !text.isEmpty { parts.append(text) }
            } catch { failed = true }
        }
        // Während der Erkennung kann ein anderer Entwurf entstanden sein: dann bleibt der Eingang für später.
        guard draft == nil else { return false }
        let text = parts.filter { !$0.isEmpty }.joined(separator: "\n\n")
        if text.isEmpty, !failed {
            error = "Im geteilten Inhalt wurde kein Text gefunden."
            inbox.remove(item.id)
            return false
        }
        error = failed ? "Die geteilten Bilder konnten nicht gelesen werden. „Verwerfen“ entfernt den Eingang." : nil
        let source = item.images.isEmpty ? "Teilen-Menü" : "Teilen-Menü (Texterkennung)"
        draft = Draft(title: "", text: text, source: source, inboxId: item.id)
        return true
    }

    var hasPendingShares: Bool { !(inbox?.pending().isEmpty ?? true) }

    func discard() {
        if let id = draft?.inboxId { inbox?.remove(id) }
        error = nil
        draft = nil
    }

    /// Blatt ist zu: Player öffnen oder den nächsten geteilten Eingang zeigen.
    func sheetDismissed() {
        if let book = pendingOpen {
            pendingOpen = nil
            LibraryPlayback.shared.open(book, autoplay: true)
        } else {
            Task { await takeFromInbox() }
        }
    }

    /// In die Bibliothek und nach dem Schließen des Blatts vorlesen.
    func readAloud() {
        guard let d = draft else { return }
        let text = d.text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { error = "Es gibt noch keinen Text zum Vorlesen."; return }
        guard text.utf16.count <= TextIntake.maxCharacters else { error = "Der Text ist zu lang (höchstens 2 Millionen Zeichen)."; return }
        let typed = d.title.trimmingCharacters(in: .whitespacesAndNewlines)
        let title = typed.isEmpty ? TextIntake.title(for: String(text.prefix(2_000)), fallback: Self.fallbackTitle()) : typed
        do {
            let book = try PhoneLibrary.shared.importPlainText(title: title, text: text, sourceName: d.source)
            if let id = d.inboxId { inbox?.remove(id) }
            error = nil
            pendingOpen = book
            draft = nil
            Task { await LibrarySync.shared.run() }
        } catch {
            self.error = "Der Text konnte nicht gespeichert werden. Bitte freien Speicher prüfen."
        }
    }

    nonisolated static func fallbackTitle(now: Date = Date()) -> String {
        "Text vom " + now.formatted(.dateTime.day().month(.twoDigits).year().hour().minute().locale(Locale(identifier: "de_DE")))
    }

    /// Bilddateien aus der Dateien-App: Sicherheitsbereich nur während des Lesens offen, abseits des Haupt-Threads.
    nonisolated static func readImageFiles(_ urls: [URL]) async -> [Data] {
        await Task.detached(priority: .userInitiated) {
            urls.compactMap { url in
                let ok = url.startAccessingSecurityScopedResource()
                defer { if ok { url.stopAccessingSecurityScopedResource() } }
                return try? Data(contentsOf: url)
            }
        }.value
    }

    nonisolated static func isImage(_ url: URL) -> Bool {
        UTType(filenameExtension: url.pathExtension.lowercased())?.conforms(to: .image) ?? false
    }

    /// Fotos aus der Fotoauswahl laden; nicht ladbare (z. B. nur in iCloud) zählen als Fehler.
    static func loadPhotos(_ items: [PhotosPickerItem]) async -> [Data] {
        var images: [Data] = []
        for item in items { if let data = try? await item.loadTransferable(type: Data.self) { images.append(data) } }
        return images
    }
}

extension PhoneLibrary {
    /// Eingefügter oder erkannter Text als neuer Eintrag. Kein Zusammenführen über den Titel:
    /// zwei Einfügungen mit gleicher erster Zeile sind verschiedene Texte. Scheitert das Schreiben,
    /// bleibt kein halber Eintrag zurück.
    func importPlainText(title: String, text: String, sourceName: String) throws -> LibraryBook {
        var book = LibraryBook(id: Self.newId(), title: title, addedMs: Self.nowMs())
        // Sprache aus dem Anfang genügt und hält den Haupt-Thread frei.
        book.language = NLLanguageRecognizer.dominantLanguage(for: String(text.prefix(5_000))).map(\.rawValue)
        book.text = LibraryTextVersion(pageId: "ios_\(book.id)", origin: "document", sourceName: sourceName)
        do {
            try AssistantContentWatch.shared.around {
                try write(book)
                try text.write(to: textURL(book), atomically: true, encoding: .utf8)
                var m = meta(book); m.files.text = nil; m.updated = Self.nowMs(); setMeta(book, m)
            }
        } catch {
            delete(book)
            throw error
        }
        reload()
        return rawBook(book.id) ?? book
    }
}

// MARK: - Oberfläche

/// Menüeinträge im Plus-Menü der Bibliothek.
struct TextIntakeMenuItems: View {
    @Binding var importingFiles: Bool
    @Binding var pickingPhotos: Bool
    @Binding var scanning: Bool
    @ObservedObject var intake = TextIntakeModel.shared

    var body: some View {
        Button("Datei hinzufügen …", systemImage: "doc.badge.plus") { importingFiles = true }
            .accessibilityIdentifier("libraryAddFile")
        Button("Text einfügen", systemImage: "doc.on.clipboard") { intake.startEmpty() }
            .accessibilityIdentifier("libraryPasteText")
        Button("Text aus Fotos", systemImage: "photo.on.rectangle") { pickingPhotos = true }
            .accessibilityIdentifier("libraryPhotoText")
        if DocumentScanner.isSupported {
            Button("Text scannen", systemImage: "doc.viewfinder") { scanning = true }
                .accessibilityIdentifier("libraryScanText")
        }
    }
}

/// Fotoauswahl und Dokumentkamera; hängt am Regal der Bibliothek.
struct TextIntakeSources: ViewModifier {
    @Binding var pickingPhotos: Bool
    @Binding var scanning: Bool
    @ObservedObject var intake = TextIntakeModel.shared
    @State private var photoItems: [PhotosPickerItem] = []

    func body(content: Content) -> some View {
        content
            .photosPicker(isPresented: $pickingPhotos, selection: $photoItems, maxSelectionCount: TextInbox.maxImages, matching: .images)
            .onChange(of: photoItems) { _, items in
                guard !items.isEmpty else { return }
                photoItems = []
                Task { await intake.recognize(await TextIntakeModel.loadPhotos(items), source: "Fotos (Texterkennung)", requested: items.count) }
            }
            .fullScreenCover(isPresented: $scanning) {
                DocumentScanner { pages in
                    scanning = false
                    Task { await intake.recognize(pages, source: "Scan (Texterkennung)") }
                }.ignoresSafeArea()
            }
    }
}

/// Prüfblatt und Erkennungsanzeige; hängt an der Tab-Leiste, damit das Blatt unabhängig vom
/// gerade sichtbaren Tab und von gepushten Seiten erscheinen kann (z. B. nach dem Teilen-Menü).
struct TextIntakeHost: ViewModifier {
    @ObservedObject var intake = TextIntakeModel.shared

    func body(content: Content) -> some View {
        content
            .sheet(item: $intake.draft, onDismiss: { intake.sheetDismissed() }) { _ in TextIntakeSheet() }
            .overlay {
                if intake.recognizing && intake.draft == nil {
                    ProgressView("Text wird erkannt …").padding(20)
                        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12))
                        .accessibilityAddTraits(.updatesFrequently)
                }
            }
    }
}

struct TextIntakeSheet: View {
    @ObservedObject var intake = TextIntakeModel.shared
    @State private var pickingPhotos = false
    @State private var scanning = false
    @State private var confirmDiscard = false
    @FocusState private var editing: Bool

    private var text: Binding<String> {
        Binding(get: { intake.draft?.text ?? "" }, set: { intake.draft?.text = $0 })
    }
    private var title: Binding<String> {
        Binding(get: { intake.draft?.title ?? "" }, set: { intake.draft?.title = $0 })
    }
    private var isEmpty: Bool { (intake.draft?.text ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
    private var words: Int { (intake.draft?.text ?? "").split(whereSeparator: { $0.isWhitespace }).count }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    TextField(TextIntake.title(for: String((intake.draft?.text ?? "").prefix(2_000)), fallback: "Titel (optional)"), text: title)
                        .font(.headline).textInputAutocapitalization(.sentences)
                        .padding(12).background(VoicePalette.background, in: RoundedRectangle(cornerRadius: 8))
                        .overlay(RoundedRectangle(cornerRadius: 8).stroke(VoicePalette.border, lineWidth: 1))
                        .accessibilityLabel("Titel")
                        .accessibilityIdentifier("intakeTitle")

                    ViewThatFits(in: .horizontal) {
                        HStack(spacing: 8) { sources }
                        VStack(alignment: .leading, spacing: 8) { sources }
                    }

                    ZStack(alignment: .topLeading) {
                        TextEditor(text: text)
                            .focused($editing)
                            .frame(minHeight: 260)
                            .scrollContentBackground(.hidden)
                            .padding(8)
                            .accessibilityLabel("Text zum Vorlesen")
                            .accessibilityIdentifier("intakeText")
                        if isEmpty {
                            Text("Text hier einfügen oder tippen. Bilder mit Text über „Fotos“ oder „Scannen“ hinzufügen.")
                                .foregroundStyle(VoicePalette.secondaryText).padding(.horizontal, 13).padding(.vertical, 16)
                                .allowsHitTesting(false).accessibilityHidden(true)
                        }
                    }
                    .background(VoicePalette.background, in: RoundedRectangle(cornerRadius: 8))
                    .overlay(RoundedRectangle(cornerRadius: 8).stroke(VoicePalette.border, lineWidth: 1))

                    if intake.recognizing { ProgressView("Text wird erkannt …") }
                    if let error = intake.error { Text(error).font(.footnote).foregroundStyle(VoicePalette.danger) }
                    Text(footnote).font(.footnote).foregroundStyle(VoicePalette.secondaryText)
                        .fixedSize(horizontal: false, vertical: true)
                }.padding(16)
            }
            .scrollDismissesKeyboard(.interactively)
            .background(VoicePalette.background)
            .navigationTitle("Text vorlesen")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Verwerfen", role: .destructive) {
                        if isEmpty { intake.discard() } else { confirmDiscard = true }
                    }
                    .accessibilityIdentifier("intakeDiscard")
                    .confirmationDialog("Text verwerfen?", isPresented: $confirmDiscard, titleVisibility: .visible) {
                        Button("Verwerfen", role: .destructive) { intake.discard() }
                        Button("Behalten", role: .cancel) {}
                    } message: {
                        Text("Der Text wird nicht vorgelesen und nicht in der Bibliothek abgelegt.")
                    }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Vorlesen", systemImage: "play.fill") { intake.readAloud() }
                        .labelStyle(.titleAndIcon)
                        .disabled(isEmpty || intake.recognizing)
                        .accessibilityIdentifier("intakeRead")
                }
            }
            .modifier(TextIntakeSources(pickingPhotos: $pickingPhotos, scanning: $scanning))
        }
        // Wischen schließt nicht still, sobald Text da ist oder ein geteilter Eingang daran hängt:
        // erst „Verwerfen“ oder „Vorlesen“.
        .interactiveDismissDisabled(!isEmpty || intake.draft?.inboxId != nil)
    }

    @ViewBuilder private var sources: some View {
        // Systemknopf: liest die Zwischenablage ohne Rückfrage, weil der Tipp die Zustimmung ist.
        PasteButton(supportedContentTypes: [.plainText, .text, .image]) { providers in
            Task { await paste(providers) }
        }
        .buttonBorderShape(.capsule)
        .accessibilityIdentifier("intakePaste")
        Button("Fotos", systemImage: "photo.on.rectangle") { pickingPhotos = true }
            .buttonStyle(.bordered).frame(minHeight: 44).accessibilityIdentifier("intakePhotos")
        if DocumentScanner.isSupported {
            Button("Scannen", systemImage: "doc.viewfinder") { scanning = true }
                .buttonStyle(.bordered).frame(minHeight: 44).accessibilityIdentifier("intakeScan")
        }
    }

    private var footnote: String {
        let origin = intake.draft?.source ?? ""
        let count = words == 1 ? "1 Wort" : "\(words) Wörter"
        return "\(count) · \(origin). Die Texterkennung läuft auf diesem iPhone; nichts verlässt das Gerät. „Vorlesen“ legt den Text in der Bibliothek ab."
    }

    /// Text aus der Zwischenablage anhängen; Bilder darin werden erkannt.
    private func paste(_ providers: [NSItemProvider]) async {
        var images: [Data] = []
        for provider in providers {
            if provider.hasItemConformingToTypeIdentifier(UTType.image.identifier) {
                if let data = await Self.loadData(provider, .image) { images.append(data) }
            } else if let string = await Self.loadString(provider) {
                intake.append(string)
            }
        }
        if !images.isEmpty { await intake.recognize(images, source: intake.draft?.source ?? "Zwischenablage (Texterkennung)") }
    }

    nonisolated static func loadData(_ provider: NSItemProvider, _ type: UTType) async -> Data? {
        await withCheckedContinuation { continuation in
            _ = provider.loadDataRepresentation(for: type) { data, _ in continuation.resume(returning: data) }
        }
    }

    nonisolated static func loadString(_ provider: NSItemProvider) async -> String? {
        guard provider.canLoadObject(ofClass: NSString.self) else { return nil }
        return await withCheckedContinuation { continuation in
            _ = provider.loadObject(ofClass: NSString.self) { object, _ in continuation.resume(returning: (object as? NSString).map { $0 as String }) }
        }
    }
}
