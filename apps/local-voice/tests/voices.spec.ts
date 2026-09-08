import { test, expect } from "@playwright/test";

// Die Stimmenliste mit echten Eintraegen: eine Stimme mit vorhandener
// Hoerprobe, eine ohne. Der Unterschied ist der Punkt — die eine ist sofort
// hoerbar, die andere kostet einen Start der Sprach-Engine, und das muss man
// sehen, bevor man klickt.
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
      tts_voice: null,
    };
    const voices = [
      {
        id: "erzaehlerin",
        meta: { display_name: "Erzählerin", tags: [] },
        origin: "recorded",
        avatar_path: null,
      },
      {
        id: "leo-lausemaus",
        meta: { display_name: "Leo Lausemaus", tags: [] },
        origin: "recorded",
        avatar_path: null,
      },
    ];
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
          if (cmd === "get_app_settings" || cmd === "get_default_settings")
            return settings;
          if (cmd === "plugin:os|locale") return "de-DE";
          if (cmd === "plugin:app|version") return "0.16.0";
          if (cmd.includes("permission")) return true;
          if (cmd === "plugin:event|listen") return ++callback;
          if (cmd === "get_selected_model") return "";
          if (cmd === "meetings_is_recording") return false;
          if (cmd === "tts_server_status")
            return { phase: "stopped", message: null };
          if (cmd === "tts_list_voice_infos") return voices;
          // Nur die eine Stimme hat eine Hoerprobe auf der Platte. Die andere
          // liefert null — genau wie das Backend, wenn noch nichts erzeugt
          // wurde.
          if (cmd === "tts_voice_demo_cached")
            return args?.voiceId === "erzaehlerin"
              ? {
                  wav_path: "C:/demo/erzaehlerin.wav",
                  transcript: "Probesatz.",
                }
              : null;
          // Ein Arbeitsblatt mit einer erzeugten Aufnahme und einer Datei,
          // die keine ist — nur die eine darf einen Abspielknopf bekommen.
          if (cmd === "pages_list")
            return [{ id: "p1", title: "Der Sturm" }];
          if (cmd === "page_files")
            return [
              { name: "Der-Sturm_2026-09-08_1405.wav", size: 120, modified_ms: 0 },
              { name: "notizen.txt", size: 12, modified_ms: 0 },
            ];
          if (cmd === "page_dir") return "C:/projects/p1";
          if (
            cmd === "tts_list_voices" ||
            cmd === "tts_list_voices" ||
            cmd === "llm_ps" ||
            cmd === "tts_reading_list"
          )
            return [];
          if (
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

async function openVoices(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await page
    .getByText("Stimmen anhören & verwalten")
    .click();
}

test("a voice with a stored sample plays without starting the engine", async ({
  page,
}) => {
  await openVoices(page);

  const row = page.locator(".py-1", { hasText: "Erzählerin" }).first();
  // Der Player steht da, ohne dass jemand geklickt hat.
  await expect(row.locator("audio")).toHaveCount(1);
  await expect(row.getByText("Probesatz.")).toBeVisible();
  // Und kein Knopf, der eine Erzeugung anbietet.
  await expect(
    row.getByRole("button", { name: "Hörprobe erzeugen" }),
  ).toHaveCount(0);
});

test("a voice without a sample says what the click will cost", async ({
  page,
}) => {
  await openVoices(page);

  const row = page.locator(".py-1", { hasText: "Leo Lausemaus" }).first();
  await expect(row.locator("audio")).toHaveCount(0);
  await expect(
    row.getByRole("button", { name: "Hörprobe erzeugen" }),
  ).toBeVisible();
  await expect(row.getByText(/startet einmalig die Sprach-Engine/)).toBeVisible();
});

test("a generated recording can be played from the file list", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Vorlesen", exact: true }).click();

  const audioRow = page
    .locator("div", { hasText: /^Der-Sturm_2026-09-08_1405\.wav/ })
    .last();
  const listen = audioRow.getByRole("button", { name: "Anhören" });
  await expect(listen).toBeVisible();

  // Der Player entsteht erst auf Wunsch — sonst laege unter jeder Datei einer.
  const filesArea = page.locator(".tts-workspace__files");
  await expect(filesArea.locator("audio")).toHaveCount(0);
  await listen.click();
  await expect(filesArea.locator("audio")).toHaveCount(1);

  // Eine Textdatei bekommt keinen Abspielknopf.
  const textRow = page.locator("div", { hasText: /^notizen\.txt/ }).last();
  await expect(textRow.getByRole("button", { name: "Anhören" })).toHaveCount(0);
});
