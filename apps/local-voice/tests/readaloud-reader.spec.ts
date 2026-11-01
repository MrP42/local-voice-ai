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
          const parts = (
            window as unknown as { __parts?: Record<string, string> }
          ).__parts;
          if (cmd === "pages_list")
            return parts
              ? Object.keys(parts).map((id, i) => ({ id, title: `Teil ${i + 1}` }))
              : [{ id: "p1", title: "Die Leuchtturmwärterin" }];
          if (cmd === "books_list")
            return parts
              ? [
                  {
                    id: "b1",
                    title: "Buch",
                    created_ms: 0,
                    page_ids: Object.keys(parts),
                    characters: [],
                  },
                ]
              : [];
          if (cmd === "page_state_load")
            return JSON.stringify({
              text:
                parts?.[String(args?.id)] ??
                (window as unknown as { __pageText?: string }).__pageText ??
                "",
              tab: "original",
            });
          if ((cmd === "tts_speak_text" || cmd === "tts_speak_resume") && parts)
            // Die Sitzung endet erst, wenn der Test es sagt.
            return new Promise((done) => {
              (window as unknown as { __endSpeak: () => void }).__endSpeak = () =>
                done(null);
            });
          if (cmd === "page_dir") return "C:\\projects\\p1";
          if (cmd === "pages_create")
            return { id: "p2", title: String(args?.title ?? ""), modified_ms: 0, preview: "" };
          if (cmd === "plugin:dialog|open")
            return (window as unknown as { __pick?: string }).__pick ?? null;
          if (cmd === "tts_extract_document")
            return (window as unknown as { __extracted?: string }).__extracted ?? "";
          if (cmd === "tts_list_voice_infos")
            return (window as unknown as { __voices?: unknown[] }).__voices ?? [];
          if (cmd === "isbn_lookup") {
            const hit = (window as unknown as { __isbn?: unknown }).__isbn;
            if (typeof hit === "string") throw hit;
            return hit ?? null;
          }
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
  await expect(page.getByTestId("reader-where")).toHaveText("Kapitel 1Satz 10 von 44");
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
  // Der aktive Timer ist als gewählt erkennbar; Klick daneben schließt.
  await page.getByTestId("reader-sleep").click();
  await expect(
    page.getByRole("menuitemradio", { name: "15 Minuten" }),
  ).toHaveAttribute("aria-checked", "true");
  await page.getByTestId("reader-flow").click({ position: { x: 5, y: 5 } });
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
  await expect(menu.getByRole("group", { name: "Wiedergabegeschwindigkeit" })).toBeVisible();
  await expect(
    menu.getByRole("menuitemradio", { name: "30 Minuten" }),
  ).toHaveAttribute("aria-checked", "true");
  // Fokus liegt auf dem gewählten Tempo; Pfeiltasten wandern.
  await expect(menu.getByRole("menuitemradio", { name: "1,00×" })).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(menu.getByRole("menuitemradio", { name: "1,00×" })).not.toBeFocused();
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);
  await expect(more).toBeFocused();
  // Klick daneben schließt; Verbreitern schließt das verborgene Menü.
  await more.click();
  await page.getByTestId("reader-flow").click({ position: { x: 5, y: 5 } });
  await expect(menu).toHaveCount(0);
  await more.click();
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(menu).toHaveCount(0);
});

const playFromBar = async (page: Page, name: string) => {
  await page.getByTestId("reader-player").getByRole("button", { name, exact: true }).click();
  // Befunde der Skriptprüfung fragen beim Start nach; hier einfach weiter.
  const proceed = page.getByTestId("script-check-proceed");
  if (await proceed.isVisible().catch(() => false)) await proceed.click();
};

test("Weiterlesen: Play setzt an der gemerkten Stelle fort, „Von vorn“ verwirft sie", async ({
  page,
}) => {
  await open(page);
  await page
    .getByTestId("reader-flow")
    .getByText("Noch zwölf Stufen", { exact: false })
    .click();
  await expect.poll(async () => (await calls(page, "tts_speak_text_from")).length).toBe(1);
  await emit(page, "tts-speak-progress", { position: 23, total: 46 });
  await emit(page, "tts-current-sentence", {
    context: "speak",
    index: 23,
    text: "Absatz 20: Die Stunden vergingen zäh, und Mara lauschte auf jedes Geräusch, das vom Meer heraufkam.",
  });
  await expect(page.locator(".reader-cur").first()).toContainText("Absatz 20");

  // Neustart der App: die Seite schlägt die Stelle auf, Play heißt „Weiterlesen“.
  await page.reload();
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await expect(page.getByTestId("reader-where")).toContainText(/Weiter ab Seite \d+/);
  // Der Hinweis wird nicht gekürzt.
  expect(
    await page
      .getByTestId("reader-where")
      .evaluate((el) => el.scrollWidth - el.clientWidth),
  ).toBeLessThanOrEqual(1);
  await expect(page.getByTestId("reader-book")).not.toHaveAttribute("data-page", "0");
  await expect(page.getByTestId("reader-flow").getByText("Absatz 20:", { exact: false })).toBeInViewport();
  await playFromBar(page, "Weiterlesen");
  await expect.poll(async () => (await calls(page, "tts_speak_text_from")).length).toBe(1);
  const [args] = await calls(page, "tts_speak_text_from");
  const text = String(args.text);
  expect(text.slice(Number(args.charOffset)).startsWith("Absatz 20:")).toBe(true);
  expect(await calls(page, "tts_speak_text")).toHaveLength(0);

  // „Von vorn“: die Stelle ist vergessen, Play beginnt am Anfang.
  await page.reload();
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await page.getByTestId("reader-restart").click();
  await expect(page.getByTestId("reader-where")).toHaveText(/^(Kapitel 1)?Seite 1 von/);
  await playFromBar(page, "Vorlesen");
  await expect.poll(async () => (await calls(page, "tts_speak_text")).length).toBe(1);
});

test("Weiterlesen: zu Ende gelesen beginnt Play wieder vorn", async ({ page }) => {
  await open(page);
  await page
    .getByTestId("reader-flow")
    .getByText("Noch zwölf Stufen", { exact: false })
    .click();
  await expect.poll(async () => (await calls(page, "tts_speak_text_from")).length).toBe(1);
  await emit(page, "tts-current-sentence", {
    context: "speak",
    index: 45,
    text: "Das ist der letzte Satz des Buches.",
  });
  await expect(page.locator(".reader-cur")).toHaveCount(1);
  await emit(page, "tts-speak-progress", { position: 46, total: 46 });
  // Erst prüfen, dass die Stelle verworfen ist, dann neu laden.
  await expect
    .poll(() =>
      page.evaluate(() =>
        Object.keys(window.localStorage).filter((k) => k.startsWith("lva.ui.tts.resume.")),
      ),
    )
    .toEqual([]);
  await page.reload();
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await expect(page.getByTestId("reader-flow")).toBeVisible();
  await expect(
    page.getByTestId("reader-player").getByRole("button", { name: "Weiterlesen" }),
  ).toHaveCount(0);
});

