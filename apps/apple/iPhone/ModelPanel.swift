import SwiftUI
import UniformTypeIdentifiers

struct ModelPanel: View {
    @ObservedObject var model: VoiceModel
    @State private var importing = false
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        NavigationStack {
            List {
                Section("Auf diesem Gerät") {
                    Label("Verarbeitung auf deinem iPhone", systemImage: "lock.shield")
                        .font(.headline).foregroundStyle(VoicePalette.accent)
                    Text(model.providerDescription).font(.subheadline).foregroundStyle(.secondary)
                    ForEach(model.installedModels) { item in
                        VStack(alignment: .leading) {
                            HStack {
                                Text(item.model.label).font(.headline)
                                Spacer()
                                Image(systemName: item.installed ? "checkmark.circle.fill" : "arrow.down.circle")
                                    .foregroundStyle(item.installed ? VoicePalette.accent : .secondary)
                                    .accessibilityLabel(item.installed ? "Installiert" : "Nicht installiert")
                            }
                            Text(item.installed ? "Vorhanden · \(ByteCountFormatter.string(fromByteCount: item.installedBytes, countStyle: .file))" : "Fehlt · \(ByteCountFormatter.string(fromByteCount: item.model.bytes, countStyle: .file)) benötigt")
                                .font(.caption)
                        }
                    }
                    Text("Vor der Nutzung wird die Integrität geprüft. Audio und Antworten werden lokal verarbeitet.").font(.caption)
                    Text("Das kleine Antwortmodell machte in den Tests inhaltliche Fehler, etwa beim Rechnen.").font(.caption)
                }
                Section("Spracherkennung") {
                    Picker("Lokales Sprachmodell", selection: $model.sttModel) {
                        Text("Base – schneller").tag("ggml-base.bin")
                        Text("Small – Qualitätsvergleich").tag("ggml-small.bin")
                    }
                    Text("Die Auswahl gilt für die nächste Verarbeitung. Small war im Simulator deutlich langsamer und nicht in allen Sprachfällen besser.").font(.caption)
                    Button("Apple-Sprachdateien laden") { model.prepareLocalSpeech() }
                }
                Section("Modell hinzufügen") {
                    Button(model.installingModel ? "Modell wird geprüft …" : "Modelldatei auswählen") { importing = true }
                        .disabled(model.installingModel)
                    Text("Unterstützt werden die dokumentierten Whisper-Base-/Small- und Qwen-Dateien. Unbekannte oder beschädigte Dateien ersetzen kein vorhandenes Modell.").font(.caption)
                    Text(model.modelMessage).font(.caption)
                }
                #if DEBUG
                Section("Prototyp-Test") { Toggle("Feste Antwort ohne Spracherkennung", isOn: $model.fixedAnswer) }
                #endif
            }
            .navigationTitle("Lokale Sprachmodelle")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Fertig") { dismiss() } } }
            .task { await model.refreshModels() }
            .fileImporter(isPresented: $importing, allowedContentTypes: [.data]) { result in
                if case .success(let url) = result { model.installModel(url) }
            }
        }
    }
}
