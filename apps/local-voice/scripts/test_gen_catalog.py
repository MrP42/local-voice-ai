"""Offline tests for gen_catalog.py: what a generator run keeps and what it corrects.

No network and no huggingface_hub needed (the module connects lazily in main()).
Run:  python scripts/test_gen_catalog.py     (or: bun run test:catalog)
"""
import copy
import datetime as dt
import importlib.util
import json
import os
import struct
import sys
import tempfile
import types
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("gen_catalog", os.path.join(HERE, "gen_catalog.py"))
gc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gc)

STAMP = "2026-10-05T08:30:00+00:00"
NEMOTRON = "diar-nemotron3-8spk-onnx-q8"
PARAKEET_UNIFIED = "handy-computer/parakeet-unified-en-0.6b-gguf"
NVIDIA_OML = "NVIDIA Open Model License"


def checked_in():
    with open(gc.CATALOG_PATH, encoding="utf-8") as f:
        return json.load(f)


def by_id(models):
    return {m["id"]: m for m in models}


class LicenseFields(unittest.TestCase):
    def test_override_beats_the_card(self):
        got = gc.license_fields({"license": "cc-by-4.0"}, "parakeet-unified-en-0.6b")
        self.assertEqual(got["license"], NVIDIA_OML)

    def test_override_also_beats_a_card_with_other(self):
        card = {"license": "other", "license_name": "x", "license_link": "https://x.example/l"}
        got = gc.license_fields(card, "parakeet-unified-en-0.6b")
        self.assertEqual(got["license"], NVIDIA_OML)

    def test_other_slugs_keep_the_card_license(self):
        got = gc.license_fields({"license": "cc-by-4.0"}, "parakeet-tdt-0.6b-v3")
        self.assertEqual(got, {"license": "cc-by-4.0", "license_url": None})

    def test_card_other_carries_license_name_and_link(self):
        card = {"license": "other", "license_name": "Some License 1.0", "license_link": "https://x.example/l"}
        got = gc.license_fields(card, "whatever")
        self.assertEqual(got, {"license": "Some License 1.0", "license_url": "https://x.example/l"})

    def test_every_override_is_in_the_checked_in_catalog(self):
        models = by_id(checked_in()["models"])
        for slug, fields in gc.LICENSE_OVERRIDES.items():
            entry = models[f"{gc.ORG}/{slug}-gguf"]
            for key, value in fields.items():
                self.assertEqual(entry[key], value, f"{slug}.{key}")


class Assemble(unittest.TestCase):
    def setUp(self):
        self.cat = checked_in()
        self.existing = self.cat["models"]
        self.generated = [copy.deepcopy(m) for m in self.existing if gc.is_generated(m)]
        self.hand = [m for m in self.existing if not gc.is_generated(m)]

    def test_hand_entries_survive_verbatim_and_in_order(self):
        # a run where the hub changed every revision: the generated part is new, the rest is not
        for m in self.generated:
            m["revision"] = "0" * 40
        got = gc.assemble(self.generated, self.existing)
        n = len(self.generated)
        self.assertEqual(got[:n], self.generated)
        self.assertEqual(got[n:], self.hand)
        self.assertEqual([m["id"] for m in got[n:]], [m["id"] for m in self.hand])

    def test_the_nemotron_entry_is_kept_with_both_files(self):
        got = by_id(gc.assemble(self.generated, self.existing))
        self.assertIn(NEMOTRON, got)
        entry = got[NEMOTRON]
        self.assertEqual(entry["purpose"], "diarization")
        self.assertEqual([f["filename"] for f in entry["files"]],
                         ["model_quantized.onnx", "model_quantized.onnx_data"])
        self.assertTrue(all(len(f["sha256"]) == 64 and f["size_bytes"] > 0 for f in entry["files"]))

    def test_the_other_hand_maintained_kinds_survive_too(self):
        ids = [m["id"] for m in gc.assemble(self.generated, self.existing)]
        for prefix in ("diar-sortformer", "spk-campplus", "ablage-e5", "phonon-", "llm-", "piper-", "emb-"):
            self.assertTrue(any(i.startswith(prefix) for i in ids), prefix)
        self.assertEqual(len(ids), len(self.existing))

    def test_a_vanished_org_repo_disappears_and_a_new_one_appears(self):
        dropped = self.generated.pop(0)
        added = copy.deepcopy(self.generated[0])
        added["id"] = f"{gc.ORG}/brand-new-gguf"
        self.generated.append(added)
        ids = [m["id"] for m in gc.assemble(self.generated, self.existing)]
        self.assertNotIn(dropped["id"], ids)
        self.assertIn(added["id"], ids)
        self.assertIn(NEMOTRON, ids)

    def test_duplicate_ids_are_refused(self):
        clash = copy.deepcopy(self.hand[0])
        with self.assertRaises(ValueError):
            gc.assemble(self.generated + [clash], self.existing)


