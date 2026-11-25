import { readFileSync } from "node:fs";
import { createRequire, stripTypeScriptTypes } from "node:module";
import vm from "node:vm";
import assert from "node:assert/strict";
import test from "node:test";

const require = createRequire(import.meta.url);
test("changed TypeScript and DE/EN controls parse without installed dependencies", () => {
  for (const path of ["../src/lib/library/importFile.ts", "../src/components/library/useLibrary.ts",
    "../src/bindings.ts", "../tests/library.spec.ts"])
    stripTypeScriptTypes(readFileSync(new URL(path, import.meta.url), "utf8"));
  const controls = (lang) => JSON.parse(readFileSync(new URL(`../src/i18n/locales/${lang}/translation.json`, import.meta.url), "utf8")).library.transcriptionControls;
  assert.deepEqual(Object.keys(controls("de")).sort(), Object.keys(controls("en")).sort());
});

const load = (path, dependencies = {}, names = ["resumeTranscription"]) => {
  const exports = {};
  let source = stripTypeScriptTypes(readFileSync(new URL(path, import.meta.url), "utf8"))
    .replace(/import\s+(\{[\s\S]*?\}|\w+)\s+from\s+["']([^"']+)["'];/g,
      (_, names, id) => `const ${names} = require(${JSON.stringify(id)})${names.startsWith("{") ? "" : ".default"};`)
    .replace(/export /g, "");
  if (process.argv.includes("--counterprobe")) source = source
    .replace("// Ein eigener Text bleibt bei der Uebernahme", 'if (book.text && book.text.origin !== "transcript") return book;\n// Ein eigener Text bleibt bei der Uebernahme')
    .replace("const ownText =", 'if (start.text && !appendTo) return;\nconst ownText =')
    .replace("    removeTranscript,", "    removeTranscript: async () => {},");
  vm.runInNewContext(`${source}\n${names.map((name) => `exports.${name} = ${name};`).join("\n")}`,
    { exports, require: (id) => dependencies[id] ?? require(id), Set, Map,
      window: { addEventListener() {}, removeEventListener() {}, setInterval() {}, clearInterval() {}, setTimeout() {}, clearTimeout() {} } });
  return exports;
};

for (const status of ["failed", "cancelled", "missing"]) test(`${status} legacy job with own text can recover`, async () => {
  const book = { id: "synthetic", text: { origin: "document", page_id: "own" },
    audio: { file: "synthetic.wav", duration_ms: 1000, transcript_job: "legacy" }, audio_path: "synthetic.wav" };
  const continued = [];
  const restarted = [];
  const deleted = [];
  const commands = {
    libraryList: async () => ({ status: "ok", data: [book] }),
    meetingsGetAudiobookRunJobInfo: async () => ({ status: "ok", data: [{ meeting_id: "legacy", run_id: "run" }] }),
    meetingsGetStatus: async () => ({ status: "ok", data: status === "missing" ? null : status }),
    meetingsContinue: async (id) => { continued.push(id); return { status: "ok" }; },
    meetingsImportAudiobookPart: async (...args) => { restarted.push(args); return { status: "ok", data: "replacement" }; },
    librarySave: async (data) => ({ status: "ok", data }),
    meetingsDelete: async (id) => { deleted.push(id); return { status: "ok" }; },
  };
  const { resumeTranscription } = load("../src/lib/library/importFile.ts", {
    "@/bindings": { commands }, "@/i18n": { default: { t: (s) => s } },
    "@/components/settings/meetings/meetingErrors": {}, "@/components/library/MetadataPicker": {},
    "./books": { pendingJobs: () => ["legacy"], tracksOf: (a) => [a] },
    "./progress": {}, "./editions": {}, "./probe": {}, "../reader/language": {},
    "./sections": { jobsOfTrack: (t) => [{ job: t.transcript_job }], isSectioned: () => false,
      RESTART_STATUSES: new Set(["failed", "cancelled", "missing"]), planSections: () => [] },
  });
  const saved = await resumeTranscription(book.id);
  assert.deepEqual(continued, status === "missing" ? [] : ["legacy"]);
  assert.equal(restarted.length, status === "missing" ? 1 : 0);
  if (status === "missing") {
    assert.deepEqual(restarted[0], ["synthetic.wav", null, null, "run"]);
    assert.deepEqual(deleted, ["legacy"]);
    assert.equal(saved.audio.transcript_job, "replacement");
  }
  assert.equal(book.text.page_id, "own");
  assert.equal(saved.text.page_id, "own");
});

