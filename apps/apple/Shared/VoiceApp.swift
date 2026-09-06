import SwiftUI
#if os(iOS)
import UIKit
#else
import WatchKit
#endif

// Native adaptation of apps/local-voice/src/styles/theme.css (WAI).
// Yellow marks an action on an ink carrier; never yellow text on a light surface.
enum VoicePalette {
    static let signalYellow = 0xFFDD00
    static let inkHex = 0x111418
    static let lightText = 0x1F2937
    static let darkText = 0xEDEDE7
    static let lightBackground = 0xF8F9FB
    static let darkBackground = 0x0B0B0C
    static let midGray = 0x808080
    static var border: Color { color(midGray).opacity(0.2) }
    static var brand: Color { color(signalYellow) }
    static var ink: Color { color(inkHex) }
    static var accent: Color { adaptive(light: inkHex, dark: signalYellow) }
    static var text: Color { adaptive(light: lightText, dark: darkText) }
    static var secondaryText: Color { text.opacity(0.72) }
    static var background: Color { adaptive(light: lightBackground, dark: darkBackground) }

    private static func color(_ hex: Int) -> Color {
        Color(red: Double((hex >> 16) & 255) / 255, green: Double((hex >> 8) & 255) / 255, blue: Double(hex & 255) / 255)
    }
    private static func adaptive(light: Int, dark: Int) -> Color {
        #if os(iOS)
        Color(uiColor: UIColor { traits in
            let hex = traits.userInterfaceStyle == .dark ? dark : light
            return UIColor(red: CGFloat((hex >> 16) & 255) / 255, green: CGFloat((hex >> 8) & 255) / 255, blue: CGFloat(hex & 255) / 255, alpha: 1)
        })
        #else
        color(dark)
        #endif
    }
}

@main
struct VoiceApp: App {
    #if os(iOS)
    @UIApplicationDelegateAdaptor(VoicePhoneDelegate.self) private var delegate
    @StateObject private var model = VoiceModel.shared
    #else
    @StateObject private var model = VoiceModel()
    #endif
    @Environment(\.scenePhase) private var phase

    var body: some Scene {
        WindowGroup {
            VoiceHome(model: model)
                .tint(VoicePalette.accent)
                .foregroundStyle(VoicePalette.text)
                .onChange(of: phase, initial: true) { _, value in model.scene(active: value == .active) }
        }
    }
}

private struct VoiceHome: View {
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @ObservedObject var model: VoiceModel
    #if os(watchOS)
    @State private var watchPath: [WatchDestination] = []
    #endif
    #if os(iOS)
    @State private var showModels = false
    @State private var query = ""
    @FocusState private var searchFocused: Bool
    #if DEBUG
    @State private var tab = ProcessInfo.processInfo.arguments.contains("--meeting-import-probe") ? 2 : 0
    #else
    @State private var tab = 0
    #endif

    #endif

