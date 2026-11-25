// Echter Hook und Recorder-Stopp, nur React/IPC durch deterministische Attrappen ersetzt.
// PowerShell (Repo-Wurzel; nur lesend, Gegenprobe bleibt auch nach einem Commit rot):
// $n = git show 9b291d29:apps/local-voice/src/components/settings/meetings/notes/useNotesAutosave.ts
// $r = git show 9b291d29:apps/local-voice/src/components/settings/meetings/RecorderCard.tsx
// @{notes=$n -join "`n"; recorder=$r -join "`n"} | ConvertTo-Json -Compress |
//   node --test --test-isolation=none apps/local-voice/scripts/notes-autosave.test.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { runInNewContext } from "node:vm";
import test from "node:test";

const require = createRequire(import.meta.url);
const ts = require("typescript");
const hookPath =
  "apps/local-voice/src/components/settings/meetings/notes/useNotesAutosave.ts";
const recorderPath =
  "apps/local-voice/src/components/settings/meetings/RecorderCard.tsx";
const read = (path) =>
  readFileSync(new URL(`../../../${path}`, import.meta.url), "utf8");
// Gegenproben ohne Kindprozess: PowerShell reicht `git show HEAD:...` als JSON auf stdin durch.
const input = process.stdin.isTTY
  ? ""
  : readFileSync(0, "utf8")
      .replace(/^\uFEFF/, "")
      .trim();
const originals = input ? JSON.parse(input) : null;
const baseline = (path) => originals[path === hookPath ? "notes" : "recorder"];
const compile = (source, imports = {}) => {
  const exports = {};
  runInNewContext(
    ts.transpileModule(source, {
      compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2022,
      },
    }).outputText,
    {
      exports,
      require: (id) => imports[id],
      TextEncoder,
      crypto: globalThis.crypto,
    },
  );
  return exports;
};
const logic = compile(read("apps/local-voice/src/lib/meetingNotes.ts"));
const block = (text) => [
  { id: "local", kind: "paragraph", text, checked: false, at_ms: null },
];
const ok = (data) => ({ status: "ok", data });
const tick = async () => {
  for (let i = 0; i < 30; i++) await Promise.resolve();
};
const deferred = () => {
  let resolve;
  const promise = new Promise((r) => {
    resolve = r;
  });
  return { promise, resolve };
};

function harness(source) {
  let active;
  const timers = new Map();
  const delays = new Map();
  let timer = 0;
  let now = 0;
  const react = {
    useRef(value) {
      const h = active;
      const i = h.index++;
      return (h.slots[i] ??= { current: value });
    },
    useState(value) {
      const h = active;
      const i = h.index++;
      if (!(i in h.slots))
        h.slots[i] = typeof value === "function" ? value() : value;
      return [
        h.slots[i],
        (v) => {
          h.slots[i] = typeof v === "function" ? v(h.slots[i]) : v;
        },
      ];
    },
    useCallback(fn, deps) {
      const h = active;
      const i = h.index++;
      const old = h.slots[i];
      if (!old || deps.some((v, n) => v !== old.deps[n]))
        h.slots[i] = { fn, deps };
      return h.slots[i].fn;
    },
    useEffect(fn, deps) {
      const h = active;
      const i = h.index++;
      const old = h.slots[i];
      if (!old || deps.some((v, n) => v !== old.deps[n]))
        h.effects.push(() => {
          old?.cleanup?.();
          h.slots[i] = { deps, cleanup: fn() };
        });
    },
  };
  let remote = { blocks: [], revision: 0 };
  const saves = [];
  const commands = {
    meetingsGet: async (id) => ok({ id }),
    meetingNotesGet: async () => ok(remote),
    meetingNotesSave: async (id, blocks, revision) => {
      saves.push({ id, blocks, revision });
      remote = { blocks, revision: revision + 1 };
      return ok(remote.revision);
    },
  };
  const exports = {};
  runInNewContext(
    ts.transpileModule(
      source + "\nexport const inspectDraft = (id: string) => drafts.get(id);",
      {
        compilerOptions: {
          module: ts.ModuleKind.CommonJS,
          target: ts.ScriptTarget.ES2022,
        },
      },
    ).outputText,
    {
      exports,
      Date: { now: () => now },
      require: (id) =>
        id === "react" ? react : id === "@/bindings" ? { commands } : logic,
      setTimeout: (fn, delay) => {
        delays.set(timer + 1, delay);
        timers.set(++timer, fn);
        return timer;
      },
      clearTimeout: (id) => timers.delete(id),
      document: {
        addEventListener() {},
        removeEventListener() {},
        visibilityState: "visible",
      },
      window: { addEventListener() {}, removeEventListener() {} },
    },
  );
  function mount(id = "m1") {
    const h = { slots: [], index: 0, effects: [] };
    const render = () => {
      active = h;
      h.index = 0;
      h.value = exports.useNotesAutosave(id);
      for (const effect of h.effects.splice(0)) effect();
      return h.value;
    };
    render();
    return {
      render,
      unmount: () => {
        for (const slot of h.slots) slot?.cleanup?.();
      },
    };
  }
  return {
    ...exports,
    commands,
    saves,
    mount,
    timers,
    delays,
    async fireTimer(delay) {
      const entry = [...timers].find(([id]) => delays.get(id) === delay);
      assert.ok(entry, `Timer mit ${delay} ms fehlt`);
      const [id, fn] = entry;
      now += delay;
      timers.delete(id);
      fn();
      await tick();
    },
    setRemote: (v) => {
      remote = v;
    },
  };
}

