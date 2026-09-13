import { test, expect } from "@playwright/test";

// Die Anbieter-Einrichtung mit echten Eintraegen: eine Verbindung mit einem
// freigegebenen Modell, eine zweite ohne. Geprueft wird, was der Nutzer
// verlangt hat -- kein Aus-Schalter mehr, nur freigegebene Modelle in der
// Auswahl, Freigabe und Schluessel landen wirklich im Backend.
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
        transcribe: {
          id: "transcribe",
          name: "Diktat",
          description: "",
          current_binding: "Ctrl+Space",
          default_binding: "Ctrl+Space",
        },
        transcribe_with_post_process: {
          id: "transcribe_with_post_process",
          name: "Diktat mit Nachbearbeitung",
          description: "",
          current_binding: "Ctrl+Shift+Space",
          default_binding: "Ctrl+Shift+Space",
        },
      },
      post_process_enabled: false,
      post_process_providers: [
        { id: "openai", label: "OpenAI", base_url: "https://api.openai.com/v1", allow_base_url_edit: false, models_endpoint: "/models", supports_structured_output: true },
        { id: "ollama", label: "Ollama (lokal)", base_url: "http://127.0.0.1:11434/v1", allow_base_url_edit: true, models_endpoint: "/models", supports_structured_output: false },
      ],
      post_process_prompts: [],
      post_process_selected_prompt_id: null,
      custom_words: [],
      post_process_models: {},
      post_process_api_keys: { "openai-1": "sk-alt" },
      llm_connections: [
        { id: "openai-1", kind: "openai", label: "OpenAI", base_url: "https://api.openai.com/v1", enabled: true },
        { id: "ollama-1", kind: "ollama", label: "Ollama (lokal)", base_url: "http://127.0.0.1:11434/v1", enabled: true },
      ],
      llm_models: [
        { id: "openai-1:gpt-4.1-mini", connection_id: "openai-1", remote_id: "gpt-4.1-mini", label: "gpt-4.1-mini", enabled: true, tags: [] },
      ],
      llm_active_model_id: "openai-1:gpt-4.1-mini",
      push_to_talk: true,
    };
    const saved: Record<string, unknown> = {};
    (window as unknown as { saved: Record<string, unknown> }).saved = saved;
    Object.assign(window, {
      __TAURI_OS_PLUGIN_INTERNALS__: {
        platform: "windows", os_type: "windows", family: "windows",
        arch: "x86_64", version: "10.0.26200", eol: "\r\n",
      },
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
      __TAURI_INTERNALS__: {
        metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
        transformCallback: (fn: unknown) => { callbacks.set(++callback, fn); return callback; },
        unregisterCallback: (id: number) => callbacks.delete(id),
        convertFileSrc: (path: string) => path,
        invoke: async (cmd: string, args?: Record<string, unknown>) => {
          if (cmd === "get_app_settings" || cmd === "get_default_settings") return settings;
          if (cmd === "plugin:os|locale") return "de-DE";
          if (cmd === "plugin:app|version") return "0.16.2";
          if (cmd.includes("permission")) return true;
          if (cmd === "plugin:event|listen") return ++callback;
          if (cmd === "get_selected_model") return "";
          if (cmd === "meetings_is_recording") return false;
          if (cmd === "tts_server_status") return { phase: "stopped", message: null };
          // Der Anbieter meldet drei Modelle; freigegeben ist eines.
          if (cmd === "llm_list_remote_models")
            return args?.connectionId === "ollama-1"
              ? ["qwen3:4b", "gemma3:4b", "nomic-embed-text"]
              : ["gpt-4.1-mini", "gpt-4.1"];
          if (cmd === "llm_upsert_model") {
            const model = args?.model as { connection_id: string; remote_id: string };
            const id = `${model.connection_id}:${model.remote_id}`;
            saved.upsertModel = id;
            const list = settings.llm_models as unknown[];
            (settings.llm_models as unknown[]) = [
              ...list.filter((m) => (m as { id: string }).id !== id),
              { ...model, id, label: model.remote_id, enabled: true, tags: [] },
            ];
            return null;
          }
          if (cmd === "llm_set_active_model") { saved.active = args?.id; settings.llm_active_model_id = args?.id; return null; }
          if (cmd === "llm_set_api_key") { saved.apiKey = [args?.connectionId, args?.apiKey]; return null; }
          if (cmd === "llm_upsert_connection") { saved.connection = args?.connection; return null; }
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

async function openTab(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByRole("button", { name: "Einstellungen", exact: true }).last().click();
  await page.getByRole("tab", { name: "KI-Textverbesserung", exact: true }).click();
}

test("the polish feature has no off switch any more", async ({ page }) => {
  await openTab(page);
  // post_process_enabled ist im Mock false -- frueher verschwand damit alles.
  await expect(page.getByText("Verbindungen", { exact: true })).toBeVisible();
  await expect(page.getByText("Aktives Modell", { exact: true })).toBeVisible();
  await expect(page.getByRole("switch", { name: /Nachbearbeitung aktivieren|Post-processing/i })).toHaveCount(0);
});

test("only released models are offered as active model", async ({ page }) => {
  await openTab(page);
  const active = page.locator(".w-72").first();
  await expect(active).toContainText("gpt-4.1-mini");
  await active.click();
  // Die zweite Verbindung hat nichts freigegeben -- nichts von ihr im Menue.
  await expect(page.getByText("qwen3:4b")).toHaveCount(0);
  await page.keyboard.press("Escape");
});

test("releasing a model from the provider list reaches the backend", async ({ page }) => {
  await openTab(page);
  // Die Karte der Verbindung: der Rahmen, der ihren Aufklapp-Knopf enthaelt.
  // Innerste Karte: der aeussere Gruppenrahmen traegt dieselben Klassen und
  // enthaelt den Knopf ebenfalls -- `.last()` ist die Karte selbst.
  const card = page
    .locator(".rounded-md.border", {
      has: page.getByRole("button", { name: "Ollama (lokal)" }),
    })
    .last();
  await page.getByRole("button", { name: "Ollama (lokal)" }).click();
  await page.getByRole("button", { name: "Modelle laden" }).click();
  await expect(page.getByText("nomic-embed-text")).toBeVisible();
  await page.getByLabel("qwen3:4b").check();
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { saved: Record<string, unknown> }).saved.upsertModel))
    .toBe("ollama-1:qwen3:4b");
  // Danach steht es im Bereich der freigegebenen Modelle dieser Verbindung.
  // exact: der Hinweis "Nur freigegebene Modelle erscheinen …" traefe sonst
  // mit -- getByText sucht Teilzeichenfolgen ohne Gross-/Kleinschreibung.
  await expect(
    card.getByText("Freigegebene Modelle", { exact: true }),
  ).toBeVisible();
  await expect(card.getByText("qwen3:4b").last()).toBeVisible();
});

test("the api key is stored under the connection, not the template", async ({ page }) => {
  await openTab(page);
  await page.getByRole("button", { name: "OpenAI", exact: true }).click();
  const key = page.locator('input[type="password"]').first();
  await key.fill("sk-neu");
  await key.blur();
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { saved: Record<string, unknown> }).saved.apiKey))
    .toEqual(["openai-1", "sk-neu"]);
});
