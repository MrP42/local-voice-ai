import SwiftUI
import UIKit

// Buch-Gespräch (Epic #307, G5): Einstieg „Fragen“ und das Gesprächsblatt. Gestaltung nach
// docs/vorschau/buch-gespraech (i1 bis i5), Nachweise in barrierefreiheit.md.

private enum BookTalkColors {
    /// Weiß darauf ≥ 4,5:1 (Vorschau: iOS-Knopf).
    static let fill = Color(red: 0, green: 0.4, blue: 0.8)
    static var card: Color { VoicePalette.text.opacity(0.06) }
}

/// Knopf „Fragen“ im Player (Vorlesen und Hörbuch): öffnet das Blatt für das laufende Buch.
struct BookTalkButton: View {
    @ObservedObject var playback = LibraryPlayback.shared
    /// Nur Symbol, für die Kopfzeile des Einheits-Players.
    var compact = false
    @State private var shown = false

    init(compact: Bool = false) { self.compact = compact }

    var body: some View {
        Button { shown = true } label: {
            if compact {
                Image(systemName: "questionmark.bubble").font(.title3).frame(width: 44, height: 44)
                    .contentShape(Rectangle())  // Trefferfläche = 44 pt, nicht nur das Symbol
            } else {
                Label("Fragen", systemImage: "questionmark.bubble")
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.white)
                    .padding(.horizontal, 14)
                    .frame(minHeight: 44)
                    .background(BookTalkColors.fill, in: Capsule())
            }
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Fragen zum Buch")
        .accessibilityHint("Öffnet das Gespräch über das Buch")
        .accessibilityIdentifier("bookTalkButton")
        .sheet(isPresented: $shown) {
            if let book = playback.book { BookTalkSheet(book: book) }
        }
    }
}

struct BookTalkSheet: View {
    @StateObject private var talk: BookTalkController
    @Environment(\.dismiss) private var dismiss
    @Environment(\.scenePhase) private var scenePhase
    @ScaledMetric(relativeTo: .body) private var micSize: CGFloat = 72
    @GestureState private var micPressed = false
    @FocusState private var fieldFocused: Bool

