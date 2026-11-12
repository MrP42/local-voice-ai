// Sofort-Sprecher (Patrick 12.11.2026): reine Logik und Gesprächsschritte der Engine mit nachgeahmtem
// Tauri. Lauf: node --test scripts/sofort.test.mjs (Node >= 22.6).
import assert from "node:assert/strict";
import { register } from "node:module";
import test from "node:test";

register("./test-stubs/hooks.mjs", import.meta.url);
globalThis.window = { addEventListener() {}, removeEventListener() {} };
const tauri = (globalThis.__tauri = {
  calls: [],
  handlers: {},
  listeners: [],
  prepare: undefined,
});

const { liesSofort, wendeAn, sofortPrompt, sofortModell, SOFORT_MAX_ZEICHEN } =
  await import("../src/components/voice-call/sofort.ts");
const { VoiceCallEngine, TUNING } =
  await import("../src/components/voice-call/engine.ts");
const { DEFAULT_TURN_CONFIG } =
  await import("../src/components/voice-call/turnTaking.ts");
const { SpeechPlayer } = await import("./test-stubs/audio.ts");

// -- Logik ---------------------------------------------------------------------

test("liesSofort: Marken, Groß-/Kleinschreibung, Zierzeichen", () => {
  assert.deepEqual(liesSofort("ANTWORT: Ja, ich höre dich."), {
    art: "antwort",
    text: "Ja, ich höre dich.",
  });
  assert.deepEqual(liesSofort("  **Weiter:** Spannend, die Mails von heute."), {
    art: "weiter",
    text: "Spannend, die Mails von heute.",
  });
  assert.deepEqual(liesSofort("WEITER:"), { art: "weiter", text: "" });
  assert.deepEqual(liesSofort("ANTWORT:"), { art: "nichts" });
  assert.deepEqual(liesSofort("Ja, ich höre dich."), { art: "nichts" });
  assert.deepEqual(liesSofort(""), { art: "nichts" });
  assert.deepEqual(liesSofort("ANTWORT: Ja. WEITER: Moment"), {
    art: "antwort",
    text: "Ja.",
  });
  assert.deepEqual(
    liesSofort(`ANTWORT: ${"x".repeat(SOFORT_MAX_ZEICHEN + 1)}`),
    {
      art: "nichts",
    },
  );
});

test("wendeAn: wer zuerst entscheidet, gewinnt; Datenabsicht verbietet Alleinantwort", () => {
  assert.deepEqual(wendeAn("wartet", { art: "antwort", text: "Ja." }, false), {
    zustand: "sprecher",
    sagen: "Ja.",
    denkerAbbrechen: true,
  });
  assert.deepEqual(wendeAn("wartet", { art: "antwort", text: "Ja." }, true), {
    zustand: "denker",
    sagen: "",
    denkerAbbrechen: false,
  });
  assert.deepEqual(
    wendeAn("wartet", { art: "weiter", text: "Gute Frage." }, true),
    {
      zustand: "denker",
      sagen: "",
      denkerAbbrechen: false,
    },
  );
  assert.deepEqual(wendeAn("denker", { art: "antwort", text: "Ja." }, false), {
    zustand: "denker",
    sagen: "",
    denkerAbbrechen: false,
  });
});

test("liesSofort: Warte-Floskeln sind nie eine Reaktion (ANTWORT wie WEITER)", () => {
  for (const f of [
    "Ich denke kurz nach.",
    "Moment, ich schaue in den Besprechungen nach.",
    "Einen Moment bitte.",
    "Let me think.",
    "Lass mich überlegen.",
  ]) {
    assert.deepEqual(liesSofort(`ANTWORT: ${f}`), { art: "nichts" }, f);
    assert.deepEqual(liesSofort(`WEITER: ${f}`), { art: "nichts" }, f);
  }
  assert.deepEqual(liesSofort("ANTWORT: I’m thinking."), { art: "nichts" });
  assert.deepEqual(liesSofort("WEITER: Hold on, let me check."), {
    art: "nichts",
  });
  assert.deepEqual(
    liesSofort("ANTWORT: Einen Moment bitte, ich prüfe das kurz."),
    {
      art: "nichts",
    },
  );
  // Echte Antworten mit ähnlichen Wörtern bleiben vollständig (Codex lva#542 R3).
  for (const echt of [
    "Momentan ist alles gut.",
    "Sekunden sind Zeiteinheiten.",
    "Moment, das stimmt so nicht.",
    "Augenblicklich läuft nichts.",
    "Es heißt Weiter: zum nächsten Schritt.",
    "Momentum matters here.",
    "I'm looking forward to it.",
    "Let me know if you need more.",
    "Hold-out data is used for testing.",
    "Gut.",
  ]) {
    assert.deepEqual(
      liesSofort(`ANTWORT: ${echt}`),
      { art: "antwort", text: echt },
      echt,
    );
  }
  // Sachsätze mit ähnlichen Wörtern bleiben erhalten.
  assert.deepEqual(liesSofort("ANTWORT: Ich denke, das passt."), {
    art: "antwort",
    text: "Ich denke, das passt.",
  });
});

