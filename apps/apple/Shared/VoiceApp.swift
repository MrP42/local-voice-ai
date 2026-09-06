import SwiftUI
#if os(iOS)
import UIKit
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
    static var brand: Color { color(signalYellow) }
    static var ink: Color { color(inkHex) }
    static var accent: Color { adaptive(light: inkHex, dark: signalYellow) }
    static var text: Color { adaptive(light: lightText, dark: darkText) }
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
    @StateObject private var model = VoiceModel()
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
    #if os(iOS)
    @State private var showModels = false
    @State private var query = ""
    @FocusState private var searchFocused: Bool
    @State private var tab = 0
    #endif

    var body: some View {
        #if os(watchOS)
        NavigationStack {
            ScrollView {
                VStack(spacing: 10) {
                    Label(model.reachable ? "iPhone verbunden" : "Übergabe später", systemImage: model.reachable ? "iphone" : "clock")
                        .font(.caption2).foregroundStyle(.secondary)
                    recordButton
                    Text(model.status).font(.caption).multilineTextAlignment(.center).accessibilityIdentifier("status")
                    if !model.reachable { Text("Deine Aufnahme bleibt auf der Watch gespeichert.").font(.caption2).foregroundStyle(.secondary) }
                    if !model.storageIssues.isEmpty { recovery }
                    if let latest = model.entries.first {
                        NavigationLink { VoiceEntryDetail(model: model, original: latest) } label: {
                            VStack(alignment: .leading, spacing: 4) {
                                Text("Zuletzt gespeichert").font(.caption2).foregroundStyle(.secondary)
                                Text(latest.reply ?? latest.transcript ?? "Sprachnotiz").font(.caption).lineLimit(2).foregroundStyle(.primary)
                            }.padding(8).frame(maxWidth: .infinity, alignment: .leading)
                                .background(.quaternary, in: RoundedRectangle(cornerRadius: 12))
                        }.buttonStyle(.plain)
                    }
                    NavigationLink {
                        List { ForEach(model.entries) { entry in
                            NavigationLink { VoiceEntryDetail(model: model, original: entry) } label: { VoiceEntryRow(entry: entry) }
                        } }.navigationTitle("Verlauf")
                    } label: { Label("Verlauf", systemImage: "clock.arrow.circlepath") }
                    Button("Erneut versuchen", systemImage: "arrow.clockwise") { model.retry(forceReload: true) }
                    Button("Wiedergabe stoppen", systemImage: "stop.circle") { model.stopPlayback() }
                }.padding(.horizontal, 4)
            }.navigationTitle("")
        }
        #else
        TabView(selection: $tab) {
            NavigationStack {
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        captureCard
                        if !model.storageIssues.isEmpty { recovery }
                        VStack(alignment: .leading, spacing: 12) {
                            HStack {
                                Text("Zuletzt gespeichert").font(.title3.bold())
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
                        Button("Lokale Sprachmodelle", systemImage: "slider.horizontal.3") { showModels = true }
                    }
                }
            }
            .tabItem { Label("Sprechen", systemImage: "waveform") }.tag(0)
            NavigationStack {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 12) {
                        HStack(spacing: 10) {
                            Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                            TextField("Aufnahmen und Antworten suchen", text: $query)
                                .focused($searchFocused).submitLabel(.search)
                                .onSubmit { searchFocused = false }
                                .accessibilityIdentifier("historySearch")
                            if !query.isEmpty {
                                Button("Suche leeren", systemImage: "xmark.circle.fill") { query = "" }
                                    .labelStyle(.iconOnly).foregroundStyle(.secondary)
                            }
                            if searchFocused {
                                Button("Fertig") { searchFocused = false }.accessibilityIdentifier("closeSearch")
                            }
                        }.padding(14)
                            .background(Color(.secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 16))
                        if !model.storageIssues.isEmpty { recovery }
                        if model.entries.isEmpty { emptyHistory }
                        else if filteredEntries.isEmpty {
                            ContentUnavailableView("Keine passende Aufnahme", systemImage: "magnifyingglass", description: Text("Versuche einen anderen Suchbegriff."))
                        }
                        ForEach(filteredEntries) { entry in
                            NavigationLink { VoiceEntryDetail(model: model, original: entry) } label: { VoiceEntryRow(entry: entry).voiceCard() }
                                .buttonStyle(.plain).accessibilityIdentifier("historyEntry")
                        }
                    }.padding(16)
                }
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
                    Image(systemName: "waveform")
                        .font(.system(size: 22, weight: .medium))
                        .foregroundStyle(VoicePalette.brand)
                        .frame(width: 44, height: 44)
                        .background(VoicePalette.ink, in: RoundedRectangle(cornerRadius: 12))
                        .accessibilityHidden(true)
                }
                VStack(alignment: .leading, spacing: 4) {
                    Text(model.recording ? "Aufnahme läuft" : "Deine Sprachnotiz").font(.headline)
                    Text(model.recording ? "Zum Beenden sichern · maximal 30 Sekunden" : "Lokal aufnehmen, verarbeiten und aufbewahren")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            if !dynamicTypeSize.isAccessibilitySize { recordButton }
            HStack(alignment: .top, spacing: 8) {
                if model.processing { ProgressView().controlSize(.small).accessibilityLabel("Verarbeitung läuft") }
                Text(model.status).font(.subheadline.weight(.medium)).accessibilityIdentifier("status")
            }
            Label(model.reachable ? "Watch verbunden" : "Watch-Übergabe bei nächster Verbindung", systemImage: model.reachable ? "applewatch" : "clock")
                .font(.caption).foregroundStyle(.secondary)
            ViewThatFits(in: .horizontal) {
                HStack { recoveryButton; Spacer(); optionsMenu }
                VStack(alignment: .leading, spacing: 8) { recoveryButton; optionsMenu }
            }.font(.footnote).buttonStyle(.borderless)
        }.padding(16).frame(maxWidth: .infinity, alignment: .leading)
            .background(Color(.secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 20))
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

    private var recordButton: some View {
        Button { model.recording ? model.stop() : model.start() } label: {
            Label(model.recording ? "Aufnahme sichern" : "Sprechen", systemImage: model.recording ? "stop.fill" : "mic.fill")
                .font(.headline)
                .foregroundStyle(model.recording ? Color.white : VoicePalette.ink)
                .frame(maxWidth: .infinity, minHeight: 44)
        }
        .buttonStyle(.borderedProminent)
        .tint(model.recording ? .red : VoicePalette.brand)
        .accessibilityIdentifier("record")
        .accessibilityLabel(model.recording ? "Aufnahme sichern" : "Sprechen")
        .accessibilityHint(model.recording ? "Beendet die Aufnahme und speichert sie auf diesem Gerät." : "Startet eine Aufnahme von höchstens 30 Sekunden.")
    }

    private var recovery: some View {
        VStack(alignment: .leading, spacing: 12) {
            Label("Wiederherstellung erforderlich", systemImage: "exclamationmark.shield").font(.headline)
            Text("Deine Dateien bleiben erhalten. Sichere ein Original oder versuche die Wiederherstellung.").font(.caption).foregroundStyle(.secondary)
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
            }.font(.caption).foregroundStyle(.secondary)
            Text(entry.transcript ?? "Gespeicherte Sprachnotiz").font(.headline).lineLimit(1)
            Text(entry.reply ?? "gespeichert – Verarbeitung folgt").font(.subheadline).foregroundStyle(.secondary).lineLimit(1)
        }.frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityElement(children: .combine)
    }
}

