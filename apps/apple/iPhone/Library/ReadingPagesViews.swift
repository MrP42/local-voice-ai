import SwiftUI
import PhotosUI
import UIKit
import VoiceCore

// Reiter „Vorlesen“ (lva#786, S2b): Seiten aus Fotos, Scans und eingefügtem Text.
// Eine Seite ist dasselbe Objekt wie am Desktop (Sync-Sammlung `page`); die Aufnahmen,
// aus denen ihr Text stammt, bleiben auf dem iPhone. Alle Änderungen laufen über
// `ReadingPagesModel`, diese Datei zeigt nur an und leitet Eingaben weiter.

// MARK: - Reiter

struct ReadingPagesTab: View {
    @ObservedObject var pages = ReadingPagesModel.shared
    @ObservedObject var intake = TextIntakeModel.shared
    @ObservedObject var playback = LibraryPlayback.shared
    @State private var path: [String] = []
    @State private var query = ""
    @State private var pickingPhotos = false
    @State private var scanning = false
    @State private var confirmDelete: ReadingPage?

    private var visible: [ReadingPage] {
        let q = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !q.isEmpty else { return pages.pages }
        return pages.pages.filter { $0.title.localizedCaseInsensitiveContains(q) || $0.text.localizedCaseInsensitiveContains(q) }
    }

    var body: some View {
        NavigationStack(path: $path) {
            Group {
                if pages.pages.isEmpty { empty } else { list }
            }
            .background(VoicePalette.background)
            .navigationTitle("Vorlesen")
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Menu {
                        ReadingPageAddItems(pickingPhotos: $pickingPhotos, scanning: $scanning)
                    } label: {
                        Image(systemName: "plus").frame(minWidth: 44, minHeight: 44).contentShape(Rectangle())
                    }
                    .accessibilityLabel("Seite hinzufügen")
                    .accessibilityIdentifier("pagesAdd")
                }
            }
            .navigationDestination(for: String.self) { id in ReadingPageDetail(pageID: id) }
        }
        .modifier(ReadingPageSources(pickingPhotos: $pickingPhotos, scanning: $scanning, target: nil))
        .modifier(ReadingPagesFeedback())
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if playback.book != nil && !playback.presented {
                LibraryMiniPlayer().padding(.horizontal, 8).padding(.bottom, 6)
            }
        }
        .onAppear { pages.reload(); playback.warmUp() }
        .onChange(of: intake.openPageID, initial: true) { _, id in
            guard let id else { return }
            intake.openPageID = nil
            pages.reload()
            path = [id]
        }
        .confirmationDialog("Seite löschen?", isPresented: Binding(get: { confirmDelete != nil }, set: { if !$0 { confirmDelete = nil } }),
                            titleVisibility: .visible, presenting: confirmDelete) { page in
            Button("Löschen", role: .destructive) { pages.deletePage(page.id) }
            Button("Behalten", role: .cancel) {}
        } message: { page in
            Text("„\(page.title)“ wird auf dem iPhone und auf verbundenen Geräten gelöscht. Die Fotos der Seite werden entfernt.")
        }
    }

    private var list: some View {
        List {
            ForEach(visible) { page in
                NavigationLink(value: page.id) { ReadingPageRow(page: page, captures: pages.captures(of: page.id).count, recognizing: pages.recognizing == page.id) }
                    .accessibilityIdentifier("pageRow")
                    .swipeActions(edge: .trailing) {
                        Button("Löschen", systemImage: "trash", role: .destructive) { confirmDelete = page }
                    }
                    .swipeActions(edge: .leading) {
                        Button("Vorlesen", systemImage: "play.fill") { pages.readAloud(page.id) }
                            .tint(VoicePalette.accent)
                    }
                    .contextMenu {
                        Button("Vorlesen", systemImage: "play.fill") { pages.readAloud(page.id) }
                        Button("Löschen", systemImage: "trash", role: .destructive) { confirmDelete = page }
                    }
            }
            if visible.isEmpty {
                Text("Keine Seite passt zu „\(query)“.").foregroundStyle(VoicePalette.secondaryText)
            }
        }
        .listStyle(.insetGrouped)
        .scrollContentBackground(.hidden)
        .searchable(text: $query, prompt: "Titel oder Text")
        .refreshable { await LibrarySync.shared.run(); pages.reload() }
    }

    private var empty: some View {
        ContentUnavailableView {
            Label("Noch keine Seiten", systemImage: "doc.plaintext")
        } description: {
            Text("Fotografiere oder scanne einen Text, oder füge ihn ein. Daraus entsteht eine Seite, die du bearbeiten, erweitern und vorlesen lassen kannst.")
        } actions: {
            HStack(spacing: 12) {
                if DocumentScanner.isSupported {
                    Button("Scannen", systemImage: "doc.viewfinder") { scanning = true }
                        .buttonStyle(.borderedProminent).tint(VoicePalette.accent)
                        .accessibilityIdentifier("pagesEmptyScan")
                }
                Button("Fotos", systemImage: "photo.on.rectangle") { pickingPhotos = true }
                    .buttonStyle(.bordered)
                    .accessibilityIdentifier("pagesEmptyPhotos")
            }
            Button("Text einfügen", systemImage: "doc.on.clipboard") { intake.startEmpty() }
                .accessibilityIdentifier("pagesEmptyPaste")
        }
    }
}

