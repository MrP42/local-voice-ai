"""Offline contracts and in-memory counterprobes for #801 T4b; no network/build."""
import ast
import json
from pathlib import Path
import re
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]


def read(path):
    file = ROOT / path
    return file.read_text(encoding='utf-8') if file.exists() else ''


class EndpointContracts(unittest.TestCase):
    maxDiff = 300

    def test_generation_invalidated_before_publication(self):
        store = read('iPhone/Assistant/AssistantEndpointStore.swift')
        for method, end in (('func save(_ endpoint:', '/// Löscht nur'), ('func removeKey()', '/// Löscht Endpunkt'),
                            ('func remove()', '/// Jede Änderung')):
            body = store.split(method, 1)[1].split(end, 1)[0]
            self.assertTrue('modelGeneration.invalidate()' in body, 'missing synchronous invalidation')
            self.assertLess(body.index('modelGeneration.invalidate()'), body.index('AssistantEndpointKeychain.'))
        self.assertIn('generation: modelGeneration.snapshot()', store)
        core = read('VoiceCore/Sources/VoiceCore/Models/ModelConsent.swift')
        self.assertIn('target.generation?.isCurrent != false', core)

    def test_persisted_minutes_route(self):
        app = read('iPhone/MeetingLibrary.swift')
        self.assertIn('doc.minutesCandidates(choices:', app)
        self.assertIn('try await archive.selectMinutesRoute', app)
        self.assertLess(app.index('try await archive.selectMinutesRoute'), app.index('let session = try await ModelGate.open'))
        self.assertEqual(app.count('minutesRouteKey: UserDefaults.standard.string'), 3)
        core = read('VoiceCore/Sources/VoiceCore/Meetings.swift')
        for token in ('public var minutesRouteKey: String?', 'func minutesCandidates(',
                      'minutesCheckpoint?.requiresRouteDecision', 'choices.first(where: { $0.id == key })',
                      'doc.minutesRouteKey = route.key'):
            self.assertIn(token, core)

    def test_legacy_minutes_exact_model(self):
        pipeline = read('VoiceCore/Sources/VoiceCore/Models/MinutesPipeline.swift')
        self.assertIn('var recoveredRoute: ModelRoute?', pipeline)
        self.assertIn('JSONDecoder().decode(ModelRoute.self', pipeline)
        self.assertNotIn('binding.contains(', pipeline)
        core = read('VoiceCore/Sources/VoiceCore/Meetings.swift')
        candidates = core.split('func minutesCandidates(', 1)[1].split('func minutesSegments()', 1)[0]
        for token in ('!checkpoint.parts.isEmpty', 'checkpoint.recoveredRoute',
                      'choices.filter { $0.route == route }', 'matches.count == 1', 'return matches'):
            self.assertIn(token, candidates)
        selection = core.split('func selectMinutesRoute(', 1)[1].split('func savePipelineMinutes(', 1)[0]
        self.assertIn('doc.minutesCandidates(choices:', selection)
        self.assertLess(selection.index('doc.minutesCandidates(choices:'), selection.index('doc.minutesRouteKey ='))

    def test_transport_budget_and_timeout(self):
        app = read('iPhone/Models/EndpointModelProvider.swift')
        for token in ('bufferingPolicy: .bufferingOldest(EndpointTransportBudget.bufferedEvents)',
                      'try budget.accept(data.count)', 'EndpointTransportBudget.yield',
                      'ModelFailure.provider(.invalidResponse)', 'dataTask.cancel()',
                      'configuration.timeoutIntervalForRequest = request.timeoutInterval'):
            self.assertIn(token, app)
        core = read('VoiceCore/Sources/VoiceCore/Models/EndpointChat.swift')
        self.assertIn('case .dropped', core)
        self.assertIn('timeoutInterval: EndpointTransportBudget.timeout(for: request.purpose)', core)
        self.assertIn('case .meetingMinutes, .meetingSummary: return 300', core)

    def test_send_boundary(self):
        core = read('VoiceCore/Sources/VoiceCore/Models/EndpointChat.swift')
        self.assertIn('authorization.permitsSending(to: url)', core)
        self.assertRegex(core, r'try await ModelProviderCheck.confirm\(self, request, authorization\)\s+let events = transport.start\(')
        self.assertIn('continuation.onTermination = { _ in task.cancel() }', core)
        app = read('iPhone/Models/EndpointModelProvider.swift')
        self.assertIn('AssistantEndpointKeychain.load()', read('iPhone/Assistant/AssistantEndpointStore.swift'))
        self.assertIn('AssistantEndpointStore.validated', app)
        self.assertIn('let apiKey = try await key()', core)
        self.assertLess(core.index('let apiKey = try await key()'), core.index('try await ModelProviderCheck.confirm'))

    def test_wire(self):
        core = read('VoiceCore/Sources/VoiceCore/Models/EndpointChat.swift')
        for token in ('AssistantSSE.requestBody', 'ModelProviderError(status: status)',
                      'guard completed', 'buffer.count <=', 'JSONSerialization.jsonObject',
                      'ModelFailure.sanitized(error)', 'error.code == .timedOut', 'permitsRedirect'):
            self.assertIn(token, core)
        transport = read('iPhone/Models/EndpointModelProvider.swift')
        for token in ('completionHandler(nil)', 'session.invalidateAndCancel()',
                      'timeoutIntervalForResource', 'task.resume()'):
            self.assertIn(token, transport)
        builder = read('VoiceCore/Sources/VoiceCore/Assistant/AssistantSSE.swift')
        for token in ('request.jsonSchema', 'request.temperature', 'request.purpose.maxOutputTokens',
                      'request.maxTokens > 0', 'request.messages.contains'):
            self.assertIn(token, builder)

    def test_catalog_and_pipeline(self):
        store = read('iPhone/Assistant/AssistantEndpointStore.swift')
        for token in ('endpointRouteID', 'refreshModelTarget()', 'setTarget(nil', 'setTarget(target'):
            self.assertIn(token, store)
        catalog = read('iPhone/Models/ModelCatalog.swift')
        for token in ('protocol ModelCatalog', 'ModelAccessState()', 'purpose.localOnly', 'EndpointModelProvider'):
            self.assertIn(token, catalog)
        app = read('iPhone/MeetingLibrary.swift')
        for token in ('modelCatalog.choices', 'modelCatalog.provider', 'localMinutesAvailable',
                      'func createMinutesLocally', 'updateModelConsent', 'route: provider.route.key',
                      'key != nil && selected == nil', 'doc.minutesCandidates(choices:',
                      'if let loading = loadingModelConsents[id]'):
            self.assertIn(token, app)

    def test_consent_mutations_and_result_hint(self):
        app = read('iPhone/MeetingLibrary.swift')
        for token in ('let previous = modelConsentUpdate', '_ = try? await previous?.value',
                      'modelConsentUpdate = update', 'try await update.value',
                      'await state.revoke(scope)', 'try await archive.updateModelConsent'):
            self.assertIn(token, app)
        self.assertNotIn('Lokal aus Originalstellen erstellt', app)
        archive = read('VoiceCore/Sources/VoiceCore/Meetings.swift')
        atomic = archive.split('public func updateModelConsent', 1)[1].split('public func saveMinutesCheckpoint', 1)[0]
        self.assertNotIn('await ', atomic)
        self.assertIn('filter { $0.scope != scope }', atomic)

    def test_steps_and_evidence(self):
        source = read('VoiceCore/Sources/VoiceCore/Models/MinutesPipeline.swift')
        schema = json.loads(re.search(r'schema = #"(.*?)"#', source)[1])
        kinds = schema['properties']['items']['items']['properties']['kind']['enum']
        self.assertIn('next_steps', kinds)
        self.assertIn('follow_ups', kinds)
        for token in ('MeetingMinutes.containsQuote(text, in: quote)',
                      'MeetingMinutes.containsAttribution($0, in: quote)'):
            self.assertIn(token, source)
        archive = read('VoiceCore/Sources/VoiceCore/Meetings.swift')
        for token in ('case .next_steps:', 'case .follow_ups:', 'owner: item.assignee', 'reason: item.quote'):
            self.assertIn(token, archive)

    def test_project_matches_generator_without_writing(self):
        tree = ast.parse(read('scripts/generate_project.py'))
        tree.body = [node for node in tree.body if not (isinstance(node, ast.FunctionDef) and node.name == 'put')]
        output = {}
        namespace = {'__file__': str(ROOT / 'scripts/generate_project.py'),
                     'put': lambda path, data: output.__setitem__(path.relative_to(ROOT).as_posix(), data)}
        with patch.object(Path, 'mkdir'):
            exec(compile(tree, 'generate_project.py', 'exec'), namespace)
        project = read('LocalVoice.xcodeproj/project.pbxproj')
        self.assertTrue(project == output['LocalVoice.xcodeproj/project.pbxproj'].decode('utf-8'), 'generated project differs')
        for file in ('EndpointChat.swift', 'EndpointModelProvider.swift', 'ModelCatalog.swift'):
            self.assertIn(file, project)

    def test_changed_files_use_lf(self):
        paths = ['LocalVoice.xcodeproj/project.pbxproj',
                 'VoiceCore/Sources/VoiceCore/Assistant/AssistantSSE.swift',
                 'VoiceCore/Sources/VoiceCore/Models/EndpointChat.swift',
                 'VoiceCore/Sources/VoiceCore/Models/MinutesPipeline.swift',
                 'VoiceCore/Sources/VoiceCore/Models/ModelConsent.swift',
                 'VoiceCore/Sources/VoiceCore/Meetings.swift',
                 'VoiceCore/Tests/VoiceCoreTests/EndpointChatTests.swift',
                 'VoiceCore/Tests/VoiceCoreTests/MeetingTests.swift',
                 'iPhone/Models/EndpointModelProvider.swift', 'iPhone/Models/ModelCatalog.swift',
                 'iPhone/Assistant/AssistantEndpointStore.swift', 'iPhone/MeetingLibrary.swift',
                 'scripts/test_endpoint_contracts.py']
        for path in paths:
            self.assertFalse(b'\r' in (ROOT / path).read_bytes(), path)


