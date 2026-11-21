// node --experimental-strip-types --test tests/unit/aufbereiten.test.ts
import assert from "node:assert/strict";
import test from "node:test";
import {
  anzeigeText,
  ergebnisText,
  fenster,
  gruppenUebersicht,
  kompensieren,
  sicherSchneiden,
  strukturVergleich,
  uebernahmeVerwerfen,
  vergleichSegmente,
} from "../../src/lib/reader/aufbereiten.ts";
import type { Vorschlag } from "../../src/lib/reader/aufbereiten.ts";

const aenderung = (werte: Partial<Vorschlag> = {}): Vorschlag => ({
  art: { regel: "leerzeichen" },
  alt: "  ",
  neu: " ",
  gruppe: "vorkommen-1",
  status: "offen",
  ...werte,
});

test("Kompensation gelingt beim zweiten Versuch", async () => {
  // Gegenprobe: Ohne Wiederholung bleibt die verworfene Fassung aktiv.
  let aufrufe = 0;
  const pausen: number[] = [];
  const ergebnis = await kompensieren(
    async () => {
      aufrufe += 1;
      return aufrufe === 1 ? { ok: false, code: "storage" } : { ok: true };
    },
    3,
    async (dauer) => {
      pausen.push(dauer);
    },
  );
  assert.deepEqual(ergebnis, { ok: true, versuche: 2 });
  assert.equal(aufrufe, 2);
  assert.deepEqual(pausen, [300]);
});

test("Kompensation endet nach drei Speicherfehlern", async () => {
  // Gegenprobe: Unbegrenzte Wiederholungen liefern kein endgültiges Fehlerergebnis.
  let aufrufe = 0;
  const pausen: number[] = [];
  const ergebnis = await kompensieren(
    async () => {
      aufrufe += 1;
      assert.ok(aufrufe <= 3);
      return { ok: false, code: "storage" };
    },
    3,
    async (dauer) => {
      pausen.push(dauer);
    },
  );
  assert.deepEqual(ergebnis, { ok: false, code: "storage", versuche: 3 });
  assert.equal(aufrufe, 3);
  assert.deepEqual(pausen, [300, 900]);
});

test("Kompensation wiederholt andere Fehler nicht", async () => {
  // Gegenprobe: Pauschale Wiederholung würde auch fachliche Fehler dreimal aufrufen.
  let aufrufe = 0;
  const ergebnis = await kompensieren(
    async () => {
      aufrufe += 1;
      return { ok: false, code: "wrong_status" };
    },
    3,
    async () => assert.fail("Keine Pause bei fachlichen Fehlern"),
  );
  assert.deepEqual(ergebnis, { ok: false, code: "wrong_status", versuche: 1 });
  assert.equal(aufrufe, 1);
});

test("Kontextanzeige schneidet bei 250 nicht durch Emoji", () => {
  // Gegenprobe: slice(0, 250) und slice(-251) lassen jeweils ein halbes Emoji stehen.
  const text = `${"a".repeat(249)}😀${"b".repeat(250)}`;
  assert.equal(sicherSchneiden(text, 0, 250), "a".repeat(249));
  assert.equal(sicherSchneiden(text, 250, text.length), `😀${"b".repeat(250)}`);
  assert.equal(sicherSchneiden(text, -251, text.length), `😀${"b".repeat(250)}`);
  const spiegel = `${"b".repeat(250)}😀${"a".repeat(249)}`;
  assert.equal(sicherSchneiden(spiegel, -250, spiegel.length), `😀${"a".repeat(249)}`);
  assert.equal(ergebnisText(text, [], [], "angenommen"), text);
});

test("Seitengrenze bei 8000 erhält Emoji und vollständigen Ergebnistext", () => {
  // Gegenprobe: Zwei rohe slice-Aufrufe verteilen die Surrogate auf getrennte Seiten.
  const text = `${"a".repeat(7999)}😀`;
  const ersteSeite = sicherSchneiden(text, 0, 8000);
  const zweiteSeite = sicherSchneiden(text, 8000, 16000);
  assert.equal(ersteSeite, "a".repeat(7999));
  assert.equal(zweiteSeite, "😀");
  assert.equal(ersteSeite + zweiteSeite, text);
  assert.equal(ergebnisText(text, [], [], "angenommen"), text);
});

test("Große Änderungen werden nur für die Anzeige gekürzt", () => {
  // Gegenprobe: Ohne Anzeigegrenze gelangen 5000 Zeichen in Beispiel und Vergleich.
  const alt = "a".repeat(5000);
  const neu = "b".repeat(5000);
  const a = aenderung({ alt, neu, status: "angenommen" });
  const [regel] = gruppenUebersicht([a], alt, [[0, alt.length]]);
  assert.equal(regel.beispiel.length, 400);
  assert.ok(regel.beispiel.endsWith("…"));
  assert.equal(regel.beispielGekuerzt, true);
  assert.equal(regel.beispielLaenge, 10003);
  const [segment] = vergleichSegmente(alt, [a], [[0, alt.length]]);
  for (const text of [segment.alt, segment.neu]) {
    assert.ok(text);
    const anzeige = anzeigeText(text);
    assert.equal(anzeige.text.length, 400);
    assert.equal(anzeige.gekuerzt, true);
    assert.equal(anzeige.laenge, 5000);
  }
  assert.equal(segment.alt, alt);
  assert.equal(segment.neu, neu);
  assert.equal(ergebnisText(alt, [a], [[0, alt.length]], "angenommen"), neu);
  assert.equal(a.alt, alt);
  assert.equal(a.neu, neu);
});

