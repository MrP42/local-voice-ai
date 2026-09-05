# Local Voice — native Apple feasibility prototype

Scope: P0/P1 only. SwiftUI iPhone + Watch; desktop Tauri code is unchanged.
The user changed acceptance to **virtual Xcode devices** on 2026-09-05.
Simulator evidence is not physical-device, battery, radio or locked-device evidence.

## Reproduce

Open `LocalVoice.xcodeproj`. Select VoicePhone or VoiceWatch and the paired
simulators. No developer team is needed for simulator builds.

```sh
# From repository root
python3 apps/apple/scripts/setup_native_engines.py --models
swift test --package-path apps/apple/VoiceCore
python3 apps/apple/scripts/generate_project.py
xcodebuild -project apps/apple/LocalVoice.xcodeproj -scheme VoicePhone \
  -destination 'generic/platform=iOS Simulator' -configuration Debug \
  -derivedDataPath apps/apple/DerivedData CODE_SIGNING_ALLOWED=NO build
```

The VoicePhone build includes its companion Watch target. The project generator
uses only Python's standard library and stable IDs. No downloaded build plugin.

Installed test pair: iPhone 15 Pro Max, iOS 26.3.1 universal runtime;
Apple Watch Ultra 3 (49 mm), watchOS 26.2 universal runtime. Xcode 26.3,
iOS/watchOS 26.2 SDKs, Intel MacBookPro16,1, macOS 15.7.9.

## Use

Tap Sprechen, then Aufnahme sichern. Recording ends at 30 seconds; changing to
an inactive scene also stops and attempts to save the current recording. This
is a bounded start/stop PTT prototype. No continuous listener or background ML
loop. Start stops current speech output. Permission refusal shows a clear error.

The initial iPhone mode returns a **fixed test response without transcription**.
Disable the fixed-response toggle for the real local pipeline. SpeechTranscriber
checks German locale and installed assets; the explicit download button can
install supported speech assets. Foundation Models must be available. There is
no server-STT or cloud-model fallback. Missing models leave the capture queued.
These APIs compiled, but both model paths are unavailable on the tested Intel
simulator. A fixed test response is not an AI-generated answer.

Watch speaks the short returned text using AVSpeechSynthesizer; the iPhone does
not run a desktop TTS stack. Playback can be stopped and retried from history.

## Storage and delivery contract

Capture has schemaVersion, sessionId, messageId, kind, createdAt and payload.audio
(base64 in JSON). Dates use Foundation's JSON reference epoch (2001-01-01 UTC,
seconds). Audio turns are capped at 1 MiB. The local store caps accepted audio at
64 MiB and rejects new captures when full. No eviction of unconfirmed captures.

A staging directory contains audio plus metadata; files are synchronized before
an atomic directory rename and parent-directory fsync. Only then can a receipt
be returned. Repeated session/message/content produces the same persisted receipt;
conflicting IDs or corrupt stored audio are rejected. The UI confirms only after
persistence. Original Watch audio remains retained even after iPhone acceptance
for this spike. There is no automatic deletion in P1. Raw recordings awaiting a
successful save are left hidden in the outbox directory for recovery; they are
never labelled confirmed. A recovery UI and retention policy belong to P2.

Both receivers use the same store. Replies update an existing session; a late
receipt cannot regress answered state. Unknown reply sessions are rejected.
Reachable delivery uses WCSession.sendMessageData for a bounded complete turn;
transferFile is the background fallback. iPhone receipts and replies can use
transferUserInfo. Temporary incoming WC files are read before the delegate
returns. File transport delivery alone never authorizes deleting source audio.

Retries occur on activation, reachability change, explicit retry, and draining
new captures after a receipt. No heartbeat polling. iPhone processing is begun
only in an active scene; a foreground job can be suspended by the OS. Persisted
captures resume on next activation. Response delivery remains best effort with
persisted replay. Force-quit and radio wake-up behavior require real hardware.

## Test hooks (Debug only)

Place a synthetic `fixture.m4a` in the simulator app's Documents directory.
Launch Watch with `--fixture-capture` and optionally `--fixture-count 100` to
exercise the same store and WC transport, bypassing microphone capture.
`--local-models` starts iPhone without fixed responses. `--record-probe` invokes
the normal permission/recorder path. `--interrupt-playback` starts and stops a
saved response. These are diagnostic launch arguments, not production UI.

`inspect_simulators.py --phone UUID --watch UUID` reads app containers and emits
IDs, states and timing summaries without transcript content. iPhone Debug also
writes capabilities.json; permission/playback probes write last-event.txt.
Core XCTest and simulator fixture results must be reported separately.

## Native CPU fallback (added after initial simulator probe)

When Apple SpeechTranscriber/assets or Foundation Models are unavailable, the
iPhone can use Whisper Base multilingual and Qwen2.5-0.5B Q4_K_M locally. There
is no host HTTP service. Two separate Objective-C++ translation units isolate
the libraries; Swift calls a small C interface on a serial actor off the UI
thread. CPU inference uses four threads, bounded input, a 90-second compute
deadline, and at most 64 generated tokens. Models are loaded per job and freed.
No idle inference loop. Original Apple APIs remain the preferred providers.

Run setup_native_engines.py --models, then copy Vendor/Models into the iPhone
app's Documents/Models directory for the spike. The model download is explicit
and does not upload any audio or transcript. Model files are not bundled into
the Watch. See THIRD_PARTY_NOTICES.md for sources and licenses. The initial
report's unavailable Apple-model result still holds; separate CPU inference
measurements are required before claiming the new path works.
