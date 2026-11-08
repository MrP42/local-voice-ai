// ADR-0028 A2: Gesprächsschritte der Engine mit nachgeahmtem Tauri (ohne Audio, ohne Modell).
// Lauf: node --test scripts/assistent-engine.test.mjs (Node >= 22.6). Der Lader (test-stubs/hooks.mjs)
// löst `@/` auf und ersetzt Tauri, den Player und die Orchestrierung; die Engine läuft unverändert.
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

const FEHLER_DE = "Das kann ich gerade nicht sicher beantworten.";
const FEHLER_EN = "I can't answer that reliably right now.";
const BRUECKE = "Moment, ich schaue kurz nach.";
const ANTWORT = "Der Termin ist um 10 Uhr.";

const settle = (ms = 5) => new Promise((r) => setTimeout(r, ms));
function deferred() {
  let resolve;
  const promise = new Promise((r) => (resolve = r));
  return { promise, resolve };
}
const ansageE = (text, stufe = "S1") => ({
  stufe,
  absicht: "kalender",
  ansage: text,
  antwort: null,
});
const s0E = (absicht, antwort) => ({
  stufe: "S0",
  absicht,
  ansage: null,
  antwort,
});
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
/** Orchestrator-Seite: ruft `onPiece` mit den Stücken (auch einer Überbrückung). */
const seite = (...stuecke) => ({
  chat: async (_id, _msgs, onPiece) => {
    for (const s of stuecke) onPiece(s);
    return stuecke.join(" ");
  },
  cancel: async () => undefined,
});

