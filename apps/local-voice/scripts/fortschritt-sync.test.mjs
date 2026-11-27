// Fortschritts-Sync (lva#358, R9): Zusammenführen von Datensätzen in verschiedenen Koordinaten.
// iPhone stempelt `source_hash` (normalisierter Text), der PC schreibt Rohzeichen ohne Stempel.
// Lauf: node --test scripts/fortschritt-sync.test.mjs (Node >= 22.6).
import assert from "node:assert/strict";
import { register } from "node:module";
import { stripTypeScriptTypes } from "node:module";
import { readFileSync } from "node:fs";
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
  beginReadingSession,
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

test("Hub-Historie: aktive Rohkoordinaten vereinigen, andere Textkoordinaten archivieren", () => {
  const local = { offset: 1, updated: 2 };
  const hub = { offset: 80, updated: 1, coordinate_history: {
    raw: { offset: 20, updated: 0, read: [[0, 20]] },
    "hash:de": { source_hash: "de", offset: 30, updated: 0, read: [[0, 30]] },
  } };
  const check = (out) => {
    assert.equal(out.offset, 1);
    assert.equal(out.frontier, 80);
    assert.deepEqual(out.read, [[0, 20]]);
    assert.equal(out.coordinate_history.raw, undefined);
    assert.equal(out.coordinate_history["hash:de"].source_hash, "de");
    assert.equal(out.coordinate_history["hash:de"].offset, 30);
    assert.deepEqual(out.coordinate_history["hash:de"].read, [[0, 30]]);
  };
  check(merge(local, hub));
  check(merge(hub, local));
  check(merge(merge(local, hub), hub));
  // Gegenprobe: ohne Pull fehlen Bereiche und archivierte Textkoordinaten.
  assert.throws(() => check(merge(local, local)), assert.AssertionError);
  // Die frühere Rust-Assertion verlangte die aktive Koordinate im Archiv.
  assert.throws(() => assert.equal(merge(local, hub).coordinate_history.raw?.offset, 20), assert.AssertionError);
});

test("Gemeinsame Budget-Fixture: gleiche Auswahl an der Grenze, auch nach erneuter Union", async () => {
  const fixture = JSON.parse(readFileSync(new URL("./fixtures/reading-log-budget.json", import.meta.url), "utf8"));
  for (const c of fixture.cases) {
    const entries = Array.from({ length: 4 }, (_, i) => ({ ...fixture.entry,
      start_ms: i + 1, future: c.padding.repeat(c.repeat) }));
    const out = merge(rec({ reading_log: entries }), rec({ reading_log: entries }));
    assert.deepEqual(out.reading_log.map(e => e.start_ms), c.retained);
    assert.deepEqual(merge(out, rec({ reading_log: entries })).reading_log, out.reading_log);
  }
  const source = readFileSync(new URL("../src/lib/library/progress.ts", import.meta.url), "utf8")
    .replace('from "./frontier"', `from "${new URL("../src/lib/library/frontier.ts", import.meta.url).href}"`)
    .replace("readingLogBudget(sorted)", "new TextEncoder().encode(JSON.stringify(sorted)).length");
  const legacy = await import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(source)).toString("base64")}`);
  const c = fixture.cases[1];
  const input = json(rec({ reading_log: Array.from({ length: 4 }, (_, i) => ({ ...fixture.entry,
    start_ms: i + 1, future: c.padding.repeat(c.repeat) })) }));
  const old = JSON.parse(legacy.mergeRecordText(input, input));
  assert.notDeepEqual(old.reading_log.map(e => e.start_ms), c.retained, "Encoderabhängige Kappung muss die Grenzprobe verletzen");
});

test("Desktop-Sitzung: monotone 30 Sekunden, stabile Kennung, persistente Checkpoints", () => {
  let wall = 1000, monotonic = 0;
  const clocks = { wall: () => wall, monotonic: () => monotonic };
  const opts = { edition: "de", medium: "text", from: 5, unit: "chars" };
  const checkpoint = beginReadingSession("synthetic", opts, clocks);
  wall = 999999; monotonic = 29999;
  checkpoint(9);
  assert.equal(loadProgress("reading:synthetic"), null);
  wall = 500; monotonic = 30000;
  checkpoint(10);
  checkpoint(8);
  let log = loadProgress("reading:synthetic").reading_log;
  assert.equal(log.length, 1);
  assert.equal(log[0].to, 10);
  assert.equal(log[0].start_ms, 1000);
  assert.ok(log[0].device.length >= 16);
  const next = beginReadingSession("synthetic", opts, clocks);
  monotonic += 30000;
  next(20);
  log = loadProgress("reading:synthetic").reading_log;
  assert.equal(log.length, 2);
  assert.equal(log[0].device, log[1].device);
});

test("Desktop-Sitzung: Wanduhrsprung vor der Schwelle verlängert das Sitzungsende nicht", async () => {
  const check = (begin) => {
    speicher.delete("lva.ui.progress.reading:synthetic-clock-jump");
    let wall = 1000, monotonic = 0;
    const checkpoint = begin("synthetic-clock-jump", {
      edition: "de", medium: "text", from: 5, unit: "chars",
    }, { wall: () => wall, monotonic: () => monotonic });
    wall = 999999; monotonic = 29999;
    checkpoint(9);
    assert.equal(loadProgress("reading:synthetic-clock-jump"), null);
    wall = 500; monotonic = 30000;
    checkpoint(8);
    let entry = loadProgress("reading:synthetic-clock-jump").reading_log[0];
    assert.equal(entry.start_ms, 1000);
    assert.equal(entry.to, 9);
    assert.equal(entry.end_ms, 1000, "Wanduhr vor der Schwelle darf das Ende nicht anheben");
    wall = 32000; monotonic = 31000;
    checkpoint(10);
    wall = 600; monotonic = 32000;
    checkpoint(8);
    entry = loadProgress("reading:synthetic-clock-jump").reading_log[0];
    assert.equal(entry.to, 10);
    assert.equal(entry.end_ms, 32000);
  };
  check(beginReadingSession);
  const source = readFileSync(new URL("../src/lib/library/progress.ts", import.meta.url), "utf8")
    .replace('from "./frontier"', `from "${new URL("../src/lib/library/frontier.ts", import.meta.url).href}"`);
  const old = source.replace(
    "if (clocks.monotonic() - start < 30_000) return;\n    entry.end_ms = Math.max(entry.end_ms, clocks.wall());",
    "entry.end_ms = Math.max(entry.end_ms, clocks.wall());\n    if (clocks.monotonic() - start < 30_000) return;",
  );
  assert.notEqual(old, source, "Gegenprobe muss die Reihenfolge zurücksetzen");
  const legacy = await import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(old)).toString("base64")}`);
  assert.throws(() => check(legacy.beginReadingSession), /Wanduhr vor der Schwelle/);
});

