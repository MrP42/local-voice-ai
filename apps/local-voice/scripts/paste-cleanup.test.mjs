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

test("repeated: Ziffern werden gleichgesetzt (Kopfzeile mit Datum)", () => {
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
  const r = stripRepeated(raw);
  assert.equal(r.changes, 3);
  assert.ok(!r.text.includes("Rundschreiben"));
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

test("webNoise: Zeile mit weiterem Text bleibt", () => {
  const r = stripWebNoise("Teilen Sie uns Ihre Meinung mit.\nTeilen");
  assert.equal(r.text, "Teilen Sie uns Ihre Meinung mit.");
  assert.equal(r.changes, 1);
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
