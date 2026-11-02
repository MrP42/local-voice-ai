// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/proofread.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  leadingParagraphs,
  paragraphChanges,
  paragraphSpans,
  remapOffset,
  remapSync,
  wordDiff,
} from "../../src/lib/library/proofread.ts";
import { textWithSync } from "../../src/lib/library/transcript.ts";

const PARAS = [
  { text: "das haus ist gross und alt", start_ms: 1000, end_ms: 4000 },
  { text: "dann kam der winter", start_ms: 9000, end_ms: 12000 },
  { text: "es schneite tagelang", start_ms: 20000, end_ms: 23000 },
];
const PREFIX = "# Mein Buch\n\nvon A. Autor\n\n";

test("Absatzgrenzen: Start und Ende je Absatz, Rückweg über join", () => {
  const text = "ab\n\ncde\n\nf";
  assert.deepEqual(paragraphSpans(text), [
    { start: 0, end: 2 },
    { start: 4, end: 7 },
    { start: 9, end: 10 },
  ]);
  assert.equal(paragraphSpans("").length, 1);
});

test("die Zeitachse bleibt auf demselben Absatz, auch wenn Text länger und kürzer wird", () => {
  const { text, sync } = textWithSync(PARAS, PREFIX);
  const fixed = text
    .replace("das haus ist gross und alt", "Das Haus ist groß und alt.")
    .replace("dann kam der winter", "Dann kam der Winter.")
    .replace("es schneite tagelang", "Es schneite tagelang.");
  assert.notEqual(fixed, text);
  const next = remapSync(text, fixed, sync);
  assert.equal(next.length, sync.length);
  const starts = paragraphSpans(fixed).map((s) => s.start);
  // Stützpunkte = Absatzanfänge (die letzten drei Absätze, davor Titelei).
  const firstPara = starts.length - 3;
  next.forEach((p, i) => {
    assert.equal(p.chars, starts[firstPara + i], `Stützpunkt ${i}`);
    assert.equal(p.ms, sync[i].ms, "Zeit bleibt");
    assert.ok(
      fixed
        .slice(p.chars)
        .startsWith(["Das Haus", "Dann kam", "Es schneite"][i]),
    );
  });
});

test("eine Stelle mitten im Absatz bleibt im selben Absatz im gleichen Verhältnis", () => {
  const before = "eins zwei\n\ndrei vier fünf sechs\n\nsieben";
  const after = "Eins, zwei.\n\nDrei, vier, fünf, sechs.\n\nSieben.";
  const at = before.indexOf("fünf");
  const mapped = remapOffset(before, after, at);
  const spans = paragraphSpans(after);
  assert.ok(mapped >= spans[1].start && mapped <= spans[1].end);
  // Absatzanfang bleibt Absatzanfang, Ende bleibt Ende.
  assert.equal(remapOffset(before, after, 0), 0);
  assert.equal(remapOffset(before, after, before.length), after.length);
  assert.equal(
    remapOffset(before, after, before.indexOf("sieben")),
    spans[2].start,
  );
});

test("bei anderer Absatzzahl wird im Verhältnis gerechnet statt falsch zugeordnet", () => {
  assert.equal(remapOffset("aaaa\n\nbbbb", "cc", 5), 1);
});

test("Zeitachse ohne Änderung bleibt gleich, Reihenfolge bleibt aufsteigend", () => {
  const { text, sync } = textWithSync(PARAS, PREFIX);
  assert.deepEqual(remapSync(text, text, sync), sync);
  const shorter = text.replace("das haus ist gross und alt", "Haus.");
  const next = remapSync(text, shorter, sync);
  for (let i = 1; i < next.length; i++)
    assert.ok(next[i].chars >= next[i - 1].chars);
});

test("Wortvergleich markiert nur geänderte Wörter", () => {
  const ops = wordDiff("das haus ist gross", "Das Haus ist groß.");
  const words = (kind: string) =>
    ops
      .filter((o) => o.kind === kind)
      .flatMap((o) => o.text.split(/\s+/))
      .filter(Boolean);
  assert.deepEqual(words("del"), ["das", "haus", "gross"]);
  assert.deepEqual(words("add"), ["Das", "Haus", "groß."]);
  assert.ok(ops.some((o) => o.kind === "same" && o.text.includes("ist")));
  // Aus den Stücken lassen sich beide Fassungen wieder zusammensetzen.
  assert.equal(
    ops
      .filter((o) => o.kind !== "add")
      .map((o) => o.text)
      .join(""),
    "das haus ist gross",
  );
  assert.equal(
    ops
      .filter((o) => o.kind !== "del")
      .map((o) => o.text)
      .join(""),
    "Das Haus ist groß.",
  );
});

test("der Absatzvergleich nennt nur geänderte Absätze mit ihrem Index", () => {
  const a = "eins\n\nzwei\n\ndrei";
  const b = "eins\n\nZwei.\n\ndrei";
  const changes = paragraphChanges(a, b);
  assert.equal(changes.length, 1);
  assert.equal(changes[0].index, 1);
  assert.equal(changes[0].before, "zwei");
  assert.equal(changes[0].after, "Zwei.");
  assert.deepEqual(paragraphChanges(a, a), []);
});

test("Titel- und Autorzeile zählen nicht zu den Absätzen der Anzeige", () => {
  assert.equal(leadingParagraphs("# Buch\n\nvon A. Autor\n\nText"), 2);
  assert.equal(leadingParagraphs("# Buch\n\nText"), 1);
  assert.equal(leadingParagraphs("Text\n\nmehr"), 0);
});