/// Menüeinträge zum Anlegen einer Seite; dieselben drei Wege wie im Plus-Menü der Bibliothek.
struct ReadingPageAddItems: View {
    @Binding var pickingPhotos: Bool
    @Binding var scanning: Bool
    @ObservedObject var intake = TextIntakeModel.shared

    var body: some View {
        if DocumentScanner.isSupported {
            Button("Text scannen", systemImage: "doc.viewfinder") { scanning = true }
                .accessibilityIdentifier("pagesScan")
        }
        Button("Text aus Fotos", systemImage: "photo.on.rectangle") { pickingPhotos = true }
            .accessibilityIdentifier("pagesPhotos")
        Button("Text einfügen", systemImage: "doc.on.clipboard") { intake.startEmpty() }
            .accessibilityIdentifier("pagesPaste")
    }
}

struct ReadingPageRow: View {
    let page: ReadingPage
    let captures: Int
    let recognizing: Bool

    private var preview: String {
        page.text.split(whereSeparator: \.isNewline).lazy
            .map { $0.trimmingCharacters(in: .whitespaces) }.first(where: { !$0.isEmpty }) ?? "Noch kein Text"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(page.title).font(.headline).foregroundStyle(VoicePalette.text).lineLimit(2)
            Text(preview).font(.subheadline).foregroundStyle(VoicePalette.secondaryText).lineLimit(2)
            HStack(spacing: 6) {
                if recognizing {
                    ProgressView().controlSize(.mini)
                    Text("Text wird erkannt …")
                } else {
                    Text(Self.meta(captures: captures, characters: page.text.count))
                }
            }
            .font(.caption).foregroundStyle(VoicePalette.secondaryText)
        }
        .padding(.vertical, 4)
        .accessibilityElement(children: .combine)
    }

    static func meta(captures: Int, characters: Int) -> String {
        var parts: [String] = []
        if captures > 0 { parts.append(captures == 1 ? "1 Aufnahme" : "\(captures) Aufnahmen") }
        let minutes = max(1, Int((Double(characters) / 900).rounded()))
        parts.append(characters == 0 ? "leer" : "ca. \(minutes) Min. Vorlesen")
        return parts.joined(separator: " · ")
    }
}

// MARK: - Seite

struct ReadingPageDetail: View {
    let pageID: String
    @ObservedObject var pages = ReadingPagesModel.shared
    @Environment(\.dismiss) private var dismiss
    @Environment(\.scenePhase) private var scenePhase
    @State private var title = ""
    @State private var text = ""
    @State private var loaded = false
    @State private var pickingPhotos = false
    @State private var scanning = false
    @State private var insertAfter: String?
    @State private var arranging = false
    @State private var confirmDelete = false
    @State private var confirmDeleteCapture: ReadingCapture?
    @State private var showing: ReadingCapture?
    @FocusState private var focus: Field?

    private enum Field { case title, text }

    private var page: ReadingPage? { pages.pages.first { $0.id == pageID } }
    private var captures: [ReadingCapture] { pages.captures(of: pageID) }
    private var busy: Bool { pages.recognizing == pageID }

