import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { stripTypeScriptTypes } from "node:module";
import { pollDue, focusDue, promptDue, waitForUpdateGate } from "../src/components/update-checker/updatePolicy.ts";

const minute = 60_000;
const source = await readFile(new URL("../src/components/update-checker/updatePolicy.ts", import.meta.url), "utf8");
const mutant = async (before, after) => {
  assert.ok(source.includes(before));
  const js = stripTypeScriptTypes(source.replace(before, after));
  return import(`data:text/javascript;base64,${Buffer.from(js).toString("base64")}`);
};

function pollingContract(poll, focus) {
  assert.equal(poll(29 * minute, 0, 28.5 * minute), false);
  assert.equal(poll(30 * minute, 0, 29.5 * minute), true);
  assert.equal(poll(4 * minute, 3 * minute, minute), true);
  assert.equal(poll(2 * minute, minute, 0), false);
  assert.equal(focus(5 * minute - 1, 0), false);
  assert.equal(focus(5 * minute, 0), true);
}

test("30 Minuten, Fokusfrist und Aufwachen; Gegenprobe ohne Intervall", async () => {
  pollingContract(pollDue, focusDue);
  const broken = await mutant("now - lastCheck >= UPDATE_INTERVAL", "false");
  assert.throws(() => pollingContract(broken.pollDue, broken.focusDue));
});

test("Aufwachen und Fokus haben eigene Gegenproben", async () => {
  const noWake = await mutant("now - lastTick > 2 * 60_000", "false");
  const noFocusLimit = await mutant("now - lastCheck >= UPDATE_FOCUS_INTERVAL", "true");
  assert.throws(() => pollingContract(noWake.pollDue, noWake.focusDue));
  assert.throws(() => pollingContract(noFocusLimit.pollDue, noFocusLimit.focusDue));
});

function promptContract(prompt) {
  const later = { version: "1.2.3", bis: 4 * 60 * minute };
  assert.equal(prompt("1.2.3", false, new Set(), later, later.bis - 1), false);
  assert.equal(prompt("1.2.3", false, new Set(), later, later.bis), true);
  assert.equal(prompt("1.2.4", false, new Set(), later, 0), true);
  assert.equal(prompt("1.2.4", true, new Set(), later, 0), false);
  assert.equal(prompt("1.2.4", false, new Set(["1.2.4"]), later, 0), false);
}

test("Aufschub ist versionsgebunden, belegt wartet, je Version ein Fenster; Gegenproben", async () => {
  promptContract(promptDue);
  for (const [before, after] of [
    ["!busy && ", ""],
    ["!prompted.has(version) && ", ""],
    ["later?.version === version && ", "later !== null && "],
    ["later.bis > now", "true"],
  ]) {
    const broken = await mutant(before, after);
    assert.throws(() => promptContract(broken.promptDue));
  }
});

async function waitingContract(wait) {
  const calls = [];
  let free;
  const done = wait(async () => {
    calls.push("begin");
    return calls.filter((v) => v === "begin").length === 1 ? "belegt" : "ok";
  }, async (notify) => {
    calls.push("listen");
    free = notify;
    return () => calls.push("unlisten");
  }, new AbortController().signal);
  await new Promise(setImmediate);
  assert.deepEqual(calls, ["listen", "begin"]);
  free();
  await done;
  assert.deepEqual(calls, ["listen", "begin", "begin", "unlisten"]);
}

test("Listener steht vor begin; belegt wartet ohne Timer; Gegenprobe ohne Sperrbeachtung", async () => {
  await waitingContract(waitForUpdateGate);
  const broken = await mutant('if (await attempt() === "ok") return;', 'if (await attempt()) return;');
  await assert.rejects(waitingContract(broken.waitForUpdateGate), assert.AssertionError);
});

test("Frei-Event waehrend begin geht nicht verloren", async () => {
  let notify;
  let attempts = 0;
  await waitForUpdateGate(async () => {
    if (++attempts === 1) { notify(); return "belegt"; }
    return "ok";
  }, async (fn) => { notify = fn; return () => {}; }, new AbortController().signal);
  assert.equal(attempts, 2);
});

test("Abbruch und geschlossener Fehler entfernen den Listener", async () => {
  const controller = new AbortController();
  let removed = 0;
  const done = waitForUpdateGate(async () => "belegt", async () => () => removed++, controller.signal);
  await new Promise(setImmediate);
  controller.abort();
  await assert.rejects(done, /update_abgebrochen/);
  await assert.rejects(waitForUpdateGate(async () => { throw new Error("nicht_verfuegbar"); }, async () => () => removed++, new AbortController().signal), /nicht_verfuegbar/);
  assert.equal(removed, 2);
});
