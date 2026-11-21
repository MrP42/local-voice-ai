import SwiftUI

// Verknüpfte Inhalte (lva#557, Paket G3): das Anhänge-Blatt. Es bietet je nutzbarem Bereich (freigegeben UND vom Assistenten gewollt) die
// Inhalte des Index an, mit Suche über die Titel und Mehrfachauswahl. Dasselbe Blatt dient dem Gespräch („Inhalt anhängen“, mit dem
// Schalter „Nur aus diesen Inhalten antworten“) und dem Editor des Assistenten („Inhalt hinzufügen“). Es ändert nichts selbst: „Fertig“
// gibt die Auswahl zurück. Angeboten wird nur, was im Index steht; ein Verweis öffnet nie einen nicht freigegebenen Bereich.

/// Eine Zeile des Blatts.
private struct AssistantPickerRow: Identifiable {
    let candidate: AssistantContextCandidate
    let selected: Bool
    /// Vom Assistenten verknüpft: im Gespräch nicht änderbar.
    let locked: Bool
    /// Die Höchstzahl ist erreicht: ein weiterer Inhalt lässt sich nicht mehr wählen.
    let full: Bool

    var id: String { candidate.id }
}

/// Ein Abschnitt des Blatts (ein Bereich).
private struct AssistantPickerSection: Identifiable {
    let area: AssistantArea
    let rows: [AssistantPickerRow]
    /// Es gibt in diesem Bereich überhaupt Inhalte (auch wenn die Suche keinen trifft).
    let hasContent: Bool

    var id: String { area.rawValue }
}

struct AssistantContextPickerView: View {
    /// Titel des Blatts.
    let title: String
    /// Die Bereiche, die der Assistent nutzen darf (freigegeben ∩ Assistent).
    let areas: Set<AssistantArea>
    /// Schon verknüpft und änderbar (Kontexte des Gesprächs bzw. des Assistenten im Editor).
    let linked: [AssistantProfile.Context]
    /// Schon verknüpft und nicht änderbar (Kontexte des Assistenten im Gespräch): mit Haken, ausgegraut, „vom Assistenten“.
    var locked: [AssistantProfile.Context] = []
    /// Zeigt den Schalter „Nur aus diesen Inhalten antworten“ (nur im Gespräch).
    var showsOnlySwitch = false
    var onlyFromThese = false
    /// Höchstzahl aller Kontexte (Schema: 50).
    var limit = AssistantConversationContexts.maxCount
    let done: (AssistantContextSelection) -> Void
    let cancel: () -> Void

    @ObservedObject private var talk = AssistantController.shared
    @State private var candidates: [AssistantContextCandidate]?
    @State private var chosen: Set<String> = []
    @State private var only = false
    @State private var query = ""
    @State private var prepared = false

