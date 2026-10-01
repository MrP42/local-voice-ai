#!/usr/bin/env python3
"""A7: belegt die `ctl`-Exit-Codes gegen eine headless Sandbox-Instanz der echten EXE.

Startet `local-voice-ai.exe --agent-bridge-serve` (kein Fenster, Sandbox-Datenbank in einem
Temp-Ordner, eigener Pipe-Name, Echo-Werkzeuge), legt Zugaenge und Rechte direkt in der
Sandbox-Datenbank an (so, wie die Oberflaeche es ueber die Commands taete), ruft `ctl` mit
verschiedenen Tokens auf und prueft Exit-Codes und Audit. Zum Schluss wird die Instanz beendet
und `ctl status` muss Exit 2 melden.

    python apps/local-voice/scripts/ctl_smoke.py [--exe PFAD] [--out datei.json]

Exit 0, wenn alle Faelle den erwarteten Exit-Code liefern. Beruehrt nie die produktive
Datenbank oder die Pipe einer laufenden App (Sandbox-Ordner, Pipe-Name `lva-smoke-...`).
"""

import argparse
import base64
import hashlib
import json
import os
import pathlib
import secrets
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import uuid

ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_EXE = ROOT / "src-tauri" / "target" / "debug" / "local-voice-ai.exe"
CREATE_NO_WINDOW = 0x08000000


def new_token():
    return "lvat_" + base64.urlsafe_b64encode(secrets.token_bytes(32)).decode().rstrip("=")


def sha256_hex(text):
    return hashlib.sha256(text.encode()).hexdigest()


def now_ms():
    return int(time.time() * 1000)


class Instance:
    """Die headless Sandbox-Instanz."""

    def __init__(self, exe, tmp, pipe):
        self.exe = str(exe)
        self.tmp = tmp
        self.pipe = pipe
        self.env = os.environ.copy()
        self.env.update(
            {
                "LVA_MEETINGS_DIR": str(tmp / "meetings"),
                "LVA_AGENT_PIPE": pipe,
                "LVA_AGENT_TEST_TOOLS": "1",
                "LVA_AGENT_APPROVAL_WAIT_MS": "1500",
            }
        )
        self.env.pop("LVA_AGENT_TOKEN", None)
        self.proc = None
        self.ready = threading.Event()
        self.lines = []

    def start(self, timeout=180):
        (self.tmp / "meetings").mkdir(parents=True, exist_ok=True)
        self.proc = subprocess.Popen(
            [self.exe, "--agent-bridge-serve"],
            env=self.env,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            creationflags=CREATE_NO_WINDOW,
            text=True,
            # Die Debug-EXE schreibt beim Start src/bindings.ts relativ zum Arbeitsordner: aus
            # src-tauri heraus ist das die Datei dieses Checkouts (unveraendert), nie eine fremde.
            cwd=str(ROOT / "src-tauri"),
        )

        def pump():
            for line in self.proc.stdout:
                self.lines.append(line.rstrip())
                if "AGENT_BRIDGE_READY" in line:
                    self.ready.set()

        threading.Thread(target=pump, daemon=True).start()
        if not self.ready.wait(timeout):
            self.stop()
            raise RuntimeError("Die Sandbox-Instanz meldete sich nicht: " + " | ".join(self.lines[-5:]))

    def stop(self):
        if self.proc and self.proc.poll() is None:
            # Nur diese Prozessbaum-Kennung (nie nach Namen beenden).
            subprocess.run(
                ["taskkill", "/PID", str(self.proc.pid), "/T", "/F"],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                creationflags=CREATE_NO_WINDOW,
            )
            try:
                self.proc.wait(timeout=20)
            except subprocess.TimeoutExpired:
                pass

    def ctl(self, *args, token=None, timeout=90):
        env = self.env.copy()
        if token:
            env["LVA_AGENT_TOKEN"] = token
        p = subprocess.run(
            [self.exe, "ctl", *args],
            env=env,
            capture_output=True,
            text=True,
            timeout=timeout,
            creationflags=CREATE_NO_WINDOW,
        )
        return p.returncode, p.stdout.strip(), p.stderr.strip()

    def db(self):
        conn = sqlite3.connect(str(self.tmp / "meetings" / "meetings.db"), timeout=30)
        conn.row_factory = sqlite3.Row
        return conn


