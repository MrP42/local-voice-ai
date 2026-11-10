"""Legt für Simulator-Proben synthetische Dokumentenablage-Objekte (lva#441, D5a-3) im Nachbau-Hub ab.

Der Hub speichert nur Chiffrat; hier wird wie am PC versiegelt (`apps/local-voice/src-tauri/src/sync/ablage.rs`,
`sync/crypto.rs`), mit dem Schlüssel der Kopplung (`--pair`). Warum Python statt eines Swift-Werkzeugs: Der Nachbau läuft
ohnehin in Python (derselbe Prozess, kein Bauschritt, `mock_library_seed.py` ist das Muster, XChaCha20-Poly1305 liegt in
`mock_pairing.py`), und die Krypto ist nicht „selbst geglaubt“: `--selftest` rechnet jede Funktion gegen die gemeinsame
Fixture `docs/superpowers/plans/ablage-testvektoren.json` nach, die der Desktop aus den echten Rust-Funktionen erzeugt
(AAD, versiegelte Grabsteine samt Nonce, Hub-Kennungen, `textrev`, Textfassung, kanonische Nutzlast, `standard_id`).
Das Gegenstück in Swift (`AblageKrypto`, `AblageVectorTests`) prüft dieselbe Datei. Fehlerhafte Objekte im Seed beweisen
zusätzlich im Simulator, dass das iPhone fail-closed liest: stimmt hier etwas nicht, zeigt der Lauf es als `angehalten`.

Nur synthetische Inhalte (Namen, Texte, Originale). Der Seed hat zwei Stände:
  seed(...)    Stand 1 beim Start des Nachbaus (`--seed-ablage`)
  phase2(...)  Stand 2 über `POST /_mock/ablage-phase2`: ein gültiger und ein gefälschter Grabstein
"""
import base64, hashlib, json, os, struct, sys
import mock_pairing as mp
import mock_library_seed as mls

DEVICE = "seed-pc"
GERAET_A = "g_01HZX4Q8E5K2V7M3N9P6R1S0TA"   # der „PC“
GERAET_B = "g_01HZX4Q8E5K2V7M3N9P6R1S0TB"   # ein zweites Gerät (Konflikt)
B32 = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
EXTRAKTOR = ("lva-text", "1")
ZEIT = 1_762_000_000_000


# --- Grundfunktionen (Gegenstücke zu `crypto.rs`, `ablage.rs`, `collect.rs`) ---------------------------------------

def sha_hex(b): return hashlib.sha256(b).hexdigest()
def key_id(key): return hashlib.sha256(key).digest()[:4].hex()

def canonical(obj):
    """`collect::canonical`: Schlüssel rekursiv nach Bytes (Python ordnet nach Codepunkten = UTF-8-Bytes), kompakt,
    `null` bleibt, Nicht-ASCII roh."""
    return json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False)

def ulid(b16):
    """16 Bytes als ULID-Text (Crockford-Base32, 26 Zeichen)."""
    n = int.from_bytes(b16, "big")
    return "".join(B32[(n >> (5 * i)) & 31] for i in range(25, -1, -1))

def standard_id(key):
    return "ord_" + ulid(hashlib.sha256(key + b"lv-ablage-allgemein").digest()[:16])

def feste_id(praefix, label):
    return praefix + ulid(hashlib.sha256(b"lva-seed-" + label.encode()).digest()[:16])

def aad_ablage(user_id, collection, object_id, hub_refs, kid, deleted):
    return (f"{user_id}\x1f{collection}\x1f{object_id}\x1fblob_refs\x1f{','.join(hub_refs)}"
            f"\x1fkey_id\x1f{kid}\x1fdeleted\x1f{1 if deleted else 0}").encode()

def seal(key, aad, plain, nonce=None):
    nonce = nonce or os.urandom(24)
    return base64.b64encode(nonce + mp.xchacha_seal(key, nonce, plain, aad)).decode()