test("wendeAn: bei Datenabsicht sagt der Sprecher auch beim Weitersprechen nichts", () => {
  assert.deepEqual(
    wendeAn("wartet", { art: "weiter", text: "Du hast keine Mails." }, true),
    { zustand: "denker", sagen: "", denkerAbbrechen: false },
  );
  assert.deepEqual(
    wendeAn("wartet", { art: "weiter", text: "Gute Frage." }, false),
    { zustand: "denker", sagen: "Gute Frage.", denkerAbbrechen: false },
  );
});

test("Prompt ohne feste Floskeln als Vorgabe, beide Sprachen", () => {
  for (const p of [sofortPrompt("de"), sofortPrompt("en")]) {
    assert.match(p, /ANTWORT:/);
    assert.match(p, /WEITER:/);
  }
});

test("sofortModell: nur bei Cloud-Gesprächsmodell, gewähltes lokales vor dem ersten", () => {
  const cloud = { id: "c", label: "GPT", local: false };
  const a = { id: "a", label: "Gemma", local: true };
  const b = { id: "b", label: "Qwen", local: true };
  assert.equal(sofortModell(null, [a], null), null);
  assert.equal(sofortModell(a, [a, b], null), null);
  assert.equal(sofortModell(cloud, [cloud], null), null);
  assert.equal(sofortModell(cloud, [cloud, a, b], null), a);
  assert.equal(sofortModell(cloud, [cloud, a, b], "b"), b);
  assert.equal(sofortModell(cloud, [cloud, a, b], "c"), a);
});

// -- Engine ----------------------------------------------------------------------

const settle = (ms = 5) => new Promise((r) => setTimeout(r, ms));
const emit = (payload) => tauri.listeners.forEach((l) => l({ payload }));
const ANTWORT = "Zwei neue Mails sind gekommen.";

/** Lokales Modell: liefert `zeile` nach `ms`; merkt sich Nachrichten und Abbruch. */
function lokal(zeile, ms = 5) {
  const m = {
    id: "loc",
    label: "Gemma",
    local: true,
    supportsTools: false,
    aufrufe: [],
    abgebrochen: false,
    async turn(req) {
      m.aufrufe.push(req);
      req.signal.addEventListener("abort", () => (m.abgebrochen = true));
      await settle(ms);
      return { text: zeile, toolCalls: [] };
    },
  };
  return m;
}

/** Denker über `voice_call_chat`: antwortet nach `ms` (oder gar nicht ohne `ms`). */
const denker =
  (ms, ...saetze) =>
  ({ requestId }) =>
    new Promise((resolve) => {
      if (ms === undefined) return;
      setTimeout(() => {
        for (const text of saetze)
          emit({ kind: "sentence", request_id: requestId, text });
        emit({ kind: "done", request_id: requestId });
        resolve(saetze.join(" "));
      }, ms);
    });

