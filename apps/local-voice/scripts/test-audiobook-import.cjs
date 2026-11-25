// Offline tests of the production import orchestration; no app or browser starts.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { stripTypeScriptTypes } = require("node:module");
const root = path.resolve(__dirname, "..");

async function load(file, imports = {}, mutate = (source) => source) {
  const source = mutate(fs.readFileSync(path.join(root, file), "utf8"));
  const module = new vm.SourceTextModule(stripTypeScriptTypes(source), { identifier: file });
  await module.link((name) => {
    if (!(name in imports)) throw new Error(`Unexpected import: ${name}`);
    const values = imports[name];
    return new vm.SyntheticModule(Object.keys(values), function () {
      for (const [key, value] of Object.entries(values)) this.setExport(key, value);
    });
  });
  await module.evaluate();
  return module.namespace;
}

const sectionsPromise = load("src/lib/library/sections.ts");
const translations = JSON.parse(fs.readFileSync(path.join(root, "src/i18n/locales/de/translation.json"), "utf8"));
const translate = (key) => key.split(".").reduce((value, part) => value?.[part], translations) ?? key;
const errorsPromise = load("src/components/settings/meetings/meetingErrors.ts", {
  "@/lib/meetingSlides": { SLIDE_ERROR_KEYS: {} },
});
async function harness({ sectioned = false, savedRun = true, deleted = false, mixed = false, allMissing = false, beginError = null, partError = null, mutate } = {}) {
  const sections = await sectionsPromise;
  let book = { id: "synthetic_book", title: "Synthetic", text: null,
    audio_paths: ["synthetic_a.wav", "synthetic_b.wav"],
    audio: { tracks: [
      { file: "a.wav", duration_ms: 1000, transcript_job: "old_a" },
      { file: "b.wav", duration_ms: 1000, transcript_job: "old_b" },
    ] },
  };
  if (sectioned) {
    book.audio_paths = ["synthetic_a.wav"];
    book.audio.tracks = [{ file: "a.wav", duration_ms: 100 * 60_000, transcript_job: null }];
  }
  const calls = [];
  let counter = 0;
  const commands = {
    libraryList: async () => ({ status: "ok", data: [structuredClone(book)] }),
    librarySave: async (next) => { book = next; return { status: "ok", data: next }; },
    meetingsBeginAudiobookRun: async (id, legacyJobIds) => {
      calls.push(["begin", id, legacyJobIds]);
      return beginError ? { status: "error", error: beginError } : {
        status: "ok", data: { id: "new_run", model_fallback_reason: null },
      };
    },
    meetingsImportAudiobookPart: async (...args) => {
      calls.push(["part", ...args]);
      if (partError) return { status: "error", error: partError };
      return { status: "ok", data: `new_${++counter}` };
    },
    meetingsGetAudiobookRunJobInfo: async (ids) => ({ status: "ok", data: savedRun ? [
      ...(!deleted ? [{ meeting_id: "old_a", run_id: "frozen_run_a" }] : []),
      ...(!mixed ? [{ meeting_id: "old_b", run_id: "frozen_run_b" }] : []),
    ].filter((job) => ids.includes(job.meeting_id)) : [] }),
    // Ersatzpfad: fehlende Auftraege; failed/cancelled setzt P1a am Cursor fort.
    meetingsGetStatus: async (id) => ({ status: "ok", data: id === "old_a" ? (deleted ? null : "missing") : allMissing ? "missing" : "processing" }),
    meetingsDelete: async (id) => { calls.push(["delete", id]); return { status: "ok", data: null }; },
  };
  const books = {
    extensionOf: () => "wav", fileNameOf: (p) => p, guessFromFileName: () => ({}),
    guessFromText: () => ({}), kindOfFile: () => "audio",
    tracksOf: (audio) => audio.tracks,
    pendingJobs: (b) => b.audio.tracks.flatMap((track) => sections.jobsOfTrack(track).map((j) => j.job)),
    assetUrl: () => null,
  };
  const imported = await load("src/lib/library/importFile.ts", {
    "@/bindings": { commands }, "@/i18n": { default: { exists: (key) => translate(key) !== key, t: translate } },
    "@/components/settings/meetings/meetingErrors": await errorsPromise,
    "@/components/library/MetadataPicker": { patchFrom: () => ({}) }, "./books": books,
    "./progress": { removeProgress: () => {}, textProgressKey: () => "" },
    "./editions": { primaryAudioKey: () => "", langName: (code) => code },
    "./probe": { probeDuration: async () => null }, "../reader/language": { detectLanguage: () => null }, "./sections": sections,
  }, mutate);
  return { imported, calls, book, clearJobs: () => book.audio.tracks.forEach((track) => { track.transcript_job = null; }) };
}

