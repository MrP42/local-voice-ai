#!/usr/bin/env python3
"""Offline integration contracts. Swift behavior tests remain a separate Mac/Linux gate."""
from pathlib import Path
import ast
import math
import re
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]


def read(path):
    file = ROOT / path
    return file.read_text(encoding='utf-8') if file.exists() else ''


class LiveRefinementContracts(unittest.TestCase):
    def test_changed_sources_use_lf_and_generator_parses_without_running(self):
        paths = ['THIRD_PARTY_NOTICES.md', 'scripts/generate_project.py',
                 'scripts/test_live_refinement_contracts.py',
                 'VoiceCore/Tests/VoiceCoreTests/LiveRefinementTests.swift',
                 'VoiceCore/Sources/VoiceCore/Meetings.swift',
                 'VoiceCore/Sources/VoiceCore/LiveMeeting.swift',
                 'iPhone/CPULocalProviders.swift', 'iPhone/LocalProviders.swift',
                 'iPhone/MeetingLibrary.swift', 'iPhone/Engines/LVEngines.h',
                 'iPhone/Engines/WhisperBridge.mm']
        paths += ['VoiceCore/Sources/VoiceCore/' + name + '.swift' for name in
                  ('LiveChunker', 'LiveLanguageLock', 'LiveHallucination', 'LiveRefinement')]
        paths += ['iPhone/LiveMeeting/' + name + '.swift' for name in
                  ('LiveCapture', 'LiveRefiner', 'LiveMeetingModel', 'LiveSummarizer')]
        for path in paths:
            with self.subTest(path=path):
                self.assertFalse(b'\r' in (ROOT / path).read_bytes(), path)
        ast.parse(read('scripts/generate_project.py'))

    def test_deferred_ranges_survive_archive_import(self):
        archive = read('VoiceCore/Sources/VoiceCore/Meetings.swift')
        self.assertIn('public var deferredRefinement: [LiveRefinementRange]?', archive)
        self.assertIn('doc.deferredRefinement = live.deferredRefinement', archive)
        self.assertIn('existing.deferredRefinement', archive)
        self.assertIn('deferredRefinement: session.deferredRefinement', read('iPhone/MeetingLibrary.swift'))

    def test_release_and_lifecycle_do_not_block_main_actor(self):
        refiner = read('iPhone/LiveMeeting/LiveRefiner.swift')
        release = refiner.split('deinit {', 1)[1].split('\n}', 1)[0]
        self.assertIn('DispatchQueue.global', release)
        capture = read('iPhone/LiveMeeting/LiveCapture.swift')
        for method in ('start()', 'stop()', 'teardown(finalize:', 'startPipeline('):
            self.assertRegex(capture, r'@MainActor\s+(?:private )?func ' + re.escape(method))

    def test_softmax_numeric_boundaries_from_native_expression(self):
        bridge = read('iPhone/Engines/WhisperBridge.mm')
        self.assertIn('std::exp(double(score - best))', bridge)
        self.assertIn('probability = sum > 0 ? 1.0 / sum : 0;', bridge)
        self.assertIn('if (guess || std::strcmp(options.language, "auto") == 0)', bridge)
        self.assertIn('if (language && std::strcmp(language, "auto") != 0) options.language = language;', bridge)
        # Numeric boundary checks supplement the source contract, not execution of C++.
        for scores, expected in [([0, math.log(4)], 0.8), ([0, math.log(9)], 0.9),
                                 ([1000, 1000], 0.5)]:
            best = max(scores)
            self.assertAlmostEqual(1 / sum(math.exp(score - best) for score in scores), expected)

    def test_all_native_mutations_take_gate(self):
        bridge = read('iPhone/Engines/WhisperBridge.mm')
        for signature in ('static int transcribe(', 'LVWhisperContext *lv_whisper_load('):
            body = bridge.split(signature, 1)[1].split('whisper_log_set', 1)[0]
            self.assertIn('if (!acquire_whisper(lock, &deadline))', body)
        for signature in ('void lv_whisper_free(', 'int32_t lv_whisper_count('):
            body = bridge.split(signature, 1)[1].split('\n}', 1)[0]
            self.assertIn('lock(whisper_mutex)', body)
        self.assertIn('if (!context) context.reset(whisper_init_from_file_with_params', bridge)

    def test_short_ring_gap_and_empty_result_preserve_fallback(self):
        core = read('VoiceCore/Sources/VoiceCore/LiveRefinement.swift')
        self.assertIn('boundaryBefore', core)
        self.assertIn('!lines.isEmpty', core)
        capture = read('iPhone/LiveMeeting/LiveCapture.swift')
        self.assertIn('if packet.boundaryBefore', capture)

    def test_context_and_global_serialization(self):
        bridge = read('iPhone/Engines/WhisperBridge.mm')
        for api in ('lv_whisper_load', 'lv_whisper_free', 'lv_whisper_run'):
            self.assertIn(api, read('iPhone/Engines/LVEngines.h'))
            self.assertIn(api, bridge)
        self.assertIn('std::timed_mutex whisper_mutex', bridge)
        self.assertIn('try_lock_for', bridge)
        self.assertIn('std::exp(double(score - best))', bridge)
        self.assertIn('@"prob"', bridge)
        self.assertNotIn('whisper_lang_auto_detect(', bridge)

    def test_capture_queue_boundaries_and_fallback(self):
        capture = read('iPhone/LiveMeeting/LiveCapture.swift')
        self.assertIn('LiveAudioRing', capture)
        self.assertIn('startFrame: writtenFrames', capture)
        self.assertIn('refinementQueue', capture)
        self.assertIn('flushRefinement', capture)
        self.assertIn('setVoiceProcessingEnabled', capture)
        self.assertIn('func followLanguage', capture)
        self.assertIn('== .installed', capture)
        self.assertNotIn('downloadAndInstall', capture)
        refiner = read('iPhone/LiveMeeting/LiveRefiner.swift')
        for marker in ('CPULocalProviders.shared', 'LiveLanguageLock', 'LiveHallucination',
                       'thermal >= .serious', 'pending.count > 3', 'deferChunk', 'cancel()'):
            self.assertIn(marker, refiner)

    def test_replacement_and_stale_summary_guard(self):
        model = read('iPhone/LiveMeeting/LiveMeetingModel.swift')
        self.assertIn('replaceRefined', model)
        self.assertIn('transcriptVersion == live.transcriptVersion', model)
        self.assertIn('preferredSegments', read('iPhone/LiveMeeting/LiveSummarizer.swift'))
        core = read('VoiceCore/Sources/VoiceCore/LiveMeeting.swift')
        self.assertIn('decodeIfPresent(Bool.self, forKey: .refined) ?? false', core)

    def test_apple_keeps_region(self):
        source = read('iPhone/LocalProviders.swift')
        method = source.split('static func speechLocale()', 1)[1].split('static func capabilities', 1)[0]
        self.assertIn('string(forKey: "sttLanguage")', method)

    def test_sources_and_attribution(self):
        generator = read('scripts/generate_project.py')
        for name in ('LiveChunker', 'LiveLanguageLock', 'LiveHallucination', 'LiveRefinement'):
            self.assertIn(name, generator)
            self.assertIn(name, read('VoiceCore/Tests/VoiceCoreTests/LiveRefinementTests.swift'))
        self.assertIn('LiveRefiner', generator)
        self.assertIn('Copyright (c) 2026 Frigade', read('THIRD_PARTY_NOTICES.md'))


