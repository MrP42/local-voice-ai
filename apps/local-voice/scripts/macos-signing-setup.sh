#!/usr/bin/env bash
# Stabiles lokales Signierzertifikat fuer den Mac-Build anlegen (lva#143). Einmalig auf dem
# MacBook ausfuehren, Mac entsperrt (das Vertrauen fuer die Codesignatur fragt das Passwort ab).
#
# Warum: Ad-hoc signiert (signingIdentity "-") ist die Designated Requirement der App ihr cdhash,
# der sich mit jedem Build aendert. macOS verlangt dann nach jedem Update Bedienungshilfen,
# Mikrofon und den Schluesselbund-Zugriff auf den Tresor (lva#134) neu. Mit einem festen
# Zertifikat lautet die Requirement "certificate leaf = H..." und bleibt ueber Updates gleich.
#
# Ablage: ~/.tauri/local-voice-signing.key und .crt (0600) - zusammen mit dem Updater-Schluessel
# sichern. Geht der Schluessel verloren, sind die Freigaben einmalig neu noetig (keine Daten weg).
# Kein Developer-ID-Zertifikat, keine Notarisierung: Gatekeeper warnt beim ersten Start wie bisher.
#
# Idempotent: ist die Identitaet schon gueltig im Schluesselbund, aendert das Skript nichts.
# Aufruf aus apps/local-voice:  scripts/macos-signing-setup.sh
set -euo pipefail

[ "$(uname -s)" = Darwin ] || { echo "Nur auf macOS." >&2; exit 2; }

NAME="${LVA_SIGNING_NAME:-Local Voice Local Signing}"
DIR="$HOME/.tauri"
KEY="$DIR/local-voice-signing.key"
CRT="$DIR/local-voice-signing.crt"
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"
# LibreSSL von macOS: dessen PKCS#12 kann `security import` lesen (OpenSSL 3 braucht sonst -legacy).
OPENSSL=/usr/bin/openssl

gueltig() { security find-identity -v -p codesigning "$KEYCHAIN" 2>/dev/null | grep -qF "\"$NAME\""; }

if gueltig; then
  echo "OK: Signieridentitaet \"$NAME\" ist bereits eingerichtet."
  exit 0
fi

mkdir -p "$DIR"
chmod 700 "$DIR"
umask 077

if [ ! -f "$KEY" ] || [ ! -f "$CRT" ]; then
  [ -f "$KEY" ] || [ -f "$CRT" ] && { echo "Nur eine von $KEY / $CRT vorhanden - bitte pruefen, nichts ueberschrieben." >&2; exit 1; }
  CNF=$(mktemp)
  trap 'rm -f "$CNF"' EXIT
  cat > "$CNF" <<EOF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $NAME
[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
subjectKeyIdentifier = hash
EOF
  "$OPENSSL" req -x509 -newkey rsa:3072 -sha256 -nodes -days 7300 \
    -config "$CNF" -keyout "$KEY" -out "$CRT"
  chmod 600 "$KEY" "$CRT"
  echo "Neu erzeugt: $KEY, $CRT (bitte sichern)."
else
  echo "Vorhandenes Zertifikat aus $DIR wird importiert."
fi

# Import: privater Schluessel nur fuer codesign freigegeben. Das PKCS#12 lebt nur kurz.
P12=$(mktemp)
PW=$("$OPENSSL" rand -hex 24)
trap 'rm -f "$P12" "${CNF:-}"' EXIT
"$OPENSSL" pkcs12 -export -inkey "$KEY" -in "$CRT" -name "$NAME" -out "$P12" -passout "pass:$PW"
security import "$P12" -k "$KEYCHAIN" -P "$PW" -T /usr/bin/codesign -f pkcs12

# Vertrauen nur fuer die Codesignatur (macOS fragt das Passwort ab).
security add-trusted-cert -r trustRoot -p codeSign -k "$KEYCHAIN" "$CRT"

if gueltig; then
  echo "OK: \"$NAME\" eingerichtet. Release-Builds nutzen sie ueber APPLE_SIGNING_IDENTITY."
  echo "Pruefen nach dem Build: codesign -d -r- \"Local Voice AI.app\"  (zeigt certificate leaf)"
else
  echo "Identitaet nicht gueltig - security find-identity -p codesigning ansehen." >&2
  exit 1
fi
