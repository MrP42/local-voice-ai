import { test, expect } from "@playwright/test";
import {
  applyCleanup,
  applyCleanupInRange,
  CLEANUP_RULES,
  joinFragments,
  linesToParagraphs,
  stripTimestamps,
  tidySpaces,
} from "../src/lib/reader/textCleanup";
import {
  applyGrammar,
  applyStructure,
  buildAssistInput,
  parseAssistAnswer,
} from "../src/lib/reader/textAssist";
import { applyFormat, findAll, replaceAll } from "../src/lib/reader/editorOps";

// Reine Logik des Editor-Menüs: Regeln ohne KI, das Zeilenprotokoll der
// KI-Textarbeit und Suchen/Formatieren. Beispiel aus Patricks Hörbuch-
// Transkript (Zeitstempel, abgerissene Initialen, Umbruch nach einer URL).
const TRANSCRIPT = [
  "[2:46]: Sie schreiben entweder langatmig oder so trocken.",
  "[3:00]: J.",
  "[3:01]: L.",
  "[3:01]: Collins hat mit diesem Stil nichts am Hut.",
  "[3:47]: Genau das passierte, als Jim eine Reihe von Beiträgen auf",
  "www.jlcollins.com",
  "[4:00]: zu schreiben begann.",
].join("\n");

test("timestamps and leftover colons go away", () => {
  const r = stripTimestamps(
    "[0:00]: Willkommen.\n: Weiter.\n(01:02:03) Ende.\nUhr 12:30: bleibt.",
  );
  expect(r.text).toBe("Willkommen.\nWeiter.\nEnde.\nUhr 12:30: bleibt.");
  expect(r.changes).toBe(3);
});

test("speaker markers and tags survive the timestamp rule", () => {
  const r = stripTimestamps("<Erzählerin> [ruhig] Es war einmal.");
  expect(r.text).toBe("<Erzählerin> [ruhig] Es war einmal.");
  expect(r.changes).toBe(0);
});

test("broken initials and lines are joined", () => {
  const r = joinFragments(stripTimestamps(TRANSCRIPT).text);
  expect(r.text).toBe(
    [
      "Sie schreiben entweder langatmig oder so trocken.",
      "J. L. Collins hat mit diesem Stil nichts am Hut.",
      "Genau das passierte, als Jim eine Reihe von Beiträgen auf www.jlcollins.com zu schreiben begann.",
    ].join("\n"),
  );
});

test("joining works with timestamps still in place", () => {
  const r = joinFragments(TRANSCRIPT);
  const lines = r.text.split("\n");
  expect(lines[1]).toBe(
    "[3:00]: J. L. Collins hat mit diesem Stil nichts am Hut.",
  );
  expect(lines).toHaveLength(3);
});

test("hyphenation at the line end is joined without the hyphen", () => {
  expect(joinFragments("während die Ge-\ndanken abschweifen.").text).toBe(
    "während die Gedanken abschweifen.",
  );
  // Großschreibung danach: kein Trennstrich, sondern Bindestrich-Wort.
  expect(joinFragments("Nord-\nSüd-Gefälle.").text).toBe("Nord-\nSüd-Gefälle.");
});

test("blank lines, speakers and headings are never joined", () => {
  const text = "# Kapitel\nerster Satz\n\n<Bob> und dann\nweiter";
  expect(joinFragments(text).text).toBe(
    "# Kapitel\nerster Satz\n\n<Bob> und dann\nweiter",
  );
});

test("spaces are tidied without touching list indentation", () => {
  expect(tidySpaces("Ein  Satz , mit   Lücken .  ").text).toBe(
    "Ein Satz, mit Lücken.",
  );
  expect(tidySpaces("  - Punkt").text).toBe("  - Punkt");
});

test("sentence lines become paragraphs", () => {
  const lines = Array.from(
    { length: 30 },
    (_, i) => `Das ist der Satz Nummer ${i + 1} in diesem Text.`,
  );
  const r = linesToParagraphs(lines.join("\n"));
  const paras = r.text.split("\n\n");
  expect(paras.length).toBeGreaterThan(1);
  expect(paras.every((p) => !p.includes("\n"))).toBe(true);
  expect(r.text.replace(/\s+/g, " ")).toBe(lines.join(" "));
});

test("all rules run in order on the transcript", () => {
  const r = applyCleanup(TRANSCRIPT, CLEANUP_RULES);
  expect(r.text).not.toMatch(/\[\d+:\d+\]|^:/m);
  expect(r.text).toContain("J. L. Collins");
  expect(r.text).toContain("www.jlcollins.com zu schreiben begann.");
});

test("cleanup in a selection leaves the rest alone", () => {
  const text = "[0:01]: Eins.\n[0:02]: Zwei.\n[0:03]: Drei.";
  const start = text.indexOf("Zwei");
  const r = applyCleanupInRange(text, ["timestamps"], start, start + 3);
  expect(r.text).toBe("[0:01]: Eins.\nZwei.\n[0:03]: Drei.");
});

