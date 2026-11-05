#!/usr/bin/env python3
"""Lokaler Nachbau des wai-portal-Hubs (VoiceSyncController + VoiceBlobController) für Simulator-Tests.
Nur Testzwecke, alles im Speicher, ein Benutzer (id 7)."""
import base64, hashlib, json, re, sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import mock_pairing

OBJECTS = {}   # (coll, id) -> dict
SEQ = [0]
BLOBS = {}     # blob -> {"chunks": {idx: bytes}, "complete": bool, "count": int}
LOG = []
PAIRING = [None]  # mock_pairing.Desktop, wenn mit --pair gestartet

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
            rows = sorted([o for o in OBJECTS.values() if o["server_seq"] > since], key=lambda o: o["server_seq"])[:limit]
            cursor = rows[-1]["server_seq"] if rows else since
            more = any(o["server_seq"] > cursor for o in OBJECTS.values())
            return self._json(200, {"objects": rows, "cursor": cursor, "more": more})
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
                if o["base_revision"] != rev:
                    out.append({"collection": k[0], "object_id": k[1], "outcome": "superseded", "revision": rev, "current": cur}); continue
                SEQ[0] += 1
                OBJECTS[k] = {"collection": k[0], "object_id": k[1], "revision": rev + 1, "payload": o["payload"], "key_id": o["key_id"],
                              "deleted": o["deleted"], "device_id": body["device_id"], "server_seq": SEQ[0]}
                out.append({"collection": k[0], "object_id": k[1], "outcome": "accepted", "revision": rev + 1})
            return self._json(200, {"results": out, "high_water_seq": SEQ[0]})
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
    if "--pair" in sys.argv:
        PAIRING[0] = mock_pairing.Desktop(f"http://127.0.0.1:{sys.argv[1]}")
        print(PAIRING[0].uri(), flush=True)
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
# Passwort des Test-Kontos: geheim. Der Nachbau prüft weder Token noch Kontingent.
