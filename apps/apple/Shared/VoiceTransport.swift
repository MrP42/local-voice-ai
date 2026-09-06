import Foundation
import WatchConnectivity

typealias Wire = VoiceEnvelope

/// Owns the WatchConnectivity lifecycle and durable delivery; it never starts inference or audio.
@MainActor
final class VoiceTransport: NSObject, WCSessionDelegate {
    var onChange: (() -> Void)?
    var onCapture: (() -> Void)?
    var onReply: ((String, UUID) -> Void)?
    var onStatus: ((String) -> Void)?
    var onReachability: ((Bool) -> Void)?
    private var store: DurableStore?
    private var chunks: ChunkInbox?
    private var entries: [Entry] = []
    private var reachable = false
    private var status = "" { didSet { onStatus?(status) } }
    private var inFlight = Set<UUID>()
    private var inFlightReplies = Set<UUID>()
    private var debugReplaySent = false
    private let encoder = JSONEncoder()
    init(store: DurableStore) throws {
        self.store = store
        chunks = try ChunkInbox(root: store.root.appendingPathComponent(".incoming-parts"), limit: 6 * 1024 * 1024,
                                additionalBytes: { try store.temporaryBytes(excludingIncomingParts: true) })
        super.init()
        refresh()
        if WCSession.isSupported() { WCSession.default.delegate = self; WCSession.default.activate() }
    }
    private func refresh() {
        do { entries = try store?.entries() ?? []; onChange?() }
        catch { status = "Verlaufprüfung erforderlich – Originale bleiben erhalten" }
    }
    private func recordDiagnostic(_ code: String) {
        #if DEBUG
        let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        try? Data(code.utf8).write(to: documents.appendingPathComponent("last-event.txt"), options: .atomic)
        #endif
    }
    func retry() {
        refresh()
        guard WCSession.isSupported() else { return }
        reachable = WCSession.default.isReachable
        onReachability?(reachable)
        if WCSession.default.activationState == .activated {
            try? store?.cleanupTransfers(keeping: Set(WCSession.default.outstandingFileTransfers.map { $0.file.fileURL }))
        }
        #if os(watchOS)
        #if DEBUG
        if ProcessInfo.processInfo.arguments.contains("--replay-capture"), reachable, !debugReplaySent,
           let entry = entries.last, let packet = try? store?.packet(for: entry),
           let data = try? encoder.encode(Wire(capture: packet)) {
            debugReplaySent = true
            WCSession.default.sendMessageData(data, replyHandler: { response in
                Task { @MainActor in
                    self.receive(response)
                    self.recordDiagnostic(response.isEmpty ? "duplicate_failed" : "duplicate_replayed")
                }
            }, errorHandler: { _ in Task { @MainActor in self.recordDiagnostic("duplicate_transport_failed") } })
        }
        #endif
        // A Watch message can wake the companion in the background. A stale
        // reachability snapshot must not prevent that attempt; errors use the
        // already queued durable transfer.
        let canAttemptInteractive = WCSession.default.activationState == .activated
        for entry in Entry.pendingDelivery(in: entries) {
            guard !inFlight.contains(entry.id), let store else { continue }
            if canAttemptInteractive && !inFlight.isEmpty { break }
            do {
                let packet = try store.packet(for: entry)
                let data = try encoder.encode(Wire(capture: packet))
                queueFile(entry, data: data)
                if canAttemptInteractive && inFlight.isEmpty && data.count > 60000 {
                    inFlight.insert(entry.id)
                    sendChunks(try CaptureChunk.split(packet), position: 0, entry: entry, started: Date())
                } else if canAttemptInteractive && inFlight.isEmpty {
                    inFlight.insert(entry.id)
                    let start = Date()
                    WCSession.default.sendMessageData(data, replyHandler: { response in
                        Task { @MainActor in
                            self.inFlight.remove(entry.id)
                            self.receive(response)
                            let state = self.entries.first(where: { $0.id == entry.id })?.state ?? .saved
                            try? self.store?.update(entry.id, state: state, timing: ("transfer_roundtrip_ms", Date().timeIntervalSince(start) * 1000))
                            self.refresh()
                            let advanced = self.entries.first(where: { $0.id == entry.id }).map { $0.state != .saved } ?? false
                            if !response.isEmpty && advanced && self.entries.contains(where: { $0.state == .saved }) { self.retry() }
                        }
                    }, errorHandler: { _ in
                        Task { @MainActor in self.inFlight.remove(entry.id); self.queuePendingFiles() }
                    })
                } else { queueFile(entry, data: data) }
            } catch { status = "Gespeicherte Aufnahme konnte nicht übertragen werden" }
        }
        #else
        for entry in entries where entry.reply != nil && entry.replyAcknowledged != true { sendAnswer(entry) }
        #endif
    }
    #if os(watchOS)
    private func queuePendingFiles() {
        guard let store else { return }
        for entry in Entry.pendingDelivery(in: entries) where !inFlight.contains(entry.id) {
            if let packet = try? store.packet(for: entry), let data = try? encoder.encode(Wire(capture: packet)) {
                queueFile(entry, data: data)
            }
        }
    }
    private func sendChunks(_ parts: [CaptureChunk], position: Int, entry: Entry, started: Date) {
        guard parts.indices.contains(position) else { inFlight.remove(entry.id); return }
        let part = parts[position]
        do {
            let data = try encoder.encode(Wire(chunk: part))
            WCSession.default.sendMessageData(data, replyHandler: { response in
                Task { @MainActor in
                    do {
                        let wire = try JSONDecoder().decode(Wire.self, from: response)
                        guard wire.schemaVersion == 1, wire.sessionId == entry.id else { throw VoiceError.invalid }
                        if wire.kind == "chunkReceipt", wire.receipt?.messageId == part.messageId, position + 1 < parts.count {
                            self.sendChunks(parts, position: position + 1, entry: entry, started: started)
                        } else if wire.kind == "receipt", wire.receipt?.messageId == entry.receipt.messageId {
                            self.inFlight.remove(entry.id)
                            self.receive(response)
                            let state = self.entries.first(where: { $0.id == entry.id })?.state ?? .saved
                            try self.store?.update(entry.id, state: state, timing: ("transfer_roundtrip_ms", Date().timeIntervalSince(started) * 1000))
                            self.refresh()
                            if self.entries.contains(where: { $0.state == .saved }) { self.retry() }
                        } else { throw VoiceError.invalid }
                    } catch {
                        self.inFlight.remove(entry.id)
                        self.queuePendingFiles()
                        self.status = "gespeichert – Verarbeitung folgt"
                    }
                }
            }, errorHandler: { error in
                Task { @MainActor in
                    self.inFlight.remove(entry.id)
                    self.recordDiagnostic("chunk_transport_error_\((error as NSError).code)")
                    self.queuePendingFiles()
                }
            })
        } catch { inFlight.remove(entry.id); status = "Gespeichert – Übertragung erneut versuchen" }
    }
    #endif
    private func queueFile(_ entry: Entry, data: Data) {
        guard WCSession.default.activationState == .activated, let store else { return }
        guard !WCSession.default.outstandingFileTransfers.contains(where: { $0.file.metadata?["sessionId"] as? String == entry.id.uuidString }) else { return }
        do {
            let url = try store.prepareTransfer(for: entry.id, data: data)
            WCSession.default.transferFile(url, metadata: ["sessionId": entry.id.uuidString])
            status = "gespeichert – Verarbeitung folgt"
        } catch { status = "Gespeichert – Übertragung erneut versuchen" }
    }
    @discardableResult
    private func receive(_ data: Data) -> Data {
        do {
            let wire = try JSONDecoder().decode(Wire.self, from: data)
            guard wire.schemaVersion == 1 else { throw VoiceError.version }
            guard ["capture", "receipt", "reply", "replyReceipt", "captureChunk", "chunkReceipt"].contains(wire.kind) else { throw VoiceError.invalid }
            guard let store else { throw VoiceError.persistence }
            #if os(iOS)
            if wire.kind == "captureChunk", let part = wire.payload.chunk, let chunks {
                guard wire.sessionId == part.sessionId, wire.messageId == part.messageId else { throw VoiceError.invalid }
                if let existing = entries.first(where: { $0.id == part.sessionId }) {
                    guard existing.receipt.messageId == part.captureMessageId, existing.digest == part.digest, existing.conversationId == part.conversationId else { throw VoiceError.conflict }
                    return receive(try encoder.encode(Wire(capture: store.packet(for: existing))))
                }
                if let packet = try chunks.receive(part) {
                    let acknowledgement = receive(try encoder.encode(Wire(capture: packet)))
                    if !acknowledgement.isEmpty { try? chunks.removeCompleted(packet.sessionId) }
                    return acknowledgement
                }
                var acknowledgement = Wire(receipt: Receipt(sessionId: part.sessionId, messageId: part.messageId, receiptId: part.messageId))
                acknowledgement.kind = "chunkReceipt"
                return try encoder.encode(acknowledgement)
            }
            if wire.kind == "replyReceipt", let receipt = wire.receipt {
                guard wire.sessionId == receipt.sessionId else { throw VoiceError.invalid }
                try store.acknowledgeReply(receipt)
                refresh()
                return Data()
            }
            if let packet = wire.capture {
                guard wire.kind == "capture", wire.sessionId == packet.sessionId, wire.messageId == packet.messageId else { throw VoiceError.invalid }
                let receipt = try store.accept(packet, replyToWatch: true)
                refresh()
                if let existing = entries.first(where: { $0.id == packet.sessionId }), existing.reply != nil { sendAnswer(existing) }
                onCapture?()
                return try encoder.encode(Wire(receipt: receipt))
            }
            #else
            if let receipt = wire.receipt {
                guard wire.kind == "receipt", wire.sessionId == receipt.sessionId else { throw VoiceError.invalid }
                guard let entry = entries.first(where: { $0.id == receipt.sessionId }), entry.receipt.messageId == receipt.messageId else { throw VoiceError.invalid }
                if entry.state != .answered {
                    try store.update(entry.id, state: .accepted)
                    status = "Auf iPhone gesichert – Verarbeitung folgt"
                } else { status = "Antwort gespeichert" }
            }
            if let id = wire.sessionId, let reply = wire.reply {
                let acceptance = try store.acceptReply(wire)
                refresh()
                status = "Antwort gespeichert"
                if acceptance.isNew { onReply?(reply, id) }
                var response = Wire(receipt: acceptance.receipt)
                response.kind = "replyReceipt"
                return try encoder.encode(response)
            }
            #endif
            refresh()
            return Data()
        } catch { status = "Nachricht nicht bestätigt – erneute Zustellung erforderlich"; return Data() }
    }
    nonisolated func session(_ session: WCSession, activationDidCompleteWith activationState: WCSessionActivationState, error: Error?) { Task { @MainActor in self.retry() } }
    nonisolated func sessionReachabilityDidChange(_ session: WCSession) { Task { @MainActor in self.retry() } }
    nonisolated func session(_ session: WCSession, didReceiveMessageData messageData: Data, replyHandler: @escaping (Data) -> Void) {
        Task { @MainActor in replyHandler(self.receive(messageData)) }
    }
    nonisolated func session(_ session: WCSession, didReceiveMessageData messageData: Data) {
        Task { @MainActor in
            let acknowledgement = self.receive(messageData)
            if !acknowledgement.isEmpty { WCSession.default.transferUserInfo(["wire": acknowledgement]) }
        }
    }
    nonisolated func session(_ session: WCSession, didFinish fileTransfer: WCSessionFileTransfer, error: Error?) {
        Task { @MainActor in
            // Transport completion only permits deleting the copy, never the confirmed original.
            if WCSession.default.activationState == .activated {
                try? self.store?.cleanupTransfers(keeping: Set(WCSession.default.outstandingFileTransfers.map { $0.file.fileURL }))
            }
        }
    }
    nonisolated func session(_ session: WCSession, didReceive file: WCSessionFile) {
        // WC removes its temporary file as soon as this delegate returns.
        guard let data = try? Data(contentsOf: file.fileURL) else { return }
        Task { @MainActor in
            let receipt = self.receive(data)
            if !receipt.isEmpty { WCSession.default.transferUserInfo(["wire": receipt]) }
        }
    }
    nonisolated func session(_ session: WCSession, didReceiveUserInfo userInfo: [String: Any]) {
        guard let data = userInfo["wire"] as? Data else { return }
        Task { @MainActor in
            let acknowledgement = self.receive(data)
            if !acknowledgement.isEmpty { WCSession.default.transferUserInfo(["wire": acknowledgement]) }
        }
    }
    #if os(iOS)
    nonisolated func sessionDidBecomeInactive(_ session: WCSession) {}
    nonisolated func sessionDidDeactivate(_ session: WCSession) { session.activate() }
    func sendAnswer(_ entry: Entry) {
        guard entry.replyToWatch == true, entry.reply != nil, entry.replyAcknowledged != true,
              !inFlightReplies.contains(entry.id), let store else { return }
        do {
            let data = try encoder.encode(store.replyEnvelope(for: entry.id))
            // Queue a durable system-owned delivery before an interactive attempt. The
            // phone can be suspended immediately after its processing lease finishes.
            queueAnswer(entry.id, data: data)
            if WCSession.default.isReachable {
                inFlightReplies.insert(entry.id)
                WCSession.default.sendMessageData(data, replyHandler: { response in
                    Task { @MainActor in
                        self.inFlightReplies.remove(entry.id)
                        self.receive(response)
                    }
                }, errorHandler: { _ in
                    Task { @MainActor in
                        self.inFlightReplies.remove(entry.id)
                        self.queueAnswer(entry.id, data: data)
                    }
                })
            } else { queueAnswer(entry.id, data: data) }
        } catch { status = "Antwort gespeichert – Zustellung erneut versuchen" }
    }
    private func queueAnswer(_ id: UUID, data: Data) {
        guard WCSession.default.activationState == .activated,
              !WCSession.default.outstandingUserInfoTransfers.contains(where: { $0.userInfo["id"] as? String == id.uuidString }) else { return }
        WCSession.default.transferUserInfo(["id": id.uuidString, "wire": data])
    }
    #endif
}
