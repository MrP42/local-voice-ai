// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/speechTiming.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  cleanSpans,
  clauses,
  spokenOffset,
} from "../../src/lib/speechTiming.ts";

const whole = (text: string) => ({ start: 0, end: text.length });
const wordAt = (text: string, at: number) => {
  let a = at;
  while (a > 0 && !/\s/.test(text[a - 1])) a--;
  let b = at;
  while (b < text.length && !/\s/.test(text[b])) b++;
  return text.slice(a, b);
};

test("Nachklang und Absatzpause verzögern die Markierung nicht mehr", () => {
  // 2 s Stimme, davor 30 ms Vorlauf, dahinter 230 ms Nachklang + 340 ms Absatzpause.
  const text = "Ich hatte zwei Väter, einen reichen und einen armen.";
  const spans = [[30, 2030]] as const;
  // Kurz vor Ende der Stimme ist das letzte Wort zu hören …
  assert.equal(
    wordAt(text, spokenOffset(text, whole(text), spans, 1950)),
    "armen.",
  );
  // … die alte Rechnung über die ganze Dauer (2600 ms) stand noch weit davor.
  const old = Math.floor((1950 / 2600) * text.length);
  assert.notEqual(wordAt(text, old), "armen.");
  assert.ok(text.length - old > 10, `alte Rechnung nur bei Zeichen ${old}`);
});

test("vor dem ersten Laut steht die Markierung am Satzanfang", () => {
  const text = "Reiche arbeiten nicht für Geld.";
  assert.equal(spokenOffset(text, whole(text), [[200, 1800]], 100), 0);
});

test("nach dem letzten Laut bleibt das letzte Wort markiert", () => {
  const text = "Reiche arbeiten nicht für Geld.";
  const at = spokenOffset(text, whole(text), [[0, 1500]], 1900);
  assert.equal(wordAt(text, at), "Geld.");
});

test("Kommapause hält die Markierung am Wort vor dem Komma", () => {
  const text = "Ich hatte zwei Väter, einen reichen und einen armen.";
  // Zwei Abschnitte = zwei Teilsätze, dazwischen 200 ms Kommapause.
  const spans = [
    [0, 900],
    [1100, 2200],
  ] as const;
  assert.deepEqual(
    clauses(text, whole(text)).map((c) => text.slice(c.start, c.end).trim()),
    ["Ich hatte zwei Väter,", "einen reichen und einen armen."],
  );
  assert.equal(
    wordAt(text, spokenOffset(text, whole(text), spans, 1000)),
    "Väter,",
  );
  // Mit Beginn des zweiten Abschnitts springt sie auf den zweiten Teilsatz.
  assert.equal(
    wordAt(text, spokenOffset(text, whole(text), spans, 1110)),
    "einen",
  );
});

test("passen Abschnitte und Teilsätze nicht zusammen, zählt die Sprechzeit als Ganzes", () => {
  const text = "Eins zwei drei vier fünf sechs sieben acht.";
  // Drei Abschnitte, aber kein Komma: Pausen werden herausgerechnet.
  const spans = [
    [0, 1000],
    [1500, 2500],
    [3000, 4000],
  ] as const;
  const inPause = spokenOffset(text, whole(text), spans, 1200);
  const afterPause = spokenOffset(text, whole(text), spans, 1500);
  assert.equal(
    inPause,
    afterPause,
    "in der Pause läuft die Markierung nicht weiter",
  );
  assert.equal(
    spokenOffset(text, whole(text), spans, 2500),
    Math.floor((2 / 3) * text.length),
  );
});

test("Satz mitten im Text: Offsets bleiben im Bereich", () => {
  const text = "Vorher. Mitten im Text steht dieser Satz. Danach.";
  const range = { start: 8, end: 41 };
  for (const t of [0, 500, 1000, 5000]) {
    const at = spokenOffset(text, range, [[0, 1000]], t);
    assert.ok(at >= range.start && at < range.end, `t=${t}: ${at}`);
  }
});

test("ungültige Abschnitte aus dem Ereignis fallen weg", () => {
  assert.deepEqual(cleanSpans(undefined), []);
  assert.deepEqual(cleanSpans([[10, 5]]), []);
  assert.deepEqual(
    cleanSpans([
      [0, 100],
      [50, 200],
    ]),
    [],
  );
  assert.deepEqual(cleanSpans([["a", 1]]), []);
  assert.deepEqual(
    cleanSpans([
      [0, 100],
      [300, 400],
    ]),
    [
      [0, 100],
      [300, 400],
    ],
  );
});
