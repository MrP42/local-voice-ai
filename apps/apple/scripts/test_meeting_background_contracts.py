"""Offline wiring checks; behavioral XCTest and SDK build belong to the Mac gate."""
import pathlib
import plistlib
import unittest
from unittest.mock import patch

ROOT = pathlib.Path(__file__).resolve().parents[1]


class MeetingBackgroundContracts(unittest.TestCase):
    def test_registration_and_manifest(self):
        info = plistlib.loads((ROOT / 'VoicePhone-Info.plist').read_bytes())
        self.assertIn('de.localvoice.prototype.meeting.*', info['BGTaskSchedulerPermittedIdentifiers'])
        self.assertIn('de.localvoice.prototype.process-meetings', info['BGTaskSchedulerPermittedIdentifiers'])
        self.assertIn('processing', info['UIBackgroundModes'])
        generator = (ROOT / 'scripts/generate_project.py').read_text(encoding='utf-8')
        for name in ['MeetingProcessing.swift', 'MeetingProcessingRuntime.swift', 'de.localvoice.prototype.meeting.*',
                     'de.localvoice.prototype.process-meetings']:
            self.assertIn(name, generator)
        delegate = (ROOT / 'iPhone/PhoneProcessingRuntime.swift').read_text(encoding='utf-8')
        self.assertIn('MeetingLibrary.shared.registerBackgroundProcessing()', delegate)
        app = (ROOT / 'Shared/VoiceApp.swift').read_text(encoding='utf-8')
        self.assertIn('MeetingLibrary.shared.scene(active: value == .active, inactive: value == .inactive)', app)

    def test_cpu_minutes_and_checkpoint_cancellation(self):
        provider = (ROOT / 'iPhone/Models/LocalModelProvider.swift').read_text(encoding='utf-8')
        self.assertIn('lv_llama_load_cpu', provider)
        library = (ROOT / 'iPhone/MeetingLibrary.swift').read_text(encoding='utf-8')
        self.assertIn('background.allowed', library)
        self.assertIn('try Task.checkCancellation()\n                        try await archive.saveMinutesCheckpoint', library)
        native = (ROOT / 'iPhone/Engines/LlamaBridge.mm').read_text(encoding='utf-8')
        self.assertIn('cp.offload_kqv = value->gpu', native)
        self.assertIn('return load_llama(path, 0, cancellation, false)', native)
        archive = (ROOT / 'VoiceCore/Sources/VoiceCore/Meetings.swift').read_text(encoding='utf-8')
        for method in ['appendChunk', 'saveMinutesCheckpoint']:
            body = archive.split('public func ' + method, 1)[1].split('\n    }', 1)[0]
            self.assertLess(body.index('try Task.checkCancellation()'), body.index('var doc = try load(id)'))

    def test_request_and_expiry_contract(self):
        runtime = (ROOT / 'iPhone/MeetingProcessingRuntime.swift').read_text(encoding='utf-8')
        for contract in ['request.strategy = .fail', 'request.requiresExternalPower = true',
                         'BGTaskScheduler.supportedResources.contains(.gpu)',
                         'MeetingProcessing.requestGPU(supported: gpuSupported, entitled: gpuSupported && hasGPUEntitlement)',
                         'task.progress.totalUnitCount = 1000', 'setTaskCompleted(success: false)']:
            self.assertIn(contract, runtime)
        entitlements = plistlib.loads((ROOT / 'VoicePhone.entitlements').read_bytes())
        self.assertNotIn('com.apple.developer.background-tasks.continued-processing.gpu', entitlements)


class Counterprobes(unittest.TestCase):
    def test_removed_wiring_and_cpu_guards_are_detected_in_memory(self):
        cases = [
            ('iPhone/PhoneProcessingRuntime.swift', 'MeetingLibrary.shared.registerBackgroundProcessing()',
             'test_registration_and_manifest'),
            ('iPhone/Models/LocalModelProvider.swift', 'lv_llama_load_cpu', 'test_cpu_minutes_and_checkpoint_cancellation'),
            ('iPhone/Engines/LlamaBridge.mm', 'cp.offload_kqv = value->gpu', 'test_cpu_minutes_and_checkpoint_cancellation'),
            ('VoiceCore/Sources/VoiceCore/Meetings.swift', 'try Task.checkCancellation()', 'test_cpu_minutes_and_checkpoint_cancellation'),
            ('iPhone/MeetingProcessingRuntime.swift', 'request.strategy = .fail', 'test_request_and_expiry_contract'),
            ('iPhone/MeetingProcessingRuntime.swift', 'request.requiresExternalPower = true', 'test_request_and_expiry_contract'),
        ]
        original = pathlib.Path.read_text
        for path, token, test in cases:
            target = ROOT / path
            self.assertIn(token, original(target, encoding='utf-8'))
            def mutated(file, *args, **kwargs):
                source = original(file, *args, **kwargs)
                return source.replace(token, 'removed') if file == target else source
            with self.subTest(path=path), patch.object(pathlib.Path, 'read_text', mutated):
                result = unittest.TestResult()
                MeetingBackgroundContracts(test).run(result)
                self.assertTrue(result.failures or result.errors, 'Counterprobe did not detect removal')


if __name__ == '__main__':
    unittest.main()
