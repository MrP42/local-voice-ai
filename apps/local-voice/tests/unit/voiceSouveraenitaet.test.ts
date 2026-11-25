import test from "node:test";
import assert from "node:assert/strict";
import { existsSync } from "node:fs";

test("Schild und Modellauswahl verwenden dieselbe Souveränitätsentscheidung", async () => {
  const url = new URL("../../src/components/voice-call/souveraenitaet.ts", import.meta.url);
  assert.ok(existsSync(url), "Gemeinsame Entscheidung für Auswahl und Schild fehlt");
  const { blockedBy, gespraechsSperren } = await import(url.href);
  const cloud = { id: "abo", label: "Abo", local: false };
  const lokal = { id: "lokal", label: "Lokal", local: true };
  const eu = { ...cloud, jurisdiction: "eu" };
  for (const stufe of ["nur_lokal", "eu", "frei"]) {
    for (const speaker of [cloud, lokal, eu]) {
      for (const denker of [cloud, lokal, eu, undefined]) {
        const sperren = gespraechsSperren(speaker, denker, stufe);
        assert.equal(sperren.includes("sprecher"), blockedBy(speaker, stufe));
        assert.equal(sperren.includes("denker"), !!denker && blockedBy(denker, stufe));
      }
    }
  }
  assert.deepEqual(gespraechsSperren(cloud, lokal, "nur_lokal"), ["sprecher"]);
  assert.deepEqual(gespraechsSperren(lokal, cloud, "nur_lokal"), ["denker"]);
  assert.deepEqual(gespraechsSperren(cloud, cloud, "frei"), []);
});