const contracts = {
  async "Unmount: laufender Fehler bleibt erfasst und Entwurf kommt wieder"(
    source,
  ) {
    const h = harness(source);
    const editor = h.mount();
    await tick();
    editor.render().setBlocks(block("Entwurf"));
    const pending = deferred();
    h.commands.meetingNotesSave = () => pending.promise;
    editor.unmount();
    let finished = false;
    const flush = h.flushAllNotes().then((v) => {
      finished = true;
      return v;
    });
    await tick();
    assert.equal(finished, false);
    pending.resolve({ status: "error", error: "store_failed" });
    assert.equal(await flush, false);
    const reopened = h.mount();
    await tick();
    assert.equal(reopened.render().blocks[0]?.text, "Entwurf");
    assert.equal(await h.flushAllNotes(), false);
  },
  async "Konflikt: Eingaben beim Save, Nachladen und Merge-Save bleiben erhalten"(
    source,
  ) {
    const h = harness(source);
    const editor = h.mount();
    await tick();
    editor.render().setBlocks(block("alt"));
    const first = deferred();
    const load = deferred();
    const second = deferred();
    let count = 0;
    h.commands.meetingNotesSave = async (id, blocks, revision) => {
      h.saves.push({ id, blocks, revision });
      if (++count === 1) return first.promise;
      if (count === 2) return second.promise;
      return ok(revision + 1);
    };
    h.commands.meetingNotesGet = () => load.promise;
    const flush = h.flushAllNotes();
    await tick();
    editor.render().setBlocks(block("beim Save"));
    first.resolve({ status: "error", error: "revision_conflict" });
    await tick();
    editor.render().setBlocks(block("beim Laden"));
    load.resolve(
      ok({ blocks: [{ ...block("fremd")[0], id: "remote" }], revision: 4 }),
    );
    await tick();
    assert.ok(h.saves[1].blocks.some((b) => b.text === "beim Laden"));
    editor
      .render()
      .setBlocks((blocks) => [
        ...blocks,
        { ...block("beim Merge-Save")[0], id: "latest" },
      ]);
    second.resolve(ok(5));
    assert.equal(await flush, true);
    assert.ok(h.saves.at(-1).blocks.some((b) => b.text === "beim Merge-Save"));
    assert.ok(editor.render().blocks.some((b) => b.text === "beim Laden"));
  },
  async "Geworfener Fehler: freigegeben und erneut sicherbar"(source) {
    const h = harness(source);
    const editor = h.mount();
    await tick();
    editor.render().setBlocks(block("Wiederholung"));
    h.commands.meetingNotesSave = async () => {
      throw new Error("synthetisch");
    };
    assert.equal(await h.flushAllNotes(), false);
    h.commands.meetingNotesSave = async () => ok(1);
    assert.equal(await h.flushAllNotes(), true);
  },
  async "Groessengrenze: Unmount behaelt Entwurf, Kuerzen erlaubt Sicherung"(
    source,
  ) {
    const h = harness(source);
    const editor = h.mount();
    await tick();
    editor.render().setBlocks(block("x".repeat(210_000)));
    editor.unmount();
    await tick();
    assert.equal(await h.flushAllNotes(), false);
    assert.equal(h.saves.length, 0);
    const reopened = h.mount();
    await tick();
    assert.equal(reopened.render().blocks[0]?.text.length, 210_000);
    reopened.render().setBlocks(block("kurz"));
    assert.equal(await h.flushAllNotes(), true);
    reopened.unmount();
    await tick();
    assert.equal(h.timers.size, 0);
  },
};

