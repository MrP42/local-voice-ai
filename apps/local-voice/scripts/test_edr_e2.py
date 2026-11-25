"""Offline source gates for lva#874; --baseline checks the unchanged HEAD."""
import pathlib
import re
import subprocess
import sys
import tomllib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASELINE = "--baseline" in sys.argv
sys.argv = [arg for arg in sys.argv if arg != "--baseline"]


def baseline_source(relative):
    path = ROOT / relative
    result = subprocess.run(
        ["git", "show", "HEAD:" + path.relative_to(ROOT.parents[1]).as_posix()],
        cwd=ROOT, capture_output=True, check=False,
    )
    return result.stdout.decode("utf-8")


def source(relative):
    return baseline_source(relative) if BASELINE else (ROOT / relative).read_text(encoding="utf-8")


class E2SourceTests(unittest.TestCase):
    def test_input_guard_has_no_hooks(self):
        code = source("src-tauri/src/input.rs")
        for token in ("SetWindowsHookExW", "WH_KEYBOARD_LL", "WH_MOUSE_LL"):
            self.assertTrue(token not in code, "input guard still installs " + token)
        self.assertTrue("GetLastInputInfo" in code, "last-input guard missing")

    def test_hook_backend_requires_persisted_opt_in(self):
        code = source("src-tauri/src/shortcut/handy_keys.rs")
        self.assertTrue("keyboard_hooks_explicitly_enabled" in code, "hook consent gate missing")
        settings = source("src-tauri/src/settings.rs")
        self.assertTrue("pub keyboard_hooks_explicitly_enabled: bool" in settings, "saved consent missing")
        for constructor in ["pub fn new(", "pub fn start_recording("]:
            body = code[code.index(constructor):].split("\n    }", 1)[0]
            self.assertTrue("require_hook_opt_in(" in body, constructor + " is ungated")
        command = source("src-tauri/src/shortcut/mod.rs")
        choice = command[command.index("pub fn change_keyboard_implementation_setting("):]
        self.assertTrue("store.save()" in choice, "choice must be saved before hooks start")
        self.assertLess(choice.index("store.save()"), choice.index("initialize_handy_keys_with_rollback("))

    def test_default_and_shutdown_are_hook_free_on_windows(self):
        settings = source("src-tauri/src/settings.rs")
        default = settings.split("impl Default for KeyboardImplementation", 1)[1].split("impl Default", 1)[0]
        self.assertTrue('any(target_os = "linux", target_os = "windows")' in default,
                        "Windows still defaults to the hook backend")
        code = source("src-tauri/src/shortcut/handy_keys.rs")
        self.assertTrue("drop(manager.take())" in code, "switching backend leaves hooks installed")
        self.assertTrue("state.suspend()?" in source("src-tauri/src/shortcut/mod.rs"),
                        "switch does not wait for hook shutdown")

    def test_windows_bypasses_dependency_release_poller(self):
        code = source("src-tauri/src/shortcut/tauri_impl.rs")
        self.assertTrue("windows_hotkey::register_shortcut" in code, "uses the dependency's unconditional poller")
        native = source("src-tauri/src/shortcut/windows_hotkey.rs")
        for token in ["RegisterHotKey", "UnregisterHotKey", "if !is_recording()", "GetAsyncKeyState"]:
            self.assertTrue(token in native, "missing native hotkey contract: " + token)
        production = native.split("#[cfg(test)]", 1)[0]
        production = re.sub(r"//[^\n]*", "", production)
        self.assertEqual(len(re.findall(r"\bGetAsyncKeyState\b", production)), 2)
        gate = source("src-tauri/src/netz/edr_sperrklinke_tests.rs")
        self.assertTrue(re.search(r'"src/shortcut/windows_hotkey.rs",\s*"GetAsyncKeyState",\s*2,', gate) is not None,
                        "EDR allowlist must match the scoped poller exactly")
        coordinator = source("src-tauri/src/transcription_coordinator.rs")
        ptt_branch = coordinator.split("if push_to_talk {", 1)[1].split("} else if is_pressed", 1)[0]
        self.assertTrue("crate::shortcut::recording_started(" in ptt_branch,
                        "release polling must start from an actual PTT recording")
        self.assertEqual(coordinator.count("crate::shortcut::recording_started("), 1,
                         "CLI/toggle recordings must not start a release poller")

    def test_input_tick_is_u32_and_ambiguous_input_is_not_whitelisted(self):
        code = source("src-tauri/src/input.rs")
        state = source("src-tauri/src/refinement/injection_state.rs")
        self.assertTrue("pub(crate) physical_generation: u32" in code, "wrong tick width")
        self.assertTrue("pub(crate) physical_generation: u32" in state, "wrong tick width in refinement")
        self.assertTrue("record_tracked_append" in state, "per-stroke proof is not consumed")
        self.assertTrue("OwnInputFrame" in code, "per-stroke frame missing")
        frame = source("src-tauri/src/input/own_input.rs")
        stroke = frame.split("pub(super) fn stroke<T>(", 1)[1].split("#[cfg(test)]", 1)[0]
        for earlier, later in [
            ("self.observe(clock.last_input())", "let start = clock.now()"),
            ("let start = clock.now()", "let result = send(self.safe)"),
            ("let result = send(self.safe)", "let end = clock.now()"),
            ("let end = clock.now()", "match clock.last_input()"),
        ]:
            self.assertLess(stroke.index(earlier), stroke.index(later))
        self.assertIn("tick.wrapping_sub(start) <= end.wrapping_sub(start)", stroke)
        injection = source("src-tauri/src/refinement/injection.rs")
        self.assertIn("frame.stroke(|| input::paste_text_direct", injection)
        self.assertIn("send_tracked_paste_ctrl_v(&mut enigo, frame, false)", injection)
        self.assertIn("send_tracked_paste_ctrl_v(&mut enigo, &mut frame, true)", injection)

    def test_legacy_custom_binding_gets_one_time_grandfathering(self):
        settings = source("src-tauri/src/settings.rs")
        migration = settings.split("fn apply_settings_migrations(", 1)[1].split(
            "// One-time onboarding migration", 1)[0]
        self.assertIn('settings_value.get("keyboard_hooks_explicitly_enabled").is_none()', migration)
        self.assertIn("settings.keyboard_hooks_explicitly_enabled = true", migration)
        self.assertIn("binding.current_binding != default.current_binding", migration)

    def test_return_to_tauri_clears_consent_and_waits_for_hook_shutdown(self):
        command = source("src-tauri/src/shortcut/mod.rs")
        choice = command.split("pub fn change_keyboard_implementation_setting(", 1)[1]
        self.assertIn("chosen.keyboard_hooks_explicitly_enabled = new_impl == KeyboardImplementation::HandyKeys", choice)
        self.assertLess(choice.index("unregister_all_shortcuts(&app, current_impl)"),
                        choice.index("register_all_shortcuts_for_implementation(&app, new_impl)"))
        shutdown = command.split("fn unregister_all_shortcuts(", 1)[1].split(
            "fn register_all_shortcuts_for_implementation(", 1)[0]
        self.assertIn("state.suspend()?", shutdown)
        worker = source("src-tauri/src/shortcut/handy_keys.rs")
        suspend = worker.split("ManagerCommand::Suspend { response } =>", 1)[1].split("}", 1)[0]
        self.assertLess(suspend.index("drop(manager.take())"), suspend.index("response.send(())"))

    def test_native_dispatch_preserves_the_binding_for_altgr_guard(self):
        native = source("src-tauri/src/shortcut/windows_hotkey.rs")
        self.assertTrue("dispatch_hotkey(r," in native, "native events bypass raw-binding dispatch")
        self.assertTrue("handle(&registered.binding_id, &registered.raw)" in native,
                        "normalization changes ctrl to control and defeats the existing AltGr guard")

    def test_backend_switch_cannot_drop_an_active_ptt_release(self):
        command = source("src-tauri/src/shortcut/mod.rs")
        choice = command[command.index("pub fn change_keyboard_implementation_setting("):]
        self.assertTrue("audio.is_recording()" in choice, "backend switch does not guard an active recording")
        self.assertLess(choice.index("audio.is_recording()"), choice.index("settings::try_write_settings("))
        native = source("src-tauri/src/shortcut/windows_hotkey.rs")
        worker = native.split("fn worker(", 1)[1].split("pub(super) fn init(", 1)[0]
        self.assertTrue("settings.keyboard_implementation" not in worker,
                        "a recording's existing release watch must survive a backend change")

    def test_rdev_removed_from_manifest_lock_and_allowlist(self):
        self.assertTrue("rdev =" not in source("src-tauri/Cargo.toml"), "rdev remains a direct dependency")
        self.assertTrue('name = "rdev"' not in source("src-tauri/Cargo.lock"), "rdev remains locked")
        gate = source("src-tauri/src/netz/edr_sperrklinke_tests.rs")
        self.assertTrue('ERLAUBTE_HOOK_CRATES: &[&str] = &["handy-keys"];' in gate, "rdev remains allowed")

    def test_lockfile_dependencies_resolve_uniquely(self):
        packages = tomllib.loads(source("src-tauri/Cargo.lock"))["package"]
        for package in packages:
            for dependency in package.get("dependencies", []):
                parts = dependency.split()
                matches = [p for p in packages if p["name"] == parts[0]
                           and (len(parts) == 1 or p["version"] == parts[1])]
                self.assertEqual(len(matches), 1, package["name"] + " has an unresolved/ambiguous dependency")

    def test_lockfile_only_prunes_rdev_and_its_unused_dependencies(self):
        def packages(text):
            return {(p["name"], p["version"]): p for p in tomllib.loads(text)["package"]}

        before = packages(baseline_source("src-tauri/Cargo.lock"))
        after = packages(source("src-tauri/Cargo.lock"))

        def edges(nodes):
            result = {}
            for key, package in nodes.items():
                targets = set()
                for dep in package.get("dependencies", []):
                    parts = dep.split()
                    targets.update(k for k in nodes if k[0] == parts[0]
                                   and (len(parts) == 1 or k[1] == parts[1]))
                result[key] = targets
            return result

        old_edges = edges(before)
        removed = {key for key in before if key[0] == "rdev"}
        while True:
            candidates = {target for key in removed for target in old_edges[key]}
            unused = {key for key in candidates - removed
                      if all(parent in removed for parent, targets in old_edges.items() if key in targets)}
            if not unused:
                break
            removed.update(unused)
        self.assertEqual(set(before) - set(after), removed)
        self.assertTrue(set(after) <= set(before), "lockfile added packages")
        new_edges = edges(after)
        for key in after:
            self.assertEqual(new_edges[key], old_edges[key] - removed, "changed dependency graph")
            self.assertEqual({k: v for k, v in after[key].items() if k != "dependencies"},
                             {k: v for k, v in before[key].items() if k != "dependencies"})


if __name__ == "__main__":
    unittest.main()
