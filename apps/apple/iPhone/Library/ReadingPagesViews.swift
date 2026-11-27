import SwiftUI
import PhotosUI
import UIKit

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
    /// Stand der Seite, auf dem der Editor gerade aufsetzt. Weicht der Speicher davon ab, kam die
    /// Änderung von außen (Sync, Erkennung, Wahl); dann gilt der Speicher, nie die alte Eingabe.
    @State private var base = ""
    /// Dasselbe für den Titel.
    @State private var titleBase = ""
    /// Rettung nach dem Löschen höchstens einmal je Seite, auch wenn mehrere Eingaben vor dem Neuzeichnen kommen.
    @State private var rescuedAfterDelete = false
    @State private var loaded = false
    @State private var pickingPhotos = false
    @State private var scanning = false
    @State private var insertAfter: String?
    @State private var arranging = false
    @State private var confirmDelete = false
    @State private var confirmDeleteCapture: ReadingCapture?
    @State private var showing: ReadingCapture?
    @State private var preparing = false
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
            // Fremde Änderung: immer übernehmen, auch während getippt wird. Jede Eingabe geht synchron über
            // `updateText` in den Puffer des Modells (auch markierter IME-/Diktattext, den TextEditor laufend
            // meldet); vor einem Sync-Abgleich sichert `prepareForRemote` ihn, und der Store legt abweichende
            // Stände (auch vor einem Grabstein) als Konfliktkopie ab. Lokale Erkennung setzt erst nach
            // `flushText` und nie still über bearbeiteten Text. Hier geht also nichts verloren.
            guard let new else { return }
            if new != text { text = new }
            base = new
        }
        .onChange(of: page?.title) { _, new in
            // Eigene Eingaben sind über den Setter schon gespeichert (dann gilt new == titleBase).
            guard let new, new != titleBase else { return }
            title = new
            titleBase = new
        }
        .onChange(of: focus) { old, _ in
            if old == .title { saveTitle() }
            if old == .text { pages.flushPending(pageID) }
        }
        .onChange(of: scenePhase) { _, phase in
            guard phase != .active else { return }
            pages.flushPending(pageID)
        }
        .onDisappear { pages.flushPending(pageID) }
        .sheet(isPresented: $arranging) { ReadingCapturesArrange(pageID: pageID) }
        .sheet(item: $showing) { capture in ReadingCaptureViewer(pageID: pageID, capture: capture) }
        .sheet(isPresented: $preparing) { ReadingTextPrepareSheet(pageID: pageID) }
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
            Text("Das Foto wird von der Seite entfernt und lässt sich in der App nicht wiederherstellen. Bearbeiteten Text ändert die App nicht ohne Nachfrage.")
        }
    }

    private var content: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                TextField("Titel", text: titleBinding, axis: .vertical)
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
                    .lineLimit(1).dynamicTypeSize(...DynamicTypeSize.xxxLarge)
                Spacer()
                if captures.count > 1 {
                    Button("Anordnen") { arranging = true }.font(.subheadline).lineLimit(1).fixedSize()
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
                    // Mit Aufnahmen liegt das Anhängen oben im Plus-Menü; die Kachel bleibt nur als Einstieg.
                    if captures.isEmpty { addTile }
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
                    .multilineTextAlignment(.center).lineLimit(2).minimumScaleFactor(0.6)
                    .dynamicTypeSize(...DynamicTypeSize.xxxLarge)
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
            HStack {
                Text("TEXT").font(.caption.weight(.medium)).tracking(0.8).foregroundStyle(VoicePalette.secondaryText)
                    .lineLimit(1).dynamicTypeSize(...DynamicTypeSize.xxxLarge)
                Spacer()
                Button { focus = nil; preparing = true } label: {
                    ViewThatFits(in: .horizontal) {
                        Label("Text aufbereiten", systemImage: "wand.and.stars").lineLimit(1)
                        Image(systemName: "wand.and.stars")
                    }
                }
                .font(.subheadline)
                .accessibilityLabel("Text aufbereiten")
                .disabled(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || busy)
                .accessibilityIdentifier("pagePrepare")
            }
            TextEditor(text: textBinding)
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
                .accessibilityLabel("Text der Seite")
                .accessibilityIdentifier("pageText")
            Text(ReadingPageRow.meta(captures: 0, characters: text.count))
                .font(.caption).foregroundStyle(VoicePalette.secondaryText)
        }
    }

    @ToolbarContentBuilder private var toolbar: some ToolbarContent {
        // Hinzufügen steht oben wie in der Bibliothek (Patrick 26.11.2026): Scans, Fotos und Text anhängen.
        ToolbarItem(placement: .topBarTrailing) {
            Menu {
                if DocumentScanner.isSupported {
                    Button("Weiter scannen", systemImage: "doc.viewfinder") { insertAfter = nil; scanning = true }
                }
                Button("Fotos anhängen", systemImage: "photo.on.rectangle") { insertAfter = nil; pickingPhotos = true }
                Button("Text aus Zwischenablage anhängen", systemImage: "doc.on.clipboard") { pasteAppend() }
                    .disabled(!UIPasteboard.general.hasStrings)
            } label: {
                Image(systemName: "plus").frame(minWidth: 44, minHeight: 44).contentShape(Rectangle())
            }
            .disabled(busy)
            .accessibilityLabel("Scan, Fotos oder Text anhängen")
            .accessibilityIdentifier("pageAdd")
        }
        ToolbarItem(placement: .topBarTrailing) {
            Menu {
                Button("Text aufbereiten", systemImage: "wand.and.stars") { focus = nil; preparing = true }
                    .disabled(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
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
        base = page.text
        titleBase = page.title
        loaded = true
    }

    /// Jede Eingabe geht im Setter, also im selben Schritt wie der Tastendruck, an das Modell. Zwischen
    /// Eingabe und Modell liegt kein Update-Zyklus, in dem ein Pull oder Grabstein sie überholen könnte.
    private var textBinding: Binding<String> {
        Binding(get: { text }, set: { new in
            guard loaded, new != text else { return }
            guard let current = page?.text else {
                // Seite inzwischen gelöscht: Eingabe nicht verlieren.
                text = new
                rescue(text: new, title: title, deleted: true)
                return
            }
            guard current == base else {
                // Der Speicher hat sich unter dem Editor geändert: nicht überschreiben, sondern sichern.
                rescue(text: new, title: nil, deleted: false)
                text = current
                base = current
                return
            }
            text = new
            pages.updateText(pageID, text: new)
            base = page?.text ?? new
        })
    }

    private func rescue(text input: String, title typed: String?, deleted: Bool) {
        if deleted {
            guard !rescuedAfterDelete else { return }
            rescuedAfterDelete = true
        }
        let copy = pages.saveConflictCopy(of: pageID, text: input, title: typed)
        let reason = deleted ? "Die Seite wurde inzwischen gelöscht (z. B. am PC)." : "Der Text wurde inzwischen an anderer Stelle geändert (z. B. am PC)."
        if let copy {
            pages.error = reason + " Deine Eingabe ist als eigene Seite „\(copy.title)“ gesichert."
        } else if pages.error == nil {
            pages.error = reason + " Deine Eingabe konnte nicht gesichert werden."
        }
    }

    /// Titel ebenso im Setter speichern. Gespeichert wird der bereinigte Titel; das Feld behält die Eingabe
    /// (z. B. Leerzeichen am Ende), bis der Fokus wechselt.
    private var titleBinding: Binding<String> {
        Binding(get: { title }, set: { new in
            guard loaded, new != title else { return }
            guard let page else {
                // Seite inzwischen gelöscht: Titel und Text als eigene Seite sichern.
                title = new
                rescue(text: text, title: new, deleted: true)
                return
            }
            guard page.title == titleBase else {
                pages.error = "Der Titel wurde inzwischen an anderer Stelle geändert. Es gilt „\(page.title)“; deine Eingabe „\(new)“ wurde nicht übernommen."
                title = page.title
                titleBase = page.title
                return
            }
            title = new
            pages.updateTitle(pageID, title: new)
            titleBase = self.page?.title ?? titleBase
        })
    }

    /// Nach dem Bearbeiten den gespeicherten (bereinigten) Titel anzeigen.
    private func saveTitle() {
        if let page { title = page.title; titleBase = page.title }
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
    @State private var removing: ReadingCapture?

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
                    // Einzeln und nur nach Rückfrage: Das Foto lässt sich in der App nicht wiederherstellen.
                    if let i = offsets.first, captures.indices.contains(i) { removing = captures[i] }
                }
            }
            .environment(\.editMode, .constant(.active))
            .confirmationDialog("Aufnahme entfernen?", isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }),
                                titleVisibility: .visible, presenting: removing) { capture in
                Button("Entfernen", role: .destructive) { pages.deleteCapture(pageID, captureID: capture.id) }
                Button("Behalten", role: .cancel) {}
            } message: { _ in
                Text("Das Foto wird von der Seite entfernt und lässt sich in der App nicht wiederherstellen. Bearbeiteten Text ändert die App nicht ohne Nachfrage.")
            }
            .navigationTitle("Aufnahmen anordnen")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Fertig") { dismiss() } } }
            .modifier(ReadingPagesFeedback(nested: true))
        }
        .onAppear { ReadingPagesPresentation.shared.arranging = true }
        .onDisappear { ReadingPagesPresentation.shared.arranging = false }
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
    @ObservedObject var presentation = ReadingPagesPresentation.shared
    /// Im Blatt „Aufnahmen anordnen“ zeigt dieses Blatt die Wahl selbst (über einem Blatt kann der Reiter nichts zeigen).
    var nested = false

    func body(content: Content) -> some View {
        content
            .alert("Hinweis", isPresented: Binding(get: { presentation.arranging == nested && pages.error != nil }, set: { if !$0 { pages.error = nil } })) {
                Button("OK", role: .cancel) { pages.error = nil }
            } message: { Text(pages.error ?? "") }
            // Eigenes Blatt statt Bestätigungsdialog: iOS blendet dort die Abbrechen-Rolle teils aus, und bei
            // großer Schrift passen drei lange Knöpfe nicht. Wischen schließt = Text behalten.
            .sheet(item: Binding(get: { presentation.arranging == nested ? pages.compositionChoice : nil },
                                 set: { if $0 == nil { cancelChoice() } })) { choice in
                ReadingCompositionChoiceSheet(choice: choice)
            }
    }

    private func cancelChoice() {
        if let choice = pages.compositionChoice { pages.resolve(choice, with: .cancel) }
    }
}