    var body: some View {
        NavigationStack {
            List {
                if showsOnlySwitch { onlySection }
                if areas.isEmpty {
                    Section {
                        Text("Es ist kein Bereich freigegeben. Schalte im Menü „Mehr“ unter „Bereiche“ Inhalte frei.")
                            .font(.footnote)
                            .foregroundStyle(VoicePalette.secondaryText)
                            .fixedSize(horizontal: false, vertical: true)
                            .accessibilityIdentifier("assistantContextPickerNoAreas")
                    }
                } else if candidates == nil {
                    Section {
                        HStack(spacing: 10) {
                            ProgressView()
                            Text("Inhalte werden gesucht …").foregroundStyle(VoicePalette.secondaryText)
                        }
                        .frame(minHeight: 44)
                        .accessibilityElement(children: .combine)
                    }
                } else {
                    ForEach(sections) { section in sectionView(section) }
                }
                if let note = limitNote {
                    Section {
                        Text(note).font(.footnote).foregroundStyle(VoicePalette.secondaryText)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
            .searchable(text: $query, prompt: "Titel suchen")
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Abbrechen", action: cancel).accessibilityIdentifier("assistantContextPickerCancel")
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Fertig") { finish() }.accessibilityIdentifier("assistantContextPickerDone")
                }
            }
            .task {
                guard !prepared else { return }
                prepared = true
                chosen = Set(linked.map { AssistantContextChip.key($0) })
                only = onlyFromThese
                candidates = await talk.contextCandidates(areas: areas)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("assistantContextPicker")
    }

    // MARK: Schalter

    private var onlySection: some View {
        Section {
            Toggle("Nur aus diesen Inhalten antworten", isOn: $only)
                .frame(minHeight: 44)
                .accessibilityIdentifier("assistantContextOnlySwitch")
        } footer: {
            Text(only
                 ? "Antworten kommen in diesem Gespräch ausschließlich aus den angehängten Inhalten."
                 : "Ohne den Schalter haben die angehängten Inhalte Vorrang; die beste Stelle aus den übrigen Inhalten bleibt erhalten.")
        }
    }

    // MARK: Abschnitte

    private func sectionView(_ section: AssistantPickerSection) -> some View {
        Section {
            if section.rows.isEmpty {
                Text(section.hasContent ? "Kein Titel passt zur Suche." : "In diesem Bereich gibt es noch keine Inhalte.")
                    .font(.footnote)
                    .foregroundStyle(VoicePalette.secondaryText)
                    .frame(minHeight: 44, alignment: .leading)
                    .accessibilityIdentifier("assistantContextPickerEmpty")
            }
            ForEach(section.rows) { row in rowView(row) }
        } header: {
            Text(AssistantAreaSettings.title(section.area))
        }
    }

    private func rowView(_ row: AssistantPickerRow) -> some View {
        Button {
            toggle(row.candidate.key)
        } label: {
            HStack(spacing: 10) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(row.candidate.title).foregroundStyle(VoicePalette.text).fixedSize(horizontal: false, vertical: true)
                    if row.locked {
                        Text("vom Assistenten").font(.caption).foregroundStyle(VoicePalette.secondaryText)
                    } else if let detail = row.candidate.detail, !detail.isEmpty {
                        Text(detail).font(.caption).foregroundStyle(VoicePalette.secondaryText).fixedSize(horizontal: false, vertical: true)
                    }
                }
                Spacer(minLength: 8)
                Image(systemName: row.selected ? "checkmark.circle.fill" : "circle")
                    .foregroundStyle(row.selected ? VoicePalette.accent : VoicePalette.secondaryText)
            }
            .frame(minHeight: 44)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(row.locked || (row.full && !row.selected))
        .accessibilityLabel(row.candidate.title)
        .accessibilityValue(row.locked ? "ausgewählt, vom Assistenten" : (row.selected ? "ausgewählt" : "nicht ausgewählt"))
        .accessibilityAddTraits(row.selected ? .isSelected : [])
        .accessibilityIdentifier("assistantContextPickerRow")
    }

    // MARK: Daten

    private var lockedKeys: Set<String> { Set(locked.map { AssistantContextChip.key($0) }) }

    private var candidateKeys: Set<String> { Set((candidates ?? []).map(\.key)) }

    /// Wie viele Kontexte schon gelten, ohne dass das Blatt sie anbietet (verwaiste Verweise, Ordner, Arten ohne Abbildung) oder ändern kann.
    private var othersCount: Int {
        let offered = candidateKeys
        let kept = linked.filter { !offered.contains(AssistantContextChip.key($0)) }.count
        let assistant = lockedKeys.subtracting(offered).count
        return kept + assistant
    }

    private var chosenCount: Int { (candidates ?? []).filter { chosen.contains($0.key) && !lockedKeys.contains($0.key) }.count }

    private var isFull: Bool { othersCount + chosenCount + lockedKeys.intersection(candidateKeys).count >= limit }

    private var limitNote: String? {
        isFull ? "Höchstens \(limit) Inhalte lassen sich verknüpfen." : nil
    }

    private var sections: [AssistantPickerSection] {
        let all = candidates ?? []
        let needle = query.trimmingCharacters(in: .whitespacesAndNewlines)
        let locks = lockedKeys
        let full = isFull
        return AssistantArea.allCases.filter { areas.contains($0) }.map { area -> AssistantPickerSection in
            let inArea = all.filter { $0.area == area }
            let shown = needle.isEmpty ? inArea : inArea.filter {
                $0.title.range(of: needle, options: [.caseInsensitive, .diacriticInsensitive]) != nil
            }
            let rows = shown.map { candidate in
                AssistantPickerRow(candidate: candidate, selected: chosen.contains(candidate.key) || locks.contains(candidate.key),
                                   locked: locks.contains(candidate.key), full: full)
            }
            return AssistantPickerSection(area: area, rows: rows, hasContent: !inArea.isEmpty)
        }
    }

    private func toggle(_ key: String) {
        if chosen.contains(key) { chosen.remove(key) } else if !isFull { chosen.insert(key) }
    }

    private func finish() {
        let locks = lockedKeys
        let picked = (candidates ?? []).filter { chosen.contains($0.key) && !locks.contains($0.key) }
        done(AssistantContextSelection(chosen: picked, offered: candidateKeys, onlyFromThese: showsOnlySwitch ? only : onlyFromThese))
    }
}
