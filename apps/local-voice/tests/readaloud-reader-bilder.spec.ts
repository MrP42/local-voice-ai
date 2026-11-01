import { test, expect, type Page } from "@playwright/test";

// Vorlesen als Buch-Reader (lva#148): Leseansicht mit Seiten, Sprecher am Rand,
// Hervorhebung des gelesenen Satzes, Spielerleiste mit Seitensprung, kompakte
// Befundleiste. Die Klappe "Ausdruck & Sprechstil" unter dem Text gibt es nicht mehr
// (Patrick 01.11.: der Ausdruck kommt über das Kontextmenü).
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
      theme: "system",
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
      tts_engine: "piper",
      tts_voice: null,
      tts_piper_voice: null,
      tts_script_check: false,
    };
    const w = window as unknown as Record<string, unknown>;
    const listeners: Record<string, number[]> = {};
    w.__calls = [] as unknown[];
    w.__emit = (name: string, payload: unknown) =>
      (listeners[name] ?? []).forEach((id) =>
        (callbacks.get(id) as (e: unknown) => void)?.({
          event: name,
          id: 0,
          payload,
        }),
      );
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
          (w.__calls as unknown[]).push([cmd, args]);
          if (cmd === "get_app_settings" || cmd === "get_default_settings")
            return JSON.parse(JSON.stringify(settings));
          if (cmd === "plugin:os|locale") return "de-DE";
          if (cmd === "plugin:app|version") return "0.21.21";
          if (cmd.includes("permission")) return true;
          if (cmd === "plugin:event|listen") {
            const name = String(args?.event);
            (listeners[name] ??= []).push(Number(args?.handler));
            return ++callback;
          }
          if (cmd === "get_selected_model") return "";
          if (cmd === "meetings_is_recording") return false;
          if (cmd === "tts_server_status")
            return { phase: "stopped", message: null };
          if (cmd === "pages_list") return [{ id: "p1", title: "Die Leuchtturmwärterin" }];
          if (cmd === "page_state_load")
            return JSON.stringify({
              text: (window as unknown as { __pageText?: string }).__pageText ?? "",
              tab: "original",
            });
          if (cmd === "page_dir") return "C:\\projects\\p1";
          if (cmd === "tts_cached_translation") return null;
          if (cmd === "tts_list_downloads") return [];
          if (
            cmd === "llm_ps" ||
            cmd === "tts_reading_list" ||
            cmd === "tts_list_voices" ||
            cmd === "tts_list_voice_infos" ||
            cmd === "page_files" ||
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


const BOOK = [
  "# Kapitel 1",
  "",
  "<Erzählerin> Der Leuchtturm stand seit über hundert Jahren auf der Klippe. Er war ca. 42 Meter **hoch**.",
  "",
  "<Mara> „Noch zwölf Stufen“, sagte sie [pause] zu niemandem.",
  "",
  "<Bob> Ich bin neu hier und habe keine Stimme.",
  "",
  "<Bob> Noch einmal Bob.",
  "",
  ...Array.from(
    { length: 40 },
    (_, i) =>
      `<Erzählerin> Absatz ${i + 1}: Die Stunden vergingen zäh, und Mara lauschte auf jedes Geräusch, das vom Meer heraufkam.`,
  ),
  "",
  "<Mara> Das ist der letzte Satz des Buches.",
].join("\n");

const open = async (page: Page, text = BOOK, width = 1440) => {
  await page.setViewportSize({ width, height: 900 });
  await page.addInitScript((t) => {
    (window as unknown as { __pageText: string }).__pageText = t;
    window.localStorage.removeItem("lva.ui.tts.view");
  }, text);
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
};

const emit = (page: Page, name: string, payload: unknown) =>
  page.evaluate(
    ([name, payload]) =>
      (window as unknown as { __emit: (n: string, p: unknown) => void }).__emit(
        name as string,
        payload,
      ),
    [name, payload] as const,
  );

const calls = (page: Page, cmd: string) =>
  page.evaluate(
    (cmd) =>
      ((window as unknown as { __calls: [string, Record<string, unknown>][] })
        .__calls ?? [])
        .filter(([c]) => c === cmd)
        .map(([, a]) => a),
    cmd,
  );


// Bilder der Buchansicht für Abnahmen (lva#148): nur mit SCREENS_DIR, z. B.
//   SCREENS_DIR=D:/bilder npx playwright test tests/readaloud-reader-bilder.spec.ts
const DIR = process.env.SCREENS_DIR ?? "";
for (const theme of ["light", "dark"]) {
  for (const width of [2560, 1440, 390]) {
    test(`bild ${theme} ${width}`, async ({ page }) => {
      test.skip(!DIR, "nur mit SCREENS_DIR");
      await page.addInitScript((t) => {
        (window as unknown as { __theme: string }).__theme = t;
      }, theme);
      await page.emulateMedia({ colorScheme: theme as "light" | "dark" });
      await open(page, BOOK, width === 2560 ? 2560 : 1440);
      if (width === 2560)
        await page.getByTestId("reader-book").and(page.locator('[data-per-view="2"]')).waitFor();
      if (width === 390) await page.setViewportSize({ width: 390, height: 844 });
      await page.getByTestId("reader-flow").waitFor();
      await page.getByTestId("tts-action-menu").click();
      await page.getByTestId("script-check-run").click();
      await page.getByTestId("script-check").waitFor();
      await emit(page, "tts-speak-progress", { position: 2, total: 46 });
      await emit(page, "tts-current-sentence", { context: "speak", index: 2, text: "Er war ca. 42 Meter hoch." });
      await page.waitForTimeout(600);
      if (width === 390) {
        await page.getByTestId("reader-player").screenshot({
          path: `${DIR}/reader-${theme}-390-spieler.png`,
        });
      }
      await page.screenshot({ path: `${DIR}/reader-${theme}-${width}.png`, fullPage: false });
    });
  }
}

// Weiterlesen nach Neustart (lva#160, Codex): 1440, 768 und 390 px; bei 390 px
// über den Menüknopf, wie ein Nutzer auf dem Telefon.
const gotoVorlesen = async (page: Page, width: number) => {
  if (width < 600) {
    await page.locator(".lv-titlebar__hamburger").click();
  }
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .filter({ visible: true })
    .first()
    .click();
};

for (const theme of ["light", "dark"]) {
  for (const width of [1440, 768, 390]) {
    test(`bild weiterlesen ${theme} ${width}`, async ({ page }) => {
      test.skip(!DIR, "nur mit SCREENS_DIR");
      await page.addInitScript((t) => {
        (window as unknown as { __theme: string }).__theme = t;
      }, theme);
      await page.emulateMedia({ colorScheme: theme as "light" | "dark" });
      await open(page, BOOK, 1440);
      await page
        .getByTestId("reader-flow")
        .getByText("Noch zwölf Stufen", { exact: false })
        .click();
      await expect
        .poll(async () => (await calls(page, "tts_speak_text_from")).length)
        .toBe(1);
      await emit(page, "tts-current-sentence", {
        context: "speak",
        index: 23,
        text: "Absatz 20: Die Stunden vergingen zäh, und Mara lauschte auf jedes Geräusch, das vom Meer heraufkam.",
      });
      await expect(page.locator(".reader-cur").first()).toContainText("Absatz 20");
      await page.setViewportSize({ width, height: width < 600 ? 844 : 900 });
      await page.reload();
      await gotoVorlesen(page, width);
      await page.getByTestId("reader-flow").waitFor();
      await page.waitForTimeout(800);
      await page.screenshot({ path: `${DIR}/weiterlesen-${theme}-${width}.png` });
      if (width < 1000) {
        await page.getByTestId("reader-more").click();
        await page.getByTestId("reader-more-resume").waitFor();
        await page.waitForTimeout(200);
        await page.screenshot({ path: `${DIR}/weiterlesen-${theme}-${width}-menue.png` });
        await page.keyboard.press("Escape");
      }
      await page.getByTestId("reader-bookmarks").click();
      await page.getByTestId("reader-bookmark-add").click();
      await page.getByTestId("reader-bookmarks").click();
      await page.waitForTimeout(200);
      await page.screenshot({ path: `${DIR}/lesezeichen-${theme}-${width}.png` });
    });
  }
}

// Inhaltsverzeichnis links und Kapitel in der Spielerleiste (lva#159).
const CHAPTERS = [
  "# Die Leuchtturmwärterin",
  "",
  "## Erstes Kapitel: Die Klippe",
  "",
  ...Array.from(
    { length: 8 },
    (_, i) =>
      `<Erzählerin> Absatz ${i + 1}: Die Stunden vergingen zäh, und Mara lauschte auf jedes Geräusch, das vom Meer heraufkam.`,
  ),
  "",
  "## Zweites Kapitel: Der Sturm",
  "",
  ...Array.from(
    { length: 8 },
    (_, i) =>
      `<Mara> Zeile ${i + 1}: Der Wind drehte, und das Licht fiel schräg durch das schmale Fenster.`,
  ),
  "",
  "### Ein kurzer Abschnitt",
  "",
  "<Erzählerin> Danach wurde es still.",
  "",
  "## Drittes Kapitel: Morgen",
  "",
  "<Erzählerin> Das ist der letzte Satz des Buches.",
].join("\n");

for (const theme of ["light", "dark"]) {
  for (const width of [1440, 390]) {
    test(`bild inhalt ${theme} ${width}`, async ({ page }) => {
      test.skip(!DIR, "nur mit SCREENS_DIR");
      await page.addInitScript((t) => {
        (window as unknown as { __theme: string }).__theme = t;
      }, theme);
      await page.emulateMedia({ colorScheme: theme as "light" | "dark" });
      await page.setViewportSize({ width, height: width < 600 ? 844 : 900 });
      await page.addInitScript((t) => {
        (window as unknown as { __pageText: string }).__pageText = t;
        window.localStorage.removeItem("lva.ui.tts.view");
      }, CHAPTERS);
      await page.goto("/");
      await gotoVorlesen(page, width);
      await page.getByTestId("reader-flow").waitFor();
      await page
        .getByTestId("reader-flow")
        .getByText("Zeile 2:", { exact: false })
        .click();
      await emit(page, "tts-speak-progress", { position: 12, total: 24 });
      await emit(page, "tts-current-sentence", {
        context: "speak",
        index: 12,
        text: "Zeile 2: Der Wind drehte, und das Licht fiel schräg durch das schmale Fenster.",
      });
      await page.waitForTimeout(800);
      await page.screenshot({ path: `${DIR}/inhalt-${theme}-${width}.png` });
      // Eingeklappt: das aktuelle Kapitel steht im Kopf.
      await page.getByTestId("reader-contents-toggle").click();
      await page.waitForTimeout(200);
      await page.screenshot({ path: `${DIR}/inhalt-eingeklappt-${theme}-${width}.png` });
      await page.getByTestId("reader-contents-toggle").click();
      if (width < 600) {
        // Schmal steht das Kapitel im Überlaufmenü der Spielerleiste.
        await page.getByTestId("reader-more").click();
        await page.getByTestId("reader-more-where").waitFor();
        await page.waitForTimeout(200);
        await page.screenshot({ path: `${DIR}/inhalt-spieler-menue-${theme}-${width}.png` });
      }
    });
  }
}
