import SwiftUI

// Gemeinsame Modellwahl (lva#801 T3, lva#804): Auswahl je Zweck. Wird von Besprechung (Protokoll,
// Zusammenfassung) und Sprechen gleich benutzt. Daten kommen vom Katalog; die Ansicht schreibt nur die
// gewählte Route. Alles außer „Auf diesem iPhone“ braucht je Besprechung bzw. Gespräch eine Zustimmung
// (ModelConsentSheet); verbindlich prüft ModelGate, die Anzeige hier ist nur Hilfe.

struct ModelPickerView: View {
    let purpose: ModelPurpose
    let choices: [ModelChoice]
    let policy: ModelPolicy
    @Binding var selectedKey: String?
    /// Öffnet die Endpunkt-Einstellungen (Schlüssel, Adresse). `nil` blendet den Eintrag aus.
    var onAddEndpoint: (() -> Void)? = nil

    private enum Group: Int, CaseIterable {
        case device, network, pc, cloud
        var title: String {
            switch self {
            case .device: return "Auf diesem iPhone"
            case .network: return "Im eigenen Netz"
            case .pc: return "Über deinen PC"
            case .cloud: return "Cloud-Anbieter"
            }
        }
        var footer: String? {
            switch self {
            case .device: return "Bleibt vollständig auf dem Gerät."
            case .network: return "Ollama, LM Studio oder ein anderer Server in deinem Netz. Vor dem ersten Senden fragt die App nach deiner Zustimmung."
            case .pc: return "Nutzt die am PC eingerichteten Modelle und Abos. Der PC muss gekoppelt und erreichbar sein."
            case .cloud: return "Nutzt deinen eigenen Schlüssel. Vor dem ersten Senden fragt die App nach deiner Zustimmung."
            }
        }
    }

    private func group(of choice: ModelChoice) -> Group {
        switch choice.route {
        case .local: return .device
        case .pcRelay: return .pc
        case .endpoint: return choice.ownNetworkHint ? .network : .cloud
        }
    }

    private var effective: ModelChoice? {
        ModelSelection.resolve(storedKey: selectedKey, choices: choices, purpose: purpose, policy: policy)
    }

    var body: some View {
        List {
            ForEach(Group.allCases, id: \.self) { group in
                let rows = choices.filter { self.group(of: $0) == group }
                if !rows.isEmpty || (group == .cloud && onAddEndpoint != nil && allowsExternal) {
                    Section {
                        ForEach(rows) { choice in row(choice) }
                        if group == .cloud, let onAddEndpoint, allowsExternal {
                            Button(action: onAddEndpoint) {
                                Label("Anbieter hinzufügen", systemImage: "plus.circle").frame(minHeight: 44)
                            }
                        }
                    } header: {
                        Text(group.title)
                    } footer: {
                        if let footer = group.footer { Text(footer) }
                    }
                }
            }
            if policy.locked && !policy.externalAllowed {
                Section {
                    Label("Externe Modelle sind in diesem Profil gesperrt.", systemImage: "lock")
                        .font(.footnote).foregroundStyle(.secondary)
                }
            }
        }
        .navigationTitle(purposeTitle)
        .navigationBarTitleDisplayMode(.inline)
    }

    private var allowsExternal: Bool { policy.externalAllowed && !purpose.localOnly }

    private var purposeTitle: String {
        switch purpose {
        case .meetingMinutes: return "Modell für Protokolle"
        case .meetingSummary: return "Modell für Zusammenfassungen"
        case .liveState: return "Modell für den Live-Stand"
        case .talk: return "Modell zum Sprechen"
        case .textPrep: return "Modell zum Aufbereiten"
        }
    }

    @ViewBuilder
    private func row(_ choice: ModelChoice) -> some View {
        let blocked = blockReason(choice)
        let selected = effective?.id == choice.id
        Button {
            guard blocked == nil else { return }
            selectedKey = choice.id
        } label: {
            HStack(alignment: .center, spacing: 12) {
                Image(systemName: icon(choice))
                    .font(.body).frame(width: 28)
                    .foregroundStyle(choice.isExternal ? Color.orange : Color.accentColor)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 2) {
                    Text(choice.label).foregroundStyle(blocked == nil ? Color.primary : Color.secondary)
                    Text(blocked ?? detail(choice)).font(.footnote).foregroundStyle(.secondary).lineLimit(2)
                }
                Spacer(minLength: 8)
                if selected {
                    Image(systemName: "checkmark").font(.body.weight(.semibold)).foregroundStyle(Color.accentColor)
                }
            }
            .frame(minHeight: 44)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(blocked != nil)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(selected ? .isSelected : [])
        .accessibilityHint(blocked ?? (choice.isExternal ? "Daten verlassen das Gerät, Zustimmung nötig" : ""))
    }

    private func blockReason(_ choice: ModelChoice) -> String? {
        if purpose.localOnly && !choice.route.isLocal { return "Diese Auswertung läuft nur auf dem Gerät." }
        if !policy.permits(choice) { return "Im Profil gesperrt" }
        return nil
    }

    private func icon(_ choice: ModelChoice) -> String {
        switch choice.route {
        case .local: return "iphone"
        case .pcRelay: return "desktopcomputer"
        case .endpoint: return choice.ownNetworkHint ? "network" : "cloud"
        }
    }

    private func detail(_ choice: ModelChoice) -> String {
        var parts: [String] = []
        if let vendor = choice.vendor { parts.append(vendor) }
        if let region = choice.region { parts.append("Verarbeitung: \(region)") }
        parts.append("bis \(Self.tokens(choice.contextTokens)) Token")
        return parts.joined(separator: " · ")
    }

    static func tokens(_ n: Int) -> String {
        n >= 1000 ? "\(n / 1000) Tsd." : "\(n)"
    }
}

/// Kompakte Zeile für Einstellungen und Auswertungsdialoge: zeigt die aktuelle Wahl und führt zur Auswahl.
struct ModelChoiceLink: View {
    let purpose: ModelPurpose
    let choices: [ModelChoice]
    let policy: ModelPolicy
    @Binding var selectedKey: String?
    var onAddEndpoint: (() -> Void)? = nil

    var body: some View {
        let current = ModelSelection.resolve(storedKey: selectedKey, choices: choices, purpose: purpose, policy: policy)
        NavigationLink {
            ModelPickerView(purpose: purpose, choices: choices, policy: policy, selectedKey: $selectedKey, onAddEndpoint: onAddEndpoint)
        } label: {
            LabeledContent("Modell") {
                HStack(spacing: 6) {
                    if current?.isExternal == true {
                        Image(systemName: "cloud").foregroundStyle(.orange).accessibilityLabel("extern")
                    }
                    Text(current?.label ?? "Kein Modell installiert")
                }
            }
            .frame(minHeight: 44)
        }
    }
}
