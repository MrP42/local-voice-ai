## Scope note

Review only; no tools used. Findings are limited to what is provable from the four files. Where a conclusion depends on a type not provided (`StorageIssue`, `StorageInventory`, `diskBytes`, `temporaryBytes`, `JobProcessor`, `CaptureController`, `VoiceTransport`, `JobRecord`), I say so explicitly.

---

## High

**1. `accept` rejects every new capture while any damaged item lacks a messageId — Store.swift:87-90**

```swift
for issue in inventory.issues where issue.sessionId != nil {
    guard issue.sessionId != packet.sessionId, let messageId = issue.messageId,
          messageId != packet.messageId else { throw VoiceError.persistence }
}
```
The `let messageId = issue.messageId` binding is part of the `guard`, so a *failed* binding takes the `else` branch. Any issue that has a `sessionId` but no readable `messageId` — the normal shape of a damaged/unreadable `entry.json`, and plausibly of a `.recording-` draft — throws `.persistence` for **every** incoming packet, even one entirely unrelated to it. One corrupt folder therefore bricks capture acceptance until it is cleared. This is exactly the state the `--corrupt-history-probe` in VoiceModel.swift:73-82 manufactures.

The inverse gap is in the same loop: `where issue.sessionId != nil` skips issues that carry only a `messageId`, so the duplicate-detection this check exists for is not performed for them.

Intended form is a plain optional comparison (`issue.messageId != packet.messageId`), which already yields `true` for `nil`.

*Confidence: the control-flow bug is proven from this file; the blast radius depends on `StorageIssue`'s field population.*

---

## Medium

**2. A reply envelope without a transcript erases a stored transcript — Store.swift:199 (guard at :186)**

`entry.transcript = envelope.transcript` is unconditional. The consistency guard at :185-187 only runs when `entry.reply != nil`. So the sequence "local transcription completes (`update(transcript:)`) → peer delivers a reply envelope with `transcript == nil`" silently nulls the persisted transcript. Assign only when `envelope.transcript != nil`, or fold the transcript into the conflict guard.

**3. `acceptReply` overwrites `replyMessageId`, invalidating in-flight acks — Store.swift:200 vs :207**

If `update(reply:)` already minted a local `replyMessageId` (:161) and that identity was handed to transport via `replyEnvelope` (:171-179), a later `acceptReply` with a different `envelope.messageId` replaces it. `acknowledgeReply` then hard-fails with `.invalid` for the ack that is actually in flight, permanently — there is no fallback to `replyReceipt.receiptId` or to a history of prior ids.

**4. Partial initialization is unrecoverable — VoiceModel.swift:48-57, 209-213**

`recoverStaging()`, `migrateIdentities()`, and `VoiceTransport.init` are all inside the `do` that also constructs `transport` and `capture`. Any throw leaves `store` set but `transport`/`capture` (and, on iOS, `jobs`, since :111 gates on `store` but the transport/capture wiring at :127-138 is unconditional and binds to `nil` objects) permanently `nil` for the process lifetime. `recoverStorage()` re-runs recovery but never reconstructs the collaborators, so a transient staging failure disables recording and sync until relaunch, with only a status string as evidence.

Related, smaller: `capture?.onChange` / `onSaved` are assigned at :127-128, **after** `recoverRecordings()` and `refresh()` run at :55-56. The initial recovery's status updates and its `onSaved → retry()` are dropped.

**5. Notification observer token discarded, never removed — VoiceModel.swift:140-150**

`addObserver(forName:object:queue:using:)` returns a token that must be retained and passed to `removeObserver` (there is no `deinit` here). The registration outlives the model; the block is `[weak self]` so it becomes a permanent no-op rather than a crash, but it is a genuine lifecycle leak and, as written, also an unused-result warning. Store the token and remove it in `deinit`.

**6. TTS *start* commits the terminal `.answered` state — VoiceModel.swift:242 with Store.swift:163 and :41-49**

`didStart` writes `state: .answered`, and `update` treats `.answered` as absorbing (`if entry.state != .answered`). If playback is then cut — the interruption handler at :145-147, `stopPlayback()` at :237, or a `didCancel` — the entry is already out of `Entry.pendingDelivery` (:42) and out of any re-speak path. Delivery should be marked on `didFinish`, not `didStart`.

**7. `update` mutates job phase as a side effect — Store.swift:164**

`if entry.reply != nil, entry.state == .answered { entry.job?.phase = .completed }` runs on *any* `update` call, including the pure timing writes from VoiceModel.swift:242/246. An entry that has a reply and was subsequently cancelled or failed can be flipped back to `.completed`, hiding it from `retryProcessing` (VoiceModel.swift:339). *Depends on `JobRecord` semantics, but the unconditional write is proven.*

**8. Timeout is indistinguishable from user cancellation — LocalProviders.swift:34-38 and :65-68**

Both deadline tasks implement the timeout by cancelling the inner task, so the caller receives `CancellationError` — the same error `jobs?.cancelCurrent()` (VoiceModel.swift:338) produces. A 60 s timeout will therefore almost certainly be recorded as `.cancelled` rather than `.failed`, and a cancelled job is not something the user is prompted to retry. Set a flag in the deadline task and rethrow a distinct timeout error. *Depends on JobProcessor's error→phase mapping.*