    init(book: LibraryBook) {
        _talk = StateObject(wrappedValue: BookTalkController(book: book))
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            hintBar
            Divider()
            content
            Divider()
            controls
        }
        .background(VoicePalette.background)
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
        .task {
            talk.close = { dismiss() }
            await talk.open()
        }
        .onDisappear { talk.teardown() }
        .onChange(of: scenePhase) { _, phase in if phase != .active { talk.sceneInactive(background: phase == .background) } }
        .onChange(of: micPressed) { _, down in if down { talk.pressDown() } else { talk.pressUp() } }
        .alert("An deinen PC senden?", isPresented: Binding(get: { talk.pendingConsent != nil }, set: { if !$0 { talk.declineConsent() } }),
               presenting: talk.pendingConsent) { pending in
            Button("Abbrechen", role: .cancel) { talk.declineConsent() }
            Button("Freigeben") { talk.grantConsent() }
        } message: { pending in
            Text(BookTalkConsentText.body(model: pending.model, desktop: talk.relays.desktopName))
        }
        .alert("KI-Antworten (Vorschau) einschalten?", isPresented: $talk.confirmingPreview) {
            Button("Abbrechen", role: .cancel) {}
            Button("Einschalten") { talk.confirmPreview() }
        } message: {
            Text(BookTalkConsentText.preview)
        }
        .alert("Spätere Kapitel einbeziehen?", isPresented: $talk.confirmingBook) {
            Button("Abbrechen", role: .cancel) {}
            Button("Einbeziehen") { talk.confirmBookScope() }
        } message: {
            Text("Antworten können dann Handlung aus Kapiteln nach deiner Leseposition verraten. Das gilt, bis du dieses Blatt schließt.")
        }
    }

    // MARK: Kopf

    private var header: some View {
        HStack(spacing: 4) {
            VStack(alignment: .leading, spacing: 2) {
                Text("Gespräch").font(.headline)
                scopeMenu
            }
            Spacer(minLength: 8)
            modelMenu
            optionsMenu
            Button { dismiss() } label: { Image(systemName: "xmark").font(.body.weight(.semibold)).frame(width: 44, height: 44) }
                .accessibilityLabel("Gespräch schließen")
        }
        .padding(.leading, 16).padding(.trailing, 4).padding(.top, 8)
    }

    private var scopeMenu: some View {
        Menu {
            Button { talk.chooseScope(.untilPosition) } label: {
                Label(talk.page.map { "Bis S. \($0) geschützt" } ?? "Bis Leseposition geschützt",
                      systemImage: talk.scope == .untilPosition ? "checkmark" : "lock")
            }
            Button { talk.chooseScope(.book) } label: {
                Label("Ganzes Buch, auch spätere Kapitel", systemImage: talk.scope == .book ? "checkmark" : "lock.open")
            }
        } label: {
            HStack(spacing: 4) {
                Image(systemName: talk.scope == .book ? "lock.open" : "lock").font(.caption)
                Text(talk.scopeTitle).font(.subheadline.weight(.semibold))
                Image(systemName: "chevron.down").font(.caption2)
            }
            .foregroundStyle(VoicePalette.text)
            .padding(.horizontal, 10)
            .frame(minHeight: 44)
            .overlay(Capsule().stroke(VoicePalette.border, lineWidth: 1))
        }
        .accessibilityLabel(talk.scope == .book ? "Bereich: ganzes Buch. Ändern" : "Bereich: \(talk.page.map { "bis Seite \($0)" } ?? "bis Leseposition") geschützt. Ändern")
    }

    /// Modellknopf im Blattkopf: Wolke, solange ein Cloud-Modell gewählt ist (Plan §11.4).
    private var modelMenu: some View {
        let cloud = talk.choice.isCloud
        return Menu {
            Picker("Modell", selection: Binding(get: { talk.choice }, set: { talk.setChoice($0) })) {
                Section("Lokal") {
                    Text("Apple-Systemmodell").tag(BookTalkModelChoice.apple)
                    Text("Gemma 4 E2B").tag(BookTalkModelChoice.gemma)
                }
                if !talk.relays.models.isEmpty {
                    Section("Cloud · über deinen PC ‚\(talk.relays.desktopName)‘") {
                        ForEach(talk.relays.models) { Label($0.label, systemImage: "cloud").tag(BookTalkModelChoice.relay($0.id)) }
                    }
                }
            }
            if let info = talk.engine.relayInfo, talk.relays.isConsented(info) {
                Button("Freigabe für \(info.label) widerrufen", systemImage: "xmark.icloud", role: .destructive) { talk.revokeCloud() }
            }
        } label: {
            HStack(spacing: 4) {
                Image(systemName: cloud ? "cloud" : "cpu").font(.body)
                Image(systemName: "chevron.down").font(.caption2)
            }
            .foregroundStyle(VoicePalette.text)
            .padding(.horizontal, 8)
            .frame(minWidth: 44, minHeight: 44)
        }
        .accessibilityLabel(cloud ? "Modell: \(talk.engine.label). Ändern" : "Modell: \(talk.engine.label). Ändern")
        .accessibilityIdentifier("bookTalkModel")
    }

    private var optionsMenu: some View {
        Menu {
            Toggle("KI-Antworten (Vorschau)", isOn: Binding(get: { talk.enabled }, set: { talk.requestEnabled($0) }))
            Toggle("Antworten mit Anton sprechen", isOn: Binding(get: { talk.speaksAloud }, set: { talk.setSpeaksAloud($0) }))
            Button("Verlauf leeren", systemImage: "trash", role: .destructive) { talk.clearHistory() }
        } label: {
            Image(systemName: "ellipsis.circle").font(.title3).frame(width: 44, height: 44)
        }
        .accessibilityLabel("Optionen")
    }

    private var hintBar: some View {
        HStack {
            Text(talk.page.map { "Vorlesen pausiert · S. \($0)" } ?? "Vorlesen pausiert")
                .font(.footnote).foregroundStyle(VoicePalette.secondaryText)
            Spacer()
            Button("Weiterlesen") { talk.resumeReading() }
                .font(.footnote.weight(.semibold))
                .frame(minHeight: 44)
                .accessibilityHint("Beendet das Gespräch und liest weiter vor")
        }
        .padding(.horizontal, 16)
        .background(BookTalkColors.card)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(talk.page.map { "Vorlesen pausiert bei Seite \($0)." } ?? "Vorlesen pausiert.")
        .accessibilityAddTraits(.isButton)
        .accessibilityAction { talk.resumeReading() }
    }

    // MARK: Verlauf

    private var content: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    if let note = talk.engine.note, !talk.loading, !talk.engine.hasModel { banner(note) }
                    if talk.engine.hasModel, talk.engine.evalStatus == .unchecked, !talk.loading {
                        banner(talk.engine.isCloud
                            ? "Vorschau: \(talk.engine.label.replacingOccurrences(of: " · Cloud über deinen PC", with: "")) hat den Prüflauf noch nicht bestanden. Antworten können falsch sein. Dein Buchtext verlässt dieses iPhone und geht über deinen PC an \(talk.engine.relayInfo?.provider.recipient ?? "den Anbieter")."
                            : "Vorschau: Das Modell hat den Prüflauf noch nicht bestanden. Antworten können falsch sein.")
                    }
                    if talk.conversation.isEmpty && !talk.loading { suggestions }
                    ForEach(talk.conversation.entries) { entry in
                        EntryView(entry: entry, talk: talk)
                    }
                    liveText
                    status
                    if let notice = talk.notice {
                        Text(notice).font(.footnote).foregroundStyle(VoicePalette.secondaryText)
                            .accessibilityLabel(notice)
                    }
                    Color.clear.frame(height: 1).id("ende")
                }
                .padding(16)
            }
            .onChange(of: talk.conversation.entries.last?.sentences.count) { _, _ in withAnimation { proxy.scrollTo("ende") } }
            .onChange(of: talk.phase) { _, _ in withAnimation { proxy.scrollTo("ende") } }
            .onChange(of: talk.conversation.entries.count) { _, _ in withAnimation { proxy.scrollTo("ende") } }
        }
    }

    private func banner(_ text: String) -> some View {
        Text(text).font(.footnote)
            .padding(12).frame(maxWidth: .infinity, alignment: .leading)
            .background(BookTalkColors.card, in: RoundedRectangle(cornerRadius: 12))
    }

    private var suggestions: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Frag zum Beispiel").font(.footnote).foregroundStyle(VoicePalette.secondaryText)
            ForEach(["Worum ging es bisher?", "Wer sind die wichtigsten Personen?", "Wo wurde das zuletzt erwähnt?"], id: \.self) { text in
                Button { talk.submit(text, input: .text) } label: {
                    Text(text).font(.subheadline).frame(maxWidth: .infinity, minHeight: 44, alignment: .leading).padding(.horizontal, 12)
                        .overlay(RoundedRectangle(cornerRadius: 12).stroke(VoicePalette.border, lineWidth: 1))
                }.buttonStyle(.plain)
            }
        }
    }

    @ViewBuilder private var liveText: some View {
        if talk.phase == .listening, !(talk.stableText + talk.volatileText).isEmpty {
            Text(Self.live(stable: talk.stableText, volatile: talk.volatileText))
                .font(.body)
                .frame(maxWidth: .infinity, alignment: .trailing)
                .accessibilityLabel("Erkannter Text")
                .accessibilityAddTraits(.updatesFrequently)
        }
    }

    private static func live(stable: String, volatile: String) -> AttributedString {
        var a = AttributedString(stable)
        var b = AttributedString((stable.isEmpty || volatile.isEmpty ? "" : " ") + volatile)
        b.foregroundColor = VoicePalette.secondaryText
        a.append(b)
        return a
    }

    @ViewBuilder private var status: some View {
        switch talk.phase {
        case .listening:
            Label("Hört zu · Stille beendet die Frage", systemImage: "waveform").font(.footnote).foregroundStyle(VoicePalette.secondaryText)
        case .thinking:
            Label(talk.engine.isCloud ? (talk.relayPhase == .sending ? "Frage geht an deinen PC …" : "PC rechnet …") : "Sucht im Buch …",
                  systemImage: talk.engine.isCloud ? "cloud" : "magnifyingglass").font(.footnote).foregroundStyle(VoicePalette.secondaryText)
        case .speaking:
            Button { talk.stopAnswer() } label: {
                Label("Anton spricht · Antwort anhalten", systemImage: "speaker.wave.2.fill").font(.footnote.weight(.semibold))
                    .frame(minHeight: 44)
            }
            .accessibilityLabel("Antwort wird vorgelesen")
            .accessibilityHint("Hält die Antwort an")
        case .idle:
            EmptyView()
        }
    }

    // MARK: Eingabe

    private var controls: some View {
        VStack(spacing: 8) {
            if talk.typing {
                HStack(spacing: 8) {
                    TextField("Frage eingeben", text: $talk.draft, axis: .vertical)
                        .lineLimit(1...4)
                        .textFieldStyle(.roundedBorder)
                        .focused($fieldFocused)
                        .submitLabel(.send)
                        .onSubmit { talk.sendDraft() }
                        .frame(minHeight: 44)
                    Button { talk.sendDraft() } label: {
                        Image(systemName: "arrow.up.circle.fill").font(.title).frame(width: 44, height: 44)
                    }
                    .disabled(talk.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    .accessibilityLabel("Frage senden")
                }
            }
            HStack {
                Button {
                    talk.typing.toggle()
                    fieldFocused = talk.typing
                } label: {
                    Image(systemName: talk.typing ? "keyboard.chevron.compact.down" : "keyboard").font(.title3).frame(width: 44, height: 44)
                }
                .accessibilityLabel(talk.typing ? "Tastatur ausblenden" : "Frage tippen")
                Spacer()
                micButton
                Spacer()
                Color.clear.frame(width: 44, height: 44)
            }
        }
        .padding(.horizontal, 16).padding(.vertical, 8)
        .background(.bar)
    }

    private var micButton: some View {
        let listening = talk.phase == .listening
        return Image(systemName: listening ? "stop.fill" : "mic.fill")
            .font(.system(size: micSize * 0.38, weight: .semibold))
            .foregroundStyle(.white)
            .frame(width: micSize, height: micSize)
            .background(listening ? VoicePalette.danger : BookTalkColors.fill, in: Circle())
            .contentShape(Circle())
            .gesture(DragGesture(minimumDistance: 0).updating($micPressed) { _, state, _ in state = true })
            .opacity(talk.loading ? 0.5 : 1)
            .accessibilityElement()
            .accessibilityLabel(talk.micLabel)
            .accessibilityAddTraits(.isButton)
            .accessibilityAction { talk.tap() }
            .accessibilityIdentifier("bookTalkMic")
    }
}