test("Lesezeichen: setzen, nach Neustart aufschlagen, ab dort weiterlesen, entfernen", async ({
  page,
}) => {
  await open(page);
  const bookmarksBtn = page.getByTestId("reader-bookmarks");
  await expect(bookmarksBtn).toHaveAttribute("title", "Lesezeichen (0)");
  // Beim Vorlesen gilt die gelesene Stelle.
  await page
    .getByTestId("reader-flow")
    .getByText("Noch zwölf Stufen", { exact: false })
    .click();
  await expect.poll(async () => (await calls(page, "tts_speak_text_from")).length).toBe(1);
  await emit(page, "tts-current-sentence", {
    context: "speak",
    index: 33,
    text: "Absatz 30: Die Stunden vergingen zäh, und Mara lauschte auf jedes Geräusch, das vom Meer heraufkam.",
  });
  await expect(page.locator(".reader-cur").first()).toContainText("Absatz 30");
  await bookmarksBtn.click();
  const menu = page.getByRole("menu", { name: "Lesezeichen" });
  await expect(menu).toContainText("Noch keine Lesezeichen");
  await expect(page.getByTestId("reader-bookmark-add")).toBeFocused();
  await page.getByTestId("reader-bookmark-add").click();
  await expect(bookmarksBtn).toHaveAttribute("title", "Lesezeichen (1)");
  // Dieselbe Stelle gibt es nicht doppelt.
  await bookmarksBtn.click();
  await page.getByTestId("reader-bookmark-add").click();
  await expect(bookmarksBtn).toHaveAttribute("title", "Lesezeichen (1)");

  // Neustart, zurück auf Seite 1, dann das Lesezeichen öffnen.
  await page.reload();
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await page.getByTestId("reader-restart").click();
  await expect(page.getByTestId("reader-book")).toHaveAttribute("data-page", "0");
  await bookmarksBtn.click();
  const item = page.getByTestId("reader-bookmark");
  await expect(item).toHaveText(/^Absatz 30: Die Stunden/);
  await item.click();
  await expect(menu).toHaveCount(0);
  await expect(page.getByTestId("reader-book")).not.toHaveAttribute("data-page", "0");
  await expect(
    page.getByTestId("reader-flow").getByText("Absatz 30:", { exact: false }),
  ).toBeInViewport();
  await playFromBar(page, "Weiterlesen");
  await expect.poll(async () => (await calls(page, "tts_speak_text_from")).length).toBe(1);
  const [args] = await calls(page, "tts_speak_text_from");
  expect(String(args.text).slice(Number(args.charOffset)).startsWith("Absatz 30:")).toBe(true);

  // Entfernen; Klick daneben schließt das Menü.
  await bookmarksBtn.click();
  await page.getByTestId("reader-bookmark-remove").click();
  await expect(menu).toContainText("Noch keine Lesezeichen");
  await page.getByTestId("reader-player").click({ position: { x: 5, y: 5 } });
  await expect(menu).toHaveCount(0);
  await expect(bookmarksBtn).toHaveAttribute("title", "Lesezeichen (0)");
});

test("Lesezeichen ohne Vorlesen: Anfang der sichtbaren Seite", async ({ page }) => {
  await open(page);
  await page.getByTestId("reader-jump").fill("3");
  await page.getByTestId("reader-jump").press("Enter");
  await expect(page.getByTestId("reader-book")).not.toHaveAttribute("data-page", "0");
  await page.getByTestId("reader-bookmarks").click();
  await page.getByTestId("reader-bookmark-add").click();
  await page.getByTestId("reader-bookmarks").click();
  await expect(page.getByTestId("reader-bookmark")).toHaveText(/^Absatz \d+/);
});

test("Weiterlesen schmal: Hinweis und „Von vorn“ im Überlaufmenü", async ({ page }) => {
  await open(page);
  await page
    .getByTestId("reader-flow")
    .getByText("Noch zwölf Stufen", { exact: false })
    .click();
  await expect.poll(async () => (await calls(page, "tts_speak_text_from")).length).toBe(1);
  await emit(page, "tts-current-sentence", {
    context: "speak",
    index: 23,
    text: "Absatz 20: Die Stunden vergingen zäh, und Mara lauschte auf jedes Geräusch, das vom Meer heraufkam.",
  });
  await expect(page.locator(".reader-cur").first()).toContainText("Absatz 20");
  await page.setViewportSize({ width: 768, height: 900 });
  await page.reload();
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await expect(page.getByTestId("reader-where")).toBeHidden();
  await page.getByTestId("reader-more").click();
  await expect(page.getByTestId("reader-more-resume")).toContainText(/Seite \d+/);
  await page.getByTestId("reader-more-restart").click();
  await expect(page.getByTestId("reader-book")).toHaveAttribute("data-page", "0");
  await page.getByTestId("reader-more").click();
  await expect(page.getByTestId("reader-more-resume")).toHaveCount(0);
});

test("Lesezeichen am Anfang: Play beginnt danach vorn", async ({ page }) => {
  const plain = Array.from(
    { length: 40 },
    (_, n) => `Absatz ${n + 1}: Die Stunden vergingen zäh, und Mara lauschte auf jedes Geräusch, das vom Meer heraufkam.`,
  ).join("\n\n");
  await open(page, plain);
  // Lesezeichen auf Seite 1 (Offset 0), dann weiter hinten lesen.
  await page.getByTestId("reader-bookmarks").click();
  await page.getByTestId("reader-bookmark-add").click();
  await page
    .getByTestId("reader-flow")
    .getByText("Absatz 3:", { exact: false })
    .click();
  await expect.poll(async () => (await calls(page, "tts_speak_text_from")).length).toBe(1);
  await emit(page, "tts-current-sentence", {
    context: "speak",
    index: 23,
    text: "Absatz 20: Die Stunden vergingen zäh, und Mara lauschte auf jedes Geräusch, das vom Meer heraufkam.",
  });
  await expect(page.locator(".reader-cur").first()).toContainText("Absatz 20");
  await page.reload();
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await expect(page.getByTestId("reader-where")).toContainText(/Weiter ab Seite/);
  await page.getByTestId("reader-bookmarks").click();
  await page.getByTestId("reader-bookmark").click();
  await expect(page.getByTestId("reader-book")).toHaveAttribute("data-page", "0");
  // Die alte Stelle (Absatz 20) gilt nicht mehr: Play beginnt am Anfang.
  const play = page.getByTestId("reader-play");
  const label = await play.getAttribute("aria-label");
  await playFromBar(page, label ?? "Vorlesen");
  await expect
    .poll(async () => {
      const from = await calls(page, "tts_speak_text_from");
      const whole = await calls(page, "tts_speak_text");
      if (whole.length) return "vorn";
      if (!from.length) return "nichts";
      const a = from[from.length - 1];
      return String(a.text).slice(Number(a.charOffset)).startsWith("Absatz 1:") ? "vorn" : "falsch";
    })
    .toBe("vorn");
});

test("Inhaltsverzeichnis: Kapitel links, aktuelles hervorgehoben, Klick blättert", async ({
  page,
}) => {
  const text = [
    "# Teil Eins",
    "",
    ...Array.from({ length: 12 }, (_, i) => `Absatz ${i + 1}: Die Stunden vergingen zäh, und Mara lauschte auf jedes Geräusch.`),
    "",
    "## Zweites Kapitel",
    "",
    ...Array.from({ length: 12 }, (_, i) => `Zeile ${i + 1}: Der Wind drehte, und das Licht fiel schräg durch das Fenster.`),
    "",
    "## Drittes Kapitel",
    "",
    "Das ist der letzte Satz des Buches.",
  ].join("\n\n");
  await open(page, text);
  const toc = page.getByTestId("reader-contents");
  await expect(toc.getByTestId("reader-contents-entry")).toHaveText([
    /^Teil Eins/,
    /^Zweites Kapitel/,
    /^Drittes Kapitel/,
  ]);
  await expect(toc.getByTestId("reader-contents-entry").nth(0)).toHaveAttribute(
    "aria-current",
    "location",
  );
  await toc.getByRole("button", { name: /Drittes Kapitel/ }).click();
  await expect(page.getByTestId("reader-book")).not.toHaveAttribute("data-page", "0");
  await expect(
    page.getByTestId("reader-flow").getByText("Drittes Kapitel", { exact: true }),
  ).toBeInViewport();
  await expect(toc.getByTestId("reader-contents-entry").nth(2)).toHaveAttribute(
    "aria-current",
    "location",
  );
  // Beim Vorlesen folgt das Kapitel der gelesenen Stelle; die Leiste nennt es.
  await page
    .getByTestId("reader-flow")
    .getByText("Das ist der letzte Satz", { exact: false })
    .click();
  await expect.poll(async () => (await calls(page, "tts_speak_text_from")).length).toBe(1);
  await emit(page, "tts-speak-progress", { position: 14, total: 30 });
  await emit(page, "tts-current-sentence", {
    context: "speak",
    index: 14,
    text: "Zeile 2: Der Wind drehte, und das Licht fiel schräg durch das Fenster.",
  });
  await expect(toc.getByTestId("reader-contents-entry").nth(1)).toHaveAttribute(
    "aria-current",
    "location",
  );
  await expect(page.getByTestId("reader-chapter")).toHaveText("Zweites Kapitel");
  // Schmal: das Kapitel steht im Überlaufmenü.
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByTestId("reader-more").click();
  await expect(page.getByTestId("reader-more-where")).toContainText("Zweites Kapitel");
  await expect(page.getByTestId("reader-more-where")).toContainText("Satz 15 von 30");
  await page.keyboard.press("Escape");
  await page.setViewportSize({ width: 1440, height: 900 });
  // Einklappen zeigt das aktuelle Kapitel im Kopf.
  await page.getByTestId("reader-contents-toggle").click();
  await expect(page.getByTestId("reader-contents-toggle")).toHaveAttribute("aria-expanded", "false");
  await expect(page.getByTestId("reader-contents-toggle")).toContainText("Zweites Kapitel");
  await expect(toc.getByTestId("reader-contents-entry").first()).toBeHidden();
  await page.getByTestId("reader-contents-toggle").click();
  // In der Bearbeiten-Ansicht kein Inhaltsverzeichnis.
  await page.getByTestId("reader-mode-edit").click();
  await expect(toc).toHaveCount(0);
});