for (const [name, contract] of Object.entries(contracts)) {
  test(name, () => contract(read(hookPath)));
  test(`Gegenprobe Ausgangsstand: ${name}`, { skip: !originals }, async () => {
    await assert.rejects(() => contract(baseline(hookPath)), {
      code: "ERR_ASSERTION",
    });
  });
}

test("Wiedereroeffnen behaelt Basisrevision und fuehrt fremden Stand zusammen", async () => {
  const h = harness(read(hookPath));
  const editor = h.mount();
  await tick();
  editor.render().setBlocks(block("Entwurf"));
  h.commands.meetingNotesSave = async () => ({
    status: "error",
    error: "store_failed",
  });
  editor.unmount();
  await tick();
  const remote = {
    blocks: [{ ...block("fremd")[0], id: "remote" }],
    revision: 7,
  };
  h.setRemote(remote);
  h.commands.meetingNotesSave = async (id, blocks, revision) => {
    h.saves.push({ id, blocks, revision });
    return revision === 7
      ? ok(8)
      : { status: "error", error: "revision_conflict" };
  };
  const reopened = h.mount();
  await tick();
  assert.equal(reopened.render().blocks[0].text, "Entwurf");
  assert.equal(await h.flushAllNotes(), true);
  assert.equal(h.saves[0].revision, 0);
  assert.deepEqual(
    Array.from(h.saves.at(-1).blocks, (b) => b.text),
    ["fremd", "Entwurf"],
  );
});

async function retryAfterUnmount(source) {
  const h = harness(source);
  const editor = h.mount();
  await tick();
  editor.render().setBlocks(block("Entwurf"));
  h.commands.meetingNotesSave = async () => {
    throw new Error("synthetisch");
  };
  editor.unmount();
  await tick();
  assert.equal(h.timers.size, 1);
  let saved = false;
  h.commands.meetingNotesSave = async () => {
    saved = true;
    return ok(1);
  };
  const [id, fn] = h.timers.entries().next().value;
  h.timers.delete(id);
  fn();
  await tick();
  assert.equal(saved, true);
  assert.equal(h.timers.size, 0);
  assert.equal(await h.flushAllNotes(), true);
}
test("Automatische Wiederholung nach Unmount und IPC-Ausnahme", () =>
  retryAfterUnmount(read(hookPath)));
test("Gegenprobe: ohne Wiederholungsplanung bleibt der Fehler liegen", async () => {
  const source = read(hookPath).replace(
    /scheduleSave\(d, (?:5000|d.retryDelayMs)\);/,
    "",
  );
  assert.notEqual(source, read(hookPath));
  await assert.rejects(() => retryAfterUnmount(source), {
    code: "ERR_ASSERTION",
  });
});

test("Mehrere Editoren/Flush-Aufrufe teilen einen Speicherauftrag", async () => {
  const h = harness(read(hookPath));
  const a = h.mount();
  const b = h.mount();
  await tick();
  a.render().setBlocks(block("eins"));
  assert.equal(b.render().blocks[0].text, "eins");
  const pending = deferred();
  let calls = 0;
  h.commands.meetingNotesSave = async () => {
    calls++;
    return pending.promise;
  };
  const runs = [h.flushAllNotes(), a.render().flush(), b.render().flush()];
  await tick();
  assert.equal(calls, 1);
  pending.resolve(ok(1));
  assert.deepEqual(await Promise.all(runs), [true, true, true]);
  a.unmount();
  b.unmount();
  await tick();
  const reopened = h.mount();
  await tick();
  assert.equal(
    reopened.render().blocks.length,
    0,
    "Bestaetigter Zustand wurde freigegeben und erneut geladen",
  );
});

test("Spaete Aenderung einer bereits geflushten Notiz verhindert globales true", async () => {
  const h = harness(read(hookPath));
  const a = h.mount("m1");
  const b = h.mount("m2");
  await tick();
  a.render().setBlocks(block("eins"));
  b.render().setBlocks(block("zwei"));
  const pending = deferred();
  h.commands.meetingNotesSave = async (id) =>
    id === "m2" ? pending.promise : ok(1);
  const run = h.flushAllNotes();
  await tick();
  a.render().setBlocks(block("spaeter"));
  pending.resolve(ok(1));
  assert.equal(await run, false);
});