// MARK: - Eintrag

private struct EntryView: View {
    let entry: BookTalkEntry
    @ObservedObject var talk: BookTalkController

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            question
            if !entry.sentences.isEmpty { answer }
            if let reason = entry.answer.flatMap({ $0.foundOnly ? $0.reason : nil }) {
                Text(caption(reason)).font(.footnote).foregroundStyle(VoicePalette.secondaryText)
            }
            if let selection = talk.selection, selection.entryId == entry.id {
                EvidenceCard(citation: selection.citation, talk: talk)
            }
            if case .aborted(let why) = entry.status { aborted(why) }
        }
    }

    private var question: some View {
        VStack(alignment: .trailing, spacing: 2) {
            Text(entry.question)
                .padding(.horizontal, 12).padding(.vertical, 8)
                .background(BookTalkColors.fill, in: RoundedRectangle(cornerRadius: 16))
                .foregroundStyle(.white)
            Text(entry.input == .voice ? "gesprochen" : "getippt").font(.caption2).foregroundStyle(VoicePalette.secondaryText)
        }
        .frame(maxWidth: .infinity, alignment: .trailing)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Deine Frage, \(entry.input == .voice ? "gesprochen" : "getippt"): \(entry.question)")
    }

    private var answer: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let badge = talk.badges[entry.id] {
                HStack(spacing: 6) {
                    Image(systemName: badge.cloud ? "cloud" : "cpu").font(.caption)
                    Text(badge.label).font(.caption.weight(.semibold))
                    if badge.preview { Text("· Vorschau").font(.caption.weight(.semibold)) }
                }
                .foregroundStyle(VoicePalette.secondaryText)
                .accessibilityElement(children: .combine)
                .accessibilityLabel(badge.cloud ? "\(badge.label). Buchtext verlässt das Gerät." : badge.label)
            }
            ForEach(entry.sentences, id: \.index) { sentence in
                VStack(alignment: .leading, spacing: 4) {
                    Text(sentence.text).font(.body).fixedSize(horizontal: false, vertical: true)
                    if sentence.general {
                        Text("allgemein, nicht aus dem Buch").font(.caption).foregroundStyle(VoicePalette.secondaryText)
                    }
                    if !sentence.citations.isEmpty {
                        FlowLayout(spacing: 6) {
                            ForEach(Array(sentence.citations.enumerated()), id: \.offset) { _, citation in chip(citation) }
                        }
                    }
                }
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(BookTalkColors.card, in: RoundedRectangle(cornerRadius: 16))
        .accessibilityElement(children: .contain)
        .accessibilityLabel(entry.isActive ? "Antwort" : "Antwort, \(entry.citations.count) Belege")
    }

    private func chip(_ citation: BookTalkCitation) -> some View {
        let selected = talk.selection == .init(entryId: entry.id, citation: citation)
        let label = citation.page.map { "S. \($0)" } ?? "Kap. \(citation.chapterIndex + 1)"
        return Button { talk.select(citation, in: entry) } label: {
            Text(label).font(.footnote.weight(.semibold))
                .padding(.horizontal, 12).frame(minHeight: 44)
                .foregroundStyle(selected ? Color.white : VoicePalette.text)
                .background(selected ? BookTalkColors.fill : Color.clear, in: Capsule())
                .overlay(Capsule().stroke(VoicePalette.border, lineWidth: 1))
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Beleg \(citation.page.map { "Seite \($0)" } ?? citation.chapterTitle), Zitat öffnen")
    }

    private func aborted(_ why: BookTalkAbort) -> some View {
        HStack {
            Label(why.note, systemImage: "exclamationmark.circle").font(.footnote).foregroundStyle(VoicePalette.secondaryText)
            Spacer()
            if why == .scopeChanged || why == .background {
                Button(why == .scopeChanged ? "Im neuen Bereich fragen" : "Weiter fragen") { talk.reask(entry) }
                    .font(.footnote.weight(.semibold)).frame(minHeight: 44)
            }
        }
    }

    private var noModelCaption: String {
        if talk.engine.relay?.rejectedByEval == true { return "Das antwortende Modell hat den Prüflauf nicht bestanden, die Antwort wurde verworfen: hier sind nur die Fundstellen." }
        return talk.engine.isCloud ? "Dein PC hat dieses Modell nicht (mehr) verfügbar: hier sind nur die Fundstellen." : "Kein Modell bereit: hier sind nur die Fundstellen."
    }

    private func caption(_ code: BookTalkCode) -> String {
        switch code {
        case .noHits: "Dazu habe ich im freigegebenen Teil des Buchs nichts gefunden."
        case .noFrontier: "Ich kenne deine Leseposition noch nicht."
        case .budget: "Zu viel Text für das Modell: hier sind nur die Fundstellen."
        case .noModel: noModelCaption
        case .offline: talk.engine.isCloud ? "Dein PC antwortet nicht: hier sind nur die Fundstellen." : "Keine Verbindung: hier sind nur die Fundstellen."
        case .consentRequired: "Die Freigabe für das Abo-Modell fehlt: hier sind nur die Fundstellen."
        case .timeout: talk.engine.isCloud
            ? "Der PC war zu langsam (oder die Uhren von iPhone und PC weichen ab): hier sind nur die Fundstellen."
            : "Das Modell war zu langsam: hier sind nur die Fundstellen."
        case .llmFailed: talk.engine.isCloud
            ? "Dein PC konnte keine Antwort liefern (Fehler des Modells oder Grenze des Abos): hier sind nur die Fundstellen."
            : "Das Modell hat nicht geantwortet: hier sind nur die Fundstellen."
        default: "Hier sind nur die Fundstellen."
        }
    }
}

