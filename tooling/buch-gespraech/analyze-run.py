#!/usr/bin/env python3
"""Auswertung EINES Claude-Laufs (stream-json) fuer isolation-check.sh und die Tests.

Aufruf: analyze-run.py <Ausgabedatei> <Whitelist> <Marker> [Exit-Code]
Ohne Exit-Code wird <Ausgabedatei>.rc gelesen. Gibt eine JSON-Zeile aus (nur
Zahlen und Marken, nie den Text). `ok` nur, wenn der Lauf in allen Punkten
gueltig ist: Exit-Code 0, Ereignisvertrag, kein Fehler, keine Kompaktierung,
Abschneiden (siehe unten) ausgeschlossen, vollstaendige und gueltige usage,
Text vorhanden.
Abschneiden wie in der App (cli.rs, parse_claude_stream): ein stop_reason aus
stop_reason ausser fehlend/end_turn/stop_sequence an IRGENDEINEM Ereignis, am
Ereignis UND an message unabhaengig (ohne Prioritaet) oder ein subtype mit "abort" sperrt, auch wenn
das Ergebnis end_turn meldet. Das Ergebnis selbst muss einen regulaeren
stop_reason aus DONE_STOP_REASONS haben; fehlt er oder ist er unbekannt, ist der
Lauf ungueltig (unbekannt ist nie Erfolg).
`reason` nennt den ersten Grund der Ungueltigkeit. Spiegel in der App:
cli.rs (parse_claude_stream) und contract.rs. isolation-check.ps1 spiegelt
diese Regeln.
"""
import json, sys
# Spiegel von CUT_STOP_REASONS/DONE_STOP_REASONS in cli.rs.
CUT_STOP_REASONS = ("max_tokens", "aborted", "model_context_window_exceeded")  # Doku; gesperrt ist alles ausser DONE
MAX_SAFE_INT = 9007199254740991
DONE_STOP_REASONS = ("end_turn", "stop_sequence")
path, events_path, marker = sys.argv[1:4]
rc = 0
if len(sys.argv) > 4:
    rc = int(sys.argv[4])
else:
    try:
        rc = int(open(path + ".rc").read().strip() or "0")
    except (OSError, ValueError):
        rc = 1
wl = json.load(open(events_path, encoding="utf-8"))
rules = {}
for r in wl.get("events", []):
    rules[r["type"]] = r
tool_blocks = set(wl.get("tool_blocks", []))
# Pflichtfelder je Ereignis (typ oder typ/untertyp): nicht negative Ganzzahlen.
numeric_fields = wl.get("numeric_fields", {})
bad, events = False, []
try:
    lines = [l for l in open(path, encoding="utf-8", errors="replace").read().splitlines() if l.strip()]
except OSError:
    lines = []
for l in lines:
    try:
        events.append(json.loads(l))
    except ValueError:
        bad = True

def label(e):
    t = e.get("type", "?")
    return t + "/" + e["subtype"] if isinstance(e.get("subtype"), str) else t

def blocks(e):
    c = (e.get("message") or {}).get("content")
    if isinstance(c, str):
        return ["text"]
    if isinstance(c, list):
        return [(b or {}).get("type", "?") for b in c]
    return []

def is_error(e):
    def flagged(v):
        return v.get("is_error") is True or v.get("error") is not None or v.get("api_error_status") is not None
    return flagged(e) or (isinstance(e.get("message"), dict) and flagged(e["message"]))

def known(e):
    r = rules.get(e.get("type"))
    if r is None:
        return False
    sub = e.get("subtype")
    if "subtypes" in r:
        if not (isinstance(sub, str) and sub in r["subtypes"]):
            return False
    elif sub is not None:
        return False
    # numeric_fields: fehlt ein Feld, ist es keine Zahl oder negativ, ist das Ereignis unbekannt.
    return all(count(e.get(f)) for f in numeric_fields.get(label(e), []))

