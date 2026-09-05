import Foundation
import Darwin
import VoiceCore

let args = CommandLine.arguments
let mode = args[1], root = URL(fileURLWithPath: args[2], isDirectory: true)
let checkpoint = args.count > 3 ? StorageCheckpoint(rawValue: args[3]) : nil
var packet = Packet.capture(audio: Data([1, 2, 3, 4]))
packet.sessionId = UUID(uuidString: "11111111-1111-1111-1111-111111111111")!
packet.messageId = UUID(uuidString: "22222222-2222-2222-2222-222222222222")!
packet.createdAt = Date(timeIntervalSince1970: 1000)
let plain = try DurableStore(root: root)
let faulty = try DurableStore(root: root, fault: { if $0 == checkpoint { _exit(77) } })
if mode == "capture" { _ = try faulty.accept(packet) }
else if mode == "reply" {
    _ = try plain.accept(packet)
    let wire = VoiceEnvelope(sessionId: packet.sessionId, transcript: "Test", reply: "Erhalten.",
                             replyMessageId: UUID(uuidString: "33333333-3333-3333-3333-333333333333")!)
    _ = try faulty.acceptReply(wire)
} else if mode == "ack" {
    _ = try plain.accept(packet)
    try plain.update(packet.sessionId, reply: "Erhalten.", state: .answered)
    let peer = try DurableStore(root: root.appendingPathComponent("peer"))
    _ = try peer.accept(packet)
    let receipt = try peer.acceptReply(plain.replyEnvelope(for: packet.sessionId)).receipt
    try faulty.acknowledgeReply(receipt)
} else if mode == "job" {
    _ = try plain.accept(packet)
    _ = try faulty.beginJob(packet.sessionId)
    try faulty.update(packet.sessionId, transcript: "Ein dauerhaftes Transkript.", state: .deferred)
    try faulty.update(packet.sessionId, reply: "Erhalten.", state: .answered)
} else if mode == "inspect" { try plain.recoverStaging(); try plain.recoverInterruptedJobs() }
else { fatalError("Unknown probe mode") }
// The fault-injecting instance writes independently, like another process.
plain.invalidateInventory()
let entries = try plain.entries()
let result: [String: Any] = [
    "entries": try JSONSerialization.jsonObject(with: JSONEncoder().encode(entries)),
    "audioMatches": try (entries.isEmpty || plain.audio(for: packet.sessionId) == packet.audio),
    "issues": try plain.inventory().issues.map(\.id)
]
print(String(data: try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys]), encoding: .utf8)!)
