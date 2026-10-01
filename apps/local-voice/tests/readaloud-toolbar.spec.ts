import { test, expect, type Page } from "@playwright/test";
import * as path from "node:path";

// Bedienspalte der Vorlesen-Seite: eine Zeile gleich grosser Symbol-Knoepfe,
// Seltenes hinter dem Menue. Die Tauri-Bruecke ist eine schlanke Attrappe wie
// in script-check.spec.ts; zusaetzlich merkt sie sich die aufgerufenen Befehle.
test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(() => {
    const callbacks = new Map<number, unknown>();
    let callback = 0;
    const calls: string[] = [];
    (window as unknown as { __calls: string[] }).__calls = calls;
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
      tts_engine: "piper",
      tts_piper_voice: null,
      tts_script_check: true,
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
        invoke: async (cmd: string) => {
          calls.push(cmd);
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
          if (cmd === "pages_list") return [{ id: "p1", title: "Der Sturm" }];
          if (cmd === "books_list") return [];
          if (cmd === "books_templates") return [];
          if (cmd === "books_memory_read") return [];
          if (cmd === "tts_tidy_text") return "Sauberer Text.";
          if (cmd === "page_files") return [];
          if (cmd === "page_dir") return "C:/projects/p1";
          if (cmd === "tts_list_downloads") return [];
          if (cmd === "get_custom_sounds") return { start: false, stop: false };
          // Testweise vorgegebene Stimmenliste (window.__voices), sonst leer.
          if (cmd === "tts_list_voices")
            return (
              (window as unknown as { __voices?: string[] }).__voices ?? []
            );
          if (
            cmd === "tts_list_voice_infos" ||
            cmd === "llm_ps" ||
            cmd === "tts_reading_list" ||
            cmd.includes("history") ||
            cmd.includes("models") ||
            cmd.includes("devices") ||
            cmd === "meetings_list"
          )
            return [];
          return null;
        },
      },
    });
  });
});

const SCRIPT = "<Erzählerin> Es war einmal.\n<Bob> Wer bin ich?";

async function openReadAloud(page: Page) {
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  const editor = page.locator("textarea").first();
  await editor.fill(SCRIPT);
  return editor;
}

const actions = (page: Page) =>
  page.locator('.tts-controls button[data-testid^="tts-action-"]');

async function openMenu(page: Page) {
  await page.getByTestId("tts-action-menu").click();
  await expect(page.getByRole("menu")).toBeVisible();
}

test("all action buttons are the same size and show no text", async ({
  page,
}) => {
  await openReadAloud(page);
  // Die Knoepfe stehen nicht schon beim ersten Bild da: erst warten, dann messen.
  await expect.poll(() => actions(page).count()).toBeGreaterThanOrEqual(5);
  const boxes = await actions(page).evaluateAll((els) =>
    els.map((el) => {
      const r = el.getBoundingClientRect();
      // Der Zaehler am Menue (`*-badge`: Pruefbefunde) ist gewollt Text. Er
      // erscheint erst, wenn die entprellte Skriptpruefung fertig ist, also je
      // nach Rechnerlast vor oder nach dieser Messung; zur Aussage "Symbol-
      // Knoepfe ohne Beschriftung" gehoert er nicht (eigener Test: "the error
      // count shows on the menu button").
      const label = el.cloneNode(true) as HTMLElement;
      label
        .querySelectorAll('[data-testid$="-badge"]')
        .forEach((badge) => badge.remove());
      return {
        id: el.getAttribute("data-testid"),
        w: r.width,
        h: r.height,
        text: (label.textContent ?? "").trim(),
      };
    }),
  );
  expect(boxes.length).toBeGreaterThanOrEqual(5);
  for (const box of boxes) {
    expect(Math.abs(box.w - boxes[0].w), `${box.id} width`).toBeLessThanOrEqual(
      1,
    );
    expect(
      Math.abs(box.h - boxes[0].h),
      `${box.id} height`,
    ).toBeLessThanOrEqual(1);
    expect(box.text, `${box.id} text`).toBe("");
  }
  expect(Math.round(boxes[0].w)).toBe(36);
});

