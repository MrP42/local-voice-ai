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

test("eine Seite mit Text öffnet in der Leseansicht: Sprecher am Rand, keine Tags im Text", async ({
  page,
}) => {
  await open(page);
  const flow = page.getByTestId("reader-flow");
  await expect(flow).toBeVisible();
  await expect(page.locator("textarea")).toHaveCount(0);
  await expect(flow.getByRole("heading", { name: "Kapitel 1" })).toBeVisible();
  // Sprecher als Randmarke, nicht als <Tag> im Fließtext.
  await expect(flow).not.toContainText("<Erzählerin>");
  await expect(flow).not.toContainText("[pause]");
  await expect(flow).not.toContainText("**");
  await expect(
    flow.getByRole("button", { name: /Sprecher Mara/ }).first(),
  ).toBeVisible();
  await expect(flow.locator("strong", { hasText: "hoch" })).toBeVisible();
  // Die Klappe "Ausdruck & Sprechstil" ist weg.
  await expect(page.getByText("Ausdruck & Sprechstil", { exact: true })).toHaveCount(0);
});

test("der gelesene Satz ist hervorgehoben und die Seite blättert mit", async ({
  page,
}) => {
  await open(page);
  const book = page.getByTestId("reader-book");
  await expect(book).toHaveAttribute("data-page", "0");
  await emit(page, "tts-speak-progress", { position: 0, total: 44 });
  await emit(page, "tts-current-sentence", {
    context: "speak",
    index: 0,
    text: "Kapitel 1.",
  });
  // Die Überschrift trägt im Sprechtext einen Punkt, auf der Seite nicht.
  await expect(page.locator(".reader-cur")).toHaveText("Kapitel 1");
  await emit(page, "tts-current-sentence", {
    context: "speak",
    index: 1,
    text: "Der Leuchtturm stand seit über hundert Jahren auf der Klippe.",
  });
  await expect(page.locator(".reader-cur").first()).toContainText(
    "Der Leuchtturm stand",
  );
  // Abkürzung im Satz: "ca." trennt keinen Satz, die Hervorhebung deckt ihn ganz.
  await emit(page, "tts-current-sentence", {
    context: "speak",
    index: 2,
    text: "Er war ca. 42 Meter hoch.",
  });
  await expect(page.locator(".reader-cur")).toHaveText(["Er war ca. 42 Meter ", "hoch", "."]);
  // Ein Satz weit hinten: die Ansicht blättert dorthin.
  await emit(page, "tts-current-sentence", {
    context: "speak",
    index: 42,
    text: "Das ist der letzte Satz des Buches.",
  });
  // Erst warten, bis die alte Hervorhebung weg ist: eine Mehrdeutigkeit bricht
  // toHaveText sofort ab, statt erneut zu prüfen.
  await expect(page.locator(".reader-cur")).toHaveCount(1);
  await expect(page.locator(".reader-cur")).toHaveText("Das ist der letzte Satz des Buches.");
  await expect(book).not.toHaveAttribute("data-page", "0");
  const last = Number(await book.getAttribute("data-pages"));
  expect(last).toBeGreaterThan(2);
  await expect(page.locator(".reader-cur")).toBeInViewport();
});

test("selbst blättern beendet das Mitblättern, „Zur Vorlesestelle“ holt zurück", async ({
  page,
}) => {
  await open(page);
  await emit(page, "tts-speak-progress", { position: 1, total: 44 });
  await emit(page, "tts-current-sentence", {
    context: "speak",
    index: 1,
    text: "Der Leuchtturm stand seit über hundert Jahren auf der Klippe.",
  });
  await page.getByTestId("reader-next-page").click();
  await expect(page.getByTestId("reader-follow")).toBeVisible();
  await page.getByTestId("reader-follow").click();
  await expect(page.getByTestId("reader-book")).toHaveAttribute("data-page", "0");
  await expect(page.getByTestId("reader-follow")).toHaveCount(0);
});

test("Seitensprung in der Spielerleiste und Pfeiltasten im Buch", async ({
  page,
}) => {
  await open(page);
  const book = page.getByTestId("reader-book");
  const pages = Number(await book.getAttribute("data-pages"));
  const perView = Number(await book.getAttribute("data-per-view"));
  await expect(page.getByTestId("reader-page-count")).toHaveText(`/ ${pages}`);
  await page.getByTestId("reader-jump").fill("3");
  await page.getByTestId("reader-jump").press("Enter");
  await expect(book).toHaveAttribute("data-page", String(2 - (2 % perView)));
  await page.getByTestId("reader-flow").focus();
  await page.keyboard.press("Home");
  await expect(book).toHaveAttribute("data-page", "0");
  await page.keyboard.press("ArrowRight");
  await expect(book).toHaveAttribute("data-page", String(perView));
});

