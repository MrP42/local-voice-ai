import { test, expect, type Page } from "@playwright/test";

// Vorlesen-Seite: der Satzzähler "Satz n/m" zeigt den Satz, der zu hören ist, und steht ab
// dem Start (das Backend meldet zu Beginn jedes Laufs die Startposition), nicht erst nach dem
// ersten fertigen Satz. Er hat eine feste Breite, damit die Zeile nicht springt. Während des
// Serverstarts gibt es nichts fortzusetzen, auch wenn der Zähler schon steht.
test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(() => {
    const callbacks = new Map<number, unknown>();
    let callback = 0;
    const settings = {
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
      },
      post_process_providers: [],
      post_process_prompts: [],
      custom_words: [],
      post_process_models: {},
      post_process_api_keys: {},
      push_to_talk: true,
      tts_engine: "piper",
      tts_voice: null,
      tts_piper_voice: null,
      tts_script_check: false,
    };
    const w = window as unknown as Record<string, unknown>;
    const listeners: Record<string, number[]> = {};
    w.__calls = [] as unknown[];
    w.__emit = (name: string, payload: unknown) =>
      (listeners[name] ?? []).forEach((id) =>
        (callbacks.get(id) as (e: unknown) => void)?.({
          event: name,
          id: 0,
          payload,
        }),
      );
    Object.assign(window, {
      __TAURI_OS_PLUGIN_INTERNALS__: {
        platform: "windows",
        os_type: "windows",
        family: "windows",
        arch: "x86_64",
        version: "10.0.26200",
        eol: "\r\n",
      },
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
      __TAURI_INTERNALS__: {
        metadata: {
          currentWindow: { label: "main" },
          currentWebview: { label: "main" },
        },
        transformCallback: (fn: unknown) => {
          callbacks.set(++callback, fn);
          return callback;
        },
        unregisterCallback: (id: number) => callbacks.delete(id),
        convertFileSrc: (path: string) => path,
        invoke: async (cmd: string, args?: Record<string, unknown>) => {
          (w.__calls as unknown[]).push([cmd, args]);
          if (cmd === "get_app_settings" || cmd === "get_default_settings")
            return JSON.parse(JSON.stringify(settings));
          if (cmd === "plugin:os|locale") return "de-DE";
          if (cmd === "plugin:app|version") return "0.21.21";
          if (cmd.includes("permission")) return true;
          if (cmd === "plugin:event|listen") {
            const name = String(args?.event);
            (listeners[name] ??= []).push(Number(args?.handler));
            return ++callback;
          }
          if (cmd === "get_selected_model") return "";
          if (cmd === "meetings_is_recording") return false;
          if (cmd === "tts_server_status")
            return { phase: "stopped", message: null };
          if (cmd === "pages_list") return [{ id: "p1", title: "Podcast" }];
          if (cmd === "page_dir") return "C:\\projects\\p1";
          if (cmd === "tts_cached_translation") return null;
          if (cmd === "tts_list_downloads") return [];
          if (
            cmd === "llm_ps" ||
            cmd === "tts_reading_list" ||
            cmd === "tts_list_voices" ||
            cmd === "tts_list_voice_infos" ||
            cmd === "page_files" ||
            cmd.includes("history") ||
            cmd.includes("models") ||
            cmd.includes("devices") ||
            cmd === "meetings_list"
          )
            return [];
          if (cmd === "get_custom_sounds") return { start: false, stop: false };
          return null;
        },
      },
    });
  });
});

const open = async (page: Page) => {
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await page.locator("textarea").first().fill("Erster Satz. Zweiter Satz.");
};

const emit = (page: Page, name: string, payload: unknown) =>
  page.evaluate(
    ([name, payload]) =>
      (window as unknown as { __emit: (n: string, p: unknown) => void }).__emit(
        name as string,
        payload,
      ),
    [name, payload] as const,
  );

test("Satzzähler steht ab dem Start und zeigt den Satz, der zu hören ist", async ({
  page,
}) => {
  await open(page);
  const counter = page.getByTestId("tts-sentence-counter");
  await expect(counter).toHaveCount(0);

  // Beginn des Laufs: Startposition 0 von 12 -- sofort "Satz 1/12".
  await emit(page, "tts-speak-progress", { position: 0, total: 12 });
  await expect(counter).toHaveText("Satz 1/12");

  // Der vierte Satz beginnt (z. B. nach einem Sprung): "Satz 4/12".
  await emit(page, "tts-current-sentence", {
    context: "speak",
    index: 3,
    text: "Vierter Satz.",
  });
  await expect(counter).toHaveText("Satz 4/12");

  // Satz 4 ist durch: sofort "Satz 5/12", ohne auf den nächsten Satz zu warten.
  await emit(page, "tts-speak-progress", { position: 4, total: 12 });
  await expect(counter).toHaveText("Satz 5/12");

  // Sprung zurück auf Satz 2 (neue Startposition 1).
  await emit(page, "tts-speak-progress", { position: 1, total: 12 });
  await expect(counter).toHaveText("Satz 2/12");

  // Der letzte Satz ist durch: nie mehr als die Gesamtzahl.
  await emit(page, "tts-speak-progress", { position: 12, total: 12 });
  await expect(counter).toHaveText("Satz 12/12");
});

test("Satzzähler hat eine feste Breite", async ({ page }) => {
  await open(page);
  const counter = page.getByTestId("tts-sentence-counter");
  const widths: number[] = [];
  for (const position of [0, 1, 8, 9, 10, 11]) {
    await emit(page, "tts-speak-progress", { position, total: 12 });
    await expect(counter).toHaveText(`Satz ${position + 1}/12`);
    widths.push((await counter.boundingBox())!.width);
  }
  // Kein Pixel Unterschied zwischen "Satz 1/12" und "Satz 12/12".
  expect(Math.max(...widths) - Math.min(...widths)).toBeLessThan(0.5);
  expect(
    await counter.evaluate((el) => getComputedStyle(el).fontVariantNumeric),
  ).toContain("tabular-nums");
});

test("während des Serverstarts steht der Zähler, aber Satzsprung ist gesperrt", async ({
  page,
}) => {
  await open(page);
  const previous = page.getByRole("button", { name: "Vorheriger Satz" });
  const status = { owns_server: true, message: null, server_up: true };
  await emit(page, "tts-state-changed", { ...status, phase: "starting" });
  await emit(page, "tts-speak-progress", { position: 0, total: 12 });
  await expect(page.getByTestId("tts-sentence-counter")).toHaveText(
    "Satz 1/12",
  );
  // Der Lauf hängt noch im Start: kein Springen, kein Fortsetzen (ein zweiter Befehl liefe
  // gegen den startenden Server).
  await expect(previous).toBeDisabled();
  // Server bereit, angehalten vor dem ersten Satz: Fortsetzen ist möglich.
  await emit(page, "tts-state-changed", { ...status, phase: "ready" });
  await expect(previous).toBeEnabled();
});
