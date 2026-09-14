import { test, expect } from "@playwright/test";

// Die Modellseite mit einem lokalen Sprachmodell-Bestand: Laufzeit für
// diesen Rechner installiert, ein Modell geladen, eines nicht. Geprüft wird,
// was der Nutzer sehen und tun soll — Merkmale statt Zahlen, "Verwenden" nur
// bei geladenen Modellen, und dass Verwenden im Backend ankommt.
test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(() => {
    const callbacks = new Map<number, unknown>();
    let callback = 0;
    const settings: Record<string, unknown> = {
      onboarding_completed: true,
      app_language: "de",
      theme: "light",
      show_whats_new_on_update: false,
      debug_mode: false,
      selected_model: "",
      bindings: {
        transcribe: { id: "transcribe", name: "Diktat", description: "", current_binding: "Ctrl+Space", default_binding: "Ctrl+Space" },
      },
      post_process_providers: [],
      post_process_prompts: [],
      custom_words: [],
      post_process_models: {},
      post_process_api_keys: {},
      llm_connections: [],
      llm_models: [],
      llm_active_model_id: null,
      push_to_talk: true,
    };
    const saved: Record<string, unknown> = {};
    (window as unknown as { saved: Record<string, unknown> }).saved = saved;
    const downloads = [
      { id: "llm-runtime-windows-x64-vulkan", kind: "runtime", name: "Sprachmodell-Laufzeit (Windows, Vulkan)", description: "llama.cpp", size_mb: 30, is_downloaded: true, is_downloading: false, tags: [], backend: "vulkan", for_this_platform: true },
      { id: "llm-runtime-macos-aarch64", kind: "runtime", name: "Sprachmodell-Laufzeit (macOS)", description: "llama.cpp", size_mb: 11, is_downloaded: false, is_downloading: false, tags: [], backend: null, for_this_platform: false },
      { id: "llm-qwen3-4b-q4", kind: "model", name: "Qwen3 4B", description: "Empfohlener Standard.", size_mb: 2381, is_downloaded: true, is_downloading: false, tags: ["empfohlen", "zusammenfassung"], backend: null, for_this_platform: true },
      { id: "llm-qwen3-8b-q4", kind: "model", name: "Qwen3 8B", description: "Mehr Qualität.", size_mb: 4795, is_downloaded: false, is_downloading: false, tags: ["qualitaet"], backend: null, for_this_platform: true },
    ];
    Object.assign(window, {
      __TAURI_OS_PLUGIN_INTERNALS__: { platform: "windows", os_type: "windows", family: "windows", arch: "x86_64", version: "10.0.26200", eol: "\r\n" },
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
      __TAURI_INTERNALS__: {
        metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
        transformCallback: (fn: unknown) => { callbacks.set(++callback, fn); return callback; },
        unregisterCallback: (id: number) => callbacks.delete(id),
        convertFileSrc: (path: string) => path,
        invoke: async (cmd: string, args?: Record<string, unknown>) => {
          if (cmd === "get_app_settings" || cmd === "get_default_settings") return settings;
          if (cmd === "plugin:os|locale") return "de-DE";
          if (cmd === "plugin:app|version") return "0.16.3";
          if (cmd.includes("permission")) return true;
          if (cmd === "plugin:event|listen") return ++callback;
          if (cmd === "get_selected_model") return "";
          if (cmd === "meetings_is_recording") return false;
          if (cmd === "tts_server_status") return { phase: "stopped", message: null };
          if (cmd === "llm_local_list") return downloads;
          if (cmd === "llm_local_status") return { phase: "stopped", model_id: null, backend: null, port: null, message: null };
          // Prognose: das 4B passt, das 8B ist knapp -- Zahlen aus dem Backend.
          if (cmd === "llm_local_fit")
            return args?.modelId === "llm-qwen3-8b-q4"
              ? { estimate: { weights_mb: 5514, kv_mb: 1152, overhead_mb: 512, total_mb: 7178, context_tokens: 8192, from_metadata: true }, free_mb: 7900, on_gpu: true, verdict: "tight" }
              : { estimate: { weights_mb: 2739, kv_mb: 1152, overhead_mb: 512, total_mb: 4403, context_tokens: 8192, from_metadata: true }, free_mb: 18920, on_gpu: true, verdict: "fits" };
          if (cmd === "llm_local_activate") {
            saved.activated = args?.modelId;
            settings.llm_connections = [{ id: "local", kind: "local", label: "In der App", base_url: "http://127.0.0.1:0/v1", enabled: true }];
            settings.llm_models = [{ id: `local:${args?.modelId}`, connection_id: "local", remote_id: args?.modelId, label: "Qwen3 4B", enabled: true, tags: [] }];
            settings.llm_active_model_id = `local:${args?.modelId}`;
            return null;
          }
          if (cmd === "get_available_models" || cmd === "get_models") return [];
          if (cmd === "tts_list_voices" || cmd === "tts_list_voice_infos" || cmd === "llm_ps" || cmd === "tts_reading_list" || cmd === "pages_list" || cmd === "page_files" || cmd === "tts_list_downloads")
            return [];
          if (cmd.includes("history") || cmd.includes("models") || cmd.includes("devices") || cmd === "meetings_list") return [];
          if (cmd === "get_custom_sounds") return { start: false, stop: false };
          return null;
        },
      },
    });
  });
});

