# Apple P0/P1 Preflight — 2026-09-05

## Verified baseline

- Source checkout: /Users/patrick/claude/local-voice-ai
- Source branch: main; HEAD ccdee8f75c2a21aa8cffdc58c0618542d908cb5c.
- origin: https://github.com/MrP42/local-voice-ai.git; no push performed.
- Source has modified TtsSettings.tsx and 25 translation files, untracked src/lib/tts/ and .DS_Store files. All retained in source checkout; not included in Apple worktree.
- Isolated worktree: /Users/patrick/Documents/Codex/local-voice-ai-apple-p0-p1, branch codex/apple-p0-p1.
- MacBookPro16,1, Intel Core i9-9980HK at 2.40 GHz, 8 cores, 32 GB RAM. No serial number collected in report.
- macOS 15.7.9 (24G830); Xcode 26.3 (17C529).
- xcodebuild -showsdks succeeded after user resolved license setup: iOS/watchOS device and simulator SDKs 26.2.
- devicectl component installation succeeded; device query returned "No devices found."
- security find-identity -v -p codesigning returned 0 valid identities. This does not establish whether an Apple account or team exists.

## Required reading

Read apps/local-voice/AGENTS.md, apps/local-voice/CLAUDE.md, docs/STATUS.md and docs/ROADMAP.md completely. Root AGENTS.md is absent. The requested plan was absent from this checkout; copied the fully read user-supplied Downloads document into its requested repository path. It is a planning reference; the user's current P0/P1 scope takes precedence over later milestones and release suggestions.

## Existing desktop commands

From apps/local-voice: bun run tauri dev/build, bun run build, bun run lint, bun run format:check, bun run test:playwright. Backend: cargo test --lib from src-tauri. Intel BUILD.md requires ONNX Runtime via ORT_LIB_LOCATION and ORT_PREFER_DYNAMIC_LINK=1 for desktop builds. These commands were inspected, not executed in this preflight. No desktop sources changed.

## P0 gate still open

Connect and trust the reference iPhone, make the paired Watch available to Xcode, enable Developer Mode where required, and provide/select a signing team. Installed iOS/watchOS versions, pairing, microphone/audio routes, and physical device deployment remain unverified.

P0 and P1 are not complete. No prototype build, lifecycle pass, latency measurement, or battery measurement is claimed. All ten requested lifecycle scenarios remain NOT RUN pending device setup and implementation. P2 planning must follow actual P1 evidence.
