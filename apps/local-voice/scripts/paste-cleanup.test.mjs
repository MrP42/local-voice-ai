// lva#530 I1: Bereinigung beim Einfügen (reine Logik, keine KI).
// Lauf: node --test scripts/paste-cleanup.test.mjs (Node >= 22.6; Typen werden beim Laden entfernt).
// Alle Vorlagen sind synthetisch (example.org, Musterkreis).
import assert from "node:assert/strict";
import test from "node:test";

import {
  cleanPasted,
  collapseBlankLines,
  isPageNumberLine,
  isWebNoiseLine,
  stripControl,
  stripPageNumbers,
  stripRepeated,
  stripWebNoise,
} from "../src/lib/reader/pasteCleanup.ts";

import { joinFragments } from "../src/lib/reader/textCleanup.ts";

import {
  PASTE_ENTRY_MAX_BYTES,
  PASTE_LOG_MAX_BYTES,
  PASTE_LOG_MAX_ENTRIES,
  appendPasteEntry,
  entryBytes,
  isIsoTime,
  isSha256,
  makeOversizedEntry,
  makePasteEntry,
  parsePasteLog,
  pasteEntryChanges,
  pasteEntryKey,
  removePasteEntry,
  restoreRaw,
  sha256Hex,
  utf8Bytes,
  verifyPasteLog,
} from "../src/lib/reader/pasteLog.ts";

const lines = (...l) => l.join("\n");
const rule = (result, name) =>
  result.items.find((i) => i.rule === name)?.count ?? 0;

/** Langer Fließtext: n Absätze à ~150 Zeichen, damit die 1500-Zeichen-Schwelle greift. */
const absatz = (n) =>
  `Absatz ${n}: Die Verwaltung informiert über den Stand der Umstellung und bittet um Rückmeldung bis Monatsende.`;

test("Kurzer Einzeiler bleibt unangetastet", () => {
  const r = cleanPasted("Hallo  Welt   12");
  assert.equal(r.text, "Hallo  Welt   12");
  assert.equal(r.changes, 0);
  assert.deepEqual(r.items, []);
});

test("Einzeiler mit einem Umbruch am Ende bleibt, nichts zu tun", () => {
  const r = cleanPasted("Guten Morgen zusammen.\n");
  assert.equal(r.changes, 0);
});

test("control: CRLF wird still vereinheitlicht und zählt nicht", () => {
  const r = cleanPasted(
    "Erste Zeile des Textes.\r\nZweite Zeile des Textes.\r\n",
  );
  assert.equal(r.text, "Erste Zeile des Textes.\nZweite Zeile des Textes.");
  assert.equal(r.changes, 0);
});

test("control: Word-Kopie mit NBSP, weichem Trennzeichen und Nullbreiten", () => {
  const raw = "Die Mit­arbeiter​ informieren﻿ sich hier.\nZweite Zeile folgt.";
  const r = cleanPasted(raw);
  assert.equal(
    r.text,
    "Die Mitarbeiter informieren sich hier.\nZweite Zeile folgt.",
  );
  assert.equal(rule(r, "control"), 5);
});

test("control: weiches Trennzeichen am Zeilenende fügt das Wort ohne Leerzeichen", () => {
  const r = stripControl("Die Mit­\narbeiterinnen sind da.");
  assert.equal(r.text, "Die Mitarbeiterinnen sind da.");
  assert.equal(r.changes, 1);
});

test("control: Formfeed wird Umbruch, Steuerzeichen verschwinden, Tab bleibt", () => {
  const r = stripControl("A\fB\u0007C\tD\u0000E");
  assert.equal(r.text, "A\nBC\tDE");
  assert.equal(r.changes, 3);
});

test("pageNumbers: alle Schreibweisen werden erkannt", () => {
  for (const s of [
    "12",
    "- 12 -",
    "– 12 –",
    "Seite 3",
    "Seite 3 von 30",
    "S. 3",
    "3 / 30",
    "Page 3 of 30",
  ]) {
    assert.ok(isPageNumberLine(s), s);
  }
});

test("pageNumbers: Jahreszahl, Liste, Text mit Zahl und unmögliche Seitenbrüche bleiben", () => {
  for (const s of [
    "2026",
    "1.",
    "2)",
    "Seite 3 ist leer",
    "12 Uhr",
    "30 / 3",
    "1234",
    "",
  ]) {
    assert.ok(!isPageNumberLine(s), s);
  }
});

test("pageNumbers: Jahreszahl 2026 allein bleibt im Text", () => {
  const r = cleanPasted(
    lines(
      "Das Jahr",
      "2026",
      "bringt Neuerungen für alle Beschäftigten im Haus.",
    ),
  );
  assert.ok(r.text.includes("2026"));
  assert.equal(rule(r, "pageNumbers"), 0);
});

test("PDF-Kopie: Kopfzeile, Seitenzahlen und Silbentrennung", () => {
  const kopf = "Landratsamt Musterkreis – Mitarbeiterinfo";
  const seite = (n) => `Seite ${n} von 4`;
  const block = (n) =>
    [
      kopf,
      absatz(n * 3 - 2) + " Alle Beschäftigten werden ge-",
      "schult und erhalten eine Einwei-",
      "sung in das neue System.",
      absatz(n * 3 - 1),
      absatz(n * 3),
      "Weitere Hinweise finden Sie unter https://example.org/info und im Intranet des Hauses.",
      "Die Ansprechpartner stehen Ihnen während der Sprechzeiten zur Verfügung und helfen gern.",
      seite(n),
    ].join("\n");
  const raw = [block(1), block(2), block(3), block(4)].join("\n");
  assert.ok(raw.length >= 1500);
  const r = cleanPasted(raw);
  assert.ok(!r.text.includes("Landratsamt Musterkreis – Mitarbeiterinfo"));
  assert.ok(!/Seite \d von 4/.test(r.text));
  assert.ok(
    r.text.includes(
      "geschult und erhalten eine Einweisung in das neue System.",
    ),
  );
  assert.equal(rule(r, "repeated"), 4);
  assert.equal(rule(r, "pageNumbers"), 4);
  assert.ok(rule(r, "fragments") >= 8);
  assert.ok(r.changes >= 16);
});

test("repeated: unter 1500 Zeichen bleibt die Wiederholung", () => {
  const raw = lines(
    "Kopf der Seite",
    "Ein Satz im Text, der hier steht.",
    "Kopf der Seite",
    "Noch ein Satz im Text, der hier steht.",
    "Kopf der Seite",
    "Und ein dritter Satz im Text.",
  );
  const r = stripRepeated(raw);
  assert.equal(r.changes, 0);
  assert.equal(r.text, raw);
});

test("repeated: Gedicht mit Kehrreim unter 1500 Zeichen bleibt", () => {
  const kehrreim = "Oh, du fröhliche Weihnachtszeit";
  const raw = lines(
    kehrreim,
    "Der Schnee fällt leise auf das Dach,",
    "die Kinder sind schon lange wach.",
    "",
    kehrreim,
    "Im Ofen knistert warm das Holz,",
    "die Katze liegt dort still und stolz.",
    "",
    kehrreim,
    "Die Kerzen leuchten hell im Raum,",
    "wir feiern froh den Weihnachtsbaum.",
  );
  const r = cleanPasted(raw);
  assert.equal(rule(r, "repeated"), 0);
  assert.equal(r.text.split(kehrreim).length - 1, 3);
});

test("repeated: dichte Wiederholung in langem Text (Abstand < 8 Zeilen) bleibt", () => {
  const refrain = "Immer wieder neu";
  const body = Array.from({ length: 20 }, (_, i) => absatz(i));
  const raw = lines(
    refrain,
    body[0],
    refrain,
    body[1],
    refrain,
    ...body.slice(2),
  );
  assert.ok(raw.length >= 1500);
  const r = stripRepeated(raw);
  assert.equal(r.changes, 0);
});

test("repeated: Zeilen mit Satzende, Sprecher, Tags und Markdown zählen nicht als Kopfzeile", () => {
  const filler = (from) =>
    Array.from({ length: 9 }, (_, i) => absatz(from + i));
  for (const kopf of [
    "Vielen Dank für Ihre Aufmerksamkeit.",
    "<Erzählerin>",
    "[Pause]",
    "## Überschrift",
  ]) {
    const raw = [
      kopf,
      ...filler(0),
      kopf,
      ...filler(10),
      kopf,
      ...filler(20),
    ].join("\n");
    assert.ok(raw.length >= 1500);
    assert.equal(stripRepeated(raw).changes, 0, kopf);
  }
});

