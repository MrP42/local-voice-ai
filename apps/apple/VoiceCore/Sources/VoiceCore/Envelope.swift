import Foundation

public struct VoiceEnvelope: Codable, Sendable {
    public struct Payload: Codable, Sendable {
        public var capture: Packet?
        public var receipt: Receipt?
        public var transcript: String?
        public var reply: String?
        public var chunk: CaptureChunk?
    }
    public var schemaVersion = 1
    public var messageId: UUID
    public var kind: String
    public var createdAt = Date()
    public var sessionId: UUID?
    public var payload: Payload
    public var capture: Packet? { payload.capture }
    public var receipt: Receipt? { payload.receipt }
    public var transcript: String? { payload.transcript }
    public var reply: String? { payload.reply }
    public init(capture: Packet? = nil, receipt: Receipt? = nil, sessionId: UUID? = nil, transcript: String? = nil, reply: String? = nil) {
        self.messageId = capture?.messageId ?? receipt?.receiptId ?? UUID()
        self.kind = capture != nil ? "capture" : (receipt != nil ? "receipt" : "reply")
        self.sessionId = capture?.sessionId ?? receipt?.sessionId ?? sessionId
        self.payload = Payload(capture: capture, receipt: receipt, transcript: transcript, reply: reply)
    }
}

extension VoiceEnvelope {
    public init(chunk: CaptureChunk) {
        self.init()
        self.kind = "captureChunk"
        self.sessionId = chunk.sessionId
        self.messageId = chunk.messageId
        self.payload.chunk = chunk
    }
}
