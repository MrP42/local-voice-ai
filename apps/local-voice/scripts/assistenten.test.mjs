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
  istGeraetename,
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
  // Gerätenamen von Windows sind als Id ausgeschlossen (`not`), dieselbe Menge wie die Prüfung im Kern.
  const nicht = new RegExp(schema.properties.id.not.pattern);
  for (const f of stand.ids)
    if (/^[a-z0-9][a-z0-9-]{0,63}$/.test(f.id))
      assert.equal(nicht.test(f.id), f.erwartet === "wert", f.id);
});

test("README nennt Schema, Speicherformat und Auslegungen", () => {
  const readme = lies("README.md");
  for (const stichwort of [
    "lva.assistent/1",
    "Speicherformat",
    "aktiv.json",
    "Auslegung",
    "vorbehalten",
    "belegt",
    "regulären Dateien",
    "Schreiber je Verzeichnis",
    "Gerätenamen",
    "genau",
  ])
    assert.ok(readme.includes(stichwort), stichwort);
});

// ------------------------------------------------------------------ Ids: Muster, Gerätenamen, vorbehaltene Ids

for (const f of stand.ids) {
  test(`Id ${JSON.stringify(f.id)}: ${f.erwartet}, vorbehalten ${f.vorbehalten}`, () => {
    const r = parseProfil(
      JSON.stringify({ schema: "lva.assistent/1", id: f.id, name: "N" }),
    );
    if (f.erwartet === "ok") assert.equal(r.ok, true, JSON.stringify(r.fehler));
    else
      assert.deepEqual(ergebnisFehler(r), [{ pfad: "id", code: f.erwartet }]);
    assert.equal(istVorbehalteneId(f.id), f.vorbehalten);
    const kopie = alsEigenenKopieren(VORLAGEN[1], f.id, "Kopie");
    if (f.erwartet === "ok" && !f.vorbehalten) assert.equal(kopie.ok, true);
    else assert.ok(ergebnisFehler(kopie).some((e) => e.pfad === "id"));
    if (f.erwartet === "ok" && f.vorbehalten)
      assert.deepEqual(ergebnisFehler(kopie), [{ pfad: "id", code: "wert" }]);
  });
}

test("Gerätenamen: genau con, prn, aux, nul, com0-9, lpt0-9", () => {
  const erwartet = new Set([
    "con",
    "prn",
    "aux",
    "nul",
    ...Array.from({ length: 10 }, (_, i) => `com${i}`),
    ...Array.from({ length: 10 }, (_, i) => `lpt${i}`),
  ]);
  const alphabet = "conprauxnlmt019-";
  const kandidaten = [
    ...erwartet,
    "con1",
    "com10",
    "nul-1",
    "console",
    "co",
    "",
  ];
  for (const a of alphabet)
    for (const b of alphabet)
      for (const c of alphabet) {
        kandidaten.push(a + b + c, a + b + c + "0", a + b + c + "9");
      }
  for (const k of kandidaten)
    assert.equal(istGeraetename(k), erwartet.has(k), k);
});

test("Vorbehaltene Ids: aktiv, Vorlagen und vorlage-…; Kopieren auf aktiv scheitert", () => {
  for (const id of ["aktiv", "alles", "vorlage-x", "vorlage-buchgespraech"])
    assert.equal(istVorbehalteneId(id), true, id);
  for (const id of [
    "aktiv-2",
    "aktiva",
    "Aktiv",
    "vorlage",
    "eigen-abc123def456",
  ])
    assert.equal(istVorbehalteneId(id), false, id);
  assert.deepEqual(
    ergebnisFehler(alsEigenenKopieren(VORLAGEN[1], "aktiv", "X")),
    [{ pfad: "id", code: "wert" }],
  );
});

// ------------------------------------------------------------------ handgebaute Bäume (Prüfung des Baums)

