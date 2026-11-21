// G2 Teil 2 (lva#556): eigene Assistenten verwalten. Reine Logik (Ids, Kopieren, Umbenennen, Export, Import), die Schreibvorgänge
// über den Zugriff (Anlegen, Überschreiben, Löschen, mit den Fehlerfällen), die serialisierten Schreibvorgänge des Stores und
// die Logik des Editors (Zähler, Fehler je Feld, Bereiche, Abgleich mit einer geänderten Datei). Das Verzeichnis ist eine
// Attrappe mit den Regeln von Rust (`voice_call/assistenten.rs`): Vergleich mit dem erwarteten Text, `occupied`, 200 Dateien.
// Die Ausspracheregeln im Editor stehen in `assistent-aussprache.test.mjs`.
// Lauf: node --test scripts/assistent-verwalten.test.mjs (Node >= 22.6).
import assert from "node:assert/strict";
import * as modul from "node:module";
import test from "node:test";

modul.register("./test-stubs/hooks.mjs", import.meta.url);
globalThis.window = { addEventListener() {}, removeEventListener() {} };
const tauri = (globalThis.__tauri = {
  calls: [],
  handlers: {},
  listeners: [],
  prepare: undefined,
});

const BASIS = "../src/components/voice-call/";
const kern = await import(`${BASIS}assistenten/index.ts`);
const speicher = await import(`${BASIS}profil/speicher.ts`);
const verwalten = await import(`${BASIS}profil/verwalten.ts`);
const verwaltung = await import(`${BASIS}profil/verwaltung.ts`);
const editor = await import(`${BASIS}profil/editor.ts`);
const { tauriZugriff } = await import(`${BASIS}profil/zugriff.ts`);
const { useAssistentStore } = await import("../src/stores/assistentStore.ts");

const {
  VORLAGEN,
  ALLES_ID,
  vorlage,
  kodiereProfil,
  parseProfil,
  alsEigenenKopieren,
  neuesProfil,
  pruefeProfil,
} = kern;

const echtesTimeout = globalThis.setTimeout;
const settle = (ms = 5) => new Promise((r) => echtesTimeout(r, ms));

// ===================================================================== Attrappe des Verzeichnisses

const MAX_DATEIEN = 200;
const MAX_BYTES = 256 * 1024;
const vorbehalten = (id) =>
  id === "alles" || id === "aktiv" || id.startsWith("vorlage-");

/**
 * Das Verzeichnis mit den Regeln von Rust. Als `Zugriff` (`s.zugriff`) und als Befehle (`tauri.handlers`) zugleich.
 * `s.vorSpeichern(id)`: Haken vor dem Prüfen (ein anderer Zugang schreibt in diesem Moment). `s.fehler.<befehl>`: wirft.
 */
function rust({
  dateien = {},
  aktiv = null,
  belegt = false,
  ohneGrossKlein = false,
} = {}) {
  const s = {
    dateien: new Map(Object.entries(dateien)),
    aktiv,
    belegt,
    log: [],
    fehler: {},
    vorSpeichern: null,
    verzoegerung: 0,
  };
  const name = (id) => `${id}.json`;
  const finde = (n) =>
    ohneGrossKlein
      ? [...s.dateien.keys()].find((k) => k.toLowerCase() === n.toLowerCase())
      : s.dateien.has(n)
        ? n
        : undefined;
  const wirf = (befehl) => {
    if (s.fehler[befehl]) throw s.fehler[befehl];
  };
  const warte = () => (s.verzoegerung ? settle(s.verzoegerung) : null);
  s.zugriff = {
    async liste() {
      s.log.push("liste");
      wirf("liste");
      await warte();
      return {
        dateien: [...s.dateien].map(([n, wert]) =>
          typeof wert === "string"
            ? { name: n, inhalt: wert, abgewiesen: null }
            : { name: n, inhalt: null, abgewiesen: wert.abgewiesen },
        ),
        aktivId: s.aktiv,
        aktivBelegt: s.belegt,
      };
    },
    async speichern(id, inhalt, ersetzt) {
      s.log.push(`save:${id}`);
      await warte();
      s.vorSpeichern?.(id);
      wirf("speichern");
      if (vorbehalten(id)) throw new speicher.SpeicherFehler("reserved_id");
      if (new TextEncoder().encode(inhalt).length > MAX_BYTES)
        throw new speicher.SpeicherFehler("too_large");
      const vorhanden = finde(name(id));
      if (vorhanden === undefined) {
        if (ersetzt !== null) throw new speicher.SpeicherFehler("not_found");
        if (s.dateien.size >= MAX_DATEIEN)
          throw new speicher.SpeicherFehler("too_many");
      } else {
        if (ersetzt === null) throw new speicher.SpeicherFehler("occupied");
        if (s.dateien.get(vorhanden) !== ersetzt)
          throw new speicher.SpeicherFehler("occupied");
      }
      s.dateien.set(vorhanden ?? name(id), inhalt);
    },
    async loeschen(id, erwartet) {
      s.log.push(`delete:${id}`);
      await warte();
      wirf("loeschen");
      const vorhanden = finde(name(id));
      if (vorhanden === undefined)
        throw new speicher.SpeicherFehler("not_found");
      if (s.dateien.get(vorhanden) !== erwartet)
        throw new speicher.SpeicherFehler("occupied");
      s.dateien.delete(vorhanden);
    },
    async aktivSetzen(id) {
      s.log.push(`aktiv:${id}`);
      await warte();
      wirf("aktivSetzen");
      if (s.belegt) throw new speicher.SpeicherFehler("occupied");
      if (!vorbehalten(id) && finde(name(id)) === undefined)
        throw new speicher.SpeicherFehler("not_found");
      s.aktiv = id;
    },
  };
  // Die Befehle von Tauri: Fehler als Zeichenkette (so kommen sie in `invoke` an).
  const alsBefehl = (fn) => async (args) => {
    try {
      return await fn(args);
    } catch (e) {
      throw e instanceof speicher.SpeicherFehler ? e.code : e;
    }
  };
  tauri.handlers.voice_assistants_list = alsBefehl(async () => {
    const l = await s.zugriff.liste();
    return {
      dateien: l.dateien,
      aktiv_id: l.aktivId,
      aktiv_belegt: l.aktivBelegt,
    };
  });
  tauri.handlers.voice_assistant_save = alsBefehl(({ id, inhalt, ersetzt }) =>
    s.zugriff.speichern(id, inhalt, ersetzt),
  );
  tauri.handlers.voice_assistant_delete = alsBefehl(({ id, erwartet }) =>
    s.zugriff.loeschen(id, erwartet),
  );
  tauri.handlers.voice_assistant_active_set = alsBefehl(({ id }) =>
    s.zugriff.aktivSetzen(id),
  );
  return s;
}

/** Ein gültiger eigener Assistent als Text (kanonisch). */
function eigenerText(id, aenderung = (p) => p) {
  const kopie = alsEigenenKopieren(
    vorlage("vorlage-lernpartner"),
    id,
    `Eigen ${id}`,
  );
  assert.equal(kopie.ok, true);
  return kodiereProfil(aenderung(kopie.profil));
}

const lade = (s) => speicher.ladeAssistenten(s.zugriff);
const profilVon = (text) => {
  const e = parseProfil(text);
  assert.equal(e.ok, true, JSON.stringify(e.fehler));
  return e.profil;
};

// ===================================================================== Ids

test("Ids: Name wird zu einer gültigen Id (Umlaute, Akzente, Sonderzeichen, Länge, leer)", () => {
  const { idAusName } = verwalten;
  assert.equal(idAusName("Mein Coach"), "mein-coach");
  assert.equal(
    idAusName("Über Größe: Straße & Fähre"),
    "ueber-groesse-strasse-faehre",
  );
  assert.equal(idAusName("Café Crème"), "cafe-creme");
  assert.equal(idAusName("  --Hallo--  "), "hallo");
  assert.equal(idAusName("日本語"), "assistent");
  assert.equal(idAusName(""), "assistent");
  assert.equal(idAusName("***"), "assistent");
  const lang = idAusName(`${"a".repeat(63)} b ${"c".repeat(20)}`);
  assert.ok(lang.length <= 64);
  assert.ok(!lang.endsWith("-"));
  for (const n of [
    "Mein Coach",
    "Über",
    "x".repeat(300),
    "a b c",
    "9 Uhr",
    "ÄÖÜ ß",
  ]) {
    assert.equal(
      kern.pruefeProfil(neuesProfil(verwalten.idAusName(n), "x")).length,
      0,
      n,
    );
  }
});

test("Ids: freie Id vermeidet Belegte, Vorbehaltene und Gerätenamen und bleibt gültig", () => {
  const { freieId } = verwalten;
  const leer = new Set();
  assert.equal(freieId("coach", leer), "coach");
  assert.equal(freieId("coach", new Set(["coach"])), "coach-2");
  assert.equal(freieId("coach", new Set(["coach", "coach-2"])), "coach-3");
  // Groß- und Kleinschreibung zählt nicht (Windows und macOS): belegt ist belegt.
  assert.equal(freieId("Coach", new Set(["coach"])), "Coach-2");
  assert.equal(freieId("alles", leer), "eigen-alles");
  assert.equal(freieId("aktiv", leer), "eigen-aktiv");
  assert.equal(
    freieId("vorlage-lernpartner", leer),
    "eigen-vorlage-lernpartner",
  );
  assert.equal(freieId("con", leer), "eigen-con");
  assert.equal(freieId("com1", leer), "eigen-com1");
  assert.equal(freieId("", leer), "assistent");
  // 64 Zeichen: die Endung kürzt die Basis, die Id bleibt gültig.
  const lang = "a".repeat(64);
  const id = freieId(lang, new Set([lang]));
  assert.equal(id.length, 64);
  assert.ok(id.endsWith("-2"));
  assert.equal(kern.pruefeProfil(neuesProfil(id, "x")).length, 0);
  // Auch viele Belegte enden bei einer freien Id.
  const viele = new Set([
    "x",
    ...Array.from({ length: 50 }, (_, i) => `x-${i + 2}`),
  ]);
  assert.equal(freieId("x", viele), "x-52");
});

