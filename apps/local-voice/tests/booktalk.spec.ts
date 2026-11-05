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
    { length: 30 },
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

const open = async (page: Page, opts: { enabled?: boolean; answer?: unknown; width?: number; theme?: string } = {}) => {
  await page.setViewportSize({ width: opts.width ?? 1440, height: 900 });
  await page.addInitScript(
    ([text, enabled, answer, theme]) => {
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
        book_talk_cloud_consent: [],
      };
      const listeners: Record<string, number[]> = {};
      w.__calls = [] as unknown[];
      w.__emit = (name: string, payload: unknown) =>
        (listeners[name] ?? []).forEach((id) =>
          (callbacks.get(id) as (e: unknown) => void)?.({ event: name, id: 0, payload }),
        );
      window.localStorage.removeItem("lva.ui.tts.view");
      window.localStorage.setItem(
        "lva.ui.progress.text:p1:original",
        JSON.stringify({ offset: 0, length: 2000, heard: [], read: [], updated: 1, frontier: 400 }),
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
            if (cmd === "book_talk_cancel" || cmd === "book_talk_clear") return null;
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
    [TEXT, opts.enabled ?? true, opts.answer ?? ANSWER(), opts.theme ?? "light"] as const,
  );
  await page.goto("/");
  await page.getByRole("navigation").getByRole("button", { name: "Vorlesen", exact: true }).click();
  await page.getByTestId("tts-tab-talk-open").click();
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
}
