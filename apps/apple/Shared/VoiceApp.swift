import SwiftUI

@main
struct VoiceApp: App {
    @StateObject private var model = VoiceModel()
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
            Toggle("Feste Testantwort (ohne STT/KI)", isOn: $model.fixedAnswer)
            Button("Deutsche Sprachdateien laden") { model.prepareLocalSpeech() }
            Text("Ohne Testantwort: ausschließlich lokale deutsche Spracherkennung und lokale KI. Fehlende Modelle führen zu späterer Verarbeitung.").font(.caption)
            #endif
        }.padding(8)
    }
    private var history: some View {
        ForEach(model.entries) { entry in
            VStack(alignment: .leading, spacing: 6) {
                Text(entry.createdAt, style: .time).font(.caption)
                Text(entry.transcript ?? "Gespeicherte Sprachnotiz")
                Text(entry.reply ?? "gespeichert – Verarbeitung folgt").foregroundStyle(.secondary)
                if let reply = entry.reply { Button("Antwort anhören") { model.speak(reply, id: entry.id) } }
            }.padding(.vertical, 6)
        }
    }
}
