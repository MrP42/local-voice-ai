// Lva#485: Ergänzen/Korrigieren in der Engine, mit nachgeahmtem Tauri und Player (ohne Audio, ohne Modell).
// Lauf: node --test scripts/ergaenzen-engine.test.mjs (Node >= 22.6). Gleicher Lader wie assistent-engine.test.mjs.
import assert from "node:assert/strict";
import { register } from "node:module";
import test from "node:test";

register("./test-stubs/hooks.mjs", import.meta.url);
globalThis.window = {
  addEventListener() {},
  removeEventListener() {},
};
const tauri = (globalThis.__tauri = {
  calls: [],
  handlers: {},
  listeners: [],
  prepare: undefined,
});

const { VoiceCallEngine, TUNING } =
  await import("../src/components/voice-call/engine.ts");
const { DEFAULT_TURN_CONFIG } =
  await import("../src/components/voice-call/turnTaking.ts");
const { SpeechPlayer } = await import("./test-stubs/audio.ts");

const settle = (ms = 5) => new Promise((r) => setTimeout(r, ms));
function deferred() {
  let resolve;
  const promise = new Promise((r) => (resolve = r));
  return { promise, resolve };
}
const emit = (payload) => tauri.listeners.forEach((l) => l({ payload }));

/** Modell über `voice_call_chat`: streamt die Sätze und liefert den Gesamttext. */
const chatSagt =
  (...saetze) =>
  ({ requestId }) => {
    for (const text of saetze)
      emit({ kind: "sentence", request_id: requestId, text });
    emit({ kind: "done", request_id: requestId });
    return saetze.join(" ");
  };

const S1 = "Morgen wird es sonnig.";
const S2 = "Es sind zwanzig Grad.";

function neu(handlers = {}) {
  tauri.calls = [];
  tauri.listeners = [];
  tauri.prepare = undefined;
  tauri.handlers = {
    voice_call_synthesize: ({ text }) => `wav:${text}`,
    voice_call_cancel: () => null,
    voice_call_chat: chatSagt(S1, S2),
    assistent_einstufen: () => null,
    assistent_letzte_ausgabe: () => null,
    assistent_ausgabe_vergessen: () => null,
    assistent_messung_melden: () => null,
    ...handlers,
  };
  const log = { user: [], assistant: [], errors: [], states: [] };
  const engine = new VoiceCallEngine(
    {
      onState: (s) => log.states.push(s),
      onLevel() {},
      onUser: (t) => log.user.push(t),
      onAssistant: (t, final, interrupted) =>
        log.assistant.push({ t, final, interrupted }),
      onMetrics() {},
      onError: (e) => log.errors.push(e),
      conversationId: () => "c1",
    },
    { systemPrompt: "sys", turn: DEFAULT_TURN_CONFIG, orchestration: {} },
  );
  const player = new SpeechPlayer(null);
  player.onIdle = () => {
    if (engine.turn?.audible && engine.turn.done) engine.finishTurn();
  };
  engine.player = player;
  engine.state = "listening";
  engine.stopped = false;
  // Mikrofonpuffer: genug stille Frames, damit `onEarlyEnd` eine Äußerung ausschneiden kann.
  engine.frames = Array.from({ length: 200 }, () => new Float32Array(320));
  engine.frameIndex = 200;
  const aufrufe = (cmd) => tauri.calls.filter((c) => c.cmd === cmd);
  /** Gesprochene Äußerung, die gerade ihre Antwort bekommt (committed, wie nach dem Satzende). */
  const start = async (text) => {
    const turn = engine.newTurn(0, performance.now());
    turn.fromSpeech = true;
    engine.turn = turn;
    turn.committed = true;
    turn.commit();
    const done = engine
      .respond(turn, text, 0)
      .catch((e) => log.errors.push(String(e)));
    await done;
    return turn;
  };
  /** Der Nutzer redet los, während die KI spricht, und hält 260 ms durch (Pegel-Barge-in). */
  const barge = () => {
    engine.onSpeechStart(2000, true);
    engine.onBargeIn();
  };
  /** Satzende der Äußerung (früh): `ms` Sprachdauer. */
  const fruehEnde = (ms = 800) => engine.onEarlyEnd(3000, ms);
  const verlauf = () => engine.history.map((m) => `${m.role}:${m.content}`);
  const letzterChat = () => aufrufe("voice_call_chat").at(-1)?.args;
  return {
    engine,
    player,
    log,
    start,
    barge,
    fruehEnde,
    aufrufe,
    verlauf,
    letzterChat,
  };
}

