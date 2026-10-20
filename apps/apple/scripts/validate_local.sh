#!/usr/bin/env bash
# Apple-Validierung lokal auf dem MacBook (ersetzt .github/workflows/apple.yml,
# macOS-Runner kosten das Zehnfache an Actions-Minuten). Gleiche Schritte wie
# der fruehere CI-Job; vor jedem Merge mit Aenderungen unter apps/apple/ laufen lassen.
#
# Aufruf aus der Repo-Wurzel: apps/apple/scripts/validate_local.sh
set -euo pipefail
cd "$(dirname "$0")/../../.."

xcodebuild -version
swift test --package-path apps/apple/VoiceCore
python3 apps/apple/scripts/setup_native_engines.py
python3 apps/apple/scripts/generate_project.py
git diff --exit-code -- apps/apple/LocalVoice.xcodeproj

OUT="${TMPDIR:-/tmp}/localvoice-apple-validate"
xcodebuild -project apps/apple/LocalVoice.xcodeproj -scheme VoicePhone \
  -configuration Release -destination 'generic/platform=iOS Simulator' \
  -derivedDataPath "$OUT/simulator" CODE_SIGNING_ALLOWED=NO build
xcodebuild -project apps/apple/LocalVoice.xcodeproj -scheme VoicePhone \
  -configuration Release -destination 'generic/platform=iOS' \
  -archivePath "$OUT/LocalVoice.xcarchive" CODE_SIGNING_ALLOWED=NO archive
echo "Apple-Validierung gruen (Archiv: $OUT/LocalVoice.xcarchive)"