class Counterprobes(unittest.TestCase):
    def test_missing_fix_is_red(self):
        cases = [
            ('VoiceCore/Sources/VoiceCore/Models/MinutesPipeline.swift', 'JSONDecoder().decode(ModelRoute.self', 'JSONDecoder().decode(String.self', 'test_legacy_minutes_exact_model'),
            ('VoiceCore/Sources/VoiceCore/Meetings.swift', 'choices.filter { $0.route == route }', 'choices.filter { $0.route.isLocal }', 'test_legacy_minutes_exact_model'),
            ('VoiceCore/Sources/VoiceCore/Meetings.swift', 'matches.count == 1', '!matches.isEmpty', 'test_legacy_minutes_exact_model'),
            ('VoiceCore/Sources/VoiceCore/Meetings.swift', 'doc.minutesCandidates(choices:', 'doc.removed(choices:', 'test_legacy_minutes_exact_model'),
            ('iPhone/Assistant/AssistantEndpointStore.swift', 'modelGeneration.invalidate()', 'removed()', 'test_generation_invalidated_before_publication'),
            ('VoiceCore/Sources/VoiceCore/Models/ModelConsent.swift', 'target.generation?.isCurrent != false', 'true', 'test_generation_invalidated_before_publication'),
            ('iPhone/MeetingLibrary.swift', 'doc.minutesCandidates(choices:', 'removed(choices:', 'test_persisted_minutes_route'),
            ('iPhone/MeetingLibrary.swift', 'try await archive.selectMinutesRoute', 'try await archive.removed', 'test_persisted_minutes_route'),
            ('VoiceCore/Sources/VoiceCore/Meetings.swift', 'doc.minutesRouteKey = route.key', 'doc.minutesRouteKey = nil', 'test_persisted_minutes_route'),
            ('iPhone/Models/EndpointModelProvider.swift', 'try budget.accept(data.count)', 'try removed(data.count)', 'test_transport_budget_and_timeout'),
            ('iPhone/Models/EndpointModelProvider.swift', 'bufferingPolicy: .bufferingOldest(EndpointTransportBudget.bufferedEvents)', 'bufferingPolicy: .unbounded', 'test_transport_budget_and_timeout'),
            ('VoiceCore/Sources/VoiceCore/Models/EndpointChat.swift', 'case .dropped', 'case .removed', 'test_transport_budget_and_timeout'),
            ('VoiceCore/Sources/VoiceCore/Models/EndpointChat.swift', 'case .meetingMinutes, .meetingSummary: return 300', 'case .meetingMinutes, .meetingSummary: return 30', 'test_transport_budget_and_timeout'),
            ('VoiceCore/Sources/VoiceCore/Models/EndpointChat.swift', 'timeoutInterval: EndpointTransportBudget.timeout(for: request.purpose)', 'timeoutInterval: 30', 'test_transport_budget_and_timeout'),
            ('VoiceCore/Sources/VoiceCore/Models/EndpointChat.swift', 'ModelProviderCheck.confirm(self, request, authorization)', 'removed()', 'test_send_boundary'),
            ('VoiceCore/Sources/VoiceCore/Models/EndpointChat.swift', 'authorization.permitsSending(to: url)', 'true', 'test_send_boundary'),
            ('VoiceCore/Sources/VoiceCore/Models/EndpointChat.swift', 'guard completed', 'guard true', 'test_wire'),
            ('iPhone/Models/EndpointModelProvider.swift', 'completionHandler(nil)', 'completionHandler(request)', 'test_wire'),
            ('iPhone/Assistant/AssistantEndpointStore.swift', 'setTarget(nil', 'removed(nil', 'test_catalog_and_pipeline'),
            ('VoiceCore/Sources/VoiceCore/Models/MinutesPipeline.swift', '"next_steps","follow_ups",', '', 'test_steps_and_evidence'),
            ('VoiceCore/Sources/VoiceCore/Models/EndpointChat.swift', 'buffer.count <=', 'buffer.count >=', 'test_wire'),
            ('VoiceCore/Sources/VoiceCore/Models/EndpointChat.swift', 'ModelProviderError(status: status)', 'ModelProviderError.server', 'test_wire'),
            ('VoiceCore/Sources/VoiceCore/Assistant/AssistantSSE.swift', 'request.purpose.maxOutputTokens', '99999', 'test_wire'),
            ('VoiceCore/Sources/VoiceCore/Assistant/AssistantSSE.swift', 'request.jsonSchema', 'nil', 'test_wire'),
            ('iPhone/Models/EndpointModelProvider.swift', 'session.invalidateAndCancel()', 'session.finishTasksAndInvalidate()', 'test_wire'),
            ('iPhone/MeetingLibrary.swift', 'key != nil && selected == nil', 'false', 'test_catalog_and_pipeline'),
            ('iPhone/MeetingLibrary.swift', 'if let loading = loadingModelConsents[id]', 'if let loading = nil', 'test_catalog_and_pipeline'),
            ('VoiceCore/Sources/VoiceCore/Meetings.swift', 'owner: item.assignee', 'owner: nil', 'test_steps_and_evidence'),
            ('LocalVoice.xcodeproj/project.pbxproj', 'EndpointModelProvider.swift', 'Missing.swift', 'test_project_matches_generator_without_writing'),
            ('iPhone/MeetingLibrary.swift', '_ = try? await previous?.value', '// removed ordering', 'test_consent_mutations_and_result_hint'),
            ('iPhone/MeetingLibrary.swift', 'Aus belegten Originalstellen erstellt', 'Lokal aus Originalstellen erstellt', 'test_consent_mutations_and_result_hint'),
        ]
        original = read
        for path, before, after, method in cases:
            with self.subTest(method=method):
                source = original(path)
                self.assertIn(before, source)
                with patch.object(sys.modules[__name__], 'read', side_effect=lambda p: source.replace(before, after) if p == path else original(p)):
                    result = unittest.TestResult()
                    EndpointContracts(method).run(result)
                self.assertFalse(result.errors)
                self.assertTrue(result.failures)


if __name__ == '__main__':
    unittest.main()