test("Verwerfen waehrend Speichern verhindert weitere Auftraege und entfernt Entwurf", async () => {
  const h = harness(read(hookPath));
  const editor = h.mount();
  await tick();
  editor.render().setBlocks(block("verwerfen"));
  const pending = deferred();
  let calls = 0;
  h.commands.meetingNotesSave = async () => {
    calls++;
    return pending.promise;
  };
  const run = h.flushAllNotes();
  await tick();
  h.discardNotesDraft("m1");
  pending.resolve({ status: "error", error: "revision_conflict" });
  await run;
  editor.unmount();
  await tick();
  assert.equal(calls, 1);
  assert.equal(h.timers.size, 0);
  assert.equal(await h.flushAllNotes(), true);
  const reopened = h.mount();
  await tick();
  assert.equal(reopened.render().blocks.length, 0);
});

async function deletionContract(source, detail, success) {
  const start = source.indexOf("  const confirmDelete = async () => {");
  const end = source.indexOf("\n  };", start) + 5;
  let discarded = null;
  const context = {
    meetingId: "m1",
    deleteTarget: { id: "m1" },
    commands: {
      meetingsDelete: async () =>
        success ? ok(null) : { status: "error", error: "busy" },
    },
    discardNotesDraft: (id) => {
      discarded = id;
    },
    toast: { error() {} },
    deleteErrorText: (e) => e,
    t: (k) => k,
    setDeleteOpen() {},
    setDeleteTarget() {},
    onDeleted() {},
    setDeleteError() {},
    setItems() {},
    loadPage() {},
    notifyMeetingsChanged() {},
  };
  await runInNewContext(
    `(async () => { ${source.slice(start, end)} await confirmDelete(); })()`,
    context,
  );
  assert.equal(discarded, success ? "m1" : null, detail);
}
for (const component of ["MeetingDetail", "MeetingList"]) {
  const path = `apps/local-voice/src/components/settings/meetings/${component}.tsx`;
  test(`${component}: nur bestaetigtes Loeschen verwirft den Entwurf`, async () => {
    await deletionContract(read(path), component, true);
    await deletionContract(read(path), component, false);
  });
  test(`${component}: Gegenprobe ohne Aufraeumen`, async () => {
    const source = read(path).replace(
      /discardNotesDraft\((meetingId|id)\);/,
      "",
    );
    assert.notEqual(source, read(path));
    await assert.rejects(() => deletionContract(source, component, true), {
      code: "ERR_ASSERTION",
    });
  });
}

async function recorderContract(source, saved, flushAllNotes = async () => saved) {
  const start = source.indexOf("  const stop = async () => {");
  const end = source.indexOf("  const recording =", start);
  assert.ok(start >= 0 && end > start);
  const state = { stopped: false, warning: null, phase: "recording", busy: false };
  const context = {
    flushAllNotes,
    commands: {
      meetingsStop: async () => {
        state.stopped = true;
        return ok("m1");
      },
    },
    t: (key) => key,
    translateMeetingError: (key) => key,
    setError: (v) => {
      state.warning = v;
    },
    setNotesWarning: (v) => {
      state.warning = v;
    },
    setPhase: (v) => {
      state.phase = v;
    },
    NO_HEALTH: {},
  };
  for (const setter of [
    "setStoreNotice",
    "setElapsedMs",
    "setTitle",
    "setEventChoice",
    "setSuggestionCleared",
    "setMicLevel",
    "setSystemLevel",
    "setStartedWithSystem",
    "setHealth",
  ])
    context[setter] = () => {};
  context.setBusy = (v) => {
    state.busy = v;
  };
  await runInNewContext(
    `(async () => { ${source.slice(start, end)} await stop(); })()`,
    context,
  );
  assert.equal(state.stopped, true);
  assert.equal(state.phase, "idle");
  assert.equal(state.busy, false);
  assert.equal(Boolean(state.warning), !saved);
}
test("Recorder stoppt trotz ungesicherter Notizen mit sichtbarem Hinweis", async () => {
  await recorderContract(read(recorderPath), false);
  const source = read(recorderPath);
  assert.match(source, /notesWarning &&/);
  assert.match(source, /t\("meetings\.notes\.notSaved"\)/);
});
test("Recorder bei bestaetigter Sicherung ohne Warnung", () =>
  recorderContract(read(recorderPath), true));
