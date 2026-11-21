// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/turnTaking.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  TurnTaker,
  withoutTurnHistory,
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

test("Dauerpegel unter minLevel beendet auch eine leise Äußerung", () => {
  const ev = run(new TurnTaker(), [
    [500, 0.002], [1000, 0.025], [1600, 0.009],
  ]);
  assert.deepEqual(kinds(ev), ["speech_start", "early_end", "end"]);
});

test("Lüfterrauschen nach der Einschwingzeit löst keinen Sprechbeginn aus", () => {
  const ev = run(new TurnTaker(), [[6000, 0.03], [6000, 0.03]]);
  assert.equal(ev.some((event) => event.kind === "speech_start" && event.at > 6000), false);
  assert.equal(ev.some((event) => event.kind === "end" && event.at <= 6000), true);
});

test("Durchgehende Sprache wird auch nach zehn Sekunden nicht als Rauschen gelernt", () => {
  const taker = new TurnTaker();
  const ev = run(taker, [[500, 0.002], [10000, 0.1]]);
  assert.deepEqual(kinds(ev), ["speech_start"]);
  assert.equal(taker.isSpeaking, true);
});

for (const quietMs of [280, 320]) {
  test(`${quietMs} ms vorherige Ruhe entscheiden über das Lernen eines Dauerpegels`, () => {
    const taker = new TurnTaker();
    const ev = run(taker, [[quietMs, 0.002], [10000, 0.1]]);
    assert.equal(kinds(ev).includes("end"), quietMs < 300);
    assert.equal(taker.isSpeaking, quietMs >= 300);
  });
}

test("Reset entfernt auch die zuvor beobachtete Ruhephase", () => {
  const taker = new TurnTaker();
  run(taker, [[500, 0.002]]);
  taker.reset();
  const ev = run(taker, [[6000, 0.03], [6000, 0.03]]);
  assert.equal(ev.some((event) => event.kind === "end" && event.at <= 6000), true);
  assert.equal(ev.some((event) => event.kind === "speech_start" && event.at > 6000), false);
  assert.equal(taker.isSpeaking, false);
});

test("Die Halteschwelle bleibt trotz eingelerntem Boden unter der Sprachreferenz", () => {
  const taker = new TurnTaker();
  const ev = run(taker, [[6000, 0.011], [1000, 0.04]]);
  assert.deepEqual(kinds(ev), ["speech_start"]);
  assert.ok(taker.threshold(false) <= 0.04 * 0.6);
});

test("Schwankendes Lüfterrauschen beendet Sprache innerhalb von endMs plus Puffer", () => {
  const taker = new TurnTaker();
  const plan: Array<[number, number]> = [[500, 0.002], [1000, 0.12]];
  for (let i = 0; i < 85; i++) {
    plan.push([100, i % 2 === 0 ? 0.025 : 0.035]);
  }
  const ev = run(taker, plan);
  assert.deepEqual(kinds(ev), ["speech_start", "early_end", "end"]);
  const end = ev.find((event) => event.kind === "end");
  assert.ok(end?.kind === "end");
  assert.equal(end.at - end.lastSpeechAt, taker.config.endMs);
  assert.ok(end.at <= 1500 + taker.config.endMs + 1000);
});

test("Konstantes Lüfterrauschen beendet Sprache auch nach nur 500 ms Ruhe", () => {
  const taker = new TurnTaker();
  const ev = run(taker, [[500, 0.002], [1000, 0.12], [3000, 0.03]]);
  assert.deepEqual(kinds(ev), ["speech_start", "early_end", "end"]);
  const end = ev.find((event) => event.kind === "end");
  assert.ok(end?.kind === "end");
  assert.equal(end.at - end.lastSpeechAt, taker.config.endMs);
  assert.ok(end.at <= 1500 + taker.config.endMs + 1000);
});