async function sharedRunScenario(mutate) {
  const h = await harness({ sectioned: true, mutate });
  await h.imported.transcribeTracks("synthetic_book");
  assert.equal(h.calls.filter(([kind]) => kind === "begin").length, 1);
  const parts = h.calls.filter(([kind]) => kind === "part");
  assert.equal(parts.length, 4);
  assert.deepEqual(parts.map((call) => call[4]), Array(4).fill("new_run"));
}
test("all sections of one start share one frozen run", () => sharedRunScenario());
test("all tracks of one start share one frozen run", async () => {
  const h = await harness(); h.clearJobs();
  await h.imported.transcribeTracks("synthetic_book");
  assert.equal(h.calls.filter(([kind]) => kind === "begin").length, 1);
  assert.deepEqual(h.calls.filter(([kind]) => kind === "part").map((call) => call[4]), ["new_run", "new_run"]);
});
test("missing explicit model fails visibly before any enqueue", async () => {
  const h = await harness({ sectioned: true, beginError: "audiobook_model_not_installed" });
  const warnings = [];
  await h.imported.transcribeTracks("synthetic_book", (warning) => warnings.push(warning));
  assert.deepEqual(warnings, [translate("audiobook.errors.audiobook_model_not_installed")]);
  assert.equal(h.calls.filter(([kind]) => kind === "part").length, 0);
});
async function resumeScenario(mutate) {
  const h = await harness({ mutate });
  await h.imported.resumeTranscription("synthetic_book");
  assert.deepEqual(h.calls.filter(([kind]) => kind === "part"), [["part", "synthetic_a.wav", null, null, "frozen_run_a"]]);
  assert.deepEqual(h.calls.filter(([kind]) => kind === "delete"), [["delete", "old_a"]]);
  assert.equal(h.calls.filter(([kind]) => kind === "begin").length, 0);
}
test("replacement keeps its saved run; active parts remain untouched", () => resumeScenario());
async function legacyResumeScenario(sectioned, mutate) {
  const h = await harness({ savedRun: false, allMissing: true, mutate });
  if (sectioned) {
    h.book.audio_paths = ["synthetic_a.wav"];
    h.book.audio.tracks = [{ file: "a.wav", transcript_jobs: [
      { job: "old_a", start_ms: 0, end_ms: 200, from_ms: 0, to_ms: 150 },
      { job: "old_b", start_ms: 100, end_ms: null, from_ms: 150, to_ms: null },
    ] }];
  }
  await h.imported.resumeTranscription("synthetic_book");
  assert.deepEqual(h.calls.filter(([kind]) => kind === "begin"), [["begin", "synthetic_book", ["old_a", "old_b"]]]);
  const parts = h.calls.filter(([kind]) => kind === "part");
  assert.equal(parts.length, 2);
  assert.deepEqual(parts.map((call) => call[4]), ["new_run", "new_run"]);
  if (sectioned) assert.deepEqual(parts.map((call) => call.slice(2, 4)), [[0, 200], [100, null]]);
  assert.ok(h.calls.findIndex(([kind]) => kind === "begin") < h.calls.findIndex(([kind]) => kind === "part"));
}
for (const sectioned of [false, true]) {
  test(`legacy ${sectioned ? "sections" : "tracks"} prepare original IDs before replacement`, () => legacyResumeScenario(sectioned));
  test(`counterprobe: legacy ${sectioned ? "sections" : "tracks"} without preparation fails`, async () => {
    await assert.rejects(legacyResumeScenario(sectioned, (source) => source.replace(
      "if (unboundIds.length)", "if (false)",
    )), { code: "ERR_ASSERTION" });
  });
}
test("failed legacy context preparation warns without replacing or deleting jobs", async () => {
  const h = await harness({ savedRun: false, beginError: "audiobook_run_missing" });
  const warnings = [];
  await h.imported.resumeTranscription("synthetic_book", (warning) => warnings.push(warning));
  assert.deepEqual(warnings, [translations.audiobook.errors.audiobook_run_missing]);
  assert.deepEqual(h.calls.map(([kind]) => kind), ["begin"]);
});

