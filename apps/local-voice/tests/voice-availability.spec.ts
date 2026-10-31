import { test, expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

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

// Anbieterliste fuer die Verwaltung: die echten Daten, unveraendert.
const PROVIDERS = JSON.parse(
  readFileSync(resolve(process.cwd(), "tests", "fixtures", "tts-providers-real.json"), "utf8"),
) as unknown[];

async function setup(page: Page, cfg: Cfg) {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript((arg: { cfg: Cfg; providers: unknown[] }) => {
    const { cfg, providers } = arg;
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
          if (cmd === "tts_providers_list") return w.__withProviders ? providers : null;
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
  }, { cfg, providers: PROVIDERS });
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
  // Der Weg zum Einrichten fuehrt auf die Modelle-Seite, gleich in den
  // Bereich Vorlesen.
  await page.getByTestId("voice-setup-button").click();
  await expect(page.getByRole("tab", { name: "Vorlesen" })).toHaveAttribute("aria-selected", "true");
  // Die Stimme steht als Karte in der Stimmenverwaltung; der Weg zum Einrichten steht auf der Karte.
  const card = page.locator("[data-voice-card]", { hasText: "Thorsten" }).first();
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
  await page.getByRole("tab", { name: "Ausgabe", exact: true }).click();
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
  await page.getByRole("tab", { name: "Ausgabe", exact: true }).click();
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
  await openModelsArea(page, "Vorlesen", "piper");
  await openGroup(page, "tts-available");
  const runtimeCard = page.locator('[data-tts-download="piper-runtime"]');
  await expect(runtimeCard.getByRole("button", { name: "Herunterladen" })).toHaveCount(0);
  await expect(runtimeCard.getByText("Auf diesem System nicht verfügbar")).toBeVisible();
  await openRow(runtimeCard);
  await expect(page.getByTestId("tts-unsupported-note")).toContainText("unvollständig");
});

/** Modellseite, gewuenschter Bereich (seit 06.10.2026 drei Bereiche). */
async function openModelsArea(page: import("@playwright/test").Page, area: string, sheet?: string) {
  // Die Anbieterliste liefert die Attrappe nur, wo die Verwaltung sie braucht (sonst bliebe die Vorlesen-Seite wie zuvor).
  if (sheet) await page.addInitScript(() => ((window as unknown as { __withProviders: boolean }).__withProviders = true));
  await page.goto("/");
  await page.getByRole("button", { name: "Modelle", exact: true }).last().click();
  await page.getByRole("tab", { name: area }).click();
  if (sheet) await openProviderSheet(page, sheet);
}
/** Piper-/Phonon-Downloads stehen seit der Stimmenverwaltung im Seitenblatt ihres Anbieters (Ansicht "Anbieter"). */
async function openProviderSheet(page: import("@playwright/test").Page, id: string) {
  const close = page.getByTestId("voice-provider-sheet-close");
  if (await close.isVisible()) {
    await close.click();
    await expect(page.getByTestId("voice-provider-sheet")).toHaveCount(0);
  }
  await page.getByTestId("voice-view-providers").click();
  await page.getByTestId(`tts-provider-open-${id}`).click();
  await expect(page.getByTestId("voice-provider-sheet")).toBeVisible();
}
async function openGroup(page: import("@playwright/test").Page, id: string) {
  const header = page.locator(`[data-model-section="${id}"] > div button[aria-expanded]`).first();
  if ((await header.getAttribute("aria-expanded")) === "false") await header.click();
}
async function openRow(row: import("@playwright/test").Locator) {
  const toggle = row.locator("button[aria-expanded]").first();
  if ((await toggle.getAttribute("aria-expanded")) === "false") await toggle.click();
}

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
  await page.addInitScript(() => ((window as unknown as { __withProviders: boolean }).__withProviders = true));
  await openModelsArea(page, "Diktat");
  await openGroup(page, "asr-available");
  // Das Kennzeichen steht schon in der zugeklappten Zeile ...
  const canaryCard = page.locator(`[data-asr-model="${canary.id}"]`);
  await expect(canaryCard.getByText("Nur nicht-kommerziell").first()).toBeVisible();
  const whisperCard = page.locator(`[data-asr-model="${whisper.id}"]`);
  await expect(whisperCard.getByText("Nur nicht-kommerziell")).toHaveCount(0);
  // ... die Begruendung im Detail.
  await openRow(canaryCard);
  const note = page.getByTestId("model-nc-note");
  await expect(note).toHaveCount(1);
  await expect(note).toContainText("CC-BY-NC-4.0");
  await expect(note).toContainText("nur für nicht-kommerzielle Nutzung");
  // Auch die nicht-kommerzielle Stimme warnt.
  await page.getByRole("tab", { name: "Vorlesen" }).click();
  await openProviderSheet(page, "piper");
  await openGroup(page, "tts-available");
  const ryan = page.locator('[data-tts-download="en_US-ryan-high"]');
  await expect(ryan.getByText("Nur nicht-kommerziell")).toBeVisible();
  await openRow(ryan);
  await expect(page.getByTestId("tts-nc-note")).toContainText("CC-BY-NC-SA-4.0");
});

