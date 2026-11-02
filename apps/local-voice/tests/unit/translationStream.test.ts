// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/translationStream.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  addChunk,
  emptyStream,
  progressParts,
  progressPercent,
  progressRatio,
  shortTitle,
  streamText,
} from "../../src/lib/reader/translationStream.ts";

test("Abschnitte in Reihenfolge werden mit Zeilenumbruch verbunden", () => {
  let s = emptyStream();
  s = addChunk(s, { index: 0, text: "A" });
  s = addChunk(s, { index: 1, text: "B" });
  assert.equal(streamText(s), "A\nB");
});

test("Ein Abschnitt außer der Reihe wartet auf die Lücke", () => {
  let s = emptyStream();
  s = addChunk(s, { index: 1, text: "B" });
  assert.equal(streamText(s), "");
  s = addChunk(s, { index: 0, text: "A" });
  assert.equal(streamText(s), "A\nB");
});

test("Doppelte Meldung (Fortsetzen) dupliziert nichts", () => {
  let s = emptyStream();
  s = addChunk(s, { index: 0, text: "A" });
  s = addChunk(s, { index: 0, text: "A" });
  assert.equal(streamText(s), "A");
});

test("Fortschritt: Zeichen vor Abschnitten, sonst Abschnitte", () => {
  const base = { kind: "translation" as const, done: 12, total: 41 };
  assert.equal(
    progressPercent({ ...base, doneChars: 290, totalChars: 1000 }),
    29,
  );
  assert.equal(progressPercent(base), 29);
  assert.equal(progressRatio({ ...base, total: 0, done: 0 }), 0);
  assert.equal(
    progressRatio({ ...base, doneChars: 5000, totalChars: 1000 }),
    1,
  );
});

test("Kapitel wird 1-basiert angezeigt, Abschnitt zählt den laufenden", () => {
  const p = progressParts({
    kind: "translation",
    done: 11,
    total: 41,
    chapter: 2,
    chapters: 18,
    chapterTitle: "Wie alles begann",
    doneChars: 290,
    totalChars: 1000,
  });
  assert.deepEqual(p, {
    chapter: 3,
    chapters: 18,
    section: 12,
    total: 41,
    percent: 29,
    title: "Wie alles begann",
  });
  assert.equal(
    progressParts({ kind: "translation", done: 0, total: 5 }).chapter,
    null,
  );
});

test("Titel wird gekürzt", () => {
  assert.equal(shortTitle(null), "");
  assert.equal(shortTitle("  Kurz  "), "Kurz");
  assert.equal(shortTitle("x".repeat(60), 10).length, 10);
});
