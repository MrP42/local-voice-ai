"""Offline-Vertragsprüfung für S1; ersetzt keine Rust-Verhaltenstests."""
import hashlib
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
