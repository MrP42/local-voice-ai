// G2 (lva#556): eigene Assistenten im Sprachgespräch. Ladeschicht (Prüfung der Dateien, Auswahl), reine Wirkung,
// die echte Orchestrierung (`prepare`: Rollenblock vor den Regeln, Begrenzung, Budget), die Engine (Rolle, Begrenzung,
// Sprechpause, Begrüßung) und der Gesprächsspeicher (Auswahl beginnt ein neues Gespräch, Laden vor dem ersten Wort).
// Lauf: node --test scripts/assistent-profil.test.mjs (Node >= 22.6; der Teil mit der echten Orchestrierung braucht 22.13).
import assert from "node:assert/strict";
import * as modul from "node:module";
import test, { after } from "node:test";

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
const wirkung = await import(`${BASIS}profil/wirkung.ts`);
const speicher = await import(`${BASIS}profil/speicher.ts`);
const { tauriZugriff } = await import(`${BASIS}profil/zugriff.ts`);
const { VoiceCallEngine } = await import(`${BASIS}engine.ts`);
const { DEFAULT_TURN_CONFIG } = await import(`${BASIS}turnTaking.ts`);
const { ALL_AREAS } = await import(`${BASIS}bereiche.ts`);
const { SpeechPlayer } = await import("./test-stubs/audio.ts");
const echt =
  typeof modul.stripTypeScriptTypes === "function"
    ? await import(`${BASIS}orchestration.ts?echt`)
    : null;

const { VORLAGEN, ALLES_ID, vorlage, kodiereProfil, alsEigenenKopieren } = kern;

const echtesTimeout = globalThis.setTimeout;
const settle = (ms = 5) => new Promise((r) => echtesTimeout(r, ms));
function deferred() {
  let resolve;
  const promise = new Promise((r) => (resolve = r));
  return { promise, resolve };
}

/** Ein gültiger eigener Assistent als Text (kanonisch, wie ihn der Editor schreiben wird). */
function eigenerText(id, aenderung = (p) => p) {
  const kopie = alsEigenenKopieren(
    vorlage("vorlage-lernpartner"),
    id,
    `Eigen ${id}`,
  );
  assert.equal(kopie.ok, true);
  return kodiereProfil(aenderung(kopie.profil));
}

// ===================================================================== Ladeschicht

/**
 * Das Verzeichnis als Attrappe der Befehle `voice_assistant*`. `dateien`: Name -> Text oder `{ abgewiesen }`.
 * `s.calls` hält jeden Aufruf; Lesen ändert nie etwas.
 */
function platte({ dateien = {}, aktiv = null, belegt = false } = {}) {
  const s = {
    dateien: new Map(Object.entries(dateien)),
    aktiv,
    belegt,
    calls: [],
    listeFehler: null,
    setzeFehler: null,
    liste: null,
  };
  const h = tauri.handlers;
  /** Das Verzeichnis, wie Rust es liefert. */
  s.lies = () => ({
    dateien: [...s.dateien].map(([name, wert]) =>
      typeof wert === "string"
        ? { name, inhalt: wert, abgewiesen: null }
        : { name, inhalt: null, abgewiesen: wert.abgewiesen },
    ),
    aktiv_id: s.aktiv,
    aktiv_belegt: s.belegt,
  });
  h.voice_assistants_list = () => {
    s.calls.push("liste");
    if (s.liste) return s.liste();
    if (s.listeFehler) throw s.listeFehler;
    return s.lies();
  };
  h.voice_assistant_active_set = ({ id }) => {
    s.calls.push(`aktiv:${id}`);
    if (s.setzeFehler) throw s.setzeFehler;
    if (s.belegt) throw "occupied";
    s.aktiv = id;
    return null;
  };
  h.voice_assistant_save = ({ id }) => {
    s.calls.push(`save:${id}`);
    return null;
  };
  h.voice_assistant_delete = ({ id }) => {
    s.calls.push(`delete:${id}`);
    return null;
  };
  return s;
}

test("Ladeschicht: leeres Verzeichnis gibt die Vorlagen und die Auswahl „Alles“", async () => {
  const s = platte();
  const a = await speicher.ladeAssistenten(tauriZugriff);
  assert.equal(a.aktivId, ALLES_ID);
  assert.equal(a.eigene.length, 0);
  assert.equal(a.abgewiesen.length, 0);
  assert.equal(a.ladefehler, null);
  assert.equal(a.vorlagen, VORLAGEN);
  assert.equal(VORLAGEN.length, 15);
  assert.deepEqual(s.calls, ["liste"]);
});

test("Ladeschicht: ein gültiger eigener Assistent kommt mit dem Text der Datei", async () => {
  const text = eigenerText("eigen-a");
  platte({ dateien: { "eigen-a.json": text } });
  const a = await speicher.ladeAssistenten(tauriZugriff);
  assert.equal(a.eigene.length, 1);
  assert.equal(a.eigene[0].profil.id, "eigen-a");
  assert.equal(a.eigene[0].text, text);
  assert.equal(speicher.profilZu(a, "eigen-a").name, "Eigen eigen-a");
});

test("Ladeschicht: ungültige Dateien stehen mit Fehlerliste da, nichts wird repariert, geschrieben oder gelöscht", async () => {
  const gut = eigenerText("eigen-a");
  const andereId = gut.replace('"id": "eigen-a"', '"id": "eigen-b"');
  const s = platte({
    dateien: {
      "kaputt.json": "{ das ist kein json",
      "falsch.json": andereId,
      "eigen-a.json": gut,
      "gross.json": { abgewiesen: "laenge" },
      "viele.json": { abgewiesen: "zu_viele" },
      "bin.json": { abgewiesen: "json" },
      "alles.json": gut.replace('"id": "eigen-a"', '"id": "alles"'),
      "vorlage-x.json": gut.replace('"id": "eigen-a"', '"id": "vorlage-x"'),
      "aktiv.json.tmp": "egal",
    },
  });
  const a = await speicher.ladeAssistenten(tauriZugriff);
  assert.deepEqual(
    a.eigene.map((e) => e.profil.id),
    ["eigen-a"],
  );
  const fehler = Object.fromEntries(
    a.abgewiesen.map((x) => [x.datei, x.fehler]),
  );
  assert.deepEqual(fehler["kaputt.json"], [{ pfad: "", code: "json" }]);
  assert.deepEqual(fehler["falsch.json"], [{ pfad: "id", code: "muster" }]);
  assert.deepEqual(fehler["gross.json"], [{ pfad: "", code: "laenge" }]);
  assert.deepEqual(fehler["viele.json"], [{ pfad: "", code: "zu_viele" }]);
  assert.deepEqual(fehler["bin.json"], [{ pfad: "", code: "json" }]);
  assert.deepEqual(fehler["alles.json"], [{ pfad: "id", code: "wert" }]);
  assert.deepEqual(fehler["vorlage-x.json"], [{ pfad: "id", code: "wert" }]);
  // Das Laden ist reines Lesen: nur ein Aufruf, keiner schreibt oder löscht.
  assert.deepEqual(s.calls, ["liste"]);
});

test("Ladeschicht: Vorlagen kommen nie aus dem Speicher (eine Datei mit der Id einer Vorlage ändert sie nicht)", async () => {
  const fremd = eigenerText("lernpartner-x", (p) => ({
    ...p,
    id: "vorlage-lernpartner",
    name: "Eingeschleust",
  }));
  platte({ dateien: { "vorlage-lernpartner.json": fremd } });
  const a = await speicher.ladeAssistenten(tauriZugriff);
  assert.equal(a.eigene.length, 0);
  assert.equal(a.abgewiesen.length, 1);
  assert.equal(speicher.profilZu(a, "vorlage-lernpartner").name, "Lernpartner");
  assert.equal(Object.isFrozen(VORLAGEN), true);
  assert.equal(Object.isFrozen(VORLAGEN[0]), true);
});

