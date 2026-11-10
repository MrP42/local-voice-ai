#!/usr/bin/env python3
"""Lokaler Nachbau des wai-portal-Hubs (VoiceSyncController + VoiceBlobController) für Simulator-Tests.
Nur Testzwecke, alles im Speicher, ein Benutzer (id 7).
Dokumentenablage (lva#441, D5a-3): Pull je Sammlung (`collection=`, eigener Cursor über `server_seq`), `blob_refs` am
Objekt (nur `ablage_datei`), `reserve`, Push-Prüfungen wie der Nachbau des Desktops (`ablage_tests.rs`) und ein Seed."""
import base64, hashlib, json, re, sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import mock_pairing

OBJECTS = {}   # (coll, id) -> dict
SEQ = [0]
BLOBS = {}     # blob -> {"chunks": {idx: bytes}, "complete": bool, "count": int}
LOG = []
PAIRING = [None]  # mock_pairing.Desktop, wenn mit --pair gestartet
RESERVED = {}     # (blob, device_id) -> Anzahl Reservierungen (24 h je Gerät; der Nachbau merkt sich nur, dass es sie gab)
MAX_PAGE = [None] # --max-page N: Seitengröße des Pulls begrenzen (zum Prüfen von Mehrseiten-Pulls)
EXPIRE = set()    # POST /_mock/expire: der nächste Pull mit Cursor > 0 je Sammlung (""= global) liefert genau einmal 410
SEED_IDS = [None] # Kennungen des Ablage-Seeds (für /_mock/ablage-phase2)
ABLAGE = ("ablage_ordner", "ablage_datei")
GRAB_MAX = 16384  # Grabstein-Siegel höchstens 16 384 Bytes (wai-portal#23)