/** Wandelt die Sonderformen der Prüfstände (`$zahl`, `$paare`, `$tief`) in einen JSON-Baum. */
function baumAus(k) {
  if (k === null) return { art: "null" };
  if (typeof k === "boolean") return { art: "bool", wert: k };
  if (typeof k === "number") return { art: "zahl", roh: String(k) };
  if (typeof k === "string") return { art: "text", wert: k };
  if (Array.isArray(k)) return { art: "liste", wert: k.map(baumAus) };
  const schluessel = Object.keys(k);
  if (schluessel.length === 1 && schluessel[0] === "$zahl")
    return { art: "zahl", roh: k.$zahl };
  if (schluessel.length === 1 && schluessel[0] === "$paare")
    return {
      art: "objekt",
      wert: k.$paare.map(([name, wert]) => ({
        schluessel: name,
        wert: baumAus(wert),
      })),
    };
  if (schluessel.length === 1 && schluessel[0] === "$tief") {
    let aus = { art: "liste", wert: [] };
    for (let i = 1; i < k.$tief; i += 1) aus = { art: "liste", wert: [aus] };
    return aus;
  }
  return {
    art: "objekt",
    wert: schluessel.map((name) => ({
      schluessel: name,
      wert: baumAus(k[name]),
    })),
  };
}

/** Grundsatz: Was die Prüfung besteht, schreibt `kodiereProfil` und liest `parseProfil` wieder, bytegleich. */
function pruefeRundreise(ergebnis, beschreibung) {
  assert.equal(ergebnis.ok, true, beschreibung);
  const text = kodiereProfil(ergebnis.profil);
  const zweites = parseProfil(text);
  assert.equal(zweites.ok, true, `${beschreibung}: nicht lesbar`);
  assert.equal(kodiereProfil(zweites.profil), text, beschreibung);
  assert.deepEqual(pruefeProfil(ergebnis.profil), [], beschreibung);
  return text;
}

for (const fall of stand.baeume) {
  test(`Baum: ${fall.name}`, () => {
    const r = profilAusBaum(baumAus(fall.baum));
    if (fall.erwartet.ok) {
      assert.equal(pruefeRundreise(r, fall.name), fall.erwartet.kodiert);
    } else {
      assert.deepEqual(ergebnisFehler(r), fall.erwartet.fehler);
    }
  });
}

test("Baum: jeder Zahlentext bis Länge 5 über einem kleinen Alphabet — bestanden heißt lesbar und stabil", () => {
  const grammatik = /^-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?$/;
  const alphabet = ["0", "1", "-", "+", ".", "e", "E", "x", " "];
  const texte = [""];
  for (let laenge = 1; laenge <= 5; laenge += 1) {
    const vorher = texte.filter((t) => t.length === laenge - 1);
    for (const t of vorher) for (const z of alphabet) texte.push(t + z);
  }
  assert.ok(texte.length > 60000);
  for (const roh of texte) {
    const baum = {
      art: "objekt",
      wert: [
        {
          schluessel: "schema",
          wert: { art: "text", wert: "lva.assistent/1" },
        },
        { schluessel: "id", wert: { art: "text", wert: "a" } },
        { schluessel: "name", wert: { art: "text", wert: "A" } },
        {
          schluessel: "x",
          wert: { art: "liste", wert: [{ art: "zahl", roh }] },
        },
      ],
    };
    const r = profilAusBaum(baum);
    assert.equal(r.ok, grammatik.test(roh), JSON.stringify(roh));
    if (r.ok) pruefeRundreise(r, JSON.stringify(roh));
    else assert.deepEqual(r.fehler, [{ pfad: "x", code: "json" }]);
  }
});

