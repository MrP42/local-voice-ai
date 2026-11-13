# /// script
# requires-python = ">=3.10"
# dependencies = ["huggingface_hub", "fsspec"]
# ///
"""
Handy model catalog generator.

Merges three sources into one catalog.json:
  1. HF card `transcribe_cpp` block  -> capabilities + benchmarks (canonical)
  2. a tiny GGUF header range-read    -> display labels only
  3. local CURATION (this file)       -> recommended set, editorial descriptions

Emits catalog.json to be committed and `include_str!`'d into the Rust binary.
Run:  HF_TOKEN=$(hf auth token) uv run gen_catalog.py [out_path]
      (default out_path: the checked-in src-tauri/src/catalog/catalog.json)

The generator owns only the entries of the `handy-computer` org (the ASR models, ids
`handy-computer/...`). Everything else in the catalog is maintained by hand and has no
generator source: Piper voices and runtimes, llama.cpp runtimes and language models,
diarization (Sortformer, Nemotron-3), speaker embedding, Ablage embedding, Phonon.
A run therefore reads the checked-in catalog, keeps every entry whose id is not an org
repo exactly as it is (same order, appended after the generated ones) and replaces only
the generated part. Corrections to a generated entry that the card does not carry
(e.g. a wrong license) live in LICENSE_OVERRIDES below, not in catalog.json, so the next
run does not undo them. `scripts/test_gen_catalog.py` proves both offline.

Determinism. The output is a pure function of the inputs and of nothing else. Its inputs
are exactly: the repo list of the org, per repo the commit sha of its listing, the card,
file listing and GGUF header of THAT sha (the header is read at `repo@sha`, never at
"main"), and the hand-maintained entries of the checked-in catalog. No clock enters the
output (`generated_at` is the latest lastModified of the repos), and neither does the file
that is being overwritten: its old bytes, stamp, layout or entries are never read back, so
a differently pre-filled output file cannot change what is written. The order of the
generated entries is a total order (the sort key ends in slug and the full repo id); the
repos are processed and reported in sorted order, not in the order the threads finish.
Hand-maintained entries are taken over by content and re-serialized by the one renderer
(key order is that of the source entry).

Fail closed. Any read that fails or comes back short or malformed, any needed field the
sources do not carry, and any value that is not of the exact expected form aborts the run
before anything is written; nothing is filled in from a default. The forms: the GGUF labels
`general.architecture` and `general.size_label` are present, are strings (a number or bool
in the header is no label) and match `ARCH_RE` / `SIZE_LABEL_RE`; a commit sha is exactly
40 lower-case hex digits; a file sha256 exactly 64; sizes are ints > 0 (a bool is not an
int here); two repos whose slugs collide (`x-gguf` and `x-gguf-gguf` are both `x`) abort,
because the slug is a key of the catalog. So the same hub state gives the same catalog,
and a new commit in a repo legitimately changes it.
"""
import json, os, re, sys, math, struct, datetime
from concurrent.futures import ThreadPoolExecutor

ORG = "handy-computer"
CATALOG_VERSION = 2
# The checked-in catalog: source of the hand-maintained entries and default output.
CATALOG_PATH = os.path.normpath(os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "..", "src-tauri", "src", "catalog", "catalog.json"))

# Download sources tried in order after Hugging Face itself. Each entry is a
# base URL; the full file URL is `{mirror}/{repo_id}/{revision}/{filename}`
# (the same three values that form the HF resolve URL, so a mirror is a plain
# static file host). Mirrors are untrusted: every download is verified against
# the per-file `sha256` below, so listing one only affects availability.
MIRRORS = ["https://blob.handy.computer"]

# ───────────────────────── scoring (one constant each) ──────────────────────
SPEED_SCALE = 8.0    # speed = 100·(1 − e^(−rtf/8))     grows toward 100
ACC_SCALE   = 15.0   # accuracy = 100·e^(−wer/15)        decays from 100
SPEED_FLOOR = 10     # no reference-machine rtf (too heavy to benchmark) → floor, it's slow
def speed_from_rtf(rtf):
    return SPEED_FLOOR if not rtf or rtf <= 0 else round(100 * (1 - math.exp(-rtf / SPEED_SCALE)))
