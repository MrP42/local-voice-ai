// lva#556: Ausspracheregeln eines Assistenten in der Sprachausgabe des Gesprächs. Teil 1: `sprechtext` (Marken, Herkunft der
// Stücke, Surrogate). Teil 2: die Engine schickt `teile` an `voice_call_synthesize`, in beiden Aufrufern (Antwort und
// Nachreichen), und nur dann, wenn ein Stück eingesetzt ist; Anzeige, Verlauf und Prompt behalten den Originaltext.
// Lauf: node --test scripts/sprechtext.test.mjs (Node >= 22.6). Gleicher Lader wie assistent-engine.test.mjs.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
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

const BASIS = "../src/components/voice-call/";
const kern = await import(`${BASIS}assistenten/index.ts`);
const editor = await import(`${BASIS}profil/editor.ts`);
const wirkung = await import(`${BASIS}profil/wirkung.ts`);
const { sprechtext, hatEinzelnenSurrogat } = await import(
  `${BASIS}sprechtext.ts`
);
const { speakable, speakableMitHerkunft } = await import(
  `${BASIS}globalContext.ts`
);
const { VoiceCallEngine } = await import(`${BASIS}engine.ts`);
const { DEFAULT_TURN_CONFIG } = await import(`${BASIS}turnTaking.ts`);
const { SpeechPlayer } = await import("./test-stubs/audio.ts");

const regel = (wort, aussprache) => ({ wort, aussprache });
const teil = (text, ersetzt = false) => ({ text, ersetzt });
const zusammen = (teile) => teile.map((t) => t.text).join("");

// ===================================================================== sprechtext (reine Logik)

test("Ohne Regeln ist es genau `speakable`, ohne teile: der Weg wie vor den Ausspracheregeln", () => {
  const stuck = "Das steht in Quelle [Q1] und [Q12] heute.";
  for (const regeln of [undefined, []])
    assert.deepEqual(sprechtext(stuck, regeln), {
      text: speakable(stuck),
      teile: null,
    });
  assert.equal(speakable(stuck), "Das steht in Quelle und heute.");
});

test("Regeln ohne Treffer: kein teile (alter Textpfad), der Text ist `speakable`", () => {
  assert.deepEqual(sprechtext("Hallo [Q1] Welt.", [regel("Wolff", "Wolf")]), {
    text: "Hallo Welt.",
    teile: null,
  });
});

test("Ein Treffer: Stücke mit Herkunft, zusammengesetzt genau der Text", () => {
  const r = sprechtext("Herr Wolff ruft an.", [regel("Wolff", "Wolf")]);
  assert.equal(r.text, "Herr Wolf ruft an.");
  assert.deepEqual(r.teile, [
    teil("Herr "),
    teil("Wolf", true),
    teil(" ruft an."),
  ]);
});

test("Belegmarken fallen aus dem Original, bevor eine Regel den Text sieht; die Herkunft der Überlebenden bleibt", () => {
  const r = sprechtext("Wolff sagt das [Q1] heute, Wolff [Q2] morgen.", [
    regel("Wolff", "Wolf"),
  ]);
  assert.equal(r.text, "Wolf sagt das heute, Wolf morgen.");
  assert.equal(zusammen(r.teile), r.text);
  assert.deepEqual(
    r.teile.filter((t) => t.ersetzt).map((t) => t.text),
    ["Wolf", "Wolf"],
  );
  // Eine Regel auf die Marke selbst findet sie nicht mehr: sie ist schon weg.
  const q = sprechtext("Das [Q1] gilt.", [regel("Q", "Kuh")]);
  assert.deepEqual(q, { text: "Das gilt.", teile: null });
});