**9. No deadline on the CPU fallback paths — LocalProviders.swift:23, :26, :75-86**

The Apple paths are bounded at 60 s; `cpuTranscribe`/`cpuReply` are wrapped only in `withTaskCancellationHandler`. A wedged CPU inference hangs the job indefinitely with no recovery except explicit user cancellation. The asymmetry is visible entirely within this file.

**10. `canStartRecording` admits captures that `accept` will reject — Store.swift:68-70 vs :101**

Admission allows total usage up to `limit + temporaryLimit`, while commit rejects at `limit` against committed audio alone. Between those thresholds the user records audio that can never be accepted; the draft then persists as a storage issue (see #11). *Depends on `diskBytes(at: root)` being recursive over the whole root, which the `+ temporaryLimit` term implies.*

**11. Unrecoverable drafts are retained forever — Store.swift:124-138 with :83**

`recoverRecording` performs no size or emptiness pre-check, so a draft over the 1 MiB cap or a zero-byte draft fails `accept` with `.invalid` on every attempt and is never deleted, while still counting toward `diskBytes(at: root)` in `canStartRecording`. Separately, :136 uses `try?` but :137 removes the issue from `cachedInventory` unconditionally, so a failed delete leaves the cache disagreeing with disk until the next invalidation.

**12. Shared non-`Sendable` store with mutable cache — Store.swift:52-58, VoiceModel.swift:50-54, :113**

`DurableStore` is a plain `final class` with mutable `cachedInventory`, documented as requiring a single serial executor. One instance is constructed on the MainActor and handed to `VoiceTransport`, `CaptureController`, and `JobProcessor`, while `VoiceModel` also calls into it directly from the MainActor (`refresh`, `update`, `retryJob`). If any collaborator touches it off the MainActor — likely for `JobProcessor`, given long-running async inference — this is a data race on the cache and on filesystem sequencing. *Cannot be confirmed without those three types; flagging the invariant as unenforced, not the violation as proven.*

---

## Low

**13. `speechAttempts` keyed by `ObjectIdentifier` without retaining the object — VoiceModel.swift:42, :233, :241.** Keys are only removed in `finishSpeech`. Any entry that outlives its utterance without a finish/cancel callback can be collided with by a new `AVSpeechUtterance` allocated at the same address, attributing timings and an `.answered` write to the wrong session. Key by the utterance itself, or store it alongside the tuple.

**14. Stale snapshot in the TTS e2e metric — VoiceModel.swift:243-247.** `self.entries` is read before `refresh()` at :248, so the `tts_e2e_ms == nil` guard and the origin timing come from a pre-update snapshot: the metric is skipped when the entry isn't loaded yet and can be recomputed/overwritten on a second start.

**15. No idempotency guard on speaking — VoiceModel.swift:120-123.** Any `onChange` carrying an id whose entry already has a reply restarts TTS; `jobs?.start()` is invoked from `retry()` on every scene activation (:218) and from `transport?.onCapture` (:138). Gate on `speakingId != id` or a persisted spoken marker.

**16. Synthesized `Codable` ignores default values — Envelope.swift:11, Store.swift:15 and :35.** The generated decoder requires `schemaVersion` to be present, so a peer that omits it fails with a decoding error, never reaching the `guard schemaVersion == 1` version checks at Store.swift:82/181 — the version-mismatch path is effectively unreachable for that case. The same applies to `Entry.timings`: an `entry.json` written before that field existed fails to decode and surfaces as a corrupt entry rather than migrating. Use explicit `CodingKeys` with `decodeIfPresent`.

**17. `init(chunk:)` relies on total overwrite — Envelope.swift:29-36.** `self.init()` mints a discarded `messageId` and sets `kind = "reply"` (the ternary at :23 defaults to reply when both `capture` and `receipt` are nil); correctness depends on every field being reassigned afterwards. Also means a default-constructed envelope self-describes as a reply with no reply text.

---

## Explicitly not concluded

- `syncDirectory` (:228-233) and `write` (:218-227) ordering is correct: per-file `fsync` followed by a directory `fsync` after the atomic rename, and staging is synced before the commit rename (:113-117). No durability gap found there beyond unretried `EINTR`.
- `cleanupTransfers` (:141-150) and the `.recording-` prefix arithmetic (`dropFirst(11)`/`dropLast(4)`, `dropFirst(10)`/`dropLast(5)`) are correct.
- The quota scan at :98-100 correctly includes damaged folders, but whether a folder missing `audio.m4a` makes `diskBytes` throw — which would make one broken folder fail all accepts — cannot be determined without that function.
- `apps/apple/Shared/VoiceModel.swift` uses `DurableStore`, `Entry`, `Packet`, `StorageIssue`, and calls `invalidateInventory`/`inventory(_:)`/`recoverStaging`/`migrateIdentities`/`retryJob`/`recoverInterruptedJobs` without an `import VoiceCore` and with no visible access-level guarantee for those members. That is either a target-membership arrangement I can't see or a build error; I'm not calling it a defect on the evidence available.
