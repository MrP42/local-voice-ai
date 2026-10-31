#!/usr/bin/env bash
# Lokaler Abnahme-Installer (Windows, Git Bash): baut mit [profile.abnahme] statt
# [profile.release] und legt den NSIS-Installer in den Ordner, den die App unter
# "Nach Updates suchen" anbietet. Echte Releases bauen weiter mit dem Release-Profil.
#
# Aufruf aus apps/local-voice:   bash scripts/build-abnahme.sh
# Umgebung (optional):
#   CARGO_TARGET_DIR   gemeinsamer Build-Ordner (Vorgabe D:/b: kurzer Pfad, dort liegt der LTO-freie Stand)
#   CARGO_BUILD_JOBS   parallele Compiler-Jobs (Vorgabe 2, Systemschutz)
#   LVA_UPDATE_DIR     Zielordner fuer den Installer
#                      (Vorgabe ~/local-voice-project/apps/local-voice/src-tauri/target/release/bundle/nsis)
#
# Schnell bleiben (Ziel unter 5 min Wiederholbau): IMMER im selben Worktree bauen (der Pfad steckt in den
# Cargo-Fingerprints; ein neuer Worktree baut alle Workspace-Crates neu), dasselbe CARGO_TARGET_DIR behalten, sccache
# fuer die Abhaengigkeiten (Vorgabe D:/lv-build/sccache, falls `sccache` vorhanden), NSIS mit zlib statt LZMA.
# Das Skript legt die Update-Notizen (`.notes.md`) selbst an: Commit-Betreffe seit dem letzten Tag app-v*.
#
# Messung 30.10.2026 (lto = false, TRANSCRIBE_DIR): Wiederholbau 90 s; der erste Lauf je Target ca. 22 min.
# Messung 27.10.2026 (i9-13900K, 2 Jobs): Wiederholbau nach Code-Aenderung
# 5 min 14 s statt 14-16 min mit dem Release-Profil (volle LTO, 1 Codegen-Einheit).
set -euo pipefail

cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-D:/b}"
# Vorgebaute Whisper-Bibliothek: der cmake-Neubau von transcribe-cpp-sys scheitert unter cargo auf diesem PC (MSB1009).
# Erzeugen: cmake wie im Build-Script von transcribe-cpp-sys mit -DTRANSCRIBE_INSTALL=ON, Release, Install nach diesem Ordner.
if [ -z "${TRANSCRIBE_DIR:-}" ] && [ -f "D:/lv-build/transcribe-prebuilt/lib/transcribe-link.json" ]; then
  export TRANSCRIBE_DIR="D:/lv-build/transcribe-prebuilt"
fi
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
if command -v sccache >/dev/null 2>&1 && [ -z "${RUSTC_WRAPPER:-}" ]; then
  export SCCACHE_DIR="${SCCACHE_DIR:-D:/lv-build/sccache}"
  export RUSTC_WRAPPER=sccache
fi
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
# Immer ein --config: zlib-Kompression fuers NSIS-Paket (spart bei grossen Ressourcen Minuten), plus ggf. die Update-Quelle.
conf_args=()
update_conf=$(node scripts/update-config-cli.mjs)
if [ -n "$update_conf" ] && [ "$update_conf" != "{}" ]; then
  update_conf_file=$(mktemp)
  printf '%s\n' "$update_conf" > "$update_conf_file"
  conf_args=(--config "$update_conf_file")
  echo "Update-Quelle aus Umgebung: $update_conf_file"
fi
speed_conf_file=$(mktemp)
printf '%s\n' '{"bundle":{"windows":{"nsis":{"compression":"zlib"}}}}' > "$speed_conf_file"
conf_args+=(--config "$speed_conf_file")

build_start=$(date +%s)
npx tauri build -b nsis "${conf_args[@]}" -- --profile abnahme || true
echo "tauri build: $(( $(date +%s) - build_start )) s (RUSTC_WRAPPER=${RUSTC_WRAPPER:-keiner})"

setup="$CARGO_TARGET_DIR/abnahme/bundle/nsis/Local Voice AI_${version}_x64-setup.exe"
if [ ! -f "$setup" ]; then
  echo "FEHLER: Installer fehlt: $setup" >&2
  exit 1
fi
# Ein schluckender `|| true` darf keinen alten Installer als neu ausgeben: er muss nach dem Bau-Start entstanden sein.
setup_mtime=$(stat -c %Y "$setup")
if [ "$setup_mtime" -lt "$build_start" ]; then
  echo "FEHLER: Installer ist aelter als dieser Lauf (Bau fehlgeschlagen, alter Stand): $setup" >&2
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

# Notizen fuer die App: Commit-Betreffe seit dem letzten Release-Tag (nur anlegen, wenn noch keine da ist).
notes="$update_dir/Local Voice AI_${version}_x64-setup.notes.md"
if [ ! -f "$notes" ]; then
  last_tag=$(git describe --tags --abbrev=0 --match 'app-v*' 2>/dev/null || true)
  range=${last_tag:+$last_tag..HEAD}
  { printf '# Local Voice AI %s\n\n' "$version"; git log --no-merges --max-count=40 --pretty='- %s' ${range:-HEAD}; } > "$notes"
  echo "Notizen angelegt: $notes"
fi

echo "Installer $version fertig in $(( $(date +%s) - start )) s"
echo "  $update_dir/Local Voice AI_${version}_x64-setup.exe (SHA-256 $src_sum)"