test("EPUB auf leerer Seite: Seite heißt wie das Buch, Titel steht im Inhalt", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const w = window as unknown as { __pick: string; __extracted: string };
    w.__pick = "C:/buecher/leuchtturm.epub";
    w.__extracted = [
      "# Die Leuchtturmwärterin",
      "",
      "von Mara Beispiel",
      "",
      "# Erstes Kapitel",
      "",
      "Es war einmal ein Leuchtturm.",
    ].join("\n");
  });
  await open(page, "");
  await page.getByRole("button", { name: /^Hinzufügen/ }).first().click();
  await page.getByRole("menuitem", { name: /Dokument laden.*EPUB/ }).click();
  await expect
    .poll(async () => (await calls(page, "pages_rename")).map((a) => a.title))
    .toEqual(["Die Leuchtturmwärterin"]);
  await expect(page.getByTestId("reader-contents-entry")).toHaveText([
    /^Die Leuchtturmwärterin/,
    /^Erstes Kapitel/,
  ]);
});

test("Buch auf einer Seite mit Text: eigene neue Seite, nichts überschrieben", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const w = window as unknown as { __pick: string; __extracted: string };
    w.__pick = "C:/buecher/x.epub";
    w.__extracted = "# Anderes Buch\n\nText.";
  });
  await open(page);
  await page.getByRole("button", { name: /^Hinzufügen/ }).first().click();
  await page.getByRole("menuitem", { name: /Buch oder Dokument laden/ }).click();
  await expect
    .poll(async () => (await calls(page, "pages_create")).map((a) => a.title))
    .toEqual(["Anderes Buch"]);
  await expect
    .poll(async () =>
      (await calls(page, "page_state_save"))
        .filter((a) => a.id === "p2")
        .map((a) => JSON.parse(String(a.state)).text),
    )
    .toContain("# Anderes Buch\n\nText.");
  // Die bisherige Seite behält Titel und Text.
  expect(await calls(page, "pages_rename")).toHaveLength(0);
  const own = (await calls(page, "page_state_save")).filter((a) => a.id === "p1");
  for (const save of own) expect(JSON.parse(String(save.state)).text).not.toContain("Anderes Buch");
});

test("TXT auf leerer Seite: Seite heißt wie die Datei", async ({ page }) => {
  await page.addInitScript(() => {
    const w = window as unknown as { __pick: string; __extracted: string };
    w.__pick = "C:/buecher/Theodor_Fontane - Effi Briest.txt";
    w.__extracted = "Effi stand am Fenster.";
  });
  await open(page, "");
  await page.getByRole("button", { name: /^Hinzufügen/ }).first().click();
  await page.getByRole("menuitem", { name: /Buch oder Dokument laden/ }).click();
  await expect
    .poll(async () => (await calls(page, "pages_rename")).map((a) => a.title))
    .toEqual(["Effi Briest"]);
  expect(await calls(page, "pages_create")).toHaveLength(0);
});

test("Weiterlesen-Stelle geht dauerhaft in den App-Kern", async ({ page }) => {
  await open(page);
  await page.evaluate(async () => {
    const m = await import("/src/lib/reader/resumePoint.ts");
    m.saveResumePoint("p1:original", "Satz eins. Satz zwei. Satz drei.", 11);
  });
  await expect
    .poll(async () =>
      (await calls(page, "library_progress_put")).flatMap((a) =>
        (a.entries as { key: string }[]).map((e) => e.key),
      ),
    )
    .toEqual(
      expect.arrayContaining(["tts.resume.p1:original", "progress.text:p1:original"]),
    );
});

test("Sprecherfarben und „Stimme zuweisen“ in der Fundleiste", async ({ page }) => {
  await page.addInitScript(() => {
    (window as unknown as { __voices: unknown[] }).__voices = [
      { id: "v-erz", meta: { display_name: "Erzählerin", color: "amber", language: "de" } },
      { id: "v-mara", meta: { display_name: "Mara", color: "teal", language: "de" } },
    ];
  });
  const text = [
    "<Erzählerin> Der Leuchtturm stand auf der Klippe.",
    "",
    "<Mara> Noch zwölf Stufen.",
    "",
    "<Bob> Ich habe keine Stimme.",
    "",
    "<Bob> Noch einmal Bob.",
  ].join("\n");
  await open(page, text);
  const flow = page.getByTestId("reader-flow");
  const mara = flow.locator(".reader-block", { hasText: "Noch zwölf Stufen" });
  const bob = flow.locator(".reader-block", { hasText: "Ich habe keine Stimme" });
  // Bekannte Stimme: ihre Farbe am Rand. Ohne Stimme: trotzdem farbig, aber gestrichelt.
  await expect(mara).toHaveAttribute("style", /--speaker-c: #14b8a6/);
  await expect(mara).not.toHaveClass(/reader-block--novoice/);
  await expect(bob).toHaveClass(/reader-block--novoice/);
  await expect(bob).not.toHaveAttribute("style", /mid-gray/);
  // Die kleinen Namen am Rand erreichen WCAG AA (4,5:1) auf der Buchseite.
  const ratios = await page.evaluate(() => {
    const rgb = (css: string) => {
      const n = css.match(/[\d.]+/g)!.map(Number);
      return css.startsWith("color(") ? n.slice(0, 3) : n.slice(0, 3).map((v) => v / 255);
    };
    const lum = (c: number[]) =>
      c
        .map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4))
        .reduce((sum, v, i) => sum + v * [0.2126, 0.7152, 0.0722][i], 0);
    const bg = lum(rgb(getComputedStyle(document.querySelector(".reader-book")!).backgroundColor));
    return Array.from(document.querySelectorAll(".reader-who")).map((el) => {
      const fg = lum(rgb(getComputedStyle(el).color));
      return (Math.max(fg, bg) + 0.05) / (Math.min(fg, bg) + 0.05);
    });
  });
  expect(ratios.length).toBeGreaterThan(0);
  for (const r of ratios) expect(r).toBeGreaterThanOrEqual(4.5);

  await page.getByTestId("tts-action-menu").click();
  await page.getByTestId("script-check-run").click();
  await expect(page.getByTestId("script-check")).toContainText("Bob");
  const assign = page.getByTestId("script-assign-voice");
  await expect(assign).toBeEnabled();
  await assign.click();
  const menu = page.getByTestId("script-assign-voice-menu");
  await expect(menu.getByRole("menuitem")).toHaveText(["Erzählerin", "Mara"]);
  // Pfeiltasten und Escape bedienen das Menü.
  await expect(menu.getByRole("menuitem").first()).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(menu.getByRole("menuitem", { name: "Mara" })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);
  await expect(assign).toBeFocused();

  await assign.click();
  await menu.getByRole("menuitem", { name: "Mara" }).click();
  // Alle Stellen von Bob sprechen jetzt mit Maras Stimme.
  await expect(flow.locator(".reader-block--novoice")).toHaveCount(0);
  await expect(flow.getByRole("button", { name: /Sprecher Bob/ })).toHaveCount(0);
  await expect(flow.getByRole("button", { name: /Sprecher Mara/ })).toHaveCount(3);
});

