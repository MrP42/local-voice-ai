import SwiftUI
import Translation

// Live-Übersetzung im Blatt „Sprache“ (lva#358, Spez §3, §5, §6). Kein eigenes Menü: Die Live-Sprachen stehen im
// selben Blatt wie die Sprachfassungen (`LanguageSourceSheet`, Chip „DE ▾“ und Knopf „Sprache“), damit es nur
// einen Ort für die Sprache gibt.

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

/// Zustand der Live-Auswahl. Liegt beim Blatt, damit `.task` und `.translationTask` genau einmal an der `List`
/// hängen (an einer `Section` verteilten sie sich auf die Zeilen und rissen beim Scrollen ab).
@MainActor final class LiveLanguageModel: ObservableObject {
    @Published var states: [String: LiveLanguageState] = [:]
    @Published var loading: String?
    @Published var download: TranslationSession.Configuration?
    @Published var message: String?
    private let playback = LibraryPlayback.shared

    /// Live geht nur mit Text des offenen Buchs (Quellsprache bekannt, Absätze geladen).
    static var offered: Bool { LibraryPlayback.shared.liveSource != nil }

    func languages(excluding editions: Set<String>) -> [(code: String, name: String)] {
        let source = playback.liveSource.map(LiveTranslation.base)
        return LiveLanguages.offered.filter { $0.code != source && !editions.contains($0.code) }
    }

    func refresh() async {
        for lang in languages(excluding: []) { states[lang.code] = await playback.liveLanguageState(lang.code) }
    }

    /// `true`, wenn gleich umgestellt wurde (Blatt schließen); sonst läuft das Laden des Pakets.
    func choose(_ code: String) -> Bool {
        message = nil
        switch states[code] ?? .unknown {
        case .installed: apply(code); return true
        case .downloadable:
            guard let source = playback.liveSource else { return false }
            loading = code
            download = TranslationSession.Configuration(source: LiveTranslationSupport.language(source),
                                                        target: LiveTranslationSupport.language(code))
            return false
        case .unsupported, .unknown: return false
        }
    }

    /// Nur für fehlende Pakete: Systemblatt mit Zustimmung, dann laden (Spez §3). `true`, wenn danach live.
    func finishDownload(_ session: TranslationSession) async -> Bool {
        guard let code = loading else { return false }
        defer { loading = nil; download = nil }
        do {
            try await session.prepareTranslation()
            await refresh()
            if states[code] == .installed { apply(code); return true }
            message = "Die Sprache ist noch nicht geladen. Bitte erneut versuchen."
        } catch is CancellationError {
            message = "Laden abgelehnt. Die Sprache bleibt ungeladen."
        } catch {
            message = "Laden fehlgeschlagen. Zum Laden ist eine Internetverbindung nötig."
        }
        return false
    }

    /// Aus dem Hörbuch: an derselben Stelle zur Vorlesestimme wechseln, dann live (Spez §5).
    private func apply(_ code: String) {
        if playback.source == .audiobook { playback.switchSource(to: .voice) }
        playback.setLive(code)
    }
}

/// Abschnitte „Buchsprache“ (nur ohne Angabe am Buch) und „Live übersetzen“ für das Blatt „Sprache“.
/// `editionLanguages`: Sprachen, die das Buch als eigene Fassung hat; sie stehen oben und werden hier nicht
/// noch einmal als Live-Sprache angeboten. Pakete laden nur auf ausdrücklichen Tipp (Systemzustimmung).
struct LiveLanguageSections: View {
    @ObservedObject var model: LiveLanguageModel
    var editionLanguages: Set<String> = []
    @ObservedObject var playback = LibraryPlayback.shared
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        if LiveLanguageModel.offered {
            sourceSection
            Section {
                ForEach(model.languages(excluding: editionLanguages), id: \.code) { lang in
                    let state = model.states[lang.code] ?? .unknown
                    Button { if model.choose(lang.code) { dismiss() } } label: {
                        row(title: lang.name, detail: detail(lang.code, state), selected: playback.liveTarget == lang.code)
                    }
                    .buttonStyle(.plain)
                    .disabled(state == .unsupported || model.loading != nil || !playback.liveSourceConfirmed)
                    .accessibilityHint("Liest das Buch live in dieser Sprache vor, die Stelle im Buch bleibt")
                }
            } header: {
                Text("Live übersetzen")
            } footer: {
                VStack(alignment: .leading, spacing: 6) {
                    if let message = model.message { Text(message).foregroundStyle(.orange) }
                    Text("Liest die gewählte Fassung in einer anderen Sprache vor. Übersetzt wird auf dem iPhone, nur die gerade gelesene Stelle mit kurzem Vorlauf, ohne Kosten. Apple kann Nutzungs- und Leistungsdaten ohne Textinhalt erheben. Maschinelle Übersetzung, Fehler sind möglich. Zurück zum Original: oben die aktuelle Sprache antippen.")
                }
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
                            Button(lang.name) { model.message = nil; playback.confirmLiveSource(lang.code) }
                        }
                    }
                    .frame(minHeight: 44)
                    .accessibilityHint("Andere Sprache als Buchsprache festlegen")
                }
                if !playback.liveSourceConfirmed {
                    Button("Ja, stimmt") { playback.confirmLiveSource(source) }
                        .frame(minHeight: 44)
                }
            } header: {
                Text("Buchsprache")
            } footer: {
                if !playback.liveSourceConfirmed {
                    Text("Aus dem Text erkannt. Bitte bestätigen, bevor live übersetzt wird.")
                }
            }
        }
    }

    private func row(title: String, detail: String?, selected: Bool) -> some View {
        HStack(spacing: 12) {
            Image(systemName: "checkmark").font(.body.weight(.semibold)).frame(width: 22)
                .opacity(selected ? 1 : 0).accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.body)
                Text(detail ?? "Live übersetzt").font(.footnote).foregroundStyle(VoicePalette.secondaryText)
            }
            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
    }

    private func detail(_ code: String, _ state: LiveLanguageState) -> String? {
        if model.loading == code { return "Live · Sprachpaket wird geladen …" }
        switch state {
        case .installed: return nil
        case .downloadable: return "Live · Antippen lädt das Sprachpaket"
        case .unsupported: return "Live · auf diesem iPhone nicht verfügbar"
        case .unknown: return "Live · wird geprüft …"
        }
    }
}