async function openModels(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByRole("button", { name: "Modelle", exact: true }).last().click();
  await expect(page.getByText("Sprachmodelle in der App")).toBeVisible();
}

test("runtimes for other platforms stay out of the list entirely", async ({ page }) => {
  await openModels(page);
  // Ein macOS-Paket auf einem Windows-Rechner ist nur Ballast -- die Seite
  // zeigt es gar nicht erst, statt es als "nicht ladbar" mitzuschleppen.
  await expect(page.locator('[data-llm-card="llm-runtime-macos-aarch64"]')).toHaveCount(0);
  const vulkan = page.locator('[data-llm-card="llm-runtime-windows-x64-vulkan"]');
  await expect(vulkan.getByText("Installiert")).toBeVisible();
  await expect(vulkan.getByText("Vulkan – jede Grafikkarte")).toBeVisible();
});

test("models carry readable traits and only downloaded ones can be used", async ({ page }) => {
  await openModels(page);
  const qwen4 = page.locator('[data-llm-card="llm-qwen3-4b-q4"]');
  // exact: die Beschreibung "Empfohlener Standard." traefe sonst ebenfalls.
  await expect(qwen4.getByText("empfohlen", { exact: true })).toBeVisible();
  await expect(qwen4.getByText("stark bei Zusammenfassungen")).toBeVisible();
  await expect(qwen4.getByRole("button", { name: "Verwenden" })).toBeVisible();
  const qwen8 = page.locator('[data-llm-card="llm-qwen3-8b-q4"]');
  await expect(qwen8.getByRole("button", { name: "Verwenden" })).toHaveCount(0);
  await expect(qwen8.getByRole("button", { name: "Laden" })).toBeVisible();
});

test("using a model reaches the backend and marks it active", async ({ page }) => {
  await openModels(page);
  const qwen4 = page.locator('[data-llm-card="llm-qwen3-4b-q4"]');
  await qwen4.getByRole("button", { name: "Verwenden" }).click();
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { saved: Record<string, unknown> }).saved.activated))
    .toBe("llm-qwen3-4b-q4");
  await expect(qwen4.getByText("Aktiv", { exact: true })).toBeVisible();
  await expect(qwen4.getByRole("button", { name: "Verwenden" })).toHaveCount(0);
});

test("each model says whether it fits, with the numbers behind it", async ({ page }) => {
  await openModels(page);
  const qwen4 = page.locator('[data-llm-card="llm-qwen3-4b-q4"]');
  await expect(qwen4.locator("[data-fit=fits]")).toContainText("braucht ≈ 4,3 GB, frei 18,5 GB");
  const qwen8 = page.locator('[data-llm-card="llm-qwen3-8b-q4"]');
  await expect(qwen8.locator("[data-fit=tight]")).toContainText("Knapp");
});
