// Lauf: node --test apps/local-voice/scripts/update-config.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { buildConfig } from "./update-config.mjs";

const ID = "de.wolffappliedai.localvoiceai";

test("ohne Variablen keine Aenderung", () => {
  assert.deepEqual(buildConfig({}, ID), {});
});

test("Basis und Kanal ergeben den Endpunkt", () => {
  const c = buildConfig({ LVA_UPDATE_BASE: "https://u.example/wai/", LVA_UPDATE_CHANNEL: "nacht" }, ID);
  assert.deepEqual(c.plugins.updater.endpoints, [`https://u.example/wai/${ID}/nacht/latest.json`]);
  assert.equal(c.plugins.updater.pubkey, undefined);
});

test("Standardkanal ist stable, Schluessel optional", () => {
  const c = buildConfig({ LVA_UPDATE_BASE: "https://u.example", LVA_UPDATE_PUBKEY: " KEY " }, ID);
  assert.ok(c.plugins.updater.endpoints[0].endsWith("/stable/latest.json"));
  assert.equal(c.plugins.updater.pubkey, "KEY");
});

test("leerer Kanal heisst stable", () => {
  const c = buildConfig({ LVA_UPDATE_BASE: "https://u.example", LVA_UPDATE_CHANNEL: " " }, ID);
  assert.ok(c.plugins.updater.endpoints[0].endsWith("/stable/latest.json"));
});

test("unsichere oder falsche Angaben werden abgewiesen", () => {
  assert.throws(() => buildConfig({ LVA_UPDATE_BASE: "http://u.example" }, ID), /HTTPS/);
  assert.throws(() => buildConfig({ LVA_UPDATE_BASE: "https://a:b@u.example" }, ID), /Zugangsdaten/);
  assert.throws(() => buildConfig({ LVA_UPDATE_BASE: "https://u.example/x?a=1" }, ID), /Query/);
  assert.throws(() => buildConfig({ LVA_UPDATE_BASE: "kaputt" }, ID), /URL/);
  assert.throws(() => buildConfig({ LVA_UPDATE_BASE: "https://u.example", LVA_UPDATE_CHANNEL: "nightly" }, ID), /Kanal|CHANNEL/);
});