def acc_from_wer(wer):
    return None if wer is None else round(100 * math.exp(-wer / ACC_SCALE))

# ───────────────────────── curation (CJ owns; editorial, not derivable) ──────
# slug -> {rank?, rec?, desc?, use_case?, default_quant?, hidden?}.  All optional.
# desc is hand-written UI copy; models without one use factual generated summaries.
# rank = editorial sort position (broad ordering). rec = the small "Recommended"
# badge / onboarding subset — independent of rank, so a model can rank high
# without carrying the recommended tag.
# Ranks differ from upstream Handy on purpose: this is a German-language app, so
# the German-tuned model leads and the English-only one drops behind it.
CURATION = {
    "parakeet-primeline":              {"rank": 1, "rec": True, "desc": "German-tuned, the most accurate model here for German"},
    "nemotron-3.5-asr-streaming-0.6b": {"rank": 2, "rec": True, "name": "Nemotron 3.5 ASR Streaming", "desc": "Live multilingual transcription across 28 languages"},
    "parakeet-unified-en-0.6b":        {"rank": 3, "rec": True, "desc": "Fast, accurate live English transcription"},
    "canary-180m-flash":               {"rank": 4, "rec": True, "desc": "Tiny and instant, runs well on any hardware"},
    "cohere-transcribe-03-2026":       {"rank": 5, "rec": True, "desc": "Highest accuracy, 14 languages, slower"},
    "whisper-medium":                  {"rank": 6, "rec": True, "desc": "Broadest language, but may run a bit slow"},
    # ranked (sorted high) but NOT tagged recommended
    "Voxtral-Mini-4B-Realtime-2602":   {"rank": 7, "desc": "Live multilingual, excellent on powerful machines"},
    "parakeet-tdt-0.6b-v3":            {"rank": 8, "desc": "Fast and accurate. Supports 25 European languages"},
    "parakeet-tdt-0.6b-v2":            {"rank": 9, "desc": "English only. The best model for English speakers"},
    "Qwen3-ASR-0.6B":                  {"rank": 10, "desc": "Excellent multilingual model"},
    "Fun-ASR-MLT-Nano-2512":           {"rank": 11, "desc": "A tiny multilingual model"},
    # description-only (unranked, not recommended) — carried over from the legacy .bin entry
    "Breeze-ASR-25":                   {"desc": "Optimized for Taiwanese Mandarin. Code-switching support."},
    # hidden until the pinned transcribe-cpp ships their arch (has no `moss`/`sortformer`
    # under src/arch/) — the app would offer a download it cannot load
    "moss-transcribe-diarize":            {"hidden": True},
    "diar_streaming_sortformer_4spk-v2.1": {"hidden": True},
}
# temporary capability corrections pending a card re-push (remove once cards fixed)
OVERRIDES = {
    "granite-4.0-1b-speech": {"timestamps": "none"},
    "granite-speech-4.1-2b": {"timestamps": "none"},
}
# slug -> {"license": ..., "license_url"?: ...}. The HF card of the GGUF repo carries a
# license that differs from the base model's; the base model's terms apply to the weights.
# Applied after the card, so it wins even if the card changes. Remove once the card is fixed.
LICENSE_OVERRIDES = {
    # card says cc-by-4.0; base model nvidia/parakeet-unified-en-0.6b is under the NVIDIA Open Model License
    "parakeet-unified-en-0.6b": {"license": "NVIDIA Open Model License"},
}

# ───────────────────────── helpers ──────────────────────────────────────────
ARCH = ["whisper","moonshine-streaming","moonshine","parakeet","canary-qwen","canary","voxtral",
        "granite-speech","granite","qwen3","gigaam","sensevoice","cohere","fun-asr","nemotron","medasr"]
