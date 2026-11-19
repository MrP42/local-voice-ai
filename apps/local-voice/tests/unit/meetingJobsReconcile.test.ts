// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/meetingJobsReconcile.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  RECONCILE_GRACE_MS,
  reconcileProgress,
  removeJob,
  type LiveProgress,
  type ProgressMap,
} from "../../src/lib/meetingJobs.ts";

const live = (id: string, receivedAt: number): LiveProgress => ({
  meeting_id: id,
  phase: "prepare",
  done: 0,
  total: 0,
  elapsed_ms: 0,
  eta_ms: null,
  state: "running",
  pausable: false,
  receivedAt,
});

const job = (id: string) => ({
  meeting_id: id,
  phase: "prepare" as const,
  done: 0,
  total: 0,
  elapsed_ms: 0,
  eta_ms: null,
  state: "running" as const,
  pausable: false,
});

test("ein Eintrag, den das Backend nicht mehr kennt, wird nach der Karenzzeit entfernt", () => {
  const map: ProgressMap = { a: live("a", 0), b: live("b", 0) };
  const now = RECONCILE_GRACE_MS + 1;
  const next = reconcileProgress(map, [job("b")], now);
  assert.deepEqual(Object.keys(next), ["b"]);
});

test("ein junger Eintrag bleibt, auch wenn die (ältere) Abfrage ihn nicht kennt", () => {
  const map: ProgressMap = { a: live("a", 10_000) };
  const next = reconcileProgress(map, [], 10_000 + RECONCILE_GRACE_MS - 1);
  assert.deepEqual(Object.keys(next), ["a"]);
});

test("Gegenprobe: ohne Abgleich bliebe der Geister-Eintrag mit weiterlaufender Uhr stehen", () => {
  const map: ProgressMap = { a: live("a", 0) };
  assert.equal(Object.keys(map).length, 1);
  assert.equal(Object.keys(reconcileProgress(map, [], RECONCILE_GRACE_MS)).length, 0);
});

test("unveränderte Karte bleibt dieselbe Referenz", () => {
  const map: ProgressMap = { a: live("a", 0) };
  assert.equal(reconcileProgress(map, [job("a")], 1_000_000), map);
});

test("removeJob nimmt genau den Eintrag weg (no_job) und ändert sonst nichts", () => {
  const map: ProgressMap = { a: live("a", 0), b: live("b", 0) };
  assert.deepEqual(Object.keys(removeJob(map, "a")), ["b"]);
  assert.equal(removeJob(map, "x"), map);
});