test("Eine Marke, die erst durch eine Ersetzung entsteht, wird auf dem zusammengesetzten Text erkannt und entfernt", () => {
  // `[` + `Q1` + `]`: das eingesetzte Stück liegt ganz in der Marke und verschwindet mit ihr.
  const ganz = sprechtext("A [Z] B", [regel("Z", "Q1")]);
  assert.deepEqual(ganz, { text: "A B", teile: null });
  // Ein eingesetztes Stück außerhalb der Marke bleibt mit seiner Herkunft.
  const rest = sprechtext("A [Z] B Z.", [regel("Z", "Q1")]);
  assert.equal(rest.text, "A B Q1.");
  assert.deepEqual(rest.teile, [
    teil("A"),
    teil(" B "),
    teil("Q1", true),
    teil("."),
  ]);
  // Das eingesetzte Stück ist selbst eine Marke: weg, und `teile` entfällt, weil nichts Eingesetztes bleibt.
  assert.deepEqual(sprechtext("Das K ist gut.", [regel("K", "[Q1]")]), {
    text: "Das ist gut.",
    teile: null,
  });
});

test("`[laXhen]`: das eingesetzte `c` macht den Tag erst zusammengesetzt; die Stücke gehen so an den Kern", () => {
  const r = sprechtext("Das ist [laXhen] lustig.", [regel("X", "c")]);
  assert.equal(r.text, "Das ist [lachen] lustig.");
  assert.deepEqual(r.teile, [
    teil("Das ist [la"),
    teil("c", true),
    teil("hen] lustig."),
  ]);
});

test("Eine Ersetzung mit leerer Aussprache bleibt ohne leeres Stück", () => {
  const r = sprechtext("Hallo (leise) Welt", [regel("(leise) ", "")]);
  assert.equal(r.text, "Hallo Welt");
  assert.equal(r.teile, null, "es ist nichts Eingesetztes übrig");
  const s = sprechtext("Wolff (leise) ruft", [
    regel("Wolff", "Wolf"),
    regel("(leise) ", ""),
  ]);
  assert.deepEqual(s.teile, [teil("Wolf", true), teil(" "), teil("ruft")]);
  assert.ok(s.teile.every((t) => t.text !== ""));
});

test("Ein zerschnittenes Surrogatpaar: das Häppchen geht ohne Ersetzung an die Stimme, nie stumm", () => {
  // Eine Regel auf die erste Hälfte von 😀 würde das Zeichen zerschneiden.
  const r = sprechtext("😀 Hallo", [regel("\ud83d", "X")]);
  assert.deepEqual(r, { text: "😀 Hallo", teile: null });
  assert.equal(hatEinzelnenSurrogat("\ud83d"), true);
  assert.equal(hatEinzelnenSurrogat("\ude00"), true);
  assert.equal(hatEinzelnenSurrogat("😀"), false);
  assert.equal(hatEinzelnenSurrogat("a\ude00b"), true);
  assert.equal(hatEinzelnenSurrogat("a\ud83d"), true);
  assert.equal(hatEinzelnenSurrogat("é東京😀"), false);
  // Ganze Zeichen als Wort und als Aussprache sind in Ordnung.
  const ok = sprechtext("😀 Hallo 東京", [
    regel("😀", "Smiley"),
    regel("東京", "Tokio"),
  ]);
  assert.equal(ok.text, "Smiley Hallo Tokio");
  assert.deepEqual(ok.teile, [
    teil("Smiley", true),
    teil(" Hallo "),
    teil("Tokio", true),
  ]);
});

test("speakableMitHerkunft: zusammengesetzt genau `speakable`, die Felder der Stücke bleiben, leere Stücke entfallen", () => {
  const teile = [
    { text: "Hallo [Q", ersetzt: false, x: 1 },
    { text: "1] Welt", ersetzt: true, x: 2 },
    { text: "", ersetzt: true, x: 3 },
    { text: " [Q2]", ersetzt: false, x: 4 },
  ];
  const aus = speakableMitHerkunft(teile);
  assert.equal(zusammen(aus), speakable(zusammen(teile)));
  assert.deepEqual(aus, [
    { text: "Hallo", ersetzt: false, x: 1 },
    { text: " Welt", ersetzt: true, x: 2 },
  ]);
  // Ohne Marke: die Stücke unverändert (nur leere fallen weg).
  assert.deepEqual(
    speakableMitHerkunft([teil("a"), teil("", true), teil("b")]),
    [teil("a"), teil("b")],
  );
});

