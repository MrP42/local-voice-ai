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
        { id: "ol", kind: "ollama", label: "Ollama", base_url: "http://127.0.0.1:11434/v1", enabled: true },
      ],
      llm_models: [
        { id: "local:llm-qwen3-4b-q4", connection_id: "local", remote_id: "llm-qwen3-4b-q4", label: "Qwen3 4B", enabled: true, tags: [] },
        { id: "openai-1:gpt-4.1-mini", connection_id: "openai-1", remote_id: "gpt-4.1-mini", label: "gpt-4.1-mini", enabled: true, tags: [], fast: true },
        { id: "aus:qwen3:8b", connection_id: "aus", remote_id: "qwen3:8b", label: "qwen3:8b", enabled: true, tags: [] },
        { id: "ol:qwen3.8:27b", connection_id: "ol", remote_id: "qwen3.8:27b", label: "qwen3.8:27b", enabled: true, tags: [] },
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
          if (cmd === "tts_server_status") return { phase: "stopped", owns_server: false, message: null };
          if (cmd === "tts_server_start") { saved.serverStarted = true; return null; }
          if (cmd === "compliance_status")
            return { level: "green", profile: "eu", active_model: "Qwen3 4B", active: null, checks: [], plaintext_keys: 0 };
          if (cmd === "llm_ps") return (window as unknown as { __ollamaLoaded?: string[] }).__ollamaLoaded ?? [];
          if (cmd === "llm_warm") { saved.warm = true; return "qwen3.8:27b"; }
          if (cmd === "llm_unload") { saved.unload = true; return 1; }
          if (cmd === "llm_local_status") return { phase: "ready", model_id: "llm-qwen3-4b-q4", backend: "vulkan", port: 18477, message: null };
          if (cmd === "llm_set_active_model") { saved.active = args?.id; settings.llm_active_model_id = args?.id; return null; }
          if (cmd === "system_memory")
            return (window as unknown as { __memOverride?: unknown }).__memOverride ?? { ram_total_mb: 65536, ram_used_mb: 12595, gpus: [
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

test("a model in fast mode carries a lightning bolt in the footer menu", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto("/");
  await expect(page.locator("[data-llm-selector]")).not.toContainText("⚡");
  await page.locator("[data-llm-selector]").click();
  const menu = page.getByRole("menu");
  await expect(menu.locator("[data-llm-fast]")).toHaveCount(1);
  await expect(menu.getByRole("menuitem").filter({ has: page.locator("[data-llm-fast]") })).toContainText("GPT-4.1-Mini");
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

const GPUS = [
  { name: "Intel UHD 770", budget_mb: 32768, used_mb: 512, dedicated_mb: 128, shared: true },
  { name: "NVIDIA GeForce RTX 4090", budget_mb: 23160, used_mb: 920, dedicated_mb: 24356, shared: false },
];

test("the footer adds the app's own share of RAM and GPU memory", async ({ page }) => {
  await page.addInitScript((gpus) => {
    (window as unknown as { __memOverride: unknown }).__memOverride = {
      ram_total_mb: 65536, ram_used_mb: 32358, gpus, app_ram_mb: 4300, app_gpu_mb: 820,
    };
  }, GPUS);
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto("/");
  const meter = page.locator("[data-resource-meter]");
  await expect(meter).toContainText("RAM 31,6 / 64,0 GB (App 4,2)");
  await expect(meter).toContainText("GPU 0,9 / 22,6 GB (App 0,8)");
  // Der Tooltip erklaert, was "App" umfasst.
  await expect(meter).toHaveAttribute("title", /App = Local Voice AI inkl\. Modell-Server/);
});

test("without a measurable app VRAM only the app RAM is added, never an invented zero", async ({ page }) => {
  await page.addInitScript((gpus) => {
    (window as unknown as { __memOverride: unknown }).__memOverride = {
      ram_total_mb: 65536, ram_used_mb: 32358, gpus, app_ram_mb: 4300, app_gpu_mb: null,
    };
  }, GPUS);
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto("/");
  const meter = page.locator("[data-resource-meter]");
  await expect(meter).toContainText("RAM 31,6 / 64,0 GB (App 4,2)");
  await expect(meter).toContainText("GPU 0,9 / 22,6 GB");
  await expect(meter).not.toContainText("GPU 0,9 / 22,6 GB (App");
});

test("the Fish server sits in the footer left of the shield and asks before starting", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto("/");
  const server = page.locator("[data-fish-server]");
  await expect(server).toHaveAttribute("data-fish-server", "stopped");
  const shield = page.locator("[data-compliance-shield]");
  const [s, c] = [await server.boundingBox(), await shield.boundingBox()];
  expect(s && c && s.x < c.x && Math.abs(s.y - c.y) < 20).toBeTruthy();
  await server.click();
  await expect(page.getByRole("dialog")).toContainText("Fish-Speech-Server starten?");
  await page.getByTestId("fish-server-start").click();
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { saved: Record<string, unknown> }).saved.serverStarted))
    .toBe(true);
});

test("an active Ollama model can be warmed and unloaded from the footer menu", async ({ page }) => {
  await page.addInitScript(() => {
    (window as unknown as { __ollamaLoaded: string[] }).__ollamaLoaded = ["qwen3.8:27b"];
  });
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto("/");
  await page.locator("[data-llm-selector]").click();
  await page.locator('[data-llm-option="ol:qwen3.8:27b"]').click();
  await page.locator("[data-llm-selector]").click();
  const section = page.locator("[data-llm-ollama]");
  await expect(section).toContainText("Ollama hält im Speicher: qwen3.8:27b");
  await section.locator("[data-llm-unload-ollama]").click();
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { saved: Record<string, unknown> }).saved.unload))
    .toBe(true);
  await page.locator("[data-llm-selector]").click();
  await page.locator("[data-llm-warm]").click();
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { saved: Record<string, unknown> }).saved.warm))
    .toBe(true);
});

test("the read-aloud page no longer carries the server and model icons", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto("/");
  await page.getByRole("button", { name: "Vorlesen", exact: true }).first().click();
  await expect(page.locator("#page-title")).toHaveText("Vorlesen");
  await expect(page.locator("main [data-fish-server]")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Fish-Speech-Server starten" })).toHaveCount(1);
});

test("screenshots for review (only with SCREENS_DIR)", async ({ page }) => {
  const dir = process.env.SCREENS_DIR;
  test.skip(!dir, "nur fuer Abnahmebilder");
  await page.addInitScript(() => {
    (window as unknown as { __ollamaLoaded: string[] }).__ollamaLoaded = ["qwen3.8:27b"];
  });
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto("/");
  await page.getByRole("button", { name: "Vorlesen", exact: true }).first().click();
  await expect(page.locator("#page-title")).toHaveText("Vorlesen");
  await page.screenshot({ path: `${dir}/vorlesen-kopf-fussleiste.png`, animations: "disabled" });
  await page.locator("[data-llm-selector]").click();
  await page.locator('[data-llm-option="ol:qwen3.8:27b"]').click();
  await page.locator("[data-llm-selector]").click();
  await expect(page.locator("[data-llm-ollama]")).toBeVisible();
  await page.screenshot({ path: `${dir}/fussleiste-sprachmodell-menue.png`, animations: "disabled" });
});
