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
