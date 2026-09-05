## Verdict

**No remaining proven P0/P1 blocker in these four files.** I re-verified the four focus areas against the code and they hold:

- **Recovery identity** — `recoverRecording` derives `sessionId == messageId` from the draft filename (`Store.swift:100-114`; `dropFirst(11)/dropLast(4)` are correct for `.recording-` / `.m4a`). A crash between `accept` and `try? fm.removeItem` re-enters the same UUID, hits the existing-entry branch in `accept` (`Store.swift:75-79`), and returns the same receipt after digest + byte comparison. `createdAt` drift from `attributes[.creationDate] ?? Date()` is irrelevant on the second pass because the existing entry is returned unchanged. Converges.
- **Original audio** — the only `removeItem` calls are on the `.partial-*` staging dir, the `.recording-*.m4a` draft (only after durable accept), and `.transfer-<uuid>.json` copies. `cleanupTransfers` is prefix + suffix + UUID + retention gated (`Store.swift:116-125`) and both callers gate on `activationState == .activated`. Nothing deletes `<uuid>/audio.m4a`.
- **Delivery convergence** — `pendingDelivery` is a total, deterministic order (state, then `createdAt`, then UUID). Watch→phone: N pending entries drain in N round trips, each self-retry consuming one `.saved` entry. Phone→watch: `replyEnvelope` persists `replyMessageId` before transport, `acceptReply` dedupes on `replyReceipt.messageId == envelope.messageId`, `acknowledgeReply` matches on `replyMessageId`. The userInfo ping-pong terminates (receipt/replyReceipt handlers return `Data()`).
- **Cancellation** — `results` is started before `analyzer.start`, the watchdog cancels both the consumer task and the analyzer, `defer { deadline.cancel() }` covers every exit including the early `AVAudioFile` throw, and the catch path re-cancels. No dangling task on any path.
- **Speech attempt bookkeeping** — the `stopSpeaking`-then-`speak` sequence is safe: `didCancel` for the superseded utterance runs after `speak` returns, removes its own `speechAttempts` key, and the `currentUtterance === utterance` guard prevents it from clobbering the new utterance's state.
- **Double-save** — `stop()` and `audioRecorderDidFinishRecording` cannot both persist: `stop()` runs to completion synchronously on the main actor (setting `recorder = nil`), and the delegate's `recorder === recorder` guard plus `saveRecording`'s `pendingURL` guard close both orderings.

## P2 findings

**1. Unbounded resend loop if a store write fails mid-delivery** — `VoiceModel.swift:233-241`

If `receive(response)` throws before advancing state (the realistic trigger is `store.update(entry.id, state: .accepted)` failing on disk error at line 346), the entry stays `.saved`, the follow-up `try? store.update(...)` also fails, and the terminating condition re-fires:

```swift
if !response.isEmpty && self.entries.contains(where: { $0.state == .saved }) { self.retry() }
```

`retry()` re-selects the same entry, resends it, gets the same receipt, fails the same way — an indefinite WC message loop with no backoff. It is not stack recursion (each hop is a fresh Task), but it hammers the phone for as long as the write error persists. Gate the recursion on the delivered entry actually having advanced:

```swift
let state = self.entries.first(where: { $0.id == entry.id })?.state ?? .saved
try? self.store?.update(entry.id, state: state, timing: ("transfer_roundtrip_ms", Date().timeIntervalSince(start) * 1000))
self.refresh()
let advanced = self.entries.first(where: { $0.id == entry.id })?.state != .saved
if !response.isEmpty && advanced && self.entries.contains(where: { $0.state == .saved }) { self.retry() }
```

`sendChunks` (line 272) is already safe here because its `try store?.update` is inside the `do` and routes a failure to `queueFile`.

**2. No deadline or cancellation on generation, unlike transcription** — `LocalProviders.swift:48-52`

`transcribe` now has a 60s watchdog and the CPU fallback has its own 90s bound, but `session.respond(to:)` has neither. Because `processPending` holds `processing = true` across that await and the `guard active` checks only run *between* awaits, a hung `respond` wedges the pipeline for the app's lifetime: every later `Task { await processPending() }` returns immediately at the `!processing` guard, and backgrounding doesn't unwind it. Nothing is lost (entries stay durable), but nothing further is processed either. Wrapping the call the same way `transcribe` wraps the analyzer would make the two paths symmetric.

Related, smaller: an entry parked in `.deferred` is only reattempted on the next scene activation or WC event — `processPending`'s own `defer` doesn't re-arm it.

**3. Dead drafts accumulate with no operator signal** — `VoiceModel.swift:163, 193`

A `.recording-*.m4a` that fails `AVAudioFile(forReading:).length > 0` is retained forever, which matches your evidence-preserving policy — I'm not suggesting deleting it. The gap is visibility: `recoverRecordings` `continue`s on the length check without setting `status` or calling `recordDiagnostic`, and `saveRecording` leaves `pendingURL` set, so the file is silently re-probed on every activation with no signal that it exists. A `recordDiagnostic("draft_unreadable")` on that branch would make it observable.

**4. `.partial-*` staging directories have no reaper** — `Store.swift:89-95`

A crash between `createDirectory` and `moveItem` orphans a staging dir. `entries()` filters it out (non-UUID name) and `cleanupTransfers` doesn't match it, so it persists indefinitely and consumes disk that the 64 MB quota — which sums only `<uuid>/audio.m4a` — never sees. Low impact at prototype scale, but it's an unbounded leak with no reclaim path.

## API assumptions worth pinning with a test (not findings)

- **`cleanupTransfers` retention depends on exact URL equality.** `standardizedFileURL` normalizes `.`/`..` but does not resolve symlinks. Retention holds only if `WCSessionFileTransfer.file.fileURL` returns the URL you passed to `transferFile` in the same path form, and if a new transfer appears in `outstandingFileTransfers` synchronously on return. Both are almost certainly true, but this is the one place where an assumption failure would delete an in-flight copy. One integration assertion (`transferFile` then immediately confirm the URL is in the retained set) would convert this to verified.
- **`SpeechTranscriber(preset: .transcription)` emits only finalized, non-overlapping results**, so `text += String(result.text.characters)` is correct. Under a progressive preset the same loop would duplicate text.
- **`analyzer.start(inputAudioFile:finishAfterFile: true)` terminates `transcriber.results`**, so `results.value` completes on the happy path; the 60s watchdog is the backstop rather than the primary mechanism.
- **`AVAudioRecorder.stop()` finalizes the container before returning**, so the immediate `AVAudioFile(forReading:)` in `saveRecording` is valid. If it weren't, the failure mode is the safe one — the draft is retained and recovered later.