    var body: some View {
        #if os(watchOS)
        NavigationStack(path: $watchPath) {
            ScrollView {
                VStack(spacing: 10) {
                    Label(model.reachable ? "iPhone verbunden" : "Übergabe später", systemImage: model.reachable ? "iphone" : "clock")
                        .font(.caption2).foregroundStyle(VoicePalette.secondaryText)
                    recordButton
                    conversationControls
                    WatchVolumeLink()
                    Text(model.status).font(.caption).multilineTextAlignment(.center).accessibilityIdentifier("status")
                    if !model.reachable { Text("Deine Aufnahme bleibt auf der Watch gespeichert.").font(.caption2).foregroundStyle(VoicePalette.secondaryText) }
                    if !model.storageIssues.isEmpty { recovery }
                    if let latest = model.entries.first {
                        NavigationLink { VoiceEntryDetail(model: model, original: latest) } label: {
                            VStack(alignment: .leading, spacing: 4) {
                                Text("Zuletzt gespeichert").font(.caption2).foregroundStyle(VoicePalette.secondaryText)
                                Text(latest.reply ?? latest.transcript ?? "Sprachnotiz").font(.caption).lineLimit(2).foregroundStyle(.primary)
                            }.padding(8).frame(maxWidth: .infinity, alignment: .leading)
                                .background(VoicePalette.background, in: RoundedRectangle(cornerRadius: 8))
                                .overlay(RoundedRectangle(cornerRadius: 8).stroke(VoicePalette.border, lineWidth: 1))
                        }.buttonStyle(.plain).accessibilityIdentifier("historyEntry")
                    }
                    NavigationLink(value: WatchDestination.history) { Label("Verlauf", systemImage: "clock.arrow.circlepath") }
                    Button("Erneut versuchen", systemImage: "arrow.clockwise") { model.retry(forceReload: true) }
                    Button("Wiedergabe stoppen", systemImage: "stop.circle") { model.stopPlayback() }
                }.padding(.horizontal, 4)
            }.navigationTitle("")
                .navigationDestination(for: WatchDestination.self) { destination in
                    if destination == .latest {
                        if let latest = model.entries.first { VoiceEntryDetail(model: model, original: latest) }
                        else { Text("Noch keine Sprachnotiz").navigationTitle("Letzte Notiz") }
                    } else {
                        List {
                            if model.entries.isEmpty { Text("Noch keine Sprachnotiz") }
                            ForEach(model.entries) { entry in
                                NavigationLink { VoiceEntryDetail(model: model, original: entry) } label: { VoiceEntryRow(entry: entry) }
                                    .modifier(NoteDeletion(model: model, entry: entry))
                            }
                        }.navigationTitle("Verlauf")
                    }
                }
        }
        .onOpenURL { url in
            guard let destination = WatchDestination(url: url) else { return }
            watchPath = destination == .speak ? [] : [destination]
        }
        #else
        TabView(selection: $tab) {

            NavigationStack {
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        captureCard
                        conversationControls.voiceCard()
                        if !model.storageIssues.isEmpty { recovery }
                        VStack(alignment: .leading, spacing: 12) {
                            HStack {
                                Text("ZULETZT GESPEICHERT").font(.caption.weight(.medium)).tracking(0.8).foregroundStyle(VoicePalette.secondaryText)
                                Spacer()
                                Button("Alle anzeigen") { tab = 1 }.font(.subheadline).accessibilityIdentifier("allHistory")
                            }
                            if model.entries.isEmpty { emptyHistory }
                            ForEach(Array(model.entries.prefix(5))) { entry in
                                NavigationLink { VoiceEntryDetail(model: model, original: entry) } label: { VoiceEntryRow(entry: entry).voiceCard() }
                                    .buttonStyle(.plain).accessibilityIdentifier("historyEntry")
                            }
                        }
                    }.padding(16)
                }
                .background(VoicePalette.background)
                .navigationTitle("Local Voice")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .topBarTrailing) {
                        Button("Einstellungen", systemImage: "slider.horizontal.3") { showModels = true }.accessibilityIdentifier("settings")
                    }
                }
            }
            .tabItem { Label("Sprechen", systemImage: "waveform") }.tag(0)
            MeetingLibraryView().tabItem { Label("Transkripte", systemImage: "doc.text") }.tag(2)
            NavigationStack {
                List {
                    Group {
                        HStack(spacing: 10) {
                            Image(systemName: "magnifyingglass").foregroundStyle(VoicePalette.secondaryText)
                            TextField("Aufnahmen und Antworten suchen", text: $query)
                                .focused($searchFocused).submitLabel(.search)
                                .onSubmit { searchFocused = false }
                                .accessibilityIdentifier("historySearch")
                            if !query.isEmpty {
                                Button("Suche leeren", systemImage: "xmark.circle.fill") { query = "" }
                                    .labelStyle(.iconOnly).foregroundStyle(VoicePalette.secondaryText)
                            }
                            if searchFocused {
                                Button("Fertig") { searchFocused = false }.accessibilityIdentifier("closeSearch")
                            }
                        }.padding(14)
                            .background(VoicePalette.background, in: RoundedRectangle(cornerRadius: 8))
                            .overlay(RoundedRectangle(cornerRadius: 8).stroke(VoicePalette.border, lineWidth: 1))
                        if !model.storageIssues.isEmpty { recovery }
                        if model.entries.isEmpty { emptyHistory }
                        else if filteredEntries.isEmpty {
                            ContentUnavailableView("Keine passende Aufnahme", systemImage: "magnifyingglass", description: Text("Versuche einen anderen Suchbegriff."))
                        }
                        ForEach(filteredEntries) { entry in
                            NavigationLink { VoiceEntryDetail(model: model, original: entry) } label: { VoiceEntryRow(entry: entry).voiceCard() }
                                .buttonStyle(.plain).accessibilityIdentifier("historyEntry")
                                .modifier(NoteDeletion(model: model, entry: entry))
                        }
                    }.listRowBackground(VoicePalette.background)
                }
                .scrollContentBackground(.hidden)
                .background(VoicePalette.background)
                .navigationTitle("Verlauf")
                .navigationBarTitleDisplayMode(.inline)

            }.tabItem { Label("Verlauf", systemImage: "clock.arrow.circlepath") }.tag(1)
        }
        .sheet(isPresented: $showModels) { ModelPanel(model: model) }
        #endif
    }

    #if os(iOS)
    private var filteredEntries: [Entry] {
        let text = query.trimmingCharacters(in: .whitespacesAndNewlines)
        return text.isEmpty ? model.entries : model.entries.filter {
            ($0.transcript ?? "").localizedCaseInsensitiveContains(text) || ($0.reply ?? "").localizedCaseInsensitiveContains(text)
        }
    }
    private var captureCard: some View {
        VStack(alignment: .leading, spacing: 12) {
            if dynamicTypeSize.isAccessibilitySize { recordButton }
            HStack(alignment: .center, spacing: 12) {
                if !dynamicTypeSize.isAccessibilitySize {
                    VoiceBrandMark().frame(width: 36, height: 36).accessibilityHidden(true)
                }
                VStack(alignment: .leading, spacing: 4) {
                    Text(model.recording ? "Aufnahme läuft" : "Deine Sprachnotiz").font(.headline)
                    Text(model.recording ? "Zum Beenden sichern · maximal 30 Sekunden" : "Lokal aufnehmen, verarbeiten und aufbewahren")
                        .font(.caption).foregroundStyle(VoicePalette.secondaryText)
                }
            }
            if !dynamicTypeSize.isAccessibilitySize { recordButton }
            HStack(alignment: .top, spacing: 8) {
                if model.processing { ProgressView().controlSize(.small).accessibilityLabel("Verarbeitung läuft") }
                Text(model.status).font(.subheadline.weight(.medium)).accessibilityIdentifier("status")
            }
            Label(model.reachable ? "Watch verbunden" : "Watch-Übergabe bei nächster Verbindung", systemImage: model.reachable ? "applewatch" : "clock")
                .font(.caption).foregroundStyle(VoicePalette.secondaryText)
            ViewThatFits(in: .horizontal) {
                HStack { recoveryButton; Spacer(); optionsMenu }
                VStack(alignment: .leading, spacing: 8) { recoveryButton; optionsMenu }
            }.font(.footnote).buttonStyle(.borderless)
        }.padding(16).frame(maxWidth: .infinity, alignment: .leading)
            .background(VoicePalette.background, in: RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(VoicePalette.border, lineWidth: 1))
    }
    private var recoveryButton: some View {
        Button { model.retry(forceReload: true) } label: {
            Text("Erneut versuchen").frame(minHeight: 44)
        }
    }
    private var optionsMenu: some View {
        Menu {
            Button("Wiedergabe stoppen", systemImage: "stop.circle") { model.stopPlayback() }
            if model.processing { Button("Verarbeitung abbrechen", systemImage: "pause.circle") { model.cancelProcessing() } }
        } label: { Label("Optionen", systemImage: "ellipsis.circle").frame(minHeight: 44) }
    }
    private var emptyHistory: some View {
        ContentUnavailableView("Raum für deine Gedanken", systemImage: "text.bubble", description: Text("Deine erste Aufnahme und die Antwort erscheinen hier."))
            .voiceCard()
    }
    #endif

    @ViewBuilder private var conversationControls: some View {
        #if os(watchOS)
        NavigationLink { ScrollView { conversationOptions.padding() }.navigationTitle("Gespräch") } label: {
            Label("Gespräch", systemImage: "bubble.left.and.bubble.right")
        }.accessibilityIdentifier("conversationControls")
        #else
        DisclosureGroup("Gespräch") { conversationOptions }
        #endif
    }
    private var conversationOptions: some View {
        VStack(alignment: .leading, spacing: 10) {
            Toggle("Antwort vorlesen", isOn: $model.autoPlayReplies).accessibilityIdentifier("autoPlayReplies")
            Toggle("Freisprechen", isOn: $model.handsFreeEnabled).accessibilityIdentifier("handsFreeEnabled")
            NavigationLink { SpeechVoicePicker(model: model) } label: {
                Label("Stimme & Hörprobe", systemImage: "speaker.wave.2")
            }.accessibilityIdentifier("voiceSettings")
            MicrophoneSettings(model: model)
            if model.handsFreeEnabled {
                Text("Nach einer Sprechpause antworten. Danach wieder zuhören. Beim Vorlesen pausiert das Mikrofon.").font(.caption2).foregroundStyle(VoicePalette.secondaryText)
                Text("Beim Verlassen dieser App pausiert das Freisprechen. Kontext: bis zu 6 vorherige Wortwechsel dieses Gesprächs, lokal gespeichert.").font(.caption2).foregroundStyle(VoicePalette.secondaryText)
                Button("Neues Gespräch", systemImage: "plus.bubble") { model.newConversation() }
                if model.conversationRunning { Text(model.recording ? "Mikrofon aktiv" : "Warte auf Antwort").font(.caption).foregroundStyle(VoicePalette.accent) }
            }
        }
    }
    private var recordButton: some View {
        Button { if model.conversationRunning { model.stopConversation() } else { model.recording ? model.stop() : model.start() } } label: {
            Label(model.conversationRunning ? "Gespräch beenden" : model.recording ? "Aufnahme sichern" : "Sprechen", systemImage: model.recording || model.conversationRunning ? "stop.fill" : "mic.fill")
                .font(.headline)
                .foregroundStyle(model.recording || model.conversationRunning ? Color.white : VoicePalette.ink)
                .frame(maxWidth: .infinity, minHeight: 44)
        }
        .buttonStyle(VoicePrimaryButtonStyle(recording: model.recording || model.conversationRunning))
        .accessibilityIdentifier("record")
        .accessibilityLabel(model.conversationRunning ? "Gespräch beenden" : model.recording ? "Aufnahme sichern" : "Sprechen")
        .accessibilityHint(model.recording ? "Beendet die Aufnahme und speichert sie auf diesem Gerät." : "Startet eine Aufnahme von höchstens 30 Sekunden.")
    }

    private var recovery: some View {
        VStack(alignment: .leading, spacing: 12) {
            Label("Wiederherstellung erforderlich", systemImage: "exclamationmark.shield").font(.headline)
            Text("Deine Dateien bleiben erhalten. Sichere ein Original oder versuche die Wiederherstellung.").font(.caption).foregroundStyle(VoicePalette.secondaryText)
            ForEach(model.storageIssues) { issue in
                VStack(alignment: .leading, spacing: 8) {
                    Text(issue.reason).font(.subheadline)
                    #if os(iOS)
                    let audio = issue.url.lastPathComponent.hasPrefix(".recording-") ? issue.url : issue.url.appendingPathComponent("audio.m4a")
                    if FileManager.default.fileExists(atPath: audio.path) { ShareLink("Audiodatei sichern", item: audio) }
                    let metadata = issue.url.appendingPathComponent("entry.json")
                    if FileManager.default.fileExists(atPath: metadata.path) { ShareLink("Metadaten sichern", item: metadata) }
                    #endif
                }
            }
            Button("Wiederherstellung versuchen") { model.recoverStorage() }
        }.voiceCard()
    }
}

