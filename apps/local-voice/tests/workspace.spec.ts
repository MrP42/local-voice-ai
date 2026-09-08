import { test, expect } from "@playwright/test";

// Exercise the production React shell with deterministic native responses.
// Audio capture/inference require separate native tests.
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
    };
    Object.assign(window, {
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
        transformCallback: (fn: unknown) => {
          callbacks.set(++callback, fn);
          return callback;
        },
        unregisterCallback: (id: number) => callbacks.delete(id),
        convertFileSrc: (path: string) => path,
        invoke: async (cmd: string) => {
          if (cmd === "get_app_settings" || cmd === "get_default_settings")
            return settings;
          if (cmd === "plugin:os|locale") return "de-DE";
          if (cmd === "plugin:app|version") return "0.14.0";
          if (cmd.includes("permission")) return true;
          if (cmd === "plugin:event|listen") return ++callback;
          if (cmd === "get_selected_model") return "";
          if (cmd === "meetings_is_recording") return false;
          if (cmd === "tts_server_status")
            return { phase: "stopped", message: null };
          if (
            cmd === "pages_list" ||
            cmd === "page_files" ||
            cmd === "tts_list_voices" ||
            cmd === "tts_list_voice_infos" ||
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
    localStorage.setItem("lva.ui.settings.tab", "dictation");
  });
});

test("home exposes working destinations and remembers navigation", async ({
  page,
}) => {
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "Was möchtest du tun?" }),
  ).toBeVisible();
  await expect(page.getByText("Ctrl+Space", { exact: true })).toBeVisible();
  const nav = page.getByRole("navigation", { name: "Hauptnavigation" });
  await nav.getByRole("button", { name: "Verlauf", exact: true }).click();
  await expect(
    nav.getByRole("button", { name: "Verlauf", exact: true }),
  ).toHaveAttribute("aria-current", "page");
  await page.reload();
  await expect(
    nav.getByRole("button", { name: "Verlauf", exact: true }),
  ).toHaveAttribute("aria-current", "page");
  await nav.getByRole("button", { name: "Start", exact: true }).click();
  await page.getByRole("button", { name: /Besprechung oder Aufnahme/ }).click();
  await expect(
    nav.getByRole("button", { name: "Aufnahmen", exact: true }),
  ).toHaveAttribute("aria-current", "page");
});

test("narrow navigation keeps labels and secondary destinations reachable", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  const nav = page.getByRole("navigation", { name: "Hauptnavigation" });
  await expect(
    nav.getByRole("button", { name: "Start", exact: true }),
  ).toBeVisible();
  await expect(
    nav.getByRole("button", { name: "Modelle", exact: true }),
  ).toBeHidden();
  await nav.getByRole("button", { name: "Mehr", exact: true }).click();
  await expect(
    nav.getByRole("button", { name: "Modelle", exact: true }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(
    nav.getByRole("button", { name: "Mehr", exact: true }),
  ).toBeFocused();
  await expect(
    nav.getByRole("button", { name: "Modelle", exact: true }),
  ).toBeHidden();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBeTruthy();
  await page.screenshot({ path: "test-results/workspace-narrow.png" });
});

test("WAI home in light and dark at desktop size", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "Was möchtest du tun?" }),
  ).toBeVisible();
  await page.screenshot({ path: "test-results/workspace-light.png" });
  await page.evaluate(() => {
    document.documentElement.dataset.theme = "dark";
  });
  await expect(
    page
      .getByRole("navigation")
      .getByRole("button", { name: "Start", exact: true }),
  ).toHaveCSS("background-color", "rgb(255, 221, 0)");
  await page.screenshot({
    path: "test-results/workspace-dark.png",
    animations: "disabled",
  });
});

test("settings tabs support keyboard selection without extra tab stops", async ({
  page,
}) => {
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Einstellungen", exact: true })
    .click();
  const tabs = page.getByRole("tablist");
  const first = tabs.getByRole("tab").first();
  await first.focus();
  await page.keyboard.press("ArrowRight");
  await expect(tabs.getByRole("tab").nth(1)).toBeFocused();
  await expect(tabs.getByRole("tab").nth(1)).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await page.keyboard.press("Home");
  await expect(first).toBeFocused();
  await expect(page.getByRole("tabpanel")).toBeVisible();
});

test("Windows shell keeps the same task navigation", async ({ page }) => {
  await page.addInitScript(() => {
    Object.assign(
      (window as unknown as { __TAURI_OS_PLUGIN_INTERNALS__: object })
        .__TAURI_OS_PLUGIN_INTERNALS__,
      { platform: "windows", os_type: "windows", family: "windows" },
    );
  });
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "Was möchtest du tun?" }),
  ).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute(
    "data-platform",
    "windows",
  );
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Verlauf", exact: true })
    .click();
  await expect(page.getByRole("main")).toHaveAttribute("aria-label", "Verlauf");
});

test("read-aloud workspace preserves editor width in a narrow window", async ({
  page,
}) => {
  await page.setViewportSize({ width: 700, height: 900 });
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await expect(page.locator(".tts-workspace")).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBeTruthy();
  await expect(
    page.getByText("Stimme & Wiedergabe", { exact: true }),
  ).toBeVisible();
  const options = page.getByText("Weitere Einstellungen zum Vorlesen", {
    exact: true,
  });
  await options.scrollIntoViewIfNeeded();
  await options.click();
  await expect(options.locator("..")).toHaveAttribute("open", "");
  await options.click();
  await page.getByRole("main").evaluate((element) => {
    element.scrollTop = 0;
  });
  const editor = page.locator(".tts-workspace > .min-w-0");
  expect((await editor.boundingBox())!.width).toBeGreaterThan(600);
  await page.screenshot({
    path: "test-results/workspace-reading.png",
    animations: "disabled",
  });
});
