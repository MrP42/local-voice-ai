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
public struct VoiceTurnDetector: Sendable {
    public enum End: Sendable { case finishedSpeaking, noSpeech }
    private var firstSpeech: Double?
    private var lastSpeech: Double?
    public init() {}
    public mutating func observe(powerDB: Float, elapsed: Double) -> End? {
        guard elapsed.isFinite, powerDB.isFinite else { return nil }
        if powerDB > -38 {
            if firstSpeech == nil { firstSpeech = elapsed }
            lastSpeech = elapsed
        }
        if let firstSpeech, let lastSpeech, lastSpeech - firstSpeech >= 0.3, elapsed - lastSpeech >= 1.2 { return .finishedSpeaking }
        if elapsed >= 8, firstSpeech == nil || (lastSpeech ?? 0) - (firstSpeech ?? 0) < 0.3 { return .noSpeech }
        return nil
    }
}
