// Lva#559 (G5-Kern): Auswertung am Gesprächsende, reine Logik (Prompt, Lesen der Rohantwort mit Erdung, Fristauflösung,
// Kodierung). Die gemeinsamen Prüfstände (`apps/shared/assistenten/auswertung-pruefstaende.json`) bestehen hier und in
// `AssistantOutcomeTests.swift` mit identischem Ergebnis.
// Lauf: node --test scripts/auswertung.test.mjs (Node >= 22.6).
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { register } from "node:module";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

register("./test-stubs/hooks.mjs", import.meta.url);

const kern = await import("../src/components/voice-call/assistenten/index.ts");
const {
  parseProfil,
  auswerten,
  auswertungPrompt,
  kodiereAuswertung,
  parseAuswertung,
  frist,
  fristDeckungImKontext,
  fristenIn,
  fundstellen,
  maximaleFristen,
  FRIST_RAND,
  normalisiere,
  gespraechZeilen,
  HINWEIS_CODES,
  entfernePraefix,
  MAX_ROHANTWORT,
} = kern;

const GEMEINSAM = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../shared/assistenten",
);
const stand = JSON.parse(
  readFileSync(path.join(GEMEINSAM, "auswertung-pruefstaende.json"), "utf8"),
);

/** Pfade bestehen nur aus Namen des Formats und Indizes. */
const PFAD_MUSTER =
  /^(|(felder|aufgaben|offene_fragen)(\[[0-9]+\])?(\.(wert|beleg|beitrag|status|text|frist_text))?|zusammenfassung)$/;

function rohAus(fall) {
  if (fall.roh_teile === undefined) return fall.roh;
  return fall.roh_teile.map((t) => t.text.repeat(t.anzahl)).join("");
}

function profilAus(daten) {
  const r = parseProfil(JSON.stringify(daten));
  assert.equal(r.ok, true, JSON.stringify(r.fehler));
  return r.profil;
}

test("Prüfstände: Umfang und jeder Hinweis-Code kommt vor", () => {
  assert.ok(stand.auswerten.length >= 12);
  assert.ok(stand.frist.length >= 40);
  assert.ok(stand.prompt.length >= 10);
  assert.ok(stand.lesen_ok.length >= 40);
  assert.ok(stand.fristen_in.length >= 20);
  const codes = new Set(
    stand.auswerten.flatMap((f) =>
      (f.erwartet.hinweise ?? []).map((h) => h.code),
    ),
  );
  for (const code of HINWEIS_CODES) assert.ok(codes.has(code), code);
});

for (const fall of stand.auswerten) {
  test(`auswerten: ${fall.name}`, () => {
    const profil = profilAus(fall.profil);
    const r = auswerten(profil, fall.verlauf, rohAus(fall), fall.heute, {
      erstellt: fall.erstellt,
      gekuerzt: fall.gekuerzt,
    });
    if (!fall.erwartet.ok) {
      assert.deepEqual(r, { ok: false, fehler: fall.erwartet.fehler });
      return;
    }
    assert.equal(r.ok, true);
    const text = kodiereAuswertung(r.ergebnis);
    assert.equal(text, fall.erwartet.kodiert);
    assert.deepEqual(r.hinweise, fall.erwartet.hinweise);
    // Rundreise: erneut lesen und schreiben ergibt dieselben Bytes.
    const noch = parseAuswertung(text);
    assert.equal(noch.ok, true, JSON.stringify(noch.fehler));
    assert.equal(kodiereAuswertung(noch.ergebnis), text);
    // Abnahme: kein Wert ohne Beleg im Gespräch.
    const normiert = fall.verlauf.map((b) => normalisiere(b.text));
    for (const feld of r.ergebnis.felder) {
      if (feld.beleg === null) {
        assert.equal(feld.wert, null);
        continue;
      }
      assert.ok(normiert[feld.beitrag - 1].includes(normalisiere(feld.beleg)));
    }
    for (const a of r.ergebnis.aufgaben) {
      assert.ok(normiert[a.beitrag - 1].includes(normalisiere(a.beleg)));
      assert.equal(a.frist === null, a.fristBeitrag === null);
      if (a.fristBeitrag !== null) {
        assert.ok(
          normiert[a.fristBeitrag - 1].includes(normalisiere(a.fristText)),
        );
      }
    }
    // Kein Hinweis-Pfad trägt Inhalt: nur Namen des Formats und Indizes.
    for (const h of r.hinweise) {
      assert.match(h.pfad, PFAD_MUSTER, h.pfad);
    }
  });
}

