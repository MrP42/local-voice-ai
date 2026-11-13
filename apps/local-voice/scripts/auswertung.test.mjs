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
  fristenIn,
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
  assert.ok(stand.prompt.length >= 6);
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
  let zustand = 7;
  const zufall = (n) => {
    zustand = (zustand * 1103515245 + 12345) % 2147483648;
    return zustand % n;
  };
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

test("fristenIn: findet Ausdrücke in Sätzen", () => {
  const heute = { jahr: 2026, monat: 11, tag: 13 };
  assert.deepEqual(fristenIn("Machen wir am 14.11. oder übermorgen.", heute), [
    "2026-11-14",
    "2026-11-15",
  ]);
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
