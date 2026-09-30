import { test, expect, type Page } from "@playwright/test";
import * as path from "node:path";

// Vorlesen-Arbeitsflaeche: eine gestapelte Spalte rechts vom Editor
// (Bedienung oben, Dateien/Hilfe darunter) oder zwei Spalten nebeneinander;
// Seitenliste und rechte Spalte sind per Ziehgriff verstellbar. Dieselbe
// Tauri-Attrappe wie in script-check.spec.ts, nur schlanker.
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
      tts_engine: "fish",
      tts_piper_voice: null,
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
          if (cmd === "pages_list")
            return [
              { id: "p1", title: "Der Sturm" },
              { id: "p2", title: "Zweite Seite" },
            ];
          if (cmd === "page_files")
            return [
              {
                name: "Der-Sturm_2026-09-08_1405.wav",
                size: 120,
                modified_ms: 0,
              },
              { name: "notizen.txt", size: 12, modified_ms: 0 },
            ];
          if (cmd === "page_dir") return "C:/projects/p1";
          if (
            cmd === "tts_list_voices" ||
            cmd === "tts_list_voice_infos" ||
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

/** Erster Start zeigt den Reiter "Hilfe"; die bisherigen Tests messen die
 *  Dateiliste und schalten deshalb ausdruecklich auf "Dateien" um.
 *  `firstStart` laesst den Standard stehen. */
async function openReadAloud(
  page: Page,
  width = 1920,
  height = 1050,
  firstStart = false,
) {
  await page.setViewportSize({ width, height });
  await page.goto("/");
  await goToReadAloud(page);
  if (!firstStart) {
    await page
      .getByTestId("tts-files")
      .getByRole("tab", { name: "Dateien", exact: true })
      .click();
  }
}

async function goToReadAloud(page: Page) {
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await expect(page.getByTestId("tts-controls")).toBeVisible();
  await expect(page.getByTestId("tts-files")).toBeVisible();
}

const box = async (page: Page, testId: string) => {
  const b = await page.getByTestId(testId).boundingBox();
  expect(b, testId).not.toBeNull();
  return b!;
};

/** Ziehgriff mit der Maus um `dx` Pixel verschieben. */
async function drag(page: Page, testId: string, dx: number) {
  const b = await box(page, testId);
  const x = b.x + b.width / 2;
  const y = b.y + b.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + dx / 2, y, { steps: 4 });
  await page.mouse.move(x + dx, y, { steps: 4 });
  await page.mouse.up();
}

for (const [width, height] of [
  [1920, 1050],
  [1366, 768],
] as const) {
  test(`stacked by default: one column right of the editor (${width}x${height})`, async ({
    page,
  }) => {
    await openReadAloud(page, width, height);
    const controls = await box(page, "tts-controls");
    const files = await box(page, "tts-files");
    const column = await box(page, "tts-right-column");
    const editor = await page.locator(".tts-editor").boundingBox();
    // Genau eine Spalte rechts vom Editor: Bedienung und Dateien teilen
    // sich dieselbe x-Spanne, die Dateien liegen darunter.
    expect(Math.abs(files.x - controls.x)).toBeLessThanOrEqual(2);
    expect(Math.abs(files.width - controls.width)).toBeLessThanOrEqual(2);
    expect(files.y).toBeGreaterThan(controls.y + controls.height - 2);
    expect(controls.x).toBeGreaterThanOrEqual(editor!.x + editor!.width);
    expect(column.width).toBeCloseTo(320, 0);
    // Die Bedienung frisst nicht mehr als die Haelfte der Fensterhoehe.
    expect(controls.height).toBeLessThanOrEqual(height * 0.5);
    // Dateien haben sichtbar Platz (nicht auf 0 zusammengedrueckt).
    expect(files.height).toBeGreaterThan(150);
    await expect(page.getByTestId("layout-toggle")).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  });
}