ACR = {"asr":"ASR","ctc":"CTC","rnnt":"RNNT","tdt":"TDT","nar":"NAR","mlt":"MLT"}
SCALAR = {0:("<B",1),1:("<b",1),2:("<H",2),3:("<h",2),4:("<I",4),5:("<i",4),
          6:("<f",4),7:("<?",1),10:("<Q",8),11:("<q",8),12:("<d",8)}

def slug(repo): return repo.split("/")[1].replace("-gguf", "")
def family(s, tags):
    for f in ARCH:
        if f in s.lower(): return "moonshine" if f.startswith("moonshine") else f.split("-")[0]
    for t in tags or []:
        if t in ("whisper","moonshine","parakeet","canary","voxtral","granite","qwen3","gigaam","sensevoice","cohere"):
            return t
    return "other"
def pretty(s):
    return " ".join(ACR.get(p.lower(), p if (p.isupper() or any(c.isdigit() for c in p)) else p.capitalize())
                    for p in s.split("-"))
def quant_of(fn):
    m = re.search(r"-(F32|F16|BF16|Q\d[\w_]*?)\.gguf$", fn);  return m.group(1) if m else "?"
def parse_params(size_label):
    """`general.size_label` ("0.6B" / "1.7B" / "62M") -> count in billions, or None."""
    m = re.match(r"([\d.]+)\s*([BM])", str(size_label or "").strip(), re.I)
    if not m: return None
    v = float(m.group(1));  return v if m.group(2).upper() == "B" else v / 1000
def pick_wer(b):
    if "wer_librispeech_test_clean" in b: return "librispeech", b["wer_librispeech_test_clean"]
    if "wer_fleurs_en" in b: return "fleurs_en", b["wer_fleurs_en"]
    fl = [k for k in b if k.startswith("wer_fleurs")]
    return (fl[0].replace("wer_", ""), b[fl[0]]) if fl else (None, {})
def headline_wer(d):
    for q in ("q8_0","f16","q5_k_m","q6_k","q4_k_m","f32","bf16"):
        if isinstance(d, dict) and q in d: return d[q], q
    return None, None
LANG_NAMES = {"en":"English","ar":"Arabic","ja":"Japanese","ko":"Korean","ru":"Russian",
              "uk":"Ukrainian","vi":"Vietnamese","zh":"Chinese"}
def auto_desc(langs, caps):
    feats = []
    if caps["translate"]:   feats.append("translation")
    if caps["lang_detect"]: feats.append("auto language detection")
    if caps["streaming"]:   feats.append("streaming")
    if caps["timestamps"] != "none": feats.append(f"{caps['timestamps']}-level timestamps")
    if len(langs) > 1:
        base = f"{len(langs)}-language speech-to-text"
    else:
        # unknown code falls back to the raw code so it's caught in diff review
        lang = LANG_NAMES.get(langs[0], langs[0]) if langs else "English"
        base = f"{lang} speech-to-text"
    return base + (" with " + ", ".join(feats) + "." if feats else ".")

api = None   # HfApi, set by connect() — keeps the pure parts importable offline (tests)
fs  = None   # HfFileSystem, set by connect()
def connect():
    global api, fs
    from huggingface_hub import HfApi, HfFileSystem
    api = HfApi(token=os.environ.get("HF_TOKEN"))
    fs  = HfFileSystem(token=os.environ.get("HF_TOKEN"))

GGUF_WANT = {"general.architecture", "general.name", "general.basename", "general.size_label"}
# What the catalog derives its fields from: `architecture`, `parameters` and, through the
# size policy, `default_quant`. A header without them stops the run, it does not default.
HEADER_REQUIRED = ("general.architecture", "general.size_label")
HEADER_BYTES = 65536
# `list_models(limit=...)` truncates silently; reaching the limit means "maybe more".
LIST_LIMIT = 500
# Exact forms. `fullmatch` everywhere: `$` would let a trailing newline through.
COMMIT_RE = re.compile(r"[0-9a-f]{40}")
SHA256_RE = re.compile(r"[0-9a-f]{64}")
ARCH_RE = re.compile(r"[a-z][a-z0-9_]{0,63}")
SIZE_LABEL_RE = re.compile(r"[0-9]+(?:\.[0-9]+)?[BM]")
def _matches(rx, value):
    """True only for a `str` (not a number, bool or bytes) that `rx` matches completely."""
    return type(value) is str and rx.fullmatch(value) is not None
