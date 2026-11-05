#!/usr/bin/env python3
"""Desktop-Seite der Geräte-Kopplung für den Mock-Hub (nur Simulator-Tests).

Eigene, unabhängige Umsetzung von X25519 und XChaCha20-Poly1305 in reinem Python,
damit die Probe nichts installieren muss und die Swift-Seite gegen zweite Quelle
läuft. `python3 mock_pairing.py <vektoren.json>` prüft sie gegen die Testvektoren.
Spec: docs/superpowers/specs/2026-11-05-geraete-kopplung-qr.md, Abschnitt 4.3."""
import base64, hashlib, hmac, json, os, struct, sys, urllib.parse

P = 2**255 - 19

def x25519(k, u):
    k = bytearray(k); k[0] &= 248; k[31] &= 127; k[31] |= 64
    k = int.from_bytes(k, "little"); u = int.from_bytes(u, "little") & ((1 << 255) - 1)
    x2, z2, x3, z3, swap = 1, 0, u, 1, 0
    for t in range(254, -1, -1):
        b = (k >> t) & 1
        if swap ^ b: x2, x3, z2, z3 = x3, x2, z3, z2
        swap = b
        a, c = (x2 + z2) % P, (x2 - z2) % P
        aa, bb = a * a % P, c * c % P
        e = (aa - bb) % P
        d, f = (x3 + z3) % P, (x3 - z3) % P
        da, cb = f * a % P, d * c % P
        x3, z3 = (da + cb) ** 2 % P, u * (da - cb) ** 2 % P
        x2, z2 = aa * bb % P, e * (aa + 121665 * e) % P
    if swap: x2, x3, z2, z3 = x3, x2, z3, z2
    return (x2 * pow(z2, P - 2, P) % P).to_bytes(32, "little")

def public(k): return x25519(k, (9).to_bytes(32, "little"))

M = 0xFFFFFFFF
def _rounds(s):
    def r(a, b, c, d):
        s[a] = (s[a] + s[b]) & M; s[d] ^= s[a]; s[d] = ((s[d] << 16) | (s[d] >> 16)) & M
        s[c] = (s[c] + s[d]) & M; s[b] ^= s[c]; s[b] = ((s[b] << 12) | (s[b] >> 20)) & M
        s[a] = (s[a] + s[b]) & M; s[d] ^= s[a]; s[d] = ((s[d] << 8) | (s[d] >> 24)) & M
        s[c] = (s[c] + s[d]) & M; s[b] ^= s[c]; s[b] = ((s[b] << 7) | (s[b] >> 25)) & M
    for _ in range(10):
        r(0, 4, 8, 12); r(1, 5, 9, 13); r(2, 6, 10, 14); r(3, 7, 11, 15)
        r(0, 5, 10, 15); r(1, 6, 11, 12); r(2, 7, 8, 13); r(3, 4, 9, 14)

SIGMA = list(struct.unpack("<4I", b"expand 32-byte k"))
def hchacha(key, n16):
    s = SIGMA + list(struct.unpack("<8I", key)) + list(struct.unpack("<4I", n16))
    _rounds(s)
    return struct.pack("<8I", *(s[0:4] + s[12:16]))

def chacha_block(key, counter, n12):
    init = SIGMA + list(struct.unpack("<8I", key)) + [counter] + list(struct.unpack("<3I", n12))
    s = list(init); _rounds(s)
    return struct.pack("<16I", *[(a + b) & M for a, b in zip(s, init)])

def poly1305(key, msg):
    r = int.from_bytes(key[:16], "little") & 0x0ffffffc0ffffffc0ffffffc0fffffff
    acc, p = 0, (1 << 130) - 5
    for i in range(0, len(msg), 16):
        block = msg[i:i + 16]
        acc = (acc + int.from_bytes(block + b"\x01", "little")) * r % p
    return ((acc + int.from_bytes(key[16:], "little")) & ((1 << 128) - 1)).to_bytes(16, "little")