class H(BaseHTTPRequestHandler):
    def _json(self, code, obj):
        body = json.dumps(obj).encode()
        self.send_response(code); self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)

    def _body(self):
        n = int(self.headers.get("Content-Length") or 0)
        return json.loads(self.rfile.read(n) or b"{}")

    def log_message(self, *a): pass

    def do_GET(self):
        LOG.append(("GET", self.path))
        p = self.path
        if PAIRING[0] and re.match(r"^/api/v1/voice/pair/[^/]+/result$", p): return self._json(*PAIRING[0].result())
        if p.startswith("/api/v1/voice/sync/pull"):
            since = int(re.search(r"since=(\d+)", p).group(1)); limit = int(re.search(r"limit=(\d+)", p).group(1))
            if MAX_PAGE[0]: limit = min(limit, MAX_PAGE[0])
            coll = re.search(r"collection=([a-z_]+)", p)   # Ablage: eigener Pull je Sammlung mit eigenem Cursor
            schluessel = coll.group(1) if coll else ""
            if since > 0 and schluessel in EXPIRE or since > SEQ[0]:   # abgelaufen, oder ein Cursor aus der Zukunft (Hub zurückgesetzt)
                EXPIRE.discard(schluessel)
                return self._json(410, {"error": {"code": "cursor_expired", "message": "Cursor abgelaufen."}})
            todo = sorted([o for o in OBJECTS.values() if o["server_seq"] > since and (not coll or o["collection"] == coll.group(1))],
                          key=lambda o: o["server_seq"])
            rows = todo[:limit]
            cursor = rows[-1]["server_seq"] if rows else since
            return self._json(200, {"objects": rows, "cursor": cursor, "more": len(todo) > len(rows)})
        m = re.match(r"^/api/v1/voice/blobs/([0-9a-f]{64})/chunks/(\d+)$", p)
        if m:
            b = BLOBS.get(m.group(1))
            if not b or not b["complete"] or int(m.group(2)) not in b["chunks"]: return self._json(404, {"error": {"code": "not_found", "message": ""}})
            data = b["chunks"][int(m.group(2))]
            return self._json(200, {"data": base64.b64encode(data).decode(), "sha256": hashlib.sha256(data).hexdigest()})
        m = re.match(r"^/api/v1/voice/blobs/([0-9a-f]{64})$", p)
        if m:
            b = BLOBS.get(m.group(1))
            if not b: return self._json(404, {"error": {"code": "not_found", "message": "Blob unbekannt."}})
            return self._json(200, {"blob_id": m.group(1), "complete": b["complete"], "chunk_count": b["count"],
                                    "chunks": [{"idx": i, "size": len(c)} for i, c in sorted(b["chunks"].items())]})
        if p == "/api/v1/voice/blobs":
            return self._json(200, {"used": sum(len(c) for b in BLOBS.values() for c in b["chunks"].values()), "quota": 1 << 35, "blobs": len(BLOBS)})
        self._json(404, {"error": {"code": "not_found", "message": "Route"}})

    def do_POST(self):
        LOG.append(("POST", self.path))
        p = self.path; body = self._body()
        if PAIRING[0] and p == "/api/v1/voice/pair/claim": return self._json(*PAIRING[0].claim(body))
        if PAIRING[0] and p.endswith("/reveal"): return self._json(*PAIRING[0].reveal(body))
        if PAIRING[0] and p.endswith("/confirm"): return self._json(*PAIRING[0].confirm(body))
        if p == "/api/v1/voice/auth/login":
            if body.get("password") != "geheim": return self._json(401, {"error": {"code": "invalid_credentials", "message": "E-Mail oder Passwort stimmen nicht."}})
            return self._json(200, {"token": "wai_pat_test", "user": {"id": 7, "email": body["email"]}, "server_time": "now"})
        if p == "/api/v1/voice/auth/logout": return self._json(200, {})
        if p == "/api/v1/voice/sync/push":
            out = []
            for o in body["objects"]:
                k = (o["collection"], o["object_id"]); cur = OBJECTS.get(k)
                rev = cur["revision"] if cur else 0
                refs = o.get("blob_refs")
                if o["collection"] in ABLAGE:   # Prüfungen wie der Nachbau des Desktops (`ablage_tests.rs`)
                    reason, missing = None, []
                    if not o.get("payload"): reason = "payload"
                    elif refs is not None and (o["collection"] != "ablage_datei" or o["deleted"]): reason = "blob_refs_not_allowed"
                    elif o["deleted"] and len(base64.b64decode(o["payload"])) > GRAB_MAX: reason = "payload_size"
                    elif cur and cur["deleted"] and not o["deleted"]: reason = "tombstoned"
                    elif refs:
                        missing = [r for r in refs if not (BLOBS.get(r) or {}).get("complete")]
                        if missing: reason = "blob_missing"
                    if reason:
                        out.append({"collection": k[0], "object_id": k[1], "outcome": "rejected", "reason": reason, "missing": missing}); continue
                if o["base_revision"] != rev:
                    out.append({"collection": k[0], "object_id": k[1], "outcome": "superseded", "revision": rev, "current": cur}); continue
                SEQ[0] += 1
                OBJECTS[k] = {"collection": k[0], "object_id": k[1], "revision": rev + 1, "payload": o["payload"], "key_id": o["key_id"],
                              "deleted": o["deleted"], "device_id": body["device_id"], "server_seq": SEQ[0]}
                if refs is not None: OBJECTS[k]["blob_refs"] = refs
                out.append({"collection": k[0], "object_id": k[1], "outcome": "accepted", "revision": rev + 1})
            return self._json(200, {"results": out, "high_water_seq": SEQ[0]})
        m = re.match(r"^/api/v1/voice/blobs/([0-9a-f]{64})/reserve$", p)
        if m:   # Reservierung: der Nachbau merkt sie nur (kein Ablauf, keine bedingte Löschung)
            RESERVED[(m.group(1), body.get("device_id", ""))] = RESERVED.get((m.group(1), body.get("device_id", "")), 0) + 1
            return self._json(200, {})
        if p == "/_mock/expire":   # der Cursor jeder Sammlung läuft einmal ab; ein Neustart von 0 geht danach durch
            EXPIRE.update({"ablage_ordner", "ablage_datei", ""})
            return self._json(200, {"expire": sorted(EXPIRE)})
        if p == "/_mock/ablage-phase2" and SEED_IDS[0]:
            import mock_ablage_seed
            mock_ablage_seed.phase2(PAIRING[0].k, PAIRING[0].user_id, OBJECTS, BLOBS, SEQ, SEED_IDS[0])
            return self._json(200, {"seq": SEQ[0]})
        m = re.match(r"^/api/v1/voice/blobs/([0-9a-f]{64})/chunks/(\d+)$", p)
        if m:
            data = base64.b64decode(body["data"])
            if hashlib.sha256(data).hexdigest() != body["sha256"]: return self._json(422, {"error": {"code": "checksum", "message": ""}})
            b = BLOBS.setdefault(m.group(1), {"chunks": {}, "complete": False, "count": 0})
            if b["complete"]: return self._json(409, {"error": {"code": "immutable", "message": ""}})
            b["chunks"][int(m.group(2))] = data
            return self._json(200, {"outcome": "stored", "size": len(data), "sha256": body["sha256"]})
        m = re.match(r"^/api/v1/voice/blobs/([0-9a-f]{64})/commit$", p)
        if m:
            b = BLOBS.get(m.group(1)); n = body["chunks"]
            if not b: return self._json(404, {"error": {"code": "not_found", "message": ""}})
            missing = [i for i in range(n) if i not in b["chunks"]]
            if missing: return self._json(409, {"outcome": "incomplete", "missing": missing})
            b["complete"] = True; b["count"] = n
            return self._json(200, {"outcome": "complete"})
        self._json(404, {"error": {"code": "not_found", "message": "Route"}})

