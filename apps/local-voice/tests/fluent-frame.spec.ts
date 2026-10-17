import { test, expect, type Page } from "@playwright/test";

// Fluent-2-Rahmen (Leitlinie windows-fluent.md §2, §12): Titelleiste mit
// Seitentitel, Navigationsbereich mit Einstellungen unten, Statusleiste;
// je Ansicht höchstens ein Primärbutton. Schreibt Abnahme-Screenshots der
// Hauptseiten hell/dunkel bei 1440 und 390 px nach test-results/fluent/.

// `desktop`: wie in der Tauri-App (window.isTauri), mit protokollierten
// Fensterbefehlen; `onboarding`: Einrichtung noch nicht abgeschlossen.
async function setup(
  page: Page,
  theme: "light" | "dark",
  opts: { desktop?: boolean; onboarding?: boolean } = {},
) {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(({ THEME, DESKTOP, ONBOARDING }) => {
    const callbacks = new Map<number, unknown>();
    const resizeHandlers: number[] = [];
    const win = { calls: [] as string[], maximized: false };
    (window as any).__win = win;
    if (DESKTOP) (window as any).isTauri = true;
    let callback = 0;
    const settings = {
      onboarding_completed: !ONBOARDING,
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
        invoke: async (cmd: string, args?: any) => {
          if (cmd.startsWith("plugin:window|")) {
            const name = cmd.slice("plugin:window|".length);
            win.calls.push(name);
            if (name === "is_maximized") return win.maximized;
            if (name === "toggle_maximize") {
              win.maximized = !win.maximized;
              for (const id of resizeHandlers)
                (callbacks.get(id) as (e: unknown) => void)?.({
                  event: "tauri://resize",
                  id: 0,
                  payload: { width: 1440, height: 900 },
                });
            }
            return null;
          }
          // Einrichtung: Berechtigungen noch offen, damit der Bildschirm
          // „Berechtigungen“ stehen bleibt (mit allen Rechten springt der
          // Assistent sofort weiter).
          if (ONBOARDING && cmd.includes("permission")) return false;
          if (cmd === "plugin:event|listen" && args?.event === "tauri://resize") {
            resizeHandlers.push(args.handler);
            return ++callback;
          }
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
  }, { THEME: theme, DESKTOP: !!opts.desktop, ONBOARDING: !!opts.onboarding });
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

// Forced Colors (Windows-Kontrastdesigns): Fokus bleibt sichtbar, die aktive
// Navigationsseite ist ohne Farbe erkennbar. Reflow 400 % (WCAG 1.4.10): bei
// 320 px Breite scrollt die Seite nicht horizontal.
const REFLOW_PAGES: [string, string][] = [
  ["Start", "home"],
  ["Einstellungen", "settings"],
  ["Vorlesen", "tts"],
];

test("Forced Colors: Fokus sichtbar, Navigationsauswahl erkennbar", async ({ page }) => {
  await setup(page, "light");
  await page.emulateMedia({ forcedColors: "active" });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  const nav = page.getByRole("navigation", { name: "Hauptnavigation" });
  for (const [label, id] of REFLOW_PAGES) {
    await nav.getByRole("button", { name: label, exact: true }).click();
    // Auswahl: aktiver Eintrag unterscheidet sich vom inaktiven in Rand,
    // Kontur, Fläche oder Markierungsbalken (nicht nur in der Textfarbe).
    const marks = await nav.evaluate((el) => {
      const sig = (b: Element) => {
        const cs = getComputedStyle(b);
        const p1 = getComputedStyle(b, "::before");
        const p2 = getComputedStyle(b, "::after");
        return [
          cs.borderLeftWidth, cs.borderLeftStyle, cs.outlineStyle,
          cs.backgroundColor, cs.boxShadow,
          p1.content, p1.width, p1.backgroundColor,
          p2.content, p2.width, p2.backgroundColor,
        ].join("|");
      };
      const cur = el.querySelector('[aria-current="page"]')!;
      const other = [...el.querySelectorAll("button")].find(
        (b) => b !== cur && !b.hasAttribute("aria-current"),
      )!;
      return { cur: sig(cur), other: sig(other) };
    });
    expect(marks.cur, `Navigation ${label}`).not.toBe(marks.other);
    await page.screenshot({
      path: `test-results/fluent/forced-1440-${id}.png`,
      animations: "disabled",
    });
  }
  // Fokus: per Tab erreichte Elemente sehen fokussiert anders aus als
  // unfokussiert (Pixelvergleich des Elementbereichs), die ersten fünf Ziele.
  await page.goto("/");
  await expect(page.locator(".lv-titlebar")).toBeVisible();
  for (let i = 0; i < 5; i++) {
    await page.keyboard.press("Tab");
    const box = await page.evaluate(() => {
      const a = document.activeElement as HTMLElement;
      const r = a.getBoundingClientRect();
      return {
        tag: a.tagName,
        x: Math.max(0, r.x - 6),
        y: Math.max(0, r.y - 6),
        width: r.width + 12,
        height: r.height + 12,
      };
    });
    expect(box.tag).not.toBe("BODY");
    const { tag, ...clip } = box;
    const focused = await page.screenshot({ clip, animations: "disabled" });
    await page.evaluate(() => (document.activeElement as HTMLElement).blur());
    const plain = await page.screenshot({ clip, animations: "disabled" });
    expect(focused.equals(plain), `Fokus ${i + 1} (${tag}) sichtbar`).toBe(false);
    // Fokus zurück, damit der nächste Tab am richtigen Element weitergeht.
    await page.keyboard.press("Shift+Tab");
    await page.keyboard.press("Tab");
  }
});

for (const [label, id] of REFLOW_PAGES) {
  test(`Reflow 320 px ohne Querscroll: ${label}`, async ({ page }) => {
    await setup(page, "light");
    await page.setViewportSize({ width: 320, height: 640 });
    await page.goto("/");
    if (id !== "home") {
      await page.getByRole("button", { name: "Menü öffnen" }).click();
      await page
        .getByRole("navigation", { name: "Hauptnavigation" })
        .getByRole("button", { name: label, exact: true })
        .click();
    }
    await expect(page.locator(".lv-titlebar")).toBeVisible();
    await page.waitForTimeout(300);
    const m = await page.evaluate(() => ({
      over:
        document.documentElement.scrollWidth -
        document.documentElement.clientWidth,
      body: document.body.scrollWidth - document.body.clientWidth,
    }));
    await page.screenshot({
      path: `test-results/fluent/reflow-320-${id}.png`,
      animations: "disabled",
    });
    // Der Hauptbereich selbst scrollt nie seitlich; Reiterleisten und
    // Chip-Reihen dürfen in sich scrollen (eigene Bedienelemente).
    const mainOver = await page.evaluate(() => {
      const el = document.querySelector(".workspace-main") as HTMLElement;
      return el.scrollWidth - el.clientWidth;
    });
    expect(mainOver).toBeLessThanOrEqual(0);
    expect(m.over).toBeLessThanOrEqual(0);
    expect(m.body).toBeLessThanOrEqual(0);
  });
}

// Rahmenloses Hauptfenster (Windows 11): die eigene Titelleiste ist die
// einzige, rechts die Fensterknöpfe. Belege hell/dunkel unter
// test-results/fluent-rahmen/ (abgenommene Fassung: docs/belege/fluent-rahmen/).
test.describe("Fensterrahmen", () => {
  test("ohne Tauri keine Fensterknöpfe", async ({ page }) => {
    await setup(page, "light");
    await page.goto("/");
    await expect(page.locator(".lv-titlebar")).toBeVisible();
    await expect(page.locator(".lv-window-controls")).toHaveCount(0);
  });

  test("Knöpfe rufen die Fenster-API, Maximieren wechselt zu Wiederherstellen", async ({ page }) => {
    await setup(page, "light", { desktop: true });
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto("/");
    const bar = page.locator(".lv-titlebar");
    await expect(bar).toHaveAttribute("data-tauri-drag-region", "deep");
    const group = bar.getByRole("group", { name: "Fenster" });
    await expect(group.getByRole("button")).toHaveCount(3);
    // bündig rechts, 46×48
    const box = await group.getByRole("button", { name: "Schließen" }).boundingBox();
    expect(box).toMatchObject({ width: 46, height: 48, y: 0 });
    expect(box!.x + box!.width).toBe(1440);
    // Produktname genau einmal sichtbar
    await expect(page.getByText("Local Voice AI", { exact: true }).filter({ visible: true })).toHaveCount(1);

    await group.getByRole("button", { name: "Minimieren" }).click();
    await group.getByRole("button", { name: "Maximieren" }).click();
    await expect(group.getByRole("button", { name: "Wiederherstellen" })).toBeVisible();
    await group.getByRole("button", { name: "Wiederherstellen" }).click();
    await expect(group.getByRole("button", { name: "Maximieren" })).toBeVisible();
    await group.getByRole("button", { name: "Schließen" }).click();
    // Die Befehle laufen asynchron (Fenster-API wird nachgeladen)
    await expect
      .poll(() =>
        page.evaluate(() =>
          ((window as any).__win.calls as string[]).filter((c) => c !== "is_maximized"),
        ),
      )
      .toEqual([
      "minimize",
      "toggle_maximize",
      "toggle_maximize",
      "close",
    ]);

    // Schließen-Hover in Systemrot
    const close = group.getByRole("button", { name: "Schließen" });
    await close.hover();
    await expect(close).toHaveCSS("background-color", "rgb(196, 43, 28)");
  });

  for (const theme of ["light", "dark"] as const) {
    test(`Beleg Titelleiste (${theme})`, async ({ page }) => {
      await setup(page, theme, { desktop: true });
      await page.setViewportSize({ width: 1440, height: 900 });
      await page.goto("/");
      await page
        .getByRole("navigation", { name: "Hauptnavigation" })
        .getByRole("button", { name: "Vorlesen", exact: true })
        .click();
      await expect(page.locator(".lv-titlebar #page-title")).toBeVisible();
      await page.locator(".lv-titlebar__app").hover();
      await page.screenshot({
        path: `test-results/fluent-rahmen/${theme}-1440-tts.png`,
        animations: "disabled",
      });
      await page.getByRole("button", { name: "Schließen" }).hover();
      await page.screenshot({
        path: `test-results/fluent-rahmen/${theme}-titelleiste-schliessen-hover.png`,
        clip: { x: 0, y: 0, width: 1440, height: 48 },
        animations: "disabled",
      });
    });

    test(`schmal ohne Querscroll mit Fensterknöpfen (${theme}, 390)`, async ({ page }) => {
      await setup(page, theme, { desktop: true });
      await page.setViewportSize({ width: 390, height: 844 });
      await page.goto("/");
      const group = page.getByRole("group", { name: "Fenster" });
      await expect(group).toBeVisible();
      const r = await group.boundingBox();
      expect(r!.x + r!.width).toBe(390);
      const overflow = await page.evaluate(
        () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
      );
      expect(overflow).toBeLessThanOrEqual(0);
      await page.screenshot({
        path: `test-results/fluent-rahmen/${theme}-390-home.png`,
        animations: "disabled",
      });
    });
  }

  test("Einrichtung: Ziehleiste mit Fensterknöpfen", async ({ page }) => {
    await setup(page, "light", { desktop: true, onboarding: true });
    await page.setViewportSize({ width: 800, height: 600 });
    await page.goto("/");
    const bar = page.locator(".lv-titlebar--plain");
    await expect(bar).toBeVisible();
    await expect(bar).toHaveAttribute("data-tauri-drag-region", "deep");
    await expect(bar.getByRole("group", { name: "Fenster" }).getByRole("button")).toHaveCount(3);
    await page.screenshot({
      path: "test-results/fluent-rahmen/light-800-einrichtung.png",
      animations: "disabled",
    });
  });
});