test("Ids: belegt sind eigene, abgewiesene Dateien (ohne Endung, klein) sowie aktiv und alles", async () => {
  const s = rust({
    dateien: {
      "eigen-a.json": eigenerText("eigen-a"),
      "Kaputt.JSON": "{ kein json",
      ohneendung: "x",
    },
  });
  const a = await lade(s);
  const belegt = verwalten.belegteIds(a);
  for (const id of ["eigen-a", "kaputt", "ohneendung", "aktiv", "alles"])
    assert.ok(belegt.has(id), id);
  assert.ok(!belegt.has("frei"));
});

test("Ids: Platz für höchstens 200 Dateien, abgewiesene zählen mit", async () => {
  const dateien = {};
  for (let i = 0; i < 199; i++) dateien[`kaputt-${i}.json`] = "{";
  const s = rust({ dateien });
  const a = await lade(s);
  assert.equal(a.abgewiesen.length, 199);
  assert.equal(verwalten.hatPlatz(a), true);
  s.dateien.set("noch-eine.json", "{");
  assert.equal(verwalten.hatPlatz(await lade(s)), false);
});

// ===================================================================== Anlegen, Kopieren, Umbenennen

test("Anlegen: neuer leerer Assistent mit freier Id, geprüft", async () => {
  const s = rust({ dateien: { "mein-coach.json": eigenerText("mein-coach") } });
  const a = await lade(s);
  const e = verwalten.neuAnlegen("  Mein Coach  ", a);
  assert.equal(e.ok, true);
  assert.equal(e.profil.id, "mein-coach-2");
  assert.equal(e.profil.name, "Mein Coach");
  assert.equal(e.profil.anweisungen, "");
  assert.equal(pruefeProfil(e.profil).length, 0);
  for (const schlecht of ["", "   ", "x".repeat(61)]) {
    const f = verwalten.neuAnlegen(schlecht, a);
    assert.equal(f.ok, false, JSON.stringify(schlecht));
    assert.deepEqual(f.fehler, [{ pfad: "name", code: "laenge" }]);
  }
  // 60 Zeichen sind erlaubt, mit Umlauten und Astralzeichen nach Unicode-Zeichen gezählt.
  assert.equal(verwalten.neuAnlegen("ä".repeat(60), a).ok, true);
  assert.equal(verwalten.neuAnlegen("😀".repeat(60), a).ok, true);
  assert.equal(verwalten.neuAnlegen("😀".repeat(61), a).ok, false);
  // Steuerzeichen im Namen lehnt der Kern ab.
  assert.equal(verwalten.neuAnlegen("a\u0007b", a).ok, false);
});

test("Kopieren: Vorlage und eigener Assistent, Kopie teilt nichts, unbekannte Felder bleiben", async () => {
  const text = eigenerText("quelle", (p) => ({
    ...p,
    extra: [{ schluessel: "zukunft", wert: { art: "zahl", roh: "1.50" } }],
    verhalten: {
      ...p.verhalten,
      extra: [{ schluessel: "neu", wert: { art: "bool", wert: true } }],
    },
  }));
  const s = rust({ dateien: { "quelle.json": text } });
  const a = await lade(s);
  const quelle = a.eigene[0].profil;
  const k = verwalten.kopieren(quelle, "Meine Kopie", a);
  assert.equal(k.ok, true);
  assert.equal(k.profil.id, "meine-kopie");
  assert.equal(k.profil.name, "Meine Kopie");
  assert.equal(kodiereProfil(k.profil).includes('"zukunft": 1.50'), true);
  assert.equal(kodiereProfil(k.profil).includes('"neu": true'), true);
  k.profil.wissen.bereiche.push("kalender");
  k.profil.verhalten.extra.push({ schluessel: "x", wert: { art: "null" } });
  assert.ok(!quelle.wissen.bereiche.includes("kalender"));
  assert.equal(quelle.verhalten.extra.length, 1);
  // Aus einer Vorlage: eigene Id, nie die der Vorlage.
  const v = vorlage("vorlage-lernpartner");
  const kv = verwalten.kopieren(v, v.name, a);
  assert.equal(kv.ok, true);
  assert.ok(!kv.profil.id.startsWith("vorlage-"));
  assert.equal(speicher.profilZu({ ...a }, "vorlage-lernpartner"), v);
  // Die Vorlage bleibt unverändert (eingefroren).
  assert.equal(Object.isFrozen(v), true);
});

test("Umbenennen: nur der Name ändert sich, die Id bleibt, ungültige Namen werden abgewiesen", async () => {
  const s = rust({ dateien: { "eigen-a.json": eigenerText("eigen-a") } });
  const a = await lade(s);
  const p = a.eigene[0].profil;
  const e = verwalten.umbenennen(p, "  Neuer Name ");
  assert.equal(e.ok, true);
  assert.equal(e.profil.id, "eigen-a");
  assert.equal(e.profil.name, "Neuer Name");
  assert.equal(e.profil.anweisungen, p.anweisungen);
  assert.equal(verwalten.umbenennen(p, "").ok, false);
  assert.equal(verwalten.umbenennen(p, "x".repeat(61)).ok, false);
  assert.equal(p.name, "Eigen eigen-a");
});

// ===================================================================== Export und Import

test("Export: gleiche Bytes wie der Kern, Dateiname ist die Id, Rundlauf ohne Verlust", async () => {
  const text = eigenerText("eigen-a", (p) => ({
    ...p,
    anweisungen: "Du bist {{name}}s Coach.\nZeile 2 mit „Umlauten“ äöü 😀.",
    extra: [{ schluessel: "zukunft", wert: { art: "objekt", wert: [] } }],
  }));
  const s = rust({ dateien: { "eigen-a.json": text } });
  const a = await lade(s);
  const aus = verwalten.exportieren(a.eigene[0].profil);
  assert.equal(aus.dateiname, "eigen-a.json");
  assert.equal(aus.text, kodiereProfil(a.eigene[0].profil));
  assert.equal(aus.text, text);
  assert.ok(aus.text.endsWith("\n"));
  // Wieder einlesen (in ein leeres Verzeichnis): gleiches Profil, gleiche Id, gleiche Bytes.
  const leer = await lade(rust());
  const ein = verwalten.importieren(aus.text, leer);
  assert.equal(ein.ok, true);
  assert.equal(ein.idGeaendert, false);
  assert.equal(kodiereProfil(ein.profil), aus.text);
});

test("Export: auch eine Vorlage lässt sich exportieren; beim Import bekommt sie eine eigene Id aus dem Namen", async () => {
  const v = vorlage("vorlage-lernpartner");
  const aus = verwalten.exportieren(v);
  assert.equal(aus.dateiname, "vorlage-lernpartner.json");
  const a = await lade(rust());
  const ein = verwalten.importieren(aus.text, a);
  assert.equal(ein.ok, true);
  assert.equal(ein.idGeaendert, true);
  assert.equal(ein.ursprungsId, "vorlage-lernpartner");
  assert.ok(!ein.profil.id.startsWith("vorlage-"));
  assert.equal(ein.profil.id, verwalten.idAusName(v.name));
  assert.equal(ein.profil.name, v.name);
  assert.equal(ein.profil.anweisungen, v.anweisungen);
});

test("Import: belegte Id bekommt eine neue, überschrieben wird nie", async () => {
  const text = eigenerText("eigen-a");
  const s = rust({ dateien: { "eigen-a.json": text } });
  const a = await lade(s);
  const ein = verwalten.importieren(text, a);
  assert.equal(ein.ok, true);
  assert.equal(ein.idGeaendert, true);
  assert.equal(ein.profil.id, "eigen-a-2");
  assert.equal(ein.ursprungsId, "eigen-a");
  // Gegen den Namen einer abgewiesenen Datei, auch in anderer Schreibweise (Windows und macOS trennen sie nicht).
  const s2 = rust({ dateien: { "Eigen-B.JSON": "{ kaputt" } });
  const a2 = await lade(s2);
  const ein2 = verwalten.importieren(eigenerText("eigen-b"), a2);
  assert.equal(ein2.idGeaendert, true);
  assert.equal(ein2.profil.id, "eigen-b-2");
  // Eine vorbehaltene Id (`aktiv`, `alles`, `vorlage-…`) wird zur Id aus dem Namen; vorhandene Dateien bleiben, wie sie sind.
  const ak = verwalten.importieren(
    text
      .replace('"id": "eigen-a"', '"id": "aktiv"')
      .replace("Eigen eigen-a", "Mein Aktiv"),
    a,
  );
  assert.equal(ak.ok, true);
  assert.equal(ak.idGeaendert, true);
  assert.equal(ak.ursprungsId, "aktiv");
  assert.equal(ak.profil.id, "mein-aktiv");
  assert.equal(s.dateien.size, 1);
  assert.equal(s.dateien.get("eigen-a.json"), text);
});

