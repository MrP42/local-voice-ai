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

    /// Führt einen Takt aus. Gibt das Delta und den gesendeten Prompt zurück (nur dessen
    /// Segmente dürfen als Beleg gelten).
    ///
    /// Das Budget wird nicht geschätzt, sondern vor dem Aufruf gemessen: mit der
    /// Tokenzählung des Systemmodells (iOS 26.4+), sonst mit der UTF-8-Bytezahl als
    /// sicherer Obergrenze (jedes Token deckt mindestens ein Byte). Gezählt werden
    /// Anweisungen, Eingabe und das JSON-kodierte Antwortschema; dazu die feste
    /// Ausgabegrenze und 10 % Sicherheitsabstand. Passt es nicht, wird das Budget
    /// verkleinert und neu gebaut; nach vier Versuchen fällt der Takt aus.
    static func tick(state: LiveState, pending: [LiveSegment], older: [LiveSegment], budget: LiveTokenBudget = LiveTokenBudget()) async throws -> (LiveDelta, LivePrompt) {
        let model = SystemLanguageModel.default
        // Das Schema geht als JSON-Schema in den Prompt (`includeSchemaInPrompt`). Gezählt wird
        // seine JSON-Kodierung; ist sie nicht verfügbar, fällt der Takt aus (nie schätzen).
        guard let schemaData = try? JSONEncoder().encode(Delta.generationSchema),
              let schemaText = String(data: schemaData, encoding: .utf8), !schemaText.isEmpty else { throw CancellationError() }
        let context = contextSize(model)
        var b = budget
        for _ in 0..<4 {
            try Task.checkCancellation()
            let prompt = LivePromptBuilder.build(state: state, pending: pending, older: older, budget: b)
            guard !prompt.segments.isEmpty else { throw CancellationError() }
            let measured = try await tokens(prompt.instructions, model) + tokens(prompt.input, model) + tokens(schemaText, model)
            let total = Int(Double(measured + b.output) * 1.1)
            if total <= context {
                let session = LanguageModelSession(instructions: prompt.instructions)
                let response = try await session.respond(to: prompt.input, generating: Delta.self, includeSchemaInPrompt: true,
                                                         options: GenerationOptions(temperature: 0.2, maximumResponseTokens: b.output))
                return (convert(response.content), prompt)
            }
            // Überhang anteilig aus Stand, neuen Segmenten und früheren Belegen nehmen.
            let factor = max(0.2, 1 - Double(total - context) / Double(max(1, b.state + b.newSegments + b.olderEvidence)) - 0.05)
            b.state = Int(Double(b.state) * factor)
            b.newSegments = Int(Double(b.newSegments) * factor)
            b.olderEvidence = Int(Double(b.olderEvidence) * factor)
        }
        throw CancellationError()
    }

    private static func contextSize(_ model: SystemLanguageModel) -> Int {
        if #available(iOS 26.4, *) { return model.contextSize }
        return 4096
    }

    private static func tokens(_ text: String, _ model: SystemLanguageModel) async throws -> Int {
        if #available(iOS 26.4, *) { return try await model.tokenCount(for: Prompt(text)) }
        return text.utf8.count
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
