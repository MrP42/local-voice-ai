#!/usr/bin/env python3
"""Smoke-Test des lokalen MCP-Servers (M6-P6e): startet die echte EXE mit --mcp
gegen eine SANDBOX-Datenbank und spricht das Protokoll wie ein KI-Client.

    python scripts/mcp_smoke.py <pfad-zu-local-voice-ai.exe> [--keep]
    python scripts/mcp_smoke.py --write [<pfad-zu-local-voice-ai.exe>] [--keep] [--out ergebnis.json]
    python scripts/mcp_smoke.py --workflows [<pfad-zu-local-voice-ai.exe>] [--keep] [--out ergebnis.json]

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

`--write` (A8, AK10/AK11) prueft die SCHREIBENDEN Werkzeuge ueber den MCP-Proxy gegen eine
headless Sandbox-Instanz der App (`--agent-bridge-serve`, echte Handler, kein Fenster, kein
Mikrofon, kein Netz: YouTube-Metadaten kommen aus einer lokalen Attrappe, der Sprach-Engine
ersetzt ein Testton). Zugang und Rechte legt das Skript direkt in der Sandbox-Datenbank an, wie
die Oberflaeche es taete; die Zustimmung des Nutzers ebenfalls. Geprueft werden:
  - transcribe_file -> meeting_id; tts_page_create + tts_render_audio -> gueltige WAV;
    add_youtube_source -> Besprechung (Herkunft: Agent, genau ein Abruf an die Attrappe);
  - fragen: pending kommt als Hinweis (kein Fehler), nach der Freigabe laeuft derselbe Aufruf;
  - start_recording ohne Einwilligung in der App startet NIE eine Aufnahme (auch nicht, wenn
    der Nutzer abgelehnt hat oder die Sandbox keinen Recorder hat);
  - Protokollversion 2026-07-28 (server/discover, _meta, -32022) neben dem alten Handshake;
  - `ctl status --json` Exit 0, ohne App Exit 2, Werkzeug „aus“ Exit 3;
  - `--audit-dump --json` enthaelt alle Aktionen, Aufbewahrung gedeckelt, kein Token im Klartext.
`--workflows` (B8, AK10) prueft die Werkzeuge der Automationen ueber den MCP-Proxy und `ctl` gegen dieselbe
headless Sandbox-Instanz (Engine mit Arbeiter, zwei Ablaeufe direkt in der Sandbox-Datenbank, vier Zugaenge:
voll, nur lesend, fragen, ohne Recht). Geprueft werden:
  - list_workflows (Felder, keine Definition), run_workflow als Trockenlauf (Standard, Herkunft agent) und
    get_run (Zustand done, Schritte mit Plan, ohne eingesetzte Parameter und Ausloeserdaten);
  - live=true nur bei scharfem, eingeschaltetem Ablauf (sonst Fehler und KEIN Lauf); request_id ist idempotent;
  - `ctl workflow list|run|get --json`: Exit 0; ohne Recht Exit 3, ohne Token Exit 4, fragen Exit 5 und die
    Freigabe gilt nur fuer dieselben Argumente (live);
  - der Audit-Dump enthaelt workflow.read/workflow.run, Verweigerungen und kein Token.
Die produktive Datenbank und die Pipe einer laufenden App bleiben unberuehrt (Sandbox-Ordner,
Pipe-Name `lva-mcp-smoke-...`).
"""

import hashlib
import json
import os
import queue
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import uuid
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
            creationflags=0x08000000 if os.name == "nt" else 0,  # CREATE_NO_WINDOW
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

    def call(self, name: str, arguments: dict) -> dict:
        """Das ganze Ergebnis eines Werkzeugaufrufs (content, isError, structuredContent)."""
        reply = self.request("tools/call", {"name": name, "arguments": arguments})
        check("result" in reply, f"{name}: Protokollfehler {reply}")
        return reply["result"]

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


# ---------------------------------------------------------------------------
# --write: schreibende Werkzeuge ueber den MCP-Proxy und die Agentenbruecke (A8, AK10, AK11)
# ---------------------------------------------------------------------------

CREATE_NO_WINDOW = 0x08000000 if os.name == "nt" else 0
YT_ID = "dQw4w9WgXcQ"
YT_LINK = f"https://youtu.be/{YT_ID}?si=TRACKING"
OEMBED_BODY = {
    "title": "Lastgang verstehen: Spitzen glätten",
    "author_name": "Wolff Applied AI",
    "author_url": "https://www.youtube.com/@wolffappliedai",
    "type": "video",
    "thumbnail_url": f"https://i.ytimg.com/vi/{YT_ID}/hqdefault.jpg",
}


class OEmbedStub:
    """Ein lokaler oEmbed-Endpunkt (Test-Attrappe): kein Netz, zaehlt die Abrufe."""

    def __init__(self) -> None:
        import http.server
        import socketserver

        stub = self
        self.requests: list[str] = []

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self) -> None:  # noqa: N802
                stub.requests.append(self.path)
                body = json.dumps(OEMBED_BODY).encode("utf-8")
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *args: object) -> None:  # still
                pass

        self.server = socketserver.TCPServer(("127.0.0.1", 0), Handler)
        self.port = self.server.server_address[1]
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    @property
    def url(self) -> str:
        return f"http://127.0.0.1:{self.port}/oembed"

    def close(self) -> None:
        self.server.shutdown()
        self.server.server_close()


def new_token() -> str:
    import base64
    import secrets

    return "lvat_" + base64.urlsafe_b64encode(secrets.token_bytes(32)).decode().rstrip("=")


