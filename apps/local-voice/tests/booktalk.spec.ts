import { test, expect, type Page } from "@playwright/test";

// Gespräch mit dem Buch (lva#309): Spalte im Leser, Bereich mit Bestätigung,
// Belege mit Karte, Fundstellen-Modus. Die Befehle book_talk_* sind gemockt;
// Antworten kommen als Ereignis "book-talk-event".
const TEXT = [
  "# Kapitel 1",
  "",
  "<Erzählerin> Der Leuchtturm stand seit über hundert Jahren auf der Klippe.",
  "",
  ...Array.from(
    { length: 160 },
    (_, i) => `<Erzählerin> Absatz ${i + 1}: Mara lauschte auf jedes Geräusch vom Meer.`,
  ),
].join("\n");

const QUOTE = "Der Leuchtturm stand seit über hundert Jahren auf der Klippe.";
const CITATION = {
  q: 1,
  page: 1,
  chapter_index: 0,
  chapter_title: "Kapitel 1",
  start: 12,
  end: 12 + QUOTE.length,
  quote: QUOTE,
};
// Ein Beleg weit hinten im Buch: nach „Hinspringen“ muss genau diese Seite zu sehen sein.
const LATE = "Absatz 150: Mara lauschte auf jedes Geräusch vom Meer.";
const LATE_START = TEXT.indexOf(LATE);
const LATE_CITATION = {
  ...CITATION,
  page: 9,
  chapter_title: "Kapitel 1",
  start: LATE_START,
  end: LATE_START + LATE.length,
  quote: LATE,
};
const ANSWER = (over: Record<string, unknown> = {}) => ({
  intent: "locate",
  sentences: [
    { index: 0, text: "Der Leuchtturm steht auf der Klippe.", speech: "Der Leuchtturm steht auf der Klippe.", citations: [CITATION], general: false },
  ],
  text: "Der Leuchtturm steht auf der Klippe.",
  speech: "Der Leuchtturm steht auf der Klippe.",
  citations: [CITATION],
  found_only: false,
  reason: null,
  dropped: 0,
  scope: "until_position",
  frontier: 400,
  ...over,
});

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => {
    throw error;
  });
});

interface OpenOptions {
  enabled?: boolean;
  answer?: unknown;
  width?: number;
  height?: number;
  theme?: string;
  /** Aktives Modell ist ein Cloud-Modell (ohne Freigabe). */
  cloud?: boolean;
  /** Die erste Anfrage hängt, bis sie abgebrochen wird (der Kern gibt den Wächter erst danach frei). */
  holdFirst?: boolean;
  /** Fortschritt des Buchs beim Öffnen. */
  frontier?: number;
}