for (const fall of stand.frist) {
  test(`frist: "${fall.ausdruck}" ab ${fall.heute.jahr}-${fall.heute.monat}-${fall.heute.tag}`, () => {
    assert.equal(frist(fall.ausdruck, fall.heute), fall.erwartet);
  });
}

for (const fall of stand.normalisierung) {
  test(`normalisiere: ${JSON.stringify(fall.eingabe)}`, () => {
    assert.equal(normalisiere(fall.eingabe), fall.erwartet);
  });
}

for (const fall of stand.lesen_fehler) {
  test(`parseAuswertung: ${fall.name}`, () => {
    const eingabe =
      typeof fall.eingabe === "string"
        ? fall.eingabe
        : JSON.stringify(fall.eingabe);
    const r = parseAuswertung(eingabe);
    assert.equal(r.ok, false);
    assert.deepEqual(r.fehler, fall.erwartet.fehler);
  });
}

for (const fall of stand.prompt) {
  test(`auswertungPrompt: ${fall.name}`, () => {
    const p = auswertungPrompt(
      profilAus(fall.profil),
      fall.verlauf,
      fall.heute,
      fall.budget,
    );
    assert.equal(p.text, fall.erwartet.text);
    assert.equal(p.gekuerzt, fall.erwartet.gekuerzt);
  });
}

for (const fall of stand.lesen_ok) {
  test(`parseAuswertung gelingt: ${fall.name}`, () => {
    const r = parseAuswertung(fall.kodiert);
    assert.equal(r.ok, true, JSON.stringify(r.fehler));
    assert.equal(kodiereAuswertung(r.ergebnis), fall.kodiert);
  });
}

for (const fall of stand.fristen_in) {
  test(`fristenIn: "${fall.text}"`, () => {
    assert.deepEqual(fristenIn(fall.text, fall.heute), fall.erwartet.tage);
    const n = normalisiere(fall.text);
    assert.equal(
      fristDeckungImKontext(n, 0, n.length, fall.heute),
      fall.erwartet.deckung,
    );
  });
}

// ------------------------------------------------------------------ Eigenschaften

test("Prompt: Kürzung hält das Budget ein, behält Anfang und Ende und ändert die Nummern nicht", () => {
  const profil = profilAus({
    schema: "lva.assistent/1",
    id: "eigen-aaaaaaaaaaaa",
    name: "T",
  });
  const verlauf = Array.from({ length: 60 }, (_, i) => ({
    rolle: i % 2 === 0 ? "nutzer" : "assistent",
    text: `Beitrag Nummer ${i + 1} mit etwas Text zum Füllen der Zeile.`,
  }));
  const heute = { jahr: 2026, monat: 11, tag: 13 };
  const voll = auswertungPrompt(profil, verlauf, heute, 1e9);
  assert.equal(voll.gekuerzt, false);
  for (const budget of [1500, 1800, 2200, 3000]) {
    const p = auswertungPrompt(profil, verlauf, heute, budget);
    assert.equal(p.gekuerzt, true);
    assert.ok([...p.text].length <= budget, `Budget ${budget}`);
    const zeilen = p.text.split("\n");
    const i = zeilen.findIndex((z) => z.startsWith("[1] "));
    assert.ok(zeilen[i + 1].startsWith("[2] "));
    assert.match(zeilen[i + 2], /^\[… \d+ Beiträge ausgelassen …\]$/);
    assert.ok(zeilen[zeilen.length - 1].startsWith("[60] "));
    // Ein Beitrag mehr passte nicht mehr hinein.
    const k = zeilen.length - (i + 3);
    const ausgelassen = Number(/\d+/.exec(zeilen[i + 2])[0]);
    assert.equal(ausgelassen, 60 - 2 - k);
    const mehr = auswertungPrompt(profil, verlauf, heute, budget + 80);
    assert.ok(mehr.text.split("\n").length >= zeilen.length);
  }
});

test("Prompt: Gesprächsfassung nummeriert, Leerraum zusammengefasst", () => {
  assert.deepEqual(
    gespraechZeilen([
      { rolle: "nutzer", text: " a \n b\t c " },
      { rolle: "assistent", text: "" },
    ]),
    ["[1] Nutzer: a b c", "[2] Assistent:"],
  );
});

