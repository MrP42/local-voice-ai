import SwiftUI

// Assistenten im Gespräch „Sprechen“ (lva#556, Paket G2): Editor für einen Assistenten.
// Er arbeitet auf einer Kopie (`draft`) des ganzen Profils: Felder, die er nicht zeigt (Stimme, Schnittstellen, Auswertung, unbekannte
// Felder anderer Fassungen, `nachfrage_stille_s`), bleiben beim Sichern unverändert erhalten. Gesichert wird nur ein gültiges Profil;
// die Prüfung ist die des Kerns (`AssistantProfile.validate`), ihre Fehler stehen je Feld als Satz (`AssistantProfileProblem`).
// Vorlagen lassen sich nur ansehen (`readOnly`) und als eigenen Assistenten kopieren.

/// Ziel des Editors: Welches Profil, neu oder bestehend, nur ansehen oder bearbeiten.
struct AssistantEditorTarget: Identifiable {
    let id = UUID()
    var profile: AssistantProfile
    /// Noch nicht gesichert (Knopf „Neuer Assistent“).
    var isNew = false
    /// Vorlage: nur ansehen.
    var readOnly = false
}

/// Fehlercodes des Kerns als Sätze für die Ansicht: ein kleiner Satz je Feld und Code, nie der Rohcode.
enum AssistantProfileProblem {
    static func sentence(for issue: AssistantProfileIssue) -> String {
        let path = issue.path
        switch (path, issue.code) {
        case ("name", .length), ("name", .missing): return "Der Name braucht 1 bis 60 Zeichen."
        case ("name", _): return "Der Name darf keine Zeilenumbrüche oder Steuerzeichen enthalten."
        case ("beschreibung", _): return "Die Beschreibung darf höchstens 200 Zeichen haben."
        case ("anweisungen", _): return "Die Anweisungen dürfen höchstens 8000 Zeichen haben."
        case ("begruessung", _): return "Die Begrüßung darf höchstens 300 Zeichen haben."
        case ("id", _): return "Die Kennung des Assistenten ist ungültig."
        default: break
        }
        if path.hasPrefix("verhalten") { return "Eine Vorgabe zum Gesprächsverhalten ist ungültig." }
        if path.hasPrefix("wissen") { return "Eine Angabe zum Wissen ist ungültig." }
        switch issue.code {
        case .schemaNewer: return "Dieser Assistent stammt aus einer neueren Fassung der App."
        case .schemaUnknown, .json: return "Dieser Assistent ist beschädigt."
        default: return "Eine Angabe in diesem Assistenten ist ungültig."
        }
    }

    /// Fehler beim Sichern oder Kopieren (Speicher des Kerns).
    static func sentence(for error: AssistantProfileStoreError) -> String {
        switch error {
        case .invalid: return "Der Assistent ist nicht gültig."
        case .tooMany: return "Es sind höchstens 200 Assistenten möglich. Lösche zuerst einen."
        case .tooLarge: return "Der Assistent ist zu groß zum Sichern."
        case .reservedId, .notFound: return "Der Assistent konnte nicht gesichert werden."
        case .occupied: return "Unter dieser Kennung liegt schon eine Datei, die sich nicht lesen lässt. Sie bleibt unverändert."
        case .writeFailed: return "Der Assistent konnte nicht gesichert werden. Prüfe den freien Speicher."
        }
    }
}

struct AssistantProfileEditorView: View {
    let target: AssistantEditorTarget
    /// Schließt den Editor (nach Sichern, Abbrechen, Fertig, Kopieren).
    let close: () -> Void
    /// Nach „Als eigenen Assistenten kopieren“ (Vorlage): die Kopie zum Bearbeiten öffnen.
    var openCopy: ((AssistantProfile) -> Void)?

    @ObservedObject private var profiles = AssistantProfiles.shared
    @ObservedObject private var talk = AssistantController.shared
    @State private var draft: AssistantProfile
    @State private var saveError: String?
    @State private var confirmDiscard = false
    @ScaledMetric(relativeTo: .body) private var editorHeight: CGFloat = 140

    private static let nameMax = 60
    private static let descriptionMax = 200
    private static let instructionsMax = 8_000
    private static let greetingMax = 300

    init(target: AssistantEditorTarget, close: @escaping () -> Void, openCopy: ((AssistantProfile) -> Void)? = nil) {
        self.target = target
        self.close = close
        self.openCopy = openCopy
        _draft = State(initialValue: target.profile)
    }

