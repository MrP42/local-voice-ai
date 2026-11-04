import SwiftUI

// Oberfläche der Live-Besprechung nach der von Codex abgenommenen Vorschau
// (docs/vorschau/ios-live-besprechung, Runde 3, 05e93140).

/// Einwilligung vor dem Start und bei „Person kommt hinzu“. Ohne vollständige
/// Angaben bleibt der Startknopf aus.
struct ConsentSheet: View {
    enum Mode { case start, personJoined(Double) }
    let mode: Mode
    let onConfirm: (ConsentRecord, String) -> Void
    let onCancel: () -> Void
    @AppStorage("consent.purpose") private var purpose = "Protokoll der Besprechung"
    @AppStorage("consent.controller") private var controller = ""
    @AppStorage("consent.retentionDays") private var retentionDays = 90
    // KI-Auswertung ist Opt-in je Aufnahme: nie vorbelegt, nicht gespeichert.
    @State private var ai = false
    @State private var title = ""

    private var isStart: Bool { if case .start = mode { return true }; return false }
    private var record: ConsentRecord {
        let offset: Double = { if case .personJoined(let s) = mode { return s }; return 0 }()
        return ConsentRecord(confirmedAt: Date(), channel: .app, reason: isStart ? .start : .personJoined,
                             scopes: ai ? [.recording, .transcription, .aiAnalysis] : [.recording, .transcription],
                             purpose: purpose, controller: controller, retentionDays: retentionDays, audioOffset: offset)
    }

    var body: some View {
        NavigationStack {
            Form {
                if isStart {
                    Section { TextField("Titel, z. B. Teamrunde Haushalt", text: $title) }
                }
                Section {
                    Label { Text("Zustimmung umfasst: **Aufnahme** und **Transkription** auf diesem iPhone.") } icon: { Image(systemName: "record.circle") }
                    Toggle(isOn: $ai) {
                        Label { Text("Optional zusätzlich: **KI-Auswertung** auf dem iPhone (Stand, Aufgaben, offene Fragen)") } icon: { Image(systemName: "sparkles") }
                    }.frame(minHeight: 44)
                }
                Section {
                    TextField("Zweck", text: $purpose)
                    TextField("Verantwortlich (z. B. Amt 12)", text: $controller)
                    Stepper("Löschfrist: \(retentionDays) Tage", value: $retentionDays, in: 1...3650)
                    LabeledContent("Widerruf", value: "Aufnahme stoppen, Dokument löschen")
                } footer: {
                    Text(isStart
                         ? "Alle Anwesenden müssen vor dem Start zugestimmt haben. Kommt jemand hinzu, pausiere die Aufnahme und hole die Zustimmung erneut ein. Deine Bestätigung wird mit Zeitpunkt an der Aufnahme gespeichert."
                         : "Die Aufnahme ist pausiert. Erst wenn auch die neu hinzugekommene Person zugestimmt hat, geht es weiter.")
                }
                Section {
                    Button { onConfirm(record, title) } label: {
                        Text(isStart ? "Alle haben zugestimmt – Aufnahme starten" : "Alle haben zugestimmt – fortsetzen")
                            .frame(maxWidth: .infinity, minHeight: 44).foregroundStyle(VoicePalette.ink)
                    }
                    .buttonStyle(VoicePrimaryButtonStyle(recording: false))
                    .disabled(!record.isComplete).opacity(record.isComplete ? 1 : 0.4)
                    .accessibilityIdentifier("consentConfirm")
                    Button("Abbrechen", role: .cancel, action: onCancel).frame(maxWidth: .infinity, minHeight: 44)
                }
            }
            .navigationTitle(isStart ? "Besprechung aufnehmen" : "Person kommt hinzu")
            .navigationBarTitleDisplayMode(.inline)
        }
    }
}

/// Vollbild während der Aufnahme: Stand, Transkript, Notizen und Aktionsleiste.
struct LiveMeetingScreen: View {
    @StateObject private var model: LiveMeetingModel
    @Environment(\.dismiss) private var dismiss
    @State private var tab = 0
    @State private var consentFor: ConsentSheet.Mode? = .start
    @State private var confirmStop = false