/// Welche Ebene Meldungen zeigt: der Reiter oder das gerade offene Blatt „Aufnahmen anordnen“.
@MainActor
final class ReadingPagesPresentation: ObservableObject {
    static let shared = ReadingPagesPresentation()
    @Published var arranging = false
}

/// Wahl, wenn Aufnahmen geändert wurden, der Seitentext aber schon bearbeitet war. Alle drei Wege sind
/// gleich sichtbar; bei großer Schrift scrollt das Blatt.
struct ReadingCompositionChoiceSheet: View {
    let choice: CompositionChoice
    @ObservedObject var pages = ReadingPagesModel.shared
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    Text("Du hast den Text dieser Seite selbst geändert. Die Änderung an den Aufnahmen ist schon gespeichert. Wie soll der Text weitergehen?")
                        .foregroundStyle(VoicePalette.text)
                        .fixedSize(horizontal: false, vertical: true)
                    option("Meinen Text behalten", detail: "Der Text bleibt, wie er ist.", systemImage: "checkmark.circle",
                           prominent: true, id: "choiceKeep") { pages.resolve(choice, with: .cancel) }
                    if !choice.appendedText.isEmpty {
                        option("Neuen Text unten anhängen", detail: "Der erkannte Text kommt unter deinen Text.", systemImage: "text.append",
                               prominent: false, id: "choiceAppend") { pages.resolve(choice, with: .append) }
                    }
                    option("Text neu aus Aufnahmen bilden", detail: "Deine Änderungen am Text gehen verloren.", systemImage: "arrow.triangle.2.circlepath",
                           prominent: false, destructive: true, id: "choiceReplace") { pages.resolve(choice, with: .replace) }
                }
                .padding(16)
            }
            .background(VoicePalette.background)
            .navigationTitle("Text wurde bearbeitet")
            .navigationBarTitleDisplayMode(.inline)
        }
        // Bei großer Schrift gleich ganz hoch, damit alle Wege ohne Ziehen erreichbar sind.
        .presentationDetents(typeSize.isAccessibilitySize ? [.large] : [.medium, .large])
    }

    private func option(_ title: String, detail: String, systemImage: String, prominent: Bool, destructive: Bool = false,
                        id: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Image(systemName: systemImage).accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 2) {
                    Text(title).font(.body.weight(.semibold))
                    Text(detail).font(.subheadline).opacity(0.8)
                }
                .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
            .padding(12)
            .foregroundStyle(destructive ? VoicePalette.danger : (prominent ? VoicePalette.ink : VoicePalette.text))
            .background(RoundedRectangle(cornerRadius: 12).fill(prominent ? VoicePalette.brand : Color(.secondarySystemGroupedBackground)))
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier(id)
    }
}