test("repeated: Ziffern werden nur bei Seitenzahl-Muster gleichgesetzt", () => {
  const filler = (from) =>
    Array.from({ length: 9 }, (_, i) => absatz(from + i));
  const raw = [
    "Rundschreiben Musterkreis – Seite 1 von 4",
    ...filler(0),
    "\fRundschreiben Musterkreis – Seite 2 von 4",
    ...filler(10),
    "\fRundschreiben Musterkreis – Seite 3 von 4",
    ...filler(20),
    "\fRundschreiben Musterkreis – Seite 4 von 4",
    ...filler(30),
  ].join("\n");
  const r = stripRepeated(raw);
  assert.equal(r.changes, 4);
  assert.ok(!r.text.includes("Rundschreiben"));
});

test("repeated: Zeilen mit anderer Ziffer ohne Seitenzahl-Muster sind verschieden", () => {
  const filler = (from) =>
    Array.from({ length: 9 }, (_, i) => absatz(from + i));
  const raw = [
    "Rundschreiben 12/2026",
    ...filler(0),
    "Rundschreiben 13/2026",
    ...filler(10),
    "Rundschreiben 14/2026",
    ...filler(20),
  ].join("\n");
  assert.equal(stripRepeated(raw).changes, 0);
});

test("Intranetbeitrag mit Navigation und Bedienresten", () => {
  const raw = lines(
    "Zum Inhalt springen",
    "Startseite",
    "",
    "Neue Telefonanlage ab Montag",
    "",
    "Ab Montag stehen alle Nebenstellen im Haus mit der neuen Anlage zur Verfügung, die Umstellung dauert etwa eine Stunde.",
    "",
    "Drucken · Teilen · E-Mail",
    "",
    "Weitere Informationen finden Sie auf https://example.org/telefon.",
    "",
    "Nach oben",
    "Zurück zur Übersicht",
    "Cookie-Einstellungen",
  );
  const r = cleanPasted(raw);
  assert.ok(
    !/Zum Inhalt springen|Drucken|Nach oben|Zurück zur Übersicht|Cookie-Einstellungen/.test(
      r.text,
    ),
  );
  assert.ok(r.text.includes("Startseite"));
  assert.ok(r.text.includes("Neue Telefonanlage ab Montag"));
  assert.ok(r.text.endsWith("https://example.org/telefon."));
  assert.equal(rule(r, "webNoise"), 5);
  assert.ok(!/\n\n\n/.test(r.text));
});

test("webNoise: Trennzeichen · | / – und Groß-/Kleinschreibung", () => {
  for (const s of [
    "Drucken",
    "DRUCKEN | Teilen",
    "Facebook / LinkedIn / X / Xing / WhatsApp",
    "Print – Share",
    "skip to content",
    "Back to top",
    "  E-Mail  ",
  ]) {
    assert.ok(isWebNoiseLine(s), s);
  }
  for (const s of [
    "Drucken Sie das Formular aus",
    "Zurück zur Übersicht der Anträge",
    "",
    "E-Mail an uns",
  ]) {
    assert.ok(!isWebNoiseLine(s), s);
  }
});

test("webNoise: Zeile mit weiterem Text bleibt, Block aus Bedienwörtern geht", () => {
  const r = stripWebNoise(
    "Teilen Sie uns Ihre Meinung mit.\nTeilen\nDrucken\nText danach.",
  );
  assert.equal(r.text, "Teilen Sie uns Ihre Meinung mit.\nText danach.");
  assert.equal(r.changes, 2);
});

test("Outlook-Newsletter: Fußzeile und Bedienreste raus, Inhalt bleibt", () => {
  const raw = lines(
    "Newsletter Musterkreis   Mai",
    "",
    "",
    "",
    "",
    "Liebe Kolleginnen und Kollegen,",
    "",
    "in diesem Monat gibt es drei Neuerungen im Haus, über die wir Sie gern informieren möchten.",
    "",
    "Link kopieren",
    "Weiterlesen",
    "",
    "Mit freundlichen Grüßen",
    "Redaktion",
    "",
    "",
    "",
    "",
  );
  const r = cleanPasted(raw);
  assert.equal(
    r.text,
    lines(
      "Newsletter Musterkreis Mai",
      "",
      "Liebe Kolleginnen und Kollegen,",
      "",
      "in diesem Monat gibt es drei Neuerungen im Haus, über die wir Sie gern informieren möchten.",
      "",
      "Mit freundlichen Grüßen",
      "Redaktion",
    ),
  );
  assert.ok(rule(r, "blankLines") >= 2);
});

test("Silbentrennung wird verbunden, „Bund- und Länder“ bleibt", () => {
  const raw = lines(
    "Die Abstimmung zwischen Bund-",
    "und Länderebene ist wichtig, weil die Zustän-",
    "digkeiten sich überschneiden.",
    "Das gilt auch für die Ge-",
    "meinden im Kreisgebiet.",
  );
  const r = cleanPasted(raw);
  assert.equal(
    r.text,
    lines(
      "Die Abstimmung zwischen Bund- und Länderebene ist wichtig, weil die Zuständigkeiten sich überschneiden.",
      "Das gilt auch für die Gemeinden im Kreisgebiet.",
    ),
  );
});

test("Ergänzungsstrich „Bund- und Länder-“ bleibt erhalten", () => {
  const r = cleanPasted(
    lines(
      "Zusammenarbeit zwischen Bund-",
      "und Länderbehörden wird ausgebaut, hieß es in Musterstadt.",
    ),
  );
  assert.ok(r.text.includes("Bund- und Länderbehörden"));
});

test("Liste „1. …“ bleibt erhalten", () => {
  const raw = lines(
    "Aufgaben bis Freitag:",
    "1. Protokoll prüfen",
    "2. Raum buchen",
    "3. Einladung versenden",
  );
  const r = cleanPasted(raw);
  assert.equal(r.text, raw);
  assert.equal(r.changes, 0);
});

test("Agenda „(10:30) Begrüßung“ bleibt (keine Zeitstempel-Regel)", () => {
  const raw = lines(
    "(10:30) Begrüßung",
    "(10:45) Bericht der Leitung",
    "(11:15) Aussprache und Beschlüsse",
    "[12:00] Mittagspause",
  );
  const r = cleanPasted(raw);
  assert.equal(r.text, raw);
  assert.equal(r.changes, 0);
});

test("Einzelne Satzzeilen werden nicht zu Absätzen verbunden", () => {
  const raw = lines(
    "Der Antrag wurde gestern eingereicht und von der Leitung bestätigt.",
    "Die Rückmeldung erfolgt innerhalb von zwei Wochen nach Eingang der Unterlagen.",
    "Bitte halten Sie die Unterlagen bereit, falls Rückfragen entstehen sollten.",
  );
  assert.equal(cleanPasted(raw).text, raw);
});

test("Sprecher, Tags, Seitenmarken und Markdown bleiben unangetastet", () => {
  const raw = lines(
    "<Erzählerin> [ruhig] Es war einmal ein kleines Haus am Rand des Dorfes.",
    "<Vater:streng> Komm sofort herein, es wird kalt.",
    "{{S. 12}}",
    "# Kapitel eins",
    "> Zitat aus dem Brief von Musterstadt",
    "- Punkt eins",
    "- Punkt zwei",
    "---",
    "[Pause 2s]",
  );
  const r = cleanPasted(raw);
  assert.equal(r.text, raw);
  assert.equal(r.changes, 0);
});

test("Seitenzahl-Zeile in Markdown-/Sprecher-Umgebung: nur die nackte Zahl fällt", () => {
  const raw = lines(
    "<Erzählerin> Es regnete den ganzen Tag lang weiter.",
    "7",
    "<Vater> Dann bleiben wir eben drinnen sitzen.",
  );
  const r = cleanPasted(raw);
  assert.equal(
    r.text,
    lines(
      "<Erzählerin> Es regnete den ganzen Tag lang weiter.",
      "<Vater> Dann bleiben wir eben drinnen sitzen.",
    ),
  );
  assert.equal(rule(r, "pageNumbers"), 1);
});

test("Doppelte Leerzeichen und Leerzeichen vor Satzzeichen werden bereinigt", () => {
  const r = cleanPasted(
    lines(
      "Das  ist  ein Satz , der  Fehler hat .",
      "Und eine zweite Zeile im Text.",
    ),
  );
  assert.equal(
    r.text,
    lines(
      "Das ist ein Satz, der Fehler hat.",
      "Und eine zweite Zeile im Text.",
    ),
  );
  assert.equal(rule(r, "spaces"), 1);
});

test("blankLines: 3+ Leerzeilen werden eine, zwei bleiben, Anfang und Ende fallen", () => {
  const r = collapseBlankLines(
    lines("", "", "A", "", "", "B", "", "", "", "C", "", ""),
  );
  assert.equal(r.text, lines("A", "", "", "B", "", "C"));
  assert.equal(r.changes, 3);
});

test("blankLines: einzelner Umbruch am Ende zählt nicht", () => {
  const r = collapseBlankLines("Text\n");
  assert.equal(r.text, "Text");
  assert.equal(r.changes, 0);
});