    init(library: MeetingLibrary) {
        _model = StateObject(wrappedValue: LiveMeetingModel(finish: { url, session in await library.importLive(url, session) }))
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            if let hint = model.hint {
                Label(hint, systemImage: "exclamationmark.triangle")
                    .font(.footnote).frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 16).padding(.vertical, 8)
                    .background(Color.orange.opacity(0.15))
                    .accessibilityAddTraits(.isStaticText)
            }
            Picker("Ansicht", selection: $tab) {
                Text("Stand").tag(0); Text("Transkript").tag(1); Text("Notizen").tag(2)
            }.pickerStyle(.segmented).padding(.horizontal, 16).padding(.vertical, 10)
            Group {
                switch tab {
                case 0: LiveStateView(model: model, jump: { id in model.jumpTo = id; tab = 1 })
                case 1: LiveTranscriptView(model: model)
                default: TextEditor(text: $model.notes).padding(12).onChange(of: model.notes) { _, _ in model.saveNotes() }
                        .accessibilityLabel("Eigene Notizen")
                }
            }.frame(maxHeight: .infinity)
            actionBar
        }
        .background(VoicePalette.background)
        .onChange(of: model.phase) { _, phase in if phase == .finished { dismiss() } }
        .sheet(isPresented: Binding(get: { consentFor != nil }, set: { if !$0 { consentFor = nil } })) {
            if let mode = consentFor {
                ConsentSheet(mode: mode, onConfirm: { record, title in
                    consentFor = nil
                    if case .start = mode { Task { await model.start(title: title, consent: record) } } else { Task { await model.confirmJoin(record) } }
                }, onCancel: {
                    consentFor = nil
                    if case .start = mode { dismiss() }
                }).interactiveDismissDisabled()
            }
        }
        .confirmationDialog("Aufnahme beenden?", isPresented: $confirmStop) {
            Button("Aufnahme beenden und sichern", role: .destructive) { Task { await model.stop(); dismiss() } }
        }
        .alert("Hinweis", isPresented: Binding(get: { model.error != nil }, set: { if !$0 { model.error = nil } })) {
            Button("OK") { if model.phase == .idle { dismiss() } }
        } message: { Text(model.error ?? "") }
    }

    private var header: some View {
        HStack(spacing: 10) {
            HStack(spacing: 6) {
                Circle().fill(model.phase == .recording ? Color.red : Color.gray).frame(width: 10, height: 10)
                Text(Self.clock(model.seconds)).font(.headline.monospacedDigit())
            }.accessibilityElement(children: .combine).accessibilityLabel(model.phase == .recording ? "Aufnahme läuft, \(Self.clock(model.seconds))" : "Aufnahme pausiert")
            Text(model.title.isEmpty ? "Besprechung" : model.title).font(.headline).lineLimit(2).frame(maxWidth: .infinity, alignment: .leading)
            if !model.aiStatus.isEmpty {
                Text(model.aiStatus.hasPrefix("KI pausiert") ? "KI pausiert" : model.aiStatus).font(.caption.weight(.semibold))
                    .padding(.horizontal, 10).padding(.vertical, 4).background(Capsule().fill(Color.secondary.opacity(0.15)))
                    .foregroundStyle(.orange)
            } else if let last = model.lastTick {
                Text("KI · \(last, style: .relative)").font(.caption.weight(.semibold))
                    .padding(.horizontal, 10).padding(.vertical, 4).background(Capsule().fill(Color.secondary.opacity(0.15)))
            }
        }.padding(.horizontal, 16).padding(.vertical, 8)
    }

    private var actionBar: some View {
        HStack {
            barButton(model.phase == .paused ? "Weiter" : "Pause", model.phase == .paused ? "play.fill" : "pause.fill") {
                model.phase == .paused ? model.resume() : model.pause()
            }
            barButton("Person kommt", "person.badge.plus") { model.pause(); consentFor = .personJoined(model.audioPosition) }
            barButton("Notiz", "square.and.pencil") { tab = 2 }
            Button { confirmStop = true } label: {
                VStack(spacing: 3) {
                    ZStack { Circle().fill(Color.red).frame(width: 44, height: 44); RoundedRectangle(cornerRadius: 4).fill(.white).frame(width: 16, height: 16) }
                    Text("Aufnahme beenden").font(.caption2).multilineTextAlignment(.center)
                }.frame(maxWidth: .infinity, minHeight: 52)
            }.buttonStyle(.plain).accessibilityLabel("Aufnahme beenden")
        }
        .disabled(model.phase == .stopping || model.phase == .idle || model.phase == .starting)
        .padding(.horizontal, 8).padding(.top, 6).padding(.bottom, 4)
        .background(VoicePalette.background).overlay(alignment: .top) { Divider() }
    }

    private func barButton(_ title: String, _ icon: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            VStack(spacing: 3) { Image(systemName: icon).font(.title3); Text(title).font(.caption) }
                .frame(maxWidth: .infinity, minHeight: 52)
        }.buttonStyle(.plain)
    }

    static func clock(_ s: Double) -> String {
        let t = Int(s.rounded(.down)); return t >= 3600 ? String(format: "%d:%02d:%02d", t / 3600, t / 60 % 60, t % 60) : String(format: "%02d:%02d", t / 60, t % 60)
    }
}

