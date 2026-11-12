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

const {
  liesSofort,
  wendeAn,
  wendeAuftragAn,
  auftragsBlock,
  sofortPrompt,
  sofortModell,
  SOFORT_MAX_ZEICHEN,
  AUFTRAG_FRAGE_MAX,
} = await import("../src/components/voice-call/sofort.ts");
const { fuegeZusammen } =
  await import("../src/components/voice-call/ergaenzen.ts");
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

test("liesSofort: Prüf-Ankündigungen, Schreibvarianten und Anführungszeichen sind Floskeln (R4)", () => {
  for (const f of [
    "Ich prüfe kurz die Angaben.",
    "I'm checking the book.",
    "moment, ich suche in den Besprechungen.",
    "„Moment, ich schaue kurz nach.“",
    '"Ich denke kurz nach."',
    "MOMENT, ICH SCHAUE KURZ NACH.",
    "I’m looking into it.",
    "Ich schaue mal nach.",
    "‘Moment, ich schaue kurz nach.’",
    "'Moment, ich schaue kurz nach.'",
    "'Ich prüfe kurz die Angaben.'",
    "Ich prüfe kurz, ob der Termin frei ist.",
    "Ich schaue mal, ob du Zeit hast.",
  ]) {
    assert.deepEqual(liesSofort(`ANTWORT: ${f}`), { art: "nichts" }, f);
    assert.deepEqual(liesSofort(`WEITER: ${f}`), { art: "nichts" }, f);
  }
});

test("liesSofort: Sachsätze mit Prüf- oder Denkverb bleiben vollständig (R4)", () => {
  for (const echt of [
    "Ich lese jeden Abend kurz.",
    "Ich denke oft über meine Kindheit nach.",
    "Ich denke.",
    "I'm thinking about my childhood.",
    "Ich lese kurz vor dem Schlafengehen.",
    "Ich lese kurz die Nachrichten und gehe dann schlafen.",
    "Ich schaue kurz in den Garten und dann gehe ich.",
    "Ich lese mal im Bett, bevor ich schlafe.",
    "Ich sehe mal wieder nichts.",
    "Ich suche in den Besprechungen vom Montag die Entscheidung, sie lautet Ja.",
  ]) {
    assert.deepEqual(
      liesSofort(`ANTWORT: ${echt}`),
      { art: "antwort", text: echt },
      echt,
    );
  }
});