test(
  "Gegenprobe Ausgangsstand: Recorder ignoriert das false",
  { skip: !originals },
  async () => {
    await assert.rejects(
      () => recorderContract(baseline(recorderPath), false),
      {
        code: "ERR_ASSERTION",
      },
    );
  },
);

test("Gegenprobe: gefangener Fehler ohne Freigabe verhindert Wiederholung", async () => {
  const source = read(hookPath).replace(
    "if (d.inflight === run) d.inflight = null;",
    "",
  );
  assert.notEqual(source, read(hookPath));
  await assert.rejects(
    () =>
      contracts["Geworfener Fehler: freigegeben und erneut sicherbar"](source),
    { code: "ERR_ASSERTION" },
  );
});

test("Gegenprobe: bedingungsloses dirty-Loeschen verliert Eingaben beim Merge-Save", async () => {
  const source = read(hookPath).replace(
    "d.dirty = d.blocks !== snapshot;",
    "d.dirty = false;",
  );
  assert.notEqual(source, read(hookPath));
  await assert.rejects(
    () =>
      contracts[
        "Konflikt: Eingaben beim Save, Nachladen und Merge-Save bleiben erhalten"
      ](source),
    { code: "ERR_ASSERTION" },
  );
});

async function deletedRemotely(source) {
  const h = harness(source);
  const editor = h.mount();
  await tick();
  editor.render().setBlocks(block("geloescht"));
  h.commands.meetingNotesSave = async () => ({
    status: "error",
    error: "Meeting m1 not found",
  });
  h.commands.meetingsGet = async () => ok(null);
  assert.equal(await h.flushAllNotes(), false);
  assert.equal(editor.render().blocks[0].text, "geloescht");
  editor.unmount();
  await tick();
  assert.ok(h.inspectDraft("m1"));
  await h.fireTimer(10000);
  assert.equal(h.timers.size, 0);
  const reopened = h.mount();
  await tick();
  assert.equal(reopened.render().blocks.length, 0);
}
test("Save not found behaelt gemounteten Text, ungemountet erst bestaetigt verwerfen", () =>
  deletedRemotely(read(hookPath)));
test("Gegenprobe: geloeschte Besprechung bleibt ohne Aufraeumen registriert", async () => {
  const source = read(hookPath).replace(
    'meeting.status === "ok" && meeting.data === null',
    "false",
  );
  assert.notEqual(source, read(hookPath));
  await assert.rejects(() => deletedRemotely(source), {
    code: "ERR_ASSERTION",
  });
});

test("Groessengrenze gilt auch nach Konflikt-Merge", async () => {
  const h = harness(read(hookPath));
  const editor = h.mount();
  await tick();
  editor.render().setBlocks(block("x".repeat(110_000)));
  h.commands.meetingNotesGet = async () =>
    ok({
      blocks: [{ ...block("y".repeat(110_000))[0], id: "remote" }],
      revision: 1,
    });
  let calls = 0;
  h.commands.meetingNotesSave = async () => {
    calls++;
    return { status: "error", error: "revision_conflict" };
  };
  assert.equal(await h.flushAllNotes(), false);
  assert.equal(calls, 1);
  assert.equal(editor.render().status, "tooLarge");
  editor.unmount();
  await tick();
  const reopened = h.mount();
  await tick();
  assert.equal(reopened.render().blocks.length, 2);
});

test("Neue Meldung in allen Sprachen, de/en mit Sicherungshinweis", () => {
  for (const lang of [
    "ar",
    "bg",
    "cs",
    "da",
    "de",
    "en",
    "es",
    "fr",
    "he",
    "hi",
    "it",
    "ja",
    "ko",
    "ne",
    "nl",
    "pl",
    "pt",
    "ru",
    "sv",
    "tr",
    "uk",
    "vi",
    "zh",
    "zh-TW",
  ]) {
    const data = JSON.parse(
      read(`apps/local-voice/src/i18n/locales/${lang}/translation.json`),
    );
    assert.ok(data.meetings.notes.notSaved);
  }
  assert.match(
    JSON.parse(read("apps/local-voice/src/i18n/locales/de/translation.json"))
      .meetings.notes.notSaved,
    /^Notizen nicht gesichert/,
  );
});