test("pageNumbers: entfernte Zeile zwischen Leerzeilen hinterlässt keine doppelte Lücke", () => {
  const r = stripPageNumbers(
    lines("Ende der Seite.", "", "12", "", "Anfang der nächsten."),
  );
  assert.equal(r.text, lines("Ende der Seite.", "", "Anfang der nächsten."));
  assert.equal(r.changes, 1);
});

test("pageNumbers: Zahl am Textanfang vor Leerzeile", () => {
  const r = stripPageNumbers(lines("3", "", "Anfang."));
  assert.equal(r.text, "Anfang.");
});

test("Abgerissene Zeile über einen Seitenumbruch hinweg wird verbunden", () => {
  const r = cleanPasted(
    lines(
      "Der Bericht wurde im vergangenen Jahr erstellt und",
      "- 14 -",
      "an alle Ämter im Kreisgebiet verschickt.",
    ),
  );
  assert.equal(
    r.text,
    "Der Bericht wurde im vergangenen Jahr erstellt und an alle Ämter im Kreisgebiet verschickt.",
  );
});

test("rules-Option beschränkt auf einzelne Regeln, Reihenfolge bleibt fest", () => {
  const raw = lines(
    "Drucken",
    "Der Text hat  Leerzeichen und eine Zeile.",
    "12",
  );
  const r = cleanPasted(raw, { rules: ["pageNumbers"] });
  assert.equal(
    r.text,
    lines("Drucken", "Der Text hat  Leerzeichen und eine Zeile."),
  );
  assert.deepEqual(r.items, [{ rule: "pageNumbers", count: 1 }]);
});

test("changes ist die Summe der Einzelposten", () => {
  const raw = lines(
    "Zum Inhalt springen",
    "Das ist der Text  hier.",
    "Seite 3 von 30",
    "Noch eine Zeile Text.",
  );
  const r = cleanPasted(raw);
  assert.equal(
    r.changes,
    r.items.reduce((s, i) => s + i.count, 0),
  );
  assert.ok(r.changes >= 4);
});

test("Idempotenz: ein zweiter Durchlauf ändert nichts mehr", () => {
  const raw = lines(
    "Zum Inhalt springen",
    "",
    "",
    "",
    "",
    "Neues aus dem Landratsamt Musterkreis informiert über Ge-",
    "bühren und  Fristen.",
    "Seite 2 von 9",
    "Drucken | Teilen",
  );
  const first = cleanPasted(raw);
  const second = cleanPasted(first.text);
  assert.equal(second.text, first.text);
  assert.equal(second.changes, 0);
});

// ---------------------------------------------------------------------------
// Fixrunde (Codex-Befunde zu PR #532)
// ---------------------------------------------------------------------------

test("control: Verbinder U+200D/U+200C bleiben (Emoji-Folge, persisches Wort)", () => {
  const frau = "\u{1F469}‍\u{1F4BB}";
  const persisch = "می‌خواهم";
  const r = stripControl(`${frau} und ${persisch}`);
  assert.equal(r.text, `${frau} und ${persisch}`);
  assert.equal(r.changes, 0);
});

test("control: Nullbreite, BOM (auch am Anfang) und weiches Trennzeichen gehen", () => {
  const r = stripControl("﻿A​B﻿C­D");
  assert.equal(r.text, "ABCD");
  assert.equal(r.changes, 4);
});

test("Emoji mit Verbinder übersteht die ganze Bereinigung", () => {
  const raw = lines(
    "Die Kollegin \u{1F469}‍\u{1F4BB} betreut die Umstellung der Telefonanlage im gesamten Haus.",
    "Drucken",
    "Teilen",
  );
  const r = cleanPasted(raw);
  assert.ok(r.text.includes("\u{1F469}‍\u{1F4BB}"));
  assert.ok(!r.text.includes("Drucken"));
});

const CODE_FENCE = lines(
  "Die Funktion liefert die Seitenzahl zurück:",
  "```js",
  "12",
  "Drucken",
  "Teilen",
  "const  x   =  1 ;",
  "Zeile mit Tren-",
  "nung am Ende",
  "",
  "",
  "",
  "",
  "wieder Code",
  "```",
  "Danach geht der Text weiter.",
);

test("Geschützt: umzäunter Codeblock bleibt vollständig unberührt", () => {
  const r = cleanPasted(CODE_FENCE);
  assert.equal(r.text, CODE_FENCE);
  assert.equal(r.changes, 0);
});

test("Geschützt: Tilde-Zaun, längerer schließender Zaun, nicht geschlossener Zaun bis Ende", () => {
  const tilde = lines(
    "Text davor.",
    "~~~",
    "7",
    "Drucken",
    "Teilen",
    "~~~~~",
    "Danach.",
  );
  assert.equal(cleanPasted(tilde).text, tilde);
  const open = lines(
    "Text davor und noch etwas mehr Text in dieser Zeile, damit sie lang ist.",
    "```",
    "7",
    "Drucken",
    "Teilen",
    "",
    "",
    "",
    "",
    "Ende ohne Zaun",
  );
  const r = cleanPasted(open);
  assert.equal(r.text, open);
  assert.equal(r.changes, 0);
});

test("Geschützt: Zaun mit Backtick im Infotext öffnet keinen Block", () => {
  const raw = lines(
    "Inline ```code``` am Zeilenanfang",
    "12",
    "Fortsetzung des Textes.",
  );
  const r = cleanPasted(raw);
  assert.ok(!/\n12\n/.test(r.text));
});

test("Geschützt: außerhalb des Zauns wird weiter bereinigt", () => {
  const raw = lines(
    "Zum Inhalt springen",
    "```",
    "12",
    "```",
    "34",
    "Der Satz  hat  Leerzeichen und steht außerhalb des Blocks hier.",
  );
  const r = cleanPasted(raw);
  assert.equal(
    r.text,
    lines(
      "```",
      "12",
      "```",
      "Der Satz hat Leerzeichen und steht außerhalb des Blocks hier.",
    ),
  );
});

test("Geschützt: eingerückter Codeblock bleibt, eingerückte Seitenzahl fällt", () => {
  const raw = lines(
    "Beispielaufruf im Handbuch:",
    "",
    "    drucken  --seite  12",
    "    12",
    "",
    "    Teilen",
    "",
    "Ende des Beispiels in diesem Absatz.",
    "          7",
  );
  const r = cleanPasted(raw);
  assert.equal(
    r.text,
    lines(
      "Beispielaufruf im Handbuch:",
      "",
      "    drucken  --seite  12",
      "    12",
      "",
      "    Teilen",
      "",
      "Ende des Beispiels in diesem Absatz.",
    ),
  );
});

test("Geschützt: Überschriften, Zitate, Listen, Tabellenzeilen werden nicht verbunden", () => {
  const raw = lines(
    "#### Vierte Ebene ohne Satzende",
    "und klein weiter",
    "> Zitat ohne Satzende",
    "und klein weiter",
    "1. Listenpunkt ohne Satzende",
    "und klein weiter",
    "+ Plus-Listenpunkt ohne Satzende",
    "und klein weiter",
    "| Spalte A | Spalte B |",
    "und klein weiter",
    "| --- | --- |",
    "| Teilen | Drucken |",
  );
  const r = cleanPasted(raw);
  assert.equal(r.text, raw);
  assert.equal(r.changes, 0);
});

test("Geschützt: Tabellenzeile mit Bedienwörtern und Zeile Kopf/Fuß bleiben", () => {
  const kopf = "| Drucken | Teilen |";
  const raw = lines(kopf, "Text vor der Tabelle und etwas mehr Text.", kopf);
  assert.equal(stripWebNoise(raw).changes, 0);
});

// -- Laufzeit ----------------------------------------------------------------

const timed = (fn) => {
  const t0 = performance.now();
  const value = fn();
  return { ms: performance.now() - t0, value };
};

test("Laufzeit: „A“ + 200 000 Leerzeichen + „B“ unter 500 ms", () => {
  const { ms, value } = timed(() => cleanPasted(`A${" ".repeat(200_000)}B`));
  assert.ok(ms < 500, `${ms} ms`);
  assert.equal(value.text, "A B");
});

test("Laufzeit: lange Leerzeichenläufe um Trennzeichen und in Zeilen unter 500 ms", () => {
  for (const raw of [
    `Drucken${" ".repeat(200_000)}· Teilen`,
    `${" ".repeat(200_000)}|${" ".repeat(200_000)}`,
    `Zeile${" ".repeat(100_000)}.${" ".repeat(100_000)}x\nNoch eine Zeile`,
    `${"· ".repeat(100_000)}Drucken\nZweite Zeile des Textes`,
    `-${" ".repeat(200_000)}5${" ".repeat(10)}-\nZeile`,
  ]) {
    const { ms } = timed(() => cleanPasted(raw));
    assert.ok(ms < 500, `${ms} ms für ${raw.slice(0, 20)}`);
  }
});

