#!/usr/bin/env python3
"""Smoke-Test des lokalen MCP-Servers (M6-P6e): startet die echte EXE mit --mcp
gegen eine SANDBOX-Datenbank und spricht das Protokoll wie ein KI-Client.

    python scripts/mcp_smoke.py <pfad-zu-local-voice-ai.exe> [--keep]

Ablauf (jeder Schritt bricht bei einer Abweichung mit Exit 1 ab):
  1. Sandbox unter %TEMP%: zwei synthetische Besprechungen ueber die
     Test-Zufuhr der EXE (`--reindex-meetings --seed-meetings`, nur mit
     LVA_MEETINGS_DIR, baut auch den Such-Index). Die produktive meetings.db
     wird nie beruehrt; der MCP-Server bekommt LVA_APPDATA_DIR und
     LVA_MEETINGS_DIR auf die Sandbox, seine Einstellungsdatei steht dort.
  2. `--mcp` starten: initialize (beide Protokollversionen + unbekannte),
     notifications/initialized (KEINE Antwort), ping, tools/list (4 Werkzeuge).
  3. Einstellung AUS: jeder Werkzeugaufruf ist isError mit Hinweis.
  4. Einstellung AN (ohne Neustart des Servers): list_meetings, search_meetings,
     get_meeting, get_transcript liefern die Sandbox-Daten; geloeschte gibt es
     nicht; unbekannte Methode -32601, kaputte Zeile -32700.
  5. Jede stdout-Zeile ist gueltiges JSON-RPC; die Datenbankdatei ist nach der
     Sitzung byte-gleich (nur lesend); stdin schliessen beendet den Server mit
     Exit 0.

Exit 0 alles ok, 1 Abweichung, 2 falscher Aufruf. Die EXE darf ein Debug-Build
sein (Konsolen-Subsystem, stdout ist damit lesbar); der Release-Build ist ein
GUI-Programm und liefert stdout nur ueber echte Pipes.
"""

import hashlib
import json
import os
import queue
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

WORD = "Zwiebelkuchenfest"  # kommt nur in der Sandbox vor
TIMEOUT = 20.0


def fail(message: str) -> None:
    print(f"FEHLER: {message}", file=sys.stderr)
    raise SystemExit(1)


def check(condition: bool, message: str) -> None:
    if not condition:
        fail(message)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def seg(index: int, start_ms: int, channel: int, text: str) -> dict:
    return {
        "segment_index": index,
        "text": text,
        "start_ms": start_ms,
        "end_ms": start_ms + 3000,
        "channel": channel,
        "speaker_index": None,
    }


def write_fixtures(directory: Path) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "a_jour_fixe.json").write_text(
        json.dumps(
            {
                "title": "Smoke Jour fixe",
                "segments": [
                    seg(0, 0, 0, "Guten Morgen, wir beginnen mit dem Quartalsbericht."),
                    seg(1, 4000, 1, f"Der Umsatz steigt, dazu gibt es das {WORD}."),
                    seg(2, 9000, 0, "Bitte die Zahlen bis Freitag nachreichen."),
                ],
                "notes": [
                    {"id": "N1", "kind": "bullet", "text": "Zahlen bis Freitag", "at_ms": None, "checked": False}
                ],
            },
            ensure_ascii=False,
        ),
        encoding="utf-8",
    )
    (directory / "b_workshop.json").write_text(
        json.dumps(
            {
                "title": "Smoke Workshop",
                "segments": [seg(0, 0, 0, "Wir sammeln Ideen für den Herbst.")],
                "notes": [],
            },
            ensure_ascii=False,
        ),
        encoding="utf-8",
    )


