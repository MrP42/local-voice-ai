// Fortschritts-Sync (lva#358, R9): Zusammenführen von Datensätzen in verschiedenen Koordinaten.
// iPhone stempelt `source_hash` (normalisierter Text), der PC schreibt Rohzeichen ohne Stempel.
// Lauf: node --test scripts/fortschritt-sync.test.mjs (Node >= 22.6).
import assert from "node:assert/strict";
import { register } from "node:module";
import test from "node:test";

register("./test-stubs/hooks.mjs", import.meta.url);

const speicher = new Map();
globalThis.__tauri = { calls: [], handlers: {} };
globalThis.window = {
  localStorage: {
    getItem: (k) => (speicher.has(k) ? speicher.get(k) : null),
    setItem: (k, v) => void speicher.set(k, String(v)),
    removeItem: (k) => void speicher.delete(k),
  },
  // Nie auslösen: der Kern-Abgleich ist hier nicht Gegenstand.
  setTimeout: () => 1,
  clearTimeout: () => {},
  addEventListener() {},
  removeEventListener() {},
  dispatchEvent() {
    return true;
  },
};

globalThis.document = {
  addEventListener() {},
  removeEventListener() {},
  visibilityState: "visible",
};

const {
  mergeRecordText,
  sameText,
  recordPosition,
  recordRead,
  setFinished,
  saveProgress,
  loadProgress,
} = await import("../src/lib/library/progress.ts");

const rec = (extra) => ({
  offset: 10,
  length: 100,
  heard: [],
  read: [],
  updated: 1,
  ...extra,
});
const json = (r) => JSON.stringify(r);
const merge = (a, b) => JSON.parse(mergeRecordText(json(a), json(b)));

test("sameText: gleicher Hash, zwei ungestempelte; gemischt und verschieden nicht", () => {
  assert.equal(sameText(rec({ source_hash: "h" }), rec({ source_hash: "h" })), true);
  assert.equal(sameText(rec(), rec()), true);
  assert.equal(sameText(rec({ source_hash: "h" }), rec()), false);
  assert.equal(sameText(rec(), rec({ source_hash: "h" })), false);
  assert.equal(sameText(rec({ source_hash: "h" }), rec({ source_hash: "g" })), false);
  // Leerer Hash zählt als fehlend.
  assert.equal(sameText(rec({ source_hash: "" }), rec()), true);
});

test("zwei ungestempelte Datensätze: wie bisher vereinigt", () => {
  const out = merge(
    rec({ updated: 1, heard: [[0, 10]], frontier: 10 }),
    rec({ updated: 2, offset: 30, heard: [[8, 30]] }),
  );
  assert.deepEqual(out.heard, [[0, 30]]);
  assert.equal(out.offset, 30);
  assert.equal(out.frontier, 30);
  assert.equal(out.source_hash, undefined);
});

test("gleicher Hash: vereinigt, Stempel vom neueren", () => {
  const out = merge(
    rec({ updated: 1, heard: [[0, 10]], source_hash: "h", source_edition: "alt" }),
    rec({ updated: 2, offset: 30, heard: [[8, 30]], source_hash: "h", source_edition: "neu" }),
  );
  assert.deepEqual(out.heard, [[0, 30]]);
  assert.equal(out.source_hash, "h");
  assert.equal(out.source_edition, "neu");
});

test("neuerer ungestempelter Satz über älterem gestempeltem: kein Erben, keine Vereinigung", () => {
  const alt = rec({ updated: 1, heard: [[0, 50]], frontier: 50, source_hash: "h", source_edition: "ed" });
  const neu = rec({ updated: 2, offset: 12, heard: [[0, 12]] });
  const text = mergeRecordText(json(alt), json(neu));
  assert.equal(text, json(neu)); // unverändert der neuere
  const out = JSON.parse(text);
  assert.equal(out.source_hash, undefined);
  assert.equal(out.source_edition, undefined);
  assert.deepEqual(out.heard, [[0, 12]]);
  assert.equal(out.frontier, undefined);
});

test("neuerer gestempelter Satz über älterem ungestempeltem: nimmt den neueren, Rohbereiche fallen weg", () => {
  const alt = rec({ updated: 1, heard: [[0, 90]], frontier: 90 });
  const neu = rec({ updated: 2, offset: 5, heard: [[0, 5]], source_hash: "h" });
  assert.equal(mergeRecordText(json(alt), json(neu)), json(neu));
});

test("verschiedene Hashes: der neuere gewinnt unverändert", () => {
  const alt = rec({ updated: 1, heard: [[0, 60]], source_hash: "g" });
  const neu = rec({ updated: 2, heard: [[0, 4]], source_hash: "h" });
  assert.equal(mergeRecordText(json(alt), json(neu)), json(neu));
  // Argumentreihenfolge ändert nichts.
  assert.equal(mergeRecordText(json(neu), json(alt)), json(neu));
});

test("Gleichstand der Zeit und unvereinbar: der erste (lokale) Satz bleibt, ohne Schreiben", () => {
  const lokal = rec({ updated: 3, heard: [[0, 10]] });
  const fern = rec({ updated: 3, heard: [[0, 99]], source_hash: "h" });
  assert.equal(mergeRecordText(json(lokal), json(fern)), json(lokal));
});

test("keine Datensätze: null", () => {
  assert.equal(mergeRecordText("{}", json(rec())), null);
  assert.equal(mergeRecordText("kaputt", json(rec())), null);
});

test("PC-Schreibwege tragen Bereiche eines gestempelten Satzes nicht unter dem ungestempelten weiter", () => {
  const schluessel = "text:seite:original";
  const lege = () =>
    saveProgress(
      schluessel,
      rec({ offset: 40, anchor: "a", heard: [[0, 40]], read: [[0, 30]], frontier: 40, finished: true, source_hash: "h", source_edition: "ed" }),
    );

  lege();
  const pos = recordPosition(schluessel, { to: 50, length: 100 });
  assert.equal(pos.source_hash, undefined);
  assert.deepEqual(pos.heard, [[40, 50]]); // nur der eigene Schritt ab der gespeicherten Stelle
  assert.deepEqual(pos.read, []);
  assert.equal(loadProgress(schluessel).frontier, 50);
  assert.equal(pos.finished, true);

  lege();
  const gelesen = recordRead(schluessel, 60, 70, 100);
  assert.equal(gelesen.source_hash, undefined);
  assert.deepEqual(gelesen.heard, []);
  assert.deepEqual(gelesen.read, [[60, 70]]);
  assert.equal(loadProgress(schluessel).frontier, 70);

  lege();
  setFinished(schluessel, false);
  const fertig = loadProgress(schluessel);
  assert.equal(fertig.source_hash, undefined);
  assert.deepEqual(fertig.heard, []);
  assert.equal(fertig.frontier, undefined);

  // Gleiche Koordinaten: die gespeicherte Grenze zählt weiter.
  saveProgress(schluessel, rec({ heard: [[0, 80]], frontier: 80 }));
  saveProgress(schluessel, rec({ heard: [[0, 20]] }));
  assert.equal(loadProgress(schluessel).frontier, 80);
});
