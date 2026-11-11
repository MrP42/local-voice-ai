// Lva#485/#509: „Bücher, Wissensbereiche und Transkripte freigeben“ (Einstellungen des Sprachgesprächs), reine Logik.
// Lauf: node --test scripts/alle-bereiche.test.mjs (Node >= 22.6).
import assert from "node:assert/strict";
import { register } from "node:module";
import test from "node:test";

register("./test-stubs/hooks.mjs", import.meta.url);

const { alleBereiche, alleFrei, schalteAllesEin, fokusNachFreigabe } =
  await import("../src/components/voice-call/alleBereiche.ts");
const { ALL_AREAS, GLOBAL_AREAS, KNOWLEDGE_AREAS, NOTEBOOK_AREA } =
  await import("../src/components/voice-call/bereiche.ts");

const OHNE_NOTIZBUCH = [...KNOWLEDGE_AREAS, ...GLOBAL_AREAS];

/**
 * Der Kern als Attrappe, mit denselben Regeln wie `app_context::berechtigung` in Rust: eine Version,
 * EIN Setzen mit Vergleich der erwarteten Version (sonst Konflikt, nichts geschrieben), jede Änderung
 * erhöht die Version. `beimLesen` läuft nach dem Lesen: dort greift der Test mit einem verschachtelten
 * Schreibvorgang ein.
 */
function kern(start = {}) {
  const k = {
    version: 1,
    werte: {
      enabled: false,
      areas: [],
      read_transcripts: false,
      ...start,
    },
    setzenAufrufe: 0,
    uebernommen: 0,
    beimLesen: null,
    /** Ein anderer Weg widerruft (etwa der Hauptschalter in einem anderen Fenster). */
    widerrufen() {
      k.werte = { enabled: false, areas: [], read_transcripts: false };
      k.version += 1;
    },
    lies: async () => {
      const stand = { version: k.version, ...structuredClone(k.werte) };
      await k.beimLesen?.();
      return stand;
    },
    setze: async (erwartet, enabled, areas, readTranscripts) => {
      k.setzenAufrufe += 1;
      if (erwartet !== k.version)
        return { status: "konflikt", version: k.version };
      k.werte = { enabled, areas, read_transcripts: readTranscripts };
      k.version += 1;
      return { status: "ok", version: k.version };
    },
    uebernehmen: async () => {
      k.uebernommen += 1;
    },
  };
  return k;
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

test("schalteAllesEin: gibt Hauptschalter, alle Bereiche und Transkripte in EINEM Aufruf frei", async () => {
  const k = kern();
  assert.equal(await schalteAllesEin(k), "ein");
  assert.equal(k.setzenAufrufe, 1, "genau ein Schreibvorgang statt drei");
  assert.deepEqual(k.werte, {
    enabled: true,
    areas: OHNE_NOTIZBUCH,
    read_transcripts: true,
  });
  assert.equal(k.uebernommen, 1, "der Stand der Oberfläche wird neu gelesen");
});

test("schalteAllesEin: das schon freigegebene Notizbuch bleibt erhalten", async () => {
  const k = kern({ areas: ["notebook"] });
  await schalteAllesEin(k);
  assert.equal(k.werte.areas.includes("notebook"), true);
});

test("schalteAllesEin: ein Widerruf zwischen Lesen und Setzen gewinnt, nichts wird geschrieben", async () => {
  const k = kern({ enabled: true, areas: ["meetings"] });
  k.beimLesen = async () => {
    k.beimLesen = null;
    k.widerrufen();
  };
  assert.equal(await schalteAllesEin(k), "konflikt");
  assert.deepEqual(
    k.werte,
    { enabled: false, areas: [], read_transcripts: false },
    "der Widerruf steht, die Freigabe hat ihn nicht überschrieben",
  );
  assert.equal(k.uebernommen, 1, "nach dem Konflikt wird neu gelesen");
});

test("schalteAllesEin: ein Widerruf, während das Setzen läuft, gewinnt ebenfalls (Version erhöht)", async () => {
  const k = kern();
  const echt = k.setze;
  k.setze = async (...args) => {
    // Der Widerruf kommt zuerst beim Kern an, die Freigabe danach mit der alten Version.
    k.widerrufen();
    return echt(...args);
  };
  assert.equal(await schalteAllesEin(k), "konflikt");
  assert.equal(k.werte.enabled, false);
  assert.deepEqual(k.werte.areas, []);
});

test("Kern: ein zweiter Aufruf mit der alten Version scheitert", async () => {
  const k = kern();
  const alt = (await k.lies()).version;
  assert.equal((await k.setze(alt, true, [], true)).status, "ok");
  const zweiter = await k.setze(alt, false, [], false);
  assert.equal(zweiter.status, "konflikt");
  assert.equal(k.werte.enabled, true, "der Konflikt überschreibt nichts");
});

test("schalteAllesEin: zwei gleichzeitige Freigaben, genau eine gelingt, die andere meldet den Konflikt", async () => {
  const k = kern();
  const r = await Promise.all([schalteAllesEin(k), schalteAllesEin(k)]);
  assert.deepEqual([...r].sort(), ["ein", "konflikt"]);
});

test("schalteAllesEin: ein halber Erfolg ist unmöglich, scheitert das Setzen, bleibt der Stand unverändert", async () => {
  const k = kern({ areas: ["meetings"] });
  const vorher = structuredClone(k.werte);
  k.setze = async () => {
    throw new Error("kaputt");
  };
  assert.equal(await schalteAllesEin(k), "fehler");
  assert.deepEqual(k.werte, vorher);
  assert.equal(k.uebernommen, 1);
});

test("schalteAllesEin: scheitert schon das Lesen, wird nichts gesetzt", async () => {
  const k = kern();
  k.lies = async () => {
    throw new Error("kaputt");
  };
  assert.equal(await schalteAllesEin(k), "fehler");
  assert.equal(k.setzenAufrufe, 0);
});

test("schalteAllesEin: scheitert das Neulesen der Oberfläche, bleibt das Ergebnis erhalten", async () => {
  const k = kern();
  k.uebernehmen = async () => {
    throw new Error("kaputt");
  };
  assert.equal(await schalteAllesEin(k), "ein");
});

// Lva#509 Befund 6: die Taste verschwindet nach Erfolg; wer sie per Tastatur ausgelöst hat, darf den
// Fokus nicht verlieren. Es gibt keine Komponententest-Umgebung, deshalb steckt die Entscheidung in
// einer reinen Funktion, die die Oberfläche nur noch ausführt.
test("fokusNachFreigabe: Erfolg per Tastatur, der Fokus geht auf die Statusanzeige", () => {
  assert.equal(fokusNachFreigabe("ein", true), "status");
});

test("fokusNachFreigabe: Konflikt oder Fehler, die Taste bleibt und bekommt den Fokus zurück", () => {
  assert.equal(fokusNachFreigabe("konflikt", true), "taste");
  assert.equal(fokusNachFreigabe("fehler", true), "taste");
});

test("fokusNachFreigabe: hatte die Taste den Fokus nicht (Maus, Tastatur woanders), wird nichts bewegt", () => {
  for (const e of ["ein", "konflikt", "fehler"])
    assert.equal(fokusNachFreigabe(e, false), "keiner");
});