test("the model only sees numbered bodies without prefixes", () => {
  const input = buildAssistInput(
    "[0:01]: Das ist gut.\n\n<Bob> Hallo Welt.\n---",
  );
  expect(input.prompt).toBe("1|Das ist gut.\n3|Hallo Welt.");
  expect(input.sent).toEqual([0, 2]);
});

test("grammar answers replace only sent lines and keep prefixes", () => {
  const text = "[0:01]: Das sind ein Fehler.\n<Bob> Alles gut.";
  const input = buildAssistInput(text);
  const r = applyGrammar(
    text,
    input,
    "Hier die Korrekturen:\n1|Das ist ein Fehler.\n7|Fremd.",
  );
  expect(r.text).toBe("[0:01]: Das ist ein Fehler.\n<Bob> Alles gut.");
  expect(r.changes).toBe(1);
});

test("grammar answers that rewrite too much or drop tags are rejected", () => {
  const text =
    "Ein [ruhig] kurzer Satz mit Tag.\nEin zweiter, ganz normaler Satz hier.";
  const input = buildAssistInput(text);
  const r = applyGrammar(
    text,
    input,
    "1|Ein kurzer Satz mit Tag.\n2|Etwas völlig anderes, das viel viel länger ist als vorher und nichts mehr gemein hat.",
  );
  expect(r.text).toBe(text);
  expect(r.rejected).toBe(2);
});

test("OK and chatter parse to nothing", () => {
  expect(parseAssistAnswer("OK").size).toBe(0);
  expect(parseAssistAnswer("- 3| H2\nL4|P").get(2)).toBe("H2");
});

test("structure answers build paragraphs and headings", () => {
  const text = [
    "[0:00]: Der Anfang",
    "[0:02]: Wenn man nach einem Stern greift, steht man da.",
    "[0:05]: Aber nicht mit schmutzigen Händen.",
    "[0:09]: Ein neuer Gedanke beginnt.",
    "[0:12]: Und er geht weiter.",
  ].join("\n");
  const input = buildAssistInput(text);
  const r = applyStructure(text, input, "1|H2\n4|P");
  expect(r.text).toBe(
    [
      "## Der Anfang",
      "",
      "[0:02]: Wenn man nach einem Stern greift, steht man da. Aber nicht mit schmutzigen Händen.",
      "",
      "[0:09]: Ein neuer Gedanke beginnt. Und er geht weiter.",
    ].join("\n"),
  );
});

test("an empty structure answer changes nothing", () => {
  const text = "Satz eins.\nSatz zwei.";
  const r = applyStructure(text, buildAssistInput(text), "OK");
  expect(r.text).toBe(text);
  expect(r.changes).toBe(0);
});

test("find respects case, whole words and expressions", () => {
  const text = "Buch buch Buchhandel";
  const base = { caseSensitive: false, wholeWord: false, regex: false };
  expect(findAll(text, "buch", base)).toHaveLength(3);
  expect(findAll(text, "Buch", { ...base, caseSensitive: true })).toHaveLength(
    2,
  );
  expect(findAll(text, "buch", { ...base, wholeWord: true })).toHaveLength(2);
  expect(findAll(text, "B\\w+l", { ...base, regex: true })).toEqual([
    { start: 10, end: 20 },
  ]);
  expect(findAll(text, "(", { ...base, regex: true })).toEqual([]);
});

test("replace all with groups", () => {
  const r = replaceAll(
    "[0:01]: A\n[0:02]: B",
    "^\\[(\\d+:\\d+)\\]: ",
    "($1) ",
    {
      caseSensitive: false,
      wholeWord: false,
      regex: true,
    },
  );
  // `^` ohne m-Flag trifft nur den Anfang.
  expect(r.text).toBe("(0:01) A\n[0:02]: B");
  expect(r.changes).toBe(1);
});

test("formatting toggles headings and wraps bold", () => {
  const text = "Kapitel eins\nText.";
  const h = applyFormat(text, 2, 2, "h1");
  expect(text.slice(0, h.start) + h.insert + text.slice(h.end)).toBe(
    "# Kapitel eins\nText.",
  );
  const again = "# Kapitel eins\nText.";
  const off = applyFormat(again, 3, 3, "h1");
  expect(again.slice(0, off.start) + off.insert + again.slice(off.end)).toBe(
    "Kapitel eins\nText.",
  );
  const b = applyFormat(text, 0, 7, "bold");
  expect(text.slice(0, b.start) + b.insert + text.slice(b.end)).toBe(
    "**Kapitel** eins\nText.",
  );
  const bold = "**Kapitel** eins";
  const unb = applyFormat(bold, 2, 9, "bold");
  expect(bold.slice(0, unb.start) + unb.insert + bold.slice(unb.end)).toBe(
    "Kapitel eins",
  );
});
