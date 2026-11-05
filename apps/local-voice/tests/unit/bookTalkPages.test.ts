// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/bookTalkPages.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import { pageAt } from "../../src/components/settings/tts/booktalk/pages.ts";

const text = "{{S. 1}}Anfang. {{S. 2}}Mitte mit 😀 Emoji. {{S. 3}}Ende.";

test("Seite = letzte Marke vor der Stelle (UTF-16)", () => {
  assert.equal(pageAt(text, 0), null);
  assert.equal(pageAt(text, 3), 1);
  assert.equal(pageAt(text, text.indexOf("Mitte")), 2);
  assert.equal(pageAt(text, text.indexOf("Ende")), 3);
  assert.equal(pageAt(text, text.length), 3);
});

test("ohne Seitenmarken gibt es keine Seite", () => {
  assert.equal(pageAt("Nur Text.", 5), null);
});

import {
  scopeLimit,
  scopeText,
} from "../../src/components/settings/tts/booktalk/pages.ts";

const starts = [0, 20, 60];

test("Beschriftung: Seite aus der letzten Marke vor der Grenze", () => {
  const l = scopeLimit(text, text.indexOf("Mitte") + 2, starts);
  assert.deepEqual(l, { kind: "page", page: 2 });
  assert.deepEqual(scopeText(l, "label"), { key: "labelPage", values: { page: 2 } });
  assert.deepEqual(scopeText(l, "aria"), { key: "ariaPage", values: { page: 2 } });
  assert.deepEqual(scopeText(l, "phrase"), { key: "limitPage", values: { page: 2 } });
});

test("Beschriftung: ohne Seitenmarken das Kapitel (Index + 1)", () => {
  const plain = "x".repeat(100);
  assert.deepEqual(scopeLimit(plain, 5, starts), { kind: "chapter", chapter: 1 });
  assert.deepEqual(scopeLimit(plain, 20, starts), { kind: "chapter", chapter: 2 });
  assert.deepEqual(scopeLimit(plain, 99, starts), { kind: "chapter", chapter: 3 });
  assert.deepEqual(scopeText(scopeLimit(plain, 70, starts), "label"), {
    key: "labelChapter",
    values: { chapter: 3 },
  });
});

test("Beschriftung: Grenze 0 oder unbekannt heißt Anfang", () => {
  assert.deepEqual(scopeLimit(text, 0, starts), { kind: "start" });
  assert.deepEqual(scopeLimit(text, undefined, starts), { kind: "start" });
  assert.equal(scopeText({ kind: "start" }, "aria").key, "ariaStart");
});

test("Beschriftung: ohne Marken und ohne Inhaltsverzeichnis bleibt nur „hier“", () => {
  assert.deepEqual(scopeLimit("Nur Text.", 5, []), { kind: "here" });
  assert.equal(scopeText({ kind: "here" }, "label").key, "labelHere");
});

test("Beschriftung: Seitenmarke hat Vorrang vor dem Kapitel", () => {
  assert.deepEqual(scopeLimit(text, text.length, [0]), { kind: "page", page: 3 });
});
