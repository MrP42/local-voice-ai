import { test, expect, type Page } from "@playwright/test";

// Issue #29 (Paket G2d): Stimmen werden nur angeboten, wenn ihre Laufzeit
// vollstaendig und startfaehig ist; sonst stehen sie als "nicht eingerichtet"
// da, mit Weg zum Einrichten. Nicht eingerichtete Fish-Speech-Funktionen
// bleiben aus Oberflaeche und Hilfe. Dazu Issue #7: Canary 1B traegt den
// Hinweis "nur nicht-kommerziell".

type Cfg = {
  engine: string;
  piperVoice: string | null;
  downloads: unknown[];
  runtime: unknown;
  models: unknown[];
};

const piperRow = (over: Record<string, unknown> = {}) => ({
  id: "piper-runtime",
  kind: "runtime",
  name: "Piper",
  description: "",
  language: null,
  size_mb: 20,
  is_downloaded: true,
  is_downloading: false,
  is_usable: true,
  unsupported_reason: null,
  license: "MIT AND GPL-3.0-or-later",
  license_url: "https://example.org/piper",
  license_non_commercial: false,
  ...over,
});
const voiceRow = (over: Record<string, unknown> = {}) => ({
  id: "de_DE-thorsten-medium",
  kind: "voice",
  name: "Thorsten (Deutsch)",
  description: "",
  language: "de",
  size_mb: 60,
  is_downloaded: true,
  is_downloading: false,
  is_usable: true,
  unsupported_reason: null,
  license: "CC0-1.0",
  license_url: "https://example.org/thorsten",
  license_non_commercial: false,
  ...over,
});
const runtimeState = (piperReady: boolean, fishReady: boolean, fishSupported = true) => ({
  platform: "windows-x64",
  piper: { supported: true, ready: piperReady, missing: piperReady ? [] : ["espeak-ng.dll"] },
  fish: {
    supported: fishSupported,
    ready: fishReady,
    missing: fishReady || !fishSupported ? [] : [".venv/Scripts/python.exe", "tools/api_server.py"],
  },
  fish_dir: "C:\\AI\\fish-speech",
});

async function setup(page: Page, cfg: Cfg) {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript((cfg: Cfg) => {
    const callbacks = new Map<number, unknown>();
    let callback = 0;
    const w = window as unknown as Record<string, unknown>;
    w.__calls = [] as unknown[];
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
      push_to_talk: true,
      tts_voice: null,
      tts_engine: cfg.engine,
      tts_piper_voice: cfg.piperVoice,
      tts_fish_dir: "C:\\AI\\fish-speech",
    };
    Object.assign(window, {
      __TAURI_OS_PLUGIN_INTERNALS__: { platform: "windows", os_type: "windows", family: "windows", arch: "x86_64", version: "10.0.26200", eol: "\r\n" },
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
      __TAURI_INTERNALS__: {
        metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
        transformCallback: (fn: unknown) => {
          callbacks.set(++callback, fn);
          return callback;
        },
        unregisterCallback: (id: number) => callbacks.delete(id),
        convertFileSrc: (path: string) => path,
        invoke: async (cmd: string, args?: Record<string, unknown>) => {
          (w.__calls as unknown[]).push([cmd, args]);
          if (cmd === "get_app_settings" || cmd === "get_default_settings") return settings;
          if (cmd === "plugin:os|locale") return "de-DE";
          if (cmd === "plugin:app|version") return "0.20.10";
          if (cmd.includes("permission")) return true;
          if (cmd === "plugin:event|listen") return ++callback;
          if (cmd === "get_selected_model") return "";
          if (cmd === "meetings_is_recording") return false;
          if (cmd === "tts_server_status") return { phase: "stopped", message: null };
          if (cmd === "tts_runtime_status") return cfg.runtime;
          if (cmd === "tts_list_downloads") return cfg.downloads;
          if (cmd === "get_available_models") return cfg.models;
          if (cmd === "tts_list_voices" || cmd === "tts_list_voice_infos") return [];
          if (cmd === "pages_list") return [{ id: "p1", title: "Seite" }];
          if (cmd === "get_custom_sounds") return { start: false, stop: false };
          if (cmd === "tts_download_model") return null;
          if (cmd === "change_tts_engine_setting") {
            w.savedEngine = args?.value;
            return null;
          }
          if (cmd.includes("history") || cmd.includes("models") || cmd.includes("devices") || cmd === "meetings_list" || cmd === "llm_ps" || cmd === "tts_reading_list" || cmd === "page_files")
            return [];
          return null;
        },
      },
    });
  }, cfg);
}

const openReadAloud = async (page: Page) => {
  await page.goto("/");
  await page.getByRole("navigation").getByRole("button", { name: "Vorlesen", exact: true }).click();
  await expect(page.getByTestId("voice-select")).toBeVisible();
};