def seal_deterministisch(key, aad, plain):
    """`seal_deterministisch`: Nonce = SHA-256("lv-ablage-siegel-nonce" ‖ 0x1F ‖ key ‖ u64_be(len aad) ‖ aad ‖ klartext)[0..24]."""
    nonce = hashlib.sha256(b"lv-ablage-siegel-nonce" + b"\x1f" + key + struct.pack(">Q", len(aad)) + aad + plain).digest()[:24]
    return seal(key, aad, plain, nonce)

def grab_nutzlast(key, user_id, collection, object_id):
    klartext = canonical({"collection": collection, "deleted": True, "id": object_id}).encode()
    return seal_deterministisch(key, aad_ablage(user_id, collection, object_id, [], key_id(key), True), klartext)

def hub_blob(key, sha256_hex):
    """`hub_blob`: SHA256(key ‖ "lv-blob-v1" ‖ sha[32 Rohbytes])."""
    return hashlib.sha256(key + b"lv-blob-v1" + bytes.fromhex(sha256_hex)).hexdigest()

def textrev(original_sha256, extraktor, version, marken=1):
    return hashlib.sha256(original_sha256.encode() + b"\0" + extraktor.encode() + b"\0" + version.encode() + b"\0" + str(marken).encode()).hexdigest()

def reg(wert, zaehler=1, geraet=GERAET_A):
    return {"wert": wert, "zaehler": zaehler, "geraet": geraet}


# --- Datensätze ---------------------------------------------------------------------------------------------------

def ordner_nutzlast(oid, name, eltern=None, klasse="erbt", nur_geraet=False, korb=None):
    return {"id": oid, "eltern": reg(eltern), "name": reg(name), "datenklasse": reg(klasse), "nur_geraet": reg(nur_geraet),
            "zielordner_ki": reg(False), "papierkorb": reg(korb)}

def textfassung_bytes(rev, original, abschnitte):
    """`serde_json` von `Textfassung`: Felder in Strukturreihenfolge (nicht sortiert), `pfad` fehlt, wenn leer."""
    return json.dumps({"marken_version": 1, "textrev": rev, "original_sha256": original, "extraktor": EXTRAKTOR[0],
                       "extraktor_version": EXTRAKTOR[1], "abschnitte": [{"anker": a, "text": t} for a, t in abschnitte]},
                      separators=(",", ":"), ensure_ascii=False).encode("utf-8")

