import SwiftUI
import UIKit
import PDFKit
import PhotosUI
import Vision
import VisionKit
import ImageIO
import NaturalLanguage
import UniformTypeIdentifiers
import CoreTransferable

// MARK: - Texterkennung (lokal, Apple Vision)

/// Erkennt Text in Bildern und Bild-PDFs auf dem Gerät. Kein Netz, kein Cloud-Dienst.
enum ImageTextReader {
    enum Failure: Error { case unreadableImage }

    /// Obergrenze der Pixel je Bild (Speicher). Große Fotos werden verkleinert, lange Bildschirmfotos nicht.
    static let maxPixels: Double = 40_000_000
    /// Höchstens so viele PDF-Seiten ohne Textschicht je Import; darüber wird abgelehnt, nicht gekürzt.
    static let maxScannedPages = 600
    /// Sprachen der genauen Erkennung, Deutsch zuerst.
    static let languages = ["de-DE", "en-US", "fr-FR", "it-IT", "es-ES"]

    /// Text eines Bilds. Läuft abseits des Haupt-Threads; die Daten werden danach freigegeben.
    static func text(ofImage data: Data) async throws -> String {
        try await Task.detached(priority: .userInitiated) {
            try autoreleasepool {
                guard let image = cgImage(data) else { throw Failure.unreadableImage }
                return try recognize(image)
            }
        }.value
    }

    static func text(ofImageFile url: URL) async throws -> String {
        try await Task.detached(priority: .userInitiated) {
            try autoreleasepool {
                guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
                      let image = cgImage(source) else { throw Failure.unreadableImage }
                return try recognize(image)
            }
        }.value
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
        return cgImage(source)
    }

    private static func cgImage(_ source: CGImageSource) -> CGImage? {
        let props = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any]
        let w = (props?[kCGImagePropertyPixelWidth] as? NSNumber)?.doubleValue ?? 0
        let h = (props?[kCGImagePropertyPixelHeight] as? NSNumber)?.doubleValue ?? 0
        guard let dimension = TextIntake.thumbnailDimension(width: w, height: h) else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: dimension,
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary),
              TextIntake.imagePixelsAllowed(width: image.width, height: image.height) else {
            return nil
        }
        return image
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

enum ScanImportError: Error { case tooManyPages(Int), reviewBusy }

/// Die Anbieter-URL gilt nur während der Übertragung; nur geprüfte Dateikopien behalten.
private struct IntakePhotoFile: Transferable {
    let url: URL

    static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(importedContentType: .image) { received in
            IntakePhotoFile(url: try copy(received.file))
        }
    }

    static func copy(_ source: URL) throws -> URL {
        guard TextIntake.imageSizeAllowed(bytes: TextInbox.fileSize(source), total: 0) else {
            throw TextInbox.Failure.tooLarge
        }
        let target = FileManager.default.temporaryDirectory
            .appendingPathComponent("vorlesen-\(UUID().uuidString)")
        do {
            try FileManager.default.copyItem(at: source, to: target)
            guard TextIntake.imageSizeAllowed(bytes: TextInbox.fileSize(target), total: 0) else {
                throw TextInbox.Failure.tooLarge
            }
            return target
        } catch {
            try? FileManager.default.removeItem(at: target)
            throw error
        }
    }
}

/// Woher ein Bild kommt. Geladen wird erst unmittelbar vor der Erkennung, eins nach dem anderen;
/// so liegt nie mehr als ein Bild zugleich im Speicher.
enum ImageSource {
    case data(Data)
    /// Datei; `scoped`: Sicherheitsbereich der Dateien-App öffnen.
    case file(URL, scoped: Bool)
    case photo(PhotosPickerItem)
    case provider(NSItemProvider)
    case scanPage(VNDocumentCameraScan, Int)

    /// Größe vorab, wo sie ohne Laden bekannt ist.
    var knownSize: Int? {
        switch self {
        case .data(let d): return d.count
        case .file(let url, let scoped):
            let ok = scoped && url.startAccessingSecurityScopedResource()
            defer { if ok { url.stopAccessingSecurityScopedResource() } }
            return TextInbox.fileSize(url)
        default: return nil
        }
    }