test("fully set up Piper voice is offered; unset Fish voices and server icon stay away", async ({ page }) => {
  await setup(page, {
    engine: "piper",
    piperVoice: "de_DE-thorsten-medium",
    downloads: [piperRow(), voiceRow()],
    runtime: runtimeState(true, false),
    models: [],
  });
  await openReadAloud(page);
  const select = page.getByTestId("voice-select");
  await expect(select).toContainText("Thorsten · Deutsch · MQ · Piper");
  await select.click();
  await expect(page.getByText("Thorsten · Deutsch · MQ · Piper").last()).toBeVisible();
  // Fish nicht eingerichtet: keine Skript-Stimmen in der Liste ...
  await expect(page.getByText("Skript mit Sprechern")).toHaveCount(0);
  await page.keyboard.press("Escape");
  // ... kein Fish-Server-Symbol und kein Hinweis, die gewaehlte Stimme laufe nicht.
  await expect(page.getByRole("button", { name: /Fish-Speech-Server/ })).toHaveCount(0);
  await expect(page.getByTestId("voice-setup-hint")).toHaveCount(0);
});

test("a downloaded Piper voice without a complete runtime is marked not set up, with a way to set it up", async ({ page }) => {
  await setup(page, {
    engine: "piper",
    piperVoice: "de_DE-thorsten-medium",
    downloads: [piperRow({ is_downloaded: false, is_usable: false }), voiceRow({ is_usable: false })],
    runtime: runtimeState(false, false),
    models: [],
  });
  await openReadAloud(page);
  const select = page.getByTestId("voice-select");
  await expect(select).toContainText("nicht eingerichtet");
  await select.click();
  const option = page.getByText("Thorsten · Deutsch · MQ · Piper – nicht eingerichtet").last();
  await expect(option).toBeVisible();
  await page.keyboard.press("Escape");
  const hint = page.getByTestId("voice-setup-hint");
  await expect(hint).toContainText("Noch keine Sprachausgabe eingerichtet");
  // Der Weg zum Einrichten fuehrt auf die Modelle-Seite.
  await page.getByTestId("voice-setup-button").click();
  await expect(page.getByText("Vorlesestimmen", { exact: true }).first()).toBeVisible();
  const card = page.locator("div.px-4.py-3", { hasText: "Thorsten" }).first();
  await expect(card.getByText("Programm fehlt – nicht nutzbar")).toBeVisible();
  await card.getByRole("button", { name: "Programm installieren" }).click();
  const calls = await page.evaluate(() => (window as unknown as { __calls: [string, Record<string, unknown>][] }).__calls);
  expect(calls.some(([cmd, args]) => cmd === "tts_download_model" && args?.id === "piper-runtime")).toBe(true);
});

test("with Fish set up, script voices and the server icon stay as before", async ({ page }) => {
  await setup(page, {
    engine: "fish",
    piperVoice: null,
    downloads: [piperRow(), voiceRow()],
    runtime: runtimeState(true, true),
    models: [],
  });
  await openReadAloud(page);
  const select = page.getByTestId("voice-select");
  await expect(select).toContainText("Skript mit Sprechern");
  await select.click();
  await expect(page.getByText("Thorsten · Deutsch · MQ · Piper").last()).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("voice-setup-hint")).toHaveCount(0);
});

test("Fish selected but not set up: named as not set up, never swapped silently", async ({ page }) => {
  await setup(page, {
    engine: "fish",
    piperVoice: null,
    downloads: [piperRow(), voiceRow()],
    runtime: runtimeState(true, false),
    models: [],
  });
  await openReadAloud(page);
  await expect(page.getByTestId("voice-select")).toContainText("Fish Speech – nicht eingerichtet");
  await expect(page.getByTestId("voice-setup-hint")).toContainText("nicht eingerichtet");
  const engineCalls = await page.evaluate(() => (window as unknown as { savedEngine?: unknown }).savedEngine);
  expect(engineCalls, "die Engine wird nicht still umgeschaltet").toBeUndefined();
});