test("models: Piper voices derived from a research/NC voice are marked non-commercial and say why (K1)", async ({ page }) => {
  await setup(page, {
    engine: "piper",
    piperVoice: null,
    downloads: [
      piperRow({ is_downloaded: false, is_usable: false }),
      // Datensatz CC0, aber vom Lessac-Modell (Blizzard-2013-Forschungslizenz) abgeleitet.
      voiceRow({ id: "de_DE-thorsten-high", name: "Thorsten (High Quality)", is_downloaded: false, is_usable: false, license_non_commercial: true }),
      voiceRow({ id: "de_DE-kerstin-low", name: "Kerstin (Low Quality)", is_downloaded: false, is_usable: false, license_non_commercial: true }),
      // Von Grund auf trainiert (M-AILABS, BSD-3-Clause): frei.
      voiceRow({ id: "de_DE-eva_k-x_low", name: "Eva K.", is_downloaded: false, is_usable: false, license: "BSD-3-Clause" }),
    ],
    runtime: runtimeState(false, false),
    models: [],
  });
  await openModelsArea(page, "Vorlesen", "piper");
  await openGroup(page, "tts-available");
  const thorsten = page.locator('[data-tts-download="de_DE-thorsten-high"]');
  const kerstin = page.locator('[data-tts-download="de_DE-kerstin-low"]');
  await expect(thorsten.getByText("Nur nicht-kommerziell").first()).toBeVisible();
  const eva = page.locator('[data-tts-download="de_DE-eva_k-x_low"]');
  await expect(eva.getByText("Nur nicht-kommerziell")).toHaveCount(0);
  await openRow(thorsten);
  await openRow(kerstin);
  await expect(thorsten.getByTestId("tts-nc-note")).toContainText("von der Lessac-Stimme abgeleitet");
  await expect(thorsten.getByTestId("tts-nc-note")).toContainText("Blizzard 2013");
  await expect(kerstin.getByTestId("tts-nc-note")).toContainText("von der Ryan-Stimme abgeleitet");
  await expect(kerstin.getByTestId("tts-nc-note")).toContainText("CC-BY-NC-SA-4.0");
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

// ── Phonon auf der Modellseite: Programm, Modelle und Stimmen wie bei Piper ──────────────────

const phononRuntime = (over: Record<string, unknown> = {}) => ({
  id: "phonon-runtime",
  kind: "runtime",
  name: "Phonon program ptts (Windows x64)",
  description: "ptts",
  language: null,
  size_mb: 7,
  size_bytes: 7_766_316,
  is_downloaded: false,
  is_downloading: false,
  is_usable: false,
  unsupported_reason: null,
  license: "MIT OR Apache-2.0",
  license_url: "https://example.org/mit",
  license_non_commercial: false,
  engine: "phonon",
  gated: false,
  requires: null,
  source: "https://github.com/gradium-ai/xn-ptts/releases/tag/v0.4.2",
  sha256: "699f34526e66523a2f59c76fcf820f28fb11c581ea135cfb1e918c8d786b8a07",
  license_note: "Programm ptts unter MIT OR Apache-2.0.",
  bundled_voices: 0,
  bundled_voices_present: 0,
  ...over,
});
const phononDe = (over: Record<string, unknown> = {}) =>
  phononRuntime({
    id: "phonon-40m-de",
    kind: "model",
    name: "Phonon 40m (German, q8)",
    language: "de",
    size_mb: 74,
    // Modell plus 13 Stimmen: so gross ist die ganze Installation.
    size_bytes: 75_158_617 + 13 * 256_136,
    bundled_voices: 13,
    bundled_voices_present: 0,
    license: "CC-BY-NC-4.0",
    license_url: "https://huggingface.co/gradium/phonon-40m-de",
    license_non_commercial: true,
    gated: true,
    requires: "phonon-runtime",
    source: "hf://gradium/phonon-40m-de@79cfc7b47fcfc0b666657b981eb366415023a862",
    sha256: null,
    license_note: "Gewichte CC BY-NC 4.0.",
    ...over,
  });
const phononEn = (over: Record<string, unknown> = {}) =>
  phononDe({
    id: "phonon-31m-en",
    name: "Phonon 31m (English, q8)",
    language: "en",
    size_bytes: 65_116_929 + 37 * 256_136,
    bundled_voices: 37,
    bundled_voices_present: 0,
    license: "Apache-2.0",
    license_url: "https://huggingface.co/gradium/phonon-31m-en",
    license_non_commercial: false,
    gated: false,
    source: "hf://gradium/phonon-31m-en@92741c5b07d085ff840e26fea8ce43110b87852f",
    ...over,
  });
/** Nach `setup`: Dialog-Antwort "ja" und Ereignisse der Oberflaeche ausloesen (`__emit`). */
async function withEvents(page: Page) {
  await page.addInitScript(() => {
    const w = window as unknown as Record<string, any>; // eslint-disable-line @typescript-eslint/no-explicit-any
    const internals = w.__TAURI_INTERNALS__;
    const fns: Record<number, (e: unknown) => void> = {};
    const listeners: Record<string, number[]> = {};
    const transform = internals.transformCallback;
    internals.transformCallback = (fn: (e: unknown) => void) => {
      const id = transform(fn);
      fns[id] = fn;
      return id;
    };
    const invoke = internals.invoke;
    internals.invoke = async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "plugin:dialog|ask") {
        (w.__asked ??= []).push(args);
        return true;
      }
      if (cmd === "plugin:event|listen") {
        (listeners[args?.event as string] ??= []).push(args?.handler as number);
      }
      return invoke(cmd, args);
    };
    w.__emit = (event: string, payload: unknown) => {
      for (const id of listeners[event] ?? []) fns[id]?.({ event, id, payload });
    };
  });
}

