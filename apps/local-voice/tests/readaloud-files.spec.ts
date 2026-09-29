import { test, expect } from "@playwright/test";

// Vorlesen: Dateileiste mit langen Namen und der Standardname beim Speichern.
// Die Tauri-Bruecke ist eine schlanke Variante der Attrappe aus
// script-check.spec.ts; zusaetzlich merkt sie sich, welchen Pfad der
// Speichern-Dialog vorgeschlagen bekam.
const LONG_NAME = "CASE-GESPRÄCH-IE2S-·-28.09.2026_2026-09-28_1736.wav";

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
      tts_engine: "fish",
      tts_piper_voice: null,
      tts_export_format: "wav",
    };
    const voices = [
      {
        id: "patrick",
        meta: { display_name: "Patrick", tags: [] },
        origin: "recorded",
        avatar_path: null,
      },
    ];
    const w = window as unknown as {
      __lvVoice?: string | null;
      __savePaths?: string[];
    };
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
            // Die Stimme je Test: null = Skript-Stimmen, sonst eine Einzelstimme.
            return { ...settings, tts_voice: w.__lvVoice ?? null };
          if (cmd === "plugin:os|locale") return "de-DE";
          if (cmd === "plugin:app|version") return "0.16.0";
          if (cmd.includes("permission")) return true;
          if (cmd === "plugin:event|listen") return ++callback;
          if (cmd === "get_selected_model") return "";
          if (cmd === "meetings_is_recording") return false;
          if (cmd === "tts_server_status")
            return { phase: "stopped", message: null };
          if (cmd === "tts_list_voice_infos") return voices;
          if (cmd === "tts_list_voices") return voices.map((v) => v.id);
          if (cmd === "pages_list") return [{ id: "p1", title: "Der Sturm" }];
          if (cmd === "page_files")
            return [
              {
                name: "Der-Sturm_2026-09-08_1405.wav",
                size: 120,
                modified_ms: 0,
              },
              {
                name: "CASE-GESPRÄCH-IE2S-·-28.09.2026_2026-09-28_1736.wav",
                size: 4096,
                modified_ms: 0,
              },
            ];
          if (cmd === "page_dir") return "C:\\projects\\p1";
          if (cmd === "plugin:dialog|save") {
            const options = args?.options as { defaultPath?: string };
            w.__savePaths = [
              ...(w.__savePaths ?? []),
              options.defaultPath ?? "",
            ];
            // Abbrechen: der Test prueft nur den Vorschlag, nicht den Export.
            return null;
          }
          if (cmd === "tts_list_downloads") return [];
          if (
            cmd === "llm_ps" ||
            cmd === "tts_reading_list" ||
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

const openReadAloud = async (page: import("@playwright/test").Page) => {
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  // Erster Start zeigt "Hilfe"; die Dateizeilen brauchen den Reiter "Dateien".
  await page.getByRole("tab", { name: "Dateien", exact: true }).click();
};

test("a long file name keeps its timestamp visible inside the row", async ({
  page,
}) => {
  await openReadAloud(page);

  const name = page.locator(`span[title="${LONG_NAME}"]`);
  await expect(name).toBeVisible();
  const tail = name.locator("span").last();
  await expect(tail).toContainText("2026-09-28_1736");
  await expect(tail).toHaveText(/_2026-09-28_1736\.wav$/);

  // Der Anfang wurde gekuerzt, der Schwanz nicht — und alles bleibt in der Zeile.
  const head = name.locator("span").first();
  expect(await head.evaluate((el) => el.scrollWidth > el.clientWidth)).toBe(
    true,
  );
  const row = await name.locator("xpath=..").boundingBox();
  const box = await tail.boundingBox();
  expect(row).not.toBeNull();
  expect(box).not.toBeNull();
  expect(box!.x).toBeGreaterThanOrEqual(row!.x - 0.5);
  expect(box!.x + box!.width).toBeLessThanOrEqual(row!.x + row!.width + 0.5);
});

test("a file name that fits is shown whole, split or not", async ({ page }) => {
  await openReadAloud(page);
  // Geteilt in Anfang und Schwanz, gelesen aber als ein Name.
  const name = page.locator(`span[title="Der-Sturm_2026-09-08_1405.wav"]`);
  await expect(name).toHaveText("Der-Sturm_2026-09-08_1405.wav");
});

const suggestedPath = async (page: import("@playwright/test").Page) => {
  await page.locator("textarea").first().fill("Hallo Welt.");
  await page.getByRole("button", { name: "Als Audio speichern…" }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { __savePaths?: string[] }).__savePaths,
      ),
    )
    .toHaveLength(1);
  return page.evaluate(
    () => (window as unknown as { __savePaths: string[] }).__savePaths[0],
  );
};

test("saving with a single voice proposes the voice as file name", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as unknown as { __lvVoice: string }).__lvVoice = "patrick";
  });
  await openReadAloud(page);
  const path = await suggestedPath(page);
  expect(path).toMatch(
    /^C:\\projects\\p1\\Patrick_\d{4}-\d{2}-\d{2}_\d{4}\.wav$/,
  );
});

test("saving with script voices proposes Skript as file name", async ({
  page,
}) => {
  await openReadAloud(page);
  const path = await suggestedPath(page);
  expect(path).toMatch(
    /^C:\\projects\\p1\\Skript_\d{4}-\d{2}-\d{2}_\d{4}\.wav$/,
  );
});
