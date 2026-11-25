import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { stripTypeScriptTypes } from "node:module";
import { runInNewContext } from "node:vm";
import test from "node:test";

const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");
const imports = read("src/lib/library/importFile.ts");

function functionSource(source, name) {
  const start = source.indexOf(name);
  assert.ok(start >= 0);
  return source.slice(start, source.indexOf("\n}", start) + 2);
}

async function retry(source, status, sectioned = false, resumeError = false) {
  const part = { job: "synthetic-job", start_ms: 0, end_ms: null, from_ms: 0, to_ms: null };
  const track = { file: "synthetic.wav", duration_ms: 1000, transcript_job: sectioned ? null : part.job, transcript_jobs: sectioned ? [part] : [] };
  const book = { id: "synthetic-book", text: null, audio: { tracks: [track] }, audio_paths: ["synthetic.wav"] };
  const calls = [];
  const context = {
    freshBook: async () => book,
    pendingJobs: () => [part.job],
    statusOfJob: async () => status,
    RESTART_STATUSES: new Set(["failed", "cancelled", "missing"]),
    tracksOf: () => [track],
    pathsOf: () => book.audio_paths,
    jobsOfTrack: () => [part],
    isSectioned: () => sectioned,
    durationOfTrack: async () => 1000,
    startTrackJobs: async () => {
      calls.push("replacement");
      return { error: "synthetic replacement refused" };
    },
    commands: {
      meetingsContinue: async (id) => {
        calls.push(["resume", id]);
        return resumeError ? { status: "error", error: "synthetic" } : { status: "ok" };
      },
      meetingsImportAudiobookPart: async () => {
        calls.push("replacement");
        return { status: "error", error: "synthetic replacement refused" };
      },
    },
  };
  runInNewContext(stripTypeScriptTypes(source), context);
  const result = await context.resumeLocked(book.id);
  return { calls, result, book };
}

const resumeSource = functionSource(imports, "async function resumeLocked");
const withoutResume = resumeSource.replace(
  /  \/\/ Bestehende Hoerbuchimporte[\s\S]*?  const tracks = tracksOf/,
  "  const tracks = tracksOf",
);

for (const sectioned of [false, true]) {
  for (const status of ["cancelled", "failed"]) {
    test(`retry keeps the same ${sectioned ? "section" : "track"} after ${status}, with counterprobe`, async () => {
      const actual = await retry(resumeSource, status, sectioned);
      assert.deepEqual(actual.calls, [["resume", "synthetic-job"]]);
      assert.equal(actual.result, actual.book);
      const counter = await retry(withoutResume, status, sectioned);
      assert.ok(counter.calls.includes("replacement"), "removing the fix must restart from zero");
    });
  }
}

test("a rejected resume preserves the existing book and does not replace its result", async () => {
  const actual = await retry(resumeSource, "failed", false, true);
  assert.deepEqual(actual.calls, [["resume", "synthetic-job"]]);
  assert.equal(actual.result, actual.book);
});

test("queued, processing and ready parts are unchanged", async () => {
  for (const status of ["queued", "processing", "ready"]) {
    assert.deepEqual((await retry(resumeSource, status)).calls, []);
  }
});

test("a genuinely missing task still uses the existing replacement path", async () => {
  assert.ok((await retry(resumeSource, null)).calls.includes("replacement"));
});