const callsOf = (page: Page) =>
  page.evaluate(() => (window as unknown as { __calls: [string, Record<string, unknown>][] }).__calls);

const phononSetup = async (page: Page, downloads: unknown[]) => {
  await setup(page, {
    engine: "piper",
    piperVoice: null,
    downloads,
    runtime: runtimeState(true, false),
    models: [],
  });
  await withEvents(page);
};

test("models: Phonon program and models are offered with size incl. voices, license, access hint and an install path", async ({ page }) => {
  await phononSetup(page, [piperRow(), voiceRow(), phononRuntime(), phononDe(), phononEn()]);
  // Die Piper-Zeilen stehen im Seitenblatt von Piper (ohne Phonon); Phonon hat sein eigenes.
  await openModelsArea(page, "Vorlesen", "piper");
  await expect(page.locator('[data-models-section="voices"] [data-tts-download="de_DE-thorsten-medium"]')).toHaveCount(1);
  await expect(page.locator('[data-models-section="phonon"]')).toHaveCount(0);
  await openProviderSheet(page, "phonon");
  await expect(page.locator('[data-models-section="phonon"]').getByText("Phonon (experimentell)")).toBeVisible();
  await expect(page.locator('[data-models-section="phonon"] [data-tts-download="de_DE-thorsten-medium"]')).toHaveCount(0);
  await openGroup(page, "phonon-available");

  const runtime = page.locator('[data-models-section="phonon"] [data-tts-download="phonon-runtime"]');
  await expect(runtime).toContainText("Phonon-Programm (ptts)");
  await expect(runtime).toContainText(/7[.,]4 MB/);
  const de = page.locator('[data-tts-download="phonon-40m-de"]');
  await expect(de).toContainText("Phonon 40m (Deutsch)");
  // Die Größe ist die ganze Installation (Modell + 13 Stimmen), die Stimmen sind keine eigenen Zeilen.
  await expect(de).toContainText(/74[.,]9 MB/);
  await expect(de.getByTestId("phonon-voices-count-phonon-40m-de")).toHaveText("13 Stimmen enthalten");
  await expect(page.locator('[data-models-section="phonon"] [data-tts-download]')).toHaveCount(3);
  await expect(page.locator('[data-tts-download^="phonon-voice"]')).toHaveCount(0);
  await expect(de.getByText("Anmeldung nötig")).toBeVisible();
  await expect(de.getByText("Nur nicht-kommerziell").first()).toBeVisible();
  const en = page.locator('[data-tts-download="phonon-31m-en"]');
  await expect(en.getByText("Anmeldung nötig")).toHaveCount(0);
  await expect(en.getByText("Nur nicht-kommerziell")).toHaveCount(0);
  await expect(en.getByTestId("phonon-voices-count-phonon-31m-en")).toHaveText("37 Stimmen enthalten");

  // Details: Lizenz, Zugangshinweis, Quelle und Pruefsumme.
  await openRow(de);
  await expect(de.getByTestId("phonon-gated-note")).toContainText(
    "Bedingungen auf huggingface.co/gradium/phonon-40m-de annehmen und anmelden (hf auth login)",
  );
  await expect(de.getByTestId("tts-nc-note")).toContainText("CC-BY-NC-4.0");
  await expect(de.getByText("Lizenz: CC-BY-NC-4.0")).toBeVisible();
  await expect(de.getByTestId("phonon-source")).toContainText("hf://gradium/phonon-40m-de@79cfc7b4");
  await expect(de).toContainText("Mehrere Dateien");
  await expect(de.getByTestId("phonon-voices-detail-phonon-40m-de")).toContainText(
    "Zum Modell gehören 13 Stimmen. Sie werden mit dem Modell geladen",
  );
  await openRow(runtime);
  await expect(runtime.getByTestId("phonon-sha256")).toContainText(
    "699f34526e66523a2f59c76fcf820f28fb11c581ea135cfb1e918c8d786b8a07",
  );
  await expect(runtime.getByTestId("phonon-gated-note")).toHaveCount(0);

  // Das Modell braucht das Programm: der Weg dahin fuehrt zuerst zum Programm, nie zur Stimme.
  await expect(runtime.getByRole("button", { name: "Herunterladen" })).toBeVisible();
  await en.getByRole("button", { name: "Programm installieren" }).click();
  // Laeuft das Programm schon, bietet die abhaengige Zeile es nicht noch einmal an.
  await expect(en.getByRole("button", { name: /installieren|Herunterladen/ })).toHaveCount(0);
  const calls = await callsOf(page);
  const downloads = calls.filter(([cmd]) => cmd === "tts_download_model").map(([, args]) => args?.id);
  expect(downloads).toEqual(["phonon-runtime"]);
});