/** Ein kleiner, fester Zufallsstrom: derselbe Lauf bei jedem Start. */
function strom(start) {
  let s = start >>> 0 || 1;
  return (n) => {
    s ^= s << 13;
    s >>>= 0;
    s ^= s >>> 17;
    s ^= s << 5;
    s >>>= 0;
    return s % n;
  };
}

test("Zufall: Stücke ergeben immer genau den Text, nie ein leeres Stück, Text = Marken weg nach den Regeln", () => {
  const zufall = strom(20260101);
  const brocken = [
    "A",
    "B",
    " ",
    "[",
    "Q",
    "1",
    "]",
    "Z",
    "😀",
    "é",
    ".",
    "\n",
    "Q1",
    "[Q1]",
    "Wolff",
  ];
  const regeln = [
    regel("Z", "Q"),
    regel("B", "["),
    regel("A", "1]"),
    regel("Wolff", "Wolf"),
    regel("é", "e"),
  ];
  let mitTeilen = 0;
  for (let lauf = 0; lauf < 3000; lauf++) {
    let stuck = "";
    for (let i = zufall(14); i > 0; i--)
      stuck += brocken[zufall(brocken.length)];
    const r = sprechtext(stuck, regeln);
    // Ohne eingesetztes Stück ist es genau der alte Weg (`speakable` einmal), sonst die zweite Filterung des Ganzen.
    const zerlegt = kern.zerlegeAussprache(speakable(stuck), regeln);
    const erwartet = zerlegt.some((t) => t.ersetzt)
      ? speakable(zusammen(zerlegt))
      : speakable(stuck);
    assert.equal(r.text, erwartet, JSON.stringify(stuck));
    if (r.teile === null) continue;
    mitTeilen++;
    assert.equal(zusammen(r.teile), r.text, JSON.stringify(stuck));
    assert.ok(
      r.teile.every((t) => t.text !== ""),
      JSON.stringify(stuck),
    );
    assert.ok(
      r.teile.some((t) => t.ersetzt),
      JSON.stringify(stuck),
    );
    assert.ok(r.teile.every((t) => !hatEinzelnenSurrogat(t.text)));
  }
  assert.ok(mitTeilen > 500, "der Lauf trifft genug Fälle mit Ersetzung");
});

// ===================================================================== Engine

const settle = (ms = 5) => new Promise((r) => setTimeout(r, ms));
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
/** Orchestrator-Seite (Buch-Gespräch): ruft `onPiece` mit den Stücken. */
const seite = (...stuecke) => ({
  chat: async (_id, _msgs, onPiece) => {
    for (const s of stuecke) onPiece(s);
    return stuecke.join(" ");
  },
  cancel: async () => undefined,
});

/** `assistent`: gilt ab dem Start (vor dem Beginn eines Gesprächs; danach würde die Wahl erst im nächsten gelten). */
function neu(handlers = {}, { seiteMit, assistent } = {}) {
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
      onDenken() {},
      conversationId: () => "c1",
    },
    { systemPrompt: "sys", turn: DEFAULT_TURN_CONFIG, orchestration: {} },
  );
  if (assistent !== undefined) engine.setAssistent(assistent);
  const player = new SpeechPlayer(null);
  player.onIdle = () => {
    if (engine.turn?.audible && engine.turn.done) engine.finishTurn();
  };
  engine.player = player;
  engine.state = "listening";
  engine.stopped = false;
  const aufrufe = (cmd) => tauri.calls.filter((c) => c.cmd === cmd);
  const start = (text) => {
    const turn = engine.newTurn(0, performance.now());
    engine.turn = turn;
    turn.committed = true;
    turn.commit();
    const done = engine
      .respond(turn, text, 0)
      .catch((e) => log.errors.push(String(e)));
    return { turn, done };
  };
  const synthese = () => aufrufe("voice_call_synthesize").map((c) => c.args);
  return { engine, player, log, start, aufrufe, synthese };
}

