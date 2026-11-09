#!/usr/bin/env bash
# Interop-Lauf des Assistent-Sync gegen den echten PHP-Hub (ADR-0028 E3.8).
#
# Kopiert das wai-portal (nur lesend) in ein Arbeitsverzeichnis, legt dort eine frische SQLite-Datenbank
# und ein Testkonto an (example.org, synthetisch), startet den PHP-Entwicklungsserver und führt den
# Rust-Interop-Test `sync::portal_hub::interop` aus. Danach wird der Server beendet.
#
# Aufruf (aus apps/local-voice):
#   scripts/portal-hub-interop.sh [PORTAL_DIR] [ARBEITSVERZEICHNIS]
# Umgebung:
#   PORT                 Port des Hubs (Standard 8765)
#   NUR_HUB=1            nur den Hub starten und die Variablen ausgeben (Server läuft bis Strg+C)
#   CARGO_TARGET_DIR     wie für den übrigen Build
set -euo pipefail

HIER="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP="$(cd "$HIER/.." && pwd)"
PORTAL="${1:-$APP/../../../wai-portal}"
ARBEIT="${2:-$(mktemp -d "${TMPDIR:-/tmp}/lva-portal-hub.XXXXXX")}"
PORT="${PORT:-8765}"
EMAIL="interop@example.org"
PASSWORT="interop-$(head -c 12 /dev/urandom | od -An -tx1 | tr -d ' \n')"

if [ ! -f "$PORTAL/bin/console.php" ]; then
  echo "wai-portal nicht gefunden: $PORTAL" >&2
  exit 2
fi
command -v php >/dev/null || { echo "php fehlt" >&2; exit 2; }
php -m | grep -qi pdo_sqlite || { echo "php: pdo_sqlite fehlt" >&2; exit 2; }

PORTAL="$(cd "$PORTAL" && pwd)"
mkdir -p "$ARBEIT"
ARBEIT="$(cd "$ARBEIT" && pwd)"
HUB="$ARBEIT/portal"
echo "Arbeitsverzeichnis: $ARBEIT"

# Nur Code, Konfiguration, Migrationen und öffentliche Dateien; vom Speicherordner nur die Struktur.
rm -rf "$HUB"
mkdir -p "$HUB"
for d in app bin config database public; do
  cp -R "$PORTAL/$d" "$HUB/$d"
done
(cd "$PORTAL" && find storage -type d) | while read -r d; do mkdir -p "$HUB/$d"; done
mkdir -p "$HUB/storage/logs"

cat >"$HUB/.env" <<EOF
APP_ENV=testing
APP_DEBUG=false
APP_URL=http://127.0.0.1:$PORT
APP_SECURE_COOKIES=false
DB_DRIVER=sqlite
DB_DATABASE=$ARBEIT/hub.sqlite
AGENT_API_ENABLED=true
EOF

rm -f "$ARBEIT/hub.sqlite"
php "$HUB/bin/console.php" migrate >/dev/null
php "$HUB/bin/console.php" seed >/dev/null
php "$HUB/bin/console.php" seed:admin "$EMAIL" "$PASSWORT" >/dev/null

php -S "127.0.0.1:$PORT" -t "$HUB/public" "$HUB/public/_router.php" >"$ARBEIT/php-server.log" 2>&1 &
SERVER=$!
trap 'kill "$SERVER" 2>/dev/null || true' EXIT

for _ in $(seq 1 50); do
  if curl -s -o /dev/null --noproxy 127.0.0.1 "http://127.0.0.1:$PORT/"; then break; fi
  sleep 0.2
done

export LVA_PORTAL_HUB_URL="http://127.0.0.1:$PORT"
export LVA_PORTAL_HUB_EMAIL="$EMAIL"
export LVA_PORTAL_HUB_PASSWORT="$PASSWORT"
export LVA_PORTAL_HUB_DB="$ARBEIT/hub.sqlite"
export NO_PROXY="127.0.0.1,localhost${NO_PROXY:+,$NO_PROXY}"
export no_proxy="$NO_PROXY"

if [ "${NUR_HUB:-0}" = "1" ]; then
  echo "LVA_PORTAL_HUB_URL=$LVA_PORTAL_HUB_URL"
  echo "LVA_PORTAL_HUB_EMAIL=$LVA_PORTAL_HUB_EMAIL"
  echo "LVA_PORTAL_HUB_PASSWORT=$LVA_PORTAL_HUB_PASSWORT"
  echo "LVA_PORTAL_HUB_DB=$LVA_PORTAL_HUB_DB"
  wait "$SERVER"
  exit 0
fi

cd "$APP/src-tauri"
status=0
cargo test --lib sync::portal_hub::interop -- --ignored --nocapture || status=$?
if [ "$status" -ne 0 ]; then
  echo "Interop fehlgeschlagen (Exit $status); Server-Log: $ARBEIT/php-server.log" >&2
fi
exit "$status"
