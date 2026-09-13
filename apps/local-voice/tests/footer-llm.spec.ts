import { test, expect } from "@playwright/test";

// Die Fussleiste mit Sprachmodell-Auswahl und Auslastung: das aktive Modell
// mit Ampel, Wechsel zwischen freigegebenen Modellen, und RAM/GPU als Zahlen,
// die aus dem Backend kommen -- nicht erfunden.
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
      bindings: { transcribe: { id: "transcribe", name: "Diktat", description: "", current_binding: "Ctrl+Space", default_binding: "Ctrl+Space" } },
      post_process_providers: [],
      post_process_prompts: [],
      custom_words: [],
      post_process_models: {},
      post_process_api_keys: {},
      llm_connections: [
        { id: "local", kind: "local", label: "In der App", base_url: "http://127.0.0.1:0/v1", enabled: true },
        { id: "openai-1", kind: "openai", label: "OpenAI", base_url: "https://api.openai.com/v1", enabled: true },
        { id: "aus", kind: "ollama", label: "Ollama aus", base_url: "http://127.0.0.1:11434/v1", enabled: false },
      ],
      llm_models: [
        { id: "local:llm-qwen3-4b-q4", connection_id: "local", remote_id: "llm-qwen3-4b-q4", label: "Qwen3 4B", enabled: true, tags: [] },
        { id: "openai-1:gpt-4.1-mini", connection_id: "openai-1", remote_id: "gpt-4.1-mini", label: "gpt-4.1-mini", enabled: true, tags: [] },
        { id: "aus:qwen3:8b", connection_id: "aus", remote_id: "qwen3:8b", label: "qwen3:8b", enabled: true, tags: [] },
      ],
      llm_active_model_id: "local:llm-qwen3-4b-q4",
      push_to_talk: true,
    };
    const saved: Record<string, unknown> = {};
    (window as unknown as { saved: Record<string, unknown> }).saved = saved;
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
          if (cmd === "plugin:app|version") return "0.16.4";
          if (cmd.includes("permission")) return true;
          if (cmd === "plugin:event|listen") return ++callback;
          if (cmd === "get_selected_model") return "";
          if (cmd === "meetings_is_recording") return false;
          if (cmd === "tts_server_status") return { phase: "stopped", message: null };
          if (cmd === "llm_local_status") return { phase: "ready", model_id: "llm-qwen3-4b-q4", backend: "vulkan", port: 18477, message: null };
          if (cmd === "llm_set_active_model") { saved.active = args?.id; settings.llm_active_model_id = args?.id; return null; }
          if (cmd === "system_memory")
            return { ram_total_mb: 65536, ram_used_mb: 12595, gpus: [
              { name: "Intel UHD 770", budget_mb: 32768, used_mb: 512, dedicated_mb: 128, shared: true },
              { name: "NVIDIA GeForce RTX 4090", budget_mb: 23160, used_mb: 4240, dedicated_mb: 24356, shared: false },
            ] };
          if (cmd === "tts_list_voices" || cmd === "tts_list_voice_infos" || cmd === "llm_ps" || cmd === "tts_reading_list" || cmd === "pages_list" || cmd === "page_files" || cmd === "tts_list_downloads" || cmd === "llm_local_list")
            return [];
          if (cmd.includes("history") || cmd.includes("models") || cmd.includes("devices") || cmd === "meetings_list") return [];
          if (cmd === "get_custom_sounds") return { start: false, stop: false };
          return null;
        },
      },
    });
  });
});

test("the footer shows the active language model with a green light when loaded", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto("/");
  const selector = page.locator("[data-llm-selector]");
  await expect(selector).toContainText("Qwen3 4B");
  await expect(selector.locator("div.rounded-full")).toHaveClass(/bg-green-400/);
});

test("switching in the footer offers only released models of enabled connections", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto("/");
  await page.locator("[data-llm-selector]").click();
  const menu = page.getByRole("menu");
  await expect(menu.getByText("gpt-4.1-mini")).toBeVisible();
  // Die abgeschaltete Verbindung bleibt draussen.
  await expect(menu.getByText("qwen3:8b")).toHaveCount(0);
  // Ein lokales Modell im Speicher laesst sich hier entladen.
  await expect(menu.getByText("Aus dem Speicher entladen")).toBeVisible();
  await menu.getByText("gpt-4.1-mini").click();
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { saved: Record<string, unknown> }).saved.active))
    .toBe("openai-1:gpt-4.1-mini");
});

test("memory is shown as measured: RAM and the dedicated GPU, not the iGPU", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto("/");
  const meter = page.locator("[data-resource-meter]");
  await expect(meter).toContainText("RAM 12,3 / 64,0 GB");
  // Die dedizierte Karte zaehlt, nicht die iGPU mit "gemeinsamem" Speicher.
  await expect(meter).toContainText("GPU 4,1 / 22,6 GB");
  await expect(meter).not.toContainText("gemeinsam");
});