async function deadlineContract(source, stage, timeoutMs = 17) {
  const h = harness(source);
  const pending = deferred();
  let calls = 0;
  if (stage === "Laden") h.commands.meetingNotesGet = () => pending.promise;
  if (stage === "Status") h.commands.meetingsGet = () => pending.promise;
  const editor = h.mount();
  await tick();
  editor.render().setBlocks(block("Frist"));
  h.commands.meetingNotesSave = async () => {
    calls++;
    return stage === "Speichern" ? pending.promise : ok(1);
  };
  if (stage === "Status") {
    editor.render().setBlocks(block("x".repeat(210_000)));
    editor.unmount();
    await tick();
  }
  if (stage === "fremder Flusher")
    h.registerNotesFlusher(() => pending.promise);
  let result;
  const run = h.flushAllNotes(timeoutMs).then((v) => {
    result = v;
  });
  await tick();
  assert.equal(result, undefined);
  await h.fireTimer(timeoutMs);
  assert.equal(result, false, "Gesamtfrist liefert false trotz offenem IPC");
  await run;
  if (stage === "Speichern") {
    editor.render().setBlocks(block("spaeter"));
    const again = editor.render().flush();
    await tick();
    assert.equal(calls, 1, "Fristablauf darf die Schreibsperre nicht loesen");
    pending.resolve(ok(1));
    assert.equal(await again, true);
    assert.equal(calls, 2);
    assert.equal(editor.render().status, "saved");
  } else {
    pending.resolve(
      stage === "Laden"
        ? ok({ blocks: [], revision: 0 })
        : stage === "Status"
          ? ok({ id: "m1" })
          : true,
    );
    await tick();
  }
}

for (const stage of ["Laden", "Status", "Speichern", "fremder Flusher"]) {
  test(`Gesamtfrist bei haengendem ${stage}`, () =>
    deadlineContract(read(hookPath), stage));
  test(`Gegenprobe ohne Frist: ${stage}`, async () => {
    const source = read(hookPath).replace(
      "return await Promise.race([saving, deadline]);",
      "return await saving;",
    );
    assert.notEqual(source, read(hookPath));
    await assert.rejects(() => deadlineContract(source, stage), {
      code: "ERR_ASSERTION",
    });
  });
}

test("Recorder erreicht Stopp nach Standardfrist trotz fremder haengender Notiz", async () => {
  const h = harness(read(hookPath));
  h.registerNotesFlusher(() => new Promise(() => {}));
  let finished = false;
  const run = recorderContract(read(recorderPath), false, h.flushAllNotes).then(
    () => {
      finished = true;
    },
  );
  await tick();
  assert.equal(finished, false);
  await h.fireTimer(3000);
  assert.equal(finished, true);
  await run;
});

async function oversizedDeletion(source) {
  const h = harness(source);
  h.setRemote({ blocks: block("gesicherte Kopie"), revision: 1 });
  const editor = h.mount();
  await tick();
  const retained = h.inspectDraft("m1");
  editor.render().setBlocks(block("x".repeat(210_000)));
  editor.unmount();
  await tick();
  assert.equal(retained.status, "tooLarge");
  h.commands.meetingsGet = async () => ok(null);
  assert.equal(await h.flushAllNotes(), false);
  assert.equal(retained.blocks[0].text.length, 210_000);
  assert.equal(await h.flushAllNotes(), false);
  await h.fireTimer(20000);
  assert.equal(retained.blocks.length, 0);
  assert.equal(retained.savedJson, "[]");
  assert.equal(h.inspectDraft("m1"), undefined);
  assert.equal(h.timers.size, 0);
  assert.equal(h.saves.length, 0);
}
test("Fremde Loeschung verwirft auch zu grossen Entwurf samt gesicherter Kopie", () =>
  oversizedDeletion(read(hookPath)));
test("Gegenprobe ohne Existenzpruefung vor Groessenpruefung", async () => {
  const source = read(hookPath).replace(
    'meeting.status === "ok" && meeting.data === null',
    "false",
  );
  assert.notEqual(source, read(hookPath));
  await assert.rejects(() => oversizedDeletion(source), {
    code: "ERR_ASSERTION",
  });
});
test("Gegenprobe ohne Leeren der gesicherten Inhaltskopie", async () => {
  const source = read(hookPath).replace('d.savedJson = "[]";', "");
  assert.notEqual(source, read(hookPath));
  await assert.rejects(() => oversizedDeletion(source), {
    code: "ERR_ASSERTION",
  });
});

