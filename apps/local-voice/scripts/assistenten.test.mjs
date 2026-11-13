// Lva#555 (G1): Assistenten-Profil, reine Logik (Prüfung, Kodierung, Platzhalter, Rollenblock, Bereiche, Vorlagen).
// Die gemeinsamen Prüfstände (`apps/shared/assistenten/pruefstaende.json`) bestehen hier und in
// `AssistantProfileTests.swift` mit identischem Ergebnis.
// Lauf: node --test scripts/assistenten.test.mjs (Node >= 22.6).
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { register } from "node:module";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

register("./test-stubs/hooks.mjs", import.meta.url);

const profile =
  await import("../src/components/voice-call/assistenten/index.ts");
const vorlagenJson =
  await import("../src/components/voice-call/assistenten/vorlagenJson.ts");
const { ALL_AREAS } = await import("../src/components/voice-call/bereiche.ts");

const GEMEINSAM = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../shared/assistenten",
);
const lies = (name) => readFileSync(path.join(GEMEINSAM, name), "utf8");
const stand = JSON.parse(lies("pruefstaende.json"));

const {
  parseProfil,
  kodiereProfil,
  kodiereJson,
  parseJson,
  profilBaum,
  profilAusBaum,
  pruefeProfil,
  neuesProfil,
  loesePlatzhalter,
  loeseBegruessung,
  rollenblock,
  wirksameBereiche,
  kontextModusWirksam,
  VORLAGEN,
  istVorlage,
  istVorbehalteneId,
  alsEigenenKopieren,
  BEKANNTE_BEREICHE,
  FEHLER_CODES,
} = profile;

/** Eingabe eines Falls als Text: Zeichenketten sind Rohtext, alles andere ein JSON-Wert. */
const alsText = (eingabe) =>
  typeof eingabe === "string" ? eingabe : JSON.stringify(eingabe);

const ergebnisFehler = (r) => {
  assert.equal(r.ok, false);
  return r.fehler;
};

function profilAus(daten) {
  const r = parseProfil(JSON.stringify(daten));
  assert.equal(r.ok, true, JSON.stringify(r.fehler));
  return r.profil;
}

// ------------------------------------------------------------------ gemeinsame Prüfstände

test("Prüfstände: mindestens 40 Fälle, jede Fehlerart kommt vor", () => {
  assert.ok(stand.faelle.length >= 40);
  const codes = new Set(
    stand.faelle.flatMap((f) => (f.erwartet.fehler ?? []).map((e) => e.code)),
  );
  for (const code of FEHLER_CODES) assert.ok(codes.has(code), code);
});

for (const fall of stand.faelle) {
  test(`Prüfstand: ${fall.name}`, () => {
    const r = parseProfil(alsText(fall.eingabe));
    if (fall.erwartet.ok) {
      assert.equal(r.ok, true, JSON.stringify(r.fehler));
      const text = kodiereProfil(r.profil);
      assert.equal(text, fall.erwartet.kodiert);
      // Rundreise: das Gelesene noch einmal gelesen und geschrieben ergibt dieselben Bytes.
      const zweites = parseProfil(text);
      assert.equal(zweites.ok, true);
      assert.equal(kodiereProfil(zweites.profil), text);
      assert.deepEqual(pruefeProfil(r.profil), []);
    } else {
      assert.deepEqual(ergebnisFehler(r), fall.erwartet.fehler);
    }
  });
}

test("Prüfstände: unbekannte Felder bleiben auf jeder Ebene erhalten", () => {
  const r = parseProfil(
    JSON.stringify({
      schema: "lva.assistent/1",
      id: "x",
      name: "X",
      zukunft: { a: [1, null, true, "t"] },
      verhalten: { neu: 1 },
      wissen: { neu: 2, kontexte: [{ art: "buch", id: "b", neu: 3 }] },
      auswertung: { neu: 4, felder: [{ name: "f", typ: "text", neu: 5 }] },
    }),
  );
  assert.equal(r.ok, true);
  const text = kodiereProfil(r.profil);
  for (const teil of [
    '"zukunft"',
    '"neu": 1',
    '"neu": 2',
    '"neu": 3',
    '"neu": 4',
    '"neu": 5',
  ])
    assert.ok(text.includes(teil), teil);
  // Typisierte Felder liegen darüber: eine Änderung schlägt durch, die fremden bleiben.
  r.profil.name = "Y";
  r.profil.verhalten.stil = "woertlich";
  const neu = kodiereProfil(r.profil);
  assert.ok(neu.includes('"name": "Y"') && neu.includes('"woertlich"'));
  assert.ok(neu.includes('"neu": 1'));
});