test("models: installed Phonon parts sit under Installed and are deleted after a confirmation", async ({ page }) => {
  await phononSetup(page, [
    piperRow(),
    phononRuntime({ is_downloaded: true, is_usable: true }),
    phononEn({ is_downloaded: true, is_usable: true, bundled_voices_present: 37 }),
    phononDe(),
  ]);
  await openModelsArea(page, "Vorlesen", "phonon");
  const installed = page.locator('[data-model-section="phonon-installed"]');
  await expect(installed.locator("[data-model-row]")).toHaveCount(2);
  // Alle Stimmen da: weder Warnung noch Nachladen.
  await expect(installed.getByText(/Stimmen fehlen/)).toHaveCount(0);
  await expect(installed.getByRole("button", { name: "Stimmen nachladen" })).toHaveCount(0);
  await expect(installed.getByText("Programm fehlt")).toHaveCount(0);
  await openGroup(page, "phonon-available");
  await expect(page.locator('[data-model-section="phonon-available"]').getByText("Phonon 40m (Deutsch)")).toBeVisible();

  // Das Modell loeschen nimmt seine Stimmen mit: ein Aufruf, die Rueckfrage nennt es.
  const en = installed.locator('[data-tts-download="phonon-31m-en"]');
  await en.getByTestId("row-menu").click();
  await page.getByTestId("row-delete").click();
  await expect
    .poll(async () => (await callsOf(page)).filter(([cmd]) => cmd === "tts_delete_model").map(([, a]) => a?.id))
    .toEqual(["phonon-31m-en"]);
  const asked = await page.evaluate(() => (window as unknown as { __asked: unknown[] }).__asked);
  expect(asked.length).toBe(1);
  expect(JSON.stringify(asked[0])).toContain("Stimmen gelöscht");
});

