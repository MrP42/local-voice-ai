# Review: durable capture / receipts / replay / lifecycle

Four files reviewed as given. Findings are ordered by severity. Items I could not prove from these files are separated at the end.

---

## P0 — capture loss / permanent non-convergence

### 1. Failed `accept` orphans the recording forever; the status message promises a recovery that does not exist
`apps/apple/Shared/VoiceModel.swift:148-167`

`saveRecording()` clears `pendingURL` at line 150 *before* the accept succeeds. On any throw from `store.accept` (`.full` at Store.swift:73, `.invalid`, `.persistence`, transient I/O) the catch at line 163 sets `"Nicht bestätigt – Audiodatei bleibt zur Wiederherstellung erhalten"` — but nothing in these files ever enumerates `.recording-*.m4a` in `store.root`. The only reference to that file was `pendingURL`, now nil. The file is dead weight on disk and the audio is unrecoverable.

Failure scenario: watch store hits the 64 MB quota (documented as intentional at Store.swift:41) → every subsequent recording is silently and permanently lost while the UI claims it is retained.

Minimal fix: keep `pendingURL` set on failure, and add a launch/retry sweep:
```swift
// in init after refresh(), and at the top of retry()
for url in (try? fm.contentsOfDirectory(at: root, includingPropertiesForKeys: nil)) ?? []
where url.lastPathComponent.hasPrefix(".recording-") {
    if let data = try? Data(contentsOf: url), !data.isEmpty,
       (try? store.accept(Packet.capture(audio: data))) != nil {
        try? fm.removeItem(at: url)
    }
}
```
(Only delete after a receipt is returned.)

### 2. `audioRecorderDidFinishRecording(successfully: false)` strands the file and then leaks it
`apps/apple/Shared/VoiceModel.swift:168-175`