test("zwei Seiten nur, wenn der Platz reicht; sonst sagt der Tooltip warum", async ({
  page,
}) => {
  await open(page, BOOK, 2560);
  await expect(page.getByTestId("reader-book")).toHaveAttribute("data-per-view", "2");
  await page.setViewportSize({ width: 1000, height: 900 });
  await expect(page.getByTestId("reader-book")).toHaveAttribute("data-per-view", "1");
  await expect(page.getByTestId("reader-two-pages")).toHaveAttribute(
    "title",
    /zu schmal/,
  );
  await page.getByTestId("reader-one-page").click();
  await expect(page.getByTestId("reader-two-pages")).toHaveAttribute(
    "title",
    "Zwei Seiten (aufgeschlagenes Buch)",
  );
});

test("Klick in den Text liest ab dieser Stelle, ohne Markdown an die Engine", async ({
  page,
}) => {
  await open(page);
  await page
    .getByTestId("reader-flow")
    .getByText("Noch zwölf Stufen", { exact: false })
    .click();
  await expect.poll(async () => (await calls(page, "tts_speak_text_from")).length).toBe(1);
  const [args] = await calls(page, "tts_speak_text_from");
  const text = String(args.text);
  expect(text).not.toContain("**");
  expect(text).not.toContain("# Kapitel");
  expect(text.startsWith("Kapitel 1.")).toBe(true);
  // Der Offset zeigt in den bereinigten Text, auf den Satz mit dem Klick.
  const offset = Number(args.charOffset);
  expect(text.slice(offset - 20, offset + 20)).toContain("zwölf");
});

test("Bearbeiten zeigt den Editor; Tippen schaltet nicht in die Leseansicht", async ({
  page,
}) => {
  await open(page, "");
  await expect(page.getByTestId("reader-mode-read")).toBeDisabled();
  const editor = page.locator("textarea").first();
  await editor.fill("Ein neuer Text. Noch ein Satz.");
  await expect(page.locator("textarea")).toHaveCount(1);
  await page.getByTestId("reader-mode-read").click();
  await expect(page.getByTestId("reader-flow")).toContainText("Ein neuer Text.");
  await page.getByTestId("reader-mode-edit").click();
  await expect(page.locator("textarea").first()).toHaveValue(
    "Ein neuer Text. Noch ein Satz.",
  );
});

test("Darstellungsmenü: Schriftgröße und Sprecher am Rand", async ({ page }) => {
  await open(page);
  await page.getByTestId("reader-display").click();
  const menu = page.getByRole("menu", { name: "Schrift und Darstellung" });
  await menu.getByRole("menuitemradio", { name: "Groß" }).click();
  await expect(page.getByTestId("reader-book")).toHaveAttribute("style", /--reader-fs: 21px/);
  await menu.getByRole("menuitemcheckbox", { name: "Sprecher am Rand zeigen" }).click();
  await expect(
    page.getByTestId("reader-flow").getByRole("button", { name: /Sprecher Mara/ }),
  ).toHaveCount(0);
});

test("Spielerleiste: Fortschritt, Satz n von m und Tooltips", async ({ page }) => {
  await open(page);
  const bar = page.getByTestId("reader-player");
  await expect(bar.getByRole("button", { name: "Vorlesen" })).toBeEnabled();
  await emit(page, "tts-speak-progress", { position: 9, total: 44 });
  await expect(page.getByTestId("reader-where")).toHaveText("Satz 10 von 44");
  await expect(bar.getByRole("button", { name: "Vorheriger Satz" })).toHaveAttribute(
    "title",
    "Vorheriger Satz",
  );
  await expect(page.getByTestId("reader-sleep")).toHaveAttribute("title", "Timer");
  await page.getByTestId("reader-sleep").click();
  await page.getByRole("menuitemradio", { name: "15 Minuten" }).click();
  await expect(page.getByTestId("reader-sleep")).toContainText("noch 15 min");
  // Escape schließt die Auswahl wieder.
  await page.getByTestId("reader-sleep").click();
  await expect(page.getByRole("menu", { name: "Timer" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("menu", { name: "Timer" })).toHaveCount(0);
});

test("Spielerleiste bei 390 px: einzeilig, Tempo und Timer im Überlaufmenü", async ({
  page,
}) => {
  await open(page, BOOK, 1440);
  await page.setViewportSize({ width: 390, height: 844 });
  const bar = page.getByTestId("reader-player");
  await expect(page.getByTestId("reader-sleep")).toBeHidden();
  await expect(page.getByTestId("reader-where")).toBeHidden();
  const more = page.getByTestId("reader-more");
  await expect(more).toBeVisible();
  // Transport und Überlauf stehen auf einer Zeile.
  const play = await bar.getByTestId("reader-play").boundingBox();
  const mb = await more.boundingBox();
  expect(play && mb).toBeTruthy();
  expect(Math.abs(play!.y + play!.height / 2 - (mb!.y + mb!.height / 2))).toBeLessThan(8);
  // Keine horizontale Überbreite der Leiste.
  const overflow = await bar.evaluate((el) => el.scrollWidth - el.clientWidth);
  expect(overflow).toBeLessThanOrEqual(1);
  await more.click();
  const menu = page.getByRole("menu", { name: "Weitere Optionen: Tempo und Timer" });
  await expect(menu).toBeVisible();
  await menu.getByRole("menuitemradio", { name: "30 Minuten" }).click();
  await expect(menu).toHaveCount(0);
  await more.click();
  await expect(menu).toContainText("noch 30 min");
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);
});
