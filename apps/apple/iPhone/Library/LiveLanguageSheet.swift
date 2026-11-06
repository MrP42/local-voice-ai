import SwiftUI
import Translation

// Sprach-Chip und Sprachauswahl der Live-Übersetzung (lva#358, Spez §3, §5, §6).

/// Knopf in der Bedienleiste: zeigt „EN → DE“ bzw. die Buchsprache und öffnet die Auswahl.
struct LiveLanguageButton: View {
    @ObservedObject var playback = LibraryPlayback.shared
    @State private var shown = false

    var body: some View {
        let source = playback.liveSource.map(LiveLanguages.short) ?? "–"
        let target = playback.source == .voice ? playback.liveTarget : nil
        let caption = target.map { "\(source) → \(LiveLanguages.short($0))" } ?? source
        ControlButton(title: "Sprache", systemImage: "globe", caption: caption) { shown = true }
            .accessibilityValue(playback.liveTarget.map { "Live übersetzt in \(LiveLanguages.name($0))" } ?? "Original")
            .accessibilityHint("Liest das Buch live in einer anderen Sprache vor, die Stelle im Buch bleibt")
            .sheet(isPresented: $shown) { LiveLanguageSheet() }
    }
}

/// Kennzeichnung (Spez §6) und Unterlauf-Hinweis mit „Original weiterlesen“ (Spez §4).
struct LiveStatusLine: View {
    @ObservedObject var playback = LibraryPlayback.shared

    var body: some View {
        if let target = playback.liveTarget {
            switch playback.liveState {
            case .stalled(let reason):
                HStack(spacing: 8) {
                    Image(systemName: "exclamationmark.triangle").foregroundStyle(.orange)
                    Text(reason).font(.caption).lineLimit(2).minimumScaleFactor(0.8)
                    Spacer(minLength: 0)
                    Button("Original weiterlesen") { playback.continueOriginal() }
                        .font(.caption.weight(.semibold)).frame(minHeight: 44)
                }
                .accessibilityElement(children: .combine)
            default:
                HStack(spacing: 6) {
                    Image(systemName: "translate")
                    Text(playback.liveState == .waiting ? "Übersetzung lädt …"
                         : "Live übersetzt in \(LiveLanguages.name(target)) · auf dem iPhone")
                        .lineLimit(1).minimumScaleFactor(0.7)
                    Spacer(minLength: 0)
                }
                .font(.caption).foregroundStyle(VoicePalette.secondaryText)
                .accessibilityElement(children: .combine)
                .accessibilityShowsLargeContentViewer()
            }
        }
    }
}

/// Auswahl: Original oder eine Live-Sprache. Pakete laden nur auf ausdrücklichen Tipp (Systemzustimmung).
struct LiveLanguageSheet: View {
    @ObservedObject var playback = LibraryPlayback.shared
    @Environment(\.dismiss) private var dismiss
    @State private var states: [String: LiveLanguageState] = [:]
    @State private var loading: String?
    @State private var download: TranslationSession.Configuration?
    @State private var message: String?

    private var languages: [(code: String, name: String)] {
        let source = playback.liveSource.map(LiveTranslation.base)
        return LiveLanguages.offered.filter { $0.code != source }
    }