test("Seitenmarken aus dem PDF: klein im Text, nicht vorgelesen", async ({ page }) => {
  const text = [
    "{{S. 11}} Der Leuchtturm stand auf der Klippe, und der Wind",
    "",
    "Noch zwölf {{S. 12}} Stufen bis zur Laterne. Dann war sie oben.",
  ].join("\n");
  await open(page, text);
  const flow = page.getByTestId("reader-flow");
  await expect(flow).not.toContainText("{{");
  const mark = flow.getByRole("note", { name: "Seite 12 im Original" });
  await expect(mark).toHaveText("12");
  await expect(flow.getByRole("note")).toHaveCount(2);
  await flow.getByText("Dann war sie oben", { exact: false }).click();
  await expect.poll(async () => (await calls(page, "tts_speak_text_from")).length).toBe(1);
  const [args] = await calls(page, "tts_speak_text_from");
  const spoken = String(args.text);
  expect(spoken).not.toContain("{{");
  expect(spoken).toContain("Noch zwölf Stufen bis zur Laterne.");
  expect(spoken.startsWith("Der Leuchtturm")).toBe(true);
  const offset = Number(args.charOffset);
  // Der Klickpunkt liegt im dritten Absatz des bereinigten Texts.
  expect(offset).toBeGreaterThanOrEqual(spoken.indexOf("Noch zwölf"));
  expect(offset).toBeLessThanOrEqual(spoken.length);
});

test("Buchinfo: Titel und Autor des importierten Buchs über dem Inhalt", async ({ page }) => {
  const text = [
    "# Der Leuchtturm",
    "",
    "von Ada Beispiel, Bo Muster",
    "",
    "## Erstes Kapitel",
    "",
    "Der Wind strich um den Turm.",
  ].join("\n");
  await open(page, text);
  const info = page.getByTestId("reader-bookinfo");
  await expect(info).toContainText("Der Leuchtturm");
  await expect(info).toContainText("Ada Beispiel, Bo Muster");
});

test("ohne Autorzeile keine Buchinfo", async ({ page }) => {
  await open(
    page,
    "# Kapitel 1\n\nvon dort aus sah man das Meer.\n\n## Zwei\n\nNoch ein Satz.",
  );
  await expect(page.getByTestId("reader-contents")).toBeVisible();
  await expect(page.getByTestId("reader-bookinfo")).toHaveCount(0);
});

const ISBN_TEXT = [
  "# Leuchtturm",
  "",
  "von Unbekannt",
  "",
  "Impressum: ISBN 978-3-16-148410-0, Beispielverlag.",
  "",
  "## Erstes Kapitel",
  "",
  "Der Wind strich um den Turm.",
].join("\n");

const ISBN_HIT = {
  isbn: "9783161484100",
  title: "Der Leuchtturm am Ende der Welt",
  subtitle: "Roman",
  authors: ["Ada Beispiel", "Bo Muster"],
  publishers: ["Beispielverlag"],
  publish_date: "2020",
  pages: 312,
  url: "https://openlibrary.org/books/OL1M/Der_Leuchtturm",
};

test("ISBN: Abfrage erst auf Knopfdruck, Übernahme erst nach Bestätigung", async ({
  page,
}) => {
  await page.addInitScript((hit) => {
    (window as unknown as { __isbn: unknown }).__isbn = hit;
  }, ISBN_HIT);
  await open(page, ISBN_TEXT);
  await page.getByTestId("reader-isbn").click();
  // Vorschlag aus dem Impressum, aber noch keine Abfrage.
  await expect(page.getByTestId("isbn-input")).toHaveValue("978-3-16-148410-0");
  expect(await calls(page, "isbn_lookup")).toHaveLength(0);
  await page.getByTestId("isbn-search").click();
  await expect(page.getByTestId("isbn-result")).toContainText(
    "Der Leuchtturm am Ende der Welt",
  );
  await expect(page.getByTestId("isbn-result")).toContainText(
    "von Ada Beispiel, Bo Muster",
  );
  expect(await calls(page, "isbn_lookup")).toEqual([
    { isbn: "978-3-16-148410-0" },
  ]);
  // Treffer angezeigt, aber noch nichts übernommen.
  await expect(page.getByTestId("reader-bookinfo")).toContainText("Unbekannt");
  expect(await calls(page, "pages_rename")).toHaveLength(0);
  await page.getByTestId("isbn-apply").click();
  const info = page.getByTestId("reader-bookinfo");
  await expect(info).toContainText("Der Leuchtturm am Ende der Welt");
  await expect(info).toContainText("Ada Beispiel, Bo Muster");
  await expect(info).not.toContainText("Unbekannt");
  await expect
    .poll(async () => (await calls(page, "pages_rename")).map((a) => a.title))
    .toEqual(["Der Leuchtturm am Ende der Welt"]);
  // Die alte Titelei ist ersetzt, nicht doppelt.
  await expect(page.getByTestId("reader-contents-entry")).toHaveText([
    /^Der Leuchtturm am Ende der Welt/,
    /^Erstes Kapitel/,
  ]);
});

test("ISBN: Abbrechen lässt den Text unverändert", async ({ page }) => {
  await page.addInitScript((hit) => {
    (window as unknown as { __isbn: unknown }).__isbn = hit;
  }, ISBN_HIT);
  await open(page, ISBN_TEXT);
  await page.getByTestId("reader-isbn").click();
  await page.getByTestId("isbn-search").click();
  await expect(page.getByTestId("isbn-apply")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("isbn-input")).toHaveCount(0);
  await expect(page.getByTestId("reader-bookinfo")).toContainText("Unbekannt");
  expect(await calls(page, "pages_rename")).toHaveLength(0);
});

test("ISBN: kein Treffer und Fehler werden gemeldet, nichts übernommen", async ({
  page,
}) => {
  await open(page, ISBN_TEXT);
  await page.getByTestId("reader-isbn").click();
  await page.getByTestId("isbn-search").click();
  await expect(page.getByTestId("isbn-result")).toContainText(
    "Open Library kennt diese ISBN nicht.",
  );
  await expect(page.getByTestId("isbn-apply")).toHaveCount(0);
  await page.evaluate(() => {
    (window as unknown as { __isbn: unknown }).__isbn =
      "Keine gültige ISBN (10 oder 13 Stellen mit passender Prüfziffer).";
  });
  await page.getByTestId("isbn-input").fill("978-3-16-148410-1");
  await page.getByTestId("isbn-search").click();
  await expect(page.getByTestId("isbn-result")).toContainText("Keine gültige ISBN");
  await expect(page.getByTestId("isbn-apply")).toHaveCount(0);
  expect(await calls(page, "pages_rename")).toHaveLength(0);
});

test("ISBN: ohne Titelei wird eine vorangestellt", async ({ page }) => {
  await page.addInitScript((hit) => {
    (window as unknown as { __isbn: unknown }).__isbn = hit;
  }, ISBN_HIT);
  await open(page, "## Erstes Kapitel\n\nISBN 3-16-148410-X\n\nDer Wind strich um den Turm.");
  await expect(page.getByTestId("reader-bookinfo")).toHaveCount(0);
  await page.getByTestId("reader-isbn").click();
  await expect(page.getByTestId("isbn-input")).toHaveValue("3-16-148410-X");
  await page.getByTestId("isbn-search").click();
  await page.getByTestId("isbn-apply").click();
  await expect(page.getByTestId("reader-bookinfo")).toContainText(
    "Der Leuchtturm am Ende der Welt",
  );
  await expect(page.getByTestId("reader-contents-entry")).toHaveText([
    /^Der Leuchtturm am Ende der Welt/,
    /^Erstes Kapitel/,
  ]);
});