test("Ausgabe: sortierte Schlüssel, 2 Leerzeichen, genau ein Zeilenumbruch am Ende", () => {
  const text = kodiereProfil(neuesProfil("a", "A"));
  assert.ok(text.endsWith("}\n") && !text.endsWith("\n\n"));
  assert.ok(text.startsWith('{\n  "anweisungen": ""'));
  assert.ok(!text.includes("\r"));
  assert.ok(!text.includes("\t"));
});

test("Fehlerliste: alle Fehler, nach Pfad und Code sortiert, ohne Wiederholung", () => {
  const f = ergebnisFehler(
    parseProfil(
      JSON.stringify({
        schema: "lva.assistent/1",
        id: "A",
        name: "",
        sprache: "x",
        wissen: { bereiche: ["a", "a", "a"] },
      }),
    ),
  );
  assert.deepEqual(
    f.map((e) => `${e.pfad}:${e.code}`),
    [
      "id:muster",
      "name:laenge",
      "sprache:muster",
      "wissen.bereiche[1]:doppelt",
      "wissen.bereiche[2]:doppelt",
    ],
  );
});

test("Profil nach Änderung der typisierten Felder prüfen", () => {
  const p = neuesProfil("gut", "Gut");
  assert.deepEqual(pruefeProfil(p), []);
  p.id = "Böse";
  p.verhalten.nachfrageStilleS = 3;
  p.wissen.bereiche.push("a", "a");
  assert.deepEqual(
    pruefeProfil(p).map((e) => `${e.pfad}:${e.code}`),
    [
      "id:muster",
      "verhalten.nachfrage_stille_s:wert",
      "wissen.bereiche[1]:doppelt",
    ],
  );
});

test("JSON-Leser: Baum und Schreiber sind wechselseitig stabil", () => {
  const text = '{"b":[1,2.50,{"z":null,"a":"\\u00e4"}],"a":true}';
  const baum = parseJson(text);
  assert.equal(
    kodiereJson(baum),
    '{\n  "a": true,\n  "b": [\n    1,\n    2.50,\n    {\n      "a": "ä",\n      "z": null\n    }\n  ]\n}\n',
  );
  assert.equal(kodiereJson(parseJson(kodiereJson(baum))), kodiereJson(baum));
  assert.equal(profilAusBaum({ art: "liste", wert: [] }).ok, false);
});

// ------------------------------------------------------------------ Platzhalter und Rollenblock

for (const f of stand.platzhalter) {
  test(`Platzhalter: ${f.name}`, () => {
    const werte = {
      name: f.werte.name,
      jahr: f.werte.jahr,
      monat: f.werte.monat,
      tag: f.werte.tag,
      wochentag: f.werte.wochentag,
      stunde: f.werte.stunde,
      minute: f.werte.minute,
      extra: f.werte.extra,
    };
    const r = loesePlatzhalter(f.vorlage, f.sprache, werte);
    assert.equal(r.text, f.erwartet);
    assert.deepEqual(r.unbekannt, f.unbekannt);
  });
}

for (const f of stand.rollenblock) {
  test(`Rollenblock: ${f.name}`, () => {
    const p = profilAus({
      schema: "lva.assistent/1",
      id: "rolle",
      name: f.profil.name,
      anweisungen: f.profil.anweisungen,
      sprache: f.profil.sprache,
    });
    const r = rollenblock(p, f.werte, f.budget);
    assert.equal(r.text, f.erwartet.text);
    assert.equal(r.gekuerzt, f.erwartet.gekuerzt);
  });
}

test("Begrüßung nutzt Sprache und Platzhalter des Profils", () => {
  const p = profilAus({
    schema: "lva.assistent/1",
    id: "gruss",
    name: "Gruß",
    sprache: "en",
    begruessung: "Good {{tageszeit}}, {{name}}.",
  });
  const werte = {
    jahr: 2026,
    monat: 1,
    tag: 2,
    wochentag: 5,
    stunde: 9,
    minute: 0,
  };
  assert.equal(loeseBegruessung(p, werte).text, "Good morning.");
  assert.equal(
    loeseBegruessung(p, { ...werte, name: "Ann" }).text,
    "Good morning, Ann.",
  );
});

