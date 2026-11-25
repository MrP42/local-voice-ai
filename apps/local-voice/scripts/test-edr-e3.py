"""Offline regression gate for E3; runtime Win32 assertions live in Rust tests."""
import pathlib
import re
import subprocess
import sys
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
SRC = ROOT / "src-tauri/src"
BASELINE = "--baseline" in sys.argv
if BASELINE:
    sys.argv.remove("--baseline")


def production(text):
    # The affected files keep their test modules after production code.
    return re.split(r"#\[cfg\(test\)\]\s*(?:pub(?:\([^)]*\))?\s+)?mod\s", text)[0]


def windows_production(text):
    text = production(text)
    # Only the explicitly preserved non-Windows functions/block in these files.
    pattern = r"#\[cfg\(not\(windows\)\)\]\s*(?:fn [^{]+)?\{"
    while match := re.search(pattern, text):
        end, depth = match.end(), 1
        while depth:
            depth += (text[end] == "{") - (text[end] == "}")
            end += 1
        text = text[:match.start()] + text[end:]
    return text


class E3(unittest.TestCase):
    maxDiff = 200

    def read(self, name):
        if BASELINE:
            return subprocess.check_output(
                ["git", "show", f"HEAD:apps/local-voice/src-tauri/src/{name}"],
                cwd=ROOT, encoding="utf-8").replace("\r\n", "\n")
        return (SRC / name).read_text(encoding="utf-8")

    def has(self, text, needle):
        self.assertTrue(needle in text, f"missing: {needle}")

    def test_system_tool_starts_removed(self):
        sites = {
            "managers/tts/piper.rs": "taskkill",
            "managers/tts/mod.rs": "netstat",
            "managers/llm/server.rs": "taskkill",
            "managers/llm/cli.rs": "taskkill",
            "sync/account.rs": "icacls",
            "commands/audio.rs": "cmd",
            "commands/pages.rs": "explorer",
        }
        for name, tool in sites.items():
            with self.subTest(name=name):
                self.assertFalse(f'Command::new("{tool}")' in windows_production(self.read(name)), name)
        self.assertFalse('Command::new("taskkill")' in windows_production(self.read("managers/tts/mod.rs")))

    def test_windows_starts_are_suspended(self):
        for name in ("managers/llm/cli.rs", "managers/llm/preview.rs",
                     "llm_versand/transport/cli.rs", "managers/tts/mod.rs"):
            with self.subTest(name=name):
                text = production(self.read(name))
                self.assertFalse("creation_flags(0x0800_0000)" in text, name)
                self.has(text, "CREATE_SUSPENDED")
        self.has(self.read("managers/llm/server.rs"), "CREATE_SUSPENDED")

    def test_native_security_and_tcp(self):
        self.has(self.read("process_guard.rs"), "GetExtendedTcpTable")
        self.has(self.read("process_guard.rs"), "TCP_TABLE_OWNER_PID_LISTENER")
        self.has(self.read("sync/account.rs"), "CreateFileW")
        self.has(self.read("sync/account.rs"), "CREATE_NEW")
        self.has(self.read("sync/account.rs"), "GetNamedSecurityInfoW")

    def test_shell_routes_and_login_exception(self):
        for name in ("commands/audio.rs", "commands/pages.rs"):
            self.has(windows_production(self.read(name)), "process_guard::shell_open")
        cli = self.read("managers/llm/cli.rs")
        login = cli.split("pub fn open_login(cli: Cli)")[1].split("pub fn open_download_login")[0]
        self.assertFalse("CREATE_SUSPENDED" in login)
        self.assertFalse("ProcessGuard" in login)
        self.has(login, 'Command::new("cmd")')

    def test_implicit_batch_launch_is_blocked(self):
        cli = production(self.read("managers/llm/cli.rs"))
        self.has(cli, "cfg!(windows) && is_batch(Path::new(program))")
        self.assertEqual(cli.count("check_cli_program(cmd.get_program())"), 2)
        self.has(cli, "check_cli_program(program.as_os_str())?")
        self.has(production(self.read("llm_versand/transport/cli.rs")), "check_cli_program(program.as_os_str())?")

    def test_nonwindows_filter_does_not_hide_windows_calls(self):
        bad = 'fn f() { Command::new("taskkill"); }'
        self.has(windows_production(bad), 'Command::new("taskkill")')
        self.assertFalse('Command::new("taskkill")' in windows_production(
            '#[cfg(not(windows))]\n' + bad))

    def test_e3_allowlist_counts_match_both_directions(self):
        text = self.read("netz/edr_sperrklinke_tests.rs")
        block = text.split("const ERLAUBTE_SYSTEMWERKZEUGE")[1].split("];", 1)[0]
        expected = {(path, tool): int(count) for path, tool, count in re.findall(
            r'\(\s*"([^"]+)",\s*"([^"]+)",\s*(\d+),', block)}
        # Exact literals used by this package; comments and test modules excluded.
        tools = {"cmd", "taskkill", "netstat", "icacls", "explorer", "curl"}
        files = ("commands/audio.rs", "commands/pages.rs", "managers/llm/cli.rs",
                 "managers/llm/preview.rs", "managers/llm/server.rs", "managers/tts/mod.rs",
                 "managers/tts/piper.rs", "sync/account.rs", "sync/curl.rs")
        actual = {}
        for name in files:
            code = re.sub(r"//[^\n]*", "", production(self.read(name)))
            for literal in re.findall(r'"([^"\n]*)"', code):
                tool = literal.removesuffix(".exe")
                if tool in tools:
                    key = ("src/" + name, tool)
                    actual[key] = actual.get(key, 0) + 1
        self.assertEqual(actual, expected)


if __name__ == "__main__":
    unittest.main()