def is_commit(value): return _matches(COMMIT_RE, value)
def is_sha256(value): return _matches(SHA256_RE, value)
# `QuantFile.size_bytes` is a `u64` in Rust: a larger number would not load at all.
U64_MAX = 2**64 - 1
def is_size(value):
    """An int in 1..=2**64-1 (the `u64` it is read into, never 0); `type(...) is int` keeps
    True/False out."""
    return type(value) is int and 0 < value <= U64_MAX

# Capability flags are exactly `bool` on the card; anything else (`"false"`, 0, 1, None) is
# a broken card, not a value to coerce. An absent flag means False. `timestamps` is text.
CAPABILITY_FLAGS = ("streaming", "translate", "lang_detect")
def capability_fields(card, repo):
    """The `capabilities` dict of an entry from the card's `transcribe_cpp` block (plus the
    OVERRIDES). Pure; raises ValueError naming each field of the wrong type."""
    bad = [k for k in CAPABILITY_FLAGS if k in card and type(card[k]) is not bool]
    stamps = card.get("timestamps", "none")
    if type(stamps) is not str or not stamps:
        bad.append("timestamps")
    if bad:
        raise ValueError(f"{repo}: capability field(s) of the wrong type: {', '.join(bad)}")
    return {**{k: card.get(k, False) for k in CAPABILITY_FLAGS}, "timestamps": stamps}

class _WindowEnd(Exception):
    """The metadata section runs past the bytes that were read (tokenizer arrays): normal."""

def parse_header(buf):
    """Friendly `general.*` labels from the first bytes of a GGUF file (no tensors, no
    capabilities). Pure: the same bytes give the same dict.

    Capabilities come from HF card data here and from Rust's GGUF probe at runtime. Keep
    this as a narrow display-label reader so the catalog generator does not become a
    second source of truth for model behavior.

    Running out of bytes before all wanted keys were seen is expected (the metadata can
    continue into the tokenizer) and just ends the scan; the caller decides whether what
    it needs is there. Everything else that is wrong (no magic, truncated fixed part, an
    unknown value type) is a ValueError.
    """
    if buf[:4] != b"GGUF":
        raise ValueError("no GGUF magic")
    if len(buf) < 24:
        raise ValueError("GGUF header is shorter than its fixed part")
    def need(o, n):
        if o + n > len(buf): raise _WindowEnd
    def u32(o): need(o, 4); return struct.unpack_from("<I", buf, o)[0]
    def u64(o): need(o, 8); return struct.unpack_from("<Q", buf, o)[0]
    def rstr(o):
        n = u64(o); need(o + 8, n)
        return buf[o+8:o+8+n].decode("utf-8", "replace"), o + 8 + n
    def skip(o, vt):
        if vt in SCALAR:
            need(o, SCALAR[vt][1]); return o + SCALAR[vt][1]
        if vt == 8: return o + 8 + u64(o)
        if vt == 9:
            et = u32(o); cnt = u64(o + 4); o += 12
            if et in SCALAR: return o + SCALAR[et][1] * cnt
            for _ in range(cnt): o = skip(o, et)      # strings and nested arrays
            return o
        raise ValueError(f"unknown GGUF value type {vt}")
    out = {}
    off = 24                      # magic+ver+n_tensors+n_kv
    n_kv = u64(16)
    try:
        for _ in range(n_kv):
            key, off = rstr(off); vt = u32(off); off += 4
            if key in GGUF_WANT and vt == 8:
                out[key], off = rstr(off)
            elif key in GGUF_WANT and vt in SCALAR:
                fmt, sz = SCALAR[vt]; need(off, sz)
                out[key] = struct.unpack_from(fmt, buf, off)[0]; off += sz
            else:
                off = skip(off, vt)
            if GGUF_WANT.issubset(out): break
    except _WindowEnd:
        pass
    return out

