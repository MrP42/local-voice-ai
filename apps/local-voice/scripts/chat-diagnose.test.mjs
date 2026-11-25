// Fixrunde #831 B: echte Modelladapter, synthetisches IPC und gesteuerte Zeitgeber.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { register } from "node:module";
import test from "node:test";

register("./test-stubs/hooks.mjs", import.meta.url);
globalThis.window = { addEventListener() {}, removeEventListener() {} };
const tauri = (globalThis.__tauri = { calls: [], handlers: {}, listeners: [] });
const { OrchestrationSession, TURN_TIMEOUT_MS } =
  await import("../src/components/voice-call/orchestration.ts?echt");
const projekt = await import("../src/components/voice-call/projekt.ts");

test("Rust-Diagnose übergibt dem Logger nur geschlossene Codes und Zahlen", () => {
  const diagnose = readFileSync(new URL("../src-tauri/src/voice_call/diagnose.rs", import.meta.url), "utf8")
    .split("#[cfg(test)]")[0];
  assert.doesNotMatch(diagnose, /log::(?:info|warn)!/);
  assert.match(diagnose, /protokoll::schreiben\(weg, code, l\.start\.elapsed\(\)\.as_millis\(\)\)/);
  // Jede spätere Erweiterung um einen Inhaltsträger muss diese Grenze erneut prüfen.
  for (const name of ["Code", "Weg"]) {
    const varianten = diagnose.match(new RegExp(`enum ${name} \\{([^}]+)\\}`))[1];
    assert.match(varianten, /^\s*(?:[A-Za-z]+(?:\(u16\))?\s*,?\s*)+$/);
  }
  const protokoll = readFileSync(new URL("../src-tauri/src/voice_call/diagnose/protokoll.rs", import.meta.url), "utf8")
    .split("#[cfg(test)]")[0];
  const funktionen = [...protokoll.matchAll(/fn \w+\(([^)]*)\)/g)];
  assert.equal(funktionen.length, 2);
  for (const [, parameter] of funktionen) {
    assert.equal(parameter, "weg: Weg, code: Code, dauer_ms: u128");
  }
});

function deferred() {
  let resolve;
  const promise = new Promise((r) => (resolve = r));
  return { promise, resolve };
}
const drain = async () => { for (let i = 0; i < 30; i++) await Promise.resolve(); };

async function modell() {
  tauri.calls = [];
  tauri.handlers = {
    get_app_settings: () => ({
      llm_connections: [{ id: "c", kind: "local", enabled: true }],
      llm_models: [{ id: "m", connection_id: "c", remote_id: "m", enabled: true }],
      llm_active_model_id: "m", function_models: {}, voice_call_knowledge: [],
    }),
    compliance_assess_models: () => [],
  };
  const s = new OrchestrationSession({
    subscribe: async () => () => {}, onUsage() {}, onStatus() {},
  });
  // Denker erzwingt den echten Orchestrator; erster Sprecher-Durchgang hängt am IPC.
  return (await s.prepare({
    systemPrompt: "synthetisch", denkerModelId: "m", souveraenitaet: "frei",
    useTools: false, sofort: false,
  })).side;
}

for (const abbruch of [false, true]) {
  test(`IPC fertig bei 19 s: keine Modellfrist in der Verbrauchswartezeit (Abbruch=${abbruch})`, async () => {
    const side = await modell();
    const ipc = deferred();
    const timers = new Map();
    const set = globalThis.setTimeout;
    const clear = globalThis.clearTimeout;
    let next = 0;
    globalThis.setTimeout = (fn, ms) => { timers.set(++next, { fn, ms }); return next; };
    globalThis.clearTimeout = (id) => timers.delete(id);
    tauri.handlers.voice_call_turn = () => ipc.promise;
    try {
      const chat = side.chat("synthetisch-A", [{ role: "user", content: "Frage A" }], () => {});
      await drain();
      assert.ok([...timers.values()].some((t) => t.ms === TURN_TIMEOUT_MS));
      ipc.resolve("Antwort A.");
      await drain();
      if (abbruch) await side.cancel("synthetisch-A");
      // Sekunde 20: nur noch tatsächlich aktive Modellfristen auslösen.
      for (const { fn, ms } of [...timers.values()]) if (ms === TURN_TIMEOUT_MS) fn();
      await drain();
      assert.equal(tauri.calls.filter((c) => c.cmd === "voice_call_frist").length, 0);
      for (const { fn } of [...timers.values()]) fn();
      await chat;
    } finally {
      side.dispose?.();
      globalThis.setTimeout = set;
      globalThis.clearTimeout = clear;
    }
  });
}

