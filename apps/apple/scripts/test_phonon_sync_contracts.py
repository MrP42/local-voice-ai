"""Offline-S2-Vertrag; Swift-Verhalten wird separat mit XCTest geprüft."""
import hashlib
import json
from pathlib import Path
import unittest
from test_reading_page_project import selected_sources

ROOT = Path(__file__).resolve().parents[3]
CORE = ROOT / "apps/apple/VoiceCore/Sources/VoiceCore/Ablage"


def vector():
    # Kopf wie Rust sample_voice_bytes; ausschließlich synthetische Null-Tensorbytes.
    header = (b'{"speaker_wavs":{"dtype":"F32","shape":[1,512,125],'
              b'"data_offsets":[0,256000]},"__metadata__":{"sig":"ba7a5a45","epoch":"300"}}')
    header += b" " * (-len(header) % 8)
    return len(header).to_bytes(8, "little") + header + bytes(256000)


class PhononVertrag(unittest.TestCase):
    maxDiff = 300

    def test_s2_engine_einstiege_mit_gegenproben(self):
        engine = (ROOT / "apps/apple/iPhone/Library/PhononEngine.swift").read_text(encoding="utf-8")
        sync = (ROOT / "apps/apple/iPhone/Library/AblageSync.swift").read_text(encoding="utf-8")
        checks = [(engine, 'PhononStimmenInjektion.abgleichen(wurzel:'),
                  (engine, 'PhononStimmenInjektion.stimmen(in: target)'),
                  (engine, 'if generation == modelGeneration { loaded = (language, tts) }'),
                  (engine, 'loaded = nil; loading_ = nil'),
                  (engine, 'cache = SentenceAudioCache()'),
                  (engine, 'if revision == stimmenRevision { break }'),
                  (sync, 'await PhononEngine.shared.stimmenAktualisieren()')]
        for source, marker in checks:
            def check(candidate):
                self.assertTrue(marker in candidate, marker)
            check(source)
            with self.assertRaises(AssertionError):
                check(source.replace(marker, ''))
        invalidation = engine.split('private func invalidateModels()', 1)[1].split('\n    }', 1)[0]
        self.assertNotIn('shutdown', invalidation)
        self.assertNotIn('silence()', invalidation)
        self.assertNotIn('loading_?.task.cancel()', invalidation)
        read = engine.split('func read(voice name:', 1)[1].split('/// Erzeugen und Einreihen', 1)[0]
        def check_cache(source):
            self.assertLess(source.index('let cache = self.cache'), source.index('try await engine('))
            self.assertIn('cache: cache, next:', source)
        check_cache(read)
        with self.assertRaises(ValueError):
            check_cache(read.replace('let cache = self.cache', ''))

    def test_s2_minimalkopf(self):
        header = b'{"speaker_wavs":{"dtype":"F32","shape":[1,512,125],"data_offsets":[0,256000]}}'
        self.assertEqual(len(header), 78)
        self.assertEqual(8 + len(header) + (-len(header) % 8) + 256000, 256088)

    def test_s2_projektquellen_mit_gegenprobe(self):
        source = (ROOT / "apps/apple/scripts/generate_project.py").read_text(encoding="utf-8")
        for name in ['AblageSystem', 'PhononStimme', 'PhononStimmenSpeicher', 'PhononStimmenInjektion']:
            path = 'VoiceCore/Sources/VoiceCore/Ablage/' + name + '.swift'
            self.assertEqual(selected_sources(source, 'iphoneos').count(path), 1)
            self.assertNotIn(path, selected_sources(source, 'watchos'))
            with self.assertRaises(AssertionError):
                self.assertIn(path, selected_sources(source.replace("'" + name + "'", "'Entfernt'"), 'iphoneos'))

    def test_vektor(self):
        sha = hashlib.sha256(vector()).hexdigest()
        self.assertEqual(len(vector()), 256136)
        for path in [ROOT / "apps/apple/VoiceCore/Tests/VoiceCoreTests/PhononStimmenTests.swift",
                     ROOT / "apps/local-voice/src-tauri/src/sync/phonon_tests.rs"]:
            self.assertIn(sha, path.read_text(encoding="utf-8"))
        swift = (ROOT / "apps/apple/VoiceCore/Tests/VoiceCoreTests/PhononStimmenTests.swift").read_text(encoding="utf-8")
        swift_header = swift.split('static let header = #"', 1)[1].split('"#', 1)[0]
        rust = (ROOT / "apps/local-voice/src-tauri/src/managers/tts/phonon_voices.rs").read_text(encoding="utf-8")
        template = rust.split('pub(crate) fn sample_voice_bytes', 1)[1].split('r#"', 1)[1].split('"#', 1)[0]
        rust_header = template.replace('{t}', '125').replace('{}', '256000').replace('{{', '{').replace('}}', '}')
        self.assertEqual(swift_header, rust_header)
        length = int.from_bytes(vector()[:8], 'little')
        self.assertEqual(vector()[8:8 + length].rstrip(b' '), swift_header.encode())

    def test_einstieg_mit_gegenprobe(self):
        text = (CORE / "AblageLauf.swift").read_text(encoding="utf-8")
        line = "try await PhononStimmenSpeicher.abgleichen("
        def check(source):
            self.assertIn(line, source)
            self.assertLess(source.index("ordnergraeber(zustand:"), source.index(line))
            self.assertLess(source.index(line), source.index("try await herunterladen("))
        check(text)
        with self.assertRaises(AssertionError):
            check(text.replace(line, ""))

    def test_sicht_mit_gegenprobe(self):
        text = (CORE / "AblageAnzeige.swift").read_text(encoding="utf-8")
        def check(source):
            self.assertIn("AblageSystem.datei", source)
            self.assertIn("AblageSystem.ordner", source)
        check(text)
        with self.assertRaises(AssertionError):
            check(text.replace("AblageSystem.", "Entfernt."))

    def test_lf_und_keine_logs(self):
        paths = [CORE / name for name in ["AblageSystem.swift", "PhononStimme.swift", "PhononStimmenSpeicher.swift",
                                         "AblageAnzeige.swift", "AblageDownloads.swift", "AblageLauf.swift", "AblageSpeicher.swift"]]
        paths += [CORE / "PhononStimmenInjektion.swift", ROOT / "apps/apple/iPhone/Library/PhononEngine.swift",
                  ROOT / "apps/apple/scripts/generate_project.py", ROOT / "apps/apple/iPhone/Library/AblageSync.swift",
                  ROOT / "apps/apple/VoiceCore/Tests/VoiceCoreTests/PhononStimmenTests.swift",
                  ROOT / "apps/local-voice/src-tauri/src/sync/phonon_tests.rs", Path(__file__)]
        for path in paths:
            data = path.read_bytes()
            self.assertFalse(b"\r" in data, str(path.relative_to(ROOT)))
        for name in ["AblageSystem.swift", "PhononStimme.swift", "PhononStimmenSpeicher.swift"]:
            data = (CORE / name).read_bytes()
            self.assertNotIn(b"print(", data)
            self.assertNotIn(b"Logger", data)

    def test_katalogreservierung_mit_gegenprobe(self):
        text = (CORE / "PhononStimmenSpeicher.swift").read_text(encoding="utf-8")
        entries = json.loads((ROOT / "apps/local-voice/src-tauri/src/catalog/catalog.json").read_text(encoding="utf-8"))["models"]
        expected = set()
        for e in entries:
            if e.get("purpose") == "phonon-voice":
                expected.add(Path(e["voice"]["filename"]).stem.lower())
                expected.add(e["voice"]["name"].replace(" ", "-").lower())
        def check(source):
            actual = set(source.split('static let katalogNamen = """', 1)[1].split('"""', 1)[0].lower().split())
            self.assertEqual(expected, actual)
        check(text)
        with self.assertRaises(AssertionError):
            check(text.replace("Nele ", ""))

    def test_pruefregeln_und_atomare_reihenfolge_mit_gegenproben(self):
        validation = (CORE / "PhononStimme.swift").read_text(encoding="utf-8")
        storage = (CORE / "PhononStimmenSpeicher.swift").read_text(encoding="utf-8")
        checks = [
            (validation, '(minBytes...maxBytes).contains(bytes.count)'),
            (validation, 'AblageKrypto.sha256Hex(bytes) == sha256'),
            (validation, 'shape == [1, 512, 125]'),
            (validation, 'guard s.0 == end'),
            (validation, 'idGueltig(id), idGueltig(nameID)'),
            (validation, 'd.id == AblageSystem.kennung'),
            (validation, 'if updated != b.updated'),
            (validation, 'if fassung.blob != b.fassung.blob'),
        ]
        for source, marker in checks:
            def check(candidate):
                self.assertTrue(marker in candidate, marker)
            check(source)
            with self.assertRaises(AssertionError):
                check(source.replace(marker, ""))
        def atomic(source):
            self.assertLess(source.index('try Data(bytes).write(to: dest, options: .atomic)'),
                            source.index('try encoder.encode(idx).write'))
            self.assertLess(source.index('try encoder.encode(idx).write'), source.index('try fm.removeItem(at: url)'))
        atomic(storage)
        with self.assertRaises(ValueError):
            atomic(storage.replace('try Data(bytes).write(to: dest, options: .atomic)', ''))

    def test_kontowechsel_abmelden_und_originalschutz_mit_gegenproben(self):
        sources = [
            ((CORE / "AblageSpeicher.swift").read_text(encoding="utf-8"),
             'try? PhononStimmenInjektion.abgleichen(wurzel: stimmenWurzel)'),
            ((CORE / "AblageDownloads.swift").read_text(encoding="utf-8"),
             'guard !AblageSystem.datei(dat, in: z.bestand)'),
            ((ROOT / "apps/apple/iPhone/Library/AblageSync.swift").read_text(encoding="utf-8"),
             'AblageSpeicher(wurzel: root, stimmenWurzel: stimmen)'),
        ]
        for source, marker in sources:
            def check(candidate):
                self.assertTrue(marker in candidate, marker)
            check(source)
            with self.assertRaises(AssertionError):
                check(source.replace(marker, ''))


if __name__ == "__main__":
    unittest.main()