test("Verworfene Textübernahme nimmt die Aktivierung zurück", async () => {
  // Gegenprobe: Ohne kompensierenden Aufruf bleibt B trotz verworfener Übernahme aktiv.
  let aktiv = "A";
  let aufloesen!: (wert: {
    text: string;
    aufbereitung: { status: string };
  }) => void;
  const anfrage = new Promise<{ text: string; aufbereitung: { status: string } }>(
    (resolve) => {
      aufloesen = resolve;
    },
  );
  const vorher = "A";
  let editor = vorher;
  editor = "C";
  aktiv = "B";
  aufloesen({ text: "B", aufbereitung: { status: "aktiv" } });
  let ruecknahmen = 0;
  const verworfen = await uebernahmeVerwerfen(
    await anfrage,
    editor !== vorher,
    async () => {
      ruecknahmen += 1;
      aktiv = "A";
    },
  );
  assert.equal(verworfen, true);
  assert.equal(ruecknahmen, 1);
  assert.equal(aktiv, "A");
  assert.equal(editor, "C");
});

test("Nur verworfene aktive Übernahmen werden zurückgenommen", async () => {
  for (const veraltet of [false, true]) {
    for (const status of ["aktiv", "veraltet"]) {
      for (const text of [null, "B"]) {
        let ruecknahmen = 0;
        const verworfen = await uebernahmeVerwerfen(
          { text, aufbereitung: { status } },
          veraltet,
          async () => {
            ruecknahmen += 1;
          },
        );
        assert.equal(
          ruecknahmen,
          veraltet && status === "aktiv" && text !== null ? 1 : 0,
        );
        assert.equal(verworfen, veraltet || status === "veraltet" || text === null);
      }
    }
  }
});

test("Anzeigegrenze erhält Emoji vollständig und zählt Zeichen", () => {
  // Gegenprobe: slice(0, 399) würde das Emoji an dieser Grenze halbieren.
  const text = `${"a".repeat(398)}😀zz`;
  const anzeige = anzeigeText(text);
  assert.equal(anzeige.text, `${"a".repeat(398)}😀…`);
  assert.equal([...anzeige.text].length, 400);
  assert.equal(anzeige.laenge, 401);
  const exakt = `${"a".repeat(399)}😀`;
  assert.deepEqual(anzeigeText(exakt), {
    text: exakt,
    gekuerzt: false,
    laenge: 400,
  });
  const [regel] = gruppenUebersicht(
    [aenderung({ alt: text, neu: "" })],
    text,
    [[0, text.length]],
  );
  assert.equal(regel.beispiel, `${"a".repeat(398)}😀…`);
});

test("Regeln zählen Änderungen und lokale Gruppen getrennt", () => {
  const a = aenderung();
  const b = aenderung({ gruppe: "vorkommen-2" });
  const [regel] = gruppenUebersicht([a, a, b], "      ", [
    [0, 2], [2, 4], [4, 6],
  ]);
  assert.equal(regel.regel, "leerzeichen");
  assert.equal(regel.anzahl, 3);
  assert.equal(regel.gruppen, 2);
  assert.equal(regel.status, "offen");
});

test("Unterschiedliche Entscheidungen ergeben gemischt", () => {
  const [regel] = gruppenUebersicht([
    aenderung(), aenderung({ status: "abgelehnt" }),
  ], "    ", [[0, 2], [2, 4]]);
  assert.equal(regel.status, "gemischt");
});

test("Sprachvorschläge sind keine Regel und null ist keine Gruppe", () => {
  const regeln = gruppenUebersicht([
    aenderung({ art: "sprache" }),
    aenderung({ gruppe: null }),
  ], "    ", [[0, 2], [2, 4]]);
  assert.equal(regeln.length, 1);
  assert.equal(regeln[0].anzahl, 1);
  assert.equal(regeln[0].gruppen, 0);
});

test("Beispiel hat höchstens 24 Zeichen Kontext und keine Zeilenumbrüche", () => {
  const text = `${"x".repeat(30)}\n  \n${"y".repeat(30)}`;
  const [regel] = gruppenUebersicht([aenderung()], text, [[31, 33]]);
  assert.equal(
    regel.beispiel,
    `${"x".repeat(23)}    →   ${"y".repeat(23)}`,
  );
  assert.ok(!regel.beispiel.includes("\n"));
});

test("UTF-16-Bereiche erhalten Umlaute und Emoji", () => {
  const text = "Ä😀  ö";
  const segmente = vergleichSegmente(text, [aenderung()], [[3, 5]]);
  assert.deepEqual(segmente.map((s) => s.text), ["Ä😀", "  ", "ö"]);
  assert.equal(segmente[1].neu, " ");
  assert.equal(segmente[1].gruppe, "vorkommen-1");
  assert.equal(ergebnisText(text, [aenderung()], [[3, 5]], "alle"), "Ä😀 ö");
});

