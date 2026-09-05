import SwiftUI

@main
struct VoiceApp: App {
    @StateObject private var model = VoiceModel()
    #if os(iOS)
    @State private var showModels = false
    #endif
    @Environment(\.scenePhase) private var phase
    var body: some Scene {
        WindowGroup {
            NavigationStack {
                #if os(watchOS)
                ScrollView { LazyVStack { controls; history } }
                    .navigationTitle("Local Voice")
                #else
                TabView {
                    ScrollView { LazyVStack { controls.padding(); history.padding() } }
                        .tabItem { Label("Sprechen", systemImage: "mic") }
                    List { history }.tabItem { Label("Tagebuch", systemImage: "book") }
                }.navigationTitle("Local Voice")
                .sheet(isPresented: $showModels) { ModelPanel(model: model) }
                #endif
            }
            .onChange(of: phase, initial: true) { _, value in model.scene(active: value == .active) }
        }
    }
    private var controls: some View {
        VStack(spacing: 12) {
            Text(model.status).font(.headline).accessibilityIdentifier("status")
            Text(model.reachable ? "iPhone/Watch erreichbar" : "Verarbeitung bei nächster Verbindung").font(.caption)
            Button(model.recording ? "Aufnahme sichern" : "Sprechen") { model.recording ? model.stop() : model.start() }
                .buttonStyle(.borderedProminent).tint(model.recording ? .red : .accentColor)
                .accessibilityIdentifier("record")
            Button("Erneut versuchen") { model.retry() }
            Button("Wiedergabe stoppen") { model.stopPlayback() }
            #if os(iOS)
            if model.processing { Button("Verarbeitung abbrechen") { model.cancelProcessing() } }
            Button("Lokale Sprachmodelle") { showModels = true }
            #endif
        }.padding(8)
    }
    @ViewBuilder
    private var history: some View {
        if !model.storageIssues.isEmpty {
            Section("Wiederherstellung erforderlich") {
                Text("Diese Dateien sind erhalten und werden nicht automatisch gelöscht.").font(.caption)
                ForEach(model.storageIssues) { issue in
                    VStack(alignment: .leading) {
                        Text(issue.reason)
                        Text(issue.sessionId?.uuidString.prefix(8) ?? "Aufnahmeentwurf").font(.caption)
                        #if os(iOS)
                        let audio = issue.url.lastPathComponent.hasPrefix(".recording-") ? issue.url : issue.url.appendingPathComponent("audio.m4a")
                        if FileManager.default.fileExists(atPath: audio.path) { ShareLink("Audiodatei sichern", item: audio) }
                        let metadata = issue.url.appendingPathComponent("entry.json")
                        if FileManager.default.fileExists(atPath: metadata.path) { ShareLink("Metadaten sichern", item: metadata) }
                        #endif
                    }
                }
                Button("Wiederherstellung versuchen") { model.recoverStorage() }
            }
        }
        ForEach(model.entries) { entry in
            VStack(alignment: .leading, spacing: 6) {
                Text(entry.createdAt, style: .time).font(.caption)
                Text(entry.transcript ?? "Gespeicherte Sprachnotiz")
                Text(entry.reply ?? "gespeichert – Verarbeitung folgt").foregroundStyle(.secondary)
                #if os(iOS)
                if let url = model.recordingURL(entry.id) { ShareLink("Originalaufnahme sichern", item: url) }
                if entry.reply == nil, let job = entry.job, !job.running {
                    Text(job.phase == .failed ? "Verarbeitung angehalten" : job.phase == .cancelled ? "Verarbeitung abgebrochen" : "Verarbeitung ausstehend").font(.caption)
                    if job.failure == .noSpeech { Text("Keine Sprache erkannt – Originalaufnahme erhalten").font(.caption) }
                    Button("Verarbeitung erneut starten") { model.retryProcessing(entry.id) }
                }
                #endif
                if let reply = entry.reply { Button("Antwort anhören") { model.speak(reply, id: entry.id) } }
            }.padding(.vertical, 6)
        }
    }
}