class Bridge:
    """Die headless Sandbox-Instanz der App (`--agent-bridge-serve`, echte Handler, kein Fenster)."""

    def __init__(self, exe: Path, sandbox: Path, pipe: str, oembed: str) -> None:
        self.exe = str(exe)
        self.sandbox = sandbox
        self.appdata = sandbox / "appdata"
        self.meetings = self.appdata / "meetings"
        self.pipe = pipe
        self.env = dict(
            os.environ,
            LVA_APPDATA_DIR=str(self.appdata),
            LVA_MEETINGS_DIR=str(self.meetings),
            LVA_AGENT_PIPE=pipe,
            LVA_AGENT_APPROVAL_WAIT_MS="1500",
            LVA_YOUTUBE_OEMBED_URL=oembed,
        )
        for key in ("LVA_AGENT_TOKEN", "LVA_AGENT_TEST_TOOLS"):
            self.env.pop(key, None)
        self.proc: "subprocess.Popen[str] | None" = None
        self.ready = threading.Event()
        self.lines: list[str] = []

    def start(self, timeout: float = 240.0) -> None:
        self.meetings.mkdir(parents=True, exist_ok=True)
        self.proc = subprocess.Popen(
            [self.exe, "--agent-bridge-serve"],
            env=self.env,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            creationflags=CREATE_NO_WINDOW,
            text=True,
            # Die Debug-EXE schreibt beim Start src/bindings.ts relativ zum Arbeitsordner.
            cwd=str(Path(self.exe).resolve().parents[2]),
        )

        def pump() -> None:
            assert self.proc is not None and self.proc.stdout is not None
            for line in self.proc.stdout:
                self.lines.append(line.rstrip())
                if "AGENT_BRIDGE_READY" in line:
                    self.ready.set()

        threading.Thread(target=pump, daemon=True).start()
        if not self.ready.wait(timeout):
            self.stop()
            fail("die Sandbox-Instanz meldete sich nicht: " + " | ".join(self.lines[-5:]))

    def stop(self) -> None:
        if self.proc and self.proc.poll() is None:
            # Nur diese Prozess-Kennung samt Kindern (nie nach Programmname).
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

    def db(self) -> sqlite3.Connection:
        conn = sqlite3.connect(str(self.meetings / "meetings.db"), timeout=30)
        conn.row_factory = sqlite3.Row
        return conn

    def ctl(self, *args: str, token: "str | None" = None, timeout: float = 90.0) -> "tuple[int, str, str]":
        env = dict(self.env)
        if token:
            env["LVA_AGENT_TOKEN"] = token
        p = subprocess.run(
            [self.exe, "ctl", *args],
            env=env,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=timeout,
            creationflags=CREATE_NO_WINDOW,
        )
        return p.returncode, p.stdout.strip(), p.stderr.strip()

    def audit_dump(self) -> dict:
        p = subprocess.run(
            [self.exe, "--audit-dump", "--json"],
            env=self.env,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=180,
            creationflags=CREATE_NO_WINDOW,
            cwd=str(Path(self.exe).resolve().parents[2]),
        )
        check(p.returncode == 0, f"--audit-dump Exit {p.returncode}: {p.stderr[-300:]}")
        last = [ln for ln in p.stdout.splitlines() if ln.startswith("{")][-1]
        return json.loads(last)


def seed_write(bridge: Bridge) -> "tuple[str, str]":
    """Zugang und Rechte, wie die Oberflaeche sie ueber die Commands anlegt (hier direkt in der
    Sandbox-Datenbank). Rueckgabe: Token, Kennung des Zugangs."""
    import hashlib

    token = new_token()
    now = int(time.time() * 1000)
    conn = bridge.db()
    with conn:
        conn.execute(
            "INSERT OR IGNORE INTO integrations (id, kind, label, enabled, direction, config_json, created_at, updated_at)"
            " VALUES ('agents', 'agent', 'Externe Agenten', 1, 'both', '{}', ?1, ?1)",
            (now,),
        )
        for cap in ("transcribe.file", "meeting.create", "tts.render", "youtube.add"):
            conn.execute(
                "INSERT OR REPLACE INTO integration_grants (integration_id, capability, caller, mode)"
                " VALUES ('agents', ?1, 'agent_external', 'allow')",
                (cap,),
            )
        conn.execute(
            "INSERT OR REPLACE INTO integration_grants (integration_id, capability, caller, mode)"
            " VALUES ('agents', 'recording.start', 'agent_external', 'ask')"
        )
        conn.execute(
            "INSERT INTO agent_clients (id, label, integration_id, token_hash, created_at)"
            " VALUES ('C-SMOKE', 'Smoke-Agent', 'agents', ?1, ?2)",
            (hashlib.sha256(token.encode()).hexdigest(), now),
        )
        for tool, mode in (
            ("transcribe_file", "allow"),
            ("create_meeting", "allow"),
            ("create_session", "ask"),  # fragen: Hinweis pending, dann Freigabe in der App
            ("tts_page_create", "allow"),
            ("tts_render_audio", "allow"),
            ("add_youtube_source", "allow"),
            ("start_recording", "ask"),
            # stop_recording bleibt ohne Zeile: aus
        ):
            conn.execute(
                "INSERT INTO agent_tool_grants (client_id, tool, mode) VALUES ('C-SMOKE', ?1, ?2)",
                (tool, mode),
            )
    conn.close()
    return token, "C-SMOKE"


def user_decides(bridge: Bridge, approval_id: str, approve: bool) -> None:
    """Der Nutzer entscheidet in der App (hier direkt in der Sandbox-Datenbank)."""
    conn = bridge.db()
    with conn:
        n = conn.execute(
            "UPDATE approvals SET state = ?1, decided_at = ?2 WHERE id = ?3 AND state = 'pending'",
            ("approved" if approve else "denied", int(time.time() * 1000), approval_id),
        ).rowcount
    conn.close()
    check(n == 1, f"Freigabe {approval_id} ist nicht offen")


