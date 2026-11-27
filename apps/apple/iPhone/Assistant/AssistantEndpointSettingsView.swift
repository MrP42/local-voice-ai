import SwiftUI

// Gesprächsmodus (Paket M): Einstellungen für den eigenen OpenAI-kompatiblen Endpunkt.
// Als Formular gebaut, das in einem NavigationStack geschoben wird (Titel „Eigener Endpunkt“).

struct AssistantEndpointSettingsView: View {
    @ObservedObject private var store = AssistantEndpointStore.shared
    @State private var address = ""
    @State private var model = ""
    @State private var label = ""
    @State private var contextText = String(AssistantEndpoint.defaultContextTokens)
    @State private var keyText = ""
    @State private var status = Status.idle
    @State private var loaded = false
    @State private var testTask: Task<Void, Never>?
    @State private var confirmingRemoval = false
    /// Zuletzt von der Schnellwahl geschriebene Werte. Weicht das Feld davon ab, stammt es vom Nutzer und bleibt.
    @State private var presetModel: String?
    @State private var presetContext: String?

    private enum Status: Equatable {
        case idle, testing, saved
        case ok(String)
        case failed(String)
    }

    init() {}

    var body: some View {
        Form {
            Section {
                ForEach(Preset.all) { item in
                    Button { apply(item) } label: {
                        HStack {
                            Label(item.title, systemImage: item.symbol)
                            Spacer()
                            if selectedPreset == item.id {
                                Image(systemName: "checkmark").foregroundStyle(VoicePalette.accent).accessibilityHidden(true)
                            }
                        }
                        .frame(minHeight: 44)
                    }
                    .accessibilityAddTraits(selectedPreset == item.id ? .isSelected : [])
                }
            } header: {
                Text("Schnellwahl")
            } footer: {
                Text("ChatGPT- und Claude-Abos melden sich am PC an. Auf dem iPhone nutzt du sie in der Modellwahl unter „Über deinen PC“, sobald der PC gekoppelt ist. Hier brauchst du einen API-Schlüssel des Anbieters; er wird getrennt vom Abo abgerechnet.")
            }

            Section {
                labeled("Adresse") {
                    TextField("Adresse", text: $address, prompt: Text(verbatim: "http://192.168.178.20:11434/v1"))
                        .keyboardType(.URL).textContentType(.URL)
                        .textInputAutocapitalization(.never).autocorrectionDisabled()
                }
                labeled("Modell") {
                    TextField("Modell", text: $model, prompt: Text(modelPrompt))
                        .textInputAutocapitalization(.never).autocorrectionDisabled()
                }
                labeled("Anzeigename (optional)") {
                    TextField("Anzeigename", text: $label, prompt: Text("Standard: Modellname"))
                }
                labeled("Kontextgröße (Token)") {
                    TextField("Kontextgröße", text: $contextText).keyboardType(.numberPad)
                }
                labeled("Schlüssel (optional)") {
                    SecureField("Schlüssel", text: $keyText,
                                prompt: Text(store.hasKey ? "Gespeichert, leer lassen zum Behalten" : "Nur falls der Anbieter einen verlangt"))
                        .textInputAutocapitalization(.never).autocorrectionDisabled()
                }
                if store.hasKey {
                    Button("Gespeicherten Schlüssel entfernen", role: .destructive) { store.removeKey() }.frame(minHeight: 44)
                }
            } header: {
                Text("Endpunkt")
            } footer: {
                VStack(alignment: .leading, spacing: 6) {
                    hint
                    Text("Die Kontextgröße muss zum Server passen. Bei Ollama stellst du sie dort ein. Der Schlüssel liegt nur im Schlüsselbund dieses iPhones.")
                }
            }

            Section {
                Button { test() } label: {
                    Label("Verbindung prüfen", systemImage: "bolt.horizontal").frame(minHeight: 44)
                }
                .disabled(status == .testing)
                statusView
            }

            Section {
                Button { save() } label: {
                    Label("Speichern", systemImage: "checkmark").frame(minHeight: 44)
                }
                if store.endpoint != nil {
                    Button("Entfernen", role: .destructive) { confirmingRemoval = true }.frame(minHeight: 44)
                }
            }
        }
        .scrollContentBackground(.hidden)
        .background(VoicePalette.background)
        .navigationTitle("Eigener Endpunkt")
        .navigationBarTitleDisplayMode(.inline)
        .onAppear(perform: loadOnce)
        .onDisappear { testTask?.cancel() }
        .confirmationDialog("Eigenen Endpunkt entfernen?", isPresented: $confirmingRemoval, titleVisibility: .visible) {
            Button("Entfernen", role: .destructive) { remove() }
            Button("Abbrechen", role: .cancel) {}
        } message: {
            Text("Adresse, Schlüssel und Freigabe werden von diesem iPhone gelöscht.")
        }
    }

