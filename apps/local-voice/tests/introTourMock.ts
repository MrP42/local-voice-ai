import { expect, type Page } from "@playwright/test";

// Gemeinsame Attrappe für die Einführungstour: native Seite gemockt;
// `erststart` steuert die Installer-Datei (undefined = keine Datei).
export const NEMOTRON = "handy-computer/nemotron-3.5-asr-streaming-0.6b-gguf";

export interface TourMockOptions {
  erststart?: string;
  onboardingCompleted?: boolean;
  /** Wie oft download_model scheitert, bevor es gelingt. */
  downloadFails?: number;
  theme?: "light" | "dark";
  pushToTalk?: boolean;
  /** Wie oft set_active_model scheitert, bevor es gelingt. */
  selectFails?: number;
  /** set_active_model wartet auf window.__releaseSelect(). */
  holdSelect?: boolean;
  /** Desktop-App (window.isTauri): Fensterknöpfe werden gezeigt. */
  desktop?: boolean;
}

export const setup = async (page: Page, opts: TourMockOptions = {}) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(
    ({
      erststart,
      onboardingCompleted,
      nemotron,
      downloadFails,
      theme,
      pushToTalk,
      selectFails,
      holdSelect,
      desktop,
    }) => {
      const callbacks = new Map<number, (e: unknown) => void>();
      const listeners = new Map<string, number[]>();
      let callback = 0;
      let failures = downloadFails;
      let selectFailures = selectFails;
      const calls: { cmd: string; args: unknown }[] = [];
      const model = {
        id: nemotron,
        name: "Nemotron 3.5 ASR Streaming",
        description: "Live multilingual transcription across 28 languages",
        filename: "nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf",
        source: { HuggingFace: { repo: nemotron } },
        size_mb: 716,
        is_downloaded: false,
        is_downloading: false,
        partial_size: 0,
        is_directory: false,
        engine_type: "Transcribe",
        accuracy_score: 0.82,
        speed_score: 0.84,
        supports_translation: false,
        is_recommended: true,
        supported_languages: ["de", "en"],
        supports_language_selection: true,
        is_custom: false,
        supports_streaming: true,
        supports_language_detection: true,
      };
      const settings = {
        onboarding_completed: onboardingCompleted,
        app_language: "de",
        theme,
        appearance: JSON.stringify({
          modus: theme === "dark" ? "dunkel" : "hell",
          designId: "microsoft",
        }),
        show_whats_new_on_update: false,
        debug_mode: false,
        selected_model: "",
        bindings: {
          cancel: {
            id: "cancel",
            name: "Abbrechen",
            description: "",
            current_binding: "escape",
            default_binding: "escape",
          },
          transcribe: {
            id: "transcribe",
            name: "Diktat",
            description: "",
            current_binding: "ctrl+space",
            default_binding: "ctrl+space",
          },
        },
        post_process_providers: [],
        post_process_prompts: [],
        custom_words: [],
        post_process_models: {},
        post_process_api_keys: {},
        push_to_talk: pushToTalk,
      };
      const emit = (event: string, payload: unknown) =>
        (listeners.get(event) ?? []).forEach((id) =>
          callbacks.get(id)?.({ event, id, payload }),
        );
      Object.assign(window, {
        ...(desktop ? { isTauri: true } : {}),
        __recording: false,
        __calls: calls,
        __emit: emit,
        __model: model,
        __TAURI_OS_PLUGIN_INTERNALS__: {
          platform: "macos",
          os_type: "macos",
          family: "unix",
          arch: "x86_64",
          version: "15.7.9",
          eol: "\n",
        },
        __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
        __TAURI_INTERNALS__: {
          metadata: {
            currentWindow: { label: "main" },
            currentWebview: { label: "main" },
          },
          transformCallback: (fn: (e: unknown) => void) => {
            callbacks.set(++callback, fn);
            return callback;
          },
          unregisterCallback: (id: number) => callbacks.delete(id),
          convertFileSrc: (path: string) => path,
          invoke: async (cmd: string, args: Record<string, unknown>) => {
            calls.push({ cmd, args });
            if (cmd === "get_app_settings" || cmd === "get_default_settings")
              return settings;
            if (cmd === "plugin:os|locale") return "de-DE";
            if (cmd === "plugin:app|version") return "0.23.0";
            if (cmd === "plugin:event|listen") {
              const name = args.event as string;
              listeners.set(name, [
                ...(listeners.get(name) ?? []),
                args.handler as number,
              ]);
              return args.handler;
            }
            if (cmd === "plugin:fs|read_text_file") {
              if (erststart === undefined) throw new Error("not found");
              return Array.from(new TextEncoder().encode(erststart));
            }
            // Kopie wie bei echter IPC (sonst sieht der Store keine Änderung).
            if (cmd === "get_available_models") return [{ ...model }];
            if (cmd === "get_current_model" || cmd === "get_selected_model")
              return "";
            if (cmd === "download_model") {
              if (failures > 0) {
                failures--;
                throw "offline";
              }
              return null;
            }
            if (cmd === "set_active_model") {
              if (selectFailures > 0) {
                selectFailures--;
                throw "Modell konnte nicht geladen werden";
              }
              if (holdSelect)
                await new Promise((resolve) =>
                  Object.assign(window, { __releaseSelect: resolve }),
                );
              return null;
            }
            if (cmd === "is_recording")
              return (window as unknown as { __recording: boolean })
                .__recording;
            if (cmd.includes("permission")) return true;
            if (cmd === "meetings_is_recording") return false;
            if (cmd === "tts_server_status")
              return { phase: "stopped", message: null };
            if (cmd === "get_custom_sounds")
              return { start: false, stop: false };
            if (
              cmd.includes("history") ||
              cmd.includes("models") ||
              cmd.includes("devices") ||
              cmd.includes("list") ||
              cmd === "llm_ps"
            )
              return [];
            return null;
          },
        },
      });
    },
    {
      erststart: opts.erststart,
      onboardingCompleted: opts.onboardingCompleted ?? false,
      nemotron: NEMOTRON,
      downloadFails: opts.downloadFails ?? 0,
      theme: opts.theme ?? "light",
      pushToTalk: opts.pushToTalk ?? true,
      selectFails: opts.selectFails ?? 0,
      holdSelect: opts.holdSelect ?? false,
      desktop: opts.desktop ?? false,
    },
  );
};