def write_test_wav(path: Path) -> None:
    import wave

    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(16000)
        w.writeframes(b"\x00\x00" * 16000)


def run_write(exe: Path, keep: bool, out: "str | None") -> int:
    import wave

    sandbox = Path(tempfile.mkdtemp(prefix="lva-mcp-write-smoke-"))
    stub = OEmbedStub()
    bridge = Bridge(exe, sandbox, f"lva-mcp-smoke-{uuid.uuid4().hex[:12]}", stub.url)
    results: list[dict] = []
    client: "Client | None" = None

    def record(name: str, ok: bool, note: str = "") -> None:
        results.append({"case": name, "ok": ok, "note": note})
        print(f"{'OK  ' if ok else 'FAIL'} {name}  {note}")
        if not ok:
            raise SystemExit(f"Abweichung: {name} {note}")

    def exit_is(name: str, got: int, expected: int, note: str = "") -> None:
        record(f"{name} (Exit {got}, erwartet {expected})", got == expected, note)

    try:
        print(f"Sandbox: {sandbox}")
        # Ohne laufende App: Exit 2 (ctl) und keine schreibenden Werkzeuge im Proxy.
        code, _, err = bridge.ctl("status")
        exit_is("ctl status ohne App", code, 2, err[:60])

        bridge.start()
        token, client_id = seed_write(bridge)
        (bridge.appdata / "settings_store.json").write_text(
            json.dumps({"settings": {"meeting_mcp_enabled": True, "meeting_mcp_include_transcript": True}}),
            encoding="utf-8",
        )

        # --- ctl ---------------------------------------------------------------------------
        code, out_text, _ = bridge.ctl("status", "--json")
        exit_is("ctl status --json (ohne Token)", code, 0)
        code, out_text, _ = bridge.ctl("status", "--json", token=token)
        info = json.loads(out_text)["result"]
        exit_is("ctl status --json (mit Token)", code, 0, f"angemeldet als {info['client']['label']}")
        code, out_text, _ = bridge.ctl("call", "stop_recording", "--json", token=token)
        exit_is("ctl call stop_recording (Werkzeug aus)", code, 3, json.loads(out_text)["error"]["code"])

        # --- MCP-Proxy mit Zugang -------------------------------------------------------------
        env = dict(bridge.env, LVA_AGENT_TOKEN=token)
        client = Client(exe, env)
        reply = client.request(
            "initialize",
            {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "mcp_smoke", "version": "1"}},
        )
        record("initialize (alter Handshake)", reply["result"]["protocolVersion"] == "2025-11-25")
        meta = {
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientInfo": {"name": "mcp_smoke", "version": "1"},
            "io.modelcontextprotocol/clientCapabilities": {},
        }
        reply = client.request("server/discover", {"_meta": meta})
        record(
            "server/discover (2026-07-28)",
            reply["result"]["supportedVersions"][0] == "2026-07-28" and reply["result"]["resultType"] == "complete",
        )
        reply = client.request("ping", {"_meta": dict(meta, **{"io.modelcontextprotocol/protocolVersion": "1900-01-01"})})
        record("unbekannte Version -> -32022 mit supported", reply["error"]["code"] == -32022 and "2026-07-28" in reply["error"]["data"]["supported"])
        listing = client.request("tools/list", {"_meta": meta})
        record("tools/list (2026-07-28): resultType und Cache-Hinweise", listing["result"]["resultType"] == "complete" and listing["result"]["ttlMs"] == 0)
        names = [t["name"] for t in listing["result"]["tools"]]
        wanted = [
            "transcribe_file", "create_meeting", "create_session", "tts_page_create", "tts_render_audio",
            "add_youtube_source", "start_recording", "get_action_status", "get_provenance",
        ]
        record("tools/list: freigegebene Werkzeuge da, stop_recording (aus) fehlt", all(n in names for n in wanted) and "stop_recording" not in names, ",".join(names))

        # transcribe_file -> meeting_id (Besprechung in der Warteschlange)
        wav = sandbox / "Jour fixe.wav"
        write_test_wav(wav)
        res = client.call("transcribe_file", {"path": str(wav)})
        record("transcribe_file -> meeting_id", not res["isError"] and "meeting_id" in res["structuredContent"], res["content"][0]["text"][:90])
        mid = res["structuredContent"]["meeting_id"]
        conn = bridge.db()
        row = conn.execute("SELECT status, source, source_path FROM meetings WHERE id = ?", (mid,)).fetchone()
        queued = conn.execute("SELECT COUNT(*) FROM import_queue WHERE meeting_id = ?", (mid,)).fetchone()[0]
        conn.close()
        record("Besprechung steht in der Warteschlange", row and row["status"] == "queued" and row["source"] == "import" and queued == 1, f"{dict(row) if row else None}")
        res = client.call("transcribe_file", {"path": r"\\server\freigabe\a.wav"})
        record("transcribe_file: Netzwerkpfad abgewiesen", res["isError"] and "Netzwerk" in res["content"][0]["text"])
        res = client.call("transcribe_file", {"path": r"C:\Windows\System32\drivers\etc\hosts"})
        record("transcribe_file: Nicht-Mediendatei abgewiesen", res["isError"])

        # tts_page_create + tts_render_audio -> WAV
        res = client.call("tts_page_create", {"title": "Begrüßung", "text": "Guten Tag. Dies ist ein Test."})
        record("tts_page_create -> page_id", not res["isError"], res["content"][0]["text"][:80])
        page_id = res["structuredContent"]["page_id"]
        res = client.call("tts_render_audio", {"page_id": page_id, "file_name": "begruessung"})
        record("tts_render_audio -> WAV", not res["isError"] and res["structuredContent"]["format"] == "wav", res["content"][0]["text"][:100])
        wav_path = Path(res["structuredContent"]["path"])
        with wave.open(str(wav_path), "rb") as w:
            frames, rate, channels = w.getnframes(), w.getframerate(), w.getnchannels()
        record("die Datei ist eine gueltige WAV-Datei", frames == 16000 and rate == 16000 and channels == 1, f"{frames} Frames, {rate} Hz")
        record("die WAV liegt im Seitenordner der Sandbox", str(sandbox) in str(wav_path))

        # add_youtube_source -> Besprechung (oEmbed-Attrappe, kein Netz, nie yt-dlp)
        res = client.call("add_youtube_source", {"url": YT_LINK})
        record("add_youtube_source -> Besprechung", not res["isError"] and res["structuredContent"]["source"] == "youtube", res["content"][0]["text"][:100])
        yid = res["structuredContent"]["meeting_id"]
        record("genau ein oEmbed-Abruf an die Attrappe", len(stub.requests) == 1 and YT_ID in stub.requests[0], str(stub.requests))
        conn = bridge.db()
        row = conn.execute("SELECT title, source, status FROM meetings WHERE id = ?", (yid,)).fetchone()
        prov = conn.execute(
            "SELECT actor_kind, actor_ref FROM provenance WHERE subject_kind = 'transcript' AND subject_id = ?", (yid,)
        ).fetchone()
        conn.close()
        record("YouTube-Besprechung mit Titel aus der Attrappe", row and row["source"] == "youtube" and row["title"] == OEMBED_BODY["title"])
        record("Herkunft nennt den Agenten", prov and prov["actor_kind"] == "agent_external" and prov["actor_ref"] == client_id, f"{dict(prov) if prov else None}")
        res = client.call("add_youtube_source", {"url": "https://example.com/video"})
        record("add_youtube_source: fremder Link abgewiesen, kein weiterer Abruf", res["isError"] and len(stub.requests) == 1)
        # Herkunft ueber das lesende MCP-Werkzeug
        prov_tool = client.call("get_provenance", {"id": yid})
        entries = json.loads(prov_tool["content"][0]["text"])["entries"]
        record("get_provenance liefert den Ausloeser Agent", any(e["actor"] == "agent_external" for e in entries), str(entries)[:120])

        # fragen: pending ist ein Hinweis, kein Fehler; nach der Freigabe laeuft derselbe Aufruf
        res = client.call("create_session", {"name": "Podcast"})
        record("create_session (fragen) -> pending als Hinweis, nicht als Fehler", not res["isError"] and res["structuredContent"]["status"] == "pending", res["content"][0]["text"][:80])
        aid = res["structuredContent"]["approval_id"]
        conn = bridge.db()
        n_sessions = conn.execute("SELECT COUNT(*) FROM meeting_folders").fetchone()[0]
        conn.close()
        record("ohne Freigabe wurde keine Session angelegt", n_sessions == 0)
        user_decides(bridge, aid, True)
        res = client.call("create_session", {"name": "Podcast", "approval_id": aid})
        record("nach der Freigabe: create_session laeuft", not res["isError"] and "session_id" in res["structuredContent"], res["content"][0]["text"][:80])
        res = client.call("create_session", {"name": "Podcast", "approval_id": aid})
        record("dieselbe Freigabe gilt nur einmal", res["isError"])

        # Aufnahme: ohne Einwilligung in der App startet NIE eine Aufnahme
        def recordings() -> int:
            c = bridge.db()
            n = c.execute("SELECT COUNT(*) FROM meetings WHERE source = 'live' OR status = 'recording'").fetchone()[0]
            c.close()
            return n

        res = client.call("start_recording", {"title": "Heimlich"})
        record("start_recording ohne Einwilligung -> pending, keine Aufnahme", not res["isError"] and res["structuredContent"]["status"] == "pending" and recordings() == 0)
        rec_aid = res["structuredContent"]["approval_id"]
        user_decides(bridge, rec_aid, False)
        res = client.call("start_recording", {"title": "Heimlich", "approval_id": rec_aid})
        record("abgelehnt: start_recording scheitert, keine Aufnahme", res["isError"] and recordings() == 0)
        res = client.call("start_recording", {"title": "Zweiter Versuch"})
        rec_aid2 = res["structuredContent"]["approval_id"]
        user_decides(bridge, rec_aid2, True)
        res = client.call("start_recording", {"title": "Zweiter Versuch", "approval_id": rec_aid2})
        record(
            "selbst freigegeben nimmt die Sandbox nichts auf (kein Recorder), Fehler mit Grund",
            res["isError"] and "nicht verfügbar" in res["content"][0]["text"] and recordings() == 0,
            res["content"][0]["text"][:90],
        )
        res = client.call("stop_recording", {})
        record("stop_recording (aus) ist kein Werkzeug fuer diesen Zugang", res["isError"])

        # Ein falscher Zugang
        bad = Client(exe, dict(bridge.env, LVA_AGENT_TOKEN=new_token()))
        bad_list = bad.request("tools/list", {})
        record("falsches Token: keine schreibenden Werkzeuge in der Liste", [t["name"] for t in bad_list["result"]["tools"]] == names_read_only())
        res = bad.call("create_meeting", {"title": "x"})
        record("falsches Token: Aufruf scheitert mit Hinweis", res["isError"] and "Zugang" in res["content"][0]["text"])
        bad.close()

        # Audit-Dump: alle Aktionen, Aufbewahrung gedeckelt
        dump = bridge.audit_dump()
        caps = dump["by_capability"]
        needed = ["transcribe.file", "tts.render", "youtube.add", "media.fetch", "meeting.create", "recording.start"]
        record("audit-dump: alle Aktionen sind drin", all(caps.get(c, 0) >= 1 for c in needed), json.dumps(caps))
        outcomes = dump["by_outcome"]
        record("audit-dump: ok, denied und pending vorhanden", all(outcomes.get(o, 0) >= 1 for o in ("ok", "denied", "pending")), json.dumps(outcomes))
        record("audit-dump: Aufbewahrung gedeckelt", dump["retention"]["max_rows"] == 20000 and dump["retention"]["rows"] <= 20000 and dump["limit"] <= 20000)
        agent_rows = [e for e in dump["entries"] if e["caller"] == "agent_external"]
        record("audit-dump: Aktionen des Agenten mit Zugang im Ziel", any("Smoke-Agent" in (e["target"] or "") for e in agent_rows))
        raw_dump = json.dumps(dump)
        record("kein Token im Audit-Dump", token not in raw_dump)

        # App beenden: ctl Exit 2, Proxy ohne schreibende Werkzeuge
        code, _, err = bridge.ctl("status")
        exit_is("ctl status mit App", code, 0)
        bridge.stop()
        code, _, err = bridge.ctl("status")
        exit_is("ctl status nach dem Beenden der App", code, 2, err[:60])
        res = client.call("create_meeting", {"title": "x"})
        record("Proxy ohne App: Hinweis statt Absturz", res["isError"] and "läuft nicht" in res["content"][0]["text"], res["content"][0]["text"][:70])

        # Das Token steht nirgends im Klartext.
        db_bytes = (bridge.meetings / "meetings.db").read_bytes()
        record("kein Token im Klartext in der Datenbank", token.encode() not in db_bytes)
        code = client.close()
        time.sleep(0.5)  # der Leser-Thread fuer stderr braucht einen Augenblick nach dem Ende
        record("Proxy endet mit Exit 0, wenn stdin schliesst", code == 0, str(code))
        stderr_text = client.stderr.decode("utf-8", "replace") if client.stderr else ""
        record("kein Token auf stderr/stdout des Proxys", token not in stderr_text and all(token not in line for line in client.raw))
        client = None
        return 0
    except SystemExit as e:
        print(f"FEHLER: {e}", file=sys.stderr)
        return 1
    finally:
        if client is not None:
            try:
                client.proc.kill()
            except Exception:  # noqa: BLE001
                pass
        bridge.stop()
        stub.close()
        failed = [r for r in results if not r["ok"]]
        print(f"\n{len(results) - len(failed)} von {len(results)} Faellen wie erwartet.")
        if out:
            Path(out).write_text(json.dumps(results, ensure_ascii=False, indent=2), encoding="utf-8")
        if keep:
            print(f"Sandbox bleibt: {sandbox}")
        else:
            shutil.rmtree(sandbox, ignore_errors=True)


