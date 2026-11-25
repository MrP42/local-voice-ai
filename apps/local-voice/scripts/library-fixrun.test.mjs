import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { stripTypeScriptTypes } from "node:module";
import { runInNewContext } from "node:vm";
import test from "node:test";

const read = (file) => readFileSync(new URL(`../${file}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const hook = read("src/components/library/useLibrary.ts");

async function missingPartScenario(mutate = (source) => source) {
  const start = hook.indexOf("async (bookId: string, durations:");
  const end = hook.indexOf("      const jobs = pendingJobs(start);", start);
  const source = mutate(hook.slice(start, end)) + 'return "adopt"; }';
  for (const ownText of [false, true]) {
    const calls = [];
    const context = {
      gaveUpRef: { current: new Set() },
      freshBook: async () => ({ audio: { tracks: [{ job: "job" }, {}] },
        text: ownText ? { page_id: "own", origin: "document" } : null }),
      tracksOf: (audio) => audio.tracks, jobsOfTrack: (t) => t.job ? [t.job] : [],
      restarted: { current: new Set() },
      transcribeTracks: async () => calls.push("start"), reload: async () => {},
      toast: { error: () => {} }, giveUp: () => assert.fail("unexpected give-up"),
    };
    const finish = runInNewContext(stripTypeScriptTypes(`(${source})`), context);
    const outcome = await finish("synthetic", new Map());
    assert.deepEqual(calls, ownText ? [] : ["start"]);
    assert.equal(outcome, ownText ? "adopt" : undefined);
  }
}

test("automatic missing-part restart respects own text", () => missingPartScenario());
test("counterprobe: own-text automatic restart fails", async () => {
  await assert.rejects(missingPartScenario((source) => source.replace("!appendTo && !ownText", "!appendTo")), { code: "ERR_ASSERTION" });
});

async function pollingScenario(mutate = (source) => source) {
  const start = hook.indexOf("  useEffect(() => {", hook.indexOf("const pendingIds"));
  const source = mutate(hook.slice(start, hook.indexOf("\n\n  /**", start)));
  let status = "failed";
  let interval;
  let cleanup;
  const finished = [];
  const context = {
    pendingIds: "job", books: [{ id: "synthetic", text: { origin: "document" } }],
    pendingJobs: () => ["job"], gaveUpRef: { current: new Set() }, gaveUp: new Set(),
    READY: new Set(["ready", "done"]), RESTART_STATUSES: new Set(["failed", "cancelled"]), POLL_MS: 5000,
    aggregateStatus: (statuses) => statuses[0], setJobs: () => {},
    finishTranscript: async (id) => finished.push(id),
    useEffect: (effect) => { cleanup = effect(); },
    window: { setInterval: (tick, ms) => { assert.equal(ms, 5000); interval = tick; return 1; }, clearInterval: () => { interval = null; } },
    commands: {
      meetingsList: async () => ({ status: "ok", data: [{ id: "job", status }] }),
      meetingsGetAudiobookRunJobInfo: async () => ({ status: "ok", data: [] }),
      libraryTranscriptionStatus: async () => ({ status: "ok", data: { state: "running" } }),
    },
  };
  // Capture the async initial poll without timers or wall-clock sleeps.
  const run = async (code) => {
    runInNewContext(stripTypeScriptTypes(code.replace("void poll();", "lastPoll = poll();").replace("() => void poll()", "() => poll()")),
      context);
    await context.lastPoll;
  };
  await run(source);
  assert.equal(interval, null);
  cleanup();
  status = "queued";
  await run(source);
  assert.equal(typeof interval, "function");
  status = "ready";
  assert.deepEqual(finished, []);
  await interval();
  assert.deepEqual(finished, ["synthetic"]);
}

test("retry restarts polling after failed; next tick adopts ready own-text job", () => pollingScenario());
test("counterprobe: no ready adoption fails the polling scenario", async () => {
  await assert.rejects(pollingScenario((source) => source.replace('status === "ready"', 'status === "never"')), { code: "ERR_ASSERTION" });
});

async function durationScenario(mutate = (source) => source) {
  let book = { id: "synthetic", audio: { file: "single.wav", duration_ms: null,
    tracks: [{ file: "b.wav", duration_ms: null }, { file: "a.wav", duration_ms: 7000 }] } };
  const w = { __TAURI_INTERNALS__: { invoke: async (cmd, args) => {
    if (cmd === "library_list") return [book];
    if (cmd === "library_save") { book = args.book; return book; }
    throw new Error("unexpected synthetic command");
  } } };
  const context = { window: w };
  const source = mutate(read("tests/libraryTranscriptionMock.ts")).replace(/^import .*;\n/m, "").replace("export async function", "async function");
  runInNewContext(stripTypeScriptTypes(source), context);
  await context.installLibraryTranscriptionMock({ addInitScript: (install) => install() });
  const adopt = async () => {
    const state = await w.__libraryTranscriptionMock("library_transcription_status", { bookId: book.id });
    // A real claim needs an attached helper job.
    const token = await w.__libraryTranscriptionMock("library_transcription_claim", { bookId: book.id, generations: state.generations });
    await w.__libraryTranscriptionMock("library_transcription_finish", { bookId: book.id, token, adoption: {
      page_id: "page", sync: [], durations: [["a.wav", 9000], ["b.wav", 15000], ["single.wav", 20000]],
    } });
  };
  book.audio.tracks[0].transcript_job = "job";
  await adopt();
  assert.equal(book.audio.tracks[0].duration_ms, 15000);
  assert.equal(book.audio.tracks[1].duration_ms, 7000);
  assert.equal(book.audio.duration_ms, null);
  book = { id: "synthetic-single", audio: { file: "single.wav", duration_ms: null, transcript_job: "job" } };
  await adopt();
  assert.equal(book.audio.duration_ms, 20000);
}

test("transcription mock adopts per-file durations like the backend", () => durationScenario());
test("counterprobe: dropping adoption durations fails", async () => {
  await assert.rejects(durationScenario((source) => source.replace(
    "const durations = new Map<string, number | null>(a.durations);",
    "const durations = new Map<string, number | null>();",
  )), { code: "ERR_ASSERTION" });
});