    // MARK: Teile

    /// Schnellwahl. Modell und Kontext stammen vom Preset, solange der Nutzer sie nicht selbst geändert hat.
    struct Preset: Identifiable, Equatable {
        let id: String, title: String, symbol: String, address: String, model: String, context: Int
        static let all = [
            Preset(id: "ollama", title: "Ollama am PC", symbol: "desktopcomputer", address: "http://<IP>:11434/v1",
                   model: "", context: AssistantEndpoint.defaultContextTokens),
            Preset(id: "anthropic", title: "Anthropic Claude (API-Schlüssel)", symbol: "cloud",
                   address: "https://api.anthropic.com/v1", model: "claude-sonnet-5-5", context: 200_000),
            Preset(id: "openai", title: "OpenAI (API-Schlüssel)", symbol: "cloud",
                   address: "https://api.openai.com/v1", model: "", context: 128_000),
            Preset(id: "other", title: "Anderer OpenAI-kompatibler Anbieter", symbol: "cloud",
                   address: "https://", model: "", context: AssistantEndpoint.defaultContextTokens),
        ]
    }

    /// Markiert, welches Preset zur eingetragenen Adresse passt („Anderer“ nur, solange nichts anderes passt).
    private var selectedPreset: String? {
        let typed = address.trimmingCharacters(in: .whitespaces)
        if typed == Preset.all[0].address { return "ollama" }
        guard let parts = URLComponents(string: typed), let scheme = parts.scheme?.lowercased() else { return nil }
        // Exakter Host statt Präfix: „api.anthropic.com.example“ ist nicht Anthropic.
        switch (scheme, parts.host?.lowercased()) {
        case ("https", "api.anthropic.com"): return "anthropic"
        case ("https", "api.openai.com"): return "openai"
        default: break
        }
        if parts.port == 11434 { return "ollama" }
        return scheme == "https" ? "other" : nil
    }

    private var modelPrompt: String {
        switch selectedPreset {
        case "anthropic": return "zum Beispiel claude-sonnet-5-5"
        case "openai", "other": return "Modellname laut Anbieter"
        default: return "zum Beispiel llama3.1:8b"
        }
    }

    private func apply(_ item: Preset) {
        address = item.address
        if model.trimmingCharacters(in: .whitespaces).isEmpty || model == presetModel {
            model = item.model; presetModel = item.model
        }
        if contextText == presetContext || Int(contextText) == nil || contextText == String(AssistantEndpoint.defaultContextTokens) {
            contextText = String(item.context); presetContext = contextText
        }
        // Ein getippter Schlüssel gehört zum vorigen Anbieter und darf nie an den neuen gehen.
        keyText = ""
        status = .idle
    }

