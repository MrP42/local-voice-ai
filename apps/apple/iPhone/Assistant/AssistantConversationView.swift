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
    @FocusState private var fieldFocused: Bool

    /// Kein Modell, und es startet auch keins gerade: statt Pause-Knopf der deaktivierte Mikrofon-Zustand und „Modell auswählen“.
    private var noModel: Bool { !talk.canSend && !talk.loading && !talk.running }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            content
                .sheet(isPresented: $showLocalModels, onDismiss: localModelsClosed) { ModelPanel(model: VoiceModel.shared) }
            Divider()
            bottom
                .confirmationDialog("Modell auswählen", isPresented: $showModelChoice, titleVisibility: .visible) { modelChoiceButtons }
        }
        .background(VoicePalette.background)
        .task {
            gemmaReady = AssistantEngine.gemmaInstalled()
            await talk.start()
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
        case .chooseModel: showModelChoice = true
        case .openLocalModels: showLocalModels = true
        }
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

    // MARK: Kopf

    private var header: some View {
        HStack(spacing: 4) {
            Button(action: close) {
                Image(systemName: "xmark").font(.body.weight(.semibold)).frame(width: 44, height: 44)
                    .contentShape(Rectangle())
            }
            .accessibilityLabel("Schließen")
            .accessibilityHint("Beendet das Gespräch")
            .accessibilityIdentifier("assistantClose")
            Spacer(minLength: 8)
            modelMenu
            optionsMenu
        }
        .padding(.horizontal, 8).padding(.top, 4)
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
                    Text(talk.modelLabel).font(.footnote).lineLimit(1)
                }
                Image(systemName: "chevron.down").font(.caption2)
            }
            .foregroundStyle(VoicePalette.text)
            .padding(.horizontal, 10)
            .frame(minHeight: 44)
            .overlay(Capsule().stroke(VoicePalette.border, lineWidth: 1))
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
            // Kurz, damit das Menü den Text nicht abschneidet; die Erklärung steht im Hinweis für Bedienungshilfen.
            Toggle("Beim Antworten zuhören", isOn: Binding(get: { talk.bargeIn }, set: { talk.setBargeIn($0) }))
                .accessibilityHint("Du kannst die Antwort mit deiner Stimme unterbrechen")
            Button("Neues Gespräch", systemImage: "arrow.counterclockwise", role: .destructive) { talk.newConversation() }
        } label: {
            Image(systemName: "ellipsis.circle").font(.title3).frame(width: 44, height: 44)
        }
        .accessibilityLabel("Mehr")
        .accessibilityHint("Bereiche, neues Gespräch")
        .accessibilityIdentifier("assistantMenu")
    }

    // MARK: Verlauf

    private var showsIntro: Bool {
        talk.turns.isEmpty && talk.currentUser == nil && talk.liveUser.isEmpty && !talk.loading
    }

    private var content: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    if let progress = talk.preparing { preparing(progress) }
                    if let notice = talk.notice { noticeView(notice) }
                    if talk.canSend { excerptNotice }
                    if hub.skipped > 0 {
                        Label("Wegen der Speichergrenze fehlen \(hub.skipped) Bücher oder Besprechungen im Gespräch.", systemImage: "exclamationmark.circle")
                            .font(.footnote).foregroundStyle(VoicePalette.secondaryText)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    if showsIntro { intro }
                    ForEach(Array(talk.turns.enumerated()), id: \.offset) { _, turn in turnView(turn) }
                    currentExchange
                    liveUserLine
                    Color.clear.frame(height: 1).id("ende")
                }
                .padding(16)
            }
            .scrollDismissesKeyboard(.interactively)
            .onChange(of: talk.turns.count) { _, _ in scroll(proxy, animated: true) }
            .onChange(of: talk.liveAnswer) { _, _ in scroll(proxy, animated: true) }
            .onChange(of: talk.liveUser) { _, _ in scroll(proxy, animated: false) }
            .onChange(of: talk.phase) { _, _ in scroll(proxy, animated: true) }
            .onChange(of: typing) { _, _ in scroll(proxy, animated: false) }
        }
    }

    private func scroll(_ proxy: ScrollViewProxy, animated: Bool) {
        if animated, !reduceMotion {
            withAnimation { proxy.scrollTo("ende", anchor: .bottom) }
        } else {
            proxy.scrollTo("ende", anchor: .bottom)
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

    private func noticeView(_ notice: AssistantNotice) -> some View {
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
        .accessibilityIdentifier("assistantNotice")
    }

    /// Die Mindesthöhe von 44 pt steht IM Label: Die ganze Fläche des Knopfes trifft, nicht nur der Text; ein `frame` um den
    /// Knopf herum machte nur den Rand höher, nicht die Tippfläche.
    private func noticeButton(_ action: AssistantNotice.Action) -> some View {
        Button { perform(action) } label: {
            Text(actionTitle(action))
                .font(.footnote.weight(.semibold))
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
        case .chooseModel: "Modell auswählen"
        case .openLocalModels: "Lokale Modelle öffnen"
        }
    }

    private func actionIdentifier(_ action: AssistantNotice.Action) -> String {
        switch action {
        case .retrySend: "assistantNoticeRetrySend"
        case .resume: "assistantNoticeResume"
        case .retryStart: "assistantNoticeRetryStart"
        case .retryAudit: "assistantNoticeRetryAudit"
        case .chooseModel: "assistantNoticeChooseModel"
        case .openLocalModels: "assistantNoticeLocalModels"
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
            Text("Oder tippe auf einen Vorschlag").font(.footnote).foregroundStyle(VoicePalette.secondaryText).padding(.top, 4)
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

    private func turnView(_ turn: AssistantTurn) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            userBubble(turn.user)
            assistantBubble(parts: parts(of: turn), interrupted: turn.interrupted, legacyChips: legacyChips(of: turn))
        }
    }

    /// Abschnitte des Beitrags: gemischte Antworten tragen ihre Abschnitte, sonst gilt `unsourced` für den ganzen Text
    /// (ältere Beiträge: wie „Allgemeinwissen“).
    private func parts(of turn: AssistantTurn) -> [AssistantTurnPart] {
        var list = turn.parts.isEmpty ? [AssistantTurnPart(text: turn.assistant, unsourced: turn.unsourced)] : turn.parts
        if turn.interrupted, let last = list.indices.last { list[last].text += " …" }
        return list
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

    /// Marke je Satz: Jeder Satz trägt seine Quellzeile (Zitat) oder seine Kennzeichnung (feste Antwort, Bestandsangabe); eine
    /// gesammelte Quellenliste unter der Antwort gibt es nicht mehr. `legacyChips`: nur für ältere Beiträge ohne Marken je Satz.
    private func assistantBubble(parts: [AssistantTurnPart], interrupted: Bool, legacyChips: [SourceChip] = []) -> some View {
        HStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 8) {
                ForEach(Array(parts.enumerated()), id: \.offset) { _, part in partView(part) }
                if !legacyChips.isEmpty {
                    AssistantFlowLayout(spacing: 6) {
                        ForEach(legacyChips, id: \.self) { chip($0) }
                    }
                }
            }
            .padding(12)
            .background(AssistantColors.card, in: RoundedRectangle(cornerRadius: 16))
            .accessibilityElement(children: .contain)
            .accessibilityLabel(interrupted ? "Antwort, unterbrochen" : "Antwort")
            Spacer(minLength: 24)
        }
    }

    /// Marke je Satz: ein Zitat (Quellzeile), ein Satz mit Kennzeichnung (feste Antwort, Bestandsangabe) oder, nur bei älteren
    /// Beiträgen, Text ohne Marke.
    @ViewBuilder private func partView(_ part: AssistantTurnPart) -> some View {
        if part.quote {
            quoteBlock(part)
        } else if let note = part.note {
            markedBlock(part.text, note: note)
        } else if part.unsourced {
            unsourcedBlock(part.text)
        } else {
            VoiceMarkdown(text: part.text)
        }
    }

    /// Wörtlicher Auszug (ein ganzer gespeicherter Block): in Anführungszeichen, mit Balken und der Quellzeile darunter („Wörtlicher
    /// Auszug aus Beispielbuch · S. 12“, Text, nicht nur Farbe). Der Block wird buchstabengleich gezeigt, Zeilenumbrüche
    /// inklusive: kein Markdown (Sternchen, Unterstriche oder Zeilenumbrüche des Auszugs blieben sonst nicht erhalten). Die
    /// Quellzeile ist nicht optional: Der Kern gibt jedem Zitat genau eine mit; fehlt sie (älterer, vor der Zeile gespeicherter
    /// Beitrag), steht das offen da, statt dass die Zeile still entfällt. Eine ältere Quellzeile ohne den Beginn „Wörtlicher
    /// Auszug aus“ bekommt ihn hier vorangestellt.
    private func quoteBlock(_ part: AssistantTurnPart) -> some View {
        let source = part.source.map(Self.excerptLine) ?? "Quelle nicht gespeichert"
        return VStack(alignment: .leading, spacing: 4) {
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
        .accessibilityLabel(part.source.map { "\(Self.excerptLine($0)): \(part.text)" } ?? "Zitat, Quelle nicht gespeichert: \(part.text)")
    }

    /// Quellzeile mit dem festen Beginn „Wörtlicher Auszug aus“ (ältere Beiträge speicherten nur „Buch · Titel · S. 12“).
    private static func excerptLine(_ source: String) -> String {
        source.hasPrefix(AssistantPassage.excerptPrefix) ? source : AssistantPassage.excerptPrefix + source
    }

    /// Feste Erklärung: Antworten sind wörtliche Auszüge und können unvollständig sein. Steht im Hinweisbereich über dem Verlauf.
    private var excerptNotice: some View {
        Text(AssistantGrounding.excerptNotice)
            .font(.footnote)
            .foregroundStyle(VoicePalette.secondaryText)
            .fixedSize(horizontal: false, vertical: true)
            .accessibilityIdentifier("assistantExcerptNotice")
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

    // MARK: Ältere Beiträge ohne Marke je Satz

    /// Abschnitt ohne Quelle (nur ältere Beiträge; der Kern gibt heute keine mehr aus): Kennzeichnung als Text („Ohne Quelle“,
    /// auch für VoiceOver) und Balken, nicht nur Farbe.
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

    /// Eine Quelle oder ein Hinweis unter der Antwort.
    private struct SourceChip: Hashable {
        let text: String
        let isSource: Bool
    }

    private func chip(_ chip: SourceChip) -> some View {
        Text(chip.text)
            .font(.footnote.weight(.medium))
            .foregroundStyle(VoicePalette.secondaryText)
            .fixedSize(horizontal: false, vertical: true)
            .padding(.horizontal, 10).padding(.vertical, 5)
            .overlay(RoundedRectangle(cornerRadius: 12).stroke(VoicePalette.border, lineWidth: 1))
            .accessibilityLabel(chip.isSource ? "Quelle: \(chip.text)" : "Hinweis: \(chip.text)")
    }

    /// Nur ältere Beiträge (ohne Marke je Satz): die deckenden Quellen (ohne Doppelte) und, wenn Sätze ohne Quelle gesprochen
    /// wurden, der Hinweis „Allgemeinwissen · ohne Quelle“ (ganz) oder „teils ohne Quelle“ (gemischt). Beiträge mit Marken je Satz
    /// (Zitat mit Quellzeile, Kennzeichnung) haben keine gesammelte Liste.
    private func legacyChips(of turn: AssistantTurn) -> [SourceChip] {
        guard !turn.parts.contains(where: { $0.quote || $0.note != nil }) else { return [] }
        var seen = Set<String>()
        var list = turn.sources.map { AssistantController.chip($0) }.filter { seen.insert($0).inserted }
            .map { SourceChip(text: $0, isSource: true) }
        if turn.unsourced { list.append(SourceChip(text: !turn.parts.isEmpty ? "teils ohne Quelle" : "Allgemeinwissen · ohne Quelle", isSource: false)) }
        return list
    }

    /// Abschnitte der laufenden Antwort (ohne Angabe der Steuerung: ein Abschnitt mit Quelle).
    private var liveAnswerParts: [AssistantTurnPart] {
        talk.liveParts.isEmpty ? [AssistantTurnPart(text: talk.liveAnswer, unsourced: false)] : talk.liveParts
    }

    /// Die laufende Anfrage, deren Antwort noch nicht im Verlauf steht.
    @ViewBuilder private var currentExchange: some View {
        if let user = talk.currentUser {
            VStack(alignment: .leading, spacing: 10) {
                userBubble(user)
                if !talk.liveAnswer.isEmpty {
                    assistantBubble(parts: liveAnswerParts, interrupted: false)
                        .accessibilityAddTraits(.updatesFrequently)
                } else if talk.phase == .thinking {
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
            stateBlock
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
        let list = AssistantAreaSettings.list(talk.areas)
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

    private var stateBlock: some View {
        HStack(spacing: 12) {
            AssistantPulse(meter: talk.meter, phase: talk.phase, muted: talk.muted, loading: talk.loading, unavailable: noModel,
                           reduceMotion: reduceMotion, size: pulseSize)
            VStack(alignment: .leading, spacing: 2) {
                Text(stateText).font(.headline)
                Text(sourcesLine).font(.footnote).foregroundStyle(VoicePalette.secondaryText).lineLimit(2)
                if let contextLine {
                    Text(contextLine).font(.footnote).foregroundStyle(VoicePalette.secondaryText).lineLimit(2)
                }
                #if DEBUG
                if let ms = talk.firstSoundMs {
                    Text("Erster Ton nach \(ms) ms").font(.caption2).foregroundStyle(VoicePalette.secondaryText)
                }
                #endif
            }
            Spacer(minLength: 0)
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
        let layout = typeSize.isAccessibilitySize ? AnyLayout(VStackLayout(spacing: 8)) : AnyLayout(HStackLayout(spacing: 8))
        return layout {
            Button { showModelChoice = true } label: {
                Label("Modell auswählen", systemImage: "cpu")
                    .font(.headline)
                    .multilineTextAlignment(.center)
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
                             stacked: !typeSize.isAccessibilitySize) {
                close()
            }
            .accessibilityLabel("Gespräch beenden")
            .accessibilityIdentifier("assistantEnd")
        }
    }

    private var controls: some View {
        let layout = typeSize.isAccessibilitySize ? AnyLayout(VStackLayout(spacing: 8)) : AnyLayout(HStackLayout(spacing: 8))
        let stacked = !typeSize.isAccessibilitySize
        return layout {
            AssistantControl(title: "Stumm", symbol: talk.muted ? "mic.slash.fill" : "mic.fill",
                             tint: talk.muted ? VoicePalette.danger : VoicePalette.text, active: talk.muted, stacked: stacked) {
                talk.toggleMute()
            }
            .disabled(!talk.running)
            .accessibilityLabel("Stumm")
            .accessibilityValue(talk.muted ? "Ein" : "Aus")
            .accessibilityHint("Schaltet das Zuhören aus oder an")
            .accessibilityAddTraits(.isToggle)
            .accessibilityIdentifier("assistantMute")

            if talk.phase == .thinking || talk.phase == .speaking {
                AssistantControl(title: "Antwort anhalten", symbol: "stop.fill", tint: VoicePalette.text, active: false, stacked: stacked) {
                    talk.stopAnswer()
                }
                .accessibilityLabel("Antwort anhalten")
                .accessibilityIdentifier("assistantStop")
            }

            AssistantControl(title: "Tastatur", symbol: typing ? "keyboard.chevron.compact.down" : "keyboard",
                             tint: VoicePalette.text, active: typing, stacked: stacked) {
                typing.toggle()
                fieldFocused = typing
            }
            .accessibilityLabel(typing ? "Tastatur ausblenden" : "Tastatur einblenden")
            .accessibilityIdentifier("assistantKeyboard")

            AssistantControl(title: "Beenden", symbol: "phone.down.fill", tint: VoicePalette.danger, active: false, stacked: stacked) {
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
            .frame(maxWidth: .infinity, minHeight: 56)
            .padding(.horizontal, 6)
            .foregroundStyle(tint)
            .background(active ? tint.opacity(0.14) : AssistantColors.card, in: RoundedRectangle(cornerRadius: 14))
            .contentShape(RoundedRectangle(cornerRadius: 14))
        }
        .buttonStyle(.plain)
    }

    private var icon: some View { Image(systemName: symbol).font(.title3) }

    private var label: some View {
        Text(title).font(.caption.weight(.semibold)).lineLimit(2).multilineTextAlignment(.center)
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

/// Quellen-Chips in Zeilen, die umbrechen (auch bei großer Schrift); zu lange Chips werden auf die Breite begrenzt.
private struct AssistantFlowLayout: Layout {
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
