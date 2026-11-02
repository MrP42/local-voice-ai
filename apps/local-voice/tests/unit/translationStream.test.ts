// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/translationStream.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  addChunk,
  describeTranslateError,
  sectionChars,
  workerStatus,
  emptyStream,
  progressParts,
  progressPercent,
  progressRatio,
  shortTitle,
  warningText,
  workerRatios,
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

test("Worker-Anteile und Warnungstexte", () => {
  const p = {
    kind: "translation" as const,
    done: 1,
    total: 4,
    workers: [
      { done: 1, total: 2 },
      { done: 0, total: 0 },
    ],
  };
  assert.deepEqual(workerRatios(p), [0.5, 0]);
  assert.deepEqual(workerRatios({ ...p, workers: null }), []);
  assert.equal(warningText("x"), "x");
  assert.equal(
    warningText({ chapter: 2, section: 4, message: "Seitenmarke fehlt" }),
    "Kapitel 3, Abschnitt 5: Seitenmarke fehlt",
  );
});

test("Fehlermeldung: Ursache und Ort werden erkannt", () => {
  const e = describeTranslateError(
    "Übersetzung bei Kapitel 3, Abschnitt 2 von 5 abgebrochen: cli_timeout after 300s",
  );
  assert.equal(e.kind, "timeout");
  assert.equal(e.chapter, 3);
  assert.equal(e.section, 2);
  assert.equal(e.sections, 5);
  assert.equal(e.detail, "cli_timeout after 300s");
});

test("Fehlermeldung: weitere bekannte Ursachen", () => {
  assert.equal(describeTranslateError("HTTP 429").kind, "rateLimit");
  assert.equal(describeTranslateError("hit the rate limit").kind, "rateLimit");
  assert.equal(
    describeTranslateError("Selected model is at capacity").kind,
    "capacity",
  );
  assert.equal(describeTranslateError("truncated").kind, "truncated");
  assert.equal(
    describeTranslateError(
      "Kapitel 4, Abschnitt 14: Zeitüberschreitung des Modells",
    ).kind,
    "timeout",
  );
});

test("Fehlermeldung: unbekannt und ohne Ort", () => {
  const e = describeTranslateError("boom");
  assert.equal(e.kind, "unknown");
  assert.equal(e.chapter, null);
  assert.equal(e.section, null);
  assert.equal(e.detail, "boom");
});

test("Abschnittsgröße aus Gesamtzeichen, gerundet", () => {
  const base = { kind: "translation" as const, done: 0 };
  assert.equal(sectionChars({ ...base, total: 41, totalChars: 492000 }), 12000);
  assert.equal(sectionChars({ ...base, total: 41, totalChars: 90000 }), 2200);
  assert.equal(sectionChars({ ...base, total: 41 }), null);
});

test("Workerzustand ohne Farbe benennbar", () => {
  assert.equal(workerStatus({ done: 10, total: 10 }, { done: 5 }), "done");
  assert.equal(workerStatus({ done: 3, total: 10 }, { done: 5 }), "running");
  assert.equal(workerStatus({ done: 0, total: 10 }, { done: 0 }), "waiting");
  assert.equal(workerStatus({ done: 0, total: 0 }, { done: 5 }), "waiting");
});
