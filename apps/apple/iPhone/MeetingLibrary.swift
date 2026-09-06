import SwiftUI
import AVFoundation
import UniformTypeIdentifiers

@MainActor
final class MeetingLibrary: ObservableObject {
    @Published private(set) var documents: [MeetingDocument] = []
    @Published private(set) var status = "Audio oder Video importieren"
    @Published private(set) var running: UUID?
    @Published private(set) var importBusy = false
    @Published var error: String?
    private var archive: MeetingArchive?
    private var worker: Task<Void, Never>?
    private var token: InferenceCancellation?
    private var exporter: AVAssetExportSession?
    private var failed = Set<UUID>()
    private var active = false

    init() {
        do {
            let root = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("Meetings")
            archive = try MeetingArchive(root: root)
        } catch { self.error = "Aufnahmespeicher konnte nicht geöffnet werden." }
    }
    func scene(active: Bool) {
        self.active = active
        if active { Task { await refresh(); start() } }
        else {
            worker?.cancel(); token?.cancel(); exporter?.cancelExport()
            if running != nil { status = "gespeichert – Verarbeitung folgt" }
        }
    }
    func refresh() async {
        do { documents = try await archive?.list() ?? [] }
        catch { self.error = "Der Aufnahmespeicher konnte nicht vollständig gelesen werden. Originaldateien bleiben erhalten." }
    }
    func importMedia(_ url: URL) async {
        guard !importBusy, let archive else { return }
        importBusy = true; defer { importBusy = false }
        error = nil
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        let startTime = ContinuousClock.now
        do {
            let asset = AVURLAsset(url: url)
            guard !(try await asset.loadTracks(withMediaType: .audio)).isEmpty else { throw VoiceError.invalid }
            let duration = try await asset.load(.duration).seconds
            let doc = try await archive.importMedia(url, duration: duration)
            status = "gespeichert – Verarbeitung folgt"
            try await archive.recordTiming(doc.id, phase: "import", milliseconds: elapsed(startTime))
            await refresh(); start()
        } catch {
            self.error = "Import nicht abgeschlossen. Bitte eine lesbare Audio-/Videodatei bis 2 Stunden und 2 GB wählen und freien Speicher prüfen."
        }
    }
    #if DEBUG
    func importProbe() async {
        let args = ProcessInfo.processInfo.arguments
        guard let index = args.firstIndex(of: "--meeting-import-probe"), args.indices.contains(index + 1) else { return }
        let name = args[index + 1]
        guard name == URL(fileURLWithPath: name).lastPathComponent else { return }
        await refresh()
        if documents.contains(where: { $0.originalName == name }) { return }
        let url = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("MeetingFixtures").appendingPathComponent(name)
        await importMedia(url)
    }
    #endif
    func retry() { failed.removeAll(); error = nil; start() }
    func original(_ id: UUID) async throws -> URL {
        guard let archive else { throw VoiceError.missing }
        return try await archive.originalURL(id)
    }
    private func start() {
        guard active, worker == nil, let archive,
              let doc = documents.first(where: { $0.minutes == nil && !failed.contains($0.id) }) else { return }
        worker = Task {
            let cancellation = InferenceCancellation(); token = cancellation; running = doc.id
            do { try await process(doc, archive: archive, cancellation: cancellation) }
            catch {
                if !Task.isCancelled {
                    failed.insert(doc.id)
                    self.error = "Die lokale Verarbeitung ist noch nicht abgeschlossen. Original und Fortschritt sind gespeichert. Lokale Modelle prüfen und erneut versuchen."
                }
                status = "gespeichert – Verarbeitung folgt"
            }
            token = nil; exporter = nil; running = nil; worker = nil
            await refresh(); start()
        }
    }
    private func process(_ original: MeetingDocument, archive: MeetingArchive, cancellation: InferenceCancellation) async throws {
        var doc = original
        if doc.nextOffset < doc.duration {
            let source = try await archive.originalURL(doc.id)
            let working = FileManager.default.temporaryDirectory.appendingPathComponent("meeting-" + doc.id.uuidString + ".m4a")
            // A cancelled conversion is never reused as a complete audio file.
            try? FileManager.default.removeItem(at: working)
            defer { try? FileManager.default.removeItem(at: working) }
            guard let session = AVAssetExportSession(asset: AVURLAsset(url: source), presetName: AVAssetExportPresetAppleM4A) else { throw VoiceError.invalid }
            exporter = session; status = "Audiospur vorbereiten"
            let audioStart = ContinuousClock.now
            try await session.export(to: working, as: .m4a)
            try Task.checkCancellation()
            try await archive.recordTiming(doc.id, phase: "audio", milliseconds: elapsed(audioStart))
            exporter = nil
            while doc.nextOffset < doc.duration {
                try Task.checkCancellation()
                let startTime = ContinuousClock.now
                let end = min(doc.nextOffset + 30, doc.duration)
                status = "Lokal transkribieren · \(Int(doc.nextOffset / doc.duration * 100)) %"
                let segments = try await CPULocalProviders.shared.transcribeSlice(working, offset: doc.nextOffset, duration: end - doc.nextOffset, cancellation: cancellation)
                try Task.checkCancellation()
                let shifted = segments.map { MeetingSegment(index: $0.index, start: min(end, doc.nextOffset + $0.start), end: min(end, doc.nextOffset + $0.end), text: $0.text) }
                try await archive.appendChunk(doc.id, offset: doc.nextOffset, nextOffset: end, segments: shifted)
                try await archive.recordTiming(doc.id, phase: "stt", milliseconds: elapsed(startTime))
                doc = try await archive.load(doc.id); await refresh()
            }
        }
        while (doc.summarizedSegments ?? 0) < doc.segments.count {
            try Task.checkCancellation()
            let from = doc.summarizedSegments ?? 0
            var through = from, text = ""
            while through < doc.segments.count {
                let next = doc.segments[through].text
                if text.count + next.count + 1 > 2400 { break }
                text += next + "\n"; through += 1
            }
            guard through > from else { throw VoiceError.invalid }
            status = "Lokal auswerten · Abschnitt \((doc.summaryParts?.count ?? 0) + 1)"
            let startTime = ContinuousClock.now
            let minutes = try await CPULocalProviders.shared.minutes(for: text, cancellation: cancellation)
            try Task.checkCancellation()
            try await archive.appendMinutes(doc.id, from: from, through: through, minutes: minutes)
            try await archive.recordTiming(doc.id, phase: "minutes", milliseconds: elapsed(startTime))
            doc = try await archive.load(doc.id); await refresh()
        }
        try await archive.finishMinutes(doc.id)
        status = "Transkript und Auswertung gespeichert"
    }
    private func elapsed(_ start: ContinuousClock.Instant) -> Double {
        let duration = start.duration(to: .now).components
        return Double(duration.seconds) * 1000 + Double(duration.attoseconds) / 1e15
    }
}