class Welt:
    """Sammelt Blobs und Objekte des Seeds."""
    def __init__(self, key, user_id, objects, blobs, seq):
        self.key, self.user_id, self.objects, self.blobs, self.seq = key, user_id, objects, blobs, seq

    def blob(self, plain):
        return mls.put_blob(self.blobs, self.key, self.user_id, plain)["blob"]

    def fassung(self, fas, vorgaenger, original, typ="pdf", zeit=ZEIT, geraet=GERAET_A, text=None):
        """`text`: Liste von (Anker, Text) für eine Textfassung `ok`, `None` = noch kein Text (eigener Import)."""
        h = sha_hex(original)
        self.blob(original)
        texte = []
        if text is not None:
            rev = textrev(h, *EXTRAKTOR)
            tb = textfassung_bytes(rev, h, text)
            self.blob(tb)
            texte.append({"textrev": rev, "blob": sha_hex(tb), "size": len(tb), "extraktor": EXTRAKTOR[0],
                          "extraktor_version": EXTRAKTOR[1], "marken_version": 1, "stand": "ok"})
        return {"fas": fas, "vorgaenger": vorgaenger, "blob": h, "size": len(original), "original_sha256": h, "typ": typ,
                "geraet": geraet, "zeit": zeit, "texte": texte}

    def datei_nutzlast(self, did, ordner, name, fassungen, klasse="public", korb=None, wahlen=None, extra=None):
        inhalt = sorted({f["blob"] for f in fassungen} | {t["blob"] for f in fassungen for t in f["texte"] if t["blob"]})
        d = {"id": did, "ordner": reg(ordner), "name": reg(name), "klasse_min": reg(klasse), "nur_geraet_min": False,
             "fassungen": sorted(fassungen, key=lambda f: f["fas"]), "wahlen": wahlen or [], "entfernt": [], "blob_refs": inhalt,
             "papierkorb": reg(korb)}
        d.update(extra or {})
        return d

    def hub_refs(self, datei):
        return sorted({hub_blob(self.key, h) for h in datei["blob_refs"]})

    def stelle(self, collection, oid, payload_b64, kid=None, revision=1, deleted=False, refs=None):
        self.seq[0] += 1
        o = {"collection": collection, "object_id": oid, "revision": revision, "payload": payload_b64,
             "key_id": kid or key_id(self.key), "deleted": deleted, "device_id": DEVICE, "server_seq": self.seq[0]}
        if refs is not None: o["blob_refs"] = refs
        self.objects[(collection, oid)] = o
        return o

    def ordner(self, o, revision=1, key=None, kid=None):
        k = key or self.key
        aad = aad_ablage(self.user_id, "ablage_ordner", o["id"], [], kid or key_id(k), False)
        return self.stelle("ablage_ordner", o["id"], seal(k, aad, canonical(o).encode()), kid=kid or key_id(k), revision=revision)

    def datei(self, d, revision=1, key=None, kid=None, refs=None, nutzlast=None, refs_im_siegel=None):
        k = key or self.key
        hub = refs if refs is not None else self.hub_refs(d)
        aad = aad_ablage(self.user_id, "ablage_datei", d["id"], refs_im_siegel if refs_im_siegel is not None else hub, kid or key_id(k), False)
        klar = (nutzlast if nutzlast is not None else canonical(d)).encode()
        return self.stelle("ablage_datei", d["id"], seal(k, aad, klar), kid=kid or key_id(k), revision=revision, refs=hub)

    def grab(self, collection, oid, revision):
        return self.stelle(collection, oid, grab_nutzlast(self.key, self.user_id, collection, oid), revision=revision, deleted=True)


# --- Seed ---------------------------------------------------------------------------------------------------------