test("every lucide symbol appears at most once in the control column", async ({
  page,
}) => {
  for (const tab of ["Original", "Übersetzung", "Zusammenfassung"]) {
    await openReadAloud(page);
    await page.getByRole("tab", { name: tab, exact: true }).click();
    const classes = await page
      .locator('.tts-controls svg[class*="lucide-"]')
      .evaluateAll((els) =>
        els.map(
          (el) =>
            (el.getAttribute("class") ?? "")
              .split(/\s+/)
              .find((c) => c.startsWith("lucide-")) ?? "",
        ),
      );
    expect(classes.length, tab).toBeGreaterThanOrEqual(3);
    const duplicates = classes.filter((c, i) => classes.indexOf(c) !== i);
    expect(duplicates, `${tab}: doppelte Symbole`).toEqual([]);
  }
});

test("original tab: add, dictate, save, prewarm and menu share one row", async ({
  page,
}) => {
  await openReadAloud(page);
  const ids = ["add", "dictate", "save", "prewarm", "menu"];
  const boxes = [];
  for (const id of ids) {
    const box = await page.getByTestId(`tts-action-${id}`).boundingBox();
    expect(box, id).not.toBeNull();
    boxes.push(box!);
  }
  for (const box of boxes) {
    expect(Math.abs(box.y - boxes[0].y)).toBeLessThanOrEqual(2);
  }
  // Hinzufuegen steht links zuerst, das Menue rechts am Ende.
  for (let i = 1; i < boxes.length; i++) {
    expect(boxes[i].x).toBeGreaterThan(boxes[i - 1].x);
  }
});

test("the menu offers workshop, tidy, check and auto-tagging and each runs", async ({
  page,
}) => {
  const editor = await openReadAloud(page);
  const menuButton = page.getByTestId("tts-action-menu");
  await expect(menuButton).toHaveAttribute("aria-haspopup", "menu");
  await expect(menuButton).toHaveAttribute("aria-expanded", "false");
  await openMenu(page);
  await expect(menuButton).toHaveAttribute("aria-expanded", "true");
  const items = page.getByRole("menuitem");
  await expect(items).toHaveCount(4);
  await expect(items.nth(0)).toContainText("Skript-Werkstatt");
  await expect(items.nth(1)).toContainText("Text aufbereiten");
  await expect(items.nth(2)).toContainText("Skript prüfen");
  await expect(items.nth(3)).toContainText("Auto-Tagging");

  // Werkstatt: Dialog auf.
  await page.getByTestId("workshop-open").click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("Skript-Werkstatt");
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);

  // Pruefen: Befund-Panel sichtbar (Bob ist kein bekannter Sprecher).
  await openMenu(page);
  await page.getByTestId("script-check-run").click();
  await expect(page.getByTestId("script-check")).toBeVisible();
  // Nach der Wahl schliesst das Menue.
  await expect(page.getByRole("menu")).toHaveCount(0);

  // Auto-Tagging: derselbe Dialog wie bisher.
  await openMenu(page);
  await page.getByTestId("autotag-open").click();
  await expect(page.getByRole("dialog")).toContainText("Auto-Tagging");
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);

  // Aufbereiten: Befehl geht raus, Text wird ersetzt.
  await openMenu(page);
  await page.getByRole("menuitem", { name: "Text aufbereiten" }).click();
  await expect(editor).toHaveValue("Sauberer Text.");
  const calls = await page.evaluate(
    () => (window as unknown as { __calls: string[] }).__calls,
  );
  expect(calls).toContain("tts_tidy_text");
});

test("the menu is keyboard operable and closes on Escape and outside click", async ({
  page,
}) => {
  await openReadAloud(page);
  const menuButton = page.getByTestId("tts-action-menu");
  await menuButton.focus();
  await page.keyboard.press("Enter");
  const items = page.getByRole("menuitem");
  await expect(items.first()).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(items.nth(1)).toBeFocused();
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("ArrowUp");
  await expect(items.last()).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("menu")).toHaveCount(0);
  await expect(menuButton).toBeFocused();
  // Klick daneben schliesst ebenfalls.
  await openMenu(page);
  await page.locator("textarea").first().click();
  await expect(page.getByRole("menu")).toHaveCount(0);
});