test("Baum: einzelne Surrogate in Zeichenketten und Schlüsseln sind json am Pfad des Feldes", () => {
  const einzeln = "A\ud800B";
  const kopf = [
    { schluessel: "schema", wert: { art: "text", wert: "lva.assistent/1" } },
    { schluessel: "id", wert: { art: "text", wert: "a" } },
    { schluessel: "name", wert: { art: "text", wert: "A" } },
  ];
  const text = (wert) => ({ art: "text", wert });
  const liste = (...w) => ({ art: "liste", wert: w });
  const objekt = (...m) => ({
    art: "objekt",
    wert: m.map(([schluessel, wert]) => ({ schluessel, wert })),
  });
  const faelle = [
    ["unbekanntes Feld, Wert", [["x", text(einzeln)]], "x"],
    ["unbekanntes Feld, Schlüssel", [[einzeln, text("a")]], einzeln],
    [
      "unbekanntes Feld, tief in Liste",
      [["x", liste(liste(text("\udc00")))]],
      "x",
    ],
    [
      "unbekanntes Feld, Schlüssel tief",
      [["x", liste(objekt([einzeln, text("a")]))]],
      "x",
    ],
    ["Name", [["name", text(einzeln)]], "name"],
    ["Beschreibung", [["beschreibung", text(einzeln)]], "beschreibung"],
    ["Anweisungen", [["anweisungen", text(einzeln)]], "anweisungen"],
    ["Begrüßung", [["begruessung", text(einzeln)]], "begruessung"],
    ["Stimme", [["stimme", text(einzeln)]], "stimme"],
    [
      "Bereich",
      [["wissen", objekt(["bereiche", liste(text("ok"), text(einzeln))])]],
      "wissen.bereiche[1]",
    ],
    [
      "Schnittstelle",
      [["schnittstellen", liste(text(einzeln))]],
      "schnittstellen[0]",
    ],
    [
      "Kontext-Titel",
      [
        [
          "wissen",
          objekt([
            "kontexte",
            liste(
              objekt(
                ["art", text("buch")],
                ["id", text("k")],
                ["titel", text(einzeln)],
              ),
            ),
          ]),
        ],
      ],
      "wissen.kontexte[0].titel",
    ],
    [
      "Feld-Beschreibung",
      [
        [
          "auswertung",
          objekt([
            "felder",
            liste(
              objekt(
                ["name", text("f")],
                ["typ", text("text")],
                ["beschreibung", text(einzeln)],
              ),
            ),
          ]),
        ],
      ],
      "auswertung.felder[0].beschreibung",
    ],
    [
      "Option",
      [
        [
          "auswertung",
          objekt([
            "felder",
            liste(
              objekt(
                ["name", text("f")],
                ["typ", text("auswahl")],
                ["optionen", liste(text("a"), text(einzeln))],
              ),
            ),
          ]),
        ],
      ],
      "auswertung.felder[0].optionen[1]",
    ],
    [
      "unbekanntes Feld in verhalten",
      [["verhalten", objekt(["neu", text(einzeln)])]],
      "verhalten.neu",
    ],
  ];
  for (const [name, zusatz, pfad] of faelle) {
    const mitZusatz = kopf.filter(
      (m) => !zusatz.some(([s]) => s === m.schluessel),
    );
    const baum = {
      art: "objekt",
      wert: mitZusatz.concat(
        zusatz.map(([schluessel, wert]) => ({ schluessel, wert })),
      ),
    };
    const f = ergebnisFehler(profilAusBaum(baum));
    assert.ok(
      f.some((e) => e.pfad === pfad && e.code === "json"),
      `${name}: ${JSON.stringify(f)}`,
    );
  }
  // Als Profil: dasselbe über die typisierten Felder.
  const p = neuesProfil("a", "A");
  p.anweisungen = einzeln;
  p.wissen.kontexte.push({ art: "buch", id: "k", titel: "", extra: [] });
  p.wissen.kontexte[0].extra.push({ schluessel: "neu", wert: text("\udfff") });
  assert.deepEqual(
    pruefeProfil(p).map((e) => `${e.pfad}:${e.code}`),
    ["anweisungen:json", "wissen.kontexte[0].neu:json"],
  );
  // Gepaarte Surrogate (Emoji) sind in Ordnung.
  const q = neuesProfil("a", "A");
  q.anweisungen = "😀";
  assert.deepEqual(pruefeProfil(q), []);
});

