#!/usr/bin/env bash
# Regressionstest fuer den Messlauf-Parser (analyze-run.py, vom isolation-check.sh
# benutzt). Prueft jede Fake-JSONL-Fixture unter tests/fixtures gegen den
# erwarteten Grund in tests/expected.tsv: fehlende/ungueltige usage,
# stop_reason <> end_turn (u. a. max_tokens), Fehler, unbekanntes Ereignis,
# Kompaktierung, Tool-Nutzung, kaputte Zeilen und Exit-Code <> 0 muessen den Lauf
# ungueltig machen. Ruft nie die echte CLI auf. Exit-Code 0: alles wie erwartet.
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
# Der Parser muss den Messreihen-Pfad des Skripts bedienen (gleiche Datei, kein Eigenbau).
grep -q 'analyze-run.py' "$DIR/isolation-check.sh" || { echo "FEHLER: isolation-check.sh nutzt analyze-run.py nicht" >&2; fail=1; }
if [ "$fail" = 0 ]; then echo "isolation-check-test: $total Faelle gruen"; else echo "isolation-check-test: ROT" >&2; fi
exit "$fail"