test("Laufzeit: 2 MiB gemischter Text unter 500 ms", () => {
  const block = lines(
    "Landratsamt Musterkreis – Mitarbeiterinfo",
    "Ab Montag stehen alle Nebenstellen im Haus zur Verfügung, die Umstellung dauert etwa eine Stun-",
    "de und wird von der Hausverwaltung begleitet.",
    "Weitere Informationen finden Sie auf https://example.org/telefon  und im Intranet.",
    "",
    "Drucken · Teilen",
    "Seite 3 von 30",
    "```",
    "12",
    "```",
    "",
    "",
    "",
    "",
  );
  const raw = block.repeat(Math.ceil((2 * 1024 * 1024) / block.length));
  assert.ok(raw.length >= 2 * 1024 * 1024);
  const { ms, value } = timed(() => cleanPasted(raw));
  assert.ok(ms < 500, `${ms} ms`);
  assert.ok(value.changes > 1000);
});

// -- Idempotenz über alle Vorlagen ------------------------------------------

const seiteBlock = (n, kopf) =>
  [
    kopf,
    absatz(n * 3 - 2) + " Alle Beschäftigten werden ge-",
    "schult und erhalten eine Einwei-",
    "sung in das neue System.",
    absatz(n * 3 - 1),
    absatz(n * 3),
    "Weitere Hinweise finden Sie unter https://example.org/info und im Intranet des Hauses.",
    "Die Ansprechpartner stehen Ihnen während der Sprechzeiten zur Verfügung und helfen gern.",
    `Seite ${n} von 4`,
  ].join("\n");

const FIXTURES = {
  codeFence: CODE_FENCE,
  pdfMitKopfzeile: [1, 2, 3, 4]
    .map((n) => seiteBlock(n, "Landratsamt Musterkreis – Mitarbeiterinfo"))
    .join("\n"),
  pdfMitFormfeed: [1, 2, 3, 4]
    .map((n) =>
      seiteBlock(n, "Landratsamt Musterkreis – Mitarbeiterinfo").replace(
        /\nSeite \d von 4$/,
        "",
      ),
    )
    .join("\n\f"),
  intranet: lines(
    "Zum Inhalt springen",
    "Startseite",
    "",
    "Neue Telefonanlage ab Montag",
    "",
    "Ab Montag stehen alle Nebenstellen im Haus mit der neuen Anlage zur Verfügung, die Umstellung dauert etwa eine Stunde.",
    "",
    "Drucken · Teilen · E-Mail",
    "",
    "Weitere Informationen finden Sie auf https://example.org/telefon.",
    "",
    "Nach oben",
    "Zurück zur Übersicht",
    "Cookie-Einstellungen",
  ),
  newsletter: lines(
    "Newsletter Musterkreis   Mai",
    "",
    "",
    "",
    "",
    "Liebe Kolleginnen und Kollegen,",
    "",
    "in diesem Monat gibt es drei Neuerungen im Haus, über die wir Sie gern informieren möchten.",
    "",
    "Link kopieren",
    "Weiterlesen",
    "",
    "Mit freundlichen Grüßen",
    "Redaktion",
    "",
    "",
    "",
    "",
  ),
  silbentrennung: lines(
    "Die Abstimmung zwischen Bund-",
    "und Länderebene ist wichtig, weil die Zustän-",
    "digkeiten sich überschneiden.",
    "Das gilt auch für die Ge-",
    "meinden im Kreisgebiet.",
  ),
  liste: lines(
    "Aufgaben bis Freitag:",
    "1. Protokoll prüfen",
    "2. Raum buchen",
    "3. Einladung versenden",
  ),
  agenda: lines(
    "(10:30) Begrüßung",
    "(10:45) Bericht der Leitung",
    "(11:15) Aussprache und Beschlüsse",
    "[12:00] Mittagspause",
  ),
  sprecher: lines(
    "<Erzählerin> [ruhig] Es war einmal ein kleines Haus am Rand des Dorfes.",
    "7",
    "<Vater:streng> Komm sofort herein, es wird kalt.",
    "{{S. 12}}",
    "# Kapitel eins",
    "> Zitat aus dem Brief von Musterstadt",
    "- Punkt eins",
    "---",
    "[Pause 2s]",
  ),
  webrestKette: lines(
    "Drucken",
    "12",
    "Teilen",
    "Text mit einem Satz, der lang genug ist, damit er nicht als kurze Zeile gilt, hier.",
    "Facebook",
    "",
    "X",
    "E-Mail",
  ),
  seitenumbruchMitte: lines(
    "Der Bericht wurde im vergangenen Jahr erstellt und",
    "- 14 -",
    "an alle Ämter im Kreisgebiet verschickt, wie es vorgesehen war.",
  ),
  leerzeilen: lines(
    "",
    "",
    "A langer Text in der ersten Zeile des Dokuments hier.",
    "",
    "",
    "",
    "",
    "B",
    "",
    "",
  ),
  verbinderUndNbsp:
    "Die Mit­arbeiterin \u{1F469}‍\u{1F4BB} grüßt​ alle zusammen im Haus.\r\nZweite Zeile folgt.",
  gedichtKehrreim: lines(
    "Oh, du fröhliche Weihnachtszeit",
    "Der Schnee fällt leise auf das Dach,",
    "die Kinder sind schon lange wach.",
    "",
    "Oh, du fröhliche Weihnachtszeit",
    "Im Ofen knistert warm das Holz,",
    "die Katze liegt dort still und stolz.",
  ),
  kapitel: Array.from({ length: 3 }, (_, i) =>
    [
      `Kapitel ${i + 1}`,
      ...Array.from({ length: 9 }, (_, j) => absatz(i * 9 + j)),
    ].join("\n"),
  ).join("\n"),
  tabelle: lines(
    "| Name | Amt |",
    "| --- | --- |",
    "| Muster | Teilen |",
    "und klein weiter",
    "Drucken",
    "Teilen",
  ),
  persisch: "می‌خواهم یک کتاب بخرم و آن را برای دوستم بفرستم.\n12\nدرود",
  mehrfachNoise: lines(
    "Skip to content",
    "Share",
    "Print",
    "Back to top",
    "Text ohne Satzende der lang genug ist, damit er bleibt, hier.",
  ),
  eingerueckt: lines(
    "Aufruf:",
    "",
    "    a  b",
    "",
    "    c",
    "",
    "Ende des Textes in dieser Zeile.",
    "        9",
  ),
  tilde: lines(
    "Vorher.",
    "~~~",
    "Drucken",
    "Teilen",
    "~~~",
    "Nachher  mit  Leerzeichen und noch mehr Text in der Zeile.",
  ),
  wortseite: lines(
    "Das Handbuch  nennt Seite",
    "5",
    "als Fundstelle für alle Einzelheiten des Verfahrens im Haus.",
  ),
  rundschreiben: Array.from({ length: 4 }, (_, i) =>
    [
      "Rundschreiben 12/2026",
      ...Array.from({ length: 9 }, (_, j) => absatz(i * 9 + j)),
    ].join("\n"),
  ).join("\n"),
  mehrSeitenZahlen: lines(
    "Kurzer Text mit Seitenzahl am Ende des Dokuments und weiterem Inhalt hier.",
    "1",
    "2",
    "3",
    "Text nach den Seitenzahlen, der lang genug ist, um keine Kurzzeile zu sein.",
  ),
};

test("Vorlagen: mindestens zwanzig", () => {
  assert.ok(Object.keys(FIXTURES).length >= 20);
});

for (const [name, raw] of Object.entries(FIXTURES)) {
  test(`Idempotenz: ${name}`, () => {
    const first = cleanPasted(raw);
    const second = cleanPasted(first.text);
    assert.equal(second.changes, 0, JSON.stringify(second.items));
    assert.equal(second.text, first.text);
  });
}

test("Idempotenz: Bedienrest-Kette geht in einem Aufruf, ein einzelnes Wort mit Leerzeile bleibt", () => {
  const r = cleanPasted(FIXTURES.webrestKette);
  assert.equal(
    r.text,
    "Text mit einem Satz, der lang genug ist, damit er nicht als kurze Zeile gilt, hier.\nFacebook",
  );
});

// -- Wiederholungsheuristik ---------------------------------------------------

const seiten = (n, kopf, withNumbers = true) =>
  Array.from({ length: n }, (_, i) =>
    [
      kopf(i),
      ...Array.from({ length: 9 }, (_, j) => absatz(i * 9 + j)),
      ...(withNumbers ? [`Seite ${i + 1} von ${n}`] : []),
    ].join("\n"),
  ).join("\n");