private struct VoiceEntryRow: View {
    let entry: Entry
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(entry.createdAt, format: .dateTime.day().month(.abbreviated).hour().minute())
                Spacer(minLength: 4)
                Image(systemName: entry.reply == nil ? "clock" : "checkmark.circle.fill")
                    .foregroundStyle(entry.reply == nil ? Color.secondary : VoicePalette.accent)
            }.font(.caption).foregroundStyle(VoicePalette.secondaryText)
            Text(entry.transcript ?? "Gespeicherte Sprachnotiz").font(.headline).lineLimit(1)
            Text((try? AttributedString(markdown: entry.reply ?? "gespeichert – Verarbeitung folgt")) ?? AttributedString(entry.reply ?? "gespeichert – Verarbeitung folgt")).font(.subheadline).foregroundStyle(VoicePalette.secondaryText).lineLimit(1)
            if let event = entry.processingEvents?.last(where: { $0.operation == "Antwort" }) {
                Text((event.isAI ? "KI · " : "System · ") + event.model).font(.caption2).foregroundStyle(VoicePalette.secondaryText).lineLimit(1)
            }
        }.frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityElement(children: .combine)
    }
}

private struct VoiceEntryDetail: View {
    @ObservedObject var model: VoiceModel
    let original: Entry
    @Environment(\.dismiss) private var dismiss
    private var entry: Entry { model.entries.first(where: { $0.id == original.id }) ?? original }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                Text(entry.createdAt, format: .dateTime.day().month(.wide).year().hour().minute())
                    .font(.caption).foregroundStyle(VoicePalette.secondaryText)
                VStack(alignment: .leading, spacing: 12) {
                    Label("Deine Aufnahme", systemImage: "waveform").font(.headline).foregroundStyle(VoicePalette.accent)
                    if model.recordingURL(entry.id) != nil {
                        OriginalRecordingControls(model: model, id: entry.id)
                    } else {
                        Text("Originalaufnahme auf diesem Gerät nicht verfügbar").font(.caption).foregroundStyle(VoicePalette.secondaryText)
                    }
                    Text(entry.transcript ?? "Deine Sprachnotiz ist gespeichert. Das Transkript folgt nach der Verarbeitung.")
                        .selectableOnPhone()
                    if let event = entry.processingEvents?.last(where: { $0.operation == "Transkription" }) {
                        ProcessingDisclosure(events: [event])
                    }
                    #if os(iOS)
                    if let url = model.recordingURL(entry.id) { ShareLink("Originalaufnahme sichern", item: url).font(.subheadline) }
                    #endif
                }.voiceCard()
                #if os(watchOS)
                WatchVolumeLink()
                #endif
                VStack(alignment: .leading, spacing: 12) {
                    Label("Antwort", systemImage: "text.bubble").font(.headline).foregroundStyle(VoicePalette.accent)
                    VoiceMarkdown(text: entry.reply ?? "gespeichert – Verarbeitung folgt").selectableOnPhone()
                    if let reply = entry.reply {
                        HStack(spacing: 8) {
                            Button("Antwort anhören", systemImage: "play.fill") { model.speak(reply, id: entry.id) }
                                .buttonStyle(VoiceMediaButtonStyle(primary: true))
                            Button("Wiedergabe stoppen", systemImage: "stop.fill") { model.stopPlayback() }
                                .buttonStyle(VoiceMediaButtonStyle(primary: false))
                            ProcessingDisclosure(events: entry.processingEvents ?? [])
                        }.labelStyle(.iconOnly)
                    }
                    #if os(iOS)
                    if entry.reply == nil, let job = entry.job {
                        Text(job.running ? "Wird verarbeitet …" : job.phase == .failed ? "Verarbeitung angehalten" : job.phase == .cancelled ? "Verarbeitung abgebrochen" : "Verarbeitung ausstehend").font(.caption).foregroundStyle(VoicePalette.secondaryText)
                        if job.failure == .noSpeech { Text("Keine Sprache erkannt – Originalaufnahme erhalten").font(.caption) }
                        if !job.running { Button("Verarbeitung erneut starten") { model.retryProcessing(entry.id) } }
                    }
                    #endif
                }.voiceCard()
            }.padding()
        }.navigationTitle("Sprachnotiz")
        .modifier(NoteDeletion(model: model, entry: entry, toolbar: true, didDelete: { dismiss() }))
        #if os(iOS)
        .navigationBarTitleDisplayMode(.inline)
        .background(VoicePalette.background)
        #endif
    }
}

