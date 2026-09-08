# Voice protocol v1 — P1 fixtures

The Swift `Packet` type lives in apps/apple/VoiceCore. Capture/duplicate fixtures
have identical IDs and content and must return the same stored receipt. The
unsupported-version fixture must not be acknowledged. Audio bytes AQID are a
**validation fixture**, not a playable recording.

Capture fields: schemaVersion=1, UUID sessionId/messageId, kind=capture,
createdAt (seconds since 2001-01-01 UTC), payload.audio (base64). Maximum audio
size 1 MiB. Exact UUID/content reuse is accepted; ID reuse with changed content
is a conflict. A receipt identifies sessionId/messageId/receiptId and is created
once, after durable storage. Watch and iPhone stores have independent receipt IDs.

The native WC envelope also carries schemaVersion/messageId/kind/createdAt/
sessionId/payload; payload contains capture, receipt, or transcript/reply.
Replies only update an existing session and never create an unknown journal row.
Audio remains retained throughout this feasibility spike. Protocol migration and
cross-platform codecs are P2 work; no FFI or desktop migration is introduced.
