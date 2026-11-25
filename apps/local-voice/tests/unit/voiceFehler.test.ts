import test from "node:test";
import assert from "node:assert/strict";
import { fehlerAktion, hinweisAusFehler } from "../../src/components/voice-call/projekt.ts";

test("Turnfehler bleiben als inhaltsfreie Hinweise sichtbar", () => {
  for (const code of [
    "cli_missing", "cli_login", "cli_limit", "zeitueberschreitung",
    "tor_verworfen", "schutz_gesperrt", "nur_lokal", "einstufung",
    "anbieter_fehlt", "netz", "http_status=407", "unbekannt",
  ]) {
    const h = hinweisAusFehler(`voiceCall.fehler.${code}`);
    assert.deepEqual(h, { code: `voiceCall.fehler.${code}` });
    assert.ok(!JSON.stringify(h).includes("GEHEIM"));
  }
  assert.deepEqual(hinweisAusFehler("GEHEIM"), { code: "voiceCall.fehler.unbekannt" });
  assert.equal(hinweisAusFehler("voiceCall.fehler.abgebrochen"), null);
  assert.deepEqual(hinweisAusFehler("projekt.budget: GEHEIM"), { code: "projekt.budget" });
});

test("Jeder Fehler hat eine konkrete Aktion, Abbruch keinen Hinweis", () => {
  for (const [code, aktion] of [
    ["cli_missing", "einrichten"], ["cli_login", "anmelden"],
    ["cli_limit", "erneut"], ["zeitueberschreitung", "erneut"],
    ["tor_verworfen", "erneut"], ["schutz_gesperrt", "modell"],
    ["nur_lokal", "modell"], ["einstufung", "modell"],
    ["anbieter_fehlt", "modell"], ["netz", "erneut"],
    ["http_status=407", "erneut"], ["unbekannt", "erneut"],
  ]) {
    assert.equal(fehlerAktion(`voiceCall.fehler.${code}`), aktion);
  }
  assert.deepEqual(hinweisAusFehler("voiceCall.fehler.cli_login: GEHEIM"), {
    code: "voiceCall.fehler.unbekannt",
  });
});