def probe_header(repo, revision, filename, size, nbytes=HEADER_BYTES):
    """The `general.*` labels of one GGUF file at the pinned `revision`.

    Reads `repo@revision/filename`, so the bytes belong to the same commit as the listing
    and the card. Every failure is an exception: an unreadable file, fewer bytes than the
    file has (up to the window), no GGUF magic, a malformed header, a missing required key.
    """
    where = f"{repo}@{revision}/{filename}"
    try:
        with fs.open(where, "rb", block_size=nbytes) as f:
            buf = f.read(nbytes)
    except Exception as e:
        raise RuntimeError(f"{where}: header not readable: {e!r}") from e
    want = min(nbytes, size)
    if len(buf) != want:
        raise ValueError(f"{where}: header read returned {len(buf)} of {want} bytes")
    try:
        out = parse_header(buf)
    except ValueError as e:
        raise ValueError(f"{where}: {e}") from e
    try:
        check_header(out)
    except ValueError as e:
        raise ValueError(f"{where}: {e}") from e
    return out

def check_header(labels):
    """The required labels are present, are strings and have the exact expected form.

    A GGUF value of another type under a label name (a number or a bool) is no label: it is
    refused instead of being turned into text. `general.name` is optional but, if there, a
    string.
    """
    missing = [k for k in HEADER_REQUIRED if k not in labels]
    if missing:
        raise ValueError(f"header lacks {', '.join(missing)}")
    bad = []
    if not _matches(ARCH_RE, labels["general.architecture"]):
        bad.append("general.architecture")
    if not _matches(SIZE_LABEL_RE, labels["general.size_label"]):
        bad.append("general.size_label")
    for k in ("general.name", "general.basename"):
        if k in labels and type(labels[k]) is not str:
            bad.append(k)
    if bad:
        raise ValueError(f"header label(s) not of the expected form: {', '.join(bad)}")

def lfs_sha256(x):
    """Content sha256 from the sibling's LFS/Xet info (dataclass or dict by hub version)."""
    lfs = getattr(x, "lfs", None)
    if lfs is None: return None
    return getattr(lfs, "sha256", None) or (lfs.get("sha256") if isinstance(lfs, dict) else None)

def gguf_files(repo, siblings):
    """Return GGUF files with mandatory size + sha256 metadata.

    `QuantFile.size_bytes` is a non-null `u64` in Rust. Failing generation here
    keeps a transient/malformed HF listing from producing a catalog that panics
    at app startup when deserialized by `include_str!("catalog.json")`.

    `sha256` is the trust anchor for downloads (HF or mirror alike), so it is
    equally mandatory, and exactly 64 lower-case hex digits. Every GGUF is
    LFS/Xet-tracked; a missing hash means the listing is broken, not that the file
    is small. A size must be an int above zero.
    """
    files = []
    invalid = []
    for x in siblings:
        if type(x.rfilename) is not str or not x.rfilename.endswith(".gguf"):
            continue
        sha = lfs_sha256(x)
        if not is_size(x.size) or not is_sha256(sha):
            invalid.append(x.rfilename)
            continue
        files.append({
            "filename": x.rfilename,
            "quant": quant_of(x.rfilename),
            "size_bytes": x.size,
            "sha256": sha,
        })
    if invalid:
        raise ValueError(f"{repo}: missing/invalid size or sha256 metadata for {', '.join(invalid)}")
    return sorted(files, key=lambda f: (f["size_bytes"], f["filename"]))

def license_fields(cd, s):
    """`license` / `license_url` of a generated entry from its card, then LICENSE_OVERRIDES.

    Card license "other" is no information: carry the card's own license_name and link
    instead, so the notices and the model list can name the real terms.
    """
    other = cd.get("license") == "other"
    out = {
        "license": (cd.get("license_name") or cd.get("license")) if other else cd.get("license"),
        "license_url": cd.get("license_link") if other else None,
    }
    out.update(LICENSE_OVERRIDES.get(s, {}))
    return out

