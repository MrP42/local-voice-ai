// Lva#485: Anhalten und Fortsetzen des echten SpeechPlayer, gegen einen nachgeahmten AudioContext.
// Lauf: node --test scripts/ergaenzen-player.test.mjs (Node >= 22.6).
import assert from "node:assert/strict";
import test from "node:test";

const { SpeechPlayer } = await import("../src/components/voice-call/audio.ts");

/** AudioContext mit steuerbarer Uhr; jede Quelle merkt sich Start, Versatz und Stopp. */
function ctxMit(dauern) {
  let n = 0;
  const ctx = {
    currentTime: 0,
    sources: [],
    destination: {},
    createGain: () => ({ connect() {}, gain: { value: 1 } }),
    decodeAudioData: async () => ({ duration: dauern[n++] }),
    createBufferSource: () => {
      const src = {
        buffer: null,
        playbackRate: { value: 1 },
        connect() {},
        onended: null,
        startAt: null,
        offset: null,
        stopped: false,
        start(at, offset) {
          src.startAt = at;
          src.offset = offset;
        },
        stop() {
          src.stopped = true;
        },
        /** Quelle läuft von selbst zu Ende (Test). */
        ende() {
          src.onended?.();
        },
      };
      ctx.sources.push(src);
      return src;
    },
  };
  return ctx;
}
const wav = Buffer.from("x").toString("base64");
const nah = (a, b) => assert.ok(Math.abs(a - b) < 1e-9, `${a} ~ ${b}`);

test("Pause hält sofort an und merkt sich die Stelle; Fortsetzen spielt den Rest in Reihenfolge", async () => {
  const ctx = ctxMit([2, 2]);
  const p = new SpeechPlayer(ctx);
  const ende = [];
  await p.enqueue(wav, () => ende.push(1));
  await p.enqueue(wav, () => ende.push(2));
  assert.equal(ctx.sources.length, 2);
  nah(ctx.sources[0].startAt, 0.02);
  nah(ctx.sources[1].startAt, 2.02);
  ctx.currentTime = 1;
  assert.equal(p.pause(), true);
  assert.equal(p.paused, true);
  assert.equal(p.active, true);
  assert.equal(ctx.sources[0].stopped && ctx.sources[1].stopped, true);
  assert.deepEqual(ende, [], "nichts davon ist zu Ende");
  assert.equal(p.pause(), false, "zweites Anhalten ändert nichts");
  ctx.currentTime = 5;
  p.resume();
  assert.equal(p.paused, false);
  assert.equal(ctx.sources.length, 4);
  nah(ctx.sources[2].startAt, 5.02);
  nah(ctx.sources[2].offset, 0.98);
  nah(ctx.sources[3].startAt, 5.02 + 1.02);
  nah(ctx.sources[3].offset, 0);
});

test("Pause nach dem Ende des ersten Häppchens: es gilt als beendet, der Rest läuft ab seiner Stelle", async () => {
  const ctx = ctxMit([2, 2]);
  const p = new SpeechPlayer(ctx);
  const ende = [];
  await p.enqueue(wav, () => ende.push(1));
  await p.enqueue(wav, () => ende.push(2));
  ctx.currentTime = 2.5;
  p.pause();
  assert.deepEqual(ende, [1]);
  ctx.currentTime = 4;
  p.resume();
  const letzte = ctx.sources.at(-1);
  assert.equal(ctx.sources.length, 3);
  nah(letzte.offset, 0.48);
  nah(letzte.startAt, 4.02);
});

test("Eintreffendes bei Pause wartet; Fortsetzen spielt es; das Ende des letzten meldet onIdle", async () => {
  const ctx = ctxMit([1, 1]);
  const p = new SpeechPlayer(ctx);
  let idle = 0;
  let erster = 0;
  p.onIdle = () => idle++;
  p.onFirstSound = () => erster++;
  await p.enqueue(wav);
  ctx.currentTime = 0.5;
  p.pause();
  await p.enqueue(wav);
  assert.equal(ctx.sources.length, 1, "nicht eingeplant, solange angehalten");
  p.resume();
  assert.equal(ctx.sources.length, 3);
  assert.equal(erster, 1, "der erste Ton wird nur einmal gemeldet");
  ctx.sources[2].ende();
  assert.equal(idle, 0);
  ctx.sources[1].ende();
  assert.equal(idle, 1);
});