private struct OriginalRecordingControls: View {
    @ObservedObject var model: VoiceModel
    let id: UUID
    private var selected: Bool { model.playingRecordingId == id }
    private var playing: Bool { selected && !model.recordingPlaybackPaused }
    private var title: String { playing ? "Aufnahme pausieren" : selected ? "Aufnahme fortsetzen" : "Aufnahme anhören" }
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Button(title, systemImage: playing ? "pause.fill" : "play.fill") { model.toggleOriginalPlayback(id) }
                    .buttonStyle(VoiceMediaButtonStyle(primary: true)).accessibilityIdentifier("originalPlayback")
                Button("Aufnahme stoppen", systemImage: "stop.fill") { model.stopPlayback() }
                    .buttonStyle(VoiceMediaButtonStyle(primary: false)).disabled(!selected).accessibilityIdentifier("originalStop")
            }.labelStyle(.iconOnly)
            if selected {
                Text((model.recordingPlaybackPaused ? "Pausiert · " : "Original · ") + time(model.recordingPlaybackTime) + " / " + time(model.recordingPlaybackDuration))
                    .font(.caption2).monospacedDigit().foregroundStyle(VoicePalette.secondaryText)
                    .accessibilityIdentifier("originalProgress")
            }
            if model.originalPlaybackIssueId == id, let message = model.originalPlaybackError {
                Text(message).font(.caption2).foregroundStyle(VoicePalette.secondaryText).accessibilityIdentifier("originalPlaybackError")
            }
        }
    }
    private func time(_ seconds: TimeInterval) -> String {
        let value = max(0, Int(seconds))
        return String(format: "%d:%02d", value / 60, value % 60)
    }
}