def build(repo):
    info = api.model_info(repo, files_metadata=True)
    if not is_commit(info.sha):
        raise ValueError(f"{repo}: listing has no commit sha to pin (need 40 hex digits)")
    modified = getattr(info, "last_modified", None)
    if modified is None:
        raise ValueError(f"{repo}: listing has no lastModified (generated_at is derived from it)")
    cd = info.card_data.to_dict() if info.card_data else {}
    s = slug(repo)
    cur = CURATION.get(s, {})
    b = dict(cd.get("transcribe_cpp") or {});  b.update(OVERRIDES.get(s, {}))
    langs = cd.get("language") or []

    files = gguf_files(repo, info.siblings)
    if not files:
        raise ValueError(f"{repo}: no GGUF files, nothing to offer")
    q8 = next((f for f in files if "Q8" in f["quant"]), files[-1])
    gg = probe_header(repo, info.sha, q8["filename"], q8["size_bytes"])

    caps = capability_fields(b, repo)
    ryz = b.get("rtf_ryzen_4750u") or {};  rtf = ryz.get("vulkan", ryz.get("cpu"))
    eval_set, werd = pick_wer(b);  hw, hwq = headline_wer(werd)

    # Default-quant policy by model size (params from general.size_label):
    #   >=1B  -> Q5_K_M   (large models tolerate it; ~30% less download/RAM)
    #   <1B   -> Q8_0     (already small; keep reference quality)
    # A per-model CURATION `default_quant` overrides the policy. If the policy
    # quant is absent for a model, fall back: Q8_0 → any Q8 → Q5_K_M → smallest.
    params = parse_params(gg.get("general.size_label"))
    policy = "Q5_K_M" if (params is not None and params >= 1.0) else "Q8_0"
    have = lambda q: next((f["quant"] for f in files if f["quant"] == q), None)
    default_quant = (cur.get("default_quant")
                     or have(policy)
                     or have("Q8_0")
                     or next((f["quant"] for f in files if "Q8" in f["quant"]), None)
                     or have("Q5_K_M")
                     or (files[0]["quant"] if files else None))

    entry = {
        "id": repo,
        # Pinned commit: downloads fetch `resolve/{revision}/{file}` so the bytes
        # provably match the hashes below even if the repo moves on. Identity
        # stays repo+filename; the pin only scopes acquisition.
        "revision": info.sha,
        "slug": s,
        # CURATION wins: a few GGUF headers carry a name that drops what the
        # model is actually called upstream (nemotron-3.5-asr -> "Nemotron
        # Streaming 3.5"), which makes the model unfindable by its real name.
        "name": cur.get("name") or gg.get("general.name") or pretty(s),
        "architecture": gg.get("general.architecture"),
        "family": family(s, info.tags),
        "parameters": gg.get("general.size_label"),          # "0.6B" / "1.7B" / "62M"
        "description": cur.get("desc") or auto_desc(langs, caps),
        "base_model": cd.get("base_model"),
        **license_fields(cd, s),
        "language_count": len(langs),
        "languages": langs,
        "capabilities": caps,
        "speed_score": speed_from_rtf(rtf),
        "accuracy_score": acc_from_wer(hw),
        "files": files,
        "default_quant": default_quant,
        "recommended": bool(cur.get("rec")),         # small badge/onboarding subset
        "recommended_rank": cur.get("rank"),          # editorial sort position (independent)
    }
    check_entry(entry)
    return entry, modified