private struct VoiceEntryDetail: View {
    @ObservedObject var model: VoiceModel
    let original: Entry
    private var entry: Entry { model.entries.first(where: { $0.id == original.id }) ?? original }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                Text(entry.createdAt, format: .dateTime.day().month(.wide).year().hour().minute())
                    .font(.caption).foregroundStyle(.secondary)
                VStack(alignment: .leading, spacing: 12) {
                    Label("Deine Aufnahme", systemImage: "waveform").font(.headline).foregroundStyle(VoicePalette.accent)
                    Text(entry.transcript ?? "Deine Sprachnotiz ist gespeichert. Das Transkript folgt nach der Verarbeitung.")
                        .selectableOnPhone()
                    #if os(iOS)
                    if let url = model.recordingURL(entry.id) { ShareLink("Originalaufnahme sichern", item: url).font(.subheadline) }
                    #endif
                }.voiceCard()
                VStack(alignment: .leading, spacing: 12) {
                    Label("Antwort", systemImage: "text.bubble").font(.headline).foregroundStyle(VoicePalette.accent)
                    Text(entry.reply ?? "gespeichert – Verarbeitung folgt").selectableOnPhone()
                    if let reply = entry.reply {
                        Button("Antwort anhören", systemImage: "play.fill") { model.speak(reply, id: entry.id) }.buttonStyle(.bordered)
                        Button("Wiedergabe stoppen", systemImage: "stop.circle") { model.stopPlayback() }.font(.subheadline)
                    }
                    #if os(iOS)
                    if entry.reply == nil, let job = entry.job {
                        Text(job.running ? "Wird verarbeitet …" : job.phase == .failed ? "Verarbeitung angehalten" : job.phase == .cancelled ? "Verarbeitung abgebrochen" : "Verarbeitung ausstehend").font(.caption).foregroundStyle(.secondary)
                        if job.failure == .noSpeech { Text("Keine Sprache erkannt – Originalaufnahme erhalten").font(.caption) }
                        if !job.running { Button("Verarbeitung erneut starten") { model.retryProcessing(entry.id) } }
                    }
                    #endif
                }.voiceCard()
            }.padding()
        }.navigationTitle("Sprachnotiz")
        #if os(iOS)
        .navigationBarTitleDisplayMode(.inline)
        .background(VoicePalette.background)
        #endif
    }
}

private extension View {
    @ViewBuilder func selectableOnPhone() -> some View {
        #if os(iOS)
        self.textSelection(.enabled)
        #else
        self
        #endif
    }
    func voiceCard() -> some View {
        self.padding(16).frame(maxWidth: .infinity, alignment: .leading)
            #if os(iOS)
            .background(Color(.secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 20))
            #else
            .background(.quaternary, in: RoundedRectangle(cornerRadius: 16))
            #endif
    }
}
