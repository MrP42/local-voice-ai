import { test, expect, type Page } from "@playwright/test";

// Fußleiste je Modul (Patrick 02.11.): Die Modelle des geöffneten Moduls
// stehen links unten, immer an derselben Stelle. Beim Vorlesen: Stimme und
// das Modell der Funktion „Vorlesen“ (Übersetzen, KI-Dokumente). Der Hinweis
// „Skript geprüft: keine Befunde.“ ist nur ein kurzer Status in der Fußleiste.
// Abnahmebilder: docs/vorschau/fussleiste-modelle/.
const SHOTS = "docs/vorschau/fussleiste-modelle";

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
      tts_script_check: true,
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
      function_models: {} as Record<string, unknown>,
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
          if (cmd === "llm_set_function_model") {
            const fm = settings.function_models;
            if (args?.modelId)
              fm[String(args.forFunction)] = {
                model_id: args.modelId,
                effort: args.effort ?? null,
              };
            else delete fm[String(args?.forFunction)];
            return null;
          }
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
            cmd === "library_list" ||
            cmd === "library_trash_list" ||
            cmd === "library_sizes"
          )
            return [];
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

const TEXT = [
  "# Kapitel 3",
  "",
  "Der Leuchtturm stand seit über hundert Jahren auf der Klippe.",
  "",
  "„Noch zwölf Stufen“, sagte sie zu niemandem.",
].join("\n");

const open = async (page: Page, width: number, theme: string) => {
  await page.emulateMedia({ colorScheme: theme === "dark" ? "dark" : "light" });
  await page.setViewportSize({ width, height: 900 });
  await page.addInitScript(
    ([t, th]) => {
      (window as unknown as { __pageText: string }).__pageText = t;
      (window as unknown as { __theme: string }).__theme = th;
      window.localStorage.removeItem("lva.ui.tts.view");
    },
    [TEXT, theme] as const,
  );
  await page.goto("/");
  if (width < 600)
    await page.getByRole("button", { name: "Menü öffnen" }).click();
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await expect(page.locator("#page-title")).toHaveText("Vorlesen");
};

const bar = (page: Page) => page.locator(".lv-statusbar");

test("Vorlesen: Stimme und Modell stehen links in der Fußleiste, nicht in der Seite", async ({
  page,
}) => {
  await open(page, 1440, "light");
  const fuss = bar(page).getByTestId("fuss-modelle");
  await expect(fuss).toHaveAttribute("data-modul", "tts");
  await expect(fuss.getByTestId("voice-select")).toHaveCount(1);
  await expect(page.getByTestId("voice-select")).toHaveCount(1);
  // Gleiche Darstellung wie in der Bibliothek (Patrick 03.11.): nur der
  // Modellname, die Funktion steht in Tooltip und Vorlesetext.
  const chip = fuss.getByTestId("fuss-modell-read_aloud");
  await expect(chip).toContainText("Claude Sonnet 5.5");
  await expect(chip).not.toContainText("Vorlesen:");
  await expect(chip).toHaveAttribute("aria-label", /Modell für Vorlesen/);
  // Übersetzen: keine eigene Modellwahl mehr mitten in der Seite.
  await page.getByRole("tab", { name: /Übersetzung/ }).click();
  await expect(page.getByTestId("tts-translation-model")).toHaveCount(0);
  // Die Fußleiste behält ihre Höhe (24 px) und liegt unter der Spielerleiste.
  expect((await bar(page).boundingBox())!.height).toBe(24);
});

