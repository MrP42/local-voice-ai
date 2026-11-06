#!/usr/bin/env bash
# Regressionstest fuer den Messlauf-Parser (analyze-run.py, vom isolation-check.sh
# benutzt). Prueft jede Fake-JSONL-Fixture unter tests/fixtures gegen den
# erwarteten Grund in tests/expected.tsv: fehlende/ungueltige usage,
# stop_reason <> end_turn (u. a. max_tokens), Fehler, unbekanntes Ereignis,
# Kompaktierung, Tool-Nutzung, kaputte Zeilen und Exit-Code <> 0 muessen den Lauf
# ungueltig machen. Zweiter Teil: der Pruefablauf (check_claude) mit einer
# Fake-CLI, die als Funktion run_claude ersetzt wird: ein ungueltiger Pruefaufruf
# (auch der Werkzeug-Vorschau-Lauf mit Exit 1) darf keine Faehigkeit true lassen.
# Ruft nie die echte CLI auf. Exit-Code 0: alles wie erwartet.
set -u
DIR="$(cd "$(dirname "$0")" && pwd)"
fail=0
total=0
while IFS=$'\t' read -r name rc want; do
  case "$name" in ''|'#'*) continue ;; esac
  total=$((total + 1))
  out="$(python3 -I "$DIR/analyze-run.py" "$DIR/tests/fixtures/$name.jsonl" "$DIR/claude-events.json" "" "$rc")" || { echo "FEHLER: Parser stuerzt ab bei $name" >&2; fail=1; continue; }
  got="$(python3 -c 'import json,sys;print(json.loads(sys.argv[1])["reason"])' "$out")"
  ok="$(python3 -c 'import json,sys;print(json.loads(sys.argv[1])["ok"])' "$out")"
  want_ok=False; [ -z "$want" ] && want_ok=True
  if [ "$got" != "$want" ] || [ "$ok" != "$want_ok" ]; then
    echo "FEHLER: $name (rc=$rc): Grund '$got' (ok=$ok), erwartet '$want' (ok=$want_ok)" >&2
    fail=1
  fi
done < "$DIR/tests/expected.tsv"

# ---- Pruefablauf mit Fake-CLI -------------------------------------------------
# Das Skript wird nur eingelesen (keine Hauptlogik); run_claude wird ersetzt.
export ISOLATION_CHECK_SOURCE_ONLY=1
set --
# shellcheck source=/dev/null
. "$DIR/isolation-check.sh"
DATA_DIR="$WORK/daten"
EVENTS_FILE="$DIR/claude-events.json"

# Szenario-Schalter der Fake-CLI (je Lauf neu gesetzt).
FAKE_TP_RC=0; FAKE_TP_STOP=end_turn; FAKE_TP_MARKER=0; FAKE_T_RC=0; FAKE_C_RC=0

fake_stream() { # $1 = Text, $2 = stop_reason der Nachricht, $3 = stop_reason des Ergebnisses, $4 = extra Block
  python3 - "$1" "$2" "$3" "${4:-}" <<'PY'
import json, sys
text, mstop, rstop, extra = sys.argv[1:5]
blocks = []
if extra == "tool_use":
    blocks.append({"type": "tool_use", "name": "Read", "input": {}})
blocks.append({"type": "text", "text": "x"})
print(json.dumps({"type": "system", "subtype": "init", "model": "claude-sonnet-5-5", "tools": [], "mcp_servers": []}))
print(json.dumps({"type": "assistant", "message": {"model": "claude-sonnet-5-5", "stop_reason": mstop, "content": blocks}}))
print(json.dumps({"type": "result", "subtype": "success", "is_error": False, "result": text, "stop_reason": rstop,
                  "usage": {"input_tokens": 3000, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0, "output_tokens": 5},
                  "modelUsage": {"claude-sonnet-5-5": {}}}))
PY
}