    @MainActor func read(total: inout Int) async throws -> (text: String, bytes: Int) {
        switch self {
        case .data(let data):
            return try await Self.read(data, total: &total)
        case .file(let url, let scoped):
            let ok = scoped && url.startAccessingSecurityScopedResource()
            defer { if ok { url.stopAccessingSecurityScopedResource() } }
            return try await Self.read(url, total: &total)
        case .photo(let item):
            guard let file = try await item.loadTransferable(type: IntakePhotoFile.self) else {
                throw ImageTextReader.Failure.unreadableImage
            }
            defer { try? FileManager.default.removeItem(at: file.url) }
            return try await Self.read(file.url, total: &total)
        case .provider(let provider):
            let url: URL = try await withCheckedThrowingContinuation { continuation in
                _ = provider.loadFileRepresentation(for: .image, openInPlace: false) {
                    url, _, _ in
                    guard let url else {
                        continuation.resume(throwing: ImageTextReader.Failure.unreadableImage)
                        return
                    }
                    do { continuation.resume(returning: try IntakePhotoFile.copy(url)) }
                    catch { continuation.resume(throwing: error) }
                }
            }
            defer { try? FileManager.default.removeItem(at: url) }
            return try await Self.read(url, total: &total)
        case .scanPage(let scan, let i):
            guard let data = autoreleasepool(invoking: {
                scan.imageOfPage(at: i).jpegData(compressionQuality: 0.9)
            }) else { throw ImageTextReader.Failure.unreadableImage }
            return try await Self.read(data, total: &total)
        }
    }

    private static func read(_ url: URL, total: inout Int) async throws -> (String, Int) {
        let size = TextInbox.fileSize(url)
        guard TextIntake.imageSizeAllowed(bytes: size, total: total), let size else {
            throw TextInbox.Failure.tooLarge
        }
        total += size
        return (try await ImageTextReader.text(ofImageFile: url), size)
    }

    private static func read(_ data: Data, total: inout Int) async throws -> (String, Int) {
        guard TextIntake.imageSizeAllowed(bytes: data.count, total: total) else {
            throw TextInbox.Failure.tooLarge
        }
        total += data.count
        return (try await ImageTextReader.text(ofImage: data), data.count)
    }
}

// MARK: - Dokumentkamera

