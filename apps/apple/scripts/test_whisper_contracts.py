#!/usr/bin/env python3
"""Offline source contracts; no substitute for the Apple build/device tests."""
import pathlib
import re
import shutil
import subprocess
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]


class WhisperContracts(unittest.TestCase):
    def test_detection_uses_abortable_full_and_sot_logits(self):
        source = (ROOT / 'iPhone/Engines/WhisperBridge.mm').read_text(encoding='utf-8')
        self.assertNotIn('whisper_lang_auto_detect(', source)
        self.assertNotIn('whisper_pcm_to_mel(', source)
        detection = source.split('auto detection = whisper_full_default_params(', 1)[1]
        detection = detection.split('options.language = whisper_lang_str(detected);', 1)[0]
        self.assertTrue(detection.startswith('WHISPER_SAMPLING_GREEDY)'))
        for setting in ('duration_ms = 30000', 'single_segment = true',
                        'no_timestamps = true', 'max_tokens = 1', 'temperature_inc = 0',
                        'n_max_text_ctx = 0', 'language = "en"', 'no_context = true',
                        'abort_callback = lv_should_abort', 'abort_callback_user_data = &deadline'):
            self.assertIn('detection.' + setting, detection)
        self.assertNotIn('initial_prompt', detection)
        self.assertNotIn('beam_search', detection)
        self.assertIn('std::min<int32_t>(count, 16000 * 30)', detection)
        for call in ('whisper_full(context.get(), detection,', 'whisper_decode(context.get(), &sot, 1, 0, options.n_threads)'):
            before, after = detection.split(call, 1)
            self.assertRegex(before, r'if \(lv_should_abort\(&deadline\)\) return 11;\s*const int \w+ =\s*$')
            self.assertRegex(after, r'^[^;]*;\s*if \(lv_should_abort\(&deadline\)\) return 11;')
        self.assertIn('whisper_token_sot(context.get())', detection)
        self.assertIn('whisper_get_logits(context.get())', detection)
        self.assertIn('id <= whisper_lang_max_id()', detection)
        self.assertIn('logits[whisper_token_lang(context.get(), id)]', detection)
        self.assertIn('score > best', detection)
        self.assertIn('whisper_is_multilingual(context.get())', source)
        self.assertIn('options.language = "en";', source)

    def test_language_is_validated_without_mutating_defaults(self):
        source = (ROOT / 'iPhone/LocalProviders.swift').read_text(encoding='utf-8')
        language = source.split('static func sttLanguage()', 1)[1].split('///', 1)[0]
        self.assertIn('throws -> String', language)
        self.assertNotIn('defaults.set(', language)
        self.assertIn('components(separatedBy: CharacterSet(charactersIn: "-_"))', language)
        self.assertIn('lv_whisper_language_valid(code)', language)
        self.assertIn('throw LiveCapture.CaptureError.invalidLanguage', language)
        self.assertIn('case invalidLanguage', (ROOT / 'iPhone/LiveMeeting/LiveCapture.swift').read_text(encoding='utf-8'))
        self.assertIn('let language = try LocalProviders.sttLanguage()',
                      (ROOT / 'iPhone/CPULocalProviders.swift').read_text(encoding='utf-8'))
        self.assertIn('let locale = try await LocalProviders.speechLocale()',
                      (ROOT / 'iPhone/LiveMeeting/LiveCapture.swift').read_text(encoding='utf-8'))
        self.assertIn('static func speechLocale() async throws -> Locale?', source)
        self.assertIn('let language = try sttLanguage()', source)
        self.assertNotIn('let locale = await speechLocale()', source)

    def test_deadline_boundaries_and_load_order(self):
        source = (ROOT / 'iPhone/Engines/WhisperBridge.mm').read_text(encoding='utf-8')
        match = re.search(r'const auto seconds = std::max<int64_t>\((\d+), \(int64_t\(count\) \* (\d+) \+ (\d+)\) / (\d+)\);', source)
        self.assertIsNotNone(match, 'Keep the budget expression inspectable offline')
        minimum, factor, rounding, rate = map(int, match.groups())
        for samples, expected in ((1, 90), (16000 * 30 - 1, 90), (16000 * 30, 90),
                                  (16000 * 30 + 1, 91), (16000 * 60, 180)):
            with self.subTest(samples=samples):
                self.assertEqual(max(minimum, (samples * factor + rounding) // rate), expected)
        self.assertIn('std::chrono::steady_clock::time_point::max()', source)
        self.assertLess(source.index('if (!context) return 2;'), source.index('deadline.deadline ='))
        self.assertEqual(source.count('deadline.deadline ='), 1, 'CPU retry must not reset the budget')
        self.assertLess(source.index('deadline.deadline ='), source.index('auto infer ='))


class STTLanguageBehavior(unittest.TestCase):
    @unittest.skipUnless(shutil.which('swift'), 'Requires Swift; static contracts run on Windows')
    def test_synthetic_settings_preserve_selection_and_report_invalid_language(self):
        # Execute the actual method, isolating defaults and the native language table.
        source = (ROOT / 'iPhone/LocalProviders.swift').read_text(encoding='utf-8')
        method = 'static func sttLanguage()' + source.split('static func sttLanguage()', 1)[1].split('///', 1)[0]
        harness = '''
import Foundation
final class UserDefaults {
    static let standard = UserDefaults()
    var saved: String? = nil
    var writes = 0
    func string(forKey: String) -> String? { saved }
    func set(_ value: String, forKey: String) { saved = value; writes += 1 }
}
enum LiveCapture { enum CaptureError: Error { case invalidLanguage } }
func lv_whisper_language_valid(_ value: String) -> Int32 {
    ["auto", "de", "en"].contains(value) ? 1 : 0
}
enum LocalProviders { METHOD }
let cases: [(String?, String?)] = [
    (nil, "auto"), ("auto", "auto"), ("de", "de"), ("en", "en"),
    ("de-DE", "de"), ("en_US", "en"), ("DE-de", "de"),
    ("ungueltig", nil), ("", nil), ("auto-DE", nil), ("_US", nil)
]
for (saved, expected) in cases {
    UserDefaults.standard.saved = saved
    UserDefaults.standard.writes = 0
    do {
        let result = try LocalProviders.sttLanguage()
        precondition(expected != nil && result == expected)
    } catch LiveCapture.CaptureError.invalidLanguage {
        precondition(expected == nil)
    } catch { preconditionFailure("Unexpected error type") }
    precondition(UserDefaults.standard.saved == saved && UserDefaults.standard.writes == 0)
}
'''.replace('METHOD', method)
        result = subprocess.run(['swift', '-'], input=harness, text=True, capture_output=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == '__main__':
    unittest.main()