test("liesSofort: eine zweite Marke am Satzanfang schneidet ab", () => {
  assert.deepEqual(liesSofort("ANTWORT: Ja, gern. WEITER: Noch etwas."), {
    art: "antwort",
    text: "Ja, gern.",
  });
  assert.deepEqual(liesSofort("WEITER: Gute Frage! ANTWORT: Nein."), {
    art: "weiter",
    text: "Gute Frage!",
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

test("liesSofort mit offenem Auftrag: SUCHE und ABBRUCH gelten, WEITER nicht; ohne Auftrag gelten sie nicht", () => {
  assert.deepEqual(liesSofort("SUCHE: Alles klar, nur Montag.", true), {
    art: "suche",
    text: "Alles klar, nur Montag.",
  });
  assert.deepEqual(liesSofort("**Suche:**", true), { art: "suche", text: "" });
  assert.deepEqual(liesSofort("ABBRUCH: Alles klar.", true), {
    art: "abbruch",
    text: "Alles klar.",
  });
  assert.deepEqual(liesSofort("abbruch:", true), { art: "abbruch", text: "" });
  assert.deepEqual(liesSofort("ANTWORT: Gern geschehen.", true), {
    art: "antwort",
    text: "Gern geschehen.",
  });
  assert.deepEqual(liesSofort("WEITER: Gute Frage.", true), { art: "nichts" });
  assert.deepEqual(liesSofort("", true), { art: "nichts" });
  assert.deepEqual(liesSofort("Keine Ahnung.", true), { art: "nichts" });
  // Ohne offenen Auftrag kennt der Sprecher beide Marken nicht.
  assert.deepEqual(liesSofort("SUCHE: Ich suche."), { art: "nichts" });
  assert.deepEqual(liesSofort("ABBRUCH: Alles klar.", false), {
    art: "nichts",
  });
  assert.deepEqual(liesSofort("WEITER: Gute Frage."), {
    art: "weiter",
    text: "Gute Frage.",
  });
});

test("liesSofort mit offenem Auftrag: Floskel, zweite Marke und Überlänge sind unbrauchbar", () => {
  assert.deepEqual(liesSofort("SUCHE: Ich schaue kurz nach.", true), {
    art: "nichts",
  });
  assert.deepEqual(liesSofort("ABBRUCH: Einen Moment bitte.", true), {
    art: "nichts",
  });
  assert.deepEqual(liesSofort("SUCHE: Ich suche neu. ANTWORT: Ja.", true), {
    art: "suche",
    text: "Ich suche neu.",
  });
  assert.deepEqual(
    liesSofort(`SUCHE: ${"x".repeat(SOFORT_MAX_ZEICHEN + 1)}`, true),
    { art: "nichts" },
  );
});

test("wendeAuftragAn: die Marke entscheidet; Unbrauchbares und Datenabsicht gelten wie SUCHE", () => {
  assert.deepEqual(wendeAuftragAn({ art: "antwort", text: "Ja." }, false), {
    art: "antwort",
    sagen: "Ja.",
  });
  assert.deepEqual(wendeAuftragAn({ art: "suche", text: "Neu." }, false), {
    art: "suche",
    sagen: "Neu.",
  });
  assert.deepEqual(wendeAuftragAn({ art: "suche", text: "" }, false), {
    art: "suche",
    sagen: "",
  });
  assert.deepEqual(wendeAuftragAn({ art: "abbruch", text: "Klar." }, false), {
    art: "abbruch",
    sagen: "Klar.",
  });
  assert.deepEqual(wendeAuftragAn({ art: "abbruch", text: "" }, false), {
    art: "abbruch",
    sagen: "",
  });
  assert.deepEqual(wendeAuftragAn({ art: "nichts" }, false), {
    art: "suche",
    sagen: "",
  });
  assert.deepEqual(wendeAuftragAn({ art: "weiter", text: "Moment." }, false), {
    art: "suche",
    sagen: "",
  });
  // Datenabsicht der Einstufung: jede Marke gilt wie SUCHE, der Sprecher sagt nichts.
  for (const e of [
    { art: "antwort", text: "Ja." },
    { art: "suche", text: "Neu." },
    { art: "abbruch", text: "Klar." },
    { art: "weiter", text: "Moment." },
    { art: "nichts" },
  ])
    assert.deepEqual(wendeAuftragAn(e, true), { art: "suche", sagen: "" });
});

test("auftragsBlock: Frage und ja/nein, beide Sprachen, Frage gekürzt, Prompt kennt die Auftragsmarken nicht", () => {
  const frage = "Was steht in meinen Mails?";
  const de = auftragsBlock(frage, false, "de");
  assert.match(de, /Offener Suchauftrag/);
  assert.ok(de.includes(`„${frage}“`));
  assert.match(de, /Ergebnis liegt vor: nein\./);
  assert.match(auftragsBlock(frage, true, "de"), /Ergebnis liegt vor: ja\./);
  const en = auftragsBlock("What is in my mail?", true, "en");
  assert.match(en, /Open search job/);
  assert.match(en, /Result available: yes\./);
  assert.match(auftragsBlock("x", false, "en"), /Result available: no\./);
  // Zeilenumbrüche der Frage werden zu Leerzeichen, Überlanges wird gekürzt.
  assert.ok(
    auftragsBlock("Zeile eins\n\nZeile zwei", false, "de").includes(
      "Zeile eins Zeile zwei",
    ),
  );
  const lang = auftragsBlock("a ".repeat(500), false, "de");
  assert.ok(lang.includes("…“"));
  assert.ok(!lang.includes("a ".repeat(AUFTRAG_FRAGE_MAX / 2 + 5)));
  for (const p of [sofortPrompt("de"), sofortPrompt("en")])
    assert.doesNotMatch(p, /SUCHE|ABBRUCH/);
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

/** Ein Treffer aus den eigenen Inhalten (für die Quellenmeldung). */
const TREFFER = {
  k: 1,
  kind: "meeting",
  title: "Runde",
  chapter: null,
  at_ms: 0,
  excerpt: "Auszug",
  jump: { kind: "meeting", id: "m1", offset: 0, page: null, book_id: null },
  score: 1,
  version: "00",
};
const KONTEXT_HANDLER = {
  app_context_retrieve: () => ({
    hits: [TREFFER],
    searched: ["meeting"],
    retrieve_ms: 1,
    epoch: 1,
    question_hash: "h",
    berechtigung: 1,
  }),
  app_context_message: () => "Kontext [Q1]",
  app_context_verify: () => [1],
};

function neu(handlers, sofort, optionen = {}) {
  tauri.calls = [];
  tauri.listeners = [];
  // Mit `kontext`: Zugriff auf eigene Inhalte an (Abruf, Kontextnachricht, Belegprüfung).
  globalThis.__settings = optionen.kontext
    ? {
        voice_global_context_enabled: true,
        voice_call_knowledge: ["meetings"],
      }
    : undefined;
  tauri.prepare = (cfg) => ({
    side: optionen.side ?? null,
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
    ...(optionen.kontext ? KONTEXT_HANDLER : {}),
    ...handlers,
  };
  const log = {
    citations: [],
    assistant: [],
    denken: [],
    denkenRoh: [],
    user: [],
    errors: [],
    verworfen: 0,
  };
  const config = {
    systemPrompt: "sys",
    turn: DEFAULT_TURN_CONFIG,
    orchestration: {},
  };
  const engine = new VoiceCallEngine(
    {
      onState() {},
      onLevel() {},
      onUser: (t) => log.user.push(t),
      onAssistant: (t, final, interrupted) =>
        log.assistant.push({ t, final, interrupted }),
      onMetrics() {},
      onError: (e) => log.errors.push(String(e)),
      onVerwerfen: () => log.verworfen++,
      onCitations: (id, hits, nachgereicht) =>
        log.citations.push({ id, n: hits.length, nachgereicht }),
      onDenken: (e) => {
        log.denken.push(e.art);
        log.denkenRoh.push(e);
      },
      conversationId: () => "c1",
      ...(optionen.projekt ? { projekt: () => optionen.projekt } : {}),
    },
    config,
  );
  const player = new SpeechPlayer(null);
  // Was wirklich zu hören war (Beginn des Häppchens), nicht nur eingereiht.
  player.gespielt = [];
  const einreihen = player.enqueue.bind(player);
  player.enqueue = (wav, onEnd, onStart) =>
    einreihen(wav, onEnd, () => {
      player.gespielt.push(wav);
      onStart?.();
    });
  player.onIdle = () => {
    if (engine.turn?.audible && engine.turn.done) engine.finishTurn();
  };
  engine.player = player;
  engine.state = "listening";
  engine.stopped = false;
  // Abruf eigener Inhalte gibt es nur bei lokalem Gesprächsmodell.
  engine.speakerLocal = !!optionen.kontext;
  // Mikrofonpuffer: genug stille Frames, damit die Äußerung ausgeschnitten werden kann.
  engine.frames = Array.from({ length: 200 }, () => new Float32Array(320));
  engine.frameIndex = 200;
  const start = (text) => {
    const turn = engine.newTurn(0, performance.now());
    engine.turn = turn;
    turn.committed = true;
    turn.commit();
    return engine.respond(turn, text, 0);
  };
  /** Gesprochene Äußerung als laufender Schritt (ohne auf das Ende der Antwort zu warten). */
  const startSprache = (text) => {
    const turn = engine.newTurn(0, performance.now());
    turn.fromSpeech = true;
    engine.turn = turn;
    turn.committed = true;
    turn.commit();
    const fertig = engine
      .respond(turn, text, 0)
      .catch((e) => log.errors.push(String(e)));
    return { turn, fertig };
  };
  /** Was die Spracherkennung als Nächstes liefert. */
  const erkenne = (text) => {
    tauri.handlers.voice_call_transcribe = () => text;
  };
  /** Der Nutzer redet los, während die KI spricht, und hält durch (Pegel-Barge-in): die Antwort wird angehalten. */
  const barge = () => {
    engine.onSpeechStart(2000, true);
    engine.onBargeIn();
  };
  /** Der Nutzer redet los, ohne dass etwas zu hören ist. */
  const redeLos = () => engine.onSpeechStart(2000, false);
  /** Frühes Satzende (`ms` Sprachdauer) und sicheres Satzende der Äußerung. */
  const fruehEnde = (ms = 800) => engine.onEarlyEnd(3000, ms);
  const ende = () => engine.onEnd();
  /** Komplette Äußerung: erkannter Text, Satzende (früh und sicher). */
  const aeussere = (text, { mitBarge = true } = {}) => {
    erkenne(text);
    if (mitBarge) barge();
    else redeLos();
    fruehEnde();
    ende();
  };
  const aufrufe = (cmd) => tauri.calls.filter((c) => c.cmd === cmd);
  const abgebrochen = (id) =>
    aufrufe("voice_call_cancel").filter((c) => c.args.requestId === id).length;
  const verlauf = () => engine.history.map((m) => `${m.role}:${m.content}`);
  return {
    engine,
    player,
    log,
    config,
    start,
    startSprache,
    erkenne,
    barge,
    redeLos,
    fruehEnde,
    ende,
    aeussere,
    aufrufe,
    abgebrochen,
    verlauf,
  };
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

test("Floskel unter ANTWORT bricht den Denker nicht ab (R4)", async () => {
  const s = lokal("ANTWORT: Ich prüfe kurz die Angaben.");
  const t = neu({ voice_call_chat: denker(40, ANTWORT) }, s);
  await t.start("Was steht in meinen Mails?");
  assert.deepEqual(t.player.enqueued, [`wav:${ANTWORT}`]);
  assert.equal(
    t
      .aufrufe("voice_call_cancel")
      .some((c) => !c.args.requestId.endsWith(":sofort")),
    false,
  );
  assert.deepEqual(t.verlauf(), [
    "user:Was steht in meinen Mails?",
    `assistant:${ANTWORT}`,
  ]);
});

// -- Offener Auftrag (ADR-0028 E4.2c 7(e)) ----------------------------------------------------
//
// Fehlerfälle (vor dem Code aufgeschrieben, je mit Test):
//  F1 Nebenläufigkeit: späte Ereignisse eines verworfenen Laufs; Verwerfen mehrfach/gleichzeitig; Nutzer spricht
//     genau beim Nachreichen; der Denker liefert, während die Äußerung erkannt wird.
//  F2 Abbruch mitten im Vorgang: Nachreichen noch ohne Ton, schon hörbar, durch Stopp/Ende/Widerruf unterbrochen.
//  F3 voller Speicher: Satzpuffer des Auftrags gedeckelt, Überlauf gezählt und gemeldet.
//  F4 fehlendes Gerät: kein Player, Synthese schlägt fehl (kein Dauerversuch, ein Fehler).
//  F5 Absturz des Denkers (Kindprozess/Tor): Fehler, `failed`, leeres Ergebnis verwerfen still.
//  F6 Fehler des Sprechers bei offenem Auftrag: gilt wie SUCHE.

const FRAGE = "Was steht in meinen Mails?";
const FOLGE = "Wie spät ist es?";
const WEITER_ZEILE = "WEITER: Gute Frage, ich schaue nach.";
const WEITER_SATZ = "wav:Gute Frage, ich schaue nach.";
const ZWEITE_ZEILE = "ANTWORT: Es ist kurz nach drei.";
const ZWEITE_SATZ = "wav:Es ist kurz nach drei.";
const ERGEBNIS = `wav:${ANTWORT}`;
// Nicht „Danke“: kurze Äußerungen wie diese gelten als Erkennungsfehler (PHANTOMS).
const WETTER = "Wie ist das Wetter?";
const WETTER_ZEILE = "ANTWORT: Heute scheint die Sonne.";
const WETTER_SATZ = "wav:Heute scheint die Sonne.";

/** Lokales Modell: eine Zeile je Aufruf (die letzte gilt weiter). */
function lokalFolge(zeilen, ms = 5) {
  const m = {
    id: "loc",
    label: "Gemma",
    local: true,
    supportsTools: false,
    aufrufe: [],
    abgebrochen: false,
    async turn(req) {
      const i = m.aufrufe.push(req) - 1;
      req.signal.addEventListener("abort", () => (m.abgebrochen = true));
      await settle(ms);
      const zeile = zeilen[Math.min(i, zeilen.length - 1)];
      if (zeile instanceof Error) throw zeile;
      return { text: zeile, toolCalls: [] };
    },
  };
  return m;
}

/** Denker, der erst auf `voice_call_cancel` zurückkehrt (wie das Tor in Rust). */
function haengend() {
  const frei = new Map();
  return {
    chat: ({ requestId }) =>
      new Promise((ok) => frei.set(requestId, () => ok(""))),
    cancel: ({ requestId }) => {
      frei.get(requestId)?.();
      return null;
    },
  };
}

/** Je Aufruf ein anderer Handler (der letzte gilt weiter). */
const nacheinander = (...hs) => {
  let i = 0;
  return (a) => hs[Math.min(i++, hs.length - 1)](a);
};

async function bis(bedingung, max = 3000) {
  const frist = performance.now() + max;
  while (!bedingung() && performance.now() < frist) await settle(10);
  return !!bedingung();
}

async function mitTuning(werte, fn) {
  const alt = Object.fromEntries(Object.keys(werte).map((k) => [k, TUNING[k]]));
  Object.assign(TUNING, werte);
  try {
    return await fn();
  } finally {
    Object.assign(TUNING, alt);
  }
}

/**
 * Erster Schritt: der Sprecher sagt WEITER, der Denker liefert noch nichts; dann eine zweite Äußerung, die der
 * Sprecher beantwortet. Danach ist der erste Schritt ein offener Auftrag, sein Vordergrund ist beendet.
 */
async function aufbau({
  zeilen = [WEITER_ZEILE, ZWEITE_ZEILE],
  denker: d = denker(300, ANTWORT),
  handlers = {},
  mitBarge = true,
  optionen,
  frageB = FOLGE,
} = {}) {
  const s = lokalFolge(zeilen);
  const t = neu({ voice_call_chat: d, ...handlers }, s, optionen);
  const a1 = t.startSprache(FRAGE);
  await settle(40);
  t.aeussere(frageB, { mitBarge });
  await settle(60);
  return { s, t, a1: a1.turn, erster: a1 };
}

const ohneBlock = (aufrufe) =>
  aufrufe.every(
    (c) => !/Offener Suchauftrag|Open search job/.test(JSON.stringify(c.args)),
  );

test("Auftrag läuft weiter: Sprecher antwortet kurz, danach kommt das Ergebnis als eigene Einheit", async () => {
  const { s, t, a1 } = await aufbau({ denker: denker(400, ANTWORT) });
  // Der Vordergrund endete, der Denker läuft weiter: kein Abbruch, nur das Weitersprechen verstummt.
  assert.ok(t.engine.auftrag && !t.engine.auftrag.verworfen);
  assert.equal(t.abgebrochen(a1.id), 0);
  assert.ok(t.player.stopped >= 1);
  assert.deepEqual(t.player.gespielt, [WEITER_SATZ, ZWEITE_SATZ]);
  assert.deepEqual(t.verlauf(), [
    `user:${FOLGE}`,
    "assistant:Es ist kurz nach drei.",
  ]);
  // Die neue Äußerung ging zuerst nur an den Sprecher: kein zweiter Denker.
  assert.equal(t.aufrufe("voice_call_chat").length, 1);
  assert.equal(s.aufrufe.length, 2);
  assert.doesNotMatch(
    s.aufrufe[0].messages[0].content,
    /Offener Suchauftrag/,
    "der erste Aufruf kennt keinen Auftrag",
  );
  const block = s.aufrufe[1].messages[0].content;
  assert.match(block, /Offener Suchauftrag/);
  assert.ok(block.includes(`„${FRAGE}“`));
  assert.match(block, /Ergebnis liegt vor: nein\./);
  assert.equal(s.aufrufe[1].messages.at(-1).content, FOLGE);

  t.player.finish();
  const frei = performance.now();
  assert.equal(t.engine.turn, null);
  assert.equal(t.player.gespielt.includes(ERGEBNIS), false);
  assert.equal(
    await bis(() => t.player.gespielt.includes(ERGEBNIS)),
    true,
    "das Ergebnis wird nachgereicht",
  );
  // Mindestens `nachreichenStilleMs` Stille seit dem Ende des Vordergrunds.
  assert.ok(performance.now() - frei >= TUNING.nachreichenStilleMs - 20);
  t.player.finish();
  await settle(10);

  assert.deepEqual(t.verlauf(), [
    `user:${FOLGE}`,
    "assistant:Es ist kurz nach drei.",
    `user:${FRAGE}`,
    `assistant:${ANTWORT}`,
  ]);
  // Eigene Assistentenzeile (nie mit der Antwort davor zusammengeführt), keine neue Nutzerzeile.
  assert.deepEqual(t.log.assistant.at(-1), {
    t: ANTWORT,
    final: true,
    interrupted: false,
  });
  assert.deepEqual(t.log.user, [FRAGE, FOLGE]);
  assert.equal(t.engine.auftrag, null);
  assert.equal(t.abgebrochen(a1.id), 0, "nie ein Abbruch für den Auftrag");
  assert.ok(t.log.denken.includes("auftrag"));
  assert.ok(t.log.denken.includes("vordergrund"));
  // Der Block geht nie an den Denker.
  assert.equal(ohneBlock(t.aufrufe("voice_call_chat")), true);
});

test("Auftrag ohne hörbares Weitersprechen (WEITER ohne Text): die Äußerung geht zuerst an den Sprecher", async () => {
  const { s, t, a1 } = await aufbau({
    zeilen: ["WEITER:", ZWEITE_ZEILE],
    denker: denker(300, ANTWORT),
    mitBarge: false,
  });
  assert.ok(t.engine.auftrag && !t.engine.auftrag.verworfen);
  assert.equal(t.abgebrochen(a1.id), 0);
  assert.deepEqual(t.player.gespielt, [ZWEITE_SATZ]);
  assert.equal(s.aufrufe.length, 2);
  assert.match(s.aufrufe[1].messages[0].content, /Offener Suchauftrag/);
  assert.equal(t.aufrufe("voice_call_chat").length, 1);
  t.player.finish();
  assert.equal(await bis(() => t.player.gespielt.includes(ERGEBNIS)), true);
  assert.equal(t.abgebrochen(a1.id), 0);
});

test("Rückmeldelaut während des Wartens auf den Denker: kein Auftrag, der Schritt läuft unverändert weiter", async () => {
  const s = lokalFolge(["WEITER:", ZWEITE_ZEILE]);
  const t = neu({ voice_call_chat: denker(150, ANTWORT) }, s);
  const { turn: a1 } = t.startSprache(FRAGE);
  await settle(40);
  t.aeussere("mhm", { mitBarge: false });
  assert.equal(await bis(() => t.player.enqueued.includes(ERGEBNIS)), true);
  assert.ok(!t.engine.auftrag);
  assert.equal(t.abgebrochen(a1.id), 0);
  assert.equal(s.aufrufe.length, 1, "der Sprecher wurde nicht erneut gefragt");
  assert.equal(t.engine.turn, a1);
});

test("Ergebnis schon da: der Block sagt ja, aber nie den Inhalt", async () => {
  await mitTuning({ nachreichenStilleMs: 100 }, async () => {
    const s = lokalFolge([WEITER_ZEILE, ZWEITE_ZEILE, WETTER_ZEILE]);
    const t = neu({ voice_call_chat: denker(150, ANTWORT) }, s);
    t.startSprache(FRAGE);
    await settle(40);
    t.aeussere(FOLGE);
    await bis(() => t.player.gespielt.includes(ZWEITE_SATZ));
    t.player.finish();
    // Der Nutzer redet wieder, bevor die Stille um ist: das Ergebnis liegt vor, wartet aber.
    t.redeLos();
    await settle(400);
    assert.equal(t.engine.auftrag.fertig, true);
    assert.equal(
      t.player.gespielt.includes(ERGEBNIS),
      false,
      "die offene Äußerung hält das Nachreichen an, auch nach langer Stille",
    );
    t.erkenne(WETTER);
    t.fruehEnde();
    t.ende();
    await bis(() => t.player.gespielt.includes(WETTER_SATZ));
    assert.equal(
      t.player.gespielt.includes(ERGEBNIS),
      false,
      "erst die Antwort auf die neue Äußerung",
    );
    const block = s.aufrufe[2].messages[0].content;
    assert.match(block, /Ergebnis liegt vor: ja\./);
    assert.equal(block.includes("Mails sind gekommen"), false);
    t.player.finish();
    assert.equal(await bis(() => t.player.gespielt.includes(ERGEBNIS)), true);
    assert.deepEqual(t.player.gespielt.slice(-2), [WETTER_SATZ, ERGEBNIS]);
    assert.equal(t.aufrufe("voice_call_chat").length, 1);
  });
});

test("SUCHE ersetzt den Auftrag: alter Lauf abgebrochen, seine späten Sätze ohne Ausgabe, neuer Denker antwortet", async () => {
  const s = lokalFolge([
    "WEITER: Ich schaue nach.",
    "SUCHE: Alles klar, nur die von Montag.",
  ]);
  const t = neu(
    {
      voice_call_chat: nacheinander(
        denker(250, "Alte Antwort."),
        denker(40, "Neue Antwort."),
      ),
    },
    s,
  );
  const { turn: a1 } = t.startSprache(FRAGE);
  await settle(40);
  t.aeussere("Nur die von Montag bitte.");
  assert.equal(
    await bis(() => t.player.enqueued.includes("wav:Neue Antwort.")),
    true,
  );
  assert.equal(t.abgebrochen(a1.id), 1, "der alte Lauf wurde abgebrochen");
  assert.equal(t.engine.auftrag, null);
  assert.deepEqual(t.player.enqueued, [
    "wav:Ich schaue nach.",
    "wav:Alles klar, nur die von Montag.",
    "wav:Neue Antwort.",
  ]);
  t.player.finish();
  await settle(400); // der alte Denker meldet sich spät: Ausgabe 0
  assert.equal(
    t.player.enqueued.some((w) => w.includes("Alte Antwort")),
    false,
  );
  assert.equal(
    t.log.assistant.some((a) => a.t.includes("Alte Antwort")),
    false,
  );
  assert.equal(t.aufrufe("voice_call_chat").length, 2);
  // Der neue Denker kennt die ursprüngliche Frage als Zusammenhang, nie den Block.
  const neuerChat = t.aufrufe("voice_call_chat")[1].args;
  assert.ok(JSON.stringify(neuerChat.messages).includes(FRAGE));
  assert.equal(ohneBlock(t.aufrufe("voice_call_chat")), true);
  assert.equal(
    t.verlauf().at(-1),
    "assistant:Alles klar, nur die von Montag. Neue Antwort.",
  );
  assert.equal(t.engine.turn, null);
  assert.equal(t.log.errors.length, 0);
});

test("SUCHE ohne Satz, unbrauchbare Zeile, Floskel und Sprecher-Fehler gelten wie SUCHE (neuer Denker)", async () => {
  for (const zeile of [
    "SUCHE:",
    "Keine Ahnung.",
    "SUCHE: Ich schaue kurz nach.",
    "WEITER: Das gibt es hier nicht.",
    new Error("Modell nicht erreichbar"),
  ]) {
    const s = lokalFolge(["WEITER: Ich schaue nach.", zeile]);
    const t = neu(
      {
        voice_call_chat: nacheinander(
          denker(250, "Alte Antwort."),
          denker(40, "Neue Antwort."),
        ),
      },
      s,
    );
    const { turn: a1 } = t.startSprache(FRAGE);
    await settle(40);
    t.aeussere("Nur die von Montag bitte.");
    assert.equal(
      await bis(() => t.player.enqueued.includes("wav:Neue Antwort.")),
      true,
      String(zeile),
    );
    assert.equal(t.abgebrochen(a1.id), 1, String(zeile));
    assert.equal(t.engine.auftrag, null, String(zeile));
    assert.deepEqual(
      t.player.enqueued,
      ["wav:Ich schaue nach.", "wav:Neue Antwort."],
      `${zeile}: nichts gesprochen außer dem neuen Denker`,
    );
    assert.equal(
      t.player.enqueued.some((w) => w.includes("Alte Antwort")),
      false,
    );
  }
});

test("Datenabsicht der Einstufung: jede Marke gilt wie SUCHE, der Sprecher sagt nichts", async () => {
  const s = lokalFolge([WEITER_ZEILE, "ANTWORT: Du hast keine Mails."]);
  const t = neu(
    {
      voice_call_chat: nacheinander(
        denker(250, "Alte Antwort."),
        denker(40, ANTWORT),
      ),
      assistent_einstufen: nacheinander(
        () => null,
        () => ({
          stufe: "S2",
          absicht: "mails",
          ansage: "Ich schaue in deine Mails.",
          antwort: null,
        }),
      ),
    },
    s,
  );
  const { turn: a1 } = t.startSprache(FRAGE);
  await settle(40);
  t.aeussere("Und was steht in den Mails von Anna?");
  assert.equal(await bis(() => t.player.enqueued.includes(ERGEBNIS)), true);
  assert.equal(t.abgebrochen(a1.id), 1);
  // Die Äußerung ging zuerst an den Sprecher (mit dem Block), der Denker startete erst danach.
  assert.match(s.aufrufe[1].messages[0].content, /Offener Suchauftrag/);
  assert.equal(
    t.player.enqueued.some((w) => w.includes("keine Mails")),
    false,
  );
  assert.equal(
    t.player.enqueued.some((w) => w.includes("Alte Antwort")),
    false,
  );
});

test("ABBRUCH verwirft den Auftrag: Bestätigung gesprochen, Ergebnis ohne Ausgabe, kein neuer Denker", async () => {
  await mitTuning({ nachreichenStilleMs: 100 }, async () => {
    const s = lokalFolge([
      "WEITER: Ich schaue nach.",
      "ABBRUCH: Alles klar, ich lasse es.",
    ]);
    const t = neu({ voice_call_chat: denker(150, ANTWORT) }, s);
    const { turn: a1 } = t.startSprache(FRAGE);
    await settle(40);
    t.aeussere("Lass die Suche, bitte.");
    assert.equal(
      await bis(() =>
        t.player.gespielt.includes("wav:Alles klar, ich lasse es."),
      ),
      true,
    );
    assert.equal(t.abgebrochen(a1.id), 1);
    assert.equal(t.engine.auftrag, null);
    assert.equal(t.aufrufe("voice_call_chat").length, 1, "kein neuer Denker");
    t.player.finish();
    await settle(500); // der alte Denker meldet sich spät, danach lange Stille
    assert.equal(
      t.player.enqueued.some((w) => w.includes("Mails sind gekommen")),
      false,
    );
    assert.deepEqual(t.verlauf(), [
      "user:Lass die Suche, bitte.",
      "assistant:Alles klar, ich lasse es.",
    ]);
    assert.equal(t.engine.turn, null);
  });
});

test("ABBRUCH ohne Bestätigung: der Schritt bleibt stumm (keine Vorlage, kein Verlauf)", async () => {
  const s = lokalFolge(["WEITER: Ich schaue nach.", "ABBRUCH:"]);
  const t = neu({ voice_call_chat: denker(150, ANTWORT) }, s);
  const { turn: a1 } = t.startSprache(FRAGE);
  await settle(40);
  t.aeussere("Lass die Suche, bitte.");
  await settle(300);
  assert.equal(t.abgebrochen(a1.id), 1);
  assert.equal(t.engine.auftrag, null);
  assert.equal(t.engine.turn, null);
  assert.equal(t.engine.state, "listening");
  assert.deepEqual(t.player.gespielt, ["wav:Ich schaue nach."]);
  assert.equal(
    t.player.enqueued.some((w) => w.includes("nicht sicher beantworten")),
    false,
  );
  assert.deepEqual(t.verlauf(), []);
  assert.equal(t.aufrufe("voice_call_chat").length, 1);
});

test("Vor dem Auftrag (Sprecher hat noch nicht entschieden): die Fortsetzung verwirft die Anfrage wie heute", async () => {
  const s = lokalFolge(["WEITER: Ich schaue nach."], 400);
  const t = neu(
    {
      voice_call_chat: nacheinander(
        denker(300, "Alte Antwort."),
        denker(40, ANTWORT),
      ),
    },
    s,
  );
  const { turn: a1 } = t.startSprache("Was steht in meinen");
  await settle(40);
  t.aeussere("Was steht in meinen Mails heute?", { mitBarge: false });
  assert.equal(await bis(() => t.player.enqueued.includes(ERGEBNIS)), true);
  assert.equal(t.abgebrochen(a1.id), 1, "die alte Anfrage ist verworfen");
  assert.ok(!t.engine.auftrag);
  assert.equal(
    t.aufrufe("voice_call_chat")[1].args.messages.at(-1).content,
    "Was steht in meinen Mails heute?",
  );
  assert.equal(
    t.player.enqueued.some((w) => w.includes("Alte Antwort")),
    false,
  );
});

test("Denker ist schon hörbar: auch nach WEITER kein Auftrag, die Ergänzung führt wie heute zusammen", async () => {
  const s = lokalFolge(["WEITER: Ich schaue nach."]);
  const t = neu(
    {
      voice_call_chat: nacheinander(
        denker(60, "Zwei Mails sind da."),
        denker(20, ANTWORT),
      ),
    },
    s,
  );
  const { turn: a1 } = t.startSprache(FRAGE);
  await bis(() => t.player.gespielt.includes("wav:Ich schaue nach."));
  await bis(() => t.player.enqueued.includes("wav:Zwei Mails sind da."));
  t.player.finishPiece(); // das Weitersprechen ist zu Ende, der erste Satz des Denkers beginnt
  assert.ok(t.player.gespielt.includes("wav:Zwei Mails sind da."));
  t.aeussere("Aber nur die von heute.");
  assert.equal(await bis(() => t.player.enqueued.includes(ERGEBNIS)), true);
  assert.ok(!t.engine.auftrag);
  assert.equal(t.abgebrochen(a1.id), 1);
  assert.equal(
    t.aufrufe("voice_call_chat")[1].args.messages.at(-1).content,
    fuegeZusammen(FRAGE, "Aber nur die von heute."),
  );
});

test("Nach Ablauf von auftragMaxMs wird der Schritt nicht mehr zum Auftrag (wie heute)", async () => {
  await mitTuning({ auftragMaxMs: 80 }, async () => {
    const s = lokalFolge([WEITER_ZEILE]);
    const h = haengend();
    const t = neu(
      {
        voice_call_chat: nacheinander(h.chat, denker(20, ANTWORT)),
        voice_call_cancel: h.cancel,
      },
      s,
    );
    const { turn: a1 } = t.startSprache(FRAGE);
    await settle(150);
    t.aeussere("Aber nur die von heute.");
    assert.equal(await bis(() => t.player.enqueued.includes(ERGEBNIS)), true);
    assert.ok(!t.engine.auftrag);
    assert.equal(t.abgebrochen(a1.id), 1);
  });
});

test("Nachreichen wartet auf den freien Vordergrund: Äußerung, Antwort und Stille kommen zuerst", async () => {
  // Siehe auch „Ergebnis schon da“ (offene Äußerung); hier: laufender Schritt und Halten.
  await mitTuning({ nachreichenStilleMs: 120 }, async () => {
    const s = lokalFolge([WEITER_ZEILE, ZWEITE_ZEILE]);
    const t = neu({ voice_call_chat: denker(100, ANTWORT) }, s);
    t.startSprache(FRAGE);
    await settle(40);
    t.aeussere(FOLGE);
    await bis(() => t.player.gespielt.includes(ZWEITE_SATZ));
    // Der Denker ist fertig, aber die Antwort des Sprechers läuft noch: nichts wird nachgereicht.
    await settle(350);
    assert.equal(t.engine.auftrag.fertig, true);
    assert.equal(t.player.gespielt.includes(ERGEBNIS), false);
    // Der Nutzer hält die Antwort an (Halten): auch dann nicht.
    t.barge();
    await settle(250);
    assert.equal(t.player.gespielt.includes(ERGEBNIS), false);
    t.engine.onEnd(); // nichts erkannt: die Antwort läuft weiter
    t.player.finish();
    assert.equal(await bis(() => t.player.gespielt.includes(ERGEBNIS)), true);
  });
});

test("Nachreichen vor dem ersten Ton: spricht der Nutzer los, wird zurückgestellt und danach erneut versucht", async () => {
  await mitTuning({ nachreichenStilleMs: 100 }, async () => {
    let tor;
    const gate = new Promise((r) => (tor = r));
    let erster = true;
    const s = lokalFolge([WEITER_ZEILE, ZWEITE_ZEILE, WETTER_ZEILE]);
    const t = neu(
      {
        voice_call_chat: denker(60, ANTWORT),
        voice_call_synthesize: ({ text }) => {
          if (text === ANTWORT && erster) {
            erster = false;
            return gate.then(() => `wav:${text}`);
          }
          return `wav:${text}`;
        },
      },
      s,
    );
    t.startSprache(FRAGE);
    await settle(40);
    t.aeussere(FOLGE);
    await bis(() => t.player.gespielt.includes(ZWEITE_SATZ));
    t.player.finish();
    // Das Nachreichen beginnt, die Synthese hängt: noch nichts hörbar.
    assert.equal(await bis(() => t.engine.turn?.nachreichen), true);
    assert.equal(t.player.gespielt.includes(ERGEBNIS), false);
    t.redeLos();
    assert.equal(t.engine.turn, null, "zurückgestellt");
    assert.ok(
      t.engine.auftrag && !t.engine.auftrag.verworfen,
      "Auftrag bleibt",
    );
    t.erkenne(WETTER);
    t.fruehEnde();
    t.ende();
    assert.equal(
      await bis(() => t.player.gespielt.includes(WETTER_SATZ)),
      true,
    );
    tor(); // die zurückgestellte Synthese kommt zu spät: sie wird nicht gespielt
    await settle(30);
    assert.equal(t.player.gespielt.includes(ERGEBNIS), false);
    t.player.finish();
    assert.equal(await bis(() => t.player.gespielt.includes(ERGEBNIS)), true);
    assert.equal(t.player.gespielt.filter((w) => w === ERGEBNIS).length, 1);
    assert.deepEqual(t.player.gespielt.slice(-2), [WETTER_SATZ, ERGEBNIS]);
    assert.equal(t.log.errors.length, 0);
  });
});

test("Nachreichen schon hörbar: der Nutzer unterbricht wie sonst, der Rest ist verworfen, der Auftrag erledigt", async () => {
  await mitTuning({ nachreichenStilleMs: 100 }, async () => {
    const s = lokalFolge([WEITER_ZEILE, ZWEITE_ZEILE, WETTER_ZEILE]);
    const t = neu(
      { voice_call_chat: denker(60, "Satz eins.", "Satz zwei.") },
      s,
    );
    t.startSprache(FRAGE);
    await settle(40);
    t.aeussere(FOLGE);
    await bis(() => t.player.gespielt.includes(ZWEITE_SATZ));
    t.player.finish();
    assert.equal(
      await bis(() => t.player.gespielt.includes("wav:Satz eins.")),
      true,
    );
    t.aeussere(WETTER);
    assert.equal(
      await bis(() => t.player.gespielt.includes(WETTER_SATZ)),
      true,
    );
    assert.equal(t.player.gespielt.includes("wav:Satz zwei."), false);
    assert.equal(t.engine.auftrag, null);
    const unterbrochen = t.engine.history.find(
      (m) =>
        m.role === "assistant" &&
        m.interrupted &&
        m.content.includes("Satz eins"),
    );
    assert.ok(unterbrochen);
    assert.equal(unterbrochen.content.includes("Satz zwei"), false);
    t.player.finish();
    await settle(350);
    assert.equal(t.player.gespielt.includes("wav:Satz zwei."), false);
    assert.equal(
      t.player.gespielt.filter((w) => w === "wav:Satz eins.").length,
      1,
    );
  });
});

test("Stoppwort bei freiem Vordergrund verwirft den Auftrag still und geht nicht ans Modell", async () => {
  const { s, t, a1 } = await aufbau({ denker: denker(300, ANTWORT) });
  t.player.finish();
  t.aeussere("Stopp.", { mitBarge: false });
  assert.equal(await bis(() => t.engine.auftrag === null), true);
  assert.equal(t.abgebrochen(a1.id), 1);
  await settle(400); // der alte Denker meldet sich spät
  assert.deepEqual(t.player.gespielt, [WEITER_SATZ, ZWEITE_SATZ]);
  assert.equal(t.engine.turn, null);
  assert.equal(t.engine.state, "listening");
  assert.deepEqual(t.log.user, [FRAGE, FOLGE], "„Stopp“ ist keine Nutzerzeile");
  assert.equal(s.aufrufe.length, 2, "der Sprecher wurde nicht gefragt");
  assert.equal(t.aufrufe("voice_call_chat").length, 1);
});

test("Stoppwort während einer Antwort verwirft den Auftrag mit der Antwort", async () => {
  const { t, a1 } = await aufbau({ denker: denker(300, ANTWORT) });
  const gestoppt = t.player.stopped;
  t.aeussere("Stopp.");
  assert.equal(await bis(() => t.engine.auftrag === null), true);
  assert.equal(t.abgebrochen(a1.id), 1);
  assert.ok(t.player.stopped > gestoppt);
  await settle(400);
  assert.equal(t.player.gespielt.includes(ERGEBNIS), false);
  assert.equal(t.engine.turn, null);
});

test("60 s ohne fertiges Ergebnis: der Auftrag wird still verworfen (keine Vorlage, kein Verlauf)", async () => {
  await mitTuning({ auftragMaxMs: 500, nachreichenStilleMs: 100 }, async () => {
    const h = haengend();
    const { t, a1 } = await aufbau({
      handlers: { voice_call_chat: h.chat, voice_call_cancel: h.cancel },
    });
    t.player.finish();
    assert.ok(t.engine.auftrag && !t.engine.auftrag.verworfen);
    assert.equal(await bis(() => t.engine.auftrag === null, 2000), true);
    assert.equal(t.abgebrochen(a1.id), 1);
    await settle(300);
    assert.deepEqual(t.player.gespielt, [WEITER_SATZ, ZWEITE_SATZ]);
    assert.deepEqual(t.verlauf(), [
      `user:${FOLGE}`,
      "assistant:Es ist kurz nach drei.",
    ]);
    assert.equal(t.log.errors.length, 0);
    assert.ok(
      t.log.denkenRoh.some((e) => e.art === "ende" && e.turnId === a1.id),
    );
  });
});

for (const [name, handler] of [
  [
    "Fehler des Tors",
    () => new Promise((_, aus) => setTimeout(() => aus("offline"), 120)),
  ],
  [
    "Ereignis failed",
    ({ requestId }) =>
      new Promise((ok) =>
        setTimeout(() => {
          emit({ kind: "failed", request_id: requestId });
          ok("");
        }, 120),
      ),
  ],
  [
    "leeres Ergebnis",
    ({ requestId }) =>
      new Promise((ok) =>
        setTimeout(() => {
          emit({ kind: "done", request_id: requestId });
          ok("");
        }, 120),
      ),
  ],
]) {
  test(`Denker-Auftrag: ${name} verwirft still (keine Vorlage, kein Verlauf, kein Nachreichen)`, async () => {
    await mitTuning({ nachreichenStilleMs: 100 }, async () => {
      const { t } = await aufbau({ denker: handler });
      t.player.finish();
      assert.equal(await bis(() => t.engine.auftrag === null, 1500), true);
      await settle(300);
      assert.deepEqual(t.player.gespielt, [WEITER_SATZ, ZWEITE_SATZ]);
      assert.deepEqual(t.verlauf(), [
        `user:${FOLGE}`,
        "assistant:Es ist kurz nach drei.",
      ]);
      assert.equal(t.log.errors.length, 0);
      assert.equal(t.engine.turn, null);
    });
  });
}

const TRIGGER = {
  "Ende des Gesprächs (stop)": (t) => t.engine.stop(),
  "Neues Gespräch (resetHistory)": (t) => t.engine.resetHistory(),
  "Esc oder Bereichswechsel (interrupt)": (t) => t.engine.interrupt(),
  "Modellwechsel (updateConfig)": (t) =>
    t.engine.updateConfig({ ...t.config, orchestration: { modell: "b" } }),
  "Widerruf der Freigabe (revoked)": (_t, a1) =>
    emit({ kind: "revoked", request_id: a1.id }),
  "getippter Text (sendText)": (t) => t.engine.sendText("Hallo"),
};
for (const [name, ausloeser] of Object.entries(TRIGGER)) {
  test(`${name} verwirft den Auftrag: ein Abbruch, späte Sätze ohne Ausgabe`, async () => {
    await mitTuning({ nachreichenStilleMs: 100 }, async () => {
      const { t, a1 } = await aufbau({ denker: denker(300, ANTWORT) });
      assert.ok(t.engine.auftrag && !t.engine.auftrag.verworfen);
      await ausloeser(t, a1);
      assert.equal(t.engine.auftrag, null);
      assert.equal(
        t.abgebrochen(a1.id),
        1,
        "genau ein Abbruch, kein doppelter",
      );
      await settle(300); // der alte Denker meldet sich spät, danach Stille
      assert.equal(
        t.player.enqueued.some((w) => w.includes("Mails sind gekommen")),
        false,
      );
      assert.equal(
        t.log.assistant.some((a) => a.t.includes("Mails sind gekommen")),
        false,
      );
      assert.equal(t.abgebrochen(a1.id), 1);
    });
  });
}

test("Wechsel der Wissensbereiche ist kein Modellwechsel: ein Auftrag ohne Orchestrator bleibt offen", async () => {
  await mitTuning({ nachreichenStilleMs: 100 }, async () => {
    const { t, a1 } = await aufbau({ denker: denker(300, ANTWORT) });
    t.engine.invalidateKnowledge();
    assert.ok(t.engine.auftrag && !t.engine.auftrag.verworfen);
    t.player.finish();
    assert.equal(await bis(() => t.player.gespielt.includes(ERGEBNIS)), true);
    assert.equal(t.abgebrochen(a1.id), 0);
  });
});

test("Verwerfen ist mehrfach und gleichzeitig aufrufbar: nur ein Abbruch, kein Fehler", async () => {
  const { t, a1 } = await aufbau({ denker: denker(300, ANTWORT) });
  t.engine.verwerfeAuftrag();
  t.engine.verwerfeAuftrag();
  t.engine.interrupt();
  await t.engine.stop();
  t.engine.resetHistory();
  assert.equal(t.abgebrochen(a1.id), 1);
  await settle(400);
  assert.equal(t.engine.auftrag, null);
  assert.equal(t.log.errors.length, 0);
});

test("Satzpuffer des Auftrags ist gedeckelt: Überlauf wird gezählt und gemeldet, der Rest verworfen", async () => {
  const warnungen = [];
  const warn = console.warn;
  console.warn = (...a) => warnungen.push(a.join(" "));
  try {
    await mitTuning(
      { auftragMaxZeichen: 40, nachreichenStilleMs: 100 },
      async () => {
        const { t } = await aufbau({
          denker: denker(
            250,
            "Erster Satz, kurz.",
            "Zweiter Satz, auch kurz.",
            "Dritter Satz, ebenfalls.",
          ),
        });
        t.player.finish();
        assert.equal(await bis(() => t.engine.auftrag?.fertig), true);
        assert.equal(t.engine.auftrag.ueberlauf, 2, "gezählt");
        assert.equal(warnungen.length, 1, "einmal gemeldet, ohne Inhalt");
        assert.equal(/Erster|Zweiter|Dritter/.test(warnungen[0]), false);
        assert.equal(
          await bis(() => t.player.gespielt.includes("wav:Erster Satz, kurz.")),
          true,
        );
        assert.equal(
          t.player.enqueued.some((w) => w.includes("Zweiter")),
          false,
        );
      },
    );
  } finally {
    console.warn = warn;
  }
});

test("Synthese schlägt beim Nachreichen fehl: ein Fehler, kein Dauerversuch, der Auftrag ist erledigt", async () => {
  await mitTuning({ nachreichenStilleMs: 100 }, async () => {
    const { t } = await aufbau({
      denker: denker(100, ANTWORT),
      handlers: {
        voice_call_synthesize: ({ text }) => {
          if (text === ANTWORT) throw new Error("Stimme nicht verfügbar");
          return `wav:${text}`;
        },
      },
    });
    t.player.finish();
    assert.equal(await bis(() => t.engine.auftrag === null, 2000), true);
    await settle(400);
    assert.equal(
      t.aufrufe("voice_call_synthesize").filter((c) => c.args.text === ANTWORT)
        .length,
      1,
    );
    assert.equal(t.log.errors.length, 1);
    assert.equal(t.engine.turn, null);
    assert.equal(t.engine.state, "listening");
    assert.deepEqual(t.verlauf(), [
      `user:${FOLGE}`,
      "assistant:Es ist kurz nach drei.",
    ]);
  });
});

test("Der Player wirft beim Nachreichen: eine Meldung, kein unbehandelter Fehler, der Auftrag ist erledigt", async () => {
  await mitTuning({ nachreichenStilleMs: 100 }, async () => {
    const { t } = await aufbau({ denker: denker(150, ANTWORT) });
    t.player.finish();
    t.player.enqueue = async () => {
      throw new Error("Audio weg");
    };
    assert.equal(await bis(() => t.engine.auftrag === null, 2000), true);
    await settle(300);
    assert.equal(t.log.errors.length, 1);
    assert.equal(t.engine.turn, null);
    assert.equal(t.engine.state, "listening");
    assert.deepEqual(t.verlauf(), [
      `user:${FOLGE}`,
      "assistant:Es ist kurz nach drei.",
    ]);
  });
});

test("Ohne Player (Gerät weg): das Ergebnis wird nicht nachgereicht, nichts stürzt ab", async () => {
  await mitTuning({ nachreichenStilleMs: 50 }, async () => {
    const { t } = await aufbau({ denker: denker(150, ANTWORT) });
    t.player.finish();
    t.engine.player = null;
    await settle(400);
    assert.ok(t.engine.auftrag && t.engine.auftrag.fertig);
    assert.equal(t.player.gespielt.includes(ERGEBNIS), false);
    assert.equal(t.engine.turn, null);
    await t.engine.stop();
    assert.equal(t.engine.auftrag, null);
    assert.equal(t.log.errors.length, 0);
  });
});

test("Der Denker liefert, während die Äußerung erkannt wird: sein Satz wird gepuffert, nie gespielt", async () => {
  await mitTuning({ nachreichenStilleMs: 100 }, async () => {
    const s = lokalFolge([WEITER_ZEILE, ZWEITE_ZEILE]);
    const t = neu(
      {
        voice_call_chat: denker(70, ANTWORT),
        voice_call_transcribe: async () => {
          await settle(150);
          return FOLGE;
        },
      },
      s,
    );
    const { turn: a1 } = t.startSprache(FRAGE);
    await settle(40);
    t.barge();
    t.fruehEnde();
    t.ende();
    await settle(250);
    assert.ok(t.engine.auftrag);
    assert.equal(t.abgebrochen(a1.id), 0);
    // Der Satz des Denkers war in der Warteschlange, aber nie zu hören.
    assert.equal(t.player.gespielt.includes(ERGEBNIS), false);
    assert.ok(t.player.gespielt.includes(ZWEITE_SATZ));
    t.player.finish();
    assert.equal(await bis(() => t.player.gespielt.includes(ERGEBNIS)), true);
    assert.equal(t.player.gespielt.filter((w) => w === ERGEBNIS).length, 1);
  });
});

test("Zeile des Weitersprechens beim Übergang: Gehörtes bleibt als unterbrochene Zeile stehen", async () => {
  const { t, a1 } = await aufbau({ denker: denker(300, ANTWORT) });
  assert.deepEqual(
    t.log.assistant.find((a) => a.final && a.t.startsWith("Gute Frage")),
    { t: "Gute Frage, ich schaue nach.", final: true, interrupted: true },
  );
  assert.equal(
    t.log.assistant.some((a) => a.t.includes(ANTWORT)),
    false,
  );
  assert.equal(t.log.verworfen, 0);
  const auftrag = t.log.denkenRoh.find((e) => e.art === "auftrag");
  assert.equal(auftrag.turnId, a1.id);
  assert.equal(auftrag.frage, FRAGE);
});

test("Zeile des Weitersprechens beim Übergang: war nichts zu hören, verschwindet die Zeile", async () => {
  const s = lokalFolge([WEITER_ZEILE, ZWEITE_ZEILE]);
  const t = neu({ voice_call_chat: denker(300, ANTWORT) }, s);
  t.player.deferStart = true; // eingereiht, aber noch nicht zu hören
  t.startSprache(FRAGE);
  await settle(40);
  t.aeussere(FOLGE);
  await settle(60);
  assert.ok(t.engine.auftrag);
  assert.equal(t.log.verworfen, 1);
  assert.equal(
    t.log.assistant.some((a) => a.final && a.t.startsWith("Gute Frage")),
    false,
  );
});

// -- Quellen des nachgereichten Ergebnisses ---------------------------------------------------

const BELEG_ZEILE = "Anna übernimmt das Protokoll[Q1].";
const BELEG_SATZ = "wav:Anna übernimmt das Protokoll.";
/** Quellenmeldungen zu dieser Turn-Id. */
const quellen = (t, id) => t.log.citations.filter((c) => c.id === id);

test("Nachreichen meldet die Quellen genau einmal mit der Turn-Id des Auftrags", async () => {
  await mitTuning({ nachreichenStilleMs: 100 }, async () => {
    const { t, a1 } = await aufbau({
      denker: denker(200, BELEG_ZEILE),
      optionen: { kontext: true },
    });
    assert.equal(
      t.engine.auftrag.turn.withContext,
      true,
      "Schritt mit Kontext",
    );
    assert.equal(quellen(t, a1.id).length, 0, "vor dem Nachreichen nichts");
    t.player.finish();
    assert.equal(await bis(() => t.player.gespielt.includes(BELEG_SATZ)), true);
    assert.equal(await bis(() => quellen(t, a1.id).length > 0), true);
    await settle(100);
    assert.deepEqual(quellen(t, a1.id), [
      { id: a1.id, n: 1, nachgereicht: true },
    ]);
    // Geprüft wurde der Text des Denkers samt Belegmarke, einmal.
    const pruefungen = t
      .aufrufe("app_context_verify")
      .filter((c) => c.args.answer.includes("Anna übernimmt"));
    assert.equal(pruefungen.length, 1);
    assert.match(pruefungen[0].args.answer, /\[Q1\]/);
    t.player.finish();
    await settle(30);
    assert.equal(quellen(t, a1.id).length, 1);
  });
});

for (const [name, wie] of Object.entries({
  ABBRUCH: { zeilen: [WEITER_ZEILE, "ABBRUCH: Alles klar."] },
  SUCHE: {
    zeilen: [WEITER_ZEILE, "SUCHE: Alles klar, nur Montag."],
    handlers: {
      voice_call_chat: nacheinander(
        denker(250, BELEG_ZEILE),
        denker(30, "Neue Antwort."),
      ),
    },
  },
  "neues Gespräch": { aus: (t) => t.engine.resetHistory() },
  "Widerruf des Zugriffs": {
    aus: () => emit({ kind: "revoked", request_id: "*" }),
  },
  "Änderung der Wissensbereiche": {
    aus: (t) => t.engine.invalidateKnowledge(),
  },
})) {
  test(`Quellen: ein durch ${name} verworfener Auftrag meldet nie welche`, async () => {
    await mitTuning({ nachreichenStilleMs: 100 }, async () => {
      const { t, a1 } = await aufbau({
        zeilen: wie.zeilen,
        denker: denker(250, BELEG_ZEILE),
        handlers: wie.handlers,
        optionen: { kontext: true },
      });
      if (wie.aus) {
        assert.ok(t.engine.auftrag && !t.engine.auftrag.verworfen);
        wie.aus(t);
      }
      assert.equal(t.engine.auftrag, null);
      t.player.finish();
      await settle(500); // der alte Denker meldet sich spät, danach Stille
      assert.equal(
        t.player.enqueued.some((w) => w.includes("Anna übernimmt")),
        false,
      );
      assert.equal(quellen(t, a1.id).length, 0);
      assert.equal(
        t
          .aufrufe("app_context_verify")
          .some((c) => c.args.answer.includes("Anna übernimmt")),
        false,
      );
    });
  });
}

test("Quellen: Auftrag ohne Kontext (Cloud-Denker) meldet beim Nachreichen nichts", async () => {
  await mitTuning({ nachreichenStilleMs: 100 }, async () => {
    const { t, a1 } = await aufbau({ denker: denker(200, BELEG_ZEILE) });
    t.player.finish();
    assert.equal(await bis(() => t.player.gespielt.includes(BELEG_SATZ)), true);
    await settle(100);
    assert.equal(quellen(t, a1.id).length, 0);
    assert.equal(t.aufrufe("app_context_verify").length, 0);
  });
});

test("Quellen: ist das Ergebnis schon fertig und kommt ohne Unterbrechung, melden sie erst, wenn der Inhalt beginnt", async () => {
  const s = lokalFolge(["WEITER: Ich schaue nach."]);
  const t = neu({ voice_call_chat: denker(40, BELEG_ZEILE) }, s, {
    kontext: true,
  });
  const { turn: a1 } = t.startSprache(FRAGE);
  await settle(200);
  // Alles ist erzeugt und eingereiht, hinter dem Weitersprechen: noch nichts zu hören, noch keine Quellen.
  assert.equal(quellen(t, a1.id).length, 0);
  t.player.finishPiece();
  assert.equal(await bis(() => quellen(t, a1.id).length > 0), true);
  await settle(50);
  assert.deepEqual(quellen(t, a1.id), [
    { id: a1.id, n: 1, nachgereicht: false },
  ]);
  assert.equal(t.engine.auftrag, null);
});

test("Quellen: wird das schon fertige Ergebnis zum Auftrag, meldet nur das Nachreichen, einmal", async () => {
  await mitTuning({ nachreichenStilleMs: 100 }, async () => {
    const s = lokalFolge([WEITER_ZEILE, ZWEITE_ZEILE]);
    const t = neu(
      {
        voice_call_chat: denker(70, BELEG_ZEILE),
        voice_call_transcribe: async () => {
          await settle(150);
          return FOLGE;
        },
      },
      s,
      { kontext: true },
    );
    const { turn: a1 } = t.startSprache(FRAGE);
    await settle(40);
    t.barge();
    t.fruehEnde();
    t.ende();
    await settle(300);
    assert.ok(t.engine.auftrag && t.engine.auftrag.fertig);
    assert.equal(
      quellen(t, a1.id).length,
      0,
      "nichts für ein ungehörtes Ergebnis",
    );
    t.player.finish();
    assert.equal(await bis(() => t.player.gespielt.includes(BELEG_SATZ)), true);
    assert.equal(await bis(() => quellen(t, a1.id).length > 0), true);
    await settle(100);
    assert.deepEqual(quellen(t, a1.id), [
      { id: a1.id, n: 1, nachgereicht: true },
    ]);
  });
});

// -- Nicht betroffen: ohne lokales Modell, Projekt, Textmodus ------------------------------------

test("Ohne lokales Modell: kein Auftrag, die Äußerung verwirft den Schritt wie heute", async () => {
  const t = neu(
    {
      voice_call_chat: nacheinander(
        denker(300, "Alte Antwort."),
        denker(20, ANTWORT),
      ),
    },
    null,
  );
  const { turn: a1 } = t.startSprache(FRAGE);
  await settle(40);
  t.aeussere("Was steht in meinen Mails heute?", { mitBarge: false });
  assert.equal(await bis(() => t.player.enqueued.includes(ERGEBNIS)), true);
  assert.ok(!t.engine.auftrag);
  assert.equal(t.abgebrochen(a1.id), 1);
  await settle(350);
  assert.equal(
    t.player.enqueued.some((w) => w.includes("Alte Antwort")),
    false,
  );
});

test("Gesprächsmodell ist lokal (kein Sofort-Sprecher): unverändert, kein Auftrag", async () => {
  // `sofortModell` liefert dann null: die Orchestrierung gibt keinen Sprecher mit.
  const t = neu(
    {
      voice_call_chat: nacheinander(
        denker(300, "Alte Antwort."),
        denker(20, ANTWORT),
      ),
    },
    undefined,
  );
  const { turn: a1 } = t.startSprache(FRAGE);
  await settle(40);
  t.aeussere("Was steht in meinen Mails heute?", { mitBarge: false });
  assert.equal(await bis(() => t.player.enqueued.includes(ERGEBNIS)), true);
  assert.ok(!t.engine.auftrag);
  assert.equal(t.abgebrochen(a1.id), 1);
});

test("Im Projekt: kein Sprecher, kein Auftrag, die Äußerung verwirft den Schritt wie heute", async () => {
  const s = lokalFolge([WEITER_ZEILE, ZWEITE_ZEILE]);
  const t = neu(
    {
      voice_call_chat: nacheinander(
        denker(300, "Alte Antwort."),
        denker(20, ANTWORT),
      ),
    },
    s,
    { projekt: { id: "p1", stand: 1 } },
  );
  const { turn: a1 } = t.startSprache(FRAGE);
  await settle(40);
  t.aeussere("Und was steht im Projekt?", { mitBarge: false });
  await settle(300);
  assert.equal(s.aufrufe.length, 0, "im Projekt läuft kein Sprecher");
  assert.ok(!t.engine.auftrag);
  assert.equal(t.abgebrochen(a1.id), 1);
  assert.equal(
    t.player.enqueued.some((w) => w.includes("Alte Antwort")),
    false,
  );
});

test("Textmodus ohne Anruf: kein Auftrag, Sprecher und Denker wie bisher", async () => {
  const s = lokalFolge(["WEITER: Schauen wir in die Mails."]);
  const t = neu({ voice_call_chat: denker(40, ANTWORT) }, s);
  t.engine.state = "idle";
  await t.engine.sendText(FRAGE);
  assert.ok(!t.engine.auftrag);
  assert.equal(
    t.log.assistant.at(-1).t,
    `Schauen wir in die Mails. ${ANTWORT}`,
  );
  assert.ok(
    s.aufrufe.every((a) => !/Offener Suchauftrag/.test(a.messages[0].content)),
  );
});

// -- Denker über `side` (Orchestrator im Prozess) -------------------------------------------------

function seite(ms, ...saetze) {
  const side = {
    chats: [],
    abgebrochen: [],
    async chat(id, messages, onModel) {
      side.chats.push({ id, messages });
      if (ms === undefined) return new Promise(() => undefined);
      await settle(ms);
      for (const text of saetze) onModel(text);
      return saetze.join(" ");
    },
    cancel(id) {
      side.abgebrochen.push(id);
    },
  };
  return side;
}

test("Denker über side: der Auftrag läuft ohne Abbruch weiter, SUCHE bricht ihn per side.cancel ab", async () => {
  const side = seite(250, "Alte Antwort.");
  const s = lokalFolge([WEITER_ZEILE, ZWEITE_ZEILE]);
  const t = neu({}, s, { side });
  const { turn: a1 } = t.startSprache(FRAGE);
  await settle(40);
  t.aeussere(FOLGE);
  await settle(60);
  assert.ok(t.engine.auftrag && !t.engine.auftrag.verworfen);
  assert.deepEqual(side.abgebrochen, [], "kein side.cancel für den Auftrag");
  assert.equal(t.abgebrochen(a1.id), 0);
  // Eine Änderung der Wissensbereiche bricht den Orchestrator ab und nimmt den Auftrag mit.
  t.engine.invalidateKnowledge();
  assert.equal(t.engine.auftrag, null);
  assert.deepEqual(side.abgebrochen, [a1.id]);
  await settle(300); // der Orchestrator meldet sich spät
  assert.equal(
    t.player.enqueued.some((w) => w.includes("Alte Antwort")),
    false,
  );
});

test("Denker über side: SUCHE ersetzt den Lauf, das Ergebnis des neuen kommt", async () => {
  const alt = seite(250, "Alte Antwort.");
  const neuerDenker = seite(30, "Neue Antwort.");
  const von = new Map();
  let n = 0;
  const side = {
    cancel: (id) => von.get(id)?.cancel(id),
    chat: (id, m, f) => {
      const d = n++ === 0 ? alt : neuerDenker;
      von.set(id, d);
      return d.chat(id, m, f);
    },
  };
  const s = lokalFolge(["WEITER: Ich schaue nach.", "SUCHE:"]);
  const t = neu({}, s, { side });
  const { turn: a1 } = t.startSprache(FRAGE);
  await settle(40);
  t.aeussere("Nur die von Montag bitte.");
  assert.equal(
    await bis(() => t.player.gespielt.includes("wav:Neue Antwort.")),
    true,
  );
  assert.deepEqual(alt.abgebrochen, [a1.id]);
  assert.match(s.aufrufe[1].messages[0].content, /Offener Suchauftrag/);
  await settle(300);
  assert.equal(
    t.player.enqueued.some((w) => w.includes("Alte Antwort")),
    false,
  );
});
