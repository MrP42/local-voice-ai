import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import { test } from "node:test";

// Replace module boundaries only; execute the production function unchanged.
const source = readFileSync(new URL("../src/lib/initializeShortcuts.ts", import.meta.url), "utf8")
  .replace(/^import .*;\n/gm, "")
  .replace("export async function", "async function");

function harness(result, code = source) {
  const notices = [];
  let refreshed = 0;
  const context = vm.createContext({
    commands: { initializeShortcuts: async () => result },
    toast: { error: (text) => notices.push(text) },
    i18n: { t: (key, args) => ({ key, ...args }) },
    useSettingsStore: { getState: () => ({ refreshSettings: async () => { refreshed++; } }) },
  });
  vm.runInContext(code, context);
  return { run: () => context.initializeShortcutsWithNotice(), notices, refreshed: () => refreshed };
}

test("startup registration failure uses the existing localized shortcut error and reloads fallback settings", async () => {
  const h = harness({ status: "error", error: "transcribe: synthetic conflict" });
  await h.run();
  assert.equal(h.notices.length, 1);
  assert.equal(h.notices[0].key, "settings.general.shortcut.errors.set");
  assert.equal(h.notices[0].error, "transcribe: synthetic conflict");
  assert.equal(h.refreshed(), 1);
});

test("successful registration does not display an error", async () => {
  const h = harness({ status: "ok", data: null });
  await h.run();
  assert.equal(h.notices.length, 0);
  assert.equal(h.refreshed(), 0);
});

test("counterprobe: ignoring the Result fails the failure-notice assertion", async () => {
  const h = harness({ status: "error", error: "synthetic conflict" },
    source.replace('result.status === "error"', "false"));
  await h.run();
  assert.throws(() => assert.equal(h.notices.length, 1), { code: "ERR_ASSERTION" });
});
