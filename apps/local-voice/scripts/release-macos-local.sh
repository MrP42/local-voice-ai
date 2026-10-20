#!/usr/bin/env bash
# macOS-Release (DMG + Updater-Artefakt) lokal auf dem MacBook bauen und an das
# GitHub-Release haengen. Ersetzt .github/workflows/release-macos.yml: macOS-Runner
# kosten das Zehnfache an Actions-Minuten, das Inklusivkontingent war damit an
# einzelnen Tagen aufgebraucht.
#
# Ablauf wie bisher: Tag app-vX.Y.Z pushen -> release-windows.yml legt das Release
# an und schreibt latest.json -> dieses Skript baut aarch64 + x86_64, laedt DMG,
# .app.tar.gz und .sig hoch und traegt die darwin-Plattformen in latest.json ein.
#
# Voraussetzungen: Xcode-CLT, rustup, pnpm, gh (angemeldet), jq, cmake.
# Signierschluessel des Updaters (derselbe wie im Repo-Secret):
#   export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/local-voice.key)"
#   export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=...   # falls gesetzt
#
# Aufruf aus apps/local-voice:  scripts/release-macos-local.sh [aarch64|x86_64|all]
set -euo pipefail

cd "$(dirname "$0")/.."
REPO="MrP42/local-voice-ai"
WHICH="${1:-all}"

: "${TAURI_SIGNING_PRIVATE_KEY:?TAURI_SIGNING_PRIVATE_KEY fehlt (ohne .sig lehnt die App jedes Update ab)}"
export CMAKE_POLICY_VERSION_MINIMUM=3.5

VERSION=$(grep -o '"version": "[^"]*"' src-tauri/tauri.conf.json | head -1 | cut -d'"' -f4)
TAG="app-v$VERSION"
echo "Version $VERSION, Release $TAG"

# Der Tag muss existieren und auf den ausgecheckten Stand zeigen.
git fetch -q --tags origin
if [ "$(git rev-parse "$TAG^{commit}" 2>/dev/null)" != "$(git rev-parse HEAD)" ]; then
  echo "HEAD ist nicht $TAG - erst den Tag auschecken (git checkout $TAG)." >&2
  exit 1
fi

# Auf das Release warten, das release-windows.yml anlegt (max. ~40 min).
for i in $(seq 1 80); do
  gh release view "$TAG" -R "$REPO" >/dev/null 2>&1 && break
  [ "$i" = 80 ] && { echo "Release $TAG existiert nicht." >&2; exit 1; }
  echo "Warte auf Release $TAG ..."; sleep 30
done

# Silero-VAD-Modell (nicht in git), gepinnter Hash wie im alten Workflow.
VAD=src-tauri/resources/models/silero_vad_v4.onnx
VAD_SHA256="a35ebf52fd3ce5f1469b2a36158dba761bc47b973ea3382b3186ca15b1f5af28"
mkdir -p "$(dirname "$VAD")"
[ -f "$VAD" ] || curl -fL -o "$VAD" https://blob.handy.computer/silero_vad_v4.onnx
echo "$VAD_SHA256  $VAD" | shasum -a 256 -c -

pnpm install --frozen-lockfile

WORK=$(mktemp -d)
CONF_BACKUP="$WORK/tauri.conf.json"
cp src-tauri/tauri.conf.json "$CONF_BACKUP"
trap 'cp "$CONF_BACKUP" src-tauri/tauri.conf.json; rm -rf "$WORK"' EXIT

PUB_DATE=$(date -u +%Y-%m-%dT%H:%M:%SZ)
# latest.json schreibt release-windows.yml erst am Ende; fehlt sie noch, mit leerem Geruest starten.
empty_latest() { jq -n --arg v "$VERSION" --arg d "$PUB_DATE" '{version: $v, notes: "", pub_date: $d, platforms: {}}'; }
gh release download "$TAG" -R "$REPO" -p latest.json -D "$WORK" --clobber 2>/dev/null || empty_latest > "$WORK/latest.json"