    var body: some View {
        Group {
            if page != nil { content } else { gone }
        }
        .background(VoicePalette.background)
        .navigationTitle(title.isEmpty ? "Seite" : title)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar { toolbar }
        .modifier(ReadingPageSources(pickingPhotos: $pickingPhotos, scanning: $scanning, target: (pageID, insertAfter)))
        .onAppear(perform: load)
        .onChange(of: page?.text) { _, new in
            // Fremde Änderung (Sync, Erkennung, Wahl): übernehmen, solange nicht gerade getippt wird.
            if let new, focus != .text, new != text { text = new }
        }
        .onChange(of: page?.title) { _, new in if let new, focus != .title { title = new } }
        .onChange(of: focus) { old, _ in
            if old == .title { pages.rename(pageID, to: title) }
            if old == .text { pages.flushPending(pageID) }
        }
        .onChange(of: scenePhase) { _, phase in if phase != .active { pages.flushPending(pageID) } }
        .onDisappear { pages.flushPending(pageID) }
        .sheet(isPresented: $arranging) { ReadingCapturesArrange(pageID: pageID) }
        .sheet(item: $showing) { capture in ReadingCaptureViewer(pageID: pageID, capture: capture) }
        .confirmationDialog("Seite löschen?", isPresented: $confirmDelete, titleVisibility: .visible) {
            Button("Löschen", role: .destructive) { pages.deletePage(pageID); dismiss() }
            Button("Behalten", role: .cancel) {}
        } message: {
            Text("Die Seite wird auf dem iPhone und auf verbundenen Geräten gelöscht. Die Fotos der Seite werden entfernt.")
        }
        .confirmationDialog("Aufnahme entfernen?", isPresented: Binding(get: { confirmDeleteCapture != nil }, set: { if !$0 { confirmDeleteCapture = nil } }),
                            titleVisibility: .visible, presenting: confirmDeleteCapture) { capture in
            Button("Entfernen", role: .destructive) { pages.deleteCapture(pageID, captureID: capture.id) }
            Button("Behalten", role: .cancel) {}
        } message: { _ in
            Text("Das Foto wird von der Seite entfernt. Bearbeiteten Text ändert die App nicht ohne Nachfrage.")
        }
    }

    private var content: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                TextField("Titel", text: $title, axis: .vertical)
                    .font(.title2.weight(.semibold))
                    .foregroundStyle(VoicePalette.text)
                    .focused($focus, equals: .title)
                    .submitLabel(.done)
                    .onSubmit { focus = nil }
                    .accessibilityIdentifier("pageTitle")

                Button { pages.readAloud(pageID) } label: {
                    Label("Vorlesen", systemImage: "play.fill").frame(maxWidth: .infinity, minHeight: 44)
                }
                .buttonStyle(.borderedProminent)
                .tint(VoicePalette.brand)
                .foregroundStyle(VoicePalette.ink)
                .disabled(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || busy)
                .accessibilityIdentifier("pageRead")

