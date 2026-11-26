"""Offline wiring checks; behavioral XCTest and SDK build belong to the Mac gate."""
import pathlib
import plistlib
import unittest
from unittest.mock import patch

ROOT = pathlib.Path(__file__).resolve().parents[1]


class MeetingBackgroundContracts(unittest.TestCase):
    def test_user_start_wiring(self):
        library = (ROOT / 'iPhone/MeetingLibrary.swift').read_text(encoding='utf-8')
        runtime = (ROOT / 'iPhone/MeetingProcessingRuntime.swift').read_text(encoding='utf-8')
        for method in ['func retry()', 'func createMinutesLocally']:
            body = library.split(method, 1)[1].split('\n    func ', 1)[0].split('\n    }', 1)[0]
            self.assertIn('start(userInitiated: true)', body)
        self.assertIn('private func start(userInitiated: Bool = false)', library)
        start = library.split('private func start(userInitiated:', 1)[1].split('worker = Task', 1)[0]
        self.assertLess(start.index('background.allowStart(userInitiated: userInitiated)'),
                        start.index('guard worker == nil'))
        self.assertIn('access.allowStart(userInitiated: userInitiated)', runtime)

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
        self.assertIn('cp.op_offload = value->gpu', native)
        self.assertIn('cp.op_offload = false', native)
        self.assertIn('if (mp.n_gpu_layers == 0) mp.devices = cpu_devices', native)
        self.assertIn('mp.devices = cpu_devices', native.split('static int generate(', 1)[1].split('int lv_generate(', 1)[0])
        self.assertIn('static ggml_backend_dev_t cpu_devices[] = { nullptr }', native)
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
                         'task.progress.totalUnitCount = queue.total', 'setTaskCompleted(success: false)']:
            self.assertIn(contract, runtime)
        entitlements = plistlib.loads((ROOT / 'VoicePhone.entitlements').read_bytes())
        self.assertNotIn('com.apple.developer.background-tasks.continued-processing.gpu', entitlements)

    def test_expiration_worker_and_queue_wiring(self):
        library = (ROOT / 'iPhone/MeetingLibrary.swift').read_text(encoding='utf-8')
        runtime = (ROOT / 'iPhone/MeetingProcessingRuntime.swift').read_text(encoding='utf-8')
        core = (ROOT / 'VoiceCore/Sources/VoiceCore/MeetingProcessing.swift').read_text(encoding='utf-8')
        start = library.split('private func start(userInitiated:', 1)[1].split('private var pendingDocument', 1)[0]
        self.assertLess(start.index('guard background.allowStart'), start.index('guard let doc'))
        self.assertLess(start.index('guard background.beginWorker()'), start.index('worker = Task'))
        self.assertIn('background.endWorker(doc.id, completed: !Task.isCancelled, success: !failed.contains(doc.id))', start)
        self.assertIn('queue: pendingDocuments.map(\\.id)', library)
        self.assertIn('document: doc.id, units:', library)
        self.assertIn('background.nextDocument(in: pendingDocuments)', library)
        self.assertIn('queue.next(in: documents.map(\\.id))', runtime)
        self.assertIn('if foreground { access.resume() }', runtime)
        self.assertIn('self.access.resume()', runtime)
        self.assertIn('self.access.expireDeferred()', runtime)
        self.assertIn('self.deferred == nil, self.access.identifier == nil', runtime)
        self.assertIn('self.access.expire(task.identifier)', runtime)
        self.assertIn('queue.update(document, units: units)', runtime)
        self.assertIn('if completed { queue.complete(document, success: success) }', runtime)
        self.assertIn('let success = success && queue.successful', runtime)
        self.assertIn('guard access.finish() else { return }', runtime)
        self.assertIn('continued?.progress.completedUnitCount = queue.completed', runtime)
        self.assertIn('!suspended &&', core)
        self.assertIn('guard allowed, !workerRunning else { return false }', core)
        expiration = library.split('runtime.onExpire =', 1)[1].split('runtime.onResume =', 1)[0]
        for cancellation in ['worker?.cancel()', 'token?.cancel()', 'exporter?.cancelExport()']:
            self.assertIn(cancellation, expiration)
        for handler in runtime.split('task.expirationHandler =')[1:]:
            body = handler.split('self.scheduleDeferred()', 1)[0]
            self.assertLess(body.index('self.onExpire?()'), body.index('task.setTaskCompleted(success: false)'))
        # Submission failure is not expiration and must not suspend foreground work.
        self.assertIn('access.finish()', runtime.split('do { try scheduler.submit(request) }', 1)[1].split('private func run', 1)[0])