# Fake-CLI: Kontrolllaeufe sehen Kanarienvoegel, fuehren den Hook aus und lesen den
# Marker (tool_use); Vorschau-Laeufe sehen nichts. Die Schalter FAKE_* stoeren einzelne Laeufe.
run_claude() { # $1 = Programm, $2 = Welt, $3 = Modus, $4 = Modell, $5 = Ausgabe, $6 = Frage
  local base="$2" mode="$3" out="$5" question="$6" rc=0 marker
  marker="$(cat "$base/marker")"
  if [ "$mode" = control ] && [ "$question" = "$QUESTION" ]; then
    touch "$base/hook_marker"
    fake_stream "KANARIE-USER KANARIE-PROJEKT KANARIE-MEMORY" end_turn end_turn > "$out"; rc=$FAKE_C_RC
  elif [ "$mode" = control ]; then
    fake_stream "$marker" end_turn end_turn tool_use > "$out"; rc=$FAKE_T_RC
  elif [ "$question" = "$QUESTION" ]; then
    fake_stream "keine" end_turn end_turn > "$out"
  else
    if [ "$FAKE_TP_MARKER" = 1 ]; then fake_stream "$marker" end_turn end_turn > "$out"
    else fake_stream "keine" "$FAKE_TP_STOP" end_turn > "$out"; fi
    rc=$FAKE_TP_RC
  fi
  printf '%s' "$rc" > "$out.rc"
  return 0
}

run_check() { # gibt die Faehigkeiten als JSON aus
  rm -rf "$DATA_DIR"; OBSERVED=""; CONTRACT_OK=true
  check_claude /nicht/benutzt sonnet 2>/dev/null
  finish_event_contract
  write_caps claude_cli "0.0.0-test" 2>/dev/null
}
all_false() { python3 -c 'import json,sys;d=json.loads(sys.argv[1]);sys.exit(0 if not any(d.values()) else 1)' "$1"; }
cap() { python3 -c 'import json,sys;print(json.loads(sys.argv[1])[sys.argv[2]])' "$1" "$2"; }
expect() { # $1 = Name, $2 = Bedingung (Exit 0 = erfuellt)
  total=$((total + 1))
  if ! eval "$2"; then echo "FEHLER: Pruefablauf '$1'" >&2; fail=1; fi
}

caps="$(run_check)"
expect "Grundfall: alle Faehigkeiten belegt" '[ "$(python3 -c "import json,sys;print(all(json.loads(sys.argv[1]).values()))" "$caps")" = True ]'

FAKE_TP_RC=1; caps="$(run_check)"
expect "Werkzeug-Vorschau mit Exit 1: keine Faehigkeit true" 'all_false "$caps"'
FAKE_TP_RC=0

FAKE_TP_STOP=max_tokens; caps="$(run_check)"
expect "Werkzeug-Vorschau abgeschnitten (Nachricht max_tokens): keine Faehigkeit true" 'all_false "$caps"'
FAKE_TP_STOP=end_turn

FAKE_T_RC=1; caps="$(run_check)"
expect "Werkzeug-Kontrolllauf mit Exit 1: keine Faehigkeit true" 'all_false "$caps"'
FAKE_T_RC=0

FAKE_C_RC=1; caps="$(run_check)"
expect "Kanarien-Kontrolllauf mit Exit 1: keine Faehigkeit true" 'all_false "$caps"'
FAKE_C_RC=0

FAKE_TP_MARKER=1; caps="$(run_check)"
expect "Marker im Werkzeug-Vorschau-Lauf: tools_blocked false" '[ "$(cap "$caps" tools_blocked)" = False ]'
FAKE_TP_MARKER=0

caps="$(run_check)"
expect "nach den Stoerungen wieder alle Faehigkeiten belegt" '[ "$(python3 -c "import json,sys;print(all(json.loads(sys.argv[1]).values()))" "$caps")" = True ]'

# Der Parser muss den Messreihen-Pfad des Skripts bedienen (gleiche Datei, kein Eigenbau).
grep -q 'analyze-run.py' "$DIR/isolation-check.sh" || { echo "FEHLER: isolation-check.sh nutzt analyze-run.py nicht" >&2; fail=1; }
if [ "$fail" = 0 ]; then echo "isolation-check-test: $total Faelle gruen"; else echo "isolation-check-test: ROT" >&2; fi
exit "$fail"