test("Protokoll: neueste 256, deterministische Union und Bytebudget", () => {
  const entries = Array.from({ length: 1000 }, (_, i) => ({ device: "synthetic", start_ms: i,
    end_ms: i + 30000, edition: "de", medium: "text", from: 0, to: i, unit: "chars" }));
  const a = rec({ reading_log: entries.slice(0, 700) });
  const b = rec({ reading_log: entries.slice(500) });
  const out = merge(a, b);
  assert.equal(out.reading_log.length, 256);
  assert.equal(out.reading_log[0].start_ms, 744);
  assert.deepEqual(merge(b, a).reading_log, out.reading_log);
  assert.deepEqual(merge(out, a).reading_log, out.reading_log);
  const large = rec({ reading_log: entries.map(e => ({ ...e, future: "ä".repeat(1000) })) });
  const bounded = merge(large, large);
  assert.ok(Buffer.byteLength(JSON.stringify(bounded.reading_log)) <= 120 * 1024);
  assert.ok(Buffer.byteLength(JSON.stringify(bounded)) < 256 * 1024);
  // Auch ein JSON-Umschlag mit doppelt escapten Sitzungsfeldern passt.
  const escaped = merge(rec({ reading_log: entries.map(e => ({ ...e, future: '\\"/'.repeat(1000) })) }), rec());
  assert.ok(Buffer.byteLength(JSON.stringify({ key: "reading:synthetic", value: JSON.stringify(escaped) })) < 256 * 1024);
});