// AK5 (Issue #61): gestapelt bleibt die Bedienung kompakt UND braucht keinen
// eigenen Scrollbalken -- auch mit Text im Editor (dann sind Vorlesen, Speichern
// usw. aktiv). Die Dateien beginnen erst unter der Bedienung.
for (const [width, height] of [
  [1920, 1050],
  [1366, 768],
] as const) {
  test(`AK5: stacked controls stay short and do not scroll with text (${width}x${height})`, async ({
    page,
  }) => {
    await openReadAloud(page, width, height);
    await page
      .locator("textarea")
      .first()
      .fill(
        "Olga: Das ist ein Satz. Und noch ein zweiter Satz.\n\nNoch ein Absatz.",
      );
    await expect(page.getByTestId("layout-toggle")).toHaveAttribute(
      "aria-pressed",
      "false",
    );
    const controls = page.getByTestId("tts-controls");
    const metrics = await controls.evaluate((el) => ({
      height: el.getBoundingClientRect().height,
      scrollHeight: el.scrollHeight,
      clientHeight: el.clientHeight,
      bottom: el.getBoundingClientRect().bottom,
      innerHeight: window.innerHeight,
    }));
    expect(metrics.height).toBeLessThanOrEqual(metrics.innerHeight * 0.5);
    expect(metrics.scrollHeight).toBeLessThanOrEqual(metrics.clientHeight + 1);
    const files = await box(page, "tts-files");
    expect(files.y).toBeGreaterThanOrEqual(metrics.bottom - 1);
  });
}

test("layout toggle switches to side by side and survives a reload", async ({
  page,
}) => {
  await openReadAloud(page);
  await page.getByTestId("layout-toggle").click();
  await expect(page.getByTestId("layout-toggle")).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  const check = async () => {
    const controls = await box(page, "tts-controls");
    const files = await box(page, "tts-files");
    // Wie bisher: Bedienung w-72 (18 rem), Dateien rechts daneben.
    const rem = await page.evaluate(() =>
      parseFloat(getComputedStyle(document.documentElement).fontSize),
    );
    expect(controls.width).toBeCloseTo(18 * rem, 0);
    expect(files.x).toBeGreaterThanOrEqual(controls.x + controls.width);
    expect(Math.abs(files.y - controls.y)).toBeLessThanOrEqual(2);
    expect(files.width).toBeCloseTo(240, 0);
  };
  await check();
  await page.reload();
  await expect(page.getByTestId("tts-controls")).toBeVisible();
  await expect(page.getByTestId("layout-toggle")).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await check();
  // Und zurueck.
  await page.getByTestId("layout-toggle").click();
  const controls = await box(page, "tts-controls");
  const files = await box(page, "tts-files");
  expect(files.y).toBeGreaterThan(controls.y);
});

test("collapsing the files only folds the lower part in stacked layout", async ({
  page,
}) => {
  await openReadAloud(page);
  await page.getByRole("button", { name: "Dateileiste einklappen" }).click();
  await expect(page.getByTestId("tts-controls")).toBeVisible();
  await expect(page.getByTestId("tts-files")).toHaveCount(0);
  await expect(page.getByTestId("resize-right")).toBeVisible();
  await page.getByRole("button", { name: "Dateileiste ausklappen" }).click();
  await expect(page.getByTestId("tts-files")).toBeVisible();
});