test("Import: Fehler kommen als Liste, nichts wird repariert", async () => {
  const a = await lade(rust());
  const gut = eigenerText("eigen-a");
  const f = (text) => verwalten.importieren(text, a);
  assert.deepEqual(f("").grund, "leer");
  assert.deepEqual(f("   \n\t ").grund, "leer");
  const gross = f("x".repeat(256 * 1024 + 1));
  assert.equal(gross.ok, false);
  assert.equal(gross.grund, "zu_gross");
  // Exakt 256 KiB wird noch gelesen (dann lehnt der Kern ab, nicht die Größe).
  assert.notEqual(f(" ".repeat(256 * 1024) + "x").grund, undefined);
  const kaputt = f("{ kein json");
  assert.equal(kaputt.grund, "abgewiesen");
  assert.deepEqual(kaputt.fehler, [{ pfad: "", code: "json" }]);
  assert.equal(f("﻿" + gut).grund, "abgewiesen"); // Byte-Order-Marke ist kein JSON
  assert.equal(f("[]").grund, "abgewiesen");
  assert.equal(f("null").grund, "abgewiesen");
  const schema = f(gut.replace("lva.assistent/1", "lva.assistent/2"));
  assert.deepEqual(schema.fehler, [{ pfad: "schema", code: "schema_neuer" }]);
  const lang = f(gut.replace(/"name": "[^"]*"/, `"name": "${"x".repeat(61)}"`));
  assert.deepEqual(lang.fehler, [{ pfad: "name", code: "laenge" }]);
  const mehrere = f(
    gut
      .replace(/"name": "[^"]*"/, '"name": ""')
      .replace(/"sprache": "[^"]*"/, '"sprache": "Deutsch"'),
  );
  assert.deepEqual(
    mehrere.fehler.map((x) => `${x.pfad}:${x.code}`),
    ["name:laenge", "sprache:muster"],
  );
  assert.equal(
    f(gut.replace('"id": "eigen-a"', '"id": "con"')).grund,
    "abgewiesen",
  );
  // Zufälliger Müll wirft nie.
  for (const m of [
    "\u0000",
    "{}",
    '{"schema":1}',
    '"',
    '{"a":',
    "\u{1F600}",
    "{".repeat(100),
  ])
    assert.doesNotThrow(() => f(m), m);
});

test("Import: unbekannte Felder bleiben bis in die Datei erhalten", async () => {
  const gut = eigenerText("eigen-a");
  const mit = gut.replace(
    '"anweisungen"',
    '"zukunft": {"x": [1, 2.50, null]},\n  "anweisungen"',
  );
  const a = await lade(rust());
  const ein = verwalten.importieren(mit, a);
  assert.equal(ein.ok, true);
  const aus = kodiereProfil(ein.profil);
  assert.ok(aus.includes('"zukunft"'));
  assert.ok(aus.includes("2.50"));
});

// ===================================================================== Schreiben über den Zugriff

test("Anlegen schreibt atomar eine neue Datei mit dem Text des Kerns und sonst nichts", async () => {
  const s = rust();
  const a = await lade(s);
  const e = verwalten.neuAnlegen("Mein Coach", a);
  const r = await verwaltung.legeAn(s.zugriff, a, e.profil);
  assert.deepEqual(r, { ok: true });
  assert.deepEqual(s.log.slice(1), ["save:mein-coach"]);
  assert.equal(s.dateien.get("mein-coach.json"), kodiereProfil(e.profil));
  const b = await lade(s);
  assert.equal(b.eigene.length, 1);
  assert.equal(b.aktivId, ALLES_ID);
});

test("Anlegen: nichts wird geschrieben bei Ladefehler, ungültigem Profil, belegter oder vorbehaltener Id, vollem Verzeichnis, zu großer Datei", async () => {
  const gut = neuesProfil("eigen-a", "Eigen");
  // Der Speicher war nicht lesbar: kein Schreiben (die Auswahl und alles weitere wären blind).
  for (const code of ["read_failed", "unsafe_path"]) {
    const s = rust();
    s.fehler.liste = new speicher.SpeicherFehler(code);
    const a = await lade(s);
    assert.equal(a.ladefehler, code);
    s.log.length = 0;
    const r = await verwaltung.legeAn(s.zugriff, a, gut);
    assert.deepEqual([r.ok, r.grund], [false, code]);
    assert.deepEqual(s.log, []);
  }
  // Ungültig (Name leer): Fehlerliste, nichts geschrieben.
  const s = rust({ dateien: { "eigen-b.json": eigenerText("eigen-b") } });
  const a = await lade(s);
  s.log.length = 0;
  const leerName = await verwaltung.legeAn(s.zugriff, a, neuesProfil("x", ""));
  assert.equal(leerName.grund, "ungueltig");
  assert.deepEqual(leerName.fehler, [{ pfad: "name", code: "laenge" }]);
  // Belegt: von einem eigenen Assistenten und von einer abgewiesenen Datei, auch in anderer Schreibweise.
  assert.equal(
    (await verwaltung.legeAn(s.zugriff, a, neuesProfil("eigen-b", "B"))).grund,
    "occupied",
  );
  const sg = rust({ dateien: { "Kaputt.JSON": "{ kaputt" } });
  const ag = await lade(sg);
  sg.log.length = 0;
  assert.equal(
    (await verwaltung.legeAn(sg.zugriff, ag, neuesProfil("kaputt", "K"))).grund,
    "occupied",
  );
  assert.deepEqual(sg.log, []);
  // Vorbehalten: der Kern lehnt es nicht ab, die Prüfung vorher schon.
  const res = await verwaltung.legeAn(s.zugriff, a, neuesProfil("alles", "A"));
  assert.equal(res.ok, false);
  assert.ok(["occupied", "reserved_id"].includes(res.grund));
  assert.deepEqual(s.log, []);
  // Zu groß: Anweisungen sind höchstens 8000 Zeichen, mit Astralzeichen über 256 KiB nicht erreichbar; also über die Prüfung.
  const zuLang = { ...gut, anweisungen: "x".repeat(8001) };
  assert.equal(
    (await verwaltung.legeAn(s.zugriff, a, zuLang)).grund,
    "ungueltig",
  );
  // Verzeichnis voll.
  const voll = rust({
    dateien: Object.fromEntries(
      Array.from({ length: 200 }, (_, i) => [`k-${i}.json`, "{"]),
    ),
  });
  const av = await lade(voll);
  voll.log.length = 0;
  assert.equal(
    (await verwaltung.legeAn(voll.zugriff, av, gut)).grund,
    "too_many",
  );
  assert.deepEqual(voll.log, []);
});

test("Anlegen: Schreibfehler (Datenträger voll) und Absturz des Aufrufs werden gemeldet, die Liste bleibt unverändert", async () => {
  const s = rust({ dateien: { "eigen-a.json": eigenerText("eigen-a") } });
  const a = await lade(s);
  const vorher = [...s.dateien];
  s.fehler.speichern = new speicher.SpeicherFehler("write_failed");
  const r = await verwaltung.legeAn(s.zugriff, a, neuesProfil("neu", "Neu"));
  assert.deepEqual([r.ok, r.grund], [false, "write_failed"]);
  s.fehler.speichern = new Error("Aufruf abgestürzt");
  const r2 = await verwaltung.legeAn(s.zugriff, a, neuesProfil("neu", "Neu"));
  assert.deepEqual([r2.ok, r2.grund], [false, "write_failed"]);
  assert.deepEqual([...s.dateien], vorher);
});

test("Anlegen mit Wiederholung: Ist die Id im selben Moment belegt, geht es mit der nächsten freien Id weiter", async () => {
  const s = rust();
  const a = await lade(s);
  let belegt = false;
  s.vorSpeichern = (id) => {
    // Ein anderer Zugang legt genau jetzt dieselbe Datei an.
    if (!belegt && id === "coach") {
      belegt = true;
      s.dateien.set("coach.json", eigenerText("coach"));
    }
  };
  const r = await verwaltung.legeNeuAn(
    s.zugriff,
    a,
    () => lade(s),
    (stand) => verwalten.neuAnlegen("Coach", stand),
  );
  assert.equal(r.ok, true);
  assert.equal(r.profil.id, "coach-2");
  assert.ok(s.dateien.has("coach.json"));
  assert.ok(s.dateien.has("coach-2.json"));
  // Die fremde Datei wurde nicht angerührt.
  assert.equal(s.dateien.get("coach.json"), eigenerText("coach"));
});

test("Anlegen mit Wiederholung: bricht nach drei Versuchen ab, ohne zu überschreiben, und meldet occupied", async () => {
  const s = rust();
  const a = await lade(s);
  s.vorSpeichern = (id) => {
    if (!s.dateien.has(`${id}.json`)) s.dateien.set(`${id}.json`, "{ fremd");
  };
  let versuche = 0;
  const r = await verwaltung.legeNeuAn(
    s.zugriff,
    a,
    () => lade(s),
    (stand) => {
      versuche++;
      return verwalten.neuAnlegen("Coach", stand);
    },
  );
  assert.equal(r.ok, false);
  assert.equal(r.grund, "occupied");
  assert.equal(versuche, verwaltung.VERSUCHE);
  assert.ok([...s.dateien.values()].every((t) => t === "{ fremd"));
});

test("Anlegen mit Wiederholung: Abweisung des Erzeugers kommt als Fehlerliste, nichts wird geschrieben", async () => {
  const s = rust();
  const a = await lade(s);
  s.log.length = 0;
  const r = await verwaltung.legeNeuAn(
    s.zugriff,
    a,
    () => lade(s),
    (stand) => verwalten.neuAnlegen("", stand),
  );
  assert.equal(r.ok, false);
  assert.equal(r.grund, "ungueltig");
  assert.deepEqual(r.fehler, [{ pfad: "name", code: "laenge" }]);
  assert.deepEqual(s.log, []);
});

test("Überschreiben: nur mit dem bekannten alten Text; ein Konflikt ändert nichts", async () => {
  const text = eigenerText("eigen-a");
  const s = rust({ dateien: { "eigen-a.json": text } });
  const a = await lade(s);
  const neu = { ...a.eigene[0].profil, anweisungen: "Neue Anweisung." };
  const r = await verwaltung.ueberschreibe(s.zugriff, a, "eigen-a", neu);
  assert.deepEqual(r, { ok: true });
  assert.equal(s.dateien.get("eigen-a.json"), kodiereProfil(neu));
  // Veralteter Stand: die Datei ist inzwischen anders (ein anderer Zugang hat gespeichert).
  const fremd = eigenerText("eigen-a", (p) => ({
    ...p,
    beschreibung: "fremd",
  }));
  s.dateien.set("eigen-a.json", fremd);
  const r2 = await verwaltung.ueberschreibe(s.zugriff, a, "eigen-a", {
    ...neu,
    anweisungen: "Noch eine.",
  });
  assert.deepEqual([r2.ok, r2.grund], [false, "occupied"]);
  assert.equal(s.dateien.get("eigen-a.json"), fremd);
  // Datei inzwischen gelöscht: not_found, nichts wird neu angelegt.
  s.dateien.delete("eigen-a.json");
  const r3 = await verwaltung.ueberschreibe(s.zugriff, a, "eigen-a", neu);
  assert.deepEqual([r3.ok, r3.grund], [false, "not_found"]);
  assert.equal(s.dateien.size, 0);
});

test("Überschreiben: unveränderter Entwurf schreibt nichts, ungültiger wird abgewiesen, die Id ist fest", async () => {
  const text = eigenerText("eigen-a");
  const s = rust({ dateien: { "eigen-a.json": text } });
  const a = await lade(s);
  s.log.length = 0;
  const p = a.eigene[0].profil;
  assert.deepEqual(await verwaltung.ueberschreibe(s.zugriff, a, "eigen-a", p), {
    ok: true,
  });
  assert.deepEqual(s.log, []);
  const schlecht = await verwaltung.ueberschreibe(s.zugriff, a, "eigen-a", {
    ...p,
    name: "",
  });
  assert.equal(schlecht.grund, "ungueltig");
  const andereId = await verwaltung.ueberschreibe(s.zugriff, a, "eigen-a", {
    ...p,
    id: "eigen-b",
  });
  assert.equal(andereId.grund, "ungueltig");
  const unbekannt = await verwaltung.ueberschreibe(
    s.zugriff,
    a,
    "gibts-nicht",
    p,
  );
  assert.equal(unbekannt.grund, "not_found");
  assert.deepEqual(s.log, []);
  // Ein Speicher, der nicht lesbar war, bekommt nichts geschrieben.
  const blind = { ...a, ladefehler: "read_failed" };
  assert.equal(
    (
      await verwaltung.ueberschreibe(s.zugriff, blind, "eigen-a", {
        ...p,
        beschreibung: "x",
      })
    ).grund,
    "read_failed",
  );
  assert.deepEqual(s.log, []);
});

test("Überschreiben: unbekannte Felder und alles, was der Editor nicht zeigt, bleiben beim Speichern erhalten", async () => {
  const text = eigenerText("eigen-a", (p) => ({
    ...p,
    extra: [{ schluessel: "zukunft", wert: { art: "text", wert: "bleibt" } }],
    verhalten: {
      ...p.verhalten,
      extra: [{ schluessel: "ton", wert: { art: "zahl", roh: "0.50" } }],
    },
    wissen: {
      ...p.wissen,
      kontexte: [{ art: "buch", id: "b-1", titel: "Ein Buch", extra: [] }],
      modus: "zuerst",
      extra: [{ schluessel: "reserve", wert: { art: "liste", wert: [] } }],
    },
    schnittstellen: ["kalender.lesen"],
    auswertung: { ...p.auswertung, zusammenfassung: true },
  }));
  const s = rust({ dateien: { "eigen-a.json": text } });
  const a = await lade(s);
  let entwurf = a.eigene[0].profil;
  entwurf = editor.mit(entwurf, {
    beschreibung: "Geändert",
    anweisungen: "Du hilfst.",
  });
  entwurf = editor.mitVerhalten(entwurf, {
    sprechpause: "lang",
    unterbrechen: false,
  });
  entwurf = editor.bereichSchalten(entwurf, "kalender", true);
  const r = await verwaltung.ueberschreibe(s.zugriff, a, "eigen-a", entwurf);
  assert.equal(r.ok, true);
  const gelesen = profilVon(s.dateien.get("eigen-a.json"));
  assert.equal(gelesen.beschreibung, "Geändert");
  assert.equal(gelesen.verhalten.sprechpause, "lang");
  assert.equal(gelesen.verhalten.unterbrechen, false);
  assert.ok(gelesen.wissen.bereiche.includes("kalender"));
  const t = s.dateien.get("eigen-a.json");
  assert.ok(t.includes('"zukunft": "bleibt"'));
  assert.ok(t.includes('"ton": 0.50'));
  assert.ok(t.includes('"reserve": []'));
  assert.ok(t.includes('"kalender.lesen"'));
  assert.ok(t.includes('"id": "b-1"'));
  assert.equal(gelesen.wissen.modus, "zuerst");
  assert.equal(gelesen.auswertung.zusammenfassung, true);
});

test("Löschen: nur mit dem bekannten Text; der gewählte Assistent geht zuerst auf „Alles“", async () => {
  const text = eigenerText("eigen-a");
  const s = rust({
    dateien: { "eigen-a.json": text, "eigen-b.json": eigenerText("eigen-b") },
    aktiv: "eigen-a",
  });
  const a = await lade(s);
  assert.equal(a.aktivId, "eigen-a");
  s.log.length = 0;
  const r = await verwaltung.loesche(s.zugriff, a, "eigen-a");
  assert.deepEqual(r, { ok: true });
  // Erst die Auswahl, dann die Datei (ein Abbruch dazwischen verliert nichts).
  assert.deepEqual(s.log, ["aktiv:alles", "delete:eigen-a"]);
  assert.equal(s.aktiv, "alles");
  assert.ok(!s.dateien.has("eigen-a.json"));
  const b = await lade(s);
  assert.equal(b.aktivId, ALLES_ID);
  // Ein nicht gewählter Assistent: keine Auswahl anfassen.
  s.log.length = 0;
  assert.deepEqual(await verwaltung.loesche(s.zugriff, b, "eigen-b"), {
    ok: true,
  });
  assert.deepEqual(s.log, ["delete:eigen-b"]);
});

test("Löschen: Fehlerfälle lassen die Datei stehen (Konflikt, Auswahl nicht setzbar, Schreibfehler, Abbruch nach der Auswahl)", async () => {
  const text = eigenerText("eigen-a");
  // Die Datei hat sich geändert: occupied, nichts gelöscht.
  const s = rust({ dateien: { "eigen-a.json": text } });
  const a = await lade(s);
  s.dateien.set("eigen-a.json", text.replace("Eigen", "Fremd"));
  const r = await verwaltung.loesche(s.zugriff, a, "eigen-a");
  assert.deepEqual([r.ok, r.grund], [false, "occupied"]);
  assert.ok(s.dateien.has("eigen-a.json"));
  // Auswahl lässt sich nicht setzen: die Datei bleibt.
  const s2 = rust({ dateien: { "eigen-a.json": text }, aktiv: "eigen-a" });
  const a2 = await lade(s2);
  s2.fehler.aktivSetzen = new speicher.SpeicherFehler("write_failed");
  const r2 = await verwaltung.loesche(s2.zugriff, a2, "eigen-a");
  assert.deepEqual([r2.ok, r2.grund], [false, "write_failed"]);
  assert.ok(s2.dateien.has("eigen-a.json"));
  assert.equal(s2.aktiv, "eigen-a");
  // Löschen scheitert nach der Auswahl (Abbruch dazwischen): der Assistent bleibt, die Auswahl ist „Alles“, nichts geht verloren.
  const s3 = rust({ dateien: { "eigen-a.json": text }, aktiv: "eigen-a" });
  const a3 = await lade(s3);
  s3.fehler.loeschen = new speicher.SpeicherFehler("write_failed");
  const r3 = await verwaltung.loesche(s3.zugriff, a3, "eigen-a");
  assert.equal(r3.grund, "write_failed");
  assert.ok(s3.dateien.has("eigen-a.json"));
  assert.equal((await lade(s3)).aktivId, ALLES_ID);
  // Unbekannte Id, Ladefehler.
  assert.equal(
    (await verwaltung.loesche(s3.zugriff, a3, "gibts-nicht")).grund,
    "not_found",
  );
  assert.equal(
    (
      await verwaltung.loesche(
        s3.zugriff,
        { ...a3, ladefehler: "read_failed" },
        "eigen-a",
      )
    ).grund,
    "read_failed",
  );
});

test("Löschen: Eine Auswahl, die ein Rest in aktiv.json auf einen fehlenden Assistenten hält, fällt beim Lesen auf „Alles“", async () => {
  const s = rust({ aktiv: "geloescht" });
  const a = await lade(s);
  assert.equal(a.aktivId, ALLES_ID);
  // Ein später neu angelegter Assistent gleicher Id wäre die Auswahl: deshalb wird beim Löschen die Auswahl zurückgesetzt.
  s.dateien.set("geloescht.json", eigenerText("geloescht"));
  assert.equal((await lade(s)).aktivId, "geloescht");
});

test("Löschen: ist aktiv.json belegt (kein Auswahldokument), wird die Auswahl nicht angefasst", async () => {
  const text = eigenerText("eigen-a");
  const s = rust({ dateien: { "eigen-a.json": text }, belegt: true });
  const a = await lade(s);
  s.log.length = 0;
  assert.deepEqual(await verwaltung.loesche(s.zugriff, a, "eigen-a"), {
    ok: true,
  });
  assert.deepEqual(s.log, ["delete:eigen-a"]);
});

// ===================================================================== Store: Schreibvorgänge

function frischerStore() {
  useAssistentStore.setState({
    assistenten: speicher.nurVorlagen(null),
    geladen: false,
    wahlFehler: null,
  });
}

test("Store: Schreibvorgänge laufen nacheinander, jeder liest danach frisch neu", async () => {
  frischerStore();
  const s = rust();
  s.verzoegerung = 3;
  await useAssistentStore.getState().laden(true);
  const reihenfolge = [];
  const st = useAssistentStore.getState();
  const eins = st.schreiben(async (z, a, neu) => {
    reihenfolge.push("1 start");
    const r = await verwaltung.legeNeuAn(z, a, neu, (stand) =>
      verwalten.neuAnlegen("Eins", stand),
    );
    reihenfolge.push("1 ende");
    return r;
  });
  const zwei = st.schreiben(async (z, a, neu) => {
    reihenfolge.push("2 start");
    const r = await verwaltung.legeNeuAn(z, a, neu, (stand) =>
      verwalten.neuAnlegen("Eins", stand),
    );
    reihenfolge.push("2 ende");
    return r;
  });
  const [r1, r2] = await Promise.all([eins, zwei]);
  assert.deepEqual(reihenfolge, ["1 start", "1 ende", "2 start", "2 ende"]);
  // Derselbe Name zweimal: der zweite sieht den Stand nach dem ersten und bekommt eine andere Id.
  assert.deepEqual([r1.profil.id, r2.profil.id], ["eins", "eins-2"]);
  assert.deepEqual(
    useAssistentStore
      .getState()
      .assistenten.eigene.map((e) => e.profil.id)
      .sort(),
    ["eins", "eins-2"],
  );
});

test("Store: nach einem Fehler wird trotzdem neu gelesen, die Auswahl bleibt, wie die Datei sie hält", async () => {
  frischerStore();
  const text = eigenerText("eigen-a");
  const s = rust({ dateien: { "eigen-a.json": text }, aktiv: "eigen-a" });
  await useAssistentStore.getState().laden(true);
  assert.equal(useAssistentStore.getState().assistenten.aktivId, "eigen-a");
  s.fehler.loeschen = new speicher.SpeicherFehler("write_failed");
  const r = await useAssistentStore
    .getState()
    .schreiben((z, a) => verwaltung.loesche(z, a, "eigen-a"));
  assert.equal(r.grund, "write_failed");
  const a = useAssistentStore.getState().assistenten;
  // Die Auswahl wurde vor dem Löschen auf „Alles“ gesetzt; der Store zeigt, was auf der Platte steht.
  assert.equal(a.aktivId, ALLES_ID);
  assert.equal(a.eigene.length, 1);
});

test("Store: ein Vorgang, der wirft, bricht die Kette nicht ab", async () => {
  frischerStore();
  rust();
  await useAssistentStore.getState().laden(true);
  await assert.rejects(
    useAssistentStore.getState().schreiben(async () => {
      throw new Error("kaputt");
    }),
    /kaputt/,
  );
  const r = await useAssistentStore
    .getState()
    .schreiben((z, a, neu) =>
      verwaltung.legeNeuAn(z, a, neu, (stand) =>
        verwalten.neuAnlegen("Danach", stand),
      ),
    );
  assert.equal(r.ok, true);
});

test("Store: ein älteres, langsames Laden überschreibt den Stand nach einem Schreibvorgang nicht", async () => {
  frischerStore();
  const s = rust();
  await useAssistentStore.getState().laden(true);
  // Ein Laden hängt (der Speicher antwortet spät) …
  const alt = s.zugriff.liste;
  let bremse = true;
  s.zugriff.liste = async function () {
    const antwort = await alt.call(this);
    if (bremse) await settle(30);
    return antwort;
  };
  tauri.handlers.voice_assistants_list = async () => {
    const l = await s.zugriff.liste();
    return {
      dateien: l.dateien,
      aktiv_id: l.aktivId,
      aktiv_belegt: l.aktivBelegt,
    };
  };
  const langsam = useAssistentStore.getState().laden(true);
  await settle(2);
  // … während ein Schreibvorgang läuft und frisch liest.
  bremse = false;
  const r = await useAssistentStore
    .getState()
    .schreiben((z, a, neu) =>
      verwaltung.legeNeuAn(z, a, neu, (stand) =>
        verwalten.neuAnlegen("Neu", stand),
      ),
    );
  assert.equal(r.ok, true);
  await langsam;
  assert.deepEqual(
    useAssistentStore.getState().assistenten.eigene.map((e) => e.profil.id),
    ["neu"],
  );
});

test("Store: ist der Speicher nicht lesbar, schreibt kein Vorgang", async () => {
  frischerStore();
  const s = rust();
  s.fehler.liste = new speicher.SpeicherFehler("unsafe_path");
  await useAssistentStore.getState().laden(true);
  assert.equal(
    useAssistentStore.getState().assistenten.ladefehler,
    "unsafe_path",
  );
  s.log.length = 0;
  const r = await useAssistentStore
    .getState()
    .schreiben((z, a, neu) =>
      verwaltung.legeNeuAn(z, a, neu, (stand) =>
        verwalten.neuAnlegen("X", stand),
      ),
    );
  assert.deepEqual([r.ok, r.grund], [false, "unsafe_path"]);
  assert.ok(!s.log.some((l) => l.startsWith("save:")));
});

test("Store: Zugriff über die Befehle von Tauri meldet Fehlercodes unverändert", async () => {
  const s = rust({ dateien: { "eigen-a.json": eigenerText("eigen-a") } });
  await assert.rejects(
    tauriZugriff.speichern("eigen-a", "{}", null),
    (e) => e.code === "occupied",
  );
  await assert.rejects(
    tauriZugriff.loeschen("gibts-nicht", "x"),
    (e) => e.code === "not_found",
  );
  s.fehler.speichern = "write_failed";
  await assert.rejects(
    tauriZugriff.speichern("neu", "{}", null),
    (e) => e.code === "write_failed",
  );
});

// ===================================================================== Editor

const WERTE = {
  name: "Anna",
  jahr: 2026,
  monat: 11,
  tag: 14,
  wochentag: 6,
  stunde: 9,
  minute: 5,
};

test("Editor: Fehler der Prüfung gehören zu Feldern der Oberfläche, Unbekanntes zu „sonst“", () => {
  const { feldVon, fehlerJeFeld } = editor;
  assert.equal(feldVon("name"), "name");
  assert.equal(feldVon("anweisungen"), "anweisungen");
  assert.equal(feldVon("verhalten.nachfrage_stille_s"), "verhalten");
  assert.equal(feldVon("wissen.bereiche[3]"), "wissen");
  assert.equal(feldVon("wissen.kontexte[0].art"), "wissen");
  assert.equal(feldVon("zukunft.tief"), "sonst");
  assert.equal(feldVon(""), "sonst");
  assert.equal(feldVon("schema"), "sonst");
  const p = {
    ...neuesProfil("a", "A"),
    name: "",
    begruessung: "x".repeat(301),
  };
  const j = fehlerJeFeld(editor.pruefe(p));
  assert.deepEqual(j.name, [{ pfad: "name", code: "laenge" }]);
  assert.deepEqual(j.begruessung, [{ pfad: "begruessung", code: "laenge" }]);
  assert.equal(j.anweisungen, undefined);
});

test("Editor: der Zähler der Anweisungen zählt Unicode-Zeichen und kennt die Budgets", () => {
  const { rollenStand } = editor;
  const p = (anw) => ({ ...neuesProfil("a", "A"), anweisungen: anw });
  const leer = rollenStand(p(""), WERTE);
  assert.deepEqual(
    [leer.zeichen, leer.aufgeloest, leer.ueberKlein, leer.ueberMax],
    [0, 0, false, false],
  );
  assert.equal(rollenStand(p("😀".repeat(10)), WERTE).zeichen, 10);
  assert.equal(rollenStand(p("x".repeat(1200)), WERTE).ueberKlein, false);
  assert.equal(rollenStand(p("x".repeat(1201)), WERTE).ueberKlein, true);
  assert.equal(rollenStand(p("x".repeat(8000)), WERTE).ueberMax, false);
  assert.equal(rollenStand(p("x".repeat(8001)), WERTE).ueberMax, true);
  assert.equal(editor.GRENZEN.anweisungen, 8000);
  // Platzhalter werden aufgelöst gezählt, Unbekannte genannt.
  const mit = rollenStand(
    p("Hallo {{name}}, heute ist {{wochentag}}. {{gibtsnicht}}"),
    WERTE,
  );
  assert.deepEqual(mit.unbekannt, ["gibtsnicht"]);
  assert.ok(mit.aufgeloest < mit.zeichen + 20);
  assert.ok(mit.aufgeloest > 0);
  for (const name of editor.PLATZHALTER)
    assert.deepEqual(rollenStand(p(`{{${name}}}`), WERTE).unbekannt, [], name);
});

test("Editor: Bereiche schalten, Reihenfolge fest, Fremdes bleibt", () => {
  const { bereichSchalten } = editor;
  let p = neuesProfil("a", "A");
  p = bereichSchalten(p, "kalender", true);
  p = bereichSchalten(p, "buecher", true);
  assert.deepEqual(p.wissen.bereiche, ["buecher", "kalender"]);
  p = bereichSchalten(p, "kalender", true);
  assert.deepEqual(p.wissen.bereiche, ["buecher", "kalender"]);
  const fremd = {
    ...p,
    wissen: { ...p.wissen, bereiche: [...p.wissen.bereiche, "neu_von_morgen"] },
  };
  const aus = bereichSchalten(fremd, "buecher", false);
  assert.deepEqual(aus.wissen.bereiche, ["kalender", "neu_von_morgen"]);
  assert.equal(pruefeProfil(aus).length, 0);
  // Die Quelle bleibt unverändert.
  assert.deepEqual(fremd.wissen.bereiche, [
    "buecher",
    "kalender",
    "neu_von_morgen",
  ]);
  for (const k of kern.BEKANNTE_BEREICHE)
    assert.equal(
      pruefeProfil(bereichSchalten(neuesProfil("a", "A"), k, true)).length,
      0,
      k,
    );
});

test("Editor: Bereiche zeigen die Freigabe getrennt von der Wahl, das Profil öffnet nie mehr, als freigegeben ist", () => {
  const { bereichsAnzeige, bereichsSumme, pcBereich } = editor;
  assert.equal(pcBereich("kalender"), "calendar");
  assert.equal(pcBereich("buecher"), "books");
  assert.equal(pcBereich("ablage"), null);
  assert.equal(pcBereich("irgendwas"), null);
  let p = neuesProfil("a", "A");
  p = editor.bereichSchalten(p, "kalender", true);
  p = editor.bereichSchalten(p, "besprechungen", true);
  p = editor.bereichSchalten(p, "ablage", true);
  const freigegeben = ["meetings", "books"]; // Termine nicht freigegeben
  const zeilen = Object.fromEntries(
    bereichsAnzeige(p, freigegeben).map((z) => [z.schluessel, z]),
  );
  assert.deepEqual(
    [
      zeilen.kalender.gewollt,
      zeilen.kalender.freigegeben,
      zeilen.kalender.wirksam,
    ],
    [true, false, false],
  );
  assert.deepEqual(
    [
      zeilen.besprechungen.gewollt,
      zeilen.besprechungen.freigegeben,
      zeilen.besprechungen.wirksam,
    ],
    [true, true, true],
  );
  assert.deepEqual(
    [
      zeilen.buecher.gewollt,
      zeilen.buecher.freigegeben,
      zeilen.buecher.wirksam,
    ],
    [false, true, false],
  );
  // Ablage hat am PC keine Wirkung.
  assert.deepEqual(
    [zeilen.ablage.gewollt, zeilen.ablage.wirksam, zeilen.ablage.pc],
    [true, false, null],
  );
  assert.deepEqual(bereichsSumme(p, freigegeben), {
    wirksam: 1,
    freigegeben: 2,
  });
  // „Alle Bereiche“ gilt als gewollt, bleibt aber auf die Freigabe begrenzt.
  const alle = editor.mitWissen(p, { alleBereiche: true });
  assert.deepEqual(bereichsSumme(alle, freigegeben), {
    wirksam: 2,
    freigegeben: 2,
  });
  assert.deepEqual(bereichsSumme(alle, []), { wirksam: 0, freigegeben: 0 });
});

test("Editor: jede angebotene Stufe der Nachfrage und jede Sprache besteht die Prüfung des Kerns", () => {
  for (const n of editor.NACHFRAGE_STUFEN) {
    const p = editor.mitVerhalten(neuesProfil("a", "A"), {
      nachfrageStilleS: n,
    });
    assert.equal(pruefeProfil(p).length, 0, String(n));
  }
  assert.ok(editor.NACHFRAGE_STUFEN.includes(editor.NACHFRAGE_VORSCHLAG));
  for (const sprache of editor.SPRACHEN)
    assert.equal(
      pruefeProfil({ ...neuesProfil("a", "A"), sprache }).length,
      0,
      sprache,
    );
  for (const stil of ["natuerlich", "woertlich"])
    assert.equal(
      pruefeProfil(editor.mitVerhalten(neuesProfil("a", "A"), { stil })).length,
      0,
    );
  // Außerhalb der Grenzen wird nicht still repariert: der Kern meldet es.
  assert.deepEqual(
    editor.fehlerJeFeld(
      pruefeProfil(
        editor.mitVerhalten(neuesProfil("a", "A"), { nachfrageStilleS: 3 }),
      ),
    ).verhalten,
    [{ pfad: "verhalten.nachfrage_stille_s", code: "wert" }],
  );
});

test("Editor: Änderungen erkennen (kanonischer Vergleich, nicht nach Objektidentität)", async () => {
  const s = rust({ dateien: { "eigen-a.json": eigenerText("eigen-a") } });
  const a = await lade(s);
  const p = a.eigene[0].profil;
  assert.equal(editor.istGeaendert(p, { ...p }), false);
  assert.equal(
    editor.istGeaendert(p, editor.mit(p, { beschreibung: "x" })),
    true,
  );
  assert.equal(
    editor.istGeaendert(
      p,
      editor.mitWissen(p, { bereiche: [...p.wissen.bereiche] }),
    ),
    false,
  );
  assert.equal(
    editor.istGeaendert(p, editor.mit(p, { stimme: "anton" })),
    true,
  );
});

test("Editor: Abgleich mit einer unter dem Entwurf geänderten Datei", async () => {
  const text = eigenerText("eigen-a");
  const s = rust({ dateien: { "eigen-a.json": text } });
  const basis = (await lade(s)).eigene[0];
  const stand = (aendere) => {
    const profil = aendere(basis.profil);
    return { profil, text: kodiereProfil(profil) };
  };
  // Unverändert.
  assert.deepEqual(editor.gleicheAb(basis, basis, basis.profil), {
    art: "gleich",
  });
  // Datei geändert, Entwurf nicht: der neue Stand gilt.
  const fremd = stand((p) => editor.mit(p, { beschreibung: "fremd" }));
  const r1 = editor.gleicheAb(basis, fremd, basis.profil);
  assert.equal(r1.art, "uebernommen");
  assert.equal(r1.entwurf.beschreibung, "fremd");
  // Entwurf geändert, Datei nur umbenannt: Entwurf bleibt, Name wird übernommen.
  const umbenannt = stand((p) => editor.mit(p, { name: "Neuer Name" }));
  const eigen = editor.mit(basis.profil, { anweisungen: "Meine Änderung." });
  const r2 = editor.gleicheAb(basis, umbenannt, eigen);
  assert.equal(r2.art, "umbenannt");
  assert.equal(r2.entwurf.name, "Neuer Name");
  assert.equal(r2.entwurf.anweisungen, "Meine Änderung.");
  // Hat der Entwurf den Namen selbst geändert, gewinnt der Entwurf.
  const r3 = editor.gleicheAb(
    basis,
    umbenannt,
    editor.mit(eigen, { name: "Mein Name" }),
  );
  assert.equal(r3.art, "umbenannt");
  assert.equal(r3.entwurf.name, "Mein Name");
  // Beides geändert (Datei nicht nur umbenannt): Konflikt, nichts wird überschrieben.
  const r4 = editor.gleicheAb(basis, fremd, eigen);
  assert.equal(r4.art, "konflikt");
  // Die Datei hat schon den Stand des Entwurfs (nach dem eigenen Speichern, bevor die Basis nachzog): kein Konflikt.
  const gespeichert = stand(() => eigen);
  const r5 = editor.gleicheAb(basis, gespeichert, eigen);
  assert.equal(r5.art, "uebernommen");
  assert.equal(r5.entwurf.anweisungen, "Meine Änderung.");
});

test("Editor: der Name der Kopie bleibt im Rahmen von 60 Zeichen", () => {
  assert.equal(editor.kopieName("Coach", "Kopie"), "Coach (Kopie)");
  const lang = editor.kopieName("x".repeat(60), "Kopie");
  assert.ok(Array.from(lang).length <= 60);
  assert.ok(lang.endsWith(" (Kopie)"));
  assert.equal(pruefeProfil(neuesProfil("a", lang)).length, 0);
  const astral = editor.kopieName("😀".repeat(60), "Kopie");
  assert.ok(Array.from(astral).length <= 60);
  assert.equal(pruefeProfil(neuesProfil("a", astral)).length, 0);
});

// ===================================================================== Ende-zu-Ende über Store und Zugriff

test("Ende zu Ende: anlegen, bearbeiten, wählen, neu laden (Neustart), löschen", async () => {
  frischerStore();
  const s = rust();
  const st = useAssistentStore.getState;
  await st().laden(true);
  const neu = await st().schreiben((z, a, nl) =>
    verwaltung.legeNeuAn(z, a, nl, (stand) =>
      verwalten.neuAnlegen("Mein Coach", stand),
    ),
  );
  assert.equal(neu.ok, true);
  const id = neu.profil.id;
  const eigener = st().assistenten.eigene.find((e) => e.profil.id === id);
  const entwurf = editor.bereichSchalten(
    editor.mit(eigener.profil, {
      anweisungen: "Du bist ein strenger Coach.",
      begruessung: "Hallo {{name}}!",
    }),
    "kalender",
    true,
  );
  const gespeichert = await st().schreiben((z, a) =>
    verwaltung.ueberschreibe(z, a, id, entwurf),
  );
  assert.equal(gespeichert.ok, true);
  assert.equal(await st().waehlen(id), true);
  assert.equal(st().assistenten.aktivId, id);
  // „Neustart“: ein frischer Store liest dieselben Dateien.
  frischerStore();
  await st().laden(true);
  const a = st().assistenten;
  assert.equal(a.aktivId, id);
  const p = speicher.profilZu(a, id);
  assert.equal(p.anweisungen, "Du bist ein strenger Coach.");
  assert.deepEqual(p.wissen.bereiche, ["kalender"]);
  assert.equal(speicher.aktivesProfil(a).begruessung, "Hallo {{name}}!");
  // Löschen: Auswahl fällt auf „Alles“, Datei weg.
  const weg = await st().schreiben((z, stand) =>
    verwaltung.loesche(z, stand, id),
  );
  assert.equal(weg.ok, true);
  assert.equal(st().assistenten.aktivId, ALLES_ID);
  assert.equal(st().assistenten.eigene.length, 0);
  assert.equal(s.dateien.size, 0);
});

test("Vorlagen bleiben unverändert und unangetastet, auch nach allen Vorgängen", () => {
  assert.equal(VORLAGEN.length, 15);
  assert.ok(Object.isFrozen(VORLAGEN));
  for (const v of VORLAGEN) assert.ok(Object.isFrozen(v), v.id);
});

// ===================================================================== Hörprobe

const hoerprobe = await import(`${BASIS}profil/hoerprobe.ts`);

/** Abhängigkeiten der Hörprobe als Attrappe: Synthese und Wiedergabe lassen sich von Hand steuern. */
function hoerAttrappe() {
  const a = {
    laeuft: false,
    synthesen: [],
    gespielt: [],
    gestoppt: 0,
    synthFehler: null,
    spielFehler: null,
    offen: [],
    anfragen: [],
    abgebrochen: [],
    abbruchFehler: null,
  };
  const abhaengig = {
    gespraechLaeuft: () => a.laeuft,
    abbrechen: (anfrage) => {
      a.abgebrochen.push(anfrage);
      if (a.abbruchFehler) throw a.abbruchFehler;
    },
    synthese: (text, anfrage) =>
      new Promise((ok, nein) => {
        a.synthesen.push(text);
        a.anfragen.push(anfrage);
        const schritt = {
          ok: () => ok(`wav:${text}`),
          nein: (e) => nein(e),
        };
        a.offen.push(schritt);
        if (a.synthFehler) schritt.nein(a.synthFehler);
        else if (a.automatisch) schritt.ok();
      }),
    spiele: (wav) => {
      if (a.spielFehler) throw a.spielFehler;
      let fertig;
      let scheitern;
      const w = {
        stopp: () => {
          a.gestoppt++;
          fertig();
        },
        ende: new Promise((ok, nein) => {
          fertig = ok;
          scheitern = nein;
        }),
        fertig: () => fertig(),
        scheitern: (e) => scheitern(e),
      };
      a.gespielt.push({ wav, w });
      return w;
    },
  };
  a.abhaengig = abhaengig;
  a.automatisch = true;
  return a;
}

test("Hörprobe: Synthese, Wiedergabe, Ende; der Zustand läuft durch bereit, lädt, spielt, bereit", async () => {
  const a = hoerAttrappe();
  const zustaende = [];
  const h = new hoerprobe.Hoerprobe(a.abhaengig, (z) => zustaende.push(z.art));
  const lauf = h.starte("Hallo Anna");
  await settle();
  assert.deepEqual(zustaende, ["laedt", "spielt"]);
  assert.deepEqual(a.synthesen, ["Hallo Anna"]);
  a.gespielt[0].w.fertig();
  await lauf;
  assert.deepEqual(zustaende, ["laedt", "spielt", "bereit"]);
  assert.deepEqual(h.zustand, { art: "bereit" });
});

test("Hörprobe: leerer Text und laufendes Gespräch geben keine Hörprobe", async () => {
  const a = hoerAttrappe();
  const h = new hoerprobe.Hoerprobe(a.abhaengig);
  await h.starte("   ");
  assert.deepEqual(h.zustand, { art: "fehler", grund: "leer" });
  a.laeuft = true;
  await h.starte("Hallo");
  assert.deepEqual(h.zustand, { art: "fehler", grund: "gespraech" });
  assert.deepEqual(a.synthesen, []);
});

test("Hörprobe: Absturz des Sprachdienstes und fehlende Wiedergabe enden im Fehler, nie in einem hängenden Zustand", async () => {
  const a = hoerAttrappe();
  const h = new hoerprobe.Hoerprobe(a.abhaengig);
  a.synthFehler = new Error("Sprachdienst abgestürzt");
  await h.starte("Hallo");
  assert.deepEqual(h.zustand, { art: "fehler", grund: "synthese" });
  a.synthFehler = null;
  a.spielFehler = new Error("kein Ausgabegerät");
  await h.starte("Hallo");
  assert.deepEqual(h.zustand, { art: "fehler", grund: "wiedergabe" });
  // Die Wiedergabe bricht mittendrin ab.
  a.spielFehler = null;
  const lauf = h.starte("Hallo");
  await settle();
  a.gespielt.at(-1).w.scheitern(new Error("Gerät weg"));
  await lauf;
  assert.deepEqual(h.zustand, { art: "fehler", grund: "wiedergabe" });
  // Danach geht es wieder.
  const lauf2 = h.starte("Nochmal");
  await settle();
  a.gespielt.at(-1).w.fertig();
  await lauf2;
  assert.deepEqual(h.zustand, { art: "bereit" });
});

test("Hörprobe: eine neue Hörprobe oder Stopp macht die ältere unwirksam, ein spätes Ergebnis spielt nie", async () => {
  const a = hoerAttrappe();
  a.automatisch = false;
  const h = new hoerprobe.Hoerprobe(a.abhaengig);
  const erste = h.starte("Erste");
  await settle();
  const zweite = h.starte("Zweite");
  await settle();
  // Die erste Synthese kommt zu spät: sie wird verworfen.
  a.offen[0].ok();
  await erste;
  assert.deepEqual(a.gespielt, []);
  a.offen[1].ok();
  await settle();
  assert.equal(a.gespielt.length, 1);
  assert.equal(a.gespielt[0].wav, "wav:Zweite");
  // Stopp während der Wiedergabe: sie wird angehalten, der Zustand ist bereit.
  h.stopp();
  await zweite;
  assert.equal(a.gestoppt, 1);
  assert.deepEqual(h.zustand, { art: "bereit" });
  // Stopp während der Synthese: später kommt nichts mehr zu Gehör.
  const dritte = h.starte("Dritte");
  await settle();
  h.stopp();
  a.offen[2].ok();
  await dritte;
  assert.equal(a.gespielt.length, 1);
  assert.deepEqual(h.zustand, { art: "bereit" });
});

test("Hörprobe: beginnt ein Gespräch während der Synthese, wird nicht abgespielt", async () => {
  const a = hoerAttrappe();
  a.automatisch = false;
  const h = new hoerprobe.Hoerprobe(a.abhaengig);
  const lauf = h.starte("Hallo");
  await settle();
  a.laeuft = true;
  a.offen[0].ok();
  await lauf;
  assert.deepEqual(a.gespielt, []);
  assert.deepEqual(h.zustand, { art: "fehler", grund: "gespraech" });
});

// ===================================================================== Fixrunde 1 (lva#598): Vergleichsbasis beim Speichern

test("Überschreiben: Vergleichsbasis ist der Text, auf dem der Entwurf begann, nicht der neuere Stand des Speichers", async () => {
  const t0 = eigenerText("eigen-a");
  const s = rust({ dateien: { "eigen-a.json": t0 } });
  const a0 = await lade(s);
  const entwurf = { ...a0.eigene[0].profil, anweisungen: "Mein Entwurf." };
  // Ein anderer Zugang speichert; der Speicher kennt schon den neuen Stand (das Nachladen war schneller als der Entwurf).
  const fremd = eigenerText("eigen-a", (p) => ({
    ...p,
    beschreibung: "fremd",
  }));
  s.dateien.set("eigen-a.json", fremd);
  const a1 = await lade(s);
  s.log.length = 0;
  const r = await verwaltung.ueberschreibe(
    s.zugriff,
    a1,
    "eigen-a",
    entwurf,
    t0,
  );
  assert.deepEqual([r.ok, r.grund], [false, "occupied"]);
  assert.equal(
    s.dateien.get("eigen-a.json"),
    fremd,
    "die fremde Änderung bleibt",
  );
  assert.deepEqual(s.log, [], "nichts geschrieben, nicht einmal versucht");
  // Ohne Basis (Umbenennen baut auf dem Stand auf, den der Vorgang selbst liest) gilt wie bisher der Stand des Speichers.
  const ohne = await verwaltung.ueberschreibe(s.zugriff, a1, "eigen-a", {
    ...a1.eigene[0].profil,
    beschreibung: "mit Stand",
  });
  assert.equal(ohne.ok, true);
});

test("Überschreiben: stimmt die Basis, geht sie als Bedingung bis zum Schreibbefehl; ändert sich die Datei dazwischen, gibt es einen Konflikt", async () => {
  const t0 = eigenerText("eigen-a");
  const s = rust({ dateien: { "eigen-a.json": t0 } });
  const a = await lade(s);
  const entwurf = { ...a.eigene[0].profil, anweisungen: "Mein Entwurf." };
  const bedingungen = [];
  const zugriff = {
    ...s.zugriff,
    speichern: (id, inhalt, ersetzt) => {
      bedingungen.push(ersetzt);
      return s.zugriff.speichern(id, inhalt, ersetzt);
    },
  };
  // Zwischen Vergleich und Schreiben schreibt ein anderer Zugang: Rust meldet `occupied`, nichts geht verloren.
  const dazwischen = eigenerText("eigen-a", (p) => ({
    ...p,
    beschreibung: "dazwischen",
  }));
  s.vorSpeichern = () => s.dateien.set("eigen-a.json", dazwischen);
  const r = await verwaltung.ueberschreibe(zugriff, a, "eigen-a", entwurf, t0);
  assert.deepEqual([r.ok, r.grund], [false, "occupied"]);
  assert.equal(s.dateien.get("eigen-a.json"), dazwischen);
  assert.deepEqual(
    bedingungen,
    [t0],
    "der Schreibbefehl bekam die Originalbytes des Entwurfs",
  );
  // Ohne dazwischenkommende Änderung gelingt es.
  s.vorSpeichern = null;
  s.dateien.set("eigen-a.json", t0);
  const ok = await verwaltung.ueberschreibe(zugriff, a, "eigen-a", entwurf, t0);
  assert.equal(ok.ok, true);
  assert.equal(s.dateien.get("eigen-a.json"), kodiereProfil(entwurf));
});

test("Store: wartet das Speichern auf ein Nachladen, überschreibt der alte Entwurf die fremde Änderung nicht", async () => {
  frischerStore();
  const t0 = eigenerText("eigen-a");
  const s = rust({ dateien: { "eigen-a.json": t0 } });
  const st = useAssistentStore.getState();
  await st.laden(true);
  const basis = useAssistentStore.getState().assistenten.eigene[0];
  const entwurf = { ...basis.profil, anweisungen: "Mein Entwurf." };
  // Die Datei ändert sich außerhalb; ein Nachladen läuft noch, während „Speichern“ gedrückt wird.
  const fremd = eigenerText("eigen-a", (p) => ({
    ...p,
    beschreibung: "fremd",
  }));
  s.dateien.set("eigen-a.json", fremd);
  s.verzoegerung = 15;
  const laden = st.laden(true);
  s.log.length = 0;
  const r = await st.schreiben((z, a) =>
    verwaltung.ueberschreibe(z, a, "eigen-a", entwurf, basis.text),
  );
  await laden;
  assert.deepEqual([r.ok, r.grund], [false, "occupied"]);
  assert.equal(s.dateien.get("eigen-a.json"), fremd);
  assert.equal(s.log.filter((x) => x.startsWith("save:")).length, 0);
  // Der Stand des Speichers ist danach der fremde: dort setzt die Oberfläche den Konflikt.
  assert.equal(useAssistentStore.getState().assistenten.eigene[0].text, fremd);
});

// ===================================================================== Fixrunde 1 (lva#598): Dateibytes lesen, nichts reparieren

const baytes = (...teile) =>
  Uint8Array.from(
    teile.flatMap((t) =>
      typeof t === "string" ? [...new TextEncoder().encode(t)] : t,
    ),
  );
const BOM = [0xef, 0xbb, 0xbf];

test("Import: eine Datei mit BOM wird abgelehnt, auch wenn der Rest ein gültiger Assistent ist", () => {
  const text = eigenerText("eigen-a");
  const r = verwalten.dekodiereDatei(baytes(BOM, text));
  assert.deepEqual([r.ok, r.grund, r.fehler], [false, "bom", []]);
  // Nur die BOM: ebenfalls abgelehnt (nicht als „leer“ durchgewunken).
  assert.equal(verwalten.dekodiereDatei(baytes(BOM)).grund, "bom");
  // Eine BOM mitten im Text ist keine BOM der Datei: der Kern entscheidet (U+FEFF steht im JSON-Text nicht als Leerraum).
  const mitten = verwalten.dekodiereDatei(baytes("{", BOM, "}"));
  assert.equal(mitten.ok, true);
  assert.equal(parseProfil(mitten.text).ok, false);
});

test("Import: ungültiges UTF-8 wird abgelehnt und nie durch U+FFFD ersetzt", () => {
  for (const kaputt of [
    [0xc3, 0x28], // zweites Byte keine Fortsetzung
    [0xff, 0xfe], // nie gültig
    [0xe2, 0x82], // abgeschnittene Folge
    [0x80], // Fortsetzung ohne Anfang
    [0xed, 0xa0, 0x80], // einzelner Surrogat
    [0xc0, 0xaf], // überlange Kodierung
  ]) {
    const r = verwalten.dekodiereDatei(baytes('{"name":"', kaputt, '"}'));
    assert.deepEqual([r.ok, r.grund], [false, "kein_utf8"], kaputt.join(","));
  }
});

test("Import: gültiger Text kommt Zeichen für Zeichen unverändert durch (Umlaute, Emoji, leer, Obergrenze)", () => {
  const text = eigenerText("eigen-a", (p) => ({
    ...p,
    anweisungen: "Größe, Übung: 😀 日本語",
  }));
  const r = verwalten.dekodiereDatei(baytes(text));
  assert.equal(r.ok, true);
  assert.equal(r.text, text);
  assert.equal(parseProfil(r.text).ok, true);
  // Leere Datei: Text ist leer; „leer“ meldet danach das Importieren.
  const leer = verwalten.dekodiereDatei(new Uint8Array(0));
  assert.deepEqual([leer.ok, leer.text], [true, ""]);
  const a = verwalten.importieren(leer.text, { eigene: [], abgewiesen: [] });
  assert.equal(a.grund, "leer");
  // Genau 256 KiB werden gelesen, ein Byte mehr nicht.
  assert.equal(
    verwalten.dekodiereDatei(new Uint8Array(MAX_BYTES).fill(0x20)).ok,
    true,
  );
  assert.equal(
    verwalten.dekodiereDatei(new Uint8Array(MAX_BYTES + 1).fill(0x20)).grund,
    "zu_gross",
  );
});

// ===================================================================== Fixrunde 1 (lva#598): Hörprobe stornieren, ein Besitzer

test("Hörprobe: Stopp während der Synthese bricht genau diese Anfrage ab, ein spätes Ergebnis spielt nie", async () => {
  const a = hoerAttrappe();
  a.automatisch = false;
  const h = new hoerprobe.Hoerprobe(a.abhaengig);
  const lauf = h.starte("Hallo");
  await settle();
  assert.equal(a.anfragen.length, 1);
  assert.ok(typeof a.anfragen[0] === "string" && a.anfragen[0] !== "");
  h.stopp();
  assert.deepEqual(a.abgebrochen, [a.anfragen[0]]);
  a.offen[0].ok();
  await lauf;
  assert.deepEqual(a.gespielt, []);
  assert.deepEqual(h.zustand, { art: "bereit" });
});

test("Hörprobe: eine neue Hörprobe bricht die laufende Synthese der alten ab und nimmt eine andere Anfrage", async () => {
  const a = hoerAttrappe();
  a.automatisch = false;
  const h = new hoerprobe.Hoerprobe(a.abhaengig);
  const erste = h.starte("Erste");
  await settle();
  const zweite = h.starte("Zweite");
  await settle();
  assert.equal(a.anfragen.length, 2);
  assert.notEqual(a.anfragen[0], a.anfragen[1]);
  assert.deepEqual(
    a.abgebrochen,
    [a.anfragen[0]],
    "nur die erste wurde abgebrochen",
  );
  a.offen[0].nein(new Error("abgebrochen"));
  await erste;
  assert.deepEqual(
    h.zustand,
    { art: "laedt" },
    "der Abbruch der alten setzt keinen Fehler für die neue",
  );
  a.offen[1].ok();
  await settle();
  a.gespielt[0].w.fertig();
  await zweite;
  assert.deepEqual(h.zustand, { art: "bereit" });
});

test("Hörprobe: nach fertiger Synthese gibt es keinen Abbruch mehr, Stopp hält nur die Wiedergabe an", async () => {
  const a = hoerAttrappe();
  const h = new hoerprobe.Hoerprobe(a.abhaengig);
  const lauf = h.starte("Hallo");
  await settle();
  assert.equal(a.gespielt.length, 1);
  h.stopp();
  await lauf;
  assert.equal(a.gestoppt, 1);
  assert.deepEqual(
    a.abgebrochen,
    [],
    "kein Abbruch ohne laufende Anfrage (er bliebe im Sprachdienst liegen)",
  );
  // Auch nach einem Fehler der Synthese und nach Ende der Wiedergabe: nichts abzubrechen.
  a.synthFehler = new Error("boom");
  await h.starte("Nochmal");
  h.stopp();
  assert.deepEqual(a.abgebrochen, []);
});

test("Hörprobe: scheitert der Abbruch selbst (Befehl wirft oder wird abgelehnt), bleibt der Zustand heil", async () => {
  const a = hoerAttrappe();
  a.automatisch = false;
  a.abbruchFehler = new Error("Sprachdienst weg");
  const h = new hoerprobe.Hoerprobe(a.abhaengig);
  const lauf = h.starte("Hallo");
  await settle();
  h.stopp();
  assert.deepEqual(a.abgebrochen.length, 1);
  a.offen[0].ok();
  await lauf;
  assert.deepEqual(h.zustand, { art: "bereit" });
  // Asynchron abgelehnter Abbruch: keine unbehandelte Zurückweisung.
  const b = hoerAttrappe();
  b.automatisch = false;
  b.abhaengig.abbrechen = (anfrage) => {
    b.abgebrochen.push(anfrage);
    return Promise.reject(new Error("abgelehnt"));
  };
  const h2 = new hoerprobe.Hoerprobe(b.abhaengig);
  const lauf2 = h2.starte("Hallo");
  await settle();
  h2.stopp();
  await settle(10);
  b.offen[0].ok();
  await lauf2;
  assert.deepEqual(h2.zustand, { art: "bereit" });
});

test("Hörprobe: Anfrage-Ids sind über Instanzen einmalig", async () => {
  const a = hoerAttrappe();
  a.automatisch = false;
  const h1 = new hoerprobe.Hoerprobe(a.abhaengig);
  const h2 = new hoerprobe.Hoerprobe(a.abhaengig);
  void h1.starte("a");
  void h2.starte("b");
  await settle();
  assert.equal(new Set(a.anfragen).size, 2);
  h1.stopp();
  h2.stopp();
});

/** Der Besitzer mit einer Attrappe und zwei „fremden“ Wiedergaben (der Stimmenvorschau). */
function besitzerAttrappe() {
  const a = hoerAttrappe();
  a.fremdGestoppt = 0;
  a.besitzer = new hoerprobe.HoerprobeBesitzer(a.abhaengig, [
    () => {
      a.fremdGestoppt++;
    },
  ]);
  return a;
}

test("Besitzer: eine zweite Hörprobe (anderer Knopf) stoppt die erste, es klingt immer nur eine", async () => {
  const a = besitzerAttrappe();
  const b = a.besitzer;
  const stande = [];
  b.abonniere(() => stande.push(`${b.stand.quelle}:${b.stand.zustand.art}`));
  const erste = b.spiele("begruessung", "Guten Morgen");
  await settle();
  assert.deepEqual(b.stand, {
    quelle: "begruessung",
    zustand: { art: "spielt" },
  });
  const zweite = b.spiele("stimme", "Probe");
  await settle();
  // Die erste Wiedergabe wurde angehalten, die zweite läuft; der Stand gehört dem zweiten Knopf.
  assert.equal(a.gestoppt, 1);
  assert.equal(a.gespielt.length, 2);
  assert.deepEqual(b.stand, { quelle: "stimme", zustand: { art: "spielt" } });
  await erste;
  assert.deepEqual(b.stand, { quelle: "stimme", zustand: { art: "spielt" } });
  assert.ok(!stande.includes("begruessung:bereit"), stande.join(" "));
  a.gespielt[1].w.fertig();
  await zweite;
  assert.deepEqual(b.stand, { quelle: null, zustand: { art: "bereit" } });
});

test("Besitzer: eine zweite Hörprobe während der Synthese bricht die erste Anfrage ab", async () => {
  const a = besitzerAttrappe();
  a.automatisch = false;
  const b = a.besitzer;
  void b.spiele("begruessung", "Guten Morgen");
  await settle();
  void b.spiele("stimme", "Probe");
  await settle();
  assert.deepEqual(a.abgebrochen, [a.anfragen[0]]);
  assert.deepEqual(b.stand, { quelle: "stimme", zustand: { art: "laedt" } });
  b.stopp();
  assert.deepEqual(
    a.abgebrochen,
    [a.anfragen[0], a.anfragen[1]],
    "Stopp storniert die laufende Anfrage",
  );
  assert.deepEqual(b.stand, { quelle: null, zustand: { art: "bereit" } });
});

test("Besitzer: eine Hörprobe stoppt fremde Wiedergaben, und eine fremde stoppt die Hörprobe", async () => {
  const a = besitzerAttrappe();
  const b = a.besitzer;
  void b.spiele("begruessung", "Hallo");
  await settle();
  assert.equal(
    a.fremdGestoppt,
    1,
    "vor der Hörprobe wird die Stimmenvorschau gestoppt",
  );
  b.fremdeBeginnt();
  assert.equal(a.gestoppt, 1);
  assert.deepEqual(b.stand, { quelle: null, zustand: { art: "bereit" } });
  assert.equal(
    a.fremdGestoppt,
    1,
    "die fremde Wiedergabe, die gerade begann, wird nicht gestoppt",
  );
});

test("Besitzer: Beginn eines Gesprächs stoppt Hörprobe (auch die Synthese) und fremde Wiedergaben", async () => {
  const a = besitzerAttrappe();
  a.automatisch = false;
  const b = a.besitzer;
  void b.spiele("begruessung", "Hallo");
  await settle();
  a.laeuft = true;
  b.gespraechBegonnen();
  assert.deepEqual(a.abgebrochen, [a.anfragen[0]]);
  assert.equal(
    a.fremdGestoppt,
    2,
    "einmal vor dem Start, einmal bei Gesprächsbeginn",
  );
  a.offen[0].ok();
  await settle();
  assert.deepEqual(a.gespielt, []);
  assert.deepEqual(b.stand, { quelle: null, zustand: { art: "bereit" } });
  // Während des Gesprächs gibt es keine Hörprobe.
  await b.spiele("stimme", "Probe");
  assert.deepEqual(b.stand, {
    quelle: "stimme",
    zustand: { art: "fehler", grund: "gespraech" },
  });
  assert.equal(a.synthesen.length, 1);
});

test("Besitzer: ein Knopf, der verschwindet, stoppt nur die eigene Hörprobe", async () => {
  const a = besitzerAttrappe();
  const b = a.besitzer;
  void b.spiele("begruessung", "Hallo");
  await settle();
  b.stoppFuer("stimme");
  assert.deepEqual(b.stand, {
    quelle: "begruessung",
    zustand: { art: "spielt" },
  });
  assert.equal(a.gestoppt, 0);
  b.stoppFuer("begruessung");
  assert.deepEqual(b.stand, { quelle: null, zustand: { art: "bereit" } });
  assert.equal(a.gestoppt, 1);
});

test("Besitzer: der Stand ändert sich nur, wenn sich etwas ändert, und Abmelden wirkt", async () => {
  const a = besitzerAttrappe();
  const b = a.besitzer;
  let n = 0;
  const ab = b.abonniere(() => n++);
  const vorher = b.stand;
  b.stopp();
  b.stopp();
  assert.equal(b.stand, vorher, "gleicher Stand, gleiche Referenz");
  assert.equal(n, 0);
  void b.spiele("stimme", "x");
  await settle();
  assert.ok(n >= 2);
  const gezaehlt = n;
  ab();
  b.stopp();
  assert.equal(n, gezaehlt);
});