test("repeated: „Kapitel 1/2/3“ bleibt (Überschriften, Ziffern unterscheiden)", () => {
  const raw = seiten(4, (i) => `Kapitel ${i + 1}`);
  assert.equal(stripRepeated(raw).changes, 0);
  for (const kopf of [
    (i) => `Teil ${i + 1}`,
    (i) => `Abschnitt ${i + 1}`,
    (i) => `${i + 1}. Einleitung`,
  ]) {
    assert.equal(stripRepeated(seiten(4, kopf)).changes, 0);
  }
});

test("repeated: Kapitelzeile mit Seitenzahl-Muster wird nicht gleichgesetzt", () => {
  const raw = seiten(4, (i) => `Kapitel ${i + 1} – Seite ${i + 1} von 4`);
  assert.equal(stripRepeated(raw).changes, 0);
});

test("repeated: echte Kopfzeile auf jeder Seite geht (Seitenzahlzeilen)", () => {
  const raw = seiten(4, () => "Landratsamt Musterkreis – Mitarbeiterinfo");
  const r = stripRepeated(raw);
  assert.equal(r.changes, 4);
  assert.ok(!r.text.includes("Landratsamt"));
});

test("repeated: echte Kopfzeile mit Seitenzahl im Kopf geht", () => {
  const raw = seiten(
    4,
    (i) => `${i > 0 ? "\f" : ""}Amt Musterkreis – Seite ${i + 1} von 4`,
    false,
  );
  assert.equal(stripRepeated(raw).changes, 4);
});

test("repeated: Formfeeds zählen als Seiten", () => {
  const raw = seiten(
    4,
    () => "Landratsamt Musterkreis – Mitarbeiterinfo",
    false,
  ).replace(/(Landratsamt Musterkreis – Mitarbeiterinfo)/g, "\f$1");
  const r = stripRepeated(raw.replace(/^\f/, ""));
  assert.equal(r.changes, 4);
});

/** Seiten gleicher Länge (11 Zeilen); die Kopfzeile steht auf Seite i an Rang `rank`, wenn `mark(i)`. */
const seitenMit = (n, mark, rank = 0) =>
  Array.from({ length: n }, (_, i) => {
    const rows = Array.from({ length: 10 }, (_, j) => absatz(i * 10 + j));
    if (mark(i)) rows.splice(rank, 0, "Intern – Vertraulich");
    else rows.push(absatz(900 + i));
    return [...rows, `Seite ${i + 1} von ${n}`].join("\n");
  }).join("\n");

test("repeated: regelmäßiger Refrain ohne Seitengrenzen bleibt", () => {
  const out = [];
  for (let i = 0; i < 6; i += 1) {
    out.push("Intern – Vertraulich");
    for (let j = 0; j < 9; j += 1) out.push(absatz(i * 9 + j));
  }
  const raw = out.join("\n");
  assert.ok(raw.length >= 1500);
  assert.equal(stripRepeated(raw).changes, 0);
  assert.ok(cleanPasted(raw).text.includes("Intern – Vertraulich"));
});

test("repeated: ≥ 4× im Rhythmus am Seitenrand geht, auch unter 60 % der Seiten", () => {
  const raw = seitenMit(7, (i) => i % 2 === 0);
  const r = stripRepeated(raw);
  assert.equal(r.changes, 4);
  assert.ok(!r.text.includes("Intern – Vertraulich"));
});

test("repeated: ≥ 4× im Rhythmus mitten auf der Seite bleibt", () => {
  const raw = seitenMit(7, (i) => i % 2 === 0, 5);
  assert.equal(stripRepeated(raw).changes, 0);
});

test("repeated: drei Wiederholungen auf derselben Seite eines fünfseitigen Texts reichen nicht", () => {
  const filler = (from, k) =>
    Array.from({ length: k }, (_, i) => absatz(from + i));
  const hinweis = "Hinweis zum Datenschutz";
  const raw = [
    ...filler(0, 9),
    "Seite 1 von 5",
    hinweis,
    ...filler(10, 9),
    hinweis,
    ...filler(20, 9),
    hinweis,
    ...filler(30, 3),
    "Seite 2 von 5",
    ...filler(40, 9),
    "Seite 3 von 5",
    ...filler(50, 9),
    "Seite 4 von 5",
    ...filler(60, 9),
    "Seite 5 von 5",
  ].join("\n");
  assert.ok(raw.length >= 1500);
  assert.equal(stripRepeated(raw).changes, 0);
  assert.ok(cleanPasted(raw).text.includes(hinweis));
});

test("repeated: der Formfeed einer entfernten Kopfzeile bleibt Seitenwechsel", () => {
  const raw = seiten(
    4,
    (i) => `${i > 0 ? "\f" : ""}Landratsamt Musterkreis – Mitarbeiterinfo`,
    false,
  );
  const r = stripRepeated(raw);
  assert.equal(r.changes, 4);
  assert.equal(r.text.split("\f").length - 1, 3);
});

test("repeated: Refrain mit unregelmäßigem Abstand bleibt", () => {
  const refrain = "Immer wieder neu";
  const gaps = [9, 20, 10, 31];
  const out = [];
  let n = 0;
  out.push(refrain);
  for (const gap of gaps) {
    for (let i = 0; i < gap; i += 1) out.push(absatz(n++));
    out.push(refrain);
  }
  const raw = out.join("\n");
  assert.ok(raw.length >= 1500);
  assert.equal(stripRepeated(raw).changes, 0);
});

test("repeated: Zeile nur auf wenigen von vielen Seiten bleibt", () => {
  const filler = (from) =>
    Array.from({ length: 9 }, (_, i) => absatz(from + i));
  const raw = [
    "Hinweis zum Datenschutz",
    ...filler(0),
    "Seite 1 von 6",
    ...filler(10),
    "Seite 2 von 6",
    "Hinweis zum Datenschutz",
    ...filler(20),
    "Seite 3 von 6",
    ...filler(30),
    "Seite 4 von 6",
    ...filler(40),
    "Seite 5 von 6",
    ...filler(50),
    "Hinweis zum Datenschutz",
    "Seite 6 von 6",
  ].join("\n");
  assert.equal(stripRepeated(raw).changes, 0);
});

// -- Web-Reste nur im Kontext -------------------------------------------------

test("webNoise: einzelnes Bedienwort als Überschrift vor Text bleibt", () => {
  for (const wort of [
    "E-Mail",
    "Zurück",
    "X",
    "Facebook",
    "Teilen",
    "Drucken",
  ]) {
    const raw = lines(
      wort,
      "Hier steht der Text zur Überschrift, der mehr als ein Wort enthält.",
    );
    const r = cleanPasted(raw);
    assert.equal(r.text, raw, wort);
  }
});

test("webNoise: einzelnes Bedienwort mit Text davor und danach bleibt", () => {
  const raw = lines(
    "Bitte melden Sie sich über das Formular an und warten Sie auf die Bestätigung.",
    "E-Mail",
    "Danach erhalten Sie eine Rückmeldung per Post oder über das Portal im Haus.",
  );
  assert.equal(cleanPasted(raw).text, raw);
});

test("webNoise: zwei aufeinanderfolgende Bedienwörter gehen", () => {
  const r = stripWebNoise(
    lines("Text bleibt.", "Facebook", "E-Mail", "Weiter im Text."),
  );
  assert.equal(r.text, lines("Text bleibt.", "Weiter im Text."));
  assert.equal(r.changes, 2);
});

test("webNoise: eindeutige Navigationsphrasen gehen auch allein", () => {
  for (const phrase of [
    "Zum Inhalt springen",
    "Skip to content",
    "Nach oben",
    "Seite drucken",
    "Artikel teilen",
    "Link kopieren",
    "Cookie-Einstellungen",
    "Alle akzeptieren",
    "Zur Navigation springen",
    "Zurück zur Übersicht",
  ]) {
    const r = stripWebNoise(lines("Text davor.", phrase, "Text danach."));
    assert.equal(r.text, lines("Text davor.", "Text danach."), phrase);
  }
});

test("webNoise: Zeile aus mindestens zwei Bedienwörtern mit Trennzeichen geht allein", () => {
  const r = stripWebNoise(
    lines("Text davor.", "E-Mail | Drucken", "Text danach."),
  );
  assert.equal(r.text, lines("Text davor.", "Text danach."));
});

// ---------------------------------------------------------------------------
// Fixrunde 2: Zeichenregel in Code, Fixpunkt, Seitengrenzen
// ---------------------------------------------------------------------------

test("control: umzäunter Codeblock behält weiches Trennzeichen, NBSP und Nullbreite", () => {
  const code = lines("```", "a\u00AD", "b", "x\u00A0y\u200Bz", "```");
  const r = stripControl(code);
  assert.equal(r.text, code);
  assert.equal(r.changes, 0);
});

