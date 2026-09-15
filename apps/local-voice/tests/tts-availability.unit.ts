import assert from "node:assert/strict";
import { test } from "node:test";
import {
  selectablePiperVoices,
  voiceIsAvailable,
  moduleHelp,
} from "../src/lib/tts/availability";
import type { TtsDownloadInfo } from "../src/bindings";

const runtime: TtsDownloadInfo = {
  id: "piper-runtime",
  name: "Piper",
  description: "",
  kind: "runtime",
  language: null,
  size_mb: 44,
  is_downloaded: true,
  is_downloading: false,
};
const voice: TtsDownloadInfo = {
  ...runtime,
  kind: "voice",
  id: "de_DE-thorsten-high",
  language: "de",
};

test("a downloaded voice without a complete runtime is never selectable", () => {
  assert.deepEqual(selectablePiperVoices([voice]), []);
  assert.deepEqual(
    selectablePiperVoices([{ ...runtime, is_downloaded: false }, voice]),
    [],
  );
  assert.deepEqual(selectablePiperVoices([runtime, voice]), [voice]);
});
test("runtime repair and partial voice downloads stay out of the menu", () => {
  assert.deepEqual(
    selectablePiperVoices([{ ...runtime, is_downloading: true }, voice]),
    [],
  );
  assert.deepEqual(
    selectablePiperVoices([runtime, { ...voice, is_downloaded: false }]),
    [],
  );
});
test("Fish default and named voices require an installed module", () => {
  assert.equal(voiceIsAvailable("@default", false, [], []), false);
  assert.equal(voiceIsAvailable("Anna", false, ["Anna"], []), false);
  assert.equal(voiceIsAvailable("@default", true, [], []), true);
  assert.equal(voiceIsAvailable("missing", true, ["Anna"], []), false);
});
test("removed selections do not silently select another voice", () => {
  assert.equal(voiceIsAvailable("piper:old", true, [], [voice]), false);
  assert.equal(
    voiceIsAvailable("piper:de_DE-thorsten-high", false, [], [voice]),
    true,
  );
});

test("contextual help hides only the absent module's instructions", () => {
  const help = "Read.<!-- module:fish -->Clone.<!-- /module:fish -->Export.";
  assert.equal(moduleHelp(help, false), "Read.Export.");
  assert.equal(moduleHelp(help, true), "Read.Clone.Export.");
});