    var body: some View {
        NavigationStack {
            List {
                Section {
                    Button { playback.setLive(nil); dismiss() } label: {
                        row(title: "Original" + (playback.liveSource.map { " (\(LiveLanguages.name($0)))" } ?? ""),
                            detail: nil, selected: playback.liveTarget == nil)
                    }
                }
                sourceSection
                Section {
                    ForEach(languages, id: \.code) { lang in
                        let state = states[lang.code] ?? .unknown
                        Button { choose(lang.code, state) } label: {
                            row(title: lang.name, detail: detail(lang.code, state), selected: playback.liveTarget == lang.code)
                        }
                        .disabled(state == .unsupported || loading != nil || !playback.liveSourceConfirmed)
                    }
                } header: {
                    Text("Live übersetzen")
                } footer: {
                    Text("Der Buchinhalt wird auf dem iPhone übersetzt, nur die gerade gelesene Stelle mit kurzem Vorlauf. Es fallen keine Kosten an. Apple kann Nutzungs- und Leistungsdaten ohne Textinhalt erheben. Maschinelle Übersetzung, Fehler sind möglich.")
                }
                if let message {
                    Section { Text(message).font(.footnote).foregroundStyle(.orange) }
                }
            }
            .navigationTitle("Sprache")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Fertig") { dismiss() } } }
            .task(id: playback.liveSource) { await refresh() }
            .translationTask(download) { session in
                // Nur für fehlende Pakete: Systemblatt mit Zustimmung, dann laden (Spez §3).
                guard let code = loading else { return }
                do {
                    try await session.prepareTranslation()
                    await refresh()
                    if states[code] == .installed { apply(code) }
                    else { message = "Die Sprache ist noch nicht geladen. Bitte erneut versuchen." }
                } catch is CancellationError {
                    message = "Laden abgelehnt. Die Sprache bleibt ungeladen."
                } catch {
                    message = "Laden fehlgeschlagen. Zum Laden ist eine Internetverbindung nötig."
                }
                loading = nil
                download = nil
            }
        }
    }

    /// Buchsprache: erkannte Sprache erst bestätigen, dann ist Live wählbar (Spez §3). Angabe am Buch gilt ohne Frage.
    @ViewBuilder private var sourceSection: some View {
        if !playback.liveSourceDeclared, let source = playback.liveSource {
            Section {
                HStack {
                    Text(playback.liveSourceConfirmed ? "Buchsprache: \(LiveLanguages.name(source))"
                                                      : "Buchsprache erkannt: \(LiveLanguages.name(source)). Stimmt das?")
                        .foregroundStyle(VoicePalette.text)
                    Spacer(minLength: 8)
                    Menu("Ändern") {
                        ForEach(LiveLanguages.offered, id: \.code) { lang in
                            Button(lang.name) { message = nil; playback.confirmLiveSource(lang.code) }
                        }
                    }
                    .frame(minHeight: 44)
                    .accessibilityHint("Andere Sprache als Buchsprache festlegen")
                }
                if !playback.liveSourceConfirmed {
                    Button("Ja, stimmt") { playback.confirmLiveSource(source) }
                        .frame(minHeight: 44)
                }
            } footer: {
                if !playback.liveSourceConfirmed {
                    Text("Aus dem Text erkannt. Bitte bestätigen, bevor live übersetzt wird.")
                }
            }
        }
    }

    private func row(title: String, detail: String?, selected: Bool) -> some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(title).foregroundStyle(VoicePalette.text)
                if let detail { Text(detail).font(.caption).foregroundStyle(VoicePalette.secondaryText) }
            }
            Spacer()
            if selected { Image(systemName: "checkmark").foregroundStyle(VoicePalette.text).accessibilityHidden(true) }
        }
        .frame(minHeight: 44).contentShape(Rectangle())
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    private func detail(_ code: String, _ state: LiveLanguageState) -> String? {
        if loading == code { return "Wird geladen …" }
        switch state {
        case .installed: return nil
        case .downloadable: return "Antippen zum Laden"
        case .unsupported: return "Mit dieser Buchsprache nicht verfügbar"
        case .unknown: return "Wird geprüft …"
        }
    }

    private func choose(_ code: String, _ state: LiveLanguageState) {
        message = nil
        switch state {
        case .installed: apply(code)
        case .downloadable:
            guard let source = playback.liveSource else { return }
            loading = code
            download = TranslationSession.Configuration(source: LiveTranslationSupport.language(source),
                                                        target: LiveTranslationSupport.language(code))
        case .unsupported, .unknown: break
        }
    }

    /// Aus dem Hörbuch: an derselben Stelle zur Vorlesestimme wechseln, dann live (Spez §5).
    private func apply(_ code: String) {
        if playback.source == .audiobook { playback.switchSource(to: .voice) }
        playback.setLive(code)
        dismiss()
    }

    private func refresh() async {
        for lang in languages { states[lang.code] = await playback.liveLanguageState(lang.code) }
    }
}