test("control: eingerückter Codeblock bleibt, der Text drumherum wird bereinigt", () => {
  const block = lines("    a\u00AD", "    b\u00A0c", "    d\u200Be");
  const raw = lines(
    "Vor\u00A0dem Block\u200B steht Text.",
    "",
    block,
    "",
    "Nach\u00A0dem Block\u00AD steht Text.",
  );
  const r = stripControl(raw);
  assert.equal(
    r.text,
    lines(
      "Vor dem Block steht Text.",
      "",
      block,
      "",
      "Nach dem Block steht Text.",
    ),
  );
  assert.equal(r.changes, 4);
});

test("control: weiches Trennzeichen am Zeilenende verbindet nicht in einen Codeblock hinein", () => {
  const raw = lines("Wort\u00AD", "```", "abc", "```");
  assert.equal(stripControl(raw).text, lines("Wort", "```", "abc", "```"));
  const code = lines("```", "a\u00AD", "bc", "```");
  assert.equal(stripControl(code).text, code);
});

test("control: Codeblock übersteht die ganze Bereinigung Zeichen für Zeichen", () => {
  const code = lines("```", "a\u00AD", "b", "x\u00A0y\u200Bz", "```");
  const raw = lines(
    "Text vor dem Block mit einem Satz, der lang genug ist, um verarbeitet zu werden.",
    code,
    "Drucken",
    "Teilen",
  );
  const r = cleanPasted(raw);
  assert.ok(r.text.includes(code));
  assert.ok(!r.text.includes("Drucken"));
});

const kopfMitLeerzeichen = `Kopf${" ".repeat(601)}Zeile`;
const ueberschriften = (n) =>
  Array.from({ length: 9 }, (_, i) =>
    `## ${"Überschrift ".repeat(i + 1 + n)}`.trimEnd(),
  );

test("Idempotenz (Codex): rhythmische Kopfzeile mit 601 Leerzeichen, ohne Seitengrenzen", () => {
  const raw = [0, 1, 2, 3]
    .flatMap((n) => [kopfMitLeerzeichen, ...ueberschriften(n)])
    .join("\n");
  assert.ok(raw.length > 1500);
  const first = cleanPasted(raw);
  const second = cleanPasted(first.text);
  assert.equal(second.changes, 0, JSON.stringify(second.items));
  assert.equal(second.text, first.text);
  assert.equal(first.text.split("Kopf Zeile").length - 1, 4);
});

test("Idempotenz (Codex): dieselbe Kopfzeile mit Seitengrenzen geht schon im ersten Aufruf", () => {
  const raw = [0, 1, 2, 3]
    .flatMap((n) => [
      kopfMitLeerzeichen,
      ...ueberschriften(n),
      `Seite ${n + 1} von 4`,
    ])
    .join("\n");
  const first = cleanPasted(raw);
  assert.ok(!first.text.includes("Zeile"));
  assert.ok(rule(first, "repeated") === 4);
  const second = cleanPasted(first.text);
  assert.equal(second.changes, 0);
  assert.equal(second.text, first.text);
});

test("Bedienreste, die erst nach dem Entfernen einer Seitenzahl nebeneinander stehen, gehen", () => {
  const raw = lines(
    "Text mit einem Satz, der lang genug ist, damit er nicht als kurze Zeile gilt, hier.",
    "Zurück",
    "9",
    "Teilen",
    "E-Mail",
  );
  const r = cleanPasted(raw);
  assert.equal(
    r.text,
    "Text mit einem Satz, der lang genug ist, damit er nicht als kurze Zeile gilt, hier.",
  );
});

// ---------------------------------------------------------------------------
// Rohfassung (pasteLog.ts, Konzept §4.1)
// ---------------------------------------------------------------------------

const entryFor = async (raw) => {
  const r = cleanPasted(raw);
  return makePasteEntry(raw, r.text, r.items, new Date("2026-11-12T10:00:00Z"));
};

test("pasteLog: sha256 stimmt mit node:crypto überein, Eintrag trägt Roh- und bereinigten Text", async () => {
  const { createHash } = await import("node:crypto");
  const raw =
    "Drucken\r\nTeilen\r\nText mit Inhalt, der bleibt, in einer ganz normalen Zeile.";
  const e = await entryFor(raw);
  const normalized = raw.replace(/\r\n/g, "\n");
  assert.equal(e.sha256, createHash("sha256").update(normalized).digest("hex"));
  assert.equal(
    await sha256Hex("abc"),
    createHash("sha256").update("abc").digest("hex"),
  );
  assert.equal(e.roh, normalized);
  assert.equal(
    e.bereinigt,
    "Text mit Inhalt, der bleibt, in einer ganz normalen Zeile.",
  );
  assert.equal(e.zeit, "2026-11-12T10:00:00.000Z");
  assert.ok(pasteEntryChanges(e) >= 2);
  assert.deepEqual(
    e.regeln.map((r) => r.rule),
    ["webNoise"],
  );
});

test("pasteLog: höchstens 20 Einträge, die ältesten fallen zuerst", async () => {
  let log = [];
  for (let i = 0; i < 25; i += 1) {
    log = appendPasteEntry(log, { zeit: String(i), sha256: "x", regeln: [] });
  }
  assert.equal(log.length, PASTE_LOG_MAX_ENTRIES);
  assert.equal(log[0].zeit, "5");
  assert.equal(log[19].zeit, "24");
});

/** Gültiger Zeitstempel für den n-ten Eintrag. */
const iso = (n) => new Date(Date.UTC(2026, 10, 12, 8, 0, n)).toISOString();
const SHA = "ab".repeat(32);
const eintrag = (zeit, roh, bereinigt = roh) => ({
  zeit,
  sha256: SHA,
  roh,
  bereinigt,
  regeln: [{ rule: "spaces", count: 1 }],
});

test("pasteLog: Gesamtumfang in Bytes begrenzt, der neueste Eintrag bleibt immer", () => {
  const big = "x".repeat(900_000);
  let log = [];
  for (let i = 0; i < 12; i += 1) {
    log = appendPasteEntry(log, eintrag(String(i), big));
  }
  const total = log.reduce((sum, e) => sum + entryBytes(e), 0);
  assert.ok(total <= PASTE_LOG_MAX_BYTES, `${total}`);
  assert.ok(log.length < 12);
  assert.equal(log[log.length - 1].zeit, "11");
});

test("pasteLog: Einzelgrenze zählt Bytes, nicht Zeichen („ä“ hat zwei)", () => {
  const ae = "ä".repeat(600_000);
  assert.ok(ae.length * 2 < PASTE_ENTRY_MAX_BYTES);
  const [viele] = appendPasteEntry([], eintrag("a", ae));
  assert.equal(viele.zuGross, true);
  assert.equal(viele.roh, undefined);
  assert.equal(viele.bereinigt, undefined);
  assert.equal(viele.zeichen, 600_000);
  assert.equal(viele.bytes, 1_200_000);
  const ascii = "a".repeat(600_000);
  const [klein] = appendPasteEntry([], eintrag("b", ascii));
  assert.equal(klein.zuGross, undefined);
  assert.equal(klein.roh, ascii);
});

test("pasteLog: zu großer Text wird nur als Prüfsumme und Größe vermerkt", async () => {
  const roh = "ä".repeat(PASTE_ENTRY_MAX_BYTES / 2 + 1);
  const e = await makeOversizedEntry(roh);
  assert.equal(e.zuGross, true);
  assert.equal(e.roh, undefined);
  assert.equal(e.bereinigt, undefined);
  assert.equal(e.zeichen, roh.length);
  assert.equal(e.bytes, utf8Bytes(roh));
  assert.equal(e.sha256.length, 64);
});

test("pasteLog: beim Laden verlieren übergroße Einträge Roh- und bereinigte Fassung", () => {
  const gross = {
    zeit: iso(1),
    sha256: SHA,
    roh: "ä".repeat(1_100_000),
    bereinigt: "x",
    regeln: [{ rule: "spaces", count: 3 }],
  };
  const klein = eintrag(iso(2), "kurz");
  const [a, b] = parsePasteLog([gross, klein]);
  assert.equal(a.zuGross, true);
  assert.equal(a.roh, undefined);
  assert.equal(a.sha256, SHA);
  assert.equal(a.zeichen, 1_100_000);
  assert.equal(a.bytes, 2_200_000);
  assert.deepEqual(a.regeln, []);
  assert.equal(b.roh, "kurz");
});

test("pasteLog: beim Laden gilt danach die Gesamtgrenze, die ältesten fallen", () => {
  const roh = "x".repeat(900_000);
  const geladen = parsePasteLog(
    Array.from({ length: 20 }, (_, i) => eintrag(iso(i), roh)),
  );
  const total = geladen.reduce((sum, e) => sum + entryBytes(e), 0);
  assert.ok(total <= PASTE_LOG_MAX_BYTES);
  assert.ok(geladen.length < 20);
  assert.equal(geladen[geladen.length - 1].zeit, iso(19));
});