function neu(handlers = {}, { seiteMit } = {}) {
  tauri.calls = [];
  tauri.listeners = [];
  tauri.prepare = seiteMit
    ? () => ({ side: seiteMit, blocked: false, systemPrompt: "sys" })
    : undefined;
  tauri.handlers = {
    voice_call_synthesize: ({ text }) => `wav:${text}`,
    voice_call_cancel: () => null,
    assistent_einstufen: () => null,
    assistent_letzte_ausgabe: () => null,
    assistent_ausgabe_vergessen: () => null,
    assistent_messung_melden: () => null,
    ...handlers,
  };
  const log = { user: [], assistant: [], errors: [], metrics: [] };
  const engine = new VoiceCallEngine(
    {
      onState() {},
      onLevel() {},
      onUser: (t) => log.user.push(t),
      onAssistant: (t, final, interrupted) =>
        log.assistant.push({ t, final, interrupted }),
      onMetrics: (m) => log.metrics.push(m),
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
  const aufrufe = (cmd) => tauri.calls.filter((c) => c.cmd === cmd);
  /** Ein Schritt ab der erkannten Äußerung; `commit: false` = noch spekulativ. */
  const start = (text, { commit = true } = {}) => {
    const turn = engine.newTurn(0, performance.now());
    engine.turn = turn;
    if (commit) {
      turn.committed = true;
      turn.commit();
    }
    const done = engine
      .respond(turn, text, 0)
      .catch((e) => log.errors.push(String(e)));
    return { turn, done };
  };
  return { engine, player, log, start, aufrufe };
}

const verlauf = (t) => t.engine.history.map((m) => `${m.role}:${m.content}`);

// -- MUSS 4: Abbruch ---------------------------------------------------------

test("Abbruch während der Einstufung: kein Ton, kein Modellaufruf, keine Messung", async () => {
  const e = deferred();
  const t = neu({
    assistent_einstufen: () => e.promise,
    voice_call_chat: chatSagt(ANTWORT),
  });
  const { done } = t.start("Was steht im Kalender?");
  await settle();
  t.engine.interrupt();
  e.resolve(ansageE("Ich schaue in den Kalender."));
  await done;
  assert.equal(t.aufrufe("voice_call_chat").length, 0);
  assert.equal(t.aufrufe("voice_call_synthesize").length, 0);
  assert.deepEqual(t.player.enqueued, []);
  assert.equal(t.aufrufe("assistent_messung_melden").length, 0);
  assert.deepEqual(verlauf(t), []);
});

test("Abbruch während der Ansage-Synthese: nichts wird eingereiht, nichts im Verlauf", async () => {
  const syn = deferred();
  const t = neu({
    assistent_einstufen: () => ansageE("Ich schaue in den Kalender."),
    voice_call_synthesize: ({ text }) =>
      text.startsWith("Ich schaue") ? syn.promise : `wav:${text}`,
    voice_call_chat: chatSagt(ANTWORT),
  });
  const { done } = t.start("Was steht im Kalender?");
  await settle();
  assert.equal(t.aufrufe("voice_call_synthesize").length >= 1, true);
  t.engine.interrupt();
  syn.resolve("wav:ansage");
  await done;
  assert.deepEqual(t.player.enqueued, []);
  assert.deepEqual(verlauf(t), []);
  assert.equal(t.aufrufe("assistent_messung_melden").length, 0);
  assert.equal(t.aufrufe("voice_call_cancel").length >= 1, true);
});

test("Verworfener spekulativer Schritt spricht keine Ansage", async () => {
  const t = neu({
    assistent_einstufen: () => ansageE("Ich schaue in den Kalender."),
    voice_call_chat: chatSagt(ANTWORT),
  });
  const { turn, done } = t.start("Was steht im Kalender?", { commit: false });
  await settle();
  // Der Nutzer spricht weiter, bevor die KI hörbar wurde.
  t.engine.cancelTurn(false);
  await done;
  assert.deepEqual(t.player.enqueued, []);
  assert.equal(turn.heard, 0);
  assert.equal(t.aufrufe("assistent_messung_melden").length, 0);
  assert.deepEqual(verlauf(t), []);
});

// -- MUSS 4: Modellantwort fehlt ----------------------------------------------

test("Leere Modellantwort ergibt die feste Vorlage, nicht den Verlauf", async () => {
  const t = neu({
    voice_call_chat: ({ requestId }) => {
      emit({ kind: "done", request_id: requestId });
      return "";
    },
  });
  await t.start("Was steht im Kalender?").done;
  assert.deepEqual(t.player.enqueued, [`wav:${FEHLER_DE}`]);
  assert.deepEqual(verlauf(t), []);
  assert.equal(t.log.assistant.at(-1).final, true);
  assert.equal(t.log.assistant.at(-1).t, FEHLER_DE);
});

test("Abgelehntes Modell (Tor in Rust, Netzfehler) ergibt dieselbe Vorlage", async () => {
  const t = neu({
    voice_call_chat: () => Promise.reject(FEHLER_DE),
  });
  await t.start("Uhrzeit").done;
  assert.deepEqual(t.player.enqueued, [`wav:${FEHLER_DE}`]);
  assert.deepEqual(verlauf(t), []);
});

test("Vorlage der fehlenden Antwort auf Englisch, wenn die App englisch ist", async () => {
  globalThis.__lang = "en";
  try {
    const t = neu({ voice_call_chat: () => Promise.reject("x") });
    await t.start("What is on my calendar?").done;
    assert.deepEqual(t.player.enqueued, [`wav:${FEHLER_EN}`]);
  } finally {
    globalThis.__lang = "de";
  }
});

test("Kein erster Inhalt innerhalb der Frist: Anfrage abbrechen, Vorlage sprechen", async () => {
  const alt = TUNING.modelFristMs;
  TUNING.modelFristMs = 30;
  try {
    const t = neu({ voice_call_chat: () => new Promise(() => undefined) });
    const { turn, done } = t.start("Was steht im Kalender?");
    await done;
    assert.deepEqual(t.player.enqueued, [`wav:${FEHLER_DE}`]);
    assert.deepEqual(verlauf(t), []);
    assert.equal(
      t.aufrufe("voice_call_cancel").some((c) => c.args.requestId === turn.id),
      true,
    );
  } finally {
    TUNING.modelFristMs = alt;
  }
});

test("Fehlende Antwort nach einer Ansage: erst die Ansage, dann die Vorlage", async () => {
  const t = neu({
    assistent_einstufen: () => ansageE("Ich schaue in den Kalender."),
    voice_call_chat: () => Promise.reject("x"),
  });
  await t.start("Was steht im Kalender?").done;
  assert.deepEqual(t.player.enqueued, [
    "wav:Ich schaue in den Kalender.",
    `wav:${FEHLER_DE}`,
  ]);
  assert.deepEqual(verlauf(t), []);
});

// -- MUSS 4: Überbrückung -------------------------------------------------------

test("Bestehende Überbrückung des Orchestrators bleibt ohne Ansage erhalten", async () => {
  const t = neu({}, { seiteMit: seite(BRUECKE, ANTWORT) });
  await t.start("Was steht im Kalender?").done;
  assert.deepEqual(t.player.enqueued, [`wav:${BRUECKE}`, `wav:${ANTWORT}`]);
});

test("Nach einer Ansage wird keine zweite Überbrückung gesprochen", async () => {
  const t = neu(
    { assistent_einstufen: () => ansageE("Ich schaue in den Kalender.") },
    {
      seiteMit: seite(
        BRUECKE,
        "Moment, ich schaue in den Terminen nach.",
        "Moment, ich schaue im Buch nach.",
        ANTWORT,
      ),
    },
  );
  await t.start("Was steht im Kalender?").done;
  assert.deepEqual(t.player.enqueued, [
    "wav:Ich schaue in den Kalender.",
    `wav:${ANTWORT}`,
  ]);
  // Verlauf: nur was das Modell gesagt hat, ohne die Ansage.
  assert.deepEqual(verlauf(t), [
    "user:Was steht im Kalender?",
    `assistant:${ANTWORT}`,
  ]);
});

test("Zweite Überbrückung unterdrückt auch im Textmodus, die Anzeige hält die Ansage", async () => {
  const t = neu(
    { assistent_einstufen: () => ansageE("Ich schaue in den Kalender.") },
    { seiteMit: seite(BRUECKE, ANTWORT) },
  );
  t.engine.state = "idle";
  await t.engine.sendText("Was steht im Kalender?");
  const letzte = t.log.assistant.at(-1);
  assert.equal(letzte.final, true);
  assert.equal(letzte.t, `Ich schaue in den Kalender. ${ANTWORT}`);
  assert.deepEqual(verlauf(t), [
    "user:Was steht im Kalender?",
    `assistant:${ANTWORT}`,
  ]);
});

// -- MUSS 1: Einstufung gescheitert, S0 ----------------------------------------

test("Einstufung gescheitert: feste Meldung, das Modell wird nicht gefragt", async () => {
  const t = neu({
    assistent_einstufen: () => {
      throw "kaputt";
    },
    voice_call_chat: chatSagt(ANTWORT),
  });
  await t.start("Was steht im Kalender?").done;
  assert.equal(t.aufrufe("voice_call_chat").length, 0);
  assert.deepEqual(t.player.enqueued, [`wav:${FEHLER_DE}`]);
  assert.deepEqual(verlauf(t), []);
});

test("S0-Vorlage: Antwort ohne Modellaufruf, sie gehört in den Verlauf", async () => {
  const t = neu({
    assistent_einstufen: () => s0E("uhrzeit", "Es ist 14:05 Uhr."),
    voice_call_chat: chatSagt(ANTWORT),
  });
  await t.start("Wie spät ist es?").done;
  assert.equal(t.aufrufe("voice_call_chat").length, 0);
  assert.deepEqual(t.player.enqueued, ["wav:Es ist 14:05 Uhr."]);
  assert.deepEqual(verlauf(t), [
    "user:Wie spät ist es?",
    "assistant:Es ist 14:05 Uhr.",
  ]);
});

test("Schalter aus (keine Einstufung): Modell mit Gesprächskennung, Tor bleibt in Rust", async () => {
  const t = neu({
    assistent_einstufen: () => null,
    voice_call_chat: chatSagt(ANTWORT),
  });
  await t.start("Was steht im Kalender?").done;
  const [chat] = t.aufrufe("voice_call_chat");
  assert.equal(chat.args.conversationId, "c1");
  assert.deepEqual(t.engine.ausgabeKey, { id: "c1" });
  assert.deepEqual(t.player.enqueued, [`wav:${ANTWORT}`]);
});

// -- MUSS 3: Wiederholen --------------------------------------------------------

test("Wiederholen spricht die letzte freigegebene Ausgabe, ohne Modell und ohne Verlauf", async () => {
  const t = neu({
    assistent_einstufen: () => s0E("wiederholen", "Ich wiederhole."),
    assistent_letzte_ausgabe: () => "Das ist die Antwort. [Q1]",
    voice_call_chat: chatSagt(ANTWORT),
  });
  t.engine.ausgabeKey = { id: "c1" };
  await t.start("Wiederhole das bitte").done;
  assert.equal(t.aufrufe("voice_call_chat").length, 0);
  assert.deepEqual(t.aufrufe("assistent_letzte_ausgabe")[0].args, {
    conversationId: "c1",
  });
  assert.deepEqual(t.player.enqueued, [
    "wav:Ich wiederhole.",
    "wav:Das ist die Antwort.",
  ]);
  assert.deepEqual(verlauf(t), []);
  // Ein zweites Wiederholen sammelt nichts an: wieder dieselbe Ausgabe, Verlauf leer.
  t.player.finish();
  await t.start("Noch einmal").done;
  assert.equal(t.aufrufe("assistent_letzte_ausgabe").length, 2);
  assert.deepEqual(verlauf(t), []);
  assert.deepEqual(t.engine.ausgabeKey, { id: "c1" });
});

test("Wiederholen ohne Ausgabe: feste Vorlage, nichts ans Modell", async () => {
  const t = neu({
    assistent_einstufen: () => s0E("wiederholen", "Ich wiederhole."),
    voice_call_chat: chatSagt(ANTWORT),
  });
  await t.start("Wiederhole das bitte").done;
  assert.equal(t.aufrufe("voice_call_chat").length, 0);
  assert.deepEqual(t.player.enqueued, [
    "wav:Ich habe noch nichts gesagt, das ich wiederholen kann.",
  ]);
  assert.deepEqual(verlauf(t), []);
});

test("Wiederholen ohne Ausgabe auf Englisch", async () => {
  globalThis.__lang = "en-US";
  try {
    const t = neu({
      assistent_einstufen: () => s0E("wiederholen", "Ich wiederhole."),
    });
    await t.start("Repeat that").done;
    assert.deepEqual(t.player.enqueued, [
      "wav:I haven't said anything yet that I could repeat.",
    ]);
  } finally {
    globalThis.__lang = "de";
  }
});

test("Unterbrochenes Wiederholen schreibt nichts in den Verlauf", async () => {
  const t = neu({
    assistent_einstufen: () => s0E("wiederholen", "x"),
    assistent_letzte_ausgabe: () => "Das ist die Antwort.",
  });
  t.engine.ausgabeKey = { id: "c1" };
  await t.start("Wiederhole das").done;
  t.engine.cancelTurn(true);
  assert.deepEqual(verlauf(t), []);
});

test("Wiederholen im Textmodus: Anzeige ohne Verlauf", async () => {
  const t = neu({
    assistent_einstufen: () => s0E("wiederholen", "x"),
    assistent_letzte_ausgabe: () => "Das ist die Antwort.",
  });
  t.engine.state = "idle";
  t.engine.ausgabeKey = { id: "c1" };
  await t.engine.sendText("Wiederhole das");
  assert.equal(
    t.log.assistant.at(-1).t,
    "Ich wiederhole. Das ist die Antwort.",
  );
  assert.deepEqual(verlauf(t), []);
});

test("Antwort ohne Tor (Orchestrator) vergisst die letzte Ausgabe; neues Gespräch ebenso", async () => {
  const t = neu({}, { seiteMit: seite(ANTWORT) });
  t.engine.ausgabeKey = { id: "c1" };
  await t.start("Was steht im Kalender?").done;
  assert.deepEqual(t.aufrufe("assistent_ausgabe_vergessen")[0].args, {
    conversationId: "c1",
  });
  assert.equal(t.engine.ausgabeKey, null);

  t.engine.ausgabeKey = { id: "c9" };
  t.engine.resetHistory();
  assert.deepEqual(t.aufrufe("assistent_ausgabe_vergessen").at(-1).args, {
    conversationId: "c9",
  });
  assert.equal(t.engine.ausgabeKey, null);
});

// -- KANN: Messung E9 -----------------------------------------------------------

test("Messung kommt erst nach dem Ende der Wiedergabe, mit warm und weg", async () => {
  const t = neu({
    assistent_einstufen: () => ansageE("Ich schaue in den Kalender."),
    voice_call_chat: chatSagt(ANTWORT),
  });
  const erster = t.start("Was steht im Kalender?");
  await erster.done;
  assert.equal(t.aufrufe("assistent_messung_melden").length, 0);
  await settle(20);
  t.player.finish();
  const [m1] = t.aufrufe("assistent_messung_melden");
  assert.equal(m1.args.messung.beitrag_id, erster.turn.id);
  assert.equal(m1.args.messung.stufe, "S1");
  assert.equal(m1.args.messung.warm, false);
  assert.equal(m1.args.messung.weg, "relay");
  assert.equal(typeof m1.args.messung.fertig_ms, "number");
  assert.equal(typeof m1.args.messung.erste_ansage_ms, "number");
  assert.equal(
    m1.args.messung.fertig_ms >= m1.args.messung.erster_inhalt_ms,
    true,
  );

  t.engine.speakerLocal = true;
  await t.start("Und morgen?").done;
  t.player.finish();
  const m2 = t.aufrufe("assistent_messung_melden")[1].args.messung;
  assert.equal(m2.warm, true);
  assert.equal(m2.weg, "lokal");
});

test("Unterbrochener Schritt wird nicht gemessen", async () => {
  const t = neu({
    assistent_einstufen: () => ansageE("Ich schaue in den Kalender."),
    voice_call_chat: chatSagt(ANTWORT),
  });
  await t.start("Was steht im Kalender?").done;
  t.engine.cancelTurn(true);
  t.player.finish();
  assert.equal(t.aufrufe("assistent_messung_melden").length, 0);
});

test("S0-Vorlage wird als warm und lokal gemessen, erst nach der Wiedergabe", async () => {
  const t = neu({
    assistent_einstufen: () => s0E("uhrzeit", "Es ist 14:05 Uhr."),
  });
  await t.start("Wie spät ist es?").done;
  assert.equal(t.aufrufe("assistent_messung_melden").length, 0);
  t.player.finish();
  const m = t.aufrufe("assistent_messung_melden")[0].args.messung;
  assert.equal(m.stufe, "S0");
  assert.equal(m.warm, true);
  assert.equal(m.weg, "lokal");
});

test("Neuer Anruf setzt warm zurück", async () => {
  globalThis.AudioContext = class {
    async resume() {}
    async close() {}
  };
  try {
    const t = neu({
      voice_call_start: () => ({
        llm: "x",
        llm_local: true,
        warnings: [],
        warmup_ms: 0,
      }),
    });
    t.engine.player = null;
    t.engine.state = "idle";
    t.engine.warm = true;
    await t.engine.start();
    assert.equal(t.engine.warm, false);
    assert.equal(t.engine.speakerLocal, true);
    assert.deepEqual(t.engine.messArt(true), { warm: false, weg: "lokal" });
    await t.engine.stop();
  } finally {
    delete globalThis.AudioContext;
  }
});

test("Messmerkmale: Vorlagen immer warm und lokal, das Modell nach Stand", () => {
  const t = neu();
  assert.deepEqual(t.engine.messArt(false), { warm: true, weg: "lokal" });
  assert.deepEqual(t.engine.messArt(true), { warm: false, weg: "relay" });
  t.engine.warm = true;
  assert.deepEqual(t.engine.messArt(true), { warm: true, weg: "relay" });
});
