// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/follow.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  makeFollowMap,
  paragraphAt,
  splitParagraphs,
  textIsNewer,
} from "../../src/lib/library/follow.ts";
import {
  textWithSync,
  transcriptParagraphs,
} from "../../src/lib/library/transcript.ts";

test("Absätze mit Beginn im Text", () => {
  const text = "# Titel\n\nvon Autor\n\nErster Absatz.\n\n\nZweiter.";
  const p = splitParagraphs(text);
  assert.deepEqual(
    p.map((x) => x.text),
    ["Titel", "von Autor", "Erster Absatz.", "Zweiter."],
  );
  assert.equal(p[0].heading, true);
  assert.equal(text.slice(p[2].start, p[2].end), "Erster Absatz.");
  assert.equal(paragraphAt(p, p[2].start + 3), 2);
  assert.equal(paragraphAt(p, 0), 0);
  assert.equal(paragraphAt(p, 9999), 3);
});

test("mit Zeitachse: Absatzbeginn ↔ Millisekunden, hin und zurück", () => {
  const map = makeFollowMap(
    [
      { chars: 20, ms: 0 },
      { chars: 120, ms: 10_000 },
      { chars: 420, ms: 40_000 },
    ],
    520,
    50_000,
  );
  assert.equal(map.approx, false);
  assert.equal(map.msToChars(10_000), 120);
  assert.equal(map.msToChars(25_000), 270);
  assert.equal(map.charsToMs(270), 25_000);
  assert.equal(map.charsToMs(420), 40_000);
  assert.equal(map.msToChars(99_999), 520);
  assert.equal(map.charsToMs(0), 0);
});

test("ohne Zeitachse anteilig und „ungefähr“", () => {
  const map = makeFollowMap([], 1000, 100_000);
  assert.equal(map.approx, true);
  assert.equal(map.msToChars(50_000), 500);
  assert.equal(map.charsToMs(250), 25_000);
});

test("Transkript → Text samt Stützpunkten über mehrere Teile mit Versatz", () => {
  const part1 = [
    { text: "Eins.", start_ms: 0, end_ms: 1000 },
    { text: "Zwei.", start_ms: 5000, end_ms: 6000 },
  ];
  const part2 = [{ text: "Drei.", start_ms: 0, end_ms: 1000 }].map((s) => ({
    ...s,
    start_ms: s.start_ms + 10_000,
    end_ms: s.end_ms + 10_000,
  }));
  const paragraphs = transcriptParagraphs([...part1, ...part2]);
  assert.equal(paragraphs.length, 3);
  const { text, sync } = textWithSync(paragraphs, "# Buch\n\n");
  assert.equal(text, "# Buch\n\nEins.\n\nZwei.\n\nDrei.");
  assert.deepEqual(sync, [
    { chars: 8, ms: 0 },
    { chars: 15, ms: 5000 },
    { chars: 22, ms: 10_000 },
  ]);
  for (const s of sync)
    assert.ok(text.slice(s.chars).match(/^(Eins|Zwei|Drei)\./));
});

test("Textfortschritt zählt nur, wenn er klar jünger ist", () => {
  assert.equal(textIsNewer(10_000, 10_000), false);
  assert.equal(textIsNewer(10_500, 10_000), false);
  assert.equal(textIsNewer(20_000, 10_000), true);
});
