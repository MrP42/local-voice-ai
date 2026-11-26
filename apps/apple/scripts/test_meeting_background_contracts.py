"""Offline wiring checks; behavioral XCTest and SDK build belong to the Mac gate."""
import pathlib
import plistlib
import unittest
from unittest.mock import patch

ROOT = pathlib.Path(__file__).resolve().parents[1]


class MeetingBackgroundContracts(unittest.TestCase):
    def test_disabling_revokes_grants_and_preserves_checkpoint_cancellation(self):
        library = (ROOT / 'iPhone/MeetingLibrary.swift').read_text(encoding='utf-8')
        settings = (ROOT / 'iPhone/Models/MinutesModelSettings.swift').read_text(encoding='utf-8')
        runtime = (ROOT / 'iPhone/MeetingProcessingRuntime.swift').read_text(encoding='utf-8')
        self.assertIn('.onChange(of: background)', settings)
        self.assertIn('MeetingLibrary.shared.updateBackgroundSetting()', settings)
        self.assertIn('func updateBackgroundSetting() { background.updateBackgroundSetting() }', library)
        revoke = runtime.split('func updateBackgroundSetting()', 1)[1].split('func setForeground', 1)[0]
        for token in ['access.setBackgroundEnabled(MeetingLibrary.backgroundEnabled)',
                      'cancel(taskRequestWithIdentifier: Self.deferredIdentifier)',
                      'if revoked { onExpire?() }', 'continued?.setTaskCompleted(success: false)',
                      'deferred?.setTaskCompleted(success: false)',
                      'cancel(taskRequestWithIdentifier: identifier)']:
            self.assertIn(token, revoke)
        self.assertNotIn('scheduleDeferred()', revoke)
        self.assertLess(revoke.index('onExpire?()'), revoke.index('setTaskCompleted(success: false)'))
        core = (ROOT / 'VoiceCore/Sources/VoiceCore/MeetingProcessing.swift').read_text(encoding='utf-8')
        self.assertIn('foreground || (backgroundEnabled && (deferred || identifier != nil))', core)
        self.assertIn('backgroundEnabled && self.identifier == identifier', core)
        setter = core.split('func setBackgroundEnabled(', 1)[1].split('\n    }', 1)[0]
        self.assertIn('backgroundEnabled = enabled', setter)
        self.assertIn('return enabled ? false : finish()', setter)

    def test_scheduler_cleanup_only_cancels_requests(self):
        runtime = (ROOT / 'iPhone/MeetingProcessingRuntime.swift').read_text(encoding='utf-8')
        self.assertNotIn('unregisterTask', runtime)
        for start, end, identifier in [
            ('func updateBackgroundSetting()', 'func setForeground', 'identifier'),
            ('catch {', 'private func run', 'identifier'),
            ('private func run', 'func update(phase:', 'task.identifier'),
            ('func finish(success:', 'private func scheduleDeferred', 'identifier'),
        ]:
            with self.subTest(path=start):
                body = runtime.split(start, 1)[1].split(end, 1)[0]
                self.assertIn('cancel(taskRequestWithIdentifier: ' + identifier + ')', body)

    def test_continued_handler_is_registered_once_at_launch(self):
        runtime = (ROOT / 'iPhone/MeetingProcessingRuntime.swift').read_text(encoding='utf-8')
        registration = runtime.split('func register()', 1)[1].split('func updateBackgroundSetting()', 1)[0]
        self.assertIn('guard !registrationAttempted else { return }', registration)
        self.assertIn('registrationAttempted = true', registration)
        self.assertLess(registration.index('registrationAttempted = true'), registration.index('.register('))
        self.assertIn('continuedRegistered = BGTaskScheduler.shared.register(', registration)
        self.assertIn('forTaskWithIdentifier: "de.localvoice.prototype.meeting.*"', registration)
        self.assertIn('self?.run(task)', registration)
        request = runtime.split('func request(', 1)[1].split('private func run', 1)[0]
        self.assertNotIn('.register(', request)
        self.assertIn('guard continuedRegistered else { scheduleDeferred(); return }', request)

    def test_disabled_setting_guards_deferred_and_late_starts(self):
        runtime = (ROOT / 'iPhone/MeetingProcessingRuntime.swift').read_text(encoding='utf-8')
        launch = runtime.split('self.deferred = task;', 1)[0]
        self.assertIn('guard MeetingLibrary.backgroundEnabled else { task.setTaskCompleted(success: false); return }', launch)
        run = runtime.split('private func run(', 1)[1].split('continued = task', 1)[0]
        self.assertIn('guard MeetingLibrary.backgroundEnabled else { task.setTaskCompleted(success: false); return }', run)
        schedule = runtime.split('private func scheduleDeferred()', 1)[1]
        self.assertIn('guard MeetingLibrary.backgroundEnabled, registered, deferred == nil else { return }', schedule)
        request = runtime.split('func request(', 1)[1].split('let identifier =', 1)[0]
        self.assertIn('guard MeetingLibrary.backgroundEnabled, access.allowed else { return }', request)
        foreground = runtime.split('func setForeground(', 1)[1].split('func beginWorker', 1)[0]
        self.assertIn('updateBackgroundSetting()', foreground)

    def test_ready_notification_is_silent_and_claimed_before_delivery(self):
        library = (ROOT / 'iPhone/MeetingLibrary.swift').read_text(encoding='utf-8')
        notify = library.split('private func notifyReady(', 1)[1].split('private func createLocalMinutes', 1)[0]
        self.assertNotIn('content.sound', notify)
        self.assertIn('guard readyNotifications.claim(doc.id) else { return }', notify)
        self.assertLess(notify.index('readyNotifications.claim(doc.id)'), notify.index('await center.add'))
        self.assertIn('identifier: "meeting-ready-" + doc.id.uuidString', notify)
        core = (ROOT / 'VoiceCore/Sources/VoiceCore/MeetingProcessing.swift').read_text(encoding='utf-8')
        self.assertIn('claimed.insert(document).inserted', core)

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
    def test_scheduler_api_regressions_are_detected_in_memory(self):
        target = ROOT / 'iPhone/MeetingProcessingRuntime.swift'
        original = pathlib.Path.read_text
        source = original(target, encoding='utf-8')
        cases = [
            ('cancel(taskRequestWithIdentifier: identifier)', 'unregisterTask(withIdentifier: identifier)',
             'test_scheduler_cleanup_only_cancels_requests'),
            ('cancel(taskRequestWithIdentifier: task.identifier)', 'unregisterTask(withIdentifier: task.identifier)',
             'test_scheduler_cleanup_only_cancels_requests'),
            ('cancel(taskRequestWithIdentifier: identifier)', '', 'test_scheduler_cleanup_only_cancels_requests'),
            ('cancel(taskRequestWithIdentifier: task.identifier)', '', 'test_scheduler_cleanup_only_cancels_requests'),
            ('guard !registrationAttempted else { return }', '', 'test_continued_handler_is_registered_once_at_launch'),
            ('registrationAttempted = true', '', 'test_continued_handler_is_registered_once_at_launch'),
            ('forTaskWithIdentifier: "de.localvoice.prototype.meeting.*"', 'forTaskWithIdentifier: "synthetic-task"',
             'test_continued_handler_is_registered_once_at_launch'),
            ('guard continuedRegistered else { scheduleDeferred(); return }', '',
             'test_continued_handler_is_registered_once_at_launch'),
            ('let scheduler = BGTaskScheduler.shared',
             'let scheduler = BGTaskScheduler.shared\n        scheduler.register(forTaskWithIdentifier: identifier)',
             'test_continued_handler_is_registered_once_at_launch'),
        ]
        for token, replacement, test in cases:
            self.assertIn(token, source)
            # Check every cleanup site independently, without changing files on disk.
            for occurrence in range(source.count(token)):
                parts = source.split(token)
                mutated_source = token.join(parts[:occurrence + 1]) + replacement + token.join(parts[occurrence + 1:])
                def mutated(file, *args, **kwargs):
                    return mutated_source if file == target else original(file, *args, **kwargs)
                with self.subTest(token=token, occurrence=occurrence), patch.object(pathlib.Path, 'read_text', mutated):
                    result = unittest.TestResult()
                    MeetingBackgroundContracts(test).run(result)
                    self.assertFalse(result.errors, 'Counterprobe must fail by assertion')
                    self.assertTrue(result.failures, 'Counterprobe did not detect regression')

    def test_b2_regressions_are_detected_in_memory(self):
        cases = [
            ('VoiceCore/Sources/VoiceCore/MeetingProcessing.swift', 'backgroundEnabled && (deferred || identifier != nil)',
             '(deferred || identifier != nil)', 'test_disabling_revokes_grants_and_preserves_checkpoint_cancellation'),
            ('VoiceCore/Sources/VoiceCore/MeetingProcessing.swift', 'return enabled ? false : finish()',
             'return false', 'test_disabling_revokes_grants_and_preserves_checkpoint_cancellation'),
            ('VoiceCore/Sources/VoiceCore/MeetingProcessing.swift', 'claimed.insert(document).inserted',
             'true', 'test_ready_notification_is_silent_and_claimed_before_delivery'),
            ('iPhone/Models/MinutesModelSettings.swift', 'MeetingLibrary.shared.updateBackgroundSetting()', 'removed',
             'test_disabling_revokes_grants_and_preserves_checkpoint_cancellation'),
            ('iPhone/MeetingProcessingRuntime.swift', 'if revoked { onExpire?() }', '',
             'test_disabling_revokes_grants_and_preserves_checkpoint_cancellation'),
            ('iPhone/MeetingProcessingRuntime.swift', 'continued?.setTaskCompleted(success: false)', '',
             'test_disabling_revokes_grants_and_preserves_checkpoint_cancellation'),
            ('iPhone/MeetingProcessingRuntime.swift', 'deferred?.setTaskCompleted(success: false)', '',
             'test_disabling_revokes_grants_and_preserves_checkpoint_cancellation'),
            ('iPhone/MeetingProcessingRuntime.swift', 'cancel(taskRequestWithIdentifier: Self.deferredIdentifier)', 'removed',
             'test_disabling_revokes_grants_and_preserves_checkpoint_cancellation'),
            ('iPhone/MeetingProcessingRuntime.swift', 'guard MeetingLibrary.backgroundEnabled else { task.setTaskCompleted(success: false); return }', '',
             'test_disabled_setting_guards_deferred_and_late_starts'),
            ('iPhone/MeetingProcessingRuntime.swift', 'guard MeetingLibrary.backgroundEnabled, registered', 'guard registered',
             'test_disabled_setting_guards_deferred_and_late_starts'),
            ('iPhone/MeetingLibrary.swift', 'content.body = MeetingListing.title(doc)',
             'content.sound = .default\n        content.body = MeetingListing.title(doc)',
             'test_ready_notification_is_silent_and_claimed_before_delivery'),
            ('iPhone/MeetingLibrary.swift', 'guard readyNotifications.claim(doc.id) else { return }', '',
             'test_ready_notification_is_silent_and_claimed_before_delivery'),
        ]
        original = pathlib.Path.read_text
        for path, token, replacement, test in cases:
            target = ROOT / path
            self.assertIn(token, original(target, encoding='utf-8'))
            def mutated(file, *args, **kwargs):
                source = original(file, *args, **kwargs)
                return source.replace(token, replacement) if file == target else source
            with self.subTest(path=path, token=token), patch.object(pathlib.Path, 'read_text', mutated):
                result = unittest.TestResult()
                MeetingBackgroundContracts(test).run(result)
                self.assertFalse(result.errors, 'Counterprobe must fail by assertion')
                self.assertTrue(result.failures, 'Counterprobe did not detect regression')

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