test("Fortsetzen ohne Rest meldet onIdle (die Antwort war schon fertig gespielt)", async () => {
  const ctx = ctxMit([1]);
  const p = new SpeechPlayer(ctx);
  let idle = 0;
  p.onIdle = () => idle++;
  await p.enqueue(wav);
  ctx.currentTime = 3;
  p.pause();
  assert.equal(p.active, false);
  p.resume();
  assert.equal(idle, 1);
});

test("stop hebt die Pause auf und verwirft alles; das Ende eines Häppchens meldet nichts mehr", async () => {
  const ctx = ctxMit([2]);
  const p = new SpeechPlayer(ctx);
  const ende = [];
  await p.enqueue(wav, () => ende.push(1));
  p.pause();
  p.stop();
  assert.equal(p.paused, false);
  assert.equal(p.active, false);
  p.resume();
  assert.equal(ctx.sources.length, 1);
  assert.deepEqual(ende, []);
});

// Lva#509 Befund 5: „gehört“ heißt tatsächlich begonnen (Startzeit der Quelle gegen die Uhr), nicht eingeplant.
test("Eingeplant ist noch nicht hörbar: erst wenn die Uhr die Startzeit der Quelle erreicht, gilt das Häppchen als begonnen", async () => {
  const ctx = ctxMit([2, 2]);
  const p = new SpeechPlayer(ctx);
  const begonnen = [];
  await p.enqueue(wav, undefined, () => begonnen.push(1));
  await p.enqueue(wav, undefined, () => begonnen.push(2));
  p.syncStarts();
  assert.deepEqual(begonnen, [], "beide sind eingeplant, keines ist zu hören");
  ctx.currentTime = 0.05;
  p.syncStarts();
  assert.deepEqual(
    begonnen,
    [1],
    "das erste läuft, das zweite ist erst bei 2,02 s dran",
  );
  p.syncStarts();
  assert.deepEqual(
    begonnen,
    [1],
    "jedes Häppchen meldet seinen Beginn nur einmal",
  );
  ctx.currentTime = 2.5;
  p.syncStarts();
  assert.deepEqual(begonnen, [1, 2]);
});

test("Pause zwischen Einplanen und Audiostart: nichts gilt als begonnen", async () => {
  const ctx = ctxMit([2]);
  const p = new SpeechPlayer(ctx);
  const begonnen = [];
  await p.enqueue(wav, undefined, () => begonnen.push(1));
  ctx.currentTime = 0.01;
  assert.equal(p.pause(), true);
  assert.deepEqual(begonnen, []);
  p.syncStarts();
  assert.deepEqual(begonnen, [], "angehalten läuft nichts an");
});

test("Pause mitten im ersten Häppchen: es gilt als begonnen, das nächste nicht", async () => {
  const ctx = ctxMit([2, 2]);
  const p = new SpeechPlayer(ctx);
  const begonnen = [];
  await p.enqueue(wav, undefined, () => begonnen.push(1));
  await p.enqueue(wav, undefined, () => begonnen.push(2));
  ctx.currentTime = 1;
  p.pause();
  assert.deepEqual(begonnen, [1]);
});

test("Läuft ein Häppchen zu Ende, ohne dass jemand nachgefragt hat, meldet es vorher seinen Beginn", async () => {
  const ctx = ctxMit([2]);
  const p = new SpeechPlayer(ctx);
  const ereignisse = [];
  await p.enqueue(
    wav,
    () => ereignisse.push("ende"),
    () => ereignisse.push("start"),
  );
  ctx.sources[0].ende();
  assert.deepEqual(ereignisse, ["start", "ende"]);
});
