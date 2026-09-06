# Local Voice — native Apple feasibility prototype

Scope: native P0/P1 prototype with P2 durability, job and model-management improvements. SwiftUI iPhone + Watch; the desktop audio backend is preserved; the shared Mac/Windows frontend now has a matching WAI workspace.
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

The default iPhone mode uses the local transcription/response pipeline. A fixed
response remains available explicitly in the model panel for diagnostics. SpeechTranscriber
checks German locale and installed assets; the explicit download button can
install supported speech assets. Missing Apple providers fall back to the explicitly installed local CPU models. There is
no server-STT or cloud-model fallback. Missing models leave the capture queued.
These Apple APIs compiled but are unavailable on the tested Intel simulator.
The CPU fallback completed 100 real local STT/response turns. A fixed test
response is not an AI-generated answer.

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
successful save stay in the outbox directory and are never labelled confirmed.
On activation, readable nonempty drafts are recovered automatically using the stable
UUID in their filename. Unreadable drafts remain on disk with a visible status.
P2 exposes damaged metadata and unconfirmed drafts with export/recovery controls.
It retains originals and budgets temporary transfer/staging files separately.

Both receivers use the same store. Replies update an existing session; a late
receipt cannot regress answered state. Unknown reply sessions are rejected.
Reachable delivery uses WCSession.sendMessageData for a bounded complete turn;
transferFile is the background fallback. iPhone receipts and replies can use
transferUserInfo. Temporary incoming WC files are read before the delegate
returns. File transport delivery alone never authorizes deleting source audio.

Retries occur on activation, reachability change, explicit retry, and draining
new captures after a receipt. No heartbeat polling. iPhone processing is begun
only in an active scene; a foreground job can be suspended by the OS. Persisted
captures resume on next activation within three attempts. Native inference is
cooperatively cancelled when inactive; a saved transcript is reused. Explicit
per-entry retry replenishes exhausted or manually cancelled jobs. Response delivery remains best effort with
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
report's unavailable Apple-model result still holds; CPU inference
measurements and limits are recorded in docs/apple-evidence/2026-09-05-simulator-report.md.
Optional `--small` also downloads Whisper Small (487.6 MB). The model panel
selects Base or Small explicitly and imports official files after SHA-256
verification and atomic installation. Small uses more time and memory.
Qwen can give incorrect answers, including basic arithmetic in the measured
quality set. A conservative German capability guard blocks the tested external
action requests; it is not a general semantic correctness guarantee.

## Recording permission lifecycle

A recording request is invalidated when the scene becomes inactive. If the first
system permission dialog interrupts the scene, allowing access does not start a
surprise recording afterward: the UI explicitly asks for another tap. A denial
remains visible. Duplicate or delayed callbacks cannot consume a newer intent.
This behavior has six core regression tests and fresh iPhone/Watch UI coverage.
Empty, whitespace-only, or oversized generated replies are rejected before a job
is marked answered; the saved recording remains eligible for later processing.


## Local recordings and meeting results (2026-09-06)

The iPhone **Aufzeichnungen** tab imports audio/video through the native file picker.
Originals are copied, hashed and durably committed before an import is acknowledged.
A separate archive retains up to 4 GiB (individual inputs up to 2 GiB / 2 hours).
Audio is decoded locally, then Whisper processes resumable 30-second chunks. Foreground
processing pauses when the app becomes inactive and resumes from persisted progress.
Failures keep the original/transcript and expose a retry action.

Meeting analysis uses the explicitly installed **Qwen 2.5 1.5B Instruct Q4_K_M**,
not the short-answer model. Install its approved file through the model panel, or
prepare simulator weights with `python3 apps/apple/scripts/setup_native_engines.py --meetings`.
The official weights are Apache-2.0: https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF .
The download is 1,117,320,736 bytes; size and SHA-256 are pinned in ModelLibrary.swift.

The LLM selects transcript sentence references rather than freely inventing factual
fields. Names and due phrases require a complete matching source phrase. Recommendations
are explicitly proposals and include their source reason. Conservative German category
checks repair observed task/question/collective-next-step confusion. This is a prototype:
classification quality and STT mistakes still require review, and mixed audio has no
automatic speaker diarization. Speaker percentages are never fabricated.

Results support native copying, selectable text, TXT/HTML/JSON/SRT sharing and original
file export. The compact WAI result view separates analysis and transcript and supports
light/dark appearance. Timing totals reflect successfully persisted processing stages;
failed/cancelled attempt time and time spent waiting are not included.

Synthetic video fixture: compile `scripts/make_meeting_video.swift` with
`swiftc -parse-as-library`, then pass a synthetic speech AIFF and output MP4 path.
The resume test (`scripts/test_meeting_resume.py`) requires that MP4 as
Documents/MeetingFixtures/TEST-video.mp4 in an installed simulator app. It terminates
the app after a persisted chunk, restarts it and verifies original SHA-256 and unique
segment indexes. It uses a DEBUG-only import entry point, not a hidden production feature.

Desktop/mobile synchronization, remote microphones and attention routing remain a
separate integration step; neither the import tab nor its export menu implements them.

## App icon

Both Apple app targets use the existing Local Voice AI waveform and AI dot from
`apps/local-voice/scripts/make-icons.py`: signal yellow #FFDD00 on ink #111418.
A shared AppIcon asset supplies iOS default/dark and watchOS; the operating system
applies the outer shape. Regenerate the opaque 1024-pixel sRGB artwork with:

```sh
swift apps/apple/scripts/make_app_icon.swift apps/apple/Shared/Assets.xcassets/AppIcon.appiconset/LocalVoiceAI.png
python3 apps/apple/scripts/generate_project.py
```

Reference: https://developer.apple.com/documentation/xcode/configuring-your-app-icon
Fresh simulator build succeeded, both apps installed and launched, and launcher
icons visually checked on iPhone and Watch (2026-09-06).
