import * as assert from "node:assert/strict";
import { test } from "node:test";
import { VadFeed } from "../../src/components/voice-call/vadFeed.ts";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 12; i++) await Promise.resolve();
}

function harness(waitForOpen = false) {
  const opens: ReturnType<typeof deferred<string>>[] = [];
  const pushes: {
    sessionId: unknown;
    marker: unknown;
    result: ReturnType<typeof deferred<number[]>>;
  }[] = [];
  const closes: unknown[] = [];
  const evidence: [number, number][] = [];
  let fallbacks = 0;
  let closeFails = false;
  const invoke = async <T>(command: string, args?: Record<string, unknown>): Promise<T> => {
    if (command === "voice_call_vad_open") {
      const result = deferred<string>();
      opens.push(result);
      if (!waitForOpen) result.resolve(`session-${opens.length}`);
      return (await result.promise) as T;
    }
    if (command === "voice_call_vad_push") {
      const result = deferred<number[]>();
      pushes.push({ sessionId: args?.sessionId, marker: args?.pcm16B64, result });
      return (await result.promise) as T;
    }
    assert.equal(command, "voice_call_vad_close");
    closes.push(args?.sessionId);
    if (closeFails) throw new Error("close");
    return undefined as T;
  };
  const feed = new VadFeed(
    invoke,
    (samples) => {
      assert.equal(samples.length, 1600);
      return String(samples[0]);
    },
    (at, prob) => evidence.push([at, prob]),
    () => fallbacks++,
  );
  function chunk(index: number, origin = 0): void {
    for (let i = 0; i < 5; i++) {
      feed.frame(new Float32Array(320).fill(index), origin + index * 100 + i * 20);
    }
  }
  return {
    feed,
    opens,
    pushes,
    closes,
    evidence,
    chunk,
    fallbacks: () => fallbacks,
    failClose: () => { closeFails = true; },
  };
}

test("fünf Capture-Frames bilden ein Häppchen; Öffnen blockiert die Zufuhr nicht", async () => {
  const h = harness(true);
  h.feed.start();
  for (let i = 0; i < 4; i++) h.feed.frame(new Float32Array(320), i * 20);
  h.opens[0].resolve("ready");
  await flush();
  assert.equal(h.pushes.length, 0);
  h.feed.frame(new Float32Array(320), 80);
  assert.equal(h.pushes.length, 1);
  assert.equal(h.pushes[0].sessionId, "ready");
  h.pushes[0].result.resolve([0.1, 0.2, 0.3]);
  await flush();
  assert.deepEqual(h.evidence, [[0, 0.1], [30, 0.2], [60, 0.3]]);
  h.feed.stop();
});

test("100-ms-Häppchen behalten das 30-ms-Raster samt Rust-Rest", async () => {
  const h = harness();
  h.feed.start();
  await flush();
  for (let i = 0; i < 3; i++) h.chunk(i, 1000);
  h.pushes[0].result.resolve([0.1, 0.2, 0.3]);
  await flush();
  h.pushes[1].result.resolve([0.4, 0.5, 0.6]);
  await flush();
  h.pushes[2].result.resolve([0.7, 0.8, 0.9, 1]);
  await flush();
  assert.deepEqual(h.evidence.map(([at]) => at),
    [1000, 1030, 1060, 1090, 1120, 1150, 1180, 1210, 1240, 1270]);
  assert.deepEqual(h.evidence.map(([, prob]) => prob),
    [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1]);
  h.feed.stop();
});

test("Pushes laufen seriell in Capture-Reihenfolge", async () => {
  const h = harness();
  h.feed.start();
  await flush();
  for (let i = 0; i < 3; i++) h.chunk(i);
  assert.equal(h.pushes.length, 1);
  for (let i = 0; i < 3; i++) {
    assert.equal(h.pushes[i].marker, String(i));
    h.pushes[i].result.resolve(Array(i === 2 ? 4 : 3).fill(0.5));
    await flush();
    assert.equal(h.pushes.length, Math.min(i + 2, 3));
  }
  h.feed.stop();
});

test("höchstens zehn wartende Häppchen; die ältesten entfallen samt Zeitlücke", async () => {
  const h = harness();
  h.feed.start();
  await flush();
  for (let i = 0; i < 15; i++) h.chunk(i);
  assert.equal(h.pushes.length, 1);
  h.pushes[0].result.resolve([0.5, 0.5, 0.5]);
  await flush();
  assert.equal(h.pushes[1].marker, "5");
  h.pushes[1].result.resolve([0.5, 0.5, 0.5]);
  await flush();
  assert.deepEqual(h.evidence.map(([at]) => at), [0, 30, 60, 90, 520, 550]);
  for (let i = 2; i < 11; i++) {
    assert.equal(h.pushes[i].marker, String(i + 4));
    h.pushes[i].result.resolve(Array(i % 3 === 2 ? 4 : 3).fill(0.5));
    await flush();
  }
  assert.equal(h.pushes.length, 11);
  h.feed.stop();
});

