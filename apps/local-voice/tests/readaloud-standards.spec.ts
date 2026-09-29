import { test, expect, type Locator, type Page } from "@playwright/test";
import * as path from "node:path";

// Vorlesen: Seiten-/Dateileiste und Editor-Reiter folgen dem App-Standard
// (Goal ui-vorlesen-kompakt, P6): Reiter mit Rollen und Tastaturführung,
// Klickflächen der Zeilenaktionen >= 24 px, keine Schrift unter 11 px,
// sichtbarer Zustand des Anhören-Knopfs, Zeitstempel in der Oberflächensprache.
//
// Die Tauri-Brücke ist eine schlanke Variante der Attrappe aus
// readaloud-layout.spec.ts, mit zwei Seiten und Dateien mit Größe/Zeit.
//
// Aufnahme fürs Protokoll (kein Verhaltenstest, nur mit SCREENS_DIR):
//   SCREENS_DIR=../../koordination/ui-vorlesen-kompakt/screens/p6 \
//   LV_DEV_PORT=1616 pnpm exec playwright test readaloud-standards --reporter=line

const AUDIO = "Der-Sturm_2026-09-08_1405.wav";

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
    const now = Date.now();
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
          if (cmd === "plugin:app|version") return "0.16.0";
          if (cmd.includes("permission")) return true;
          if (cmd === "plugin:event|listen") return ++callback;
          if (cmd === "get_selected_model") return "";
          if (cmd === "meetings_is_recording") return false;
          if (cmd === "tts_server_status")
            return { phase: "stopped", message: null };
          if (cmd === "pages_list")
            return [
              {
                id: "p1",
                title: "Der Sturm",
                preview: "Es war eine dunkle Nacht.",
                modified_ms: now - 20 * 60_000,
              },
              {
                id: "p2",
                title: "Zweite Seite",
                preview: "Noch ein Text.",
                modified_ms: now - 3 * 86_400_000,
              },
            ];
          if (cmd === "page_files")
            return [
              {
                name: "Der-Sturm_2026-09-08_1405.wav",
                size: 4_500_000,
                modified_ms: now,
              },
              { name: "notizen.txt", size: 12, modified_ms: now },
            ];
          if (cmd === "page_dir") return "C:/projects/p1";
          if (cmd === "page_audio_note")
            return args?.name === "Der-Sturm_2026-09-08_1405.wav"
              ? {
                  text: "Es war eine dunkle Nacht. Niemand sprach.",
                  voice: "erzaehlerin",
                  seed: 42,
                  // 08.09.2025, mittags UTC: in jeder Zeitzone derselbe Tag.
                  created_ms: 1757333100000,
                  segments: [
                    {
                      text: "Es war eine dunkle Nacht.",
                      voice: "erzaehlerin",
                      start_ms: 0,
                      end_ms: 2000,
                    },
                    {
                      text: "Niemand sprach.",
                      voice: "leo-lausemaus",
                      start_ms: 2000,
                      end_ms: 4000,
                    },
                  ],
                }
              : null;
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

async function openReadAloud(page: Page) {
  await page.setViewportSize({ width: 1920, height: 1050 });
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await expect(page.getByTestId("tts-pages")).toBeVisible();
  await expect(page.getByTestId("tts-files")).toBeVisible();
  // Erster Start zeigt den Reiter "Hilfe"; die Dateizeilen brauchen "Dateien".
  await page
    .getByTestId("tts-files")
    .getByRole("tab", { name: "Dateien", exact: true })
    .click();
  await expect(page.getByTestId("tts-files").getByText(AUDIO)).toBeVisible();
}

const size = async (locator: Locator) => {
  const b = await locator.boundingBox();
  expect(b, "Element ohne Box").not.toBeNull();
  return b!;
};