/// Belegchip: springt ins Transkript und hebt die Stelle hervor. 44-pt-Trefferfläche.
private struct EvidenceChip: View {
    let evidence: LiveEvidence
    let jump: (String) -> Void
    var body: some View {
        Button { jump(evidence.segmentId) } label: {
            Label(LiveMeetingScreen.clock(evidence.start), systemImage: "text.alignleft")
                .font(.caption.monospacedDigit()).padding(.horizontal, 7).padding(.vertical, 2)
                .overlay(RoundedRectangle(cornerRadius: 7).stroke(Color.secondary.opacity(0.35)))
                .frame(minWidth: 44, minHeight: 44).contentShape(Rectangle())
        }.buttonStyle(.plain).foregroundStyle(VoicePalette.secondaryText)
        .accessibilityLabel("Zur Stelle \(LiveMeetingScreen.clock(evidence.start)) im Transkript")
    }
}

private struct LiveStateView: View {
    @ObservedObject var model: LiveMeetingModel
    let jump: (String) -> Void

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 12) {
                if !model.aiAllowed {
                    Text("KI-Auswertung ist für diese Aufnahme nicht freigegeben. Transkript und Notizen laufen weiter.").font(.subheadline).voiceCard()
                }
                if model.live.summary.isEmpty && model.live.items.isEmpty && model.aiAllowed {
                    Text("Der Stand erscheint nach den ersten Minuten.").font(.subheadline).foregroundStyle(VoicePalette.secondaryText).voiceCard()
                }
                if !model.live.summary.isEmpty {
                    card("Aktueller Stand") {
                        Text(model.live.summary)
                        let refs = Self.unique(model.live.items.filter { $0.kind != .suggestion }.flatMap(\.evidence)).prefix(4)
                        if !refs.isEmpty { HStack(spacing: 4) { Text("Belege").font(.caption).foregroundStyle(VoicePalette.secondaryText); ForEach(Array(refs), id: \.segmentId) { EvidenceChip(evidence: $0, jump: jump) } } }
                    }
                }
                section("Aufgaben", .todo)
                section("Entscheidungen", .decision)
                section("Offene Punkte", .open)
                section("Themen", .topic)
                let suggestions = model.live.items(.suggestion).filter { $0.status != .done }
                if !suggestions.isEmpty {
                    VStack(alignment: .leading, spacing: 8) {
                        HStack { Text("JETZT ANSPRECHEN?").font(.caption.weight(.bold)); Text("Vorschlag der KI").font(.caption2.weight(.bold)).padding(.horizontal, 5).overlay(RoundedRectangle(cornerRadius: 6).stroke()) }
                            .foregroundStyle(.purple)
                        ForEach(suggestions) { item in
                            Text(item.text)
                            if let ev = item.evidence.first { HStack { Text("Anlass").font(.caption).foregroundStyle(VoicePalette.secondaryText); EvidenceChip(evidence: ev, jump: jump) } }
                        }
                    }.padding(14).frame(maxWidth: .infinity, alignment: .leading)
                    .background(RoundedRectangle(cornerRadius: 14).fill(Color.purple.opacity(0.1)))
                    .accessibilityElement(children: .contain).accessibilityLabel("Vorschläge der KI")
                }
                Text("Die Auswertung ist ein Entwurf. Jeder Eintrag führt zu seiner Stelle im Transkript.").font(.caption).foregroundStyle(VoicePalette.secondaryText)
            }.padding(.horizontal, 16).padding(.bottom, 28)
        }
    }

    static func unique(_ list: [LiveEvidence]) -> [LiveEvidence] {
        var seen = Set<String>(); return list.filter { seen.insert($0.segmentId).inserted }
    }

    @ViewBuilder private func section(_ title: String, _ kind: LiveItem.Kind) -> some View {
        let items = model.live.items(kind).filter { $0.status != .done }
        if !items.isEmpty {
            card("\(title) · \(items.count)") {
                ForEach(items) { item in row(item) }
            }
        }
    }

    private func row(_ item: LiveItem) -> some View {
        HStack(alignment: .top, spacing: 6) {
            if item.kind == .todo {
                Button { model.confirmDone(item.id) } label: {
                    Image(systemName: "square").font(.title3).frame(width: 44, height: 44).contentShape(Rectangle())
                }.buttonStyle(.plain).accessibilityLabel("Als erledigt abhaken")
            }
            VStack(alignment: .leading, spacing: 2) {
                Text(item.text)
                HStack(spacing: 6) {
                    if let owner = item.owner { Text(owner) }
                    if let due = item.due { Text("bis " + due) }
                    if item.status == .answerProposed { Text("Beantwortet?").fontWeight(.semibold).foregroundStyle(.orange) }
                    ForEach((item.status == .answerProposed ? item.answerEvidence : item.evidence).prefix(2), id: \.segmentId) { EvidenceChip(evidence: $0, jump: jump) }
                }.font(.caption).foregroundStyle(VoicePalette.secondaryText)
                if item.status == .answerProposed {
                    HStack {
                        Button("Ja, abschließen") { model.confirmDone(item.id) }.buttonStyle(.bordered).frame(minHeight: 44)
                        Button("Noch offen") { model.rejectAnswer(item.id) }.buttonStyle(.bordered).frame(minHeight: 44)
                    }
                }
            }
        }
    }

    private func card<C: View>(_ title: String, @ViewBuilder _ content: () -> C) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title.uppercased()).font(.caption.weight(.bold)).foregroundStyle(VoicePalette.secondaryText)
            content()
        }.voiceCard()
    }
}

