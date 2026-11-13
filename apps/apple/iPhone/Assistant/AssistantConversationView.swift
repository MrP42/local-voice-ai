import SwiftUI

// Gesprächsmodus „Sprechen“ (Paket A): Vollbildansicht. Gestaltung wie das Buch-Gespräch (`BookTalkSheet`): ruhige
// Flächen, Ziele von mindestens 44 pt, Farben aus `VoicePalette`. Jeder Zustand steht als Text da; die Bewegung ist nur
// Beiwerk und entfällt bei „Bewegung reduzieren“.

private enum AssistantColors {
    /// Weiß darauf ≥ 4,5:1 (wie im Buch-Gespräch).
    static let fill = Color(red: 0, green: 0.4, blue: 0.8)
    static var card: Color { VoicePalette.text.opacity(0.06) }
}

struct AssistantConversationView: View {
    @ObservedObject private var talk = AssistantController.shared
    @ObservedObject private var relays = BookTalkRelayStore.shared
    @ObservedObject private var endpoints = AssistantEndpointStore.shared
    @ObservedObject private var hub = AssistantSources.shared
    @ObservedObject private var profiles = AssistantProfiles.shared
    @Environment(\.dismiss) private var dismiss
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.dynamicTypeSize) private var typeSize
    @ScaledMetric(relativeTo: .title) private var pulseSize: CGFloat = 56
    @State private var typing = false
    @State private var draft = ""
    @State private var showEndpoint = false
    @State private var endpointBefore: AssistantEndpoint?
    @State private var gemmaReady = false
    @State private var showModelChoice = false
    @State private var showLocalModels = false
    @State private var showAreas = false
    @State private var showOptions = false
    /// Blatt „Assistenten“ (lva#556) und der Editor, der aus „Optionen“ heraus aufgeht (Assistent bearbeiten, Vorlage kopieren).
    @State private var showProfiles = false
    /// Anhänge-Blatt (lva#557): Inhalte an dieses Gespräch hängen.
    @State private var showAttach = false
    @State private var optionsEditor: AssistantEditorTarget?
    @State private var optionsFailure: String?
    #if DEBUG
    /// `--assistant-demo-profile-editor`: der Editor mit dem erfundenen Assistenten, direkt beim Öffnen.
    @State private var demoEditor: AssistantEditorTarget?
    #endif
    @State private var thinkingOpen = false
    /// Messwerte: nicht veröffentlicht, deshalb beim Anzeigen und nach dem Zurücksetzen neu gelesen.
    @State private var latency = ""
    @FocusState private var fieldFocused: Bool

    /// Kein Modell, und es startet auch keins gerade: statt Pause-Knopf der deaktivierte Mikrofon-Zustand und „Modell auswählen“.
    private var noModel: Bool { !talk.canSend && !talk.loading && !talk.running }

    /// Größte Schrifteinstellungen (Bedienungshilfen): Fest bleiben nur die Bedienknöpfe. Zustandszeile, Modellwahl, Hinweise und
    /// Vorschläge stehen im Inhalt, der scrollt; der Verlauf behält so mehr als die Hälfte des Bildschirms. Nichts wird abgeschnitten.
    private var accessible: Bool { typeSize.isAccessibilitySize }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            contextRow
            if talk.style == .natural { aiNotice }
            content
                .sheet(isPresented: $showLocalModels, onDismiss: localModelsClosed) { ModelPanel(model: VoiceModel.shared) }
                .sheet(isPresented: $showAreas) { areasSheet }
                .sheet(isPresented: $showOptions) { optionsSheet }
                .sheet(isPresented: $showProfiles) { AssistantProfilesView(close: { showProfiles = false }) }
                .sheet(isPresented: $showAttach) { attachSheet }
                #if DEBUG
                .sheet(item: $demoEditor) { target in AssistantProfileEditorView(target: target, close: { demoEditor = nil }) }
                #endif
            Divider()
            bottom
                .confirmationDialog("Modell auswählen", isPresented: $showModelChoice, titleVisibility: .visible) { modelChoiceButtons }
        }
        .background(VoicePalette.background)
        .task {
            gemmaReady = AssistantEngine.gemmaInstalled()
            await talk.start()
            #if DEBUG
            if AssistantController.demoProfiles { showProfiles = true }
            if AssistantController.demoContextPicker { showAttach = true }
            if AssistantController.demoProfileEditor { demoEditor = AssistantEditorTarget(profile: AssistantProfiles.demoFront) }
            #endif
        }
        .task { await relays.refresh() }
        .onDisappear { talk.end() }
        .onChange(of: scenePhase) { _, phase in
            if phase != .active { talk.sceneInactive(background: phase == .background) } else if noModel { talk.resume() }
        }
        .accessibilityAction(.magicTap) { magicTap() }
        .accessibilityAction(.escape) { close() }
        .alert(talk.consentRequest?.title ?? "", isPresented: Binding(get: { talk.consentRequest != nil }, set: { if !$0 { talk.declineConsent() } }),
               presenting: talk.consentRequest) { _ in
            Button("Abbrechen", role: .cancel) { talk.declineConsent() }
            Button("Freigeben") { talk.grantConsent() }
        } message: { request in
            Text(request.message)
        }
        .sheet(isPresented: $showEndpoint, onDismiss: endpointClosed) {
            NavigationStack {
                AssistantEndpointSettingsView()
                    .toolbar {
                        ToolbarItem(placement: .confirmationAction) { Button("Fertig") { showEndpoint = false } }
                    }
            }
        }
    }

    // MARK: Aktionen

    private func close() {
        talk.end()
        dismiss()
    }

    /// Zwei-Finger-Doppeltipp (VoiceOver): läuft eine Antwort, anhalten; sonst Stumm umschalten.
    private func magicTap() {
        if talk.phase == .thinking || talk.phase == .speaking { talk.stopAnswer() } else if talk.running { talk.toggleMute() }
    }

    private func sendDraft() {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        talk.send(text: text)
        draft = ""
    }

    /// Nach den Einstellungen des Endpunkts: Neuer oder geänderter Endpunkt wird gewählt; ein entfernter, der gerade
    /// gewählt war, fällt auf das Apple-Modell zurück (sonst liefe das Gespräch weiter an die alte Adresse).
    private func endpointClosed() {
        let now = endpoints.endpoint
        guard now != endpointBefore else { return }
        if now == nil {
            if talk.selectedChoice == .endpoint { talk.selectModel(.apple) }
        } else {
            talk.selectModel(.endpoint)
        }
    }

    private func perform(_ action: AssistantNotice.Action) {
        switch action {
        case .retrySend: talk.retry()
        case .resume, .retryStart: talk.resume()
        case .retryAudit: talk.retry()
        case .retryRelease: talk.retryRelease()
        case .chooseModel: showModelChoice = true
        case .openLocalModels: showLocalModels = true
        case .openAreas: showAreas = true
        }
    }

    /// Die Bereiche als eigenes Blatt (dieselben Schalter wie im Menü „Mehr“): Der Hinweis „Freigaben prüfen“ öffnet es direkt.
    private var areasSheet: some View {
        NavigationStack {
            List {
                Section {
                    ForEach(AssistantArea.allCases, id: \.self) { area in
                        Toggle(AssistantAreaSettings.title(area),
                               isOn: Binding(get: { talk.areas.contains(area) }, set: { _ in talk.toggleArea(area) }))
                    }
                } header: {
                    Text(areaTitle)
                } footer: {
                    Text("Der Assistent antwortet nur aus den Bereichen, die hier eingeschaltet sind.")
                }
            }
            .navigationTitle("Bereiche")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) { Button("Fertig") { showAreas = false }.accessibilityIdentifier("assistantAreasDone") }
            }
        }
        .accessibilityIdentifier("assistantAreasSheet")
    }

    /// Optionen als Blatt: Listenzeilen brechen lange Beschriftungen um (ein Menü schnitt die Schalter ab) und tragen Erklärzeilen.
    private var optionsSheet: some View {
        NavigationStack {
            List {
                if !profiles.isEverything { assistantGivenSection }

                Section {
                    Picker("Antwortstil", selection: Binding(get: { talk.style }, set: { talk.setStyle($0) })) {
                        Text("Natürlich").tag(AssistantStyle.natural)
                        Text("Wörtlich").tag(AssistantStyle.verbatim)
                    }
                    .pickerStyle(.inline)
                    .labelsHidden()   // die Abschnittsüberschrift trägt die Beschriftung; sonst steht sie als eigene Zeile doppelt
                    .disabled(!profiles.isEverything)
                    .accessibilityIdentifier("assistantStylePicker")
                } header: {
                    Text("Antwortstil")
                } footer: {
                    Text(styleExplanation)
                }

                Section {
                    Picker("Sprechpause", selection: Binding(get: { talk.pause }, set: { talk.setPause($0) })) {
                        ForEach(AssistantPauseSetting.allCases) { setting in Text(setting.title).tag(setting) }
                    }
                    .pickerStyle(.inline)
                    .labelsHidden()   // die Abschnittsüberschrift trägt die Beschriftung; sonst steht sie als eigene Zeile doppelt
                    .disabled(!profiles.isEverything)
                    .accessibilityIdentifier("assistantPausePicker")
                } header: {
                    Text("Sprechpause")
                } footer: {
                    Text("Wie lange ich warte, bevor ich antworte.")
                }

                Section {
                    Toggle("Beim Antworten zuhören", isOn: Binding(get: { talk.bargeIn }, set: { talk.setBargeIn($0) }))
                        .disabled(!profiles.isEverything)
                        .accessibilityIdentifier("assistantBargeInToggle")
                } footer: {
                    Text("Du kannst die Antwort mit deiner Stimme unterbrechen.")
                }

                Section {
                    Text(verbatim: latency.isEmpty ? "Noch keine Messwerte." : latency)
                        .font(.system(.footnote, design: .monospaced))
                        .textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .accessibilityIdentifier("assistantLatencyText")
                    Button("Zurücksetzen", role: .destructive) {
                        talk.resetLatency()
                        latency = talk.latencyText
                    }
                    .frame(minHeight: 44)
                    .accessibilityIdentifier("assistantLatencyReset")
                } header: {
                    Text("Messwerte")
                } footer: {
                    Text("Nur Zeiten und Zähler, keine Inhalte.")
                }
            }
            .navigationTitle("Optionen")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) { Button("Fertig") { showOptions = false }.accessibilityIdentifier("assistantOptionsDone") }
            }
            .onAppear { latency = talk.latencyText }
            .sheet(item: $optionsEditor) { target in AssistantProfileEditorView(target: target, close: { optionsEditor = nil }) }
            .alert("Nicht möglich", isPresented: Binding(get: { optionsFailure != nil }, set: { if !$0 { optionsFailure = nil } })) {
                Button("OK", role: .cancel) {}
            } message: {
                Text(optionsFailure ?? "")
            }
        }
        .accessibilityIdentifier("assistantOptionsSheet")
    }

    /// Bei einem Assistenten außer „Alles“ sind Antwortstil, Sprechpause und „Beim Antworten zuhören“ seine Vorgaben: Die Optionen zeigen sie
    /// nur an. Der Knopf führt zum Editor (eigener Assistent) oder legt von der Vorlage eine eigene Kopie an, die sich bearbeiten lässt.
    private var assistantGivenSection: some View {
        let profile = profiles.active
        let template = AssistantProfiles.isTemplate(profile)
        return Section {
            Label("Vom Assistenten ‚\(profile.name)‘ vorgegeben.", systemImage: "person.crop.circle")
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityIdentifier("assistantProfileGivenNote")
            Button(template ? "Als eigenen Assistenten kopieren" : "Assistent bearbeiten") {
                if template {
                    switch profiles.copy(profile) {
                    case .success(let copy): optionsEditor = AssistantEditorTarget(profile: copy)
                    case .failure(let error): optionsFailure = AssistantProfileProblem.sentence(for: error)
                    }
                } else {
                    optionsEditor = AssistantEditorTarget(profile: profile)
                }
            }
            .frame(minHeight: 44)
            .accessibilityIdentifier("assistantProfileGivenEdit")
        }
    }

    private var styleExplanation: String {
        switch talk.style {
        case .natural: "Antwortet frei in eigenen Worten. Quellen werden angezeigt, nicht vorgelesen."
        case .verbatim: "Liest wörtliche Auszüge aus deinen Inhalten vor."
        }
    }

    /// Dauerhafter KI-Hinweis im Stil „Natürlich“, schon vor der ersten Antwort sichtbar; bricht um.
    private var aiNotice: some View {
        Label("Antworten sind KI-erzeugt und frei formuliert. Sie können Fehler enthalten.", systemImage: "sparkles")
            .font(.footnote)
            .foregroundStyle(VoicePalette.secondaryText)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 16).padding(.vertical, 8)
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("assistantAINotice")
    }

    /// Haltehinweis über der Bedienleiste: Die Antwort wartet, weil du noch sprichst (oder die Erkennung nachhinkt).
    private func holdBand(_ hold: AssistantHoldNotice) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Label(hold.text, systemImage: "hourglass")
                .font(.footnote)
                .fixedSize(horizontal: false, vertical: true)
            AssistantFlowLayout(spacing: 8) {
                if hold.offersRelease { holdButton("Antwort jetzt hören", identifier: "assistantHoldRelease") { talk.releaseHold() } }
                if hold.offersDiscard { holdButton("Verwerfen", identifier: "assistantHoldDiscard") { talk.discardHeldAnswer() } }
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(AssistantColors.card, in: RoundedRectangle(cornerRadius: 12))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("assistantHoldNotice")
    }

    private func holdButton(_ title: String, identifier: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title)
                .font(.footnote.weight(.semibold))
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
                .frame(minHeight: 44)
                .contentShape(Rectangle())
        }
        .buttonStyle(.bordered)
        .accessibilityLabel(title)
        .accessibilityIdentifier(identifier)
    }

    /// Nach den lokalen Modellen in den Einstellungen: ohne Modell neu prüfen (Download fertig, Systemmodell geladen).
    private func localModelsClosed() {
        gemmaReady = AssistantEngine.gemmaInstalled()
        if noModel { talk.resume() }
    }

    /// Modellwahl als Auswahlblatt (Hauptknopf „Modell auswählen“); gleiche Auswahl wie das Menü im Kopf, die Art steht im Titel.
    @ViewBuilder private var modelChoiceButtons: some View {
        Button("Apple-Systemmodell · lokal") { talk.selectModel(.apple) }
        if gemmaReady || talk.selectedChoice == .gemma { Button("Gemma 4 E2B · lokal") { talk.selectModel(.gemma) } }
        ForEach(relays.models) { info in
            Button("\(info.label) · Cloud") { talk.selectModel(.relay(info.id)) }
        }
        if let endpoint = endpoints.endpoint {
            Button("\(endpoint.label) · \(endpoints.isCloud ? "Cloud" : "eigener Endpunkt")") { talk.selectModel(.endpoint) }
        }
        Button("Eigener Endpunkt …") {
            endpointBefore = endpoints.endpoint
            showEndpoint = true
        }
        Button("Abbrechen", role: .cancel) {}
    }

    // MARK: Verknüpfte Inhalte (lva#557)

    /// Chip-Zeile unter der Kopfzeile: die Inhalte des Assistenten und die des Gesprächs. Ohne Kontexte keine Zeile.
    @ViewBuilder private var contextRow: some View {
        let chips = talk.contextChips
        if !chips.isEmpty {
            AssistantContextChipsView(chips: chips, modeLine: talk.contextModeLine) { chip in
                talk.removeConversationContext(kind: chip.kind, id: chip.contextId)
            }
            Divider()
        }
    }

    /// Anhänge-Blatt: Bereiche, die der Assistent nutzen darf (freigegeben ∩ Assistent); die Inhalte des Assistenten stehen mit Haken, nicht änderbar.
    private var attachSheet: some View {
        let profile = profiles.isEverything ? nil : profiles.active
        let assistantContexts = profile.map { $0.effectiveContextMode == .all ? [] : $0.knowledge.contexts } ?? []
        return AssistantContextPickerView(
            title: "Inhalt anhängen",
            areas: talk.usableAreas,
            linked: talk.conversationContexts.contexts.compactMap { $0.context },
            locked: assistantContexts,
            showsOnlySwitch: true,
            onlyFromThese: talk.conversationContexts.onlyFromThese,
            done: { selection in
                talk.applyAttachment(selection)
                showAttach = false
            },
            cancel: { showAttach = false })
    }

    // MARK: Kopf

    /// Schließen und Menü bleiben fest; der Modellchip nur bei normaler Schrift (bei sehr großer steht er im Inhalt).
    private var header: some View {
        HStack(spacing: 4) {
            Button(action: close) {
                Image(systemName: "xmark").font(.body.weight(.semibold)).frame(width: 44, height: 44)
                    .contentShape(Rectangle())
            }
            .accessibilityLabel("Schließen")
            .accessibilityHint("Beendet das Gespräch")
            .accessibilityIdentifier("assistantClose")
            Button { showAttach = true } label: {
                Image(systemName: "paperclip").font(.body.weight(.semibold)).frame(width: 44, height: 44).contentShape(Rectangle())
            }
            .accessibilityLabel("Inhalt anhängen")
            .accessibilityHint("Wählt Inhalte, die in diesem Gespräch Vorrang haben")
            .accessibilityIdentifier("assistantContextAttach")
            Spacer(minLength: 8)
            if !accessible {
                assistantChip
                modelMenu.layoutPriority(1)
            }
            optionsMenu
        }
        .padding(.horizontal, 8).padding(.top, 4)
    }

    /// Assistent-Chip (lva#556): „Assistent: <Name>“ in einer Zeile (zu langer Name wird gekürzt), mindestens 44 pt hoch und etwa 96 pt
    /// breit. Antippen öffnet das Blatt „Assistenten“.
    private var assistantChip: some View {
        let name = profiles.active.name
        return Button { showProfiles = true } label: {
            HStack(spacing: 6) {
                Image(systemName: "person.crop.circle").font(.body)
                Text("Assistent: \(name)")
                    .font(.footnote)
                    .lineLimit(1)
                    .truncationMode(.tail)
                if accessible { Spacer(minLength: 0) }
            }
            .foregroundStyle(VoicePalette.text)
            .padding(.horizontal, 10).padding(.vertical, accessible ? 8 : 0)
            .frame(minWidth: 96, maxWidth: accessible ? .infinity : 200, minHeight: 44, alignment: .leading)
            .overlay(RoundedRectangle(cornerRadius: 22).stroke(VoicePalette.border, lineWidth: 1))
            .contentShape(RoundedRectangle(cornerRadius: 22))
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Assistent: \(name)")
        .accessibilityHint("Öffnet die Liste der Assistenten")
        .accessibilityIdentifier("assistantProfileChip")
    }

    /// Modellchip: „Lokal“ oder „Cloud“ und der Name des Modells; das Menü wählt das Modell.
    private var modelMenu: some View {
        Menu {
            Section("Lokal") {
                choice("Apple-Systemmodell", .apple, icon: "cpu")
                if gemmaReady || talk.selectedChoice == .gemma { choice("Gemma 4 E2B", .gemma, icon: "cpu") }
            }
            if !relays.models.isEmpty {
                Section("Cloud · über deinen PC ‚\(relays.desktopName)‘") {
                    ForEach(relays.models) { info in choice(info.label, .relay(info.id), icon: "cloud") }
                }
            }
            Section("Eigener Endpunkt") {
                if let endpoint = endpoints.endpoint {
                    choice(endpoint.label, .endpoint, icon: endpoints.isCloud ? "cloud" : "network")
                }
                Button("Eigener Endpunkt …", systemImage: "slider.horizontal.3") {
                    endpointBefore = endpoints.endpoint
                    showEndpoint = true
                }
            }
        } label: {
            HStack(spacing: 6) {
                Image(systemName: talk.isCloud ? "cloud" : "cpu").font(.body)
                VStack(alignment: .leading, spacing: 0) {
                    Text(talk.isCloud ? "Cloud" : "Lokal").font(.caption2.weight(.semibold))
                    // Bei sehr großer Schrift bricht der Name um, statt abgeschnitten zu werden.
                    Text(talk.modelLabel).font(.footnote)
                        .lineLimit(accessible ? nil : 1)
                        .fixedSize(horizontal: false, vertical: accessible)
                        .multilineTextAlignment(.leading)
                }
                if accessible { Spacer(minLength: 0) }
                Image(systemName: "chevron.down").font(.caption2)
            }
            .foregroundStyle(VoicePalette.text)
            .padding(.horizontal, 10).padding(.vertical, accessible ? 8 : 0)
            .frame(maxWidth: accessible ? .infinity : nil, minHeight: 44, alignment: .leading)
            .overlay(RoundedRectangle(cornerRadius: 22).stroke(VoicePalette.border, lineWidth: 1))
            .contentShape(RoundedRectangle(cornerRadius: 22))
        }
        .disabled(talk.loading)
        .accessibilityLabel("Modellauswahl")
        .accessibilityValue("\(talk.modelLabel), \(talk.isCloud ? "Cloud" : "lokal")")
        .accessibilityHint("Öffnet die Liste der Modelle")
        .accessibilityIdentifier("assistantModel")
    }

    private func choice(_ title: String, _ value: AssistantModelChoice, icon: String) -> some View {
        Button { talk.selectModel(value) } label: {
            Label(title, systemImage: talk.selectedChoice == value ? "checkmark" : icon)
        }
    }

    private var areaTitle: String { talk.isCloud ? "Bereiche für Cloud-Modelle" : "Bereiche für lokale Modelle" }

    private var optionsMenu: some View {
        Menu {
            Section(areaTitle) {
                ForEach(AssistantArea.allCases, id: \.self) { area in
                    Toggle(AssistantAreaSettings.title(area),
                           isOn: Binding(get: { talk.areas.contains(area) }, set: { _ in talk.toggleArea(area) }))
                }
            }
            // Ein Menü schneidet lange Schalterbeschriftungen ab: Schalter mit Erklärung stehen im Blatt „Optionen“.
            Button("Optionen …", systemImage: "slider.horizontal.3") {
                latency = talk.latencyText
                showOptions = true
            }
            .accessibilityIdentifier("assistantOptions")
            Button("Neues Gespräch", systemImage: "arrow.counterclockwise", role: .destructive) { talk.newConversation() }
        } label: {
            Image(systemName: "ellipsis.circle").font(.title3).frame(width: 44, height: 44)
        }
        .accessibilityLabel("Mehr")
        .accessibilityHint("Bereiche, Optionen, neues Gespräch")
        .accessibilityIdentifier("assistantMenu")
    }

    // MARK: Verlauf

    /// Mindestens ein wörtlicher Auszug ist sichtbar (ein Beitrag mit Zitat im Verlauf oder in der laufenden Antwort, auch in den
    /// Beispielrunden): Nur dann steht der feste Hinweissatz. Er hängt nicht an der Sendefähigkeit (`canSend`): Ohne Sitzung
    /// (`--assistant-no-model`) zeigt die Beispielrunde ihre Auszüge ebenfalls mit dem Satz.
    private var showsExcerpt: Bool {
        talk.turns.contains { $0.parts.contains { $0.quote && $0.isMarked } } || talk.liveParts.contains { $0.quote && $0.isMarked }
    }

    private static let currentID = "aktuell"
    private static let liveLineID = "live"
    private static let thinkingID = "denken"
    private static let backgroundID = "hintergrund"
    private static func turnID(_ index: Int) -> String { "beitrag-\(index)" }

    private var showsIntro: Bool {
        talk.turns.isEmpty && talk.currentUser == nil && talk.liveUser.isEmpty && !talk.loading
    }

    private var content: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    if accessible {
                        assistantChip
                        modelMenu
                        stateBlock
                    }
                    if let progress = talk.preparing { preparing(progress) }
                    if let alert = talk.releaseAlert { noticeView(alert, identifier: "assistantReleaseNotice") }
                    if let notice = talk.notice { noticeView(notice) }
                    if showsExcerpt { excerptNotice }
                    if hub.skipped > 0 {
                        Label("Wegen der Speichergrenze fehlen \(hub.skipped) Bücher oder Besprechungen im Gespräch.", systemImage: "exclamationmark.circle")
                            .font(.footnote).foregroundStyle(VoicePalette.secondaryText)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    if showsIntro { intro }
                    if let greeting = talk.greeting { greetingBubble(greeting) }
                    ForEach(Array(talk.turns.enumerated()), id: \.offset) { index, turn in turnView(turn).id(Self.turnID(index)) }
                    backgroundExchange.id(Self.backgroundID)
                    currentExchange.id(Self.currentID)
                    thinkingRow.id(Self.thinkingID)
                    liveUserLine.id(Self.liveLineID)
                }
                .padding(16)
            }
            .scrollDismissesKeyboard(.interactively)
            // Der ANFANG des letzten Austauschs liegt oben im sichtbaren Bereich (beim Öffnen, bei einer neuen Frage, wenn eine Antwort
            // in den Verlauf kommt), nie ihr Ende. Beim Wachsen der Antwort, beim Wechsel des Zustands und beim Einblenden der Tastatur
            // wird nicht gescrollt: Wer gerade liest, wird nicht zurückgeworfen.
            .onAppear { scrollToLastExchange(proxy, animated: false) }
            .onChange(of: talk.turns.count) { _, _ in scrollToLastExchange(proxy, animated: true) }
            .onChange(of: talk.currentUser) { _, user in
                if user != nil { thinkingOpen = false }
                scrollToLastExchange(proxy, animated: true)
            }
            .onChange(of: talk.backgroundTurn) { _, _ in scrollToLastExchange(proxy, animated: true) }
            .onChange(of: talk.thinking) { _, _ in
                if thinkingOpen { jump(proxy, to: Self.thinkingID, anchor: .bottom, animated: false) }
            }
            .onChange(of: talk.liveUser.isEmpty) { _, empty in
                if !empty { jump(proxy, to: Self.liveLineID, anchor: .bottom, animated: false) }
            }
        }
    }

    /// Die laufende Frage, sonst der letzte Beitrag des Verlaufs, mit seinem Anfang oben.
    private func scrollToLastExchange(_ proxy: ScrollViewProxy, animated: Bool) {
        if talk.currentUser != nil {
            jump(proxy, to: Self.currentID, anchor: .top, animated: animated)
        } else if let last = talk.turns.indices.last {
            jump(proxy, to: Self.turnID(last), anchor: .top, animated: animated)
        }
    }

    private func jump(_ proxy: ScrollViewProxy, to id: String, anchor: UnitPoint, animated: Bool) {
        if animated, !reduceMotion {
            withAnimation { proxy.scrollTo(id, anchor: anchor) }
        } else {
            proxy.scrollTo(id, anchor: anchor)
        }
    }

    private func preparing(_ progress: AssistantSources.Progress) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Inhalte werden vorbereitet (\(progress.done) von \(progress.total))").font(.footnote)
            ProgressView(value: Double(min(progress.done, progress.total)), total: Double(max(progress.total, 1)))
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(AssistantColors.card, in: RoundedRectangle(cornerRadius: 12))
        .accessibilityElement(children: .combine)
    }

    private func noticeView(_ notice: AssistantNotice, identifier: String = "assistantNotice") -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Label(notice.text, systemImage: "info.circle")
                .font(.footnote)
                .fixedSize(horizontal: false, vertical: true)
            if let detail = notice.detail {
                Text(detail)
                    .font(.footnote).foregroundStyle(VoicePalette.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if notice.action != nil || notice.secondary != nil {
                AssistantFlowLayout(spacing: 8) {
                    if let action = notice.action { noticeButton(action) }
                    if let secondary = notice.secondary { noticeButton(secondary) }
                }
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(AssistantColors.card, in: RoundedRectangle(cornerRadius: 12))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(identifier)
    }

    /// Die Mindesthöhe von 44 pt steht IM Label: Die ganze Fläche des Knopfes trifft, nicht nur der Text; ein `frame` um den
    /// Knopf herum machte nur den Rand höher, nicht die Tippfläche.
    private func noticeButton(_ action: AssistantNotice.Action) -> some View {
        Button { perform(action) } label: {
            Text(actionTitle(action))
                .font(.footnote.weight(.semibold))
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
                .frame(minHeight: 44)
                .contentShape(Rectangle())
        }
        .buttonStyle(.bordered)
        .accessibilityIdentifier(actionIdentifier(action))
    }

    private func actionTitle(_ action: AssistantNotice.Action) -> String {
        switch action {
        case .retrySend: "Erneut senden"
        case .resume: "Weiter"
        case .retryStart, .retryAudit: "Erneut versuchen"
        case .retryRelease: "Erneut sichern"
        case .chooseModel: "Modell auswählen"
        case .openLocalModels: "Lokale Modelle öffnen"
        case .openAreas: "Bereiche öffnen"
        }
    }

    private func actionIdentifier(_ action: AssistantNotice.Action) -> String {
        switch action {
        case .retrySend: "assistantNoticeRetrySend"
        case .resume: "assistantNoticeResume"
        case .retryStart: "assistantNoticeRetryStart"
        case .retryAudit: "assistantNoticeRetryAudit"
        case .retryRelease: "assistantNoticeRetryRelease"
        case .chooseModel: "assistantNoticeChooseModel"
        case .openLocalModels: "assistantNoticeLocalModels"
        case .openAreas: "assistantNoticeOpenAreas"
        }
    }

    /// „Sprich einfach los“ nur mit geladenem Modell. Ohne Modell steht dort der Weg zur Modellwahl (Karte und Knopf „Modell
    /// auswählen“), nicht eine Einladung, die ins Leere liefe.
    private var intro: some View {
        VStack(alignment: .leading, spacing: 10) {
            if talk.canSend {
                Text("Sprich einfach los").font(.title3.weight(.semibold))
                Text("Ich antworte aus deinen Büchern, Besprechungen, Notizen und der Ablage, je nachdem, was du im Menü zulässt. "
                     + (talk.bargeIn ? "Du kannst mich jederzeit unterbrechen." : "Während ich antworte, höre ich nicht zu."))
                    .font(.subheadline).foregroundStyle(VoicePalette.secondaryText)
            } else {
                Text("Zuerst ein Modell wählen").font(.title3.weight(.semibold))
                Text("Ohne Modell kann ich nicht antworten. Wähle eines über die Karte oben oder den Knopf „Modell auswählen“.")
                    .font(.subheadline).foregroundStyle(VoicePalette.secondaryText)
            }
            // Ohne Modell führt ein Vorschlag nicht zur Antwort, sondern zuerst zur Modellwahl: Das steht hier, nicht erst im Hinweis des Knopfes.
            Text(talk.canSend ? "Oder tippe auf einen Vorschlag" : "Ein Vorschlag öffnet zuerst die Modellauswahl")
                .font(.footnote).foregroundStyle(VoicePalette.secondaryText)
                .fixedSize(horizontal: false, vertical: true).padding(.top, 4)
            ForEach(["Was wurde in der letzten Besprechung entschieden?", "Worum geht es in meinem aktuellen Buch?",
                     "Was habe ich zuletzt in meinen Notizen festgehalten?"], id: \.self) { text in suggestion(text) }
        }
    }

    /// Vorschlag zum Antippen: Fläche, Pfeil und normaler Textkontrast (nicht ausgegraut, auch ohne Modell). Ohne Modell
    /// führt das Antippen zur Modellwahl.
    private func suggestion(_ text: String) -> some View {
        Button {
            if talk.canSend { talk.send(text: text) } else { showModelChoice = true }
        } label: {
            HStack(spacing: 10) {
                Text(text).font(.subheadline).foregroundStyle(VoicePalette.text).multilineTextAlignment(.leading)
                Spacer(minLength: 4)
                Image(systemName: "arrow.up.right.circle").font(.body).foregroundStyle(VoicePalette.secondaryText).accessibilityHidden(true)
            }
            .padding(.horizontal, 12).padding(.vertical, 6)
            .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
            .background(AssistantColors.card, in: RoundedRectangle(cornerRadius: 12))
            .overlay(RoundedRectangle(cornerRadius: 12).stroke(VoicePalette.border, lineWidth: 1))
            .contentShape(RoundedRectangle(cornerRadius: 12))
        }
        .buttonStyle(.plain)
        .accessibilityHint(talk.canSend ? "Stellt diese Frage" : "Kein Modell bereit. Öffnet die Modellauswahl")
    }

    // MARK: Blasen

    /// Ein beendeter Beitrag: nur, was der Kern zeigen lässt (`displayedParts`: jede Einheit mit vollständiger Marke, sonst nichts,
    /// kein Rückfall auf den bloßen Text). Eine Unterbrechung steht als eigene Zeile unter dem Beitrag, nie im Text der Einheiten.
    @ViewBuilder private func turnView(_ turn: AssistantTurn) -> some View {
        let parts = turn.displayedParts
        if !parts.isEmpty {
            VStack(alignment: .leading, spacing: 10) {
                userBubble(turn.user)
                assistantBubble(parts: parts, interrupted: turn.interrupted, sources: turn.sources)
            }
        }
    }

    /// Die Begrüßung des Assistenten am Anfang des Gesprächs: ein Beitrag der KI mit der Kennzeichnung „Begrüßung“ (nicht „Frei formuliert“),
    /// nie im Verlauf und nie im Prompt.
    private func greetingBubble(_ text: String) -> some View {
        let part = AssistantTurnPart(text: text, unsourced: false, note: AssistantTurnPart.greetingNote)
        return HStack(spacing: 0) {
            partView(part)
                .padding(12)
                .background(AssistantColors.card, in: RoundedRectangle(cornerRadius: 16))
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier("assistantProfileGreetingBubble")
            Spacer(minLength: 24)
        }
    }

    private func userBubble(_ text: String) -> some View {
        HStack(spacing: 0) {
            Spacer(minLength: 48)
            Text(text)
                .padding(.horizontal, 12).padding(.vertical, 8)
                .background(AssistantColors.fill, in: RoundedRectangle(cornerRadius: 16))
                .foregroundStyle(.white)
                .accessibilityLabel("Du: \(text)")
        }
    }

    /// Marke je Ausgabeeinheit: Jede Einheit trägt ihre Quellzeile (Zitat, ein ganzer Quellblock) oder ihre Kennzeichnung (feste Antwort,
    /// Bestandsangabe); eine gesammelte Quellenliste unter der Antwort gibt es nicht. Ist die Antwort unterbrochen, steht das
    /// als eigene Zeile unter den Einheiten (Text und VoiceOver), nicht als „ …“ im Text der letzten Einheit.
    private func assistantBubble(parts: [AssistantTurnPart], interrupted: Bool, sources: [AssistantPassage] = []) -> some View {
        HStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 8) {
                ForEach(Array(parts.enumerated()), id: \.offset) { _, part in partView(part) }
                if parts.contains(where: { $0.note == AssistantTurnPart.summaryNote }), !sources.isEmpty { sourceList(sources) }
                if interrupted { interruptedMark }
            }
            .padding(12)
            .background(AssistantColors.card, in: RoundedRectangle(cornerRadius: 16))
            .accessibilityElement(children: .contain)
            .accessibilityLabel(interrupted ? "Antwort, unterbrochen" : "Antwort")
            Spacer(minLength: 24)
        }
    }

    /// Die Unterbrechung als eigene Zeile unter dem Beitrag: Symbol und Text („unterbrochen“, nicht nur Farbe), umbrechend. Der
    /// Auszug darüber bleibt unverändert; was nicht ganz gesprochen wurde, steht nicht im Beitrag.
    private var interruptedMark: some View {
        Label("Antwort unterbrochen", systemImage: "pause.circle")
            .font(.caption.weight(.semibold))
            .foregroundStyle(VoicePalette.secondaryText)
            .fixedSize(horizontal: false, vertical: true)
            .accessibilityLabel("Antwort unterbrochen")
            .accessibilityIdentifier("assistantInterruptedMark")
    }

    /// Eine Ausgabeeinheit mit ihrer Marke: ein Zitat (Quellzeile) oder ein Satz mit Kennzeichnung (feste Antwort, Bestandsangabe,
    /// „Ohne Quelle“). Eine Einheit ohne vollständige Marke (`isMarked`) zeigt die Ansicht nie, auch nicht als einfachen Text.
    @ViewBuilder private func partView(_ part: AssistantTurnPart) -> some View {
        if !part.isMarked {
            EmptyView()
        } else if part.quote, let source = part.source {
            quoteBlock(part, source: source)
        } else if part.isFreeform, let note = part.note {
            freeformBlock(part.text, note: note)
        } else if let note = part.note {
            markedBlock(part.text, note: note)
        } else if part.unsourced {
            unsourcedBlock(part.text)
        }
    }

    /// Wörtlicher Auszug (ein ganzer gespeicherter Block): in Anführungszeichen, mit Balken und der Quellzeile darunter („Wörtlicher
    /// Auszug aus Beispielbuch · S. 12“, Text, nicht nur Farbe). Der Block wird buchstabengleich gezeigt, Zeilenumbrüche
    /// inklusive: kein Markdown (Sternchen, Unterstriche oder Zeilenumbrüche des Auszugs blieben sonst nicht erhalten). Die
    /// Quellzeile ist nicht optional: Ein Zitat ohne gespeicherte Quellzeile (älterer Beitrag) wird nicht gezeigt, siehe
    /// `partView`, und nie mit einer erfundenen Quelle ergänzt.
    private func quoteBlock(_ part: AssistantTurnPart, source: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(verbatim: "„" + part.text + "“")
                .font(.body)
                .fixedSize(horizontal: false, vertical: true)
            Text(source)
                .font(.caption.weight(.semibold))
                .foregroundStyle(VoicePalette.secondaryText)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.leading, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(alignment: .leading) {
            RoundedRectangle(cornerRadius: 1.5).fill(AssistantColors.fill).frame(width: 3)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(source): \(part.text)")
    }

    /// Feste Erklärung: Antworten sind wörtliche Auszüge und können unvollständig sein. Steht im Hinweisbereich über dem Verlauf.
    private var excerptNotice: some View {
        Text(AssistantGrounding.excerptNotice)
            .font(.footnote)
            .foregroundStyle(VoicePalette.secondaryText)
            .fixedSize(horizontal: false, vertical: true)
            .accessibilityIdentifier("assistantExcerptNotice")
    }

    /// Frei formulierte Antwort (Stil „Natürlich“): Text und der Hinweis des Kerns darunter, ohne Balken. Die Quellen einer
    /// Zusammenfassung stehen gesammelt unter der Blase (`sourceList`), nie im Text.
    private func freeformBlock(_ text: String, note: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            VoiceMarkdown(text: text)
            Label(note, systemImage: "sparkles")
                .font(.caption.weight(.semibold))
                .foregroundStyle(VoicePalette.secondaryText)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(note): \(text)")
    }

    /// Quellen einer zusammengefassten Antwort: nur angezeigt, nie vorgelesen; Quellzeile wie bei einem Zitat, aber ohne Auszug.
    private func sourceList(_ sources: [AssistantPassage]) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text("Quellen").font(.caption.weight(.semibold)).foregroundStyle(VoicePalette.secondaryText)
            ForEach(Array(sources.enumerated()), id: \.offset) { _, passage in
                Text(passage.excerptSource)
                    .font(.caption).foregroundStyle(VoicePalette.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("assistantSources")
    }

    /// Satz, der keine Aussage über deine Inhalte macht und deshalb keine Quellzeile hat (feste Antwort des Systems,
    /// Bestandsangabe): eigene, sichtbar andere Marke statt einer Quelle. Kein Zitat (keine Anführungszeichen), gedämpfter
    /// Balken, die Kennzeichnung als Text (auch für VoiceOver).
    private func markedBlock(_ text: String, note: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            VoiceMarkdown(text: text)
            Label(note, systemImage: "info.circle")
                .font(.caption.weight(.semibold))
                .foregroundStyle(VoicePalette.secondaryText)
        }
        .padding(.leading, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(alignment: .leading) {
            RoundedRectangle(cornerRadius: 1.5).fill(VoicePalette.secondaryText).frame(width: 3)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(note): \(text)")
    }

    /// Abschnitt mit der Kennzeichnung „Ohne Quelle“ (nur bei freigeschalteter Funktion; der Kern filtert sonst schon beim Laden):
    /// Kennzeichnung als Text (auch für VoiceOver) und Balken, nicht nur Farbe.
    private func unsourcedBlock(_ text: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Label("Ohne Quelle", systemImage: "questionmark.circle")
                .font(.caption.weight(.semibold))
                .foregroundStyle(VoicePalette.secondaryText)
            VoiceMarkdown(text: text)
        }
        .padding(.leading, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(alignment: .leading) {
            RoundedRectangle(cornerRadius: 1.5).fill(VoicePalette.secondaryText).frame(width: 3)
        }
        .accessibilityElement(children: .combine)
    }

    /// Die Frage, deren Suche im Hintergrund läuft: wie ein abgeschlossener Beitrag (Nutzerblase, KI-Blase nur mit gesprochenem
    /// Zwischensatz). Steht nach den fertigen Beiträgen und vor der laufenden Runde, weil sie zuerst gestellt wurde.
    @ViewBuilder private var backgroundExchange: some View {
        if let background = talk.backgroundTurn {
            let parts = AssistantTurnPart.displayed(background.parts)
            VStack(alignment: .leading, spacing: 10) {
                userBubble(background.user)
                if !parts.isEmpty { assistantBubble(parts: parts, interrupted: false) }
            }
        }
    }

    /// Die laufende Anfrage, deren Antwort noch nicht im Verlauf steht. Die laufende Antwort zeigt nur Einheiten mit vollständiger
    /// Marke (`AssistantTurnPart.displayed`); ohne eine solche steht noch keine Antwort da (kein Rückfall auf rohen Text).
    @ViewBuilder private var currentExchange: some View {
        if let user = talk.currentUser {
            let live = AssistantTurnPart.displayed(talk.liveParts)
            VStack(alignment: .leading, spacing: 10) {
                userBubble(user)
                if !live.isEmpty {
                    assistantBubble(parts: live, interrupted: false)
                        .accessibilityAddTraits(.updatesFrequently)
                } else if talk.phase == .thinking, talk.thinking == nil {
                    HStack(spacing: 0) {
                        ProgressView()
                            .padding(12)
                            .background(AssistantColors.card, in: RoundedRectangle(cornerRadius: 16))
                            .accessibilityLabel("Antwort wird erstellt")
                        Spacer(minLength: 24)
                    }
                }
            }
        }
    }

    /// Denkanzeige unter der letzten KI-Nachricht (bzw. unter der laufenden Antwort): graue Zeile ohne Blase; Tippen klappt den
    /// gestreamten Denkprozess auf. Feste Sätze des Sprechers stehen hier nie.
    @ViewBuilder private var thinkingRow: some View {
        if let thinking = talk.thinking, thinking.active || !thinking.steps.isEmpty || !thinking.draft.isEmpty {
            VStack(alignment: .leading, spacing: 6) {
                Button {
                    thinkingOpen.toggle()
                } label: {
                    HStack(spacing: 6) {
                        if thinking.active {
                            AssistantThinkingDots(reduceMotion: reduceMotion)
                            Text("Denkt nach")
                        } else {
                            Text("Denkprozess")
                        }
                        Image(systemName: thinkingOpen ? "chevron.up" : "chevron.down").font(.caption2).accessibilityHidden(true)
                    }
                    .font(.footnote)
                    .foregroundStyle(VoicePalette.secondaryText)
                    .frame(minHeight: 44, alignment: .leading)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(thinkingOpen ? "Denkprozess ausblenden" : "Denkprozess anzeigen")
                .accessibilityValue(thinking.steps.last ?? "")
                .accessibilityIdentifier("assistantThinkingToggle")
                if thinkingOpen { thinkingDetail(thinking) }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            // Behälter bleibt durchlässig: sonst überdeckt sein Bezeichner den des Knopfs, und UI-Tests finden den Umschalter nicht.
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("assistantThinking")
        }
    }

    private func thinkingDetail(_ thinking: AssistantThinking) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            ForEach(Array(thinking.steps.enumerated()), id: \.offset) { _, step in
                Text(verbatim: step).font(.caption)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if !thinking.draft.isEmpty {
                Text(verbatim: thinking.draft).font(.caption)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.top, thinking.steps.isEmpty ? 0 : 4)
            }
        }
        .foregroundStyle(VoicePalette.secondaryText)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.leading, 12)
        .accessibilityElement(children: .combine)
    }

    /// Erkannter Text der laufenden Äußerung (vorläufiger Teil gedämpft; bei „Ergänzen“ mit dem Anfang davor).
    @ViewBuilder private var liveUserLine: some View {
        if !talk.liveUser.isEmpty {
            let stable = [talk.mergePrefix, talk.liveStable].filter { !$0.isEmpty }.joined(separator: " ")
            HStack(spacing: 0) {
                Spacer(minLength: 48)
                Text(Self.live(stable: stable, volatile: talk.liveVolatile))
                    .padding(.horizontal, 12).padding(.vertical, 8)
                    .overlay(RoundedRectangle(cornerRadius: 16).stroke(AssistantColors.fill, lineWidth: 1.5))
                    .accessibilityLabel("Erkannter Text: \(talk.liveUser)")
                    .accessibilityAddTraits(.updatesFrequently)
            }
        }
    }

    private static func live(stable: String, volatile: String) -> AttributedString {
        var a = AttributedString(stable)
        var b = AttributedString((stable.isEmpty || volatile.isEmpty ? "" : " ") + volatile)
        b.foregroundColor = VoicePalette.secondaryText
        a.append(b)
        return a
    }

    // MARK: Unten

    private var bottom: some View {
        VStack(spacing: 10) {
            if let hold = talk.holdNotice { holdBand(hold) }
            if !accessible { stateBlock }
            if typing { composer }
            if noModel { noModelControls } else { controls }
        }
        .padding(.horizontal, 16).padding(.vertical, 10)
        .background(.bar)
    }

    private var stateText: String {
        #if DEBUG
        if AssistantController.demoQuote { return "Beispielansicht" }
        #endif
        if talk.loading { return "Wird gestartet …" }
        guard talk.running else { return talk.canSend ? "Angehalten" : "Kein Modell bereit" }
        if talk.muted, talk.phase == .listening || talk.phase == .hearing { return "Stumm, ich höre nicht zu" }
        switch talk.phase {
        case .idle: return "Angehalten"
        case .listening: return "Ich höre zu"
        case .hearing: return "Du sprichst"
        case .thinking: return "Ich denke nach"
        case .speaking: return "Ich spreche"
        }
    }

    private var sourcesLine: String {
        if noModel { return "Wähle ein Modell, um zu sprechen." }
        let list = AssistantAreaSettings.list(talk.usableAreas)
        guard !list.isEmpty else { return "Keine Quellen aktiv" }
        return "\(talk.isCloud ? "Cloud" : "Lokal") · Quellen: \(list)"
    }

    /// Zustandszeile: woraus die nächste Antwort kommt. Ohne Modell gibt es nichts zu sagen (auch nicht, solange es noch lädt).
    private var contextLine: String? {
        talk.canSend ? "Antwortet aus deinen Inhalten" : nil
    }

    /// Zustand, Datenkontext und Quellen als ein Satz für VoiceOver.
    private var stateLabel: String {
        var parts = [stateText, sourcesLine]
        if let contextLine { parts.append(contextLine) }
        return parts.joined(separator: ". ")
    }

    /// Zustandszeile: Text bricht um, nichts wird abgeschnitten (kein `lineLimit`). Bei sehr großer Schrift steht der Zustandskreis
    /// über dem Text statt daneben, damit der Text die ganze Breite hat; der Block steht dann im scrollenden Inhalt.
    private var stateBlock: some View {
        let layout = accessible ? AnyLayout(VStackLayout(alignment: .leading, spacing: 8)) : AnyLayout(HStackLayout(spacing: 12))
        return layout {
            AssistantPulse(meter: talk.meter, phase: talk.phase, muted: talk.muted, loading: talk.loading, unavailable: noModel,
                           reduceMotion: reduceMotion, size: pulseSize)
            VStack(alignment: .leading, spacing: 2) {
                Text(stateText).font(.headline).fixedSize(horizontal: false, vertical: true)
                Text(sourcesLine).font(.footnote).foregroundStyle(VoicePalette.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
                if let contextLine {
                    Text(contextLine).font(.footnote).foregroundStyle(VoicePalette.secondaryText)
                        .fixedSize(horizontal: false, vertical: true)
                }
                #if DEBUG
                if let ms = talk.firstSoundMs {
                    Text("Erster Ton nach \(ms) ms").font(.caption2).foregroundStyle(VoicePalette.secondaryText)
                }
                #endif
            }
            if !accessible { Spacer(minLength: 0) }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(stateLabel)
        .accessibilityIdentifier("assistantState")
    }

    private var composer: some View {
        HStack(spacing: 8) {
            TextField("Nachricht eingeben", text: $draft, axis: .vertical)
                .lineLimit(1...4)
                .textFieldStyle(.roundedBorder)
                .focused($fieldFocused)
                .submitLabel(.send)
                .onSubmit { sendDraft() }
                .frame(minHeight: 44)
                .accessibilityIdentifier("assistantField")
            Button { sendDraft() } label: {
                Image(systemName: "arrow.up.circle.fill").font(.title).frame(width: 44, height: 44)
            }
            .disabled(draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !talk.canSend)
            .accessibilityLabel("Senden")
            .accessibilityIdentifier("assistantSend")
        }
    }

    /// Ohne Modell: Hauptknopf „Modell auswählen“ statt Stumm und Tastatur (beides ginge ins Leere).
    private var noModelControls: some View {
        let layout = accessible ? AnyLayout(VStackLayout(spacing: 8)) : AnyLayout(HStackLayout(spacing: 8))
        return layout {
            Button { showModelChoice = true } label: {
                Label("Modell auswählen", systemImage: "cpu")
                    .font(.headline)
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, minHeight: 56)
                    .padding(.horizontal, 6)
                    .foregroundStyle(.white)
                    .background(AssistantColors.fill, in: RoundedRectangle(cornerRadius: 14))
                    .contentShape(RoundedRectangle(cornerRadius: 14))
            }
            .buttonStyle(.plain)
            .accessibilityHint("Öffnet die Liste der Modelle")
            .accessibilityIdentifier("assistantChooseModel")

            AssistantControl(title: "Beenden", symbol: "phone.down.fill", tint: VoicePalette.danger, active: false,
                             stacked: !accessible, fillsRow: false) {
                close()
            }
            .accessibilityLabel("Gespräch beenden")
            .accessibilityIdentifier("assistantEnd")
        }
    }

    /// Die Bedienknöpfe: normal in einer Reihe, bei sehr großer Schrift in zwei Spalten (Symbol über dem Text, der Text bricht um),
    /// damit die feste Leiste unten nicht die halbe Höhe braucht und der Verlauf mehr als die Hälfte des Bildschirms behält.
    private var controls: some View {
        let layout = accessible ? AnyLayout(AssistantGridLayout(columns: 2, spacing: 8)) : AnyLayout(HStackLayout(spacing: 8))
        let stacked = true
        let fills = accessible
        return layout {
            AssistantControl(title: "Stumm", symbol: talk.muted ? "mic.slash.fill" : "mic.fill",
                             tint: talk.muted ? VoicePalette.danger : VoicePalette.text, active: talk.muted, stacked: stacked, fillsRow: fills) {
                talk.toggleMute()
            }
            .disabled(!talk.running)
            .accessibilityLabel("Stumm")
            .accessibilityValue(talk.muted ? "Ein" : "Aus")
            .accessibilityHint("Schaltet das Zuhören aus oder an")
            .accessibilityAddTraits(.isToggle)
            .accessibilityIdentifier("assistantMute")

            if talk.phase == .thinking || talk.phase == .speaking {
                AssistantControl(title: "Antwort anhalten", symbol: "stop.fill", tint: VoicePalette.text, active: false, stacked: stacked, fillsRow: fills) {
                    talk.stopAnswer()
                }
                .accessibilityLabel("Antwort anhalten")
                .accessibilityIdentifier("assistantStop")
            }

            AssistantControl(title: "Tastatur", symbol: typing ? "keyboard.chevron.compact.down" : "keyboard",
                             tint: VoicePalette.text, active: typing, stacked: stacked, fillsRow: fills) {
                typing.toggle()
                fieldFocused = typing
            }
            .accessibilityLabel(typing ? "Tastatur ausblenden" : "Tastatur einblenden")
            .accessibilityIdentifier("assistantKeyboard")

            AssistantControl(title: "Beenden", symbol: "phone.down.fill", tint: VoicePalette.danger, active: false, stacked: stacked, fillsRow: fills) {
                close()
            }
            .accessibilityLabel("Gespräch beenden")
            .accessibilityIdentifier("assistantEnd")
        }
    }
}

// MARK: - Bausteine

/// Knopf der Steuerleiste: Symbol über dem Text (bei sehr großer Schrift daneben), mindestens 56 pt hoch.
private struct AssistantControl: View {
    let title: String
    let symbol: String
    let tint: Color
    let active: Bool
    let stacked: Bool
    /// Füllt die Höhe der Zeile im Raster (gleich hohe Knöpfe nebeneinander); in der einzeiligen Leiste nicht.
    let fillsRow: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Group {
                if stacked {
                    VStack(spacing: 4) { icon; label }
                } else {
                    HStack(spacing: 10) { icon; label }
                }
            }
            .frame(maxWidth: .infinity, minHeight: 56, maxHeight: fillsRow ? .infinity : nil)
            .padding(.horizontal, 6)
            .foregroundStyle(tint)
            .background(active ? tint.opacity(0.14) : AssistantColors.card, in: RoundedRectangle(cornerRadius: 14))
            .contentShape(RoundedRectangle(cornerRadius: 14))
        }
        .buttonStyle(.plain)
    }

    private var icon: some View { Image(systemName: symbol).font(.title3) }

    private var label: some View {
        Text(title).font(.caption.weight(.semibold)).multilineTextAlignment(.center).fixedSize(horizontal: false, vertical: true)
    }
}