test("settings: without Fish only a compact optional entry remains, with Fish the details return", async ({ page }) => {
  await setup(page, {
    engine: "piper",
    piperVoice: "de_DE-thorsten-medium",
    downloads: [piperRow(), voiceRow()],
    runtime: runtimeState(true, false),
    models: [],
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Einstellungen", exact: true }).first().click();
  await page.getByRole("tab", { name: "Vorlesen", exact: true }).click();
  await expect(page.getByText("Fish Speech (optional, nicht eingerichtet)")).toBeVisible();
  await expect(page.getByTestId("fish-dir-input")).toBeVisible();
  await expect(page.getByText("Server-Port")).toHaveCount(0);
  await expect(page.getByText("Leerlauf-Stopp", { exact: false })).toHaveCount(0);
});

test("settings: with Fish set up the port and server details are shown", async ({ page }) => {
  await setup(page, {
    engine: "fish",
    piperVoice: null,
    downloads: [piperRow(), voiceRow()],
    runtime: runtimeState(true, true),
    models: [],
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Einstellungen", exact: true }).first().click();
  await page.getByRole("tab", { name: "Vorlesen", exact: true }).click();
  await expect(page.getByText("Server-Port")).toBeVisible();
  await expect(page.getByText("Fish Speech (optional, nicht eingerichtet)")).toHaveCount(0);
});

test("models: an unsupported Piper runtime is not offered for download, with the reason", async ({ page }) => {
  await setup(page, {
    engine: "piper",
    piperVoice: null,
    downloads: [
      piperRow({ is_downloaded: false, is_usable: false, unsupported_reason: "incomplete_archive" }),
      voiceRow({ is_downloaded: false, is_usable: false }),
    ],
    runtime: { ...runtimeState(false, false), platform: "macos-x64" },
    models: [],
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Modelle", exact: true }).last().click();
  const note = page.getByTestId("tts-unsupported-note");
  await expect(note).toContainText("unvollständig");
  const runtimeCard = page.locator("div.px-4.py-3", { has: note });
  await expect(runtimeCard.getByRole("button", { name: "Herunterladen" })).toHaveCount(0);
  await expect(runtimeCard.getByText("Auf diesem System nicht verfügbar")).toBeVisible();
});

test("models: non-commercial Piper voices and Canary 1B carry a clear license warning", async ({ page }) => {
  const canary = {
    id: "handy-computer/canary-1b-gguf/canary-1b-Q5_K_M.gguf",
    name: "Canary 1B",
    description: "4-language speech-to-text with translation.",
    filename: "canary-1b-Q5_K_M.gguf",
    source: { HuggingFace: { repo_id: "handy-computer/canary-1b-gguf", revision: "x" } },
    size_mb: 700,
    is_downloaded: false,
    is_downloading: false,
    partial_size: 0,
    is_directory: false,
    engine_type: "TranscribeCpp",
    accuracy_score: 0.5,
    speed_score: 0.5,
    supports_translation: true,
    is_recommended: false,
    supported_languages: ["en", "de"],
    supports_language_selection: true,
    is_custom: false,
    supports_streaming: false,
    supports_language_detection: false,
    supports_stream_lookahead: false,
    license: "cc-by-nc-4.0",
    license_url: "https://creativecommons.org/licenses/by-nc/4.0/",
    license_non_commercial: true,
  };
  const whisper = { ...canary, id: "handy-computer/whisper-small-gguf/w.gguf", name: "Whisper Small", license: "apache-2.0", license_non_commercial: false };
  await setup(page, {
    engine: "piper",
    piperVoice: null,
    downloads: [
      piperRow({ is_downloaded: false, is_usable: false }),
      voiceRow({ id: "en_US-ryan-high", name: "Ryan (High Quality)", language: "en", is_downloaded: false, is_usable: false, license: "CC-BY-NC-SA-4.0", license_non_commercial: true }),
    ],
    runtime: runtimeState(false, false),
    models: [canary, whisper],
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Modelle", exact: true }).last().click();
  const note = page.getByTestId("model-nc-note");
  await expect(note).toHaveCount(1);
  await expect(note).toContainText("CC-BY-NC-4.0");
  await expect(note).toContainText("nur für nicht-kommerzielle Nutzung");
  const canaryCard = page.locator('div[role="button"]', { hasText: "Canary 1B" }).first();
  await expect(canaryCard.getByText("Nur nicht-kommerziell").first()).toBeVisible();
  const whisperCard = page.locator('div[role="button"]', { hasText: "Whisper Small" }).first();
  await expect(whisperCard.getByText("Nur nicht-kommerziell")).toHaveCount(0);
  // Auch die nicht-kommerzielle Stimme warnt.
  await expect(page.getByTestId("tts-nc-note")).toContainText("CC-BY-NC-SA-4.0");
});

test("help hides Fish content when Fish is not set up and shows it when it is", async ({ page }) => {
  await setup(page, {
    engine: "piper",
    piperVoice: "de_DE-thorsten-medium",
    downloads: [piperRow(), voiceRow()],
    runtime: runtimeState(true, false),
    models: [],
  });
  await openReadAloud(page);
  await page.getByRole("button", { name: /Hilfe/ }).first().click();
  const help = page.getByTestId("help-panel").first();
  await expect(help).toContainText("Sprachausgabe einrichten");
  await expect(help).not.toContainText("Zwei Engines");
  await expect(help).not.toContainText("Fish-Speech-Server");
});

test("help shows the Fish sections once Fish is set up", async ({ page }) => {
  await setup(page, {
    engine: "fish",
    piperVoice: null,
    downloads: [piperRow(), voiceRow()],
    runtime: runtimeState(true, true),
    models: [],
  });
  await openReadAloud(page);
  await page.getByRole("button", { name: /Hilfe/ }).first().click();
  const help = page.getByTestId("help-panel").first();
  await expect(help).toContainText("Zwei Engines");
  await expect(help).not.toContainText("Sprachausgabe einrichten");
});