private struct LiveTranscriptView: View {
    @ObservedObject var model: LiveMeetingModel
    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 10) {
                    if model.aiStatus.hasPrefix("KI pausiert") {
                        Text("**\(model.aiStatus).** Aufnahme und Transkript laufen weiter.").font(.subheadline).voiceCard()
                    }
                    ForEach(Array(model.segments.enumerated()), id: \.element.id) { i, s in
                        if hasBreak(before: i) {
                            Label("Unterbrechung – hier fehlt Gesprächszeit", systemImage: "pause.circle")
                                .font(.caption).foregroundStyle(VoicePalette.secondaryText)
                                .frame(maxWidth: .infinity, alignment: .center)
                        }
                        VStack(alignment: .leading, spacing: 2) {
                            Text(LiveMeetingScreen.clock(s.start)).font(.caption.monospacedDigit()).foregroundStyle(VoicePalette.secondaryText)
                            Text(s.text)
                        }
                        .padding(6)
                        .background(RoundedRectangle(cornerRadius: 6).fill(model.jumpTo == s.id ? Color.yellow.opacity(0.4) : .clear))
                        .id(s.id)
                    }
                    if hasBreak(before: model.segments.count) {
                        Label("Unterbrechung – hier fehlt Gesprächszeit", systemImage: "pause.circle")
                            .font(.caption).foregroundStyle(VoicePalette.secondaryText)
                            .frame(maxWidth: .infinity, alignment: .center)
                    }
                    if !model.volatile.isEmpty {
                        Text(model.volatile).italic().foregroundStyle(VoicePalette.secondaryText).id("volatile")
                            .accessibilityLabel("Wird noch erkannt: " + model.volatile)
                    }
                    Text("Grau = wird noch erkannt. Ausgewertet werden nur fertige Sätze.").font(.caption).foregroundStyle(VoicePalette.secondaryText)
                }.padding(.horizontal, 16).padding(.bottom, 28)
            }
            .onChange(of: model.jumpTo, initial: true) { _, id in if let id { withAnimation { proxy.scrollTo(id, anchor: .center) } } }
            .onChange(of: model.segments.count) { _, _ in if model.jumpTo == nil, let last = model.segments.last { proxy.scrollTo(last.id, anchor: .bottom) } }
        }
    }

    /// Liegt zwischen dem vorigen und diesem Segment eine Pause oder Unterbrechung?
    private func hasBreak(before i: Int) -> Bool {
        let lower = i > 0 ? model.segments[i - 1].start : -1
        let upper = i < model.segments.count ? model.segments[i].start : .infinity
        return model.breaks.contains { $0 > lower && $0 <= upper }
    }
}