    private var readOnly: Bool { target.readOnly }
    private var issues: [AssistantProfileIssue] { draft.validate() }
    private var dirty: Bool { draft != target.profile }
    /// Gesichert wird nur ein gültiges Profil mit Namen.
    private var canSave: Bool { !readOnly && issues.isEmpty && dirty }

    /// Die Sätze zu den Fehlern, deren Pfad mit einem der Präfixe beginnt (ohne Wiederholung, in der Reihenfolge des Kerns).
    private func problems(_ prefixes: String...) -> [String] {
        var seen: [String] = []
        for issue in issues where prefixes.contains(where: { issue.path.hasPrefix($0) }) {
            let text = AssistantProfileProblem.sentence(for: issue)
            if !seen.contains(text) { seen.append(text) }
        }
        return seen
    }

    private func codePoints(_ text: String) -> Int { text.unicodeScalars.count }

    private var greeting: AssistantGreeting {
        AssistantProfileText.spokenGreeting(draft, values: AssistantPlaceholderValues(date: Date(), name: nil))
    }

    var body: some View {
        NavigationStack {
            Form {
                // Eine Vorlage ist nur zum Ansehen: Eingaben sind aus; „Als eigenen Assistenten kopieren“ und „Anhören“ bleiben bedienbar.
                if readOnly { templateSection }
                nameSection.disabled(readOnly)
                instructionsSection.disabled(readOnly)
                greetingSection
                knowledgeSection.disabled(readOnly)
                behaviorSection.disabled(readOnly)
            }
            .navigationTitle(readOnly ? "Vorlage" : (target.isNew ? "Neuer Assistent" : "Assistent bearbeiten"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { toolbar }
            .interactiveDismissDisabled(dirty && !readOnly)
            .confirmationDialog("Änderungen verwerfen?", isPresented: $confirmDiscard, titleVisibility: .visible) {
                Button("Verwerfen", role: .destructive) { leave() }
                Button("Weiter bearbeiten", role: .cancel) {}
            }
            .alert("Nicht gesichert", isPresented: Binding(get: { saveError != nil }, set: { if !$0 { saveError = nil } })) {
                Button("OK", role: .cancel) {}
            } message: {
                Text(saveError ?? "")
            }
            .onDisappear { talk.stopPreview() }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("assistantProfileEditor")
    }

    // MARK: Leiste

    @ToolbarContentBuilder private var toolbar: some ToolbarContent {
        if readOnly {
            ToolbarItem(placement: .confirmationAction) {
                Button("Fertig") { leave() }.accessibilityIdentifier("assistantProfileDone")
            }
        } else {
            ToolbarItem(placement: .cancellationAction) {
                Button("Abbrechen") { if dirty { confirmDiscard = true } else { leave() } }
                    .accessibilityIdentifier("assistantProfileCancel")
            }
            ToolbarItem(placement: .confirmationAction) {
                Button("Sichern") { save() }
                    .disabled(!canSave)
                    .accessibilityIdentifier("assistantProfileSave")
            }
        }
    }

    private func leave() {
        talk.stopPreview()
        close()
    }

    /// Sichert das geprüfte Profil. War es der aktive Assistent, gelten die Änderungen ab der nächsten Anfrage.
    private func save() {
        var checked = draft
        checked.name = checked.name.trimmingCharacters(in: .whitespacesAndNewlines)
        switch profiles.save(checked) {
        case .success(let saved):
            if saved.id == profiles.active.id { talk.activeAssistantEdited() }
            leave()
        case .failure(let error):
            saveError = AssistantProfileProblem.sentence(for: error)
        }
    }

    // MARK: Vorlage

    private var templateSection: some View {
        Section {
            Text("Eine Vorlage lässt sich nicht ändern. Als eigener Assistent kannst du sie anpassen.")
                .font(.footnote)
                .foregroundStyle(VoicePalette.secondaryText)
                .fixedSize(horizontal: false, vertical: true)
            Button("Als eigenen Assistenten kopieren") { copyTemplate() }
                .frame(minHeight: 44)
                .accessibilityIdentifier("assistantProfileCopy")
        }
    }

    private func copyTemplate() {
        switch profiles.copy(target.profile) {
        case .success(let copy):
            leave()
            openCopy?(copy)
        case .failure(let error):
            saveError = AssistantProfileProblem.sentence(for: error)
        }
    }

    // MARK: Name und Beschreibung

    private var nameSection: some View {
        Section {
            TextField("Name", text: $draft.name)
                .textInputAutocapitalization(.sentences)
                .frame(minHeight: 44)
                .accessibilityIdentifier("assistantProfileName")
            TextField("Kurze Beschreibung (optional)", text: $draft.descriptionText, axis: .vertical)
                .lineLimit(1...4)
                .frame(minHeight: 44)
                .accessibilityIdentifier("assistantProfileDescription")
            problemLines(problems("name", "beschreibung"), identifier: "assistantProfileNameProblem")
        } header: {
            Text("Assistent")
        }
    }

    // MARK: Anweisungen

    private var instructionsSection: some View {
        let count = codePoints(draft.instructions)
        return Section {
            ZStack(alignment: .topLeading) {
                TextEditor(text: $draft.instructions)
                    .frame(minHeight: editorHeight)
                    .accessibilityLabel("Anweisungen")
                    .accessibilityIdentifier("assistantProfileInstructions")
                if draft.instructions.isEmpty {
                    Text("Wie soll sich der Assistent verhalten? Zum Beispiel: Du bist die Empfangsstelle eines Büros. Antworte kurz und freundlich.")
                        .foregroundStyle(VoicePalette.secondaryText)
                        .padding(.horizontal, 5).padding(.vertical, 8)
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
                }
            }
            counter(count, Self.instructionsMax, identifier: "assistantProfileInstructionsCount")
            problemLines(problems("anweisungen"), identifier: "assistantProfileInstructionsProblem")
        } header: {
            Text("Anweisungen")
        } footer: {
            VStack(alignment: .leading, spacing: 4) {
                Text(verbatim: "Platzhalter: {{name}}, {{tageszeit}}, {{datum}}, {{wochentag}}, {{uhrzeit}}")
                    .accessibilityIdentifier("assistantProfilePlaceholderHelp")
                if count > talk.roleBudgetChars {
                    Text("Für das aktuelle Modell werden etwa die ersten \(talk.roleBudgetChars) Zeichen verwendet.")
                        .accessibilityIdentifier("assistantProfileTruncation")
                }
            }
        }
    }

    // MARK: Begrüßung

    private var greetingSection: some View {
        let resolved = greeting
        return Section {
            TextField("Begrüßung (optional)", text: $draft.greeting, axis: .vertical)
                .lineLimit(1...5)
                .frame(minHeight: 44)
                .disabled(readOnly)
                .accessibilityIdentifier("assistantProfileGreeting")
            counter(codePoints(draft.greeting), Self.greetingMax, identifier: "assistantProfileGreetingCount")
            problemLines(problems("begruessung"), identifier: "assistantProfileGreetingProblem")
            if !resolved.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Vorschau").font(.caption.weight(.semibold)).foregroundStyle(VoicePalette.secondaryText)
                    Text(resolved.text).fixedSize(horizontal: false, vertical: true)
                }
                .accessibilityElement(children: .combine)
                .accessibilityLabel("Vorschau: \(resolved.text)")
                .accessibilityIdentifier("assistantProfileGreetingPreview")
                Button {
                    if talk.previewing { talk.stopPreview() } else { talk.previewSpeech(resolved.text) }
                } label: {
                    Label(talk.previewing ? "Anhalten" : "Anhören", systemImage: talk.previewing ? "stop.fill" : "speaker.wave.2")
                        .frame(minHeight: 44)
                }
                .accessibilityIdentifier("assistantProfileGreetingPlay")
                if resolved.containsStopWord {
                    Label {
                        Text("Die Begrüßung enthält ein Wort, das als „Stopp“ verstanden wird (zum Beispiel „halt“, „warte“ oder „moment“). Das Mikrofon kann sie dann abbrechen.")
                    } icon: {
                        Image(systemName: "exclamationmark.triangle").foregroundStyle(.orange)
                    }
                    .font(.footnote)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("assistantProfileGreetingStopWarning")
                }
                if !resolved.unknownPlaceholders.isEmpty {
                    Text("Unbekannter Platzhalter: \(resolved.unknownPlaceholders.joined(separator: ", "))")
                        .font(.footnote)
                        .foregroundStyle(VoicePalette.secondaryText)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityIdentifier("assistantProfileGreetingUnknown")
                }
            }
        } header: {
            Text("Begrüßung")
        } footer: {
            Text("Wird zu Beginn eines neuen Gesprächs gesprochen. Ohne bekannten Namen entfällt {{name}}.")
        }
    }

    // MARK: Wissen

    private var knowledgeSection: some View {
        Section {
            Toggle("Alle freigegebenen Bereiche", isOn: $draft.knowledge.allAreas)
                .frame(minHeight: 44)
                .accessibilityIdentifier("assistantProfileAllAreas")
            if !draft.knowledge.allAreas {
                ForEach(AssistantProfile.areaMapping, id: \.key) { entry in
                    Toggle(AssistantAreaSettings.title(entry.area), isOn: areaBinding(entry.key))
                        .frame(minHeight: 44)
                        .accessibilityIdentifier("assistantProfileArea-\(entry.key)")
                }
                if !AssistantProfile.areaMapping.contains(where: { draft.knowledge.areas.contains($0.key) }) {
                    Text("Kein Bereich gewählt: Der Assistent nutzt keine deiner Inhalte.")
                        .font(.footnote)
                        .foregroundStyle(VoicePalette.secondaryText)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityIdentifier("assistantProfileNoAreas")
                }
            }
            problemLines(problems("wissen"), identifier: "assistantProfileKnowledgeProblem")
        } header: {
            Text("Wissen")
        } footer: {
            Text("Es gilt zusätzlich, was im Menü freigegeben ist.")
        }
    }

    /// Ein Bereich des Profils an oder aus; unbekannte Schlüssel anderer Fassungen bleiben in der Liste stehen.
    private func areaBinding(_ key: String) -> Binding<Bool> {
        Binding(
            get: { draft.knowledge.areas.contains(key) },
            set: { on in
                draft.knowledge.areas.removeAll { $0 == key }
                if on { draft.knowledge.areas.append(key) }
            })
    }

    // MARK: Verhalten

    private var behaviorSection: some View {
        Group {
            Section {
                Picker("Antwortstil", selection: $draft.behavior.style) {
                    Text("Natürlich").tag(AssistantProfile.Behavior.Style.natural)
                    Text("Wörtlich").tag(AssistantProfile.Behavior.Style.literal)
                }
                .pickerStyle(.inline)
                .labelsHidden()
                .accessibilityLabel("Antwortstil")
                .accessibilityIdentifier("assistantProfileStyle")
            } header: {
                Text("Antwortstil")
            } footer: {
                Text(draft.behavior.style == .natural
                     ? "Antwortet frei in eigenen Worten. Quellen werden angezeigt, nicht vorgelesen."
                     : "Liest wörtliche Auszüge aus deinen Inhalten vor.")
            }

            Section {
                Picker("Sprechpause", selection: $draft.behavior.pause) {
                    Text("Kurz").tag(AssistantProfile.Behavior.Pause.short)
                    Text("Normal").tag(AssistantProfile.Behavior.Pause.normal)
                    Text("Lang").tag(AssistantProfile.Behavior.Pause.long)
                }
                .pickerStyle(.inline)
                .labelsHidden()
                .accessibilityLabel("Sprechpause")
                .accessibilityIdentifier("assistantProfilePause")
            } header: {
                Text("Sprechpause")
            } footer: {
                Text("Wie lange ich warte, bevor ich antworte.")
            }

            Section {
                Toggle("Beim Antworten zuhören", isOn: $draft.behavior.interruptible)
                    .frame(minHeight: 44)
                    .accessibilityIdentifier("assistantProfileInterruptible")
            } footer: {
                Text("Du kannst die Antwort mit deiner Stimme unterbrechen.")
            }
        }
    }

    // MARK: Bausteine

    /// Zeichenzähler „n / Höchstzahl“, über der Grenze rot (und als Text für VoiceOver, nicht nur Farbe).
    private func counter(_ count: Int, _ limit: Int, identifier: String) -> some View {
        Text(verbatim: "\(count) / \(limit)")
            .font(.footnote.monospacedDigit())
            .foregroundStyle(count > limit ? Color.red : VoicePalette.secondaryText)
            .frame(maxWidth: .infinity, alignment: .trailing)
            .accessibilityLabel(count > limit ? "\(count) von \(limit) Zeichen, zu lang" : "\(count) von \(limit) Zeichen")
            .accessibilityIdentifier(identifier)
    }

    @ViewBuilder private func problemLines(_ lines: [String], identifier: String) -> some View {
        if !lines.isEmpty {
            VStack(alignment: .leading, spacing: 2) {
                ForEach(lines, id: \.self) { line in
                    Label(line, systemImage: "exclamationmark.circle")
                        .font(.footnote)
                        .foregroundStyle(Color.red)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier(identifier)
        }
    }
}