# ---------------------------------------------------------------------------
# --workflows: Automationen ueber die Agentenbruecke (B8, AK10)
# ---------------------------------------------------------------------------

WF_ARMED = "WF-SMOKE-ARMED"
WF_DRAFT = "WF-SMOKE-DRAFT"


def wf_definition(name: str, steps: list, variables: "dict | None" = None) -> str:
    definition = {"schema": "lva-workflow@1", "name": name, "trigger": {"type": "manual"}, "steps": steps}
    if variables:
        definition["variables"] = variables
    return json.dumps(definition, ensure_ascii=False)


def seed_workflows(bridge: "Bridge") -> "dict[str, tuple[str, str]]":
    """Zugaenge, Rechte und zwei Ablaeufe, wie die Oberflaeche sie anlegt (hier direkt in der
    Sandbox-Datenbank). Rueckgabe: Name -> (Token, Kennung des Zugangs)."""
    now = int(time.time() * 1000)
    conn = bridge.db()
    clients: "dict[str, tuple[str, str]]" = {}
    with conn:
        conn.execute(
            "INSERT OR IGNORE INTO integrations (id, kind, label, enabled, direction, config_json, created_at, updated_at)"
            " VALUES ('agents', 'agent', 'Externe Agenten', 1, 'both', '{}', ?1, ?1)",
            (now,),
        )
        for cap in ("workflow.read", "workflow.run"):
            conn.execute(
                "INSERT OR REPLACE INTO integration_grants (integration_id, capability, caller, mode)"
                " VALUES ('agents', ?1, 'agent_external', 'allow')",
                (cap,),
            )
        for key, label, tools in (
            ("full", "Smoke-Agent", {"list_workflows": "allow", "run_workflow": "allow", "get_run": "allow"}),
            ("readonly", "Nur-Lesen-Agent", {"list_workflows": "allow", "get_run": "allow"}),
            ("ask", "Frage-Agent", {"list_workflows": "allow", "run_workflow": "ask", "get_run": "allow"}),
            ("none", "Rechtloser Agent", {}),
        ):
            token = new_token()
            client_id = f"C-WF-{key.upper()}"
            conn.execute(
                "INSERT INTO agent_clients (id, label, integration_id, token_hash, created_at)"
                " VALUES (?1, ?2, 'agents', ?3, ?4)",
                (client_id, label, hashlib.sha256(token.encode()).hexdigest(), now),
            )
            for tool, mode in tools.items():
                conn.execute(
                    "INSERT INTO agent_tool_grants (client_id, tool, mode) VALUES (?1, ?2, ?3)",
                    (client_id, tool, mode),
                )
            clients[key] = (token, client_id)
        # Ein scharfer Ablauf (wartet nur: ohne Aussenwirkung) und ein Entwurf (nicht scharf).
        conn.execute(
            "INSERT INTO workflows (id, name, enabled, dry_run, schema_version, definition_json, created_at, updated_at)"
            " VALUES (?1, 'Smoke Wartezeit', 1, 0, 1, ?2, ?3, ?3)",
            (
                WF_ARMED,
                wf_definition(
                    "Smoke Wartezeit",
                    [{"id": "warten", "action": "wait", "params": {"minutes": 1}}],
                    {"thema": {"type": "string", "default": "Budget"}},
                ),
                now,
            ),
        )
        conn.execute(
            "INSERT INTO workflows (id, name, enabled, dry_run, schema_version, definition_json, created_at, updated_at)"
            " VALUES (?1, 'Smoke Entwurf', 1, 1, 1, ?2, ?3, ?3)",
            (
                WF_DRAFT,
                wf_definition(
                    "Smoke Entwurf",
                    [
                        {"id": "hinweis", "action": "notify.local", "params": {"title": "Hallo aus dem Entwurf"}},
                        {"id": "pause", "action": "wait", "params": {"minutes": 5}},
                    ],
                ),
                now,
            ),
        )
    conn.close()
    return clients