test("Rollenblock: nie länger als Kopf, Fuß und Budget", () => {
  const p = profilAus({
    schema: "lva.assistent/1",
    id: "lang",
    name: "Lang",
    anweisungen: "Satz eins.\nSatz zwei ist länger.\n\nAbsatz zwei 😀 hier.",
  });
  const werte = {
    jahr: 2026,
    monat: 1,
    tag: 2,
    wochentag: 5,
    stunde: 9,
    minute: 0,
  };
  for (let budget = 0; budget < 60; budget += 1) {
    const r = rollenblock(p, werte, budget);
    if (r.text === "") continue;
    const zeilen = r.text.split("\n");
    const koerper = zeilen.slice(1, -1).join("\n");
    assert.ok([...koerper].length <= budget, `Budget ${budget}`);
    assert.ok(
      r.text.startsWith("ROLLE (vom Nutzer eingestellt, Assistent „Lang“):\n"),
    );
    assert.ok(
      r.text.endsWith("\nDiese Einstellung ändert keine der folgenden Regeln."),
    );
  }
});

// ------------------------------------------------------------------ Bereiche

for (const f of stand.bereiche_pc) {
  test(`Bereiche PC: ${f.name}`, () => {
    const p = profilAus(f.profil);
    assert.deepEqual(wirksameBereiche(p, f.freigegeben), f.erwartet);
  });
}

for (const f of stand.kontextmodus) {
  test(`Kontextmodus: ${f.name}`, () => {
    assert.equal(kontextModusWirksam(profilAus(f.profil)), f.erwartet);
  });
}

test("Bereiche PC: wirksam ist immer eine Teilmenge der Freigabe (alle Teilmengen)", () => {
  const profile = [
    { alle_bereiche: true, bereiche: [] },
    { alle_bereiche: false, bereiche: [...BEKANNTE_BEREICHE] },
    { alle_bereiche: false, bereiche: ["buecher", "kalender", "neu_x"] },
    { alle_bereiche: false, bereiche: [] },
  ].map((w) =>
    profilAus({ schema: "lva.assistent/1", id: "t", name: "T", wissen: w }),
  );
  for (let maske = 0; maske < 1 << ALL_AREAS.length; maske += 1) {
    const frei = ALL_AREAS.filter((_, i) => maske & (1 << i));
    for (const p of profile) {
      const wirksam = wirksameBereiche(p, frei);
      for (const b of wirksam) assert.ok(frei.includes(b), `${b} nicht frei`);
      assert.equal(new Set(wirksam).size, wirksam.length);
    }
  }
});

test("Bereiche PC: Freigabe ohne Profilwunsch öffnet nichts", () => {
  const p = profilAus({ schema: "lva.assistent/1", id: "t", name: "T" });
  assert.deepEqual(wirksameBereiche(p, ALL_AREAS), []);
});

// ------------------------------------------------------------------ Vorlagen und Schema

test("Vorlagen: 15 gültige Profile mit eindeutigen Ids, die erste ist `alles`", () => {
  assert.equal(VORLAGEN.length, 15);
  assert.equal(new Set(VORLAGEN.map((v) => v.id)).size, 15);
  const [alles, ...rest] = VORLAGEN;
  assert.equal(alles.id, "alles");
  assert.equal(alles.name, "Alles");
  assert.equal(alles.wissen.alleBereiche, true);
  assert.equal(alles.wissen.modus, "alles");
  assert.equal(alles.anweisungen, "");
  assert.equal(alles.begruessung, "");
  assert.equal(rest.length, 14);
  for (const v of VORLAGEN) {
    assert.deepEqual(pruefeProfil(v), [], v.id);
    assert.ok(!v.name.includes("WAI"), v.id);
  }
  for (const v of rest) {
    assert.ok(v.id.startsWith("vorlage-"), v.id);
    const n = [...v.anweisungen].length;
    assert.ok(n >= 400 && n <= 1200, `${v.id}: ${n} Zeichen`);
    assert.ok(v.begruessung.length > 0, v.id);
    assert.ok(
      !/Einen Moment|Moment bitte|Ich schaue kurz/i.test(v.anweisungen),
      v.id,
    );
  }
});