test("800 ms Lüfterpause nach kurzer Ruhe beendet den Turn nicht", () => {
  const taker = new TurnTaker();
  const ev = run(taker, [[500, 0.002], [1000, 0.12], [800, 0.03], [800, 0.12]]);
  assert.equal(kinds(ev).includes("end"), false);
  assert.equal(kinds(ev).filter((kind) => kind === "speech_start").length, 1);
  assert.equal(taker.isSpeaking, true);
});

for (const level of [0.02, 0.03]) {
  test(`Leise Sprache ${level} bleibt bei Rauschboden 0.004 erkennbar`, () => {
    const ev = run(new TurnTaker(), [[500, 0.004], [3000, level], [1400, 0.004]]);
    assert.deepEqual(kinds(ev), ["speech_start", "early_end", "end"]);
  });
}

for (const warmup of [0.002, 0.03]) {
  test(`Lüfterrauschen beendet Sprache auch nach Vorpegel ${warmup}`, () => {
    const taker = new TurnTaker();
    const ev = run(taker, [[6000, warmup], [1000, 0.12], [3000, 0.03]]);
    const end = ev.find((event) => event.kind === "end" && event.at > 7000);
    assert.ok(end?.kind === "end");
    assert.equal(end.at - end.lastSpeechAt, taker.config.endMs);
    assert.ok(end.at <= 7000 + taker.config.endMs + 1000);
  });
}

test("800 ms Satzpause mit Lüfterrauschen beendet den Turn nicht vor endMs", () => {
  const taker = new TurnTaker();
  const ev = run(taker, [[6000, 0.03], [1000, 0.12], [800, 0.03], [800, 0.12]]);
  assert.equal(ev.some((event) => event.kind === "end" && event.at > 6000), false);
  assert.equal(
    ev.filter((event) => event.kind === "speech_start" && event.at >= 6000).length,
    1,
  );
  assert.equal(taker.isSpeaking, true);
});

test("KI-Ausgabe lernt auch einen stabilen Dauerpegel nicht als Rauschboden", () => {
  const taker = new TurnTaker();
  const initial = taker.threshold(false);
  run(taker, [[6000, 0.02, true]]);
  assert.equal(taker.threshold(false), initial);
});

test("steigender Rauschboden hebt die Startschwelle an", () => {
  const taker = new TurnTaker();
  const initial = taker.threshold(false);
  const ev = run(taker, [
    [6000, 0.006], [6000, 0.008], [6000, 0.011],
  ]);
  assert.deepEqual(kinds(ev), []);
  assert.ok(taker.threshold(false) > initial);
  assert.ok(taker.threshold(false) > 0.025);
  // Dieses Signal liegt auch über der tatsächlich eingelernten Rauschschwelle.
  for (let at = 18020; at <= 19000; at += FRAME) {
    ev.push(...taker.frame(at, 0.04, false));
  }
  assert.deepEqual(kinds(ev), ["speech_start"]);
  assert.ok(taker.threshold(false) < 0.04);
});

for (const [speech, background] of [[0.025, 0.008], [0.1, 0.009]]) {
  test(`Dauerpegel ${background} nach Sprache ${speech} hält den Satz nicht offen`, () => {
    const taker = new TurnTaker();
    const ev = run(taker, [
      [500, 0.002], [1000, speech], [60000, background],
    ]);
    assert.deepEqual(kinds(ev), ["speech_start", "early_end", "end"]);
    const end = ev.find((event) => event.kind === "end");
    assert.ok(end?.kind === "end");
    assert.equal(end.at - end.lastSpeechAt, taker.config.endMs);
    assert.ok(end.at - 1500 < 4000);
  });
}