def run_workflows(exe: Path, keep: bool, out: "str | None") -> int:
    sandbox = Path(tempfile.mkdtemp(prefix="lva-mcp-wf-smoke-"))
    bridge = Bridge(exe, sandbox, f"lva-mcp-smoke-{uuid.uuid4().hex[:12]}", "http://127.0.0.1:9/oembed")
    results: list[dict] = []
    clients_open: "list[Client]" = []

    def record(name: str, ok: bool, note: str = "") -> None:
        results.append({"case": name, "ok": ok, "note": note})
        print(f"{'OK  ' if ok else 'FAIL'} {name}  {note}")
        if not ok:
            raise SystemExit(f"Abweichung: {name} {note}")

    def exit_is(name: str, got: int, expected: int, note: str = "") -> None:
        record(f"{name} (Exit {got}, erwartet {expected})", got == expected, note)

    def proxy(token: str) -> "Client":
        c = Client(exe, dict(bridge.env, LVA_AGENT_TOKEN=token))
        clients_open.append(c)
        c.request(
            "initialize",
            {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "mcp_smoke", "version": "1"}},
        )
        return c

    def runs() -> "list[sqlite3.Row]":
        conn = bridge.db()
        rows = conn.execute("SELECT * FROM workflow_runs ORDER BY created_at").fetchall()
        conn.close()
        return rows

    try:
        print(f"Sandbox: {sandbox}")
        bridge.start()
        seeded = seed_workflows(bridge)
        token, client_id = seeded["full"]
        (bridge.appdata / "settings_store.json").write_text(
            json.dumps({"settings": {"meeting_mcp_enabled": True, "meeting_mcp_include_transcript": True}}),
            encoding="utf-8",
        )

        # --- MCP-Proxy: die Werkzeuge sind da -----------------------------------------------
        client = proxy(token)
        names = [t["name"] for t in client.request("tools/list", {})["result"]["tools"]]
        record(
            "tools/list: list_workflows, run_workflow, get_run",
            all(n in names for n in ("list_workflows", "run_workflow", "get_run")),
            ",".join(names),
        )
        ro = proxy(seeded["readonly"][0])
        ro_names = [t["name"] for t in ro.request("tools/list", {})["result"]["tools"]]
        record(
            "tools/list ohne Starten-Recht: run_workflow fehlt, Lesen bleibt",
            "run_workflow" not in ro_names and "list_workflows" in ro_names and "get_run" in ro_names,
        )

        # --- list_workflows ------------------------------------------------------------------
        res = client.call("list_workflows", {})
        record("list_workflows ohne Fehler", not res["isError"], res["content"][0]["text"][:80])
        listing = res["structuredContent"]
        by_id = {w["id"]: w for w in listing["workflows"]}
        record("list_workflows: beide Ablaeufe, count/total", listing["count"] == 2 and listing["total"] == 2 and WF_ARMED in by_id and WF_DRAFT in by_id)
        armed, draft = by_id[WF_ARMED], by_id[WF_DRAFT]
        record(
            "list_workflows: Felder (name, enabled, armed, trigger, steps, variables, last_run)",
            all(k in armed for k in ("name", "enabled", "armed", "can_run_live", "trigger", "steps", "variables", "open_runs", "last_run"))
            and armed["name"] == "Smoke Wartezeit"
            and armed["enabled"] is True
            and armed["armed"] is True
            and draft["armed"] is False
            and armed["can_run_live"] is True
            and draft["can_run_live"] is False
            and armed["trigger"] == "manual"
            and armed["steps"] == 1
            and armed["variables"] == [{"name": "thema", "type": "string", "required": False}]
            and armed["last_run"] is None,
            json.dumps(armed, ensure_ascii=False)[:160],
        )
        record("list_workflows liefert keine Definition", "definition" not in json.dumps(listing) and "params" not in json.dumps(listing))

        # --- run_workflow: Trockenlauf ---------------------------------------------------------
        res = client.call("run_workflow", {"workflow_id": WF_DRAFT})
        record("run_workflow (Standard) -> Trockenlauf", not res["isError"] and res["structuredContent"]["dry_run"] is True, res["content"][0]["text"][:100])
        started = res["structuredContent"]
        run_id = started["run_id"]
        record(
            "run_workflow: erwartete Felder",
            all(k in started for k in ("run_id", "workflow_id", "workflow_name", "dry_run", "created", "origin", "state", "hint"))
            and started["origin"] == "agent"
            and started["created"] is True
            and started["workflow_id"] == WF_DRAFT,
        )
        rows = runs()
        record(
            "Lauf in der Datenbank: Herkunft agent, Trockenlauf, Zugang im Schluessel",
            len(rows) == 1 and rows[0]["origin"] == "agent" and rows[0]["dry_run"] == 1 and client_id in rows[0]["trigger_key"],
            str([dict(r)["trigger_key"] for r in rows]),
        )

        # --- get_run: wartet, bis der Lauf fertig ist ---------------------------------------------
        deadline = time.time() + 30
        log: dict = {}
        while time.time() < deadline:
            res = client.call("get_run", {"run_id": run_id})
            log = res["structuredContent"]
            if log["finished"]:
                break
            time.sleep(0.3)
        record("get_run: der Trockenlauf wird fertig (done)", log.get("finished") is True and log["run"]["state"] == "done", json.dumps(log.get("run"), ensure_ascii=False)[:160])
        run = log["run"]
        record(
            "get_run: erwartete Felder des Laufs",
            all(k in run for k in ("id", "workflow_id", "workflow_name", "origin", "state", "dry_run", "created_at", "started_at", "ended_at", "error", "error_code"))
            and run["origin"] == "agent"
            and run["dry_run"] is True
            and run["workflow_name"] == "Smoke Entwurf",
        )
        steps = log["steps"]
        record(
            "get_run: zwei Schritte mit Aktion, Titel, Zustand und Plan-Ausgabe",
            log["steps_total"] == 2
            and [s["step_id"] for s in steps] == ["hinweis", "pause"]
            and steps[0]["action"] == "notify.local"
            and steps[0]["output"]["status"] == "planned"
            and all(k in steps[0] for k in ("title", "state", "attempt", "error", "started_at", "ended_at", "output")),
            json.dumps(steps[0], ensure_ascii=False)[:160],
        )
        flat = json.dumps(log, ensure_ascii=False)
        record("get_run: keine eingesetzten Parameter, Definition oder Ausloeserdaten", "params" not in flat and "definition" not in flat and "context" not in flat)
        res = client.call("get_run", {"run_id": "01GIBTESNICHT"})
        record("get_run: unbekannter Lauf -> Fehler im Klartext", res["isError"] and "gibt es nicht" in res["content"][0]["text"])

        # --- scharf nur bei scharfem Ablauf ---------------------------------------------------------
        before = len(runs())
        res = client.call("run_workflow", {"workflow_id": WF_DRAFT, "live": True})
        record(
            "run_workflow live=true bei nicht scharfem Ablauf -> Fehler, KEIN Lauf",
            res["isError"] and "nicht scharf" in res["content"][0]["text"] and len(runs()) == before,
            res["content"][0]["text"][:100],
        )
        res = client.call("run_workflow", {"workflow_id": WF_ARMED, "live": True, "vars": {"thema": "Smoke"}, "request_id": "smoke-live-1"})
        record("run_workflow live=true bei scharfem Ablauf -> echter Lauf", not res["isError"] and res["structuredContent"]["dry_run"] is False, res["content"][0]["text"][:100])
        live_id = res["structuredContent"]["run_id"]
        again = client.call("run_workflow", {"workflow_id": WF_ARMED, "live": True, "vars": {"thema": "Smoke"}, "request_id": "smoke-live-1"})
        record(
            "dieselbe request_id -> derselbe Lauf (created=false), kein zweiter",
            again["structuredContent"]["run_id"] == live_id and again["structuredContent"]["created"] is False and len(runs()) == before + 1,
        )
        deadline = time.time() + 30
        while time.time() < deadline:
            live = client.call("get_run", {"run_id": live_id})["structuredContent"]
            if live["run"]["wait_reason"]:
                break
            time.sleep(0.3)
        record(
            "get_run: der echte Lauf wartet im Schritt „Warten“ (ohne Aussenwirkung)",
            live["run"]["dry_run"] is False
            and live["run"]["state"] in ("queued", "running")
            and live["run"]["wait_reason"] == "defer"
            and live["steps"][0]["step_id"] == "warten"
            and live["steps"][0]["state"] == "waiting"
            and "Wartet" in (live["steps"][0]["error"] or ""),
            json.dumps(live["run"], ensure_ascii=False)[:160],
        )
        res = client.call("run_workflow", {"workflow_id": WF_ARMED, "vars": {"unbekannt": 1}})
        record("run_workflow: nicht deklarierte Variable abgewiesen", res["isError"] and "unbekannt" in res["content"][0]["text"])
        res = client.call("run_workflow", {"workflow_id": WF_ARMED, "dry_run": False})
        record("run_workflow: unbekanntes Argument abgewiesen", res["isError"])

        # --- ctl --------------------------------------------------------------------------------------
        code, text, _ = bridge.ctl("workflow", "list", "--json", token=token)
        exit_is("ctl workflow list --json", code, 0)
        record("ctl workflow list: Ablaeufe", json.loads(text)["result"]["result"]["count"] == 2)
        code, text, _ = bridge.ctl("workflow", "run", WF_DRAFT, "--json", token=token)
        exit_is("ctl workflow run <id> --json (Trockenlauf)", code, 0)
        ctl_run = json.loads(text)["result"]["result"]
        record("ctl workflow run: dry_run und Lauf-Kennung", ctl_run["dry_run"] is True and bool(ctl_run["run_id"]))
        code, text, _ = bridge.ctl("workflow", "get", ctl_run["run_id"], "--json", token=token)
        exit_is("ctl workflow get <run> --json", code, 0)
        record("ctl workflow get: Lauf mit Herkunft agent", json.loads(text)["result"]["result"]["run"]["origin"] == "agent")
        code, text, _ = bridge.ctl("workflow", "run", WF_DRAFT, "--live", "--json", token=token)
        exit_is("ctl workflow run --live bei nicht scharfem Ablauf (Fehler, nicht Erfolg)", code, 1, json.loads(text)["error"]["code"])
        code, _, _ = bridge.ctl("workflow", "run", WF_ARMED, "--live", "--vars", '{"thema":"ctl"}', "--json", token=token)
        exit_is("ctl workflow run --live bei scharfem Ablauf", code, 0)

        # Ohne Recht: Exit 3; ohne Token: Exit 4
        code, text, _ = bridge.ctl("workflow", "run", WF_DRAFT, "--json", token=seeded["readonly"][0])
        exit_is("ctl workflow run ohne Recht (nur Lesen)", code, 3, json.loads(text)["error"]["code"])
        code, text, _ = bridge.ctl("workflow", "list", "--json", token=seeded["none"][0])
        exit_is("ctl workflow list ohne jedes Recht", code, 3, json.loads(text)["error"]["code"])
        code, _, _ = bridge.ctl("workflow", "list", "--json")
        exit_is("ctl workflow list ohne Token", code, 4)
        record("ohne Recht entstand kein weiterer Lauf", all(r["origin"] == "agent" for r in runs()) and len(runs()) == before + 3, str(len(runs())))

        # fragen: Exit 5, nach der Freigabe der Nutzer laeuft derselbe Aufruf; die Freigabe gilt an die Argumente gebunden.
        ask_token = seeded["ask"][0]
        n_before = len(runs())
        code, text, _ = bridge.ctl("workflow", "run", WF_ARMED, "--live", "--json", token=ask_token)
        exit_is("ctl workflow run (fragen) wartet auf den Nutzer", code, 5)
        aid = json.loads(text)["result"]["approval_id"]
        record("ohne Freigabe kein Lauf", len(runs()) == n_before)
        user_decides(bridge, aid, True)
        code, text, _ = bridge.ctl("workflow", "run", WF_ARMED, "--approval", aid, "--json", token=ask_token)
        exit_is("freigegeben fuer live, aufgerufen OHNE live -> abgelehnt (Argumente gebunden)", code, 1, json.loads(text)["error"]["code"])
        code, _, _ = bridge.ctl("workflow", "run", WF_ARMED, "--live", "--approval", aid, "--json", token=ask_token)
        exit_is("freigegeben, mit denselben Argumenten", code, 0)
        record("genau ein Lauf kam dazu", len(runs()) == n_before + 1)

        # --- Audit ---------------------------------------------------------------------------------------
        dump = bridge.audit_dump()
        caps = dump["by_capability"]
        record("audit-dump: workflow.read und workflow.run stehen drin", caps.get("workflow.read", 0) >= 1 and caps.get("workflow.run", 0) >= 1, json.dumps(caps))
        agent_rows = [e for e in dump["entries"] if e["caller"] == "agent_external" and e["capability"] in ("workflow.read", "workflow.run")]
        record("audit-dump: Aktionen mit Zugang im Ziel", any("Smoke-Agent" in (e["target"] or "") for e in agent_rows))
        record("audit-dump: Verweigerungen (ohne Recht) stehen als denied drin", any(e["outcome"] == "denied" for e in dump["entries"]))
        raw = json.dumps(dump)
        record("kein Token im Audit-Dump", not any(t in raw for t, _ in seeded.values()))
        return 0
    except SystemExit as e:
        print(f"FEHLER: {e}", file=sys.stderr)
        return 1
    finally:
        for c in clients_open:
            try:
                c.proc.kill()
            except Exception:  # noqa: BLE001
                pass
        bridge.stop()
        failed = [r for r in results if not r["ok"]]
        print(f"\n{len(results) - len(failed)} von {len(results)} Faellen wie erwartet.")
        if out:
            Path(out).write_text(json.dumps(results, ensure_ascii=False, indent=2), encoding="utf-8")
        if keep:
            print(f"Sandbox bleibt: {sandbox}")
        else:
            shutil.rmtree(sandbox, ignore_errors=True)