                capturesSection
                textSection
            }
            .padding(16)
        }
        .scrollDismissesKeyboard(.interactively)
    }

    @ViewBuilder private var capturesSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text("AUFNAHMEN").font(.caption.weight(.medium)).tracking(0.8).foregroundStyle(VoicePalette.secondaryText)
                Spacer()
                if captures.count > 1 {
                    Button("Anordnen") { arranging = true }.font(.subheadline)
                        .accessibilityIdentifier("pageArrange")
                }
            }
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 10) {
                    ForEach(Array(captures.enumerated()), id: \.element.id) { index, capture in
                        ReadingCaptureThumb(pageID: pageID, capture: capture, number: index + 1)
                            .onTapGesture { showing = capture }
                            .contextMenu { captureMenu(capture) }
                            .accessibilityAddTraits(.isButton)
                            .accessibilityAction(named: "Neu erkennen") { Task { await pages.rerecognize(pageID, captureID: capture.id) } }
                            .accessibilityAction(named: "Entfernen") { confirmDeleteCapture = capture }
                    }
                    addTile
                }
                .padding(.vertical, 2)
            }
            if busy {
                HStack(spacing: 8) { ProgressView(); Text("Text wird erkannt …") }
                    .font(.subheadline).foregroundStyle(VoicePalette.secondaryText)
                    .accessibilityAddTraits(.updatesFrequently)
            }
        }
    }

    @ViewBuilder private func captureMenu(_ capture: ReadingCapture) -> some View {
        Button("Ansehen", systemImage: "eye") { showing = capture }
        Button("Neu erkennen", systemImage: "text.viewfinder") { Task { await pages.rerecognize(pageID, captureID: capture.id) } }
        Menu("Danach einfügen", systemImage: "text.insert") {
            if DocumentScanner.isSupported {
                Button("Scannen", systemImage: "doc.viewfinder") { insertAfter = capture.id; scanning = true }
            }
            Button("Fotos", systemImage: "photo.on.rectangle") { insertAfter = capture.id; pickingPhotos = true }
        }
        Divider()
        Button("Entfernen", systemImage: "trash", role: .destructive) { confirmDeleteCapture = capture }
    }

    private var addTile: some View {
        Menu {
            if DocumentScanner.isSupported {
                Button("Weiter scannen", systemImage: "doc.viewfinder") { insertAfter = nil; scanning = true }
            }
            Button("Fotos anhängen", systemImage: "photo.on.rectangle") { insertAfter = nil; pickingPhotos = true }
        } label: {
            VStack(spacing: 6) {
                Image(systemName: "plus").font(.title2)
                Text(captures.isEmpty ? "Foto oder Scan" : "Anhängen").font(.caption)
            }
            .frame(width: ReadingCaptureThumb.width, height: ReadingCaptureThumb.height)
            .foregroundStyle(VoicePalette.accent)
            .background(RoundedRectangle(cornerRadius: 10).strokeBorder(VoicePalette.border, style: StrokeStyle(lineWidth: 1.5, dash: [5, 4])))
        }
        .disabled(busy)
        .accessibilityLabel(captures.isEmpty ? "Foto oder Scan hinzufügen" : "Weitere Fotos oder Scans anhängen")
        .accessibilityIdentifier("pageAppend")
    }

    private var textSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("TEXT").font(.caption.weight(.medium)).tracking(0.8).foregroundStyle(VoicePalette.secondaryText)
            TextEditor(text: $text)
                .focused($focus, equals: .text)
                .font(.body)
                .foregroundStyle(VoicePalette.text)
                .scrollContentBackground(.hidden)
                .frame(minHeight: 280)
                .padding(8)
                .background(RoundedRectangle(cornerRadius: 12).fill(Color(.secondarySystemGroupedBackground)))
                .overlay(alignment: .topLeading) {
                    if text.isEmpty {
                        Text("Text eingeben oder Fotos anhängen").foregroundStyle(VoicePalette.secondaryText)
                            .padding(.horizontal, 13).padding(.vertical, 16).allowsHitTesting(false)
                    }
                }
                .onChange(of: text) { _, new in
                    guard loaded, new != page?.text else { return }
                    pages.updateText(pageID, text: new)
                }
                .accessibilityLabel("Text der Seite")
                .accessibilityIdentifier("pageText")
            Text(ReadingPageRow.meta(captures: 0, characters: text.count))
                .font(.caption).foregroundStyle(VoicePalette.secondaryText)
        }
    }

    @ToolbarContentBuilder private var toolbar: some ToolbarContent {
        ToolbarItem(placement: .topBarTrailing) {
            Menu {
                if DocumentScanner.isSupported {
                    Button("Weiter scannen", systemImage: "doc.viewfinder") { insertAfter = nil; scanning = true }
                }
                Button("Fotos anhängen", systemImage: "photo.on.rectangle") { insertAfter = nil; pickingPhotos = true }
                Button("Text aus Zwischenablage anhängen", systemImage: "doc.on.clipboard") { pasteAppend() }
                    .disabled(!UIPasteboard.general.hasStrings)
                Divider()
                ShareLink(item: text, subject: Text(title)) { Label("Text teilen", systemImage: "square.and.arrow.up") }
                Divider()
                Button("Seite löschen", systemImage: "trash", role: .destructive) { confirmDelete = true }
            } label: {
                Image(systemName: "ellipsis.circle").frame(minWidth: 44, minHeight: 44).contentShape(Rectangle())
            }
            .disabled(busy)
            .accessibilityLabel("Weitere Aktionen")
            .accessibilityIdentifier("pageMore")
        }
        ToolbarItem(placement: .keyboard) {
            HStack { Spacer(); Button("Fertig") { focus = nil } }
        }
    }

    private var gone: some View {
        ContentUnavailableView("Seite nicht mehr vorhanden", systemImage: "doc.badge.ellipsis",
                               description: Text("Sie wurde gelöscht, eventuell auf einem anderen Gerät."))
    }

    private func load() {
        guard !loaded, let page else { return }
        title = page.title
        text = page.text
        loaded = true
    }

    private func pasteAppend() {
        guard let pasted = UIPasteboard.general.string, !pasted.isEmpty else { return }
        pages.flushPending(pageID)
        pages.append(text: pasted, to: pageID)
        if let new = page?.text { text = new }
    }
}