// -- Rückmeldelaute (1c) ---------------------------------------------------------

test("Barge-in hält nur an: Wiedergabe pausiert, Erzeugung und Schritt laufen weiter", async () => {
  const t = neu();
  const turn = await t.start("Wie wird das Wetter morgen?");
  t.barge();
  assert.equal(t.player.paused, true);
  assert.equal(t.player.stopped, 0, "kein harter Abbruch");
  assert.equal(t.aufrufe("voice_call_cancel").length, 0);
  assert.equal(t.engine.turn, turn);
  assert.equal(turn.cancelled, false);
});

test("„Mhm“ während der Antwort: Wiedergabe läuft weiter, kein neuer Beitrag, nichts abgebrochen", async () => {
  const stt = deferred();
  const t = neu({ voice_call_transcribe: () => stt.promise });
  const turn = await t.start("Wie wird das Wetter morgen?");
  t.barge();
  t.fruehEnde();
  stt.resolve("Mhm.");
  await settle();
  assert.equal(t.player.paused, false);
  assert.equal(t.player.resumes, 1);
  assert.equal(t.player.stopped, 0);
  assert.equal(t.aufrufe("voice_call_cancel").length, 0);
  assert.equal(
    t.aufrufe("voice_call_chat").length,
    1,
    "kein zweiter Modellaufruf",
  );
  assert.deepEqual(t.log.user, ["Wie wird das Wetter morgen?"]);
  assert.equal(t.engine.turn, turn);
  assert.equal(turn.cancelled, false);
});

test("Rückmeldelaut, nachdem die Erzeugung schon fertig war: nach dem Fortsetzen endet der Schritt regulär", async () => {
  const stt = deferred();
  const t = neu({ voice_call_transcribe: () => stt.promise });
  await t.start("Wie wird das Wetter morgen?");
  t.barge();
  t.fruehEnde();
  stt.resolve("ja");
  await settle();
  t.player.finish();
  assert.equal(t.engine.turn, null);
  assert.equal(t.log.assistant.at(-1).final, true);
  assert.equal(t.log.assistant.at(-1).interrupted, false);
  assert.deepEqual(t.verlauf(), [
    "user:Wie wird das Wetter morgen?",
    `assistant:${S1} ${S2}`,
  ]);
});

test("Zu kurzes Geräusch (unter der Mindestdauer): kein Erkennen, nach dem Satzende läuft die Antwort weiter", async () => {
  const t = neu({
    voice_call_transcribe: () => "sollte nie aufgerufen werden",
  });
  await t.start("Wie wird das Wetter morgen?");
  t.barge();
  t.fruehEnde(200);
  assert.equal(t.aufrufe("voice_call_transcribe").length, 0);
  assert.equal(t.player.paused, true, "noch gehalten bis zum sicheren Ende");
  t.engine.onEnd();
  assert.equal(t.player.paused, false);
  assert.equal(t.player.resumes, 1);
});

test("Leere oder Phantom-Erkennung beim Anhalten: Wiedergabe läuft weiter", async () => {
  const t = neu({ voice_call_transcribe: () => "" });
  await t.start("Wie wird das Wetter morgen?");
  t.barge();
  t.fruehEnde();
  await settle();
  assert.equal(t.player.paused, false);
  assert.equal(t.aufrufe("voice_call_chat").length, 1);
});

test("Erkennung hängt: nach der Frist läuft die Wiedergabe weiter", async () => {
  const alt = TUNING.holdSttMaxMs;
  TUNING.holdSttMaxMs = 30;
  try {
    const t = neu({
      voice_call_transcribe: () => new Promise(() => undefined),
    });
    await t.start("Wie wird das Wetter morgen?");
    t.barge();
    t.fruehEnde();
    await settle(80);
    assert.equal(t.player.paused, false);
  } finally {
    TUNING.holdSttMaxMs = alt;
  }
});

test("Spricht der Nutzer weiter, wird das Ergebnis der alten Erkennung verworfen", async () => {
  const d1 = deferred();
  const t = neu({ voice_call_transcribe: () => d1.promise });
  await t.start("Wie wird das Wetter morgen?");
  t.barge();
  t.fruehEnde();
  t.engine.onResume();
  d1.resolve("mhm");
  await settle();
  assert.equal(
    t.player.paused,
    true,
    "weiter gehalten: die Äußerung ist noch nicht zu Ende",
  );
  assert.equal(t.player.resumes, 0);
});

// -- Stoppwörter (1d) ------------------------------------------------------------

