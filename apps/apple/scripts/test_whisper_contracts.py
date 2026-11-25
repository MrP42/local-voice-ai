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
                        'abort_callback = whisper_should_abort', 'abort_callback_user_data = &run'):
            self.assertIn('detection.' + setting, detection)
        self.assertNotIn('initial_prompt', detection)
        self.assertNotIn('beam_search', detection)
        self.assertIn('std::min<int32_t>(count, 16000 * 30)', detection)
        for call in ('whisper_full(context.get(), detection,', 'whisper_decode(context.get(), &sot, 1, 0, options.n_threads)'):
            before, after = detection.split(call, 1)
            self.assertRegex(before, r'if \(const int stop = whisper_stop\(&run\)\) return stop;\s*const int \w+ =\s*$')
            self.assertRegex(after, r'^[^;]*;\s*if \(const int stop = whisper_stop\(&run\)\) return stop;')
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

    def test_full_language_identifier_before_reduction(self):
        source = (ROOT / 'iPhone/LocalProviders.swift').read_text(encoding='utf-8')
        method = source.split('static func sttLanguage()', 1)[1].split('///', 1)[0]
        match = re.search(r'language.range\(of: "([^"]+)", options: \.regularExpression\)', method)
        self.assertIsNotNone(match, 'Validate the entire identifier before taking its primary code')
        self.assertLess(match.start(), method.index('components(separatedBy:'))
        pattern = match[1].replace('\\\\', '\\').replace(r'\z', r'\Z')
        valid = ('de', 'DE-de', 'en_US', 'zh-Hant', 'zh_Hant_TW', 'es-419', 'eng-Latn-001')
        invalid = ('de-@@@', 'de-', 'de-DE-extra', 'de--DE', 'de_1234', 'de-Latn-1',
                   'de-DE-Latn', 'de\n', ' de', 'de ', 'd', 'abcd', 'dé', 'de-ÄÖ', 'de-１２３')
        for value in valid + invalid:
            with self.subTest(value=value):
                self.assertEqual(re.search(pattern, value) is not None,
                                 value in valid)
        self.assertIn('lv_whisper_language_valid(code)', method)

    def test_gpu_permission_lifecycle_and_cpu_retry(self):
        source = (ROOT / 'iPhone/Engines/WhisperBridge.mm').read_text(encoding='utf-8')
        header = (ROOT / 'iPhone/Engines/LVEngines.h').read_text(encoding='utf-8')
        runtime = (ROOT / 'iPhone/PhoneProcessingRuntime.swift').read_text(encoding='utf-8')
        self.assertIn('void lv_set_gpu_allowed(int allowed)', header)
        self.assertIn('std::atomic<uint64_t> gpu_permission{0}', source)
        self.assertIn('compare_exchange_weak', source)
        self.assertIn('(previous + 2) & ~uint64_t(1)', source)
        self.assertIn('gpu_permission.load() != run->permission', source)
        stop = source.split('static int whisper_stop(', 1)[1].split('static bool whisper_should_abort', 1)[0]
        self.assertLess(stop.index('lv_should_abort'), stop.index('return 13'))
        self.assertIn('run->use_gpu', stop)
        self.assertIn('allow_gpu && !TARGET_OS_SIMULATOR && !TARGET_OS_WATCH', source)
        self.assertIn('(permission & 1) != 0', source)
        self.assertIn('options.abort_callback = whisper_should_abort', source)
        self.assertIn('options.abort_callback_user_data = &run', source)
        for marker in ('const int encoded =', 'const int decoded =', 'const int status = whisper_full'):
            self.assertRegex(source.split(marker, 1)[0],
                             r'if \(const int stop = whisper_stop\(&run\)\) return stop;\s*(?:phase_marker\("transcribing"\);\s*)?$')
        load = source.split('auto load =', 1)[1].split('if (!context) return 2;', 1)[0]
        self.assertIn('whisper_stop(&run)', load)
        self.assertIn('!context || whisper_stop(&run) == 13', load)
        self.assertIn('load_cpu()', load)
        retry = source.split('int status = infer();', 1)[1].split('if (status != 0)', 1)[0]
        self.assertIn('(status == 3 || status == 13) && parameters.use_gpu', retry)
        self.assertEqual(retry.count('status = infer();'), 1)
        self.assertIn('load_cpu()', retry)
        cpu = source.split('auto load_cpu =', 1)[1].split('auto load =', 1)[0]
        self.assertLess(cpu.index('context.reset();'), cpu.index('whisper_init_from_file_with_params'))
        self.assertIn('run.use_gpu = false;', cpu)
        self.assertNotIn('deadline.deadline =', cpu + retry)
        launch = runtime.split('didFinishLaunchingWithOptions', 1)[1]
        self.assertLess(launch.index('observeGPULifecycle(application)'), launch.index('registerBackgroundProcessing()'))
        observer = runtime.split('private func observeGPULifecycle', 1)[1].split('func application(', 1)[0]
        for event in ('willResignActiveNotification', 'didEnterBackgroundNotification', 'didBecomeActiveNotification'):
            self.assertIn('UIApplication.' + event, observer)
        self.assertIn('queue: nil', observer, 'Revoke synchronously on the posting thread')
        self.assertIn('lv_set_gpu_allowed(0)', observer)
        self.assertIn('lv_set_gpu_allowed(1)', observer)
        self.assertIn('application.applicationState == .active', observer)

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
    ("de-Latn-DE", "de"), ("en_001", "en"), ("de-lAtN", "de"),
    ("ungueltig", nil), ("", nil), ("auto-DE", nil), ("_US", nil), ("zz-ZZ", nil),
    ("de-@@@", nil), ("de-", nil), ("de-DE-extra", nil), ("de--DE", nil),
    ("de-DE-Latn", nil), ("de\\n", nil), (" de", nil), ("de ", nil), ("de-ÄÖ", nil)
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