def xchacha_seal(key, nonce, plain, aad):
    sub, n12 = hchacha(key, nonce[:16]), b"\0" * 4 + nonce[16:]
    otk = chacha_block(sub, 0, n12)[:32]
    stream = b"".join(chacha_block(sub, i + 1, n12) for i in range(len(plain) // 64 + 1))
    ct = bytes(a ^ b for a, b in zip(plain, stream))
    pad = lambda b: b + b"\0" * (-len(b) % 16)
    tag = poly1305(otk, pad(aad) + pad(ct) + struct.pack("<QQ", len(aad), len(ct)))
    return ct + tag

def b64u(b): return base64.urlsafe_b64encode(b).rstrip(b"=").decode()
def unb64u(s): return base64.urlsafe_b64decode(s + "=" * (-len(s) % 4))
def sha(*parts): return hashlib.sha256(b"".join(parts)).digest()
def mac(key, *parts): return hmac.new(key, b"".join(parts), hashlib.sha256).digest()

def hkdf(ikm, salt, info):
    return hmac.new(hmac.new(salt, ikm, hashlib.sha256).digest(), info + b"\x01", hashlib.sha256).digest()

def commitment(pid, p_pk, n_p, name):
    raw = name.encode()
    return sha(b"LV-PAIR-COMMIT-v2", pid.encode(), p_pk, n_p, struct.pack(">H", len(raw)), raw)

def session_key(z, q, d_pk, p_pk, pid): return hkdf(z, q + d_pk + p_pk, b"LV-PAIR-KEY-v2" + pid.encode())
def seal_aad(pid, d_pk, p_pk): return b"LV-PAIR-SEAL-v2" + pid.encode() + d_pk + p_pk
def sas(pid, d_pk, p_pk, n_p, n_d):
    return "%06d" % (struct.unpack(">I", sha(b"LV-PAIR-SAS-v2", pid.encode(), d_pk, p_pk, n_p, n_d)[:4])[0] % 10**6)

class Desktop:
    """Eine Kopplung im Speicher. `approve_after`: so viele Abfragen nach dem Reveal bestätigt der „PC“."""
    def __init__(self, hub, key=None, email="test@example.com", user_id=7, approve_after=4):
        self.hub, self.email, self.user_id, self.approve_after = hub, email, user_id, approve_after
        self.id = b64u(os.urandom(16)); self.secret = b64u(os.urandom(32))
        self.q, self.d_sk, self.n_d = os.urandom(32), os.urandom(32), os.urandom(32)
        self.d_pk = public(self.d_sk); self.k = key or os.urandom(32)
        self.status, self.polls, self.confirmed, self.sas = "pending", 0, False, None

    def uri(self):
        h = urllib.parse.quote(self.hub, safe="")
        return f"localvoice://pair?v=2&h={h}&p={self.id}&s={self.secret}&q={b64u(self.q)}&k={b64u(self.d_pk)}"

    def claim(self, b):
        if self.status != "pending" or b.get("pairing_id") != self.id or b.get("secret") != self.secret: return 410, {}
        c = unb64u(b["phone_commit"])
        if mac(self.q, b"LV-PAIR-CLAIM-v2", self.id.encode(), c) != unb64u(b["phone_mac"]): return 400, {}
        self.c, self.name, self.status = c, b["device_name"], "challenged"
        return 200, {"poll_token": "poll", "expires_at": 0}

    def reveal(self, b):
        if self.status != "challenged": return 410, {}
        self.p_pk, self.n_p = unb64u(b["phone_pub"]), unb64u(b["phone_nonce"])
        if commitment(self.id, self.p_pk, self.n_p, self.name) != self.c: return 409, {"error": {"code": "commitment_mismatch"}}
        self.sas = sas(self.id, self.d_pk, self.p_pk, self.n_p, self.n_d)
        print("Vergleichscode am PC:", self.sas, "Name:", self.name, flush=True)
        self.status = "revealed"
        return 200, {"status": "revealed"}

    def result(self):
        if self.status == "challenged": return 200, {"status": "challenge", "desktop_nonce": b64u(self.n_d)}
        if self.status == "revealed":
            self.polls += 1
            if self.polls <= self.approve_after: return 202, {"status": "waiting"}
            self.status = "done"
            return 200, {"status": "ok", "token": "wai_pat_paired", "sealed": self.sealed(os.urandom(24)),
                         "user": {"id": self.user_id, "email": self.email}}
        return (202, {"status": "waiting"}) if self.status == "claimed" else (410, {})

    def s(self):
        return session_key(x25519(self.d_sk, self.p_pk), self.q, self.d_pk, self.p_pk, self.id)

    def sealed(self, nonce):
        plain = json.dumps({"v": 1, "k": b64u(self.k), "email": self.email, "user_id": self.user_id,
                            "key_id": sha(self.k)[:4].hex()}, separators=(",", ":")).encode()
        return base64.b64encode(nonce + xchacha_seal(self.s(), nonce, plain, seal_aad(self.id, self.d_pk, self.p_pk))).decode()

    def confirm(self, b):
        ok = mac(self.s(), b"LV-PAIR-CONFIRM-v2", self.id.encode()) == unb64u(b.get("confirm", ""))
        print("Bestätigung vom iPhone:", "gültig" if ok else "FALSCH", flush=True)
        self.confirmed = ok
        return (200, {"status": "confirmed"}) if ok else (410, {})

def selftest(path):
    v = json.load(open(path)); i, w = v["eingaben"], v["ergebnisse"]
    d_sk, p_sk, q, n_p, n_d = (unb64u(i[k]) for k in ("d_sk", "p_sk", "q", "n_p", "n_d"))
    d_pk, p_pk, pid = public(d_sk), public(p_sk), i["pair_id"]
    assert b64u(d_pk) == w["d_pk"] and b64u(p_pk) == w["p_pk"]
    assert b64u(commitment(pid, p_pk, n_p, i["device_name"])) == w["phone_commit"]
    assert sas(pid, d_pk, p_pk, n_p, n_d) == w["sas"]
    z = x25519(d_sk, p_pk); assert b64u(z) == w["z"]
    s = session_key(z, q, d_pk, p_pk, pid); assert b64u(s) == w["s"]
    nonce = unb64u(i["seal_nonce"])
    sealed = xchacha_seal(s, nonce, w["plaintext"].encode(), seal_aad(pid, d_pk, p_pk))
    assert base64.b64encode(nonce + sealed).decode() == w["sealed"]
    print("Vektoren stimmen")

if __name__ == "__main__":
    selftest(sys.argv[1])