On `flag == false` the code sets a status and returns without touching `pendingURL`. The next `start()` overwrites `self.pendingURL` at line 131 with a new URL, so the previous partial file is orphaned with no reference and no sweep (same gap as #1). AVAudioRecorder finalizes the container on `stop()`/interruption, so this audio is frequently salvageable.

Minimal fix: attempt the save anyway and let `store.accept` validate:
```swift
if flag { self.saveRecording() }
else { self.status = "Aufnahme unterbrochen – Wiederherstellung wird versucht"; self.saveRecording() }
```
`accept` already rejects empty/oversize audio, so a truly unusable file degrades to case #1 (which the sweep then covers).

### 3. Replies are persisted without the validation the receiver enforces → infinite, non-converging resend loop
`apps/apple/Shared/VoiceModel.swift:439-447`, `apps/apple/iPhone/LocalProviders.swift:41-45`, checked against `Store.swift:113-114`

`Store.acceptReply` rejects `text.isEmpty`, `text.count > 500`, or `transcript.count > 16000`. Nothing on the iPhone enforces these before persisting:

- `Store.update(reply:)` (Store.swift:86-99) has no length/emptiness check.
- `LocalProviders.reply` only truncates the **FoundationModels** path (`.prefix(500)`, line 44). The `CPULocalProviders` fallback at line 42 is unbounded, and either path may return an empty string.
- `transcribe` returns an unbounded transcript, which is stored in full at line 443 and shipped in the envelope; only the *model input* is capped at 2000 (line 44).

Failure scenario: fallback generates a 700-char reply (or transcription of a 30 s clip exceeds 16000 chars). iPhone persists it. `Store.update:90` makes `entry.reply` immutable thereafter. Watch `acceptReply` throws `.invalid` → `receive` catch (VoiceModel.swift:330) returns `Data()` → iPhone's `replyHandler` gets empty data, `replyAcknowledged` stays nil → `retry()` line 222 resends on every activation and reachability change, forever. The reply can never be delivered and the entry can never be repaired.

Minimal fix — validate at the point of persistence, in `processPending` before line 440/447:
```swift
let reply = String(try await LocalProviders.reply(to: transcript).prefix(500))
guard !reply.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw VoiceError.invalid }
```
and cap the transcript: `try store.update(entry.id, transcript: String(transcript.prefix(16000)), ...)`. Ideally mirror the same guards inside `Store.update` so the invariant is enforced by the store rather than by each caller.

### 4. watchOS never activates the audio session
`apps/apple/Shared/VoiceModel.swift:124-126` and `337-339`

Both `setActive(true)` calls are wrapped in `#if os(iOS)`. On watchOS the category is set but the session is never activated, for recording *or* for TTS playback. On watchOS an active session is required before `AVAudioRecorder.record()` will capture and before `AVSpeechSynthesizer` will route to the speaker; `record(forDuration:)` returning `false` throws `.invalid` at line 130 and surfaces only as `"Aufnahme konnte nicht starten"`.

Minimal fix: drop the `#if os(iOS)` on both, or on watchOS use the async form:
```swift
#if os(watchOS)
try await AVAudioSession.sharedInstance().activate(options: [])
#else
try audio.setActive(true)
#endif
```
I flag this as P0 because it gates the primary watch capture path, but see "missing context" — if the watch target intentionally relies on a different activation path elsewhere, this collapses to a non-issue.

---

## P1 — stalls, leaks, availability

### 5. No timeout on transcription wedges all deferred processing permanently
`apps/apple/iPhone/LocalProviders.swift:21-40`, consumed at `VoiceModel.swift:442`

`try await results.value` (line 36) only returns when `transcriber.results` terminates. If `SpeechAnalyzer` stops delivering without finishing the stream, this never returns. `processPending` holds `processing = true` (VoiceModel.swift:432, released only by `defer`), so **every** subsequent `processPending()` call returns immediately at the `guard !processing` on line 431. All queued captures stop being processed for the rest of the process lifetime, with no status change.

Minimal fix: race the analyzer against a deadline and cancel on expiry:
```swift
let text = try await withThrowingTaskGroup(of: String.self) { group in
    group.addTask { try await results.value }
    group.addTask { try await Task.sleep(for: .seconds(60)); throw VoiceError.persistence }
    defer { group.cancelAll() }
    return try await group.next()!
}
```
keeping the existing `results.cancel(); await analyzer.cancelAndFinishNow()` cleanup on the error path.

### 6. `.transfer-*.json` files are written and never deleted, and are invisible to the quota
`apps/apple/Shared/VoiceModel.swift:262-271`

`queueFile` writes `root/.transfer-<sessionId>.json` and hands it to `transferFile`. There is no `session(_:didFinish:error:)` delegate method in this file and no startup sweep. `entries()` skips non-UUID names (Store.swift:52) and `used` only sums entry audio (Store.swift:72), so these files — roughly 1.37× the audio size, since the audio is base64 in JSON — accumulate outside the 64 MB budget on the watch, which is the most disk-constrained device here.

Minimal fix: implement the delegate and sweep on launch:
```swift
nonisolated func session(_ session: WCSession, didFinish fileTransfer: WCSessionFileTransfer, error: Error?) {
    if error == nil { try? FileManager.default.removeItem(at: fileTransfer.file.fileURL) }
}
```
plus a `.transfer-*` sweep alongside the `.recording-*` sweep from #1, skipping any path still in `outstandingFileTransfers`.

### 7. Single-slot `inFlight` combined with newest-first, unstable ordering starves older captures
`apps/apple/Shared/VoiceModel.swift:194-219`

`entries()` returns newest-first (Store.swift:54). The sort at line 194 has a two-valued comparator (`.saved` = 0, else 1) and `sorted(by:)` is not stable in Swift, so within the `.saved` group order is arbitrary and skews newest-first. `guard inFlight.isEmpty` then allows exactly one reachable send per `retry()` invocation. Two concrete consequences:

- Delivery is effectively LIFO: the oldest unsent capture can be repeatedly overtaken.
- On the error path (line 214-216) only the one in-flight entry gets `queueFile`'d; every other unsent entry is skipped by the `continue` and is not queued at all. It then only advances on the next `retry()` trigger — and the success-path re-entry at line 212 is gated on `!response.isEmpty`, so a rejected receipt provides no re-entry either.

Nothing is lost (state is durable), but delivery can stall until the next scene activation or reachability change. Minimal fix: make ordering explicit and drain non-reachable entries regardless of `inFlight`:
```swift
for entry in entries.sorted(by: { ($0.state == .saved ? 0 : 1, $0.createdAt) < ($1.state == .saved ? 0 : 1, $1.createdAt) })
where entry.state != .answered {
    guard let store else { continue }
    ...
    if reachable && inFlight.isEmpty { /* message or chunk path */ }
    else { queueFile(entry, data: data) }
}
```

### 8. One unreadable `entry.json` disables the entire store, including new captures
`apps/apple/Store.swift:50-55`, amplified at `61-73` — file path `apps/apple/VoiceCore/Sources/VoiceCore/Store.swift`

`entries()` maps with `try` over every UUID-named directory, so a single missing or truncated `entry.json` makes it throw. `accept` calls `entries()` at line 65 before doing anything, and the quota loop at line 72 calls `audio(for:)` for every entry — so one bad directory or one missing `audio.m4a` means **no new capture can ever be accepted**, on either device. `refresh()` (VoiceModel.swift:108-111) also silently keeps stale data.

The commit path (staging + `moveItem`, lines 76-83) makes a torn entry unlikely, so I am treating this as a durability hardening gap rather than a proven corruption path — but the blast radius is total loss of capture capability, which is exactly the invariant you asked to protect.

Minimal fix: skip undecodable directories instead of failing the batch, and make the quota tolerant:
```swift
.compactMap { try? JSONDecoder().decode(Entry.self, from: Data(contentsOf: $0.appendingPathComponent("entry.json"))) }
```
and at line 72 use `fm.attributesOfItem(atPath:)[.size]` with a `?? 0` fallback.

### 9. Quota check reads every stored audio file fully into memory on every accept
`Store.swift:72`

`current.reduce(0) { try $0 + audio(for: $1.id).count }` materializes up to the full 64 MB limit as `Data` for each `accept` call — and `accept` is on the hot path for every watch message and every chunk reassembly (VoiceModel.swift:284, 287, 303). On a watch this is a realistic memory-pressure termination during capture.

Minimal fix: `try fm.attributesOfItem(atPath: audioURL(for: $1.id).path)[.size] as? Int ?? 0`. Same substitution avoids the full-buffer compare at line 68 for the common case (the SHA-256 digest is already compared at line 67; the byte-for-byte re-read is redundant unless you are guarding against digest collision).

### 10. Chunked send has no fallback when the peer's response is unusable
`apps/apple/Shared/VoiceModel.swift:226-260`

The `errorHandler` (line 250) falls back to `queueFile`, but the `catch` inside the `replyHandler` (line 245-248) only removes `inFlight` and sets a status. So when the iPhone returns `Data()` — which it does for *any* throw in `receive`, including `accept` failing with `.full` (Store.swift:73) — the watch abandons the multi-chunk transfer with no file-transfer fallback and no retry scheduled. Combined with #7 the capture waits for the next activation.

Minimal fix: mirror the errorHandler's fallback in the catch:
```swift
} catch {
    self.inFlight.remove(entry.id)
    if let packet = try? self.store?.packet(for: entry), let data = try? self.encoder.encode(Wire(capture: packet)) {
        self.queueFile(entry, data: data)
    }
    self.status = "gespeichert – Verarbeitung folgt"
}
```

---

## P2 — correctness footguns

### 11. `VoiceEnvelope` mints a random `messageId` for replies; idempotency depends on remembering to overwrite it
`apps/apple/VoiceCore/Sources/VoiceCore/Envelope.swift:21-26`, `Store.swift:107-108`

The initializer sets `messageId = capture?.messageId ?? receipt?.receiptId ?? UUID()`. For the reply case that is a fresh UUID every call; `replyEnvelope` repairs it by assigning `envelope.messageId = entry.replyMessageId!` on the *next line*. Any other construction site that omits the overwrite produces a new messageId per send, which `acceptReply` treats as a conflict (`Store.swift:120`) and rejects forever — the same non-converging loop as #3. Also note `messageId` is set from `receiptId`, not `receipt.messageId`, so envelope-level dedupe is not possible at any receiver.

Minimal fix: add a `replyMessageId` parameter to the initializer so the stable identity is set at construction rather than patched afterwards, and make `replyEnvelope` the only reply construction path.

### 12. Transcription result segments are concatenated without separators
`apps/apple/iPhone/LocalProviders.swift:30`

`text += String(result.text.characters)` joins consecutive finalized segments with no delimiter. Whether this is visible depends on whether the framework includes leading whitespace in each segment's attributed text, which I cannot confirm from these files — but the concatenation is unconditional, so if it does not, the stored transcript is a run-on string. Low cost to make it explicit: accumulate into an array and `joined(separator: " ")` with a whitespace-collapsing pass, or preserve the attributed run boundaries.

### 13. `sendChunks` indexes `parts[position]` without a bounds guard
`apps/apple/Shared/VoiceModel.swift:227`

If `CaptureChunk.split` ever returns an empty array, this traps. `accept` guarantees non-empty audio so this should not occur in practice, but a `guard parts.indices.contains(position) else { inFlight.remove(entry.id); return }` is a one-line elimination of a crash class on the capture path.

---

## Missing context (not scored)

- `CaptureChunk` and `ChunkInbox` are not in scope. I could not verify: whether `split` produces envelopes under the ~65 KB `sendMessageData` limit after JSON+base64 expansion (VoiceModel.swift:201 splits based on a 60000-byte threshold measured on the *unsplit* envelope); whether `ChunkInbox.receive` persists parts durably; and whether partially-received sessions that never complete are ever garbage-collected. That last one is the same unbounded-growth shape as #6 and is worth checking.
- `CPULocalProviders` is not in scope — its output bounds are the trigger for #3, and its cancellation behavior is relevant to #5.
- `SpeechTranscriber` / `SpeechAnalyzer` / `AssetInventory` / `SystemLanguageModel` API surface: the availability checks at `LocalProviders.swift:8, 16, 22, 26, 42` and the `finishAfterFile:` termination semantics at line 35 read as correct but I verified them only for internal consistency, not against the framework.
- Whether anything outside `VoiceModel` touches `DurableStore`. Every access in this file is `@MainActor`, which satisfies the "one serial executor" contract at `Store.swift:40`; the store itself has no internal locking and is not `Sendable`, so any additional caller would break it.
- The `#if DEBUG` blocks (VoiceModel.swift:48-60, 85-98, 181-193) are fixture/probe scaffolding; I did not review them for production behavior since they cannot ship.