// MARK: - Aufnahmen

struct ReadingCaptureThumb: View {
    static let width: CGFloat = 84
    static let height: CGFloat = 112
    let pageID: String
    let capture: ReadingCapture
    let number: Int

    var body: some View {
        ReadingCaptureImage(pageID: pageID, capture: capture, size: CGSize(width: Self.width * 2, height: Self.height * 2))
            .frame(width: Self.width, height: Self.height)
            .clipShape(RoundedRectangle(cornerRadius: 10))
            .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(VoicePalette.border))
            .overlay(alignment: .bottomLeading) {
                Text("\(number)").font(.caption2.weight(.semibold).monospacedDigit())
                    .padding(.horizontal, 6).padding(.vertical, 2)
                    .background(.ultraThinMaterial, in: Capsule())
                    .padding(5)
            }
            .overlay(alignment: .topTrailing) {
                if capture.recognizedText.isEmpty {
                    Image(systemName: "exclamationmark.triangle.fill").font(.caption)
                        .foregroundStyle(VoicePalette.danger).padding(5)
                        .accessibilityHidden(true)
                }
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Aufnahme \(number)" + (capture.recognizedText.isEmpty ? ", kein Text erkannt" : ""))
            .accessibilityIdentifier("pageCapture")
    }
}

/// Seitenbild verkleinert laden; die Originale können groß sein (bis 40 MB).
struct ReadingCaptureImage: View {
    let pageID: String
    let capture: ReadingCapture
    let size: CGSize
    @State private var image: UIImage?

    var body: some View {
        ZStack {
            Color(.tertiarySystemFill)
            if let image {
                Image(uiImage: image).resizable().scaledToFill()
            } else {
                Image(systemName: "photo").foregroundStyle(VoicePalette.secondaryText)
            }
        }
        .task(id: capture.image) {
            guard let url = ReadingPagesModel.shared.imageURL(capture, page: pageID) else { return }
            let target = size
            image = await Task.detached(priority: .userInitiated) {
                UIImage(contentsOfFile: url.path)?.preparingThumbnail(of: target)
            }.value
        }
    }
}

/// Bild und erkannter Text einer Aufnahme; „Neu erkennen“ von hier aus.
struct ReadingCaptureViewer: View {
    let pageID: String
    let capture: ReadingCapture
    @ObservedObject var pages = ReadingPagesModel.shared
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    ReadingCaptureImage(pageID: pageID, capture: capture, size: CGSize(width: 1600, height: 2200))
                        .aspectRatio(3 / 4, contentMode: .fit)
                        .frame(maxWidth: .infinity)
                        .clipShape(RoundedRectangle(cornerRadius: 12))
                    Text("ERKANNTER TEXT").font(.caption.weight(.medium)).tracking(0.8).foregroundStyle(VoicePalette.secondaryText)
                    Text(capture.recognizedText.isEmpty ? "Auf diesem Bild wurde kein Text erkannt." : capture.recognizedText)
                        .foregroundStyle(capture.recognizedText.isEmpty ? VoicePalette.secondaryText : VoicePalette.text)
                        .textSelection(.enabled)
                }
                .padding(16)
            }
            .background(VoicePalette.background)
            .navigationTitle("Aufnahme")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Schließen") { dismiss() } }
                ToolbarItem(placement: .primaryAction) {
                    Button("Neu erkennen", systemImage: "text.viewfinder") {
                        dismiss()
                        Task { await pages.rerecognize(pageID, captureID: capture.id) }
                    }
                    .disabled(pages.recognizing != nil)
                    .accessibilityIdentifier("captureRerecognize")
                }
            }
        }
    }
}

