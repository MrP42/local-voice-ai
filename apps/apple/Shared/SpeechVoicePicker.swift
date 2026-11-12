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
    #if os(iOS)
    @ObservedObject private var phonon = PhononEngine.shared
    #endif
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
                if !model.selectedVoiceID.isEmpty, !model.selectedVoiceID.hasPrefix("phonon:"), !model.availableVoices.contains(where: { $0.id == model.selectedVoiceID }) {
                    Text("Gewählte Stimme derzeit nicht verfügbar. Antworten verwenden vorübergehend die Systemstimme.").font(.caption)
                }
            }
            #if os(iOS)
            ForEach(PhononEngine.languages, id: \.self) { language in
                if let names = phonon.voices[language] {
                    Section(PhononEngine.sectionTitle(language)) {
                        ForEach(names, id: \.self) { name in
                            let id = PhononEngine.identifier(language: language, voice: name)
                            HStack(spacing: 8) {
                                Button { model.selectedVoiceID = id } label: {
                                    HStack {
                                        Text(name).font(.subheadline)
                                        Spacer(minLength: 2)
                                        if model.selectedVoiceID == id { Image(systemName: "checkmark").font(.caption) }
                                    }.contentShape(Rectangle())
                                }.buttonStyle(.borderless)
                                    .accessibilityValue(model.selectedVoiceID == id ? "Ausgewählt" : "Nicht ausgewählt")
                                Button(model.previewingVoiceID == id ? "Hörprobe stoppen" : "Hörprobe anhören",
                                       systemImage: model.previewingVoiceID == id ? "stop.fill" : "play.fill") { model.previewPhonon(id) }
                                    .labelStyle(.iconOnly).buttonStyle(.borderless).frame(minWidth: 44, minHeight: 44)
                            }
                        }
                    }
                }
            }
            #endif
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
        .task {
            model.refreshVoices()
            #if os(iOS)
            await phonon.refresh()
            #endif
        }
        #if os(iOS)
        .navigationBarTitleDisplayMode(.inline)
        .scrollContentBackground(.hidden).background(VoicePalette.background)
        #endif
    }
}

struct MicrophoneSettings: View {
    @ObservedObject var model: VoiceModel
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            #if os(watchOS)
            Text("Mikrofonempfindlichkeit").font(.subheadline)
            Picker("Mikrofonempfindlichkeit", selection: $model.microphoneSensitivity) {
                ForEach(MicrophoneSensitivity.allCases, id: \.self) { Text($0.label).tag($0) }
            }.labelsHidden().accessibilityIdentifier("microphoneSensitivity")
            #else
            // Beschriftung links, Auswahl rechts in derselben Zeile; nur bei sehr großer Schrift untereinander.
            if dynamicTypeSize.isAccessibilitySize {
                Text("Mikrofonempfindlichkeit").accessibilityHidden(true)
                sensitivityMenu
            } else {
                HStack(alignment: .center, spacing: 12) {
                    Text("Mikrofonempfindlichkeit").lineLimit(1).minimumScaleFactor(0.8)
                        .frame(maxWidth: .infinity, alignment: .leading).accessibilityHidden(true)
                    sensitivityMenu
                }
            }
            #endif
            VoiceToggleRow(title: "Umgebungsgeräusche berücksichtigen", isOn: $model.automaticNoiseFloor, identifier: "automaticNoiseFloor")
            Text("Steuert, wann Freisprechen Sprache vermutet. Unempfindlich hilft bei Lärm, Empfindlich bei leiser Stimme. Automatisch wird der Umgebungspegel zu Beginn kurz eingemessen. Keine Änderung der Aufnahmelautstärke.")
                .font(.caption2).foregroundStyle(VoicePalette.secondaryText)
        }
    }
    #if !os(watchOS)
    /// Die Auswahl als Menü mit eigener Beschriftung (gewählter Wert und Pfeile): ohne den Innenabstand der Systemauswahl, damit sie
    /// rechts bündig mit den Schaltern abschließt und ihre Breite nur vom gewählten Wert abhängt.
    private var sensitivityMenu: some View {
        Menu {
            Picker("Mikrofonempfindlichkeit", selection: $model.microphoneSensitivity) {
                ForEach(MicrophoneSensitivity.allCases, id: \.self) { Text($0.label).tag($0) }
            }
        } label: {
            HStack(spacing: 4) {
                Text(model.microphoneSensitivity.label)
                Image(systemName: "chevron.up.chevron.down").font(.footnote.weight(.semibold)).accessibilityHidden(true)
            }.frame(minHeight: 44).contentShape(Rectangle())
        }
        .fixedSize().padding(.vertical, -8)   // 44 pt Tippfläche, ohne die Zeile höher zu machen
        .accessibilityLabel("Mikrofonempfindlichkeit").accessibilityValue(model.microphoneSensitivity.label)
        .accessibilityIdentifier("microphoneSensitivity")
    }
    #endif
}