test("„Moment“ allein: anhalten, kein neuer Beitrag, der Verlauf behält das Gesprochene", async () => {
  const t = neu({ voice_call_transcribe: () => "Moment!" });
  await t.start("Wie wird das Wetter morgen?");
  t.barge();
  t.fruehEnde();
  await settle();
  assert.equal(t.aufrufe("voice_call_chat").length, 1, "nichts ans Modell");
  assert.deepEqual(t.log.user, ["Wie wird das Wetter morgen?"]);
  assert.equal(t.aufrufe("voice_call_cancel").length >= 1, true);
  assert.equal(t.player.stopped >= 1, true);
  assert.equal(t.engine.turn, null);
  assert.equal(t.log.assistant.at(-1).interrupted, true);
  assert.equal(t.verlauf()[1], `assistant:${S1} ${S2} …`);
});

// -- Zusammenführen (1a) ---------------------------------------------------------

test("Ergänzung vor dem ersten Satzende: eine Nutzeräußerung, nichts Halbes im Verlauf", async () => {
  const stt = deferred();
  const t = neu({ voice_call_transcribe: () => stt.promise });
  const alt = await t.start("Wie wird das Wetter");
  t.barge();
  t.fruehEnde();
  t.engine.onEnd();
  stt.resolve("morgen in Hamburg");
  await settle(20);
  // Der alte Schritt ist still verworfen: kein unterbrochener Assistentenbeitrag.
  assert.equal(alt.cancelled, true);
  assert.equal(
    t.log.assistant.some((a) => a.interrupted),
    false,
  );
  assert.equal(
    t.log.assistant.some((a) => a.final && a.t.includes(S1)),
    false,
    "die Antwort auf die halbe Äußerung wird nie als fertig gemeldet",
  );
  // Die eine Äußerung geht an Modell und Anzeige.
  assert.deepEqual(t.log.user, [
    "Wie wird das Wetter",
    "Wie wird das Wetter morgen in Hamburg",
  ]);
  const messages = t.letzterChat().messages;
  assert.equal(
    messages.at(-1).content,
    "Wie wird das Wetter morgen in Hamburg",
  );
  assert.equal(messages.filter((m) => m.role === "user").length, 1);
  // Verlauf: genau ein Paar, mit der zusammengeführten Äußerung.
  assert.deepEqual(t.verlauf(), [
    "user:Wie wird das Wetter morgen in Hamburg",
    `assistant:${S1} ${S2}`,
  ]);
  assert.equal(t.engine.turn.fromSpeech, true);
  assert.equal(t.engine.turn.committed, true, "das Satzende war schon da");
});

test("Ergänzung vor dem Ende der Äußerung: der neue Schritt wird erst mit dem Satzende hörbar", async () => {
  const stt = deferred();
  const t = neu({ voice_call_transcribe: () => stt.promise });
  await t.start("Wie wird das Wetter");
  t.barge();
  t.fruehEnde();
  stt.resolve("morgen");
  await settle(20);
  const neuer = t.engine.turn;
  assert.equal(neuer.committed, false);
  assert.equal(neuer.audible, false);
  t.engine.onEnd();
  assert.equal(neuer.committed, true);
});

test("Mehrfaches Ergänzen hintereinander: die Kette ergibt eine Äußerung", async () => {
  let n = 0;
  const texte = ["zweitens", "drittens"];
  const t = neu({ voice_call_transcribe: () => texte[n++] });
  await t.start("Erstens");
  for (const erwartet of ["Erstens zweitens", "Erstens zweitens drittens"]) {
    t.barge();
    t.fruehEnde();
    t.engine.onEnd();
    await settle(20);
    assert.equal(t.log.user.at(-1), erwartet);
    assert.equal(t.engine.turn.userText, erwartet);
  }
  assert.deepEqual(t.verlauf(), [
    "user:Erstens zweitens drittens",
    `assistant:${S1} ${S2}`,
  ]);
  assert.equal(t.aufrufe("voice_call_chat").length, 3);
});

