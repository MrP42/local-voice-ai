// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/kurzbuch.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  KURZBUCH_HEADS,
  minutes,
  parseKurzbuch,
  sectionAt,
  titleId,
  trackSentence,
} from "../../src/lib/kurzbuch/kurzbuch.ts";

const DOC = [
  "## Worum es geht",
  "",
  "Ein Buch über Routinen.",
  "",
  "## Für wen",
  "",
  "- Alle",
  "- Teams",
  "- Lernende",
  "",
  "## Anfang: Klein beginnen.",
  "",
  "Erster Satz. " + "Wort ".repeat(60),
  "",
  "## Rückfall: Nie zweimal.",
  "",
  "Zweiter Text. " + "Wort ".repeat(60),
  "",
  "## Fazit",
  "",
  "Ende.",
  "",
].join("\n");

test("die Gliederung wird erkannt", () => {
  const kb = parseKurzbuch(DOC);
  assert.ok(kb);
  assert.equal(kb.about, "Ein Buch über Routinen.");
  assert.deepEqual(kb.audience, ["Alle", "Teams", "Lernende"]);
  assert.equal(kb.ideas.length, 2);
  assert.equal(kb.ideas[0].keyword, "Anfang");
  assert.equal(kb.ideas[0].statement, "Klein beginnen.");
  assert.equal(
    DOC.slice(kb.ideas[1].start).split("\n")[0],
    "## Rückfall: Nie zweimal.",
  );
  assert.equal(kb.ideas[0].end, kb.ideas[1].start);
  assert.equal(kb.conclusion.title, "Fazit");
  assert.equal(sectionAt(kb, kb.ideas[1].start + 5)?.id, kb.ideas[1].id);
});

test("ohne Gliederung bleibt es ein normales Dokument", () => {
  assert.equal(parseKurzbuch("Nur Text."), null);
  assert.equal(parseKurzbuch("## Worum es geht\n\nx\n\n## Fazit\n\ny"), null);
  assert.equal(
    parseKurzbuch(DOC.replace("## Fazit", "## Schluss")),
    null,
    "ohne Fazit unvollständig",
  );
});

test("gehört nur, wenn Anfang und Ende einer Kernaussage gehört wurden", () => {
  const kb = parseKurzbuch(DOC)!;
  const [a, b] = kb.ideas;
  const entered = new Set<string>();
  // Sprung mitten in die erste und ans Ende: nichts.
  assert.equal(
    trackSentence(kb, entered, a.start + (a.end - a.start) / 2, a.end - 1),
    null,
  );
  // Von vorn durch: gehört.
  assert.equal(trackSentence(kb, entered, a.start + 2, a.start + 20), null);
  assert.equal(trackSentence(kb, entered, a.end - 30, a.end - 1), a.id);
  // Die zweite übersprungen: nichts.
  assert.equal(trackSentence(kb, entered, b.end - 30, b.end - 1), null);
});

test("Dauer und Kennungen", () => {
  assert.equal(minutes(10, 200), 1);
  assert.equal(minutes(3000, 200), 15);
  assert.equal(titleId("Anfang: X"), titleId(" anfang: x "));
  assert.notEqual(titleId("A"), titleId("B"));
});

test("die Gliederung wird in jeder Kurzbuch-Sprache erkannt (lva#217)", () => {
  const en = DOC.replace("## Worum es geht", "## What it is about")
    .replace("## Für wen", "## Who it is for")
    .replace("## Fazit", "## Conclusion");
  const kb = parseKurzbuch(en);
  assert.ok(kb);
  assert.equal(kb.about, "Ein Buch über Routinen.");
  assert.equal(kb.audience.length, 3);
  assert.equal(kb.ideas.length, 2);
  assert.equal(kb.conclusion.title, "Conclusion");
  for (const [code, [about, who, end]] of Object.entries(KURZBUCH_HEADS)) {
    const doc = DOC.replace("Worum es geht", about)
      .replace("Für wen", who)
      .replace("## Fazit", `## ${end}`);
    assert.ok(parseKurzbuch(doc), code);
  }
});
