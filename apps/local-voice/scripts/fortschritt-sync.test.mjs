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
const assertActive = (a, b, active) => {
  const { coordinate_history, ...out } = merge(a, b);
  assert.deepEqual(out, JSON.parse(json(active)));
  const other = active === a ? b : a;
  const key = other.source_hash ? `hash:${other.source_hash}` : "raw";
  assert.deepEqual(coordinate_history[key], JSON.parse(json(other)));
};

test("Altstand ohne frontier verliert seine Lesegrenze beim Rücksprung nicht", () => {
  for (const key of ["text:de:original", "text:en:original", "doc:book:kurzbuch", "text:live:original"]) {
    const result = merge(rec({ offset: 800, updated: 1 }), rec({ offset: 20, updated: 2 }));
    assert.equal(result.frontier, 800, key);
  }
});

test("neue große Sprünge werden trotz Altstands-Migration nicht als gelesen gezählt", () => {
  const key = "text:l1-jump:original";
  recordPosition(key, { to: 50000, length: 100000 });
  recordPosition(key, { to: 10, length: 100000 });
  assert.equal(loadProgress(key).frontier, 0);
  assert.deepEqual(loadProgress(key).heard, []);
});

test("Lesesitzungen: Union je Gerät und Start, unabhängige Maxima, unbekannte Felder", () => {
  const a = rec({ reading_log: [{ device: "a", start_ms: 1, end_ms: 31000, to: 10, future: true }] });
  const b = rec({ updated: 2, reading_log: [{ device: "a", start_ms: 1, end_ms: 41000, to: 8 },
    { device: "b", start_ms: 1, end_ms: 31000, to: 5 }] });
  const result = merge(a, b);
  assert.equal(result.reading_log.length, 2);
  assert.equal(result.reading_log[0].to, 10);
  assert.equal(result.reading_log[0].end_ms, 41000);
  assert.equal(result.reading_log[0].future, true);
  assert.deepEqual(merge(result, result), result);
});

test("Sprachwechsel und Live: fremde Koordinaten bleiben wiederherstellbar", () => {
  const a = rec({ source_hash: "de", heard: [[0, 80]], read: [[80, 90]], frontier: 90 });
  const b = rec({ source_hash: "en", updated: 2, heard: [[0, 5]], frontier: 5 });
  const switched = merge(a, b);
  assert.deepEqual(switched.heard, b.heard);
  assert.deepEqual(switched.coordinate_history["hash:de"].read, a.read);
  const restored = merge(switched, rec({ source_hash: "de", updated: 3, frontier: 2 }));
  assert.equal(restored.frontier, 90);
  assert.deepEqual(restored.heard, a.heard);
  assert.deepEqual(restored.coordinate_history["hash:en"].heard, b.heard);
});

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

test("gleicher Hash: fehlende Fassung des neueren wird nicht vom älteren geerbt", () => {
  const alt = rec({ updated: 1, source_hash: "h", source_edition: "alt" });
  const neu = rec({ updated: 2, source_hash: "h" });
  for (const [a, b] of [[alt, neu], [neu, alt]]) {
    const out = merge(a, b);
    assert.equal(out.source_hash, "h");
    assert.equal(Object.hasOwn(out, "source_edition"), false);
  }
});

test("neuerer ungestempelter Satz über älterem gestempeltem: kein Erben, keine Vereinigung", () => {
  const alt = rec({ updated: 1, heard: [[0, 50]], frontier: 50, source_hash: "h", source_edition: "ed" });
  const neu = rec({ updated: 2, offset: 12, heard: [[0, 12]] });
  const text = mergeRecordText(json(alt), json(neu));
  assertActive(alt, neu, neu);
  const out = JSON.parse(text);
  assert.equal(out.source_hash, undefined);
  assert.equal(out.source_edition, undefined);
  assert.deepEqual(out.heard, [[0, 12]]);
  assert.equal(out.frontier, undefined);
});

test("neuerer gestempelter Satz über älterem ungestempeltem: nimmt den neueren, Rohbereiche fallen weg", () => {
  const alt = rec({ updated: 1, heard: [[0, 90]], frontier: 90 });
  const neu = rec({ updated: 2, offset: 5, heard: [[0, 5]], source_hash: "h" });
  assertActive(alt, neu, neu);
});