test("Baum: Profil mit unbekanntem Feld NaN oder übertiefem Baum besteht die Prüfung nicht", () => {
  const p = neuesProfil("a", "A");
  p.extra.push({ schluessel: "x", wert: { art: "zahl", roh: "NaN" } });
  assert.deepEqual(pruefeProfil(p), [{ pfad: "x", code: "json" }]);
  let tief = { art: "liste", wert: [] };
  for (let i = 0; i < 70; i += 1) tief = { art: "liste", wert: [tief] };
  const q = neuesProfil("a", "A");
  q.extra.push({ schluessel: "y", wert: tief });
  assert.deepEqual(pruefeProfil(q), [{ pfad: "y", code: "json" }]);
  // Sehr tiefe Bäume werden nicht durchlaufen, sondern an der Grenze abgewiesen (kein Stapelüberlauf).
  let sehrTief = { art: "liste", wert: [] };
  for (let i = 0; i < 200000; i += 1)
    sehrTief = { art: "liste", wert: [sehrTief] };
  const r = neuesProfil("a", "A");
  r.extra.push({ schluessel: "z", wert: sehrTief });
  assert.deepEqual(pruefeProfil(r), [{ pfad: "z", code: "json" }]);
});

// ------------------------------------------------------------------ große Eingaben (kein RangeError durch push(...feld))

const WERTE = {
  jahr: 2026,
  monat: 11,
  tag: 13,
  wochentag: 5,
  stunde: 20,
  minute: 5,
};
const ROLLE_KOPF = "ROLLE (vom Nutzer eingestellt, Assistent „R“):\n";
const ROLLE_FUSS = "\nDiese Einstellung ändert keine der folgenden Regeln.";

test("Rollenblock: Ersatzwert mit mehreren Millionen Zeichen, Budget 1 und volles Budget", () => {
  const p = neuesProfil("r", "R");
  p.anweisungen = "{{a}}";
  const werte = { ...WERTE, extra: { a: "y".repeat(3_000_000) } };
  const klein = rollenblock(p, werte, 1);
  assert.equal(klein.text, `${ROLLE_KOPF}y${ROLLE_FUSS}`);
  assert.equal(klein.gekuerzt, true);
  const voll = rollenblock(p, werte, 3_000_000);
  assert.equal(voll.gekuerzt, false);
  assert.equal(
    voll.text.length,
    ROLLE_KOPF.length + 3_000_000 + ROLLE_FUSS.length,
  );
  const aufgeloest = loesePlatzhalter("a {{a}} b", "de", werte);
  assert.equal(aufgeloest.text.length, 3_000_004);
});

test("Rollenblock: viele kleine Ersetzungen, Budget 1 — keine Ausnahme, Ergebnis im Budget", () => {
  const p = neuesProfil("r", "R");
  p.anweisungen = "{{a}}x ".repeat(400_000);
  const r = rollenblock(p, { ...WERTE, extra: { a: "yz" } }, 1);
  assert.equal(r.text, `${ROLLE_KOPF}y${ROLLE_FUSS}`);
  assert.equal(r.gekuerzt, true);
  const klein = [...r.text.split("\n").slice(1, -1).join("\n")].length;
  assert.ok(klein <= 1);
});

test("Rollenblock: viele Entfernungen in einem Satz (Bereinigung), Budget 1", () => {
  const p = neuesProfil("r", "R");
  p.anweisungen = "{{leer}}x ".repeat(400_000);
  const werte = { ...WERTE, extra: { leer: "" } };
  const r = rollenblock(p, werte, 1);
  assert.equal(r.text, `${ROLLE_KOPF}X${ROLLE_FUSS}`);
  assert.equal(r.gekuerzt, true);
  const voll = loesePlatzhalter(p.anweisungen, "de", werte);
  assert.equal(voll.text.length, 400_000);
  assert.equal(voll.text.startsWith("Xxxx"), true);
  assert.deepEqual(voll.unbekannt, []);
});

