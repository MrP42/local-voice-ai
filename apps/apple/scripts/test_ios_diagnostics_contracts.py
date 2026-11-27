"""Offline guards for the on-device diagnostics log and the launch path (27.11.: black screen at launch,
minutes aborting). Not Swift behavior tests; each guard has a counterprobe. Nothing is built or touched."""
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]


def read(path):
    return (ROOT / path).read_text(encoding="utf-8")


def section(source, start, end):
    assert start in source, start
    return source.split(start, 1)[1].split(end, 1)[0]


def diagnostics_guard(runtime):
    diag = section(runtime, "enum PhoneDiagnostics {", "/// Uses only system-granted runtime.")
    assert "queue.async { append(line) }" in diag
    assert "limit = 512 * 1024" in diag and "size > limit" in diag
    launch = section(runtime, "didFinishLaunchingWithOptions", "handleEventsForBackgroundURLSession")
    for step in ["MeetingLibrary", "ModelDownloads", "AudioDownloads", "Assistent"]:
        assert f'PhoneDiagnostics.measure("{step}")' in launch, step


def meeting_guard(meetings):
    worker = section(meetings, "worker = Task {", "/// Zwischendatei der Audiospur")
    assert "PhoneDiagnostics.note(\"verarbeitung fehler" in worker
    assert "Self.failureKind(error)" in worker
    # Keine Meldungstexte ins Protokoll: sie können Inhalte enthalten.
    assert "localizedDescription" not in section(meetings, "static func failureKind(", "static func failureMessage(")
    assert "error.localizedDescription" not in worker


def library_guard(library):
    init = section(library, "init(root: URL? = nil) {", "static func excludeFromBackup(")
    assert 'if !UserDefaults.standard.bool(forKey: retrofit) {' in init


class DiagnosticsTests(unittest.TestCase):
    def test_diagnostics_off_main_and_bounded(self):
        runtime = read("iPhone/PhoneProcessingRuntime.swift")
        diagnostics_guard(runtime)
        for old, new in [("queue.async { append(line) }", "append(line)"),
                         ("size > limit", "false"),
                         ('PhoneDiagnostics.measure("AudioDownloads") { _ = AudioDownloads.shared }', "_ = AudioDownloads.shared")]:
            with self.subTest(counterprobe=old), self.assertRaises(AssertionError):
                diagnostics_guard(runtime.replace(old, new))

    def test_minutes_log_kind_without_content(self):
        meetings = read("iPhone/MeetingLibrary.swift")
        meeting_guard(meetings)
        with self.assertRaises(AssertionError):
            meeting_guard(meetings.replace("Self.failureKind(error))\"", "error.localizedDescription)\""))

    def test_backup_retrofit_runs_once(self):
        library = read("iPhone/Library/PhoneLibrary.swift")
        library_guard(library)
        with self.assertRaises(AssertionError):
            library_guard(library.replace("if !UserDefaults.standard.bool(forKey: retrofit) {", "if true {"))


if __name__ == "__main__":
    unittest.main()