if __name__ == "__main__":
    srv = ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), H)
    if "--max-page" in sys.argv: MAX_PAGE[0] = int(sys.argv[sys.argv.index("--max-page") + 1])
    if "--pair" in sys.argv:
        key = bytes.fromhex(sys.argv[sys.argv.index("--key-hex") + 1]) if "--key-hex" in sys.argv else None   # gleicher Schlüssel nach Neustart
        PAIRING[0] = mock_pairing.Desktop(f"http://127.0.0.1:{sys.argv[1]}", key=key)
        print(PAIRING[0].uri(), flush=True)
        if "--seed-book" in sys.argv:  # synthetisches Buch mit Sprachfassungen, nur zusammen mit --pair
            import mock_library_seed
            fixture = sys.argv[sys.argv.index("--seed-book") + 1]
            print("Buch abgelegt:", mock_library_seed.seed(fixture, PAIRING[0].k, PAIRING[0].user_id, OBJECTS, BLOBS, SEQ), flush=True)
        if "--seed-ablage" in sys.argv or "--seed-ablage-suche" in sys.argv:  # synthetische Ablage (Ordner, Dateien, Konflikt, Papierkorb, Grabsteine, Fehlerfälle)
            import mock_ablage_seed
            SEED_IDS[0] = mock_ablage_seed.seed(PAIRING[0].k, PAIRING[0].user_id, OBJECTS, BLOBS, SEQ)
            if "--seed-ablage-suche" in sys.argv:  # D5a-4: zusätzliche Dateien mit mehreren Abschnitten für Suche und Sprung zum Anker
                mock_ablage_seed.seed_suche(PAIRING[0].k, PAIRING[0].user_id, OBJECTS, BLOBS, SEQ, SEED_IDS[0])
            print("Ablage abgelegt:", json.dumps(SEED_IDS[0], sort_keys=True), flush=True)
    try: srv.serve_forever()
    finally:
        print(json.dumps({"objects": len(OBJECTS), "blobs": {k[:8]: {"complete": v["complete"], "chunks": len(v["chunks"])} for k, v in BLOBS.items()},
                          "requests": len(LOG)}), flush=True)

# Simulator-Probe (Debug-Build):
#   python3 apps/apple/scripts/mock_voice_hub.py 8765 &
#   xcrun simctl launch <sim> de.localvoice.prototype --library-tab --sync-test http://127.0.0.1:8765 [--library-listen]
# Kopplung: mit --pair startet der Nachbau auch die PC-Seite und gibt den Link aus; dann
#   xcrun simctl launch <sim> de.localvoice.prototype --library-tab --pair-auto
#   xcrun simctl openurl <sim> '<Link>'
# Buch mit Sprachfassungen (lva#344, Fixture DE+EN, Schema 2, stille WAV-Teile):
#   python3 apps/apple/scripts/mock_voice_hub.py 8765 --pair --seed-book apps/apple/scripts/fixtures/sprachfassungen-buch.json
# Dokumentenablage (lva#441, D5a-3): synthetische Ordner und Dateien samt Fehlerfällen, nur zusammen mit --pair:
#   python3 apps/apple/scripts/mock_voice_hub.py 8765 --pair --seed-ablage [--max-page 3] [--key-hex <64 hex>]
#   xcrun simctl launch <sim> de.localvoice.prototype --library-tab --pair-auto --pair-url '<Link>' --ablage-test [--ablage-test-original]
#   (Bericht: <Container>/Documents/ablage-testbericht.json; Stand 2 mit gültigem und gefälschtem Grabstein:
#    curl -X POST http://127.0.0.1:8765/_mock/ablage-phase2, dann erneut starten; 410: curl -X POST .../_mock/expire)
#   D5a-4: --seed-ablage-suche legt zusätzlich Dateien mit mehreren Abschnitten, Überschriftenpfaden und Textoriginalen ab
#   (Seite „Ablage“, Suche mit Sprung zum Anker; Start der App mit --ablage-tab).
#   Selbsttest der Seed-Krypto gegen die Rust-Vektoren: python3 apps/apple/scripts/mock_ablage_seed.py docs/superpowers/plans/ablage-testvektoren.json
# Passwort des Test-Kontos: geheim. Der Nachbau prüft weder Token noch Kontingent.