test("the add button opens the sources menu", async ({ page }) => {
  await openReadAloud(page);
  await page.getByTestId("tts-action-add").click();
  const items = page.getByRole("menuitem");
  await expect(items).toHaveCount(3);
  await expect(items.nth(0)).toContainText("Dokument laden");
  await expect(items.nth(1)).toContainText("Webseite laden");
  await expect(items.nth(2)).toContainText("Datei zu den Projekt-Dateien");
  await items.nth(1).click();
  await expect(page.getByRole("dialog")).toBeVisible();
});

test("a tooltip names and explains the action on hover and on keyboard focus", async ({
  page,
}) => {
  await openReadAloud(page);
  const save = page.getByTestId("tts-action-save");
  // Hover: erst nach kurzer Verzoegerung.
  await save.hover();
  const tip = page.getByRole("tooltip");
  await expect(tip).toBeVisible();
  await expect(tip).toContainText("Als Audio speichern");
  await expect(tip.locator("strong")).toHaveCount(1);
  // Name fett + eine Erklaerungszeile.
  expect((await tip.innerText()).split("\n").length).toBeGreaterThanOrEqual(2);
  const id = await tip.getAttribute("id");
  expect(id).toBeTruthy();
  expect(await save.getAttribute("aria-describedby")).toBe(id);
  await page.mouse.move(600, 500);
  await expect(page.getByRole("tooltip")).toHaveCount(0);

  // Tastaturfokus: Tab zum Knopf, sofort sichtbar; Esc schliesst.
  await page.getByTestId("tts-action-add").focus();
  await page.keyboard.press("Tab");
  await page.keyboard.press("Tab");
  await expect(page.getByTestId("tts-action-save")).toBeFocused();
  await expect(page.getByRole("tooltip")).toContainText("Als Audio speichern");
  await page.keyboard.press("Escape");
  await expect(page.getByRole("tooltip")).toHaveCount(0);
  // Der Zustand steht im Namen: Diktat laeuft -> "Aufnahme beenden".
  await expect(page.getByTestId("tts-action-dictate")).toHaveAttribute(
    "aria-label",
    "Diktieren",
  );
});

test("the voice hint is a description of the voice select, not a permanent line", async ({
  page,
}) => {
  await openReadAloud(page);
  const hint = page.getByTestId("voice-mode-hint");
  await expect(hint).toContainText("Sprecher im Skript");
  // Nicht mehr als sichtbare Dauerzeile.
  const hintBox = await hint.boundingBox();
  expect(hintBox!.width).toBeLessThanOrEqual(1);
  expect(hintBox!.height).toBeLessThanOrEqual(1);
  const describedBy = await page
    .getByTestId("voice-select")
    .getAttribute("aria-describedby");
  expect(describedBy).toBe(await hint.getAttribute("id"));
});

test("the voice select names a missing Piper voice instead of showing the raw value", async ({
  page,
}) => {
  // Die Attrappe steht auf Piper ohne gewaehlte Stimme: kein "piper:" im Feld.
  await openReadAloud(page);
  const select = page.getByTestId("voice-select");
  await expect(select).toContainText("Piper – keine Stimme gewählt");
  await expect(select).not.toContainText("piper:");
});

