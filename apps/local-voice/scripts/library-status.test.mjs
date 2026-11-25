import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { stripTypeScriptTypes } from "node:module";
import { runInNewContext } from "node:vm";
import test from "node:test";

const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const hook = read("src/components/library/useLibrary.ts");
const poll = hook.slice(hook.indexOf("const poll = async () =>"), hook.indexOf("    void poll();"));

async function runPoll(source, response, reject = false) {
  let jobs;
  const requests = [];
  const transfers = [];
  const context = {
    Map, Set, Array,
    stop: false,
    handle: 0,
    pendingIds: "job",
    books: [{ id: "plain" }, { id: "audio-only" }, { id: "book", job: "job" }],
    pendingJobs: (b) => b.job ? [b.job] : [],
    READY: new Set(["ready", "done"]),
    RESTART_STATUSES: new Set(["failed", "missing"]),
    gaveUpRef: { current: new Set() },
    aggregateStatus: () => "ready",
    finishTranscript: (id) => transfers.push(id),
    setJobs: (next) => { jobs = next; },
    window: { clearInterval() {} },
    commands: {
      meetingsGetAudiobookRunJobInfo: async () => ({ status: "ok", data: [] }),
      meetingsList: async () => ({ status: "ok", data: [{ id: "job", status: "ready" }] }),
      libraryTranscriptionStatus: async (id) => {
        requests.push(id);
        if (reject) throw new Error("synthetic unsupported command");
        return response;
      },
    },
  };
  await runInNewContext(`${stripTypeScriptTypes(source)}\npoll();`, context);
  return { jobs, requests, transfers };
}

for (const response of [null, { status: "ok", data: null }, { status: "ok", data: { state: "future" } }, { status: "error" }]) {
  test(`missing or unknown control is inert: ${JSON.stringify(response)}`, async () => {
    const result = await runPoll(poll, response);
    assert.equal(result.jobs.book.controlState, undefined);
    assert.deepEqual(result.requests, ["book"]);
    assert.deepEqual(result.transfers, []);
  });
}

test("an unsupported command leaves the library mounted", async () => {
  assert.equal((await runPoll(poll, null, true)).jobs.book.controlState, undefined);
});

test("a valid running control permits transfer; paused control blocks it", async () => {
  assert.deepEqual((await runPoll(poll, { status: "ok", data: { state: "running" } })).transfers, ["book"]);
  assert.deepEqual((await runPoll(poll, { status: "ok", data: { state: "paused" } })).transfers, []);
});

test("null-status counterprobe crashes at the old dereference", async () => {
  const old = poll.replace(/const controlState = [\s\S]*?;\n/, 'const controlState = control.status === "ok" ? control.data.state : undefined;\n');
  await assert.rejects(runPoll(old, { status: "ok", data: null }), /state/);
});