test("Referenzpegel fällt auch ohne akzeptierte Sprache zeitbezogen ab", () => {
  const original = console.debug;
  const entries: Array<Record<string, unknown>> = [];
  console.debug = (...args: unknown[]) => {
    entries.push(args[1] as Record<string, unknown>);
  };
  try {
    const taker = new TurnTaker({
      ...new TurnTaker().config,
      earlyEndMs: 4000,
      endMs: 10000,
      debug: true,
    });
    run(taker, [[500, 0.002], [1000, 0.5]]);
    assert.deepEqual(kinds(taker.frame(5500, 0.002, false)), ["early_end"]);
    const early = entries.find((entry) => entry.kind === "early_end");
    assert.ok(early);
    assert.ok(Math.abs(Number(early.speechLevel) - 0.25) < 0.000001);
  } finally {
    console.debug = original;
  }
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

for (const pause of [800, 1400]) {
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

test("Unterbrechung übernimmt die Sprachdauer über die Fenstergrenze", () => {
  const taker = new TurnTaker();
  const ev = run(taker, [
    [500, 0.002], [1000, 0.1], [1840, 0.002, true],
  ]);
  const early = ev.find((event) => event.kind === "early_end");
  assert.ok(early?.kind === "early_end");
  assert.equal(early.at + 1500, 3600);
  for (let at = 3360; at <= 3600; at += FRAME) {
    assert.deepEqual(kinds(taker.frame(at, 0.1, true)), []);
  }
  assert.deepEqual(kinds(taker.frame(3620, 0.1, true)), ["barge_in"]);
  const following: TurnEvent[] = [];
  for (let at = 3640; at <= 4000; at += FRAME) {
    following.push(...taker.frame(at, 0.1, true));
  }
  assert.deepEqual(kinds(following), ["speech_start"]);
  assert.equal(following[0].at, 3620);
});

test("Reset entfernt das Fortsetzungsfenster", () => {
  const taker = new TurnTaker();
  run(taker, [[500, 0.002], [1000, 0.1], [800, 0.002]]);
  taker.reset();
  const ev = run(taker, [[600, 0.1, true]]);
  assert.deepEqual(kinds(ev), ["speech_start", "barge_in"]);
});

for (const bargeIn of [false, true]) {
  test(`Echo im Fortsetzungsfenster löst kein Resume aus, bargeIn=${bargeIn}`, () => {
    const taker = new TurnTaker({ ...new TurnTaker().config, bargeIn });
    const ev = run(taker, [
      [500, 0.002], [1000, 0.025], [1400, 0.002, true], [400, 0.015, true],
    ]);
    assert.equal(kinds(ev).includes("resume"), false);
  });
}

test("Deaktiviertes Unterbrechen verhindert auch eine laute Fortsetzung", () => {
  const taker = new TurnTaker({ ...new TurnTaker().config, bargeIn: false });
  const ev = run(taker, [
    [500, 0.002], [1000, 0.025], [1400, 0.002, true], [400, 0.3, true],
  ]);
  assert.equal(kinds(ev).includes("resume"), false);
});

test("Fortsetzung braucht minSpeechMs ununterbrochene Sprache", () => {
  const taker = new TurnTaker();
  run(taker, [[500, 0.002], [1000, 0.025], [1400, 0.002]]);
  const ev: TurnEvent[] = [];
  for (let at = 2920; at <= 3020; at += FRAME) {
    ev.push(...taker.frame(at, 0.025, false));
  }
  assert.equal(kinds(ev).includes("resume"), false);
  taker.frame(3040, 0.002, false);
  for (let at = 3060; at <= 3160; at += FRAME) {
    ev.push(...taker.frame(at, 0.025, false));
  }
  assert.equal(kinds(ev).includes("resume"), false);
  assert.deepEqual(kinds(taker.frame(3180, 0.025, false)), ["resume"]);
});

test("140 ms Echo über der KI-Schwelle lösen keine Fortsetzung aus", () => {
  const ev = run(new TurnTaker(), [
    [500, 0.002], [1000, 0.025], [1400, 0.002, true], [140, 0.025, true],
  ]);
  assert.deepEqual(kinds(ev), ["speech_start", "early_end", "end"]);
});

test("Fortsetzung während KI-Ausgabe braucht bargeInMs ununterbrochene Sprache", () => {
  const taker = new TurnTaker();
  run(taker, [[500, 0.002], [1000, 0.025], [1400, 0.002, true]]);
  const ev: TurnEvent[] = [];
  for (let at = 2920; at <= 3160; at += FRAME) {
    ev.push(...taker.frame(at, 0.025, true));
  }
  assert.deepEqual(kinds(ev), []);
  taker.frame(3180, 0.002, true);
  for (let at = 3200; at <= 3440; at += FRAME) {
    ev.push(...taker.frame(at, 0.025, true));
  }
  assert.deepEqual(kinds(ev), []);
  assert.deepEqual(kinds(taker.frame(3460, 0.025, true)), ["resume"]);
  assert.deepEqual(kinds(taker.frame(3480, 0.025, true)), []);
  assert.deepEqual(kinds(taker.frame(3500, 0.025, true)), []);
});

for (const aiSpeaking of [false, true]) {
  test(`Späte Ergänzung verlängert die feste Pufferfrist nicht, KI=${aiSpeaking}`, () => {
    const taker = new TurnTaker();
    const ev = run(taker, [
      [500, 0.002], [1000, 0.025], [2060, 0.002, aiSpeaking],
    ]);
    const early = ev.find((event) => event.kind === "early_end");
    assert.ok(early?.kind === "early_end");
    const deadline = early.at + 1500;
    assert.deepEqual(kinds(taker.frame(deadline - FRAME, 0.025, aiSpeaking)), []);
    assert.deepEqual(kinds(taker.frame(deadline, 0.025, aiSpeaking)), []);
    for (let at = deadline + FRAME; at <= deadline + 400; at += FRAME) {
      ev.push(...taker.frame(at, 0.025, aiSpeaking));
    }
    assert.equal(kinds(ev).includes("resume"), false);
    const starts = ev.filter((event) => event.kind === "speech_start");
    assert.equal(starts.length, 2);
    assert.equal(starts[1].at, deadline + FRAME);
    assert.equal(
      kinds(ev).filter((kind) => kind === "barge_in").length,
      aiSpeaking ? 1 : 0,
    );
  });
}

test("Ohne hörbare KI ist Ergänzen auch bei deaktiviertem Unterbrechen erlaubt", () => {
  const taker = new TurnTaker({ ...new TurnTaker().config, bargeIn: false });
  const ev = run(taker, [
    [500, 0.002], [1000, 0.025], [1400, 0.002], [400, 0.025],
  ]);
  assert.equal(kinds(ev).filter((kind) => kind === "resume").length, 1);
});

for (const interrupted of [false, true]) {
  test(`Ersetzte Antwort verschwindet aus dem KI-Verlauf, interrupted=${interrupted}`, () => {
    const history = [
      { role: "user", turnId: "vorher", content: "gleich" },
      { role: "assistant", content: "vorher" },
      { role: "user", turnId: "alt", content: "gleich" },
      { role: "assistant", content: "Antwort", interrupted },
      { role: "user", turnId: "danach", content: "danach" },
    ];
    const result = withoutTurnHistory(history, "alt");
    assert.deepEqual(result, [history[0], history[1], history[4]]);
    assert.equal(history.length, 5);
    result.push({ role: "user", turnId: "neu", content: "gleich ergänzt" });
    assert.equal(result.filter((m) => m.content === "gleich ergänzt").length, 1);
    assert.equal(result.some((m) => m.turnId === "alt"), false);
  });
}

test("Verlaufsrücknahme lässt benachbarte Nutzer und unbekannte Kennungen stehen", () => {
  const history = [{ role: "user", turnId: "alt" }, { role: "user", turnId: "neu" }];
  assert.deepEqual(withoutTurnHistory(history, "alt"), [history[1]]);
  assert.deepEqual(withoutTurnHistory(history, "unbekannt"), history);
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