class Writing(unittest.TestCase):
    def test_missing_catalog_is_fatal_not_empty(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(SystemExit):
                gc.read_catalog(os.path.join(d, "gibt-es-nicht.json"))

    def test_broken_catalog_is_fatal_not_empty(self):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "catalog.json")
            with open(p, "w", encoding="utf-8") as f:
                f.write("{ nicht json")
            with self.assertRaises(SystemExit):
                gc.read_catalog(p)

    def test_render_round_trips_the_whole_catalog(self):
        # the compact layout must never change content, hand-maintained entries included
        cat = checked_in()
        self.assertEqual(json.loads(gc.render(cat))["models"], cat["models"])

    def test_render_leaves_file_objects_with_a_url_alone(self):
        entry = by_id(checked_in()["models"])[NEMOTRON]
        text = gc.render({"models": [entry]})
        self.assertEqual(json.loads(text)["models"][0], entry)
        self.assertEqual(text.count('"filename"'), 2)
        for line in text.splitlines():
            if '"filename"' in line:
                self.assertNotIn('"sha256"', line, "a url-ending file object must stay multi-line")

    def test_render_puts_a_generated_file_on_one_line(self):
        f = {"filename": "m-Q8_0.gguf", "quant": "Q8_0", "size_bytes": 5, "sha256": "ab" * 32}
        text = gc.render({"models": [{"id": "handy-computer/m-gguf", "files": [f]}]})
        self.assertEqual(sum('"filename"' in line for line in text.splitlines()), 1)
        self.assertEqual(json.loads(text)["models"][0]["files"], [f])

    def test_a_run_writes_generated_plus_all_hand_entries_atomically(self):
        cat = checked_in()
        generated = [m for m in cat["models"] if gc.is_generated(m)]
        with tempfile.TemporaryDirectory() as d:
            out = os.path.join(d, "catalog.json")
            models, wrote = gc.write_catalog(out, generated, STAMP, gc.CATALOG_PATH)
            self.assertTrue(wrote)
            self.assertEqual(len(models), len(cat["models"]))
            with open(out, encoding="utf-8") as f:
                text = f.read()
            written = json.loads(text)
            self.assertEqual(written["generated_at"], STAMP)
            self.assertEqual(written["models"], cat["models"])
            self.assertEqual(written["catalog_version"], cat["catalog_version"])
            self.assertTrue(text.endswith("}\n"))
            self.assertEqual(os.listdir(d), ["catalog.json"], "no temp file left behind")

    def test_a_failing_write_keeps_the_old_file(self):
        cat = checked_in()
        generated = [m for m in cat["models"] if gc.is_generated(m)]
        with tempfile.TemporaryDirectory() as d:
            out = os.path.join(d, "catalog.json")
            with open(out, "w", encoding="utf-8") as f:
                f.write("ALT")
            broken = generated + [{"id": "handy-computer/x-gguf", "boom": object()}]
            with self.assertRaises(TypeError):
                gc.write_catalog(out, broken, STAMP, gc.CATALOG_PATH)
            with open(out, encoding="utf-8") as f:
                self.assertEqual(f.read(), "ALT")
            self.assertEqual(os.listdir(d), ["catalog.json"], "the half-written temp file is removed")

    def test_main_end_to_end_with_a_fake_hub(self):
        cat = checked_in()
        generated = {m["id"]: m for m in cat["models"] if gc.is_generated(m)}
        saved = (gc.api, gc.build, gc.connect, sys.argv)
        try:
            gc.connect = lambda: None
            gc.api = types.SimpleNamespace(
                list_models=lambda **kw: [types.SimpleNamespace(id=i) for i in generated])
            gc.build = lambda repo: (copy.deepcopy(generated[repo]), dt.datetime(2026, 10, 5, 8, 30, tzinfo=dt.timezone.utc))
            with tempfile.TemporaryDirectory() as d:
                out = os.path.join(d, "catalog.json")
                sys.argv = ["gen_catalog.py", out]
                gc.main()
                with open(out, encoding="utf-8") as f:
                    written = json.load(f)
        finally:
            gc.api, gc.build, gc.connect, sys.argv = saved
        got = by_id(written["models"])
        self.assertEqual(len(got), len(cat["models"]))
        self.assertIn(NEMOTRON, got)
        self.assertEqual(got[NEMOTRON], by_id(cat["models"])[NEMOTRON])
        self.assertEqual(got[PARAKEET_UNIFIED]["license"], NVIDIA_OML)

    def test_main_does_not_write_when_a_repo_fails(self):
        saved = (gc.api, gc.build, gc.connect, sys.argv)
        try:
            gc.connect = lambda: None
            gc.api = types.SimpleNamespace(
                list_models=lambda **kw: [types.SimpleNamespace(id=f"{gc.ORG}/x-gguf")])

            def boom(repo):
                raise ValueError("listing broken")
            gc.build = boom
            with tempfile.TemporaryDirectory() as d:
                out = os.path.join(d, "catalog.json")
                sys.argv = ["gen_catalog.py", out]
                with self.assertRaises(SystemExit):
                    gc.main()
                self.assertFalse(os.path.exists(out))
        finally:
            gc.api, gc.build, gc.connect, sys.argv = saved


