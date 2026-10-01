import { test, expect, type Page } from "@playwright/test";

// Issue #5: die Ollama-Glaettung ist eine optionale Einstellung im Reiter
// Diktat (Gruppe Ausgabe), Standard aus. Sie erscheint nur, wenn die
// Live-Einfuegung an ist -- ohne sie gibt es nichts zu glaetten.
async function setup(page: Page, streamInjection: boolean, refine = false) {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(
    ({ streamInjection, refine }) => {
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
        stream_injection: streamInjection,
        refine_enabled: refine,
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
            if (cmd === "change_refine_enabled_setting") {
              settings.refine_enabled = args?.enabled;
              return null;
            }
            if (cmd === "get_custom_sounds") return { start: false, stop: false };
            if (cmd.includes("history") || cmd.includes("models") || cmd.includes("devices") || cmd === "meetings_list") return [];
            return null;
          },
        },
      });
    },
    { streamInjection, refine },
  );
}

const openDictation = async (page: Page) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Einstellungen", exact: true }).first().click();
  await page.getByRole("tab", { name: "Diktat", exact: true }).click();
  await expect(page.getByText("Einfügemethode").first()).toBeVisible();
};

test("without live insertion the refinement switch is not shown at all", async ({ page }) => {
  await setup(page, false);
  await openDictation(page);
  await expect(page.getByText("Live-Text nachträglich mit Ollama glätten")).toHaveCount(0);
});

test("with live insertion the switch is shown, off by default, and writes the setting", async ({ page }) => {
  await setup(page, true);
  await openDictation(page);
  // Der Schalter hat keinen eigenen Namen; die Ueberschrift der Zeile nennt ihn.
  const toggle = page
    .getByRole("heading", { name: "Live-Text nachträglich mit Ollama glätten" })
    .locator("xpath=ancestor::div[.//input[@type='checkbox']][1]")
    .getByRole("checkbox");
  await expect(toggle).toBeVisible();
  await expect(toggle).not.toBeChecked();
  await toggle.check({ force: true });
  await expect
    .poll(async () => {
      const calls = await page.evaluate(() => (window as unknown as { __calls: [string, Record<string, unknown>][] }).__calls);
      return calls.find(([cmd]) => cmd === "change_refine_enabled_setting")?.[1]?.enabled;
    })
    .toBe(true);
});
