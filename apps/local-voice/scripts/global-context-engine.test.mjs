// Lva#511 (Nacharbeit zu #509): Widerruf des Inhaltszugriffs WÄHREND einer laufenden Antwort, über den
// echten Antwortpfad der Engine (`respond`, `runTextOnly`) mit einem Fake-Anbieter (`voice_call_chat`).
// Das Modell streamt einige Sätze, dann kommt der Widerruf (so meldet ihn der Kern: `revoked` für `*`
// und für die Anfrage), DANACH treffen weitere Sätze und das Stream-Ende ein. Nichts davon darf
// angezeigt, gesprochen oder in den Verlauf geschrieben werden; es gibt keinen weiteren Abruf, und eine
// Folgerunde bekommt keine Auszüge.
// Lauf: node --test scripts/global-context-engine.test.mjs (Node >= 22.6).
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

const { VoiceCallEngine } =
  await import("../src/components/voice-call/engine.ts");
const { DEFAULT_TURN_CONFIG } =
  await import("../src/components/voice-call/turnTaking.ts");
const { SpeechPlayer } = await import("./test-stubs/audio.ts");

const FRAGE = "Wer übernimmt das Protokoll?";
const S1 = "Laut Besprechung übernimmt Anna das Protokoll.";
const S2 = "Das steht im Termin vom Montag.";
const SPAET1 = "Spaeter Satz nach dem Widerruf.";
const SPAET2 = "Noch ein spaeter Satz.";
const AUSZUG = "Geheimer Auszug";

const settle = (ms = 10) => new Promise((r) => setTimeout(r, ms));
const emit = (payload) => tauri.listeners.forEach((l) => l({ payload }));
function deferred() {
  let resolve;
  const promise = new Promise((r) => (resolve = r));
  return { promise, resolve };
}

const TREFFER = {
  k: 1,
  kind: "meeting",
  title: "Runde",
  chapter: null,
  at_ms: 0,
  excerpt: AUSZUG,
  jump: { kind: "meeting", id: "m1", offset: 0, page: null, book_id: null },
  score: 1,
  version: "00",
};