class Counterprobes(unittest.TestCase):
    def test_removed_wiring_and_cpu_guards_are_detected_in_memory(self):
        cases = [
            ('iPhone/MeetingLibrary.swift', 'start(userInitiated: true)', 'test_user_start_wiring'),
            ('iPhone/MeetingProcessingRuntime.swift', 'access.allowStart(userInitiated: userInitiated)', 'test_user_start_wiring'),
            ('iPhone/PhoneProcessingRuntime.swift', 'MeetingLibrary.shared.registerBackgroundProcessing()',
             'test_registration_and_manifest'),
            ('iPhone/Models/LocalModelProvider.swift', 'lv_llama_load_cpu', 'test_cpu_minutes_and_checkpoint_cancellation'),
            ('iPhone/Engines/LlamaBridge.mm', 'cp.offload_kqv = value->gpu', 'test_cpu_minutes_and_checkpoint_cancellation'),
            ('iPhone/Engines/LlamaBridge.mm', 'cp.op_offload = value->gpu', 'test_cpu_minutes_and_checkpoint_cancellation'),
            ('iPhone/Engines/LlamaBridge.mm', 'cp.op_offload = false', 'test_cpu_minutes_and_checkpoint_cancellation'),
            ('iPhone/Engines/LlamaBridge.mm', 'if (mp.n_gpu_layers == 0) mp.devices = cpu_devices', 'test_cpu_minutes_and_checkpoint_cancellation'),
            ('iPhone/Engines/LlamaBridge.mm', 'static ggml_backend_dev_t cpu_devices[] = { nullptr }', 'test_cpu_minutes_and_checkpoint_cancellation'),
            ('VoiceCore/Sources/VoiceCore/Meetings.swift', 'try Task.checkCancellation()', 'test_cpu_minutes_and_checkpoint_cancellation'),
            ('iPhone/MeetingProcessingRuntime.swift', 'request.strategy = .fail', 'test_request_and_expiry_contract'),
            ('iPhone/MeetingProcessingRuntime.swift', 'request.requiresExternalPower = true', 'test_request_and_expiry_contract'),
            ('iPhone/MeetingProcessingRuntime.swift', 'task.progress.totalUnitCount = queue.total', 'test_request_and_expiry_contract'),
            ('iPhone/MeetingLibrary.swift', 'guard background.allowStart', 'test_expiration_worker_and_queue_wiring'),
            ('iPhone/MeetingLibrary.swift', 'guard background.beginWorker()', 'test_expiration_worker_and_queue_wiring'),
            ('iPhone/MeetingLibrary.swift', 'background.endWorker(doc.id, completed: !Task.isCancelled, success: !failed.contains(doc.id))', 'test_expiration_worker_and_queue_wiring'),
            ('iPhone/MeetingLibrary.swift', 'worker?.cancel()', 'test_expiration_worker_and_queue_wiring'),
            ('iPhone/MeetingProcessingRuntime.swift', 'if completed { queue.complete(document, success: success) }', 'test_expiration_worker_and_queue_wiring'),
            ('iPhone/MeetingProcessingRuntime.swift', 'let success = success && queue.successful', 'test_expiration_worker_and_queue_wiring'),
            ('iPhone/MeetingProcessingRuntime.swift', 'guard access.finish() else { return }', 'test_expiration_worker_and_queue_wiring'),
            ('iPhone/MeetingProcessingRuntime.swift', 'self.access.expireDeferred()', 'test_expiration_worker_and_queue_wiring'),
            ('iPhone/MeetingProcessingRuntime.swift', 'self.deferred == nil, self.access.identifier == nil', 'test_expiration_worker_and_queue_wiring'),
            ('iPhone/MeetingProcessingRuntime.swift', 'self.access.expire(task.identifier)', 'test_expiration_worker_and_queue_wiring'),
            ('iPhone/MeetingProcessingRuntime.swift', 'if foreground { access.resume() }', 'test_expiration_worker_and_queue_wiring'),
            ('iPhone/MeetingProcessingRuntime.swift', 'self.access.resume()', 'test_expiration_worker_and_queue_wiring'),
            ('iPhone/MeetingProcessingRuntime.swift', 'queue.update(document, units: units)', 'test_expiration_worker_and_queue_wiring'),
            ('iPhone/MeetingLibrary.swift', 'background.nextDocument(in: pendingDocuments)', 'test_expiration_worker_and_queue_wiring'),
            ('VoiceCore/Sources/VoiceCore/MeetingProcessing.swift', '!suspended &&', 'test_expiration_worker_and_queue_wiring'),
            ('VoiceCore/Sources/VoiceCore/MeetingProcessing.swift', 'guard allowed, !workerRunning else { return false }', 'test_expiration_worker_and_queue_wiring'),
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