test("Reset verwirft Antwort, Warteschlange und Teilstück der alten Sitzung", async () => {
  const h = harness();
  h.feed.start();
  await flush();
  h.chunk(0);
  h.chunk(1);
  h.feed.frame(new Float32Array(320).fill(99), 200);
  h.feed.reset();
  await flush();
  h.chunk(3);
  assert.equal(h.pushes.length, 1);
  h.pushes[0].result.resolve([1, 1, 1]);
  await flush();
  assert.deepEqual(h.evidence, []);
  assert.equal(h.pushes.length, 2);
  assert.equal(h.pushes[1].sessionId, "session-2");
  assert.equal(h.pushes[1].marker, "3");
  h.pushes[1].result.resolve([0.2, 0.3, 0.4]);
  await flush();
  assert.deepEqual(h.evidence, [[300, 0.2], [330, 0.3], [360, 0.4]]);
  assert.deepEqual(h.closes, ["session-1"]);
  h.feed.stop();
});

test("alter Push-Fehler deaktiviert eine neue Sitzung nicht", async () => {
  const h = harness();
  h.feed.start();
  await flush();
  h.chunk(0);
  h.feed.reset();
  await flush();
  h.chunk(1);
  h.pushes[0].result.reject("Beschaeftigt");
  await flush();
  assert.equal(h.fallbacks(), 0);
  assert.equal(h.pushes.length, 2);
  h.pushes[1].result.resolve([1, 1, 1]);
  await flush();
  assert.equal(h.evidence.length, 3);
  h.feed.stop();
});

test("Push-Fehler deaktiviert bis zum nächsten Gespräch und meldet nur einmal", async () => {
  const h = harness();
  h.feed.start();
  await flush();
  h.chunk(0);
  h.chunk(1);
  h.pushes[0].result.reject("Beschaeftigt");
  await flush();
  h.feed.reset();
  h.chunk(2);
  await flush();
  assert.equal(h.pushes.length, 1);
  assert.equal(h.opens.length, 1);
  assert.equal(h.fallbacks(), 1);
  assert.deepEqual(h.closes, ["session-1"]);
  assert.deepEqual(h.evidence, []);
  h.feed.start();
  await flush();
  h.chunk(3);
  assert.equal(h.pushes.length, 2);
  h.pushes[1].result.resolve([1, 1, 1]);
  await flush();
  assert.equal(h.evidence.length, 3);
  h.feed.stop();
});

test("Open-Fehler fällt einmal auf RMS zurück und leert die Zufuhr", async () => {
  const h = harness(true);
  h.feed.start();
  h.chunk(0);
  h.opens[0].reject("ModellFehlt");
  await flush();
  h.feed.reset();
  h.chunk(1);
  assert.equal(h.fallbacks(), 1);
  assert.equal(h.opens.length, 1);
  assert.equal(h.pushes.length, 0);
  h.feed.stop();
});

test("Stop schließt einmal, ignoriert Close-Fehler und verwirft späte Antworten", async () => {
  const h = harness();
  h.feed.start();
  await flush();
  h.chunk(0);
  h.chunk(1);
  h.failClose();
  h.feed.stop();
  h.feed.stop();
  h.pushes[0].result.resolve([1, 1, 1]);
  await flush();
  assert.deepEqual(h.closes, ["session-1"]);
  assert.deepEqual(h.evidence, []);
  assert.equal(h.pushes.length, 1);
  assert.equal(h.fallbacks(), 0);
});

test("verspätetes Open nach Sitzungswechsel oder Stop wird nur geschlossen", async () => {
  const h = harness(true);
  h.feed.start();
  h.chunk(0);
  h.feed.start();
  h.opens[0].resolve("old");
  h.opens[1].resolve("new");
  await flush();
  assert.deepEqual(h.closes, ["old"]);
  assert.equal(h.pushes.length, 0);
  h.feed.reset();
  h.feed.stop();
  h.opens[2].resolve("late");
  await flush();
  assert.deepEqual(h.closes, ["old", "new", "late"]);
  assert.equal(h.fallbacks(), 0);
});