test("Ladeschicht: die Auswahl gilt nur für Vorlagen und gültige eigene Assistenten, sonst „Alles“", async () => {
  const gut = eigenerText("eigen-a");
  const dateien = { "eigen-a.json": gut, "kaputt.json": "{" };
  for (const [aktiv, erwartet] of [
    [null, ALLES_ID],
    ["alles", ALLES_ID],
    ["vorlage-lernpartner", "vorlage-lernpartner"],
    ["eigen-a", "eigen-a"],
    ["kaputt", ALLES_ID],
    ["gibt-es-nicht", ALLES_ID],
    ["vorlage-gibt-es-nicht", ALLES_ID],
  ]) {
    platte({ dateien, aktiv });
    const a = await speicher.ladeAssistenten(tauriZugriff);
    assert.equal(a.aktivId, erwartet, `aktiv=${aktiv}`);
    assert.equal(speicher.aktivesProfil(a).id, erwartet);
  }
  platte({ dateien, aktiv: "alles", belegt: true });
  assert.equal(
    (await speicher.ladeAssistenten(tauriZugriff)).aktivBelegt,
    true,
  );
});

test("Ladeschicht: ein nicht lesbarer Speicher gibt nur die Vorlagen und „Alles“, geschrieben wird nichts", async () => {
  for (const fehler of ["read_failed", "occupied", new Error("boom"), 42]) {
    const s = platte({ dateien: { "eigen-a.json": eigenerText("eigen-a") } });
    s.listeFehler = fehler;
    const a = await speicher.ladeAssistenten(tauriZugriff);
    assert.equal(a.aktivId, ALLES_ID);
    assert.equal(a.eigene.length, 0);
    assert.notEqual(a.ladefehler, null);
    assert.deepEqual(s.calls, ["liste"]);
  }
});

test("Auswahl: unbekannte Id ist not_found ohne Aufruf, eine gültige schreibt genau einmal, ein Fehler bleibt ein SpeicherFehler", async () => {
  const s = platte({ dateien: { "eigen-a.json": eigenerText("eigen-a") } });
  const a = await speicher.ladeAssistenten(tauriZugriff);
  await assert.rejects(
    () => speicher.waehleAssistent(tauriZugriff, a, "gibt-es-nicht"),
    (e) => e.name === "SpeicherFehler" && e.code === "not_found",
  );
  assert.deepEqual(s.calls, ["liste"]);
  await speicher.waehleAssistent(tauriZugriff, a, "eigen-a");
  await speicher.waehleAssistent(tauriZugriff, a, "vorlage-lernpartner");
  await speicher.waehleAssistent(tauriZugriff, a, "alles");
  assert.deepEqual(s.calls, [
    "liste",
    "aktiv:eigen-a",
    "aktiv:vorlage-lernpartner",
    "aktiv:alles",
  ]);
  s.belegt = true;
  await assert.rejects(
    () => speicher.waehleAssistent(tauriZugriff, a, "eigen-a"),
    (e) => e.name === "SpeicherFehler" && e.code === "occupied",
  );
});

test("Die Auswahl übersteht den Neustart: neu laden liest, was gespeichert wurde", async () => {
  const s = platte({ dateien: { "eigen-a.json": eigenerText("eigen-a") } });
  const a = await speicher.ladeAssistenten(tauriZugriff);
  await speicher.waehleAssistent(tauriZugriff, a, "eigen-a");
  // „Neustart“: nichts außer dem Verzeichnis bleibt übrig.
  const danach = await speicher.ladeAssistenten(tauriZugriff);
  assert.equal(danach.aktivId, "eigen-a");
  assert.equal(s.aktiv, "eigen-a");
});

// ===================================================================== Wirkung (rein)

const werte =
  (name = "Anna", jetzt = new Date(2026, 10, 13, 9, 15)) =>
  () =>
    wirkung.werteJetzt(name, jetzt);
const wirkungVon = (id, w = werte()) => wirkung.wirkungFuer(vorlage(id), w);

test("Wirkung: „Alles“ wirkt nicht, jede andere Vorlage schon", () => {
  assert.equal(wirkung.wirkungFuer(vorlage(ALLES_ID), werte()), null);
  for (const v of VORLAGEN.filter((p) => p.id !== ALLES_ID))
    assert.notEqual(wirkung.wirkungFuer(v, werte()), null, v.id);
});

test("Rollenbudget: bis 4096 Token 1200 Zeichen, sonst höchstens 8000 und drei Fünftel des Kontexts", () => {
  const b = (contextTokens, local = true) =>
    wirkung.rollenBudget({ contextTokens, local });
  assert.equal(b(1), 1200);
  assert.equal(b(4096), 1200);
  assert.equal(b(4097), Math.floor((4097 * 3) / 5));
  assert.equal(b(8192), 4915);
  assert.equal(b(13333), 7999);
  assert.equal(b(13334), 8000);
  assert.equal(b(1_000_000), 8000);
  // Unbekannt: lokal der Standard (8192), entfernt die Obergrenze; Unsinn gilt als unbekannt.
  assert.equal(b(null, true), 4915);
  assert.equal(b(undefined, false), 8000);
  assert.equal(b(0, true), 4915);
  assert.equal(b(-5, false), 8000);
  assert.equal(b(Number.NaN, true), 4915);
  assert.equal(wirkung.kleinstesBudget([]), 4915);
  assert.equal(
    wirkung.kleinstesBudget([
      { contextTokens: 100_000, local: false },
      { contextTokens: 4096, local: true },
    ]),
    1200,
  );
});

test("Rollenblock: kommt vor den Regeln, steht nie allein, leer ohne Anweisungen", () => {
  const lp = wirkungVon("vorlage-lernpartner");
  const block = wirkung.rolle(lp, lp.werte(), 1200);
  assert.match(
    block.text,
    /^ROLLE \(vom Nutzer eingestellt, Assistent „Lernpartner“\):\n/,
  );
  assert.match(
    block.text,
    /Diese Einstellung ändert keine der folgenden Regeln\.$/,
  );
  assert.equal(
    wirkung.mitRolle(block.text, "REGELN"),
    `${block.text}\n\nREGELN`,
  );
  assert.equal(wirkung.mitRolle("", "REGELN"), "REGELN");
  const leer = wirkung.wirkungFuer(
    { ...vorlage("vorlage-lernpartner"), anweisungen: "" },
    werte(),
  );
  assert.equal(wirkung.rolle(leer, leer.werte(), 1200).text, "");
});

