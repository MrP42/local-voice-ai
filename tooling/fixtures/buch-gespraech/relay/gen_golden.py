#!/usr/bin/env python3
"""Erzeugt golden.json fuer das Abo-Relay (Spec 11.7 R2/R3, lva#320).

Unabhaengig von Rust und Swift gerechnet (hashlib/hmac). Aufruf aus dem
Repo-Wurzelordner: python3 tooling/fixtures/buch-gespraech/relay/gen_golden.py
"""
import base64
import hashlib
import hmac
import json
import pathlib

key = bytes(range(32))
key_id = hashlib.sha256(b"lv-booktalk-relay-key-id-v1" + key).digest()[:8].hex()


def b64u(b):
    return base64.urlsafe_b64encode(b).decode().rstrip("=")


def canon(obj):
    # Wie die Swift-Fassung: Schluessel alphabetisch, kein Leerraum, UTF-8 unmaskiert.
    return json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def mac(s):
    return hmac.new(key, s.encode("utf-8"), hashlib.sha256).hexdigest()


def sha(s):
    return hashlib.sha256(s.encode("utf-8")).hexdigest()


req = dict(
    v=1,
    request_id="0123456789abcdef0123456789abcdef",
    device_id="iphone-0123456789abcdef0123456789ab",
    key_id=key_id,
    function="book_talk",
    model_id="sonnet",
    system="Antworte kurz und nur aus den Auszügen.",
    prompt='Frage: Warum gibt es "zwei Väter"?\nAuszug [Q1]: Tab\there, Rückstrich \\ und Zeilenende\r\n',
    max_tokens=400,
    created_at=1793900000,
    expires_at=1793900090,
)
req_c = canon(
    dict(
        created_at=req["created_at"],
        device_id=req["device_id"],
        expires_at=req["expires_at"],
        function=req["function"],
        key_id=key_id,
        max_tokens=req["max_tokens"],
        model_id=req["model_id"],
        prompt_sha256=sha(req["prompt"]),
        request_id=req["request_id"],
        system_sha256=sha(req["system"]),
        v=1,
    )
)
req["mac"] = mac(req_c)


def resp(seq, model_tuple, text, done, error):
    base = dict(
        v=1,
        request_id=req["request_id"],
        device_id=req["device_id"],
        seq=seq,
        model_tuple=model_tuple,
        text=text,
        done=done,
    )
    c = canon(
        dict(
            device_id=base["device_id"],
            done=done,
            error=error or "",
            model_tuple=model_tuple,
            request_id=base["request_id"],
            seq=seq,
            text_sha256=sha(text),
            v=1,
        )
    )
    out = dict(base)
    if error:
        out["error"] = error
    out["mac"] = mac(c)
    return dict(response=out, canonical=c, mac=out["mac"])


responses = [
    resp(1, "pending", "Die Väter ", False, None),
    resp(2, "claude-sonnet-5-5", "Die Väter sind der leibliche und der Adoptivvater. [Q1]", True, None),
    resp(1, "unknown", "", True, "timeout"),
]
enroll = dict(
    v=1,
    key_id=key_id,
    k=b64u(key),
    desktop_device_id="desk0123456789abcdef0123456789abcd",
    desktop_name="Büro-PC",
    device_id=req["device_id"],
    model_ids=["sonnet", "gpt-6.1-sol"],
    expires_at=1801676000,
)
golden = dict(
    _doc=(
        "Gemeinsame Golden-Datei fuer das Abo-Relay (Spec 11.7 R2/R3, lva#320). "
        "MAC = hex(HMAC-SHA-256(key, canonical als UTF-8)). Zeiten: Unix-Sekunden. "
        "Erzeugt mit gen_golden.py (unabhaengig von Rust und Swift)."
    ),
    key_hex=key.hex(),
    key_b64url=b64u(key),
    key_id=key_id,
    request=dict(
        request=req,
        canonical=req_c,
        mac=req["mac"],
        system_sha256=sha(req["system"]),
        prompt_sha256=sha(req["prompt"]),
    ),
    responses=responses,
    enrollment=dict(json=enroll, code="lvrelay1:" + canon(enroll)),
)
out = pathlib.Path(__file__).with_name("golden.json")
out.write_text(json.dumps(golden, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
print(out)