/** Der Kern als Attrappe: Berechtigungsversion und Bereiche, `app_context_*` und `voice_call_chat`. */
function neu({ lokal = true, imGespraech = true } = {}) {
  const kern = {
    version: 3,
    retrieve: [],
    chat: [],
    stream: null,
    frei: null,
    id: "",
  };
  globalThis.__settings = {
    voice_global_context_enabled: true,
    voice_call_knowledge: ["meetings"],
  };
  /** Ein Widerruf wie im Kern: Version steigt, Bereiche leer, Meldung an die Oberfläche. */
  kern.widerrufen = (requestId) => {
    kern.version += 1;
    globalThis.__settings = {
      voice_global_context_enabled: false,
      voice_call_knowledge: [],
    };
    emit({ kind: "revoked", request_id: "*" });
    if (requestId) emit({ kind: "revoked", request_id: requestId });
  };
  tauri.calls = [];
  tauri.listeners = [];
  tauri.prepare = undefined;
  tauri.handlers = {
    voice_call_synthesize: ({ text }) => `wav:${text}`,
    voice_call_cancel: () => null,
    assistent_einstufen: () => null,
    assistent_letzte_ausgabe: () => null,
    assistent_ausgabe_vergessen: () => null,
    assistent_messung_melden: () => null,
    app_context_retrieve: () => {
      kern.retrieve.push(kern.version);
      return {
        hits: [TREFFER],
        searched: ["meeting"],
        retrieve_ms: 1,
        epoch: 1,
        question_hash: "h",
        berechtigung: kern.version,
      };
    },
    app_context_message: () => "Kontext [Q1]",
    app_context_verify: () => [],
    voice_call_chat: (args) => {
      kern.chat.push(args);
      return kern.stream(args);
    },
  };
  const log = { user: [], assistant: [], errors: [] };
  const engine = new VoiceCallEngine(
    {
      onState() {},
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
  engine.state = imGespraech ? "listening" : "idle";
  engine.stopped = !imGespraech;
  engine.speakerLocal = lokal;
  const aufrufe = (cmd) => tauri.calls.filter((c) => c.cmd === cmd);
  return {
    kern,
    engine,
    player,
    log,
    aufrufe,
    verlauf: () => engine.history.map((m) => `${m.role}:${m.content}`),
    angezeigt: () => log.assistant.map((a) => a.t).join(" | "),
    gesprochen: () => player.enqueued.join(" | "),
    synthetisiert: () =>
      aufrufe("voice_call_synthesize")
        .map((c) => c.args.text)
        .join(" | "),
  };
}

/** Der Fake-Anbieter streamt `vorher`, hält dann an und liefert nach `frei.resolve()` `nachher` und `done`. */
function stream(kern, vorher, nachher) {
  kern.frei = deferred();
  kern.stream = ({ requestId }) => {
    kern.id = requestId;
    for (const text of vorher)
      emit({ kind: "sentence", request_id: requestId, text });
    return kern.frei.promise.then(() => {
      for (const text of nachher)
        emit({ kind: "sentence", request_id: requestId, text });
      emit({ kind: "done", request_id: requestId });
      return [...vorher, ...nachher].join(" ");
    });
  };
}

function sprachschritt(t, frage = FRAGE) {
  const turn = t.engine.newTurn(0, performance.now());
  t.engine.turn = turn;
  turn.committed = true;
  turn.commit();
  const fertig = t.engine
    .respond(turn, frage, 0)
    .catch((e) => t.log.errors.push(String(e)));
  return { turn, fertig };
}

/** Nichts vom Nachlauf darf irgendwo sichtbar, hörbar oder gespeichert sein. */
function keinNachlauf(t) {
  for (const spaet of [SPAET1, SPAET2]) {
    assert.equal(t.angezeigt().includes(spaet), false, "angezeigt");
    assert.equal(t.gesprochen().includes(spaet), false, "gesprochen");
    assert.equal(t.synthetisiert().includes(spaet), false, "synthetisiert");
    assert.equal(
      t.verlauf().some((v) => v.includes(spaet)),
      false,
      "im Verlauf",
    );
  }
}

test("Sprachschritt: Widerruf mitten im Stream, Nachläufer werden weder angezeigt noch gesprochen noch gespeichert", async () => {
  const t = neu();
  stream(t.kern, [S1, S2], [SPAET1, SPAET2]);
  await t.engine.ensureListener();
  const { turn, fertig } = sprachschritt(t);
  await settle(40);
  // Gegenprobe: vor dem Widerruf lief die Antwort mit Kontext.
  assert.equal(t.kern.chat.length, 1);
  assert.equal(t.kern.chat[0].context.hits.hits.length, 1);
  assert.equal(turn.withContext, true);
  assert.equal(t.angezeigt().includes(S1), true);
  assert.equal(t.synthetisiert().includes(S1), true);
  const abrufe = t.kern.retrieve.length;
  const synthese = t.aufrufe("voice_call_synthesize").length;

  t.kern.widerrufen(t.kern.id);
  t.kern.frei.resolve();
  await fertig;
  await settle(40);

  assert.equal(turn.cancelled, true);
  keinNachlauf(t);
  assert.equal(t.aufrufe("voice_call_synthesize").length, synthese);
  assert.deepEqual(t.verlauf(), [], "auch der Anteil vor dem Widerruf fehlt");
  assert.equal(t.kern.retrieve.length, abrufe, "kein weiterer Abruf");
  assert.deepEqual([...new Set(t.kern.retrieve)], [3]);
  assert.equal(
    t.aufrufe("voice_call_cancel").some((c) => c.args.requestId === t.kern.id),
    true,
  );
  assert.equal(t.engine.turn, null);
});

test("Sprachschritt: die Meldung der Anfrage allein (ohne `*`) genügt ebenso", async () => {
  const t = neu();
  stream(t.kern, [S1], [SPAET1, SPAET2]);
  await t.engine.ensureListener();
  const { turn, fertig } = sprachschritt(t);
  await settle(40);
  assert.equal(t.angezeigt().includes(S1), true);
  emit({ kind: "revoked", request_id: t.kern.id });
  t.kern.frei.resolve();
  await fertig;
  await settle(40);
  assert.equal(turn.cancelled, true);
  keinNachlauf(t);
  assert.deepEqual(t.verlauf(), []);
});

test("Folgerunde nach dem Widerruf: kein Abruf mit der alten Version, kein Kontext an das Modell", async () => {
  const t = neu();
  stream(t.kern, [S1], [SPAET1]);
  await t.engine.ensureListener();
  const { fertig } = sprachschritt(t);
  await settle(40);
  t.kern.widerrufen(t.kern.id);
  t.kern.frei.resolve();
  await fertig;
  await settle(20);
  const abrufe = t.kern.retrieve.length;

  t.kern.stream = ({ requestId }) => {
    emit({ kind: "sentence", request_id: requestId, text: "Ohne Kontext." });
    emit({ kind: "done", request_id: requestId });
    return "Ohne Kontext.";
  };
  await sprachschritt(t, "Und bis wann?").fertig;
  assert.equal(t.kern.retrieve.length, abrufe, "kein Abruf mit alter Version");
  const zweite = t.kern.chat.at(-1);
  assert.equal(zweite.context, null, "kein Kontext an das Modell");
  assert.equal(
    JSON.stringify(zweite.messages).includes(AUSZUG),
    false,
    "kein Auszug in den Nachrichten",
  );
  assert.equal(
    zweite.messages.some((m) => m.derived),
    false,
    "kein abgeleiteter Verlauf",
  );
});

test("Cloud-Empfänger: auch bei freigegebenen Bereichen geht weder ein Abruf noch ein Auszug hinaus", async () => {
  const t = neu({ lokal: false });
  t.kern.stream = ({ requestId }) => {
    emit({ kind: "sentence", request_id: requestId, text: S1 });
    emit({ kind: "done", request_id: requestId });
    return S1;
  };
  await t.engine.ensureListener();
  await sprachschritt(t).fertig;
  assert.equal(t.kern.retrieve.length, 0, "kein Abruf für ein Cloud-Modell");
  assert.equal(t.aufrufe("app_context_message").length, 0);
  assert.equal(t.kern.chat[0].context, null);
  assert.equal(JSON.stringify(t.kern.chat[0].messages).includes(AUSZUG), false);
});

test("Textweg (ohne Anruf): Widerruf mitten im Stream, Nachläufer erscheinen nicht und kommen nicht in den Verlauf", async () => {
  const t = neu({ imGespraech: false });
  stream(t.kern, [S1, S2], [SPAET1, SPAET2]);
  const lauf = t.engine
    .sendText(FRAGE)
    .catch((e) => t.log.errors.push(String(e)));
  await settle(40);
  assert.equal(t.kern.chat.length, 1);
  assert.equal(t.kern.chat[0].context.hits.hits.length, 1);
  assert.equal(t.angezeigt().includes(S1), true);
  const abrufe = t.kern.retrieve.length;

  t.kern.widerrufen(t.kern.id);
  t.kern.frei.resolve();
  await lauf;
  await settle(40);

  keinNachlauf(t);
  assert.deepEqual(t.verlauf(), []);
  assert.equal(t.kern.retrieve.length, abrufe);
  assert.equal(t.engine.textRun, null);
  assert.deepEqual(t.log.errors, []);
});
