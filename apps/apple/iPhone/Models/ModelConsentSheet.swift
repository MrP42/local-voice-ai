import SwiftUI

// Gemeinsame Modellwahl (lva#801 T3, lva#804): Zustimmung vor dem ersten Senden an einen externen Anbieter,
// je Besprechung bzw. Gespräch und Zweck. Der Nachweis entsteht erst mit „Zustimmen“; „Auf dem iPhone
// erstellen“ ist gleichwertig sichtbar, damit nie stillschweigend extern gerechnet wird.

struct ModelConsentSheet: View {
    enum Decision { case grant(ModelConsentRecord), local, cancel }

    let choice: ModelChoice
    /// Aus `ModelAccessState.consentScope(...)`: geprüfter Geltungsbereich und aufgelöstes Ziel (nie aus der Anzeige).
    let scope: ModelConsentScope
    let target: ModelResolvedTarget
    /// Lokale Alternative vorhanden (Knopf „Auf dem iPhone erstellen“).
    let localAvailable: Bool
    let onDecision: (Decision) -> Void

    @ScaledMetric(relativeTo: .footnote) private var factIconWidth: CGFloat = 20
    @ScaledMetric(relativeTo: .title2) private var headerIconSize: CGFloat = 44
    private var vendor: String { target.vendor }
    private var purpose: ModelPurpose { scope.purpose }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    // Frage als Überschrift im Inhalt: bricht bei großer Schrift um statt im Titel gekürzt zu werden.
                    Text("An \(vendor) senden?").font(.title2.bold()).fixedSize(horizontal: false, vertical: true)
                        .accessibilityAddTraits(.isHeader)
                    HStack(spacing: 12) {
                        Image(systemName: icon).font(.title2).foregroundStyle(.orange)
                            .frame(width: headerIconSize, height: headerIconSize)
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
                        Text("Zustimmen und senden").font(.body.weight(.semibold))
                            .multilineTextAlignment(.center).fixedSize(horizontal: false, vertical: true)
                            .frame(maxWidth: .infinity, minHeight: 44).foregroundStyle(VoicePalette.ink)
                    }
                    // Eigene Farben statt .borderedProminent: der globale Tint machte die Schrift unlesbar.
                    .buttonStyle(VoicePrimaryButtonStyle(recording: false))
                    .accessibilityIdentifier("consentGrant")
                    if localAvailable {
                        Button { onDecision(.local) } label: {
                            Text("Auf dem iPhone erstellen")
                                .multilineTextAlignment(.center).fixedSize(horizontal: false, vertical: true)
                                .frame(maxWidth: .infinity, minHeight: 44).foregroundStyle(VoicePalette.text)
                                .padding(.horizontal, 16).padding(.vertical, 4)
                                .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(VoicePalette.secondaryText.opacity(0.6)))
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("consentLocal")
                    }
                }
                .padding(.horizontal, 16).padding(.vertical, 12)
                .background(.bar)
            }
            .navigationTitle("Zustimmung")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Abbrechen") { onDecision(.cancel) } }
            }
        }
        // Volle Höhe: Inhalt und Knöpfe dürfen sich nicht verdecken.
        .presentationDetents([.large])
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
            Image(systemName: symbol).foregroundStyle(.secondary).frame(width: factIconWidth).accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.footnote.weight(.semibold))
                Text(text).font(.footnote).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
        }
        .accessibilityElement(children: .combine)
    }
}