/** Gehe die Reiter mit Pfeiltasten, Pos1 und Ende durch. */
async function expectKeyboardTabs(page: Page, tabs: Locator[]): Promise<void> {
  const selected = async (index: number) => {
    for (const [i, tab] of tabs.entries()) {
      await expect(tab).toHaveAttribute(
        "aria-selected",
        i === index ? "true" : "false",
      );
      // Roving tabIndex: nur der gewählte Reiter ist ein Tabstopp.
      await expect(tab).toHaveAttribute("tabindex", i === index ? "0" : "-1");
    }
    await expect(tabs[index]).toBeFocused();
  };
  await tabs[0].focus();
  await page.keyboard.press("ArrowRight");
  await selected(1);
  await page.keyboard.press("ArrowLeft");
  await selected(0);
  // Umlauf: links vom ersten landet beim letzten, rechts vom letzten beim ersten.
  await page.keyboard.press("ArrowLeft");
  await selected(tabs.length - 1);
  await page.keyboard.press("ArrowRight");
  await selected(0);
  await page.keyboard.press("End");
  await selected(tabs.length - 1);
  await page.keyboard.press("Home");
  await selected(0);
}

test("editor tabs are real tabs with keyboard navigation and standard height", async ({
  page,
}) => {
  await openReadAloud(page);
  const original = page.getByRole("tab", { name: "Original", exact: true });
  const translation = page.getByRole("tab", {
    name: "Übersetzung",
    exact: true,
  });
  const summary = page.getByRole("tab", {
    name: "Zusammenfassung",
    exact: true,
  });

  await expect(original).toHaveAttribute("aria-selected", "true");
  await expectKeyboardTabs(page, [original, translation, summary]);

  // Standardmaß der Einstellungen-Reiter (min-h-11), nicht mehr 32 px.
  for (const tab of [original, translation, summary]) {
    expect((await size(tab)).height).toBeGreaterThanOrEqual(40);
  }
  await expect(original).toHaveCSS("font-weight", "500");
  // Inaktiver Reiter wie in den Einstellungen: text/60 statt text/50.
  await expect(translation).toHaveClass(/text-text\/60/);
});

test("files/help tabs are real tabs with keyboard navigation", async ({
  page,
}) => {
  await openReadAloud(page);
  const files = page.getByTestId("tts-files");
  const dateien = files.getByRole("tab", { name: "Dateien", exact: true });
  const hilfe = files.getByRole("tab", { name: "Hilfe", exact: true });

  await expect(dateien).toHaveAttribute("aria-selected", "true");
  await dateien.focus();
  await page.keyboard.press("ArrowRight");
  await expect(hilfe).toBeFocused();
  await expect(hilfe).toHaveAttribute("aria-selected", "true");
  await expect(page.getByTestId("help-panel")).toBeVisible();
  await page.keyboard.press("Home");
  await expect(dateien).toBeFocused();
  await expect(dateien).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("End");
  await expect(hilfe).toBeFocused();

  // In der schmalen Leiste darf es kompakter sein, aber nicht unter 32 px.
  expect((await size(dateien)).height).toBeGreaterThanOrEqual(32);
  // Unterstrich statt Pille: kein Hintergrund, font-medium, keine Großbuchstaben.
  await expect(dateien).toHaveCSS("font-weight", "500");
  await expect(dateien).toHaveCSS("text-transform", "none");
  await expect(dateien).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
});

