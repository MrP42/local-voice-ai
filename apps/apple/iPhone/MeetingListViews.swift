import SwiftUI

/// Abschnitt der Liste (Heute, Gestern, …); der Titel ist innerhalb einer Liste eindeutig.
struct MeetingGroup: Identifiable {
    let title: String
    let items: [MeetingDocument]
    var id: String { title }
}

/// Zeile der Besprechungsliste (lva#837): Uhrzeit, Titel, Eckdaten, Status.
struct MeetingRow: View {
    let doc: MeetingDocument
    let status: MeetingListing.Status
    /// Statuszeile der laufenden Verarbeitung (nur für den Eintrag, der gerade verarbeitet wird).
    let detail: String?
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        // Ab den Bedienungshilfen-Größen steht die Zeit über dem Titel, damit Titel und Status die volle Breite haben.
        if typeSize.isAccessibilitySize {
            VStack(alignment: .leading, spacing: 6) {
                Text(MeetingListing.date(doc), format: .dateTime.day().month(.abbreviated).hour().minute())
                    .font(.subheadline.weight(.semibold).monospacedDigit()).foregroundStyle(VoicePalette.secondaryText)
                Text(MeetingListing.title(doc)).font(.body.weight(.semibold))
                if !facts.isEmpty { Text(facts).font(.subheadline).foregroundStyle(VoicePalette.secondaryText) }
                MeetingStatusLine(status: status, detail: detail)
            }
            .padding(.vertical, 2)
        } else {
            compact
        }
    }

    private var compact: some View {
        HStack(alignment: .top, spacing: 12) {
            VStack(spacing: 2) {
                Text(MeetingListing.date(doc), format: .dateTime.hour().minute())
                    .font(.subheadline.weight(.semibold).monospacedDigit()).lineLimit(1).fixedSize()
                Text(MeetingListing.date(doc), format: .dateTime.day().month(.abbreviated))
                    .font(.caption2).foregroundStyle(VoicePalette.secondaryText).lineLimit(1).fixedSize()
            }
            .frame(minWidth: 48)
            .padding(.vertical, 6).padding(.horizontal, 4)
            .background(VoicePalette.border, in: RoundedRectangle(cornerRadius: 8))
            VStack(alignment: .leading, spacing: 4) {
                Text(MeetingListing.title(doc)).font(.body.weight(.semibold)).lineLimit(2)
                if !facts.isEmpty {
                    Text(facts).font(.subheadline).foregroundStyle(VoicePalette.secondaryText).lineLimit(1)
                }
                MeetingStatusLine(status: status, detail: detail)
            }
        }
        .padding(.vertical, 2)
    }

    private var facts: String {
        var parts = [MeetingListing.duration(doc.duration)]
        if let count = MeetingListing.participantCount(doc) { parts.append(count == 1 ? "1 Person" : "\(count) Personen") }
        let languages = MeetingListing.languages(doc)
        if !languages.isEmpty { parts.append(languages.prefix(3).joined(separator: " ")) }
        return parts.filter { $0 != "–" }.joined(separator: " · ")
    }
}

/// Status als kurze Marke; laufende Verarbeitung mit Fortschritt.
struct MeetingStatusLine: View {
    let status: MeetingListing.Status
    let detail: String?

