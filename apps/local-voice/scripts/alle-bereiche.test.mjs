// Lva#485: „Zugriff auf alle App-Inhalte einschalten“ (Einstellungen des Sprachgesprächs), reine Logik.
// Lauf: node --test scripts/alle-bereiche.test.mjs (Node >= 22.6).
import assert from "node:assert/strict";
import { register } from "node:module";
import test from "node:test";

register("./test-stubs/hooks.mjs", import.meta.url);

const { alleBereiche, alleFrei, schalteAllesEin, ZUGRIFF_SCHLUESSEL } =
  await import("../src/components/voice-call/alleBereiche.ts");
const { ALL_AREAS, GLOBAL_AREAS, KNOWLEDGE_AREAS, NOTEBOOK_AREA } =
  await import("../src/components/voice-call/bereiche.ts");

const OHNE_NOTIZBUCH = [...KNOWLEDGE_AREAS, ...GLOBAL_AREAS];

/** Einstellungen als Attrappe; `scheitert`: Schlüssel, deren Speichern der Store zurückrollt. */
function store(start = {}, scheitert = []) {
  const werte = {
    voice_call_knowledge: [],
    voice_call_read_transcripts: false,
    voice_global_context_enabled: false,
    ...start,
  };
  const log = [];
  return {
    werte,
    log,
    lies: (k) => werte[k],
    setze: async (k, v) => {
      log.push([k, v]);
      // Wie `updateSetting`: wirft nie, rollt bei Fehler den Schlüssel zurück.
      if (!scheitert.includes(k)) werte[k] = v;
    },
  };
}

test("alleBereiche: alle vorhandenen Bereiche aus den Listen, ohne Notizbuch", () => {
  assert.deepEqual(alleBereiche([]), OHNE_NOTIZBUCH);
  assert.equal(alleBereiche([]).includes(NOTEBOOK_AREA), false);
});

test("alleBereiche: bleibt das Notizbuch schon freigegeben, bleibt es, in fester Reihenfolge", () => {
  const r = alleBereiche(["notebook", "books"]);
  assert.deepEqual(r, [...OHNE_NOTIZBUCH, NOTEBOOK_AREA]);
  assert.deepEqual(
    r,
    ALL_AREAS.filter((a) => r.includes(a)),
  );
});

test("alleBereiche: unbekannte Einträge fallen weg (wie im Kern)", () => {
  assert.deepEqual(alleBereiche(["gibt-es-nicht"]), OHNE_NOTIZBUCH);
});

test("alleFrei: nur mit Hauptschalter, allen Bereichen und Transkripten", () => {
  const voll = {
    enabled: true,
    areas: OHNE_NOTIZBUCH,
    transcripts: true,
  };
  assert.equal(alleFrei(voll), true);
  assert.equal(alleFrei({ ...voll, enabled: false }), false);
  assert.equal(alleFrei({ ...voll, transcripts: false }), false);
  assert.equal(alleFrei({ ...voll, areas: OHNE_NOTIZBUCH.slice(1) }), false);
  assert.equal(alleFrei({ ...voll, areas: [] }), false);
  assert.equal(
    alleFrei({ ...voll, areas: [...OHNE_NOTIZBUCH, NOTEBOOK_AREA] }),
    true,
    "das Notizbuch ist eine eigene Freigabe und zählt nicht mit",
  );
});

test("schalteAllesEin: setzt Hauptschalter, alle Bereiche und Transkripte", async () => {
  const s = store();
  assert.equal(await schalteAllesEin(s), true);
  assert.equal(s.werte.voice_global_context_enabled, true);
  assert.equal(s.werte.voice_call_read_transcripts, true);
  assert.deepEqual(s.werte.voice_call_knowledge, OHNE_NOTIZBUCH);
  // Der Hauptschalter kommt zuletzt: bricht etwas ab, wirkt nichts.
  assert.equal(s.log.at(-1)[0], "voice_global_context_enabled");
});

test("schalteAllesEin: schreibt nur, was sich ändert", async () => {
  const s = store({
    voice_call_knowledge: OHNE_NOTIZBUCH,
    voice_call_read_transcripts: true,
  });
  assert.equal(await schalteAllesEin(s), true);
  assert.deepEqual(s.log, [["voice_global_context_enabled", true]]);
});

test("schalteAllesEin: das schon freigegebene Notizbuch bleibt erhalten, Abschnitte bleiben unberührt", async () => {
  const s = store({ voice_call_knowledge: ["notebook"] });
  await schalteAllesEin(s);
  assert.equal(s.werte.voice_call_knowledge.includes("notebook"), true);
  assert.equal(
    s.log.some(([k]) => k === "voice_call_notebook_sections"),
    false,
  );
});

for (const kaputt of ZUGRIFF_SCHLUESSEL) {
  test(`schalteAllesEin: scheitert ${kaputt}, ist alles auf dem alten Stand und das Ergebnis meldet den Fehler`, async () => {
    const start = {
      voice_call_knowledge: ["meetings"],
      voice_call_read_transcripts: false,
      voice_global_context_enabled: false,
    };
    const s = store(start, [kaputt]);
    assert.equal(await schalteAllesEin(s), false);
    assert.deepEqual(s.werte, start);
  });
}

test("schalteAllesEin: ein Schlüssel, der schon im Zielzustand war, wird beim Rückrollen nicht angefasst", async () => {
  const s = store({ voice_call_read_transcripts: true }, [
    "voice_global_context_enabled",
  ]);
  assert.equal(await schalteAllesEin(s), false);
  assert.equal(s.werte.voice_call_read_transcripts, true);
  assert.deepEqual(s.werte.voice_call_knowledge, []);
  assert.equal(s.werte.voice_global_context_enabled, false);
});
