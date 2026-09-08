import Foundation

public struct ConversationTurn: Sendable, Equatable {
    public var user: String
    public var assistant: String
}
public enum ConversationContext {
    /// A bounded, conversation-scoped context; all original turns remain in history.
    public static func previousTurns(for entry: Entry, in entries: [Entry]) -> [ConversationTurn] {
        guard let group = entry.conversationId else { return [] }
        let candidates = entries.filter { $0.id != entry.id && $0.conversationId == group && $0.createdAt < entry.createdAt && $0.transcript != nil && $0.reply != nil }
            .sorted { $0.createdAt < $1.createdAt }.suffix(6).reversed()
        var remaining = 3400
        var turns: [ConversationTurn] = []
        for item in candidates {
            let turn = ConversationTurn(user: String((item.transcript ?? "").prefix(1200)), assistant: String((item.reply ?? "").prefix(500)))
            let count = turn.user.count + turn.assistant.count
            guard count <= remaining else { break }
            turns.append(turn); remaining -= count
        }
        return turns.reversed()
    }
}
public enum MicrophoneSensitivity: String, CaseIterable, Sendable {
    case low, balanced, high
    public var label: String { switch self { case .low: "Unempfindlich"; case .balanced: "Ausgewogen"; case .high: "Empfindlich" } }
    var threshold: Float { switch self { case .low: -30; case .balanced: -38; case .high: -46 } }
    var margin: Float { switch self { case .low: 12; case .balanced: 9; case .high: 6 } }
}
public enum ConversationOutcome {
    public static let noSpeechMessage = "Keine Sprache erkannt · Du kannst weitersprechen"
    public static func isNoSpeech(_ entry: Entry) -> Bool {
        entry.processingEvents?.contains { $0.operation == "Keine Sprache" && !$0.isAI } == true
    }
}
public struct VoiceTurnDetector: Sendable {
    public enum End: Sendable { case finishedSpeaking, noSpeech }
    private var firstSpeech: Double?
    private var lastSpeech: Double?
    private let sensitivity: MicrophoneSensitivity
    private let automaticNoiseFloor: Bool
    private var noiseFloor: Float = -60
    public init(sensitivity: MicrophoneSensitivity = .balanced, automaticNoiseFloor: Bool = true) {
        self.sensitivity = sensitivity; self.automaticNoiseFloor = automaticNoiseFloor
    }
    public mutating func observe(powerDB: Float, elapsed: Double) -> End? {
        guard elapsed.isFinite, powerDB.isFinite else { return nil }
        if automaticNoiseFloor && elapsed < 0.6 {
            noiseFloor = max(noiseFloor, powerDB)
            return nil
        }
        let threshold = automaticNoiseFloor ? max(sensitivity.threshold, noiseFloor + sensitivity.margin) : sensitivity.threshold
        if powerDB > threshold {
            if firstSpeech == nil { firstSpeech = elapsed }
            lastSpeech = elapsed
        }
        else if automaticNoiseFloor {
            // Follow changing ambient level slowly; never adapt upward during detected speech.
            noiseFloor += (powerDB - noiseFloor) * 0.03
        }
        if let firstSpeech, let lastSpeech, lastSpeech - firstSpeech >= 0.3, elapsed - lastSpeech >= 1.2 { return .finishedSpeaking }
        if elapsed >= 8, firstSpeech == nil || (lastSpeech ?? 0) - (firstSpeech ?? 0) < 0.3 { return .noSpeech }
        return nil
    }
}