private struct MeetingSection<Content: View>: View {
    let title: String
    let content: Content
    init(_ title: String, @ViewBuilder content: () -> Content) { self.title = title; self.content = content() }
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(title).font(.headline)
            content
        }.voiceCard()
    }
}

struct MeetingLibraryView: View {
    @StateObject private var library = MeetingLibrary()
    @Environment(\.scenePhase) private var phase
    @State private var importer = false
    var body: some View {
        NavigationStack {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 12) {
                    VStack(alignment: .leading, spacing: 10) {
                        Button { importer = true } label: {
                            Label("Audio oder Video importieren", systemImage: "square.and.arrow.down")
                                .frame(maxWidth: .infinity, minHeight: 44).foregroundStyle(VoicePalette.ink)
                        }.buttonStyle(VoicePrimaryButtonStyle(recording: false))
                            .disabled(library.importBusy).opacity(library.importBusy ? 0.6 : 1)
                            .accessibilityIdentifier("importMeeting")
                        if library.importBusy { ProgressView("Original sichern") }
                        Text(library.status).font(.subheadline).foregroundStyle(VoicePalette.secondaryText)
                        if let error = library.error {
                            Text(error).font(.subheadline)
                            Button("Erneut versuchen", systemImage: "arrow.clockwise") { library.retry() }.frame(minHeight: 44)
                        }
                    }.voiceCard()
                    Text("Anrufaufzeichnungen aus Dateien importieren. Kein Live-Zugriff auf Telefon-, Teams- oder WhatsApp-Anrufe.")
                        .font(.caption).foregroundStyle(VoicePalette.secondaryText)
                    Text("Gespeicherte Aufzeichnungen").font(.headline)
                    if library.documents.isEmpty { Text("Noch keine Aufzeichnung").foregroundStyle(VoicePalette.secondaryText) }
                    ForEach(library.documents) { doc in
                        NavigationLink { MeetingDetailView(library: library, original: doc) } label: {
                            HStack {
                                VStack(alignment: .leading, spacing: 5) {
                                    Text(doc.originalName).lineLimit(2)
                                    Text(doc.minutes != nil ? "Transkript & Auswertung" : "gespeichert – Verarbeitung folgt")
                                        .font(.caption).foregroundStyle(VoicePalette.secondaryText)
                                }
                                Spacer()
                                Image(systemName: "chevron.right").font(.caption).foregroundStyle(VoicePalette.secondaryText)
                            }.voiceCard()
                        }.buttonStyle(.plain).accessibilityIdentifier("meetingEntry")
                    }
                }.padding(16)
            }
            .background(VoicePalette.background)
            .navigationTitle("Aufzeichnungen").navigationBarTitleDisplayMode(.inline)
            .fileImporter(isPresented: $importer, allowedContentTypes: [.audio, .movie], allowsMultipleSelection: false) { result in
                switch result {
                case .success(let urls): if let first = urls.first { Task { await library.importMedia(first) } }
                case .failure: library.error = "Datei konnte nicht geöffnet werden."
                }
            }
        }
        .onChange(of: phase, initial: true) { _, value in library.scene(active: value == .active) }
        #if DEBUG
        .task { await library.importProbe() }
        #endif
    }
}

