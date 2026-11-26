"""Offline-Gegenproben; Rust/XCTest laufen im jeweiligen Build-Thread."""
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[3]
RUST = 'apps/local-voice/src-tauri/src/managers/tts/phonon_voices/sync.rs'
SWIFT = 'apps/apple/VoiceCore/Sources/VoiceCore/Ablage/PhononStimme.swift'


def contract(test, rust, swift):
    test.assertTrue('self.emb_bereitstellen(a, &idx, konto.key)?;' in rust)
    test.assertIn('entry.key == "emb"', swift)
    test.assertIn('shape[1] <= 1024', swift)
    test.assertIn('static func embedding(', swift)
    test.assertIn('source_sha256 == k.fassung.blob', swift)


class EmbeddingContract(unittest.TestCase):
    def test_lf_and_projection_wiring_with_counterexamples(self):
        base = ROOT / 'apps/local-voice/src-tauri/src/managers/tts/phonon_voices'
        source = (base / 'embedding.rs').read_text(encoding='utf-8')
        def check(s):
            for marker in ['w[o * c + i] * x[i * t + time]', 'out[time * c + o] = sum',
                           'matches!(typ, 0 | 30)', '512, 512', 'start.checked_add(end)',
                           'tensor["shape"][2]', 'v.to_le_bytes()']:
                self.assertTrue(marker in s, marker)
        check(source)
        with self.assertRaises(AssertionError):
            check(source.replace('w[o * c + i]', 'w[i * c + o]'))
        with self.assertRaises(AssertionError):
            check(source.replace('x[i * t + time]', 'x[time * c + i]'))
        for p in [base / 'embedding.rs', base / 'embedding_tests.rs', Path(__file__)]:
            self.assertNotIn(b'\r', p.read_bytes(), str(p.relative_to(ROOT)))

    def test_implementation_and_baseline_counterexample(self):
        rust = (ROOT / RUST).read_text(encoding='utf-8')
        swift = (ROOT / SWIFT).read_text(encoding='utf-8')
        contract(self, rust, swift)
        for marker in ['entry.key == "emb"', 'shape[1] <= 1024', 'source_sha256 == k.fassung.blob']:
            with self.assertRaises(AssertionError):
                contract(self, rust, swift.replace(marker, 'entfernt'))
        baseline = [subprocess.run(['git', 'show', 'HEAD:' + p], cwd=ROOT,
                                  check=True, capture_output=True).stdout.decode('utf-8') for p in [RUST, SWIFT]]
        with self.assertRaises(AssertionError):
            contract(self, *baseline)


if __name__ == '__main__':
    unittest.main()
