import SwiftUI

// Assistenten im Gespräch „Sprechen“ (lva#556, Paket G2): das Blatt „Assistenten“. Eigene Assistenten und Vorlagen, der aktive mit Haken;
// Antippen wählt (und beginnt ein neues Gespräch, mit vorheriger Rückfrage, wenn es schon eines gibt). Je Zeile ein Menü zum Bearbeiten,
// Duplizieren und Löschen (eigene) bzw. Ansehen und Kopieren (Vorlagen).

struct AssistantProfilesView: View {
    /// Schließt das Blatt.
    let close: () -> Void

    @ObservedObject private var profiles = AssistantProfiles.shared
    @ObservedObject private var talk = AssistantController.shared
    @State private var editing: AssistantEditorTarget?
    @State private var switching: AssistantProfile?
    @State private var deleting: AssistantProfile?
    @State private var failure: String?

    var body: some View {
        NavigationStack {
            List {
                Section {
                    if profiles.own.isEmpty {
                        Text("Noch keine eigenen Assistenten.")
                            .foregroundStyle(VoicePalette.secondaryText)
                            .accessibilityIdentifier("assistantProfilesNoOwn")
                    }
                    ForEach(profiles.own, id: \.id) { profile in row(profile) }
                } header: {
                    Text("Eigene")
                } footer: {
                    if profiles.rejectedCount > 0 {
                        Text(rejectedHint)
                            .accessibilityIdentifier("assistantProfilesRejected")
                    }
                }

                Section("Vorlagen") {
                    ForEach(profiles.templates, id: \.id) { profile in row(profile) }
                }

                Section {
                    Button { newProfile() } label: {
                        Label("Neuer Assistent", systemImage: "plus").frame(minHeight: 44)
                    }
                    .accessibilityIdentifier("assistantProfilesNew")
                }
            }
            .navigationTitle("Assistenten")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Fertig") { close() }.accessibilityIdentifier("assistantProfilesDone")
                }
            }
            .sheet(item: $editing) { target in
                AssistantProfileEditorView(target: target, close: { editing = nil }, openCopy: { copy in
                    // Das Blatt des Editors schließt zuerst, dann öffnet die Kopie zum Bearbeiten.
                    DispatchQueue.main.async { editing = AssistantEditorTarget(profile: copy) }
                })
            }
            .alert(switching.map { "Mit ‚\($0.name)‘ beginnt ein neues Gespräch." } ?? "", isPresented: Binding(get: { switching != nil }, set: { if !$0 { switching = nil } }),
                   presenting: switching) { profile in
                Button("Abbrechen", role: .cancel) {}
                Button("Wechseln") { choose(profile) }
            } message: { _ in
                Text("Der bisherige Verlauf wird gelöscht.")
            }
            .confirmationDialog(deleting.map { "‚\($0.name)‘ löschen?" } ?? "", isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }),
                                titleVisibility: .visible, presenting: deleting) { profile in
                Button("Löschen", role: .destructive) { remove(profile) }
                Button("Abbrechen", role: .cancel) {}
            } message: { profile in
                Text(profile.id == profiles.active.id
                     ? "Dieser Assistent ist aktiv. Danach gilt „Alles“, und ein neues Gespräch beginnt."
                     : "Der Assistent wird von diesem Gerät gelöscht.")
            }
            .alert("Nicht möglich", isPresented: Binding(get: { failure != nil }, set: { if !$0 { failure = nil } })) {
                Button("OK", role: .cancel) {}
            } message: {
                Text(failure ?? "")
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("assistantProfilesSheet")
    }

    /// „N Assistenten konnten nicht gelesen werden.“ (bei einem: „Ein Assistent konnte nicht gelesen werden.“). Die Dateien bleiben liegen.
    private var rejectedHint: String {
        let count = profiles.rejectedCount
        return count == 1 ? "Ein Assistent konnte nicht gelesen werden." : "\(count) Assistenten konnten nicht gelesen werden."
    }

    // MARK: Zeile

    private func row(_ profile: AssistantProfile) -> some View {
        let active = profile.id == profiles.active.id
        let template = AssistantProfiles.isTemplate(profile)
        return HStack(spacing: 4) {
            Button { tap(profile) } label: {
                HStack(spacing: 12) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(profile.name).font(.body).foregroundStyle(VoicePalette.text)
                            .fixedSize(horizontal: false, vertical: true)
                        if !profile.descriptionText.isEmpty {
                            Text(profile.descriptionText).font(.footnote).foregroundStyle(VoicePalette.secondaryText)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    Spacer(minLength: 8)
                    if active {
                        Image(systemName: "checkmark").font(.body.weight(.semibold)).foregroundStyle(VoicePalette.accent)
                            .accessibilityHidden(true)
                    }
                }
                .frame(minHeight: 44)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(profile.descriptionText.isEmpty ? profile.name : "\(profile.name). \(profile.descriptionText)")
            .accessibilityValue(active ? "Aktiv" : "")
            .accessibilityHint("Wählt diesen Assistenten")
            .accessibilityAddTraits(active ? .isSelected : [])
            .accessibilityIdentifier("assistantProfileRow-\(profile.id)")

            Menu {
                if template {
                    Button("Ansehen", systemImage: "eye") { editing = AssistantEditorTarget(profile: profile, readOnly: true) }
                    Button("Als eigenen Assistenten kopieren", systemImage: "plus.square.on.square") { copy(profile) }
                } else {
                    Button("Bearbeiten", systemImage: "pencil") { editing = AssistantEditorTarget(profile: profile) }
                    Button("Duplizieren", systemImage: "plus.square.on.square") { copy(profile) }
                    Button("Löschen", systemImage: "trash", role: .destructive) { deleting = profile }
                }
            } label: {
                Image(systemName: "ellipsis.circle").font(.title3).frame(width: 44, height: 44).contentShape(Rectangle())
            }
            .accessibilityLabel("Weitere Aktionen zu \(profile.name)")
            .accessibilityIdentifier("assistantProfileMenu-\(profile.id)")
        }
        .accessibilityElement(children: .contain)
    }

    // MARK: Aktionen

    private func tap(_ profile: AssistantProfile) {
        guard profile.id != profiles.active.id else {
            close()
            return
        }
        if talk.hasConversation { switching = profile } else { choose(profile) }
    }

    private func choose(_ profile: AssistantProfile) {
        talk.selectAssistant(profile.id)
        close()
    }

    private func remove(_ profile: AssistantProfile) {
        talk.deleteAssistant(profile.id)
    }

    private func copy(_ profile: AssistantProfile) {
        switch profiles.copy(profile) {
        case .success(let copy): editing = AssistantEditorTarget(profile: copy)
        case .failure(let error): failure = AssistantProfileProblem.sentence(for: error)
        }
    }

    private func newProfile() {
        guard let blank = profiles.blank() else {
            failure = "Der Assistent konnte nicht angelegt werden."
            return
        }
        editing = AssistantEditorTarget(profile: blank, isNew: true)
    }
}