test("Modell der Funktion „Vorlesen“: Liste öffnet nach oben, Wahl gilt nur hier", async ({
  page,
}) => {
  await open(page, 1440, "light");
  const knopf = page.getByTestId("fuss-modell-read_aloud");
  await expect(knopf).toContainText("Claude Sonnet 5.5");
  await knopf.click();
  const menue = page.getByTestId("fuss-modell-menue-read_aloud");
  await expect(menue).toBeVisible();
  const m = (await menue.boundingBox())!;
  const k = (await knopf.boundingBox())!;
  expect(m.y + m.height).toBeLessThanOrEqual(k.y);
  await expect(menue).toBeInViewport({ ratio: 0.98 });
  for (const item of await menue.getByRole("menuitemradio").all())
    expect((await item.boundingBox())!.height).toBeGreaterThanOrEqual(44);
  await menue.locator('[data-function-option="m2"]').click();
  const calls = await page.evaluate(
    () => (window as unknown as { __calls: [string, unknown][] }).__calls,
  );
  expect(
    calls.find(([c]) => c === "llm_set_function_model")?.[1],
  ).toMatchObject({ forFunction: "read_aloud", modelId: "m2" });
  // Global unverändert: kein Wechsel des aktiven Modells.
  expect(calls.some(([c]) => c === "llm_set_active_model")).toBe(false);
});

test("Andere Module: eigene Modelle an derselben Stelle", async ({ page }) => {
  await open(page, 1440, "light");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Verlauf", exact: true })
    .click();
  const fuss = bar(page).getByTestId("fuss-modelle");
  await expect(fuss).not.toHaveAttribute("data-modul", "tts");
  await expect(fuss.locator("[data-llm-selector]")).toHaveCount(1);
  await expect(page.getByTestId("voice-select")).toHaveCount(0);
});

test("Skript geprüft: kurzer Status in der Fußleiste, kein Kasten über dem Text", async ({
  page,
}) => {
  await open(page, 1440, "light");
  await page.getByTestId("tts-action-menu").click();
  await page.getByTestId("script-check-run").click();
  const status = bar(page).getByTestId("fuss-status");
  await expect(status).toHaveText("Skript geprüft: keine Befunde.");
  await expect(status).toHaveAttribute("role", "status");
  await expect(page.getByTestId("script-check-clean")).toHaveCount(0);
  await page.screenshot({
    path: `${SHOTS}/status-skript-geprueft-light-1440.png`,
    animations: "disabled",
  });
  await expect(status).toHaveText("", { timeout: 8000 });
});

test.describe("Touch", () => {
  test.use({ hasTouch: true });
  test("Fußleiste mit 44 px Trefferfläche", async ({ page }) => {
    await open(page, 1024, "light");
    expect(
      await page.evaluate(() => matchMedia("(pointer: coarse)").matches),
    ).toBe(true);
    for (const b of await bar(page).locator("button").all()) {
      if (!(await b.isVisible())) continue;
      const box = (await b.boundingBox())!;
      expect(
        box.height,
        (await b.getAttribute("aria-label")) ??
          (await b.getAttribute("class")) ??
          "",
      ).toBeGreaterThanOrEqual(43.5);
    }
    await page.screenshot({
      path: `${SHOTS}/vorlesen-touch-light-1024.png`,
      animations: "disabled",
    });
  });
});

for (const theme of ["light", "dark"]) {
  test(`Abnahmebilder (${theme})`, async ({ page }) => {
    await open(page, 1440, theme);
    await page.screenshot({
      path: `${SHOTS}/vorlesen-${theme}-1440.png`,
      animations: "disabled",
    });
    await page.getByTestId("fuss-modell-read_aloud").click();
    await page.screenshot({
      path: `${SHOTS}/modellwahl-${theme}-1440.png`,
      animations: "disabled",
    });
    await page.keyboard.press("Escape");
    await page
      .getByTestId("fuss-stimme")
      .locator(".app-select__control")
      .click();
    await page.screenshot({
      path: `${SHOTS}/stimme-${theme}-1440.png`,
      animations: "disabled",
    });
  });
}