private struct MeetingDetailView: View {
    @ObservedObject var library: MeetingLibrary
    let original: MeetingDocument
    @State private var files: [URL] = []
    @State private var originalURL: URL?
    @State private var copied = false
    @State private var exportError = false
    @State private var tab = 0
    private var doc: MeetingDocument { library.documents.first(where: { $0.id == original.id }) ?? original }
    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 12) {
                VStack(alignment: .leading, spacing: 8) {
                    Text(doc.originalName).font(.headline).lineLimit(2)
                    HStack {
                        Button(copied ? "Kopiert" : "Kopieren", systemImage: copied ? "checkmark" : "doc.on.doc") {
                            let text = MeetingExport.text(doc)
                            UIPasteboard.general.items = [[UTType.utf8PlainText.identifier: text, UTType.html.identifier: Data(MeetingExport.html(text).utf8)]]
                            copied = true
                        }.frame(minHeight: 44)
                        Spacer()
                        Menu("Exportieren", systemImage: "square.and.arrow.up") {
                            ForEach(files, id: \.self) { file in ShareLink(file.pathExtension.uppercased(), item: file) }
                            if let originalURL { ShareLink("Originaldatei", item: originalURL) }
                        }.disabled(files.isEmpty).frame(minHeight: 44)
                    }.buttonStyle(.borderless)
                    if exportError { Text("Export konnte nicht vorbereitet werden.").font(.caption).foregroundStyle(VoicePalette.secondaryText) }
                }.voiceCard()
                Picker("Ergebnisansicht", selection: $tab) {
                    Text("Auswertung").tag(0)
                    Text("Transkript").tag(1)
                }.pickerStyle(.segmented)
                if tab == 0 { analysis }
                else {
                    if doc.segments.isEmpty { Text("gespeichert – Verarbeitung folgt").foregroundStyle(VoicePalette.secondaryText) }
                    ForEach(doc.segments, id: \.index) { item in
                        VStack(alignment: .leading, spacing: 4) {
                            Text(Duration.seconds(item.start).formatted(.time(pattern: .minuteSecond))).font(.caption).foregroundStyle(VoicePalette.secondaryText)
                            Text(item.text).textSelection(.enabled)
                        }.voiceCard()
                    }
                    Text("Keine belegte Sprecherzuordnung: Für gemischte Aufnahmen werden keine Redeanteile geschätzt.").font(.caption).foregroundStyle(VoicePalette.secondaryText)
                }
                DisclosureGroup("Verarbeitungszeiten") {
                    ForEach([("import", "Import"), ("audio", "Audiospur"), ("stt", "Transkription"), ("minutes", "Auswertung")], id: \.0) { key, label in
                        LabeledContent(label, value: String(format: "%.1f s", (doc.timings?[key] ?? 0) / 1000))
                    }.font(.caption)
                }.font(.subheadline).voiceCard()
            }.padding(16)
        }
        .background(VoicePalette.background)
        .navigationTitle("Ergebnisse").navigationBarTitleDisplayMode(.inline)
        .task(id: "\(doc.nextOffset):\(doc.summarizedSegments ?? 0):\(doc.minutes != nil)") { await prepareExports() }
    }
    @ViewBuilder private var analysis: some View {
        if let report = doc.minutes {
            MeetingSection("Zusammenfassung") { Text(report.summary.isEmpty ? "Keine Sprache erkannt." : report.summary).textSelection(.enabled) }
            if !report.scope.isEmpty && !report.summary.contains(report.scope) {
                MeetingSection("Kontext") { Text(report.scope).textSelection(.enabled) }
            }
            if !report.decisions.isEmpty {
                MeetingSection("Entscheidungen") { ForEach(Array(report.decisions.enumerated()), id: \.offset) { _, item in Text(item.text + (item.context.isEmpty ? "" : "\n" + item.context)).textSelection(.enabled) } }
            }
            if !report.tasks.isEmpty {
                MeetingSection("Aufgaben") { ForEach(Array(report.tasks.enumerated()), id: \.offset) { _, item in
                    VStack(alignment: .leading, spacing: 4) {
                        Text(item.text).textSelection(.enabled)
                        Text("\(item.assignee ?? "Zuständig: offen") · \(item.due ?? "Termin: offen")").font(.caption).foregroundStyle(VoicePalette.secondaryText)
                    }
                } }
            }
            if !report.next_steps.isEmpty {
                MeetingSection("Nächste Schritte") { ForEach(Array(report.next_steps.enumerated()), id: \.offset) { _, item in Text(item.text + (item.owner.map { " · " + $0 } ?? "")).textSelection(.enabled) } }
            }
            if !report.follow_ups.isEmpty {
                MeetingSection("Handlungsempfehlungen") { ForEach(Array(report.follow_ups.enumerated()), id: \.offset) { _, item in
                    VStack(alignment: .leading, spacing: 4) {
                        Text(item.text).textSelection(.enabled)
                        Text("Bezug: " + item.reason).font(.caption).foregroundStyle(VoicePalette.secondaryText).textSelection(.enabled)
                    }
                } }
            }
            if !report.open_questions.isEmpty {
                MeetingSection("Offene Fragen") { ForEach(Array(report.open_questions.enumerated()), id: \.offset) { _, item in Text(item.text).textSelection(.enabled) } }
            }
            Text("Lokal aus Originalstellen erstellt. Handlungsempfehlungen sind Vorschläge. Das Transkript bleibt zur Prüfung erhalten.").font(.caption).foregroundStyle(VoicePalette.secondaryText)
        } else { Text("gespeichert – Verarbeitung folgt").foregroundStyle(VoicePalette.secondaryText).voiceCard() }
    }
    private func prepareExports() async {
        copied = false; files = []; exportError = false
        do {
            let folder = FileManager.default.temporaryDirectory.appendingPathComponent("export-" + doc.id.uuidString)
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            let text = MeetingExport.text(doc)
            let values: [(String, Data)] = [("txt", Data(text.utf8)), ("html", Data(MeetingExport.html(text).utf8)), ("srt", Data(MeetingExport.srt(doc.segments).utf8)), ("json", try JSONEncoder().encode(doc))]
            var result: [URL] = []
            for (ext, data) in values {
                let url = folder.appendingPathComponent("Ergebnisse." + ext)
                try data.write(to: url, options: .atomic); result.append(url)
            }
            let original = try await library.original(doc.id)
            try Task.checkCancellation()
            originalURL = original; files = result; exportError = false
        } catch { if !Task.isCancelled { exportError = true } }
    }
}