#if os(watchOS)
private struct WatchVolumeControl: WKInterfaceObjectRepresentable {
    func makeWKInterfaceObject(context: Context) -> WKInterfaceVolumeControl {
        let control = WKInterfaceVolumeControl(origin: .local)
        control.setTintColor(UIColor(red: 1, green: 221.0 / 255, blue: 0, alpha: 1))
        control.focus()
        return control
    }
    func updateWKInterfaceObject(_ object: WKInterfaceVolumeControl, context: Context) {}
    static func dismantleWKInterfaceObject(_ object: WKInterfaceVolumeControl, coordinator: ()) { object.resignFocus() }
}
private struct WatchVolumeLink: View {
    var body: some View {
        NavigationLink {
            VStack(spacing: 12) {
                WatchVolumeControl().frame(height: 55).accessibilityIdentifier("watchVolumeControl")
                Text("Medienlautstärke der Watch. Mit der Digital Crown anpassen.")
                    .font(.caption2).foregroundStyle(VoicePalette.secondaryText).multilineTextAlignment(.center)
            }.padding().navigationTitle("Lautstärke")
        } label: { Label("Lautstärke", systemImage: "speaker.wave.2") }
            .accessibilityIdentifier("watchVolume")
    }
}
#endif

extension View {
    @ViewBuilder func selectableOnPhone() -> some View {
        #if os(iOS)
        self.textSelection(.enabled)
        #else
        self
        #endif
    }
    func voiceCard() -> some View {
        self.padding(16).frame(maxWidth: .infinity, alignment: .leading)
            .background(VoicePalette.background, in: RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(VoicePalette.border, lineWidth: 1))
    }
}

