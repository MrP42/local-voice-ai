import SwiftUI
import AVFoundation

struct SpeechVoiceOption: Identifiable {
    let id: String
    let name: String
    let language: String
    let quality: String
    init(_ voice: AVSpeechSynthesisVoice) {
        id = voice.identifier; name = voice.name; language = voice.language
        quality = voice.quality == .premium ? "Premium" : voice.quality == .enhanced ? "Erweitert" : "Standard"
    }
}

struct SpeechVoicePicker: View {
    @ObservedObject var model: VoiceModel
    @State private var germanOnly = true
    private var voices: [SpeechVoiceOption] { model.availableVoices.filter { !germanOnly || $0.language.hasPrefix("de") } }
    var body: some View {
        List {
            Section {
                Toggle("Nur Deutsch", isOn: $germanOnly)
                Button {
                    model.selectedVoiceID = ""
                } label: {
                    HStack { Text("Systemstimme"); Spacer(); if model.selectedVoiceID.isEmpty { Image(systemName: "checkmark") } }
                }.buttonStyle(.borderless).accessibilityIdentifier("voice-system")
                if !model.selectedVoiceID.isEmpty, !model.availableVoices.contains(where: { $0.id == model.selectedVoiceID }) {
                    Text("Gewählte Stimme derzeit nicht verfügbar. Antworten verwenden vorübergehend die Systemstimme.").font(.caption)
                }
            }
            Section("Verfügbare Stimmen") {
                ForEach(voices) { voice in
                    HStack(spacing: 8) {
                        Button { model.selectedVoiceID = voice.id } label: {
                            HStack {
                                VStack(alignment: .leading, spacing: 3) {
                                    Text(voice.name).font(.subheadline)
                                    Text(voice.language + " · " + voice.quality).font(.caption2).foregroundStyle(VoicePalette.secondaryText)
                                }
                                Spacer(minLength: 2)
                                if model.selectedVoiceID == voice.id { Image(systemName: "checkmark").font(.caption) }
                            }.contentShape(Rectangle())
                        }.buttonStyle(.borderless).accessibilityIdentifier("voice-" + voice.id)
                            .accessibilityValue(model.selectedVoiceID == voice.id ? "Ausgewählt" : "Nicht ausgewählt")
                        Button(model.previewingVoiceID == voice.id ? "Hörprobe stoppen" : "Hörprobe anhören", systemImage: model.previewingVoiceID == voice.id ? "stop.fill" : "play.fill") { model.previewVoice(voice.id) }
                            .labelStyle(.iconOnly).buttonStyle(.borderless).frame(minWidth: 44, minHeight: 44)
                            .accessibilityIdentifier("preview-voice-" + voice.id)
                    }
                }
                if voices.isEmpty { Text("Auf diesem Gerät sind noch keine passenden Stimmen verfügbar.").font(.caption) }
            }
            Section {
                Text("Auswahl und Hörprobe gelten auf diesem Gerät. Angezeigt werden die von Apple für Apps freigegebenen Stimmen; die Siri-/Apple-Intelligence-Auswahl kann abweichen.").font(.caption2).foregroundStyle(VoicePalette.secondaryText)
            }
        }
        .navigationTitle("Stimme")
        .task { model.refreshVoices() }
        #if os(iOS)
        .navigationBarTitleDisplayMode(.inline)
        .scrollContentBackground(.hidden).background(VoicePalette.background)
        #endif
    }
}

struct MicrophoneSettings: View {
    @ObservedObject var model: VoiceModel
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Picker("Mikrofonempfindlichkeit", selection: $model.microphoneSensitivity) {
                ForEach(MicrophoneSensitivity.allCases, id: \.self) { Text($0.label).tag($0) }
            }.accessibilityIdentifier("microphoneSensitivity")
            Toggle("Umgebungsgeräusche berücksichtigen", isOn: $model.automaticNoiseFloor)
                .accessibilityIdentifier("automaticNoiseFloor")
            Text("Steuert, wann Freisprechen Sprache vermutet. Unempfindlich hilft bei Lärm, Empfindlich bei leiser Stimme. Automatisch wird der Umgebungspegel zu Beginn kurz eingemessen. Keine Änderung der Aufnahmelautstärke.")
                .font(.caption2).foregroundStyle(VoicePalette.secondaryText)
        }
    }
}