test("Erdung: ein Wert ohne Beleg im Gespräch entsteht nie, auch nicht bei zufälligen Antworten", () => {
  const profil = profilAus({
    schema: "lva.assistent/1",
    id: "eigen-aaaaaaaaaaaa",
    name: "T",
    auswertung: {
      zusammenfassung: true,
      aufgaben: true,
      felder: [
        { name: "a", typ: "text" },
        { name: "b", typ: "zahl" },
        { name: "c", typ: "datum" },
        { name: "d", typ: "auswahl", optionen: ["x", "y"] },
      ],
    },
  });
  const verlauf = [
    { rolle: "nutzer", text: "Es geht um 5 Dinge bis morgen." },
    { rolle: "assistent", text: "Gut." },
  ];
  const heute = { jahr: 2026, monat: 11, tag: 13 };
  const werte = ["5", "x", "2026-11-14", "fünf", "morgen", "99", "y", ""];
  const belege = ["5 Dinge", "bis morgen", "Gut.", "erfunden", "", "Es geht"];
  const zufall = zufallserzeuger(7);
  const stati = ["klar", "unklar", "nicht_genannt"];
  for (let lauf = 0; lauf < 300; lauf += 1) {
    const felder = ["a", "b", "c", "d"].map((name) => ({
      name,
      status: stati[zufall(3)],
      wert: werte[zufall(werte.length)],
      beleg: belege[zufall(belege.length)],
      beitrag: zufall(4),
    }));
    const aufgaben = [
      {
        text: "t",
        frist_text: "morgen",
        beleg: belege[zufall(belege.length)],
        beitrag: 1,
      },
    ];
    const r = auswerten(
      profil,
      verlauf,
      JSON.stringify({ felder, aufgaben }),
      heute,
    );
    assert.equal(r.ok, true);
    const normiert = verlauf.map((b) => normalisiere(b.text));
    for (const f of r.ergebnis.felder) {
      if (f.wert !== null) {
        assert.notEqual(f.beleg, null);
        assert.ok(normiert[f.beitrag - 1].includes(normalisiere(f.beleg)));
      }
      if (f.status === "klar") assert.notEqual(f.wert, null);
      if (f.status === "nicht_genannt") {
        assert.deepEqual([f.wert, f.beleg, f.beitrag], [null, null, null]);
      }
    }
    for (const a of r.ergebnis.aufgaben) {
      assert.ok(normiert[a.beitrag - 1].includes(normalisiere(a.beleg)));
    }
  }
});

test("Hinweise tragen nur Pfad und Code, nie Inhalt des Modells", () => {
  const profil = profilAus({
    schema: "lva.assistent/1",
    id: "eigen-aaaaaaaaaaaa",
    name: "T",
  });
  const geheim = "GEHEIMER-SCHLUESSEL-TEXT";
  const r = auswerten(
    profil,
    [{ rolle: "nutzer", text: "Hallo" }],
    JSON.stringify({
      [geheim]: 1,
      [geheim.toLowerCase()]: 2,
      felder: [{ name: geheim }],
    }),
    { jahr: 2026, monat: 11, tag: 13 },
  );
  assert.equal(r.ok, true);
  assert.ok(!JSON.stringify(r.hinweise).includes("GEHEIM"));
  assert.ok(!JSON.stringify(r.hinweise).toLowerCase().includes("geheim-"));
  for (const h of r.hinweise)
    assert.deepEqual(Object.keys(h).sort(), ["code", "pfad"]);
});

test("maximaleFristen: eine abgeschnittene Wortliste lässt fehlenden Folgekontext nicht als sicher gelten", () => {
  const heute = { jahr: 2026, monat: 11, tag: 13 };
  const woerter = ["spätestens", "bis", "zum", "ende", "der", "woche", "und"];
  const unsicher = (liste, abgeschnitten) =>
    maximaleFristen(liste, 0, 5, heute, abgeschnitten).map((t) => t.unsicher);
  assert.equal(FRIST_RAND, 5);
  // am echten Textende gelten die normalen Regeln
  assert.deepEqual(unsicher(woerter, undefined), [false]);
  assert.deepEqual(unsicher(woerter, { vorn: false, hinten: false }), [false]);
  // hinten abgeschnitten, weniger als FRIST_RAND Wörter dahinter: unsicher
  assert.deepEqual(unsicher(woerter, { vorn: false, hinten: true }), [true]);
  // vorn abgeschnitten, weniger als FRIST_RAND Wörter davor: unsicher
  assert.deepEqual(unsicher(woerter, { vorn: true, hinten: false }), [true]);
  // genug Rand: sicher, auch wenn die Liste abgeschnitten ist
  const vorn = ["a1", "a2", "a3", "a4", "a5"];
  const lang = [...vorn, ...woerter, "b1", "b2", "b3", "b4"];
  const t = maximaleFristen(lang, 5, 10, heute, { vorn: true, hinten: true });
  assert.deepEqual(
    t.map((x) => x.unsicher),
    [false],
  );
});