// Geometry mirrors LocalVoiceAiMark's 32-point SVG viewBox on desktop.
struct VoiceBrandMark: View {
    var body: some View {
        GeometryReader { geometry in
            let scale = min(geometry.size.width, geometry.size.height) / 32
            ZStack(alignment: .topLeading) {
                RoundedRectangle(cornerRadius: 7 * scale).fill(VoicePalette.ink)
                Path { path in
                    for (x, y, height) in [(7.0, 14.5, 5.0), (11.5, 11.5, 11.0), (16.0, 9.0, 16.0), (20.5, 11.5, 11.0), (25.0, 14.5, 5.0)] {
                        path.move(to: CGPoint(x: x * scale, y: y * scale))
                        path.addLine(to: CGPoint(x: x * scale, y: (y + height) * scale))
                    }
                }.stroke(VoicePalette.brand, style: StrokeStyle(lineWidth: 2.4 * scale, lineCap: .round))
                Circle().fill(VoicePalette.brand).frame(width: 3.4 * scale, height: 3.4 * scale)
                    .offset(x: 23.3 * scale, y: 6.5 * scale)
            }
        }
    }
}

struct VoicePrimaryButtonStyle: ButtonStyle {
    let recording: Bool
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.padding(.horizontal, 16).padding(.vertical, 4)
            .background(recording ? Color.red : VoicePalette.brand, in: RoundedRectangle(cornerRadius: 8))
            .opacity(configuration.isPressed ? 0.8 : 1)
    }
}

private struct VoiceMediaButtonStyle: ButtonStyle {
    @Environment(\.colorScheme) private var scheme
    let primary: Bool
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.font(.system(size: 19, weight: .medium)).frame(width: 44, height: 44)
            .foregroundStyle(primary ? (scheme == .light ? VoicePalette.brand : VoicePalette.ink) : VoicePalette.text)
            .background(primary ? (scheme == .light ? VoicePalette.ink : VoicePalette.brand) : Color.clear, in: Circle())
            .contentShape(Circle()).opacity(configuration.isPressed ? 0.7 : 1)
    }
}