const hookFixture = (status) => {
  let book = { id: "synthetic", title: "Synthetic", authors: [], language: "de", editions: [],
    text: { page_id: "own", origin: "document" },
    audio: { file: "synthetic.wav", duration_ms: 1000, transcript_job: "legacy", sync: [] } };
  const originalText = book.text;
  const effects = [];
  const removed = [];
  const pages = [];
  const errors = [];
  let states = 0;
  const commands = {
    libraryList: async () => ({ status: "ok", data: [book] }),
    pageFiles: async () => ({ status: "ok", data: [] }),
    meetingsList: async () => ({ status: "ok", data: [{ id: "legacy", status, duration_ms: 1000 }] }),
    meetingsGetStatus: async () => ({ status: "ok", data: status }),
    meetingsGetAudiobookRunJobInfo: async () => ({ status: "ok", data: [] }),
    libraryTranscriptionStatus: async () => ({ status: "ok", data: { state: "running", generations: [["legacy", 0]] } }),
    libraryTranscriptionClaim: async () => ({ status: "ok", data: "synthetic-token" }),
    libraryTranscriptionRelease: async () => ({ status: "ok" }),
    meetingsGetSegments: async () => ({ status: "ok", data: [{ text: "Synthetic transcript.", start_ms: 0, end_ms: 1000 }] }),
    pagesCreate: async () => ({ status: "ok", data: { id: "transcript" } }),
    pageStateSave: async (id, state) => { pages.push([id, JSON.parse(state)]); return { status: "ok" }; },
    librarySave: async (next) => { book = next; return { status: "ok", data: book }; },
    libraryTranscriptionFinish: async (bookId, adoption) => {
      assert.equal(bookId, book.id);
      assert.equal(adoption.append_page, null);
      book = { ...book, editions: [{ id: "ed_synthetic", text: book.text, audio: book.audio },
        { id: `ed_${adoption.page_id}`, text: { page_id: adoption.page_id, origin: "transcript" }, audio_paths: [] }],
        audio: { ...book.audio, transcript_job: null } };
      return { status: "ok", data: book };
    },
    libraryTranscriptionRemove: async (_bookId, ids) => {
      if (!["ready", "done", "failed", "cancelled"].includes(status)) return { status: "error", error: "job_stopping" };
      book = { ...book, audio: { ...book.audio, transcript_job: null } };
      removed.push(...ids);
      return { status: "ok" };
    },
    meetingsDelete: async (id) => { removed.push(id); return { status: "ok" }; },
  };
  const { useLibrary } = load("../src/components/library/useLibrary.ts", {
    react: { useCallback: (f) => f, useRef: (value) => ({ current: value }),
      useMemo: (f) => f(), useEffect: (f) => effects.push(f),
      useState: (initial) => [states++ === 0 ? [book] : initial, () => {}] },
    "@/bindings": { commands }, "@/lib/reader/pageStore": {}, "@/lib/library/progress": {},
    "@/lib/library/books": { assetUrl: () => null,
      tracksOf: (a) => [a], pendingJobs: (b) => b.audio.transcript_job ? [b.audio.transcript_job] : [],
      transcriptParagraphs: (s) => s, textWithSync: (s, prefix) => ({ text: prefix + s.map((p) => p.text).join("\n"), sync: [{ chars: prefix.length, ms: 0 }] }) },
    sonner: { toast: { error: (message) => errors.push(message) } },
    "@tauri-apps/api/event": { listen: async () => () => {} },
    "@/i18n": { default: { t: (key) => key } }, "@/lib/library/probe": {},
    "@/lib/library/editions": { langCode: (l) => l, editionsOf: (b) => b.editions.length ? b.editions : [{ id: "ed_synthetic", text: b.text, audio: b.audio }] },
    "@/lib/library/importFile": {}, "@/hooks/useMeetingJobs": { useMeetingProgress: () => ({}) },
    "@/lib/library/sections": { aggregateStatus: (s) => s[0], isSectioned: () => false,
      jobKey: (t) => t.transcript_job, jobsOfTrack: (t) => t.transcript_job ? [{ job: t.transcript_job }] : [],
      mergeSections: (s) => s.flatMap((p) => p.segments), RESTART_STATUSES: new Set(["failed"]) },
  }, ["useLibrary"]);
  return { hook: useLibrary(), effects, removed, pages, errors, book: () => book, originalText };
};

test("ready transcript becomes an additional edition; own page is untouched", async () => {
  const f = hookFixture("ready");
  f.effects.at(-1)();
  for (let i = 0; i < 20 && !f.removed.length; i++) await new Promise(setImmediate);
  assert.deepEqual(f.errors, []);
  assert.equal(f.book().text, f.originalText);
  assert.equal(f.book().editions.length, 2);
  assert.equal(f.book().editions[0].text, f.originalText);
  assert.equal(f.book().editions[1].text.page_id, "transcript");
  assert.equal(f.book().editions[1].text.origin, "transcript");
  assert.equal(f.book().audio.transcript_job, null);
  assert.equal(f.pages[0][0], "transcript");
  assert.match(f.pages[0][1].text, /Synthetic transcript/);
  assert.deepEqual(f.removed, ["legacy"]);
});

test("removing a failed legacy helper clears only job references", async () => {
  const f = hookFixture("failed");
  await f.hook.removeTranscript("synthetic");
  assert.equal(f.book().text, f.originalText);
  assert.equal(f.book().audio.file, "synthetic.wav");
  assert.equal(f.book().audio.transcript_job, null);
  assert.deepEqual(f.removed, ["legacy"]);
  assert.deepEqual(f.pages, []);
});

test("removing with a stale failed status cannot delete an active helper", async () => {
  const f = hookFixture("processing");
  await assert.rejects(() => f.hook.removeTranscript("synthetic"));
  assert.deepEqual(f.removed, []);
  assert.equal(f.book().audio.transcript_job, "legacy");
});