async function translatedErrorsScenario(mutate = (source) => source) {
  const errors = await load("src/components/settings/meetings/meetingErrors.ts", {
    "@/lib/meetingSlides": { SLIDE_ERROR_KEYS: {} },
  }, mutate);
  for (const [code, expected] of Object.entries(translations.audiobook.errors)) {
    assert.equal(errors.translateMeetingError(`${code}: synthetic`, translate), expected);
  }
  assert.equal(errors.translateMeetingError("synthetic_unknown", translate), "synthetic_unknown");
}
test("shared meeting error translation includes audiobook codes and details", () => translatedErrorsScenario());
test("counterprobe: missing audiobook translations fails shared error assertions", async () => {
  await assert.rejects(translatedErrorsScenario((source) => source.replace(
    '  audiobook_model_not_installed: "audiobook.errors.audiobook_model_not_installed",', "",
  )), { code: "ERR_ASSERTION" });
});

async function resumeErrorScenario(sectioned, mutate) {
  const h = await harness({ partError: "audiobook_model_not_installed: synthetic", mutate });
  if (sectioned) {
    h.book.audio_paths = ["synthetic_a.wav"];
    h.book.audio.tracks = [{ file: "a.wav", transcript_jobs: [
      { job: "old_a", start_ms: 0, end_ms: 200, from_ms: 0, to_ms: 200 },
    ] }];
  }
  const warnings = [];
  await h.imported.resumeTranscription("synthetic_book", (warning) => warnings.push(warning));
  assert.deepEqual(warnings, [translations.audiobook.errors.audiobook_model_not_installed]);
  assert.equal(h.calls.filter(([kind]) => kind === "delete").length, 0);
}
for (const sectioned of [false, true]) {
  test(`resume translates ${sectioned ? "section" : "track"} replacement errors`, () => resumeErrorScenario(sectioned));
  test(`counterprobe: raw ${sectioned ? "section" : "track"} warning fails translation assertion`, async () => {
    await assert.rejects(resumeErrorScenario(sectioned, (source) => source.replace(
      "if (failed) onWarning?.(audiobookError(failed));", "if (failed) onWarning?.(failed);",
    )), { code: "ERR_ASSERTION" });
  });
}
test("counterprobe: dropping the start run fails the section assertion", async () => {
  await assert.rejects(sharedRunScenario((source) => source.replace(
    "startTrackJobs(paths[i], duration, true, runId)", "startTrackJobs(paths[i], duration, true, null)",
  )), { code: "ERR_ASSERTION" });
});
test("counterprobe: dropping saved run fails the replacement assertion", async () => {
  await assert.rejects(resumeScenario((source) => source.replace(
    "runByJob.get(job) ?? missingRun", "null",
  )), { code: "ERR_ASSERTION" });
});

async function deletedScenario(sectioned, mutate) {
  const h = await harness({ deleted: true, mutate });
  if (sectioned) {
    h.book.audio_paths = ["synthetic_a.wav"];
    h.book.audio.tracks = [{ file: "a.wav", transcript_job: null, transcript_jobs: [
      { job: "old_a", start_ms: 0, end_ms: 200, from_ms: 0, to_ms: 150 },
      { job: "old_b", start_ms: 100, end_ms: null, from_ms: 150, to_ms: null },
    ] }];
  }
  await h.imported.resumeTranscription("synthetic_book");
  const parts = h.calls.filter(([kind]) => kind === "part");
  assert.equal(parts.length, 1);
  assert.equal(parts[0][4], "new_run");
  if (sectioned) assert.deepEqual(parts[0].slice(2, 4), [0, 200]);
  assert.deepEqual(h.calls.filter(([kind]) => kind === "begin"), [["begin", "synthetic_book", ["old_a"]]]);
}

