import { test, expect, type Page } from "@playwright/test";

// Buchübersetzung im Vorlesemodul: Ergebnis streamt abschnittsweise auf die
// Übersetzungsseite, feine Fortschrittszeile, Fortsetzen nach Fehler.
// Die Bilder für die Abnahme landen in docs/vorschau/buchuebersetzung/.
const SHOTS = "docs/vorschau/buchuebersetzung";

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
      // Erst beim Abruf lesen: die Init-Skripte laufen in Registrierungsreihenfolge,
      // __theme wird erst von start() gesetzt (nach diesem Skript).
      get theme() {
        return (window as unknown as { __theme?: string }).__theme ?? "light";
      },
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
      llm_connections: [
        {
          id: "c1",
          kind: "claude_cli",
          label: "Claude (Abo)",
          base_url: "",
          enabled: true,
        },
      ],
      llm_models: [
        {
          id: "m1",
          connection_id: "c1",
          remote_id: "sonnet",
          label: "sonnet",
          enabled: true,
        },
        {
          id: "m2",
          connection_id: "c1",
          remote_id: "opus",
          label: "opus",
          enabled: true,
        },
      ],
      llm_active_model_id: "m1",
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
              ? Object.keys(parts).map((id, i) => ({
                  id,
                  title: `Teil ${i + 1}`,
                }))
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
              (window as unknown as { __endSpeak: () => void }).__endSpeak =
                () => done(null);
            });
          if (cmd === "page_dir") return "C:\\projects\\p1";
          if (cmd === "pages_create")
            return {
              id: "p2",
              title: String(args?.title ?? ""),
              modified_ms: 0,
              preview: "",
            };
          if (cmd === "plugin:dialog|open")
            return (window as unknown as { __pick?: string }).__pick ?? null;
          if (cmd === "tts_extract_document")
            return (
              (window as unknown as { __extracted?: string }).__extracted ?? ""
            );
          if (cmd === "tts_list_voice_infos")
            return (
              (window as unknown as { __voices?: unknown[] }).__voices ?? []
            );
          if (cmd === "isbn_lookup") {
            const hit = (window as unknown as { __isbn?: unknown }).__isbn;
            if (typeof hit === "string") throw hit;
            return hit ?? null;
          }
          if (cmd === "tts_cached_translation") return null;
          if (cmd === "tts_translate")
            return new Promise((done, fail) => {
              const x = window as unknown as {
                __finishTranslate: (v: string) => void;
                __failTranslate: (e: string) => void;
              };
              x.__finishTranslate = (v) => done(v);
              x.__failTranslate = (e) => fail(e);
            });
          if (cmd === "library_progress_load")
            return (
              (window as unknown as { __remoteProgress?: unknown })
                .__remoteProgress ?? {}
            );
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

const emit = (page: Page, name: string, payload: unknown) =>
  page.evaluate(
    ([name, payload]) =>
      (window as unknown as { __emit: (n: string, p: unknown) => void }).__emit(
        name as string,
        payload,
      ),
    [name, payload] as const,
  );

const BOOK = [
  "# Kapitel 3",
  "",
  "<Erzählerin> Der Leuchtturm stand seit über hundert Jahren auf der Klippe. {{S. 12}}",
  "",
  "<Mara> „Noch zwölf Stufen“, sagte sie zu niemandem.",
].join("\n");

const chunk = (index: number, text: string) => ({
  index,
  total: 41,
  chapter: 2,
  chapters: 18,
  chapterTitle: "Wie das Licht zurückkam und was die Wärterin darüber dachte",
  text,
  cached: false,
});

const start = async (page: Page, width: number, theme: string) => {
  await page.emulateMedia({ colorScheme: theme === "dark" ? "dark" : "light" });
  await page.setViewportSize({ width, height: 900 });
  await page.addInitScript(
    ([t, th]) => {
      (window as unknown as { __pageText: string }).__pageText = t;
      (window as unknown as { __theme: string }).__theme = th;
      window.localStorage.removeItem("lva.ui.tts.view");
    },
    [BOOK, theme] as const,
  );
  await page.goto("/");
  if (width < 600)
    await page.getByRole("button", { name: "Menü öffnen" }).click();
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await page.getByRole("tab", { name: /Übersetzung/ }).click();
  await page.getByTestId("tts-action-translate").click();
  await emit(page, "tts-translate-start", {
    total: 41,
    chapters: 18,
    totalChars: 90000,
  });
  await emit(page, "tts-translate-chunk", chunk(0, "# Chapter 3"));
  await emit(
    page,
    "tts-translate-chunk",
    chunk(
      1,
      "<Narrator> The lighthouse had stood on the cliff for over a hundred years. {{S. 12}}",
    ),
  );
  await emit(
    page,
    "tts-translate-chunk",
    chunk(2, "<Mara> \u201cTwelve more steps,\u201d she said to no one."),
  );
  await emit(page, "tts-transform-progress", {
    kind: "translation",
    done: 12,
    total: 41,
    chapter: 2,
    chapters: 18,
    chapterTitle: "Wie das Licht zurückkam und was die Wärterin darüber dachte",
    doneChars: 26100,
    totalChars: 90000,
    workers: [
      { done: 5, total: 10 },
      { done: 4, total: 10 },
      { done: 2, total: 11 },
      { done: 1, total: 10 },
    ],
  });
};

for (const theme of ["light", "dark"]) {
  for (const width of [1440, 390]) {
    test(`Übersetzung streamt mit Fortschrittszeile und Fortsetzen (${theme}, ${width})`, async ({
      page,
    }) => {
      await start(page, width, theme);
      const flow = page.getByTestId("reader-flow");
      await expect(flow).toContainText("The lighthouse had stood");
      await expect(page.locator("textarea")).toHaveCount(0);
      await expect(page.getByTestId("tts-translation-model")).toContainText(
        "Claude (Abo) · Claude Sonnet 5.5",
      );
      await expect(
        page.getByText("Kapitel 3 von 18 · Abschnitt 13 von 41 · 29 %"),
      ).toBeVisible();
      await expect(
        page.getByTestId("tts-worker-bars").locator("> div"),
      ).toHaveCount(4);
      await page.screenshot({ path: `${SHOTS}/laeuft-${theme}-${width}.png` });

      const args = await page.evaluate(() =>
        (
          window as unknown as { __calls: [string, Record<string, unknown>][] }
        ).__calls
          .filter(([c]) => c === "tts_translate")
          .map(([, a]) => a),
      );
      // Seitenmarken gehen unverändert ans Backend.
      expect(String(args[0]?.text)).toContain("{{S. 12}}");

      await page.evaluate(() =>
        (
          window as unknown as { __failTranslate: (e: string) => void }
        ).__failTranslate(
          "Kapitel 4, Abschnitt 14: Zeitüberschreitung des Modells",
        ),
      );
      await expect(page.getByTestId("tts-translate-partial")).toContainText(
        "Zeitüberschreitung",
      );
      await expect(flow).toContainText("The lighthouse had stood");
      const resume = page.getByTestId("tts-translate-resume");
      await expect(resume).toBeVisible();
      expect((await resume.boundingBox())!.height).toBeGreaterThanOrEqual(
        width === 390 ? 44 : 20,
      );
      await page.screenshot({ path: `${SHOTS}/fehler-${theme}-${width}.png` });
    });
  }
}