struct DocumentScanner: UIViewControllerRepresentable {
    enum Outcome { case scanned(VNDocumentCameraScan), cancelled, failed }
    static var isSupported: Bool { VNDocumentCameraViewController.isSupported }
    let onDone: (Outcome) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(onDone: onDone) }
    func makeUIViewController(context: Context) -> VNDocumentCameraViewController {
        let controller = VNDocumentCameraViewController()
        controller.delegate = context.coordinator
        return controller
    }
    func updateUIViewController(_ controller: VNDocumentCameraViewController, context: Context) {}

    final class Coordinator: NSObject, VNDocumentCameraViewControllerDelegate {
        let onDone: (Outcome) -> Void
        init(onDone: @escaping (Outcome) -> Void) { self.onDone = onDone }
        func documentCameraViewController(_ controller: VNDocumentCameraViewController, didFinishWith scan: VNDocumentCameraScan) {
            onDone(.scanned(scan))
        }
        func documentCameraViewControllerDidCancel(_ controller: VNDocumentCameraViewController) { onDone(.cancelled) }
        func documentCameraViewController(_ controller: VNDocumentCameraViewController, didFailWithError error: Error) { onDone(.failed) }
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

    /// Jeder Wechsel des Entwurfs (neu, verworfen, vorgelesen) lässt laufende Vorgänge verfallen.
    @Published var draft: Draft? {
        didSet { if draft?.id != oldValue?.id { draftGuard.switchTo(draft?.id) } }
    }
    @Published private(set) var recognizing = false
    @Published var error: String?

    private var draftGuard = DraftGuard()
    /// Buch, das nach dem Schließen des Blatts in den Player geht. Erst danach öffnen: Ein Vollbild
    /// lässt sich nicht zeigen, solange das Blatt noch verschwindet.
    private var pendingOpen: LibraryBook?

    private var inbox: TextInbox? { TextInbox.sharedRoot().map { TextInbox(root: $0) } }

    /// Vor dem ersten Warten ziehen; das Ergebnis zählt nur, solange das Ticket gilt.
    func ticket() -> DraftGuard.Ticket { draftGuard.ticket() }

    func startEmpty() { error = nil; draft = Draft(title: "", text: "", source: "Eingefügter Text") }

    /// Fertig erkannter Text (Scan-PDF) zur Prüfung. `false`, wenn schon ein Entwurf offen ist.
    func openReview(title: String, text: String, source: String) -> Bool {
        guard draft == nil else { return false }
        error = nil
        draft = Draft(title: title, text: TextIntake.clean(text), source: source)
        return true
    }

    /// Bilder einzeln erkennen; Dateigrößen vor dem Dekodieren prüfen, auch in Anbieter-Rückrufen.
    /// Kameraseiten und vorhandene Daten werden vor der Erkennung geprüft. Ergebnis: neuer Entwurf oder
    /// Anhang an den Entwurf, der beim Start offen war. Hat der Nutzer ihn inzwischen verworfen,
    /// vorgelesen oder einen anderen begonnen, fällt das Ergebnis weg.
    func recognize(_ sources: [ImageSource], source: String, ticket: DraftGuard.Ticket) async {
        guard !sources.isEmpty else { return }
        guard !recognizing else { error = "Es läuft noch eine Texterkennung. Bitte kurz warten."; return }
        recognizing = true
        defer { recognizing = false }
        let result = await Self.read(sources)
        guard draftGuard.isValid(ticket) else { return }
        error = result.note
        guard !result.text.isEmpty else {
            if result.note == nil { error = "Auf den Bildern wurde kein Text erkannt." }
            return
        }
        if draft != nil { append(result.text) } else { draft = Draft(title: "", text: result.text, source: source) }
    }

    /// Kern der Erkennung: eins nach dem anderen, nichts gesammelt außer dem Text.
    private static func read(_ sources: [ImageSource]) async -> (text: String, note: String?, failed: Bool) {
        var parts: [String] = []
        var notes: [String] = []
        var total = 0, unreadable = 0, tooBig = 0
        let used = sources.prefix(TextInbox.maxImages)
        if sources.count > TextInbox.maxImages { notes.append("Nur die ersten \(TextInbox.maxImages) Bilder wurden gelesen.") }
        for item in used {
            do {
                let result = try await item.read(total: &total)
                if !result.text.isEmpty { parts.append(result.text) }
            } catch TextInbox.Failure.tooLarge {
                tooBig += 1
            } catch { unreadable += 1 }
        }
        if unreadable > 0 { notes.append("\(unreadable) Bild(er) konnten nicht gelesen werden.") }
        if tooBig > 0 { notes.append("\(tooBig) Bild(er) ausgelassen (je bis 40 MB, zusammen bis 200 MB).") }
        return (parts.joined(separator: "\n\n"), notes.isEmpty ? nil : notes.joined(separator: " "), unreadable > 0)
    }

    /// Text anhängen, nur solange das Ticket gilt (Einfügen aus der Zwischenablage lädt asynchron).
    func append(_ text: String, ticket: DraftGuard.Ticket) {
        guard draftGuard.isValid(ticket) else { return }
        append(text)
    }

    private func append(_ text: String) {
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
        if inbox.sweep() > 0 { error = "Ein beschädigter geteilter Inhalt wurde entfernt." }
        guard let item = inbox.pending().first else { return false }
        let ticket = draftGuard.ticket()
        var parts: [String] = []
        if let text = item.text { parts.append(TextIntake.clean(text)) }
        var note: String?, failed = false
        if !item.images.isEmpty {
            recognizing = true
            defer { recognizing = false }
            let result = await Self.read(item.images.map { .file($0, scoped: false) })
            if !result.text.isEmpty { parts.append(result.text) }
            note = result.note; failed = result.failed
        }
        // Während der Erkennung kann ein anderer Entwurf entstanden sein: dann bleibt der Eingang für später.
        guard draftGuard.isValid(ticket) else { return false }
        let text = parts.filter { !$0.isEmpty }.joined(separator: "\n\n")
        if text.isEmpty, !failed {
            error = "Im geteilten Inhalt wurde kein Text gefunden."
            inbox.remove(item.id)
            return false
        }
        error = failed ? (note ?? "") + " „Verwerfen“ entfernt den Eingang." : note
        let source = item.images.isEmpty ? "Teilen-Menü" : "Teilen-Menü (Texterkennung)"
        draft = Draft(title: "", text: text, source: source, inboxId: item.id)
        return true
    }

    var hasPendingShares: Bool { !(inbox?.pending().isEmpty ?? true) }

    /// Auch ohne lesbaren Eingang und bei offenem Entwurf beim Aktivieren aufräumen.
    func sweepInbox() {
        if let inbox, inbox.sweep() > 0 {
            error = "Ein beschädigter geteilter Inhalt wurde entfernt."
        }
    }

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
        guard let d = draft, !recognizing else { return }
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

    nonisolated static func isImage(_ url: URL) -> Bool {
        UTType(filenameExtension: url.pathExtension.lowercased())?.conforms(to: .image) ?? false
    }

    /// Ergebnis der Dokumentkamera.
    func scanned(_ outcome: DocumentScanner.Outcome, ticket: DraftGuard.Ticket) async {
        switch outcome {
        case .cancelled: return
        case .failed: error = "Der Scan ist fehlgeschlagen. Bitte erneut versuchen."
        case .scanned(let scan):
            await recognize((0..<scan.pageCount).map { .scanPage(scan, $0) }, source: "Scan (Texterkennung)", ticket: ticket)
        }
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

/// Fotoauswahl und Dokumentkamera; hängt am Regal der Bibliothek und im Prüfblatt.
struct TextIntakeSources: ViewModifier {
    @Binding var pickingPhotos: Bool
    @Binding var scanning: Bool
    @ObservedObject var intake = TextIntakeModel.shared
    @State private var photoItems: [PhotosPickerItem] = []
    /// Ticket beim Öffnen der Auswahl bzw. Kamera: Das Ergebnis gehört zum Entwurf von diesem Moment.
    @State private var ticket: DraftGuard.Ticket?

    func body(content: Content) -> some View {
        content
            .photosPicker(isPresented: $pickingPhotos, selection: $photoItems, maxSelectionCount: TextInbox.maxImages, matching: .images)
            .onChange(of: pickingPhotos) { _, open in if open { ticket = intake.ticket() } }
            .onChange(of: scanning) { _, open in if open { ticket = intake.ticket() } }
            .onChange(of: photoItems) { _, items in
                guard !items.isEmpty else { return }
                photoItems = []
                let t = ticket ?? intake.ticket()
                Task { await intake.recognize(items.map { .photo($0) }, source: "Fotos (Texterkennung)", ticket: t) }
            }
            .fullScreenCover(isPresented: $scanning) {
                DocumentScanner { outcome in
                    scanning = false
                    let t = ticket ?? intake.ticket()
                    Task { await intake.scanned(outcome, ticket: t) }
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
            let ticket = intake.ticket()
            Task { await paste(providers, ticket: ticket) }
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
        return "\(count) · \(origin). Die Texterkennung läuft nur auf diesem iPhone. „Vorlesen“ legt den Text in der Bibliothek ab; mit eingerichtetem Konto wird sie wie jedes Buch verschlüsselt synchronisiert."
    }

    /// Text aus der Zwischenablage anhängen; Bilder darin werden erkannt. Das Ticket stammt vom Tipp.
    private func paste(_ providers: [NSItemProvider], ticket: DraftGuard.Ticket) async {
        var images: [ImageSource] = []
        for provider in providers {
            if provider.hasItemConformingToTypeIdentifier(UTType.image.identifier) {
                images.append(.provider(provider))
            } else if let string = await Self.loadString(provider) {
                intake.append(string, ticket: ticket)
            }
        }
        if !images.isEmpty { await intake.recognize(images, source: intake.draft?.source ?? "Zwischenablage (Texterkennung)", ticket: ticket) }
    }

    nonisolated static func loadString(_ provider: NSItemProvider) async -> String? {
        guard provider.canLoadObject(ofClass: NSString.self) else { return nil }
        return await withCheckedContinuation { continuation in
            _ = provider.loadObject(ofClass: NSString.self) { object, _ in continuation.resume(returning: (object as? NSString).map { $0 as String }) }
        }
    }
}
