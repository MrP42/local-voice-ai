import Foundation
import FoundationModels

/// Ein Takt des Live-Stands mit dem Apple-Systemmodell (Epic #247, P3).
/// Je Takt eine frische Session ohne Verlauf; Zustand und Segmente stehen im
/// Prompt, gebaut im festen Tokenbudget (`LivePromptBuilder`).
enum LiveSummarizer {
    @Generable
    enum Kind {
        case topic, decision, todo, open, suggestion

        var item: LiveItem.Kind {
            switch self {
            case .topic: return .topic
            case .decision: return .decision
            case .todo: return .todo
            case .open: return .open
            case .suggestion: return .suggestion
            }
        }
    }

    @Generable
    struct NewItem {
        @Guide(description: "Art: topic=Thema, decision=Entscheidung, todo=Aufgabe, open=offene Frage, suggestion=Frage, die man jetzt ansprechen könnte")
        var kind: Kind
        @Guide(description: "Kurz, höchstens 15 Wörter, auf Deutsch")
        var text: String
        @Guide(description: "Zuständige Person, nur wenn wörtlich im Beleg genannt, sonst leer")
        var owner: String
        @Guide(description: "Termin, nur wenn wörtlich im Beleg genannt, sonst leer")
        var due: String
        @Guide(description: "Segment-IDs in eckigen Klammern aus der Eingabe, z. B. a12")
        var evidence: [String]
    }

    @Generable
    struct Change {
        @Guide(description: "ID eines bestehenden Eintrags aus STAND")
        var id: String
        @Guide(description: "Neuer Text oder leer")
        var text: String
        var owner: String
        var due: String
        var evidence: [String]
    }

    @Generable
    struct Answer {
        @Guide(description: "ID einer offenen Frage aus STAND, die jetzt beantwortet wurde")
        var id: String
        var evidence: [String]
    }

    @Generable
    struct Delta {
        @Guide(description: "Aktueller Stand in höchstens fünf Sätzen")
        var summary: String
        @Guide(.maximumCount(8))
        var add: [NewItem]
        @Guide(.maximumCount(6))
        var update: [Change]
        @Guide(.maximumCount(4))
        var answered: [Answer]
    }

    static var available: Bool { SystemLanguageModel.default.availability == .available }

    /// Führt einen Takt aus. Gibt das Delta und die gezeigten Segmente zurück
    /// (nur diese dürfen als Beleg gelten) sowie die Audioposition, bis zu der ausgewertet ist.
    static func tick(state: LiveState, pending: [LiveSegment], older: [LiveSegment], budget: LiveTokenBudget = LiveTokenBudget()) async throws -> (LiveDelta, LivePrompt) {
        let prompt = LivePromptBuilder.build(state: state, pending: pending, older: older, budget: budget)
        // Harte Grenze vor dem Modellaufruf.
        guard prompt.fits, !prompt.segments.isEmpty else { throw CancellationError() }
        let session = LanguageModelSession(instructions: prompt.instructions)
        let response = try await session.respond(to: prompt.input, generating: Delta.self,
                                                 options: GenerationOptions(temperature: 0.2, maximumResponseTokens: budget.output))
        return (convert(response.content), prompt)
    }

    static func convert(_ d: Delta) -> LiveDelta {
        func opt(_ s: String) -> String? { let t = s.trimmingCharacters(in: .whitespacesAndNewlines); return t.isEmpty ? nil : t }
        func ids(_ list: [String]) -> [String] { list.map { $0.trimmingCharacters(in: CharacterSet(charactersIn: "[] ")) }.filter { !$0.isEmpty } }
        return LiveDelta(
            summary: opt(d.summary),
            add: d.add.map { .init(kind: $0.kind.item, text: $0.text, owner: opt($0.owner), due: opt($0.due), evidence: ids($0.evidence)) },
            update: d.update.map { .init(id: $0.id, text: opt($0.text), owner: opt($0.owner), due: opt($0.due), evidence: ids($0.evidence)) },
            answered: d.answered.map { .init(id: $0.id, evidence: ids($0.evidence)) })
    }
}