// MARK: - Text aufbereiten (lva#786 S3)

/// Regelvorschläge wie am Desktop: jeder einzeln an- oder abwählbar, übernommen wird nur auf dem
/// Stand, aus dem die Vorschläge entstanden sind. Alles läuft auf dem iPhone, ohne KI.
struct ReadingTextPrepareSheet: View {
    let pageID: String
    @ObservedObject var pages = ReadingPagesModel.shared
    @Environment(\.dismiss) private var dismiss
    @State private var stufe: TextAufbereiten.Stufe = .bereinigung
    @State private var vorschlaege: TextAufbereiten.Vorschlaege?
    @State private var angenommen: Set<String> = []
    @State private var laedt = false
    @State private var uebernimmt = false
    @State private var hinweis: String?
    /// Nur das Ergebnis des jüngsten Ladeauftrags zählt (Stufenwechsel, erneutes Laden, Verlassen).
    @State private var auftrag = 0

    var body: some View {
        NavigationStack {
            List {
                Section {
                    Picker("Stufe", selection: $stufe) {
                        Text("Bereinigen").tag(TextAufbereiten.Stufe.bereinigung)
                        Text("Schreibweise").tag(TextAufbereiten.Stufe.format)
                    }
                    .pickerStyle(.segmented)
                    .accessibilityIdentifier("prepareStage")
                } footer: {
                    Text(stufe == .bereinigung
                         ? "Entfernt Seitenzahlen, wiederholte Kopf- und Fußzeilen, Trennungen am Zeilenende, überflüssige Leerzeichen und Zeichenreste."
                         : "Vereinheitlicht Anführungszeichen, Gedankenstriche, Auslassungspunkte, Ligaturen und Listen.")
                }
                if let hinweis {
                    Section { Label(hinweis, systemImage: "exclamationmark.triangle").foregroundStyle(VoicePalette.danger) }
                }
                if laedt {
                    Section { HStack(spacing: 8) { ProgressView(); Text("Vorschläge werden erstellt …") } }
                } else if let v = vorschlaege {
                    if v.aenderungen.isEmpty {
                        Section {
                            Label("Keine Vorschläge. Der Text ist in dieser Stufe schon sauber.", systemImage: "checkmark.circle")
                                .foregroundStyle(VoicePalette.secondaryText)
                        }
                    } else {
                        Section {
                            ForEach(v.aenderungen) { a in
                                // Zusammengehörige Änderungen (z. B. öffnendes und schließendes Anführungszeichen)
                                // nur gemeinsam an- oder abwählen; sonst wäre die Auswahl nicht anwendbar.
                                ReadingPrepareRow(aenderung: a, gruppiert: a.gruppe != nil, an: Binding(
                                    get: { angenommen.contains(a.id) },
                                    set: { an in
                                        let ids = a.gruppe.map { g in v.aenderungen.filter { $0.gruppe == g }.map(\.id) } ?? [a.id]
                                        if an { angenommen.formUnion(ids) } else { angenommen.subtract(ids) }
                                    }))
                            }
                        } header: {
                            HStack {
                                Text("\(angenommen.count) von \(v.aenderungen.count) ausgewählt")
                                Spacer()
                                Button(angenommen.count == v.aenderungen.count ? "Keine" : "Alle") {
                                    angenommen = angenommen.count == v.aenderungen.count ? [] : Set(v.aenderungen.map(\.id))
                                }
                                .textCase(nil)
                                .accessibilityIdentifier("prepareToggleAll")
                            }
                        }
                    }
                }
            }
            .navigationTitle("Text aufbereiten")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    // Während der Übernahme nicht schließbar: Sie schreibt sonst nach dem Schließen weiter.
                    Button("Abbrechen") { auftrag += 1; dismiss() }.disabled(uebernimmt)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(angenommen.isEmpty ? "Übernehmen" : "Übernehmen (\(angenommen.count))") { Task { await uebernehmen() } }
                        .disabled(angenommen.isEmpty || laedt || uebernimmt)
                        .accessibilityIdentifier("prepareApply")
                }
            }
            .task(id: stufe) { await laden() }
            .onDisappear { auftrag += 1 }
        }
        .interactiveDismissDisabled(uebernimmt)
    }

    private func laden() async {
        auftrag += 1
        let meiner = auftrag
        laedt = true
        hinweis = nil
        vorschlaege = nil
        angenommen = []
        do {
            let v = try await pages.aufbereitenVorschlaege(pageID: pageID, stufe: stufe)
            guard meiner == auftrag, !Task.isCancelled else { return }
            vorschlaege = v
            angenommen = Set(v.aenderungen.map(\.id))
        } catch {
            guard meiner == auftrag, !Task.isCancelled else { return }
            hinweis = pages.error ?? "Die Vorschläge konnten nicht erstellt werden."
            pages.error = nil
        }
        laedt = false
    }

    private func uebernehmen() async {
        guard let v = vorschlaege, !laedt else { return }
        uebernimmt = true
        defer { uebernimmt = false }
        do {
            try await pages.uebernehmen(pageID: pageID, angenommen: v.auswahl(angenommen: angenommen))
            dismiss()
        } catch {
            // Meldung hier im Blatt zeigen (über dem Blatt kann der Reiter nichts anzeigen) und neu vorschlagen.
            hinweis = pages.error ?? "Die Änderungen konnten nicht übernommen werden."
            pages.error = nil
            let keep = hinweis
            await laden()
            hinweis = keep
        }
    }
}