def contract(evs):
    if not evs or label(evs[0]) != "system/init":
        return "event_order"
    if sum(1 for e in evs if label(e) == "system/init") != 1:
        return "event_order"
    results = [i for i, e in enumerate(evs) if e.get("type") == "result"]
    for i in results:
        if i != len(evs) - 1:
            return "event_after_result"
    if len(results) != 1:
        return "event_order"
    for e in evs:
        bl = blocks(e)
        if e.get("type") in tool_blocks or any(b in tool_blocks for b in bl):
            return "tool_use"
        if not known(e):
            return "event_unknown"
        if is_error(e):
            return "error_event"
        allowed = rules[e["type"]].get("content_blocks", [])
        if any(b not in allowed for b in bl):
            return "content_unknown"
    return ""

models, text, inp, out, stop, err, compacted, tool_use = set(), "", 0, 0, "", "", False, False
cut = False
usage_ok = False


def count(v):
    # Nur echte, nicht negative ganze Zahlen (kein bool, kein String, kein float).
    # Obergrenze 2^53-1 (JSON-sicher), gemeinsam mit contract.rs und der PS1.
    return isinstance(v, int) and not isinstance(v, bool) and 0 <= v <= MAX_SAFE_INT

for e in events:
    t = e.get("type")
    if t == "system" and e.get("subtype") == "compact_boundary" or "compact_metadata" in e:
        compacted = True
    # Beide Felder UNABHAENGIG (Ereignis und message), keine Prioritaet: erlaubt nur
    # fehlend/None, end_turn, stop_sequence; alles andere (auch unbekannt) sperrt.
    msg = e.get("message") if isinstance(e.get("message"), dict) else {}
    for v in (e.get("stop_reason"), msg.get("stop_reason")):
        if v is not None and not (isinstance(v, str) and v in DONE_STOP_REASONS):
            cut = True
    if isinstance(e.get("subtype"), str) and "abort" in e["subtype"]:
        cut = True
    if t == "system" and isinstance(e.get("model"), str):
        models.add(e["model"].strip())
    if t == "assistant" and isinstance((e.get("message") or {}).get("model"), str):
        models.add(e["message"]["model"].strip())
    if t == "assistant":
        if any(b in tool_blocks for b in blocks(e)):
            tool_use = True
    if t == "result":
        text = e.get("result", "") or ""
        u = e.get("usage")
        if isinstance(u, dict) and count(u.get("input_tokens")) and count(u.get("output_tokens")) \
                and all(k not in u or count(u[k]) for k in ("cache_creation_input_tokens", "cache_read_input_tokens")):
            inp = sum(u.get(k, 0) for k in ("input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"))
            out = u["output_tokens"]
            usage_ok = inp > 0 and out > 0
        for k in (e.get("modelUsage") or {}):
            models.add(k.strip())
        stop = e.get("stop_reason", "") or ""
        if e.get("is_error") is not False or e.get("subtype") != "success":
            err = e.get("subtype", "error")
init = next((e for e in events if label(e) == "system/init"), {})
violation = contract(events) if not bad else "bad_line"
# Erster Ungueltigkeitsgrund (leer = gueltiger Lauf). Unbekannt oder fehlend ist nie gueltig.
if rc != 0:
    reason = "exit_code"
elif violation:
    reason = violation
elif err:
    reason = "error_event"
elif compacted:
    reason = "compacted"
elif cut or stop not in DONE_STOP_REASONS:
    reason = "stop_reason"
elif not usage_ok:
    reason = "usage"
elif not text:
    reason = "no_text"
else:
    reason = ""
print(json.dumps({
    "reason": reason,
    "rc": rc, "bad": bad, "error": err, "compacted": compacted, "stop": stop, "cut": cut,
    "input": inp, "output": out, "models": sorted(models), "tool_use": tool_use,
    "marker_seen": bool(marker) and marker in text,
    "tools": init.get("tools"), "mcp": init.get("mcp_servers"),
    "canaries": [w for w in ("KANARIE-USER", "KANARIE-PROJEKT", "KANARIE-MEMORY") if w in text],
    "observed": sorted({label(e) for e in events}),
    "violation": violation,
    "ok": reason == "",
}))