test("translation and summary tabs show their action as a symbol", async ({
  page,
}) => {
  await openReadAloud(page);
  await page.getByRole("tab", { name: "Übersetzung", exact: true }).click();
  const translate = page.getByTestId("tts-action-translate");
  await expect(translate).toBeVisible();
  await expect(translate).toHaveAttribute("aria-label", /Übersetzen/);
  expect(((await translate.textContent()) ?? "").trim()).toBe("");
  await expect(page.getByTestId("tts-action-save")).toBeVisible();
  await expect(page.getByTestId("tts-action-prewarm")).toBeVisible();
  // Zielsprache in derselben Zeile und Hoehe.
  const row = await translate.boundingBox();
  const lang = await page
    .locator(".tts-controls .app-select__control")
    .last()
    .boundingBox();
  expect(Math.abs(lang!.y - row!.y)).toBeLessThanOrEqual(2);
  expect(Math.abs(lang!.height - row!.height)).toBeLessThanOrEqual(1);

  await page.getByRole("tab", { name: "Zusammenfassung", exact: true }).click();
  const summarize = page.getByTestId("tts-action-summarize");
  await expect(summarize).toBeVisible();
  expect(((await summarize.textContent()) ?? "").trim()).toBe("");
  const options = page.getByTestId("tts-action-summary-options");
  await expect(options).toHaveAttribute("aria-haspopup", /dialog|true|menu/);
  await options.click();
  const popover = page.getByTestId("tts-summary-options");
  await expect(popover).toBeVisible();
  await expect(popover.locator(".app-select__control")).toHaveCount(3);
  await expect(popover).toContainText("Umfang");
  await expect(popover).toContainText("Detailgrad");
  await expect(popover).toContainText("Zielgruppe");
  await page.keyboard.press("Escape");
  await expect(popover).toHaveCount(0);
  await expect(options).toBeFocused();
});

test("the error count shows on the menu button and on the check entry", async ({
  page,
}) => {
  await openReadAloud(page);
  const badge = page.getByTestId("tts-action-menu-badge");
  await expect(badge).toBeVisible();
  const count = ((await badge.textContent()) ?? "").trim();
  expect(Number(count)).toBeGreaterThan(0);
  await openMenu(page);
  await expect(page.getByTestId("script-check-badge")).toHaveText(count);
});

// Echte Bedienbarkeit statt toBeVisible(): Ein Menue, das die scrollende
// Bedienspalte abschneidet, ist fuers DOM "sichtbar", aber nicht anklickbar.
// Darum trifft die Mitte jedes Eintrags per elementFromPoint den Eintrag selbst,
// und der Eintrag liegt ganz im Fenster.
async function expectFullyUsable(page: Page, root: string, what: string) {
  const report = await page.evaluate((selector) => {
    const items = Array.from(
      document.querySelectorAll<HTMLElement>(selector),
    ).filter((el) => el.offsetParent !== null || el.getClientRects().length);
    return items.map((el) => {
      const r = el.getBoundingClientRect();
      const hit = document.elementFromPoint(
        r.left + r.width / 2,
        r.top + r.height / 2,
      );
      return {
        name: (el.getAttribute("data-testid") ?? el.textContent ?? "").trim(),
        hit: !!hit && (hit === el || el.contains(hit)),
        inside:
          r.left >= 0 &&
          r.top >= 0 &&
          r.right <= window.innerWidth &&
          r.bottom <= window.innerHeight,
      };
    });
  }, root);
  expect(report.length, `${what}: keine Eintraege`).toBeGreaterThan(0);
  for (const entry of report) {
    expect(
      entry.hit,
      `${what}: "${entry.name}" ist verdeckt/abgeschnitten`,
    ).toBe(true);
    expect(entry.inside, `${what}: "${entry.name}" ragt aus dem Fenster`).toBe(
      true,
    );
  }
}

const controlsScrollTop = (page: Page) =>
  page.getByTestId("tts-controls").evaluate((el) => el.scrollTop);

