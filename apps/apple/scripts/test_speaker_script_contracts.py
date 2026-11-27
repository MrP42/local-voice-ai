"""Offline-Integrationsverträge mit Gegenproben; Swift-Verhalten prüft das Mac-Tor."""
import json
from pathlib import Path
import unittest
from test_reading_page_project import selected_sources

ROOT = Path(__file__).resolve().parents[1]


class SpeakerScriptContracts(unittest.TestCase):
    def test_project_membership_with_counterprobe(self):
        source = (ROOT / "scripts/generate_project.py").read_text(encoding="utf-8")
        for path in ["VoiceCore/Sources/VoiceCore/SpeakerScript.swift",
                     "VoiceCore/Sources/VoiceCore/SpeakerScriptTranslation.swift"]:
            self.assertTrue(path in selected_sources(source, "iphoneos"), path)
            self.assertNotIn(path, selected_sources(source, "watchos"))
            with self.assertRaises(AssertionError):
                self.assertTrue(path in selected_sources(source.replace(path, "removed.swift"), "iphoneos"), path)

    def test_playback_contracts_with_counterprobes(self):
        checks = {
            "iPhone/Library/LibraryPlayback.swift": ["scriptPresentation(inParagraph", "scriptSentence(text:", "speakSystemScript()"],
            "iPhone/Library/PhononEngine.swift": ["var voice: String?", "item.parts", "cache.samples(part.voice, sentence)", "tts.setVoice(parts.voice)",
                                                  "guard readers == 0 else { return }", "defer { readers -= 1 }", "await preparing?.value",
                                                  "SpeakerScript.maskedControls(trimmed)"],
            "iPhone/Library/LiveTranslationApple.swift": ["SpeakerScript.translate(texts", "speakers: speakers"],
        }
        for path, tokens in checks.items():
            source = (ROOT / path).read_text(encoding="utf-8")
            for token in tokens:
                with self.subTest(path=path, token=token):
                    self.assertTrue(token in source, token)
                    with self.assertRaises(AssertionError):
                        self.assertTrue(token in source.replace(token, "REMOVED"), token)

    def test_golden_offsets_are_utf16_and_ranges_are_literal(self):
        golden = json.loads((ROOT / "tests/speaker-script-golden.json").read_text(encoding="utf-8"))
        for case in golden["cases"]:
            raw = case["text"].encode("utf-16-le")
            for start, end in case["expected"]["tags"]:
                tag = raw[start * 2:end * 2].decode("utf-16-le")
                self.assertTrue(tag.startswith("[") and tag.endswith("]"))
            for marker in case["expected"]["markers"]:
                text = raw[marker["start"] * 2:marker["end"] * 2].decode("utf-16-le")
                self.assertTrue(text.startswith("<") and text.endswith(">"))


if __name__ == "__main__":
    unittest.main()