test("ISBN: Titelei mit Initialen wird ersetzt, nicht verdoppelt", async ({ page }) => {
  await page.addInitScript((hit) => {
    (window as unknown as { __isbn: unknown }).__isbn = hit;
  }, ISBN_HIT);
  await open(
    page,
    "# Alt\n\nvon A. B. Müller\n\nisbn 978-3-16-148410-1 (Druck), ISBN-13: 978-3-16-148410-0\n\n## Erstes Kapitel\n\nText.",
  );
  await expect(page.getByTestId("reader-bookinfo")).toContainText("A. B. Müller");
  await page.getByTestId("reader-isbn").click();
  // Die erste ISBN hat eine falsche Prüfziffer: vorgeschlagen wird die gültige.
  await expect(page.getByTestId("isbn-input")).toHaveValue("978-3-16-148410-0");
  await page.getByTestId("isbn-search").click();
  await page.getByTestId("isbn-apply").click();
  await expect(page.getByTestId("reader-bookinfo")).toContainText("Ada Beispiel");
  await expect(page.getByTestId("reader-contents-entry")).toHaveText([
    /^Der Leuchtturm am Ende der Welt/,
    /^Erstes Kapitel/,
  ]);
});

// Buchteile (lva#150): am Ende eines Teils im nächsten weiterlesen.
const PARTS = {
  p1: "# Teil eins\n\nDer Wind strich um den Turm. Mara stieg die Stufen hinauf.",
  p2: "# Teil zwei\n\nOben brannte das Licht. Die Nacht war ruhig.",
};

const startPart = async (page: Page) => {
  await page.addInitScript((parts) => {
    (window as unknown as { __parts: Record<string, string> }).__parts = parts;
    window.localStorage.setItem("lva.ui.tts.activePage", "p1");
  }, PARTS);
  await open(page, "");
  await expect(page.getByTestId("reader-flow")).toContainText("Teil eins");
  await playFromBar(page, "Vorlesen");
  await expect.poll(async () => (await calls(page, "tts_speak_text")).length).toBe(1);
};

test("Buchteile: am Ende liest das Vorlesen im nächsten Teil weiter, vorberechnet", async ({
  page,
}) => {
  await startPart(page);
  // Der letzte Satz läuft: der Anfang von Teil zwei wird still vorberechnet.
  await emit(page, "tts-speak-progress", { position: 1, total: 3 });
  await expect
    .poll(async () => (await calls(page, "tts_prefetch_head")).length)
    .toBe(1);
  const [prefetch] = await calls(page, "tts_prefetch_head");
  expect(String(prefetch.text)).toContain("Oben brannte das Licht.");
  expect(String(prefetch.text)).not.toContain("Der Wind");

  await emit(page, "tts-speak-progress", { position: 3, total: 3 });
  await page.evaluate(() =>
    (window as unknown as { __endSpeak: () => void }).__endSpeak(),
  );
  await expect(page.getByTestId("reader-flow")).toContainText("Teil zwei");
  await expect.poll(async () => (await calls(page, "tts_speak_text")).length).toBe(2);
  const [, second] = await calls(page, "tts_speak_text");
  expect(String(second.text)).toContain("Oben brannte das Licht.");
  await expect(page.getByText("Weiter mit „Teil 2“")).toBeVisible();
});

test("Buchteile: nach einem Stopp bleibt das Vorlesen im Teil", async ({ page }) => {
  await startPart(page);
  // Abgebrochen nach dem ersten von drei Sätzen.
  await emit(page, "tts-speak-progress", { position: 1, total: 3 });
  await page.evaluate(() =>
    (window as unknown as { __endSpeak: () => void }).__endSpeak(),
  );
  await page.waitForTimeout(400);
  expect(await calls(page, "tts_speak_text")).toHaveLength(1);
  await expect(page.getByTestId("reader-flow")).toContainText("Teil eins");
});

test("Buchteile: Pause und Play lesen trotzdem im nächsten Teil weiter", async ({ page }) => {
  await startPart(page);
  await emit(page, "tts-speak-progress", { position: 1, total: 3 });
  await page.evaluate(() =>
    (window as unknown as { __endSpeak: () => void }).__endSpeak(),
  );
  await playFromBar(page, "Vorlesen");
  await expect.poll(async () => (await calls(page, "tts_speak_resume")).length).toBe(1);
  await emit(page, "tts-speak-progress", { position: 3, total: 3 });
  await page.evaluate(() =>
    (window as unknown as { __endSpeak: () => void }).__endSpeak(),
  );
  await expect(page.getByTestId("reader-flow")).toContainText("Teil zwei");
  await expect.poll(async () => (await calls(page, "tts_speak_text")).length).toBe(2);
});

test("Buchteile: nach einem Reiterwechsel bleibt das Vorlesen im Teil", async ({ page }) => {
  await startPart(page);
  await emit(page, "tts-speak-progress", { position: 2, total: 3 });
  await page.getByRole("tab", { name: "Übersetzung" }).click();
  await emit(page, "tts-speak-progress", { position: 3, total: 3 });
  await page.evaluate(() =>
    (window as unknown as { __endSpeak: () => void }).__endSpeak(),
  );
  await page.waitForTimeout(400);
  expect(await calls(page, "tts_speak_text")).toHaveLength(1);
});

// Seitenumbruch (Patrick 01.11.): der Sprechername steht nie allein unten.
const SPEAKER_BOOK = Array.from({ length: 40 }, (_, i) => {
  const who = i % 3 === 0 ? "Patrick" : i % 3 === 1 ? "Annika" : "Erzählerin";
  const words = "Das ist ein Satz mit einigen Wörtern, die den Block füllen.";
  return `<${who}> ${Array.from({ length: 1 + ((i * 7) % 6) }, () => words).join(" ")}`;
}).join("\n\n");

const lonelyNames = (page: Page) =>
  page.evaluate(() => {
    const flow = document.querySelector<HTMLElement>("[data-testid=reader-flow]")!;
    const step = parseFloat(flow.style.columnWidth) + parseFloat(flow.style.columnGap);
    const left = flow.getBoundingClientRect().left;
    const pageOf = (r: DOMRect) => Math.floor((r.left - left + 2) / step);
    const bad: string[] = [];
    for (const block of flow.querySelectorAll<HTMLElement>(".reader-block")) {
      const who = block.querySelector<HTMLElement>(".reader-who");
      const first = block.querySelector<HTMLElement>("[data-start]");
      if (!who || !first || block.classList.contains("reader-cont")) continue;
      const range = document.createRange();
      range.setStart(first.firstChild!, 0);
      range.setEnd(first.firstChild!, 1);
      const textPage = pageOf(range.getClientRects()[0]);
      if (pageOf(who.getBoundingClientRect()) !== textPage)
        bad.push(`${who.textContent}@${pageOf(who.getBoundingClientRect())}/${textPage}`);
      const bar = block.querySelector<HTMLElement>(".reader-bar");
      void bar;
    }
    return bad;
  });

for (const width of [1440, 1100, 800])
  test(`Seitenumbruch: kein Sprechername allein am Seitenende (${width}px)`, async ({ page }) => {
    await page.addInitScript(() => window.localStorage.setItem("lva.ui.tts.view", "read"));
    await open(page, SPEAKER_BOOK, width);
    await expect(page.getByTestId("reader-flow")).toContainText("Annika");
    expect(await lonelyNames(page)).toEqual([]);
  });

// Keine Sprecherstummel (Patrick 01.11. abends): eine reine Pause zeigt weder
// Strich noch Namen, und jedes Strichstück gehört zu sichtbarem Text.
const PAUSE_BOOK = Array.from({ length: 30 }, (_, i) => {
  const who = i % 2 === 0 ? "Patrick" : "Annika";
  const words = "Das ist ein Satz mit einigen Wörtern, die den Block füllen.";
  const text = Array.from({ length: 1 + ((i * 5) % 7) }, () => words).join(" ");
  return i % 4 === 1 ? `<${who}> ${text}\n\n[long pause]` : `<${who}> ${text}`;
}).join("\n\n");

