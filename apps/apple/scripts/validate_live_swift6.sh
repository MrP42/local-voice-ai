#!/usr/bin/env bash
# Separates Gate: Swift 5 im Generator belegt keine Swift-6-Verträglichkeit.
# Nur mit bereits eingerichteten nativen Engines auf dem Mac ausführen.
set -euo pipefail
cd "$(dirname "$0")/../../.."
swift test --package-path apps/apple/VoiceCore -Xswiftc -strict-concurrency=complete
xcodebuild -project apps/apple/LocalVoice.xcodeproj -scheme VoicePhone \
  -configuration Debug -destination 'generic/platform=iOS Simulator' \
  -derivedDataPath "${TMPDIR:-/tmp}/localvoice-live-swift6" \
  CODE_SIGNING_ALLOWED=NO SWIFT_VERSION=6.0 SWIFT_STRICT_CONCURRENCY=complete build
