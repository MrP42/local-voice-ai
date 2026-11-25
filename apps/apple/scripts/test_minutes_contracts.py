"""Offline source contracts and in-memory counterprobes for #801 T4a."""
import ast
import json
from pathlib import Path
import sys
import re
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]


def read(path):
    file = ROOT / path
    return file.read_text(encoding='utf-8') if file.exists() else ''


class MinutesContracts(unittest.TestCase):
    maxDiff = 500

    def test_changed_files_use_lf(self):
        paths = [
            'VoiceCore/Sources/VoiceCore/Models/MinutesPipeline.swift',
            'VoiceCore/Sources/VoiceCore/Meetings.swift', 'VoiceCore/Sources/VoiceCore/Store.swift',
            'VoiceCore/Tests/VoiceCoreTests/MinutesPipelineTests.swift', 'VoiceCore/Tests/VoiceCoreTests/MeetingTests.swift',
            'iPhone/Models/LocalModelProvider.swift', 'iPhone/MeetingLibrary.swift', 'iPhone/ModelLibrary.swift',
            'iPhone/CPULocalProviders.swift', 'iPhone/Engines/LVEngines.h', 'iPhone/Engines/Cancellation.hpp',
            'iPhone/Engines/Cancellation.mm', 'iPhone/Engines/WhisperBridge.mm', 'iPhone/Engines/LlamaBridge.mm',
            'scripts/generate_project.py', 'scripts/test_minutes_contracts.py',
            'VoicePhone.entitlements', 'THIRD_PARTY_NOTICES.md',
        ]
        for path in paths:
            with self.subTest(path=path):
                self.assertNotIn(b'\r', (ROOT / path).read_bytes(), path)

    def test_schema_and_json_repair(self):
        source = read('VoiceCore/Sources/VoiceCore/Models/MinutesPipeline.swift')
        schema = re.search(r'schema = #"(.*?)"#', source)
        self.assertIsNotNone(schema)
        self.assertEqual(json.loads(schema[1])['required'], ['items'])
        for token in ('if quoted {', 'else if c ==', 'if next == "}" || next == "]"',
                      'ids.allSatisfy({ byID[$0] != nil })', 'result.rejected += 1'):
            self.assertIn(token, source)

    def test_resume_and_hierarchical_reduce(self):
        source = read('VoiceCore/Sources/VoiceCore/Models/MinutesPipeline.swift')
        for token in ('checkpoint?.binding == binding', 'encode(consent)', 'encode(session.provider.route)',
                      'saved.parts[key] = part', 'while parts.count > 1', 'guard after < before',
                      'evidence: source.filter { ids.contains($0.id) }'):
            self.assertIn(token, source)

    def test_budget_and_gate(self):
        core = read('VoiceCore/Sources/VoiceCore/Models/MinutesPipeline.swift')
        for token in ('provider.boundedTokens', 'session.complete', 'Task.checkCancellation()',
                      'contextTokens - outputTokens - 128', 'consent.allows(.aiAnalysis',
                      'onCheckpoint', 'LiveRefinement.preferredSegments', 'Antworte immer auf Deutsch'):
            self.assertIn(token, core)

    def test_local_adapter(self):
        source = read('iPhone/Models/LocalModelProvider.swift')
        for token in ('ModelProviderCheck.confirm', 'lv_llama_chat', 'lv_llama_free',
                      'lv_inference_try_acquire', 'lv_inference_release', '8_000_000_000',
                      'onTermination', '4096', 'ModelFailure.sanitized'):
            self.assertIn(token, source)
        self.assertNotIn('URLSession', source)
        for token in ('CPULocalProviders.shared.minutesModelURL(model)',):
            self.assertIn(token, source)
        native_call = source.split('ModelProviderCheck.confirm(provider, request, authorization)', 1)[1].split('lv_llama_chat', 1)[0]
        self.assertNotIn('await ', native_call)
        self.assertLess(source.index('installed(LocalModel.qwen3Name)'), source.index('if SystemLanguageModel.default.availability'))

    def test_native_gate(self):
        for path, count in (('iPhone/Engines/LlamaBridge.mm', 5), ('iPhone/Engines/WhisperBridge.mm', 4)):
            self.assertEqual(read(path).count('LVInferenceLease inference'), count)
        self.assertIn('lv_inference_try_acquire', read('iPhone/Engines/Cancellation.mm'))
        self.assertIn('if (acquired) lv_inference_release()', read('iPhone/Engines/Cancellation.hpp'))
        source = read('iPhone/Models/LocalModelProvider.swift')
        self.assertIn('withTaskCancellationHandler', source)
        self.assertIn('lv_llama_load_cancellable(url.path, 1, cancellation.pointer)', source)
        self.assertIn('lv_llama_load_cancellable(url.path, 0, cancellation.pointer)', source)

    def test_archive_and_catalog(self):
        archive = read('VoiceCore/Sources/VoiceCore/Meetings.swift')
        app = read('iPhone/MeetingLibrary.swift')
        self.assertIn('minutesCheckpoint', archive)
        self.assertIn('savePipelineMinutes', archive)
        self.assertIn('MinutesPipeline', app)
        self.assertIn('guard let consent = doc.consent', app)
        self.assertIn('route: provider.route', app)
        self.assertNotIn('CPULocalProviders.shared.minutes(for:', app)
        self.assertIn('increased-memory-limit', read('scripts/generate_project.py'))
        self.assertNotIn('Classify the CURRENT numbered German transcript', read('iPhone/CPULocalProviders.swift'))
        self.assertIn('doc.consent?.allows(.aiAnalysis', archive)
        self.assertIn('e.start == segment.start && e.end == segment.end', archive)
        self.assertIn('subtracting(["evidence"])', archive)
        catalog = read('iPhone/ModelLibrary.swift')
        for token in ('a06e946bb6b655725eafa393f4a9745d460374c9', '2497281120',
                      '3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597'):
            self.assertIn(token, catalog)
        self.assertIn('Qwen3-4B-Instruct-2507', read('THIRD_PARTY_NOTICES.md'))
        ast.parse(read('scripts/generate_project.py'))


