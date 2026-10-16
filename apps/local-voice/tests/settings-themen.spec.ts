import { test, expect, type Page } from "@playwright/test";

// Einstellungen nach Thema (09.10.2026): Eingabe, Ausgabe, KI-Modelle &
// Anbieter, Allgemein, Ueber. Ein gespeicherter Reiter aus der alten
// Gliederung landet auf dem Reiter, der seinen Inhalt jetzt traegt.
async function setup(page: Page, tab?: string) {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(
    ({ tab }) => {
      if (tab) localStorage.setItem("lva.ui.settings.tab", tab);
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
    { tab },
  );
}


const shots = process.env.SCREENS_DIR;

const open = async (page: Page) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Einstellungen", exact: true }).first().click();
};

test("die Reiter sind nach Thema sortiert", async ({ page }) => {
  await setup(page);
  await open(page);
  const names = await page.getByRole("tab").allInnerTexts();
  expect(names).toEqual(["Eingabe", "Ausgabe", "KI-Modelle & Anbieter", "Allgemein", "Über"]);
  await expect(page.getByRole("tab", { name: "Eingabe", exact: true })).toHaveAttribute("aria-selected", "true");
});

test("Eingabe: Mikrofon und Prompts der Textverbesserung gehoeren dazu", async ({ page }) => {
  await setup(page);
  await open(page);
  await expect(page.getByText("Mikrofon", { exact: true }).first()).toBeVisible();
  await expect(page.getByText("Einfügemethode").first()).toBeVisible();
  await expect(page.getByText("Prompts", { exact: false }).first()).toBeVisible();
  if (shots) await page.screenshot({ path: `${shots}/einstellungen-eingabe.png`, animations: "disabled" });
});

test("Ausgabe: Vorlesen und hoerbare Rueckmeldung", async ({ page }) => {
  await setup(page);
  await open(page);
  await page.getByRole("tab", { name: "Ausgabe", exact: true }).click();
  await expect(page.getByText("Hörbare Rückmeldung").first()).toBeVisible();
  if (shots) await page.screenshot({ path: `${shots}/einstellungen-ausgabe.png`, animations: "disabled" });
});

test("KI-Modelle & Anbieter: Verbindungen, ohne Prompts", async ({ page }) => {
  await setup(page);
  await open(page);
  await page.getByRole("tab", { name: "KI-Modelle & Anbieter", exact: true }).click();
  await expect(page.getByText("Verbindungen", { exact: true }).first()).toBeVisible();
  await expect(page.getByText("Einfügemethode")).toHaveCount(0);
  if (shots) await page.screenshot({ path: `${shots}/einstellungen-modelle.png`, animations: "disabled" });
});

for (const [alt, neu] of [
  ["dictation", "Eingabe"],
  ["sound", "Ausgabe"],
  ["readaloud", "Ausgabe"],
  ["postprocessing", "KI-Modelle & Anbieter"],
] as const) {
  test(`gespeicherter alter Reiter ${alt} landet auf ${neu}`, async ({ page }) => {
    await setup(page, alt);
    await open(page);
    await expect(page.getByRole("tab", { name: neu, exact: true })).toHaveAttribute("aria-selected", "true");
  });
}