for (const size of [
  { width: 1920, height: 1050 },
  { width: 1366, height: 768 },
]) {
  test(`menus stay fully usable in the stacked control column at ${size.width}x${size.height}`, async ({
    page,
  }) => {
    await page.setViewportSize(size);
    await openReadAloud(page);
    await expect(page.getByTestId("tts-controls")).toHaveClass(
      /tts-controls--stacked/,
    );

    // Menue (Original-Reiter).
    await openMenu(page);
    await expectFullyUsable(page, '[role="menuitem"]', "Menue");
    expect(await controlsScrollTop(page), "Menue scrollt die Spalte").toBe(0);
    await page.keyboard.press("Escape");
    await expect(page.getByRole("menu")).toHaveCount(0);

    // Hinzufuegen-Menue.
    await page.getByTestId("tts-action-add").click();
    await expect(page.getByRole("menu")).toBeVisible();
    await expectFullyUsable(page, '[role="menuitem"]', "Hinzufuegen");
    expect(await controlsScrollTop(page), "Hinzufuegen scrollt").toBe(0);
    await page.keyboard.press("Escape");

    // Optionen-Popover (Zusammenfassung).
    await page
      .getByRole("tab", { name: "Zusammenfassung", exact: true })
      .click();
    await page.getByTestId("tts-action-summary-options").click();
    await expect(page.getByTestId("tts-summary-options")).toBeVisible();
    await expectFullyUsable(
      page,
      '[data-testid="tts-summary-options"] .app-select__control',
      "Optionen",
    );
    const panel = await page.getByTestId("tts-summary-options").boundingBox();
    expect(panel!.y + panel!.height).toBeLessThanOrEqual(size.height);
    expect(await controlsScrollTop(page), "Optionen scrollt").toBe(0);
  });
}

// Auswahllisten in der gestapelten Bedienspalte: Die Liste schwebt ueber dem
// Dateibereich (Portal an body), die Bedienspalte scrollt dabei nie. Belegt
// wird das wie bei den Menues per elementFromPoint statt toBeVisible().
const VOICES = ["anna", "ben", "clara", "dora", "emil", "fritz", "gustav"];

async function expectFloatingSelectMenu(
  page: Page,
  size: { width: number; height: number },
  what: string,
  minOptions = 4,
) {
  const menu = page.locator(".app-select__menu");
  await expect(menu, `${what}: Liste offen`).toBeVisible();
  const report = await page.evaluate(() => {
    const controls = document.querySelector<HTMLElement>(
      '[data-testid="tts-controls"]',
    )!;
    const menuEl = document.querySelector<HTMLElement>(".app-select__menu")!;
    const list = menuEl.querySelector<HTMLElement>(".app-select__menu-list")!;
    const listRect = list.getBoundingClientRect();
    const menuRect = menuEl.getBoundingClientRect();
    const options = Array.from(
      menuEl.querySelectorAll<HTMLElement>(".app-select__option"),
    )
      .map((el) => ({ el, r: el.getBoundingClientRect() }))
      // Sichtbar = ganz im Ausschnitt der (ggf. eigens scrollenden) Liste.
      .filter(
        ({ r }) =>
          r.top >= listRect.top - 0.5 && r.bottom <= listRect.bottom + 0.5,
      )
      .map(({ el, r }) => {
        const hit = document.elementFromPoint(
          r.left + r.width / 2,
          r.top + r.height / 2,
        );
        return {
          name: (el.textContent ?? "").trim(),
          hit: !!hit && (hit === el || el.contains(hit)),
        };
      });
    return {
      controlsScrollTop: controls.scrollTop,
      controlsOverflow: controls.scrollHeight - controls.clientHeight,
      inWindow:
        menuRect.left >= 0 &&
        menuRect.top >= 0 &&
        menuRect.right <= window.innerWidth &&
        menuRect.bottom <= window.innerHeight,
      inControls: controls.contains(menuEl),
      options,
    };
  });
  expect(report.controlsScrollTop, `${what}: Spalte scrollt`).toBe(0);
  expect(
    report.controlsOverflow,
    `${what}: aeussere Scrollbar in der Bedienspalte`,
  ).toBeLessThanOrEqual(1);
  expect(report.inControls, `${what}: Liste steckt in der Bedienspalte`).toBe(
    false,
  );
  expect(report.inWindow, `${what}: Liste ragt aus dem Fenster`).toBe(true);
  expect(
    report.options.length,
    `${what}: zu wenige sichtbare Eintraege`,
  ).toBeGreaterThanOrEqual(minOptions);
  for (const option of report.options) {
    expect(option.hit, `${what}: "${option.name}" ist verdeckt`).toBe(true);
  }
  void size;
}