test("Teilannahme wendet ausschließlich angenommene Vorschläge an", () => {
  const a = [
    aenderung({ status: "angenommen" }),
    aenderung({ status: "abgelehnt" }),
    aenderung(),
  ];
  assert.equal(ergebnisText("a  b  c  d", a, [
    [1, 3], [4, 6], [7, 9],
  ], "angenommen"), "a b  c  d");
  assert.equal(ergebnisText("a  b  c  d", a, [
    [1, 3], [4, 6], [7, 9],
  ], "alle"), "a b c d");
});

test("Fehlende Bereiche und falscher Alttext bleiben unverändert", () => {
  assert.equal(ergebnisText("abc", [aenderung()], [], "alle"), "abc");
  assert.equal(ergebnisText("abc", [aenderung()], [[0, 2]], "alle"), "abc");
});

test("Negative, gebrochene, zu große und umgekehrte Bereiche werden verworfen", () => {
  for (const bereich of [
    [-1, 1], [0, 4], [2, 1], [0.5, 2], [0, NaN],
    [0, Infinity], [2 ** 31 + 1, 2 ** 31 + 2],
  ] as [number, number][]) {
    // Passender Alttext: Die Bereichsprüfung selbst muss den Eintrag sperren.
    const alt = "abc".slice(bereich[0], bereich[1]);
    assert.equal(
      ergebnisText("abc", [aenderung({ alt })], [bereich], "alle"),
      "abc",
    );
  }
});

test("Überlappende und unsortierte Bereiche werden nicht angewandt", () => {
  const a = aenderung({ alt: "aa", neu: "X" });
  assert.equal(ergebnisText("aaaaaa", [a, a, a], [
    [2, 4], [0, 2], [3, 5],
  ], "alle"), "aaXaa");
});

test("Ein ungültiger Bereich verschluckt keine spätere gültige Änderung", () => {
  assert.equal(ergebnisText("a  b", [
    aenderung({ alt: "falsch" }), aenderung(),
  ], [[0, 4], [1, 3]], "alle"), "a b");
});

test("Einfügen und Löschen funktionieren auch an Textgrenzen", () => {
  assert.equal(ergebnisText("ab", [
    aenderung({ alt: "", neu: "X" }),
    aenderung({ alt: "b", neu: "" }),
  ], [[0, 0], [1, 2]], "alle"), "Xa");
});

test("Ohne Vorschläge bleibt der ganze Basistext als Kontext erhalten", () => {
  assert.deepEqual(vergleichSegmente("unverändert", [], []), [
    { art: "gleich", text: "unverändert" },
  ]);
  assert.deepEqual(vergleichSegmente("", [], []), []);
});

test("Strukturvergleich zählt Blöcke, Überschriften und Seitenmarken", () => {
  assert.deepEqual(strukturVergleich(
    "# Titel\r\n\r\nText\r\nFortsetzung\r\n \r\n{{S. 12}}",
    "## Titel\nText {{S. 12}} {{S. 13}}",
  ), {
    absaetze: { vorher: 3, nachher: 1 },
    ueberschriften: { vorher: 1, nachher: 1 },
    seitenmarken: { vorher: 1, nachher: 2 },
  });
  assert.deepEqual(strukturVergleich(" \n\n", "").absaetze, {
    vorher: 0, nachher: 0,
  });
});

test("Fenster zählt Änderungen statt Kontextsegmente", () => {
  const s = vergleichSegmente("a  b  c  d", [
    aenderung(), aenderung(), aenderung(),
  ], [[1, 3], [4, 6], [7, 9]]);
  assert.deepEqual(fenster(s, 1, 1), s.slice(2, 5));
  assert.deepEqual(fenster(s, 0, 2), s.slice(0, 5));
  assert.deepEqual(fenster(s, 3, 1), []);
});

test("Fenster verdoppelt keinen gemeinsamen Kontext", () => {
  const s = vergleichSegmente("a  b  c", [
    aenderung(), aenderung(),
  ], [[1, 3], [4, 6]]);
  assert.deepEqual(fenster(s, 0, 2), s);
});

test("Fenster ohne Änderungen und ungültige Seitengrößen bleiben leer", () => {
  const s = vergleichSegmente("Text", [], []);
  assert.deepEqual(fenster(s, 0, 20), []);
  for (const [seite, groesse] of [[-1, 20], [0, 0], [0, 1.5]]) {
    assert.deepEqual(fenster(s, seite, groesse), []);
  }
});

test("Ein Megabyte Kontext wird nur als vorhandenes Segment referenziert", () => {
  const text = `${"x".repeat(1_000_000)}  Ende`;
  const s = vergleichSegmente(text, [aenderung()], [[1_000_000, 1_000_002]]);
  const sichtbar = fenster(s, 0, 20);
  assert.equal(sichtbar.length, 3);
  assert.equal(sichtbar[0], s[0]);
  assert.equal(sichtbar[1].art, "aenderung");
});