// Patrick 03.11.: Das Modell steht in Vorlesen und Aufnahmen genauso in
// der Fußleiste wie in der Bibliothek: Funkel-Symbol mit Ampel, Modellname,
// Abzeichen, Pfeil. Verglichen wird der Aufbau des Knopfs ohne Text.
const GLEICH = "docs/vorschau/fussleiste-modell-gleich";
const aufbau = (page: Page) =>
  bar(page)
    .getByTestId("fuss-modelle")
    .locator("[data-llm-selector], [data-testid^='fuss-modell-']")
    .first()
    .evaluate((b) => {
      const walk = (n: Element): string =>
        `${n.tagName}.${(n.getAttribute("class") ?? "")
          .split(/\s+/)
          .filter((c) => c && !/^bg-(green|yellow|red)-|animate-pulse/.test(c))
          .sort()
          .join(".")}(${Array.from(n.children).map(walk).join(",")})`;
      return walk(b);
    });

const zu = async (page: Page, width: number, name: string) => {
  if (width < 600)
    await page.getByRole("button", { name: "Menü öffnen" }).click();
  await page
    .getByRole("navigation")
    .getByRole("button", { name, exact: true })
    .click();
  await expect(page.locator("#page-title")).toHaveText(name);
};

test("Modellmenü: Pfeiltasten wandern, Escape schließt und gibt den Fokus zurück", async ({
  page,
}) => {
  await open(page, 1440, "light");
  const knopf = page.getByTestId("fuss-modell-read_aloud");
  await knopf.click();
  const menue = page.getByTestId("fuss-modell-menue-read_aloud");
  await expect(menue).toBeVisible();
  await page.keyboard.press("ArrowDown");
  await expect(menue.locator('[data-function-option=""]')).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(menue.locator('[data-function-option="m1"]')).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(menue).toHaveCount(0);
  await expect(knopf).toBeFocused();
});

test("Bibliothek, Vorlesen und Aufnahmen zeigen das Modell gleich", async ({
  page,
}) => {
  await open(page, 1440, "light");
  const vorlesen = await aufbau(page);
  await zu(page, 1440, "Bibliothek");
  const bibliothek = await aufbau(page);
  await zu(page, 1440, "Aufnahmen");
  const aufnahmen = await aufbau(page);
  expect(vorlesen).toBe(bibliothek);
  expect(aufnahmen).toBe(bibliothek);
});

test.describe("Abnahmebilder gleiche Modellanzeige", () => {
  // Schärfere Bilder für die Abnahme (die Fußleiste ist nur 24 px hoch).
  test.use({ deviceScaleFactor: 2 });
  for (const theme of ["light", "dark"])
    for (const width of [390, 1440])
      test(`Abnahmebilder gleiche Modellanzeige (${theme}, ${width})`, async ({
        page,
      }) => {
        await open(page, width, theme);
        // Die schmale Fußleiste rollt seitlich; das Modell muss zu sehen sein.
        await bar(page)
          .getByTestId("fuss-modell-read_aloud")
          .scrollIntoViewIfNeeded();
        const leiste = bar(page);
        await leiste.screenshot({
          path: `${GLEICH}/vorlesen-${theme}-${width}.png`,
          animations: "disabled",
        });
        if (width >= 600)
          await page.screenshot({
            path: `${GLEICH}/vorlesen-seite-${theme}-${width}.png`,
            animations: "disabled",
          });
        await page.getByTestId("fuss-modell-read_aloud").click();
        await expect(
          page.getByTestId("fuss-modell-menue-read_aloud"),
        ).toBeInViewport({ ratio: 0.98 });
        await page.screenshot({
          path: `${GLEICH}/vorlesen-menue-${theme}-${width}.png`,
          animations: "disabled",
        });
        await page.keyboard.press("Escape");
        for (const [name, datei] of [
          ["Bibliothek", "bibliothek"],
          ["Aufnahmen", "aufnahmen"],
        ] as const) {
          await zu(page, width, name);
          await leiste.screenshot({
            path: `${GLEICH}/${datei}-${theme}-${width}.png`,
            animations: "disabled",
          });
        }
      });
});