class Counterprobes(unittest.TestCase):
    def test_contracts_reject_removed_fixes_without_touching_worktree(self):
        native = 'iPhone/Engines/WhisperBridge.mm'
        model = 'iPhone/LiveMeeting/LiveMeetingModel.swift'
        core = 'VoiceCore/Sources/VoiceCore/LiveRefinement.swift'
        mutations = [
            (native, 'std::exp(double(score - best))', 'std::exp(double(score))', 'test_context_and_global_serialization'),
            (native, 'if (!acquire_whisper(lock, &deadline))', 'if (false)', 'test_all_native_mutations_take_gate'),
            (model, 'transcriptVersion == live.transcriptVersion', 'true', 'test_replacement_and_stale_summary_guard'),
            (core, '!lines.isEmpty', 'true', 'test_short_ring_gap_and_empty_result_preserve_fallback'),
            ('iPhone/MeetingLibrary.swift', 'deferredRefinement: session.deferredRefinement', 'deferredRefinement: nil', 'test_deferred_ranges_survive_archive_import'),
            ('iPhone/LocalProviders.swift', 'string(forKey: "sttLanguage")', 'string(forKey: "primaryCode")', 'test_apple_keeps_region'),
        ]
        original = read
        for path, before, after, test in mutations:
            with self.subTest(test=test):
                source = original(path)
                self.assertIn(before, source)
                changed = source.replace(before, after)
                with patch.object(sys.modules[__name__], 'read', side_effect=lambda p: changed if p == path else original(p)):
                    result = unittest.TestResult()
                    LiveRefinementContracts(test).run(result)
                self.assertEqual(len(result.errors), 0, 'Counterprobe must fail an assertion, not crash')
                self.assertGreater(len(result.failures), 0, 'Removed fix escaped the contract')


if __name__ == '__main__':
    unittest.main()