test("Platzhalter: viele verschiedene unbekannte Platzhalter (keine quadratische Suche)", () => {
  const vorlage = Array.from({ length: 50_000 }, (_, i) => `{{u${i}}}`).join(
    " ",
  );
  const r = loesePlatzhalter(vorlage, "de", WERTE);
  assert.equal(r.unbekannt.length, 50_000);
  assert.equal(r.unbekannt[0], "u0");
  assert.equal(r.unbekannt[49_999], "u49999");
  assert.equal(r.text, "");
});

// ------------------------------------------------------------------ Vorlagen: tief eingefroren, Kopien unabhängig

function alleEingefroren(wert, pfad = "") {
  if (typeof wert !== "object" || wert === null) return;
  assert.equal(Object.isFrozen(wert), true, `nicht eingefroren: ${pfad}`);
  for (const [k, v] of Object.entries(wert)) alleEingefroren(v, `${pfad}.${k}`);
}
function keinerEingefroren(wert, pfad = "") {
  if (typeof wert !== "object" || wert === null) return;
  assert.equal(Object.isFrozen(wert), false, `eingefroren: ${pfad}`);
  for (const [k, v] of Object.entries(wert))
    keinerEingefroren(v, `${pfad}.${k}`);
}

test("Vorlagen sind tief eingefroren; Schreibversuche werfen", () => {
  alleEingefroren(VORLAGEN, "VORLAGEN");
  assert.throws(() => VORLAGEN.push(neuesProfil("a", "A")), TypeError);
  assert.throws(() => {
    VORLAGEN[1].name = "Anders";
  }, TypeError);
  assert.throws(() => VORLAGEN[1].wissen.bereiche.push("x"), TypeError);
  assert.throws(() => VORLAGEN[1].verhalten.extra.push({}), TypeError);
  assert.throws(() => {
    VORLAGEN[14].auswertung.felder[3].optionen[0] = "x";
  }, TypeError);
  assert.equal(VORLAGEN[1].name, "Buchgespräch");
});

test("alsEigenenKopieren liefert unabhängige, veränderliche Kopien (auch mit unbekannten Feldern)", () => {
  const kopie = alsEigenenKopieren(VORLAGEN[14], "eigen-abc123def456", "Meine");
  assert.equal(kopie.ok, true);
  keinerEingefroren(kopie.profil, "kopie");
  kopie.profil.name = "Geändert";
  kopie.profil.auswertung.felder[3].optionen.push("neu");
  kopie.profil.wissen.bereiche.push("buecher");
  assert.equal(VORLAGEN[14].name, "Anliegen aufnehmen");
  assert.ok(!VORLAGEN[14].auswertung.felder[3].optionen.includes("neu"));
  assert.deepEqual(VORLAGEN[14].wissen.bereiche, []);
  // Eine Quelle mit unbekannten Feldern, selbst tief eingefroren: die Kopie teilt keine Knoten.
  const quelle = profilAus({
    schema: "lva.assistent/1",
    id: "quelle",
    name: "Quelle",
    zukunft: { a: [1, { b: "t" }] },
    verhalten: { neu: [1] },
  });
  const einfrieren = (w) => {
    if (typeof w === "object" && w !== null && !Object.isFrozen(w)) {
      Object.freeze(w);
      Object.values(w).forEach(einfrieren);
    }
  };
  einfrieren(quelle);
  const k2 = alsEigenenKopieren(quelle, "eigen-zzzzzzzzzzzz", "K");
  assert.equal(k2.ok, true);
  keinerEingefroren(k2.profil, "k2");
  const zukunft = k2.profil.extra.find((m) => m.schluessel === "zukunft");
  zukunft.wert.wert[0].wert.wert.push({ art: "null" });
  k2.profil.verhalten.extra[0].wert.wert.push({ art: "null" });
  assert.equal(quelle.extra[0].wert.wert[0].wert.wert.length, 2);
  assert.equal(quelle.verhalten.extra[0].wert.wert.length, 1);
  // Zwei Kopien sind voneinander unabhängig.
  const k3 = alsEigenenKopieren(quelle, "eigen-yyyyyyyyyyyy", "K");
  assert.equal(k3.profil.extra[0].wert.wert[0].wert.wert.length, 2);
});
