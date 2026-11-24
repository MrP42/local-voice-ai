import SwiftUI

// Gemeinsame Modellwahl (lva#801 T3, lva#804): Zustimmung vor dem ersten Senden an einen externen Anbieter,
// je Besprechung bzw. Gespräch und Zweck. Der Nachweis entsteht erst mit „Zustimmen“; „Auf dem iPhone
// erstellen“ ist gleichwertig sichtbar, damit nie stillschweigend extern gerechnet wird.

struct ModelConsentSheet: View {
    enum Decision { case grant(ModelConsentRecord), local, cancel }

    let choice: ModelChoice
    /// Aus `ModelGate.consentScope(...)`: geprüfter Geltungsbereich und aufgelöstes Ziel (nie aus der Anzeige).
    let scope: ModelConsentScope
    let target: ModelResolvedTarget
    /// Lokale Alternative vorhanden (Knopf „Auf dem iPhone erstellen“).
    let localAvailable: Bool
    let onDecision: (Decision) -> Void

    private var vendor: String { target.vendor }
    private var purpose: ModelPurpose { scope.purpose }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    HStack(spacing: 12) {
                        Image(systemName: icon).font(.title2).foregroundStyle(.orange)
                            .frame(width: 44, height: 44)
                            .background(Color.orange.opacity(0.12), in: RoundedRectangle(cornerRadius: 10))
                            .accessibilityHidden(true)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(choice.label).font(.headline)
                            Text(via).font(.subheadline).foregroundStyle(.secondary)
                        }
                    }

                    Text(ModelConsentRecord.summary(choice: choice, scope: scope, vendor: vendor, region: target.region))
                        .font(.body).fixedSize(horizontal: false, vertical: true)

                    VStack(alignment: .leading, spacing: 10) {
                        fact("doc.text", "Was gesendet wird", scope.dataCategories.joined(separator: ", "))
                        fact("link", "Ziel", scope.destination == "pc-relay" ? "Dein gekoppelter PC" : scope.destination)
                        if let region = target.region { fact("globe.europe.africa", "Ort der Verarbeitung", region) }
                        fact("person.badge.shield.checkmark", "Gilt für", validity)
                        fact("arrow.uturn.backward", "Danach", "Beim nächsten Mal fragt die App erneut. Bereits Gesendetes bleibt beim Anbieter nach dessen Regeln.")
                    }
                    .padding(14)
                    .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 12))
                }
                .padding(.horizontal, 16).padding(.vertical, 20)
            }
            .safeAreaInset(edge: .bottom) {
                VStack(spacing: 10) {
                    Button {
                        onDecision(.grant(ModelConsentRecord(scope: scope, grantedAt: Date())))
                    } label: {
                        Text("Zustimmen und senden").frame(maxWidth: .infinity, minHeight: 44)
                    }
                    .buttonStyle(.borderedProminent)
                    if localAvailable {
                        Button { onDecision(.local) } label: {
                            Text("Auf dem iPhone erstellen").frame(maxWidth: .infinity, minHeight: 44)
                        }
                        .buttonStyle(.bordered)
                    }
                }
                .padding(.horizontal, 16).padding(.vertical, 12)
                .background(.bar)
            }
            .navigationTitle("An \(vendor) senden?")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Abbrechen") { onDecision(.cancel) } }
            }
        }
        .presentationDetents([.medium, .large])
        .interactiveDismissDisabled()
    }

    private var icon: String { if case .pcRelay = choice.route { return "desktopcomputer" }; return "cloud" }

    private var via: String {
        if case .pcRelay = choice.route { return "\(vendor) über deinen PC" }
        return vendor
    }

    private var validity: String {
        switch purpose {
        case .meetingMinutes, .meetingSummary, .liveState: return "Nur diese Besprechung und diese Auswertung"
        case .talk: return "Nur dieses Gespräch"
        case .textPrep: return "Nur diesen Text"
        }
    }

    private func fact(_ symbol: String, _ title: String, _ text: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Image(systemName: symbol).foregroundStyle(.secondary).frame(width: 20).accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.footnote.weight(.semibold))
                Text(text).font(.footnote).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
        }
        .accessibilityElement(children: .combine)
    }
}