build_one() {
  local arch="$1" triple="$1-apple-darwin"
  rustup target add "$triple" >/dev/null
  cp "$CONF_BACKUP" src-tauri/tauri.conf.json
  unset ORT_LIB_LOCATION ORT_PREFER_DYNAMIC_LINK

  if [ "$arch" = x86_64 ]; then
    # ONNX Runtime fuer Intel (Microsoft liefert kein osx-x86_64 mehr), gepinnt.
    local ORT_VERSION="1.24.2"
    local ORT_SHA256="590cee05699047b85e4a8115b4cec792073c2972ce78de383356a552a296a9c4"
    curl -fL -o "$WORK/ort.tgz" "https://blob.handy.computer/onnxruntime-osx-x86_64-${ORT_VERSION}.tgz"
    echo "${ORT_SHA256}  $WORK/ort.tgz" | shasum -a 256 -c -
    tar xzf "$WORK/ort.tgz" -C "$WORK"
    local ORT_DIR="$WORK/onnxruntime-osx-x86_64-${ORT_VERSION}"
    export ORT_LIB_LOCATION="$ORT_DIR/lib" ORT_PREFER_DYNAMIC_LINK=1
    jq --arg lib "$ORT_DIR/lib/libonnxruntime.${ORT_VERSION}.dylib" \
      '.bundle.macOS.frameworks = [$lib]' "$CONF_BACKUP" > src-tauri/tauri.conf.json
  fi

  pnpm tauri build --target "$triple" --bundles app,dmg,updater

  local B="src-tauri/target/$triple/release/bundle"
  local dmg tgz
  dmg=$(ls "$B"/dmg/*.dmg | head -1)
  tgz=$(ls "$B"/macos/*.app.tar.gz | head -1)
  local base="Local.Voice.AI_${VERSION}_${arch}"
  cp "$dmg" "$WORK/$base.dmg"
  cp "$tgz" "$WORK/$base.app.tar.gz"
  cp "$tgz.sig" "$WORK/$base.app.tar.gz.sig"
  gh release upload "$TAG" -R "$REPO" --clobber \
    "$WORK/$base.dmg" "$WORK/$base.app.tar.gz" "$WORK/$base.app.tar.gz.sig"

  local url="https://github.com/$REPO/releases/download/$TAG/$base.app.tar.gz"
  local sig; sig=$(cat "$tgz.sig")
  local key="darwin-$arch"
  jq --arg k "$key" --arg u "$url" --arg s "$sig" --arg d "$PUB_DATE" \
    '.platforms[$k] = {signature: $s, url: $u} | .platforms[$k + "-app"] = {signature: $s, url: $u} | .pub_date = (.pub_date // $d)' \
    "$WORK/latest.json" > "$WORK/latest.new" && mv "$WORK/latest.new" "$WORK/latest.json"
}

case "$WHICH" in
  aarch64|x86_64) build_one "$WHICH" ;;
  all) build_one aarch64; build_one x86_64 ;;
  *) echo "Argument: aarch64 | x86_64 | all" >&2; exit 2 ;;
esac

# Kurz vor dem Hochladen neu holen und nur die darwin-Eintraege uebernehmen,
# damit ein spaeter fertig gewordener Windows-Eintrag nicht verloren geht.
gh release download "$TAG" -R "$REPO" -p latest.json -O "$WORK/remote.json" --clobber 2>/dev/null || empty_latest > "$WORK/remote.json"
mkdir -p "$WORK/upload"
jq -s '.[0] * {platforms: (.[0].platforms + (.[1].platforms | with_entries(select(.key | startswith("darwin")))))}' \
  "$WORK/remote.json" "$WORK/latest.json" > "$WORK/upload/latest.json"
jq -e '.platforms | keys | length > 0' "$WORK/upload/latest.json" >/dev/null
gh release upload "$TAG" -R "$REPO" --clobber "$WORK/upload/latest.json"
echo "Fertig: $(jq -r '.platforms | keys | join(", ")' "$WORK/upload/latest.json")"