/// Drei graue Punkte, die nacheinander aufleuchten; ohne Bewegung (Bewegung reduzieren) stehen sie still.
private struct AssistantThinkingDots: View {
    let reduceMotion: Bool

    var body: some View {
        Group {
            if reduceMotion {
                dots(phase: 0, animated: false)
            } else {
                TimelineView(.animation) { context in
                    dots(phase: Int(context.date.timeIntervalSinceReferenceDate / 0.35) % 3, animated: true)
                }
            }
        }
        .accessibilityHidden(true)
    }

    private func dots(phase: Int, animated: Bool) -> some View {
        HStack(spacing: 3) {
            ForEach(0..<3, id: \.self) { index in
                Circle().fill(VoicePalette.secondaryText).frame(width: 5, height: 5)
                    .opacity(animated && index != phase ? 0.35 : 1)
            }
        }
    }
}

/// Ruhige Zustandsanzeige: Pegel beim Zuhören, langsames Atmen beim Denken, schnelleres beim Sprechen. Ohne Bewegung
/// (Bewegung reduzieren) bleibt ein stehender Ring.
private struct AssistantPulse: View {
    @ObservedObject var meter: AssistantLevelMeter
    let phase: AssistantPhase
    let muted: Bool
    let loading: Bool
    /// Kein Modell: grauer, deaktivierter Mikrofon-Zustand statt des blauen Kreises.
    let unavailable: Bool
    let reduceMotion: Bool
    let size: CGFloat

