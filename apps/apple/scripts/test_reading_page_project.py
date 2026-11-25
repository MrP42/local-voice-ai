"""Offline: exercise the generator's source selection without writing a project.

Swift behavior belongs to ReadingPageTests (Mac gate). These tests cover only
target membership, including counterprobes that remove either new source entry.
"""
import ast
import pathlib
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
GENERATOR = ROOT / "scripts" / "generate_project.py"
REQUIRED = {
    "VoiceCore/Sources/VoiceCore/ReadingPage.swift",
    "VoiceCore/Sources/VoiceCore/ReadingPageStore.swift",
    "VoiceCore/Sources/VoiceCore/ReadingPageEditing.swift",
    "iPhone/Library/ReadingPagesModel.swift",
}


def selected_sources(source, platform):
    tree = ast.parse(source)
    target_loop = next(
        node for node in tree.body
        if isinstance(node, ast.For) and "VoicePhone" in ast.unparse(node.iter)
    )
    statements = []
    for node in target_loop.body:
        if isinstance(node, ast.Assign) and any(
            isinstance(target, ast.Name) and target.id == "buildfiles"
            for target in node.targets
        ):
            break
        statements.append(node)
    namespace = {
        "platform": platform,
        "ASSISTANT_IPHONE": [], "MODELS_CORE": [], "MODELS_IPHONE": [],
    }
    exec(compile(ast.Module(body=statements, type_ignores=[]), str(GENERATOR), "exec"), namespace)
    return namespace["sources"]


class ReadingPageProjectTests(unittest.TestCase):
    def test_iphone_includes_both_sources_once_and_watch_excludes_them(self):
        source = GENERATOR.read_text(encoding="utf-8")
        iphone = selected_sources(source, "iphoneos")
        watch = selected_sources(source, "watchos")
        for path in REQUIRED:
            self.assertEqual(iphone.count(path), 1)
            self.assertNotIn(path, watch)
            self.assertTrue((ROOT / path).is_file())

    def test_counterprobes_detect_either_missing_source(self):
        source = GENERATOR.read_text(encoding="utf-8")
        for old, new in [("'ReadingPage',", ""), (",'ReadingPageStore'", ""),
                         (",'ReadingPageEditing'", ""), ("'iPhone/Library/ReadingPagesModel.swift'", "")]:
            with self.subTest(entry=old):
                self.assertIn(old, source)
                mutated = selected_sources(source.replace(old, new, 1), "iphoneos")
                with self.assertRaises(AssertionError):
                    self.assertTrue(REQUIRED.issubset(mutated))


if __name__ == "__main__":
    unittest.main()