test("controls reject absent and unknown states, with counterprobe", () => {
  const source = read("src/components/library/TranscriptionControls.tsx");
  const guard = source.match(/  if \(!state[^\n]+return null;/)?.[0] ?? "";
  const render = runInNewContext(`(state) => { ${guard} return "controls"; }`);
  for (const state of [undefined, null, "future"]) assert.equal(render(state), null);
  for (const state of ["running", "pausing", "paused", "stopping", "cancelled"])
    assert.equal(render(state), "controls");
  assert.equal(runInNewContext('(state) => "controls"')(undefined), "controls");
});

test("missing adoption status is superseded without exhausting retries, with counterprobe", async () => {
  const source = hook.slice(
    hook.indexOf("async (bookId: string, durations:"),
    hook.indexOf("\n    [reload, giveUp]", hook.indexOf("const finishTranscript")),
  ).trim().replace(/,$/, "");
  async function attempt(code, response) {
    const failures = new Map();
    const gaveUp = [];
    const track = { transcript_job: "job" };
    const context = {
      Map, Set, Array, Error,
      Superseded: class extends Error {},
      gaveUpRef: { current: new Set() },
      freshBook: async () => ({ audio: { tracks: [track] }, text: null }),
      tracksOf: (audio) => audio.tracks,
      jobsOfTrack: () => [{ job: "job" }],
      pendingJobs: () => ["job"],
      finishing: { current: new Set() },
      inFlight: { current: new Set() },
      releaseTimers: { current: new Map() },
      failures: { current: failures },
      MAX_FAILURES: 3,
      giveUp: (id) => gaveUp.push(id),
      reload: async () => {},
      window: { setTimeout: () => 1, clearTimeout() {} },
      commands: { libraryTranscriptionStatus: async () => response },
    };
    runInNewContext(`finish = ${stripTypeScriptTypes(code)}`, context);
    for (let i = 0; i < 3; i++) {
      context.finishing.current.clear();
      await context.finish("book", new Map());
    }
    return { failures, gaveUp };
  }
  for (const response of [null, { status: "ok", data: null }, { status: "ok", data: { state: "running" } }]) {
    const actual = await attempt(source, response);
    assert.equal(actual.failures.size, 0);
    assert.deepEqual(actual.gaveUp, []);
  }
  const old = source.replace(/if \(control\?\.status[^\n]+/, 'if (control.status !== "ok" || control.data.state !== "running")');
  assert.deepEqual((await attempt(old, { status: "ok", data: null })).gaveUp, ["book"]);
});

test("library command mock covers multipart controls and adoption, with counterprobe", async () => {
  const source = read("tests/libraryTranscriptionMock.ts")
    .replace(/^import[^\n]*\n/, "").replace("export async function", "async function");
  const book = { id: "book", audio: { tracks: [
    { transcript_job: "a" },
    { transcript_jobs: [{ job: "b" }, { job: "a" }] },
  ] } };
  const saved = [];
  const w = {};
  const context = { window: w, Map, Set };
  runInNewContext(stripTypeScriptTypes(source), context);
  await context.installLibraryTranscriptionMock({ addInitScript: async (fn) => fn() });
  const invoke = async (cmd, args = {}) => {
    if (cmd.startsWith("library_transcription_")) return w.__libraryTranscriptionMock(cmd, args);
    if (cmd === "library_list") return [book];
    if (cmd === "library_save") { saved.push(args.book); return args.book; }
    return null;
  };
  w.__TAURI_INTERNALS__ = { invoke };
  const status = () => invoke("library_transcription_status", { bookId: "book" });
  assert.equal(JSON.stringify((await status()).generations), '[["a",0],["b",0]]');
  for (const [cmd, state] of [["pause", "paused"], ["resume", "running"], ["cancel", "cancelled"]]) {
    await invoke(`library_transcription_${cmd}`, { bookId: "book", keepPartial: true });
    assert.equal((await status()).state, state);
    assert.equal((await status()).generations[0][1], 0);
  }
  await invoke("library_transcription_cancel", { bookId: "book", keepPartial: false });
  assert.equal((await status()).generations[0][1], 1);
  await assert.rejects(invoke("library_transcription_claim", { bookId: "book" }));
  await invoke("library_transcription_resume", { bookId: "book" });
  const token = await invoke("library_transcription_claim", { bookId: "book", generations: (await status()).generations });
  await assert.rejects(invoke("library_transcription_pause", { bookId: "book" }));
  const adoption = { page_id: "synthetic_page", append_page: null, sync: [], durations: [], edition_title: "Synthetic" };
  await assert.rejects(invoke("library_transcription_finish", { bookId: book.id, adoption, token: "stale" }));
  const adopted = await invoke("library_transcription_finish", { bookId: book.id, adoption, token });
  assert.equal(adopted.text.page_id, "synthetic_page");
  assert.equal(adopted.audio.tracks[0].transcript_job, null);
  assert.deepEqual(saved, [adopted]);
  await invoke("library_transcription_release", { bookId: "book", token });
  // Die bisherige Attrappe lieferte fuer jeden dieser Commands null.
  const old = async () => null;
  const oldStatus = await old();
  assert.throws(() => oldStatus.state, /state/);
});