    private var fill: Color { unavailable ? VoicePalette.secondaryText : muted ? VoicePalette.danger : AssistantColors.fill }
    private var iconColor: Color { unavailable ? VoicePalette.background : .white }
    private var level: Double { muted ? 0 : meter.level }

    private var symbol: String {
        if unavailable { return "mic.slash.fill" }
        if loading { return "ellipsis" }
        if muted, phase == .listening || phase == .hearing { return "mic.slash.fill" }
        switch phase {
        case .idle: return "pause.fill"
        case .listening: return "mic.fill"
        case .hearing: return "waveform"
        case .thinking: return "ellipsis"
        case .speaking: return "speaker.wave.2.fill"
        }
    }

    var body: some View {
        ZStack {
            halo
            Circle().fill(fill).frame(width: size * 0.7, height: size * 0.7)
            Image(systemName: symbol).font(.system(size: size * 0.28, weight: .semibold)).foregroundStyle(iconColor)
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }

    @ViewBuilder private var halo: some View {
        if reduceMotion || loading || unavailable || phase == .idle {
            Circle().stroke(fill.opacity(0.3), lineWidth: 2).frame(width: size * 0.9, height: size * 0.9)
        } else if phase == .thinking || phase == .speaking {
            TimelineView(.animation) { context in
                Circle().fill(fill.opacity(0.2)).scaleEffect(wave(at: context.date))
            }
        } else {
            Circle().fill(fill.opacity(0.2))
                .scaleEffect(0.72 + 0.28 * level)
                .animation(.easeOut(duration: 0.12), value: level)
        }
    }

    private func wave(at date: Date) -> Double {
        let speed = phase == .speaking ? 5.0 : 2.2
        return 0.78 + 0.22 * (sin(date.timeIntervalSinceReferenceDate * speed) + 1) / 2
    }
}

/// Raster mit gleich breiten Spalten; jede Zeile ist so hoch wie ihr höchster Knopf (Text bricht um, nichts wird abgeschnitten).
private struct AssistantGridLayout: Layout {
    var columns: Int
    var spacing: CGFloat = 8