def seed(inst):
    """Legt Zugaenge und Rechte an (wie die Oberflaeche ueber die Commands)."""
    good, revoked = new_token(), new_token()
    t = now_ms()
    conn = inst.db()
    with conn:
        conn.execute(
            "INSERT OR IGNORE INTO integrations (id, kind, label, enabled, direction, config_json, created_at, updated_at)"
            " VALUES ('agents', 'agent', 'Externe Agenten', 1, 'both', '{}', ?1, ?1)",
            (t,),
        )
        for cap in ("transcribe.file", "meeting.create", "recording.start"):
            mode = "ask" if cap == "recording.start" else "allow"
            conn.execute(
                "INSERT OR REPLACE INTO integration_grants (integration_id, capability, caller, mode)"
                " VALUES ('agents', ?1, 'agent_external', ?2)",
                (cap, mode),
            )
        conn.execute(
            "INSERT INTO agent_clients (id, label, integration_id, token_hash, created_at) VALUES ('C-GOOD', 'Smoke-Agent', 'agents', ?1, ?2)",
            (sha256_hex(good), t),
        )
        conn.execute(
            "INSERT INTO agent_clients (id, label, integration_id, token_hash, created_at, revoked_at) VALUES ('C-OLD', 'Alter Zugang', 'agents', ?1, ?2, ?2)",
            (sha256_hex(revoked), t),
        )
        for tool, mode in (("transcribe_file", "ask"), ("create_meeting", "allow")):
            conn.execute(
                "INSERT INTO agent_tool_grants (client_id, tool, mode) VALUES ('C-GOOD', ?1, ?2)",
                (tool, mode),
            )
    conn.close()
    return good, revoked


