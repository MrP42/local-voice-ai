#!/usr/bin/env bash
# Vorabpruefung fuer scripts/release-macos-local.sh auf dem MacBook. Baut nichts, aendert nichts.
# Aufruf aus apps/local-voice:  scripts/release-macos-check.sh
set -uo pipefail
cd "$(dirname "$0")/.."
fehler=0
ok()   { printf '  OK    %s\n' "$1"; }
nein() { printf '  FEHLT %s\n' "$1"; fehler=$((fehler + 1)); }
pruef() { command -v "$1" >/dev/null 2>&1 && ok "$1 ($(command -v "$1"))" || nein "$1 – $2"; }

echo "Werkzeuge:"
xcode-select -p >/dev/null 2>&1 && ok "Xcode-CLT" || nein "Xcode-CLT – xcode-select --install"
pruef rustup "https://rustup.rs"
pruef pnpm   "brew install pnpm (oder corepack enable)"
pruef gh     "brew install gh"
pruef jq     "brew install jq"
pruef cmake  "brew install cmake"
pruef node   "brew install node@22"

echo "Rust-Ziele:"
for t in aarch64-apple-darwin x86_64-apple-darwin; do
  rustup target list --installed 2>/dev/null | grep -qx "$t" && ok "$t" || nein "$t – rustup target add $t"
done

echo "GitHub:"
gh auth status >/dev/null 2>&1 && ok "gh angemeldet" || nein "gh auth login"

echo "Signierschluessel des Updaters:"
if [ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then ok "TAURI_SIGNING_PRIVATE_KEY gesetzt (Inhalt wird nicht angezeigt)"
else nein 'TAURI_SIGNING_PRIVATE_KEY – export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/local-voice.key)"'; fi

echo "Signatur (lva#143):"
SIGN_NAME="${LVA_SIGNING_NAME:-Local Voice Local Signing}"
if [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then ok "APPLE_SIGNING_IDENTITY=$APPLE_SIGNING_IDENTITY"
elif security find-identity -v -p codesigning 2>/dev/null | grep -qF "\"$SIGN_NAME\""; then ok "$SIGN_NAME"
else nein "$SIGN_NAME - scripts/macos-signing-setup.sh (oder LVA_ADHOC=1 fuer ad-hoc)"; fi

echo "Stand:"
VERSION=$(grep -o '"version": "[^"]*"' src-tauri/tauri.conf.json | head -1 | cut -d'"' -f4)
TAG="app-v$VERSION"
git fetch -q --tags origin 2>/dev/null
if [ "$(git rev-parse "$TAG^{commit}" 2>/dev/null)" = "$(git rev-parse HEAD)" ]; then ok "HEAD = $TAG"
else nein "HEAD ist nicht $TAG – git fetch --tags && git checkout $TAG"; fi
[ -z "$(git status --porcelain --untracked-files=no)" ] && ok "Arbeitsbaum sauber" || nein "Arbeitsbaum hat Aenderungen (git status)"
gh release view "$TAG" -R MrP42/local-voice-ai >/dev/null 2>&1 && ok "Release $TAG existiert (Windows-Lauf fertig)" \
  || printf '  WARTET Release %s noch nicht da – release-macos-local.sh wartet bis zu 40 min\n' "$TAG"

echo
if [ "$fehler" -eq 0 ]; then echo "RESULT: OK – weiter mit scripts/release-macos-local.sh"; exit 0; fi
echo "RESULT: $fehler Punkt(e) fehlen"; exit 1