def seed(key, user_id, objects, blobs, seq):
    """Stand 1. Liefert die Kennungen (für Prüfskripte)."""
    w = Welt(key, user_id, objects, blobs, seq)
    ids = {}
    allgemein = standard_id(key)
    o_vertraege = feste_id("ord_", "vertraege")
    o_miete = feste_id("ord_", "mietvertraege")
    o_geheim = feste_id("ord_", "geheim")
    o_archiv = feste_id("ord_", "archiv")
    o_alt = feste_id("ord_", "alt")
    ids.update(allgemein=allgemein, vertraege=o_vertraege, miete=o_miete, geheim=o_geheim, archiv=o_archiv, alt=o_alt)
    w.ordner(ordner_nutzlast(allgemein, "Allgemein"))
    w.ordner(ordner_nutzlast(o_vertraege, "Verträge", klasse="internal"))
    w.ordner(ordner_nutzlast(o_miete, "Mietverträge", eltern=o_vertraege))
    w.ordner(ordner_nutzlast(o_geheim, "Vertraulich", klasse="confidential"))
    w.ordner(ordner_nutzlast(o_archiv, "Archiv", korb=ZEIT + 5))                  # im Papierkorb
    w.ordner(ordner_nutzlast(o_alt, "Alt"))
    w.grab("ablage_ordner", o_alt, 2)                                                 # Ordner begraben, mit Siegel

    # D1: eine Fassung mit Text -> synchronisiert (Text wird vorab geladen)
    f1 = w.fassung(feste_id("fas_", "d1"), None, b"Synthetischer Mietvertrag: Die Miete betraegt 850 Euro.", text=[
        ("seite=1", "Die Miete beträgt 850 Euro."), ("seite=2", "Kündigung zum Quartal.")])
    ids["d1"] = feste_id("dat_", "d1")
    w.datei(w.datei_nutzlast(ids["d1"], o_miete, "Mietvertrag.pdf", [f1]))
    # D2: zwei Köpfe (Konflikt), beide mit Text
    wurzel = w.fassung(feste_id("fas_", "d2a"), None, b"Angebot Fassung A", typ="docx", zeit=ZEIT, text=[("absatz=1", "Angebot A")])
    kopf1 = w.fassung(feste_id("fas_", "d2b"), wurzel["fas"], b"Angebot Fassung B vom PC", typ="docx", zeit=ZEIT + 1000, text=[("absatz=1", "Angebot B")])
    kopf2 = w.fassung(feste_id("fas_", "d2c"), wurzel["fas"], b"Angebot Fassung C vom Laptop", typ="docx", zeit=ZEIT + 2000, geraet=GERAET_B, text=[("absatz=1", "Angebot C")])
    ids["d2"] = feste_id("dat_", "d2")
    w.datei(w.datei_nutzlast(ids["d2"], o_vertraege, "Angebot.docx", [wurzel, kopf1, kopf2]))
    # D3: eigener Import ohne Text -> „Text wird am PC erstellt“
    f3 = w.fassung(feste_id("fas_", "d3"), None, b"Eine Notiz vom iPhone", typ="txt", text=None)
    ids["d3"] = feste_id("dat_", "d3")
    w.datei(w.datei_nutzlast(ids["d3"], allgemein, "Notiz.txt", [f3]))
    # D4: im Papierkorb
    f4 = w.fassung(feste_id("fas_", "d4"), None, b"altes Bild", typ="png", text=None)
    ids["d4"] = feste_id("dat_", "d4")
    w.datei(w.datei_nutzlast(ids["d4"], allgemein, "Altes Foto.png", [f4], korb=ZEIT + 9))
    # D5: unter vertraulichem Ordner -> nur Gerät, nie im Index
    f5 = w.fassung(feste_id("fas_", "d5"), None, b"Geheimes Dokument", text=[("seite=1", "Geheimer Inhalt")])
    ids["d5"] = feste_id("dat_", "d5")
    w.datei(w.datei_nutzlast(ids["d5"], o_geheim, "Geheim.pdf", [f5]))
    # D6: vom PC gelöscht (nur der Grabstein liegt am Hub)
    ids["d6"] = feste_id("dat_", "d6")
    w.grab("ablage_datei", ids["d6"], 2)
    # D7, D8: lebend in Stand 1; in Stand 2 gefälschter bzw. gültiger Grabstein
    f7 = w.fassung(feste_id("fas_", "d7"), None, b"Dokument, das nicht gefaelscht geloescht werden darf", text=[("seite=1", "Bleibt erhalten")])
    ids["d7"] = feste_id("dat_", "d7")
    w.datei(w.datei_nutzlast(ids["d7"], allgemein, "Bleibt.pdf", [f7]))
    f8 = w.fassung(feste_id("fas_", "d8"), None, b"Dokument, das spaeter geloescht wird", text=[("seite=1", "Wird gelöscht")])
    ids["d8"] = feste_id("dat_", "d8")
    w.datei(w.datei_nutzlast(ids["d8"], allgemein, "Wird gelöscht.pdf", [f8]))

    # Fehlerfälle (fail-closed): jedes Objekt muss auf dem iPhone `angehalten` sein und nichts verändern.
    fe = w.fassung(feste_id("fas_", "e1"), None, b"fremder Schluessel", text=None)
    other = hashlib.sha256(key + b"anderes-passwort").digest()
    ids["e1"] = feste_id("dat_", "e1")
    w.datei(w.datei_nutzlast(ids["e1"], allgemein, "FremderSchluessel.pdf", [fe]), key=other)       # fremde key_id und Schlüssel
    ids["e2"] = feste_id("dat_", "e2")
    w.datei(w.datei_nutzlast(ids["e2"], allgemein, "NeuesSchema.pdf", [w.fassung(feste_id("fas_", "e2"), None, b"schema 2", text=None)], extra={"v": 2}))
    ids["e3"] = feste_id("dat_", "e3")
    kaputt = w.datei(w.datei_nutzlast(ids["e3"], allgemein, "KaputtesSiegel.pdf", [w.fassung(feste_id("fas_", "e3"), None, b"kaputt", text=None)]))
    roh = bytearray(base64.b64decode(kaputt["payload"])); roh[len(roh) // 2] ^= 1
    kaputt["payload"] = base64.b64encode(bytes(roh)).decode()
    ids["e4"] = feste_id("dat_", "e4")
    w.stelle("ablage_datei", ids["e4"], "", revision=2, deleted=True)                                # Grabstein ohne Siegel
    ids["e5"] = feste_id("dat_", "e5")
    d5 = w.datei_nutzlast(ids["e5"], allgemein, "FalscheRefs.pdf", [w.fassung(feste_id("fas_", "e5"), None, b"refs", text=None)])
    w.datei(d5, refs=["ab" * 32])                                                                    # blob_refs passen nicht zur Nutzlast
    return ids


def phase2(key, user_id, objects, blobs, seq, ids):
    """Stand 2: D8 wird mit gültigem Siegel gelöscht, D7 erhält einen gefälschten Grabstein (anderer Schlüssel in der AAD)."""
    w = Welt(key, user_id, objects, blobs, seq)
    w.grab("ablage_datei", ids["d8"], 2)
    other = hashlib.sha256(key + b"anderes-passwort").digest()
    aad = aad_ablage(user_id, "ablage_datei", ids["d7"], [], key_id(key), True)
    klar = canonical({"collection": "ablage_datei", "deleted": True, "id": ids["d7"]}).encode()
    w.stelle("ablage_datei", ids["d7"], seal(other, aad, klar), revision=2, deleted=True)          # nicht vom Konto versiegelt


# --- Selbsttest gegen die Rust-Vektoren -----------------------------------------------------------------------------

def selftest(path):
    v = json.load(open(path, encoding="utf-8"))
    key = bytes.fromhex(v["schluessel_hex"]); user = v["user_id"]
    assert key_id(key) == v["key_id"], "key_id"
    assert standard_id(key) == v["standard_id"], "standard_id"
    for a in v["aad"]:
        refs = a["blob_refs"]
        got = aad_ablage(a["user_id"], a["collection"], a["object_id"], refs, a["key_id"], a["deleted"])
        assert got.hex() == a["aad_hex"], "aad " + a["name"]
    for g in v["grabstein"]:
        klar = canonical({"collection": g["collection"], "deleted": True, "id": g["object_id"]})
        assert klar == g["klartext"], "grab klartext"
        assert grab_nutzlast(key, user, g["collection"], g["object_id"]) == g["payload_base64"], "grab payload " + g["collection"]
        assert base64.b64decode(grab_nutzlast(key, user, g["collection"], g["object_id"])).hex() == g["payload_hex"]
    for h in v["hub_kennungen"]:
        assert hub_blob(key, h["sha256"]) == h["hub_kennung"], "hub_kennung"
    for t in v["textrev"]:
        assert textrev(t["original_sha256"], t["extraktor"], t["extraktor_version"], t["marken_version"]) == t["textrev"], "textrev"
    tf = v["textfassung"]
    roh = tf["json_utf8"].encode("utf-8")
    assert sha_hex(roh) == tf["sha256"] and hub_blob(key, tf["sha256"]) == tf["hub_kennung"], "textfassung"
    for name in ("datei", "ordner"):
        n = v["nutzlast"][name]
        assert canonical(json.loads(n["kanonisch"])) == n["kanonisch"], "kanonisch " + name
        assert sha_hex(n["kanonisch"].encode("utf-8")) == n["hash"], "hash " + name
    d = v["nutzlast"]["datei"]
    assert sorted(hub_blob(key, h) for h in d["blob_refs_inhalt"]) == d["blob_refs_hub"], "blob_refs_hub"
    print("Vektoren stimmen")


if __name__ == "__main__":
    selftest(sys.argv[1])