// Abnahme 0.20.4: beim Hover blendeten Umbenennen + Loeschen ein und schoben
// den Papierkorb genau dorthin, wo eben noch "Anhoeren" stand. Jetzt sind alle
// Zeilenaktionen immer da und stehen fest.
test("file row actions are always visible and never move on hover", async ({
  page,
}) => {
  await openReadAloud(page);
  const files = page.getByTestId("tts-files");
  const audioRow = files.locator(".group").filter({ hasText: AUDIO });
  const textRow = files.locator(".group").filter({ hasText: "notizen.txt" });
  const names = ["Anhören", "Umbenennen", "Datei löschen"] as const;

  const measure = async (row: Locator) => {
    const out: Record<string, { x: number; y: number; w: number; h: number }> =
      {};
    for (const name of names) {
      const button = row.getByRole("button", { name, exact: true });
      if ((await button.count()) === 0) continue;
      // Sichtbar heisst hier: das Element an der Stelle IST der Knopf.
      const hit = await button.evaluate((el) => {
        const r = el.getBoundingClientRect();
        const top = document.elementFromPoint(
          r.x + r.width / 2,
          r.y + r.height / 2,
        );
        return !!top && (el === top || el.contains(top));
      });
      expect(hit, `${name} sichtbar und oben`).toBe(true);
      const b = await size(button);
      out[name] = { x: b.x, y: b.y, w: b.width, h: b.height };
    }
    return out;
  };

  // Maus sicher weg von der Zeile.
  await page.mouse.move(2, 2);
  const audioBefore = await measure(audioRow);
  expect(Object.keys(audioBefore)).toEqual([...names]);
  const textBefore = await measure(textRow);
  expect(Object.keys(textBefore)).toEqual(["Umbenennen", "Datei löschen"]);

  await audioRow.hover({ position: { x: 20, y: 6 } });
  const audioAfter = await measure(audioRow);
  for (const name of names) {
    for (const key of ["x", "y", "w", "h"] as const) {
      expect(
        Math.abs(audioAfter[name][key] - audioBefore[name][key]),
        `${name}.${key}`,
      ).toBeLessThanOrEqual(1);
    }
  }
  await textRow.hover({ position: { x: 20, y: 6 } });
  const textAfter = await measure(textRow);
  for (const name of ["Umbenennen", "Datei löschen"] as const) {
    for (const key of ["x", "w", "h"] as const) {
      expect(
        Math.abs(textAfter[name][key] - textBefore[name][key]),
        `${name}.${key} (Textdatei)`,
      ).toBeLessThanOrEqual(1);
    }
  }
  // Reihenfolge von links: Anhören, Umbenennen, Löschen (ganz aussen).
  expect(audioBefore["Anhören"].x).toBeLessThan(audioBefore["Umbenennen"].x);
  expect(audioBefore["Umbenennen"].x).toBeLessThan(
    audioBefore["Datei löschen"].x,
  );
  // Nicht-Audio: Löschen steht auf derselben x-Position wie in der Audiozeile
  // (der Platz des Anhören-Knopfs bleibt frei).
  expect(
    Math.abs(textBefore["Datei löschen"].x - audioBefore["Datei löschen"].x),
  ).toBeLessThanOrEqual(1);
  expect(
    Math.abs(textBefore["Umbenennen"].x - audioBefore["Umbenennen"].x),
  ).toBeLessThanOrEqual(1);
  // Die Größe steht in einer Metazeile und wechselt nicht mit dem Hover.
  await expect(audioRow.getByText(/^\d+(\.\d+)? ?(B|KB|MB)$/)).toBeVisible();
});

test("every row action has a click target of at least 24x24 px", async ({
  page,
}) => {
  await openReadAloud(page);

  const check = async (row: Locator, expected: number) => {
    await row.hover();
    const buttons = row.getByRole("button");
    await expect(buttons).toHaveCount(expected);
    for (let index = 0; index < expected; index++) {
      const button = buttons.nth(index);
      await expect(button).toBeVisible();
      const b = await size(button);
      const label = await button.getAttribute("aria-label");
      expect(b.width, label ?? "").toBeGreaterThanOrEqual(24);
      expect(b.height, label ?? "").toBeGreaterThanOrEqual(24);
      // Nur 16-px-Symbole in Zeilen und Köpfen.
      const icon = await size(button.locator("svg"));
      expect(icon.width, label ?? "").toBe(16);
    }
  };

  // Seitenzeile: hoch, runter, umbenennen, exportieren, löschen.
  await check(page.getByTestId("tts-pages").locator(".group").nth(0), 5);
  // Audio-Dateizeile: anhören, umbenennen, löschen.
  await check(
    page.getByTestId("tts-files").locator(".group").filter({ hasText: AUDIO }),
    3,
  );
  // Textdatei: nur umbenennen und löschen.
  await check(
    page
      .getByTestId("tts-files")
      .locator(".group")
      .filter({ hasText: "notizen.txt" }),
    2,
  );

  // Leisten-Köpfe: ebenfalls nur 16 px.
  for (const id of ["tts-pages", "tts-files"]) {
    // Ausgeblendete Zeilenaktionen (display: none) haben keine Fläche: übersprungen.
    const widths = await page
      .getByTestId(id)
      .locator("button svg")
      .evaluateAll((icons) =>
        icons
          .map((icon) => icon.getBoundingClientRect().width)
          .filter((width) => width > 0),
      );
    expect(widths.length, id).toBeGreaterThan(0);
    for (const width of widths) expect(width, id).toBe(16);
  }
});