const profilMit = (id, regeln) =>
  editor.mitAussprache(kern.neuesProfil(id, `Eigen ${id}`), regeln);
const wirkungMit = (id, regeln) =>
  wirkung.wirkungFuer(profilMit(id, regeln), () => wirkung.werteJetzt(null));

const WOLFF = [regel("Wolff", "Wolf")];

test("Engine mit Regeln: `teile` gehen mit, Anzeige und Verlauf behalten den Originaltext", async () => {
  const t = neu(
    { voice_call_chat: chatSagt("Herr Wolff ruft an.", "Danach ist Ruhe.") },
    { assistent: wirkungMit("eigen-a", WOLFF) },
  );
  const { done } = t.start("Wer ruft an?");
  await done;
  await settle(20);
  const s = t.synthese();
  assert.equal(s.length, 2);
  assert.equal(s[0].text, "Herr Wolf ruft an.");
  assert.deepEqual(s[0].teile, [
    teil("Herr "),
    teil("Wolf", true),
    teil(" ruft an."),
  ]);
  assert.equal(zusammen(s[0].teile), s[0].text);
  assert.ok(s[0].requestId, "die Anfrage-Id geht mit");
  // Ohne Treffer: kein `teile`, nicht einmal der Schlüssel.
  assert.equal(s[1].text, "Danach ist Ruhe.");
  assert.equal(Object.hasOwn(s[1], "teile"), false);
  // Der Originaltext bleibt in Anzeige und Verlauf.
  assert.equal(
    t.log.assistant.at(-1).t,
    "Herr Wolff ruft an. Danach ist Ruhe.",
  );
  assert.deepEqual(
    t.engine.history.map((m) => `${m.role}:${m.content}`),
    ["user:Wer ruft an?", "assistant:Herr Wolff ruft an. Danach ist Ruhe."],
  );
  assert.equal(t.log.errors.length, 0);
});

test("Engine ohne Assistent und mit „Alles“: die Aufrufe sind genau die von früher (kein `teile`)", async () => {
  for (const auswahl of ["keiner", "alles"]) {
    const t = neu({ voice_call_chat: chatSagt("Herr Wolff ruft an. [Q1]") });
    if (auswahl === "alles") t.engine.setAssistent(null);
    await t.start("Wer ruft an?").done;
    await settle(20);
    assert.deepEqual(
      t.synthese().map((a) => Object.keys(a).sort()),
      [["requestId", "text"]],
      auswahl,
    );
    assert.equal(t.synthese()[0].text, "Herr Wolff ruft an.", auswahl);
  }
});

test("Engine: das Buch-Gespräch (eigene Seite) hat keinen wirksamen Assistenten, also keine Ersetzung", async () => {
  const t = neu({}, { assistent: wirkungMit("eigen-a", WOLFF) });
  t.engine.setSideOverride(seite("Herr Wolff ruft an."));
  await t.start("Wer ruft an?").done;
  await settle(20);
  assert.deepEqual(
    t.synthese().map((a) => a.text),
    ["Herr Wolff ruft an."],
  );
  assert.equal(Object.hasOwn(t.synthese()[0], "teile"), false);
});

test("Engine: der Assistent ist zum Beginn des Gesprächs festgelegt, eine spätere Wahl gilt ab dem nächsten Gespräch", async () => {
  const t = neu(
    { voice_call_chat: chatSagt("Herr Wolff ruft an.") },
    { assistent: wirkungMit("eigen-a", [regel("Wolff", "Wolf")]) },
  );
  await t.start("Erste Frage?").done;
  await settle(20);
  t.player.finish?.();
  assert.equal(t.synthese().at(-1).text, "Herr Wolf ruft an.");
  // Das Gespräch hat begonnen: die andere Wahl wartet.
  const sofort = t.engine.setAssistent(
    wirkungMit("eigen-b", [regel("Wolff", "Wulf")]),
  );
  assert.equal(sofort, false);
  await t.start("Zweite Frage?").done;
  await settle(20);
  assert.equal(
    t.synthese().at(-1).text,
    "Herr Wolf ruft an.",
    "noch die Regeln von A",
  );
  // Ein neues Gespräch: jetzt gilt B.
  t.engine.resetHistory();
  await t.start("Dritte Frage?").done;
  await settle(20);
  assert.equal(t.synthese().at(-1).text, "Herr Wulf ruft an.");
  assert.equal(t.log.errors.length, 0);
});