function neu(handlers, sofort) {
  tauri.calls = [];
  tauri.listeners = [];
  tauri.prepare = (cfg) => ({
    side: null,
    blocked: false,
    systemPrompt: cfg.systemPrompt,
    sofort,
  });
  tauri.handlers = {
    voice_call_synthesize: ({ text }) => `wav:${text}`,
    voice_call_cancel: () => null,
    assistent_einstufen: () => null,
    assistent_letzte_ausgabe: () => null,
    assistent_ausgabe_vergessen: () => null,
    assistent_messung_melden: () => null,
    ...handlers,
  };
  const log = { assistant: [], denken: [] };
  const engine = new VoiceCallEngine(
    {
      onState() {},
      onLevel() {},
      onUser() {},
      onAssistant: (t, final) => log.assistant.push({ t, final }),
      onMetrics() {},
      onError() {},
      onDenken: (e) => log.denken.push(e.art),
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
  const start = (text) => {
    const turn = engine.newTurn(0, performance.now());
    engine.turn = turn;
    turn.committed = true;
    turn.commit();
    return engine.respond(turn, text, 0);
  };
  const aufrufe = (cmd) => tauri.calls.filter((c) => c.cmd === cmd);
  const verlauf = () => engine.history.map((m) => `${m.role}:${m.content}`);
  return { engine, player, log, start, aufrufe, verlauf };
}

test("Sprecher antwortet allein: sofort gesprochen, Denker abgebrochen, ein Verlaufseintrag", async () => {
  const s = lokal("ANTWORT: Ja, ich höre dich gut.");
  const t = neu({ voice_call_chat: denker(200, "Ja.") }, s);
  await t.start("Kannst du mich hören?");
  assert.deepEqual(t.player.enqueued, ["wav:Ja, ich höre dich gut."]);
  assert.equal(
    t
      .aufrufe("voice_call_cancel")
      .some((c) => !c.args.requestId.endsWith(":sofort")),
    true,
  );
  assert.deepEqual(t.verlauf(), [
    "user:Kannst du mich hören?",
    "assistant:Ja, ich höre dich gut.",
  ]);
  // Der Sprecher sieht nur Verlauf und Frage: Systemanweisung des Sprechers plus die Äußerung.
  const msgs = s.aufrufe[0].messages;
  assert.equal(msgs[0].role, "system");
  assert.equal(msgs.at(-1).content, "Kannst du mich hören?");
  assert.equal(msgs.length, 2);
});

test("Sprecher spricht weiter, der Denker liefert danach in derselben Antwort", async () => {
  const s = lokal("WEITER: Die Mails von heute, gute Idee.");
  const t = neu({ voice_call_chat: denker(60, ANTWORT) }, s);
  await t.start("Was steht in meinen Mails?");
  assert.deepEqual(t.player.enqueued, [
    "wav:Die Mails von heute, gute Idee.",
    `wav:${ANTWORT}`,
  ]);
  assert.ok(t.log.denken.includes("sofort"));
  assert.ok(t.log.denken.includes("zwischen"));
  assert.deepEqual(t.verlauf(), [
    "user:Was steht in meinen Mails?",
    `assistant:Die Mails von heute, gute Idee. ${ANTWORT}`,
  ]);
});

test("Denker zuerst: der Sprecher wird verworfen und nie gesprochen", async () => {
  const s = lokal("ANTWORT: Falsch und spät.", 150);
  const t = neu({ voice_call_chat: denker(10, ANTWORT) }, s);
  await t.start("Was steht in meinen Mails?");
  assert.deepEqual(t.player.enqueued, [`wav:${ANTWORT}`]);
  assert.equal(s.abgebrochen, true);
});

test("Datenabsicht: eine Alleinantwort des Sprechers gilt nicht, der Denker antwortet", async () => {
  const s = lokal("ANTWORT: Du hast keine Mails.");
  const t = neu(
    {
      voice_call_chat: denker(40, ANTWORT),
      assistent_einstufen: () => ({
        stufe: "S2",
        absicht: "mails",
        ansage: "Ich schaue in deine Mails.",
        antwort: null,
      }),
    },
    s,
  );
  await t.start("Was steht in meinen Mails?");
  assert.deepEqual(t.player.enqueued, [`wav:${ANTWORT}`]);
});

test("Unbrauchbare Zeile oder Fehler des Sprechers: nur der Denker", async () => {
  const s = lokal("Ich weiß nicht.");
  const t = neu({ voice_call_chat: denker(30, ANTWORT) }, s);
  await t.start("Was steht in meinen Mails?");
  assert.deepEqual(t.player.enqueued, [`wav:${ANTWORT}`]);
});

test("Ohne lokales Modell: nichts ändert sich (nur der Denker)", async () => {
  const t = neu({ voice_call_chat: denker(5, ANTWORT) }, null);
  await t.start("Was steht in meinen Mails?");
  assert.deepEqual(t.player.enqueued, [`wav:${ANTWORT}`]);
  assert.equal(t.log.denken.includes("sofort"), false);
});

test("Textmodus: weitersprechen und nachreichen in einer Antwort", async () => {
  const s = lokal("WEITER: Schauen wir in die Mails.");
  const t = neu({ voice_call_chat: denker(40, ANTWORT) }, s);
  t.engine.state = "idle";
  await t.engine.sendText("Was steht in meinen Mails?");
  const letzte = t.log.assistant.at(-1);
  assert.equal(letzte.final, true);
  assert.equal(letzte.t, `Schauen wir in die Mails. ${ANTWORT}`);
});

test("Textmodus: Sprecher antwortet allein, kein Fehlertext trotz abgebrochenem Denker", async () => {
  const s = lokal("ANTWORT: Hallo!");
  const t = neu({ voice_call_chat: denker(undefined) }, s);
  const alt = TUNING.modelFristMs;
  TUNING.modelFristMs = 100;
  try {
    t.engine.state = "idle";
    await t.engine.sendText("Hallo");
    assert.equal(t.log.assistant.at(-1).t, "Hallo!");
  } finally {
    TUNING.modelFristMs = alt;
  }
});

test("Weitersprechen ist keine Antwort: bleibt der Denker aus, kommt die Vorlage, kein Verlauf", async () => {
  const s = lokal("WEITER: Deine Mails, gute Frage.");
  const t = neu({ voice_call_chat: denker(undefined) }, s);
  const alt = TUNING.modelFristMs;
  TUNING.modelFristMs = 100;
  try {
    await t.start("Was steht in meinen Mails?");
    assert.deepEqual(t.player.enqueued, [
      "wav:Deine Mails, gute Frage.",
      "wav:Das kann ich gerade nicht sicher beantworten.",
    ]);
    assert.deepEqual(t.verlauf(), []);
  } finally {
    TUNING.modelFristMs = alt;
  }
});

test("Denker scheitert, bevor der Sprecher entscheidet: seine Antwort gilt statt der Vorlage", async () => {
  const s = lokal("ANTWORT: Hallo, schön dich zu hören.", 80);
  const t = neu({ voice_call_chat: () => Promise.reject("offline") }, s);
  await t.start("Hallo");
  await settle(200);
  assert.deepEqual(t.player.enqueued, ["wav:Hallo, schön dich zu hören."]);
  assert.deepEqual(t.verlauf(), [
    "user:Hallo",
    "assistant:Hallo, schön dich zu hören.",
  ]);
  assert.equal(t.log.assistant.filter((a) => !a.final).length >= 1, true);
  assert.equal(t.log.assistant.at(-1).final, true);
});

test("Textmodus: Denker scheitert, Sprecher antwortet danach: nur seine Antwort", async () => {
  const s = lokal("ANTWORT: Gern geschehen!", 60);
  const t = neu({ voice_call_chat: () => Promise.reject("offline") }, s);
  t.engine.state = "idle";
  await t.engine.sendText("Danke");
  assert.equal(t.log.assistant.at(-1).t, "Gern geschehen!");
  assert.equal(t.log.assistant.at(-1).final, true);
  assert.deepEqual(t.verlauf(), ["user:Danke", "assistant:Gern geschehen!"]);
});

test("Alleinantwort des Sprechers: „Wiederholen“ kennt keine ältere Ausgabe mehr", async () => {
  const s = lokal("ANTWORT: Gern!");
  const t = neu({ voice_call_chat: denker(500, ANTWORT) }, s);
  t.engine.ausgabeKey = { id: "c1" };
  await t.start("Danke");
  assert.equal(t.aufrufe("assistent_ausgabe_vergessen").length, 1);
});

test("Floskel des Sprechers wird weder gesprochen noch angezeigt (Sprache und Text)", async () => {
  const s = lokal("ANTWORT: Ich denke kurz nach.");
  const t = neu({ voice_call_chat: denker(40, ANTWORT) }, s);
  await t.start("Was steht in meinen Mails?");
  assert.deepEqual(t.player.enqueued, [`wav:${ANTWORT}`]);

  const s2 = lokal("WEITER: Ich denke kurz nach.");
  const t2 = neu({ voice_call_chat: denker(40, ANTWORT) }, s2);
  t2.engine.state = "idle";
  await t2.engine.sendText("Was steht in meinen Mails?");
  assert.equal(t2.log.assistant.at(-1).t, ANTWORT);
  assert.equal(
    t2.log.assistant.some((a) => a.t.includes("denke kurz nach")),
    false,
  );
});

test("Datenabsicht: falsche Sachauskunft beim Weitersprechen wird nicht gesprochen", async () => {
  const s = lokal("WEITER: Du hast keine neuen Mails.");
  const t = neu(
    {
      voice_call_chat: denker(40, ANTWORT),
      assistent_einstufen: () => ({
        stufe: "S2",
        absicht: "mails",
        ansage: "Ich schaue in deine Mails.",
        antwort: null,
      }),
    },
    s,
  );
  await t.start("Was steht in meinen Mails?");
  assert.equal(
    t.player.enqueued.some((w) => w.includes("keine neuen Mails")),
    false,
  );
  assert.equal(t.player.enqueued.at(-1), `wav:${ANTWORT}`);
});
