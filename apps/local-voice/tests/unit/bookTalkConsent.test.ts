// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/bookTalkConsent.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  consentKeyOf,
  isLocalConnection,
  pendingConsent,
  withConsent,
  withoutConsent,
} from "../../src/components/settings/tts/booktalk/consent.ts";

const conn = (
  id: string,
  kind = "openai",
  base_url = "https://api.example.invalid/v1",
) => ({ id, kind, label: id, base_url, enabled: true }) as never;
const model = (id: string, connection_id: string, remote_id: string) =>
  ({ id, connection_id, remote_id, label: remote_id, enabled: true }) as never;

const settings = (consent: string[], active = "m1") => ({
  llm_connections: [
    conn("c1"),
    conn("c2"),
    conn("loc", "local", "http://127.0.0.1:0/v1"),
  ],
  llm_models: [
    model("m1", "c1", "gpt-a"),
    model("m2", "c2", "gpt-a"),
    model("m3", "c1", "gpt-b"),
    model("ml", "loc", "gemma"),
  ],
  llm_active_model_id: active,
  function_models: {},
  book_talk_cloud_consent: consent,
});

test("die Freigabe gilt je Verbindung und Modell, nicht je Anbietertyp", () => {
  assert.equal(
    consentKeyOf({ connection: conn("c1"), model: model("m1", "c1", "gpt-a") }),
    "c1::gpt-a",
  );
  // Gleicher Typ, andere Verbindung: eigene Freigabe.
  assert.equal(pendingConsent(settings(["c1::gpt-a"], "m2"))?.model.id, "m2");
  // Gleiche Verbindung, anderes Modell: eigene Freigabe.
  assert.equal(pendingConsent(settings(["c1::gpt-a"], "m3"))?.model.id, "m3");
  // Genau dieses Paar ist freigegeben.
  assert.equal(pendingConsent(settings(["c1::gpt-a"], "m1")), null);
});

test("lokale Modelle brauchen keine Freigabe", () => {
  assert.equal(
    isLocalConnection(conn("loc", "local", "http://127.0.0.1:0/v1/")),
    true,
  );
  assert.equal(isLocalConnection(conn("c1")), false);
  assert.equal(pendingConsent(settings([], "ml")), null);
});

test("Freigeben und Widerrufen ohne Doppelte", () => {
  assert.deepEqual(withConsent(["a::b"], "a::b"), ["a::b"]);
  assert.deepEqual(withConsent(undefined, "x::y"), ["x::y"]);
  assert.deepEqual(withoutConsent(["a::b", "x::y"], "a::b"), ["x::y"]);
});
