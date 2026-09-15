import { test } from "node:test";
import assert from "node:assert/strict";
import { hasLanguageModel, isLanguageModelSetupError } from "../src/lib/llmSetup.ts";
import type { AppSettings } from "../src/bindings.ts";

const configured = (kind: string): Partial<AppSettings> => ({
  llm_active_model_id: "chosen",
  llm_connections: [{ id: "connection", kind, label: kind, base_url: "", enabled: true }],
  llm_models: [{ id: "chosen", connection_id: "connection", remote_id: "model", label: "Model", enabled: true, tags: [] }],
});

test("an idle in-app model, a system model and an external model are configured without RAM probes", () => {
  for (const kind of ["local", "apple_intelligence", "ollama", "openai"]) {
    assert.equal(hasLanguageModel(configured(kind)), true, kind);
  }
});
test("a transcription model alone does not enable language-model tasks", () => {
  assert.equal(hasLanguageModel({ selected_model: "apple-speech" }), false);
  assert.equal(hasLanguageModel(null), false);
});
test("disabled or removed selections need setup even if stale legacy fields remain", () => {
  const state = configured("local");
  state.llm_connections![0].enabled = false;
  assert.equal(hasLanguageModel(state), false);
  state.llm_connections![0].enabled = true;
  state.llm_active_model_id = "removed";
  assert.equal(hasLanguageModel(state), false);
});
test("existing legacy configurations stay usable, but blank model names need setup", () => {
  const state: Partial<AppSettings> = {
    post_process_provider_id: "custom",
    post_process_providers: [{ id: "custom", label: "Custom", base_url: "http://localhost:11434/v1", allow_base_url_edit: true, models_endpoint: null, supports_structured_output: false }],
    post_process_models: { custom: "qwen" },
  };
  assert.equal(hasLanguageModel(state), true);
  state.post_process_models!.custom = "  ";
  assert.equal(hasLanguageModel(state), false);
});
test("only the structured setup error is converted; runtime errors remain errors", () => {
  assert.equal(isLanguageModelSetupError("language_model_setup_required"), true);
  assert.equal(isLanguageModelSetupError("language_model_setup_required: Choose a model"), true);
  for (const error of [null, "model load failed", "Server returned language_model_setup_required", "HTTP 500"]) {
    assert.equal(isLanguageModelSetupError(error), false);
  }
});
