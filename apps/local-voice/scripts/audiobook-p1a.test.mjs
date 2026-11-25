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
