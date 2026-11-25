"""Offline regression gates and exhaustive synthetic lifecycle model for lva#874.

--baseline runs the same contracts against HEAD, without changing the tree.
The model is supplemental; Rust tests exercise the production input selector.
"""
import itertools
import pathlib
import subprocess
import sys
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASELINE = "--baseline" in sys.argv
sys.argv = [arg for arg in sys.argv if arg != "--baseline"]


def source(path):
    if BASELINE:
        return subprocess.check_output(
            ["git", "show", "HEAD:apps/local-voice/" + path], cwd=ROOT
        ).decode("utf-8")
    return (ROOT / path).read_text(encoding="utf-8")


COORDINATOR = source("src-tauri/src/transcription_coordinator.rs")
TOGGLE_DEBOUNCE = "(push_to_talk || !recording)" not in COORDINATOR
KEEP_INPUT_BINDING = "Some(id) => (id, recording_push_to_talk" not in COORDINATOR


class Lifecycle:
    """Synthetic coordinator queue plus native watch/HandyKeys edge delivery.

    D delays the next start; a second D completes it. F fails either the
    pending start or the next start. Settings and hardware change immediately
    while coordinator inputs wait behind the synchronous microphone start.
    """
    def __init__(self, native, mode, fixed):
        self.native, self.mode, self.fixed = native, mode, fixed
        self.active = self.starting = self.watch = None
        self.keys = set()
        self.queue = []
        self.chord = 0
        self.delay = self.fail = False
        self.now = 0
        self.deadline = None
        self.last_press = None

    def input(self, key, chord, pressed, setting):
        if self.starting:
            self.queue.append((key, chord, pressed, setting))
            return
        mode = self.active[1] if self.active and self.fixed else setting
        owner = self.active[0] if self.active and self.fixed and not KEEP_INPUT_BINDING else key
        if self.active and KEEP_INPUT_BINDING and owner != self.active[0]:
            return
        if self.active and mode:
            if pressed:
                pending = self.deadline is not None
                self.deadline = None
                if self.native and self.fixed:
                    self.watch = (owner, chord, True)
                if pending:
                    return
            elif owner == self.active[0] and self.deadline is None:
                self.deadline = self.now + 50
                return
        if not pressed:
            return
        stopping_toggle = self.active and not mode and not TOGGLE_DEBOUNCE
        if not stopping_toggle and self.last_press is not None and self.now - self.last_press < 30:
            return
        self.last_press = self.now
        if self.active:
            if pressed and owner == self.active[0]:
                if not mode:
                    self.stop()
        elif pressed:
            self.starting = (key, setting, chord)
            if not self.delay:
                self.complete()

    def complete(self):
        if not self.starting:
            return
        key, mode, chord = self.starting
        self.starting = None
        if not self.fail:
            self.active = (key, mode)
            if self.native and mode:
                self.watch = (key, chord, True)
        self.fail = self.delay = False
        queued, self.queue = self.queue, []
        for event in queued:
            if event is None:
                self.stop()
            else:
                self.input(*event)
        self.sample()

    def stop(self):
        self.active = self.watch = self.deadline = None

    def sample(self):
        if self.active and self.watch:
            key, chord, held = self.watch
            down = chord in self.keys
            if held != down:
                self.watch = (key, chord, down)
                self.input(key, chord, down, True)
        if self.deadline is not None and self.now >= self.deadline:
            self.stop()

    def event(self, event):
        self.now += 10
        if event in "AB":
            chord = (event, self.chord if event == "A" else 0)
            self.keys.add(chord)
            # The native fast path owns its original raw chord. A rebound
            # press must go through the coordinator rather than disappear.
            if self.native and self.watch and self.watch[0] == event:
                if self.watch[1] == chord:
                    self.watch = (event, chord, True)
                    self.input(event, chord, True, True)
                elif self.fixed:
                    self.input(event, chord, True, self.mode)
            else:
                self.input(event, chord, True, self.mode)
        elif event == "R":
            released, self.keys = self.keys, set()
            if not self.native:
                for key, version in sorted(released):
                    # Rebinding may remove the old HandyKeys registration.
                    if version == (self.chord if key == "A" else 0):
                        self.input(key, (key, version), False, self.mode)
        elif event == "M":
            self.mode = not self.mode
        elif event == "K":
            self.chord ^= 1
        elif event == "C":
            if self.starting:
                self.queue.append(None)
            else:
                self.stop()
        elif event == "F":
            self.fail = True
            self.complete()
        elif self.starting:
            self.complete()
        else:
            self.delay = True
        self.sample()

    def settle(self):
        self.complete()
        self.now += 60
        self.sample()