test("pages handle: drag, arrow keys, persistence, double click, limits", async ({
  page,
}) => {
  await openReadAloud(page);
  const width = async () => (await box(page, "tts-pages")).width;
  expect(await width()).toBeCloseTo(208, 0);

  await drag(page, "resize-pages", 40);
  expect(await width()).toBeCloseTo(248, 0);

  const handle = page.getByTestId("resize-pages");
  await expect(handle).toHaveAttribute("role", "separator");
  await expect(handle).toHaveAttribute("aria-orientation", "vertical");
  await expect(handle).toHaveAttribute("aria-valuenow", "248");
  await expect(handle).toHaveAttribute("aria-valuemin", "160");
  await expect(handle).toHaveAttribute("aria-valuemax", "420");
  await handle.focus();
  await page.keyboard.press("ArrowRight");
  expect(await width()).toBeCloseTo(264, 0);
  await page.keyboard.press("ArrowLeft");
  await page.keyboard.press("ArrowLeft");
  expect(await width()).toBeCloseTo(232, 0);

  await page.reload();
  await expect(page.getByTestId("tts-pages")).toBeVisible();
  expect(await width()).toBeCloseTo(232, 0);

  await page.getByTestId("resize-pages").dblclick();
  expect(await width()).toBeCloseTo(208, 0);

  await drag(page, "resize-pages", 900);
  expect(await width()).toBeCloseTo(420, 0);
  await drag(page, "resize-pages", -900);
  expect(await width()).toBeCloseTo(160, 0);
});

test("right handle: drag, arrow keys, persistence, double click, limits", async ({
  page,
}) => {
  await openReadAloud(page);
  const width = async () => (await box(page, "tts-right-column")).width;
  expect(await width()).toBeCloseTo(320, 0);

  // Nach links ziehen macht die rechte Spalte breiter.
  await drag(page, "resize-right", -60);
  expect(await width()).toBeCloseTo(380, 0);

  const handle = page.getByTestId("resize-right");
  await expect(handle).toHaveAttribute("aria-valuenow", "380");
  await expect(handle).toHaveAttribute("aria-valuemin", "260");
  await expect(handle).toHaveAttribute("aria-valuemax", "560");
  await handle.focus();
  await page.keyboard.press("ArrowLeft");
  expect(await width()).toBeCloseTo(396, 0);
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("ArrowRight");
  expect(await width()).toBeCloseTo(364, 0);

  await page.reload();
  await expect(page.getByTestId("tts-right-column")).toBeVisible();
  expect(await width()).toBeCloseTo(364, 0);

  await page.getByTestId("resize-right").dblclick();
  expect(await width()).toBeCloseTo(320, 0);

  await drag(page, "resize-right", -900);
  expect(await width()).toBeCloseTo(560, 0);
  await drag(page, "resize-right", 900);
  expect(await width()).toBeCloseTo(260, 0);
});

test("side by side: the handle sizes the files column", async ({ page }) => {
  await openReadAloud(page);
  await page.getByTestId("layout-toggle").click();
  const width = async () => (await box(page, "tts-files")).width;
  expect(await width()).toBeCloseTo(240, 0);
  await drag(page, "resize-right", -40);
  expect(await width()).toBeCloseTo(280, 0);
  await expect(page.getByTestId("resize-right")).toHaveAttribute(
    "aria-valuemax",
    "520",
  );
  await drag(page, "resize-right", -900);
  expect(await width()).toBeCloseTo(520, 0);
  await drag(page, "resize-right", 900);
  expect(await width()).toBeCloseTo(200, 0);
  await page.getByTestId("resize-right").dblclick();
  expect(await width()).toBeCloseTo(240, 0);
});

test("the editor keeps at least 360 px in a narrow window", async ({
  page,
}) => {
  await openReadAloud(page, 1280, 800);
  await drag(page, "resize-right", -900);
  await drag(page, "resize-pages", 900);
  const editor = await page.locator(".tts-editor").boundingBox();
  expect(editor!.width).toBeGreaterThanOrEqual(358);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBeTruthy();
});

test("handles are hidden in the wrapped layout (<= 1100 px)", async ({
  page,
}) => {
  await page.setViewportSize({ width: 900, height: 900 });
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await expect(page.getByTestId("tts-controls")).toBeVisible();
  await expect(page.getByTestId("resize-pages")).toBeHidden();
  await expect(page.getByTestId("resize-right")).toBeHidden();
});

