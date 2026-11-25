"""Offline wiring checks; behavioral Win32 tests live in process_guard/listener_tests.rs."""
import pathlib
import re
import subprocess
import sys
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASELINE = "--baseline" in sys.argv
if BASELINE:
    sys.argv.remove("--baseline")


def source(name):
    path = "apps/local-voice/src-tauri/src/" + name
    if BASELINE:
        return subprocess.check_output(["git", "show", "HEAD:" + path], cwd=ROOT, encoding="utf-8")
    return (ROOT / "src-tauri/src" / name).read_text(encoding="utf-8")


class ListenerOwnership(unittest.TestCase):
    def test_path_check_precedes_job_assignment_on_held_handle(self):
        code = source("process_guard.rs").split("fn kill_listener", 1)[1].split("fn drain_listener_tree", 1)[0]
        check = code.find("paths.allows(")
        self.assertGreaterEqual(check, 0, "listener has no ownership check")
        self.assertLess(code.index("image_path(root.0)"), check)
        self.assertLess(check, code.index("ProcessGuard::attach_handle"))

    def test_all_tts_stop_routes_pass_the_manager(self):
        code = source("managers/tts/mod.rs")
        self.assertEqual(len(re.findall(r"kill_pid\(self, pid, port\)", code)), 3)
        self.assertTrue("tts.data_base_dir()" in code, "data folder missing")
        self.assertTrue("&tts.fish_dir()" in code, "configured runtime missing")

    def test_open_process_sites_unchanged(self):
        for name in ("process_guard.rs", "prozessliste.rs"):
            code = source(name).split("#[cfg(test)]", 1)[0]
            code = re.sub(r"//[^\n]*", "", code)
            self.assertEqual(len(re.findall(r"\bOpenProcess\b", code)), 2, name)

    def test_unhealthy_listener_is_not_reported_as_success_without_check(self):
        code = source("managers/tts/mod.rs").split("pub async fn stop_server_any", 1)[1].split("pub fn kill_server_hard", 1)[0]
        self.assertTrue("!healthy && (!cfg!(windows) || listening_pid(port).is_none())" in code,
                        "unhealthy listener bypasses ownership check")


if __name__ == "__main__":
    unittest.main()
