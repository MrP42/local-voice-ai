// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/turnTaking.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  TurnTaker,
  type TurnEvent,
} from "../../src/components/voice-call/turnTaking.ts";

const FRAME = 20;
function run(t: TurnTaker, plan: Array<[number, number, boolean?]>): TurnEvent[] {
  // plan: [Dauer ms, Pegel, KI spricht]
  const ev: TurnEvent[] = [];
  let at = 0;
  for (const [dur, level, ai] of plan) {
    for (let i = 0; i < dur / FRAME; i++) {
      at += FRAME;
      ev.push(...t.frame(at, level, ai ?? false));
    }
  }
  return ev;
}
const kinds = (ev: TurnEvent[]) => ev.map((e) => e.kind);

test("Satz: Sprechbeginn, früh, Ende", () => {
  const ev = run(new TurnTaker(), [[1000, 0.002], [1500, 0.1], [1400, 0.002]]);
  assert.deepEqual(kinds(ev), ["speech_start", "early_end", "end"]);
});

test("Weitersprechen nach kurzer Pause verwirft die Vorarbeit", () => {
  const ev = run(new TurnTaker(), [
    [500, 0.002], [800, 0.1], [800, 0.002], [800, 0.1], [1400, 0.002],
  ]);
  assert.deepEqual(kinds(ev), ["speech_start", "early_end", "resume", "early_end", "end"]);
});

test("kurzes Klacken löst keinen Sprechbeginn aus", () => {
  const ev = run(new TurnTaker(), [[500, 0.002], [60, 0.2], [1000, 0.002]]);
  assert.deepEqual(kinds(ev), []);
});

test("Unterbrechen nur bei anhaltender Sprache während die KI spricht", () => {
  const t = new TurnTaker();
  const ev = run(t, [[500, 0.002, true], [100, 0.2, true], [200, 0.002, true], [600, 0.2, true]]);
  assert.equal(kinds(ev).filter((k) => k === "barge_in").length, 1);
});

test("leises Echo während die KI spricht unterbricht nicht", () => {
  const ev = run(new TurnTaker(), [[500, 0.002, true], [2000, 0.02, true]]);
  assert.deepEqual(kinds(ev), []);
});

test("Unterbrechen abgeschaltet", () => {
  const t = new TurnTaker({ ...new TurnTaker().config, bargeIn: false });
  const ev = run(t, [[500, 0.002, true], [1000, 0.3, true]]);
  assert.equal(kinds(ev).includes("barge_in"), false);
});

for (const level of [0.025, 0.1]) {
  test(`800 ms Satzpause bei Pegel ${level} beendet die Äußerung nicht`, () => {
    const ev = run(new TurnTaker(), [
      [500, 0.002], [1000, level], [800, 0.002], [800, level],
    ]);
    assert.deepEqual(kinds(ev), ["speech_start", "early_end", "resume"]);
  });
}

test("leise Silben bleiben relativ zum bisherigen Sprachpegel Sprache", () => {
  const ev = run(new TurnTaker(), [
    [500, 0.002], [1000, 0.025], [1600, 0.009],
  ]);
  assert.deepEqual(kinds(ev), ["speech_start"]);
});

test("steigender Rauschboden verdeckt ein leises Mikro nicht", () => {
  const taker = new TurnTaker();
  const ev = run(taker, [
    [6000, 0.006], [6000, 0.008], [6000, 0.011], [1000, 0.025],
  ]);
  assert.deepEqual(kinds(ev), ["speech_start"]);
  assert.ok(taker.threshold(false) < 0.025);
});

test("alte benutzerdefinierte Pausen werden auf sichere Mindestwerte geklemmt", () => {
  const taker = new TurnTaker({
    ...new TurnTaker().config,
    earlyEndMs: 280,
    endMs: 550,
  });
  const ev = run(taker, [[500, 0.002], [1000, 0.1], [1400, 0.002]]);
  const early = ev.find((event) => event.kind === "early_end");
  const end = ev.find((event) => event.kind === "end");
  assert.ok(early?.kind === "early_end");
  assert.ok(end?.kind === "end");
  assert.ok(early.at - early.lastSpeechAt >= 500);
  assert.ok(end.at - end.lastSpeechAt >= 1000);
});

for (const pause of [800, 1400, 2080]) {
  test(`Fortsetzung nach ${pause} ms trotz Sofort-Sprecher bleibt derselbe Turn`, () => {
    const ev = run(new TurnTaker(), [
      [500, 0.002], [1000, 0.025], [pause, 0.002, true], [600, 0.025, true],
    ]);
    assert.equal(kinds(ev).filter((kind) => kind === "speech_start").length, 1);
    assert.equal(kinds(ev).filter((kind) => kind === "resume").length, 1);
    assert.equal(kinds(ev).includes("barge_in"), false);
  });
}

test("nach dem Fortsetzungsfenster bleibt echte Unterbrechung ein Barge-in", () => {
  const ev = run(new TurnTaker(), [
    [500, 0.002], [1000, 0.1], [2200, 0.002, true], [600, 0.1, true],
  ]);
  assert.equal(kinds(ev).filter((kind) => kind === "speech_start").length, 2);
  assert.equal(kinds(ev).filter((kind) => kind === "barge_in").length, 1);
  assert.equal(kinds(ev).includes("resume"), false);
});

test("Reset entfernt das Fortsetzungsfenster", () => {
  const taker = new TurnTaker();
  run(taker, [[500, 0.002], [1000, 0.1], [800, 0.002]]);
  taker.reset();
  const ev = run(taker, [[600, 0.1, true]]);
  assert.deepEqual(kinds(ev), ["speech_start", "barge_in"]);
});

test("Diagnose ist abschaltbar und enthält genau einen Eintrag je Ereignis", () => {
  const original = console.debug;
  const entries: unknown[][] = [];
  console.debug = (...args: unknown[]) => {
    entries.push(args);
  };
  try {
    const plan: Array<[number, number]> = [
      [500, 0.002], [1000, 0.1], [1400, 0.002],
    ];
    run(new TurnTaker(), plan);
    assert.equal(entries.length, 0);
    const ev = run(new TurnTaker({ ...new TurnTaker().config, debug: true }), plan);
    assert.equal(entries.length, ev.length);
    for (const [index, entry] of entries.entries()) {
      assert.equal(entry[0], "[voice-call:turn]");
      const data = entry[1] as Record<string, unknown>;
      assert.deepEqual(Object.keys(data).sort(), [
        "at", "frameAt", "kind", "lastSpeechAt", "noise", "rms", "speechLevel", "threshold",
      ]);
      assert.equal(data.kind, ev[index].kind);
      for (const [key, value] of Object.entries(data)) {
        if (key !== "kind") assert.equal(typeof value, "number");
      }
    }
  } finally {
    console.debug = original;
  }
});
