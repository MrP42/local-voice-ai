// Sofort-Sprecher (Patrick 12.11.2026): reine Logik und Gesprächsschritte der Engine mit nachgeahmtem
// Tauri. Lauf: node --test scripts/sofort.test.mjs (Node >= 22.6).
import assert from "node:assert/strict";
import { register } from "node:module";
import test, { after, mock } from "node:test";

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
const { mitQuellen, denkOffenSetzen } =
  await import("../src/stores/voiceLines.ts");
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

// Die echte Uhr: Tests mit gesteuerter Uhr (`mitUhr`) ersetzen `setTimeout` der Engine, nicht das Warten des Tests.
const echtesTimeout = globalThis.setTimeout;
const settle = (ms = 5) => new Promise((r) => echtesTimeout(r, ms));
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

/** Alle Engines des Laufs: am Ende angehalten, damit keine Frist (60 s) den Prozess am Beenden hindert. */
const engines = [];
after(async () => {
  for (const e of engines) await e.stop().catch(() => undefined);
});

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
    /** Wie `assistant`, mit der Kennung des Auftrags (`auftragVon`) der Zeile. */
    zeilen: [],
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
  const mitLog = {
    onUser: (t) => log.user.push(t),
    onAssistant: (t, final, interrupted, auftragVon) => {
      log.assistant.push({ t, final, interrupted });
      log.zeilen.push({ t, final, interrupted, auftragVon });
    },
    onError: (e) => log.errors.push(String(e)),
    onVerwerfen: () => log.verworfen++,
    onCitations: (id, hits, nachgereicht) =>
      log.citations.push({ id, n: hits.length, nachgereicht }),
    onDenken: (e) => {
      log.denken.push(e.art);
      log.denkenRoh.push(e);
    },
  };
  // Mit `optionen.engine` (die Engine des echten Gesprächsspeichers) läuft alles durch seine Rückrufe weiter,
  // das Protokoll sieht mit.
  const engine =
    optionen.engine ??
    new VoiceCallEngine(
      {
        onState() {},
        onLevel() {},
        onMetrics() {},
        ...mitLog,
        conversationId: optionen.conversationId ?? (() => "c1"),
        ...(optionen.projekt ? { projekt: () => optionen.projekt } : {}),
      },
      config,
    );
  if (optionen.engine) {
    engine.cbEcht ??= engine.cb;
    const echt = engine.cbEcht;
    engine.cb = {
      ...echt,
      ...Object.fromEntries(
        Object.entries(mitLog).map(([name, f]) => [
          name,
          (...a) => {
            f(...a);
            return echt[name]?.(...a);
          },
        ]),
      ),
    };
  }
  engines.push(engine);
  const player = new SpeechPlayer(null);
  // Was wirklich zu hören war (Beginn des Häppchens), nicht nur eingereiht.
  player.gespielt = [];
  const einreihen = player.enqueue.bind(player);
  player.enqueue = (wav, onEnd, onStart) =>
    einreihen(wav, onEnd, () => {
      player.gespielt.push(wav);
      onStart?.();
    });
  // Wie in `start()`: das Ende der Wiedergabe geht an die Engine (Ende des Schritts, Wartezeit auf das nächste Stück).
  player.onIdle = () => engine.playerLeer();
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
  /** Wie in der App: ein Fehler beim Antworten meldet sich und beendet den Schritt (`antworteSicher`, `dropTurn`). */
  const startSicher = (text) => {
    const turn = engine.newTurn(0, performance.now());
    engine.turn = turn;
    turn.committed = true;
    turn.commit();
    return engine.antworteSicher(turn, text, 0);
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
    startSicher,
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

// -- Runde 3 (Codex zu lva#547) ----------------------------------------------------------------------
//
// Fehlerfälle dieser Runde (Status im Bericht):
//  G1 Nebenläufigkeit: die späte Erkennung einer Äußerung wirkt nicht, wenn inzwischen getippt, der Anruf beendet
//     oder neu gestartet oder das Gespräch zurückgesetzt wurde (ein beendeter Anruf lebt nicht wieder auf).
//  G2 Abbruch mitten im Vorgang: das Nachreichen ist eingereiht, aber ohne Ton, wenn der Nutzer losspricht:
//     zurückgestellt, nicht verworfen (maßgeblich ist der tatsächliche Beginn des Tons, nicht das Einreihen).
//  G3 Gedächtnis: das Tor in Rust gehört dem offenen Auftrag; eine kurze Sprecherantwort löscht es nicht, und
//     ein noch ungesprochenes Ergebnis wird nicht vorweg wiederholt. Fertige, nie gesprochene Ergebnisse verfallen.
//  G4 Getippter Text: nie ein Auftrag (seine Antwortzeile ginge schon beim Fertigwerden hinaus).

/** Gedächtnis des Tors in Rust, nachgeahmt: je Gespräch die letzte Ausgabe (`assistent_letzte_ausgabe`/`…_vergessen`). */
function gedaechtnis(vorher) {
  const m = new Map(vorher ? [["c1", vorher]] : []);
  const key = (a) => a?.conversationId ?? "";
  return {
    m,
    handlers: {
      assistent_letzte_ausgabe: (a) => m.get(key(a)) ?? null,
      assistent_ausgabe_vergessen: (a) => {
        m.delete(key(a));
        return null;
      },
    },
  };
}

/**
 * Denker über `voice_call_chat`, der erst auf `liefere(i, …)` antwortet (kein Zeitgeber). Wie das Tor in Rust merkt er
 * die Ausgabe am Ende des Laufs (`rust`); ein Abbruch löst ihn ohne Ergebnis.
 */
function denkerSteuerbar(rust) {
  const laeufe = [];
  return {
    laeufe,
    chat: (args) => new Promise((resolve) => laeufe.push({ ...args, resolve })),
    cancel: ({ requestId }) => {
      laeufe.find((l) => l.requestId === requestId)?.resolve("");
      return null;
    },
    liefere: (i, ...saetze) => {
      const l = laeufe[i];
      const text = saetze.join(" ");
      for (const satz of saetze)
        emit({ kind: "sentence", request_id: l.requestId, text: satz });
      rust?.m.set(l.conversationId ?? "", text);
      emit({ kind: "done", request_id: l.requestId });
      l.resolve(text);
    },
  };
}

/**
 * Wie `aufbau`, ohne Zeitgeber: der Sprecher sagt WEITER, der Denker (`d`) wartet auf `liefere`, die Äußerung
 * (mit Pegel-Barge-in) wird vom Sprecher kurz beantwortet. Danach ist der erste Schritt ein offener Auftrag.
 */
async function aufbauSteuerbar({
  rust,
  zeilen = [WEITER_ZEILE, ZWEITE_ZEILE],
  handlers = {},
  optionen,
} = {}) {
  const d = denkerSteuerbar(rust);
  const s = lokalFolge(zeilen);
  const t = neu(
    {
      voice_call_chat: d.chat,
      voice_call_cancel: d.cancel,
      ...(rust?.handlers ?? {}),
      ...handlers,
    },
    s,
    optionen,
  );
  const erster = t.startSprache(FRAGE);
  assert.equal(
    await bis(
      () => d.laeufe.length === 1 && t.player.gespielt.includes(WEITER_SATZ),
    ),
    true,
  );
  t.aeussere(FOLGE);
  assert.equal(
    await bis(
      () => t.engine.auftrag && t.player.gespielt.includes(ZWEITE_SATZ),
    ),
    true,
  );
  return { d, s, t, a1: erster.turn };
}

/** Spracherkennung, die erst mit `frei()` antwortet: die Äußerung ist dann erkannt, wann der Test es will. */
function erkennungMitTor(text) {
  let frei;
  const tor = new Promise((r) => (frei = () => r(text)));
  tauri.handlers.voice_call_transcribe = () => tor;
  return frei;
}

/**
 * Der Schritt ist ein Kandidat für einen Auftrag (Sprecher: „WEITER“ ohne Text, der Denker wartet), und eine
 * Äußerung wird gerade erkannt (die Erkennung hängt bis `frei()`).
 */
async function mitOffenerErkennung(optionen) {
  const d = denkerSteuerbar();
  const s = lokalFolge(["WEITER:", ZWEITE_ZEILE]);
  const t = neu(
    { voice_call_chat: d.chat, voice_call_cancel: d.cancel },
    s,
    optionen,
  );
  const k = t.startSprache(FRAGE);
  assert.equal(
    await bis(() => d.laeufe.length === 1 && k.turn.sprecher?.weiter()),
    true,
  );
  const frei = erkennungMitTor(WETTER);
  t.redeLos();
  t.fruehEnde();
  t.ende();
  assert.equal(
    await bis(() => t.aufrufe("voice_call_transcribe").length === 1),
    true,
  );
  return { d, s, t, k: k.turn, frei };
}

/** Neustart des Anrufs ohne Audio: nur, was `start()` für den Schritt ändert (neue Sitzung, Player, Zustand). */
function neustart(t) {
  t.engine.stopped = false;
  t.engine.startGen++;
  t.engine.player = new SpeechPlayer(null);
  t.engine.state = "listening";
}

// G1 ----------------------------------------------------------------------------------------------

test("G1 späte Erkennung: getippter Text währenddessen bleibt unberührt (kein Abbruch, kein eigener Schritt)", async () => {
  const { s, t, frei } = await mitOffenerErkennung();
  void t.engine.sendText(FOLGE);
  const id = t.engine.turn.id;
  assert.equal(await bis(() => t.player.gespielt.includes(ZWEITE_SATZ)), true);
  frei();
  await settle(60);
  assert.deepEqual(
    t.log.user,
    [FRAGE, FOLGE],
    "keine Zeile für die späte Erkennung",
  );
  assert.equal(
    s.aufrufe.length,
    2,
    "der Sprecher wurde nicht noch einmal gefragt",
  );
  assert.equal(t.engine.turn?.id, id, "der getippte Schritt läuft weiter");
  assert.equal(t.engine.turn.cancelled, false);
  assert.equal(t.player.enqueued.includes(WETTER_SATZ), false);
  assert.equal(t.player.enqueued.filter((w) => w === ZWEITE_SATZ).length, 1);
});

const SPAETE_ERKENNUNG = {
  "Ende des Anrufs (stop)": async (t) => {
    await t.engine.stop();
  },
  "Ende und Neustart des Anrufs": async (t) => {
    await t.engine.stop();
    neustart(t);
  },
  "neues Gespräch (resetHistory)": async (t) => t.engine.resetHistory(),
};

for (const [name, aus] of Object.entries(SPAETE_ERKENNUNG)) {
  test(`G1 späte Erkennung: ${name}, der Schritt wartete noch auf den Denker: sie wirkt nicht mehr`, async () => {
    const { s, t, frei } = await mitOffenerErkennung();
    await aus(t);
    const zustand = t.engine.state;
    frei();
    await settle(60);
    assert.deepEqual(t.log.user, [FRAGE], "keine Zeile, kein neuer Schritt");
    assert.equal(
      s.aufrufe.length,
      1,
      "der Sprecher wurde nicht noch einmal gefragt",
    );
    assert.equal(t.engine.turn, null);
    assert.equal(t.engine.auftrag, null);
    assert.equal(
      t.engine.state,
      zustand,
      "der Zustand folgt der späten Erkennung nicht",
    );
    assert.equal(t.player.enqueued.includes(WETTER_SATZ), false);
  });

  test(`G1 späte Erkennung: ${name}, der Schritt lief schon von selbst zu Ende: sie wirkt nicht mehr`, async () => {
    const { d, s, t, frei } = await mitOffenerErkennung();
    d.liefere(0, ANTWORT);
    assert.equal(await bis(() => t.player.enqueued.includes(ERGEBNIS)), true);
    t.player.finish();
    assert.equal(await bis(() => t.engine.turn === null), true);
    await aus(t);
    const zustand = t.engine.state;
    frei();
    await settle(60);
    assert.deepEqual(t.log.user, [FRAGE]);
    assert.equal(s.aufrufe.length, 1);
    assert.equal(t.engine.turn, null);
    assert.equal(t.engine.state, zustand);
    assert.equal(t.player.enqueued.includes(WETTER_SATZ), false);
  });
}

test("G1 späte Erkennung: Esc (interrupt) währenddessen beendet den Schritt, die Äußerung davor wirkt nicht mehr", async () => {
  const { s, t, frei } = await mitOffenerErkennung();
  t.engine.interrupt();
  assert.equal(t.engine.turn, null);
  frei();
  await settle(60);
  assert.deepEqual(t.log.user, [FRAGE]);
  assert.equal(s.aufrufe.length, 1);
  assert.equal(t.engine.turn, null);
  assert.equal(t.engine.auftrag, null);
});

test("G1 späte Erkennung ohne Zwischenfall: der Schritt endet als Auftrag, die Äußerung geht an den Sprecher (Regression)", async () => {
  const { d, s, t, k, frei } = await mitOffenerErkennung();
  frei();
  assert.equal(await bis(() => t.engine.auftrag), true);
  assert.equal(await bis(() => t.player.gespielt.includes(ZWEITE_SATZ)), true);
  assert.equal(t.abgebrochen(k.id), 0, "der Denker läuft weiter");
  assert.equal(s.aufrufe.length, 2);
  assert.deepEqual(t.log.user, [FRAGE, WETTER]);
  d.liefere(0, ANTWORT);
  assert.equal(await bis(() => t.engine.auftrag?.fertig), true);
});

// G4 ----------------------------------------------------------------------------------------------

test("G4 getippte Frage im Anruf wird nie zum Auftrag: die gesprochene Äußerung ersetzt sie wie bisher", async () => {
  const d = denkerSteuerbar();
  const s = lokalFolge(["WEITER:", ZWEITE_ZEILE]);
  const t = neu({ voice_call_chat: d.chat, voice_call_cancel: d.cancel }, s);
  void t.engine.sendText(FRAGE);
  const id = t.engine.turn.id;
  assert.equal(
    await bis(() => d.laeufe.length === 1 && t.engine.turn?.sprecher?.weiter()),
    true,
  );
  t.aeussere(FOLGE, { mitBarge: false });
  assert.equal(await bis(() => t.player.gespielt.includes(ZWEITE_SATZ)), true);
  assert.ok(!t.engine.auftrag, "kein Auftrag");
  assert.equal(
    t.abgebrochen(id),
    1,
    "der getippte Schritt fällt, sein Denker wird abgebrochen",
  );
  d.liefere(0, ANTWORT); // der alte Denker meldet sich spät: ohne Ausgabe
  await settle(150);
  assert.equal(t.player.enqueued.includes(ERGEBNIS), false);
});

test("G4 getippte Frage, Ergebnis schon fertig, aber ungehört: die Äußerung ersetzt sie, die Antwortzeile bleibt die einzige", async () => {
  await mitTuning({ nachreichenStilleMs: 100 }, async () => {
    const d = denkerSteuerbar();
    const s = lokalFolge([WEITER_ZEILE, ZWEITE_ZEILE]);
    const t = neu({ voice_call_chat: d.chat, voice_call_cancel: d.cancel }, s);
    void t.engine.sendText(FRAGE);
    assert.equal(
      await bis(() => t.player.gespielt.includes(WEITER_SATZ)),
      true,
    );
    d.liefere(0, ANTWORT);
    // Getippt geht die Antwortzeile gleich hinaus (nicht erst nach dem ersten Satz).
    assert.equal(
      await bis(() =>
        t.log.zeilen.some((z) => z.final && z.t.includes(ANTWORT)),
      ),
      true,
    );
    t.aeussere(FOLGE);
    assert.equal(
      await bis(() => t.player.gespielt.includes(ZWEITE_SATZ)),
      true,
    );
    assert.ok(!t.engine.auftrag, "kein Auftrag");
    t.player.finish();
    await settle(400); // länger als die Stille: ein Nachreichen käme jetzt
    assert.equal(t.player.enqueued.filter((w) => w === ERGEBNIS).length, 1);
    assert.equal(
      t.log.zeilen.filter(
        (z) => z.final && !z.interrupted && z.t.includes(ANTWORT),
      ).length,
      1,
      "genau eine fertige Zeile mit dem Ergebnis",
    );
    assert.equal(
      t.log.zeilen.some((z) => z.auftragVon),
      false,
    );
  });
});

// G2 ----------------------------------------------------------------------------------------------

for (const [name, los] of [
  ["ohne Pegel-Barge-in", (t) => t.redeLos()],
  ["mit Pegel-Barge-in", (t) => t.barge()],
]) {
  test(`G2 Nachreichen eingereiht, aber ohne Ton (${name}): zurückgestellt statt verworfen, später erneut gesprochen`, async () => {
    await mitTuning({ nachreichenStilleMs: 100 }, async () => {
      const { d, t } = await aufbauSteuerbar({
        zeilen: [WEITER_ZEILE, ZWEITE_ZEILE, WETTER_ZEILE],
      });
      d.liefere(0, ANTWORT);
      assert.equal(await bis(() => t.engine.auftrag?.fertig), true);
      // Das Häppchen wird eingereiht (`audible` steht), der Ton beginnt aber nicht.
      t.player.deferStart = true;
      t.player.finish();
      assert.equal(await bis(() => t.player.enqueued.includes(ERGEBNIS)), true);
      assert.equal(
        t.player.gespielt.includes(ERGEBNIS),
        false,
        "noch kein Ton",
      );
      assert.equal(t.engine.turn.nachreichen !== undefined, true);
      assert.equal(
        t.engine.turn.audible,
        true,
        "genau das Fenster nach der Synthese, vor dem Ton",
      );
      los(t);
      assert.ok(
        t.engine.auftrag && !t.engine.auftrag.verworfen,
        "der Auftrag bleibt (zurückgestellt, nicht verworfen)",
      );
      assert.equal(t.engine.turn, null);
      t.player.deferStart = false;
      t.erkenne(WETTER);
      t.fruehEnde();
      t.ende();
      assert.equal(
        await bis(() => t.player.gespielt.includes(WETTER_SATZ)),
        true,
      );
      t.player.finish();
      assert.equal(await bis(() => t.player.gespielt.includes(ERGEBNIS)), true);
      assert.equal(t.player.enqueued.filter((w) => w === ERGEBNIS).length, 2);
      assert.equal(t.player.gespielt.filter((w) => w === ERGEBNIS).length, 1);
      t.player.finish();
      assert.equal(
        await bis(() => t.engine.auftrag === null),
        true,
        "danach erledigt",
      );
      assert.equal(t.log.errors.length, 0);
    });
  });
}

test("G2 zurückgestellter Auftrag verfällt trotzdem mit der Frist (kein Ergebnis nach Minuten)", async () => {
  await mitTuning(
    { auftragMaxMs: 1200, nachreichenStilleMs: 100 },
    async () => {
      const rust = gedaechtnis();
      const { d, t, a1 } = await aufbauSteuerbar({
        rust,
        zeilen: [WEITER_ZEILE, ZWEITE_ZEILE, WETTER_ZEILE],
      });
      d.liefere(0, ANTWORT);
      assert.equal(await bis(() => t.engine.auftrag?.fertig), true);
      t.player.deferStart = true;
      t.player.finish();
      assert.equal(await bis(() => t.player.enqueued.includes(ERGEBNIS)), true);
      t.redeLos();
      assert.ok(
        t.engine.auftrag && !t.engine.auftrag.verworfen,
        "zurückgestellt",
      );
      t.player.deferStart = false;
      t.erkenne(WETTER);
      t.fruehEnde();
      t.ende();
      assert.equal(
        await bis(() => t.player.gespielt.includes(WETTER_SATZ)),
        true,
      );
      // Der Vordergrund bleibt belegt (die Antwort läuft weiter): das Ergebnis kommt nicht mehr dran.
      assert.equal(
        await bis(() => t.engine.auftrag === null, 2500),
        true,
        "verfallen",
      );
      t.player.finish();
      await settle(300);
      assert.equal(t.player.enqueued.filter((w) => w === ERGEBNIS).length, 1);
      assert.equal(t.abgebrochen(a1.id), 0);
      assert.equal(
        rust.m.has("c1"),
        false,
        "das Gedächtnis des Tors ist vergessen",
      );
    },
  );
});

test("G2 Nachreichen mit begonnenem Ton: der Nutzer unterbricht, der Auftrag ist erledigt, die unterbrochene Zeile trägt die Kennung", async () => {
  await mitTuning({ nachreichenStilleMs: 100 }, async () => {
    const { d, t, a1 } = await aufbauSteuerbar({
      zeilen: [WEITER_ZEILE, ZWEITE_ZEILE, WETTER_ZEILE],
    });
    d.liefere(0, "Satz eins.", "Satz zwei.");
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
    assert.equal(t.engine.auftrag, null, "unterbrochen: erledigt");
    // Die unterbrochene Zeile trägt die Kennung des Auftrags (Quellen und Meldungen finden sie darüber).
    const z = t.log.zeilen.find((x) => x.interrupted && x.auftragVon);
    assert.ok(z);
    assert.equal(z.auftragVon, a1.id);
    assert.equal(z.t, "Satz eins.");
  });
});

// G3 ----------------------------------------------------------------------------------------------

const WIEDERHOLEN = "Bitte wiederhole das noch einmal.";
const NICHTS_GESAGT =
  "wav:Ich habe noch nichts gesagt, das ich wiederholen kann.";
const einstufungWiederholen = ({ text }) =>
  text === WIEDERHOLEN
    ? {
        stufe: "S0",
        absicht: "wiederholen",
        ansage: null,
        antwort: "Ich wiederhole.",
      }
    : null;

test("G3 Wiederholen nach dem Nachreichen: dazwischen antwortete der Sprecher kurz, gesagt wird genau die Antwort des Denkers", async () => {
  await mitTuning({ nachreichenStilleMs: 100 }, async () => {
    const rust = gedaechtnis("Eine ältere Ausgabe.");
    const { d, t } = await aufbauSteuerbar({
      rust,
      handlers: { assistent_einstufen: einstufungWiederholen },
    });
    // Ablauf: Auftrag, kurze Antwort des Sprechers (läuft), Ergebnis des Denkers, Nachreichen, ganz gehört.
    d.liefere(0, ANTWORT);
    assert.equal(await bis(() => t.engine.auftrag?.fertig), true);
    t.player.finish();
    assert.equal(await bis(() => t.player.gespielt.includes(ERGEBNIS)), true);
    t.player.finish();
    assert.equal(await bis(() => t.engine.auftrag === null), true);
    const davor = t.player.enqueued.length;
    t.aeussere(WIEDERHOLEN, { mitBarge: false });
    assert.equal(await bis(() => t.player.enqueued.length >= davor + 2), true);
    assert.deepEqual(t.player.enqueued.slice(davor), [
      "wav:Ich wiederhole.",
      ERGEBNIS,
    ]);
    assert.equal(
      t.aufrufe("voice_call_chat").length,
      1,
      "kein Modellaufruf zum Wiederholen",
    );
    assert.equal(t.player.enqueued.includes(NICHTS_GESAGT), false);
  });
});

test("G3 Wiederholen als Unterbrechung des Nachreichens: gesagt wird die ganze Antwort des Denkers", async () => {
  await mitTuning({ nachreichenStilleMs: 100 }, async () => {
    const rust = gedaechtnis();
    const { d, t } = await aufbauSteuerbar({
      rust,
      handlers: { assistent_einstufen: einstufungWiederholen },
    });
    d.liefere(0, "Satz eins.", "Satz zwei.");
    t.player.finish();
    assert.equal(
      await bis(() => t.player.gespielt.includes("wav:Satz eins.")),
      true,
    );
    const davor = t.player.enqueued.length;
    t.aeussere(WIEDERHOLEN);
    assert.equal(await bis(() => t.player.enqueued.length >= davor + 2), true);
    // Die Antwort wurde unterbrochen: „wiederholen“ sagt sie ganz (wie bei jeder unterbrochenen Antwort).
    assert.deepEqual(t.player.enqueued.slice(davor), [
      "wav:Ich wiederhole.",
      "wav:Satz eins. Satz zwei.",
    ]);
    assert.equal(t.engine.auftrag, null);
  });
});

test("G3 „Wiederholen“ als die Äußerung, die den Schritt zum Auftrag macht, sagt das ungesprochene Ergebnis nicht vorweg", async () => {
  await mitTuning({ nachreichenStilleMs: 100 }, async () => {
    const rust = gedaechtnis();
    const d = denkerSteuerbar(rust);
    const s = lokalFolge([WEITER_ZEILE]);
    const t = neu(
      {
        voice_call_chat: d.chat,
        voice_call_cancel: d.cancel,
        ...rust.handlers,
        assistent_einstufen: einstufungWiederholen,
      },
      s,
    );
    t.startSprache(FRAGE);
    assert.equal(
      await bis(
        () => d.laeufe.length === 1 && t.player.gespielt.includes(WEITER_SATZ),
      ),
      true,
    );
    // Das Ergebnis ist fertig und gemerkt, steht aber hinter dem Weitersprechen (noch nichts davon zu hören).
    d.liefere(0, ANTWORT);
    assert.equal(await bis(() => t.player.enqueued.includes(ERGEBNIS)), true);
    assert.equal(rust.m.get("c1"), ANTWORT);
    t.aeussere(WIEDERHOLEN);
    assert.equal(
      await bis(() => t.player.enqueued.includes(NICHTS_GESAGT)),
      true,
    );
    assert.ok(t.engine.auftrag, "der Schritt ist zum Auftrag geworden");
    assert.equal(t.player.enqueued.filter((w) => w === ERGEBNIS).length, 1);
    t.player.finish();
    assert.equal(await bis(() => t.player.gespielt.includes(ERGEBNIS)), true);
    assert.equal(t.player.enqueued.filter((w) => w === ERGEBNIS).length, 2);
  });
});

test("G3 Schlüssel des Tors: die Runden des offenen Auftrags behalten den einmal gewählten, auch wenn die Gesprächskennung inzwischen da ist", async () => {
  await mitTuning({ nachreichenStilleMs: 100 }, async () => {
    let kennung = null; // das Gespräch ist neu: die Kennung entsteht erst mit dem ersten Beitrag
    const rust = gedaechtnis();
    const { d, t } = await aufbauSteuerbar({
      rust,
      optionen: { conversationId: () => kennung },
      handlers: { assistent_einstufen: einstufungWiederholen },
    });
    assert.equal(d.laeufe[0].conversationId, null);
    kennung = "c1";
    assert.equal(
      await t.engine.session.deps.ausgabeKey(),
      null,
      "der Auftrag legt weiter dort ab, wo „wiederholen“ später sucht",
    );
    d.liefere(0, ANTWORT);
    assert.equal(await bis(() => t.engine.auftrag?.fertig), true);
    t.player.finish();
    assert.equal(await bis(() => t.player.gespielt.includes(ERGEBNIS)), true);
    t.player.finish();
    assert.equal(await bis(() => t.engine.auftrag === null), true);
    assert.equal(
      await t.engine.session.deps.ausgabeKey(),
      null,
      "nach dem Nachreichen gilt der Schlüssel der Engine, derselbe",
    );
    const davor = t.player.enqueued.length;
    t.aeussere(WIEDERHOLEN, { mitBarge: false });
    assert.equal(await bis(() => t.player.enqueued.length >= davor + 2), true);
    assert.deepEqual(t.player.enqueued.slice(davor), [
      "wav:Ich wiederhole.",
      ERGEBNIS,
    ]);
  });
});

test("G3 nach einer weiteren Sprecherantwort ist die Ausgabe des Denkers nicht mehr zu wiederholen (wie bisher)", async () => {
  await mitTuning({ nachreichenStilleMs: 100 }, async () => {
    const rust = gedaechtnis();
    const { d, t } = await aufbauSteuerbar({
      rust,
      zeilen: [WEITER_ZEILE, ZWEITE_ZEILE, WETTER_ZEILE],
      handlers: { assistent_einstufen: einstufungWiederholen },
    });
    d.liefere(0, ANTWORT);
    t.player.finish();
    assert.equal(await bis(() => t.player.gespielt.includes(ERGEBNIS)), true);
    t.player.finish();
    assert.equal(await bis(() => t.engine.auftrag === null), true);
    t.aeussere(WETTER, { mitBarge: false });
    assert.equal(
      await bis(() => t.player.gespielt.includes(WETTER_SATZ)),
      true,
    );
    t.player.finish();
    await settle(30);
    t.aeussere(WIEDERHOLEN, { mitBarge: false });
    assert.equal(
      await bis(() => t.player.enqueued.includes(NICHTS_GESAGT)),
      true,
    );
  });
});

test("G3 Wiederholen, solange das Ergebnis noch nicht gesprochen ist, sagt es nicht vorweg; es kommt danach genau einmal", async () => {
  await mitTuning({ nachreichenStilleMs: 300 }, async () => {
    const rust = gedaechtnis();
    const { d, t } = await aufbauSteuerbar({
      rust,
      handlers: { assistent_einstufen: einstufungWiederholen },
    });
    d.liefere(0, ANTWORT);
    assert.equal(await bis(() => t.engine.auftrag?.fertig), true);
    assert.equal(
      rust.m.get("c1"),
      ANTWORT,
      "das Tor hat es gemerkt, gesprochen ist es noch nicht",
    );
    t.player.finish();
    t.aeussere(WIEDERHOLEN, { mitBarge: false });
    assert.equal(
      await bis(() => t.player.enqueued.includes(NICHTS_GESAGT)),
      true,
    );
    assert.equal(t.player.enqueued.includes(ERGEBNIS), false);
    t.player.finish();
    assert.equal(await bis(() => t.player.gespielt.includes(ERGEBNIS)), true);
    assert.equal(t.player.enqueued.filter((w) => w === ERGEBNIS).length, 1);
    t.player.finish();
    assert.equal(await bis(() => t.engine.auftrag === null), true);
  });
});

test("G3 verworfener Auftrag: das Gedächtnis des Tors wird vergessen, „wiederholen“ sagt das Ergebnis nie", async () => {
  await mitTuning({ nachreichenStilleMs: 100 }, async () => {
    const rust = gedaechtnis("Eine ältere Ausgabe.");
    const { d, t } = await aufbauSteuerbar({
      rust,
      handlers: { assistent_einstufen: einstufungWiederholen },
    });
    d.liefere(0, ANTWORT);
    assert.equal(await bis(() => t.engine.auftrag?.fertig), true);
    t.engine.resetHistory();
    assert.equal(t.engine.auftrag, null);
    assert.equal(
      await bis(() => !rust.m.has("c1")),
      true,
      "weder die ältere noch die ungesprochene Ausgabe bleibt stehen",
    );
    assert.deepEqual(t.aufrufe("assistent_ausgabe_vergessen").at(-1).args, {
      conversationId: "c1",
    });
  });
});

// (a) Frist --------------------------------------------------------------------------------------

test("Frist: auch ein fertiges, nie nachgereichtes Ergebnis verfällt (60 s ab Beginn, hier verkürzt), ohne Abbruch und ohne Ausgabe", async () => {
  await mitTuning({ auftragMaxMs: 900, nachreichenStilleMs: 100 }, async () => {
    const rust = gedaechtnis();
    const { d, t, a1 } = await aufbauSteuerbar({ rust });
    d.liefere(0, ANTWORT);
    assert.equal(await bis(() => t.engine.auftrag?.fertig), true);
    // Der Vordergrund bleibt belegt (die Antwort des Sprechers wird nicht beendet): das Ergebnis wartet.
    assert.equal(
      await bis(() => t.engine.auftrag === null, 2500),
      true,
      "verfallen",
    );
    t.player.finish();
    await settle(300); // länger als die Stille: ein Nachreichen käme jetzt
    assert.equal(t.player.enqueued.includes(ERGEBNIS), false);
    assert.equal(
      t.abgebrochen(a1.id),
      0,
      "der Denker war fertig: kein Abbruch",
    );
    assert.equal(t.log.errors.length, 0);
    assert.equal(
      rust.m.has("c1"),
      false,
      "das Gedächtnis des Tors ist vergessen",
    );
    assert.equal(
      t.log.zeilen.some((z) => z.auftragVon),
      false,
    );
  });
});

test("Frist: ein Ergebnis, das rechtzeitig zu sprechen beginnt, wird nicht abgeschnitten", async () => {
  await mitTuning({ auftragMaxMs: 900, nachreichenStilleMs: 100 }, async () => {
    const { d, t } = await aufbauSteuerbar();
    d.liefere(0, ANTWORT);
    t.player.finish();
    assert.equal(await bis(() => t.player.gespielt.includes(ERGEBNIS)), true);
    await settle(1100); // länger als die Frist: das gesprochene Ergebnis läuft weiter
    assert.ok(t.engine.turn?.nachreichen);
    assert.equal(t.engine.turn.cancelled, false);
    t.player.finish();
    assert.equal(await bis(() => t.engine.auftrag === null), true);
    assert.deepEqual(t.log.assistant.at(-1), {
      t: ANTWORT,
      final: true,
      interrupted: false,
    });
  });
});

// (b) Kennung der Zeile ----------------------------------------------------------------------------

test("Zeilen des Nachreichens tragen die Kennung des Auftrags, alle anderen nicht", async () => {
  await mitTuning({ nachreichenStilleMs: 100 }, async () => {
    const { d, t, a1 } = await aufbauSteuerbar();
    d.liefere(0, "Satz eins.", "Satz zwei.");
    t.player.finish();
    assert.equal(
      await bis(() => t.player.enqueued.includes("wav:Satz zwei.")),
      true,
    );
    t.player.finish();
    assert.equal(await bis(() => t.engine.auftrag === null), true);
    const mit = t.log.zeilen.filter((z) => z.auftragVon);
    assert.ok(mit.length >= 2, "Zwischenstand und Endfassung");
    assert.ok(mit.every((z) => z.auftragVon === a1.id));
    assert.deepEqual(
      { t: mit.at(-1).t, final: mit.at(-1).final },
      { t: "Satz eins. Satz zwei.", final: true },
    );
    const ohne = t.log.zeilen.filter((z) => !z.auftragVon);
    assert.ok(
      ohne.length >= 2 && ohne.every((z) => !z.t.includes("Satz eins")),
    );
  });
});

// -- Runde 4 (Codex R2 zu lva#547) ----------------------------------------------------------------------
//
// Fehlerfälle dieser Runde (vor dem Code aufgeschrieben, Status im Bericht):
//  H1 Nebenläufigkeit: eine späte Erkennung wirkt auch dann nicht, wenn inzwischen ein neuerer Schritt gestartet
//     UND beendet wurde (der Kandidat selbst wurde nie abgebrochen) oder Esc nach dem natürlichen Ende kam.
//  H2 Nebenläufigkeit/Gedächtnis: die Kennungsabfrage des Denkers dauert, der Auftrag entsteht in diesem Fenster;
//     der Schlüssel gehört dem Lauf, nicht dem Feld der Engine (sonst löscht ihn die kurze Antwort des Sprechers).
//  H3 Abbruch mitten im Vorgang: Nachreichen eingereiht, Ton noch offen, der Nutzer spricht: nichts davon ist
//     endgültig gemeldet oder gespeichert, der Verlauf nimmt es zurück; Quellen nur am gültigen Versuch.
//     Dazu: ein Fehler des Players nimmt die angezeigte, ungehörte Zeile zurück; Gehörtes bleibt unterbrochen stehen.
//  H4 Hängende Geräte: Synthese oder Playerstart antworten nie: die Frist des Auftrags läuft bis zum echten
//     Beginn des ersten Stücks, danach verfällt der Auftrag still; ein rechtzeitig begonnenes Stück spricht weiter.
//  H5 Gespräch gewechselt, während Belege geprüft werden: keine Meldung mehr.
//  Nicht neu: voller Speicher (Satzpuffer gedeckelt, Runde 1), fehlendes Gerät und Absturz des Denkers (Runde 1).

// H1 ----------------------------------------------------------------------------------------------

/** Der Kandidat endet von selbst, während die Erkennung der Folgeäußerung noch hängt. */
async function kandidatEndetVonSelbst() {
  const r = await mitOffenerErkennung();
  r.d.liefere(0, ANTWORT);
  assert.equal(await bis(() => r.t.player.enqueued.includes(ERGEBNIS)), true);
  r.t.player.finish();
  assert.equal(await bis(() => r.t.engine.turn === null), true);
  return r;
}

/** Nichts darf von der späten Erkennung ausgehen: weder Zeile noch Sprecher, Synthese oder Denker. */
function keineFolgen(t, s, vorher) {
  assert.deepEqual(t.log.user, vorher.user, "keine Nutzerzeile");
  assert.equal(
    s.aufrufe.length,
    vorher.sprecher,
    "keine Anfrage an den Sprecher",
  );
  assert.equal(
    t.aufrufe("voice_call_chat").length,
    vorher.chats,
    "keine Anfrage an den Denker",
  );
  assert.equal(
    t.aufrufe("voice_call_synthesize").length,
    vorher.synthesen,
    "keine Synthese",
  );
  assert.equal(t.player.enqueued.includes(WETTER_SATZ), false);
  assert.equal(t.engine.turn, null);
}

const stand = (t, s) => ({
  user: [...t.log.user],
  sprecher: s.aufrufe.length,
  chats: t.aufrufe("voice_call_chat").length,
  synthesen: t.aufrufe("voice_call_synthesize").length,
});

test("H1 späte Erkennung: ein neuerer Schritt, der gestartet und ganz beendet wurde (getippt), hebt die Überholung nicht auf", async () => {
  const { s, t, frei } = await kandidatEndetVonSelbst();
  const getippt = t.engine.sendText(FOLGE);
  assert.equal(await bis(() => t.player.gespielt.includes(ZWEITE_SATZ)), true);
  t.player.finish();
  await getippt;
  assert.equal(await bis(() => t.engine.turn === null), true);
  const vorher = stand(t, s);
  assert.deepEqual(vorher.user, [FRAGE, FOLGE]);
  frei();
  await settle(80);
  keineFolgen(t, s, vorher);
  assert.equal(t.engine.state, "listening");
});

test("H1 späte Erkennung: Esc nach dem natürlichen Ende des Schritts beendet auch die Erkennung", async () => {
  const { s, t, frei } = await kandidatEndetVonSelbst();
  t.engine.interrupt();
  const vorher = stand(t, s);
  frei();
  await settle(80);
  keineFolgen(t, s, vorher);
  assert.deepEqual(t.log.user, [FRAGE]);
  assert.equal(t.engine.state, "listening");
});

test("H1 späte Erkennung ohne Zwischenfall nach dem natürlichen Ende: sie gilt als gewöhnlicher Beitrag (Regression)", async () => {
  const { s, t, frei } = await kandidatEndetVonSelbst();
  frei();
  assert.equal(await bis(() => t.player.gespielt.includes(ZWEITE_SATZ)), true);
  assert.deepEqual(t.log.user, [FRAGE, WETTER]);
  assert.equal(s.aufrufe.length, 2);
});

// H2 ----------------------------------------------------------------------------------------------

/** Lokales Modell wie `lokalFolge`, dessen Aufruf Nummer `i` erst mit `frei()` antwortet (kein Zeitgeber). */
function lokalFolgeMitTor(zeilen, i) {
  const m = lokalFolge(zeilen, 0);
  const antwort = m.turn;
  let frei;
  const tor = new Promise((r) => (frei = r));
  m.turn = async (req) => {
    if (m.aufrufe.length === i) await tor;
    return antwort(req);
  };
  return { m, frei };
}

test("H2 Schlüssel des Tors: kommt er erst nach der Bildung des Auftrags an, gehört er dem Auftrag, und „wiederholen“ sagt das Ergebnis", async () => {
  await mitTuning({ nachreichenStilleMs: 100 }, async () => {
    const rust = gedaechtnis();
    const d = denkerSteuerbar(rust);
    const { m: s, frei: sprecherFrei } = lokalFolgeMitTor(
      [WEITER_ZEILE, ZWEITE_ZEILE],
      1,
    );
    // Die zweite Abfrage (die des Denkers, nach der Einstufung) hängt; alle anderen antworten gleich.
    let kennungFrei;
    const kennung = new Promise((r) => (kennungFrei = () => r("c1")));
    let abfragen = 0;
    const t = neu(
      {
        voice_call_chat: d.chat,
        voice_call_cancel: d.cancel,
        ...rust.handlers,
        assistent_einstufen: einstufungWiederholen,
      },
      s,
      { conversationId: () => (++abfragen === 2 ? kennung : "c1") },
    );
    const erster = t.startSprache(FRAGE);
    assert.equal(
      await bis(
        () =>
          erster.turn.denkerStart !== undefined &&
          t.player.gespielt.includes(WEITER_SATZ),
      ),
      true,
    );
    assert.equal(
      d.laeufe.length,
      0,
      "die Kennungsabfrage hängt: noch kein Denker",
    );
    // Der Auftrag entsteht im Fenster, in dem die Kennung noch fehlt.
    t.aeussere(FOLGE);
    assert.equal(await bis(() => t.engine.auftrag), true);
    kennungFrei();
    assert.equal(await bis(() => d.laeufe.length === 1), true);
    assert.equal(d.laeufe[0].conversationId, "c1");
    // Erst jetzt antwortet der Sprecher kurz allein (er löscht das Gedächtnis, das dem Gespräch gehört).
    sprecherFrei();
    assert.equal(
      await bis(() => t.player.gespielt.includes(ZWEITE_SATZ)),
      true,
    );
    d.liefere(0, ANTWORT);
    assert.equal(await bis(() => t.engine.auftrag?.fertig), true);
    t.player.finish();
    assert.equal(await bis(() => t.player.gespielt.includes(ERGEBNIS)), true);
    t.player.finish();
    assert.equal(await bis(() => t.engine.auftrag === null), true);
    const davor = t.player.enqueued.length;
    t.aeussere(WIEDERHOLEN, { mitBarge: false });
    assert.equal(await bis(() => t.player.enqueued.length >= davor + 2), true);
    assert.deepEqual(t.player.enqueued.slice(davor), [
      "wav:Ich wiederhole.",
      ERGEBNIS,
    ]);
    assert.equal(t.player.enqueued.includes(NICHTS_GESAGT), false);
  });
});

// H3 ----------------------------------------------------------------------------------------------

/** Der echte Gesprächsspeicher mit seiner Engine und einem Spion für das, was ans Backend zum Speichern ginge. */
let speicherLauf = 0;
async function mitEchtemSpeicher() {
  const { useVoiceCallStore: store } =
    await import("../src/stores/voiceCallStore.ts");
  const gespeichert = [];
  const handlers = {
    conversation_get: ({ id }) => ({
      conversation: { id, title: "Neues Gespräch" },
      entries: [],
    }),
    conversations_list: () => [],
    conversation_create: () => ({ id: "k-neu" }),
    conversation_append_entry: ({ entry }) => {
      gespeichert.push(entry);
      return { id: `e${gespeichert.length}` };
    },
    conversation_update_entry: () => null,
    conversation_update: () => null,
  };
  tauri.handlers = handlers;
  tauri.calls = [];
  await store.getState().openConversation(`k${++speicherLauf}`);
  const engine = globalThis.window.__voiceCallEngine;
  // Frischer Hörer: `neu` setzt die Liste der Hörer des Tests zurück.
  await engine.dispose();
  return { store, engine, gespeichert, handlers };
}

const ZWEI_SAETZE = [BELEG_ZEILE, "Das Protokoll liegt bis Freitag vor."];
const ERGEBNIS_ZEILE = ZWEI_SAETZE.join(" ");

test("H3 echter Speicher: Nachreichen eingereiht, Ton noch offen, der Nutzer spricht: eine endgültige Antwort, ein Speicherbeitrag, Quellen am gültigen Versuch", async () => {
  await mitTuning({ nachreichenStilleMs: 100 }, async () => {
    const { store, engine, gespeichert, handlers } = await mitEchtemSpeicher();
    const { d, t, a1 } = await aufbauSteuerbar({
      zeilen: [WEITER_ZEILE, ZWEITE_ZEILE, WETTER_ZEILE],
      handlers,
      optionen: { kontext: true, engine },
    });
    d.liefere(0, ...ZWEI_SAETZE);
    assert.equal(await bis(() => t.engine.auftrag?.fertig), true);
    // Erster Versuch: eingereiht, aber der Ton beginnt nicht (Player hängt).
    t.player.finish();
    t.player.deferStart = true;
    assert.equal(await bis(() => t.player.enqueued.includes(BELEG_SATZ)), true);
    assert.equal(await bis(() => t.engine.turn?.done === true), true);
    await settle(60);
    const ergebnis = (l) => l.role === "assistant" && l.auftragVon === a1.id;
    assert.deepEqual(
      t.log.zeilen.filter((z) => z.auftragVon === a1.id && z.final),
      [],
      "vor dem Ton nichts endgültig gemeldet",
    );
    assert.equal(
      gespeichert.filter((e) => e.text.includes("Anna übernimmt")).length,
      0,
      "vor dem Ton nichts gespeichert",
    );
    // Der Nutzer spricht: das Nachreichen wird zurückgestellt, der Sprecher antwortet.
    t.player.deferStart = false;
    t.aeussere(WETTER, { mitBarge: false });
    assert.equal(
      await bis(() => t.player.gespielt.includes(WETTER_SATZ)),
      true,
    );
    t.player.finish();
    assert.equal(
      await bis(() => t.player.gespielt.includes(BELEG_SATZ)),
      true,
      "zweiter Versuch: das Ergebnis wird erneut gesprochen",
    );
    t.player.finish();
    assert.equal(await bis(() => t.engine.auftrag === null), true);
    assert.equal(
      await bis(
        () =>
          quellen(t, a1.id).length > 0 &&
          gespeichert.some((e) => e.text.includes("Anna")),
      ),
      true,
    );
    await settle(80);
    // Genau eine endgültige Antwort, ein Speicherbeitrag, die Quellen nur an dieser Zeile.
    const finale = store.getState().lines.filter(ergebnis);
    assert.equal(finale.length, 1);
    assert.equal(finale[0].final, true);
    assert.equal(finale[0].text, ERGEBNIS_ZEILE);
    assert.equal(finale[0].sources?.length, 1);
    assert.equal(
      store.getState().lines.filter((l) => l.auftragVon).length,
      1,
      "keine zweite Zeile mit der Kennung des Auftrags (kein überholter Versuch)",
    );
    assert.equal(
      gespeichert.filter((e) => e.text.includes("Anna übernimmt")).length,
      1,
    );
    assert.deepEqual(
      t.log.zeilen.filter((z) => z.auftragVon === a1.id && z.final).length,
      1,
    );
    assert.deepEqual(quellen(t, a1.id), [
      { id: a1.id, n: 1, nachgereicht: true },
    ]);
    // Der Verlauf der KI kennt das Ergebnis einmal.
    assert.equal(t.verlauf().filter((m) => m === `user:${FRAGE}`).length, 1);
  });
});

test("H3 Nachreichen ohne Ton wird zurückgestellt: sein Verlaufseintrag geht zurück, der zweite Versuch legt ihn neu an", async () => {
  await mitTuning({ nachreichenStilleMs: 100 }, async () => {
    const { d, t } = await aufbauSteuerbar({
      zeilen: [WEITER_ZEILE, ZWEITE_ZEILE, WETTER_ZEILE],
    });
    d.liefere(0, ANTWORT);
    assert.equal(await bis(() => t.engine.auftrag?.fertig), true);
    t.player.finish();
    t.player.deferStart = true;
    assert.equal(await bis(() => t.engine.turn?.historyPushed === true), true);
    assert.ok(t.verlauf().includes(`assistant:${ANTWORT}`));
    t.player.deferStart = false;
    t.redeLos();
    assert.ok(t.engine.auftrag && !t.engine.auftrag.nachreichend);
    assert.equal(
      t.verlauf().includes(`assistant:${ANTWORT}`),
      false,
      "zurückgestellt: der Verlauf kennt das Ergebnis noch nicht",
    );
  });
});

test("H3 Playerfehler beim Nachreichen: die angezeigte, ungehörte Zeile wird zurückgenommen", async () => {
  await mitTuning({ nachreichenStilleMs: 100 }, async () => {
    const { t } = await aufbau({ denker: denker(150, ANTWORT) });
    const verworfen = t.log.verworfen;
    t.player.finish();
    t.player.enqueue = async () => {
      throw new Error("Audio weg");
    };
    assert.equal(await bis(() => t.engine.auftrag === null, 2000), true);
    await settle(100);
    assert.equal(t.log.errors.length, 1);
    assert.equal(
      t.log.verworfen,
      verworfen + 1,
      "die Zeile mit dem Zwischenstand verschwindet",
    );
    assert.equal(
      t.log.zeilen.some((z) => z.auftragVon && z.final),
      false,
    );
    assert.equal(t.engine.turn, null);
    assert.equal(t.engine.state, "listening");
  });
});

test("H3 Playerfehler beim zweiten Stück: Gehörtes bleibt als unterbrochene Zeile des Auftrags stehen", async () => {
  await mitTuning({ nachreichenStilleMs: 100 }, async () => {
    const { d, t, a1 } = await aufbauSteuerbar();
    const verworfen = t.log.verworfen;
    d.liefere(0, "Satz eins.", "Satz zwei.");
    assert.equal(await bis(() => t.engine.auftrag?.fertig), true);
    t.player.finish();
    const einreihen = t.player.enqueue;
    let n = 0;
    t.player.enqueue = async (...a) => {
      if (++n === 2) throw new Error("Audio weg");
      return einreihen(...a);
    };
    assert.equal(await bis(() => t.engine.auftrag === null, 2000), true);
    await settle(100);
    assert.equal(t.log.errors.length, 1);
    assert.deepEqual(t.log.zeilen.at(-1), {
      t: "Satz eins.",
      final: true,
      interrupted: true,
      auftragVon: a1.id,
    });
    assert.equal(t.log.verworfen, verworfen, "nichts wird zurückgenommen");
    assert.equal(t.engine.turn, null);
  });
});

// H4 ----------------------------------------------------------------------------------------------

/** Gesteuerte Uhr: `setTimeout` der Engine steht still, bis der Test sie mit `tick` weiterstellt. */
async function mitUhr(fn) {
  mock.timers.enable({ apis: ["setTimeout"] });
  try {
    return await fn((ms) => mock.timers.tick(ms));
  } finally {
    mock.timers.reset();
  }
}

/**
 * Auftrag mit fertigem Ergebnis, noch hinter der kurzen Antwort des Sprechers: mit `t.player.finish()` endet sie,
 * und (ohne Stille) beginnt das Nachreichen sofort.
 */
async function ergebnisBereit(saetze = [ANTWORT], optionen = {}) {
  const r = await aufbauSteuerbar(optionen);
  r.d.liefere(0, ...saetze);
  assert.equal(await bis(() => r.t.engine.auftrag?.fertig), true);
  return r;
}

const ohneStille = (fn) => mitTuning({ nachreichenStilleMs: 0 }, fn);

test("H4 hängende Synthese: die Frist läuft weiter, der Auftrag verfällt still, spätes Freigeben gibt nichts aus", () =>
  mitUhr((tick) =>
    ohneStille(async () => {
      let freigeben;
      const haengt = new Promise((r) => (freigeben = r));
      const { t } = await ergebnisBereit([ANTWORT], {
        handlers: {
          voice_call_synthesize: ({ text }) =>
            text === ANTWORT ? haengt : `wav:${text}`,
        },
      });
      t.player.finish();
      assert.equal(
        await bis(() => t.engine.turn?.nachreichen !== undefined),
        true,
        "das Nachreichen läuft",
      );
      assert.equal(t.engine.state, "thinking");
      tick(TUNING.auftragMaxMs);
      assert.equal(t.engine.auftrag, null, "der Auftrag ist verfallen");
      assert.equal(t.engine.turn, null, "der Schritt ist frei");
      assert.equal(t.engine.state, "listening");
      freigeben(`wav:${ANTWORT}`);
      await settle(50);
      assert.equal(t.player.enqueued.includes(ERGEBNIS), false);
      assert.equal(
        t.log.zeilen.some((z) => z.auftragVon),
        false,
        "keine Zeile, auch keine unfertige",
      );
      assert.equal(t.log.errors.length, 0);
      assert.deepEqual(t.verlauf(), [
        `user:${FOLGE}`,
        "assistant:Es ist kurz nach drei.",
      ]);
    }),
  ));

test("H4 hängender Playerstart: der Auftrag verfällt still, die eingereihte Zeile verschwindet, spätes Starten gibt nichts aus", () =>
  mitUhr((tick) =>
    ohneStille(async () => {
      const { t } = await ergebnisBereit();
      t.player.deferStart = true;
      t.player.finish();
      assert.equal(await bis(() => t.player.enqueued.includes(ERGEBNIS)), true);
      assert.equal(
        t.player.gespielt.includes(ERGEBNIS),
        false,
        "noch kein Ton",
      );
      const gestoppt = t.player.stopped;
      const verworfen = t.log.verworfen;
      tick(TUNING.auftragMaxMs);
      assert.equal(t.engine.auftrag, null);
      assert.equal(t.engine.turn, null);
      assert.equal(t.engine.state, "listening");
      assert.ok(t.player.stopped > gestoppt, "die Warteschlange wurde geleert");
      assert.equal(
        t.log.verworfen,
        verworfen + 1,
        "die angezeigte Zeile ist weg",
      );
      t.player.audioStart();
      t.player.finish();
      await settle(30);
      assert.equal(t.player.gespielt.includes(ERGEBNIS), false);
      assert.equal(
        t.log.zeilen.some((z) => z.auftragVon && z.final),
        false,
      );
      assert.deepEqual(t.verlauf(), [
        `user:${FOLGE}`,
        "assistant:Es ist kurz nach drei.",
      ]);
    }),
  ));

test("H4 ein rechtzeitig begonnenes erstes Stück spricht weiter, auch wenn die Frist danach abläuft", () =>
  mitUhr((tick) =>
    ohneStille(async () => {
      let freigeben;
      const haengt = new Promise((r) => (freigeben = r));
      const { t, a1 } = await ergebnisBereit(["Satz eins.", "Satz zwei."], {
        handlers: {
          voice_call_synthesize: ({ text }) =>
            text === "Satz zwei." ? haengt : `wav:${text}`,
        },
      });
      t.player.finish();
      assert.equal(
        await bis(() => t.player.gespielt.includes("wav:Satz eins.")),
        true,
        "das erste Stück klingt",
      );
      tick(TUNING.auftragMaxMs);
      assert.ok(t.engine.auftrag && !t.engine.auftrag.verworfen);
      assert.equal(t.engine.turn?.nachreichen?.turnId, a1.id);
      assert.equal(t.engine.turn.cancelled, false);
      freigeben("wav:Satz zwei.");
      assert.equal(
        await bis(() => t.player.enqueued.includes("wav:Satz zwei.")),
        true,
      );
      t.player.finish();
      assert.equal(await bis(() => t.engine.auftrag === null), true);
      assert.deepEqual(t.log.zeilen.at(-1), {
        t: "Satz eins. Satz zwei.",
        final: true,
        interrupted: false,
        auftragVon: a1.id,
      });
    }),
  ));

test("H4 ein Stück, das schon klingt, aber noch nicht gemeldet ist (der echte Player meldet nur auf Anfrage), wird von der Frist nicht abgeschnitten", () =>
  mitUhr((tick) =>
    ohneStille(async () => {
      let freigeben;
      const haengt = new Promise((r) => (freigeben = r));
      const { t, a1 } = await ergebnisBereit(["Satz eins.", "Satz zwei."], {
        handlers: {
          voice_call_synthesize: ({ text }) =>
            text === "Satz zwei." ? haengt : `wav:${text}`,
        },
      });
      t.player.lazy = true;
      t.player.finish();
      assert.equal(
        await bis(() => t.player.enqueued.includes("wav:Satz eins.")),
        true,
      );
      assert.equal(
        t.player.gespielt.includes("wav:Satz eins."),
        false,
        "der Beginn ist noch nicht gemeldet",
      );
      const gestoppt = t.player.stopped;
      tick(TUNING.auftragMaxMs);
      assert.ok(t.engine.auftrag && !t.engine.auftrag.verworfen);
      assert.equal(t.player.stopped, gestoppt, "nichts wurde abgebrochen");
      assert.equal(t.engine.turn?.cancelled, false);
      assert.equal(
        t.player.gespielt.includes("wav:Satz eins."),
        true,
        "die Frist hat den Beginn erfragt",
      );
      freigeben("wav:Satz zwei.");
      assert.equal(
        await bis(() => t.player.enqueued.includes("wav:Satz zwei.")),
        true,
      );
      t.player.finish();
      assert.equal(await bis(() => t.engine.auftrag === null), true);
      assert.deepEqual(t.log.zeilen.at(-1), {
        t: "Satz eins. Satz zwei.",
        final: true,
        interrupted: false,
        auftragVon: a1.id,
      });
    }),
  ));

test("H3 der Beginn wird nur auf Anfrage gemeldet (wie beim echten Player): die endgültige Antwort geht genau einmal hinaus", () =>
  ohneStille(async () => {
    const { t, a1 } = await ergebnisBereit();
    t.player.lazy = true;
    t.player.finish();
    assert.equal(await bis(() => t.player.enqueued.includes(ERGEBNIS)), true);
    assert.equal(await bis(() => t.engine.turn?.done === true), true);
    t.player.finish();
    assert.equal(await bis(() => t.engine.auftrag === null), true);
    assert.equal(
      t.log.zeilen.filter((z) => z.final && z.auftragVon === a1.id).length,
      1,
    );
    assert.deepEqual(
      t.verlauf().filter((m) => m === `assistant:${ANTWORT}`).length,
      1,
    );
  }));

test("H4 Frist, Stille und Nachreichen ohne Zwischenfall: das Ergebnis wird wie bisher gesprochen (Regression)", () =>
  mitUhr((tick) =>
    ohneStille(async () => {
      const { t, a1 } = await ergebnisBereit();
      const verworfen = t.log.verworfen;
      t.player.finish();
      assert.equal(await bis(() => t.player.gespielt.includes(ERGEBNIS)), true);
      t.player.finish();
      assert.equal(await bis(() => t.engine.auftrag === null), true);
      assert.deepEqual(t.log.zeilen.at(-1), {
        t: ANTWORT,
        final: true,
        interrupted: false,
        auftragVon: a1.id,
      });
      // Die Frist ist mit dem Beginn des Tons beendet: ein späterer Zeitablauf ändert nichts mehr.
      tick(TUNING.auftragMaxMs * 2);
      assert.equal(t.log.errors.length, 0);
      assert.equal(t.log.verworfen, verworfen);
    }),
  ));

// H5 ----------------------------------------------------------------------------------------------

test("H5 Quellen: ein neues Gespräch, während die Belege geprüft werden, verwirft die Meldung", async () => {
  let freigeben;
  const tor = new Promise((r) => (freigeben = r));
  const s = lokal("Ich weiß nicht.");
  const t = neu(
    {
      voice_call_chat: denker(30, BELEG_ZEILE),
      app_context_verify: async () => {
        await tor;
        return [1];
      },
    },
    s,
    { kontext: true },
  );
  const fertig = t.start(FRAGE);
  assert.equal(
    await bis(() => t.aufrufe("app_context_verify").length === 1),
    true,
  );
  t.engine.resetHistory();
  freigeben();
  await fertig;
  await settle(60);
  assert.deepEqual(t.log.citations, []);
});

test("H5 Quellen ohne Zwischenfall: die Meldung kommt (Regression)", async () => {
  const s = lokal("Ich weiß nicht.");
  const t = neu({ voice_call_chat: denker(30, BELEG_ZEILE) }, s, {
    kontext: true,
  });
  await t.start(FRAGE);
  assert.equal(await bis(() => t.log.citations.length === 1), true);
});

// Reine Zeilenlogik des Speichers ---------------------------------------------------------------------

const zeile = (id, role, extra = {}) => ({
  id,
  role,
  text: `z${id}`,
  at: 0,
  ...extra,
});

test("mitQuellen: gewöhnliche Antwort: die erste Antwortzeile nach der Frage", () => {
  const lines = [
    zeile(1, "user", { turnId: "A" }),
    zeile(2, "assistant", { final: true }),
    zeile(3, "user", { turnId: "B" }),
    zeile(4, "assistant", { final: true }),
  ];
  const r = mitQuellen(lines, "B", [TREFFER]);
  assert.deepEqual(
    r.map((l) => l.sources?.length),
    [undefined, undefined, undefined, 1],
  );
  assert.equal(r[3].sources[0].title, "Runde");
  assert.equal(
    mitQuellen(lines, "B", []),
    null,
    "ohne Treffer nichts zu ändern",
  );
  assert.equal(mitQuellen(lines, "unbekannt", [TREFFER]), null);
});

test("mitQuellen: das Ergebnis eines Auftrags findet seine Zeile über die Kennung, nicht über die Lage", () => {
  const lines = [
    zeile(1, "user", { turnId: "A", fest: true }),
    zeile(2, "assistant", { final: true }),
    zeile(3, "user", { turnId: "B" }),
    zeile(4, "assistant", { final: true }),
    zeile(5, "assistant", { final: true, auftragVon: "A" }),
    // Die Prüfung der Belege dauert: inzwischen steht eine spätere Antwort als letzte Zeile da.
    zeile(6, "user", { turnId: "C" }),
    zeile(7, "assistant", { final: true }),
  ];
  const r = mitQuellen(lines, "A", [TREFFER], true);
  assert.deepEqual(
    r.map((l) => l.sources?.length),
    [undefined, undefined, undefined, undefined, 1, undefined, undefined],
  );
  assert.equal(
    lines[4].sources,
    undefined,
    "das Eingangsfeld bleibt unverändert",
  );
  assert.equal(
    mitQuellen(lines, "X", [TREFFER], true),
    null,
    "keine Zeile mit dieser Kennung: nichts raten",
  );
});

test("mitQuellen: tragen zwei Zeilen die Kennung des Auftrags (überholter Versuch), gehen die Quellen an die jüngste", () => {
  const lines = [
    zeile(1, "user", { turnId: "A", fest: true }),
    zeile(2, "assistant", { final: true, auftragVon: "A" }),
    zeile(3, "user", { turnId: "B" }),
    zeile(4, "assistant", { final: true, auftragVon: "A" }),
  ];
  const r = mitQuellen(lines, "A", [TREFFER], true);
  assert.deepEqual(
    r.map((l) => l.sources?.length),
    [undefined, undefined, undefined, 1],
  );
});

test("denkOffenSetzen: klappt die laufende Anzeige, die des Auftrags im Hintergrund und die an der Antwort auf und zu", () => {
  const prozess = (seit, extra = {}) => ({ seit, schritte: [], ...extra });
  const s = {
    denken: null,
    auftrag: prozess(10, { turnId: "A" }),
    lines: [zeile(1, "assistant", { denken: prozess(5) })],
  };
  const auf = denkOffenSetzen(s, 10, true);
  assert.equal(auf.auftrag.offen, true);
  assert.equal(
    auf.lines[0].denken.offen,
    undefined,
    "ein anderer Prozess bleibt",
  );
  assert.equal(denkOffenSetzen(auf, 10, false).auftrag.offen, false);
  // Die Anzeige an der Antwort und die laufende Anzeige gehen wie bisher.
  const an = denkOffenSetzen({ ...s, denken: prozess(7) }, 5, true);
  assert.equal(an.lines[0].denken.offen, true);
  assert.equal(an.auftrag.offen, undefined);
  assert.equal(
    denkOffenSetzen({ ...s, denken: prozess(7) }, 7, true).denken.offen,
    true,
  );
  // Nichts davon passt: der Zustand bleibt, wie er war.
  assert.deepEqual(denkOffenSetzen(s, 99, true), s);
});

// -- Runde 5 (Codex R3 zu lva#547) ----------------------------------------------------------------------
//
// Fehlerfälle dieser Runde (vor dem Code aufgeschrieben, Status im Bericht):
//  H6 Abbruch mitten im Vorgang: das Ergebnis eines Auftrags ist eingereiht und angezeigt (Zeile mit Zwischenstand),
//     sein Ton hat nie begonnen; Esc, Stoppwort, Modell-/Gesprächswechsel, Widerruf, getippter Text oder Rückstellung
//     (der Nutzer spricht): die Zeile verschwindet, nichts ist gespeichert, kein Verlauf, keine Quellen; ein später
//     Beginn des Players oder eine späte Synthese bleibt wirkungslos und berührt die Frist des wartenden Auftrags nicht.
//  H7 Hängende Synthese ab dem zweiten Stück: erster Ton klingt, das nächste Stück kommt nie: nach
//     `stueckMaxMs` (ab Ende des Players) sind Schritt und Auftrag frei, das Gehörte bleibt als
//     unterbrochene Zeile, eine späte Synthese bleibt ohne Wirkung. Klingendes wird nie abgeschnitten, ein
//     rechtzeitiges Stück läuft weiter, angehaltene Wiedergabe (Entscheidung läuft) und Nutzerunterbrechung sind
//     keine Zeitüberschreitung.
//  Nicht neu: voller Speicher (Satzpuffer gedeckelt, Runde 1), fehlendes Gerät und Absturz des Denkers (Runde 1),
//  Player wirft (H3), Synthese des ersten Stücks hängt (H4: Frist des Auftrags).

/**
 * Wartet, bis alles abgearbeitet ist, was schon angestoßen wurde (Mikroaufgaben und anstehende Rückrufe), ohne
 * Zeitgeber: Grundlage negativer Nachweise („es kommt nichts mehr“).
 */
const flush = async () => {
  for (let i = 0; i < 3; i++) await new Promise((r) => setImmediate(r));
};

/**
 * Das Ergebnis des Auftrags (zwei Sätze) ist eingereiht und angezeigt, sein Ton hat nicht begonnen (der Player
 * hängt): der Schritt des Nachreichens ist komplett (`historyPushed`), aber noch nichts war zu hören.
 */
async function ergebnisStumm(aufbauOptionen = {}) {
  const r = await aufbauSteuerbar({
    zeilen: [WEITER_ZEILE, ZWEITE_ZEILE, WETTER_ZEILE],
    ...aufbauOptionen,
  });
  r.d.liefere(0, ...ZWEI_SAETZE);
  assert.equal(await bis(() => r.t.engine.auftrag?.fertig), true);
  r.t.player.deferStart = true;
  r.t.player.finish();
  assert.equal(
    await bis(() => r.t.engine.turn?.historyPushed === true),
    true,
    "das Ergebnis ist eingereiht, der Ton hat nicht begonnen",
  );
  return r;
}

/** Was in der Liste des echten Speichers zum Ergebnis des Auftrags steht. */
const ergebnisZeilen = (store, id) =>
  store.getState().lines.filter((l) => l.auftragVon === id);

/** Was die KI zum Ergebnis dieser Auftragsfrage im Verlauf kennt (Frage oder Ergebnistext). */
const kenntErgebnis = (t) =>
  t.verlauf().some((m) => m === `user:${FRAGE}` || m.includes("Protokoll"));

test("H6 echter Speicher, Esc bei eingereihtem, noch stummem Ergebnis: keine Zeile, kein Speicherbeitrag, kein Verlauf, kein späterer Ton", () =>
  ohneStille(async () => {
    const { store, engine, gespeichert, handlers } = await mitEchtemSpeicher();
    const { t, a1 } = await ergebnisStumm({
      handlers,
      optionen: { kontext: true, engine },
    });
    const gehoert = [...t.player.gespielt];
    const verworfen = t.log.verworfen;
    const zeilen = ergebnisZeilen(store, a1.id);
    assert.equal(zeilen.length, 1, "die Zeile mit dem Zwischenstand steht");
    assert.equal(zeilen[0].final, false);
    assert.equal(t.player.gespielt.length, gehoert.length, "noch kein Ton");

    t.engine.interrupt();

    assert.deepEqual(ergebnisZeilen(store, a1.id), [], "Esc nimmt sie zurück");
    assert.equal(
      store.getState().lines.some((l) => l.role === "assistant" && !l.final),
      false,
      "auch keine andere unfertige Antwortzeile",
    );
    assert.equal(t.log.verworfen, verworfen + 1);
    assert.equal(t.engine.auftrag, null);
    assert.equal(t.engine.turn, null);
    assert.equal(t.engine.state, "listening");
    assert.equal(store.getState().denken, null);
    assert.equal(store.getState().auftrag, null);
    assert.equal(kenntErgebnis(t), false, "kein Verlauf");
    assert.deepEqual(quellen(t, a1.id), [], "keine Quellen");
    // Später Beginn und Ende des Players, späte Ereignisse: nichts davon wirkt.
    t.player.deferStart = false;
    t.player.audioStart();
    t.player.finish();
    await flush();
    assert.deepEqual(t.player.gespielt, gehoert, "kein späterer Ton");
    assert.deepEqual(ergebnisZeilen(store, a1.id), []);
    assert.equal(
      gespeichert.some((e) => e.text.includes("Anna")),
      false,
      "nichts gespeichert",
    );
    assert.equal(kenntErgebnis(t), false);
    assert.deepEqual(quellen(t, a1.id), []);
    assert.equal(t.log.errors.length, 0);
  }));

test("H6 ein Player, der nach Esc noch einen Beginn meldet: keine endgültige Antwort, keine Quellen, kein Speicherbeitrag", () =>
  ohneStille(async () => {
    const { store, engine, gespeichert, handlers } = await mitEchtemSpeicher();
    const { t, a1 } = await ergebnisStumm({
      handlers,
      optionen: { kontext: true, engine },
    });
    // Der Player vergisst sein Eingereihtes beim Stoppen nicht (zu spät gemeldete Beginne sind denkbar).
    t.player.stop = function () {
      this.stopped++;
    };
    t.engine.interrupt();
    assert.deepEqual(ergebnisZeilen(store, a1.id), []);
    t.player.deferStart = false;
    t.player.audioStart();
    t.player.finish();
    await flush();
    assert.deepEqual(ergebnisZeilen(store, a1.id), []);
    assert.equal(
      t.log.zeilen.some((z) => z.final && z.auftragVon),
      false,
    );
    assert.deepEqual(quellen(t, a1.id), []);
    assert.equal(
      gespeichert.some((e) => e.text.includes("Anna")),
      false,
    );
    assert.equal(t.engine.state, "listening");
    assert.equal(t.engine.turn, null);
  }));

test("H6 Esc, solange die Synthese des Ergebnisses noch läuft: nichts erscheint, auch nicht, wenn sie später fertig wird", () =>
  ohneStille(async () => {
    let freigeben;
    const haengt = new Promise((r) => (freigeben = r));
    const { t, a1 } = await ergebnisBereit([ANTWORT], {
      handlers: {
        voice_call_synthesize: ({ text }) =>
          text === ANTWORT ? haengt : `wav:${text}`,
      },
    });
    t.player.finish();
    assert.equal(
      await bis(() => t.engine.turn?.nachreichen !== undefined),
      true,
    );
    const verworfen = t.log.verworfen;
    t.engine.interrupt();
    assert.equal(t.engine.auftrag, null);
    assert.equal(t.engine.turn, null);
    assert.equal(t.engine.state, "listening");
    freigeben(ERGEBNIS);
    await flush();
    assert.equal(t.player.enqueued.includes(ERGEBNIS), false);
    assert.equal(
      t.log.zeilen.some((z) => z.auftragVon),
      false,
      "keine Zeile, auch keine unfertige",
    );
    assert.equal(t.log.verworfen, verworfen, "nichts zurückzunehmen");
    assert.equal(kenntErgebnis(t), false);
    assert.deepEqual(quellen(t, a1.id), []);
  }));

const ABBRUCHWEGE = {
  "Esc (interrupt)": (t) => t.engine.interrupt(),
  "Ende des Gesprächs (stop)": (t) => t.engine.stop(),
  "Neues Gespräch (resetHistory)": (t) => t.engine.resetHistory(),
  "Modellwechsel (updateConfig)": (t) =>
    t.engine.updateConfig({ ...t.config, orchestration: { modell: "b" } }),
  "Widerruf der Freigabe (revoked)": () =>
    emit({ kind: "revoked", request_id: "*" }),
  "getippter Text (sendText)": (t) => void t.engine.sendText("Hallo"),
};
for (const [name, ausloeser] of Object.entries(ABBRUCHWEGE)) {
  test(`H6 ${name} bei eingereihtem, noch stummem Ergebnis: genau eine Rücknahme der Zeile, kein Verlauf, keine Quellen, kein Nachreichen`, () =>
    mitUhr(() =>
      ohneStille(async () => {
        const { t, a1 } = await ergebnisStumm({ optionen: { kontext: true } });
        const verworfen = t.log.verworfen;
        assert.ok(
          t.log.zeilen.some((z) => z.auftragVon === a1.id && !z.final),
          "die Zeile mit dem Zwischenstand steht",
        );
        await ausloeser(t, a1);
        assert.equal(t.log.verworfen, verworfen + 1);
        assert.equal(t.engine.auftrag, null);
        assert.equal(
          t.log.zeilen.some((z) => z.final && z.auftragVon),
          false,
          "nie eine endgültige Zeile des Auftrags",
        );
        assert.equal(kenntErgebnis(t), false);
        // Späte Ereignisse des alten Versuchs: Beginn, Ende, Synthese.
        t.player.deferStart = false;
        t.player.audioStart();
        t.player.finish();
        await flush();
        assert.equal(
          t.log.zeilen.some((z) => z.final && z.auftragVon),
          false,
        );
        assert.equal(t.log.verworfen, verworfen + 1, "keine zweite Rücknahme");
        assert.deepEqual(quellen(t, a1.id), []);
        assert.equal(t.log.errors.length, 0);
      }),
    ));
}

test("H6 echter Speicher: der Nutzer spricht (Rückstellung), danach Stoppwort: die Zeile verschwindet sofort, der Auftrag ist verworfen, nichts bleibt", () =>
  ohneStille(async () => {
    const { store, engine, gespeichert, handlers } = await mitEchtemSpeicher();
    const { t, a1 } = await ergebnisStumm({
      handlers,
      optionen: { kontext: true, engine },
    });
    t.redeLos();
    assert.ok(
      t.engine.auftrag && !t.engine.auftrag.verworfen,
      "zurückgestellt, der Auftrag bleibt",
    );
    assert.deepEqual(
      ergebnisZeilen(store, a1.id),
      [],
      "die Zeile steht nicht gespenstisch weiter, bis irgendwann eine Nutzerzeile kommt",
    );
    // Das Stoppwort geht nie als Nutzerzeile ins Gespräch: nichts würde die Zeile sonst wegräumen.
    t.erkenne("Stopp.");
    t.fruehEnde();
    t.ende();
    assert.equal(await bis(() => t.engine.auftrag === null), true);
    await flush();
    assert.deepEqual(ergebnisZeilen(store, a1.id), []);
    assert.equal(
      store.getState().lines.some((l) => l.role === "assistant" && !l.final),
      false,
    );
    assert.equal(
      store.getState().lines.some((l) => l.text === "Stopp."),
      false,
    );
    assert.equal(
      gespeichert.some((e) => e.text.includes("Anna")),
      false,
    );
    assert.equal(kenntErgebnis(t), false);
    assert.deepEqual(quellen(t, a1.id), []);
    assert.equal(t.engine.turn, null);
  }));

test("H6 ein später Beginn des zurückgestellten Versuchs lässt die Frist des wartenden Auftrags unberührt", () =>
  mitUhr((tick) =>
    ohneStille(async () => {
      const { t, a1 } = await ergebnisStumm({ optionen: { kontext: true } });
      // Der Player gibt sein Eingereihtes beim Stoppen nicht frei: der Beginn kommt nach der Rückstellung doch noch.
      t.player.stop = function () {
        this.stopped++;
      };
      t.redeLos();
      assert.ok(t.engine.auftrag && !t.engine.auftrag.nachreichend);
      t.player.deferStart = false;
      t.player.audioStart();
      await flush();
      assert.ok(
        t.engine.auftrag && !t.engine.auftrag.verworfen,
        "der Auftrag wartet weiter",
      );
      tick(TUNING.auftragMaxMs);
      assert.equal(
        t.engine.auftrag,
        null,
        "die Frist des wartenden Auftrags lief weiter und endete ihn",
      );
      assert.equal(
        t.log.zeilen.some((z) => z.final && z.auftragVon === a1.id),
        false,
      );
    }),
  ));

// H7 ----------------------------------------------------------------------------------------------

/** Zwei Stücke: das erste klingt, die Synthese des zweiten hängt bis `freigeben`. */
async function zweitesStueckHaengt() {
  let freigeben;
  const haengt = new Promise((r) => (freigeben = r));
  const r = await ergebnisBereit(["Satz eins.", "Satz zwei."], {
    zeilen: [WEITER_ZEILE, ZWEITE_ZEILE, WETTER_ZEILE],
    handlers: {
      voice_call_synthesize: ({ text }) =>
        text === "Satz zwei." ? haengt : `wav:${text}`,
    },
  });
  r.t.player.finish();
  assert.equal(
    await bis(() => r.t.player.gespielt.includes("wav:Satz eins.")),
    true,
    "das erste Stück klingt",
  );
  return { ...r, freigeben };
}

test("H7 hängende Synthese ab dem zweiten Stück: nach stueckMaxMs sind Schritt und Auftrag frei, das Gehörte bleibt unterbrochen stehen, die späte Synthese bleibt ohne Wirkung", () =>
  mitUhr((tick) =>
    ohneStille(async () => {
      const { t, a1, freigeben } = await zweitesStueckHaengt();
      // Solange das erste Stück klingt, läuft keine Wartezeit: nichts wird abgeschnitten, so lange es auch dauert.
      tick(TUNING.auftragMaxMs + 3 * TUNING.stueckMaxMs);
      assert.equal(t.engine.turn?.nachreichen?.turnId, a1.id);
      assert.equal(t.engine.turn.cancelled, false);
      assert.ok(t.engine.auftrag && !t.engine.auftrag.verworfen);
      // Das erste Stück ist zu Ende: der Player ist leer, das zweite fehlt.
      t.player.finishPiece();
      tick(TUNING.stueckMaxMs - 1);
      assert.equal(t.engine.turn?.nachreichen?.turnId, a1.id, "noch geduldig");
      assert.equal(t.engine.state !== "listening", true);
      tick(1);
      assert.equal(t.engine.turn, null, "der Schritt ist frei");
      assert.equal(t.engine.auftrag, null, "der Auftrag ist erledigt");
      assert.equal(t.engine.state, "listening");
      assert.deepEqual(t.log.zeilen.at(-1), {
        t: "Satz eins.",
        final: true,
        interrupted: true,
        auftragVon: a1.id,
      });
      assert.ok(
        t.engine.history.some(
          (m) =>
            m.role === "assistant" &&
            m.interrupted &&
            m.content.includes("Satz eins"),
        ),
        "der Verlauf kennt das Gehörte als unterbrochen",
      );
      assert.equal(
        t.engine.history.some((m) => m.content.includes("Satz zwei")),
        false,
      );
      const zeilen = t.log.zeilen.length;
      const verworfen = t.log.verworfen;
      freigeben("wav:Satz zwei.");
      await flush();
      assert.equal(t.player.enqueued.includes("wav:Satz zwei."), false);
      assert.equal(t.log.zeilen.length, zeilen, "keine weitere Zeile");
      assert.equal(t.log.verworfen, verworfen);
      assert.equal(t.log.errors.length, 0);
      assert.equal(t.engine.turn, null);
      assert.equal(t.engine.state, "listening");
    }),
  ));

test("H7 das zweite Stück kommt vor Ablauf der Wartezeit: das Ergebnis wird ganz gesprochen, danach ändert kein Zeitablauf etwas", () =>
  mitUhr((tick) =>
    ohneStille(async () => {
      const { t, a1, freigeben } = await zweitesStueckHaengt();
      t.player.finishPiece();
      tick(TUNING.stueckMaxMs - 1);
      freigeben("wav:Satz zwei.");
      assert.equal(
        await bis(() => t.player.enqueued.includes("wav:Satz zwei.")),
        true,
      );
      tick(TUNING.stueckMaxMs * 3);
      assert.equal(t.engine.turn?.nachreichen?.turnId, a1.id);
      assert.equal(t.engine.turn.cancelled, false, "nicht abgeschnitten");
      t.player.finish();
      assert.equal(await bis(() => t.engine.auftrag === null), true);
      assert.deepEqual(t.log.zeilen.at(-1), {
        t: "Satz eins. Satz zwei.",
        final: true,
        interrupted: false,
        auftragVon: a1.id,
      });
      tick(TUNING.stueckMaxMs * 3);
      assert.equal(t.log.zeilen.at(-1).interrupted, false);
      assert.equal(t.log.errors.length, 0);
      assert.equal(t.engine.state, "listening");
    }),
  ));

test("H7 drei Stücke: jedes Stück stellt die Wartezeit neu, nur die Lücke zählt, nicht die Gesamtdauer", () =>
  mitUhr((tick) =>
    ohneStille(async () => {
      const gates = new Map();
      const tor = (text) =>
        new Promise((r) => gates.set(text, () => r(`wav:${text}`)));
      const { t, a1 } = await ergebnisBereit(
        ["Satz eins.", "Satz zwei.", "Satz drei."],
        {
          handlers: {
            voice_call_synthesize: ({ text }) =>
              text === "Satz zwei." || text === "Satz drei."
                ? tor(text)
                : `wav:${text}`,
          },
        },
      );
      t.player.finish();
      assert.equal(
        await bis(() => t.player.gespielt.includes("wav:Satz eins.")),
        true,
      );
      for (const [satz, naechster] of [
        ["Satz zwei.", "Satz drei."],
        ["Satz drei.", null],
      ]) {
        t.player.finishPiece();
        tick(TUNING.stueckMaxMs - 1);
        assert.equal(t.engine.turn?.nachreichen?.turnId, a1.id);
        assert.equal(await bis(() => gates.has(satz)), true);
        gates.get(satz)();
        assert.equal(
          await bis(() => t.player.enqueued.includes(`wav:${satz}`)),
          true,
        );
        if (naechster) assert.equal(t.engine.turn?.cancelled, false);
      }
      t.player.finish();
      assert.equal(await bis(() => t.engine.auftrag === null), true);
      assert.deepEqual(t.log.zeilen.at(-1), {
        t: "Satz eins. Satz zwei. Satz drei.",
        final: true,
        interrupted: false,
        auftragVon: a1.id,
      });
    }),
  ));

test("H7 der Player lief leer, während das zweite Stück noch eingereiht wird (Dekodieren) und das dritte fehlt: das klingende zweite Stück wird nach Ablauf der Wartezeit nicht abgeschnitten", () =>
  mitUhr((tick) =>
    ohneStille(async () => {
      const frei = {};
      const tor = (text) =>
        new Promise((r) => (frei[text] = () => r(`wav:${text}`)));
      const { t, a1 } = await ergebnisBereit(
        ["Satz eins.", "Satz zwei.", "Satz drei."],
        {
          zeilen: [WEITER_ZEILE, ZWEITE_ZEILE, WETTER_ZEILE],
          handlers: {
            voice_call_synthesize: ({ text }) =>
              text === "Satz zwei." || text === "Satz drei."
                ? tor(text)
                : `wav:${text}`,
          },
        },
      );
      t.player.finish();
      assert.equal(
        await bis(() => t.player.gespielt.includes("wav:Satz eins.")),
        true,
      );
      const einreihen = t.player.enqueue;
      let dekodiert;
      const dekodieren = new Promise((r) => (dekodiert = r));
      t.player.enqueue = async (wav, ...rest) => {
        if (wav === "wav:Satz zwei.") await dekodieren;
        return einreihen(wav, ...rest);
      };
      assert.equal(await bis(() => frei["Satz zwei."]), true);
      frei["Satz zwei."]();
      await flush();
      // Das erste Stück endet, bevor das zweite im Player liegt: die Wartezeit beginnt.
      t.player.finishPiece();
      dekodiert();
      assert.equal(
        await bis(() => t.player.gespielt.includes("wav:Satz zwei.")),
        true,
        "das zweite Stück klingt",
      );
      tick(TUNING.stueckMaxMs * 3);
      assert.equal(t.engine.turn?.nachreichen?.turnId, a1.id);
      assert.equal(t.engine.turn.cancelled, false, "nichts abgeschnitten");
      assert.equal(await bis(() => frei["Satz drei."]), true);
      frei["Satz drei."]();
      assert.equal(
        await bis(() => t.player.enqueued.includes("wav:Satz drei.")),
        true,
      );
      t.player.finish();
      assert.equal(await bis(() => t.engine.auftrag === null), true);
      assert.deepEqual(t.log.zeilen.at(-1), {
        t: "Satz eins. Satz zwei. Satz drei.",
        final: true,
        interrupted: false,
        auftragVon: a1.id,
      });
    }),
  ));

test("H7 angehaltene Wiedergabe (die Entscheidung läuft) ist keine Zeitüberschreitung", () =>
  mitUhr((tick) =>
    ohneStille(async () => {
      const { t, a1 } = await zweitesStueckHaengt();
      t.player.finishPiece();
      t.barge();
      assert.equal(t.engine.held?.nachreichen?.turnId, a1.id, "angehalten");
      tick(TUNING.stueckMaxMs * 3);
      assert.equal(t.engine.turn?.nachreichen?.turnId, a1.id);
      assert.equal(t.engine.turn.cancelled, false);
      assert.ok(t.engine.auftrag && !t.engine.auftrag.verworfen);
      t.engine.interrupt();
      assert.equal(t.engine.turn, null);
    }),
  ));

test("H7 der Nutzer unterbricht in der Wartezeit wie sonst: ein Abbruch, die Wartezeit läuft danach nicht noch einmal", () =>
  mitUhr((tick) =>
    ohneStille(async () => {
      const { t, a1 } = await zweitesStueckHaengt();
      t.player.finishPiece();
      t.aeussere(WETTER);
      assert.equal(
        await bis(() => t.player.gespielt.includes(WETTER_SATZ)),
        true,
      );
      assert.equal(t.engine.auftrag, null, "der Auftrag ist erledigt");
      const unterbrochen = t.log.zeilen.filter(
        (z) => z.auftragVon === a1.id && z.interrupted,
      );
      assert.equal(unterbrochen.length, 1);
      assert.equal(unterbrochen[0].t, "Satz eins.");
      const turn = t.engine.turn;
      tick(TUNING.stueckMaxMs * 3);
      assert.equal(t.engine.turn, turn, "der neue Schritt läuft ungestört");
      assert.equal(turn.cancelled, false);
      assert.equal(
        t.log.zeilen.filter((z) => z.auftragVon === a1.id && z.interrupted)
          .length,
        1,
        "keine zweite Unterbrechungszeile",
      );
    }),
  ));

// -- Runde 6 (Codex R4 zu lva#547) ----------------------------------------------------------------------
//
// Fehlerfälle dieser Runde (vor dem Code aufgeschrieben, Status im Bericht):
//  H8 Abbruch mitten im Vorgang / Nebenläufigkeit: Pause (Halten durch Barge-in) und Wartezeit auf das nächste
//     Stück. Die Wartezeit darf während der Pause nicht ablaufen, danach aber nie verloren sein:
//     a) die Pause beginnt, wenn der Player schon leer ist (Wartezeit läuft), die Uhr läuft über die Frist, die
//        Pause endet ohne Erkennung (`releaseHold`), die Folgesynthese bleibt ungelöst: danach sind Turn und Auftrag
//        nach der Wartezeit frei, die Zeile ist genau einmal unterbrochen, eine späte Synthese bleibt wirkungslos;
//     b) die Pause beginnt, solange noch Ton läuft (keine Wartezeit), das Stück endet während der Pause;
//     c) die Pause beginnt bei laufendem Ton, die Wiedergabe geht nach der Pause weiter und endet erst danach;
//     d) die Pause endet über die Frist der Erkennung (`holdSttMaxMs`, späte Auswertung) statt über das Ende der Äußerung.
//     Zeit: gesteuerte Uhr, kein abschließendes `interrupt()`, kein begrenztes Warten für negative Nachweise.

/**
 * Pause bei hängender Folgesynthese, Fortsetzung ohne Erkennung (die Äußerung endet, ohne dass erkannt wird):
 * danach sind Turn und Auftrag nach der Wartezeit frei. Gemeinsame Prüfung nach der Fortsetzung.
 */
async function nachFortsetzungFrei(t, a1, freigeben, tick) {
  assert.equal(t.engine.held, null, "die Pause ist beendet");
  assert.equal(
    t.engine.turn?.nachreichen?.turnId,
    a1.id,
    "das Nachreichen läuft",
  );
  assert.equal(t.engine.turn.cancelled, false);
  tick(TUNING.stueckMaxMs - 1);
  assert.equal(
    t.engine.turn?.nachreichen?.turnId,
    a1.id,
    "die Wartezeit läuft von neuem, noch geduldig",
  );
  tick(1);
  assert.equal(t.engine.turn, null, "Turn frei");
  assert.equal(t.engine.auftrag, null, "Auftrag frei");
  assert.equal(t.engine.state, "listening");
  assert.equal(
    t.log.zeilen.filter((z) => z.auftragVon === a1.id && z.interrupted).length,
    1,
    "die gehörte Zeile ist genau einmal unterbrochen",
  );
  assert.deepEqual(t.log.zeilen.at(-1), {
    t: "Satz eins.",
    final: true,
    interrupted: true,
    auftragVon: a1.id,
  });
  const zeilen = t.log.zeilen.length;
  const verworfen = t.log.verworfen;
  freigeben("wav:Satz zwei.");
  // Nur Folgen von Versprechen stehen aus (kein Zeitgeber, kein E/A): ein Durchlauf der Ereignisschleife leert sie.
  await new Promise((r) => setImmediate(r));
  assert.equal(t.player.enqueued.includes("wav:Satz zwei."), false);
  assert.equal(t.log.zeilen.length, zeilen, "späte Synthese: keine neue Zeile");
  assert.equal(t.log.verworfen, verworfen);
  assert.equal(t.log.errors.length, 0);
  assert.equal(t.engine.turn, null);
}

/**
 * Ein Player, der das Fortsetzen einer leeren Wiedergabe nicht als Leerlauf meldet (dokumentiert ist es anders; die
 * Engine darf sich darauf nicht allein verlassen).
 */
function fortsetzenStumm(t) {
  const fortsetzen = t.player.resume.bind(t.player);
  t.player.resume = () => {
    const leerlauf = t.player.onIdle;
    t.player.onIdle = undefined;
    try {
      fortsetzen();
    } finally {
      t.player.onIdle = leerlauf;
    }
  };
}

const PLAYERARTEN = [
  ["der Player meldet beim Fortsetzen den Leerlauf", false],
  ["der Player bleibt beim Fortsetzen stumm", true],
];
for (const [art, stumm] of PLAYERARTEN) {
  const aufbauen = async () => {
    const r = await zweitesStueckHaengt();
    if (stumm) fortsetzenStumm(r.t);
    return r;
  };

  test(`H8 Pause bei leerem Player (Wartezeit läuft), Fortsetzung ohne Erkennung, Folgesynthese bleibt aus (${art}): danach ist die Wartezeit wieder begrenzt, Turn und Auftrag frei`, () =>
    mitUhr((tick) =>
      ohneStille(async () => {
        const { t, a1, freigeben } = await aufbauen();
        t.player.finishPiece();
        t.barge();
        assert.equal(t.engine.held?.nachreichen?.turnId, a1.id, "angehalten");
        // Die Uhr läuft in der Pause weit über die Frist: keine Zeitüberschreitung.
        tick(TUNING.stueckMaxMs * 3);
        assert.equal(t.engine.turn?.nachreichen?.turnId, a1.id);
        assert.equal(t.engine.turn.cancelled, false);
        assert.equal(t.engine.held?.nachreichen?.turnId, a1.id);
        // Die Äußerung endet, bevor erkannt wurde: die Wiedergabe läuft weiter (leer).
        t.ende();
        await nachFortsetzungFrei(t, a1, freigeben, tick);
      }),
    ));

  test(`H8 Pause beginnt bei klingendem Stück, dieses endet in der Pause, Fortsetzung ohne Erkennung (${art}): die Wartezeit ist danach wieder begrenzt`, () =>
    mitUhr((tick) =>
      ohneStille(async () => {
        const { t, a1, freigeben } = await aufbauen();
        t.barge();
        assert.equal(t.engine.held?.nachreichen?.turnId, a1.id, "angehalten");
        t.player.finishPiece();
        tick(TUNING.stueckMaxMs * 3);
        assert.equal(t.engine.turn?.nachreichen?.turnId, a1.id);
        assert.equal(t.engine.turn.cancelled, false, "in der Pause nichts");
        t.ende();
        await nachFortsetzungFrei(t, a1, freigeben, tick);
      }),
    ));

  test(`H8 Pause endet über die Frist der Erkennung (späte Auswertung), die Folgesynthese bleibt aus (${art}): die Wartezeit ist danach wieder begrenzt`, () =>
    mitUhr((tick) =>
      ohneStille(async () => {
        const { t, a1, freigeben } = await aufbauen();
        t.player.finishPiece();
        // Die Erkennung antwortet nie (Tor), die Äußerung endet: die Pause wartet auf `holdSttMaxMs`.
        let erkennungBegonnen;
        const begonnen = new Promise((r) => (erkennungBegonnen = r));
        tauri.handlers.voice_call_transcribe = () => {
          erkennungBegonnen();
          return new Promise(() => undefined);
        };
        let pauseBeendet;
        const beendet = new Promise((r) => (pauseBeendet = r));
        const fortsetzen = t.engine.releaseHold.bind(t.engine);
        t.engine.releaseHold = () => {
          fortsetzen();
          pauseBeendet();
        };
        t.barge();
        t.fruehEnde();
        t.ende();
        assert.equal(t.engine.held?.nachreichen?.turnId, a1.id, "angehalten");
        await begonnen;
        // Beide Fristen laufen in diesem einen Schritt der Uhr ab (zuerst die der Erkennung, dann die Wartezeit auf das
        // Stück); die Folgen der Erkennungsfrist (Versprechen) kommen erst danach: die Wartezeit lief in der Pause.
        tick(TUNING.stueckMaxMs * 3);
        assert.equal(
          t.engine.held?.nachreichen?.turnId,
          a1.id,
          "die Wartezeit lief ab, während die Pause noch galt",
        );
        await beendet;
        assert.equal(t.engine.held, null, "Pause über die Frist beendet");
        await nachFortsetzungFrei(t, a1, freigeben, tick);
      }),
    ));
}

test("H8 Pause beginnt bei klingendem Stück und endet vor dessen Ende: die Wiedergabe geht weiter, erst ihr Ende startet die Wartezeit", () =>
  mitUhr((tick) =>
    ohneStille(async () => {
      const { t, a1, freigeben } = await zweitesStueckHaengt();
      t.barge();
      tick(TUNING.stueckMaxMs * 3);
      t.ende();
      assert.equal(t.engine.held, null);
      // Das Stück klingt weiter: auch lange danach wird nichts abgeschnitten.
      tick(TUNING.stueckMaxMs * 3);
      assert.equal(t.engine.turn?.nachreichen?.turnId, a1.id);
      assert.equal(t.engine.turn.cancelled, false);
      t.player.finishPiece();
      await nachFortsetzungFrei(t, a1, freigeben, tick);
    }),
  ));

// -- Runde 7 (lva#551: gewöhnlicher Schritt) ----------------------------------------------------------------
//
// Fehlerfälle dieser Runde (vor dem Code aufgeschrieben, Status im Bericht):
//  I1 Nebenläufigkeit / Absturz des Kindprozesses: die Synthese eines Stücks ab dem zweiten antwortet nie (Tor, das
//     der Test nie oder erst nach Ablauf öffnet). Der gewöhnliche Schritt endet nach `stueckMaxMs` ab Leerlauf des
//     Players wie ein Abbruch: Gehörtes bleibt als unterbrochene Zeile, Schritt frei, Zustand listening, Denker
//     abgebrochen, späte Synthese ohne Wirkung. Bei drei Stücken beginnt die Wartezeit bei jedem Leerlauf neu.
//  I2 Abbruch mitten im Vorgang: der Nutzer unterbricht während der Wartezeit (mit und ohne Pause): mit Pause ist das
//     Gehörte genau einmal unterbrochen, ohne Pause (nichts zu hören) ersetzt die Äußerung den Schritt wie bisher; der
//     neue Schritt läuft ungestört, die Frist des alten trifft ihn nicht, seine späte Synthese bleibt ohne Wirkung.
//  I3 Pause (`held`): die Wartezeit läuft in der Pause nicht ab und geht danach nicht verloren (Player meldet beim
//     Fortsetzen den Leerlauf oder bleibt stumm; Pause bei leerem Player, bei klingendem Ton mit Ende in der Pause,
//     bei klingendem Ton, der erst nach der Pause endet).
//  I4 Dekodier-Race: das Stück ist eingereiht, der Ton hat noch nicht begonnen: kein Ablauf.
//  I5 Warten auf das Modell: nichts zu synthetisieren, also keine Wartezeit (die Fristen des Modells gelten).
//  I6 Stummes Fortsetzen: ein fertiger Schritt endet nach der Pause, auch wenn der Player keinen Leerlauf meldet.
//  I7 Fehlendes Gerät: die Synthese schlägt fehl: ein Fehler, der Schritt endet, später nichts mehr.
//  I8 Textpfad und Projektgespräch: ohne Synthese kein Hänger (Textpfad); im Projekt dieselbe Kette wie sonst.
//  Zeit: gesteuerte Uhr, kein abschließendes `interrupt()`, keine begrenzten Wartezeiten für negative Nachweise.

/**
 * Gewöhnlicher Schritt (kein Auftrag): der Denker (`d`) liefert die Sätze als Stücke. Die Synthese der Sätze in
 * `haengt` antwortet erst mit `frei(satz)`. Der Schritt läuft im Hintergrund. `sprecher`: lokaler Sofort-Sprecher
 * (dann als gesprochene Äußerung), `ohneDenker`: Vorlagenantwort ohne Modellaufruf.
 */
async function gewoehnlich({
  haengt = [],
  optionen,
  handlers = {},
  frage = FRAGE,
  sprecher,
  ohneDenker = false,
  sicher = false,
  vorbereiten,
} = {}) {
  const d = denkerSteuerbar();
  const tore = new Map(
    haengt.map((satz) => {
      let frei;
      const tor = new Promise((r) => (frei = () => r(`wav:${satz}`)));
      return [satz, { tor, frei }];
    }),
  );
  const t = neu(
    {
      voice_call_chat: d.chat,
      voice_call_cancel: d.cancel,
      voice_call_synthesize: ({ text }) => tore.get(text)?.tor ?? `wav:${text}`,
      ...handlers,
    },
    sprecher,
    optionen,
  );
  vorbereiten?.(t);
  const lauf = sprecher
    ? t.startSprache(frage).fertig
    : sicher
      ? t.startSicher(frage)
      : t.start(frage).catch((e) => t.log.errors.push(String(e)));
  if (!ohneDenker) assert.equal(await bis(() => d.laeufe.length === 1), true);
  return { d, t, lauf, frei: (satz) => tore.get(satz).frei() };
}

/** Wie ein Abbruch durch den Nutzer: das Gehörte (`gehoert`) steht als unterbrochene Zeile, der Schritt ist frei. */
function wieAbbruch(t, d, gehoert) {
  assert.equal(t.engine.turn, null, "Schritt frei");
  assert.equal(t.engine.state, "listening");
  assert.deepEqual(t.log.zeilen.at(-1), {
    t: gehoert,
    final: true,
    interrupted: true,
    auftragVon: undefined,
  });
  assert.equal(
    t.log.zeilen.filter((z) => z.interrupted).length,
    1,
    "genau eine unterbrochene Zeile",
  );
  assert.deepEqual(t.verlauf(), [`user:${FRAGE}`, `assistant:${gehoert} …`]);
  assert.equal(t.abgebrochen(d.laeufe[0].requestId), 1, "Denker abgebrochen");
}

/** Die späte Synthese eines verworfenen Schritts bleibt ohne Wirkung (nur Versprechen stehen aus: ein Durchlauf genügt). */
async function spaeteSynthese(t, frei, satz) {
  const eingereiht = t.player.enqueued.length;
  const zeilen = t.log.zeilen.length;
  const verworfen = t.log.verworfen;
  frei(satz);
  await new Promise((r) => setImmediate(r));
  assert.equal(t.player.enqueued.length, eingereiht, "kein Ton");
  assert.equal(t.log.zeilen.length, zeilen, "keine neue Zeile");
  assert.equal(t.log.verworfen, verworfen);
  assert.equal(t.log.errors.length, 0);
  assert.equal(t.engine.turn, null);
}

const ersterKlingt = (t) =>
  bis(() => t.player.gespielt.includes("wav:Satz eins."));

test("H9 gewöhnlicher Schritt, Synthese ab Stück 2 bleibt aus: nach stueckMaxMs ab Leerlauf ist der Schritt wie bei einem Abbruch frei, die späte Synthese bleibt ohne Wirkung", () =>
  mitUhr(async (tick) => {
    const { d, t, frei } = await gewoehnlich({ haengt: ["Satz zwei."] });
    d.liefere(0, "Satz eins.", "Satz zwei.");
    assert.equal(await ersterKlingt(t), true);
    // Solange das erste Stück klingt, läuft keine Wartezeit: nichts wird abgeschnitten, so lange es auch dauert.
    tick(3 * TUNING.stueckMaxMs);
    assert.ok(t.engine.turn);
    assert.equal(t.engine.turn.cancelled, false);
    // Das erste Stück ist zu Ende: der Player ist leer, das zweite fehlt.
    t.player.finishPiece();
    tick(TUNING.stueckMaxMs - 1);
    assert.ok(t.engine.turn, "noch geduldig");
    assert.equal(t.engine.turn.cancelled, false);
    tick(1);
    wieAbbruch(t, d, "Satz eins.");
    await spaeteSynthese(t, frei, "Satz zwei.");
  }));

test("H9 drei Stücke: die Wartezeit beginnt bei jedem Leerlauf neu, ein gerade noch rechtzeitiges Stück verlängert sie nicht", () =>
  mitUhr(async (tick) => {
    const { d, t, frei } = await gewoehnlich({
      haengt: ["Satz zwei.", "Satz drei."],
    });
    d.liefere(0, "Satz eins.", "Satz zwei.", "Satz drei.");
    assert.equal(await ersterKlingt(t), true);
    t.player.finishPiece();
    tick(TUNING.stueckMaxMs - 1);
    assert.ok(t.engine.turn, "Stück zwei: noch geduldig");
    frei("Satz zwei.");
    assert.equal(
      await bis(() => t.player.gespielt.includes("wav:Satz zwei.")),
      true,
      "Stück zwei kam gerade noch rechtzeitig",
    );
    // Es klingt: auch lange danach wird nichts abgeschnitten, die verbrauchte Zeit zählt nicht mit.
    tick(3 * TUNING.stueckMaxMs);
    assert.ok(t.engine.turn);
    assert.equal(t.engine.turn.cancelled, false);
    // Stück drei fehlt: die Wartezeit beginnt mit diesem Leerlauf von neuem.
    t.player.finishPiece();
    tick(TUNING.stueckMaxMs - 1);
    assert.ok(t.engine.turn, "Stück drei: noch geduldig");
    tick(1);
    wieAbbruch(t, d, "Satz eins. Satz zwei.");
    await spaeteSynthese(t, frei, "Satz drei.");
  }));

test("H9 rechtzeitiges Stück (Regression): der Schritt wird vollständig gesprochen und endet regulär, keine unterbrochene Zeile", () =>
  mitUhr(async (tick) => {
    const { d, t, frei } = await gewoehnlich({ haengt: ["Satz zwei."] });
    d.liefere(0, "Satz eins.", "Satz zwei.");
    assert.equal(await ersterKlingt(t), true);
    t.player.finishPiece();
    tick(TUNING.stueckMaxMs - 1);
    frei("Satz zwei.");
    assert.equal(
      await bis(() => t.player.gespielt.includes("wav:Satz zwei.")),
      true,
    );
    assert.equal(
      await bis(() => t.engine.turn?.done),
      true,
      "die Antwort ist fertig erzeugt",
    );
    t.player.finishPiece();
    assert.equal(t.engine.turn, null, "der Schritt ist regulär zu Ende");
    assert.equal(t.engine.state, "listening");
    assert.equal(
      t.log.zeilen.some((z) => z.interrupted),
      false,
    );
    assert.deepEqual(t.log.zeilen.at(-1), {
      t: "Satz eins. Satz zwei.",
      final: true,
      interrupted: false,
      auftragVon: undefined,
    });
    assert.deepEqual(t.verlauf(), [
      `user:${FRAGE}`,
      "assistant:Satz eins. Satz zwei.",
    ]);
    assert.equal(t.abgebrochen(d.laeufe[0].requestId), 0);
    assert.equal(t.log.errors.length, 0);
  }));

test("H9 Warten auf das Modell (kein Stück liegt vor, nichts zu synthetisieren): keine Wartezeit, der Schritt läuft nach langer Lücke weiter", () =>
  mitUhr(async (tick) => {
    const { d, t } = await gewoehnlich();
    const id = d.laeufe[0].requestId;
    emit({ kind: "sentence", request_id: id, text: "Satz eins." });
    assert.equal(await ersterKlingt(t), true);
    t.player.finishPiece();
    // Das Modell braucht lange: für die Fristen des Modells ist die Engine nicht zuständig.
    tick(3 * TUNING.stueckMaxMs);
    assert.ok(t.engine.turn);
    assert.equal(t.engine.turn.cancelled, false);
    emit({ kind: "sentence", request_id: id, text: "Satz zwei." });
    emit({ kind: "done", request_id: id });
    d.laeufe[0].resolve("Satz eins. Satz zwei.");
    assert.equal(
      await bis(() => t.player.gespielt.includes("wav:Satz zwei.")),
      true,
    );
    assert.equal(await bis(() => t.engine.turn?.done), true);
    t.player.finishPiece();
    assert.equal(t.engine.turn, null);
    assert.equal(
      t.log.zeilen.some((z) => z.interrupted),
      false,
    );
  }));

test("H9 Dekodier-Race: das zweite Stück ist eingereiht, sein Ton hat noch nicht begonnen: kein Ablauf, danach endet der Schritt regulär", () =>
  mitUhr(async (tick) => {
    const { d, t, frei } = await gewoehnlich({ haengt: ["Satz zwei."] });
    d.liefere(0, "Satz eins.", "Satz zwei.");
    assert.equal(await ersterKlingt(t), true);
    t.player.finishPiece();
    tick(TUNING.stueckMaxMs - 1);
    // Der Ton des nächsten Stücks beginnt erst auf `audioStart()` (der Player dekodiert noch).
    t.player.deferStart = true;
    frei("Satz zwei.");
    assert.equal(
      await bis(() => t.player.enqueued.includes("wav:Satz zwei.")),
      true,
    );
    assert.equal(await bis(() => t.engine.turn?.done), true);
    tick(3 * TUNING.stueckMaxMs);
    assert.ok(t.engine.turn, "eingereiht: nichts läuft ab");
    assert.equal(t.engine.turn.cancelled, false);
    t.player.deferStart = false;
    t.player.audioStart();
    assert.equal(t.player.gespielt.includes("wav:Satz zwei."), true);
    t.player.finishPiece();
    assert.equal(t.engine.turn, null);
    assert.equal(
      t.log.zeilen.some((z) => z.interrupted),
      false,
    );
  }));

for (const [art, stumm] of PLAYERARTEN) {
  const aufbauen = async () => {
    const r = await gewoehnlich({ haengt: ["Satz zwei."] });
    if (stumm) fortsetzenStumm(r.t);
    r.d.liefere(0, "Satz eins.", "Satz zwei.");
    assert.equal(await ersterKlingt(r.t), true);
    return r;
  };
  /** Nach der Pause: die Wartezeit beginnt von neuem, danach ist der Schritt wie bei einem Abbruch frei. */
  const nachPause = async (t, d, frei, tick) => {
    assert.equal(t.engine.held, null, "die Pause ist beendet");
    assert.ok(t.engine.turn);
    assert.equal(t.engine.turn.cancelled, false);
    tick(TUNING.stueckMaxMs - 1);
    assert.ok(t.engine.turn, "die Wartezeit läuft von neuem, noch geduldig");
    tick(1);
    wieAbbruch(t, d, "Satz eins.");
    await spaeteSynthese(t, frei, "Satz zwei.");
  };

  test(`H9 Pause bei leerem Player (Wartezeit läuft), Fortsetzung ohne Erkennung (${art}): in der Pause läuft nichts ab, danach ist die Wartezeit wieder begrenzt`, () =>
    mitUhr(async (tick) => {
      const { d, t, frei } = await aufbauen();
      t.player.finishPiece();
      t.barge();
      assert.equal(t.engine.held, t.engine.turn, "angehalten");
      // Die Uhr läuft in der Pause weit über die Frist: keine Zeitüberschreitung.
      tick(3 * TUNING.stueckMaxMs);
      assert.ok(t.engine.turn);
      assert.equal(t.engine.turn.cancelled, false);
      t.ende();
      await nachPause(t, d, frei, tick);
    }));

  test(`H9 Pause beginnt bei klingendem Stück, dieses endet in der Pause (${art}): die Wartezeit ist nach der Pause wieder begrenzt`, () =>
    mitUhr(async (tick) => {
      const { d, t, frei } = await aufbauen();
      t.barge();
      assert.equal(t.engine.held, t.engine.turn, "angehalten");
      t.player.finishPiece();
      tick(3 * TUNING.stueckMaxMs);
      assert.ok(t.engine.turn, "in der Pause nichts");
      assert.equal(t.engine.turn.cancelled, false);
      t.ende();
      await nachPause(t, d, frei, tick);
    }));

  test(`H9 Pause beginnt bei klingendem Stück und endet vor dessen Ende (${art}): die Wiedergabe geht weiter, erst ihr Ende startet die Wartezeit`, () =>
    mitUhr(async (tick) => {
      const { d, t, frei } = await aufbauen();
      t.barge();
      tick(3 * TUNING.stueckMaxMs);
      t.ende();
      assert.equal(t.engine.held, null);
      // Das Stück klingt weiter: auch lange danach wird nichts abgeschnitten.
      tick(3 * TUNING.stueckMaxMs);
      assert.ok(t.engine.turn);
      assert.equal(t.engine.turn.cancelled, false);
      t.player.finishPiece();
      await nachPause(t, d, frei, tick);
    }));

  test(`H9 stummes Fortsetzen: ein fertiger Schritt endet nach der Pause auch ohne Leerlaufmeldung des Players (${art})`, () =>
    mitUhr(async (tick) => {
      const { d, t } = await gewoehnlich();
      if (stumm) fortsetzenStumm(t);
      d.liefere(0, "Satz eins.");
      assert.equal(await ersterKlingt(t), true);
      assert.equal(
        await bis(() => t.engine.turn?.done),
        true,
        "die Antwort ist fertig erzeugt",
      );
      t.barge();
      assert.equal(t.engine.held, t.engine.turn, "angehalten");
      // Das letzte Stück endet in der Pause: ein angehaltener Schritt endet nicht von selbst.
      t.player.finishPiece();
      tick(3 * TUNING.stueckMaxMs);
      assert.ok(t.engine.turn, "angehalten: kein Ende");
      t.ende();
      assert.equal(t.engine.held, null);
      assert.equal(t.engine.turn, null, "der fertige Schritt ist zu Ende");
      assert.equal(t.engine.state, "listening");
      assert.deepEqual(
        t.log.zeilen.filter((z) => z.final),
        [
          {
            t: "Satz eins.",
            final: true,
            interrupted: false,
            auftragVon: undefined,
          },
        ],
        "genau eine Schlusszeile",
      );
      assert.deepEqual(t.verlauf(), [`user:${FRAGE}`, "assistant:Satz eins."]);
      assert.equal(t.abgebrochen(d.laeufe[0].requestId), 0);
    }));
}

for (const mitBarge of [true, false]) {
  test(`H9 Unterbrechung während der Wartezeit (${mitBarge ? "mit Pause" : "ohne Pause"}): eine unterbrochene Zeile, der neue Schritt läuft ungestört, die Frist des alten trifft ihn nicht`, () =>
    mitUhr(async (tick) => {
      const { d, t, frei } = await gewoehnlich({ haengt: ["Satz zwei."] });
      d.liefere(0, "Satz eins.", "Satz zwei.");
      assert.equal(await ersterKlingt(t), true);
      t.player.finishPiece();
      // Die Wartezeit ist fast abgelaufen, da redet der Nutzer.
      tick(TUNING.stueckMaxMs - 1);
      t.aeussere(WETTER, { mitBarge });
      assert.equal(
        await bis(() => d.laeufe.length === 2),
        true,
        "der neue Schritt fragt den Denker",
      );
      const turn = t.engine.turn;
      assert.ok(turn);
      assert.equal(turn.cancelled, false);
      // Die Frist des alten Schritts wäre hier lange abgelaufen; der neue wartet noch auf den Denker (nichts klingt).
      tick(3 * TUNING.stueckMaxMs);
      assert.equal(t.engine.turn, turn, "der neue Schritt läuft ungestört");
      assert.equal(turn.cancelled, false);
      d.liefere(1, "Satz drei.");
      assert.equal(
        await bis(() => t.player.gespielt.includes("wav:Satz drei.")),
        true,
      );
      // Mit Pause ist das Gehörte unterbrochen; ohne Pause (nichts war zu hören) ersetzt die Äußerung den Schritt.
      const erwartet = mitBarge ? ["Satz eins."] : [];
      assert.deepEqual(
        t.log.zeilen.filter((z) => z.interrupted).map((z) => z.t),
        erwartet,
      );
      assert.equal(t.abgebrochen(d.laeufe[0].requestId), 1);
      // Die späte Synthese des alten Schritts bleibt ohne Wirkung.
      const eingereiht = t.player.enqueued.length;
      frei("Satz zwei.");
      await new Promise((r) => setImmediate(r));
      assert.equal(t.player.enqueued.length, eingereiht, "kein Ton");
      assert.equal(t.engine.turn, turn);
      assert.equal(await bis(() => t.engine.turn?.done), true);
      t.player.finishPiece();
      assert.equal(t.engine.turn, null, "der neue Schritt endet regulär");
      assert.deepEqual(
        t.log.zeilen.filter((z) => z.interrupted).map((z) => z.t),
        erwartet,
        "keine zweite unterbrochene Zeile",
      );
      assert.equal(t.log.errors.length, 0);
    }));
}

test("H9 Synthese des zweiten Stücks schlägt fehl (kein Gerät): ein Fehler, der Schritt endet, später läuft nichts mehr ab", () =>
  mitUhr(async (tick) => {
    let scheitern;
    const kaputt = new Promise((_, r) => (scheitern = r));
    const { d, t } = await gewoehnlich({
      handlers: {
        voice_call_synthesize: ({ text }) =>
          text === "Satz zwei." ? kaputt : `wav:${text}`,
      },
    });
    d.liefere(0, "Satz eins.", "Satz zwei.");
    assert.equal(await ersterKlingt(t), true);
    assert.equal(
      await bis(() =>
        t
          .aufrufe("voice_call_synthesize")
          .some((c) => c.args.text === "Satz zwei."),
      ),
      true,
    );
    t.player.finishPiece();
    scheitern(new Error("Stimme nicht verfügbar"));
    assert.equal(
      await bis(() => t.engine.turn === null),
      true,
      "der Schritt endet",
    );
    tick(3 * TUNING.stueckMaxMs);
    assert.equal(t.log.errors.length, 1, "genau ein Fehler");
    assert.equal(t.engine.turn, null);
    assert.equal(t.engine.state, "listening");
    assert.equal(
      t.log.zeilen.some((z) => z.interrupted),
      false,
      "kein Abbruch durch die Wartezeit",
    );
  }));

test("H9 Projektgespräch: dieselbe Kette, die Synthese bleibt aus, der Schritt endet nach stueckMaxMs wie bei einem Abbruch", () =>
  mitUhr(async (tick) => {
    const { d, t, frei } = await gewoehnlich({
      haengt: ["Satz zwei."],
      optionen: { projekt: { id: "p1", stand: 1 } },
    });
    d.liefere(0, "Satz eins.", "Satz zwei.");
    assert.equal(await ersterKlingt(t), true);
    t.player.finishPiece();
    tick(TUNING.stueckMaxMs - 1);
    assert.ok(t.engine.turn, "noch geduldig");
    tick(1);
    assert.equal(t.engine.turn, null, "Schritt frei");
    assert.equal(t.engine.state, "listening");
    assert.equal(t.abgebrochen(d.laeufe[0].requestId), 1);
    const zeilen = t.log.zeilen.length;
    const eingereiht = t.player.enqueued.length;
    frei("Satz zwei.");
    await new Promise((r) => setImmediate(r));
    assert.equal(t.player.enqueued.length, eingereiht);
    assert.equal(t.log.zeilen.length, zeilen);
    assert.equal(t.log.errors.length, 0);
  }));

test("H9 Textpfad ohne Anruf: keine Synthese, ein hängendes Synthesegerät hält den Schritt nicht auf", async () => {
  const t = neu(
    {
      voice_call_chat: denker(5, "Satz eins.", "Satz zwei."),
      voice_call_synthesize: () => new Promise(() => undefined),
    },
    undefined,
  );
  t.engine.state = "idle";
  await t.engine.sendText(FRAGE);
  assert.equal(t.aufrufe("voice_call_synthesize").length, 0);
  assert.equal(t.engine.turn, null);
  assert.equal(t.log.errors.length, 0);
  assert.equal(t.log.assistant.at(-1).t, "Satz eins. Satz zwei.");
});

test("H9 ein weiteres Stück vom Modell verlängert die laufende Wartezeit nicht", () =>
  mitUhr(async (tick) => {
    const { d, t, frei } = await gewoehnlich({ haengt: ["Satz zwei."] });
    const satz = (text) =>
      emit({ kind: "sentence", request_id: d.laeufe[0].requestId, text });
    satz("Satz eins.");
    satz("Satz zwei.");
    assert.equal(await ersterKlingt(t), true);
    t.player.finishPiece();
    tick(TUNING.stueckMaxMs - 5);
    // Das Modell liefert weiter, Stück zwei hängt noch: die Wartezeit läuft vom Leerlauf an, nicht vom neuen Stück.
    satz("Satz drei.");
    tick(4);
    assert.ok(t.engine.turn, "noch geduldig");
    tick(1);
    wieAbbruch(t, d, "Satz eins.");
    await spaeteSynthese(t, frei, "Satz zwei.");
  }));

test("H9 das Modell liefert das nächste Stück erst nach dem Leerlauf, seine Synthese bleibt aus: die Wartezeit beginnt mit dem Stück", () =>
  mitUhr(async (tick) => {
    const { d, t, frei } = await gewoehnlich({ haengt: ["Satz zwei."] });
    const satz = (text) =>
      emit({ kind: "sentence", request_id: d.laeufe[0].requestId, text });
    satz("Satz eins.");
    assert.equal(await ersterKlingt(t), true);
    t.player.finishPiece();
    tick(3 * TUNING.stueckMaxMs);
    assert.ok(t.engine.turn, "das Modell ist langsam: nichts läuft ab");
    assert.equal(t.engine.turn.cancelled, false);
    // Der Player ist längst leer und meldet nichts mehr: das neue Stück beginnt die Wartezeit selbst.
    satz("Satz zwei.");
    tick(TUNING.stueckMaxMs - 1);
    assert.ok(t.engine.turn, "noch geduldig");
    tick(1);
    wieAbbruch(t, d, "Satz eins.");
    await spaeteSynthese(t, frei, "Satz zwei.");
  }));

test("H9 Synthese eines Stücks schlägt fehl, das Modell liefert später weiter: die Wartezeit erlischt mit dem gescheiterten Stück, der Schritt läuft regulär weiter", () =>
  mitUhr(async (tick) => {
    let scheitern;
    const kaputt = new Promise((_, r) => (scheitern = r));
    const { d, t } = await gewoehnlich({
      handlers: {
        voice_call_synthesize: ({ text }) =>
          text === "Satz zwei." ? kaputt : `wav:${text}`,
      },
    });
    const id = d.laeufe[0].requestId;
    const satz = (text) => emit({ kind: "sentence", request_id: id, text });
    satz("Satz eins.");
    satz("Satz zwei.");
    assert.equal(await ersterKlingt(t), true);
    assert.equal(
      await bis(() =>
        t
          .aufrufe("voice_call_synthesize")
          .some((c) => c.args.text === "Satz zwei."),
      ),
      true,
    );
    t.player.finishPiece();
    scheitern(new Error("Stimme nicht verfügbar"));
    assert.equal(await bis(() => t.log.errors.length === 1), true);
    // Nichts liegt mehr vor, das Modell ist langsam: die Fristen des Modells gelten, keine der Engine.
    tick(3 * TUNING.stueckMaxMs);
    assert.ok(t.engine.turn);
    assert.equal(t.engine.turn.cancelled, false);
    satz("Satz drei.");
    emit({ kind: "done", request_id: id });
    d.laeufe[0].resolve("Satz eins. Satz zwei. Satz drei.");
    assert.equal(
      await bis(() => t.player.gespielt.includes("wav:Satz drei.")),
      true,
    );
    assert.equal(await bis(() => t.engine.turn?.done), true);
    t.player.finishPiece();
    assert.equal(t.engine.turn, null);
    assert.equal(
      t.log.zeilen.some((z) => z.interrupted),
      false,
    );
    assert.equal(t.log.errors.length, 1);
  }));

// -- Runde 8 (Codex R1 zu lva#564) --------------------------------------------------------------------------
//
// Fehlerfälle dieser Runde (vor dem Code aufgeschrieben, Status im Bericht):
//  J1 Abbruch mitten im Vorgang, Nebenläufigkeit: „wiederholen“ liefert zwei Vorlagenstücke, das zweite hängt. Nach
//     dem Ablauf steht nur das Gehörte als unterbrochene Zeile, der ungesprochene Text ist weg, es gibt keinen
//     Verlauf (Vorlage). Mit dem echten Speicher auch beim Beginn des nächsten Schritts: kein Rest, nichts Ungesprochenes
//     gespeichert.
//  J2 Weitersprechen gehört, die erste Inhaltssynthese hängt: die Zeile ist auf das Weitersprechen gekürzt
//     (unterbrochen), auch im echten Speicher.
//  J3 Nichts gehört (Ton hat nie begonnen): bricht der Nutzer ab, verschwindet die angezeigte, noch offene Zeile.
//  J4 Haltefrist der Erkennung, danach Wartegrenze, dann kommt die Erkennung: die Äußerung geht nicht verloren (ein
//     neuer Schritt, Text unverändert), ebenso auf dem Weg des Auftragskandidaten (Erkennung ohne Pause).
//  J5 Die späte Erkennung nach dem Ablauf ist ein Stoppwort: sie wirkt wie sonst (kein neuer Schritt).
//  J6 Zwischen Ablauf und später Erkennung beginnt ein anderer Schritt (getippter Text): die späte Erkennung gilt
//     nicht mehr und beginnt keinen zweiten Schritt.
//  Zeit: gesteuerte Uhr, kein abschließendes `interrupt()`, keine begrenzten Wartezeiten für negative Nachweise.

/** „Wiederholen“ als Vorlage mit zwei Stücken: „Ich wiederhole.“ klingt, die Synthese der älteren Ausgabe hängt. */
async function wiederholenHaengt({
  optionen,
  handlers,
  haengt = ["Eine ältere Ausgabe."],
} = {}) {
  const r = await gewoehnlich({
    frage: WIEDERHOLEN,
    ohneDenker: true,
    haengt,
    handlers: {
      assistent_einstufen: einstufungWiederholen,
      assistent_letzte_ausgabe: () => "Eine ältere Ausgabe.",
      ...handlers,
    },
    optionen,
    vorbereiten: (t) => (t.engine.ausgabeKey = { id: "c1" }),
  });
  assert.equal(
    await bis(() => r.t.player.gespielt.includes("wav:Ich wiederhole.")),
    true,
  );
  return r;
}

/** Weitersprechen des Sprechers klingt, die Synthese des ersten Inhaltsstücks des Denkers hängt. */
async function weiterHaengt({ optionen, handlers } = {}) {
  const r = await gewoehnlich({
    sprecher: lokalFolge([WEITER_ZEILE, ZWEITE_ZEILE]),
    haengt: ["Satz eins."],
    handlers,
    optionen,
  });
  assert.equal(
    await bis(() => r.t.player.gespielt.includes(WEITER_SATZ)),
    true,
  );
  r.d.liefere(0, "Satz eins.");
  assert.equal(
    await bis(() => r.t.log.zeilen.at(-1)?.t === `${WEITER_TEXT} Satz eins.`),
    true,
    "beides ist angezeigt, nur das Weitersprechen ist zu hören",
  );
  return r;
}
const WEITER_TEXT = "Gute Frage, ich schaue nach.";

/** Die Zeilen des echten Speichers als (Rolle, Text, endgültig, unterbrochen). */
const speicherZeilen = (store) =>
  store
    .getState()
    .lines.map((l) => [l.role, l.text, !!l.final, !!l.interrupted]);

const assistentZeilen = (store) =>
  speicherZeilen(store).filter(([rolle]) => rolle === "assistant");

for (const echt of [false, true]) {
  const art = echt ? "echter Speicher" : "Protokoll";
  const aufbauen = async (bau) => {
    if (!echt) return { ...(await bau({})), store: null, gespeichert: [] };
    const { store, engine, gespeichert, handlers } = await mitEchtemSpeicher();
    return {
      ...(await bau({ optionen: { engine }, handlers })),
      store,
      gespeichert,
    };
  };

  test(`H10 Wiederholen mit hängendem zweiten Stück (${art}): nach dem Ablauf steht nur das Gehörte als unterbrochene Zeile, der ungesprochene Text ist weg`, () =>
    mitUhr(async (tick) => {
      const { d, t, frei, store, gespeichert } =
        await aufbauen(wiederholenHaengt);
      assert.deepEqual(t.log.zeilen.at(-1), {
        t: "Ich wiederhole. Eine ältere Ausgabe.",
        final: false,
        interrupted: false,
        auftragVon: undefined,
      });
      t.player.finishPiece();
      tick(TUNING.stueckMaxMs - 1);
      assert.ok(t.engine.turn, "noch geduldig");
      tick(1);
      assert.equal(t.engine.turn, null, "Schritt frei");
      assert.equal(t.engine.state, "listening");
      assert.deepEqual(t.log.zeilen.at(-1), {
        t: "Ich wiederhole.",
        final: true,
        interrupted: true,
        auftragVon: undefined,
      });
      assert.deepEqual(t.verlauf(), [], "eine Vorlage hat keinen Verlauf");
      await spaeteSynthese(t, frei, "Eine ältere Ausgabe.");
      if (!echt) return;
      assert.deepEqual(assistentZeilen(store), [
        ["assistant", "Ich wiederhole.", true, true],
      ]);
      // Beim Beginn des nächsten Schritts bleibt die gekürzte Zeile, nichts Ungesprochenes taucht auf.
      t.aeussere(WETTER, { mitBarge: false });
      assert.equal(await bis(() => d.laeufe.length === 1), true);
      d.liefere(0, "Satz drei.");
      assert.equal(
        await bis(() => t.player.gespielt.includes("wav:Satz drei.")),
        true,
      );
      assert.deepEqual(
        speicherZeilen(store)
          .slice(0, 3)
          .map(([rolle, text]) => [rolle, text]),
        [
          ["user", WIEDERHOLEN],
          ["assistant", "Ich wiederhole."],
          ["user", WETTER],
        ],
      );
      assert.deepEqual(assistentZeilen(store).slice(0, 1), [
        ["assistant", "Ich wiederhole.", true, true],
      ]);
      assert.equal(
        store.getState().lines.some((l) => l.text.includes("ältere")),
        false,
      );
      await settle(80);
      assert.equal(
        gespeichert.some((e) => e.text.includes("ältere")),
        false,
        "nichts Ungesprochenes gespeichert",
      );
    }));

  test(`H10 Wiederholen ganz eingereiht, der Nutzer unterbricht beim ersten Stück (${art}): die schon endgültige Zeile ist auf das Gehörte gekürzt`, () =>
    mitUhr(async () => {
      const { d, t, store } = await aufbauen((o) =>
        wiederholenHaengt({ ...o, haengt: [] }),
      );
      assert.equal(
        await bis(() => t.engine.turn?.finalSent === true),
        true,
        "die Zeile ist endgültig, das zweite Stück wartet im Player",
      );
      t.aeussere(WETTER);
      assert.equal(await bis(() => d.laeufe.length === 1), true);
      assert.deepEqual(
        t.log.zeilen.filter((z) => z.interrupted),
        [
          {
            t: "Ich wiederhole.",
            final: true,
            interrupted: true,
            auftragVon: undefined,
          },
        ],
      );
      assert.equal(t.log.errors.length, 0);
      if (!echt) return;
      assert.deepEqual(assistentZeilen(store).slice(0, 1), [
        ["assistant", "Ich wiederhole.", true, true],
      ]);
      assert.equal(
        store.getState().lines.some((l) => l.text.includes("ältere")),
        false,
      );
    }));

  test(`H10 Weitersprechen gehört, die erste Inhaltssynthese hängt (${art}): nach dem Ablauf steht nur das Weitersprechen als unterbrochene Zeile`, () =>
    mitUhr(async (tick) => {
      const { d, t, frei, store, gespeichert } = await aufbauen(weiterHaengt);
      t.player.finishPiece();
      tick(TUNING.stueckMaxMs - 1);
      assert.ok(t.engine.turn, "noch geduldig");
      tick(1);
      assert.equal(t.engine.turn, null, "Schritt frei");
      assert.equal(t.engine.state, "listening");
      assert.deepEqual(t.log.zeilen.at(-1), {
        t: WEITER_TEXT,
        final: true,
        interrupted: true,
        auftragVon: undefined,
      });
      assert.equal(
        t.abgebrochen(d.laeufe[0].requestId),
        1,
        "Denker abgebrochen",
      );
      await spaeteSynthese(t, frei, "Satz eins.");
      if (!echt) return;
      assert.deepEqual(assistentZeilen(store), [
        ["assistant", WEITER_TEXT, true, true],
      ]);
      await settle(80);
      assert.equal(
        gespeichert.some((e) => e.text.includes("Satz eins")),
        false,
        "nichts Ungesprochenes gespeichert",
      );
    }));
}

test("H10 nichts gehört (der Ton hat nie begonnen): bricht der Nutzer ab, verschwindet die angezeigte, noch offene Zeile", () =>
  mitUhr(async () => {
    const { d, t } = await gewoehnlich({ haengt: ["Satz zwei."] });
    t.player.deferStart = true;
    d.liefere(0, "Satz eins.", "Satz zwei.");
    assert.equal(
      await bis(() => t.player.enqueued.includes("wav:Satz eins.")),
      true,
    );
    assert.equal(t.player.gespielt.length, 0, "es war nichts zu hören");
    assert.equal(t.log.zeilen.at(-1)?.final, false, "die Zeile ist offen");
    const verworfen = t.log.verworfen;
    t.aeussere(WETTER);
    assert.equal(
      await bis(() => d.laeufe.length === 2),
      true,
      "der neue Schritt beginnt",
    );
    assert.equal(t.log.verworfen, verworfen + 1, "die offene Zeile ist zurück");
    assert.equal(
      t.log.zeilen.some((z) => z.interrupted),
      false,
    );
  }));

/** Erkennung, die erst mit `frei()` antwortet, und ein Schritt, dessen Wartegrenze der Test ablaufen lässt. */
async function spaeteErkennung(text, { stopp = false } = {}) {
  const { d, t, frei } = await gewoehnlich({ haengt: ["Satz zwei."] });
  d.liefere(0, "Satz eins.", "Satz zwei.");
  assert.equal(await ersterKlingt(t), true);
  t.player.finishPiece();
  const erkannt = erkennungMitTor(text);
  t.barge();
  t.fruehEnde();
  t.ende();
  assert.equal(t.engine.held, t.engine.turn, "angehalten, die Erkennung läuft");
  return { d, t, frei, erkannt, stopp };
}

/** Haltefrist der Erkennung, danach die Wartegrenze des Schritts: der Schritt ist zu Ende, die Erkennung läuft noch. */
async function bisZumAblauf(t, tick) {
  tick(TUNING.holdSttMaxMs);
  await new Promise((r) => setImmediate(r));
  assert.equal(t.engine.held, null, "die Haltefrist hat die Pause beendet");
  assert.ok(t.engine.turn, "der Schritt wartet auf sein Stück");
  tick(TUNING.stueckMaxMs);
  assert.equal(t.engine.turn, null, "Wartegrenze: der Schritt ist zu Ende");
}

test("H10 Haltefrist der Erkennung, danach Wartegrenze, dann kommt die Erkennung: die Äußerung geht nicht verloren, genau ein neuer Schritt mit unverändertem Text", () =>
  mitUhr(async (tick) => {
    const { d, t, erkannt } = await spaeteErkennung(WETTER);
    await bisZumAblauf(t, tick);
    erkannt();
    assert.equal(
      await bis(() => d.laeufe.length === 2),
      true,
      "ein neuer Schritt",
    );
    assert.deepEqual(t.log.user, [FRAGE, WETTER]);
    d.liefere(1, "Satz drei.");
    assert.equal(
      await bis(() => t.player.gespielt.includes("wav:Satz drei.")),
      true,
    );
    assert.equal(await bis(() => t.engine.turn?.done), true);
    t.player.finishPiece();
    assert.equal(t.engine.turn, null);
    assert.deepEqual(t.verlauf(), [
      `user:${FRAGE}`,
      "assistant:Satz eins. …",
      `user:${WETTER}`,
      "assistant:Satz drei.",
    ]);
    assert.equal(d.laeufe.length, 2, "kein weiterer Schritt");
    assert.equal(t.log.errors.length, 0);
  }));

test("H10 späte Erkennung nach dem Ablauf ist ein Stoppwort: kein neuer Schritt", () =>
  mitUhr(async (tick) => {
    const { d, t, erkannt } = await spaeteErkennung("Stopp.");
    await bisZumAblauf(t, tick);
    erkannt();
    await new Promise((r) => setImmediate(r));
    await new Promise((r) => setImmediate(r));
    assert.equal(d.laeufe.length, 1, "nichts ging ans Modell");
    assert.deepEqual(t.log.user, [FRAGE]);
    assert.equal(t.engine.turn, null);
    assert.equal(t.engine.state, "listening");
  }));

test("H10 zwischen Ablauf und später Erkennung beginnt ein anderer Schritt (getippter Text): die späte Erkennung gilt nicht mehr", () =>
  mitUhr(async (tick) => {
    const { d, t, erkannt } = await spaeteErkennung(WETTER);
    await bisZumAblauf(t, tick);
    void t.engine.sendText(FOLGE);
    assert.equal(
      await bis(() => d.laeufe.length === 2),
      true,
      "der getippte Text beginnt einen Schritt",
    );
    const turn = t.engine.turn;
    erkannt();
    await new Promise((r) => setImmediate(r));
    await new Promise((r) => setImmediate(r));
    assert.equal(d.laeufe.length, 2, "die späte Erkennung beginnt keinen");
    assert.equal(t.engine.turn, turn);
    assert.equal(turn.cancelled, false);
    assert.deepEqual(t.log.user, [FRAGE, FOLGE]);
  }));

test("H10 Auftragskandidat: die Wartegrenze läuft ab, während die Äußerung erkannt wird: sie geht nicht verloren, genau ein neuer Schritt", () =>
  mitUhr(async (tick) => {
    const { d, t } = await weiterHaengt();
    t.player.finishPiece();
    const erkannt = erkennungMitTor(WETTER);
    t.redeLos();
    t.fruehEnde();
    t.ende();
    assert.equal(
      await bis(() => t.aufrufe("voice_call_transcribe").length === 1),
      true,
      "die Äußerung wird erkannt",
    );
    tick(TUNING.stueckMaxMs);
    assert.equal(t.engine.turn, null, "Wartegrenze: der Schritt ist zu Ende");
    const nutzer = t.log.user.length;
    erkannt();
    assert.equal(
      await bis(() => t.log.user.length === nutzer + 1),
      true,
      "ein neuer Schritt",
    );
    assert.equal(t.log.user.at(-1), WETTER);
    assert.equal(t.log.user.filter((u) => u === WETTER).length, 1);
    assert.equal(t.log.errors.length, 0);
    void d;
  }));

test("H10 nichts angezeigt (der Denker antwortet noch nicht): eine Unterbrechung durch getippten Text nimmt keine Zeile zurück", () =>
  mitUhr(async () => {
    const { d, t } = await gewoehnlich();
    const verworfen = t.log.verworfen;
    void t.engine.sendText(FOLGE);
    assert.equal(
      await bis(() => d.laeufe.length === 2),
      true,
      "der getippte Text beginnt einen Schritt",
    );
    assert.equal(t.log.verworfen, verworfen, "nichts war angezeigt");
    assert.equal(t.abgebrochen(d.laeufe[0].requestId), 1);
  }));

// -- Runde 9 (Codex R2 zu lva#564) --------------------------------------------------------------------------
//
// Fehlerfälle dieser Runde (vor dem Code aufgeschrieben, Status im Bericht):
//  K1 Zeitpunkt der endgültigen Meldung: die Antwort ist ganz synthetisiert und eingereiht, der Start des Players
//     wird zurückgehalten (Vorlage, kein Zusammenführen): endgültig (und gespeichert) wird sie erst, wenn etwas zu
//     hören war. Bricht der Nutzer vorher ab (getippter Text, Stoppwort), bleibt weder Anzeige noch Speicherbeitrag.
//     Beginnt der Ton später, geht die Zeile dann hinaus (nie für immer zurückgehalten).
//  K2 Anrufende (`stop`), Gesprächswechsel (`resetHistory`) und Neustart des Anrufs zwischen Wartegrenze und später
//     Erkennung: die Erkennung wirkt nicht mehr (keine Nutzerzeile, keine Modellanfrage, keine Zustandsänderung, nach
//     `stop` bleibt der Zustand idle); auf dem Weg des Auftragskandidaten ebenso.
//  K3 Weitersprechen gehört, danach Inhalt gehört, das Folgestück hängt: die unterbrochene Zeile zeigt alles Gehörte,
//     nur der Verlauf der KI kennt den Inhalt allein. Anzeige und Speicherbeitrag wie beim regulären Abschluss.
//  Zeit: gesteuerte Uhr, kein abschließendes `interrupt()`, keine begrenzten Wartezeiten für negative Nachweise.

/** Mit dem echten Speicher (`echt`) oder nur dem Protokoll: der Aufbau eines Tests (`bau` bekommt Optionen und Handler). */
async function aufbauenMit(echt, bau) {
  if (!echt) return { ...(await bau({})), store: null, gespeichert: [] };
  const { store, engine, gespeichert, handlers } = await mitEchtemSpeicher();
  return {
    ...(await bau({ optionen: { engine }, handlers })),
    store,
    gespeichert,
  };
}

/** „Wiederholen“ ganz synthetisiert und eingereiht, der Start des Players wird zurückgehalten: nichts war zu hören. */
async function wiederholenOhneTon({ optionen, handlers } = {}) {
  const r = await gewoehnlich({
    frage: WIEDERHOLEN,
    ohneDenker: true,
    haengt: [],
    handlers: {
      assistent_einstufen: einstufungWiederholen,
      assistent_letzte_ausgabe: () => "Eine ältere Ausgabe.",
      ...handlers,
    },
    optionen,
    vorbereiten: (t) => {
      t.engine.ausgabeKey = { id: "c1" };
      t.player.deferStart = true;
    },
  });
  assert.equal(
    await bis(() => r.t.engine.turn?.done === true),
    true,
    "die Antwort ist ganz erzeugt und eingereiht",
  );
  assert.equal(r.t.player.enqueued.length, 2);
  assert.equal(r.t.player.gespielt.length, 0, "es war nichts zu hören");
  return r;
}

for (const echt of [false, true]) {
  const art = echt ? "echter Speicher" : "Protokoll";

  test(`H11 ganz eingereiht, aber nichts zu hören (${art}): die Zeile wird nicht endgültig, und der Abbruch durch getippten Text hinterlässt weder Anzeige noch Speicherbeitrag`, () =>
    mitUhr(async () => {
      const { d, t, store, gespeichert } = await aufbauenMit(
        echt,
        wiederholenOhneTon,
      );
      assert.equal(!!t.engine.turn.finalSent, false, "noch nichts endgültig");
      assert.deepEqual(
        t.log.zeilen.filter((z) => z.final),
        [],
      );
      void t.engine.sendText(FOLGE);
      assert.equal(await bis(() => d.laeufe.length === 1), true);
      assert.deepEqual(
        t.log.zeilen.filter((z) => z.final),
        [],
        "keine endgültige Zeile",
      );
      assert.equal(t.log.verworfen, 1, "die offene Zeile ist zurück");
      if (!echt) return;
      assert.equal(
        store.getState().lines.some((l) => l.text.includes("ältere")),
        false,
        "nichts Ungesprochenes in der Anzeige",
      );
      await settle(80);
      assert.equal(
        gespeichert.some((e) => e.text.includes("ältere")),
        false,
        "nichts Ungesprochenes gespeichert",
      );
    }));

  test(`H11 weiter hat der Ton nicht begonnen (${art}): ein Stoppwort beendet den Schritt ohne Anzeige und ohne Speicherbeitrag`, () =>
    mitUhr(async () => {
      const { t, store, gespeichert } = await aufbauenMit(
        echt,
        wiederholenOhneTon,
      );
      t.aeussere("Stopp.");
      assert.equal(await bis(() => t.engine.turn === null), true);
      assert.deepEqual(
        t.log.zeilen.filter((z) => z.final),
        [],
      );
      if (!echt) return;
      assert.equal(
        store.getState().lines.some((l) => l.text.includes("ältere")),
        false,
      );
      await settle(80);
      assert.equal(
        gespeichert.some((e) => e.text.includes("ältere")),
        false,
      );
    }));
}

test("H11 der zurückgehaltene Start kommt doch: die Zeile geht jetzt endgültig hinaus, der Schritt endet regulär", () =>
  mitUhr(async () => {
    const { t } = await wiederholenOhneTon();
    t.player.deferStart = false;
    t.player.audioStart();
    assert.equal(t.player.gespielt.includes("wav:Ich wiederhole."), true);
    assert.equal(
      await bis(() => t.engine.turn?.finalSent === true),
      true,
      "mit dem ersten Ton geht die Antwort hinaus",
    );
    assert.deepEqual(t.log.zeilen.at(-1), {
      t: "Ich wiederhole. Eine ältere Ausgabe.",
      final: true,
      interrupted: false,
      auftragVon: undefined,
    });
    t.player.finish();
    assert.equal(t.engine.turn, null);
    assert.equal(t.log.zeilen.filter((z) => z.final).length, 1);
    assert.equal(t.log.verworfen, 0);
  }));

/** Weitersprechen und erstes Inhaltsstück klingen, die Synthese des zweiten Inhaltsstücks hängt, der Player ist leer. */
async function weiterInhaltHaengt({ optionen, handlers } = {}) {
  const r = await gewoehnlich({
    sprecher: lokalFolge([WEITER_ZEILE, ZWEITE_ZEILE]),
    haengt: ["Satz zwei."],
    handlers,
    optionen,
  });
  assert.equal(
    await bis(() => r.t.player.gespielt.includes(WEITER_SATZ)),
    true,
  );
  r.d.liefere(0, "Satz eins.", "Satz zwei.");
  r.t.player.finishPiece();
  assert.equal(
    await bis(() => r.t.player.gespielt.includes("wav:Satz eins.")),
    true,
    "das Weitersprechen ist gehört, der erste Inhalt klingt",
  );
  r.t.player.finishPiece();
  return r;
}

for (const echt of [false, true]) {
  const art = echt ? "echter Speicher" : "Protokoll";

  test(`H11 Weitersprechen und erster Inhalt gehört, das Folgestück hängt (${art}): die unterbrochene Zeile zeigt alles Gehörte, der Verlauf der KI nur den Inhalt`, () =>
    mitUhr(async (tick) => {
      const { d, t, frei, store, gespeichert } = await aufbauenMit(
        echt,
        weiterInhaltHaengt,
      );
      tick(TUNING.stueckMaxMs - 1);
      assert.ok(t.engine.turn, "noch geduldig");
      tick(1);
      assert.equal(t.engine.turn, null, "Schritt frei");
      assert.deepEqual(t.log.zeilen.at(-1), {
        t: `${WEITER_TEXT} Satz eins.`,
        final: true,
        interrupted: true,
        auftragVon: undefined,
      });
      assert.deepEqual(t.verlauf(), [
        `user:${FRAGE}`,
        "assistant:Satz eins. …",
      ]);
      assert.equal(t.abgebrochen(d.laeufe[0].requestId), 1);
      await spaeteSynthese(t, frei, "Satz zwei.");
      if (!echt) return;
      assert.deepEqual(assistentZeilen(store), [
        ["assistant", `${WEITER_TEXT} Satz eins.`, true, true],
      ]);
      await settle(80);
      assert.equal(
        gespeichert.filter((e) => e.text === `${WEITER_TEXT} Satz eins.`)
          .length,
        1,
        "ein Speicherbeitrag mit allem Gehörten",
      );
      assert.equal(
        gespeichert.some((e) => e.text.includes("Satz zwei")),
        false,
      );
    }));
}

/**
 * Eine späte Erkennung wirkt nicht: keine Nutzerzeile, kein Modellaufruf, der Zustand bleibt `zustand`, und es gibt
 * keine weitere Fehlermeldung als die `fehler` erwarteten.
 */
async function ohneWirkung(t, d, nutzer, zustand, fehler = 0) {
  await new Promise((r) => setImmediate(r));
  await new Promise((r) => setImmediate(r));
  assert.deepEqual(t.log.user, nutzer, "keine neue Nutzerzeile");
  assert.equal(d.laeufe.length, 1, "keine Modellanfrage");
  assert.equal(t.engine.turn, null);
  assert.equal(t.engine.state, zustand);
  assert.equal(t.log.errors.length, fehler);
}

test("H11 Anrufende zwischen Wartegrenze und später Erkennung: die Erkennung beginnt nichts mehr, der Zustand bleibt idle", () =>
  mitUhr(async (tick) => {
    const { d, t, erkannt } = await spaeteErkennung(WETTER);
    await bisZumAblauf(t, tick);
    await t.engine.stop();
    erkannt();
    await ohneWirkung(t, d, [FRAGE], "idle");
  }));

test("H11 Gesprächswechsel (resetHistory) zwischen Wartegrenze und später Erkennung: die Erkennung beginnt nichts mehr", () =>
  mitUhr(async (tick) => {
    const { d, t, erkannt } = await spaeteErkennung(WETTER);
    await bisZumAblauf(t, tick);
    t.engine.resetHistory();
    erkannt();
    await ohneWirkung(t, d, [FRAGE], "listening");
    assert.deepEqual(t.verlauf(), []);
  }));

test("H11 Neustart des Anrufs zwischen Wartegrenze und später Erkennung: die alte Erkennung beginnt nichts mehr", () =>
  mitUhr(async (tick) => {
    const { d, t, erkannt } = await spaeteErkennung(WETTER);
    await bisZumAblauf(t, tick);
    neustart(t);
    erkannt();
    await ohneWirkung(t, d, [FRAGE], "listening");
  }));

test("H11 Auftragskandidat: Anrufende zwischen Wartegrenze und später Erkennung: die Erkennung beginnt nichts mehr", () =>
  mitUhr(async (tick) => {
    const { d, t } = await weiterHaengt();
    t.player.finishPiece();
    const erkannt = erkennungMitTor(WETTER);
    t.redeLos();
    t.fruehEnde();
    t.ende();
    assert.equal(
      await bis(() => t.aufrufe("voice_call_transcribe").length === 1),
      true,
    );
    tick(TUNING.stueckMaxMs);
    assert.equal(t.engine.turn, null, "Wartegrenze: der Schritt ist zu Ende");
    await t.engine.stop();
    const nutzer = [...t.log.user];
    erkannt();
    await new Promise((r) => setImmediate(r));
    await new Promise((r) => setImmediate(r));
    assert.deepEqual(t.log.user, nutzer);
    assert.equal(d.laeufe.length, 1);
    assert.equal(t.engine.state, "idle");
  }));

// -- Runde 10 (Codex R3 zu lva#564) -------------------------------------------------------------------------
//
// Fehlerfälle dieser Runde (vor dem Code aufgeschrieben, Status im Bericht):
//  L1 Nebenläufigkeit: Der Schritt endet mit einem Fehler (Player lehnt Stück 2 ab, `dropTurn`), während die Erkennung
//     der Unterbrechung noch läuft (Pause weg, Schritt weg, `holdRun` unverändert). Vor Ablauf der Haltefrist kommt
//     ein Gesprächswechsel (`resetHistory`): die Erkennung wirkt nicht mehr.
//  L2 Dasselbe mit Esc (`interrupt`).
//  L3 Dasselbe mit Anrufende (`stop`): danach bleibt der Zustand idle, keine Fehlermeldung, kein Zuhören.
//  L4 Dasselbe mit Neustart des Anrufs.
//  L5 Schutz: Fehlerende ohne Wechsel: die Erkennung gilt weiter, genau ein neuer Schritt (die Äußerung geht nicht
//     verloren, nur die Fehlermeldung des Fehlers selbst).
//  Zeit: gesteuerte Uhr, nie weiter als bis zur Haltefrist, also der Weg ohne Frist (`erstes !== "frist"`).

/**
 * Stück 1 klingt, das Einreihen von Stück 2 hängt, der Nutzer unterbricht (Pause, die Erkennung hängt bis `erkannt()`),
 * dann lehnt der Player Stück 2 ab: der Schritt endet mit Fehler, Pause und Schritt sind frei, die Erkennung läuft noch.
 */
async function fehlerendeBeiErkennung(text) {
  const { d, t } = await gewoehnlich({ sicher: true });
  let ablehnen;
  const einreihen = t.player.enqueue.bind(t.player);
  t.player.enqueue = (wav, ...rest) =>
    wav === "wav:Satz zwei."
      ? new Promise(
          (_, nein) => (ablehnen = () => nein(new Error("dekodieren"))),
        )
      : einreihen(wav, ...rest);
  d.liefere(0, "Satz eins.", "Satz zwei.");
  assert.equal(await ersterKlingt(t), true);
  assert.equal(
    await bis(() => ablehnen !== undefined),
    true,
    "das Einreihen von Stück 2 hängt",
  );
  const erkannt = erkennungMitTor(text);
  t.barge();
  t.fruehEnde();
  t.ende();
  assert.equal(t.engine.held, t.engine.turn, "angehalten, die Erkennung läuft");
  ablehnen();
  assert.equal(
    await bis(() => t.engine.turn === null),
    true,
    "der Fehler beendet den Schritt",
  );
  assert.equal(t.engine.held, null, "die Pause ist weg");
  assert.equal(t.log.errors.length, 1, "genau die Fehlermeldung des Fehlers");
  assert.equal(t.aufrufe("voice_call_transcribe").length, 1);
  return { d, t, erkannt };
}

const WECHSEL = {
  "Gesprächswechsel (resetHistory)": [
    (t) => t.engine.resetHistory(),
    "listening",
  ],
  "Esc (interrupt)": [(t) => t.engine.interrupt(), "listening"],
  "Anrufende (stop)": [(t) => t.engine.stop(), "idle"],
  "Neustart des Anrufs": [(t) => neustart(t), "listening"],
};

for (const [name, [wechsel, zustand]] of Object.entries(WECHSEL))
  test(`H12 Fehlerende des Schritts bei laufender Erkennung, dann ${name} vor der Haltefrist: die Erkennung beginnt nichts mehr`, () =>
    mitUhr(async () => {
      const { d, t, erkannt } = await fehlerendeBeiErkennung(WETTER);
      await wechsel(t);
      erkannt();
      await ohneWirkung(t, d, [FRAGE], zustand, 1);
    }));

test("H12 Fehlerende des Schritts bei laufender Erkennung ohne Wechsel: die Erkennung gilt weiter, genau ein neuer Schritt", () =>
  mitUhr(async () => {
    const { d, t, erkannt } = await fehlerendeBeiErkennung(WETTER);
    erkannt();
    assert.equal(
      await bis(() => d.laeufe.length === 2),
      true,
      "ein neuer Schritt",
    );
    assert.deepEqual(t.log.user, [FRAGE, WETTER]);
    await new Promise((r) => setImmediate(r));
    await new Promise((r) => setImmediate(r));
    assert.equal(d.laeufe.length, 2, "nicht mehr als ein neuer Schritt");
    assert.equal(t.log.errors.length, 1, "nur die Meldung des Fehlers");
    d.liefere(1, "Satz drei.");
    assert.equal(
      await bis(() => t.player.gespielt.includes("wav:Satz drei.")),
      true,
    );
  }));