test("pause labels take precedence over multipart progress, with counterprobe", () => {
  const page = read("src/components/library/LibraryPage.tsx");
  const source = functionSource(page, "function jobBase");
  const context = {};
  const t = (key) => key;
  runInNewContext(stripTypeScriptTypes(source), context);
  const job = { controlState: "paused", status: "processing", total: 4, failed: 0, done: 0, running: 2 };
  assert.equal(context.jobBase(t, job), "library.transcription.paused");
  const counter = {};
  runInNewContext(stripTypeScriptTypes(source.replace(/  if \(job\?\.controlState[\s\S]*?  if \(job &&/, "  if (job &&")), counter);
  assert.equal(counter.jobBase(t, job), "library.job.partsParallel");
});

test("EN and DE contain the same control and tempo keys", () => {
  const en = JSON.parse(read("src/i18n/locales/en/translation.json")).library.transcription;
  const de = JSON.parse(read("src/i18n/locales/de/translation.json")).library.transcription;
  assert.deepEqual(Object.keys(en).sort(), Object.keys(de).sort());
});

const hook = read("src/components/library/useLibrary.ts");
const finish = hook.slice(
  hook.indexOf("async (bookId: string, durations:"),
  hook.indexOf("\n    [reload, giveUp]", hook.indexOf("const finishTranscript")),
).trim().replace(/,$/, "");
const retryAdoption = hook.slice(
  hook.indexOf("async (bookId: string) =>", hook.indexOf("const retryTranscript")),
  hook.indexOf("\n    [reload]", hook.indexOf("const retryTranscript")),
).trim().replace(/,$/, "");

async function finishDuringDiscard(source, { append = false, editAfterWrite = false, editText = false, failFinish = false, discardDuringRead = true, secondAttempt = false, explicitRetry = false, rollbackFailure = false, firstWriteFailure = false } = {}) {
  const track = { file: "synthetic.wav", duration_ms: 1000, transcript_job: "job" };
  const book = { id: "book", title: "Synthetic", authors: [], text: null, audio: { tracks: [track] } };
  const calls = [];
  const messages = [];
  let generation = 0;
  let claimed = false;
  let page = { text: "before", summary: "before-summary" };
  if (append) book.text = { page_id: "page", origin: "transcript" };
  const context = {
    Map, Set, Error,
    Superseded: class extends Error {},
    gaveUpRef: { current: new Set() },
    freshBook: async () => book,
    tracksOf: (audio) => audio.tracks,
    jobsOfTrack: (t) => [{ job: t.transcript_job }],
    pendingJobs: () => ["job"],
    jobKey: (t) => t.transcript_job,
    isSectioned: () => false,
    finishing: { current: new Set() },
    inFlight: { current: new Set() },
    releaseTimers: { current: new Map() },
    failures: { current: new Map() },
    restarted: { current: new Set() },
    setGaveUp: (update) => { context.gaveUpRef.current = update(context.gaveUpRef.current); },
    resumeTranscription: async () => {},
    i18n: { t: (key) => key },
    giveUp: (id, message) => {
      context.gaveUpRef.current.add(id);
      messages.push(message);
    },
    MAX_FAILURES: 3,
    getPageStateStore: () => ({ mutate: async (_id, change) => {
      if (rollbackFailure && calls.includes("write-page")) return false;
      const next = { ...page };
      if (!change(next, JSON.stringify(page))) return false;
      if (firstWriteFailure) {
        firstWriteFailure = false;
        calls.push("failed-write-page");
        return false;
      }
      page = next;
      calls.push("write-page");
      if (editAfterWrite && calls.filter((c) => c === "write-page").length === 1) {
        page.summary = "edited-summary";
        page.einfuegungen = [{ roh: "synthetic-paste" }];
        if (editText) page.text += " edited";
      }
      return true;
    } }),
    mergeSections: (sections) => sections[0].segments,
    transcriptParagraphs: (segments) => segments,
    textWithSync: (_paragraphs, prefix) => ({ text: `${prefix}synthetic`, sync: [{ chars: 0, ms: 0 }] }),
    reload: async () => {},
    window: { setTimeout: () => 1, clearTimeout: () => {} },
    commands: {
      libraryTranscriptionStatus: async () => ({ status: "ok", data: { state: "running", generations: [["job", generation]] } }),
      libraryTranscriptionClaim: async (_book, generations) => {
        if (generations[0][1] !== generation) return { status: "error", error: "stale_audiobook_completion" };
        claimed = true;
        calls.push("claimed");
        return { status: "ok", data: "token" };
      },
      libraryTranscriptionRelease: async () => { claimed = false; calls.push("released"); return { status: "ok" }; },
      meetingsGetSegments: async () => {
        // Poll war fertig, der Text wird gelesen, danach verwirft der Nutzer.
        if (discardDuringRead) generation++;
        return { status: "ok", data: [{ text: "synthetic", start_ms: 0, end_ms: 1000 }] };
      },
      pagesCreate: async () => ({ status: "ok", data: { id: "page" } }),
      pageStateSave: async () => {
        calls.push("write-page");
        if (!claimed) generation++;
        return { status: "ok" };
      },
      pagesDelete: async () => calls.push("rollback-page"),
      libraryTranscriptionFinish: async (_book, token) => {
        if (failFinish || token !== "token" || !claimed)
          return { status: "error", error: failFinish ? "synthetic-save-failure" : "stale_audiobook_completion" };
        calls.push("saved-book");
        return { status: "ok" };
      },
      librarySave: async () => { calls.push("saved-book"); return { status: "ok" }; },
      meetingsDelete: async () => calls.push("deleted-job"),
    },
  };
  runInNewContext(`finish = ${stripTypeScriptTypes(source)}`, context);
  await context.finish("book", new Map());
  const afterFailure = structuredClone(page);
  if (secondAttempt) {
    // Nach Ablauf der Wiederholpause; die Auftragsverknuepfungen bestehen weiter.
    context.finishing.current.clear();
    failFinish = false;
    if (explicitRetry) {
      runInNewContext(`retryAdoption = ${stripTypeScriptTypes(retryAdoption)}`, context);
      await context.retryAdoption("book");
    }
    await context.finish("book", new Map());
  }
  return { calls, page, afterFailure, messages };
}

test("discard after reading ready segments prevents transfer and job deletion, with counterprobe", async () => {
  assert.deepEqual((await finishDuringDiscard(finish)).calls, []);
  const old = finish.replace("commands.libraryTranscriptionFinish(", "commands.librarySave(");
  // Ohne Claim kann das alte Finish trotz des inzwischen verworfenen Texts speichern.
  const withoutClaim = old.replace(/        const claim = await commands.libraryTranscriptionClaim[\s\S]*?        claimToken = claim.data;/, "");
  assert.ok((await finishDuringDiscard(withoutClaim)).calls.includes("saved-book"));
});

test("a valid claim precedes the first page write and is released on failure", async () => {
  const actual = await finishDuringDiscard(finish, { discardDuringRead: false, failFinish: true });
  assert.deepEqual(actual.calls, ["claimed", "write-page", "rollback-page", "released"]);
});

const wholePageRollback = (source) => source
  .replace("const before = state.text;", "const before = JSON.stringify(state);")
  .replace("restore = { pageId, text: before, written: next.text };", "restore = { pageId, text: before, written: JSON.stringify(state) };")
  .replace(
    /            if \(state.text !== back.written\) return false;\n            if \(back.text === undefined\) delete state.text;\n            else state.text = back.text;/,
    "            if (JSON.stringify(state) !== back.written) return false;\n            for (const key of Object.keys(state)) delete state[key];\n            Object.assign(state, JSON.parse(back.text));",
  )
  .replace(/          if \(!restored(?: && !alreadyRestored)?\) \{[\s\S]*?\n          \}/, "");

test("a failed first page write leaves the original text and permits retry, with counterprobe", async () => {
  const options = { append: true, firstWriteFailure: true, discardDuringRead: false, secondAttempt: true };
  const actual = await finishDuringDiscard(finish, options);
  assert.equal(actual.afterFailure.text, "before");
  assert.equal(actual.page.text, "before\n\nsynthetic");
  assert.deepEqual(actual.messages, []);
  assert.deepEqual(actual.calls, ["claimed", "failed-write-page", "released", "claimed", "write-page", "saved-book", "deleted-job", "released"]);
  const withoutCheck = finish.replace(/            if \(state.text === back.text\) \{[\s\S]*?\n            \}\n/, "");
  const counter = await finishDuringDiscard(withoutCheck, options);
  assert.deepEqual(counter.messages, ["library.job.adoptionUnclear"]);
  assert.equal(counter.page.text, "before");
});

test("rollback restores only its text and retry appends once, with counterprobe", async () => {
  const options = { append: true, editAfterWrite: true, failFinish: true, discardDuringRead: false, secondAttempt: true };
  const actual = await finishDuringDiscard(finish, options);
  assert.equal(actual.afterFailure.text, "before");
  assert.equal(actual.page.summary, "edited-summary");
  assert.deepEqual(actual.page.einfuegungen, [{ roh: "synthetic-paste" }]);
  assert.equal(actual.page.text, "before\n\nsynthetic");
  assert.equal(actual.messages.length, 0);
  assert.ok(actual.calls.includes("released"));
  const counter = await finishDuringDiscard(wholePageRollback(finish), options);
  assert.notEqual(counter.afterFailure.text, "before");
  assert.equal(counter.page.text, "before\n\nsynthetic\n\nsynthetic");
});

test("an intervening text edit blocks automatic adoption until explicit retry, with counterprobe", async () => {
  const options = { append: true, editAfterWrite: true, editText: true, failFinish: true, discardDuringRead: false, secondAttempt: true };
  const actual = await finishDuringDiscard(finish, options);
  assert.equal(actual.page.text, "before\n\nsynthetic edited");
  assert.deepEqual(actual.messages, ["library.job.adoptionUnclear"]);
  assert.equal(actual.calls.filter((c) => c === "claimed").length, 1);
  assert.equal(actual.page.summary, "edited-summary");
  const withoutBlock = finish.replace('giveUp(bookId, i18n.t("library.job.adoptionUnclear"));', "");
  const counter = await finishDuringDiscard(withoutBlock, options);
  assert.equal(counter.calls.filter((c) => c === "claimed").length, 2);
  const explicit = await finishDuringDiscard(finish, { ...options, explicitRetry: true });
  assert.equal(explicit.calls.filter((c) => c === "claimed").length, 2);
});

test("a failed rollback also blocks automatic adoption", async () => {
  const actual = await finishDuringDiscard(finish, { append: true, failFinish: true, discardDuringRead: false, secondAttempt: true, rollbackFailure: true });
  assert.deepEqual(actual.messages, ["library.job.adoptionUnclear"]);
  assert.equal(actual.calls.filter((c) => c === "claimed").length, 1);
  assert.ok(actual.calls.includes("released"));
});

test("successful adoption links the book before deleting helper jobs", async () => {
  assert.deepEqual((await finishDuringDiscard(finish, { discardDuringRead: false })).calls,
    ["claimed", "write-page", "saved-book", "deleted-job", "released"]);
});

test("an unchanged page is restored before releasing a failed adoption", async () => {
  const actual = await finishDuringDiscard(finish, { append: true, failFinish: true, discardDuringRead: false });
  assert.equal(actual.page.text, "before");
  assert.deepEqual(actual.calls, ["claimed", "write-page", "write-page", "released"]);
});
