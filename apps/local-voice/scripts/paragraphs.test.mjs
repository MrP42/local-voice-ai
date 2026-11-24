// lva#781: Absatzsprünge der Wiedergabeleiste.
// Lauf: node --test scripts/paragraphs.test.mjs
import assert from "node:assert/strict";
import test from "node:test";

import { paragraphStarts, paragraphTarget } from "../src/lib/reader/paragraphs.ts";

const TEXT = "Erster Absatz. Zwei Sätze.\n\nZweiter Absatz.\n \n\nDritter.";
const P2 = TEXT.indexOf("Zweiter");
const P3 = TEXT.indexOf("Dritter");

test("Absatzanfänge nach Leerzeilen", () => {
  assert.deepEqual(paragraphStarts(TEXT), [0, P2, P3]);
  assert.deepEqual(paragraphStarts("  \n"), []);
  assert.deepEqual(paragraphStarts("\n\nNur einer"), [2]);
});

test("vor springt zum nächsten Absatz, am Ende nicht weiter", () => {
  assert.equal(paragraphTarget(TEXT, 5, 1), P2);
  assert.equal(paragraphTarget(TEXT, P2, 1), P3);
  assert.equal(paragraphTarget(TEXT, P3 + 3, 1), null);
});

test("zurück: erst Absatzanfang, dann voriger Absatz", () => {
  assert.equal(paragraphTarget(TEXT, P2 + 8, -1), P2);
  assert.equal(paragraphTarget(TEXT, P2, -1), 0);
  assert.equal(paragraphTarget(TEXT, P2 + 1, -1), 0);
  assert.equal(paragraphTarget(TEXT, 0, -1), 0);
  assert.equal(paragraphTarget("", 0, -1), null);
});
