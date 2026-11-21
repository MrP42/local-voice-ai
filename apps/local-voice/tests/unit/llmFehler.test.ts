// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/llmFehler.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  cliSperre,
  modellListenFehler,
  modellListenSchluessel,
  netzFehlerCode,
} from "../../src/lib/llmFehler.ts";

test("Sperre vor dem Installer wird erkannt", () => {
  assert.equal(cliSperre("cli_blocked:blocked"), "blocked");
  assert.equal(cliSperre("cli_blocked:proxyAuth"), "proxyAuth");
  assert.equal(cliSperre("cli_blocked:proxy"), "proxy");
  assert.equal(cliSperre("cli_blocked:unknown"), null);
  assert.equal(cliSperre("cli_failed: x"), null);
});

test("Modellliste: Status und Anbietermeldung", () => {
  assert.deepEqual(modellListenFehler("http_status:401"), {
    status: 401,
    anbieter: "",
  });
  assert.deepEqual(
    modellListenFehler("http_status:401: Incorrect API key provided: sk-…"),
    { status: 401, anbieter: "Incorrect API key provided: sk-…" },
  );
  assert.equal(modellListenFehler("HTTP 401"), null);
  assert.equal(modellListenFehler("Netzanfrage fehlgeschlagen."), null);
});

test("Modellliste: Schlüssel je Status", () => {
  assert.equal(modellListenSchluessel(401), "unauthorized");
  assert.equal(modellListenSchluessel(403), "forbidden");
  assert.equal(modellListenSchluessel(407), "proxyAuth");
  assert.equal(modellListenSchluessel(502), "server");
  assert.equal(modellListenSchluessel(418), "other");
});

test("Download: Netzfehler-Code", () => {
  assert.equal(netzFehlerCode("net_fault:blocked"), "blocked");
  assert.equal(netzFehlerCode("verification_failed"), null);
});