private struct EvidenceCard: View {
    let citation: BookTalkCitation
    @ObservedObject var talk: BookTalkController

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text([citation.page.map { "S. \($0)" }, citation.chapterTitle].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · "))
                    .font(.footnote.weight(.semibold)).foregroundStyle(VoicePalette.secondaryText)
                Spacer()
                Button { talk.clearSelection() } label: { Image(systemName: "xmark.circle.fill").frame(width: 44, height: 44) }
                    .accessibilityLabel("Beleg schließen")
            }
            Text("„\(citation.quote)“")
                .font(.custom("Georgia", size: 17, relativeTo: .body))
                .fixedSize(horizontal: false, vertical: true)
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 8) { buttons }
                VStack(spacing: 8) { buttons }
            }
        }
        .padding(12)
        .background(BookTalkColors.card, in: RoundedRectangle(cornerRadius: 16))
        .accessibilityElement(children: .contain)
    }

    @ViewBuilder private var buttons: some View {
        Button { talk.go(to: citation, autoplay: false) } label: {
            Label("Hinspringen", systemImage: "arrow.turn.down.right").frame(maxWidth: .infinity, minHeight: 44)
        }.buttonStyle(.bordered)
        Button { talk.go(to: citation, autoplay: true) } label: {
            Label("Ab hier vorlesen", systemImage: "play.fill").frame(maxWidth: .infinity, minHeight: 44)
        }.buttonStyle(.borderedProminent)
    }
}

