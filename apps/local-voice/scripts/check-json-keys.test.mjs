import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { stripTypeScriptTypes } from "node:module";
import path from "node:path";
import { runInNewContext } from "node:vm";
import test from "node:test";

const source = readFileSync(new URL("./check-json-keys.ts", import.meta.url), "utf8");
const context = {};
runInNewContext(stripTypeScriptTypes(source.replace("export function", "function")), context);
const parse = context.parseUniqueJson;

test("duplicate keys are rejected in each object, including escaped names", () => {
  for (const json of ['{"a":1,"a":2}', '{"library":{"transcription":{},"transcription":{}}}', '{"a":1,"\\u0061":2}', '[{"a":1,"a":2}]']) {
    assert.throws(() => parse(json), /Duplicate JSON key/);
    assert.doesNotThrow(() => JSON.parse(json), "counterprobe: JSON.parse loses the duplicate");
  }
});

test("independent scopes and punctuation in strings remain valid", () => {
  const json = '{"a":{"x":1},"b":{"x":2},"c":[{"x":3}],"text":"\\\"x\\\":{},[]"}';
  assert.equal(JSON.stringify(parse(json)), JSON.stringify(JSON.parse(json)));
  assert.throws(() => parse('{"a":}'));
});

test("every locale has unique keys before translation validation", () => {
  const locales = new URL("../src/i18n/locales/", import.meta.url);
  for (const entry of readdirSync(locales, { withFileTypes: true })) {
    if (!entry.isDirectory()) continue;
    assert.doesNotThrow(() => parse(readFileSync(new URL(`${entry.name}/translation.json`, locales), "utf8")), entry.name);
  }
});

test("translation check rejects duplicates in reference and other locales, with counterprobe", () => {
  const checker = readFileSync(new URL("./check-translations.ts", import.meta.url), "utf8")
    .replace(/^import .*;\r?\n/gm, "")
    .replace(/const __dirname = .*;/, 'const __dirname = ".";');
  const run = (source, duplicateLang) => {
    let exitCode;
    const stop = new Error("synthetic exit");
    const sandbox = {
      path,
      parseUniqueJson: parse,
      fs: {
        readdirSync: () => ["en", "de"].map((name) => ({ name, isDirectory: () => true })),
        readFileSync: (file) => file.includes(`${path.sep}${duplicateLang}${path.sep}`) ? '{"key":"one","key":"two"}' : '{"key":"two"}',
      },
      console: { log() {}, error() {} },
      process: { exit(code) { exitCode = code; throw stop; } },
    };
    assert.throws(() => runInNewContext(stripTypeScriptTypes(source), sandbox), (error) => error === stop);
    return exitCode;
  };
  assert.equal(run(checker, "none"), 0);
  for (const lang of ["en", "de"]) {
    assert.equal(run(checker, lang), 1);
    assert.equal(run(checker.replace("parseUniqueJson(content)", "JSON.parse(content)"), lang), 0);
  }
});

test("restoring the conflicting control block is rejected before keys are lost", () => {
  for (const lang of ["en", "de"]) {
    const json = readFileSync(new URL(`../src/i18n/locales/${lang}/translation.json`, import.meta.url), "utf8");
    const counter = json.replace('"transcriptionControls": {', '"transcription": {');
    assert.throws(() => parse(counter), /Duplicate JSON key/);
    assert.equal(JSON.parse(counter).library.transcription.pause, undefined);
  }
});