const open = async (page: Page, opts: OpenOptions = {}) => {
  await page.setViewportSize({ width: opts.width ?? 1440, height: opts.height ?? 900 });
  await page.addInitScript(
    ([text, enabled, answer, theme, cloud, holdFirst, frontier]) => {
      const callbacks = new Map<number, unknown>();
      let callback = 0;
      const w = window as unknown as Record<string, unknown>;
      const settings = {
        onboarding_completed: true,
        app_language: "de",
        theme,
        show_whats_new_on_update: false,
        debug_mode: false,
        selected_model: "",
        bindings: {},
        post_process_providers: [],
        post_process_prompts: [],
        custom_words: [],
        post_process_models: {},
        post_process_api_keys: {},
        push_to_talk: true,
        tts_engine: "piper",
        book_talk_enabled: enabled,
        book_talk_cloud_consent: [] as string[],
        llm_connections: cloud
          ? [{ id: "c1", kind: "openai", label: "OpenAI", base_url: "https://api.example.invalid/v1", enabled: true }]
          : [],
        llm_models: cloud
          ? [{ id: "m1", connection_id: "c1", remote_id: "gpt-a", label: "GPT A", enabled: true, price_input_per_mtok: 1, price_output_per_mtok: 2 }]
          : [],
        llm_active_model_id: cloud ? "m1" : null,
        function_models: {},
      };
      (w as Record<string, unknown>).__lvaReaderSpans = true;
      let inflight = false;
      let holdLeft = holdFirst ? 1 : 0;
      let releaseHeld: (() => void) | null = null;
      const listeners: Record<string, number[]> = {};
      w.__calls = [] as unknown[];
      w.__emit = (name: string, payload: unknown) =>
        (listeners[name] ?? []).forEach((id) =>
          (callbacks.get(id) as (e: unknown) => void)?.({ event: name, id: 0, payload }),
        );
      window.localStorage.removeItem("lva.ui.tts.view");
      window.localStorage.setItem(
        "lva.ui.progress.text:p1:original",
        JSON.stringify({ offset: 0, length: 12000, heard: [], read: [], updated: 1, frontier }),
      );
      Object.assign(window, {
        __TAURI_OS_PLUGIN_INTERNALS__: { platform: "windows", os_type: "windows", family: "windows", arch: "x86_64", version: "10.0.26200", eol: "\r\n" },
        __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
        __TAURI_INTERNALS__: {
          metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
          transformCallback: (fn: unknown) => {
            callbacks.set(++callback, fn);
            return callback;
          },
          unregisterCallback: (id: number) => callbacks.delete(id),
          convertFileSrc: (p: string) => p,
          invoke: async (cmd: string, args?: Record<string, unknown>) => {
            (w.__calls as unknown[]).push([cmd, args]);
            if (cmd === "get_app_settings" || cmd === "get_default_settings") return JSON.parse(JSON.stringify(settings));
            if (cmd === "plugin:os|locale") return "de-DE";
            if (cmd === "plugin:app|version") return "0.21.21";
            if (cmd.includes("permission")) return true;
            if (cmd === "plugin:event|listen") {
              (listeners[String(args?.event)] ??= []).push(Number(args?.handler));
              return ++callback;
            }
            if (cmd === "pages_list") return [{ id: "p1", title: "Die Leuchtturmwärterin" }];
            if (cmd === "books_list") return [];
            if (cmd === "page_state_load") return JSON.stringify({ text, tab: "original" });
            if (cmd === "page_dir") return "C:\\projects\\p1";
            if (cmd === "library_progress_load") return {};
            if (cmd === "book_talk_ask") {
              // Der Kern lässt nur eine Antwort zugleich zu (Code `chat_busy`).
              if (inflight) throw "chat_busy";
              inflight = true;
              if (holdLeft > 0) {
                holdLeft--;
                return new Promise((_, reject) => {
                  releaseHeld = () => {
                    setTimeout(() => {
                      inflight = false;
                      reject("cancelled");
                    }, 80);
                  };
                });
              }
              inflight = false;
              const req = args?.req as { request_id: string } | undefined;
              const a = answer as { sentences: unknown[] };
              (w.__emit as (n: string, p: unknown) => void)("book-talk-event", {
                kind: "done",
                request_id: req?.request_id,
                answer: a,
              });
              return a;
            }
            if (cmd === "book_talk_history") return [];
            if (cmd === "book_talk_cancel") {
              releaseHeld?.();
              releaseHeld = null;
              return true;
            }
            if (cmd === "book_talk_clear") return null;
            if (cmd === "tts_server_status") return { phase: "stopped", message: null };
            if (
              cmd === "llm_ps" ||
              cmd === "tts_reading_list" ||
              cmd === "tts_list_voices" ||
              cmd === "tts_list_voice_infos" ||
              cmd === "tts_list_downloads" ||
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
    },
    [
      TEXT,
      opts.enabled ?? true,
      opts.answer ?? ANSWER(),
      opts.theme ?? "light",
      opts.cloud ?? false,
      opts.holdFirst ?? false,
      opts.frontier ?? 400,
    ] as const,
  );
  await page.goto("/");
  if ((opts.width ?? 1440) < 768) await page.locator(".lv-titlebar__hamburger").click();
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .filter({ visible: true })
    .first()
    .click();
  if ((opts.width ?? 1440) <= 640) {
    // Schmales Fenster: das Gespräch liegt im Blatt „Gespräch“, nicht in der Spalte.
    await page.getByTestId("book-sheet-chat").click();
  } else {
    await page.getByTestId("tts-tab-talk-open").click();
  }
  await expect(page.getByTestId("booktalk")).toBeVisible();
};

const calls = (page: Page, cmd: string) =>
  page.evaluate(
    (cmd) =>
      ((window as unknown as { __calls: [string, Record<string, unknown>][] }).__calls ?? [])
        .filter(([c]) => c === cmd)
        .map(([, a]) => a),
    cmd,
  );

test("Spalte zeigt den Schutzbereich und Vorschläge", async ({ page }) => {
  await open(page);
  await expect(page.getByTestId("booktalk-scope")).toContainText("geschützt");
  await expect(page.getByTestId("booktalk-suggestions")).toBeVisible();
});

test("Ganzes Buch braucht eine Bestätigung", async ({ page }) => {
  await open(page);
  await page.getByTestId("booktalk-scope").click();
  await page.getByTestId("booktalk-scope-book").click();
  await page.getByTestId("booktalk-scope-confirm").click();
  await expect(page.getByTestId("booktalk-scope")).toContainText("Ganzes Buch");
});

test("Frage senden, Beleg öffnet die Karte, Hinspringen markiert die Stelle", async ({ page }) => {
  await open(page);
  await page.getByTestId("booktalk-input").fill("Wo steht der Leuchtturm?");
  await page.getByTestId("booktalk-send").click();
  const [req] = (await calls(page, "book_talk_ask")) as { req: { scope: string; input: string; frontier: number } }[];
  expect(req.req.scope).toBe("until_position");
  expect(req.req.input).toBe("text");
  expect(req.req.frontier).toBeGreaterThanOrEqual(400);
  await page.getByTestId("booktalk-chip").first().click();
  await expect(page.getByTestId("booktalk-card-quote")).toContainText("Leuchtturm");
  await page.getByTestId("booktalk-jump").click();
  await expect(page.locator(".reader-cur, ::highlight(reader-cur)").first()).toBeDefined();
});

test("Fundstellen-Modus zeigt Passagen mit Hinweis", async ({ page }) => {
  await open(page, {
    answer: ANSWER({ found_only: true, reason: "no_model", sentences: [], text: "", speech: "" }),
  });
  await page.getByTestId("booktalk-input").fill("Leuchtturm");
  await page.getByTestId("booktalk-send").click();
  await expect(page.getByTestId("booktalk-found")).toBeVisible();
});

test("Schalter aus: Hinweis statt Modellantwort", async ({ page }) => {
  await open(page, { enabled: false });
  await expect(page.getByTestId("booktalk-disabled")).toBeVisible();
});

const frontierOfStore = (page: Page) =>
  page.evaluate(() => {
    const raw = window.localStorage.getItem("lva.ui.progress.text:p1:original");
    return raw ? ((JSON.parse(raw) as { frontier?: number }).frontier ?? 0) : 0;
  });

/** Sichtbar heißt: der Satz liegt im Ausschnitt des Lesers, nicht auf einer verdeckten Seite. */
const visibleInReader = async (page: Page, selector: string) => {
  const el = page.locator(selector).first();
  await expect(el).toHaveCount(1);
  const [box, book] = await Promise.all([
    el.boundingBox(),
    page.getByTestId("reader-book").boundingBox(),
  ]);
  expect(box, "Satz hat eine Box").not.toBeNull();
  expect(book, "Leser hat eine Box").not.toBeNull();
  const cx = box!.x + box!.width / 2;
  expect(cx).toBeGreaterThan(book!.x);
  expect(cx).toBeLessThan(book!.x + book!.width);
};

const consentFree = ANSWER();

test("Cloud-Modell: der Freigabedialog erscheint vor der ersten Anfrage, je Modell", async ({ page }) => {
  await open(page, { cloud: true, answer: consentFree });
  await page.getByTestId("booktalk-input").fill("Wo steht der Leuchtturm?");
  await page.getByTestId("booktalk-send").click();
  const dialog = page.getByTestId("booktalk-consent");
  await expect(dialog).toBeVisible();
  await expect(page.getByTestId("booktalk-consent-provider")).toHaveText("OpenAI");
  await expect(page.getByTestId("booktalk-consent-model")).toContainText(/GPT/i);
  await expect(page.getByTestId("booktalk-consent-cost")).toContainText("USD");
  expect(await calls(page, "book_talk_ask"), "nichts geht vor der Freigabe an den Kern").toHaveLength(0);
  await page.getByTestId("booktalk-consent-allow").click();
  await expect(page.getByTestId("booktalk-answer")).toBeVisible();
  const saved = await calls(page, "change_book_talk_cloud_consent_setting");
  expect(saved.at(-1)).toEqual({ providers: ["c1::gpt-a"] });
  expect(await calls(page, "book_talk_ask")).toHaveLength(1);
  // Freigegeben: die nächste Frage geht ohne Dialog durch.
  await page.getByTestId("booktalk-input").fill("Und danach?");
  await page.getByTestId("booktalk-send").click();
  await expect(page.getByTestId("booktalk-turn")).toHaveCount(2);
  await expect(page.getByTestId("booktalk-consent")).toHaveCount(0);
  expect(await calls(page, "book_talk_ask")).toHaveLength(2);
});

test("Freigabe abbrechen: nichts wird gesendet; Widerruf steht im Menü", async ({ page }) => {
  await open(page, { cloud: true });
  await page.getByTestId("booktalk-input").fill("Frage");
  await page.getByTestId("booktalk-send").click();
  await page.getByRole("button", { name: "Nicht senden" }).click();
  expect(await calls(page, "book_talk_ask")).toHaveLength(0);
  // Menü „Weitere Aktionen“: ohne Freigabe kein Widerruf.
  await page.getByTestId("booktalk-more").click();
  await expect(page.getByTestId("booktalk-revoke")).toHaveCount(0);
  await page.keyboard.press("Escape");
  // Freigeben, dann im Menü widerrufen.
  await page.getByTestId("booktalk-send").isVisible();
  await page.getByTestId("booktalk-input").fill("Frage");
  await page.getByTestId("booktalk-send").click();
  await page.getByTestId("booktalk-consent-allow").click();
  await expect(page.getByTestId("booktalk-answer")).toBeVisible();
  await page.getByTestId("booktalk-more").click();
  await page.getByTestId("booktalk-revoke").click();
  const saved = await calls(page, "change_book_talk_cloud_consent_setting");
  expect(saved.at(-1)).toEqual({ providers: [] });
});

test("Schalter „KI-Antworten (Vorschau)“ mit Hinweis, Standard aus; jede Antwort trägt „Vorschau“", async ({ page }) => {
  await open(page, { enabled: false });
  await page.getByTestId("booktalk-more").click();
  const item = page.getByTestId("booktalk-preview");
  await expect(item).toHaveAttribute("aria-checked", "false");
  await expect(page.getByTestId("booktalk-preview-hint")).toContainText("noch nicht bestanden");
  await expect(page.getByTestId("booktalk-preview-hint")).toContainText("Spoilerschutz verringert das Risiko");
  await item.click();
  const set = await calls(page, "change_book_talk_enabled_setting");
  expect(set.at(-1)).toEqual({ enabled: true });
  await expect(page.getByTestId("booktalk-disabled")).toHaveCount(0);
  await page.getByTestId("booktalk-input").fill("Wo steht der Leuchtturm?");
  await page.getByTestId("booktalk-send").click();
  await expect(page.getByTestId("booktalk-preview-tag")).toContainText("Vorschau");
});

test("nach einem Bereichswechsel fragt „Im neuen Bereich fragen“ ohne chat_busy neu", async ({ page }) => {
  await open(page, { holdFirst: true });
  await page.getByTestId("booktalk-input").fill("Wo steht der Leuchtturm?");
  await page.getByTestId("booktalk-send").click();
  await expect.poll(async () => (await calls(page, "book_talk_ask")).length).toBe(1);
  await page.getByTestId("booktalk-scope").click();
  await page.getByTestId("booktalk-scope-book").click();
  await page.getByTestId("booktalk-scope-confirm").click();
  await expect(page.getByTestId("booktalk-cancelled")).toContainText("Bereich gewechselt");
  expect(await calls(page, "book_talk_cancel")).toHaveLength(1);
  await page.getByTestId("booktalk-ask-again").click();
  await expect(page.getByTestId("booktalk-turn")).toHaveCount(2);
  await expect(page.getByTestId("booktalk-turn").nth(1)).toHaveAttribute("data-status", "done");
  const asks = (await calls(page, "book_talk_ask")) as { req: { scope: string } }[];
  expect(asks).toHaveLength(2);
  expect(asks[1].req.scope).toBe("book");
  await expect(page.getByRole("alert")).toHaveCount(0);
});

test("Hinspringen zeigt wirklich die Seite des Belegs und hebt die Lesegrenze nicht", async ({ page }) => {
  await open(page, { answer: ANSWER({ citations: [LATE_CITATION], sentences: [{ index: 0, text: "Mara lauscht.", speech: "Mara lauscht.", citations: [LATE_CITATION], general: false }] }) });
  const before = await frontierOfStore(page);
  const label = page.getByTestId("reader-page-label");
  const first = await label.textContent();
  await page.getByTestId("booktalk-input").fill("Wann lauscht Mara?");
  await page.getByTestId("booktalk-send").click();
  await page.getByTestId("booktalk-chip").first().click();
  await page.getByTestId("booktalk-jump").click();
  await expect(label).not.toHaveText(first ?? "");
  await visibleInReader(page, `[data-start="${LATE_START}"]`);
  await visibleInReader(page, "[data-cur]");
  expect(await frontierOfStore(page), "ein Sprung hebt die Grenze nie").toBe(before);
});

test("nur sequentielles Weiterblättern hebt die Lesegrenze", async ({ page }) => {
  await open(page, { frontier: 0 });
  const first = await frontierOfStore(page);
  await page.getByTestId("reader-next-page").click();
  await expect.poll(() => frontierOfStore(page)).toBeGreaterThan(first);
  const turned = await frontierOfStore(page);
  // Ein Sprung weit nach hinten (Seitenzahl-Eingabe) lässt sie, wo sie ist.
  await page.getByTestId("reader-flow").focus();
  await page.keyboard.press("End");
  await expect(page.getByTestId("reader-next-page")).toBeDisabled();
  expect(await frontierOfStore(page)).toBe(turned);
  // Weiterblättern nach dem Sprung schließt nicht an: auch das hebt sie nicht.
  await page.getByTestId("reader-prev-page").click();
  await page.getByTestId("reader-next-page").click();
  expect(await frontierOfStore(page)).toBe(turned);
});

test("schmal: nur ein Gesprächsbereich, Hinspringen schließt das Blatt und zeigt den Beleg", async ({ page }) => {
  await open(page, {
    width: 390,
    height: 800,
    answer: ANSWER({ citations: [LATE_CITATION], sentences: [{ index: 0, text: "Mara lauscht.", speech: "Mara lauscht.", citations: [LATE_CITATION], general: false }] }),
  });
  await expect(page.getByTestId("booktalk")).toHaveCount(1);
  await expect(page.getByTestId("tts-talk-column")).toHaveCount(0);
  // Die Seitenzahl des Lesers stimmt mit dem Umbruch überein (sonst landen Sprünge falsch).
  const layoutPages = await page.evaluate(() => {
    const f = document.querySelector<HTMLElement>('[data-testid="reader-flow"]')!;
    const col = parseFloat(f.style.columnWidth);
    const gap = parseFloat(f.style.columnGap);
    return Math.round((f.scrollWidth + gap) / (col + gap));
  });
  await expect(page.getByTestId("reader-page-label")).toHaveText(new RegExp(`von ${layoutPages}$`));
  await page.getByTestId("booktalk-input").fill("Wann lauscht Mara?");
  await page.getByTestId("booktalk-send").click();
  await page.getByTestId("booktalk-chip").first().click();
  await page.getByTestId("booktalk-jump").click();
  await expect(page.getByTestId("booktalk")).toHaveCount(0);
  await visibleInReader(page, `[data-start="${LATE_START}"]`);
});


test("schmal: die Eingabe im Blatt ist vollständig sichtbar, nichts wird abgeschnitten", async ({ page }) => {
  await open(page, { width: 390, height: 800 });
  await page.getByTestId("booktalk-input").fill("Wo steht der Leuchtturm?");
  await page.getByTestId("booktalk-send").click();
  await page.getByTestId("booktalk-chip").first().click();
  const rects = await page.evaluate(() => {
    const r = (sel: string) => {
      const b = document.querySelector<HTMLElement>(sel)!.getBoundingClientRect();
      return { left: b.left, right: b.right, top: b.top, bottom: b.bottom };
    };
    return { body: r(".kidoc-sheet .lv-dialog__body"), mic: r(".bt__mic"), send: r(".bt__send"), field: r(".bt__field"), vh: window.innerHeight, vw: window.innerWidth };
  });
  for (const part of [rects.mic, rects.send, rects.field]) {
    expect(part.bottom).toBeLessThanOrEqual(rects.body.bottom);
    expect(part.right).toBeLessThanOrEqual(Math.min(rects.body.right, rects.vw));
    expect(part.left).toBeGreaterThanOrEqual(rects.body.left);
  }
  expect(rects.mic.bottom).toBeLessThanOrEqual(rects.vh);
  // Ziele mindestens 44 px (Plan §9, Desktop schmal).
  expect(rects.send.right - rects.send.left).toBeGreaterThanOrEqual(44);
  expect(rects.mic.bottom - rects.mic.top).toBeGreaterThanOrEqual(44);
});

test("breit: Taste G blendet die Spalte ein und aus, gehaltene Taste wiederholt nicht", async ({ page }) => {
  await open(page);
  await expect(page.getByTestId("tts-talk-column")).toBeVisible();
  await page.locator("body").press("g");
  await expect(page.getByTestId("tts-talk-column")).toHaveCount(0);
  await page.locator("body").press("g");
  await expect(page.getByTestId("tts-talk-column")).toBeVisible();
  // Gehalten (Wiederholung): nur ein Wechsel.
  await page.evaluate(() => {
    for (const repeat of [false, true, true, true])
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "g", repeat, bubbles: true }));
  });
  await expect(page.getByTestId("tts-talk-column")).toHaveCount(0);
});

test("Reiter und Menüs: Fokus, Pfeiltasten, tabpanel", async ({ page }) => {
  await open(page);
  const tab = page.getByTestId("tts-tab-talk");
  await expect(tab).toHaveAttribute("aria-controls", "tts-talk-panel");
  await expect(page.locator("#tts-talk-panel")).toHaveAttribute("role", "tabpanel");
  await expect(page.locator("#tts-talk-panel")).toHaveAttribute("aria-labelledby", "tts-tab-talk-id");
  // Menü: Fokus hinein, Pfeiltasten wechseln, Esc bringt den Fokus zurück.
  const more = page.getByTestId("booktalk-more");
  await more.focus();
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("booktalk-preview")).toBeFocused();
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("ArrowDown");
  await expect(page.getByTestId("booktalk-preview")).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("booktalk-more-menu")).toHaveCount(0);
  await expect(more).toBeFocused();
  // Reiterwechsel setzt den Fokus auf den neuen Reiter.
  await tab.focus();
  await page.keyboard.press("ArrowLeft");
  await expect(page.getByTestId("tts-tab-talk-open")).toBeFocused();
  await page.getByTestId("tts-tab-talk-open").click();
  await expect(page.getByTestId("tts-tab-talk")).toBeFocused();
});

for (const theme of ["light", "dark"]) {
  for (const width of [1440]) {
    test(`Vorschau ${width} ${theme}`, async ({ page }) => {
      test.skip(!process.env.BOOKTALK_SHOTS, "nur mit BOOKTALK_SHOTS");
      await open(page, { width, theme });
      await page.getByTestId("booktalk-input").fill("Wo steht der Leuchtturm?");
      await page.getByTestId("booktalk-send").click();
      await page.getByTestId("booktalk-chip").first().click();
      await page.screenshot({
        path: `../../docs/vorschau/buch-gespraech-desktop/${width}-${theme}.png`,
      });
    });
  }
  test(`Vorschau 390 Blatt Gespräch ${theme}`, async ({ page }) => {
    test.skip(!process.env.BOOKTALK_SHOTS, "nur mit BOOKTALK_SHOTS");
    await open(page, { width: 390, height: 800, theme });
    await page.getByTestId("booktalk-input").fill("Wo steht der Leuchtturm?");
    await page.getByTestId("booktalk-send").click();
    await page.getByTestId("booktalk-chip").first().click();
    await page.screenshot({
      path: `../../docs/vorschau/buch-gespraech-desktop/390-blatt-${theme}.png`,
    });
  });
}