test("pasteLog: Prüfsumme passt nicht zur Rohfassung: beschädigt, Wiederherstellen gesperrt", async () => {
  const gut = await entryFor(
    "Drucken\nTeilen\nText mit Inhalt, der bleibt, in einer ganz normalen Zeile.",
  );
  const kaputt = {
    ...gut,
    roh: `${gut.roh}!`,
    zeit: "2026-11-12T11:00:00.000Z",
  };
  const [a, b] = await verifyPasteLog([gut, kaputt]);
  assert.equal(a.beschaedigt, undefined);
  assert.equal(b.beschaedigt, true);
  assert.deepEqual(restoreRaw(`${b.bereinigt}`, b), {
    ok: false,
    reason: "damaged",
  });
  assert.equal(restoreRaw(`${a.bereinigt}`, a).ok, true);
  // Das Merkmal wird nicht gespeichert gelesen: Neuladen prüft neu.
  assert.equal(parsePasteLog([b])[0].beschaedigt, undefined);
});

test("pasteLog: Eintrag löschen über seinen Schlüssel", () => {
  const log = [eintrag("a", "1"), eintrag("b", "2"), eintrag("c", "3")];
  const next = removePasteEntry(log, pasteEntryKey(log[1]));
  assert.deepEqual(
    next.map((e) => e.zeit),
    ["a", "c"],
  );
  assert.equal(removePasteEntry(log, "gibt|es nicht").length, 3);
});

test("pasteLog: Speichern und Neuladen (JSON) übersteht alte Zustände und Müll", async () => {
  const e1 = await entryFor(
    "Drucken\nTeilen\nText mit Inhalt, der bleibt, in einer ganz normalen Zeile.",
  );
  const e2 = await makeOversizedEntry("z".repeat(10));
  const state = JSON.stringify({
    text: "Hallo",
    tab: "original",
    einfuegungen: [
      e1,
      e2,
      7,
      null,
      { zeit: 1 },
      {
        zeit: iso(5),
        sha256: SHA,
        regeln: [
          { rule: "nope", count: 1 },
          { rule: "spaces", count: 2 },
        ],
      },
    ],
  });
  const loaded = parsePasteLog(JSON.parse(state).einfuegungen);
  assert.equal(loaded.length, 3);
  assert.deepEqual(loaded[0], e1);
  assert.equal(loaded[1].zuGross, true);
  assert.deepEqual(loaded[2].regeln, [{ rule: "spaces", count: 2 }]);
  // Alter Zustand ohne das Feld.
  assert.deepEqual(
    parsePasteLog(JSON.parse('{"text":"Hallo"}').einfuegungen),
    [],
  );
  assert.deepEqual(parsePasteLog("kein Array"), []);
});

test("pasteLog: Wiederherstellen nur bei genau einem unveränderten Vorkommen", async () => {
  const raw =
    "Drucken\nTeilen\nText mit Inhalt, der bleibt, in einer ganz normalen Zeile.";
  const e = await entryFor(raw);
  const ok = restoreRaw(`Vorher. ${e.bereinigt} Nachher.`, e);
  assert.deepEqual(ok, { ok: true, text: `Vorher. ${e.roh} Nachher.` });
  assert.deepEqual(restoreRaw("Ganz anderer Text", e), {
    ok: false,
    reason: "missing",
  });
  assert.deepEqual(restoreRaw(`${e.bereinigt} und ${e.bereinigt}`, e), {
    ok: false,
    reason: "ambiguous",
  });
  assert.deepEqual(
    restoreRaw(`${e.bereinigt}X`.replace("Zeile.", "Zeile"), e),
    { ok: false, reason: "missing" },
  );
  assert.deepEqual(restoreRaw("x", await makeOversizedEntry("abc")), {
    ok: false,
    reason: "noRaw",
  });
  const leer = await entryFor("Drucken\nTeilen");
  assert.equal(leer.bereinigt, "");
  assert.deepEqual(restoreRaw("irgendwas", leer), {
    ok: false,
    reason: "empty",
  });
});

test("pasteLog: überlappendes Vorkommen gilt als mehrfach", () => {
  const e = { zeit: "t", sha256: "s", roh: "R", bereinigt: "aa", regeln: [] };
  assert.deepEqual(restoreRaw("aaa", e), { ok: false, reason: "ambiguous" });
});

// ---------------------------------------------------------------------------
// Fixrunde 3: joinFragments linear, Formfeed in Code, Fixpunkt, Protokoll-Parser
// ---------------------------------------------------------------------------