    var body: some View {
        switch status {
        case .consentMissing:
            tag("Einwilligung fehlt", "exclamationmark.shield", VoicePalette.danger)
        case .failed:
            tag("Angehalten · erneut versuchen", "exclamationmark.triangle", VoicePalette.danger)
        case .waiting:
            tag("Wartet auf Verarbeitung", "clock", VoicePalette.secondaryText)
        case .running:
            VStack(alignment: .leading, spacing: 4) {
                if let value = detail.flatMap(MeetingListing.progress(fromStatus:)) {
                    ProgressView(value: value).tint(VoicePalette.accent)
                } else {
                    ProgressView().progressViewStyle(.linear).tint(VoicePalette.accent)
                }
                Text(detail ?? "Wird verarbeitet").font(.caption).foregroundStyle(VoicePalette.secondaryText).lineLimit(1)
            }
        case let .ready(tasks, openQuestions):
            // Nebeneinander nur, wenn alles einzeilig passt; sonst untereinander, die Marke bricht nie mitten im Wort.
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 8) { readyParts(tasks, openQuestions, single: true) }
                VStack(alignment: .leading, spacing: 2) { readyParts(tasks, openQuestions, single: false) }
            }
        }
    }

    @ViewBuilder private func readyParts(_ tasks: Int, _ openQuestions: Int, single: Bool) -> some View {
        tag("Protokoll fertig", "checkmark.circle.fill", VoicePalette.success).fixedSize(horizontal: single, vertical: false)
        if tasks > 0 { counter(tasks == 1 ? "1 Aufgabe" : "\(tasks) Aufgaben").fixedSize(horizontal: single, vertical: false) }
        if openQuestions > 0 { counter(openQuestions == 1 ? "1 offene Frage" : "\(openQuestions) offene Fragen").fixedSize(horizontal: single, vertical: false) }
    }

    private func tag(_ text: String, _ symbol: String, _ color: Color) -> some View {
        Label(text, systemImage: symbol).font(.caption.weight(.medium)).foregroundStyle(color).labelStyle(.titleAndIcon)
    }
    private func counter(_ text: String) -> some View {
        Text(text).font(.caption).foregroundStyle(VoicePalette.secondaryText)
    }
}

/// Auswahl über der Liste: Alle, Mit Protokoll, In Arbeit, Importiert.
struct MeetingFilterBar: View {
    @Binding var selection: MeetingListing.Filter

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(MeetingListing.Filter.allCases, id: \.self) { filter in
                    let active = filter == selection
                    Button { selection = filter } label: {
                        Text(filter.rawValue).font(.subheadline.weight(active ? .semibold : .regular))
                            .padding(.horizontal, 14).frame(minHeight: 36)
                            .foregroundStyle(active ? VoicePalette.ink : VoicePalette.text)
                            .background(active ? VoicePalette.brand : VoicePalette.border, in: Capsule())
                    }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(active ? .isSelected : [])
                    .accessibilityIdentifier("meetingFilter-\(filter.rawValue)")
                }
            }
            .padding(.horizontal, 16).padding(.vertical, 4)
        }
    }
}

/// Kopf der Detailansicht: Titel, Datum und Eckdaten.
struct MeetingHeader: View {
    let doc: MeetingDocument

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(MeetingListing.title(doc)).font(.title3.weight(.semibold)).lineLimit(3)
                .accessibilityAddTraits(.isHeader).accessibilityIdentifier("meetingTitle")
            Text(MeetingListing.date(doc), format: .dateTime.weekday(.wide).day().month(.wide).year().hour().minute())
                .font(.subheadline).foregroundStyle(VoicePalette.secondaryText)
            HStack(alignment: .top, spacing: 16) {
                fact("Dauer", MeetingListing.duration(doc.duration))
                if let count = MeetingListing.participantCount(doc) { fact("Teilnehmende", "\(count)") }
                let languages = MeetingListing.languages(doc)
                if !languages.isEmpty { fact("Sprachen", languages.joined(separator: " ")) }
                fact("Quelle", MeetingListing.isRecording(doc) ? "Aufnahme" : "Import")
            }
            if let names = doc.participantNames?.filter({ !$0.trimmingCharacters(in: .whitespaces).isEmpty }), !names.isEmpty {
                Text(names.joined(separator: ", ")).font(.caption).foregroundStyle(VoicePalette.secondaryText).lineLimit(3)
            }
        }
    }

    private func fact(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label).font(.caption).foregroundStyle(VoicePalette.secondaryText)
            Text(value).font(.subheadline.weight(.semibold).monospacedDigit())
        }
        .accessibilityElement(children: .combine)
    }
}