test("models: an installed Phonon model without its voices says so and reloads only the voices", async ({ page }) => {
  await phononSetup(page, [
    piperRow(),
    phononRuntime({ is_downloaded: true, is_usable: true }),
    // Stand von 0.21.20: Modell da, Stimmen fehlen (3 von 37 einzeln geladen).
    phononEn({ is_downloaded: true, is_usable: true, bundled_voices_present: 3 }),
    phononDe({ is_downloaded: true, is_usable: true, bundled_voices_present: 0 }),
  ]);
  await openModelsArea(page, "Vorlesen", "phonon");
  const installed = page.locator('[data-model-section="phonon-installed"]');
  const en = installed.locator('[data-tts-download="phonon-31m-en"]');
  await expect(en.getByText("Stimmen fehlen (3 von 37)")).toBeVisible();
  // Nicht das ganze Modell neu: ein eigener Knopf, und kein "Herunterladen".
  await expect(en.getByRole("button", { name: "Herunterladen" })).toHaveCount(0);
  await en.getByRole("button", { name: "Stimmen nachladen" }).click();
  await installed
    .locator('[data-tts-download="phonon-40m-de"]')
    .getByRole("button", { name: "Stimmen nachladen" })
    .click();
  const calls = await callsOf(page);
  const downloads = calls.filter(([cmd]) => cmd === "tts_download_model").map(([, args]) => args?.id);
  expect(downloads).toEqual(["phonon-31m-en", "phonon-40m-de"]);

  // Waehrend es laeuft: Fortschritt in derselben Zeile, kein zweiter Knopf.
  await page.evaluate(() =>
    (window as unknown as { __emit: (e: string, p: unknown) => void }).__emit("model-download-progress", {
      model_id: "phonon-31m-en",
      downloaded: 40,
      total: 100,
      percentage: 40,
    }),
  );
  await expect(en.locator("[data-row-progress]")).toContainText(/40\s?%/);
  await expect(en.getByRole("button", { name: "Stimmen nachladen" })).toHaveCount(0);
});