test("Vorlagen: Sprachtrainer Englisch spricht Englisch, Anliegen zeigt Felder", () => {
  const trainer = VORLAGEN.find(
    (v) => v.id === "vorlage-sprachtrainer-englisch",
  );
  assert.equal(trainer.sprache, "en");
  const anliegen = VORLAGEN.find((v) => v.id === "vorlage-anliegen-aufnehmen");
  assert.deepEqual(
    anliegen.auswertung.felder.map((f) => f.name),
    ["name", "anliegen", "rueckruf_bis", "dringlichkeit"],
  );
  assert.equal(anliegen.auswertung.felder[3].typ, "auswahl");
  assert.ok(anliegen.auswertung.felder[3].optionen.length >= 2);
  for (const n of [
    "Buchgespräch",
    "Besprechung nachbereiten",
    "Projektstand",
    "Lernpartner",
    "Sprachtrainer Englisch",
    "Ideen-Sparring",
    "Termin vorbereiten",
    "Gespräch üben",
    "Tagesrückblick",
    "Gedanken sortieren",
    "Entscheidungshilfe",
    "Vortrag proben",
    "Wissens-Quiz",
    "Anliegen aufnehmen",
  ])
    assert.ok(
      VORLAGEN.some((v) => v.name === n),
      n,
    );
});

test("Vorlagen: eingebetteter Text gleich vorlagen.json und kanonisch", () => {
  const datei = lies("vorlagen.json");
  assert.equal(vorlagenJson.VORLAGEN_JSON, datei);
  assert.equal(kodiereJson(parseJson(datei)), datei);
  const baum = parseJson(datei);
  baum.wert.forEach((eintrag, i) => {
    assert.equal(kodiereJson(profilBaum(VORLAGEN[i])), kodiereJson(eintrag));
  });
});

test("Vorlagen sind nur lesbar: istVorlage, vorbehaltene Ids, Kopie", () => {
  assert.equal(istVorlage("alles"), true);
  assert.equal(istVorlage("vorlage-buchgespraech"), true);
  assert.equal(istVorlage("eigen-abc123def456"), false);
  assert.equal(istVorbehalteneId("vorlage-neu"), true);
  assert.equal(istVorbehalteneId("eigen-abc123def456"), false);
  const kopie = alsEigenenKopieren(
    VORLAGEN[1],
    "eigen-abc123def456",
    "  Mein Buch  ",
  );
  assert.equal(kopie.ok, true);
  assert.equal(kopie.profil.id, "eigen-abc123def456");
  assert.equal(kopie.profil.name, "Mein Buch");
  assert.equal(kopie.profil.anweisungen, VORLAGEN[1].anweisungen);
  assert.equal(VORLAGEN[1].id, "vorlage-buchgespraech");
  kopie.profil.wissen.bereiche.push("neu_x");
  assert.ok(!VORLAGEN[1].wissen.bereiche.includes("neu_x"));
  const vorbehalten = alsEigenenKopieren(VORLAGEN[1], "alles", "X");
  assert.deepEqual(ergebnisFehler(vorbehalten), [{ pfad: "id", code: "wert" }]);
  const schlecht = alsEigenenKopieren(VORLAGEN[1], "Böse", "");
  assert.deepEqual(
    ergebnisFehler(schlecht).map((e) => `${e.pfad}:${e.code}`),
    ["id:muster", "name:laenge"],
  );
});

test("Schema-Datei: gültiges JSON mit allen Feldern des Profils", () => {
  const schema = JSON.parse(lies("assistent.schema.json"));
  assert.equal(schema.$schema, "https://json-schema.org/draft/2020-12/schema");
  assert.deepEqual(Object.keys(schema.properties).sort(), [
    "anweisungen",
    "auswertung",
    "begruessung",
    "beschreibung",
    "id",
    "name",
    "schema",
    "schnittstellen",
    "sprache",
    "stimme",
    "verhalten",
    "wissen",
  ]);
  assert.deepEqual(schema.required.sort(), ["id", "name", "schema"]);
  assert.deepEqual(Object.keys(schema.properties.verhalten.properties).sort(), [
    "nachfrage_stille_s",
    "sprechpause",
    "stil",
    "unterbrechen",
  ]);
  assert.deepEqual(Object.keys(schema.properties.wissen.properties).sort(), [
    "alle_bereiche",
    "bereiche",
    "kontexte",
    "modus",
  ]);
  assert.deepEqual(
    Object.keys(schema.properties.auswertung.properties).sort(),
    ["aufgaben", "felder", "zusammenfassung"],
  );
  assert.equal(schema.properties.schema.const, "lva.assistent/1");
});

test("README nennt Schema, Speicherformat und Auslegungen", () => {
  const readme = lies("README.md");
  for (const stichwort of [
    "lva.assistent/1",
    "Speicherformat",
    "aktiv.json",
    "Auslegung",
  ])
    assert.ok(readme.includes(stichwort), stichwort);
});