def names_read_only() -> "list[str]":
    return ["list_meetings", "search_meetings", "get_meeting", "get_transcript", "get_provenance"]


def main() -> int:
    argv = sys.argv[1:]
    out = argv[argv.index("--out") + 1] if "--out" in argv and argv.index("--out") + 1 < len(argv) else None
    if "--workflows" in argv:
        positional = [a for i, a in enumerate(argv) if not a.startswith("--") and not (i > 0 and argv[i - 1] == "--out")]
        default = Path(__file__).resolve().parents[1] / "apps" / "local-voice" / "src-tauri" / "target" / "debug" / "local-voice-ai.exe"
        exe = Path(positional[0]).resolve() if positional else default
        if not exe.is_file():
            print(f"EXE nicht gefunden: {exe}", file=sys.stderr)
            return 2
        return run_workflows(exe, "--keep" in argv, out)
    if "--write" in argv:
        positional = [a for i, a in enumerate(argv) if not a.startswith("--") and not (i > 0 and argv[i - 1] == "--out")]
        default = Path(__file__).resolve().parents[1] / "apps" / "local-voice" / "src-tauri" / "target" / "debug" / "local-voice-ai.exe"
        exe = Path(positional[0]).resolve() if positional else default
        if not exe.is_file():
            print(f"EXE nicht gefunden: {exe}", file=sys.stderr)
            return 2
        return run_write(exe, "--keep" in argv, out)
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
            names == ["list_meetings", "search_meetings", "get_meeting", "get_transcript", "get_provenance"],
            f"Werkzeuge: {names}",
        )
        check(all(t["annotations"]["readOnlyHint"] is True for t in tools), "ein Werkzeug ist nicht als lesend markiert")

        # -- Einstellung aus (Standard: Datei fehlt) --------------------------
        for name, arguments in [
            ("list_meetings", {}),
            ("search_meetings", {"query": WORD}),
            ("get_meeting", {"id": "x"}),
            ("get_transcript", {"id": "x"}),
            ("get_provenance", {"id": "x"}),
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