/** Ein Auftrag im Zustand „fertig, wartet auf Nachreichen“, mit genau den Feldern, die `nachreichen` liest. */
const auftrag = (saetze) => ({
  saetze,
  turnId: "auftrag-1",
  frage: "Was ist mit Wolff?",
  turn: { withContext: false, kontext: null },
  timer: undefined,
  verworfen: false,
  nachreichend: false,
  fertig: true,
});

test("Nachreichen (zweiter Aufrufer): dieselben Stücke und `teile`, Anzeige und Verlauf mit dem Originaltext", async () => {
  const t = neu({}, { assistent: wirkungMit("eigen-a", WOLFF) });
  const a = auftrag(["Herr Wolff ruft an.", "Danach ist Ruhe."]);
  t.engine.nachreichen(a);
  await settle(30);
  const s = t.synthese();
  assert.equal(s.length, 2);
  assert.equal(s[0].text, "Herr Wolf ruft an.");
  assert.deepEqual(s[0].teile, [
    teil("Herr "),
    teil("Wolf", true),
    teil(" ruft an."),
  ]);
  assert.equal(s[0].requestId, "auftrag-1");
  assert.equal(s[1].text, "Danach ist Ruhe.");
  assert.equal(Object.hasOwn(s[1], "teile"), false);
  assert.equal(t.log.assistant.at(0).t, "Herr Wolff ruft an.");
  t.player.finish?.();
  await settle(20);
  assert.ok(
    t.engine.history.some(
      (m) =>
        m.role === "assistant" &&
        m.content === "Herr Wolff ruft an. Danach ist Ruhe.",
    ),
    "der Verlauf hat den Originaltext",
  );
  assert.equal(t.log.errors.length, 0);
});

test("Nachreichen ohne Assistent: kein `teile`, der Aufruf wie früher", async () => {
  const t = neu();
  t.engine.nachreichen(auftrag(["Herr Wolff ruft an. [Q1]"]));
  await settle(30);
  assert.deepEqual(
    t.synthese().map((a) => Object.keys(a).sort()),
    [["requestId", "text"]],
  );
  assert.equal(t.synthese()[0].text, "Herr Wolff ruft an.");
});

test("Quelltext: beide Aufrufer von `voice_call_synthesize` gehen über `sprechtextFuer` und reichen `teile` nur mit", () => {
  const quelle = readFileSync(
    new URL(`${BASIS}engine.ts`, import.meta.url),
    "utf8",
  ).replace(/\r\n/g, "\n");
  const aufrufe = quelle
    .split('invoke<string>("voice_call_synthesize", {')
    .slice(1);
  assert.equal(aufrufe.length, 2, "genau zwei Aufrufer");
  for (const rest of aufrufe) {
    const kopf = quelle.slice(0, quelle.indexOf(rest));
    assert.match(kopf.slice(-400), /this\.sprechtextFuer\(piece\)/);
    assert.doesNotMatch(
      kopf.slice(-400),
      /speakable\(piece\)/,
      "kein Aufrufer geht mit `speakable(piece)` am Text vorbei",
    );
    const block = rest.slice(0, rest.indexOf("});"));
    assert.match(block, /text: spoken,/);
    assert.match(block, /requestId: turn\.id,/);
    assert.match(block, /\.\.\.\(teile \? \{ teile \} : \{\}\),/);
  }
});