/// Aufnahmen per Ziehen umsortieren oder entfernen.
struct ReadingCapturesArrange: View {
    let pageID: String
    @ObservedObject var pages = ReadingPagesModel.shared
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        let captures = pages.captures(of: pageID)
        NavigationStack {
            List {
                ForEach(Array(captures.enumerated()), id: \.element.id) { index, capture in
                    HStack(spacing: 12) {
                        ReadingCaptureImage(pageID: pageID, capture: capture, size: CGSize(width: 96, height: 128))
                            .frame(width: 48, height: 64).clipShape(RoundedRectangle(cornerRadius: 6))
                        VStack(alignment: .leading, spacing: 2) {
                            Text("Aufnahme \(index + 1)").font(.subheadline.weight(.semibold))
                            Text(capture.recognizedText.isEmpty ? "Kein Text erkannt" : capture.recognizedText)
                                .font(.caption).foregroundStyle(VoicePalette.secondaryText).lineLimit(2)
                        }
                    }
                }
                .onMove { from, to in pages.moveCaptures(pageID, from: from, to: to) }
                .onDelete { offsets in
                    for i in offsets.sorted(by: >) where captures.indices.contains(i) {
                        pages.deleteCapture(pageID, captureID: captures[i].id)
                    }
                }
            }
            .environment(\.editMode, .constant(.active))
            .navigationTitle("Aufnahmen anordnen")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Fertig") { dismiss() } } }
        }
    }
}

// MARK: - Quellen und Rückmeldungen

/// Fotoauswahl und Dokumentkamera für Seiten: ohne `target` entsteht eine neue Seite,
/// sonst werden die Aufnahmen an diese Seite angehängt (am Ende oder nach einer Aufnahme).
struct ReadingPageSources: ViewModifier {
    @Binding var pickingPhotos: Bool
    @Binding var scanning: Bool
    let target: (page: String, after: String?)?
    @ObservedObject var pages = ReadingPagesModel.shared
    @ObservedObject var intake = TextIntakeModel.shared
    @State private var photoItems: [PhotosPickerItem] = []

    func body(content: Content) -> some View {
        content
            .photosPicker(isPresented: $pickingPhotos, selection: $photoItems, maxSelectionCount: TextInbox.maxImages, matching: .images)
            .onChange(of: photoItems) { _, items in
                guard !items.isEmpty else { return }
                photoItems = []
                take(items.map { .photo($0) }, source: "Fotos (Texterkennung)")
            }
            .fullScreenCover(isPresented: $scanning) {
                DocumentScanner { outcome in
                    scanning = false
                    switch outcome {
                    case .cancelled: return
                    case .failed: pages.error = "Der Scan ist fehlgeschlagen. Bitte erneut versuchen."
                    case .scanned(let scan): take((0..<scan.pageCount).map { .scanPage(scan, $0) }, source: "Scan (Texterkennung)")
                    }
                }.ignoresSafeArea()
            }
    }

    private func take(_ sources: [ImageSource], source: String) {
        if let target {
            Task { await pages.append(sources, to: target.page, after: target.after, source: source) }
        } else {
            Task {
                if let page = await pages.createPage(from: sources, source: source) { intake.openPageID = page.id }
            }
        }
    }
}

/// Fehler und die Wahl bei bearbeitetem Text; hängt am Reiter, damit sie auch nach dem Zurückgehen erscheinen.
struct ReadingPagesFeedback: ViewModifier {
    @ObservedObject var pages = ReadingPagesModel.shared

    func body(content: Content) -> some View {
        content
            .alert("Hinweis", isPresented: Binding(get: { pages.error != nil }, set: { if !$0 { pages.error = nil } })) {
                Button("OK", role: .cancel) { pages.error = nil }
            } message: { Text(pages.error ?? "") }
            .confirmationDialog("Text wurde bearbeitet", isPresented: Binding(get: { pages.compositionChoice != nil }, set: { if !$0 { cancelChoice() } }),
                                titleVisibility: .visible, presenting: pages.compositionChoice) { choice in
                Button("Text aus Aufnahmen neu zusammensetzen", role: .destructive) { pages.resolve(choice, with: .replace) }
                if !choice.appendedText.isEmpty {
                    Button("Neuen Text unten anhängen") { pages.resolve(choice, with: .append) }
                }
                Button("Meinen Text behalten", role: .cancel) { pages.resolve(choice, with: .cancel) }
            } message: { _ in
                Text("Du hast den Text der Seite geändert. Beim Neuzusammensetzen gehen diese Änderungen verloren.")
            }
    }

    private func cancelChoice() {
        if let choice = pages.compositionChoice { pages.resolve(choice, with: .cancel) }
    }
}