# ---- a faked hub: only the hub and the header access are faked, `build` is the real one ----

def kv_key(key):
    kb = key.encode()
    return struct.pack("<Q", len(kb)) + kb


def kv_str(key, val):
    vb = val.encode()
    return kv_key(key) + struct.pack("<IQ", 8, len(vb)) + vb


def kv_strarr(key, items):
    body = b"".join(struct.pack("<Q", len(i.encode())) + i.encode() for i in items)
    return kv_key(key) + struct.pack("<IIQ", 9, 8, len(items)) + body


def kv_nested(key):
    """An array whose two elements are u32 arrays."""
    inner = struct.pack("<IQ", 4, 1) + struct.pack("<I", 7)
    return kv_key(key) + struct.pack("<IIQ", 9, 9, 2) + inner + inner


def gguf(kvs, pad_to=0):
    head = b"GGUF" + struct.pack("<IQQ", 3, 0, len(kvs))
    body = head + b"".join(kvs)
    return body + b"\0" * max(0, pad_to - len(body))


def asr_gguf(arch="whisper", size_label="0.6B", name="Fake ASR", with_label=True, pad_to=0):
    kvs = [
        kv_str("general.architecture", arch),
        kv_strarr("tokenizer.ggml.tokens", ["a", "bb", "ccc"]),
        kv_nested("tokenizer.nested"),
        kv_str("general.name", name),
    ]
    if with_label:
        kvs.append(kv_str("general.size_label", size_label))
    return gguf(kvs, pad_to)


UTC = dt.timezone.utc
CARD = {
    "license": "apache-2.0",
    "language": ["en"],
    "transcribe_cpp": {
        "streaming": True,
        "timestamps": "word",
        "rtf_ryzen_4750u": {"cpu": 2.0},
        "wer_librispeech_test_clean": {"q8_0": 3.1},
    },
}