def check_entry(entry):
    """A generated entry in its final shape; the last gate before it can be written.

    Collects every violation so one run names them all. Pure; raises ValueError.
    """
    bad = []
    rid = entry.get("id")
    if type(rid) is not str or not rid.startswith(ORG + "/") or rid.count("/") != 1:
        bad.append("id")
    elif entry.get("slug") != slug(rid):
        bad.append("slug")
    if not is_commit(entry.get("revision")):
        bad.append("revision")
    if not _matches(ARCH_RE, entry.get("architecture")):
        bad.append("architecture")
    if not _matches(SIZE_LABEL_RE, entry.get("parameters")):
        bad.append("parameters")
    caps = entry.get("capabilities")
    if not isinstance(caps, dict):
        bad.append("capabilities")
    else:
        bad.extend(f"capabilities.{k}" for k in CAPABILITY_FLAGS if type(caps.get(k)) is not bool)
        if type(caps.get("timestamps")) is not str or not caps["timestamps"]:
            bad.append("capabilities.timestamps")
    files = entry.get("files")
    if not isinstance(files, list) or not files:
        bad.append("files")
    else:
        for f in files:
            name = f.get("filename") if isinstance(f, dict) else None
            if type(name) is not str or not name:
                bad.append("files[].filename")
            elif not is_size(f.get("size_bytes")) or not is_sha256(f.get("sha256")):
                bad.append(f"files[{name}] size_bytes/sha256")
        quants = {f.get("quant") for f in files if isinstance(f, dict)}
        if entry.get("default_quant") not in quants:
            bad.append("default_quant")
    if bad:
        raise ValueError(f"{rid!r}: invalid field(s): {', '.join(bad)}")

def check_generated(entries):
    """Every entry passes `check_entry`; ids and slugs are unique (case-insensitively)."""
    seen_id, seen_slug, clash = {}, {}, []
    for m in entries:
        check_entry(m)
        for table, key in ((seen_id, m["id"].casefold()), (seen_slug, m["slug"].casefold())):
            if key in table: clash.append(f"{table[key]} / {m['id']}")
            table[key] = m["id"]
    if clash:
        raise ValueError(f"colliding ids or slugs: {', '.join(sorted(set(clash)))}")

def check_repos(repos):
    """Two repos whose slugs collide would fight over one catalog key: abort before any read."""
    seen, clash = {}, []
    for r in repos:
        key = slug(r).casefold()
        if key in seen: clash.append(f"{seen[key]} / {r}")
        seen[key] = r
    if clash:
        raise SystemExit(f"repos with the same catalog slug: {', '.join(clash)}")

def is_generated(entry):
    """True for the entries this script owns: the repos of the org."""
    return entry["id"].startswith(ORG + "/")

def assemble(generated, existing):
    """Generated entries first, then every hand-maintained entry of `existing` untouched.

    `existing` is the previous catalog's `models` list. Entries the generator owns are
    dropped from it (they are replaced by `generated`); all others keep their content
    and relative order. Refuses duplicate ids instead of silently shadowing one.
    """
    kept = [m for m in existing if not is_generated(m)]
    models = list(generated) + kept
    seen, dup = set(), []
    for m in models:
        if m["id"] in seen: dup.append(m["id"])
        seen.add(m["id"])
    if dup:
        raise ValueError(f"duplicate catalog ids: {', '.join(sorted(set(dup)))}")
    return models

def read_catalog(path):
    """Existing catalog; a missing or unreadable file is an error, never an empty list.

    An empty fallback would make a run drop every hand-maintained entry silently.
    """
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)["models"]
    except (OSError, ValueError, KeyError) as e:
        raise SystemExit(f"cannot read the existing catalog {path} (needed to keep the "
                         f"hand-maintained entries): {e}")

def render(catalog):
    text = json.dumps(catalog, indent=2, ensure_ascii=False, allow_nan=False)
    # print each `languages` array on a single line for readability
    text = re.sub(r'"languages": \[(.*?)\]',
                  lambda m: '"languages": [' + ", ".join(re.findall(r'"[^"]*"', m.group(1))) + ']',
                  text, flags=re.S)
    # one line per `files[]` entry: a file is one diff line, so schema additions
    # and hash changes don't cascade into per-key comma churn. `[^{}]` keeps the
    # match inside one object: entries whose file objects end in another key than
    # sha256 (hand-maintained ones carry `url`) stay in the plain layout instead of
    # being fused with a later object.
    text = re.sub(r'\{\s+("filename":[^{}]*?"sha256": "[0-9a-f]{64}")\s+\}',
                  lambda m: "{" + re.sub(r",\s+", ", ", m.group(1)) + "}",
                  text, flags=re.S)
    return text + "\n"

