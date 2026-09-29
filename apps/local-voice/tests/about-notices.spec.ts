import { test, expect } from "@playwright/test";

// QG6 (Goal Granola-Besprechungen): die Info-Seite nennt die neuen Modelle und
// Bibliotheken mit Lizenz, darunter den Hinweis zur NVIDIA Open Model License
// fuer Sortformer (E22) mit Link auf den Lizenztext.
test.beforeEach(async ({ page }) => {
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
    };
    const opened: string[] = [];
    (window as unknown as { opened: string[] }).opened = opened;
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
          if (cmd === "plugin:app|version") return "0.20.3";
          if (cmd.includes("permission")) return true;
          if (cmd === "plugin:event|listen") return ++callback;
          if (cmd === "plugin:opener|open_url") {
            opened.push(String(args?.url));
            return null;
          }
          if (cmd === "get_selected_model") return "";
          if (cmd === "meetings_is_recording") return false;
          if (cmd === "tts_server_status")
            return { phase: "stopped", message: null };
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

test("Info nennt neue Modelle und Bibliotheken mit Lizenz und verlinkt die NVIDIA-Lizenz", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto("/");
  await page
    .getByRole("button", { name: "Einstellungen", exact: true })
    .click();
  await page.getByRole("tab", { name: "Über" }).click();

  await expect(page.getByText("Sortformer (NVIDIA)")).toBeVisible();
  await expect(
    page.getByText(
      "NVIDIA Streaming Sortformer 4spk v2.1 steht unter der NVIDIA Open Model License",
    ),
  ).toBeVisible();
  await expect(
    page.getByText("Parakeet TDT 0.6B v3 (NVIDIA, CC-BY-4.0)"),
  ).toBeVisible();
  await expect(page.getByText("BGE-M3 (BAAI, MIT)")).toBeVisible();
  await expect(page.getByText("sonora (BSD-3-Clause")).toBeVisible();
  await expect(page.getByText("calcard (Apache-2.0 oder MIT")).toBeVisible();

  await page.getByRole("button", { name: "Lizenztext lesen" }).click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as unknown as { opened: string[] }).opened),
    )
    .toEqual([
      "https://www.nvidia.com/en-us/agreements/enterprise-software/nvidia-open-model-license/",
    ]);
});