class FakeHub:
    """Repo list, listings and header reads of a Hugging Face org, all in memory."""

    def __init__(self):
        self.repos = {}
        self.fault = None
        self.opened = []
        self.model_info_calls = []

    def add(self, name, sha, modified, header=None, files=None):
        header = header if header is not None else asr_gguf()
        files = files if files is not None else {
            f"{name}-Q8_0.gguf": header,
            f"{name}-Q5_K_M.gguf": header + b"\0" * 10,
        }
        repo = f"{gc.ORG}/{name}-gguf"
        self.repos[repo] = types.SimpleNamespace(sha=sha, modified=modified, files=files)
        return repo

    def api(self):
        import hashlib

        def list_models(author=None, limit=None):
            return [types.SimpleNamespace(id=r) for r in self.repos]

        def model_info(repo, files_metadata=False):
            self.model_info_calls.append(repo)
            r = self.repos[repo]
            siblings = [
                types.SimpleNamespace(
                    rfilename=n, size=len(b),
                    lfs=types.SimpleNamespace(sha256=hashlib.sha256(b).hexdigest()))
                for n, b in r.files.items()
            ]
            return types.SimpleNamespace(
                sha=r.sha,
                card_data=types.SimpleNamespace(to_dict=lambda: copy.deepcopy(CARD)),
                siblings=siblings, tags=[], last_modified=r.modified)

        return types.SimpleNamespace(list_models=list_models, model_info=model_info)

    def fs(self):
        hub = self

        class Handle:
            def __init__(self, data):
                self.data = data

            def __enter__(self):
                return self

            def __exit__(self, *exc):
                return False

            def read(self, n):
                return self.data[:n]

        def open_(path, mode="rb", block_size=None):
            hub.opened.append(path)
            if hub.fault == "raise":
                raise OSError("connection reset")
            head, filename = path.rsplit("/", 1)
            if "@" not in head:
                raise FileNotFoundError("unpinned read: " + path)
            repo, sha = head.split("@", 1)
            r = hub.repos.get(repo)
            if r is None or r.sha != sha or filename not in r.files:
                raise FileNotFoundError(path)
            data = r.files[filename]
            if hub.fault == "short":
                return Handle(data[:100])
            if hub.fault == "short1":
                return Handle(data[:min(block_size, len(data)) - 1])
            if hub.fault == "html":
                return Handle(b"<html>Too Many Requests</html>".ljust(min(block_size, len(data)), b" "))
            return Handle(data)

        return types.SimpleNamespace(open=open_)


