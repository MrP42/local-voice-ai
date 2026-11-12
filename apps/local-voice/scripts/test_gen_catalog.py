"""Offline tests for gen_catalog.py: what a generator run keeps and what it corrects.

No network and no huggingface_hub needed (the module connects lazily in main()).
Run:  python scripts/test_gen_catalog.py     (or: bun run test:catalog)
"""
import copy
import importlib.util
import json
import os
import sys
import tempfile
import types
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("gen_catalog", os.path.join(HERE, "gen_catalog.py"))
gc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gc)

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
            gc.write_catalog(out, generated, gc.CATALOG_PATH)
            with open(out, encoding="utf-8") as f:
                text = f.read()
            written = json.loads(text)
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
                gc.write_catalog(out, broken, gc.CATALOG_PATH)
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
            gc.build = lambda repo: copy.deepcopy(generated[repo])
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


if __name__ == "__main__":
    unittest.main(verbosity=2)