// Patricks Abnahme 0.20.4 (P10): die rechte Leiste faengt mit "Hilfe" an,
// der Ausklappen-Knopf sitzt immer rechts, und Reiter- wie Klappzustand
// ueberstehen Neuladen, Seitenwechsel und Modulwechsel.
const tabOf = (page: Page, name: "Dateien" | "Hilfe") =>
  page.getByTestId("tts-files").getByRole("tab", { name, exact: true });

/** Seite in der Seitenliste waehlen. Die Zeilenaktionen blenden beim Hover
 *  ueber dem Titel ein und fangen einen Klick in der Mitte ab; links in der
 *  Zeile ist frei. */
async function selectPage(page: Page, title: string) {
  await page
    .getByTestId("tts-pages")
    .locator(".group")
    .filter({ hasText: title })
    .click({ position: { x: 6, y: 6 } });
}

test("first start: stacked, files column open, help tab selected", async ({
  page,
}) => {
  await openReadAloud(page, 1920, 1050, true);
  await expect(page.getByTestId("layout-toggle")).toHaveAttribute(
    "aria-pressed",
    "false",
  );
  await expect(tabOf(page, "Hilfe")).toHaveAttribute("aria-selected", "true");
  await expect(tabOf(page, "Dateien")).toHaveAttribute(
    "aria-selected",
    "false",
  );
  await expect(page.getByTestId("help-panel")).toBeVisible();
  const help = await box(page, "help-panel");
  expect(help.height).toBeGreaterThan(50);
  // Kein gespeicherter Wert vorher: der Standard kommt aus dem Code.
  // Ein vorhandener Wert bleibt unangetastet (siehe Persistenz-Tests).
});

for (const [width, height] of [
  [1920, 1050],
  [1366, 768],
] as const) {
  test(`collapsed: the expand button sits at the right edge, stacked and side by side (${width}x${height})`, async ({
    page,
  }) => {
    await openReadAloud(page, width, height);
    const collapse = page.getByRole("button", {
      name: "Dateileiste einklappen",
    });
    const headButton = (await collapse.boundingBox())!;
    await collapse.click();
    const expand = page.getByTestId("files-expand");
    await expect(expand).toBeVisible();
    // Gestapelt: rechte Kante des Knopfs = rechte Kante der rechten Spalte.
    const column = await box(page, "tts-right-column");
    const stackedButton = await box(page, "files-expand");
    expect(
      Math.abs(
        stackedButton.x + stackedButton.width - (column.x + column.width),
      ),
    ).toBeLessThanOrEqual(2);
    // Und der Knopf ist tatsaechlich das oberste Element an seiner Stelle.
    const hit = await expand.evaluate((el) => {
      const r = el.getBoundingClientRect();
      const top = document.elementFromPoint(
        r.x + r.width / 2,
        r.y + r.height / 2,
      );
      return el === top || el.contains(top);
    });
    expect(hit).toBe(true);
    // Gleiche Flaeche wie die Leistenkopf-Knoepfe (gemessen am Einklappen-Knopf).
    expect(stackedButton.width).toBeCloseTo(headButton.width, 0);
    expect(stackedButton.height).toBeCloseTo(headButton.height, 0);

    // Nebeneinander: ganz rechts im Arbeitsbereich. Der Umschalter sitzt im
    // Leistenkopf, also erst aufklappen, umschalten, wieder zuklappen.
    await expand.click();
    await page.getByTestId("layout-toggle").click();
    await collapse.click();
    await expect(expand).toBeVisible();
    const workspace = (await page.locator(".tts-workspace").boundingBox())!;
    const sideButton = await box(page, "files-expand");
    const gapRight =
      workspace.x + workspace.width - (sideButton.x + sideButton.width);
    expect(gapRight).toBeGreaterThanOrEqual(-1);
    expect(gapRight).toBeLessThanOrEqual(2);
  });
}