// MARK: - Umbruch

/// Chips in Zeilen, die umbrechen (auch bei großer Schrift).
private struct FlowLayout: Layout {
    var spacing: CGFloat = 6

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = arrange(width: proposal.width ?? .infinity, subviews: subviews)
        return CGSize(width: rows.width, height: rows.height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let result = arrange(width: bounds.width, subviews: subviews)
        for (i, origin) in result.origins.enumerated() {
            subviews[i].place(at: CGPoint(x: bounds.minX + origin.x, y: bounds.minY + origin.y), proposal: .unspecified)
        }
    }

    private func arrange(width: CGFloat, subviews: Subviews) -> (origins: [CGPoint], width: CGFloat, height: CGFloat) {
        var origins: [CGPoint] = []
        var x: CGFloat = 0, y: CGFloat = 0, rowHeight: CGFloat = 0, maxX: CGFloat = 0
        for view in subviews {
            let size = view.sizeThatFits(.unspecified)
            if x > 0, x + size.width > width { x = 0; y += rowHeight + spacing; rowHeight = 0 }
            origins.append(CGPoint(x: x, y: y))
            x += size.width + spacing
            rowHeight = max(rowHeight, size.height)
            maxX = max(maxX, x - spacing)
        }
        return (origins, maxX, y + rowHeight)
    }
}

