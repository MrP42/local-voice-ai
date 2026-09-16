import type { AppSettings } from "@/bindings";

/** Inspect configuration, not whether a model happens to be loaded in RAM.
 * Legacy provider settings still serve translation, summaries and minutes.
 * Do not treat an idle local model or a system model as a missing download. */
export function hasLanguageModel(
  settings: Partial<AppSettings> | null,
): boolean {
  if (!settings) return false;
  const active = settings.llm_models?.find(
    (model) => model.id === settings.llm_active_model_id && model.enabled,
  );
  if (active) {
    return Boolean(
      active.remote_id.trim() &&
      settings.llm_connections?.some(
        (connection) =>
          connection.id === active.connection_id && connection.enabled,
      ),
    );
  }
  if (settings.llm_models?.length || settings.llm_connections?.length)
    return false;
  const provider = settings.post_process_providers?.find(
    (item) => item.id === settings.post_process_provider_id,
  );
  return Boolean(
    provider && settings.post_process_models?.[provider.id]?.trim(),
  );
}

/** A stable backend code handles configuration changes during an in-flight job.
 * Runtime errors must remain visible, even if they mention a model. */
export function isLanguageModelSetupError(error: string | null): boolean {
  return error?.split(":", 1)[0] === "language_model_setup_required";
}

export function openLanguageModels() {
  window.dispatchEvent(
    new CustomEvent("lv-navigate", { detail: { section: "models" } }),
  );
}

export function openLanguageModelConnections() {
  window.localStorage.setItem("lva.ui.settings.tab", "postprocessing");
  window.dispatchEvent(
    new CustomEvent("lv-navigate", { detail: { section: "settings" } }),
  );
}