for (const size of [
  { width: 1920, height: 1050 },
  { width: 1366, height: 768 },
]) {
  test(`select lists float above the file area in the stacked column at ${size.width}x${size.height}`, async ({
    page,
  }) => {
    await page.addInitScript((voices) => {
      (window as unknown as { __voices: string[] }).__voices = voices;
    }, VOICES);
    await page.setViewportSize(size);
    await openReadAloud(page);
    await expect(page.getByTestId("tts-controls")).toHaveClass(
      /tts-controls--stacked/,
    );

    // Stimmen-Select: Liste offen, Wahl uebernimmt die Stimme.
    const voice = page.getByTestId("voice-select");
    await voice.locator(".app-select__control").click();
    await expectFloatingSelectMenu(page, size, "Stimme");
    await page
      .locator(".app-select__option", { hasText: /^dora$/ })
      .evaluate((el) => (el as HTMLElement).click());
    await expect(page.locator(".app-select__menu")).toHaveCount(0);
    await expect(voice).toContainText("dora");

    // Zielsprache (Reiter Uebersetzung).
    await page.getByRole("tab", { name: "Übersetzung", exact: true }).click();
    const lang = page.locator(".tts-controls .app-select__control").last();
    await lang.click();
    await expectFloatingSelectMenu(page, size, "Zielsprache");
    const langOption = page.locator(".app-select__option").nth(1);
    const langName = ((await langOption.textContent()) ?? "").trim();
    await langOption.evaluate((el) => (el as HTMLElement).click());
    await expect(page.locator(".app-select__menu")).toHaveCount(0);
    await expect(lang).toContainText(langName);

    // Select im Optionen-Popover: Popover bleibt bis zur Wahl offen.
    await page
      .getByRole("tab", { name: "Zusammenfassung", exact: true })
      .click();
    await page.getByTestId("tts-action-summary-options").click();
    const popover = page.getByTestId("tts-summary-options");
    await expect(popover).toBeVisible();
    const first = popover.locator(".app-select__control").first();
    await first.click();
    await expectFloatingSelectMenu(page, size, "Popover-Select", 3);
    await expect(popover).toBeVisible();
    // Escape schliesst zuerst nur die Liste, nicht das Popover dahinter.
    await page.keyboard.press("Escape");
    await expect(page.locator(".app-select__menu")).toHaveCount(0);
    await expect(popover).toBeVisible();
    await first.click();
    await expect(page.locator(".app-select__menu")).toBeVisible();
    const optionBox = await page
      .locator(".app-select__option", { hasText: "Lang" })
      .boundingBox();
    // Echter Mausklick (mousedown zaehlt fuer die Aussenklick-Erkennung).
    await page.mouse.click(
      optionBox!.x + optionBox!.width / 2,
      optionBox!.y + optionBox!.height / 2,
    );
    await expect(popover).toBeVisible();
    await expect(first).toContainText("Lang");
    await expect(page.locator(".app-select__menu")).toHaveCount(0);
  });
}

test("screenshots of the control column", async ({ page }) => {
  test.skip(
    !process.env.SCREENS_DIR,
    "nur mit SCREENS_DIR (Aufnahme, kein Verhaltenstest)",
  );
  await page.addInitScript((voices) => {
    (window as unknown as { __voices: string[] }).__voices = voices;
  }, VOICES);
  await page.setViewportSize({ width: 1920, height: 1050 });
  await openReadAloud(page);
  const dir = path.resolve(process.env.SCREENS_DIR!);
  await page
    .getByTestId("voice-select")
    .locator(".app-select__control")
    .click();
  await page.screenshot({
    path: path.join(dir, "stimmen-offen-1920.png"),
    animations: "disabled",
  });
  await page.keyboard.press("Escape");
  await openMenu(page);
  await page.screenshot({
    path: path.join(dir, "menue-offen-1920.png"),
    animations: "disabled",
  });
  await page.keyboard.press("Escape");
  await page.getByTestId("tts-action-prewarm").hover();
  await expect(page.getByRole("tooltip")).toBeVisible();
  await page.screenshot({
    path: path.join(dir, "tooltip-1920.png"),
    animations: "disabled",
  });
});