/// Render inline Markdown while keeping paragraphs, headings and lists readable.
struct VoiceMarkdown: View {
    let text: String
    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            ForEach(Array(text.components(separatedBy: "\n").enumerated()), id: \.offset) { _, line in
                let trimmed = line.trimmingCharacters(in: .whitespaces)
                if !trimmed.hasPrefix("```") {
                    let heading = trimmed.hasPrefix("#") && trimmed.contains(" ")
                    let content = heading ? String(trimmed.drop(while: { $0 == "#" || $0 == " " })) : trimmed.hasPrefix("- ") || trimmed.hasPrefix("* ") ? "• " + trimmed.dropFirst(2) : line
                    Text((try? AttributedString(markdown: content, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(content))
                        .font(heading ? .headline : .body)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
    }
}

struct ProcessingDisclosure: View {
    let events: [ProcessingEvent]
    @State private var showing = false
    private var latest: ProcessingEvent? { events.last(where: { $0.operation == "Antwort" }) ?? events.last }
    var body: some View {
        Button { showing = true } label: {
            VStack(alignment: .leading, spacing: 2) {
                Text(latest.map { ($0.isAI ? "KI · " : "System · ") + $0.model } ?? "KI-Info · nicht dokumentiert")
                    .lineLimit(2)
                if let event = latest { Text(event.completedAt, format: .dateTime.hour().minute()) }
            }.font(.caption2).foregroundStyle(VoicePalette.secondaryText)
        }
        .buttonStyle(.plain).accessibilityIdentifier("processingDisclosure")
        .sheet(isPresented: $showing) {
            NavigationStack {
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        if events.isEmpty { Text("Für diesen älteren Eintrag wurden Modell und Verarbeitungszeit nicht gespeichert.") }
                        ForEach(Array(events.enumerated()), id: \.offset) { _, event in
                            VStack(alignment: .leading, spacing: 4) {
                                Text(event.operation).font(.headline)
                                Text(event.model)
                                Text(event.isAI ? "Mit KI lokal verarbeitet" : "Lokale Systemverarbeitung · keine LLM-Antwort")
                                Text(event.completedAt, format: .dateTime.day().month().year().hour().minute().second())
                                Text(String(format: "Dauer: %.2f s", event.durationMS / 1000))
                            }
                        }
                        Text("Apple stellt die genaue interne Version seiner Systemmodelle nicht bereit.").font(.caption)
                    }.padding()
                }.navigationTitle("Verarbeitung")
                .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Fertig") { showing = false } } }
            }
        }
    }
}

private struct NoteDeletion: ViewModifier {
    @ObservedObject var model: VoiceModel
    let entry: Entry
    var toolbar = false
    var didDelete: () -> Void = {}
    @State private var confirming = false
    @State private var errorMessage: String?
    func body(content: Content) -> some View {
        content
            .safeAreaInset(edge: .top) {
                #if os(watchOS)
                if toolbar {
                    Button("Sprachnotiz löschen", systemImage: "trash", role: .destructive) { confirming = true }
                        .font(.caption).accessibilityIdentifier("deleteNote")
                }
                #endif
            }
            .toolbar {
                #if os(iOS)
                if toolbar {
                    ToolbarItem(placement: .topBarTrailing) {
                        Button("Sprachnotiz löschen", systemImage: "trash", role: .destructive) { confirming = true }
                            .labelStyle(.iconOnly).accessibilityIdentifier("deleteNote")
                    }
                }
                #endif
            }
            .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                if !toolbar {
                    Button("Löschen", systemImage: "trash") { confirming = true }
                        .tint(.red)
                        .accessibilityIdentifier("swipeDeleteNote")
                }
            }
            .alert(errorMessage == nil ? "Sprachnotiz löschen?" : "Löschen fehlgeschlagen",
                   isPresented: Binding(get: { confirming || errorMessage != nil }, set: { if !$0 { confirming = false; errorMessage = nil } })) {
                if errorMessage == nil {
                    Button("Endgültig löschen", role: .destructive) {
                        do { try model.deleteNote(entry.id); didDelete() }
                        catch {
                            Task { @MainActor in
                                model.refresh()
                                errorMessage = "Löschen konnte nicht vollständig abgeschlossen werden. Bitte erneut versuchen."
                            }
                        }
                    }.accessibilityIdentifier("confirmDeleteNote")
                    Button("Abbrechen", role: .cancel) {}
                } else { Button("OK") { errorMessage = nil } }
            } message: {
                Text(errorMessage ?? "Originalaufnahme, Transkript und Antwort werden auf diesem Gerät gelöscht. Kopien auf anderen Geräten bleiben erhalten.")
            }
    }
}