// MARK: - Texte (Plan §11.4, §11.5, R9)

enum BookTalkConsentText {
    /// Freigabedialog je Cloud-Modell: Empfänger, Datenmenge, Region, Speicherung, Kontingent (R6, R9).
    static func body(model: BookTalkRelayModelInfo, desktop: String) -> String {
        let to = model.provider.recipient
        return "Buchtext verlässt dieses iPhone. Frage, die letzten zwei Wechsel und Buchauszüge (bis ca. 20.000 Zeichen je Frage, beim ersten Mal der Höchstwert) gehen verschlüsselt an deinen PC ‚\(desktop)‘ und von dort an \(to) (\(model.label), dein Abo). Nie Audio, nie das ganze Buch.\n\nRegion: unbekannt, \(to) nennt für Abo-Pfade keine EU-Bindung. Speicherung und Training richten sich nach deinem Abo-Vertrag; die App kann das nicht prüfen. Zählt auf dein \(model.provider.quotaName)-Abo-Kontingent; Zusatzkosten keine, solange am PC kein API-Schlüssel gesetzt ist.\n\nWiderruf jederzeit im Modellmenü."
    }

    static let preview = "Vorschau: Das gewählte Modell hat den Prüflauf noch nicht bestanden. Antworten können falsch sein. Der Spoilerschutz verringert das Risiko, schließt es aber nicht aus."
}

// MARK: - Einstellungen

/// Abschnitt „Buch-Gespräch“ in den Einstellungen: Vorschau-Schalter (Standard aus), Modellwahl inkl. Abo-Modelle über den PC,
/// Einrichtung und Widerruf, Verlauf löschen.
struct BookTalkSettings: View {
    @AppStorage(BookTalkEngine.enabledKey) private var enabled = false
    @AppStorage(BookTalkModelChoice.key) private var choice = BookTalkModelChoice.apple.rawValue
    @ObservedObject private var relays = BookTalkRelayStore.shared
    @State private var anton = BookTalkController.storedSpeaksAloud()
    @State private var confirming = false
    @State private var confirmingPreview = false
    @State private var cleared = false
    @State private var scanning = false
    @State private var code = ""
    @State private var enrollError: String?