def stamp(moment):
    """`generated_at` text for a lastModified: UTC, whole seconds (a naive value counts as UTC)."""
    if moment.tzinfo is None:
        moment = moment.replace(tzinfo=datetime.timezone.utc)
    return moment.astimezone(datetime.timezone.utc).isoformat(timespec="seconds")

def write_catalog(out, generated, generated_at, existing_path=None):
    """Write generated + hand-maintained entries to `out`; returns (models, written).

    The bytes are a function of the arguments and of the hand-maintained entries of
    `existing_path`; whatever `out` holds now is neither read nor compared (a stale stamp
    or another layout there changes nothing). The generated entries are checked first;
    a violation raises before `out` is touched, and the write is a temp file renamed over
    it. `written` is False only when `out` already has exactly these bytes.
    """
    check_generated(generated)
    models = assemble(generated, read_catalog(existing_path or CATALOG_PATH))
    catalog = {
        "catalog_version": CATALOG_VERSION,
        "generated_at": generated_at,
        "mirrors": MIRRORS,
        "models": models,
    }
    text = render(catalog)
    try:
        with open(out, "rb") as f:
            if f.read() == text.encode("utf-8"):
                return models, False
    except OSError:
        pass
    tmp = out + ".tmp"
    try:
        with open(tmp, "w", encoding="utf-8", newline="\n") as f:
            f.write(text)
        os.replace(tmp, out)      # a crash mid-write never leaves a half catalog behind
    except BaseException:
        if os.path.exists(tmp): os.remove(tmp)
        raise
    return models, True

def main():
    connect()
    out = sys.argv[1] if len(sys.argv) > 1 else CATALOG_PATH
    listed = [m.id for m in api.list_models(author=ORG, limit=LIST_LIMIT)]
    if len(listed) >= LIST_LIMIT:
        raise SystemExit(f"the org lists {len(listed)} repos, the limit of {LIST_LIMIT} may cut it off; raise LIST_LIMIT")
    # Hidden repos never reach the catalog, so they must not influence the run either.
    repos = sorted(r for r in listed if not CURATION.get(slug(r), {}).get("hidden"))
    if not repos:
        raise SystemExit("the org lists no usable repo; refusing to write an empty ASR catalog")
    check_repos(repos)
    models = []
    stamps = []
    failures = []
    with ThreadPoolExecutor(max_workers=10) as ex:
        # Collected in the sorted order of `repos`, never in the order the threads finish:
        # neither the entries nor the error messages depend on network timing.
        futs = [(r, ex.submit(build, r)) for r in repos]
        for repo, f in futs:
            try:
                entry, modified = f.result()
                models.append(entry)
                stamps.append(modified)
            except Exception as e:
                failures.append((repo, e))
                print(f"!! {repo}: {e}", file=sys.stderr)
    if failures:
        print(f"catalog generation failed for {len(failures)} repo(s), nothing written", file=sys.stderr)
        raise SystemExit(1)
    try:
        check_generated(models)
    except ValueError as e:
        raise SystemExit(f"catalog generation refused, nothing written: {e}")
    models.sort(key=lambda m: (not m["recommended"], m["recommended_rank"] or 1e9,
                               m["family"], -(m["speed_score"] or 0), m["slug"], m["id"]))
    all_models, written = write_catalog(out, models, stamp(max(stamps)))
    state = (f"wrote {out}" if written else f"{out} unchanged")
    print(f"{state}: {len(models)} generated + {len(all_models) - len(models)} hand-maintained "
          f"models, {os.path.getsize(out)/1024:.1f} KB", file=sys.stderr)

if __name__ == "__main__":
    main()
