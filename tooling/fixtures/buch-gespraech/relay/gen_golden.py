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


def quote(s):
    # Wie Rusts json_quote und Swifts BookTalkCanonical.quote: nur " \\ \n \r \t mit
    # Kuerzel, alle uebrigen Zeichen unter 0x20 als \\u00xx (klein), der Rest roh
    # als UTF-8 (auch U+2028, U+007F, Emoji). Pythons json.dumps kennt zusaetzlich
    # \\b und \\f als Kuerzel und waere damit anders.
    out = ['"']
    for c in s:
        if c == '"':
            out.append('\\"')
        elif c == "\\":
            out.append("\\\\")
        elif c == "\n":
            out.append("\\n")
        elif c == "\r":
            out.append("\\r")
        elif c == "\t":
            out.append("\\t")
        elif ord(c) < 0x20:
            out.append("\\u%04x" % ord(c))
        else:
            out.append(c)
    out.append('"')
    return "".join(out)


def canon(obj):
    # Schluessel alphabetisch, kein Leerraum, Zeichenketten wie `quote`.
    if isinstance(obj, bool):
        return "true" if obj else "false"
    if isinstance(obj, int):
        return str(obj)
    if isinstance(obj, str):
        return quote(obj)
    if isinstance(obj, list):
        return "[" + ",".join(canon(x) for x in obj) + "]"
    if isinstance(obj, dict):
        return "{" + ",".join(quote(k) + ":" + canon(obj[k]) for k in sorted(obj)) + "}"
    raise TypeError(type(obj))


def mac(s):
    return hmac.new(key, s.encode("utf-8"), hashlib.sha256).hexdigest()


def sha(s):
    return hashlib.sha256(s.encode("utf-8")).hexdigest()


def make_request(model_id="sonnet", **over):
    r = dict(
        v=1,
        request_id="0123456789abcdef0123456789abcdef",
        device_id="iphone-0123456789abcdef0123456789ab",
        key_id=key_id,
        function="book_talk",
        model_id=model_id,
        system="Antworte kurz und nur aus den Auszügen.",
        prompt='Frage: Warum gibt es "zwei Väter"?\nAuszug [Q1]: Tab\there, Rückstrich \\ und Zeilenende\r\n',
        max_tokens=400,
        created_at=1793900000,
        expires_at=1793900090,
    )
    r.update(over)
    c = canon(
        dict(
            created_at=r["created_at"],
            device_id=r["device_id"],
            expires_at=r["expires_at"],
            function=r["function"],
            key_id=r["key_id"],
            max_tokens=r["max_tokens"],
            model_id=r["model_id"],
            prompt_sha256=sha(r["prompt"]),
            request_id=r["request_id"],
            system_sha256=sha(r["system"]),
            v=1,
        )
    )
    r["mac"] = mac(c)
    return r, c


req, req_c = make_request()
# Steuer- und Nicht-ASCII-Zeichen in model_id: \b, \f, \u0001, Umlaut, Emoji, U+2028.
SPECIAL = "mod\b\f\u0001-ü😀\u2028\u007f\"/"
req_special, req_special_c = make_request(model_id=SPECIAL, request_id="fedcba9876543210fedcba9876543210")


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
    # Ab hier (Runde 5b): Lebenszeichen beim Annehmen, Abweisung mit Fehlercode,
    # Steuer-/Nicht-ASCII-Zeichen in model_tuple und error.
    resp(1, "pending", "", False, None),
    resp(1, "unknown", "", True, "clock"),
    resp(2, SPECIAL, "Text mit Zeilenumbruch\nund Tab\t", True, "err\b\f\u0001-ü😀\u2028"),
]
enroll = dict(
    v=1,
    key_id=key_id,
    k=b64u(key),
    desktop_device_id="desk0123456789abcdef0123456789abcd",
    desktop_name="Büro-PC",
    device_id=req["device_id"],
    model_ids=["sonnet", "gpt-6.1-sol"],
    # Ablauf des Codes (10 Minuten) und der Freigabe (90 Tage), Unix-Sekunden.
    expires_at=1793900600,
    grant_expires_at=1801676000,
)

# Modellliste je Geraet, mit dem Relay-Schluessel dieses Geraets signiert (mac ausserhalb
# der Signatur, v mitsigniert, Eintraege nach id sortiert).
models = dict(
    v=1,
    desktop_device_id="desk0123456789abcdef0123456789abcd",
    device_id=req["device_id"],
    issued_at=1793900000,
    models=sorted(
        [
            dict(id="sonnet", label="Claude Sonnet 5.5 (Büro)", provider="claude_cli", context_tokens=200000),
            dict(id="gpt-6.1-sol", label="Codex \"Sol\"\t😀", provider="codex_cli", context_tokens=128000),
        ],
        key=lambda m: m["id"].encode("utf-8"),
    ),
)
models_c = canon(models)
models["mac"] = mac(models_c)  # nur in der Nutzlast, nicht in canonical
golden = dict(
    _doc=(
        "Gemeinsame Golden-Datei fuer das Abo-Relay (Spec 11.7 R2/R3, lva#320). "
        "MAC = hex(HMAC-SHA-256(key, canonical als UTF-8)). Zeiten: Unix-Sekunden. "
       "Zeichenketten in canonical: nur \\\" \\\\ \\n \\r \\t "
        "mit Kuerzel, sonst Zeichen unter 0x20 als \\u00xx, Rest roh UTF-8. "
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
    request_special=dict(
        request=req_special,
        canonical=req_special_c,
        mac=req_special["mac"],
        system_sha256=sha(req_special["system"]),
        prompt_sha256=sha(req_special["prompt"]),
    ),
    responses=responses,
    model_list=dict(list=models, canonical=models_c, mac=models["mac"]),
    enrollment=dict(json=enroll, code="lvrelay1:" + canon(enroll)),
)
out = pathlib.Path(__file__).with_name("golden.json")
out.write_text(json.dumps(golden, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
print(out)
