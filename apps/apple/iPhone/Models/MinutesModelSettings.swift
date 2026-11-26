import SwiftUI

/// Einstellungen „Besprechungsprotokolle“ (lva#801 T3b): Kundenprofil, Modellwahl, eigener Server.
/// Die Wahl gilt für neue Besprechungen; jede externe Route fragt je Besprechung nach Zustimmung.
struct MinutesModelSettings: View {
    @AppStorage(ModelPolicyProfile.userKey) private var allowExternal = false
    @AppStorage(MeetingLibrary.backgroundSettingKey) private var background = true
    @AppStorage(ModelSelection.settingsKey(.meetingMinutes)) private var selectedKey: String?
    @State private var choices: [ModelChoice] = []
    @State private var policy: ModelPolicy = .unknown
    @State private var loaded = false

    var body: some View {
        Section {
            // Gesperrt (Organisation oder noch nicht geladen): kein Schalter, nur die Erklärung unten.
            if !policy.locked {
                Toggle("Externe Modelle erlauben", isOn: $allowExternal)
                    .accessibilityIdentifier("allowExternalModels")
            }
            ModelChoiceLink(purpose: .meetingMinutes, choices: choices, policy: policy, selectedKey: $selectedKey,
                            onAddEndpoint: { Task { await reload() } })
            Toggle("Im Hintergrund weiterverarbeiten", isOn: $background)
                .accessibilityIdentifier("meetingBackgroundProcessing")
        } header: {
            Text("Besprechungsprotokolle")
        } footer: {
            if loaded {
                Text(ModelPolicyProfile.explanation(policy) + " Der Live-Stand während der Aufnahme bleibt immer auf dem iPhone.\n\n"
                     + "Im Hintergrund rechnet das iPhone langsamer und braucht mehr Akku. Beendet iOS die Verarbeitung, geht sie am Ladekabel oder beim nächsten Öffnen weiter.")
            }
        }
        .task { await reload() }
        .onChange(of: allowExternal) { _, _ in Task { await reload() } }
        .onChange(of: background) { _, _ in MeetingLibrary.shared.updateBackgroundSetting() }
    }

    private func reload() async {
        let catalog = AppModelCatalog.shared
        choices = await catalog.choices(for: .meetingMinutes)
        policy = catalog.policy
        loaded = true
    }
}