test("Altclient entfernt Historie: bekannte Bereiche und Grenze bleiben monoton", async () => {
  const old = rec({ source_hash: "h", heard: [[0, 300]], read: [[300, 500]], frontier: 500,
    coordinate_history: { raw: rec({ frontier: 600 }) } });
  const legacy = rec({ source_hash: "h", updated: 2, frontier: 10 });
  const out = merge(old, legacy);
  assert.equal(out.frontier, 500);
  assert.deepEqual(out.read, old.read);
  assert.equal(out.coordinate_history.raw.frontier, 600);
  // Gegenprobe im Speicher: ein Ersatz ohne Erhaltung würde den Stand senken.
  const source = readFileSync(new URL("../src/lib/library/progress.ts", import.meta.url), "utf8")
    .replace('from "./frontier"', `from "${new URL("../src/lib/library/frontier.ts", import.meta.url).href}"`)
    .replace("const frontier = Math.max(reach(older), reach(newer));", "const frontier = reach(newer);")
    .replace("heard: mergeRanges([...(older.heard ?? []), ...(newer.heard ?? [])])", "heard: newer.heard")
    .replace("read: mergeRanges([...(older.read ?? []), ...(newer.read ?? [])])", "read: newer.read");
  const { mergeRecordText: replacement } = await import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(source)).toString("base64")}`);
  assert.notEqual(JSON.parse(replacement(json(old), json(legacy))).frontier, 500);
});

test("Reader-Lebenszyklus: Blättern, Pause, Hintergrund, Buchwechsel und Verlassen", async () => {
  const refs = [], effects = [], queued = [];
  let slot = 0, wall = 100000, mono = 0;
  const winEvents = new Map(), docEvents = new Map(), intervals = new Map();
  const originalNow = Date.now, originalPerformance = globalThis.performance;
  const originalWindow = globalThis.window, originalDocument = globalThis.document;
  globalThis.__sessionHooks = {
    useRef(value) { const i = slot++; return refs[i] ??= { current: value }; },
    useEffect(fn, deps) {
      const i = slot++, old = effects[i];
      if (!old || deps.some((d, n) => d !== old.deps[n])) queued.push(() => {
        old?.cleanup?.(); effects[i] = { deps, cleanup: fn() };
      });
    },
  };
  const source = readFileSync(new URL("../src/lib/library/useReadingSession.ts", import.meta.url), "utf8")
    .replace('import { useEffect, useRef } from "react";', "const { useEffect, useRef } = globalThis.__sessionHooks;")
    .replace('from "./progress"', `from "${new URL("../src/lib/library/progress.ts", import.meta.url).href}"`);
  const { useReadingSession } = await import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(source)).toString("base64")}`);
  const render = (id, active, at, playing = false, medium = "text") => {
    slot = 0; useReadingSession(id, "de", medium, "chars", active, at, playing);
    queued.splice(0).forEach(fn => fn());
  };
  const advance = () => { wall += 30000; mono += 30000; };
  try {
    Date.now = () => wall;
    globalThis.performance = { now: () => mono };
    globalThis.window = { ...originalWindow,
      addEventListener: (name, fn) => winEvents.set(name, fn),
      removeEventListener: name => winEvents.delete(name),
      setInterval: fn => { const id = intervals.size + 1; intervals.set(id, fn); return id; },
      clearInterval: id => intervals.delete(id),
    };
    globalThis.document = { visibilityState: "visible",
      addEventListener: (name, fn) => docEvents.set(name, fn),
      removeEventListener: name => docEvents.delete(name),
    };
    render("lifecycle-a", true, 5);
    advance(); render("lifecycle-a", true, 50);
    assert.equal(loadProgress("reading:lifecycle-a").reading_log[0].to, 50);
    render("lifecycle-b", true, 900);
    assert.equal(loadProgress("reading:lifecycle-a").reading_log[0].to, 50);
    advance(); winEvents.get("pagehide")();
    assert.equal(loadProgress("reading:lifecycle-b").reading_log[0].to, 900);
    render("lifecycle-c", true, 1, false, "kurzbuch");
    advance(); document.visibilityState = "hidden"; docEvents.get("visibilitychange")();
    const ended = loadProgress("reading:lifecycle-c").reading_log[0].end_ms;
    advance(); intervals.forEach(fn => fn());
    assert.equal(loadProgress("reading:lifecycle-c").reading_log[0].end_ms, ended);
    document.visibilityState = "visible";
    render("lifecycle-audio", true, 10, true, "audio");
    advance(); document.visibilityState = "hidden"; docEvents.get("visibilitychange")();
    advance(); intervals.forEach(fn => fn());
    const playingEnd = loadProgress("reading:lifecycle-audio").reading_log[0].end_ms;
    render("lifecycle-audio", false, 20, false, "audio");
    advance(); intervals.forEach(fn => fn());
    assert.equal(loadProgress("reading:lifecycle-audio").reading_log[0].end_ms, playingEnd);
  } finally {
    effects.forEach(e => e?.cleanup?.());
    Date.now = originalNow; globalThis.performance = originalPerformance;
    globalThis.window = originalWindow; globalThis.document = originalDocument;
    delete globalThis.__sessionHooks;
  }
});
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
