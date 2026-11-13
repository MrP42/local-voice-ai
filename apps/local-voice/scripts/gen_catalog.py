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

Determinism. A run is a function of the hub metadata and nothing else. Its inputs are
exactly: the repo list of the org, per repo the commit sha of its listing, and the card,
file listing and GGUF header of THAT sha (the header is read at `repo@sha`, never at
"main"). No clock enters the output: `generated_at` is the latest lastModified of the
repos. Any read that fails or comes back short or malformed, and any needed field the
sources do not carry, aborts the run before anything is written; nothing is filled in
from a default. So the same hub state gives the same catalog, and a new commit in a repo
legitimately changes it. If the generated content equals what the file already holds
(`generated_at` aside), the file is left byte for byte as it is.
"""
import json, os, re, sys, math, struct, datetime
from concurrent.futures import ThreadPoolExecutor, as_completed

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
    missing = [k for k in HEADER_REQUIRED if k not in out]
    if missing:
        raise ValueError(f"{where}: header lacks {', '.join(missing)}")
    return out

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
    equally mandatory. Every GGUF is LFS/Xet-tracked; a missing hash means the
    listing is broken, not that the file is small.
    """
    files = []
    invalid = []
    for x in siblings:
        if not x.rfilename.endswith(".gguf"):
            continue
        sha = lfs_sha256(x)
        if type(x.size) is not int or x.size <= 0 or not sha:
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
    if not info.sha:
        raise ValueError(f"{repo}: listing has no commit sha to pin")
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

    caps = {"streaming": bool(b.get("streaming")), "translate": bool(b.get("translate")),
            "lang_detect": bool(b.get("lang_detect")), "timestamps": b.get("timestamps", "none")}
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
    return entry, modified

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
    text = json.dumps(catalog, indent=2, ensure_ascii=False)
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

def same_content(a, b):
    """Equal apart from `generated_at`."""
    strip = lambda c: {k: v for k, v in c.items() if k != "generated_at"}
    return strip(a) == strip(b)

def load_catalog(path):
    """The catalog at `path` as a dict, or None if there is none or it is unreadable."""
    try:
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
    except (OSError, ValueError):
        return None
    return data if isinstance(data, dict) else None

def write_catalog(out, generated, generated_at, existing_path=None):
    """Write generated + hand-maintained entries to `out`; returns (models, written).

    `generated_at` comes from the inputs (see the module docstring). If `out` already
    holds the same content apart from `generated_at`, nothing is written: the file keeps
    its bytes, layout and old stamp.
    """
    models = assemble(generated, read_catalog(existing_path or CATALOG_PATH))
    catalog = {
        "catalog_version": CATALOG_VERSION,
        "generated_at": generated_at,
        "mirrors": MIRRORS,
        "models": models,
    }
    current = load_catalog(out)
    if current is not None and same_content(current, json.loads(json.dumps(catalog))):
        return models, False
    tmp = out + ".tmp"
    try:
        with open(tmp, "w", encoding="utf-8", newline="\n") as f:
            f.write(render(catalog))
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
    models = []
    stamps = []
    failures = []
    with ThreadPoolExecutor(max_workers=10) as ex:
        futs = {ex.submit(build, r): r for r in repos}
        for f in as_completed(futs):
            try:
                entry, modified = f.result()
                models.append(entry)
                stamps.append(modified)
            except Exception as e:
                failures.append((futs[f], e))
                print(f"!! {futs[f]}: {e}", file=sys.stderr)
    if failures:
        print(f"catalog generation failed for {len(failures)} repo(s), nothing written", file=sys.stderr)
        raise SystemExit(1)
    models.sort(key=lambda m: (not m["recommended"], m["recommended_rank"] or 1e9,
                               m["family"], -(m["speed_score"] or 0), m["slug"]))
    all_models, written = write_catalog(out, models, stamp(max(stamps)))
    state = (f"wrote {out}" if written else f"{out} unchanged")
    print(f"{state}: {len(models)} generated + {len(all_models) - len(models)} hand-maintained "
          f"models, {os.path.getsize(out)/1024:.1f} KB", file=sys.stderr)

if __name__ == "__main__":
    main()
