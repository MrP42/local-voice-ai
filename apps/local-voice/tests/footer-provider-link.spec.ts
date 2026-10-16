import { test, expect, type Page } from "@playwright/test";

// Fussleiste -> Einstellungen: "Anbieter-Einstellungen ..." am Ende der Liste
// und im Rechtsklick-Menue fuehren in den Reiter KI-Modelle & Anbieter, zu den
// Verbindungen -- auch bei schon geoeffneten Einstellungen in einem anderen Reiter.
async function setup(page: Page) {
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
        invoke: async (cmd: string) => {
          if (cmd === "get_app_settings" || cmd === "get_default_settings") return settings;
          if (cmd === "plugin:os|locale") return "de-DE";
          if (cmd === "plugin:app|version") return "0.21.10";
          if (cmd.includes("permission")) return true;
          if (cmd === "plugin:event|listen") return ++callback;
          if (cmd === "get_selected_model") return "";
          if (cmd === "meetings_is_recording") return false;
          if (cmd === "get_custom_sounds") return { start: false, stop: false };
          if (cmd.includes("history") || cmd.includes("models") || cmd.includes("devices") || cmd === "meetings_list") return [];
          return null;
        },
      },
    });
  });
}

const connectionsVisible = async (page: Page) => {
  await expect(page.getByRole("tab", { name: "KI-Modelle & Anbieter", exact: true })).toHaveAttribute("aria-selected", "true");
  await expect(page.locator('[data-settings-anchor="connections"]')).toBeInViewport();
};

test("Liste: letzter Eintrag fuehrt zu den Anbieter-Einstellungen", async ({ page }) => {
  await setup(page);
  await page.goto("/");
  await page.locator("[data-llm-selector]").click();
  await page.locator("[data-llm-provider-settings-link]").click();
  await connectionsVisible(page);
});

test("Rechtsklick auf das Modellfeld: Menue mit Sprung", async ({ page }) => {
  await setup(page);
  await page.goto("/");
  await page.locator("[data-llm-selector]").click({ button: "right" });
  await expect(page.locator("[data-llm-context-menu]")).toBeVisible();
  await page.locator("[data-llm-provider-settings]").click();
  await connectionsVisible(page);
});

test("bei geoeffneten Einstellungen in anderem Reiter wechselt der Sprung den Reiter", async ({ page }) => {
  await setup(page);
  await page.goto("/");
  await page.getByRole("button", { name: "Einstellungen", exact: true }).first().click();
  await page.getByRole("tab", { name: "Ausgabe", exact: true }).click();
  await page.locator("[data-llm-selector]").click({ button: "right" });
  await page.locator("[data-llm-provider-settings]").click();
  await connectionsVisible(page);
});

test("Escape schliesst das Rechtsklick-Menue", async ({ page }) => {
  await setup(page);
  await page.goto("/");
  await page.locator("[data-llm-selector]").click({ button: "right" });
  await page.keyboard.press("Escape");
  await expect(page.locator("[data-llm-context-menu]")).toHaveCount(0);
});