test("verschiedene Hashes: der neuere gewinnt unverändert", () => {
  const alt = rec({ updated: 1, heard: [[0, 60]], source_hash: "g" });
  const neu = rec({ updated: 2, heard: [[0, 4]], source_hash: "h" });
  assertActive(alt, neu, neu);
  // Argumentreihenfolge ändert nichts.
  assertActive(neu, alt, neu);
});

test("unvereinbare Hashes: auch gelesene Bereiche und Fertig-Zustand bleiben beim neueren", () => {
  for (const [alterHash, neuerHash] of [
    ["h", undefined],
    ["h", ""],
    [undefined, "h"],
    ["g", "h"],
  ]) {
    const alt = rec({
      updated: 1,
      length: 200,
      read: [[0, 90]],
      frontier: 90,
      finished: true,
      source_hash: alterHash,
      source_edition: "alt",
    });
    const neu = rec({
      updated: 2,
      offset: 12,
      read: [[10, 12]],
      source_hash: neuerHash,
    });
    for (const [a, b] of [[alt, neu], [neu, alt]]) {
      // Ohne Koordinatenprüfung würden alte Bereiche und Stempel weitergetragen.
      assertActive(a, b, neu);
    }
  }
});

test("Gleichstand der Zeit und unvereinbar: der erste (lokale) Satz bleibt aktiv", () => {
  const lokal = rec({ updated: 3, heard: [[0, 10]] });
  const fern = rec({ updated: 3, heard: [[0, 99]], source_hash: "h" });
  assertActive(lokal, fern, lokal);
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
  assert.deepEqual(pos.heard, []); // kein Rohbereich ab einer normalisierten Stelle
  assert.deepEqual(pos.read, []);
  assert.equal(loadProgress(schluessel).frontier, 0);
  assert.equal(pos.finished, true);

  lege();
  const gelesen = recordRead(schluessel, 60, 70, 100);
  assert.equal(gelesen.source_hash, undefined);
  assert.deepEqual(gelesen.heard, []);
  assert.deepEqual(gelesen.read, [[60, 70]]);
  assert.equal(loadProgress(schluessel).frontier, 100, "fertiger Rohtext bleibt bis zum Ende bekannt");

  lege();
  setFinished(schluessel, false);
  const fertig = loadProgress(schluessel);
  assert.equal(fertig.offset, 0);
  assert.equal(fertig.source_hash, undefined);
  assert.deepEqual(fertig.heard, []);
  assert.deepEqual(fertig.read, [[60, 70]], "früherer Rohtext-Stand bleibt nach Rückkehr erhalten");
  assert.equal(fertig.frontier, 100);
  assert.equal(fertig.finished, false);
  assert.deepEqual(fertig.coordinate_history["hash:h"].heard, [[0, 40]]);

  // Gleiche Koordinaten: die gespeicherte Grenze zählt weiter.
  saveProgress(schluessel, rec({ heard: [[0, 80]], frontier: 80 }));
  saveProgress(schluessel, rec({ heard: [[0, 20]] }));
  assert.equal(loadProgress(schluessel).frontier, 100);
});

test("CRLF: normalisierte Stelle 4 wird nicht als Rohstelle vor B bei 6 übernommen", () => {
  const text = "A.\r\n\r\nB.\r\n";
  const normalisiert = text.replaceAll("\r\n", "\n");
  const schluessel = "text:crlf:original";
  assert.equal(normalisiert.indexOf("B"), 4);
  assert.equal(text.indexOf("B"), 6);
  const lege = () =>
    saveProgress(
      schluessel,
      rec({
        offset: 4,
        length: normalisiert.length,
        heard: [[0, 4]],
        source_hash: "h",
      }),
    );

  lege();
  const pos = recordPosition(schluessel, { to: 6, length: text.length });
  assert.equal(pos.offset, 6);
  assert.equal(pos.length, text.length);
  assert.equal(pos.source_hash, undefined);
  assert.deepEqual(pos.heard, []);

  lege();
  setFinished(schluessel, false);
  const fertig = loadProgress(schluessel);
  assert.equal(fertig.offset, 0);
  assert.equal(fertig.source_hash, undefined);
  assert.deepEqual(fertig.heard, []);
  assert.equal(fertig.length, 0); // Gegenprobe: keine Länge aus normalisierten Koordinaten
  const danach = recordPosition(schluessel, { to: 6, length: text.length });
  assert.equal(danach.length, text.length);
  assert.deepEqual(danach.heard, [[0, 6]]);
});