for (const width of [1100, 1440])
  test(`Pausen ohne Sprecherstrich, keine Stummel (${width}px)`, async ({ page }) => {
    await page.addInitScript(() => window.localStorage.setItem("lva.ui.tts.view", "read"));
    await open(page, PAUSE_BOOK, width);
    const flow = page.getByTestId("reader-flow");
    await expect(flow).toContainText("Annika");
    await expect(flow.locator(".reader-pause")).toHaveCount(8);
    await expect(flow.locator(".reader-pause .reader-who")).toHaveCount(0);
    const empty = await page.evaluate(() =>
      Array.from(document.querySelectorAll<HTMLElement>(".reader-block--bar"))
        .filter((b) => !b.querySelector("[data-start]"))
        .map((b) => b.textContent),
    );
    expect(empty).toEqual([]);
  });

// Sekundengenau und wortgenau (Patrick 01.11.): der Kern meldet die Dauer
// des Satzes, die Ansicht läuft innerhalb des Satzes mit.
const LONG_SENTENCE =
  "# Ein langer Satz\n\n" +
  Array.from({ length: 140 }, (_, i) => `Wort${i}`).join(" ") +
  " Ende.";

test("Wort und Zeit laufen innerhalb des Satzes mit", async ({ page }) => {
  await page.clock.install();
  await page.addInitScript(() => window.localStorage.setItem("lva.ui.tts.view", "read"));
  await open(page, LONG_SENTENCE, 800);
  await emit(page, "tts-state-changed", { phase: "speaking", message: null });
  await emit(page, "tts-speak-progress", { position: 1, total: 2 });
  await emit(page, "tts-current-sentence", {
    context: "speak",
    index: 1,
    text: LONG_SENTENCE.split("\n\n")[1],
    duration_ms: 60_000,
  });
  await expect(page.locator(".reader-word")).toHaveText("Wort0");
  const elapsed0 = await page.getByTestId("reader-elapsed").textContent();
  await page.clock.runFor(30_000);
  // Halb durch: ein Wort aus der Mitte, und die Zeit ist weitergelaufen.
  const mid = Number((await page.locator(".reader-word").textContent())?.slice(4));
  expect(mid).toBeGreaterThan(55);
  expect(mid).toBeLessThan(85);
  await expect(page.getByTestId("reader-elapsed")).not.toHaveText(elapsed0 ?? "");
});

test("Umblättern wortgenau: mitten im Satz, sobald das gehörte Wort auf der nächsten Seite steht", async ({
  page,
}) => {
  await page.clock.install();
  await page.addInitScript(() => window.localStorage.setItem("lva.ui.tts.view", "read"));
  await open(page, LONG_SENTENCE, 800);
  const book = page.getByTestId("reader-book");
  expect(Number(await book.getAttribute("data-pages"))).toBeGreaterThan(1);
  await emit(page, "tts-state-changed", { phase: "speaking", message: null });
  await emit(page, "tts-speak-progress", { position: 1, total: 2 });
  await emit(page, "tts-current-sentence", {
    context: "speak",
    index: 1,
    text: LONG_SENTENCE.split("\n\n")[1],
    duration_ms: 120_000,
  });
  await expect(book).toHaveAttribute("data-page", "0");
  // Bis das Wort die Seite verlässt, bleibt die Seite stehen …
  let turned = -1;
  for (let s = 0; s < 120 && turned < 0; s += 2) {
    await page.clock.runFor(2_000);
    if ((await book.getAttribute("data-page")) !== "0") turned = s;
  }
  expect(turned).toBeGreaterThan(0);
  // … und das gehörte Wort steht dann auf der sichtbaren Seite.
  const visible = await page.evaluate(() => {
    const word = document.querySelector(".reader-word")!.getBoundingClientRect();
    const b = document.querySelector("[data-testid=reader-book]")!.getBoundingClientRect();
    return word.left >= b.left && word.right <= b.right;
  });
  expect(visible).toBe(true);
});

test("Blättern animiert nur beim Umblättern, nicht beim Öffnen", async ({ page }) => {
  await page.addInitScript(() => window.localStorage.setItem("lva.ui.tts.view", "read"));
  await open(page, LONG_SENTENCE, 800);
  const flow = page.getByTestId("reader-flow");
  await expect(flow).not.toHaveClass(/reader-flow--turning/);
  await flow.focus();
  await page.keyboard.press("ArrowRight");
  await expect(page.getByTestId("reader-book")).toHaveAttribute("data-page", "1");
  await expect(flow).toHaveClass(/reader-flow--turning/);
  await expect(flow).not.toHaveClass(/reader-flow--turning/);
});

test("nach dem Wechsel in einen anderen App-Bereich steht dieselbe Seite, ohne Umblättern", async ({
  page,
}) => {
  await page.addInitScript(() => window.localStorage.setItem("lva.ui.tts.view", "read"));
  await open(page, LONG_SENTENCE, 800);
  const flow = page.getByTestId("reader-flow");
  await flow.focus();
  await page.keyboard.press("ArrowRight");
  await expect(page.getByTestId("reader-book")).toHaveAttribute("data-page", "1");
  await page.getByRole("navigation").getByRole("button", { name: "Aufnahmen", exact: true }).click();
  await page.getByRole("navigation").getByRole("button", { name: "Vorlesen", exact: true }).click();
  await expect(page.getByTestId("reader-book")).toHaveAttribute("data-page", "1");
  await expect(page.getByTestId("reader-flow")).not.toHaveClass(/reader-flow--turning/);
});