class HubRuns(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.dir = tmp.name
        hand = [m for m in checked_in()["models"] if not gc.is_generated(m)][:3]
        self.hand = hand
        self.existing = os.path.join(self.dir, "existing.json")
        with open(self.existing, "w", encoding="utf-8") as f:
            json.dump({"catalog_version": 2, "generated_at": "alt", "mirrors": [], "models": hand}, f)
        self.out = os.path.join(self.dir, "catalog.json")
        self.hub = FakeHub()
        self.t1 = dt.datetime(2026, 9, 1, 12, 0, tzinfo=UTC)
        self.t2 = dt.datetime(2026, 10, 5, 8, 30, tzinfo=UTC)
        self.hub.add("fakeasr", "a" * 40, self.t1, header=asr_gguf(pad_to=100_000))
        self.hub.add("other", "b" * 40, self.t2)

    def run_main(self, out=None):
        import contextlib
        import io
        saved = (gc.api, gc.fs, gc.connect, gc.CATALOG_PATH, sys.argv)
        try:
            gc.connect = lambda: None
            gc.api, gc.fs = self.hub.api(), self.hub.fs()
            gc.CATALOG_PATH = self.existing
            sys.argv = ["gen_catalog.py", out or self.out]
            with contextlib.redirect_stderr(io.StringIO()):
                gc.main()
        finally:
            gc.api, gc.fs, gc.connect, gc.CATALOG_PATH, sys.argv = saved

    def read(self, path=None):
        with open(path or self.out, "rb") as f:
            return f.read()

    def assert_aborts_without_touching_the_file(self, sentinel=b"ALT"):
        with open(self.out, "wb") as f:
            f.write(sentinel)
        with self.assertRaises(SystemExit):
            self.run_main()
        self.assertEqual(self.read(), sentinel)
        self.assertEqual([n for n in os.listdir(self.dir) if n.endswith(".tmp")], [], "temp file left behind")

    # -- header access -------------------------------------------------------

    def test_the_header_is_read_at_the_pinned_revision(self):
        self.run_main()
        want = sorted(
            f"{repo}@{r.sha}/{name}"
            for repo, r in self.hub.repos.items()
            for name in r.files if "Q8" in name
        )
        self.assertEqual(sorted(self.hub.opened), want)
        for path in self.hub.opened:
            self.assertRegex(path, r"@[ab]{40}/")

    def test_a_new_commit_changes_the_pin_the_header_is_read_at(self):
        self.run_main()
        self.hub.opened.clear()
        self.hub.repos[f"{gc.ORG}/other-gguf"].sha = "c" * 40
        self.run_main()
        self.assertIn(f"{gc.ORG}/other-gguf@{'c' * 40}/other-Q8_0.gguf", self.hub.opened)

    def test_identical_metadata_with_a_working_and_a_failing_header_access(self):
        good = os.path.join(self.dir, "good.json")
        self.run_main(good)
        written = json.loads(self.read(good))
        entry = by_id(written["models"])[f"{gc.ORG}/fakeasr-gguf"]
        self.assertEqual((entry["architecture"], entry["parameters"], entry["default_quant"]),
                         ("whisper", "0.6B", "Q8_0"))
        # same listing, same cards, but the header access fails: no catalog, old file intact
        self.hub.fault = "raise"
        self.assert_aborts_without_touching_the_file()

    def test_every_kind_of_header_failure_is_a_hard_error(self):
        for fault in ("raise", "short", "html"):
            with self.subTest(fault=fault):
                self.hub.fault = fault
                self.assert_aborts_without_touching_the_file()

    def test_a_read_one_byte_short_is_a_hard_error_even_with_all_labels_in_it(self):
        del self.hub.repos[f"{gc.ORG}/other-gguf"]       # only the padded header: labels come first
        self.hub.fault = "short1"
        self.assert_aborts_without_touching_the_file()

    def test_a_header_without_a_required_label_stops_the_run(self):
        self.hub.add("nolabel", "d" * 40, self.t1, header=asr_gguf(with_label=False))
        self.assert_aborts_without_touching_the_file()

    def test_a_malformed_header_stops_the_run(self):
        bad = gguf([kv_key("x.y") + struct.pack("<I", 99), kv_str("general.architecture", "w")])
        self.hub.add("malformed", "d" * 40, self.t1, header=bad)
        self.assert_aborts_without_touching_the_file()

    def test_hidden_repos_are_not_built_and_cannot_stop_the_run(self):
        hidden = self.hub.add("moss-transcribe-diarize", "e" * 40, self.t1, header=b"kaputt")
        self.run_main()
        self.assertNotIn(hidden, self.hub.model_info_calls)
        self.assertNotIn(hidden, [p.rsplit("@", 1)[0] for p in self.hub.opened])
        self.assertNotIn(hidden, [m["id"] for m in json.loads(self.read())["models"]])

    # -- the other inputs ----------------------------------------------------

    def test_a_repo_without_gguf_files_stops_the_run(self):
        self.hub.add("empty", "d" * 40, self.t1, files={"README.md": b"x"})
        self.assert_aborts_without_touching_the_file()

    def test_a_repo_without_last_modified_stops_the_run(self):
        self.hub.add("undated", "d" * 40, None)
        self.assert_aborts_without_touching_the_file()

    def test_an_empty_listing_never_replaces_the_catalog(self):
        self.hub.repos.clear()
        self.assert_aborts_without_touching_the_file()

    def test_a_listing_at_the_limit_may_be_cut_off_and_stops_the_run(self):
        saved = gc.LIST_LIMIT
        gc.LIST_LIMIT = len(self.hub.repos)
        try:
            self.assert_aborts_without_touching_the_file()
        finally:
            gc.LIST_LIMIT = saved

    # -- clock and byte stability ---------------------------------------------

    def test_the_clock_does_not_enter_the_output(self):
        calls = []

        def clock(value):
            class FakeDT(dt.datetime):
                @classmethod
                def now(cls, tz=None):
                    calls.append(value)
                    return value
            return types.SimpleNamespace(datetime=FakeDT, timezone=dt.timezone)

        outs = []
        for i, now in enumerate((dt.datetime(2001, 1, 1, tzinfo=UTC), dt.datetime(2040, 6, 6, tzinfo=UTC))):
            out = os.path.join(self.dir, f"out{i}.json")
            saved, gc.datetime = gc.datetime, clock(now)
            try:
                self.run_main(out)
            finally:
                gc.datetime = saved
            outs.append(self.read(out))
        self.assertEqual(outs[0], outs[1], "two clocks, two different catalogs")
        self.assertEqual(calls, [], "the output read the clock")
        self.assertEqual(json.loads(outs[0])["generated_at"], STAMP)

    def test_two_runs_over_the_same_hub_state_are_byte_identical(self):
        a, b = os.path.join(self.dir, "a.json"), os.path.join(self.dir, "b.json")
        self.run_main(a)
        self.run_main(b)
        self.assertEqual(self.read(a), self.read(b))

    def test_unchanged_content_leaves_the_file_byte_identical(self):
        self.run_main()
        first, mtime = self.read(), os.stat(self.out).st_mtime_ns
        for r in self.hub.repos.values():          # only a README changed: later lastModified
            r.modified += dt.timedelta(days=3)
        self.run_main()
        self.assertEqual(self.read(), first)
        self.assertEqual(os.stat(self.out).st_mtime_ns, mtime, "the file was rewritten")
        self.assertEqual(json.loads(first)["generated_at"], STAMP)

    def test_unchanged_content_leaves_a_file_in_another_layout_alone(self):
        self.run_main()
        plain = (json.dumps(json.loads(self.read()), indent=2, ensure_ascii=False) + "\n").encode()
        with open(self.out, "wb") as f:
            f.write(plain)
        self.run_main()
        self.assertEqual(self.read(), plain)

    def test_changed_content_is_written_with_the_stamp_of_the_new_state(self):
        self.run_main()
        repo = f"{gc.ORG}/other-gguf"
        self.hub.repos[repo].sha = "c" * 40
        later = dt.datetime(2026, 11, 1, 9, 15, 30, tzinfo=UTC)
        self.hub.repos[repo].modified = later
        self.run_main()
        written = json.loads(self.read())
        self.assertEqual(by_id(written["models"])[repo]["revision"], "c" * 40)
        self.assertEqual(written["generated_at"], "2026-11-01T09:15:30+00:00")
        self.assertEqual([m for m in written["models"] if not gc.is_generated(m)], self.hand)

    def test_the_stamp_is_utc_whole_seconds(self):
        self.assertEqual(gc.stamp(dt.datetime(2026, 1, 2, 3, 4, 5, 999999, tzinfo=UTC)), "2026-01-02T03:04:05+00:00")
        cet = dt.timezone(dt.timedelta(hours=1))
        self.assertEqual(gc.stamp(dt.datetime(2026, 1, 2, 4, 4, 5, tzinfo=cet)), "2026-01-02T03:04:05+00:00")
        self.assertEqual(gc.stamp(dt.datetime(2026, 1, 2, 3, 4, 5)), "2026-01-02T03:04:05+00:00")


class HeaderParsing(unittest.TestCase):
    def test_arrays_before_the_labels_are_skipped(self):
        got = gc.parse_header(asr_gguf())
        self.assertEqual(got, {"general.architecture": "whisper", "general.name": "Fake ASR",
                               "general.size_label": "0.6B"})

    def test_running_out_of_bytes_ends_the_scan_without_an_error(self):
        big = gguf([kv_strarr("tokenizer.ggml.tokens", ["abcdefgh"] * 5000),
                    kv_str("general.architecture", "whisper")])
        self.assertEqual(gc.parse_header(big[:2000]), {})

    def test_a_label_cut_in_half_is_not_taken(self):
        full = gguf([kv_str("general.architecture", "whisper-large")])
        self.assertEqual(gc.parse_header(full[:-4]), {})

    def test_no_magic_a_short_header_and_unknown_types_are_errors(self):
        for buf in (b"<html>", b"GGUF" + b"\0" * 8,
                    gguf([kv_key("a") + struct.pack("<I", 99)]),
                    gguf([kv_key("a") + struct.pack("<IIQ", 9, 99, 1)])):
            with self.subTest(buf=buf[:12]):
                with self.assertRaises(ValueError):
                    gc.parse_header(buf)

    def test_files_of_equal_size_are_ordered_by_name_whatever_the_listing_order(self):
        def sib(name):
            return types.SimpleNamespace(rfilename=name, size=10, lfs=types.SimpleNamespace(sha256="a" * 64))
        names = ["x-Q8_0.gguf", "x-F16.gguf", "x-Q5_K_M.gguf"]
        one = gc.gguf_files("r", [sib(n) for n in names])
        two = gc.gguf_files("r", [sib(n) for n in reversed(names)])
        self.assertEqual(one, two)


if __name__ == "__main__":
    unittest.main(verbosity=2)
