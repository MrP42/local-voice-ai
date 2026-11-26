import assert from "node:assert/strict";
import { register, stripTypeScriptTypes } from "node:module";
import { readFileSync } from "node:fs";
import test from "node:test";

register("./test-stubs/hooks.mjs", import.meta.url);

async function playbackScenario(mutate = source => source) {
  const storage = new Map(), events = new Map(), intervals = new Map();
  const original = { window: globalThis.window, document: globalThis.document, performance: globalThis.performance, now: Date.now };
  let wall = 100000, mono = 0;
  globalThis.__tauri = { calls: [], handlers: {} };
  globalThis.window = {
    localStorage: { getItem: k => storage.get(k) ?? null, setItem: (k, v) => storage.set(k, v), removeItem: k => storage.delete(k) },
    setTimeout: () => 1, clearTimeout() {},
    setInterval: fn => { const id = intervals.size + 1; intervals.set(id, fn); return id; },
    clearInterval: id => intervals.delete(id), addEventListener() {}, dispatchEvent() {},
  };
  globalThis.document = { visibilityState: "visible", addEventListener() {} };
  globalThis.__readingListen = (name, fn) => { events.set(name, fn); return Promise.resolve(() => {}); };
  Date.now = () => wall;
  globalThis.performance = { now: () => mono };
  try {
    const base = new URL("../src/components/settings/tts/readerSession.ts", import.meta.url);
    const source = mutate(readFileSync(base, "utf8"))
      .replace('import { listen } from "@tauri-apps/api/event";', "const listen = globalThis.__readingListen;")
      .replaceAll('from "./sentenceCounter"', `from "${new URL("./sentenceCounter.ts", base).href}"`);
    const session = await import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(source)).toString("base64")}`);
    assert.equal(typeof session.startReaderReading, "function", "Audioaufzeichnung muss außerhalb der Komponente leben");
    const { parseReaderDoc } = await import("../src/lib/reader/readerDoc.ts");
    const { loadProgress } = await import("../src/lib/library/progress.ts");
    session.ensureReaderSessionListeners();
    session.readerSession.active = true;
    session.startReaderReading("synthetic-playback", "original:de", "audio", parseReaderDoc("Erster Satz. Zweiter Satz. Zweiter Satz.", []), 0);
    const emit = (name, payload) => events.get(name)({ payload });
    const advance = () => { wall += 30000; mono += 30000; intervals.forEach(fn => fn()); };
    emit("tts-state-changed", { phase: "speaking" });
    advance();
    // Kein React-Render: anderer Reiter und danach ausgehängter App-Bereich.
    emit("tts-current-sentence", { context: "speak", text: "Zweiter Satz.", index: 1 });
    advance();
    const recorded = loadProgress("reading:synthetic-playback");
    assert.ok(recorded, "Ohne Komponenten-Render muss weiter aufgezeichnet werden");
    let log = recorded.reading_log;
    assert.equal(log.length, 1);
    assert.equal(log[0].edition, "original:de");
    assert.equal(log[0].medium, "audio");
    assert.ok(log[0].to > 0);
    const position = log[0].to;
    emit("tts-current-sentence", { context: "speak", text: "Zweiter Satz.", index: 1 });
    advance();
    assert.equal(loadProgress("reading:synthetic-playback").reading_log[0].to, position);
    emit("tts-state-changed", { phase: "ready" });
    const pausedEnd = loadProgress("reading:synthetic-playback").reading_log[0].end_ms;
    advance();
    assert.equal(loadProgress("reading:synthetic-playback").reading_log[0].end_ms, pausedEnd);
    emit("tts-state-changed", { phase: "speaking" });
    advance();
    session.endReaderSession();
    log = loadProgress("reading:synthetic-playback").reading_log;
    assert.equal(log.length, 2);
    const stopped = log[1].end_ms;
    advance();
    assert.equal(loadProgress("reading:synthetic-playback").reading_log[1].end_ms, stopped);
  } finally {
    globalThis.window = original.window; globalThis.document = original.document;
    globalThis.performance = original.performance; Date.now = original.now;
    delete globalThis.__readingListen;
  }
}

test("Audioquelle protokolliert ohne sichtbaren Reiter bis Pause, Fortsetzen und Stopp", async () => {
  await playbackScenario();
  await assert.rejects(playbackScenario(source => source.replace(
    'if (readerSession.active && e.payload.phase === "speaking") beginReaderReading();',
    'if (false) beginReaderReading();',
  )), /Ohne Komponenten-Render muss weiter aufgezeichnet werden/);
});