    private func columnWidth(_ width: CGFloat) -> CGFloat {
        (width - spacing * CGFloat(columns - 1)) / CGFloat(columns)
    }

    private func rowHeights(width: CGFloat, subviews: Subviews) -> [CGFloat] {
        let cell = columnWidth(width)
        var heights: [CGFloat] = []
        for (index, view) in subviews.enumerated() {
            let height = view.sizeThatFits(ProposedViewSize(width: cell, height: nil)).height
            if index % columns == 0 { heights.append(height) } else { heights[heights.count - 1] = max(heights[heights.count - 1], height) }
        }
        return heights
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.width ?? 320
        let heights = rowHeights(width: width, subviews: subviews)
        return CGSize(width: width, height: heights.reduce(0, +) + spacing * CGFloat(max(heights.count - 1, 0)))
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let cell = columnWidth(bounds.width)
        let heights = rowHeights(width: bounds.width, subviews: subviews)
        var y = bounds.minY
        for (row, height) in heights.enumerated() {
            for column in 0..<columns {
                let index = row * columns + column
                guard index < subviews.count else { break }
                subviews[index].place(at: CGPoint(x: bounds.minX + CGFloat(column) * (cell + spacing), y: y),
                                      proposal: ProposedViewSize(width: cell, height: height))
            }
            y += height + spacing
        }
    }
}

/// Knöpfe in Zeilen, die umbrechen (auch bei großer Schrift); zu breite Knöpfe werden auf die Breite begrenzt.
struct AssistantFlowLayout: Layout {
    var spacing: CGFloat = 6

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let result = arrange(width: proposal.width, subviews: subviews)
        return CGSize(width: result.width, height: result.height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let result = arrange(width: bounds.width, subviews: subviews)
        for (i, frame) in result.frames.enumerated() {
            subviews[i].place(at: CGPoint(x: bounds.minX + frame.minX, y: bounds.minY + frame.minY),
                              proposal: ProposedViewSize(width: frame.width, height: frame.height))
        }
    }

    private func arrange(width: CGFloat?, subviews: Subviews) -> (frames: [CGRect], width: CGFloat, height: CGFloat) {
        let limit = width ?? .infinity
        var frames: [CGRect] = []
        var x: CGFloat = 0, y: CGFloat = 0, rowHeight: CGFloat = 0, maxX: CGFloat = 0
        for view in subviews {
            var size = view.sizeThatFits(ProposedViewSize(width: width, height: nil))
            if size.width > limit { size.width = limit }
            if x > 0, x + size.width > limit { x = 0; y += rowHeight + spacing; rowHeight = 0 }
            frames.append(CGRect(origin: CGPoint(x: x, y: y), size: size))
            x += size.width + spacing
            rowHeight = max(rowHeight, size.height)
            maxX = max(maxX, x - spacing)
        }
        return (frames, maxX, y + rowHeight)
    }
}