test("models: an installed Phonon model without its program is marked, with a way to install the program", async ({ page }) => {
  await phononSetup(page, [
    piperRow(),
    phononRuntime(),
    phononEn({ is_downloaded: true, is_usable: false, bundled_voices_present: 37 }),
  ]);
  await openModelsArea(page, "Vorlesen", "phonon");
  const en = page.locator('[data-model-section="phonon-installed"] [data-tts-download="phonon-31m-en"]');
  await expect(en.getByText("Programm fehlt – nicht nutzbar")).toBeVisible();
  await en.getByRole("button", { name: "Programm installieren" }).click();
  const calls = await callsOf(page);
  expect(calls.some(([cmd, a]) => cmd === "tts_download_model" && a?.id === "phonon-runtime")).toBe(true);
});

test("models: Phonon shows progress, can be cancelled and reports a refused access with the way forward", async ({ page }) => {
  await phononSetup(page, [
    piperRow(),
    phononRuntime({ is_downloaded: true, is_usable: true }),
    phononEn(),
    phononDe(),
  ]);
  await openModelsArea(page, "Vorlesen", "phonon");
  await openGroup(page, "phonon-available");
  await page.locator('[data-tts-download="phonon-31m-en"]').getByRole("button", { name: "Herunterladen" }).click();
  await page.evaluate(() =>
    (window as unknown as { __emit: (e: string, p: unknown) => void }).__emit("model-download-progress", {
      model_id: "phonon-31m-en",
      downloaded: 30,
      total: 100,
      percentage: 30,
    }),
  );
  // Die laufende Zeile steht unter "Installiert", mit Fortschritt und Abbrechen.
  const row = page.locator('[data-model-section="phonon-installed"] [data-tts-download="phonon-31m-en"]');
  await expect(row.locator("[data-row-progress]")).toContainText(/30\s?%/);
  await row.getByRole("button", { name: "Abbrechen" }).click();
  const calls = await callsOf(page);
  expect(calls.some(([cmd, a]) => cmd === "tts_cancel_download" && a?.id === "phonon-31m-en")).toBe(true);

  // Verweigerter Zugang: der Hinweis des Backends erscheint als Meldung.
  await page.evaluate(() =>
    (window as unknown as { __emit: (e: string, p: unknown) => void }).__emit("model-download-failed", {
      model_id: "phonon-40m-de",
      error:
        "Phonon 40m ist gesperrt: der Server verweigert den Zugriff. Bedingungen auf huggingface.co/gradium/phonon-40m-de annehmen und anmelden (hf auth login).",
    }),
  );
  await expect(page.locator("[data-sonner-toast]").first()).toContainText("annehmen und anmelden (hf auth login)");
});

test("Bild der Modellseite mit Phonon (nur mit SCREENS_DIR)", async ({ page }) => {
  const dir = process.env.SCREENS_DIR;
  test.skip(!dir, "nur mit SCREENS_DIR");
  await page.setViewportSize({ width: 1180, height: 1500 });
  await phononSetup(page, [
    piperRow(),
    voiceRow(),
    phononRuntime({ is_downloaded: true, is_usable: true }),
    // Englisch: Stand von 0.21.20, 3 von 37 Stimmen einzeln geladen -> "Stimmen nachladen".
    phononEn({ is_downloaded: true, is_usable: true, bundled_voices_present: 3 }),
    phononDe(),
  ]);
  await openModelsArea(page, "Vorlesen", "phonon");
  await openGroup(page, "phonon-available");
  await openRow(page.locator('[data-tts-download="phonon-40m-de"]'));
  await expect(page.getByTestId("phonon-gated-note")).toBeVisible();
  await openRow(page.locator('[data-tts-download="phonon-31m-en"]'));
  await page.screenshot({ path: `${dir}/phonon-modellseite.png`, fullPage: true });
});
