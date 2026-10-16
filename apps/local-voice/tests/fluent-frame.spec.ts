import { test, expect, type Page } from "@playwright/test";

// Fluent-2-Rahmen (Leitlinie windows-fluent.md §2, §12): Titelleiste mit
// Seitentitel, Navigationsbereich mit Einstellungen unten, Statusleiste;
// je Ansicht höchstens ein Primärbutton. Schreibt Abnahme-Screenshots der
// Hauptseiten hell/dunkel bei 1440 und 390 px nach test-results/fluent/.

async function setup(page: Page, theme: "light" | "dark") {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript((THEME) => {
    const callbacks = new Map<number, unknown>();
    let callback = 0;
    const settings = {
      onboarding_completed: true,
      app_language: "de",
      theme: THEME,
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
            cmd === "tts_reading_list" ||
            cmd === "tts_list_downloads" ||
            cmd === "llm_local_list"
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
          // Geraete-Sync: abgemeldet, bis der Test sich anmeldet.
          const syncState = {
            connected: false, user_email: null, device_name: null, hub_url: null,
            running: false, last_success_ms: null, last_error: null,
            pending: 0, dead_letters: 0, key_mismatch: false, pages: 0,
          };
          if (cmd === "sync_status") return (window as any).__syncState ?? syncState;
          if (cmd === "sync_default_device_name") return "Test-PC";
          if (cmd === "sync_login") {
            (window as any).__syncState = {
              ...syncState, connected: true, user_email: "p@wai.test",
              device_name: "Test-PC", hub_url: "https://portal.wolffappliedai.de",
              last_success_ms: Date.now(), pages: 2,
            };
            return (window as any).__syncState;
          }
          if (cmd === "sync_hub_status")
            return { hub_seq: 3, objects: { page: 2 }, devices: [{ device: "abcdef123456", last_push: "2026-09-14 12:00:00" }] };
          if (cmd === "sync_logout") {
            (window as any).__syncState = syncState;
            return syncState;
          }
          return null;
        },
      },
    });
    localStorage.setItem("lva.ui.settings.tab", "dictation");
  }, theme);
}

const PAGES: [string, string][] = [
  ["Start", "home"],
  ["Verlauf", "history"],
  ["Aufnahmen", "meetings"],
  ["Vorlesen", "tts"],
  ["Modelle", "models"],
  ["Integrationen", "integrations"],
  ["Einstellungen", "settings"],
];

for (const theme of ["light", "dark"] as const) {
  test(`Rahmen und Hauptseiten (${theme}, 1440)`, async ({ page }) => {
    await setup(page, theme);
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto("/");
    await expect(page.locator("html")).toHaveAttribute("data-scheme", theme);
    const nav = page.getByRole("navigation", { name: "Hauptnavigation" });
    for (const [label, id] of PAGES) {
      await nav.getByRole("button", { name: label, exact: true }).click();
      await expect(
        nav.getByRole("button", { name: label, exact: true }),
      ).toHaveAttribute("aria-current", "page");
      const bar = page.locator(".lv-titlebar");
      await expect(bar).toBeVisible();
      if (id !== "home") {
        // Seitentitel in der Kopfleiste
        await expect(bar.locator("#page-title")).toBeVisible();
      }
      await expect(page.locator(".lv-statusbar")).toBeVisible();
      expect(
        await page.locator(".lv-btn--primary:visible").count(),
      ).toBeLessThanOrEqual(1);
      await page.screenshot({
        path: `test-results/fluent/${theme}-1440-${id}.png`,
        animations: "disabled",
      });
    }
    // Einstellungen ist der letzte Eintrag des Navigationsbereichs
    const names = await nav.getByRole("button").allInnerTexts();
    expect(names.map((n) => n.trim()).at(-1)).toBe("Einstellungen");
  });

  test(`schmal mit Navigations-Drawer (${theme}, 390)`, async ({ page }) => {
    await setup(page, theme);
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto("/");
    await page.screenshot({
      path: `test-results/fluent/${theme}-390-home.png`,
      animations: "disabled",
    });
    await page.getByRole("button", { name: "Menü öffnen" }).click();
    const nav = page.getByRole("navigation", { name: "Hauptnavigation" });
    await expect(nav).toBeVisible();
    await page.screenshot({
      path: `test-results/fluent/${theme}-390-nav.png`,
      animations: "disabled",
    });
    await nav.getByRole("button", { name: "Vorlesen", exact: true }).click();
    await expect(nav).toBeHidden();
    await page.screenshot({
      path: `test-results/fluent/${theme}-390-tts.png`,
      animations: "disabled",
    });
    const overflow = await page.evaluate(
      () =>
        document.documentElement.scrollWidth -
        document.documentElement.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(0);
  });
}

// Statusleiste < 480 px: kompakte Einträge, nichts abgeschnitten, kein
// verdeckter Querscroll, höchstens zwei Zeilen; genau ein Primärbutton auf Start.
for (const width of [390, 360]) {
  test(`Statusleiste schmal ohne Abschnitt (${width})`, async ({ page }) => {
    await setup(page, "light");
    await page.setViewportSize({ width, height: 800 });
    await page.goto("/");
    const bar = page.locator(".lv-statusbar");
    await expect(bar).toBeVisible();
    await expect(page.locator(".lv-btn--primary:visible")).toHaveCount(1);
    const m = await bar.evaluate((el) => {
      const r = el.getBoundingClientRect();
      const btns = [...el.querySelectorAll("button")].map((b) => {
        const x = b.getBoundingClientRect();
        return { l: x.left, r: x.right, t: x.top, b: x.bottom };
      });
      const scrolls = [...el.querySelectorAll("*")].filter(
        (n) => !n.classList.contains("lv-status-label") &&
          n.scrollWidth > n.clientWidth + 1 &&
          getComputedStyle(n).overflowX !== "visible",
      ).length;
      return { h: r.height, w: r.width, btns, scrolls };
    });
    expect(m.btns.length).toBeGreaterThanOrEqual(4);
    for (const b of m.btns) {
      expect(b.l).toBeGreaterThanOrEqual(0);
      expect(b.r).toBeLessThanOrEqual(width);
    }
    expect(m.scrolls).toBe(0);
    expect(m.h).toBeLessThanOrEqual(64);
  });
}