test("Antwort noch nicht fertig erzeugt (Frage ans Modell läuft): Ergänzung führt zusammen, der Verlauf bleibt leer", async () => {
  const chat = deferred();
  const ids = [];
  const t = neu({
    voice_call_chat: ({ requestId }) => {
      ids.push(requestId);
      emit({ kind: "sentence", request_id: requestId, text: S1 });
      return chat.promise;
    },
    voice_call_transcribe: () => "und morgen",
  });
  const turn = t.engine.newTurn(0, performance.now());
  turn.fromSpeech = true;
  t.engine.turn = turn;
  turn.committed = true;
  turn.commit();
  void t.engine.respond(turn, "Wie wird das Wetter", 0);
  await settle(20);
  assert.equal(t.player.enqueued.length, 1, "der erste Satz ist schon hörbar");
  t.barge();
  t.fruehEnde();
  t.engine.onEnd();
  await settle(20);
  assert.equal(turn.cancelled, true);
  assert.deepEqual(t.log.user.at(-1), "Wie wird das Wetter und morgen");
  assert.deepEqual(t.verlauf(), []);
  chat.resolve("");
  for (const id of ids) emit({ kind: "done", request_id: id });
  await settle(20);
});

test("Fertig erzeugte Antwort bleibt bis zum Ende des ersten Satzes ungespeichert (nicht final gemeldet)", async () => {
  const t = neu();
  await t.start("Wie wird das Wetter morgen?");
  assert.equal(
    t.log.assistant.some((a) => a.final),
    false,
    "noch nicht final: der Beitrag darf noch ersetzt werden",
  );
  t.player.finishPiece();
  const final = t.log.assistant.filter((a) => a.final);
  assert.equal(final.length, 1);
  assert.equal(final[0].t, `${S1} ${S2}`);
  // Ende der Wiedergabe meldet nicht noch einmal.
  t.player.finish();
  assert.equal(t.log.assistant.filter((a) => a.final).length, 1);
});

// -- Spätes Unterbrechen (1b) ------------------------------------------------------

test("Spätes Unterbrechen nach dem ersten Satz: neuer Beitrag, abgeschnittene Antwort, Hinweis ans Modell", async () => {
  const stt = deferred();
  const t = neu({ voice_call_transcribe: () => stt.promise });
  const alt = await t.start("Wie wird das Wetter morgen?");
  t.player.finishPiece();
  t.barge();
  t.fruehEnde();
  t.engine.onEnd();
  stt.resolve("und in Köln?");
  await settle(20);
  assert.equal(alt.cancelled, true);
  assert.deepEqual(t.log.user, ["Wie wird das Wetter morgen?", "und in Köln?"]);
  assert.equal(
    t.log.assistant.some((a) => a.interrupted),
    true,
  );
  assert.equal(
    t.log.assistant.some((a) => a.final && !a.interrupted),
    true,
    "die erste Antwort wurde nach dem ersten Satz regulär gemeldet",
  );
  // Verlauf: zwei Beiträge, der erste abgeschnitten.
  assert.deepEqual(t.verlauf(), [
    "user:Wie wird das Wetter morgen?",
    `assistant:${S1} ${S2} …`,
    "user:und in Köln?",
    `assistant:${S1} ${S2}`,
  ]);
  const system = t.letzterChat().messages[0];
  assert.equal(system.role, "system");
  assert.match(system.content, /^sys\n\n/);
  assert.match(system.content, /unterbrochen/);
});

test("Ohne Unterbrechung bekommt das Modell keinen Hinweis", async () => {
  const t = neu();
  await t.start("Erste Frage");
  t.player.finish();
  await t.start("Zweite Frage");
  assert.equal(t.letzterChat().messages[0].content, "sys");
});

test("Der Hinweis gilt nur für den nächsten Aufruf", async () => {
  const stt = deferred();
  const t = neu({ voice_call_transcribe: () => stt.promise });
  await t.start("Wie wird das Wetter morgen?");
  t.player.finishPiece();
  t.barge();
  t.fruehEnde();
  t.engine.onEnd();
  stt.resolve("und in Köln?");
  await settle(20);
  t.player.finish();
  await t.start("Danke");
  assert.equal(t.letzterChat().messages[0].content, "sys");
});

// -- Nicht zusammenführen: Vorlagen ---------------------------------------------------

test("S0-Vorlage wird nie zusammengeführt (sie wirkt und steht schon im Verlauf): neuer Beitrag", async () => {
  const stt = deferred();
  const t = neu({
    assistent_einstufen: () => ({
      stufe: "S0",
      absicht: "uhrzeit",
      ansage: null,
      antwort: "Es ist zehn Uhr.",
    }),
    voice_call_transcribe: () => stt.promise,
  });
  const alt = await t.start("Wie spät ist es?");
  t.barge();
  t.fruehEnde();
  t.engine.onEnd();
  tauri.handlers.assistent_einstufen = () => null;
  stt.resolve("und wie ist das Wetter");
  await settle(20);
  assert.equal(alt.cancelled, true);
  assert.equal(t.log.user.at(-1), "und wie ist das Wetter");
});