struct ReadingPrepareRow: View {
    let aenderung: TextAufbereiten.Aenderung
    let gruppiert: Bool
    @Binding var an: Bool
    @State private var ganz = false

    /// Lang genug, dass drei Zeilen nicht alles zeigen: dann aufklappbar, damit nichts Unsichtbares übernommen wird.
    private var lang: Bool {
        let n = max(aenderung.alt.count, aenderung.neu.count)
        return n > 90 || aenderung.alt.filter(\.isNewline).count > 2 || aenderung.neu.filter(\.isNewline).count > 2
    }

    var body: some View {
        Toggle(isOn: $an) {
            VStack(alignment: .leading, spacing: 4) {
                Text(Self.regelName(aenderung.regel) + (gruppiert ? " · gehört zusammen" : ""))
                    .font(.caption.weight(.semibold)).foregroundStyle(VoicePalette.secondaryText)
                Text(Self.sichtbar(aenderung.alt)).strikethrough().foregroundStyle(VoicePalette.danger).lineLimit(ganz ? nil : 3)
                Text(aenderung.neu.isEmpty ? "(entfernt)" : Self.sichtbar(aenderung.neu))
                    .foregroundStyle(aenderung.neu.isEmpty ? VoicePalette.secondaryText : VoicePalette.success).lineLimit(ganz ? nil : 3)
                if lang {
                    Button(ganz ? "Weniger anzeigen" : "Ganz anzeigen") { ganz.toggle() }
                        .font(.caption).buttonStyle(.borderless)
                        .accessibilityIdentifier("prepareExpand")
                }
            }
            .font(.callout.monospaced())
        }
        .tint(VoicePalette.accent)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(Self.regelName(aenderung.regel)): aus „\(aenderung.alt)“ wird „\(aenderung.neu)“")
        .accessibilityIdentifier("prepareChange")
    }