for (const failure of ["Result", "Ausnahme"]) {
  test(`Lesefehler (${failure}) verwirft keinen zu grossen Entwurf`, async () => {
    const h = harness(read(hookPath));
    const editor = h.mount();
    await tick();
    editor.render().setBlocks(block("x".repeat(210_000)));
    h.commands.meetingsGet = async () => {
      if (failure === "Ausnahme") throw new Error("synthetisch");
      return { status: "error", error: "store_failed" };
    };
    editor.unmount();
    await tick();
    assert.equal(await h.flushAllNotes(), false);
    const reopened = h.mount();
    await tick();
    assert.equal(reopened.render().blocks[0].text.length, 210_000);
    assert.ok(h.inspectDraft("m1"));
    h.commands.meetingsGet = async () =>
      ok({ id: "m1", consent_confirmed_at: null });
    reopened.render().setBlocks(block("nur Notizen"));
    assert.equal(await h.flushAllNotes(), true);
    assert.equal(h.saves.at(-1).blocks[0].text, "nur Notizen");
  });
}

async function backoffContract(source, oversized = false) {
  const h = harness(source);
  const editor = h.mount();
  await tick();
  editor.render().setBlocks(
    block(oversized ? "x".repeat(210_000) : "Wiederholung"),
  );
  h.commands.meetingNotesSave = async () => ({
    status: "error",
    error: "store_failed",
  });
  editor.unmount();
  await tick();
  for (const delay of [5000, 10000, 20000, 40000, 60000, 60000]) {
    assert.equal(h.timers.size, 1);
    await h.fireTimer(delay);
  }
  h.commands.meetingsGet = async () => ok(null);
  await h.fireTimer(60000);
  assert.ok(h.inspectDraft("m1"));
  await h.fireTimer(60000);
  assert.equal(h.timers.size, 0);
  assert.equal(await h.flushAllNotes(), true);
  assert.equal(h.inspectDraft("m1"), undefined);
}
for (const oversized of [false, true]) {
  test(`Wiederholung waechst bis 60 s und endet bei Loeschung (zu gross: ${oversized})`, () =>
    backoffContract(read(hookPath), oversized));
}
test("Gegenprobe ohne wachsenden Abstand", async () => {
  const source = read(hookPath).replace(
    "d.retryDelayMs = Math.min(d.retryDelayMs * 2, 60_000);",
    "",
  );
  assert.notEqual(source, read(hookPath));
  await assert.rejects(() => backoffContract(source), { code: "ERR_ASSERTION" });
});

async function normalSave(source) {
  const h = harness(source);
  let reads = 0;
  h.commands.meetingsGet = async () => {
    reads++;
    return ok(null);
  };
  const editor = h.mount();
  await tick();
  editor.render().setBlocks(block("live"));
  assert.equal(await h.flushAllNotes(), true);
  assert.equal(h.saves[0]?.blocks[0].text, "live");
  editor.render().setBlocks(block("beim Unmount"));
  editor.unmount();
  await tick();
  assert.equal(h.saves[1]?.blocks[0].text, "beim Unmount");
  assert.equal(reads, 0, "Normalweg darf meetingsGet nicht aufrufen");
}
test("Live-Notizen und normales Unmount speichern ohne Existenzabfrage", () =>
  normalSave(read(hookPath)));

async function mountedRetention(source, oversized) {
  const h = harness(source);
  let reads = 0;
  h.commands.meetingsGet = async () => {
    reads++;
    return ok(null);
  };
  h.commands.meetingNotesSave = async () => ({
    status: "error", error: "Meeting m1 not found",
  });
  const editor = h.mount();
  await tick();
  const text = oversized ? "x".repeat(210_000) : "Entwurf";
  editor.render().setBlocks(block(text));
  assert.equal(await h.flushAllNotes(), false);
  await h.fireTimer(5000);
  await h.fireTimer(10000);
  assert.equal(editor.render().blocks[0]?.text, text);
  assert.equal(reads, 0);
}
for (const oversized of [false, true]) {
  test(`Gemounteter Editor behaelt Text ohne Existenzabfrage (zu gross: ${oversized})`, () =>
    mountedRetention(read(hookPath), oversized));
}

