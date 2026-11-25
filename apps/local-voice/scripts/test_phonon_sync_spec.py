"""Offline-Vertragsprüfung für S1; ersetzt keine Rust-Verhaltenstests."""
import hashlib
import json
from pathlib import Path
import subprocess
import unittest


ROOT = Path(__file__).resolve().parents[3]
SPEC = "docs/superpowers/specs/2026-11-08-dokumentenablage.md"


def sichtvertrag(text):
    start = text.index("**Abschirmung.**")
    end = text.index("**Prüfstand.**", start)
    contract = " ".join(text[start:end].split())
    assert "`Ablage::lage` und `lage_intern` liefern den vollständigen internen Bestand" in contract
    assert "Filterung geschieht in den jeweiligen Benutzersichten" in contract
    assert "Neue Leser müssen diese Filter ausdrücklich übernehmen" in contract
    assert "projiziert den Benutzerbestand zentral" not in contract


class Stimmenvertrag(unittest.TestCase):
    def test_erster_empfang_hat_elternordner_mit_gegenprobe(self):
        path = ROOT / "apps/local-voice/src-tauri/src/managers/tts/phonon_voices/sync.rs"
        mkdir = "fs::create_dir_all(self.store_dir()).map_err(|e| io_err(&e))?;"

        def vertrag(bridge):
            install = bridge.split("let mut voices = idx.voices.clone();", 1)[1]
            self.assertIn(mkdir, install)
            self.assertLess(install.index(mkdir), install.index("write_atomic(&self.receive_path(uid), bytes)?;"))

        current = path.read_text(encoding="utf-8")
        vertrag(current)
        with self.assertRaises(AssertionError):
            vertrag(current.replace(mkdir, ""))

    def test_metadatenfixture_setzt_updated_wie_app_mit_gegenprobe(self):
        path = ROOT / "apps/local-voice/src-tauri/src/sync/phonon_tests.rs"
        update = 'v["voices"][0]["updated"] = serde_json::json!(updated + 1);'

        def vertrag(tests):
            fixture = tests.split("fn lokale_metadatenaenderung_behaelt_ihre_uid()", 1)[1].split("#[test]", 1)[0]
            self.assertIn(update, fixture)
            self.assertLess(fixture.index(update), fixture.index("index_schreiben(&sb, &v)"))

        current = path.read_text(encoding="utf-8")
        vertrag(current)
        with self.assertRaises(AssertionError):
            vertrag(current.replace(update, ""))

    def test_lww_bereitstellen_mit_gegenprobe(self):
        path = "apps/local-voice/src-tauri/src/managers/tts/phonon_voices/sync.rs"
        baseline = subprocess.run(["git", "show", f"840a3877:{path}"], cwd=ROOT,
                                  check=True, capture_output=True).stdout.decode("utf-8")

        def inhaltsvertrag(bridge):
            self.assertIn("l.updated != zeit(&r) || l.sha256 != sha", bridge)
            self.assertNotIn("zeit(&r).max(u.checked_add(1)", bridge)

        def generationsvertrag(bridge):
            self.assertIn("let mut hub_rank = None;", bridge)
            self.assertIn("rank <= (zeit(&r), sha.as_str())", bridge)
            self.assertNotIn("if unchanged && old.is_some()", bridge)

        for vertrag in [inhaltsvertrag, generationsvertrag]:
            with self.subTest(vertrag=vertrag.__name__):
                vertrag((ROOT / path).read_text(encoding="utf-8"))
                with self.assertRaises(AssertionError):
                    vertrag(baseline)

    def test_ios_gewinnerregel_an_generationsstelle_mit_gegenprobe(self):
        def vertrag(text):
            stelle = " ".join(text.split("Dateien verschiedener lebender Generationen", 1)[1]
                               .split("Alte Clients", 1)[0].split())
            self.assertIn("höchste `updated`", stelle)
            self.assertIn("lexikografisch größte `sha256`", stelle)
            self.assertIn("Generation dient nur zur Ortsbestimmung", stelle)
            self.assertNotIn("gewinnt die höchste Generation", stelle)
        vertrag((ROOT / SPEC).read_text(encoding="utf-8"))
        baseline = subprocess.run(["git", "show", f"840a3877:{SPEC}"], cwd=ROOT,
                                  check=True, capture_output=True).stdout.decode("utf-8")
        with self.assertRaises(AssertionError):
            vertrag(baseline)

    def pruefe_s1a_fix_mit_gegenprobe(self, vertrag):
        paths = ["apps/local-voice/src-tauri/src/managers/tts/phonon_voices" + suffix
                 for suffix in [".rs", "/sync.rs"]]
        current = [(ROOT / p).read_text(encoding="utf-8") for p in paths]
        baseline = [subprocess.run(["git", "show", f"ad72cbe27dd855f37f084bd7ec7ec6a1ae5ba683:{p}"],
                    cwd=ROOT, check=True, capture_output=True).stdout.decode("utf-8") for p in paths]
        vertrag(*current)
        with self.assertRaises(AssertionError):
            vertrag(*baseline)

    def test_s1a_journal_quellen_mit_gegenprobe(self):
        def vertrag(store, bridge):
            resolver = bridge.split("fn projected_bytes(", 1)
            self.assertEqual(len(resolver), 2, "gemeinsame SHA-geprüfte Master-/Staging-Auflösung fehlt")
            body = resolver[1].split("pub(super) fn finish_receive", 1)[0]
            self.assertLess(body.index("master_path"), body.index("receive_path"))
            self.assertIn("sha256_hex", body)
            fallback = store.split("fn best_effort_index", 1)[1].split("fn check_locked", 1)[0]
            self.assertIn("p.voices.clone()", fallback)
            self.assertIn("projected_bytes", fallback)
            self.assertIn("sync_receive.is_none()", fallback)
            sync = store.split("pub fn sync(&self)", 1)[1]
            self.assertIn("best_effort_index", sync)
            self.assertIn("projected_bytes", sync)
            installs = bridge.split("// Alle Quellen liegen vor dem Journal", 1)[1].split("let p = idx.sync_receive.take()", 1)[0]
            self.assertLess(installs.index("installs.sort_by_key"), installs.index("for r in installs"))
            self.assertLess(installs.index("for r in installs"), installs.index("write_atomic"))
        self.pruefe_s1a_fix_mit_gegenprobe(vertrag)

    def test_s1a_unabhaengige_uids_mit_gegenprobe(self):
        def uidvertrag(store, bridge):
            for removed in ["sync_aliases", "sync_removed_initial", "RemovedInitial", "sync_linked", "merged:", "linked:"]:
                self.assertTrue(removed not in store + bridge, removed)

        def lokalvertrag(store, bridge):
            self.assertIn("(*updated, m.sha256.as_str()) >= (zeit(r), sha.as_str())", bridge)
            self.assertIn("l.snapshot == snapshot(&r, &sha)", bridge)
            self.assertIn("*updated >= zeit(&r) && link.is_none_or", bridge)

        def rangvertrag(store, bridge):
            self.assertIn("(updated, &m.sha256)", bridge)
            self.assertIn("(a.zeit, &a.blob, &a.fas)", bridge)

        paths = ["apps/local-voice/src-tauri/src/managers/tts/phonon_voices" + suffix
                 for suffix in [".rs", "/sync.rs"]]
        current = [(ROOT / p).read_text(encoding="utf-8") for p in paths]
        baseline = [subprocess.run(["git", "show", f"a2ef4a0b:{p}"], cwd=ROOT,
                    check=True, capture_output=True).stdout.decode("utf-8") for p in paths]
        for vertrag in [uidvertrag, lokalvertrag, rangvertrag]:
            with self.subTest(vertrag=vertrag.__name__):
                vertrag(*current)
                with self.assertRaises(AssertionError):
                    vertrag(*baseline)

    def test_ios_nutzlastvektor_ohne_zusammenfuehrung_mit_gegenprobe(self):
        def vertrag(spec):
            vector = spec.split("Die zusätzliche Eigenschaft `phonon_voice` ist ein Array:", 1)[1]
            rank, meta = json.loads(vector.split("```json", 1)[1].split("```", 1)[0])
            self.assertEqual(len(rank), 20)
            self.assertEqual(set(meta), {"id", "uid", "name", "language", "origin", "consent_at",
                                        "voice_id", "source_note", "created", "sha256", "fas", "generation"})
        vertrag((ROOT / SPEC).read_text(encoding="utf-8"))
        baseline = subprocess.run(["git", "show", f"a2ef4a0b:{SPEC}"], cwd=ROOT,
                                  check=True, capture_output=True).stdout.decode("utf-8")
        with self.assertRaises(AssertionError):
            vertrag(baseline)

    def test_s1a_entfernen_atom_fortsetzen_mit_gegenprobe(self):
        def vertrag(store, bridge):
            remove = store.split("fn remove_locked", 1)[1].split("pub fn sync", 1)[0]
            self.assertEqual(remove.count("self.write_index(&idx)?"), 1)
            self.assertLess(remove.index("idx.voices.retain"), remove.index("self.write_index(&idx)?"))
            read = store.split("fn read_index", 1)[1].split("fn rebuild_index", 1)[0]
            self.assertIn("idx.voices.retain(|r| !idx.sync_removed.contains(&r.uid))", read)
            cleanup = bridge.split("fn finish_model_changes", 1)[1].split("fn name_blocked", 1)[0]
            self.assertIn("fs::remove_file(&master)", cleanup)
        self.pruefe_s1a_fix_mit_gegenprobe(vertrag)

    def test_review_fixes_mit_gegenprobe(self):
        paths = ["apps/local-voice/src-tauri/src/managers/tts/phonon_voices" + suffix
                 for suffix in [".rs", "/sync.rs"]]

        def modellvertrag(store, bridge):
            self.assertTrue("sync_models_open" in store)
            self.assertTrue("finish_model_changes" in bridge)
            self.assertTrue("best_effort_voices" in store)

        current = [(ROOT / p).read_text(encoding="utf-8") for p in paths]
        baseline = [subprocess.run(["git", "show", f"548fe0ec:{p}"], cwd=ROOT,
                    check=True, capture_output=True).stdout.decode("utf-8") for p in paths]
        for vertrag in [modellvertrag]:
            with self.subTest(fix=vertrag.__name__):
                vertrag(*current)
                with self.assertRaises(AssertionError):
                    vertrag(*baseline)

    def test_s1a_ohne_loeschweitergabe_mit_gegenprobe(self):
        def vertrag(bridge, store, transport):
            for forbidden in ["remove_received", "Eingang::Grab", "sync_pending", "phonon_pull_offen"]:
                self.assertTrue(forbidden not in bridge, forbidden)
            for forbidden in ["sync_pending", "struct Grave", "remove_received"]:
                self.assertTrue(forbidden not in store + bridge, forbidden)
            for forbidden in ["phonon_pull_offen", "phonon_grab_", "phonon_dateiordner"]:
                self.assertTrue(forbidden not in transport, forbidden)
            self.assertIn("sync_removed", store)
            self.assertIn('"-eigen"', bridge)

        paths = ["apps/local-voice/src-tauri/src/" + p for p in [
            "managers/tts/phonon_voices/sync.rs", "managers/tts/phonon_voices.rs", "sync/ablage.rs"]]
        vertrag(*[(ROOT / p).read_text(encoding="utf-8") for p in paths])
        baseline = [subprocess.run(["git", "show", f"972b5e13:{p}"], cwd=ROOT,
                    check=True, capture_output=True).stdout.decode("utf-8") for p in paths]
        with self.assertRaises(AssertionError):
            vertrag(*baseline)

    def test_sichten_filtern_und_lage_bleibt_intern_mit_gegenprobe(self):
        sichtvertrag((ROOT / SPEC).read_text(encoding="utf-8"))
        baseline = subprocess.run(
            ["git", "show", f"c4da4e85a029db9677597188878e932203e85a67:{SPEC}"],
            cwd=ROOT, check=True, capture_output=True,
        ).stdout.decode("utf-8")
        with self.assertRaises(AssertionError):
            sichtvertrag(baseline)

    def test_ios_kennungsvektoren(self):
        alphabet = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
        spec = (ROOT / SPEC).read_text(encoding="utf-8")
        for folder, generation, uid, expected in [
            (True, 0, "", "ord_2CAS9MY90GTRQEBVKFKP5TXZAB"),
            (True, 1, "", "ord_2CAS9MYNV00X4X0TY6MQE0ECCH"),
            (False, 0, "00112233445566778899aabbccddeeff", "dat_2CAS9MD268N1T7K9V0N4GCDVT8"),
        ]:
            digest = hashlib.sha256(bytes(range(32)) + b"lv-phonon-sync-v1"
                                    + bytes([folder]) + generation.to_bytes(4, "big") + uid.encode()).digest()
            raw = (b"LVSO" if folder else b"LVSF") + digest[:12]
            number = int.from_bytes(raw, "big")
            encoded = "".join(alphabet[(number >> shift) & 31] for shift in range(125, -1, -5))
            self.assertEqual(("ord_" if folder else "dat_") + encoded, expected)
            self.assertIn(expected, spec)

    def test_geaenderte_dateien_haben_lf(self):
        for relative in [
            SPEC,
            "apps/local-voice/scripts/test_phonon_sync_spec.py",
            "apps/local-voice/src-tauri/src/managers/tts/phonon_voices.rs",
            "apps/local-voice/src-tauri/src/managers/tts/phonon_voices/sync.rs",
            "apps/local-voice/src-tauri/src/sync/ablage.rs",
            "apps/local-voice/src-tauri/src/sync/phonon_tests.rs",
            "apps/local-voice/src-tauri/src/sync/phonon_regression_tests.rs",
        ]:
            with self.subTest(file=relative):
                self.assertNotIn(b"\r", (ROOT / relative).read_bytes())


if __name__ == "__main__":
    unittest.main()