export const commands = (page: Page) =>
  page.evaluate(() =>
    (
      (window as unknown as { __calls: { cmd: string; args: unknown }[] })
        .__calls ?? []
    ).map((c) => ({ cmd: c.cmd, args: c.args })),
  );

export const emit = (page: Page, event: string, payload: unknown) =>
  page.evaluate(
    ([e, p]) =>
      (window as unknown as { __emit: (e: string, p: unknown) => void }).__emit(
        e as string,
        p,
      ),
    [event, payload] as const,
  );

export const finishDownload = (page: Page) =>
  page.evaluate((id) => {
    const w = window as unknown as {
      __emit: (e: string, p: unknown) => void;
      __model: { is_downloaded: boolean };
    };
    w.__model.is_downloaded = true;
    w.__emit("model-download-complete", id);
  }, NEMOTRON);

export const progress = (page: Page, pct: number) =>
  emit(page, "model-download-progress", {
    model_id: NEMOTRON,
    downloaded: pct,
    total: 100,
    percentage: pct,
  });

/** Download fertig und Modell ausgewählt: „Tour starten“ ist frei. */
export const ready = async (page: Page) => {
  await finishDownload(page);
  await expect(
    page
      .getByTestId("intro-tour")
      .getByRole("button", { name: "Tour starten" }),
  ).toBeEnabled();
};

/** Schritt n (1 = Diktieren) nach dem Willkommensdialog. */
export const gotoStep = async (page: Page, n: number) => {
  await ready(page);
  const tour = page.getByTestId("intro-tour");
  await tour.getByRole("button", { name: "Tour starten" }).click();
  for (let i = 1; i < n; i++)
    await tour.getByRole("button", { name: "Weiter" }).click();
};