test("Rollenblock: das Budget kürzt die Anweisungen, nie die Rahmenzeilen", () => {
  const lang = Array.from(
    { length: 40 },
    (_, i) => `Absatz ${i} ${"x".repeat(150)}`,
  ).join("\n\n");
  const w = wirkung.wirkungFuer(
    { ...vorlage("vorlage-lernpartner"), anweisungen: lang.slice(0, 7999) },
    werte(),
  );
  const klein = wirkung.rolle(w, w.werte(), 1200);
  const gross = wirkung.rolle(w, w.werte(), 8000);
  assert.equal(klein.gekuerzt, true);
  assert.equal(gross.gekuerzt, false);
  assert.ok(klein.text.length < gross.text.length);
  assert.ok(klein.text.length <= 1200 + 200);
  for (const t of [klein.text, gross.text]) {
    assert.match(t, /^ROLLE \(vom Nutzer eingestellt/);
    assert.match(t, /Diese Einstellung ändert keine der folgenden Regeln\.$/);
  }
});

test("Sprechpause und Unterbrechen: „Alles“ ist der Nutzer, sonst gilt das Profil; Pegel und Schwellen bleiben", () => {
  const nutzer = {
    ...DEFAULT_TURN_CONFIG,
    endMs: 900,
    earlyEndMs: 500,
    bargeIn: false,
  };
  assert.equal(wirkung.wirksamerTurn(nutzer, null), nutzer);
  const kurz = wirkung.wirkungFuer(
    {
      ...vorlage("vorlage-lernpartner"),
      verhalten: {
        ...vorlage("vorlage-lernpartner").verhalten,
        sprechpause: "kurz",
        unterbrechen: true,
      },
    },
    werte(),
  );
  const t = wirkung.wirksamerTurn(nutzer, kurz);
  assert.equal(t.earlyEndMs, 300);
  assert.equal(t.endMs, 500);
  assert.equal(t.bargeIn, true);
  const rest = { ...t };
  delete rest.earlyEndMs;
  delete rest.endMs;
  delete rest.bargeIn;
  const rest0 = { ...nutzer };
  delete rest0.earlyEndMs;
  delete rest0.endMs;
  delete rest0.bargeIn;
  assert.deepEqual(rest, rest0);
  const lang = wirkungVon("vorlage-gedanken-sortieren");
  const tl = wirkung.wirksamerTurn(nutzer, lang);
  assert.deepEqual([tl.earlyEndMs, tl.endMs, tl.bargeIn], [700, 1300, false]);
  // „normal“ ist die Vorgabe der App.
  const normal = wirkungVon("vorlage-lernpartner");
  const tn = wirkung.wirksamerTurn(DEFAULT_TURN_CONFIG, normal);
  assert.deepEqual(
    [tn.earlyEndMs, tn.endMs],
    [DEFAULT_TURN_CONFIG.earlyEndMs, DEFAULT_TURN_CONFIG.endMs],
  );
  // Alle Werte liegen im Bereich der Regler der Gesprächseinstellungen (Ende 400–1500, früh 200 bis Ende − 100).
  for (const v of VORLAGEN) {
    const w = wirkung.wirkungFuer(v, werte());
    const x = wirkung.wirksamerTurn(DEFAULT_TURN_CONFIG, w);
    assert.ok(x.endMs >= 400 && x.endMs <= 1500, v.id);
    assert.ok(x.earlyEndMs >= 200 && x.earlyEndMs <= x.endMs - 100, v.id);
  }
});

test("Begrenzung: nur ein Assistent mit eigenen Bereichen begrenzt, und nie über die Freigabe hinaus", () => {
  assert.equal(wirkung.begrenzung(null), null);
  const lp = wirkungVon("vorlage-lernpartner");
  assert.deepEqual(
    wirkung.begrenzung(lp),
    ALL_AREAS.filter((b) => ["books", "book_notes", "ki_docs"].includes(b)),
  );
  const alle = wirkung.wirkungFuer(
    {
      ...vorlage("vorlage-lernpartner"),
      wissen: { ...vorlage("vorlage-lernpartner").wissen, alleBereiche: true },
    },
    werte(),
  );
  assert.equal(wirkung.begrenzung(alle), null);
  const keiner = wirkungVon("vorlage-gespraech-ueben");
  assert.deepEqual(wirkung.begrenzung(keiner), []);
  // Das Profil öffnet nie mehr, als die Einstellungen freigeben.
  assert.deepEqual(wirkung.wirksam(lp, ["books", "meetings"]), ["books"]);
  assert.deepEqual(wirkung.wirksam(lp, []), []);
  assert.deepEqual(wirkung.wirksam(null, ["meetings", "tts"]), [
    "meetings",
    "tts",
  ]);
  assert.deepEqual(wirkung.wirksam(alle, ["meetings"]), ["meetings"]);
  assert.deepEqual(wirkung.wirksam(keiner, ["meetings", "books"]), []);
});

test("Abruf: ohne Bereich mit Abrufindex gibt es keinen Abruf (Termine und Notizbuch sind nur Werkzeuge)", () => {
  assert.equal(wirkung.hatAbrufBereich(null), true);
  assert.equal(wirkung.hatAbrufBereich([]), false);
  assert.equal(wirkung.hatAbrufBereich(["calendar"]), false);
  assert.equal(wirkung.hatAbrufBereich(["calendar", "notebook"]), false);
  assert.equal(wirkung.hatAbrufBereich(["calendar", "books"]), true);
  for (const b of [
    "meetings",
    "dictations",
    "conversations",
    "tts",
    "books",
    "book_notes",
    "ki_docs",
  ])
    assert.equal(wirkung.hatAbrufBereich([b]), true, b);
});

test("Begrüßung: fester Satz, ohne Namen ohne Anrede, „Alles“ und leere Begrüßung geben null", () => {
  assert.equal(wirkung.begruessung(null, werte()()), null);
  const lp = wirkungVon("vorlage-lernpartner");
  assert.equal(
    wirkung.begruessung(lp, lp.werte()),
    "Guten Morgen, Anna. Welches Thema üben wir heute?",
  );
  const ohne = wirkungVon("vorlage-lernpartner", werte(null));
  const text = wirkung.begruessung(ohne, ohne.werte());
  assert.equal(text, "Guten Morgen. Welches Thema üben wir heute?");
  const nurName = wirkungVon("vorlage-projektstand", werte(null));
  assert.equal(
    wirkung.begruessung(nurName, nurName.werte()),
    "Zu welchem Projekt willst du den Stand hören?",
  );
  for (const v of VORLAGEN) {
    const w = wirkung.wirkungFuer(v, werte(null));
    const t = w ? wirkung.begruessung(w, w.werte()) : null;
    if (t !== null) assert.doesNotMatch(t, /\{\{|undefined|null|, \./, v.id);
  }
  const stumm = wirkung.wirkungFuer(
    { ...vorlage("vorlage-lernpartner"), begruessung: "" },
    werte(),
  );
  assert.equal(wirkung.begruessung(stumm, stumm.werte()), null);
});

test("Werte der Platzhalter: Namen werden bereinigt, nie ein Ersatzname", () => {
  assert.equal(wirkung.werteJetzt(null).name, null);
  assert.equal(wirkung.werteJetzt("   ").name, null);
  assert.equal(wirkung.werteJetzt("  Anna ").name, "Anna");
  const w = wirkung.werteJetzt("A", new Date(2026, 0, 5, 7, 3)); // Montag
  assert.deepEqual(
    [w.jahr, w.monat, w.tag, w.wochentag, w.stunde, w.minute],
    [2026, 1, 5, 1, 7, 3],
  );
  assert.equal(wirkung.werteJetzt("A", new Date(2026, 0, 11)).wochentag, 7); // Sonntag
});

// ===================================================================== Echte Orchestrierung

const ohneEchte = echt
  ? false
  : "braucht Node >= 22.13 (module.stripTypeScriptTypes)";

/** Einstellungen mit einem Sprecher (und optional Denker und lokalem Sofort-Modell). */
function einstellungen({
  sprecher,
  denker,
  lokal,
  wissen = ["meetings"],
  transkripte = false,
}) {
  const modelle = [];
  const verbindungen = [
    { id: "cloud", kind: "openai", label: "Cloud", enabled: true },
    { id: "lokal", kind: "local", label: "Lokal", enabled: true },
  ];
  const modell = (id, verbindung, kontext) => ({
    id,
    connection_id: verbindung,
    remote_id: id,
    label: id,
    enabled: true,
    tags: [],
    ...(kontext ? { context_limit: kontext } : {}),
  });
  modelle.push(
    modell("sprecher", sprecher.lokal ? "lokal" : "cloud", sprecher.kontext),
  );
  if (denker) modelle.push(modell("denker", "cloud", denker.kontext));
  if (lokal) modelle.push(modell("sofort", "lokal", lokal.kontext));
  return {
    llm_connections: verbindungen,
    llm_models: modelle,
    llm_active_model_id: "sprecher",
    function_models: {},
    voice_call_knowledge: wissen,
    voice_call_read_transcripts: transkripte,
  };
}

function session() {
  return new echt.OrchestrationSession({
    subscribe: async () => () => undefined,
    onUsage() {},
    onStatus() {},
    conversationId: () => "c1",
  });
}

const BRUECKE = {
  name: "meetings_search",
  title: "Besprechungen",
  description: "Sucht",
  parameters_json: '{"type":"object","properties":{}}',
  capability: "meetings",
};

function kernEinstellungen(e, tools = [BRUECKE]) {
  tauri.calls = [];
  tauri.handlers = {
    get_app_settings: () => e,
    compliance_assess_models: () => [],
    voice_tools_list: () => tools,
  };
}

const KONFIG = { denkerModelId: "", souveraenitaet: "frei", useTools: false };

/** Eine Assistentensicht wie die Engine sie baut. */
function sicht(id, w = werte()) {
  const a = wirkungVon(id, w);
  return {
    rolle: (b) => wirkung.rolle(a, a.werte(), b),
    grenze: wirkung.begrenzung(a),
  };
}

test(
  "prepare ohne Assistent: Systemanweisung und Hinweis unverändert, keine Rolle, keine Begrenzung",
  { skip: ohneEchte },
  async () => {
    kernEinstellungen(
      einstellungen({ sprecher: { lokal: true, kontext: 8192 } }),
    );
    const p = await session().prepare({ ...KONFIG, systemPrompt: "REGELN" });
    assert.equal(p.sofortRolle ?? "", "");
    assert.match(p.systemPrompt, /^REGELN\n\nDie App kennt Besprechungen/);
    assert.doesNotMatch(p.systemPrompt, /ROLLE/);
    assert.doesNotMatch(p.systemPrompt, /Dieser Assistent nutzt davon nicht/);
    // `null` und fehlend sind dasselbe.
    const q = await session().prepare({
      ...KONFIG,
      systemPrompt: "REGELN",
      assistent: null,
    });
    assert.equal(q.systemPrompt, p.systemPrompt);
  },
);

test(
  "prepare mit Assistent: der Rollenblock steht vor den Regeln, der Hinweis nennt die ausgeschlossenen Bereiche",
  { skip: ohneEchte },
  async () => {
    kernEinstellungen(
      einstellungen({
        sprecher: { lokal: true, kontext: 100_000 },
        wissen: ["meetings", "dictations", "conversations", "calendar", "tts"],
      }),
    );
    const s = sicht("vorlage-lernpartner");
    const p = await session().prepare({
      ...KONFIG,
      systemPrompt: "REGELN",
      assistent: s,
    });
    const rolle = s.rolle(8000).text;
    assert.ok(
      p.systemPrompt.startsWith(`${rolle}\n\nREGELN`),
      "Rolle vor Regeln",
    );
    assert.match(
      p.systemPrompt,
      /Dieser Assistent nutzt davon nicht: Besprechungen, Transkripte, Diktate, frühere Gespräche, Termine und Vorlesen-Seiten\./,
    );
    // Was der Assistent ausschließt, steht nicht im Satz über die Freigabe der Einstellungen.
    assert.doesNotMatch(p.systemPrompt, /nicht freigegeben/);
  },
);

test(
  "prepare: ohne Anweisungen des Assistenten steht nichts vor den Regeln",
  { skip: ohneEchte },
  async () => {
    kernEinstellungen(
      einstellungen({ sprecher: { lokal: true, kontext: 8192 } }),
    );
    const leer = { rolle: () => ({ text: "", gekuerzt: false }), grenze: null };
    const p = await session().prepare({
      ...KONFIG,
      systemPrompt: "REGELN",
      assistent: leer,
    });
    assert.match(p.systemPrompt, /^REGELN\n\n/);
  },
);

test(
  "prepare: Sprecher und Denker bekommen denselben Block, das kleinere Budget gilt; der Sofort-Sprecher rechnet mit seinem eigenen",
  { skip: ohneEchte },
  async () => {
    const lang = Array.from(
      { length: 50 },
      (_, i) => `Absatz ${i} ${"y".repeat(140)}`,
    ).join("\n\n");
    const profil = {
      ...vorlage("vorlage-lernpartner"),
      anweisungen: lang.slice(0, 7999),
    };
    const a = wirkung.wirkungFuer(profil, werte());
    const s = { rolle: (b) => wirkung.rolle(a, a.werte(), b), grenze: null };
    const budgets = [];
    const spion = { rolle: (b) => (budgets.push(b), s.rolle(b)), grenze: null };
    kernEinstellungen(
      einstellungen({
        sprecher: { lokal: false, kontext: 200_000 },
        denker: { kontext: 4096 },
        lokal: { kontext: 100_000 },
      }),
    );
    const p = await session().prepare({
      ...KONFIG,
      denkerModelId: "denker",
      systemPrompt: "REGELN",
      assistent: spion,
    });
    // Erst der gemeinsame Block (Denker mit 4096 Token: 1200), dann der des Sofort-Sprechers (100000 Token: 8000).
    assert.deepEqual(budgets, [1200, 8000]);
    assert.ok(p.systemPrompt.startsWith(s.rolle(1200).text + "\n\nREGELN"));
    assert.equal(p.sofortRolle, s.rolle(8000).text);
    assert.ok(p.sofortRolle.length > s.rolle(1200).text.length);
    assert.notEqual(p.side, null);
  },
);

test(
  "prepare: ohne lokales Modell gibt es keine Rolle für den Sofort-Sprecher",
  { skip: ohneEchte },
  async () => {
    kernEinstellungen(
      einstellungen({ sprecher: { lokal: false, kontext: 100_000 } }),
    );
    const p = await session().prepare({
      ...KONFIG,
      systemPrompt: "REGELN",
      assistent: sicht("vorlage-lernpartner"),
    });
    assert.equal(p.sofortRolle, "");
    assert.equal(p.sofort ?? null, null);
  },
);

test(
  "prepare: die Begrenzung geht an voice_tools_list, und eine andere Begrenzung lädt die Werkzeuge neu",
  { skip: ohneEchte },
  async () => {
    kernEinstellungen(
      einstellungen({ sprecher: { lokal: true, kontext: 8192 } }),
    );
    const sitzung = session();
    const liste = () =>
      tauri.calls
        .filter((c) => c.cmd === "voice_tools_list")
        .map((c) => c.args.grenze);
    const cfg = { ...KONFIG, useTools: true, systemPrompt: "REGELN" };
    await sitzung.prepare(cfg);
    await sitzung.prepare(cfg);
    assert.deepEqual(
      liste(),
      [null],
      "ohne Assistent einmal geladen, ohne Begrenzung",
    );
    const lp = sicht("vorlage-lernpartner");
    await sitzung.prepare({ ...cfg, assistent: lp });
    await sitzung.prepare({ ...cfg, assistent: lp });
    assert.deepEqual(
      liste(),
      [null, lp.grenze],
      "andere Begrenzung: neu geladen, danach aus dem Zwischenspeicher",
    );
    const pj = sicht("vorlage-projektstand");
    await sitzung.prepare({ ...cfg, assistent: pj });
    await sitzung.prepare({ ...cfg, assistent: null });
    assert.deepEqual(liste(), [null, lp.grenze, pj.grenze, null]);
  },
);

test(
  "prepare: ein Rollenwechsel baut den Orchestrator neu, gleiche Rolle behält ihn",
  { skip: ohneEchte },
  async () => {
    kernEinstellungen(
      einstellungen({ sprecher: { lokal: true, kontext: 8192 } }),
    );
    const sitzung = session();
    const cfg = { ...KONFIG, useTools: true, systemPrompt: "REGELN" };
    const a1 = await sitzung.prepare({
      ...cfg,
      assistent: sicht("vorlage-lernpartner"),
    });
    const a2 = await sitzung.prepare({
      ...cfg,
      assistent: sicht("vorlage-lernpartner"),
    });
    assert.equal(a1.side, a2.side, "gleiche Rolle: derselbe Orchestrator");
    const b = await sitzung.prepare({
      ...cfg,
      assistent: sicht("vorlage-ideen-sparring"),
    });
    assert.notEqual(b.side, a1.side, "andere Rolle: neuer Orchestrator");
    const c = await sitzung.prepare(cfg);
    assert.notEqual(c.side, b.side, "ohne Assistent: wieder der alte Weg");
  },
);

test(
  "kontextGroesse: das Kleinere aus Eingabegrenze und Kontextgrenze, unbekannt bleibt unbekannt",
  { skip: ohneEchte },
  () => {
    assert.equal(echt.kontextGroesse({}), undefined);
    assert.equal(
      echt.kontextGroesse({ max_input_tokens: 0, context_limit: -1 }),
      undefined,
    );
    assert.equal(echt.kontextGroesse({ max_input_tokens: 8000 }), 8000);
    assert.equal(echt.kontextGroesse({ context_limit: 4096 }), 4096);
    assert.equal(
      echt.kontextGroesse({ max_input_tokens: 8000, context_limit: 4096 }),
      4096,
    );
    assert.equal(
      echt.kontextGroesse({ max_input_tokens: null, context_limit: 5000 }),
      5000,
    );
  },
);

test(
  "knowledgeHint: ohne Begrenzung bleibt der Text wie er war; eine Begrenzung ergänzt, ändert nie den Satz über die Freigabe",
  { skip: ohneEchte },
  () => {
    const k = { areas: ["meetings"], transcripts: false };
    const alt = echt.knowledgeHint(k);
    assert.equal(echt.knowledgeHint(k, null), alt);
    assert.match(
      alt,
      /Dir sind davon nicht freigegeben: Transkripte, Diktate, frühere Gespräche, Termine und Vorlesen-Seiten\./,
    );
    // Alles frei und nichts begrenzt: kein Hinweis.
    const alle = {
      areas: ["meetings", "dictations", "conversations", "calendar", "tts"],
      transcripts: true,
    };
    assert.equal(echt.knowledgeHint(alle), "");
    assert.equal(echt.knowledgeHint(alle, null), "");
    // Begrenzung auf Termine: alles andere ist „nicht genutzt“, nicht „nicht freigegeben“.
    const mit = echt.knowledgeHint(alle, ["calendar"]);
    assert.match(
      mit,
      /Dieser Assistent nutzt davon nicht: Besprechungen, Transkripte, Diktate, frühere Gespräche und Vorlesen-Seiten\./,
    );
    assert.doesNotMatch(mit, /nicht freigegeben/);
    // Beides: was der Assistent ausschließt, steht nicht im Satz über die Freigabe (der Nutzer kann es dort nicht ändern).
    const beide = echt.knowledgeHint(
      { areas: ["meetings"], transcripts: true },
      ["meetings", "dictations"],
    );
    assert.match(beide, /Dir sind davon nicht freigegeben: Diktate\./);
    assert.match(
      beide,
      /Dieser Assistent nutzt davon nicht: frühere Gespräche, Termine und Vorlesen-Seiten\./,
    );
    // Schneidet die Begrenzung genau weg, was nicht freigegeben ist, bleibt nur der Satz über den Assistenten.
    const gleich = echt.knowledgeHint(
      { areas: ["meetings"], transcripts: true },
      ["meetings"],
    );
    assert.doesNotMatch(gleich, /nicht freigegeben/);
    assert.match(
      gleich,
      /Dieser Assistent nutzt davon nicht: Diktate, frühere Gespräche, Termine und Vorlesen-Seiten\./,
    );
    assert.deepEqual(echt.excludedAreas(null), []);
    assert.deepEqual(echt.excludedAreas(["meetings"]), [
      "Diktate",
      "frühere Gespräche",
      "Termine",
      "Vorlesen-Seiten",
    ]);
    assert.equal(echt.excludedAreas([]).length, 6);
  },
);

// ===================================================================== Engine

/** Alle Engines des Laufs: am Ende angehalten, damit keine Frist den Prozess am Beenden hindert. */
const engines = [];
after(async () => {
  for (const e of engines) await e.stop().catch(() => undefined);
});

const emit = (payload) => tauri.listeners.forEach((l) => l({ payload }));
const chatSagt =
  (...saetze) =>
  ({ requestId }) => {
    for (const text of saetze)
      emit({ kind: "sentence", request_id: requestId, text });
    emit({ kind: "done", request_id: requestId });
    return saetze.join(" ");
  };

/** Lokales Modell (Sofort-Sprecher): merkt sich die Nachrichten. */
function sofortModell(zeile) {
  const m = {
    id: "loc",
    label: "Gemma",
    local: true,
    supportsTools: false,
    aufrufe: [],
    async turn(req) {
      m.aufrufe.push(req);
      return { text: zeile, toolCalls: [] };
    },
  };
  return m;
}

const TREFFER = {
  k: 1,
  kind: "meeting",
  title: "Runde",
  chapter: null,
  at_ms: 0,
  excerpt: "Auszug",
  jump: { kind: "meeting", id: "m1", offset: 0, page: null, book_id: null },
  score: 1,
  version: "00",
};

/** Die Engine mit nachgeahmtem Tauri; `prepare` merkt sich, was die Orchestrierung bekommen hat. */
function neu({
  handlers = {},
  sofort = null,
  kontext = false,
  projekt = null,
} = {}) {
  tauri.calls = [];
  tauri.listeners = [];
  globalThis.__settings = kontext
    ? {
        voice_global_context_enabled: true,
        voice_call_knowledge: ["meetings", "books"],
      }
    : undefined;
  const preps = [];
  tauri.prepare = (cfg) => {
    preps.push(cfg);
    return {
      side: null,
      blocked: false,
      systemPrompt: cfg.assistent
        ? `${cfg.assistent.rolle(1200).text}\n\n${cfg.systemPrompt}`
        : cfg.systemPrompt,
      sofort,
      sofortRolle:
        cfg.assistent && sofort ? cfg.assistent.rolle(1200).text : "",
    };
  };
  tauri.handlers = {
    voice_call_synthesize: ({ text }) => `wav:${text}`,
    voice_call_cancel: () => null,
    assistent_einstufen: () => null,
    assistent_letzte_ausgabe: () => null,
    assistent_ausgabe_vergessen: () => null,
    assistent_messung_melden: () => null,
    voice_call_chat: chatSagt("Ja."),
    app_context_retrieve: () => ({
      hits: [TREFFER],
      searched: ["meeting"],
      retrieve_ms: 1,
      epoch: 1,
      question_hash: "h",
      berechtigung: 1,
    }),
    app_context_message: () => "Kontext [Q1]",
    app_context_verify: () => [1],
    ...handlers,
  };
  const log = { errors: [], assistant: [] };
  const engine = new VoiceCallEngine(
    {
      onState() {},
      onLevel() {},
      onMetrics() {},
      onUser() {},
      onAssistant: (t, final) => log.assistant.push({ t, final }),
      onError: (e) => log.errors.push(String(e)),
      conversationId: () => "c1",
      ...(projekt ? { projekt: () => projekt } : {}),
    },
    { systemPrompt: "sys", turn: DEFAULT_TURN_CONFIG, orchestration: {} },
  );
  engines.push(engine);
  const player = new SpeechPlayer(null);
  player.onIdle = () => engine.playerLeer();
  engine.player = player;
  engine.state = "listening";
  engine.stopped = false;
  engine.speakerLocal = kontext;
  const start = (text) => {
    const turn = engine.newTurn(0, performance.now());
    engine.turn = turn;
    turn.committed = true;
    turn.commit();
    return engine
      .respond(turn, text, 0)
      .catch((e) => log.errors.push(String(e)));
  };
  const aufrufe = (cmd) => tauri.calls.filter((c) => c.cmd === cmd);
  return { engine, player, log, start, aufrufe, preps };
}

const turnVon = (engine) => engine.taker.config;

test("Engine ohne Assistent: nichts ändert sich (keine Sicht, keine Begrenzung, Sprechpause des Nutzers)", async () => {
  const t = neu({ kontext: true });
  await t.start("Wer übernimmt das Protokoll?");
  assert.equal(t.preps.length, 1);
  assert.equal(t.preps[0].assistent, null);
  const req = t.aufrufe("app_context_retrieve")[0].args.req;
  assert.equal(req.area_cap, null);
  assert.deepEqual(turnVon(t.engine), DEFAULT_TURN_CONFIG);
  assert.equal(t.log.errors.length, 0);
});

test("Engine mit „Alles“ gewählt ist dasselbe wie ohne Auswahl (setAssistent(null))", async () => {
  const t = neu({ kontext: true });
  t.engine.setAssistent(null);
  await t.start("Wer übernimmt das Protokoll?");
  assert.equal(t.preps[0].assistent, null);
  assert.equal(t.aufrufe("app_context_retrieve")[0].args.req.area_cap, null);
  assert.deepEqual(turnVon(t.engine), DEFAULT_TURN_CONFIG);
});

test("Engine mit Assistent: Rolle und Begrenzung gehen an die Orchestrierung, der Abruf wird geschnitten", async () => {
  const t = neu({ kontext: true });
  const lp = wirkungVon("vorlage-lernpartner");
  t.engine.setAssistent(lp);
  await t.start("Frag mich ab.");
  const sicht = t.preps[0].assistent;
  assert.deepEqual(sicht.grenze, wirkung.begrenzung(lp));
  assert.match(
    sicht.rolle(1200).text,
    /^ROLLE \(vom Nutzer eingestellt, Assistent „Lernpartner“\):/,
  );
  const req = t.aufrufe("app_context_retrieve")[0].args.req;
  assert.deepEqual(req.area_cap, ["books", "book_notes", "ki_docs"]);
  // Die Rolle steht vor den Regeln im Verlauf zum Modell (hier: die Systemanweisung der Orchestrierung).
  assert.ok(sicht.rolle(1200).text.length > 0);
});

test("Engine: ohne Bereich mit Abrufindex gibt es keinen Abruf, sonst genau einen mit der Begrenzung", async () => {
  for (const [id, abrufe] of [
    ["vorlage-gespraech-ueben", 0],
    ["vorlage-anliegen-aufnehmen", 0],
    ["vorlage-termin-vorbereiten", 1],
    ["vorlage-ideen-sparring", 1],
  ]) {
    const t = neu({ kontext: true });
    t.engine.setAssistent(wirkungVon(id));
    await t.start("Hallo.");
    assert.equal(t.aufrufe("app_context_retrieve").length, abrufe, id);
    assert.equal(t.aufrufe("app_context_message").length, abrufe, id);
    if (abrufe)
      assert.deepEqual(
        t.aufrufe("app_context_retrieve")[0].args.req.area_cap,
        wirkung.begrenzung(wirkungVon(id)),
        id,
      );
  }
});

test("Engine: Sprechpause und Unterbrechen kommen vom Assistenten; ohne ihn wieder die des Nutzers", () => {
  const t = neu();
  t.engine.setAssistent(wirkungVon("vorlage-gedanken-sortieren"));
  assert.deepEqual(
    [
      turnVon(t.engine).earlyEndMs,
      turnVon(t.engine).endMs,
      turnVon(t.engine).bargeIn,
    ],
    [700, 1300, false],
  );
  // Ändert der Nutzer seine Einstellungen, gilt der Assistent weiter.
  t.engine.updateConfig({
    systemPrompt: "sys",
    turn: { ...DEFAULT_TURN_CONFIG, endMs: 600 },
    orchestration: {},
  });
  assert.equal(turnVon(t.engine).endMs, 1300);
  t.engine.setAssistent(null);
  assert.equal(turnVon(t.engine).endMs, 600);
  assert.equal(turnVon(t.engine).bargeIn, DEFAULT_TURN_CONFIG.bargeIn);
});

test("Engine: der Sofort-Sprecher bekommt die Rolle vor seinen Regeln, ohne Assistent bleibt sein Prompt gleich", async () => {
  const { sofortPrompt } = await import(`${BASIS}sofort.ts`);
  const spaet = () => new Promise((r) => echtesTimeout(() => r("Ja."), 100));
  const ohne = sofortModell("ANTWORT: Ja, ich höre dich.");
  const t0 = neu({ sofort: ohne, handlers: { voice_call_chat: spaet } });
  await t0.start("Hörst du mich?");
  assert.equal(ohne.aufrufe[0].messages[0].content, sofortPrompt("de"));

  const mit = sofortModell("ANTWORT: Ja, ich höre dich.");
  const t1 = neu({ sofort: mit, handlers: { voice_call_chat: spaet } });
  const lp = wirkungVon("vorlage-lernpartner");
  t1.engine.setAssistent(lp);
  await t1.start("Hörst du mich?");
  const rolle = wirkung.rolle(lp, lp.werte(), 1200).text;
  assert.equal(
    mit.aufrufe[0].messages[0].content,
    `${rolle}\n\n${sofortPrompt("de")}`,
  );
});

test("Engine (Textmodus): auch dort bekommt der Sofort-Sprecher die Rolle vor seinen Regeln", async () => {
  const { sofortPrompt } = await import(`${BASIS}sofort.ts`);
  const spaet = () => new Promise((r) => echtesTimeout(() => r("Ja."), 100));
  const ohne = sofortModell("ANTWORT: Hallo!");
  const t0 = neu({ sofort: ohne, handlers: { voice_call_chat: spaet } });
  t0.engine.state = "idle";
  await t0.engine.sendText("Hallo");
  assert.equal(ohne.aufrufe[0].messages[0].content, sofortPrompt("de"));
  const mit = sofortModell("ANTWORT: Hallo!");
  const t1 = neu({ sofort: mit, handlers: { voice_call_chat: spaet } });
  const lp = wirkungVon("vorlage-lernpartner");
  t1.engine.setAssistent(lp);
  t1.engine.state = "idle";
  await t1.engine.sendText("Hallo");
  assert.equal(
    mit.aufrufe[0].messages[0].content,
    `${wirkung.rolle(lp, lp.werte(), 1200).text}\n\n${sofortPrompt("de")}`,
  );
});

test("Engine: ein Wechsel des Assistenten wirft den vorbereiteten Abrufkontext weg (er gehört zur alten Rolle)", async () => {
  const frage = { turnId: "x", transcript: "Wer übernimmt das Protokoll?" };
  const mitWechsel = neu({ kontext: true });
  await mitWechsel.engine.globalContext.prepare({ ...frage, grenze: null });
  assert.ok(
    await mitWechsel.engine.globalContext.take("x", frage.transcript),
    "Kontrolle: ohne Wechsel bleibt er",
  );
  const t = neu({ kontext: true });
  await t.engine.globalContext.prepare({ ...frage, grenze: null });
  t.engine.setAssistent(wirkungVon("vorlage-lernpartner"));
  assert.ok(
    !(await t.engine.globalContext.take("x", frage.transcript)),
    "nach dem Wechsel weg",
  );
});

test("Engine: im Projektgespräch gilt der Assistent für Rolle und Wissen nicht (keine Orchestrierung, kein Abruf)", async () => {
  const t = neu({ kontext: true, projekt: { projectId: "p1" } });
  t.engine.setAssistent(wirkungVon("vorlage-lernpartner"));
  await t.start("Wie steht das Projekt?");
  assert.equal(t.preps.length, 0, "das Projekt hat seinen eigenen Weg");
  assert.equal(t.aufrufe("app_context_retrieve").length, 0);
});

test("Engine: die Werte der Platzhalter gelten ab dem Beginn des Gesprächs, nicht je Schritt", async () => {
  let n = 0;
  const w = () => (
    n++,
    wirkung.werteJetzt("Anna", new Date(2026, 10, 13, 9, n))
  );
  const t = neu();
  const a = wirkung.wirkungFuer(vorlage("vorlage-lernpartner"), w);
  t.engine.setAssistent(a);
  assert.equal(n, 1, "beim Setzen gelesen");
  await t.start("Eins.");
  await t.start("Zwei.");
  assert.equal(n, 1, "Schritte lesen nicht neu");
  t.engine.resetHistory();
  assert.equal(n, 2, "neues Gespräch liest neu");
  t.engine.setAssistent(null);
  t.engine.resetHistory();
  assert.equal(n, 2, "ohne Assistent liest niemand");
  t.engine.setAssistent(a);
  t.engine.werteNeu();
  assert.equal(n, 4);
  t.engine.setAssistent(null);
  t.engine.werteNeu();
  assert.equal(n, 4, "ohne Assistent tut werteNeu nichts");
});

test("Engine: ein Wechsel des Assistenten verwirft vorbereiteten Abrufkontext und baut neu", async () => {
  const t = neu({ kontext: true });
  t.engine.setAssistent(wirkungVon("vorlage-lernpartner"));
  await t.start("Eins.");
  t.engine.setAssistent(wirkungVon("vorlage-besprechung-nachbereiten"));
  await t.start("Zwei.");
  const caps = t
    .aufrufe("app_context_retrieve")
    .map((c) => c.args.req.area_cap);
  assert.deepEqual(caps, [["books", "book_notes", "ki_docs"], ["meetings"]]);
  assert.equal(t.preps.length, 2);
});

// ===================================================================== Gesprächsspeicher

const { useVoiceCallStore: store } =
  await import("../src/stores/voiceCallStore.ts");
const { useAssistentStore: assistenten, LADEN_WARTEZEIT_MS } =
  await import("../src/stores/assistentStore.ts");

let speicherLauf = 0;
const gespeichert = [];
function speicherHandlers(extra = {}) {
  return {
    conversation_get: ({ id }) => ({
      conversation: { id, title: "Neues Gespräch" },
      entries: [],
    }),
    conversations_list: () => [],
    conversation_create: () => ({ id: "k-neu" }),
    conversation_append_entry: ({ entry }) => {
      gespeichert.push(entry);
      return { id: `e${gespeichert.length}` };
    },
    conversation_update_entry: () => null,
    conversation_update: () => null,
    voice_call_synthesize: ({ text }) => `wav:${text}`,
    voice_call_cancel: () => null,
    assistent_einstufen: () => null,
    assistent_letzte_ausgabe: () => null,
    assistent_ausgabe_vergessen: () => null,
    assistent_messung_melden: () => null,
    voice_call_chat: chatSagt("Antwort."),
    ...extra,
  };
}

/** Der echte Speicher in einem bekannten Anfangszustand; `platteStand` ist das Verzeichnis. */
async function speicherNeu({
  dateien = {},
  aktiv = null,
  geladen = true,
  name = null,
} = {}) {
  const s = platte({ dateien, aktiv });
  tauri.listeners = [];
  tauri.prepare = undefined;
  Object.assign(tauri.handlers, speicherHandlers());
  assistenten.setState({
    assistenten: speicher.nurVorlagen(null),
    geladen: false,
    name,
    wahlFehler: null,
  });
  if (geladen) await assistenten.getState().laden();
  await store.getState().openConversation(`k${++speicherLauf}`);
  const engine = globalThis.window.__voiceCallEngine;
  await engine.dispose();
  tauri.calls = [];
  s.calls.length = 0;
  return { s, engine };
}

test("Speicher: eine Auswahl wird gemerkt, beginnt ein neues Gespräch und gilt für die Engine", async () => {
  const { s, engine } = await speicherNeu();
  store.setState({
    lines: [
      {
        id: 1,
        role: "user",
        text: "Hallo",
        at: 1,
        final: true,
        persisted: true,
      },
    ],
    conversationId: "k-alt",
  });
  const ok = await store.getState().assistentWechseln("vorlage-lernpartner");
  assert.equal(ok, true);
  assert.deepEqual(s.calls, ["aktiv:vorlage-lernpartner"]);
  assert.equal(s.aktiv, "vorlage-lernpartner");
  assert.equal(
    assistenten.getState().assistenten.aktivId,
    "vorlage-lernpartner",
  );
  assert.equal(store.getState().lines.length, 0, "neues, leeres Gespräch");
  assert.equal(store.getState().conversationId, null);
  assert.equal(engine.assistent?.profil.id, "vorlage-lernpartner");
  // Zurück zu „Alles“: die Engine ist wieder ohne Eingriff.
  assert.equal(await store.getState().assistentWechseln("alles"), true);
  assert.equal(engine.assistent, null);
  assert.deepEqual(turnVon(engine), store.getState().config.turn);
});

test("Speicher: scheitert das Merken, ändert sich nichts (Gespräch, Engine und Auswahl bleiben)", async () => {
  const { s, engine } = await speicherNeu();
  const zeile = {
    id: 1,
    role: "user",
    text: "Hallo",
    at: 1,
    final: true,
    persisted: true,
  };
  store.setState({ lines: [zeile], conversationId: "k-alt" });
  s.setzeFehler = "write_failed";
  assert.equal(
    await store.getState().assistentWechseln("vorlage-lernpartner"),
    false,
  );
  assert.equal(assistenten.getState().wahlFehler, "write_failed");
  assert.equal(assistenten.getState().assistenten.aktivId, "alles");
  assert.deepEqual(store.getState().lines, [zeile]);
  assert.equal(store.getState().conversationId, "k-alt");
  assert.equal(engine.assistent, null);
  // Belegte Datei `aktiv.json`: derselbe Fehler, nichts geändert.
  s.setzeFehler = null;
  s.belegt = true;
  assert.equal(
    await store.getState().assistentWechseln("vorlage-lernpartner"),
    false,
  );
  assert.equal(assistenten.getState().wahlFehler, "occupied");
  // Danach klappt es wieder, und der Fehler ist weg.
  s.belegt = false;
  assert.equal(
    await store.getState().assistentWechseln("vorlage-lernpartner"),
    true,
  );
  assert.equal(assistenten.getState().wahlFehler, null);
});

test("Speicher: ein nicht lesbarer Speicher lässt keine Auswahl zu und schreibt nichts", async () => {
  const s = platte();
  s.listeFehler = "read_failed";
  Object.assign(tauri.handlers, speicherHandlers());
  assistenten.setState({
    assistenten: speicher.nurVorlagen(null),
    geladen: false,
    name: null,
    wahlFehler: null,
  });
  await assistenten.getState().laden();
  await store.getState().openConversation(`k${++speicherLauf}`);
  await globalThis.window.__voiceCallEngine.dispose();
  s.calls.length = 0;
  assert.equal(assistenten.getState().assistenten.ladefehler, "read_failed");
  assert.equal(
    await store.getState().assistentWechseln("vorlage-lernpartner"),
    false,
  );
  assert.equal(assistenten.getState().wahlFehler, "read_failed");
  assert.deepEqual(s.calls, [], "kein Schreibversuch");
});

test("Speicher: ein unverändertes Neuladen fasst die Engine nicht an, ein geänderter Text derselben Id wirkt", async () => {
  const { s, engine } = await speicherNeu({ aktiv: "vorlage-lernpartner" });
  assert.equal(engine.assistent?.profil.id, "vorlage-lernpartner");
  let gesetzt = 0;
  const original = engine.setAssistent.bind(engine);
  engine.setAssistent = (a) => (gesetzt++, original(a));
  let werteNeu = 0;
  const w0 = engine.werteNeu.bind(engine);
  engine.werteNeu = () => (werteNeu++, w0());
  await assistenten.getState().laden();
  await assistenten.getState().laden();
  assert.equal(gesetzt, 0, "unverändertes Neuladen fasst die Engine nicht an");
  assert.equal(werteNeu, 0);
  // Ein geänderter Text derselben Id (z. B. im Editor) wirkt dagegen.
  const text = eigenerText("eigen-a");
  s.dateien.set("eigen-a.json", text);
  s.aktiv = "eigen-a";
  await assistenten.getState().laden();
  assert.equal(gesetzt, 1);
  s.dateien.set(
    "eigen-a.json",
    eigenerText("eigen-a", (p) => ({ ...p, anweisungen: "Neu." })),
  );
  await assistenten.getState().laden();
  assert.equal(gesetzt, 2);
  engine.setAssistent = original;
  engine.werteNeu = w0;
});

test("Speicher: der Name kommt auch nach dem Start an (Platzhalter), ein gleicher Name tut nichts", async () => {
  const { engine } = await speicherNeu({ aktiv: "vorlage-lernpartner" });
  assert.equal(engine.rolleWerte.name, null);
  assistenten.getState().setzeName("Anna");
  assert.equal(engine.rolleWerte.name, "Anna");
  let n = 0;
  const w0 = engine.werteNeu.bind(engine);
  engine.werteNeu = () => (n++, w0());
  assistenten.getState().setzeName("Anna");
  assistenten.getState().setzeName("  Anna ");
  assert.equal(n, 0);
  assistenten.getState().setzeName("   ");
  assert.equal(assistenten.getState().name, null);
  assert.equal(n, 1);
  engine.werteNeu = w0;
});

test("Speicher: der erste Schritt wartet auf das Laden, die Rolle ist von Anfang an dabei", async () => {
  const s = platte({ aktiv: "vorlage-lernpartner" });
  const offen = deferred();
  s.liste = () => offen.promise;
  Object.assign(tauri.handlers, speicherHandlers());
  assistenten.setState({
    assistenten: speicher.nurVorlagen(null),
    geladen: false,
    name: null,
    wahlFehler: null,
  });
  const preps = [];
  tauri.prepare = (cfg) => {
    preps.push(cfg);
    return { side: null, blocked: false, systemPrompt: cfg.systemPrompt };
  };
  tauri.calls = [];
  const gesendet = store.getState().sendText("Erste Frage");
  await settle(30);
  assert.equal(preps.length, 0, "vor dem Laden geht nichts los");
  offen.resolve({
    dateien: [],
    aktiv_id: "vorlage-lernpartner",
    aktiv_belegt: false,
  });
  await gesendet;
  assert.equal(preps.length >= 1, true);
  assert.equal(preps[0].assistent?.rolle(1200).text.startsWith("ROLLE"), true);
  await globalThis.window.__voiceCallEngine.dispose();
});

test("Speicher: hängt das Laden, geht das Gespräch nach der Wartezeit mit „Alles“ weiter", async () => {
  const s = platte({ aktiv: "vorlage-lernpartner" });
  s.liste = () => new Promise(() => undefined);
  Object.assign(tauri.handlers, speicherHandlers());
  assistenten.setState({
    assistenten: speicher.nurVorlagen(null),
    geladen: false,
    name: null,
    wahlFehler: null,
  });
  const preps = [];
  tauri.prepare = (cfg) => {
    preps.push(cfg);
    return { side: null, blocked: false, systemPrompt: cfg.systemPrompt };
  };
  const t0 = Date.now();
  await store.getState().sendText("Frage trotz hängendem Speicher");
  const gewartet = Date.now() - t0;
  assert.ok(
    gewartet >= LADEN_WARTEZEIT_MS - 50 && gewartet < LADEN_WARTEZEIT_MS + 1500,
    `gewartet ${gewartet} ms`,
  );
  assert.equal(preps[0].assistent ?? null, null);
  await globalThis.window.__voiceCallEngine.dispose();
});

test("Speicher: Auswahlen laufen nacheinander, der zuletzt gewünschte Stand gilt", async () => {
  const { s } = await speicherNeu();
  const erste = deferred();
  const echt0 = tauri.handlers.voice_assistant_active_set;
  let n = 0;
  tauri.handlers.voice_assistant_active_set = (a) => {
    n++;
    return n === 1 ? erste.promise.then(() => echt0(a)) : echt0(a);
  };
  const a = assistenten.getState().waehlen("vorlage-lernpartner");
  const b = assistenten.getState().waehlen("vorlage-ideen-sparring");
  await settle(20);
  assert.equal(n, 1, "die zweite wartet auf die erste");
  erste.resolve();
  assert.deepEqual(await Promise.all([a, b]), [true, true]);
  assert.equal(s.aktiv, "vorlage-ideen-sparring");
  assert.equal(
    assistenten.getState().assistenten.aktivId,
    "vorlage-ideen-sparring",
  );
});

test("Speicher: ein Laden, das während einer Auswahl liest, überschreibt sie nicht mit dem älteren Stand", async () => {
  const { s } = await speicherNeu();
  const frueh = deferred();
  let erster = true;
  s.liste = () => {
    if (!erster) return s.lies();
    erster = false;
    return frueh.promise;
  };
  const laden = assistenten.getState().laden();
  await settle(10);
  assert.equal(
    await assistenten.getState().waehlen("vorlage-lernpartner"),
    true,
  );
  frueh.resolve({ dateien: [], aktiv_id: null, aktiv_belegt: false });
  await laden;
  assert.equal(
    assistenten.getState().assistenten.aktivId,
    "vorlage-lernpartner",
  );
});
