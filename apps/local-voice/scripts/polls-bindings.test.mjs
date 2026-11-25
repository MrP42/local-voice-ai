import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { stripTypeScriptTypes } from "node:module";
import vm from "node:vm";

const root = new URL("../", import.meta.url);
const source = readFileSync(new URL("src/bindings.ts", root), "utf8");
const calls = [];
function load(text, invoke) {
  const start = text.indexOf("export const commands =");
  const end = text.indexOf("/** user-defined events **/", start);
  assert.ok(start >= 0 && end > start);
  const js = stripTypeScriptTypes(text.slice(start, end).replace("export const", "const"));
  const context = vm.createContext({ TAURI_INVOKE: invoke, Error });
  vm.runInContext(`${js}\nglobalThis.result = commands;`, context);
  return context.result;
}
const commands = load(source, async (name, args) => {
  calls.push({ name, args: args === undefined ? undefined : JSON.parse(JSON.stringify(args)) });
  return { synthetic: true };
});
const cases = [
  ["pollCreate", "poll_create", ["synthetic", { titel: "Synthetic" }], { integrationId: "synthetic", poll: { titel: "Synthetic" } }],
  ["pollGet", "poll_get", ["T-ABCD"], { id: "T-ABCD" }],
  ["pollUpdate", "poll_update", ["T-ABCD", { titel: "Synthetic" }], { id: "T-ABCD", poll: { titel: "Synthetic" } }],
  ["pollRank", "poll_rank", ["T-ABCD"], { id: "T-ABCD" }],
  ["pollList", "poll_list", [], undefined],
  ["pollAvailability", "poll_availability", ["T-ABCD", { time_zone: "Europe/Berlin" }], { id: "T-ABCD", window: { time_zone: "Europe/Berlin" } }],
  ["pollSend", "poll_send", ["T-ABCD"], { id: "T-ABCD" }],
  ["pollAnswerConfirm", "poll_answer_confirm", ["T-ABCD", 42], { id: "T-ABCD", answerId: 42 }],
  ["pollAnswerSet", "poll_answer_set", ["T-ABCD", "one@example.invalid", []], { id: "T-ABCD", person: "one@example.invalid", values: [] }],
  ["pollClose", "poll_close", ["T-ABCD", false], { id: "T-ABCD", decided: false }],
];

for (const [method, name, args, expected] of cases) {
  test(`${method} forwards the command and named arguments`, async () => {
    const result = await commands[method](...args);
    assert.equal(result.status, "ok");
    assert.deepEqual(calls.pop(), { name, args: expected });
  });
}

test("wire errors remain errors", async () => {
  const failing = load(source, async () => { throw "poll_account_changed"; });
  const result = await failing.pollSend("T-ABCD");
  assert.equal(result.status, "error");
  assert.equal(result.error, "poll_account_changed");
});

test("all poll wrappers are registered in Rust", () => {
  const lib = readFileSync(new URL("src-tauri/src/lib.rs", root), "utf8");
  for (const [, name] of cases) assert.ok(lib.includes(`commands::polls::${name},`));
});

test("counterprobe: the same contract fails without the TF2 wrappers", () => {
  const old = source.replaceAll(/async poll[A-Z]\w*\(/g, (m) => m.replace("poll", "ohne"));
  assert.notEqual(old, source);
  const before = load(old, async () => null);
  assert.throws(() => {
    for (const [method] of cases) assert.equal(typeof before[method], "function");
  }, assert.AssertionError);
});

test("locale labels and capability switches agree", () => {
  const switches = readFileSync(new URL("src/components/integrations/m365.ts", root), "utf8");
  for (const locale of ["de", "en"]) {
    const t = JSON.parse(readFileSync(new URL(`src/i18n/locales/${locale}/translation.json`, root), "utf8"));
    for (const key of ["calendar_availability", "mail_answers"]) {
      assert.ok(t.integrations.capabilities[key].title);
      assert.ok(t.integrations.m365.caps[key]);
      assert.ok(switches.includes(`"${key.replace("_", ".")}"`));
    }
  }
});