    var body: some View {
        Section {
            Toggle("KI-Antworten (Vorschau)", isOn: Binding(get: { enabled }, set: { value in
                if value { confirmingPreview = true } else { enabled = false }
            }))
            Picker("Modell", selection: $choice) {
                Text("Apple-Systemmodell · lokal").tag(BookTalkModelChoice.apple.rawValue)
                Text("Gemma 4 E2B · lokal").tag(BookTalkModelChoice.gemma.rawValue)
                ForEach(relays.models) { Text("\($0.label) · Cloud").tag(BookTalkModelChoice.relay($0.id).rawValue) }
            }
            Toggle("Antworten mit Anton sprechen", isOn: $anton)
                .onChange(of: anton) { _, value in UserDefaults.standard.set(value, forKey: BookTalkController.antonKey) }
            Button("Alle Buchgespräche löschen", role: .destructive) { confirming = true }
                .frame(minHeight: 44)
            if cleared { Text("Alle Buchgespräche sind gelöscht.").font(.caption) }
        } header: {
            Text("Buch-Gespräch")
        } footer: {
            Text("Aus: das Gespräch zeigt nur Fundstellen im Buch. Fragen und Antworten bleiben auf diesem iPhone und werden nicht gesichert.")
        }
        .confirmationDialog("Alle Buchgespräche löschen?", isPresented: $confirming, titleVisibility: .visible) {
            Button("Löschen", role: .destructive) {
                BookTalkHistoryStore(directory: BookTalkHistoryStore.standardDirectory()).clearAll()
                cleared = true
            }
            Button("Abbrechen", role: .cancel) {}
        }
        .alert("KI-Antworten (Vorschau) einschalten?", isPresented: $confirmingPreview) {
            Button("Abbrechen", role: .cancel) {}
            Button("Einschalten") { enabled = true }
        } message: { Text(BookTalkConsentText.preview) }
        relaySection
    }

    @ViewBuilder private var relaySection: some View {
        Section {
            if let e = relays.enrollment {
                Label("Eingerichtet mit ‚\(e.desktopName)‘", systemImage: "desktopcomputer")
                if relays.models.isEmpty {
                    Text(relays.status.isEmpty ? "Keine Abo-Modelle freigegeben." : relays.status).font(.footnote).foregroundStyle(VoicePalette.secondaryText)
                }
                ForEach(relays.models) { model in
                    HStack {
                        VStack(alignment: .leading) {
                            Text(model.label)
                            Text(relays.isConsented(model) ? "Freigegeben" : "Noch nicht freigegeben")
                                .font(.caption).foregroundStyle(VoicePalette.secondaryText)
                        }
                        Spacer()
                        if relays.isConsented(model) {
                            Button("Widerrufen", role: .destructive) { relays.revoke(model) }.frame(minHeight: 44)
                        }
                    }
                }
                Button("Modelle neu laden") { Task { await relays.refresh() } }.frame(minHeight: 44).disabled(relays.refreshing)
                Button("Einrichtung entfernen", role: .destructive) { relays.forget() }.frame(minHeight: 44)
            } else {
                Button { scanning = true } label: { Label("Einrichtungscode scannen", systemImage: "qrcode.viewfinder").frame(minHeight: 44) }
                Button { paste() } label: { Label("Code aus der Zwischenablage", systemImage: "doc.on.clipboard").frame(minHeight: 44) }
                if let enrollError { Text(enrollError).font(.footnote).foregroundStyle(VoicePalette.danger) }
            }
        } header: {
            Text("Abo-Modelle über deinen PC")
        } footer: {
            Text("Claude- und Codex-Abo laufen nur über die CLI auf deinem PC. Am PC unter Einstellungen › Modelle je Funktion dieses iPhone freigeben und den Einrichtungscode scannen. Buchtext verlässt dann das iPhone (verschlüsselt, über deinen PC an Anthropic bzw. OpenAI), erst nach deiner Freigabe je Modell.")
        }
        .sheet(isPresented: $scanning) {
            NavigationStack {
                RelayCodeScanner { text in scanning = false; apply(text) }
                    .ignoresSafeArea()
                    .navigationTitle("Einrichtungscode")
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Abbrechen") { scanning = false } } }
            }
        }
    }

    private func paste() { apply(UIPasteboard.general.string ?? "") }

    private func apply(_ text: String) {
        enrollError = relays.enroll(text)
    }
}
