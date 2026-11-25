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

async function finishDuringDiscard(source) {
  const track = { file: "synthetic.wav", duration_ms: 1000, transcript_job: "job" };
  const book = { id: "book", title: "Synthetic", authors: [], text: null, audio: { tracks: [track] } };
  const calls = [];
  let generation = 0;
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
    mergeSections: (sections) => sections[0].segments,
    transcriptParagraphs: (segments) => segments,
    textWithSync: () => ({ text: "synthetic", sync: [{ chars: 0, ms: 0 }] }),
    reload: async () => {},
    window: { setTimeout: () => 1, clearTimeout: () => {} },
    commands: {
      libraryTranscriptionStatus: async () => ({ status: "ok", data: { state: "running", generations: [["job", generation]] } }),
      meetingsGetSegments: async () => {
        // Poll war fertig, der Text wird gelesen, danach verwirft der Nutzer.
        generation++;
        return { status: "ok", data: [{ text: "synthetic", start_ms: 0, end_ms: 1000 }] };
      },
      pagesCreate: async () => ({ status: "ok", data: { id: "page" } }),
      pageStateSave: async () => ({ status: "ok" }),
      pagesDelete: async () => calls.push("rollback-page"),
      libraryTranscriptionFinish: async (_book, tokens) => {
        if (tokens[0][1] !== generation)
          return { status: "error", error: "stale_audiobook_completion" };
        calls.push("saved-book");
        return { status: "ok" };
      },
      librarySave: async () => { calls.push("saved-book"); return { status: "ok" }; },
      meetingsDelete: async () => calls.push("deleted-job"),
    },
  };
  runInNewContext(`finish = ${stripTypeScriptTypes(source)}`, context);
  await context.finish("book", new Map());
  return calls;
}

test("discard after reading ready segments prevents transfer and job deletion, with counterprobe", async () => {
  assert.deepEqual(await finishDuringDiscard(finish), ["rollback-page"]);
  const old = finish.replace("commands.libraryTranscriptionFinish(", "commands.librarySave(");
  assert.deepEqual(await finishDuringDiscard(old), ["saved-book", "deleted-job"]);
});
