"""Offline guards for the background Live Activity (Patrick 28.11.: only briefly large, show remaining time).
Behavior of the estimate: MeetingRemainingTimeTests. Each guard has a counterprobe; nothing is built."""
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]


def read(path):
    return (ROOT / path).read_text(encoding="utf-8")


def section(source, start, end):
    assert start in source, start
    return source.split(start, 1)[1].split(end, 1)[0]


def runtime_guard(runtime):
    update = section(runtime, "func update(phase: String, subtitle: String, document: UUID, units: Int64) {",
                     "/// Restdauer aus dem echten Fortschritt")
    assert "updateTitle" not in update, "Phasenwechsel zeigen die Live-Aktivität wieder groß"
    assert "refreshRemaining(force: nextMeeting)" in update
    refresh = section(runtime, "private func refreshRemaining(force: Bool = false) {", "func finish(success: Bool) {")
    assert "continued.progress.estimatedTimeRemaining = seconds" in refresh
    assert "now.timeIntervalSince(lastTitleUpdate) >= Self.titleInterval" in refresh
    assert "title != shownTitle" in refresh
    assert "MeetingRemainingTime.label(seconds) ?? Self.title" in refresh and "?? shownTitle" not in refresh
    assert "static let titleInterval: TimeInterval = 60" in runtime


class LiveActivityTests(unittest.TestCase):
    def test_title_changes_are_rare_and_show_remaining_time(self):
        runtime = read("iPhone/MeetingProcessingRuntime.swift")
        runtime_guard(runtime)
        for old, new in [("        refreshRemaining(force: nextMeeting)\n",
                          "        refreshRemaining(force: nextMeeting)\n        continued?.updateTitle(phase, subtitle: subtitle)\n"),
                         ("now.timeIntervalSince(lastTitleUpdate) >= Self.titleInterval", "true"),
                         ("continued.progress.estimatedTimeRemaining = seconds", "_ = seconds"),
                         ("MeetingRemainingTime.label(seconds) ?? Self.title",
                          "MeetingRemainingTime.label(seconds) ?? shownTitle ?? Self.title")]:
            with self.subTest(counterprobe=old), self.assertRaises(AssertionError):
                runtime_guard(runtime.replace(old, new))


if __name__ == "__main__":
    unittest.main()