for (const sectioned of [false, true]) {
  test(`unbound deleted ${sectioned ? "section" : "track"} prepares its own legacy context`, () => deletedScenario(sectioned));
  test(`counterprobe: unbound deleted ${sectioned ? "section" : "track"} without legacy fallback fails`, async () => {
    await assert.rejects(deletedScenario(sectioned, (source) => source.replaceAll(
      "?? missingRun", "?? null",
    )), { code: "ERR_ASSERTION" });
  });
}

async function mixedResumeScenario(sectioned, mutate) {
  const h = await harness({ mixed: true, allMissing: true, mutate });
  if (sectioned) {
    h.book.audio_paths = ["synthetic_a.wav"];
    h.book.audio.tracks = [{ file: "a.wav", transcript_jobs: [
      { job: "old_a", start_ms: 0, end_ms: 200, from_ms: 0, to_ms: 150 },
      { job: "old_b", start_ms: 100, end_ms: null, from_ms: 150, to_ms: null },
    ] }];
  }
  await h.imported.resumeTranscription("synthetic_book");
  assert.deepEqual(h.calls.filter(([kind]) => kind === "begin"), [["begin", "synthetic_book", ["old_b"]]]);
  assert.deepEqual(h.calls.filter(([kind]) => kind === "part").map((call) => call[4]), ["frozen_run_a", "new_run"]);
  if (sectioned) assert.deepEqual(h.calls.filter(([kind]) => kind === "part").map((call) => call.slice(2, 4)), [[0, 200], [100, null]]);
}

for (const sectioned of [false, true]) {
  test(`mixed ${sectioned ? "sections" : "tracks"} keep explicit A separate from unbound B`, () => mixedResumeScenario(sectioned));
  test(`counterprobe: sibling fallback fails mixed ${sectioned ? "sections" : "tracks"}`, async () => {
    await assert.rejects(mixedResumeScenario(sectioned, (source) => source
      .replace("let missingRun: string | null = null;", "let missingRun: string | null = info.data[0]?.run_id ?? null;")
      .replace("if (unboundIds.length)", "if (!missingRun && unboundIds.length)")), { code: "ERR_ASSERTION" });
  });
}

test("a track without jobs never inherits a bound sibling run", async () => {
  const h = await harness();
  h.book.audio.tracks[1].transcript_job = null;
  await h.imported.resumeTranscription("synthetic_book");
  assert.deepEqual(h.calls.filter(([kind]) => kind === "begin"), [["begin", "synthetic_book", null]]);
  assert.deepEqual(h.calls.filter(([kind]) => kind === "part").map((call) => call[4]), ["frozen_run_a", "new_run"]);
});

async function ownTextScenario(mutate) {
  const h = await harness({ mutate });
  h.clearJobs();
  h.book.text = { page_id: "synthetic_own", origin: "epub" };
  await h.imported.transcribeTracks(h.book.id);
  assert.equal(h.calls.length, 0);
  await h.imported.transcribeTracks(h.book.id, undefined, undefined, true);
  assert.equal(h.calls.filter(([kind]) => kind === "part").length, 2);
  assert.equal(h.book.text.page_id, "synthetic_own");
}

test("own text requires an explicit transcription start", () => ownTextScenario());
test("explicit resume starts missing parts despite own text", async () => {
  const h = await harness();
  h.clearJobs();
  h.book.text = { page_id: "synthetic_own", origin: "document" };
  await h.imported.resumeTranscription(h.book.id);
  assert.equal(h.calls.filter(([kind]) => kind === "part").length, 2);
  assert.equal(h.book.text.page_id, "synthetic_own");
});
test("counterprobe: automatic transcription of own text fails", async () => {
  await assert.rejects(ownTextScenario((source) => source.replace(
    'if (book.text && book.text.origin !== "transcript" && !explicit) return book;', "",
  )), { code: "ERR_ASSERTION" });
});
