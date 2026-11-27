"""Offline guards for lva#960 (Protokoll hält bei unlesbarer Antwort an), not Swift behavior tests.

Behavior: MinutesPipelineTests (Mac gate). Each guard has a counterprobe.
"""
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]


def read(path):
    return (ROOT / path).read_text(encoding="utf-8")


def section(source, start, end):
    return source.split(start, 1)[1].split(end, 1)[0]


def library_guard(source):
    failure = section(source, "if !Task.isCancelled {\n                    failed.insert(doc.id)", "status = \"gespeichert")
    assert "let delay = scheduleRetry(doc.id, after: error)" in failure
    assert "Self.failureMessage(error, transcribed: doc.nextOffset >= doc.duration, retryIn: delay)" in failure
    schedule = section(source, "private func scheduleRetry(", "static func failureMessage(")
    assert "retryTasks.removeValue(forKey: id)?.cancel()" in schedule
    assert schedule.index("retryTasks.removeValue(forKey: id)?.cancel()") < schedule.index("guard Self.isTransient(error)")
    assert "guard Self.isTransient(error), attempt < Self.retryDelays.count else { return nil }" in schedule
    assert "guard self.failed.remove(id) != nil else { return }" in schedule
    assert "self.start()" in schedule
    resume = section(source, "private func resumeTransientFailures()", "/// Plant die nächste")
    assert "retryAttempts[id] = nil" not in resume
    assert "retryAttempts[id] = attempt + 1" in resume
    assert "retryAttempts[doc.id] = nil; transientFailures.remove(doc.id)" in source
    transient = section(source, "static func isTransient(", "private func scheduleRetry(")
    assert "case .unavailable, .timeout, .stateChanged: return true" in transient
    assert "default: return false" in transient


def pipeline_guard(source):
    call = section(source, "private func call(", "public func run(")
    assert "} catch ModelFailure.provider(.invalidResponse) {" in call
    assert call.count("session.complete(request)") == 2
    decode = section(source, "public static func decode(", "/// Bindung an Eingabe")
    assert "init(from decoder: Decoder) throws { item = try? Item(from: decoder) }" in decode
    assert "guard result.items.count < 12 else { result.rejected += 1; continue }" in decode
    assert "prefix(12)" not in decode
    assert "reply.items.count <= 12" not in decode


class MinutesRetryTests(unittest.TestCase):
    def test_library_retries_transient_failures(self):
        source = read("iPhone/MeetingLibrary.swift")
        library_guard(source)
        for old, new in [("let delay = scheduleRetry(doc.id, after: error)", "let delay: UInt64? = nil"),
                         ("guard self.failed.remove(id) != nil else { return }", ""),
                         ("        retryTasks.removeValue(forKey: id)?.cancel()\n        let attempt", "        let attempt"),
                         ("default: return false", "default: return true"),
                         ("retryAttempts[id] = attempt + 1", "retryAttempts[id] = nil")]:
            with self.subTest(counterprobe=old), self.assertRaises(AssertionError):
                library_guard(source.replace(old, new))

    def test_pipeline_tolerates_and_retries_unreadable_replies(self):
        source = read("VoiceCore/Sources/VoiceCore/Models/MinutesPipeline.swift")
        pipeline_guard(source)
        for old, new in [("} catch ModelFailure.provider(.invalidResponse) {", "} catch ModelFailure.timeout {"),
                         ("init(from decoder: Decoder) throws { item = try? Item(from: decoder) }",
                          "init(from decoder: Decoder) throws { item = try Item(from: decoder) }"),
                         ("guard result.items.count < 12 else { result.rejected += 1; continue }", "")]:
            with self.subTest(counterprobe=old), self.assertRaises(AssertionError):
                pipeline_guard(source.replace(old, new))


if __name__ == "__main__":
    unittest.main()
