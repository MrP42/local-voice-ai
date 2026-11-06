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

# Entwicklungskanal (src-tauri/src/dev_update.rs, Codex-Review 06.11.2026): Abnahme-Bauten bekommen den
# Pubkey eines EIGENEN Entwicklungsschluessels eingebacken und spielen damit signierte Testversionen aus dem
# Update-Ordner selbst ein. Der Produktionsschluessel (local-voice-ai.key) signiert hier nie; Releases
# kennen den Entwicklungsschluessel nicht (die Release-Workflows brechen ab, wenn er gesetzt ist).
dev_key="${LVA_DEV_UPDATE_KEY:-$HOME/.tauri/local-voice-dev.key}"
if [ ! -f "$dev_key" ]; then
  echo "Entwicklungsschluessel fehlt, lege ihn an: $dev_key"
  mkdir -p "$(dirname "$dev_key")"
  npx tauri signer generate --ci -p "" -w "$dev_key"
fi
if [ ! -f "$dev_key.pub" ]; then
  echo "FEHLER: $dev_key.pub fehlt" >&2
  exit 1
fi
LVA_DEV_UPDATE_PUBKEY=$(tr -d '\r\n' < "$dev_key.pub")
export LVA_DEV_UPDATE_PUBKEY
TAURI_SIGNING_PRIVATE_KEY=$(tr -d '\r\n' < "$dev_key")
export TAURI_SIGNING_PRIVATE_KEY
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
echo "Signatur mit Entwicklungsschluessel $dev_key"
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
target_exe="$update_dir/Local Voice AI_${version}_x64-setup.exe"
# Atomar veroeffentlichen: erst unter .partial schreiben (passt auf kein Installer-Muster), dann umbenennen.
# Die Signatur zuerst, damit der Installer nie ohne seine Signatur sichtbar ist. Eine alte Signatur zum
# selben Namen wird vorher entfernt; die App prueft jede Signatur ohnehin selbst (private Kopie).
fresh_sig=0
if [ -f "$setup.sig" ] && [ "$(stat -c %Y "$setup.sig")" -ge "$build_start" ]; then
  fresh_sig=1
fi
rm -f "$target_exe.sig"
cp "$setup" "$target_exe.partial"
src_sum=$(sha256sum "$setup" | cut -d' ' -f1)
dst_sum=$(sha256sum "$target_exe.partial" | cut -d' ' -f1)
if [ "$src_sum" != "$dst_sum" ]; then
  rm -f "$target_exe.partial"
  echo "FEHLER: Pruefsumme der Kopie weicht ab" >&2
  exit 1
fi
if [ "$fresh_sig" = 1 ]; then
  cp "$setup.sig" "$target_exe.sig.partial"
  mv -f "$target_exe.sig.partial" "$target_exe.sig"
fi
mv -f "$target_exe.partial" "$target_exe"
if [ "$fresh_sig" = 1 ]; then
  echo "Signatur: $target_exe.sig (die App spielt diese Testversion selbst ein)"
else
  echo "WARNUNG: keine frische Signatur ($setup.sig); die App bietet den Installer nur per Klick an" >&2
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
