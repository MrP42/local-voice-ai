"""Legt für Simulator-Sichtprüfungen ein synthetisches Buch mit Sprachfassungen (lva#344) im Nachbau-Hub ab.
Der Hub speichert nur Chiffrat; hier wird wie am PC versiegelt, mit dem Schlüssel der Kopplung (`--pair`).
Nur synthetische Inhalte: Text aus der Fixture, Hörbuchteile sind stille WAV-Dateien."""
import base64, hashlib, io, json, os, wave
import mock_pairing as mp

CHUNK = 3 * 1024 * 1024
DEVICE = "seed-pc"


def silent_wav(seconds):
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(8000)
        w.writeframes(b"\0\0" * 8000 * seconds)
    return buf.getvalue()


def put_blob(blobs, key, user_id, plain, **extra):
    """Wie `LibrarySyncFormat`: Inhaltsadresse, Stücke versiegelt mit AAD `<nutzer>\\x1fblob\\x1f<blob>\\x1f<index>`."""
    blob = hashlib.sha256(key + b"lv-blob-v1" + hashlib.sha256(plain).digest()).hexdigest()
    parts = [plain[i:i + CHUNK] for i in range(0, len(plain), CHUNK)] or [b""]
    chunks = {}
    for i, p in enumerate(parts):
        nonce = os.urandom(24)
        aad = f"{user_id}\x1fblob\x1f{blob}\x1f{i}".encode()
        chunks[i] = nonce + mp.xchacha_seal(key, nonce, p, aad)
    blobs[blob] = {"chunks": chunks, "complete": True, "count": len(parts)}
    return {"blob": blob, "size": len(plain), "chunks": len(parts), **extra}


def seed(fixture_path, key, user_id, objects, blobs, seq):
    fx = json.load(open(fixture_path, encoding="utf-8"))
    book = fx["book"]
    primary = book["primary"]["edition"]
    files = {"audio": [], "editions": {}}
    by_id = {e["id"]: e for e in book["editions"]}
    for ed, text in fx["texts"].items():
        ref = put_blob(blobs, key, user_id, text.encode("utf-8"))
        if ed == primary: files["text"] = ref
        else: files["editions"].setdefault(ed, {})["text"] = ref
    for ed, spec in fx["audio"].items():
        tracks, refs = [], []
        for p in spec["parts"]:
            refs.append(put_blob(blobs, key, user_id, silent_wav(p["seconds"]), file=p["file"]))
            tracks.append({"file": p["file"], "title": p["title"], "duration_ms": p["seconds"] * 1000})
        total = sum(t["duration_ms"] for t in tracks)
        version = {"file": tracks[0]["file"], "duration_ms": total, "narrator": spec.get("narrator"), "tracks": tracks}
        by_id[ed]["audio"] = version
        if ed == primary:
            book["audio"] = version; files["audio"] = refs
        else:
            files["editions"].setdefault(ed, {})["audio"] = refs
    plain = dict(book, files=files, updated=book["added_ms"] + 4000)
    oid = book["id"]
    aad = f"{user_id}\x1fbook\x1f{oid}".encode()
    nonce = os.urandom(24)
    sealed = base64.b64encode(nonce + mp.xchacha_seal(key, nonce, json.dumps(plain, sort_keys=True).encode(), aad)).decode()
    seq[0] += 1
    objects[("book", oid)] = {"collection": "book", "object_id": oid, "revision": 1, "payload": sealed,
                              "key_id": mp.sha(key)[:4].hex(), "deleted": False, "device_id": DEVICE, "server_seq": seq[0]}
    return oid