def decide(inst, approval_id, approve):
    """Der Nutzer entscheidet in der App (hier direkt in der Sandbox-Datenbank)."""
    conn = inst.db()
    with conn:
        n = conn.execute(
            "UPDATE approvals SET state = ?1, decided_at = ?2 WHERE id = ?3 AND state = 'pending'",
            ("approved" if approve else "denied", now_ms(), approval_id),
        ).rowcount
    conn.close()
    assert n == 1, "Freigabe nicht offen"


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--exe", default=str(DEFAULT_EXE))
    ap.add_argument("--out", default=None, help="Ergebnis als JSON in diese Datei")
    ap.add_argument("--keep", action="store_true", help="Sandbox-Ordner behalten")
    args = ap.parse_args()
    exe = pathlib.Path(args.exe)
    if not exe.is_file():
        print(f"EXE fehlt: {exe}", file=sys.stderr)
        return 1

    tmp = pathlib.Path(tempfile.mkdtemp(prefix="lva-ctl-smoke-"))
    inst = Instance(exe, tmp, f"lva-smoke-{uuid.uuid4().hex[:12]}")
    results = []

    def check(name, got, expected, extra=""):
        ok = got == expected
        results.append({"case": name, "exit": got, "expected": expected, "ok": ok, "note": extra})
        print(f"{'OK  ' if ok else 'FAIL'} exit={got} (erwartet {expected})  {name}  {extra}")
        return ok

    try:
        # Ohne laufende App: Exit 2.
        code, _, err = inst.ctl("status")
        check("ohne App: ctl status", code, 2, err[:70])

        inst.start()
        good, revoked = seed(inst)

        code, out, _ = inst.ctl("status", "--json")
        j = json.loads(out)
        check("status --json ohne Token", code, 0, f"authenticated={j['result']['authenticated']}")
        code, out, _ = inst.ctl("status", "--json", token=good)
        j = json.loads(out)
        check("status --json mit Token", code, 0, f"client={j['result']['client']['label']}")
        code, _, err = inst.ctl("status", token=new_token())
        check("status mit ungueltigem Token", code, 4, err[:60])
        code, _, err = inst.ctl("status", token=revoked)
        check("status mit zurueckgezogenem Token", code, 4, err[:60])
        code, _, err = inst.ctl("tools")
        check("tools ohne Token", code, 4, err[:60])

        code, out, _ = inst.ctl("tools", "--json", token=good)
        names = [t["name"] for t in json.loads(out)["result"]["tools"]]
        ok = "create_meeting" in names and "transcribe_file" in names and "start_recording" not in names
        check("tools: erlaubte da, ausgeschaltete fehlen", 0 if ok else 1, 0, ",".join(names))

        code, out, _ = inst.ctl("call", "create_meeting", "--args", '{"title":"Rauchtest"}', "--json", token=good)
        j = json.loads(out)
        check("Werkzeug erlaubt: call create_meeting", code, 0, j["result"]["result"]["tool"] if code == 0 else out[:60])

        code, out, _ = inst.ctl("call", "start_recording", "--json", token=good)
        check("Werkzeug aus: call start_recording", code, 3, json.loads(out)["error"]["code"])

        # fragen: nach der verkuerzten Wartezeit pending (Exit 5), Freigabe, Nachholen.
        code, out, _ = inst.ctl("call", "transcribe_file", "--args", '{"path":"C:/Aufnahmen/a.wav"}', "--json", token=good)
        j = json.loads(out)
        check("fragen ohne Antwort: call transcribe_file", code, 5, "pending")
        aid = j["result"]["approval_id"]
        code, _, _ = inst.ctl("approval", aid, token=good)
        check("Stand der Freigabe (offen)", code, 5)
        decide(inst, aid, True)
        code, _, _ = inst.ctl("approval", aid, token=good)
        check("Stand der Freigabe (erteilt)", code, 0)
        code, out, _ = inst.ctl("call", "transcribe_file", "--args", '{"path":"C:/Aufnahmen/a.wav"}', "--approval", aid, "--json", token=good)
        check("Ausfuehrung mit Freigabe", code, 0)
        code, out, _ = inst.ctl("call", "transcribe_file", "--args", '{"path":"C:/Aufnahmen/a.wav"}', "--approval", aid, "--json", token=good)
        check("dieselbe Freigabe ein zweites Mal", code, 1, json.loads(out)["error"]["code"])

        # Freigabe abgelehnt: Exit 3.
        code, out, _ = inst.ctl("call", "transcribe_file", "--args", '{"path":"C:/Aufnahmen/b.wav"}', "--json", token=good)
        aid2 = json.loads(out)["result"]["approval_id"]
        decide(inst, aid2, False)
        code, _, _ = inst.ctl("approval", aid2, token=good)
        check("Freigabe abgelehnt", code, 3)

        # Audit und Klartext-Token.
        conn = inst.db()
        rows = [dict(r) for r in conn.execute("SELECT caller, outcome, capability, detail_json FROM audit_log")]
        conn.close()
        outcomes = sorted({r["outcome"] for r in rows})
        check("Audit enthaelt ok, denied und pending", 0 if {"ok", "denied", "pending"} <= set(outcomes) else 1, 0, ",".join(outcomes))
        db_bytes = (tmp / "meetings" / "meetings.db").read_bytes()
        check("kein Token im Klartext in der Datenbank", 0 if good.encode() not in db_bytes and revoked.encode() not in db_bytes else 1, 0)

        # App beenden: Exit 2.
        inst.stop()
        code, _, err = inst.ctl("status")
        check("nach dem Beenden: ctl status", code, 2, err[:70])
    except Exception as e:  # noqa: BLE001
        print(f"FEHLER: {e}", file=sys.stderr)
        results.append({"case": "Ausnahme", "ok": False, "note": str(e)})
    finally:
        inst.stop()
        if not args.keep:
            shutil.rmtree(tmp, ignore_errors=True)

    failed = [r for r in results if not r.get("ok")]
    print(f"\n{len(results) - len(failed)} von {len(results)} Faellen wie erwartet.")
    if args.out:
        pathlib.Path(args.out).write_text(json.dumps(results, ensure_ascii=False, indent=2), encoding="utf-8")
    return 0 if not failed else 1


if __name__ == "__main__":
    sys.exit(main())
