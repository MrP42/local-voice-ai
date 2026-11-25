// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/llmFehler.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  cliSperre,
  modellListenFehler,
  modellListenSchluessel,
  netzFehlerCode,
  netzFehlerHost,
  netzFehlerText,
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

test("Download: nur der validierte Zielhost darf in die Meldung", () => {
  assert.equal(netzFehlerHost("net_fault:blocked:cdn-lfs.hf.co"), "cdn-lfs.hf.co");
  assert.equal(netzFehlerCode("net_fault:blocked:cdn-lfs.hf.co"), "blocked");
  for (const roh of [
    "net_fault:proxyAuth",
    "net_fault:blocked:https://cdn.hf.co/pfad?token=GEHEIM",
    "net_fault:blocked:cdn.hf.co?token=GEHEIM",
    "net_fault:blocked:token@cdn.hf.co",
    "net_fault:blocked:cdn.hf.co\nGEHEIM",
    "net_fault:blocked:cdn.hf.co\n",
    `net_fault:blocked:${"a".repeat(254)}`,
  ]) assert.equal(netzFehlerHost(roh), null);
});

test("Download: Meldung nennt den CDN-Host fuer die IT", () => {
  const roh = "net_fault:blocked:cdn-lfs.hf.co";
  assert.equal(netzFehlerText(roh, "Bitte huggingface.co freigeben."), "Bitte cdn-lfs.hf.co freigeben.");
  assert.equal(netzFehlerText(roh, "Proxy nicht erreichbar."), "Proxy nicht erreichbar. (cdn-lfs.hf.co)");
  assert.equal(netzFehlerText("net_fault:proxy", "Proxy pruefen."), "Proxy pruefen.");
  assert.equal(netzFehlerText("net_fault:blocked:cdn.hf.co?token=GEHEIM", "Gesperrt."), "Gesperrt.");
  assert.equal(netzFehlerText(roh, ""), "");
});
