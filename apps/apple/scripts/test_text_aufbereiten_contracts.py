"""Offline wiring guards; Swift behavior runs at the Mac gate. Synthetic counterprobes."""
import pathlib
import re
import unittest

from test_reading_page_project import selected_sources

ROOT = pathlib.Path(__file__).resolve().parents[1]


def model_guard(source):
    generate = source.split("func aufbereitenVorschlaege(", 1)[1].split("func uebernehmen(", 1)[0]
    assert "try flushText(pageID)" in generate
    assert "Task.detached" in generate
    assert "TextAufbereiten.vorschlaege" in generate
    apply = source.split("func uebernehmen(", 1)[1].split("func moveCaptures(", 1)[0]
    assert apply.index("try flushText(pageID)") < apply.index("e.page(pageID)")
    assert "Task.detached" in apply
    assert apply.count("try flushText(pageID)") == 2
    assert "TextAufbereiten.identisch" in apply
    assert apply.index("TextAufbereiten.identisch") < apply.index("e.applyPreparedText")
    assert "changed()" in apply
    assert "import VoiceCore" not in source


class AufbereitenContracts(unittest.TestCase):
    def test_swift_delimiters_and_lf(self):
        paths = list((ROOT / "VoiceCore/Sources/VoiceCore").glob("TextAufbereiten*.swift"))
        paths.append(ROOT / "VoiceCore/Tests/VoiceCoreTests/TextAufbereitenTests.swift")
        def balanced(source):
            tokens = re.sub(r'//[^\n]*|#".*?"#|"(?:\\.|[^"\\])*"', '', source, flags=re.S)
            stack = []
            for char in tokens:
                if char in "{([": stack.append(char)
                elif char in "})]":
                    assert stack and stack.pop() == {"}": "{", ")": "(", "]": "["}[char]
            assert not stack
        for path in paths:
            with self.subTest(path=path.name):
                self.assertNotIn(b"\r", path.read_bytes())
                source = path.read_text(encoding="utf-8")
                balanced(source)
                with self.assertRaises(AssertionError):
                    balanced(source + "}")

    def test_model_and_counterprobes(self):
        source = (ROOT / "iPhone/Library/ReadingPagesModel.swift").read_text(encoding="utf-8")
        model_guard(source)
        for token in ["Task.detached", "try flushText(pageID)", "TextAufbereiten.identisch", "e.applyPreparedText"]:
            with self.subTest(token=token), self.assertRaises((AssertionError, IndexError, ValueError)):
                model_guard(source.replace(token, "REMOVED"))

    def test_store_guard_and_counterprobe(self):
        source = (ROOT / "VoiceCore/Sources/VoiceCore/ReadingPageEditing.swift").read_text(encoding="utf-8")
        def guard(s):
            body = s.split("public func applyPreparedText(", 1)[1].split("public func insert(", 1)[0]
            assert "TextAufbereiten.identisch(page.text, basis)" in body
            assert body.index("TextAufbereiten.identisch(page.text, basis)") < body.index("try updateText")
            assert "throw TextAufbereiten.Fehler.veraltet" in body
            assert "await " not in body
        guard(source)
        with self.assertRaises(AssertionError):
            guard(source.replace("TextAufbereiten.identisch(page.text, basis)", "true"))

    def test_target_membership_and_counterprobes(self):
        source = (ROOT / "scripts/generate_project.py").read_text(encoding="utf-8")
        for name in ["TextAufbereiten", "TextAufbereitenPaste", "TextAufbereitenFormat"]:
            path = f"VoiceCore/Sources/VoiceCore/{name}.swift"
            self.assertEqual(selected_sources(source, "iphoneos").count(path), 1)
            self.assertNotIn(path, selected_sources(source, "watchos"))
            self.assertTrue((ROOT / path).is_file())
            with self.assertRaises(AssertionError):
                self.assertIn(path, selected_sources(source.replace(f"'{name}'", "'Removed'"), "iphoneos"))


if __name__ == "__main__":
    unittest.main()