    /// Leerraum sichtbar machen, sonst sieht „Leerzeile entfernen“ wie nichts aus.
    static func sichtbar(_ s: String) -> String {
        s.replacingOccurrences(of: "\n", with: "⏎")
         .replacingOccurrences(of: "\t", with: "⇥")
         .replacingOccurrences(of: "\u{00AD}", with: "‐")
         .replacingOccurrences(of: " ", with: "·")
    }

    static func regelName(_ regel: String) -> String {
        let namen = [
            "repeated": "Kopf-/Fußzeile", "control": "Steuerzeichen", "pageNumbers": "Seitenzahl", "webNoise": "Webreste",
            "fragments": "Bruchstück", "spaces": "Leerzeichen", "blankLines": "Leerzeilen", "nfc": "Zeichenform",
            "ligaturen": "Ligatur", "weiche_trennstriche": "Trennung", "anfuehrungszeichen": "Anführungszeichen",
            "gedankenstrich": "Gedankenstrich", "auslassung": "Auslassungspunkte", "leerzeichen": "Leerzeichen",
            "listen": "Liste", "format": "Schreibweise",
        ]
        let teile = regel.split(separator: "+").map { namen[String($0)] ?? String($0) }
        return teile.isEmpty ? "Änderung" : teile.joined(separator: ", ")
    }
}