class Round3Tests(unittest.TestCase):
    def test_recording_mode_and_input_binding_are_selected_before_release_and_debounce(self):
        code = source("src-tauri/src/transcription_coordinator.rs")
        body = code.split("Command::Input {", 1)[1]
        self.assertTrue("fn recording_input" in code, "recording mode is not frozen")
        self.assertIn("Some(_) => (binding_id, recording_push_to_talk", code)
        self.assertLess(body.index("recording_input("), body.index("match classify_ptt_event("))
        self.assertIn("recording_push_to_talk = push_to_talk;", body)
        self.assertIn("native_release_monitor: true,", code)
        self.assertIn("if native_release_monitor && is_pressed && push_to_talk", body)
        foreign = body.index("if recording_binding.is_some_and(|id| id != binding_id.as_str())")
        self.assertLess(foreign, body.index("if native_release_monitor && is_pressed && push_to_talk"))

    def test_rebound_native_press_is_dispatched_instead_of_discarded(self):
        code = source("src-tauri/src/shortcut/windows_hotkey.rs")
        self.assertTrue("if !watch.press(&r.raw) {\n                        continue;" not in code,
                        "rebinding discards the only recovery press")
        self.assertTrue("if watch.press(&r.raw)" in code, "native re-press bypasses watch")

    def test_save_precedes_unregister_and_failed_handoff_restores_registrations(self):
        code = source("src-tauri/src/shortcut/tauri_impl.rs")
        migration = code.split("if user_settings.keyboard_hooks_migration_pending {", 1)[1]
        self.assertTrue("migration_transaction(" in migration, "handoff has no rollback")
        transaction = code.split("fn migration_transaction(", 1)[1].split("fn persist_migration(", 1)[0]
        self.assertIn("persist(false).and_then(|()| handoff())", transaction)
        self.assertLess(transaction.index("persist(true)"), transaction.index("restore_native()"))
        self.assertIn("store.save()", code.split("fn persist_migration(", 1)[1])
        self.assertIn("register_shortcut(app, binding.clone())", migration)
        self.assertIn("if rollback { user_settings.clone() }", migration)

    def test_failed_initialization_is_retryable_and_serialized(self):
        code = source("src-tauri/src/commands/mod.rs")
        body = code.split("pub fn initialize_shortcuts(", 1)[1]
        self.assertTrue(".try_lock()" in body, "UI must not wait for initialization on a plugin worker")
        self.assertLess(body.index(".try_lock()"), body.index("try_state::<ShortcutsInitialized>"))
        self.assertLess(body.index("if result.is_ok()"), body.index("app.manage(ShortcutsInitialized)"))
        native = source("src-tauri/src/shortcut/windows_hotkey.rs")
        self.assertIn("r.binding_id == binding.id && r.raw == binding.current_binding", native)
        handy = source("src-tauri/src/shortcut/handy_keys.rs").split("pub fn init_shortcuts(", 1)[1]
        self.assertLess(handy.index("try_state::<HandyKeysState>"), handy.index("HandyKeysState::new"))

    def test_plugin_retry_reuses_only_an_owned_registration(self):
        code = source("src-tauri/src/shortcut/tauri_impl.rs")
        self.assertTrue("fn owns_registration(" in code, "plugin retries reject their own successful registrations")
        register = code.split('pub fn register_shortcut(app: &AppHandle, binding: ShortcutBinding)', 2)[2]
        self.assertLess(register.index("if owns_registration("), register.index(".on_shortcut("))
        self.assertIn("owned.insert(binding.id, binding.current_binding)", register)
        self.assertIn(".remove(&binding.id)", register)

    def test_plugin_ownership_lock_is_acquired_only_on_the_main_thread(self):
        code = source("src-tauri/src/shortcut/tauri_impl.rs")
        self.assertTrue("fn plugin_transaction(" in code, "plugin call waits for main while holding the ownership lock")
        transaction = code.split("fn plugin_transaction(", 1)[1].split("/// Initialize shortcuts", 1)[0]
        self.assertLess(transaction.index("run_on_main_thread("), transaction.index(".lock()"))
        self.assertLess(transaction.index("action(&app"), transaction.index("rx.recv()"))

    def test_explicit_cli_and_signal_toggle_keeps_its_stop_semantics(self):
        signal = source("src-tauri/src/signal_handle.rs")
        self.assertTrue("c.send_toggle(binding_id, source)" in signal,
                        "explicit toggles are indistinguishable from a changed keyboard setting")
        code = source("src-tauri/src/transcription_coordinator.rs")
        self.assertIn("recording_push_to_talk && !explicit_toggle", code)
        toggle = code.split("pub fn send_toggle(", 1)[1].split("pub fn notify_cancel(", 1)[0]
        self.assertIn("explicit_toggle: true", toggle)
        stop = code.split("Stage::Recording(id) if id == &binding_id => {", 1)[1]
        self.assertLess(stop.index("pending_release = None"), stop.index("stop(&app"))

    def test_mode_changes_during_recording_and_delayed_start(self):
        fixed = "fn recording_input" in source("src-tauri/src/transcription_coordinator.rs")
        for native, initial, delayed in itertools.product((False, True), repeat=3):
            model = Lifecycle(native, initial, fixed)
            for event in ("D" if delayed else "") + "AM":
                model.event(event)
            model.settle()  # Recovery press is outside the debounce window.
            if not initial:
                model.event("A")  # Toggle start must still stop on press.
            model.event("R")  # PTT start must still stop on release.
            model.settle()
            self.assertIsNone(model.active, (native, initial, delayed))

    def test_toggle_bounce_is_ignored_and_next_owner_press_stops(self):
        code = source("src-tauri/src/transcription_coordinator.rs")
        fixed = "fn recording_input" in code
        for native in [False, True]:
            model = Lifecycle(native, False, fixed)
            for event in "AARA":  # Bounce at 10 ms; accepted stop at 30 ms.
                model.event(event)
                if model.now < 40:
                    self.assertEqual(model.active, ("A", False), native)
            model.settle()
            self.assertIsNone(model.active, native)

    def test_other_binding_press_and_release_leave_owner_recording(self):
        for native, mode in itertools.product((False, True), repeat=2):
            with self.subTest(native=native, mode=mode):
                model = Lifecycle(native, mode, True)
                model.event("A")
                model.now += 30
                model.event("B")
                model.keys.remove(("B", 0))
                model.input("B", ("B", 0), False, mode)
                model.settle()
                self.assertEqual(model.active, ("A", mode))
                if native and mode:
                    self.assertEqual(model.watch[:2], ("A", ("A", 0)))
                model.event("A")
                model.event("R")
                model.settle()
                self.assertIsNone(model.active)

    def test_rebound_owner_can_stop_in_both_start_modes(self):
        for native, mode in itertools.product((False, True), repeat=2):
            model = Lifecycle(native, mode, True)
            model.event("A")
            model.event("K")
            model.settle()
            model.event("A")
            model.event("R")
            model.settle()
            self.assertIsNone(model.active, (native, mode))

    def test_debounced_native_ptt_press_keeps_release_watch(self):
        model = Lifecycle(True, True, True)
        for event in "AA":
            model.event(event)
        self.assertEqual(model.watch, ("A", ("A", 0), True))
        model.event("R")
        model.settle()
        self.assertIsNone(model.active)

    def test_only_owner_native_ptt_press_updates_the_watched_chord(self):
        fixed = "fn recording_input" in source("src-tauri/src/transcription_coordinator.rs")
        for events, chord in [("AB", ("A", 0)), ("AKA", ("A", 1))]:
            model = Lifecycle(True, True, fixed)
            for event in events:
                model.event(event)
            self.assertEqual(model.watch[1], chord, events)
            model.event("R")
            model.settle()
            self.assertIsNone(model.active)

    def test_all_sequences_through_length_six_eventually_stop(self):
        fixed = "fn recording_input" in source("src-tauri/src/transcription_coordinator.rs")
        events = "ABRMKCFD"
        checked = 0
        for native, initial in itertools.product((False, True), repeat=2):
            for length in range(7):
                for seq in itertools.product(events, repeat=length):
                    model = Lifecycle(native, initial, fixed)
                    for event in seq:
                        model.event(event)
                    # The promised recovery applies to an existing recording;
                    # do not count a newly started toggle recording as a hang.
                    model.event("R")
                    model.settle()
                    if model.active:
                        model.event(model.active[0])
                        model.event("R")
                        model.settle()
                        self.assertIsNone(model.active, (native, initial, seq))
                    checked += 1
        self.assertEqual(checked, 1198372)


if __name__ == "__main__":
    unittest.main()
