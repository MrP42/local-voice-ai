import { test, expect, type Page } from "@playwright/test";

// Erstinstallation: Diktatmodell (Nemotron 3.5) lädt ohne Auswahlbildschirm,
// die Einführungstour zeigt Fortschritt und Seiten. Native Seite gemockt;
// `erststart` steuert die Installer-Datei (undefined = keine Datei).
const NEMOTRON = "handy-computer/nemotron-3.5-asr-streaming-0.6b-gguf";

const setup = async (
  page: Page,
  opts: { erststart?: string; onboardingCompleted?: boolean } = {},
) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(
    ({ erststart, onboardingCompleted, nemotron }) => {
      const callbacks = new Map<number, (e: unknown) => void>();
      const listeners = new Map<string, number[]>();
      let callback = 0;
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
        theme: "light",
        show_whats_new_on_update: false,
        debug_mode: false,
        selected_model: "",
        bindings: {
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
        push_to_talk: true,
      };
      const emit = (event: string, payload: unknown) =>
        (listeners.get(event) ?? []).forEach((id) =>
          callbacks.get(id)?.({ event, id, payload }),
        );
      Object.assign(window, {
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
            if (cmd === "get_available_models") return [model];
            if (cmd === "get_current_model" || cmd === "get_selected_model")
              return "";
            if (cmd === "download_model") return null;
            if (cmd === "set_active_model") return null;
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
    },
  );
};

const commands = (page: Page) =>
  page.evaluate(() =>
    (
      (window as unknown as { __calls: { cmd: string; args: unknown }[] })
        .__calls ?? []
    ).map((c) => ({ cmd: c.cmd, args: c.args })),
  );

test("Erstinstallation lädt Nemotron 3.5 und startet die Einführungstour", async ({
  page,
}) => {
  await setup(page);
  await page.goto("/");

  const tour = page.getByTestId("intro-tour");
  const dialog = tour.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await expect(
    dialog.getByRole("heading", { name: "Schön, dass du da bist" }),
  ).toBeVisible();
  await expect(dialog.getByRole("status")).toContainText(
    "Nemotron 3.5 ASR Streaming wird eingerichtet",
  );
  // Kein Auswahlbildschirm: Download ohne Rückfrage gestartet.
  await expect
    .poll(async () =>
      (await commands(page)).some(
        (c) =>
          c.cmd === "download_model" &&
          (c.args as { modelId?: string }).modelId === NEMOTRON,
      ),
    )
    .toBe(true);

  // Fortschritt kommt als Ereignis aus dem Backend.
  await page.evaluate((id) => {
    (window as unknown as { __emit: (e: string, p: unknown) => void }).__emit(
      "model-download-progress",
      { model_id: id, downloaded: 40, total: 100, percentage: 40 },
    );
  }, NEMOTRON);
  await expect(dialog.getByRole("progressbar")).toHaveAttribute(
    "aria-valuenow",
    "40",
  );

  // Weiter: Diktieren mit Kürzel aus den Einstellungen.
  await dialog.getByRole("button", { name: "Tour starten" }).click();
  await expect(
    tour.getByRole("heading", { name: "In jede App diktieren" }),
  ).toBeVisible();
  await expect(tour).toContainText("Strg + Leertaste gedrückt halten");

  // Pfeiltaste: Übungsfeld, dann Spotlight auf „Verlauf“.
  await page.keyboard.press("ArrowRight");
  await expect(tour.getByRole("textbox", { name: /Übungsfeld/ })).toBeVisible();
  await tour.getByRole("button", { name: "Weiter" }).click();
  await expect(tour).toHaveAttribute("data-mode", "spot");
  await expect(
    tour.getByRole("heading", { name: "Alles wiederfinden" }),
  ).toBeVisible();

  // Download fertig: Modell wird zum Diktatmodell.
  await page.evaluate((id) => {
    const w = window as unknown as {
      __emit: (e: string, p: unknown) => void;
      __model: { is_downloaded: boolean };
    };
    w.__model.is_downloaded = true;
    w.__emit("model-download-complete", id);
  }, NEMOTRON);
  await expect
    .poll(async () =>
      (await commands(page)).some(
        (c) =>
          c.cmd === "set_active_model" &&
          (c.args as { modelId?: string }).modelId === NEMOTRON,
      ),
    )
    .toBe(true);

  // Esc schließt die Tour; Startseite bietet sie erneut an.
  await page.keyboard.press("Escape");
  await expect(tour).toHaveCount(0);
  await page.getByTestId("workspace-start-tour").click();
  await expect(
    page.getByRole("heading", { name: "Schön, dass du da bist" }),
  ).toBeVisible();
});

test("Installer ohne Diktatmodell zeigt die Modellauswahl", async ({
  page,
}) => {
  await setup(page, { erststart: "[diktat]\r\nmodell=aus\r\n" });
  await page.goto("/");
  await expect(page.getByText("Nemotron 3.5 ASR Streaming")).toBeVisible();
  await expect(page.getByTestId("intro-tour")).toHaveCount(0);
  expect((await commands(page)).some((c) => c.cmd === "download_model")).toBe(
    false,
  );
});

test("Bestehende Installation startet ohne Tour", async ({ page }) => {
  await setup(page, { onboardingCompleted: true });
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "Was möchtest du tun?" }),
  ).toBeVisible();
  await expect(page.getByTestId("intro-tour")).toHaveCount(0);
});
