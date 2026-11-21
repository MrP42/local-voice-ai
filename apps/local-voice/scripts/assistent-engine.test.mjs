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
  const log = {
    user: [],
    assistant: [],
    errors: [],
    metrics: [],
    denken: [],
    ende: [],
  };
  const engine = new VoiceCallEngine(
    {
      onState() {},
      onLevel() {},
      onUser: (t) => log.user.push(t),
      onAssistant: (t, final, interrupted) =>
        log.assistant.push({ t, final, interrupted }),
      onMetrics: (m) => log.metrics.push(m),
      onError: (e) => log.errors.push(e),
      onDenken: (e) => {
        if (e.art === "ende") {
          log.ende.push(e.turnId);
          return;
        }
        log.denken.push(
          e.art === "start"
            ? "start"
            : e.art === "gedanke"
              ? `gedanke:${e.text}`
              : e.text,
        );
      },
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

test("Ansage wird nie synthetisiert, sie steht nur in der Denk-Anzeige (Patrick 12.11.2026)", async () => {
  const t = neu({
    assistent_einstufen: () => ansageE("Ich schaue in den Kalender."),
    voice_call_chat: chatSagt(ANTWORT),
  });
  await t.start("Was steht im Kalender?").done;
  assert.equal(
    t
      .aufrufe("voice_call_synthesize")
      .some((c) => c.args.text.startsWith("Ich schaue")),
    false,
  );
  assert.deepEqual(t.player.enqueued, [`wav:${ANTWORT}`]);
  assert.deepEqual(t.log.denken, ["start", "Ich schaue in den Kalender."]);
  assert.equal(
    t.log.assistant.some((a) => a.t.includes("Ich schaue")),
    false,
  );
});

test("Allgemeine Ansage („Ich denke kurz nach.“) erscheint nirgends, auch nicht als Schritt", async () => {
  const t = neu({
    assistent_einstufen: () => ({
      stufe: "S2",
      absicht: "allgemein",
      ansage: "Ich denke kurz nach.",
      antwort: null,
    }),
    voice_call_chat: chatSagt("Ja, ich höre dich."),
  });
  await t.start("Kannst du mich hören?").done;
  assert.deepEqual(t.player.enqueued, ["wav:Ja, ich höre dich."]);
  assert.deepEqual(t.log.denken, ["start"]);
  assert.equal(
    t.log.assistant.some((a) => a.t.includes("denke")),
    false,
  );
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

test("Fehlende Antwort nach einer Ansage: nur die Vorlage", async () => {
  const t = neu({
    assistent_einstufen: () => ansageE("Ich schaue in den Kalender."),
    voice_call_chat: () => Promise.reject("x"),
  });
  await t.start("Was steht im Kalender?").done;
  assert.deepEqual(t.player.enqueued, [`wav:${FEHLER_DE}`]);
  assert.deepEqual(verlauf(t), []);
});

// -- MUSS 4: Überbrückung -------------------------------------------------------

test("Überbrückung des Orchestrators wird nicht gesprochen, nur Schritt der Denk-Anzeige", async () => {
  const t = neu({}, { seiteMit: seite(BRUECKE, ANTWORT) });
  await t.start("Was steht im Kalender?").done;
  assert.deepEqual(t.player.enqueued, [`wav:${ANTWORT}`]);
  assert.deepEqual(t.log.denken, ["start", BRUECKE]);
});

test("Weder Ansage noch Überbrückungen werden gesprochen", async () => {
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
  assert.deepEqual(t.player.enqueued, [`wav:${ANTWORT}`]);
  assert.equal(t.log.denken.length, 5);
  // Verlauf: nur was das Modell gesagt hat, ohne die Ansage.
  assert.deepEqual(verlauf(t), [
    "user:Was steht im Kalender?",
    `assistant:${ANTWORT}`,
  ]);
});

test("Textmodus: Ansage und Überbrückung nur in der Denk-Anzeige, nie im Chat", async () => {
  const t = neu(
    { assistent_einstufen: () => ansageE("Ich schaue in den Kalender.") },
    { seiteMit: seite(BRUECKE, ANTWORT) },
  );
  t.engine.state = "idle";
  await t.engine.sendText("Was steht im Kalender?");
  const letzte = t.log.assistant.at(-1);
  assert.equal(letzte.final, true);
  assert.equal(letzte.t, ANTWORT);
  assert.deepEqual(t.log.denken, [
    "start",
    "Ich schaue in den Kalender.",
    BRUECKE,
  ]);
  assert.deepEqual(verlauf(t), [
    "user:Was steht im Kalender?",
    `assistant:${ANTWORT}`,
  ]);
});

test("Überbrückung verlängert die Frist: Inhalt nach der ersten Frist wird noch gesprochen", async () => {
  const alt = TUNING.modelFristMs;
  TUNING.modelFristMs = 30;
  try {
    const spaet = {
      chat: async (_id, _msgs, onPiece) => {
        onPiece(BRUECKE);
        await settle(80);
        onPiece(ANTWORT);
        return `${BRUECKE} ${ANTWORT}`;
      },
      cancel: async () => undefined,
    };
    const t = neu({}, { seiteMit: spaet });
    await t.start("Was steht im Kalender?").done;
    assert.deepEqual(t.player.enqueued, [`wav:${ANTWORT}`]);
    assert.deepEqual(verlauf(t), [
      "user:Was steht im Kalender?",
      `assistant:${ANTWORT}`,
    ]);
  } finally {
    TUNING.modelFristMs = alt;
  }
});

test("Nach einer Überbrückung ohne Inhalt endet die verlängerte Frist trotzdem: Vorlage, Abbruch", async () => {
  const alt = { ...TUNING };
  TUNING.modelFristMs = 30;
  TUNING.denkerFristMs = 120;
  try {
    let abgebrochen = 0;
    const haengt = {
      chat: (_id, _msgs, onPiece) => {
        onPiece(BRUECKE);
        return new Promise(() => undefined);
      },
      cancel: async () => {
        abgebrochen++;
      },
    };
    const t = neu({}, { seiteMit: haengt });
    const start = Date.now();
    await t.start("Was steht im Kalender?").done;
    assert.ok(Date.now() - start >= 100, "die längere Frist gilt");
    assert.deepEqual(t.player.enqueued, [`wav:${FEHLER_DE}`]);
    assert.equal(abgebrochen, 1);
    assert.deepEqual(verlauf(t), []);
  } finally {
    Object.assign(TUNING, alt);
  }
});

test("Tor-Weg: eine Überbrückung wird weder gesprochen noch Teil von Chat oder Verlauf", async () => {
  const t = neu({ voice_call_chat: chatSagt(BRUECKE, ANTWORT) });
  await t.start("Was steht im Kalender?").done;
  assert.deepEqual(t.player.enqueued, [`wav:${ANTWORT}`]);
  assert.deepEqual(t.log.denken, ["start", BRUECKE]);
  assert.deepEqual(verlauf(t), [
    "user:Was steht im Kalender?",
    `assistant:${ANTWORT}`,
  ]);
});

test("Tor-Weg im Textmodus: die Rückgabe mit Überbrückung zählt nicht, nur das Angezeigte", async () => {
  const t = neu({ voice_call_chat: chatSagt(BRUECKE, ANTWORT) });
  t.engine.state = "idle";
  await t.engine.sendText("Was steht im Kalender?");
  assert.equal(t.log.assistant.at(-1).t, ANTWORT);
  assert.deepEqual(verlauf(t), [
    "user:Was steht im Kalender?",
    `assistant:${ANTWORT}`,
  ]);
});

test("Projekt: der Verlauf führt genau die registrierte Rückgabe des Tors (Sprache und Text)", async () => {
  // Überbrückungen gibt das Belegtor in Rust gar nicht frei; die Oberfläche führt die Rückgabe
  // unverändert (mit Marken), damit das Register sie beim nächsten Turn wiedererkennt.
  const RUECKGABE = `${ANTWORT} [Q1]`;
  const tor = ({ requestId }) => {
    emit({ kind: "sentence", request_id: requestId, text: RUECKGABE });
    emit({ kind: "done", request_id: requestId });
    return RUECKGABE;
  };
  const t = neu({ voice_call_chat: tor });
  t.engine.cb.projekt = () => ({ id: "p1", stand: null });
  await t.start("Was steht im Kalender?").done;
  assert.deepEqual(verlauf(t), [
    "user:Was steht im Kalender?",
    `assistant:${RUECKGABE}`,
  ]);

  const t2 = neu({ voice_call_chat: tor });
  t2.engine.cb.projekt = () => ({ id: "p1", stand: null });
  t2.engine.state = "idle";
  await t2.engine.sendText("Was steht im Kalender?");
  assert.deepEqual(verlauf(t2), [
    "user:Was steht im Kalender?",
    `assistant:${RUECKGABE}`,
  ]);
});

test("Denktext des Modells (reasoning) geht nur an die Denk-Anzeige", async () => {
  const t = neu({
    voice_call_chat: ({ requestId }) => {
      emit({ kind: "reasoning", request_id: requestId, text: "Ich prüfe " });
      emit({ kind: "reasoning", request_id: requestId, text: "den Kalender." });
      emit({ kind: "sentence", request_id: requestId, text: ANTWORT });
      emit({ kind: "done", request_id: requestId });
      return ANTWORT;
    },
  });
  await t.start("Was steht im Kalender?").done;
  assert.deepEqual(t.player.enqueued, [`wav:${ANTWORT}`]);
  assert.deepEqual(t.log.denken, [
    "start",
    "gedanke:Ich prüfe ",
    "gedanke:den Kalender.",
  ]);
  assert.deepEqual(verlauf(t), [
    "user:Was steht im Kalender?",
    `assistant:${ANTWORT}`,
  ]);
});

test("Abbruch des Schritts meldet das Ende der Denk-Anzeige", async () => {
  const t = neu({ voice_call_chat: () => new Promise(() => undefined) });
  t.engine.state = "idle";
  const p = t.engine.sendText("Was steht im Kalender?");
  await settle(20);
  t.engine.cancelText(false);
  await p;
  assert.equal(t.log.ende.length >= 1, true);
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

test("Wiederholen spricht eine gemerkte Überbrückung nicht noch einmal", async () => {
  const t = neu({
    assistent_einstufen: () => s0E("wiederholen", "Ich wiederhole."),
    assistent_letzte_ausgabe: () =>
      "Moment, ich schaue in den Besprechungen nach. Das ist die Antwort.",
    voice_call_chat: chatSagt(ANTWORT),
  });
  t.engine.ausgabeKey = { id: "c1" };
  await t.start("Wiederhole das bitte").done;
  assert.deepEqual(t.player.enqueued, [
    "wav:Ich wiederhole.",
    "wav:Das ist die Antwort.",
  ]);
});

test("Wiederholen behält eine zitierte Überbrückung innerhalb der Antwort", async () => {
  const zitat =
    "Die Ansage lautet: „Moment, ich schaue in den Besprechungen nach.“";
  const t = neu({
    assistent_einstufen: () => s0E("wiederholen", "Ich wiederhole."),
    assistent_letzte_ausgabe: () => zitat,
    voice_call_chat: chatSagt(ANTWORT),
  });
  t.engine.ausgabeKey = { id: "c1" };
  await t.start("Wiederhole das bitte").done;
  assert.deepEqual(t.player.enqueued, ["wav:Ich wiederhole.", `wav:${zitat}`]);
});

test("Wiederholen behält eine Überbrückung in einem Zitat aus mehreren Sätzen (R4)", async () => {
  const zitat =
    "Die Ansage lautet: „Bitte warten. Moment, ich schaue kurz nach. Danke.“";
  const t = neu({
    assistent_einstufen: () => s0E("wiederholen", "Ich wiederhole."),
    assistent_letzte_ausgabe: () => zitat,
    voice_call_chat: chatSagt(ANTWORT),
  });
  t.engine.ausgabeKey = { id: "c1" };
  await t.start("Wiederhole das bitte").done;
  assert.deepEqual(t.player.enqueued, ["wav:Ich wiederhole.", `wav:${zitat}`]);
});

for (const [name, zitat] of [
  [
    "verschachtelt",
    'Die Ansage lautet: „Er sagt: "Bitte warten. Moment, ich schaue kurz nach. Danke." Ende.“',
  ],
  ["ungeschlossen", "Er sagt: „Bitte warten. Moment, ich schaue kurz nach."],
]) {
  test(`Wiederholen behält eine Überbrückung in einem Zitat (${name}, R5)`, async () => {
    const t = neu({
      assistent_einstufen: () => s0E("wiederholen", "Ich wiederhole."),
      assistent_letzte_ausgabe: () => zitat,
      voice_call_chat: chatSagt(ANTWORT),
    });
    t.engine.ausgabeKey = { id: "c1" };
    await t.start("Wiederhole das bitte").done;
    assert.deepEqual(t.player.enqueued, [
      "wav:Ich wiederhole.",
      `wav:${zitat}`,
    ]);
  });
}

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

test("Orchestrator-Antwort läuft mit dem Schlüssel des Gesprächs durchs Tor; neues Gespräch vergisst", async () => {
  const t = neu({}, { seiteMit: seite(ANTWORT) });
  await t.start("Was steht im Kalender?").done;
  assert.deepEqual(t.engine.ausgabeKey, { id: "c1" });
  assert.equal(t.aufrufe("assistent_ausgabe_vergessen").length, 0);

  t.engine.resetHistory();
  assert.deepEqual(t.aufrufe("assistent_ausgabe_vergessen").at(-1).args, {
    conversationId: "c1",
  });
  assert.equal(t.engine.ausgabeKey, null);
});

test("Einstufung bekommt den Schlüssel des Gesprächs (Rust entscheidet „ja“ nach Rückfrage)", async () => {
  const t = neu({ voice_call_chat: chatSagt(ANTWORT) });
  await t.start("Ja").done;
  assert.deepEqual(t.aufrufe("assistent_einstufen")[0].args, {
    text: "Ja",
    sprache: "de",
    conversationId: "c1",
  });
});

test("Ja nach einer Rückfrage (Rust: S1 fortsetzung) geht zum Modell", async () => {
  const t = neu({
    assistent_einstufen: () => ({
      stufe: "S1",
      absicht: "fortsetzung",
      ansage: null,
      antwort: null,
    }),
    voice_call_chat: chatSagt(ANTWORT),
  });
  await t.start("Ja").done;
  assert.equal(t.aufrufe("voice_call_chat").length, 1);
  assert.deepEqual(t.player.enqueued, [`wav:${ANTWORT}`]);
});

test("Ja ohne Rückfrage (Rust: S0 bestaetigung): Vorlage, kein Modellaufruf", async () => {
  const t = neu({
    assistent_einstufen: () => s0E("bestaetigung", "In Ordnung."),
    voice_call_chat: chatSagt(ANTWORT),
  });
  await t.start("Ja").done;
  assert.equal(t.aufrufe("voice_call_chat").length, 0);
  assert.deepEqual(t.player.enqueued, ["wav:In Ordnung."]);
});

// -- S0 bleibt lokal: Bedienung ----------------------------------------------------

test("Lauter: kein Netzaufruf, die Lautstärke ändert sich, die Vorlage wird gesprochen", async () => {
  const t = neu({
    assistent_einstufen: () => s0E("lauter", "Lauter."),
    voice_call_chat: chatSagt(ANTWORT),
  });
  assert.equal(t.player.volume, 1);
  await t.start("Sprich lauter").done;
  assert.equal(t.aufrufe("voice_call_chat").length, 0);
  assert.equal(t.player.volume, 1.4);
  assert.deepEqual(t.player.enqueued, ["wav:Lauter."]);
  assert.deepEqual(verlauf(t), []);
});

test("Lauter an der Grenze: wahre Meldung, nichts ändert sich", async () => {
  const t = neu({ assistent_einstufen: () => s0E("lauter", "Lauter.") });
  await t.start("lauter").done;
  t.player.finish();
  await t.start("noch lauter").done;
  assert.deepEqual(t.player.enqueued, [
    "wav:Lauter.",
    "wav:Lauter geht es nicht.",
  ]);
  assert.equal(t.player.volume, 1.4);
});

test("Leiser, schneller, langsamer stellen Lautstärke und Tempo", async () => {
  let art = "leiser";
  const t = neu({ assistent_einstufen: () => s0E(art, "x") });
  await t.start("leiser").done;
  assert.equal(t.player.volume, 0.7);
  t.player.finish();
  art = "schneller";
  await t.start("schneller").done;
  assert.equal(t.player.rate, 1.15);
  t.player.finish();
  art = "langsamer";
  await t.start("langsamer").done;
  t.player.finish();
  await t.start("langsamer").done;
  assert.equal(t.player.rate, 0.9);
  assert.equal(t.aufrufe("voice_call_chat").length, 0);
});

test("Verworfener spekulativer Schritt ändert die Lautstärke nicht", async () => {
  const t = neu({ assistent_einstufen: () => s0E("lauter", "Lauter.") });
  const { done } = t.start("lauter", { commit: false });
  await settle();
  t.engine.cancelTurn(false);
  await done;
  assert.equal(t.player.volume, 1);
  assert.deepEqual(t.player.enqueued, []);
});

test("Bedienung im Textmodus ohne Wiedergabe: wahre Meldung statt falscher Zusage", async () => {
  const t = neu({ assistent_einstufen: () => s0E("schneller", "Schneller.") });
  t.engine.state = "idle";
  await t.engine.sendText("schneller");
  assert.equal(
    t.log.assistant.at(-1).t,
    "Das Tempo kann ich hier nicht ändern.",
  );
  assert.equal(t.player.rate, 1);
  assert.equal(t.aufrufe("voice_call_chat").length, 0);
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
  // Keine Ansage mehr: die Marke bleibt leer.
  assert.equal(m1.args.messung.erste_ansage_ms, null);
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