const contrast = (fg: string, bg: string) => {
  const lum = (c: string) => {
    const [r, g, b] = (c.match(/\d+(\.\d+)?/g) ?? []).slice(0, 3).map((v) => {
      const x = Number(v) / 255;
      return x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  };
  const [a, b] = [lum(fg), lum(bg)].sort((x, y) => y - x);
  return (a + 0.05) / (b + 0.05);
};

test("Farbthemen wie beim Kindle: Kontrast mindestens 7:1, gemerkt", async ({ page }) => {
  await page.addInitScript(() => window.localStorage.setItem("lva.ui.tts.view", "read"));
  await open(page);
  for (const theme of ["white", "sepia", "green", "black"]) {
    await page.getByTestId("reader-display").click();
    await page.getByTestId(`reader-theme-${theme}`).click();
    await page.keyboard.press("Escape");
    const colors = await page.evaluate(() => {
      const book = document.querySelector("[data-testid=reader-book]")!;
      const run = document.querySelector(".reader-run");
      const s = getComputedStyle(book);
      return {
        fg: s.color,
        bg: s.backgroundColor,
        muted: run ? getComputedStyle(run).color : s.color,
      };
    });
    expect(contrast(colors.fg, colors.bg), theme).toBeGreaterThanOrEqual(7);
    expect(contrast(colors.muted, colors.bg), theme).toBeGreaterThanOrEqual(7);
  }
  await page.reload();
  await page.getByRole("navigation").getByRole("button", { name: "Vorlesen", exact: true }).click();
  await expect(page.locator(".reader-frame")).toHaveAttribute("data-reader-theme", "black");
});

test("Schriftart im T-Menü wechselt die Buchschrift", async ({ page }) => {
  await page.addInitScript(() => window.localStorage.setItem("lva.ui.tts.view", "read"));
  await open(page);
  const fontOf = () =>
    page.evaluate(
      () => getComputedStyle(document.querySelector("[data-testid=reader-flow]")!).fontFamily,
    );
  expect(await fontOf()).toContain("Georgia");
  await page.getByTestId("reader-display").click();
  await page.getByTestId("reader-font-wide").click();
  await expect(page.getByTestId("reader-book")).toHaveAttribute("data-font", "wide");
  expect(await fontOf()).toContain("Verdana");
  await expect(page.getByTestId("reader-font-wide")).toHaveAttribute("aria-checked", "true");
});

test("Sonnen-Knopf hält den Bildschirm wach, gibt ihn frei und verschiebt gegen OLED-Dimmen", async ({
  page,
}) => {
  await page.clock.install();
  await page.addInitScript(() => window.localStorage.setItem("lva.ui.tts.view", "read"));
  await open(page);
  const sun = page.getByTestId("reader-keep-awake");
  await sun.click();
  await expect(sun).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByText(/ASBL/)).toBeVisible();
  await expect.poll(() => calls(page, "screen_keep_awake")).toEqual([{ on: true }]);
  const frame = page.locator(".reader-frame");
  await page.clock.runFor(46_000);
  await expect(frame).toHaveAttribute("style", /left: 1px/);
  await page.clock.runFor(45_000);
  await expect(frame).not.toHaveAttribute("style", /left: 1px/);
  await sun.click();
  await expect(sun).toHaveAttribute("aria-pressed", "false");
  await expect
    .poll(() => calls(page, "screen_keep_awake"))
    .toEqual([{ on: true }, { on: false }]);
  // Auch im Bearbeiten erreichbar.
  await page.getByTestId("reader-mode-edit").click();
  await expect(page.getByTestId("reader-keep-awake")).toBeVisible();
});

test("Schreibregeln stehen hinter dem ⓘ-Knopf, nicht mehr rechts", async ({ page }) => {
  await open(page);
  await expect(page.locator("summary", { hasText: "Schreibregeln" })).toHaveCount(0);
  await page.getByTestId("reader-info").click();
  const panel = page.getByTestId("reader-info-panel");
  await expect(panel).toContainText("Schreibregeln: Sprecher & Tags");
  await page.keyboard.press("Escape");
  await expect(panel).toHaveCount(0);
});

test("Vollbild-Knopf schaltet die Lesespalte in den Vollbildmodus", async ({ page }) => {
  await page.addInitScript(() => window.localStorage.setItem("lva.ui.tts.view", "read"));
  await open(page);
  const btn = page.getByTestId("reader-fullscreen");
  await expect(btn).toHaveAttribute("aria-label", "Vollbild");
  await btn.click();
  await expect
    .poll(() => page.evaluate(() => document.fullscreenElement?.className ?? ""))
    .toContain("reader-frame");
  await expect(btn).toHaveAttribute("aria-label", "Vollbild beenden");
  await btn.click();
  await expect
    .poll(() => page.evaluate(() => document.fullscreenElement === null))
    .toBe(true);
});

test("T-Menü bei 390 px: ganz im Fenster, der Rest scrollt im Menü", async ({ page }) => {
  await page.addInitScript(() => window.localStorage.setItem("lva.ui.tts.view", "read"));
  await open(page, BOOK, 1440);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByTestId("reader-display").click();
  const menu = page.getByTestId("reader-display-menu");
  const box = (await menu.boundingBox())!;
  const frame = (await page.locator(".reader-frame").boundingBox())!;
  expect(box.y + box.height).toBeLessThanOrEqual(Math.min(844, frame.y + frame.height));
  // Der letzte Eintrag ist per Scrollen erreichbar und klickbar.
  const last = menu.getByRole("menuitemcheckbox", { name: "Automatisch mitblättern" });
  await last.scrollIntoViewIfNeeded();
  await expect(last).toBeInViewport();
});

test("Variante A: rechte Spalte ausblenden, Stimme und Werkzeuge in der Leiste, Leiste nicht höher", async ({
  page,
}) => {
  await page.addInitScript(() => window.localStorage.setItem("lva.ui.tts.view", "read"));
  await open(page);
  const bar = page.getByTestId("reader-player");
  const before = (await bar.boundingBox())!.height;
  await page.getByTestId("right-column-hide").click();
  await expect(page.getByTestId("tts-right-column")).toHaveCount(0);
  const dock = page.getByTestId("reader-dock");
  await expect(dock.getByTestId("voice-select")).toBeVisible();
  await expect(dock.getByTestId("tts-action-menu")).toBeVisible();
  await expect(dock.getByTestId("tts-action-add")).toBeVisible();
  // Ohne eingerichtete Stimme: Warnknopf statt verborgenem Hinweistext.
  await expect(dock.getByTestId("reader-dock-voice-setup")).toHaveAttribute(
    "aria-label",
    "Sprachausgabe einrichten",
  );
  // Die Leiste bleibt so hoch wie vorher (zwei Zeilen, keine dritte).
  expect((await bar.boundingBox())!.height).toBeLessThanOrEqual(before + 1);
  await expect(page.getByTestId("reader-where")).toBeVisible();
  // Gemerkt; der Streifen rechts (nur Symbol) blendet wieder ein.
  await page.reload();
  await page.getByRole("navigation").getByRole("button", { name: "Vorlesen", exact: true }).click();
  const show = page.getByTestId("right-column-show");
  await expect(show).toHaveText("");
  await expect(show).toHaveAttribute("aria-label", "Rechte Spalte einblenden");
  await show.click();
  await expect(page.getByTestId("tts-right-column")).toBeVisible();
  await expect(page.getByTestId("reader-dock")).toHaveCount(0);
});

test("Variante A schmal: Werkzeuge hinter einem Knopf, Leiste einzeilig in der Transportzeile", async ({
  page,
}) => {
  await page.addInitScript(() => {
    window.localStorage.setItem("lva.ui.tts.view", "read");
    window.localStorage.setItem("lva.ui.tts.rightHidden", "1");
  });
  await open(page, BOOK, 1000);
  const bar = page.getByTestId("reader-player");
  await expect(page.getByTestId("reader-dock")).toHaveCount(0);
  const btn = page.getByTestId("reader-dock-open");
  await expect(btn).toHaveAttribute("aria-label", "Stimme und Werkzeuge");
  // Transportzeile: Werkzeugknopf, Play und Tempo auf einer Höhe.
  const y = async (id: string) => {
    const b = (await page.getByTestId(id).boundingBox())!;
    return b.y + b.height / 2;
  };
  expect(Math.abs((await y("reader-dock-open")) - (await y("reader-play")))).toBeLessThan(4);
  const height = (await bar.boundingBox())!.height;
  await btn.click();
  const panel = page.getByTestId("reader-dock-panel");
  await expect(panel.getByTestId("voice-select")).toBeVisible();
  await expect(panel.getByTestId("tts-action-menu")).toBeVisible();
  expect((await bar.boundingBox())!.height).toBeLessThanOrEqual(height + 1);
  // Fokus im Fenster; Escape schließt und gibt ihn an den Knopf zurück.
  await expect(panel.locator(":focus")).toHaveCount(1);
  await page.keyboard.press("Escape");
  await expect(panel).toBeHidden();
  await expect(btn).toBeFocused();
  // Bei 390 px bleibt der Knopf neben Play, Kapitel/Satz im Überlaufmenü.
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(page.getByTestId("reader-dock-open")).toBeVisible();
  expect(Math.abs((await y("reader-dock-open")) - (await y("reader-play")))).toBeLessThan(4);
});

test("Variante A: Kapitel und Satz in der Leiste, bei 390 px im Überlaufmenü", async ({ page }) => {
  await page.addInitScript(() => {
    window.localStorage.setItem("lva.ui.tts.view", "read");
    window.localStorage.setItem("lva.ui.tts.rightHidden", "1");
  });
  await open(page);
  const where = page.getByTestId("reader-where");
  // Ohne Sitzung: Kapitel über der Seitenangabe.
  await expect(where.getByTestId("reader-chapter")).toHaveText("Kapitel 1");
  await expect(where).toContainText("Seite 1 von");
  // Mit Sitzung: Kapitel und Satz.
  await emit(page, "tts-state-changed", { phase: "speaking", message: null });
  await emit(page, "tts-speak-progress", { position: 1, total: 6 });
  await expect(where).toContainText("Kapitel 1");
  await expect(where).toContainText("Satz 2 von 6");
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(where).toBeHidden();
  await page.getByRole("button", { name: "Weitere Optionen" }).click();
  const head = page.getByTestId("reader-more-where");
  await expect(head).toContainText("Kapitel 1");
  await expect(head).toContainText("Satz 2 von 6");
});

test("Variante A: alle Symbolknöpfe haben Namen und Tooltip", async ({ page }) => {
  await page.addInitScript(() => window.localStorage.setItem("lva.ui.tts.view", "read"));
  await open(page);
  const hide = page.getByRole("button", { name: "Rechte Spalte ausblenden" });
  await expect(hide).toHaveAttribute("title", "Rechte Spalte ausblenden");
  await hide.click();
  const show = page.getByRole("button", { name: "Rechte Spalte einblenden" });
  await expect(show).toHaveAttribute("title", "Rechte Spalte einblenden");
  const bar = page.getByRole("region", { name: "Wiedergabe" });
  await expect(
    bar.getByRole("button", { name: "Sprachausgabe einrichten" }),
  ).toHaveAttribute("title", /.+/);
  for (const name of ["Hinzufügen", "Diktieren", "Als Audio speichern…", "Änderungen vorab erzeugen", "Weitere Aktionen"]) {
    const btn = bar.getByRole("button", { name, exact: true });
    await expect(btn, name).toBeVisible();
    // Tooltip beim Fokus (IconAction / ActionMenu).
    await btn.focus();
    await expect(page.getByRole("tooltip").filter({ hasText: name }).first(), name).toBeVisible();
    await btn.blur();
  }
  // Schmal: der Werkzeugknopf ist benannt und hat einen Tooltip.
  await page.setViewportSize({ width: 1000, height: 900 });
  const tools = bar.getByRole("button", { name: "Stimme und Werkzeuge" });
  await expect(tools).toHaveAttribute("title", "Stimme und Werkzeuge");
  await tools.click();
  await expect(page.getByRole("dialog", { name: "Stimme und Werkzeuge" })).toBeVisible();
});

// Rückmeldung Patrick 01.11. abends: Abspielen vorn/mittig, Werkzeuge rechts,
// Stimme nur als Dropdown-Knopf, Spaltenstreifen links und rechts gleich,
// kein Untertitel.
test("Leiste: Abspielen vor den Werkzeugen, Stimme als kompakter Knopf", async ({ page }) => {
  await page.addInitScript(() => {
    window.localStorage.setItem("lva.ui.tts.view", "read");
    window.localStorage.setItem("lva.ui.tts.rightHidden", "1");
  });
  await open(page);
  const play = (await page.getByTestId("reader-play").boundingBox())!;
  const dock = page.getByTestId("reader-dock");
  const voice = (await dock.getByTestId("voice-select").boundingBox())!;
  const tools = (await dock.getByTestId("tts-action-menu").boundingBox())!;
  expect(play.x).toBeLessThan(voice.x);
  expect(play.x).toBeLessThan(tools.x);
  // Nur Symbol + Pfeil, der Name im Tooltip und als zugänglicher Name.
  expect(voice.width).toBeLessThan(90);
  await expect(dock.getByTestId("voice-compact")).toHaveAttribute("title", /^Stimme/);
  // Die Liste öffnet breit und bleibt im Fenster.
  await dock.locator(".app-select__control").click();
  const menu = (await page.locator(".app-select__menu").boundingBox())!;
  expect(menu.width).toBeGreaterThanOrEqual(290);
  expect(menu.x + menu.width).toBeLessThanOrEqual(1440);
});

test("Spaltenstreifen links und rechts gleich: nur Symbol, gleich breit, oben", async ({ page }) => {
  await page.addInitScript(() => {
    window.localStorage.setItem("lva.ui.tts.view", "read");
    window.localStorage.setItem("lva.ui.tts.rightHidden", "1");
    window.localStorage.setItem("lva.ui.tts.pagesCollapsed.v3", "1");
  });
  await open(page);
  await expect(page.getByText("Texte und Dokumente anhören und die Ausgabe speichern.")).toHaveCount(0);
  const left = page.getByTestId("pages-expand");
  const right = page.getByTestId("right-column-show");
  await expect(left).toHaveText("");
  await expect(right).toHaveText("");
  await expect(left).toHaveAttribute("title", "Seitenliste ausklappen");
  await expect(right).toHaveAttribute("title", "Rechte Spalte einblenden");
  const l = (await left.boundingBox())!;
  const r = (await right.boundingBox())!;
  expect(l.width).toBe(r.width);
  expect(Math.abs(l.y - r.y)).toBeLessThan(2);
  expect((await page.getByTestId("pages-expand-strip").boundingBox())!.width).toBeLessThanOrEqual(24);
});

test("Wechsel ein-/zweiseitig blättert nicht animiert", async ({ page }) => {
  await page.addInitScript(() => {
    window.localStorage.setItem("lva.ui.tts.view", "read");
    window.localStorage.setItem("lva.ui.tts.rightHidden", "1");
  });
  await open(page, BOOK, 1920);
  const book = page.getByTestId("reader-book");
  await emit(page, "tts-speak-progress", { position: 43, total: 44 });
  await emit(page, "tts-current-sentence", {
    context: "speak",
    index: 43,
    text: "Das ist der letzte Satz des Buches.",
  });
  await expect(book).not.toHaveAttribute("data-page", "0");
  await page.waitForTimeout(700);
  await page.getByTestId("reader-flow").evaluate((el) => {
    const w = window as unknown as { __turned: boolean };
    w.__turned = false;
    new MutationObserver(() => {
      if (el.classList.contains("reader-flow--turning")) w.__turned = true;
    }).observe(el, { attributes: true, attributeFilter: ["class"] });
  });
  for (const id of ["reader-one-page", "reader-two-pages", "reader-one-page"]) {
    await page.getByTestId(id).click();
    await page.waitForTimeout(300);
    await expect(page.locator(".reader-cur").first()).toBeInViewport();
  }
  expect(await page.evaluate(() => (window as unknown as { __turned: boolean }).__turned)).toBe(false);
});

for (const hidden of ["0", "1"])
  test(`Wiedergabe rechtsbündig, Spalte ${hidden === "1" ? "aus" : "ein"}geblendet`, async ({ page }) => {
    await page.addInitScript((h) => {
      window.localStorage.setItem("lva.ui.tts.view", "read");
      window.localStorage.setItem("lva.ui.tts.rightHidden", h);
    }, hidden);
    await open(page);
    const box = (s: string) => page.locator(s).first().boundingBox().then((b) => b!);
    const ctrls = await box(".reader-player__ctrls");
    const media = await box(".reader-player__ctrls > .mediabar");
    const tools = await box(".reader-player__tools");
    expect(Math.abs(tools.x + tools.width - (ctrls.x + ctrls.width))).toBeLessThan(2);
    expect(tools.x - (media.x + media.width)).toBeLessThan(12);
  });

test("Sprecher allein auf der Zeile: Name steht am folgenden Text", async ({ page }) => {
  await page.addInitScript(() => window.localStorage.setItem("lva.ui.tts.view", "read"));
  await open(page, "<Anna>\nErste Zeile von Anna.\n\n<Ben> [long pause]\nBen spricht weiter.", 1440);
  const flow = page.getByTestId("reader-flow");
  await expect(flow.locator(".reader-pause")).toHaveCount(2);
  for (const name of ["Anna", "Ben"]) {
    const who = flow.locator(`.reader-who[data-speaker="${name}"]`);
    await expect(who).toHaveCount(1);
    await expect(who).toBeVisible();
    await expect(who).toHaveCSS("opacity", "1");
  }
});

for (const width of [900, 1000, 1150, 1300])
  test(`Leiste ohne Spalte: kein Überlauf (${width}px)`, async ({ page }) => {
    await page.addInitScript(() => {
      window.localStorage.setItem("lva.ui.tts.view", "read");
      window.localStorage.setItem("lva.ui.tts.rightHidden", "1");
    });
    await open(page, BOOK, width);
    const ctrls = page.locator(".reader-player__ctrls");
    await page.waitForTimeout(300);
    expect(await ctrls.evaluate((el) => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(1);
    const play = (await page.getByTestId("reader-play").boundingBox())!;
    const box = (await ctrls.boundingBox())!;
    expect(play.y + play.height).toBeLessThanOrEqual(box.y + box.height + 1);
    expect(box.height).toBeLessThan(70);
  });
