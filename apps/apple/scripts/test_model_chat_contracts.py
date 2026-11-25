"""Offline wiring checks and in-memory counterprobes; no builds or writes."""
from pathlib import Path
import hashlib
import hmac
import json
import unittest

ROOT = Path(__file__).resolve().parents[3]
RUST = 'apps/local-voice/src-tauri/src/sync/model_chat.rs'
SWIFT = 'apps/apple/VoiceCore/Sources/VoiceCore/Models/RelayChat.swift'
CHANGED = [RUST, SWIFT, 'apps/local-voice/src-tauri/src/sync/model_chat_tests.rs',
           'apps/local-voice/src-tauri/src/sync/booktalk_relay.rs',
           'apps/local-voice/src-tauri/src/sync/booktalk_relay/tests.rs',
           'apps/local-voice/src-tauri/src/llm_versand/versand.rs',
           'apps/local-voice/src-tauri/src/llm_versand/versand_tests.rs',
           'apps/local-voice/src-tauri/src/llm_versand/uebergang.rs',
           'apps/local-voice/src-tauri/src/llm_client.rs',
           'apps/local-voice/src-tauri/src/lib.rs', 'apps/local-voice/src/bindings.ts',
           'apps/local-voice/docs/llm-versand-inventar.md',
           'apps/apple/iPhone/BookTalk/BookTalkRelayHub.swift',
           'apps/apple/iPhone/Library/LibrarySync.swift',
           'apps/apple/iPhone/Models/RelayModelProvider.swift',
           'apps/apple/iPhone/Models/ModelCatalog.swift',
           'apps/apple/VoiceCore/Tests/VoiceCoreTests/RelayChatTests.swift',
           'apps/apple/LocalVoice.xcodeproj/project.pbxproj',
           'apps/apple/docs/model-chat-relay.md', 'apps/apple/scripts/test_model_chat_contracts.py',
           'tooling/fixtures/model-chat/golden.json']


def read(path):
    return (ROOT / path).read_text(encoding='utf-8')


def verify(sources):
    rust, swift = sources[RUST], sources[SWIFT]
    for token in ('check_request(', 'mac_ok(', 'has_seen(', 'MAX_PER_HOUR',
                  'max_tokens > input.purpose.limit()', 'grant.model_chat_models',
                  'CLI_CANCEL.scope(', 'Purpose::Minutes', 'CALL_TIMEOUT',
                  'current_grant(', 'available(&settings)', 'Semaphore::new(2)'):
        assert token in rust, token
    for token in ('ModelProviderCheck.confirm(self, request, authorization)',
                  'let sent = upload.start()', 'task.cancel()', 'seq == index + 1',
                  'ModelFailure.timeout', 'cleanup()', 'isAuthentic('):
        assert token in swift, token
    assert swift.index('ModelProviderCheck.confirm') < swift.index('let sent = upload.start()')
    assert 'await' not in swift.split('ModelProviderCheck.confirm(self, request, authorization)', 1)[1].split('let sent = upload.start()', 1)[0]


class Contracts(unittest.TestCase):
    def test_wiring(self):
        verify({RUST: read(RUST), SWIFT: read(SWIFT)})

    def test_counterprobes(self):
        sources = {RUST: read(RUST), SWIFT: read(SWIFT)}
        for path, token in ((RUST, 'has_seen('), (RUST, 'mac_ok('),
                            (RUST, 'max_tokens > input.purpose.limit()'),
                            (RUST, 'CLI_CANCEL.scope('), (RUST, 'current_grant('),
                            (SWIFT, 'seq == index + 1'), (SWIFT, 'task.cancel()'),
                            (SWIFT, 'ModelProviderCheck.confirm(self, request, authorization)')):
            changed = dict(sources)
            changed[path] = changed[path].replace(token, 'REMOVED')
            with self.assertRaises(AssertionError, msg=token):
                verify(changed)

    def test_vectors(self):
        fixture = json.loads(read('tooling/fixtures/model-chat/golden.json'))
        key = bytes.fromhex(fixture['key_hex'])
        for entry in fixture['vectors']:
            wire = dict(entry['wire'])
            mac = wire.pop('mac')
            canonical = json.dumps(wire, ensure_ascii=False, sort_keys=True, separators=(',', ':'))
            self.assertEqual(canonical, entry['canonical'])
            self.assertEqual(mac, hmac.new(key, canonical.encode(), hashlib.sha256).hexdigest())
            self.assertNotEqual(mac, hmac.new(bytes(32), canonical.encode(), hashlib.sha256).hexdigest())

    def test_integration(self):
        rust = read(RUST)
        self.assertIn('model_chat::start(app.clone())', read('apps/local-voice/src-tauri/src/sync/booktalk_relay.rs'))
        for command in ('model_chat_relay_pair', 'model_chat_relay_authorize'):
            self.assertIn(command, read('apps/local-voice/src-tauri/src/lib.rs'))
            self.assertIn(command, read('apps/local-voice/src/bindings.ts'))
        self.assertIn('RelayModelStore.shared.choices()', read('apps/apple/iPhone/Models/ModelCatalog.swift'))
        hub = read('apps/apple/iPhone/BookTalk/BookTalkRelayHub.swift')
        for token in ('SyncCrypto.seal(', 'collection: BookTalkRelay.collection',
                      'enrollment.keyId == wire.keyId', 'current.token == expected.token',
                      'RelayModelStore.shared.acceptsRequests'):
            self.assertIn(token, hub)
        for token in ('current_grant(', 'cancel_owned(', 'grant.functions', 'permitted(grant, &input.model)'):
            self.assertIn(token, rust)
        ledger = read('apps/local-voice/src-tauri/src/llm_versand/versand.rs')
        self.assertIn('Err(Fehlschlag::Fehler(VersandFehler::Abgebrochen))', ledger)
        self.assertIn('abschliessen(quelle, r2.as_ref(), Ok(versuch))', ledger)
        for path in (RUST.replace('model_chat.rs', 'model_chat_tests.rs'),
                     'apps/apple/VoiceCore/Tests/VoiceCoreTests/RelayChatTests.swift'):
            self.assertIn('tooling/fixtures/model-chat/golden.json', read(path))


if __name__ == '__main__':
    unittest.main()
