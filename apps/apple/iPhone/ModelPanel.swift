import SwiftUI
import UniformTypeIdentifiers

struct ModelPanel: View {
    @ObservedObject var model: VoiceModel
    @ObservedObject private var downloads = ModelDownloads.shared
    @State private var importing = false
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        NavigationStack {
            List {
                Section {
                    NavigationLink { SpeechVoicePicker(model: model) } label: {
                        Label("Stimme & Hörprobe", systemImage: "speaker.wave.2")
                    }.accessibilityIdentifier("voiceSettings")
                }
                Section("Mikrofon & Gespräch") { MicrophoneSettings(model: model) }
                Section {
                    Text(model.providerDescription).font(.caption).foregroundStyle(VoicePalette.secondaryText)
                    ForEach(model.installedModels) { item in modelRow(item) }
                } header: { Text("Lokale Sprachmodelle") } footer: {
                    Text("Downloads laufen beim Verlassen der App weiter. Nach dem Wegwischen der App wird der Auftrag beim nächsten Öffnen wieder aufgenommen. Erst nach erfolgreicher Prüfung wird das Modell freigegeben.")
                }
                if !downloads.notificationHint.isEmpty {
                    Section {
                        Text(downloads.notificationHint).font(.caption)
                        Link("Mitteilungen einstellen", destination: URL(string: UIApplication.openSettingsURLString)!)
                    }
                }
                Section("Spracherkennung") {
                    Picker("Sprachmodell", selection: $model.sttModel) {
                        Text("Whisper Base").tag("ggml-base.bin")
                        Text("Whisper Small").tag("ggml-small.bin")
                    }
                    HStack {
                        Text("Apple-Sprachdateien").font(.subheadline)
                        Spacer()
                        if model.preparingSpeech { ProgressView() }
                        else {
                            Button("Apple-Sprachdateien laden", systemImage: "arrow.down.circle") { model.prepareLocalSpeech() }
                                .labelStyle(.iconOnly).buttonStyle(.borderless).frame(minWidth: 44, minHeight: 44)
                        }
                    }
                    if !model.speechModelMessage.isEmpty { Text(model.speechModelMessage).font(.caption).accessibilityIdentifier("appleSpeechStatus") }
                }
                Section("Modelldatei importieren") {
                    Button(model.installingModel ? "Modell wird geprüft …" : "Datei auswählen") { importing = true }
                        .disabled(model.installingModel)
                    if model.installingModel { ProgressView() }
                    if !model.modelMessage.isEmpty { Text(model.modelMessage).font(.caption).accessibilityIdentifier("modelMessage") }
                }
                #if DEBUG
                Section("Prototyp-Test") { Toggle("Feste Antwort ohne Spracherkennung", isOn: $model.fixedAnswer) }
                #endif
            }
            .scrollContentBackground(.hidden)
            .background(VoicePalette.background)
            .navigationTitle("Einstellungen")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Fertig") { dismiss() } } }
            .task { await model.refreshModels() }
            .fileImporter(isPresented: $importing, allowedContentTypes: [.data]) { result in
                if case .success(let url) = result { model.installModel(url) }
            }
        }
    }
    private func modelRow(_ item: InstalledModel) -> some View {
        let record = downloads.records[item.id]
        let busy = record?.busy == true
        return VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 8) {
                VStack(alignment: .leading, spacing: 3) {
                    Text(item.model.label).font(.subheadline.weight(.medium))
                    Text(ByteCountFormatter.string(fromByteCount: item.model.bytes, countStyle: .file) + (item.installed ? " · Installiert" : ""))
                        .font(.caption2).foregroundStyle(VoicePalette.secondaryText)
                }
                Spacer(minLength: 4)
                if busy {
                    Button("Download abbrechen", systemImage: "xmark.circle") { downloads.cancel(item.model) }
                        .labelStyle(.iconOnly).buttonStyle(.borderless).frame(minWidth: 44, minHeight: 44)
                        .accessibilityIdentifier("cancel-download-" + item.id)
                } else if item.installed {
                    Image(systemName: "checkmark.circle.fill").foregroundStyle(VoicePalette.accent).frame(width: 44)
                        .accessibilityIdentifier("installed-" + item.id).accessibilityLabel("Installiert")
                } else {
                    Button(item.model.label + " herunterladen", systemImage: "arrow.down.circle") { model.downloadModel(item.model) }
                        .labelStyle(.iconOnly).buttonStyle(.borderless).frame(minWidth: 44, minHeight: 44)
                        .disabled(model.installingModel).accessibilityIdentifier("download-" + item.id)
                }
            }
            if let record {
                if record.phase == .downloading {
                    ProgressView(value: min(1, max(0, Double(record.bytes) / Double(item.model.bytes))))
                    Text(record.message + " · " + ByteCountFormatter.string(fromByteCount: record.bytes, countStyle: .file) + " / " + ByteCountFormatter.string(fromByteCount: item.model.bytes, countStyle: .file))
                        .font(.caption2).foregroundStyle(VoicePalette.secondaryText).accessibilityIdentifier("download-status-" + item.id)
                } else {
                    HStack(spacing: 6) {
                        if record.phase == .verifying { ProgressView().controlSize(.small) }
                        Text(record.message).font(.caption2).foregroundStyle(VoicePalette.secondaryText)
                            .accessibilityIdentifier("download-status-" + item.id)
                    }
                }
            }
        }
    }
}