test("right tab choice survives reload, page switch and module switch", async ({
  page,
}) => {
  await openReadAloud(page, 1920, 1050, true);
  await expect(tabOf(page, "Hilfe")).toHaveAttribute("aria-selected", "true");
  await tabOf(page, "Dateien").click();
  await expect(tabOf(page, "Dateien")).toHaveAttribute("aria-selected", "true");
  await expect(page.getByTestId("tts-files")).toContainText("notizen.txt");

  // Neuladen.
  await page.reload();
  await goToReadAloud(page);
  await expect(tabOf(page, "Dateien")).toHaveAttribute("aria-selected", "true");

  // Zweite Seite in der Seitenliste.
  await selectPage(page, "Zweite Seite");
  await expect(tabOf(page, "Dateien")).toHaveAttribute("aria-selected", "true");
  await selectPage(page, "Der Sturm");
  await expect(tabOf(page, "Dateien")).toHaveAttribute("aria-selected", "true");

  // Modulwechsel: Verlauf und zurueck.
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Verlauf", exact: true })
    .click();
  await expect(page.getByTestId("tts-files")).toHaveCount(0);
  await goToReadAloud(page);
  await expect(tabOf(page, "Dateien")).toHaveAttribute("aria-selected", "true");

  // Und zurueck auf Hilfe: bleibt ebenfalls.
  await tabOf(page, "Hilfe").click();
  await page.reload();
  await goToReadAloud(page);
  await expect(tabOf(page, "Hilfe")).toHaveAttribute("aria-selected", "true");
});

test("collapsed files column survives reload, page switch and module switch", async ({
  page,
}) => {
  await openReadAloud(page);
  await page.getByRole("button", { name: "Dateileiste einklappen" }).click();
  const collapsed = async () => {
    await expect(page.getByTestId("files-expand")).toBeVisible();
    await expect(page.getByTestId("tts-files")).toHaveCount(0);
  };
  await collapsed();

  await page.reload();
  await expect(page.getByTestId("tts-controls")).toBeVisible();
  await collapsed();

  await selectPage(page, "Zweite Seite");
  await collapsed();

  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Verlauf", exact: true })
    .click();
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await expect(page.getByTestId("tts-controls")).toBeVisible();
  await collapsed();

  // Wieder aufklappen bleibt ebenso.
  await page.getByTestId("files-expand").click();
  await expect(page.getByTestId("tts-files")).toBeVisible();
  await page.reload();
  await goToReadAloud(page);
});

test("screenshots: first start and collapsed files (P10)", async ({ page }) => {
  test.skip(
    !process.env.SCREENS_DIR,
    "nur mit SCREENS_DIR (Aufnahme, kein Verhaltenstest)",
  );
  const dir = path.resolve(process.env.SCREENS_DIR!);
  await openReadAloud(page, 1920, 1050, true);
  await page.screenshot({
    path: path.join(dir, "erster-start-1920.png"),
    animations: "disabled",
  });
  await page.getByRole("button", { name: "Dateileiste einklappen" }).click();
  await expect(page.getByTestId("files-expand")).toBeVisible();
  await page.screenshot({
    path: path.join(dir, "gestapelt-zugeklappt-1920.png"),
    animations: "disabled",
  });
});

test("screenshots: stacked and side by side", async ({ page }) => {
  test.skip(
    !process.env.SCREENS_DIR,
    "nur mit SCREENS_DIR (Aufnahme, kein Verhaltenstest)",
  );
  const dir = path.resolve(process.env.SCREENS_DIR!);
  await openReadAloud(page);
  await page.screenshot({
    path: path.join(dir, "gestapelt-1920.png"),
    animations: "disabled",
  });
  await page.getByTestId("layout-toggle").click();
  await page.screenshot({
    path: path.join(dir, "nebeneinander-1920.png"),
    animations: "disabled",
  });
});
