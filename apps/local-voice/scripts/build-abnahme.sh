#!/usr/bin/env bash
# Lokaler Abnahme-Installer (Windows, Git Bash): baut mit [profile.abnahme] statt
# [profile.release] und legt den NSIS-Installer in den Ordner, den die App unter
# "Nach Updates suchen" anbietet. Echte Releases bauen weiter mit dem Release-Profil.
#
# Aufruf aus apps/local-voice:   bash scripts/build-abnahme.sh
# Umgebung (optional):
#   CARGO_TARGET_DIR   gemeinsamer Build-Ordner (Vorgabe D:/lv-build/target-dokumente)
#   CARGO_BUILD_JOBS   parallele Compiler-Jobs (Vorgabe 2, Systemschutz)
#   LVA_UPDATE_DIR     Zielordner fuer den Installer
#                      (Vorgabe ~/local-voice-project/apps/local-voice/src-tauri/target/release/bundle/nsis)
#
# Messung 27.10.2026 (i9-13900K, 2 Jobs): Wiederholbau nach Code-Aenderung
# 5 min 14 s statt 14-16 min mit dem Release-Profil (volle LTO, 1 Codegen-Einheit).
set -euo pipefail

cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-D:/lv-build/target-dokumente}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
update_dir="${LVA_UPDATE_DIR:-$HOME/local-voice-project/apps/local-voice/src-tauri/target/release/bundle/nsis}"

version=$(node -p "require('./package.json').version")
start=$(date +%s)

# Systemschutz: erst bauen, wenn genug Speicher frei ist (falls der Waechter vorhanden ist).
if [ -f /d/lv-build/memgate.sh ]; then
  # shellcheck disable=SC1091
  source /d/lv-build/memgate.sh 28 12 30
fi

# Die Signatur des Update-Artefakts scheitert lokal immer (kein privater Schluessel);
# der Installer ist dann trotzdem fertig. Deshalb zaehlt hier nur, ob er existiert.
# Optional: Update-Quelle fuer den Beta-Kanal (LVA_UPDATE_BASE/_CHANNEL/_PUBKEY, siehe
# scripts/update-config-cli.mjs). Ohne Variablen bleibt die GitHub-Quelle aus tauri.conf.json.
conf_args=()
update_conf=$(node scripts/update-config-cli.mjs)
if [ -n "$update_conf" ] && [ "$update_conf" != "{}" ]; then
  update_conf_file=$(mktemp)
  printf '%s\n' "$update_conf" > "$update_conf_file"
  conf_args=(--config "$update_conf_file")
  echo "Update-Quelle aus Umgebung: $update_conf_file"
fi

setup="$CARGO_TARGET_DIR/abnahme/bundle/nsis/Local Voice AI_${version}_x64-setup.exe"
# Ein Installer aus einem frueheren Lauf darf einen gescheiterten Build nicht verdecken.
rm -f "$setup"

build_log=$(mktemp)
set +e
npx tauri build -b nsis "${conf_args[@]}" -- --profile abnahme 2>&1 | tee "$build_log"
build_rc=${PIPESTATUS[0]}
set -e
if [ "$build_rc" -ne 0 ]; then
  # Nur der fehlende Signaturschluessel ist erwartbar; jeder andere Fehler bricht ab.
  if ! grep -qiE 'TAURI_SIGNING_PRIVATE_KEY|failed to sign|signature' "$build_log"; then
    echo "FEHLER: tauri build scheiterte (Exit $build_rc), kein Signaturproblem" >&2
    exit "$build_rc"
  fi
  echo "Hinweis: Signatur fehlgeschlagen (erwartet ohne privaten Schluessel), Installer wird geprueft."
fi
rm -f "$build_log"

if [ ! -f "$setup" ]; then
  echo "FEHLER: Installer fehlt: $setup" >&2
  exit 1
fi

mkdir -p "$update_dir"
cp "$setup" "$update_dir/"
src_sum=$(sha256sum "$setup" | cut -d' ' -f1)
dst_sum=$(sha256sum "$update_dir/Local Voice AI_${version}_x64-setup.exe" | cut -d' ' -f1)
if [ "$src_sum" != "$dst_sum" ]; then
  echo "FEHLER: Pruefsumme der Kopie weicht ab" >&2
  exit 1
fi

echo "Installer $version fertig in $(( $(date +%s) - start )) s"
echo "  $update_dir/Local Voice AI_${version}_x64-setup.exe (SHA-256 $src_sum)"
echo "  Notizen fuer die App: Local Voice AI_${version}_x64-setup.notes.md daneben ablegen."