class Counterprobes(unittest.TestCase):
    def test_removed_guards_fail_without_changing_files(self):
        mutations = [
            ('VoiceCore/Sources/VoiceCore/Models/MinutesPipeline.swift', 'consent.allows(.aiAnalysis', 'true //', 'test_budget_and_gate'),
            ('iPhone/Models/LocalModelProvider.swift', 'ModelProviderCheck.confirm', 'removed', 'test_local_adapter'),
            ('iPhone/Engines/WhisperBridge.mm', 'LVInferenceLease', 'removed', 'test_native_gate'),
            ('iPhone/MeetingLibrary.swift', 'guard let consent = doc.consent', 'guard false', 'test_archive_and_catalog'),
            ('VoiceCore/Sources/VoiceCore/Models/MinutesPipeline.swift', 'contextTokens - outputTokens - 128', 'contextTokens', 'test_budget_and_gate'),
            ('VoiceCore/Sources/VoiceCore/Models/MinutesPipeline.swift', 'ids.allSatisfy({ byID[$0] != nil })', 'true', 'test_schema_and_json_repair'),
            ('VoiceCore/Sources/VoiceCore/Models/MinutesPipeline.swift', 'if next == "}" || next == "]"', 'if false', 'test_schema_and_json_repair'),
            ('VoiceCore/Sources/VoiceCore/Models/MinutesPipeline.swift', 'checkpoint?.binding == binding', 'true', 'test_resume_and_hierarchical_reduce'),
            ('VoiceCore/Sources/VoiceCore/Models/MinutesPipeline.swift', 'while parts.count > 1', 'while false', 'test_resume_and_hierarchical_reduce'),
            ('VoiceCore/Sources/VoiceCore/Models/MinutesPipeline.swift', 'Task.checkCancellation()', 'removed()', 'test_budget_and_gate'),
            ('VoiceCore/Sources/VoiceCore/Models/MinutesPipeline.swift', 'evidence: source.filter { ids.contains($0.id) }', 'evidence: source', 'test_resume_and_hierarchical_reduce'),
            ('iPhone/Models/LocalModelProvider.swift', 'lv_llama_free', 'removed', 'test_local_adapter'),
            ('iPhone/Models/LocalModelProvider.swift', '8_000_000_000', '0', 'test_local_adapter'),
            ('VoiceCore/Sources/VoiceCore/Meetings.swift', 'doc.consent?.allows(.aiAnalysis', 'true //', 'test_archive_and_catalog'),
            ('VoiceCore/Sources/VoiceCore/Meetings.swift', 'e.start == segment.start && e.end == segment.end', 'true', 'test_archive_and_catalog'),
            ('iPhone/Models/LocalModelProvider.swift', 'CPULocalProviders.shared.minutesModelURL(model)', 'URL(fileURLWithPath: model)', 'test_local_adapter'),
            ('VoiceCore/Sources/VoiceCore/Models/MinutesPipeline.swift', 'Antworte immer auf Deutsch', 'Antworte', 'test_budget_and_gate'),
            ('iPhone/Models/LocalModelProvider.swift', 'lv_llama_load_cancellable(url.path, 1, cancellation.pointer)', 'lv_llama_load(url.path, 1)', 'test_native_gate'),
            ('iPhone/Models/LocalModelProvider.swift', 'lv_llama_load_cancellable(url.path, 0, cancellation.pointer)', 'lv_llama_load(url.path, 0)', 'test_native_gate'),
        ]
        original = read
        for path, before, after, test in mutations:
            with self.subTest(test=test):
                source = original(path)
                self.assertIn(before, source)
                with patch.object(sys.modules[__name__], 'read', side_effect=lambda p: source.replace(before, after) if p == path else original(p)):
                    result = unittest.TestResult()
                    MinutesContracts(test).run(result)
                self.assertEqual(len(result.errors), 0)
                self.assertGreater(len(result.failures), 0)


if __name__ == '__main__':
    unittest.main()
