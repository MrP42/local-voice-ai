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
      llm_model_dirs: ["D:\\ollama\\models"],
      push_to_talk: true,
    };
    const saved: Record<string, unknown> = {};
    (window as unknown as { saved: Record<string, unknown> }).saved = saved;
    const downloads = [
      { id: "llm-runtime-windows-x64-vulkan", kind: "runtime", name: "Sprachmodell-Laufzeit (Windows, Vulkan)", description: "llama.cpp", size_mb: 30, is_downloaded: true, is_downloading: false, tags: [], backend: "vulkan", for_this_platform: true },
      { id: "llm-runtime-macos-aarch64", kind: "runtime", name: "Sprachmodell-Laufzeit (macOS)", description: "llama.cpp", size_mb: 11, is_downloaded: false, is_downloading: false, tags: [], backend: null, for_this_platform: false },
      { id: "llm-qwen3-4b-q4", kind: "model", name: "Qwen3 4B", description: "Empfohlener Standard.", size_mb: 2381, is_downloaded: true, is_downloading: false, tags: ["empfohlen", "zusammenfassung"], backend: null, for_this_platform: true, external: null, replaceable_by: "qwen3:4b" },
      { id: "llm-qwen3-8b-q4", kind: "model", name: "Qwen3 8B", description: "Mehr Qualität.", size_mb: 4795, is_downloaded: false, is_downloading: false, tags: ["qualitaet"], backend: null, for_this_platform: true, external: null, replaceable_by: null },
      // Aus Modellordnern: geprueft, ungeprueft, Ollama-Sonderformat.
      { id: "ext-aaaaaaaaaaaa", kind: "model", name: "qwen3.8:27b", description: "", size_mb: 16031, is_downloaded: true, is_downloading: false, tags: [], backend: null, for_this_platform: true, external: { source: "ollama", path: "D:\\ollama\\models\\blobs\\sha256-f5f1", compat: { state: "ok" } }, replaceable_by: null },
      { id: "ext-bbbbbbbbbbbb", kind: "model", name: "gpt-oss-20b-MXFP4", description: "", size_mb: 11548, is_downloaded: true, is_downloading: false, tags: [], backend: null, for_this_platform: true, external: { source: "folder", path: "C:\\Users\\x\\.lmstudio\\models\\gpt-oss-20b-MXFP4.gguf", compat: { state: "unchecked" } }, replaceable_by: null },
      { id: "ext-cccccccccccc", kind: "model", name: "qwen3.5:9b", description: "", size_mb: 6288, is_downloaded: true, is_downloading: false, tags: [], backend: null, for_this_platform: true, external: { source: "ollama", path: "D:\\ollama\\models\\blobs\\sha256-dec5", compat: { state: "incompatible", reason: "ollama_merged_vision" } }, replaceable_by: null },
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
          if (cmd === "llm_suggest_model_dirs") return ["D:\\ollama\\models", "C:\\Users\\x\\.lmstudio\\models"];
          if (cmd === "llm_set_model_dirs") {
            saved.dirs = args?.dirs;
            settings.llm_model_dirs = args?.dirs;
            return null;
          }
          if (cmd === "llm_rescan_model_dirs") return null;
          if (cmd === "llm_probe_external") {
            saved.probed = args?.id;
            const hit = downloads.find((d) => d.id === args?.id);
            if (hit?.external) hit.external.compat = { state: "ok" };
            return { state: "ok" };
          }
          if (cmd === "llm_local_delete") {
            saved.deleted = args?.id;
            return null;
          }
          if (cmd === "plugin:dialog|ask") return true;
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

// Modellordner (05.10.2026): Modelle aus Ollama/LM Studio erscheinen mit
// Herkunft und Ladestatus, fremde Dateien lassen sich nicht entfernen, und was
// die Laufzeit nicht laden kann, ist nicht waehlbar -- mit Grund.
test("models from model folders show source and load status", async ({ page }) => {
  await openModels(page);
  await expect(page.getByTestId("llm-model-dirs")).toContainText("D:\\ollama\\models");
  const ok = page.locator('[data-llm-card="ext-aaaaaaaaaaaa"]');
  await expect(ok.getByText("Ollama", { exact: true })).toBeVisible();
  await expect(ok.locator("[data-compat=ok]")).toHaveText("Geprüft");
  await expect(ok.getByRole("button", { name: "Verwenden" })).toBeVisible();
  await expect(ok.getByRole("button", { name: "Entfernen" })).toHaveCount(0);

  const blocked = page.locator('[data-llm-card="ext-cccccccccccc"]');
  await expect(blocked.locator("[data-compat=incompatible]")).toHaveText("Lädt nicht");
  await expect(blocked.locator("[data-incompatible-reason]")).toContainText("Verbindung „Ollama“");
  await expect(blocked.getByRole("button", { name: "Verwenden" })).toHaveCount(0);
});

test("an unchecked model can be checked from its card", async ({ page }) => {
  await openModels(page);
  const unchecked = page.locator('[data-llm-card="ext-bbbbbbbbbbbb"]');
  await expect(unchecked.locator("[data-compat=unchecked]")).toHaveText("Ungeprüft");
  await unchecked.getByRole("button", { name: "Prüfen" }).click();
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { saved: Record<string, unknown> }).saved.probed))
    .toBe("ext-bbbbbbbbbbbb");
  await expect(unchecked.locator("[data-compat=ok]")).toHaveText("Geprüft");
});

test("a detected folder is offered, not added on its own", async ({ page }) => {
  await openModels(page);
  const dirs = page.getByTestId("llm-model-dirs");
  // Ollama steht schon drin, also wird nur LM Studio vorgeschlagen.
  await expect(dirs.locator("[data-suggested-dir]")).toHaveCount(1);
  await dirs.locator('[data-suggested-dir="C:\\Users\\x\\.lmstudio\\models"]').click();
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { saved: Record<string, unknown> }).saved.dirs))
    .toEqual(["D:\\ollama\\models", "C:\\Users\\x\\.lmstudio\\models"]);
});

test("an app copy with a checked twin in a folder can be deleted after asking", async ({ page }) => {
  await openModels(page);
  const qwen4 = page.locator('[data-llm-card="llm-qwen3-4b-q4"]');
  const hint = qwen4.locator('[data-replaceable-by="qwen3:4b"]');
  await expect(hint).toContainText("„qwen3:4b“");
  await hint.getByRole("button", { name: /App-Kopie löschen/ }).click();
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { saved: Record<string, unknown> }).saved.deleted))
    .toBe("llm-qwen3-4b-q4");
  // Ohne Zwilling kein Angebot.
  await expect(page.locator('[data-llm-card="llm-qwen3-8b-q4"] [data-replaceable-by]')).toHaveCount(0);
});
