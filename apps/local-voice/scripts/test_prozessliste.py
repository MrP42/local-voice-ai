"""Offline source gate for E1; --baseline checks the unchanged HEAD as counterprobe."""
import pathlib
import re
import subprocess
import sys
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1] / "src-tauri"
BASELINE = "--baseline" in sys.argv
if BASELINE:
    sys.argv.remove("--baseline")


def source(name):
    path = ROOT / name
    if BASELINE:
        result = subprocess.run(
            ["git", "show", "HEAD:" + path.relative_to(ROOT.parents[2]).as_posix()],
            capture_output=True, check=False,
        )
        return result.stdout.decode("utf-8")
    return path.read_text(encoding="utf-8") if path.exists() else ""


def windows_tokens(text):
    # Auxiliary source check only; the Rust gate owns the full Rust tokenizer.
    parts = re.findall(
        r'//[^\n]*|/\*[\s\S]*?\*/|r#*"[\s\S]*?"#*|"(?:\\.|[^"\\])*"'
        r"|'(?:\\.|[^'\\])'|[A-Za-z_]\w*|[^\s]", text,
    )
    parts = [p for p in parts if not p.startswith(("//", "/*"))]
    result = []
    index = 0
    excluded = (
        ["#", "[", "cfg", "(", "not", "(", "windows", ")", ")", "]"],
        ["#", "[", "cfg", "(", "test", ")", "]"],
    )
    while index < len(parts):
        attribute = next((p for p in excluded if parts[index:index + len(p)] == p), None)
        if attribute:
            index += len(attribute)
            depth = 0
            while index < len(parts):
                token = parts[index]
                index += 1
                if token in ("(", "[", "{"):
                    depth += 1
                elif token in (")", "]", "}"):
                    depth -= 1
                    if depth == 0 and token == "}":
                        break
                elif depth == 0 and token in (";", ","):
                    break
        else:
            result.append(parts[index])
            index += 1
    return result


class ProcessSourceGate(unittest.TestCase):
    def test_windows_callers_use_the_process_module(self):
        for name in ("managers/llm/app_usage.rs", "managers/meeting_detect/mod.rs",
                     "managers/meetings/search/indexer.rs", "media.rs",
                     "managers/meetings/slides/test_support.rs", "managers/meetings/pdf.rs"):
            with self.subTest(file=name):
                self.assertTrue("crate::prozessliste::" in source("src/" + name), name)

    def test_only_limited_query_access_and_no_memory_read_api(self):
        text = source("src/prozessliste.rs")
        self.assertIn("TH32CS_SNAPPROCESS", text)
        self.assertIn("K32GetProcessMemoryInfo", text)
        self.assertRegex(text, r"OpenProcess\(PROCESS_QUERY_LIMITED_INFORMATION, false, pid\)")
        for name in ("PROCESS_VM_READ", "PROCESS_QUERY_INFORMATION", "ReadProcessMemory",
                     "NtQueryInformationProcess", "refresh_processes"):
            self.assertNotIn(name, text)

    def test_memory_uses_validated_owned_handle(self):
        text = source("src/prozessliste.rs")
        self.assertIn("GetProcessTimes", text)
        self.assertIn("child_belongs", text)
        self.assertIn("same_candidate(row, now)", text)
        self.assertRegex(text, r"impl OwnedProcess[\s\S]*fn memory_bytes\(&self\)")
        self.assertIn("own_tree()?", text)
        usage = source("src/managers/llm/app_usage.rs")
        self.assertTrue("let pids = fill_memory(&mut rows, &pids);" in usage)

    def test_windows_production_has_no_sysinfo_process_refresh(self):
        for name in ("managers/llm/app_usage.rs", "managers/meeting_detect/mod.rs",
                     "managers/meetings/search/indexer.rs"):
            with self.subTest(file=name):
                tokens = windows_tokens(source("src/" + name))
                forbidden = {"ProcessesToUpdate", "refresh_processes", "refresh_processes_specifics"}
                self.assertFalse(forbidden.intersection(tokens), name)

    def test_windows_test_helpers_no_longer_read_foreign_command_lines(self):
        for name in ("media.rs", "managers/meetings/pdf.rs", "managers/meetings/slides/test_support.rs"):
            with self.subTest(file=name):
                text = source("src/" + name)
                self.assertFalse("refresh_processes_specifics" in text, name)
                self.assertFalse("with_cmd" in text, name)


if __name__ == "__main__":
    unittest.main()