/** Kleinste berechnete Schriftgröße eines Textelements unter `root`. */
const smallestText = (root: Locator) =>
  root.evaluate((element) => {
    let min = Infinity;
    let which = "";
    const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
    while (walker.nextNode()) {
      const node = walker.currentNode;
      if (!node.textContent?.trim()) continue;
      const parent = node.parentElement!;
      const px = parseFloat(getComputedStyle(parent).fontSize);
      if (px < min) {
        min = px;
        which = `${parent.className} "${node.textContent.trim().slice(0, 30)}"`;
      }
    }
    return { min, which };
  });

test("no text in the side bars is smaller than 11 px, meta text is readable", async ({
  page,
}) => {
  await openReadAloud(page);
  // Aufnahme aufklappen: Herkunft, Zeit und Satzliste sind die kleinsten Texte.
  await page
    .getByTestId("tts-files")
    .getByRole("button", { name: "Anhören" })
    .click();
  await expect(page.getByTestId("tts-files").getByRole("listitem")).toHaveCount(
    2,
  );

  for (const id of ["tts-pages", "tts-files"]) {
    const { min, which } = await smallestText(page.getByTestId(id));
    expect(min, `${id}: ${which}`).toBeGreaterThanOrEqual(11);
  }

  // Metatext mindestens text/60 (vorher /35 bis /45).
  const files = page.getByTestId("tts-files");
  await expect(files.getByText(/^\d+(\.\d)? MB$/).first()).toHaveClass(
    /text-text\/60/,
  );
  await expect(
    page.getByTestId("tts-pages").getByText("Es war eine dunkle Nacht."),
  ).toHaveClass(/text-text\/60/);
  await expect(
    files.getByRole("listitem").filter({ hasText: "Niemand sprach." }),
  ).toHaveClass(/text-text\/60/);
});

test("the listen button shows its open state", async ({ page }) => {
  await openReadAloud(page);
  const row = page
    .getByTestId("tts-files")
    .locator(".group")
    .filter({ hasText: AUDIO });
  const listen = row.getByRole("button", { name: "Anhören" });

  await expect(listen).toHaveAttribute("aria-pressed", "false");
  await expect(listen.locator("svg.lucide-play")).toHaveCount(1);
  await expect(listen).not.toHaveClass(/bg-logo-primary\/15/);

  await listen.click();
  await expect(listen).toHaveAttribute("aria-pressed", "true");
  await expect(listen).toHaveClass(/bg-logo-primary\/15/);
  await expect(listen.locator("svg.lucide-square")).toHaveCount(1);
  await expect(listen.locator("svg.lucide-play")).toHaveCount(0);

  // Und wieder zu: das Symbol kehrt zurück.
  await listen.click();
  await expect(listen).toHaveAttribute("aria-pressed", "false");
  await expect(listen.locator("svg.lucide-play")).toHaveCount(1);
});

test("the recording time follows the UI language (German format)", async ({
  page,
}) => {
  await openReadAloud(page);
  await page
    .getByTestId("tts-files")
    .getByRole("button", { name: "Anhören" })
    .click();
  const meta = page.getByTestId("tts-files").getByText(/erzaehlerin ·/);
  await expect(meta).toBeVisible();
  // T.M.JJJJ (de-Locale ohne führende Nullen), 24-Stunden-Zeit; nie „9/29/2026, 11:45 AM".
  await expect(meta).toHaveText(/\d{1,2}\.\d{1,2}\.\d{4}, \d{2}:\d{2}:\d{2}/);
  await expect(meta).not.toHaveText(/AM|PM|\d\/\d/);
});

test("screenshot of both side bars with an opened recording", async ({
  page,
}) => {
  const dir = process.env.SCREENS_DIR;
  test.skip(!dir, "nur mit SCREENS_DIR (Aufnahme, kein Verhaltenstest)");
  await openReadAloud(page);
  await page
    .getByTestId("tts-files")
    .getByRole("button", { name: "Anhören" })
    .click();
  await expect(page.getByTestId("tts-files").getByRole("listitem")).toHaveCount(
    2,
  );
  // Zeile mit eingeblendeten Aktionen zeigen.
  await page.getByTestId("tts-pages").locator(".group").nth(0).hover();
  await page.screenshot({
    path: path.resolve(dir!, "leisten-1920.png"),
    animations: "disabled",
  });
});