test("fundstellen: nur an Wortgrenzen, überlappende Vorkommen, höchstens so viele wie verlangt", () => {
  assert.deepEqual(fundstellen("ja jahr ja", "ja", 10), [0, 8]);
  assert.deepEqual(fundstellen("jahr", "ja", 10), []);
  assert.deepEqual(fundstellen("a a a", "a", 2), [0, 2]);
  assert.deepEqual(fundstellen("12,5 liter", "12", 5), [0]);
  assert.deepEqual(fundstellen("x+5 und +5", "+5", 5), [1, 8]);
  assert.deepEqual(fundstellen("a😀b 😀", "😀", 5), [5]);
  assert.deepEqual(fundstellen("abc", "", 5), []);
});

test("fristenIn: findet Ausdrücke in Sätzen", () => {
  const heute = { jahr: 2026, monat: 11, tag: 13 };
  assert.deepEqual(
    fristenIn("Machen wir am 14.11., dann melde ich mich übermorgen.", heute),
    ["2026-11-14", "2026-11-15"],
  );
});

// ------------------------------------------------------------------ Grenzen und große Eingaben

const heuteFix = { jahr: 2026, monat: 11, tag: 13 };
const leeresProfil = () =>
  profilAus({ schema: "lva.assistent/1", id: "eigen-aaaaaaaaaaaa", name: "T" });
const feldProfil = () =>
  profilAus({
    schema: "lva.assistent/1",
    id: "eigen-aaaaaaaaaaaa",
    name: "T",
    auswertung: {
      zusammenfassung: true,
      aufgaben: true,
      felder: [{ name: "projekt", typ: "text" }],
    },
  });

function pruefeLaengen(ergebnis) {
  assert.ok([...ergebnis.zusammenfassung].length <= 1200);
  assert.ok(ergebnis.aufgaben.length <= 20);
  for (const a of ergebnis.aufgaben) {
    assert.ok([...a.text].length <= 300);
    assert.ok([...a.beleg].length <= 500);
  }
  for (const f of ergebnis.felder) {
    if (f.wert !== null) assert.ok([...f.wert].length <= 500);
    if (f.beleg !== null) assert.ok([...f.beleg].length <= 500);
  }
  for (const q of ergebnis.offeneFragen) assert.ok([...q].length <= 300);
}

