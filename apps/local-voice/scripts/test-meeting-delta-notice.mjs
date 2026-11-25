// lva#863: Originalen Ereignishoerer ohne Browser ausfuehren; Ausgangsstand als rote Gegenprobe.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

const current = readFileSync(new URL("../src/components/settings/meetings/RecorderCard.tsx", import.meta.url), "utf8");
// Aufruf von der Repo-Wurzel: git show 43c808960f94aed1e862c4acd4e342c529880e06:apps/local-voice/src/components/settings/meetings/RecorderCard.tsx |
// node --test --test-isolation=none apps/local-voice/scripts/test-meeting-delta-notice.mjs
const baseline = readFileSync(0, "utf8");

function recorder(source) {
  const start = source.indexOf("const un = events.meetingEvent.listen((e) => {");
  const end = source.indexOf("const unNotes =", start);
  assert.ok(start >= 0 && end > start);
  const state = { error: null, notice: null, loss: null };
  const setter = (key) => (value) => { state[key] = typeof value === "function" ? value(state[key]) : value; };
  let listener;
  runInNewContext(source.slice(start, end), {
    events: { meetingEvent: { listen: (fn) => { listener = fn; } } },
    recordingIdRef: { current: "m1" }, notesMeetingRef: { current: null },
    tRef: { current: (code) => code }, translateMeetingError: (code) => code,
    NOTICE_CODES: new Set(), setError: setter("error"),
    setStoreNotice: setter("notice"), setStoreLoss: setter("loss"),
    setPhase() {}, setAutoNotes() {}, setBusy() {}, setStartNotice() {},
  });
  return { state, emit: (payload) => listener({ payload: { meeting_id: "m1", ...payload } }) };
}

function recoveryContract(source, kind) {
  const { state, emit } = recorder(source);
  emit({ kind: "error", message: "delta_store_failed" });
  assert.ok(state.notice || state.error);
  emit(kind === "segments" ? { kind, appended: [] } : { kind, status: "processing", paused: false });
  assert.equal(state.notice, null);
  assert.equal(state.error, null);
}

function consecutiveRecordingsContract(source) {
  const { state, emit } = recorder(source);
  emit({ kind: "error", message: "delta_store_lost:60000:80000" });
  emit({ kind: "state", status: "processing", paused: false });
  assert.ok(state.loss, "Verlust bleibt nach dem Abschluss sichtbar");
  emit({ kind: "state", meeting_id: "m2", status: "recording", paused: false });
  assert.equal(state.loss, null, "Neue Aufnahme uebernimmt keinen alten Verlust");
  emit({ kind: "error", message: "delta_store_lost:60000:80000" });
  assert.equal(state.loss, null, "Spaetes Ereignis der alten Aufnahme bleibt draussen");
  emit({ kind: "error", meeting_id: "m2", message: "delta_store_lost:1000:2000" });
  emit({ kind: "state", meeting_id: "m2", status: "paused", paused: true });
  emit({ kind: "state", meeting_id: "m2", status: "recording", paused: false });
  assert.ok(state.loss, "Pause und Fortsetzen derselben Aufnahme behalten den Verlust");
  emit({ kind: "state", meeting_id: "m3", status: "recording", paused: false });
  emit({ kind: "state", meeting_id: "m3", status: "ready", paused: false });
  emit({ kind: "error", message: "delta_store_lost:60000:80000" });
  assert.equal(state.loss, null, "Vollstaendige Folgeaufnahme zeigt keinen alten Verlust");
}

test("Aufeinanderfolgende Aufnahmen behalten nur ihren eigenen Verlust", () => consecutiveRecordingsContract(current));
test("Gegenprobe: ungebundener Verlust verletzt den Aufnahmewechsel-Vertrag", () => {
  const unbound = current
    .replace(/setStoreLoss\(\(loss\) => \(loss\?\.meetingId === payload\.meeting_id \? loss : null\)\);/g, "")
    .replace(/if \(meetingId !== null && payload\.meeting_id !== meetingId\) return;/g, "");
  assert.throws(() => consecutiveRecordingsContract(unbound), { code: "ERR_ASSERTION" });
});

for (const kind of ["segments", "state"]) {
  test(`${kind}: Speicherhinweis wird zurueckgenommen`, () => recoveryContract(current, kind));
  test(`${kind}: Gegenprobe auf dem Ausgangsstand verletzt denselben Vertrag`, () => {
    assert.throws(() => recoveryContract(baseline, kind), { code: "ERR_ASSERTION" });
  });
}

test("Fremde Segments raeumen nichts ab; andere Fehler und Verlust bleiben", () => {
  const { state, emit } = recorder(current);
  emit({ kind: "error", message: "delta_store_failed" });
  emit({ kind: "segments", meeting_id: "other", appended: [] });
  assert.equal(state.notice, "m1");
  emit({ kind: "error", message: "audio_write_failed" });
  emit({ kind: "error", message: "delta_store_lost:60000:80000" });
  emit({ kind: "segments", appended: [] });
  emit({ kind: "state", status: "ready", paused: false });
  assert.equal(state.notice, null);
  assert.equal(state.error, "audio_write_failed");
  assert.equal(state.loss?.message, "delta_store_lost:60000:80000");
});

test("Einwilligungsablehnung nimmt nur den Speicherhinweis zurueck", () => {
  const { state, emit } = recorder(current);
  emit({ kind: "error", message: "delta_store_failed" });
  emit({ kind: "error", message: "nur_notizen: abgelehnt" });
  assert.equal(state.notice, null);
  assert.equal(state.error, "nur_notizen: abgelehnt");
});

test("Verlusttext hat mm:ss und alle Sprachen haben die neuen Schluessel", () => {
  const source = readFileSync(new URL("../src/components/settings/meetings/meetingErrors.ts", import.meta.url), "utf8");
  const start = source.indexOf("  const loss =");
  const end = source.indexOf("  const normalized =", start);
  assert.ok(start >= 0 && end > start);
  const format = source.slice(start, end).replace("(ms: string)", "(ms)");
  const output = runInNewContext(`(() => { ${format} })()`, {
    code: "delta_store_lost:4203000:4367000",
    t: (key, values) => ({ key, ...values }),
  });
  assert.equal(output.key, "meetings.errors.deltaStoreLost");
  assert.equal(output.start, "70:03");
  assert.equal(output.end, "72:47");
  for (const lang of ["ar", "bg", "cs", "da", "de", "en", "es", "fr", "he", "hi", "it", "ja", "ko", "ne", "nl", "pl", "pt", "ru", "sv", "tr", "uk", "vi", "zh-TW", "zh"]) {
    const { meetings: { errors } } = JSON.parse(readFileSync(new URL(`../src/i18n/locales/${lang}/translation.json`, import.meta.url), "utf8"));
    assert.ok(errors.deltaStoreFailed && errors.deltaStoreLost);
    assert.ok(errors.deltaStoreLost.includes("{{start}}") && errors.deltaStoreLost.includes("{{end}}"));
    assert.doesNotMatch(errors.deltaStoreFailed, /läuft weiter|keeps running/);
  }
});