class Client:
    """Der Server als Kindprozess; stdout wird von einem Thread zeilenweise gelesen."""

    def __init__(self, exe: Path, env: dict) -> None:
        self.proc = subprocess.Popen(
            [str(exe), "--mcp"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
        )
        self.lines: "queue.Queue[bytes | None]" = queue.Queue()
        self.raw: list[str] = []
        self.stderr = b""
        threading.Thread(target=self._pump_stdout, daemon=True).start()
        threading.Thread(target=self._pump_stderr, daemon=True).start()
        self.next_id = 0

    def _pump_stdout(self) -> None:
        assert self.proc.stdout is not None
        for line in self.proc.stdout:
            self.lines.put(line)
        self.lines.put(None)

    def _pump_stderr(self) -> None:
        assert self.proc.stderr is not None
        self.stderr = self.proc.stderr.read()

    def send_raw(self, line: bytes) -> None:
        assert self.proc.stdin is not None
        self.proc.stdin.write(line + b"\n")
        self.proc.stdin.flush()

    def read_line(self, timeout: float = TIMEOUT) -> dict:
        try:
            line = self.lines.get(timeout=timeout)
        except queue.Empty:
            fail("keine Antwort vom Server (Zeitüberschreitung)")
        if line is None:
            fail("der Server hat stdout geschlossen, bevor eine Antwort kam")
        text = line.decode("utf-8")
        self.raw.append(text)
        try:
            value = json.loads(text)
        except json.JSONDecodeError as e:
            fail(f"stdout-Zeile ist kein JSON ({e}): {text[:200]!r}")
        check(value.get("jsonrpc") == "2.0", f"keine JSON-RPC-2.0-Zeile: {text[:200]!r}")
        return value

    def request(self, method: str, params: dict | None = None) -> dict:
        self.next_id += 1
        message = {"jsonrpc": "2.0", "id": self.next_id, "method": method}
        if params is not None:
            message["params"] = params
        self.send_raw(json.dumps(message).encode("utf-8"))
        reply = self.read_line()
        check(reply.get("id") == self.next_id, f"Antwort auf die falsche Anfrage: {reply}")
        return reply

    def notify(self, method: str) -> None:
        self.send_raw(json.dumps({"jsonrpc": "2.0", "method": method}).encode("utf-8"))

    def tool(self, name: str, arguments: dict) -> tuple[str, bool]:
        reply = self.request("tools/call", {"name": name, "arguments": arguments})
        check("result" in reply, f"{name}: Protokollfehler {reply}")
        result = reply["result"]
        return result["content"][0]["text"], bool(result["isError"])

    def close(self) -> int:
        assert self.proc.stdin is not None
        self.proc.stdin.close()
        try:
            return self.proc.wait(timeout=TIMEOUT)
        except subprocess.TimeoutExpired:
            self.proc.kill()  # nach PID, nie nach Programmname
            fail("der Server endet nicht, nachdem stdin geschlossen wurde")


def run_seed(exe: Path, seed_dir: Path, meetings_dir: Path, out_file: Path) -> None:
    env = dict(os.environ, LVA_MEETINGS_DIR=str(meetings_dir))
    proc = subprocess.run(
        [str(exe), "--reindex-meetings", "--seed-meetings", str(seed_dir), "--out", str(out_file)],
        env=env,
        capture_output=True,
        timeout=600,
    )
    # Exit 3 = Einbettungsmodell da, aber nicht alles eingebettet: fuer die Suche unerheblich.
    if proc.returncode not in (0, 3):
        fail(
            f"Sandbox-Aufbau scheiterte (Exit {proc.returncode}): "
            f"{proc.stderr.decode('utf-8', 'replace')[-600:]}"
        )
    report = json.loads(out_file.read_text(encoding="utf-8"))
    check(report.get("seeded") == 2, f"zwei Besprechungen erwartet: {report}")
    check(str(meetings_dir).lower() in report.get("db", "").lower(), f"Datenbank nicht in der Sandbox: {report.get('db')}")


def main() -> int:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    keep = "--keep" in sys.argv[1:]
    if len(args) != 1:
        print(__doc__)
        return 2
    exe = Path(args[0]).resolve()
    if not exe.is_file():
        print(f"EXE nicht gefunden: {exe}", file=sys.stderr)
        return 2

    sandbox = Path(tempfile.mkdtemp(prefix="lva-mcp-smoke-"))
    appdata = sandbox / "appdata"
    meetings = appdata / "meetings"
    meetings.mkdir(parents=True)
    settings = appdata / "settings_store.json"
    try:
        print(f"Sandbox: {sandbox}")
        write_fixtures(sandbox / "seed")
        run_seed(exe, sandbox / "seed", meetings, sandbox / "seed_report.json")
        db = meetings / "meetings.db"
        check(db.is_file(), "Sandbox-Datenbank fehlt")

        env = dict(os.environ, LVA_APPDATA_DIR=str(appdata), LVA_MEETINGS_DIR=str(meetings))
        db_before = sha256(db)
        client = Client(exe, env)

        # -- Handshake -------------------------------------------------------
        for version in ("2025-06-18", "2025-11-25"):
            reply = client.request(
                "initialize",
                {"protocolVersion": version, "capabilities": {}, "clientInfo": {"name": "mcp_smoke", "version": "1"}},
            )
            check(reply["result"]["protocolVersion"] == version, f"Version {version} nicht bestätigt: {reply}")
        reply = client.request("initialize", {"protocolVersion": "1999-01-01"})
        check(reply["result"]["protocolVersion"] == "2025-11-25", f"unbekannte Version: {reply}")
        check(reply["result"]["serverInfo"]["name"] == "local-voice-ai", f"serverInfo: {reply}")
        client.notify("notifications/initialized")
        reply = client.request("ping")  # kommt die Notification beantwortet, passt die id nicht
        check(reply["result"] == {}, f"ping: {reply}")

        tools = client.request("tools/list")["result"]["tools"]
        names = [t["name"] for t in tools]
        check(
            names == ["list_meetings", "search_meetings", "get_meeting", "get_transcript"],
            f"Werkzeuge: {names}",
        )
        check(all(t["annotations"]["readOnlyHint"] is True for t in tools), "ein Werkzeug ist nicht als lesend markiert")

        # -- Einstellung aus (Standard: Datei fehlt) --------------------------
        for name, arguments in [
            ("list_meetings", {}),
            ("search_meetings", {"query": WORD}),
            ("get_meeting", {"id": "x"}),
            ("get_transcript", {"id": "x"}),
        ]:
            text, is_error = client.tool(name, arguments)
            check(is_error and "ausgeschaltet" in text, f"{name} lieferte trotz AUS: {text[:200]}")
            check("Smoke" not in text and WORD not in text, f"{name} leakt Inhalt bei AUS")

        # -- Einstellung an, ohne den Server neu zu starten -------------------
        settings.write_text(
            json.dumps({"settings": {"meeting_mcp_enabled": True, "meeting_mcp_include_transcript": True}}),
            encoding="utf-8",
        )
        text, is_error = client.tool("list_meetings", {})
        check(not is_error, f"list_meetings: {text[:300]}")
        listing = json.loads(text)
        titles = sorted(m["title"] for m in listing["meetings"])
        check(titles == ["Smoke Jour fixe", "Smoke Workshop"], f"Besprechungen: {titles}")
        check(listing["total"] == 2, f"total: {listing}")
        jour_fixe = next(m["id"] for m in listing["meetings"] if m["title"] == "Smoke Jour fixe")

        text, is_error = client.tool("search_meetings", {"query": WORD})
        check(not is_error, f"search_meetings: {text[:300]}")
        found = json.loads(text)
        check([m["id"] for m in found["meetings"]] == [jour_fixe], f"Suche nach {WORD}: {found}")
        check(WORD.lower() in (found["meetings"][0]["snippet"] or "").lower(), f"Auszug ohne Fundstelle: {found}")

        text, is_error = client.tool("get_meeting", {"id": jour_fixe})
        check(not is_error and "Zahlen bis Freitag" in text and jour_fixe in text, f"get_meeting: {text[:300]}")

        text, is_error = client.tool("get_transcript", {"id": jour_fixe})
        check(not is_error, f"get_transcript: {text[:300]}")
        page = json.loads(text)
        check(page["next_cursor"] is None and WORD in page["text"], f"Transkript: {page}")
        check("[00:04] Gegenseite:" in page["text"], f"Sprecherzeile fehlt: {page['text'][:200]!r}")

        text, is_error = client.tool("get_meeting", {"id": "01GIBTESNICHT"})
        check(is_error and "nicht gefunden" in text, f"unbekannte ID: {text[:200]}")

        # -- Protokollfehler --------------------------------------------------
        reply = client.request("gibt/es/nicht")
        check(reply["error"]["code"] == -32601, f"unbekannte Methode: {reply}")
        client.send_raw(b"{das ist kein json")
        reply = client.read_line()
        check(reply["error"]["code"] == -32700 and reply["id"] is None, f"kaputte Zeile: {reply}")
        reply = client.request("ping")
        check(reply["result"] == {}, "der Server lief nach einer kaputten Zeile nicht weiter")

        # -- Ende: stdin zu = Exit 0, Datei unveraendert -----------------------
        code = client.close()
        check(code == 0, f"Exit-Code {code} statt 0; stderr: {client.stderr.decode('utf-8', 'replace')[-400:]}")
        leftover = []
        while True:
            try:
                item = client.lines.get_nowait()
            except queue.Empty:
                break
            if item is not None:
                leftover.append(item)
        check(not leftover, f"unerwartete Zeilen nach dem Ende: {leftover[:2]}")
        check(sha256(db) == db_before, "die Datenbank wurde durch den Server verändert")
        print(f"OK: {len(client.raw)} stdout-Zeilen, alle gültiges JSON-RPC; Datenbank unverändert; Exit 0")
        return 0
    finally:
        if keep:
            print(f"Sandbox bleibt: {sandbox}")
        else:
            shutil.rmtree(sandbox, ignore_errors=True)


if __name__ == "__main__":
    started = time.time()
    code = main()
    print(f"({time.time() - started:.1f} s)")
    sys.exit(code)