/** Die bisherige Fassung von joinFragments (quadratisch), als Referenz der Semantik. */
const OLD = (() => {
  const TIMESTAMP =
    /^[ \t]*[[(]\d{1,2}:\d{2}(?::\d{2})?(?:[.,]\d{1,3})?[\])][ \t]*(?::[ \t]*)?/;
  const LONE_COLON = /^[ \t]*:[ \t]+(?=\S)/;
  const BLOCK_START =
    /^\s*(?:#{1,3}\s|>\s?|[-*]\s|-{3,}\s*$|\*{3,}\s*$|_{3,}\s*$|⁂)/;
  const SPEAKER_START = /^\s*<[^<>\n]{1,40}>/;
  const SENTENCE_END = /[.!?…:;]["'»«“”‘’)\]]*\s*$/;
  const INITIALS = /^(?:\p{Lu}\.\s*){1,3}$/u;
  const HYPHEN_END = /\p{Ll}-$/u;
  const STARTS_LOWER = /^\p{Ll}/u;
  const SUSPENDED = /^(?:und|oder|bzw\.|sowie|als auch|noch)\s/;
  const isPlain = (line) =>
    line.trim().length > 0 &&
    !BLOCK_START.test(line) &&
    !SPEAKER_START.test(line) &&
    !/^\s*\{\{S\./.test(line);
  return function joinOld(text) {
    const out = [];
    let changes = 0;
    for (const raw of text.split("\n")) {
      const prev = out.length > 0 ? out[out.length - 1] : null;
      if (prev !== null && isPlain(prev) && isPlain(raw)) {
        const body = raw.replace(TIMESTAMP, "").replace(LONE_COLON, "").trim();
        const prevBody = prev
          .replace(TIMESTAMP, "")
          .replace(LONE_COLON, "")
          .trim();
        if (body.length > 0) {
          if (HYPHEN_END.test(prev.trimEnd()) && STARTS_LOWER.test(body)) {
            out[out.length - 1] = SUSPENDED.test(body)
              ? `${prev.trimEnd()} ${body}`
              : prev.trimEnd().slice(0, -1) + body;
            changes += 1;
            continue;
          }
          if (
            INITIALS.test(prevBody) ||
            (!SENTENCE_END.test(prev) && STARTS_LOWER.test(body))
          ) {
            out[out.length - 1] = `${prev.trimEnd()} ${body}`;
            changes += 1;
            continue;
          }
        }
      }
      out.push(raw);
    }
    return { text: out.join("\n"), changes };
  };
})();

test("joinFragments: 200 000 verbundene Kleinbuchstabenzeilen in linearer Zeit", () => {
  const raw = ["Der Anfang eines Satzes", ...Array(200_000).fill("wort")].join(
    "\n",
  );
  const start = performance.now();
  const r = joinFragments(raw);
  const ms = performance.now() - start;
  assert.equal(r.changes, 200_000);
  assert.equal(r.text.length, "Der Anfang eines Satzes".length + 5 * 200_000);
  assert.ok(r.text.endsWith(" wort wort"));
  assert.ok(ms < 1500, `${Math.round(ms)} ms`);
});

test("joinFragments: ebenso bei langen Satzenden-Prüfungen (Zeilen mit Anführungszeichen und Leerzeichen)", () => {
  const raw = [
    "Beginn",
    ...Array.from({ length: 60_000 }, () => "teil »x«   "),
  ].join("\n");
  const start = performance.now();
  const r = joinFragments(raw);
  const ms = performance.now() - start;
  assert.equal(r.changes, 60_000);
  assert.ok(ms < 1500, `${Math.round(ms)} ms`);
});

test("cleanPasted: 200 000 abgerissene Zeilen unter 3 s, Ergebnis wie erwartet", () => {
  const raw = [
    "Ein langer Satz beginnt hier",
    ...Array(200_000).fill("wort"),
  ].join("\n");
  const start = performance.now();
  const r = cleanPasted(raw);
  const ms = performance.now() - start;
  assert.equal(r.text.split("\n").length, 1);
  assert.ok(ms < 3000, `${Math.round(ms)} ms`);
});

test("joinFragments: gleiche Ergebnisse wie die bisherige Fassung (Zufallsvorlagen, kleine Fälle)", () => {
  const pool = [
    "J.",
    "L.",
    "M.",
    "Collins hat",
    "wort",
    "Ge-",
    "danken",
    "und Sozialpolitik",
    "Wirtschafts-",
    "[0:01] ",
    "[0:02] sagte er",
    "(00:12) dann",
    ": Rest",
    "# Titel",
    "> Zitat",
    "<Anna> hallo",
    "",
    "   ",
    "Ende.",
    "ende!)",
    "etwas:",
    "- Liste",
    "-",
    "x -",
    "ä-",
    "  eingerückt klein",
    "Zeile mit Leerzeichen   ",
    "{{S. 3}}",
    "»zitat«",
    "Satz.«  ",
    "noch eine",
    "Dr.",
    "A. B. C. D.",
    "abc;",
    "---",
    "Wort-",
    "oder so",
    "-- ",
    "\t",
    "ÖPNV",
    "klein",
  ];
  let seed = 12345;
  const rnd = (n) => {
    seed = (seed * 1103515245 + 12345) & 0x7fffffff;
    return seed % n;
  };
  for (let k = 0; k < 4000; k += 1) {
    const count = 1 + rnd(12);
    const text = Array.from(
      { length: count },
      () => pool[rnd(pool.length)],
    ).join("\n");
    const got = joinFragments(text);
    const want = OLD(text);
    assert.deepEqual(got, want, JSON.stringify(text));
  }
});

test("control: Formfeed in umzäuntem Code bleibt byte-gleich", () => {
  const code = "```text\nA\fB\n```";
  const r = stripControl(code);
  assert.equal(r.text, code);
  assert.equal(r.changes, 0);
});

test("control: Formfeed in eingerücktem Code bleibt, außerhalb wird er zum Umbruch", () => {
  const raw = lines(
    "Vor dem Block.\fNach dem Umbruch.",
    "",
    "    a\fb",
    "    c",
  );
  const r = stripControl(raw);
  assert.equal(
    r.text,
    lines("Vor dem Block.", "Nach dem Umbruch.", "", "    a\fb", "    c"),
  );
  assert.equal(r.changes, 1);
});

test("repeated: Formfeeds in Code sind keine Seitengrenzen", () => {
  const code = lines("```", "\f", "\f", "\f", "```");
  const kopf = "Rundschreiben Musterkreis – Seite";
  const raw = lines(
    code,
    ...Array.from({ length: 4 }, (_, i) =>
      lines(kopf, ...Array.from({ length: 9 }, (_, j) => absatz(i * 9 + j))),
    ),
  );
  assert.ok(raw.length > 1500);
  // Ohne echte Seitengrenzen bleibt jede Zeile stehen.
  assert.equal(stripRepeated(raw).changes, 0);
  const all = cleanPasted(raw);
  assert.ok(all.text.startsWith(code), "Codeblock bleibt unverändert");
});

test("Fixpunkt: nicht zur Ruhe gekommen heißt unverändert, sonst gilt cleanPasted(cleanPasted(x)) == cleanPasted(x)", () => {
  const raw = lines(
    "Text mit einem Satz, der lang genug ist, damit er nicht als kurze Zeile gilt, hier.",
    "Zurück",
    "9",
    "Teilen",
    "E-Mail",
  );
  assert.equal(cleanPasted(raw, { maxPasses: 1 }).changes, 0);
  assert.equal(cleanPasted(raw, { maxPasses: 1 }).text, raw);
  const full = cleanPasted(raw);
  assert.ok(full.changes > 0);
  const again = cleanPasted(full.text);
  assert.equal(again.text, full.text);
  assert.equal(again.changes, 0);
});

test("Fixpunkt: Zufallsvorlagen sind nach einem Aufruf stabil", () => {
  const pool = [
    "Drucken",
    "Teilen",
    "Zurück",
    "9",
    "E-Mail",
    "Seite 3 von 9",
    "12",
    "Ein ganz normaler Satz, der lang genug ist, um zu bleiben.",
    "",
    "",
    "wort",
    "Ge-",
    "danken",
    "J.",
    "L.",
    "```",
    "code­",
    "    eingerückt ",
    "\f",
    "# Titel",
    "<Anna> hallo",
    "[0:05] sagte",
    "Skip to content",
    "Nach oben",
    "Zeile  mit   Leerzeichen",
    "Kopf Zeile",
    "Fußzeile – Seite 1",
    "​",
    "- Liste",
    "klein weiter",
    "A.",
    "x",
    "https://example.org/x",
    "|a|b|",
  ];
  let seed = 4242;
  const rnd = (n) => {
    seed = (seed * 1103515245 + 12345) & 0x7fffffff;
    return seed % n;
  };
  for (let k = 0; k < 600; k += 1) {
    const text = Array.from(
      { length: 2 + rnd(40) },
      () => pool[rnd(pool.length)],
    ).join("\n");
    const first = cleanPasted(text);
    const second = cleanPasted(first.text);
    assert.equal(second.text, first.text, JSON.stringify(text));
    assert.equal(second.changes, 0, JSON.stringify(text));
  }
});

test("pasteLog: Zeitstempel nur als ISO-Zeit bis 40 Zeichen, sha256 nur 64 Hex", () => {
  assert.ok(isIsoTime("2026-11-12T10:00:00.000Z"));
  assert.ok(isIsoTime("2026-11-12T10:00:00+01:00"));
  assert.ok(!isIsoTime("morgen"));
  assert.ok(!isIsoTime("2026-13-45T99:00:00Z"));
  assert.ok(!isIsoTime(`2026-11-12T10:00:00.${"0".repeat(60)}Z`));
  assert.ok(isSha256("ab".repeat(32)));
  assert.ok(isSha256(""));
  assert.ok(!isSha256("abc"));
  assert.ok(!isSha256("zz".repeat(32)));
  assert.ok(!isSha256("ab".repeat(33)));
});

test("pasteLog: Laden verwirft Einträge mit 17-MiB-Zeitstempel oder ungültiger Prüfsumme", () => {
  const gut = eintrag(iso(1), "gut");
  const geladen = parsePasteLog([
    { ...gut, zeit: "z".repeat(17 * 1024 * 1024) },
    { ...gut, sha256: "nicht-hex" },
    { ...gut, sha256: "a".repeat(10_000_000) },
    { ...gut, zeit: "gestern" },
    gut,
  ]);
  assert.equal(geladen.length, 1);
  assert.equal(geladen[0].roh, "gut");
  assert.ok(entryBytes(geladen[0]) < 1000);
});

test("pasteLog: regeln begrenzt (je Regel einmal, ganze Zahlen, nicht negativ)", () => {
  const viele = Array.from({ length: 5000 }, () => ({
    rule: "spaces",
    count: 1,
  }));
  const [e] = parsePasteLog([
    {
      ...eintrag(iso(1), "x"),
      regeln: [
        { rule: "spaces", count: 1 },
        { rule: "control", count: 2.5 },
        { rule: "webNoise", count: -1 },
        { rule: "fragments", count: 3 },
        ...viele,
      ],
    },
  ]);
  assert.deepEqual(e.regeln, [
    { rule: "spaces", count: 1 },
    { rule: "fragments", count: 3 },
  ]);
  const [g] = parsePasteLog([{ ...eintrag(iso(2), "x"), regeln: viele }]);
  assert.ok(g.regeln.length <= 1);
});

test("pasteLog: Gesamtgrenze gilt nach dem Kürzen; Metadaten zählen mit, nichts bleibt darüber", () => {
  const gross = (n) => ({
    ...eintrag(iso(n), "ä".repeat(1_100_000)),
    bereinigt: "x",
  });
  const mittel = (n) => eintrag(iso(n), "x".repeat(1_900_000));
  const log = parsePasteLog([
    ...Array.from({ length: 6 }, (_, i) => gross(i)),
    ...Array.from({ length: 14 }, (_, i) => mittel(10 + i)),
  ]);
  const total = log.reduce((sum, e) => sum + entryBytes(e), 0);
  assert.ok(total <= PASTE_LOG_MAX_BYTES, `${total}`);
  assert.ok(log.every((e) => entryBytes(e) <= PASTE_ENTRY_MAX_BYTES));
  assert.ok(log.length >= 1);
  assert.equal(log[log.length - 1].zeit, iso(23));
});

test("pasteLog: ein einzelner Eintrag, der auch gekürzt über der Grenze liegt, wird verworfen", () => {
  const unmoeglich = {
    ...eintrag(iso(1), "x"),
    // Nur von Hand möglich: die Metadaten allein sprengen die Einzelgrenze.
    zeit: "z".repeat(PASTE_ENTRY_MAX_BYTES),
  };
  const log = appendPasteEntry([eintrag(iso(0), "alt")], unmoeglich);
  assert.equal(log.length, 1);
  assert.equal(log[0].roh, "alt");
});
