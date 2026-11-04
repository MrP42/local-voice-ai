import SwiftUI

/// Bestätigung beim Übernehmen einer fertigen Aufnahme (Teilen-Menü, „Öffnen in“,
/// Dateiauswahl). Die Person bestätigt, dass alle Beteiligten zugestimmt haben;
/// das ersetzt deren Zustimmung nicht, sondern dokumentiert sie (Epic #247, P4).
/// Zweck, Verantwortlicher und Löschfrist werden in der App-Gruppe gemerkt,
/// die KI-Auswertung ist je Aufnahme Opt-in und nie vorbelegt.
struct ImportConsentView: View {
    let fileName: String
    let channel: ConsentRecord.Channel
    let onConfirm: (ConsentRecord) -> Void
    let onCancel: () -> Void

    @AppStorage("consent.purpose", store: ImportConsentView.store) private var purpose = "Protokoll der Besprechung"
    @AppStorage("consent.controller", store: ImportConsentView.store) private var controller = ""
    @AppStorage("consent.retentionDays", store: ImportConsentView.store) private var retentionDays = 90
    @State private var ai = false
    @State private var allAgreed = false

    static let store = UserDefaults(suiteName: MeetingInbox.appGroup)

    private var record: ConsentRecord {
        ConsentRecord(confirmedAt: Date(), channel: channel, reason: .import,
                      scopes: ai ? [.recording, .transcription, .aiAnalysis] : [.recording, .transcription],
                      purpose: purpose.trimmingCharacters(in: .whitespacesAndNewlines),
                      controller: controller.trimmingCharacters(in: .whitespacesAndNewlines),
                      retentionDays: retentionDays)
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Text(fileName).font(.headline).lineLimit(3)
                    Text("Die Datei wird auf dieses Gerät kopiert und dort transkribiert. Nichts verlässt das Gerät.")
                        .font(.subheadline).foregroundStyle(.secondary)
                }
                Section("Angaben für den Nachweis") {
                    TextField("Zweck", text: $purpose)
                    TextField("Verantwortlich (Stelle oder Person)", text: $controller)
                    Stepper("Löschen nach \(retentionDays) Tagen", value: $retentionDays, in: 1...3650)
                }
                Section {
                    Toggle("Mit KI auswerten (Zusammenfassung, Aufgaben, offene Fragen)", isOn: $ai)
                        .frame(minHeight: 44)
                } footer: {
                    Text("Standard aus. Ohne Zustimmung wird nur transkribiert.")
                }
                Section {
                    Toggle("Alle Beteiligten haben der Aufnahme zugestimmt", isOn: $allAgreed)
                        .frame(minHeight: 44)
                        .accessibilityIdentifier("importConsentAll")
                } footer: {
                    Text(ConsentNotice.text(purpose: purpose.isEmpty ? "–" : purpose, controller: controller.isEmpty ? "–" : controller,
                                            retentionDays: retentionDays, ai: ai))
                }
            }
            .navigationTitle("Aufnahme übernehmen")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Abbrechen", action: onCancel) }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Übernehmen") { onConfirm(record) }
                        .disabled(!allAgreed || !record.isComplete)
                        .accessibilityIdentifier("importConsentConfirm")
                }
            }
        }
    }
}
