// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/turnTaking.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import { TurnTaker, type TurnEvent } from "../../src/components/voice-call/turnTaking.ts";

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
  const ev = run(new TurnTaker(), [[1000, 0.002], [1500, 0.1], [1000, 0.002]]);
  assert.deepEqual(kinds(ev), ["speech_start", "early_end", "end"]);
});

test("Weitersprechen nach kurzer Pause verwirft die Vorarbeit", () => {
  const ev = run(new TurnTaker(), [[500, 0.002], [800, 0.1], [500, 0.002], [800, 0.1], [1000, 0.002]]);
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