async function consecutiveChecks(source, interruption) {
  const h = harness(source);
  const editor = h.mount();
  await tick();
  editor.render().setBlocks(block("x".repeat(210_000)));
  let reads = 0;
  h.commands.meetingsGet = async () => {
    reads++;
    return ok(null);
  };
  editor.unmount();
  await tick();
  assert.ok(h.inspectDraft("m1"), "Ein null darf nicht verwerfen");
  await h.flushAllNotes();
  assert.equal(reads, 1, "Flush vor Wiederholungsfrist darf nicht erneut pruefen");
  h.commands.meetingsGet = async () => {
    if (interruption === "Ausnahme") throw new Error("synthetisch");
    return interruption === "vorhanden"
      ? ok({ id: "m1" }) : { status: "error", error: "store_failed" };
  };
  await h.fireTimer(10000);
  h.commands.meetingsGet = async () => ok(null);
  await h.fireTimer(20000);
  assert.ok(h.inspectDraft("m1"), "Unterbrechung setzt Null-Bestaetigung zurueck");
  await h.fireTimer(40000);
  assert.equal(h.inspectDraft("m1"), undefined);
}
for (const interruption of ["vorhanden", "Fehler", "Ausnahme"]) {
  test(`Nur aufeinanderfolgende Null-Pruefungen verwerfen: ${interruption}`, () =>
    consecutiveChecks(read(hookPath), interruption));
}

async function reopenDuringCheck(source) {
  const h = harness(source);
  const editor = h.mount();
  await tick();
  editor.render().setBlocks(block("x".repeat(210_000)));
  h.commands.meetingsGet = async () => ok(null);
  editor.unmount();
  await tick();
  const pending = deferred();
  h.commands.meetingsGet = () => pending.promise;
  await h.fireTimer(5000);
  const reopened = h.mount();
  await tick();
  pending.resolve(ok(null));
  await tick();
  assert.equal(reopened.render().blocks[0]?.text.length, 210_000);
  assert.equal(await h.flushAllNotes(), false);
}
test("Wiedereroeffnen waehrend zweiter Existenzabfrage behaelt Text", () =>
  reopenDuringCheck(read(hookPath)));

async function failedSaveCheck(source) {
  const h = harness(source);
  let reads = 0;
  let saves = 0;
  h.commands.meetingsGet = async () => {
    reads++;
    return ok(null);
  };
  h.commands.meetingNotesSave = async () => {
    saves++;
    return { status: "error", error: "store_failed" };
  };
  const editor = h.mount();
  await tick();
  editor.render().setBlocks(block("fehlgeschlagene Sicherung"));
  editor.unmount();
  await tick();
  assert.equal(saves, 1);
  assert.equal(reads, 0, "Erst speichern, dann Existenz nach Fehler pruefen");
  await h.fireTimer(5000);
  assert.equal(reads, 1);
  assert.ok(h.inspectDraft("m1"));
  await h.fireTimer(10000);
  assert.equal(reads, 2);
  assert.equal(h.inspectDraft("m1"), undefined);
}
test("Ungemountet beginnt Existenzpruefung erst nach fehlgeschlagenem Save", () =>
  failedSaveCheck(read(hookPath)));

for (const [name, from, to, contract] of [
  ["Normalweg mit Existenzabfrage", "(notesTooLarge(d.blocks) || d.status === \"error\")", "true", normalSave],
  ["gemountete Entwuerfe pruefen", "d.dirty && !d.listeners.size", "d.dirty", (s) => mountedRetention(s, true)],
  ["erste Null-Antwort verwirft", "if (checkAfter !== null)", "if (true)", (s) => consecutiveChecks(s, "Fehler")],
  ["ohne Mindestabstand", "(d.missingCheckAfter === null || Date.now() >= d.missingCheckAfter)", "true", (s) => consecutiveChecks(s, "vorhanden")],
  ["Lesefehler behaelt Null-Bestaetigung", "d.missingCheckAfter = null;\n        const meeting", "const meeting", (s) => consecutiveChecks(s, "Fehler")],
  ["Mount nach IPC ignoriert", "if (!d.listeners.size) {", "if (true) {", reopenDuringCheck],
  ["Save-Fehler nicht pruefen", 'd.status === "error"', "false", failedSaveCheck],
]) {
  test(`Gegenprobe: ${name}`, async () => {
    const source = read(hookPath).replace(from, to);
    assert.notEqual(source, read(hookPath));
    await assert.rejects(() => contract(source), { code: "ERR_ASSERTION" });
  });
}