    private func labeled<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title).font(.caption).foregroundStyle(VoicePalette.secondaryText).accessibilityHidden(true)
            content().frame(minHeight: 44)
        }
    }

    /// Hinweis je nach Einordnung der eingegebenen Adresse.
    @ViewBuilder private var hint: some View {
        if address.contains("<") || address.contains(">") {
            Text("Ersetze <IP> durch die Adresse deines PCs, zum Beispiel 192.168.178.20. Du findest sie in den Netzwerkeinstellungen des PCs.")
        } else if let host = AssistantEndpointStore.hostName(of: address) {
            if AssistantEndpointStore.isLocalHost(host) {
                Text("Bleibt in deinem Netz.")
            } else {
                Text("Fragen und passende Auszüge gehen an \(host). Du wirst vor dem ersten Gespräch gefragt.")
                if address.lowercased().hasPrefix("http://") {
                    Text("Außerhalb deines Netzes ist https nötig.").foregroundStyle(VoicePalette.danger)
                }
            }
        } else {
            Text("Adresse des Servers bis einschließlich /v1. Lokal ist es meist http, bei einem Anbieter https.")
        }
    }

    @ViewBuilder private var statusView: some View {
        switch status {
        case .idle:
            EmptyView()
        case .testing:
            HStack(spacing: 8) { ProgressView(); Text("Prüfe die Verbindung …") }
        case .saved:
            Label("Gespeichert.", systemImage: "checkmark.circle.fill").foregroundStyle(VoicePalette.success)
        case .ok(let text):
            Label(text, systemImage: "checkmark.circle.fill").foregroundStyle(VoicePalette.success)
        case .failed(let text):
            Label(text, systemImage: "exclamationmark.triangle.fill").foregroundStyle(VoicePalette.danger)
        }
    }

    // MARK: Aktionen

    private func loadOnce() {
        guard !loaded else { return }
        loaded = true
        guard let endpoint = store.endpoint else { return }
        address = endpoint.baseURL
        model = endpoint.model
        label = endpoint.label == endpoint.model ? "" : endpoint.label
        contextText = String(endpoint.contextTokens)
    }

    /// Eingaben als geprüfter Endpunkt; bei Fehler steht die Meldung im Status.
    private func draft() -> AssistantEndpoint? {
        guard let tokens = Int(contextText.trimmingCharacters(in: .whitespacesAndNewlines)) else {
            status = .failed(AssistantEndpointError.badContext.localizedDescription)
            return nil
        }
        do {
            return try AssistantEndpointStore.validated(AssistantEndpoint(baseURL: address, model: model, label: label, contextTokens: tokens))
        } catch {
            status = .failed(AssistantEndpointError.describe(error))
            return nil
        }
    }

    private func test() {
        testTask?.cancel()
        guard let endpoint = draft(), let host = AssistantEndpointStore.hostName(of: endpoint.baseURL) else { return }
        let typed = keyText.trimmingCharacters(in: .whitespacesAndNewlines)
        let key = typed.isEmpty ? store.storedKey(forHost: host) : typed
        status = .testing
        testTask = Task {
            let probe = AssistantEndpointModel(endpoint: endpoint, apiKey: key)
            let start = ContinuousClock.now
            do {
                for try await _ in probe.stream(instructions: "Du bist ein Verbindungstest. Antworte mit einem Wort.",
                                                prompt: "Antworte mit OK.", maxTokens: 64) {
                    let parts = start.duration(to: .now).components
                    let seconds = Double(parts.seconds) + Double(parts.attoseconds) / 1e18
                    status = .ok("Verbindung steht. Erstes Zeichen nach \(seconds.formatted(.number.precision(.fractionLength(1)))) s.")
                    return
                }
                status = .failed(AssistantEndpointError.describe(AssistantEndpointError.emptyAnswer))
            } catch is CancellationError {
                status = .idle
            } catch {
                status = .failed(AssistantEndpointError.describe(error))
            }
        }
    }

    private func save() {
        guard let endpoint = draft() else { return }
        let typed = keyText.trimmingCharacters(in: .whitespacesAndNewlines)
        do {
            try store.save(endpoint, apiKey: typed.isEmpty ? nil : typed)
            keyText = ""
            address = endpoint.baseURL
            model = endpoint.model
            status = .saved
        } catch {
            status = .failed(AssistantEndpointError.describe(error))
        }
    }

    private func remove() {
        testTask?.cancel()
        store.remove()
        address = ""
        model = ""
        label = ""
        contextText = String(AssistantEndpoint.defaultContextTokens)
        keyText = ""
        status = .idle
    }
}
