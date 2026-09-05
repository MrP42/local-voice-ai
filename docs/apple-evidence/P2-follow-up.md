# P2 follow-up after simulator spike

P2 implementation has not begun. Full P1 acceptance remains open when local
STT/model execution or requested lifecycle behavior cannot be demonstrated in
the Intel simulator. The user currently requests virtual devices.

1. Decide the supported local-STT/model verification environment. Current Intel
iPhone simulator reports SpeechTranscriber unavailable and Foundation Models
modelNotReady. Do not disguise fixture transcripts or fixed responses as model
results. Verify on supported physical iPhone or supported model-capable test host
when authorized; keep capture/deferred mode usable meanwhile.
2. Add deterministic interruption/fault injection around every durable commit,
receipt and reply step, including disk-full mid-write and receipt-loss recovery.
Extend the six initial core tests with actual process-kill tests for the store.
3. Separate UI, transport and job orchestration into testable components. Persist
job attempt state, receipt/reply outboxes, bounded backoff and response delivery
acknowledgements. Add bounded model cancellation and foreground-expiry handling.
4. Add recoverable-recording UI, explicit audio retention/deletion, storage-budget
accounting for staging/transfer copies, data-protection verification, and metadata
migration. Keep all confirmed audio until the new retention protocol is proven.
5. Measure complete-turn interactive delivery against background file transfer;
use this measurement before adding streaming. Add per-attempt metrics and
monotonic end-to-end timing; do not mix simulator cold starts with device p95.
6. Add automated UI permission/start/stop/interrupt tests and accessibility checks.
Wrist-down, lock, user force-quit, radio reconnection and energy still require an
explicitly agreed hardware acceptance round. No fake workout or unlimited runtime.
7. Run the planned energy comparison on Watch and iPhone (three comparable runs,
with/without app, 8 h/30 turns). Until then battery impact is unknown.

No desktop redesign, Android, App Store publication or push to main is included.