test("Große Eingabe: ein Beitrag mit mehreren Millionen Zeichen", () => {
  const riesig = "Es geht um das Projekt Alpha. ".repeat(100000); // 3 Mio.
  assert.ok(riesig.length >= 3_000_000);
  const verlauf = [
    { rolle: "nutzer", text: riesig },
    { rolle: "assistent", text: "Verstanden." },
  ];
  const profil = feldProfil();
  // Mit höchstens zwei Beiträgen wird nie geschnitten (README), der Prompt bleibt ganz.
  const ganz = auswertungPrompt(profil, verlauf, heuteFix, 20000);
  assert.equal(ganz.gekuerzt, false);
  assert.ok(ganz.text.length > riesig.length);
  // Der riesige Beitrag in der Mitte fällt unter die Auslassung, das Budget gilt.
  const mitte = [
    { rolle: "nutzer", text: "a" },
    { rolle: "assistent", text: "b" },
    { rolle: "nutzer", text: riesig },
    { rolle: "assistent", text: "c" },
  ];
  const p = auswertungPrompt(profil, mitte, heuteFix, 20000);
  assert.equal(p.gekuerzt, true);
  assert.ok([...p.text].length <= 20000);
  assert.match(p.text, /\[… 1 Beiträge? ausgelassen …\]|\[… \d+ Beitr/);
  const roh = JSON.stringify({
    zusammenfassung: "x".repeat(5000),
    felder: [
      {
        name: "projekt",
        status: "klar",
        wert: "Alpha",
        beleg: "projekt alpha",
        beitrag: 1,
      },
    ],
    aufgaben: [
      { text: "t", frist_text: "morgen", beleg: "projekt alpha", beitrag: 1 },
    ],
  });
  const r = auswerten(profil, verlauf, roh, heuteFix);
  assert.equal(r.ok, true);
  pruefeLaengen(r.ergebnis);
  assert.equal(r.ergebnis.felder[0].wert, "Alpha");
  assert.equal(r.ergebnis.felder[0].beitrag, 1);
  assert.equal(normalisiere(riesig).length > 2_000_000, true);
  assert.ok(Array.isArray(fristenIn("morgen ".repeat(500000), heuteFix)));
  assert.ok(fristenIn("am 14.11. ".repeat(500000), heuteFix).length <= 1024);
  assert.equal(frist("morgen ".repeat(500000), heuteFix), null);
});

test("Große Eingabe: sehr viele Beiträge", () => {
  const n = 500000;
  const verlauf = Array.from({ length: n }, (_, i) => ({
    rolle: i % 2 === 0 ? "nutzer" : "assistent",
    text: i === n - 1 ? "Projekt Omega" : "ja",
  }));
  const profil = feldProfil();
  const p = auswertungPrompt(profil, verlauf, heuteFix, 20000);
  assert.equal(p.gekuerzt, true);
  assert.ok([...p.text].length <= 20000);
  assert.ok(gespraechZeilen(verlauf).length === n);
  const roh = JSON.stringify({
    felder: [
      {
        name: "projekt",
        status: "klar",
        wert: "Omega",
        beleg: "Projekt Omega",
        beitrag: n,
      },
    ],
  });
  const r = auswerten(profil, verlauf, roh, heuteFix);
  assert.equal(r.ok, true);
  pruefeLaengen(r.ergebnis);
  assert.equal(r.ergebnis.felder[0].beitrag, n);
  assert.equal(r.ergebnis.felder[0].wert, "Omega");
});

test("Große Eingabe: sehr langer Beleg und Wert innerhalb der Rohantwort-Grenze", () => {
  const langer = "wort ".repeat(5000); // 25000 Zeichen, siebenmal in der Antwort
  const verlauf = [{ rolle: "nutzer", text: `Anfang ${langer} Ende` }];
  const roh = JSON.stringify({
    zusammenfassung: langer,
    felder: [
      {
        name: "projekt",
        status: "klar",
        wert: langer,
        beleg: langer,
        beitrag: 1,
      },
    ],
    aufgaben: [{ text: langer, frist_text: langer, beleg: langer, beitrag: 1 }],
    offene_fragen: [langer],
  });
  assert.ok(roh.length < MAX_ROHANTWORT);
  const r = auswerten(feldProfil(), verlauf, roh, heuteFix);
  assert.equal(r.ok, true);
  pruefeLaengen(r.ergebnis);
  assert.ok(kodiereAuswertung(r.ergebnis).length > 0);
});

test("Obergrenze: Rohantwort über 262144 Codepunkten ist json, ohne weitere Arbeit", () => {
  const ok = `{"offene_fragen":["Wer?"]}${" ".repeat(MAX_ROHANTWORT - 26)}`;
  assert.equal([...ok].length, MAX_ROHANTWORT);
  const verlauf = [{ rolle: "nutzer", text: "Hallo" }];
  assert.equal(auswerten(leeresProfil(), verlauf, ok, heuteFix).ok, true);
  const zuViel = `${ok} `;
  assert.deepEqual(auswerten(leeresProfil(), verlauf, zuViel, heuteFix), {
    ok: false,
    fehler: "json",
  });
  const t0 = performance.now();
  const gross = "{".repeat(5_000_000);
  assert.deepEqual(auswerten(leeresProfil(), verlauf, gross, heuteFix), {
    ok: false,
    fehler: "json",
  });
  assert.ok(performance.now() - t0 < 30000);
});

test("Obergrenze: 200000 öffnende Klammern enden schnell als json (nicht quadratisch)", () => {
  const verlauf = [{ rolle: "nutzer", text: "Hallo" }];
  const t0 = performance.now();
  const r = auswerten(leeresProfil(), verlauf, "{".repeat(200000), heuteFix);
  assert.deepEqual(r, { ok: false, fehler: "json" });
  assert.ok(performance.now() - t0 < 30000, `${performance.now() - t0} ms`);
});

test("Obergrenze: viele ausbalancierte, ungültige Objekte enden schnell", () => {
  const verlauf = [{ rolle: "nutzer", text: "Hallo" }];
  const t0 = performance.now();
  const r = auswerten(leeresProfil(), verlauf, "{,}".repeat(80000), heuteFix);
  assert.deepEqual(r, { ok: false, fehler: "json" });
  assert.ok(performance.now() - t0 < 30000, `${performance.now() - t0} ms`);
  const t1 = performance.now();
  const mit = auswerten(
    leeresProfil(),
    verlauf,
    `${"{,}".repeat(80000)}{"offene_fragen":["y"]}`,
    heuteFix,
  );
  assert.equal(mit.ok, true);
  assert.deepEqual(mit.ergebnis.offeneFragen, ["y"]);
  assert.ok(performance.now() - t1 < 30000);
});

test("Obergrenze: verschachtelt und verschränkt, Zeichenketten mit Klammern", () => {
  const verlauf = [{ rolle: "nutzer", text: "Hallo" }];
  const t0 = performance.now();
  for (const roh of [
    '{"a":'.repeat(40000),
    '{"a":['.repeat(40000),
    '{"'.repeat(60000),
    `{"x":"${"{".repeat(100000)}`,
    `${"{ ".repeat(16)}${"{,}".repeat(50000)}`,
  ]) {
    const r = auswerten(leeresProfil(), verlauf, roh, heuteFix);
    assert.deepEqual(r, { ok: false, fehler: "json" });
  }
  assert.ok(performance.now() - t0 < 60000, `${performance.now() - t0} ms`);
});

test("entfernePraefix: Beitragsnummer, Rolle, englische Rollen, kein Präfix", () => {
  assert.deepEqual(entfernePraefix("[3] Nutzer: Projekt Alpha"), {
    text: "Projekt Alpha",
    beitrag: 3,
  });
  assert.deepEqual(entfernePraefix("Nutzer: Projekt Alpha"), {
    text: "Projekt Alpha",
    beitrag: null,
  });
  assert.deepEqual(entfernePraefix("assistent:Projekt"), {
    text: "Projekt",
    beitrag: null,
  });
  assert.deepEqual(entfernePraefix("[12] User: x"), { text: "x", beitrag: 12 });
  assert.deepEqual(entfernePraefix("Assistant : x"), {
    text: "x",
    beitrag: null,
  });
  assert.deepEqual(entfernePraefix("[7] nur Nummer"), {
    text: "nur Nummer",
    beitrag: 7,
  });
  for (const unveraendert of [
    "Projekt Alpha",
    "Chef: Projekt",
    "[x] Nutzer: a",
    "[] Nutzer: a",
    "[1234567] a",
    "Nutzerin: a",
  ]) {
    assert.deepEqual(entfernePraefix(unveraendert), {
      text: unveraendert,
      beitrag: null,
    });
  }
});

// ------------------------------------------------------------------ Invarianten

/**
 * Ganzzahliger 32-Bit-Zufallserzeuger (Linearer Kongruenzgenerator, Zahlen nach Numerical Recipes): `Math.imul` und `>>> 0`
 * rechnen modulo 2^32 ohne Gleitkomma; der Swift-Test rechnet mit `&*` und `&+` auf `UInt32` dieselbe Folge.
 */
function zufallserzeuger(start) {
  let zustand = start >>> 0;
  return (n) => {
    zustand = (Math.imul(zustand, 1664525) + 1013904223) >>> 0;
    return (zustand >>> 16) % n;
  };
}

test("Zufallserzeuger: dieselbe Folge wie in Swift (unabhängig in Python berechnet)", () => {
  const folge = (start) => {
    const z = zufallserzeuger(start);
    return Array.from({ length: 12 }, () => z(1000));
  };
  assert.deepEqual(
    folge(7),
    [648, 866, 140, 750, 233, 84, 657, 480, 507, 483, 753, 816],
  );
  assert.deepEqual(
    folge(11),
    [750, 108, 24, 918, 871, 484, 579, 36, 792, 7, 623, 727],
  );
});

/** Jede Beitragsnummer des Ergebnisses liegt im Gespräch (1 bis Anzahl der Beiträge). */
function pruefeBeitragsnummern(ergebnis, anzahl, was) {
  const imGespraech = (n) => Number.isInteger(n) && n >= 1 && n <= anzahl;
  for (const f of ergebnis.felder) {
    if (f.beitrag !== null) assert.ok(imGespraech(f.beitrag), `${was}: Feld`);
  }
  for (const a of ergebnis.aufgaben) {
    assert.ok(imGespraech(a.beitrag), `${was}: Aufgabe`);
    if (a.fristBeitrag !== null) {
      assert.ok(imGespraech(a.fristBeitrag), `${was}: Frist`);
    }
  }
}

function pruefeRundreise(ergebnis, was) {
  const text = kodiereAuswertung(ergebnis);
  const r = parseAuswertung(text);
  assert.equal(r.ok, true, `${was}: ${JSON.stringify(r.fehler)}`);
  assert.equal(kodiereAuswertung(r.ergebnis), text, was);
}

test("Invariante: parse(encode(auswerten(…))) gelingt für alle Prüfstandsfälle und kodiert bytegleich zurück", () => {
  let gezaehlt = 0;
  for (const fall of stand.auswerten) {
    const r = auswerten(
      profilAus(fall.profil),
      fall.verlauf,
      rohAus(fall),
      fall.heute,
      { erstellt: fall.erstellt, gekuerzt: fall.gekuerzt },
    );
    if (!r.ok) continue;
    pruefeRundreise(r.ergebnis, fall.name);
    pruefeBeitragsnummern(r.ergebnis, fall.verlauf.length, fall.name);
    gezaehlt += 1;
  }
  assert.ok(gezaehlt >= 60);
});

test("Invariante: auch bei Extremwerten bleibt jedes Ergebnisfeld lesbar (zufällige Antworten)", () => {
  const profil = profilAus({
    schema: "lva.assistent/1",
    id: "eigen-aaaaaaaaaaaa",
    name: "T",
    auswertung: {
      zusammenfassung: true,
      aufgaben: true,
      felder: [
        { name: "a", typ: "text" },
        { name: "b", typ: "zahl" },
        { name: "c", typ: "datum" },
        { name: "d", typ: "auswahl", optionen: ["ja", "nein"] },
        { name: "e", typ: "zahl" },
      ],
    },
  });
  const verlauf = [
    { rolle: "nutzer", text: "Es geht um 5 Dinge, 0001 Stück, ja gerne." },
    {
      rolle: "assistent",
      text: "Gut, übernächsten Montag oder Montag in zwei Wochen?",
    },
    { rolle: "nutzer", text: "Hallo, morgen ist das Jahr 2027 und eins." },
  ];
  const lang = (z, n) => z.repeat(n);
  const werte = [
    "5",
    "0001",
    "-0",
    "00.50",
    "1e5",
    "NaN",
    "eins",
    "ja",
    "Jahr",
    "morgen",
    "2026-11-14",
    "2026-02-30",
    "übernächsten Montag",
    "Montag in zwei Wochen",
    "12,5",
    "1.250,50",
    "",
    "  ",
    lang("0", 500) + "1",
    lang("0", 501) + "1",
    lang("9", 600),
    "-" + lang("0", 499) + "5",
    lang("1", 501),
    lang("ä", 501),
    "x" + lang(" ", 600) + "y",
    lang("😀", 520),
    "-" + lang("0", 700),
  ];
  const belege = [
    "Es geht um 5 Dinge",
    "0001 Stück",
    "ja gerne",
    "übernächsten Montag",
    "Montag in zwei Wochen",
    "Hallo",
    "eins",
    "morgen ist das Jahr 2027",
    "erfunden",
    "",
    "[3] Nutzer: Hallo",
    "Assistent: Gut",
    lang("Hallo ", 200),
  ];
  const fristTexte = [
    "Montag",
    "morgen",
    "übernächsten Montag",
    "heute",
    "",
    lang("x", 150),
    "in zwei Wochen",
    "Jahr",
  ];
  const langtexte = [
    "t",
    lang("Satz eins. ", 100),
    lang("ä", 400),
    lang("😀", 1300),
    "  ",
    "",
  ];
  const erstellt = [
    "",
    "heute",
    "2026-02-30T10:00:00Z",
    "2026-11-13T10:00:00Z",
    "2026-11-13T25:00:00Z",
  ];
  const zufall = zufallserzeuger(11);
  const wahl = (liste) => liste[zufall(liste.length)];
  const stati = ["klar", "unklar", "nicht_genannt", undefined, "?"];
  for (let lauf = 0; lauf < 600; lauf += 1) {
    const felder = ["a", "b", "c", "d", "e"].map((name) => ({
      name,
      status: wahl(stati),
      wert: wahl(werte),
      beleg: wahl(belege),
      beitrag: wahl([1, 2, 3, 4, 0, null, "2"]),
    }));
    const aufgaben = Array.from({ length: zufall(4) }, () => ({
      text: wahl(langtexte),
      frist_text: wahl(fristTexte),
      beleg: wahl(belege),
      beitrag: wahl([1, 2, 3, 9]),
    }));
    const roh = JSON.stringify({
      felder,
      aufgaben,
      zusammenfassung: wahl(langtexte),
      offene_fragen: [wahl(langtexte), wahl(langtexte)],
    });
    const r = auswerten(profil, verlauf, roh, heuteFix, {
      erstellt: wahl(erstellt),
      gekuerzt: zufall(2) === 0,
    });
    assert.equal(r.ok, true);
    pruefeLaengen(r.ergebnis);
    pruefeRundreise(r.ergebnis, `Lauf ${lauf}`);
    pruefeBeitragsnummern(r.ergebnis, verlauf.length, `Lauf ${lauf}`);
    for (const h of r.hinweise) assert.match(h.pfad, PFAD_MUSTER);
  }
});

test("Hinweise: kein Pfad trägt einen unbekannten Schlüssel (Wurzel, Feld, Aufgabe)", () => {
  const profil = profilAus({
    schema: "lva.assistent/1",
    id: "eigen-aaaaaaaaaaaa",
    name: "T",
    auswertung: {
      aufgaben: true,
      felder: [{ name: "iban_de00", typ: "text" }],
    },
  });
  const geheim = { token_abc123: 1, passwort: 2, iban_de00: 3, Zusatz: 4 };
  const roh = JSON.stringify({
    ...geheim,
    felder: [
      {
        name: "iban_de00",
        status: "klar",
        wert: "A",
        beleg: "A",
        beitrag: 1,
        ...geheim,
      },
      { name: "token_abc123", ...geheim },
    ],
    aufgaben: [{ text: "t", beleg: "A", beitrag: 1, ...geheim }],
  });
  const r = auswerten(profil, [{ rolle: "nutzer", text: "A" }], roh, heuteFix);
  assert.equal(r.ok, true);
  const alle = JSON.stringify(r.hinweise);
  for (const wort of ["token_abc123", "passwort", "iban_de00", "Zusatz"]) {
    assert.ok(!alle.includes(wort), wort);
  }
  assert.deepEqual(r.hinweise, [
    { pfad: "", code: "unbekannt" },
    { pfad: "aufgaben[0]", code: "unbekannt" },
    { pfad: "felder[0]", code: "unbekannt" },
    { pfad: "felder[1]", code: "unbekannt" },
  ]);
});

test("Zahl: 500 Nullen und eine 1 ergeben die kanonische Zahl 1, die Rundreise gelingt", () => {
  const profil = profilAus({
    schema: "lva.assistent/1",
    id: "eigen-aaaaaaaaaaaa",
    name: "T",
    auswertung: { felder: [{ name: "n", typ: "zahl" }] },
  });
  const roh = JSON.stringify({
    felder: [
      {
        name: "n",
        status: "klar",
        wert: "0".repeat(500) + "1",
        beleg: "eins",
        beitrag: 1,
      },
    ],
  });
  const r = auswerten(
    profil,
    [{ rolle: "nutzer", text: "eins" }],
    roh,
    heuteFix,
  );
  assert.equal(r.ergebnis.felder[0].wert, "1");
  assert.equal(r.ergebnis.felder[0].status, "klar");
  pruefeRundreise(r.ergebnis, "500 Nullen");
});

test("Prompt: keine Zeilentrenner aus Beiträgen oder Profiltexten bleiben im Prompt", () => {
  const profil = profilAus({
    schema: "lva.assistent/1",
    id: "eigen-aaaaaaaaaaaa",
    name: "T",
    auswertung: {
      felder: [
        {
          name: "a",
          beschreibung: "x\u2028y\u0085z\u2029w\rv",
          typ: "auswahl",
          optionen: ['a"b', "c\u2028d"],
        },
      ],
    },
  });
  const p = auswertungPrompt(
    profil,
    [{ rolle: "nutzer", text: "1\u20282\u00853\u20294\r5\n6" }],
    heuteFix,
    100000,
  );
  const nachGespraech = p.text.slice(p.text.indexOf("Gespräch:\n") + 10);
  assert.equal(nachGespraech, "[1] Nutzer: 1 2 3 4 5 6");
  for (const c of ["\u2028", "\u0085", "\u2029", "\r"])
    assert.ok(!p.text.includes(c));
  assert.ok(p.text.includes('"a\'b", "c d"'));
});

test("erstellt: ein ungültiger Wert gilt als nicht gesetzt", () => {
  for (const [wert, erwartet] of [
    ["2026-11-13T10:00:00Z", "2026-11-13T10:00:00Z"],
    ["2026-11-13T10:00:00", ""],
    ["2026-02-30T10:00:00Z", ""],
    ["2026-11-13T24:00:00Z", ""],
    ["heute", ""],
    ["", ""],
  ]) {
    const r = auswerten(
      leeresProfil(),
      [{ rolle: "nutzer", text: "Hallo" }],
      '{"offene_fragen":[]}',
      heuteFix,
      { erstellt: wert },
    );
    assert.equal(r.ergebnis.erstellt, erwartet, wert);
    pruefeRundreise(r.ergebnis, wert);
  }
});