test("Erneut bleibt bei Frage A, obwohl B inzwischen die letzte Frage ist", () => {
  const h = { code: "voiceCall.fehler.netz", turnId: "A", conversationId: "c1" };
  const lines = [
    { role: "user", turnId: "A", text: "Frage A" },
    { role: "user", turnId: "B", text: "Frage B" },
  ];
  assert.equal(projekt.wiederholungsFrage(h, "c1", lines), "Frage A");
  assert.equal(projekt.wiederholungsFrage(h, "c2", lines), null);
  assert.equal(projekt.wiederholungsFrage(h, "c1", lines.slice(1)), null);
});

test("Abbruch vor IPC-Ende räumt die Frist ab; ein schon eingeplanter Rückruf meldet nichts", async () => {
  const side = await modell();
  const ipc = deferred();
  const timers = new Map();
  const set = globalThis.setTimeout;
  const clear = globalThis.clearTimeout;
  let next = 0;
  globalThis.setTimeout = (fn, ms) => { timers.set(++next, { fn, ms }); return next; };
  globalThis.clearTimeout = (id) => timers.delete(id);
  tauri.handlers.voice_call_turn = () => ipc.promise;
  try {
    const chat = side.chat("synthetisch-abbruch", [{ role: "user", content: "Frage" }], () => {});
    await drain();
    const frist = [...timers.values()].find((t) => t.ms === TURN_TIMEOUT_MS);
    assert.ok(frist);
    await side.cancel("synthetisch-abbruch");
    assert.equal([...timers.values()].filter((t) => t.ms === TURN_TIMEOUT_MS).length, 0);
    frist.fn();
    await drain();
    assert.equal(tauri.calls.filter((c) => c.cmd === "voice_call_frist").length, 0);
    ipc.resolve("Antwort");
    await drain();
    for (const { fn } of [...timers.values()]) fn();
    await chat;
  } finally {
    side.dispose?.();
    globalThis.setTimeout = set;
    globalThis.clearTimeout = clear;
  }
});

test("Hinweisleiste behält den ersten konkreten Grund desselben Turns", () => {
  for (const code of ["voiceCall.fehler.cli_login", "voiceCall.fehler.zeitueberschreitung", "projekt.zeitueberschreitung"]) {
    const konkret = { code, turnId: "A", conversationId: "c1" };
    assert.deepEqual(projekt.zusammen(konkret, { ...konkret, code: "voiceCall.fehler.unbekannt" }), konkret);
  }
  const erster = { code: "voiceCall.fehler.cli_login", turnId: "A", conversationId: "c1" };
  for (const code of ["voiceCall.fehler.unbekannt", "voiceCall.fehler.netz"]) {
    assert.deepEqual(projekt.zusammen(erster, { ...erster, code }), erster);
  }
  const unbekannt = { ...erster, code: "voiceCall.fehler.unbekannt" };
  assert.deepEqual(projekt.zusammen(unbekannt, erster), erster);
  // Ein neuer Turn oder ein anderes Gespräch darf einen eigenen Fehler zeigen.
  for (const neu of [
    { ...unbekannt, turnId: "B" },
    { ...unbekannt, conversationId: "c2" },
  ]) assert.deepEqual(projekt.zusammen(erster, neu), neu);
  // Ein informativer Projekthinweis blockiert keinen Fehler; Budgetdaten bleiben erhalten.
  assert.deepEqual(projekt.zusammen({ ...erster, code: "projekt.zurueckgesetzt" }, erster), erster);
  const budget = { ...erster, code: "projekt.budget", budget: { fenster: 100 } };
  assert.equal(projekt.zusammen(budget, { ...budget, budget: null }).budget.fenster, 100);
});

test("Hinweisleiste von B wird durch den Hintergrundfehler A nicht ersetzt", () => {
  // Nur den echten Store-Rückruf ausführen: Zustand/React sind hier nicht installiert.
  const quelle = readFileSync(new URL("../src/stores/voiceCallStore.ts", import.meta.url), "utf8");
  const rueckruf = quelle.slice(quelle.indexOf("onProjekt: (h) =>") + "onProjekt: ".length,
    quelle.indexOf("onVerwerfen: () =>")).trim().replace(/,$/, "");
  let state = {
    conversationId: "synthetisch-c1", projektHinweis: null, lines: [
      { id: 1, role: "user", turnId: "A", text: "Frage A", fest: true },
      { id: 2, role: "user", turnId: "B", text: "Frage B" },
    ],
  };
  const melden = new Function("set", "zusammen", `return (${rueckruf});`)(
    (update) => { state = { ...state, ...update(state) }; }, projekt.zusammen,
  );
  melden({ code: "voiceCall.fehler.cli_login", turnId: "B" });
  melden({ code: "voiceCall.fehler.unbekannt", turnId: "B" });
  melden({ code: "voiceCall.fehler.netz", turnId: "A" });
  assert.equal(state.projektHinweis.code, "voiceCall.fehler.cli_login");
  assert.equal(state.projektHinweis.turnId, "B");
  assert.equal(state.projektHinweis.conversationId, "synthetisch-c1");
});
