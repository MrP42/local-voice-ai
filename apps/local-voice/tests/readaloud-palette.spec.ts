import { test, expect } from "@playwright/test";
import * as path from "node:path";

// Ausdruck & Sprechstil unter dem Editor (Goal ui-vorlesen-kompakt, P3): der
// geoeffnete Klappbereich ueberlagert den Text nie und der Editor behaelt
// seine Hoehe. Die Tauri-Bruecke ist dieselbe Attrappe wie in script-check.spec.ts.
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
    const voices = [
      {
        id: "erzaehlerin",
        meta: { display_name: "Erzählerin", tags: [] },
        origin: "recorded",
        avatar_path: null,
      },
      {
        id: "leo-lausemaus",
        meta: { display_name: "Leo Lausemaus", tags: [] },
        origin: "recorded",
        avatar_path: null,
      },
      // Ein realistischer Bestand: Patrick hat zwanzig Stimmen. Mit zweien
      // passt die Liste noch in jedes Fenster und beweist nichts.
      ...Array.from({ length: 18 }, (_, index) => ({
        id: `stimme-${index}`,
        meta: { display_name: `Stimme ${index}`, tags: [] },
        origin: "recorded",
        avatar_path: null,
      })),
    ];
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
            return {
              ...settings,
              // Engine je Test: Piper setzt keine Tags um (dort sind Tags
              // Befunde), Fish nimmt jede Beschreibung.
              tts_engine:
                (window as unknown as { __lvEngine?: string }).__lvEngine ??
                settings.tts_engine,
              // Ein Test schaltet die Automatik ab (siehe unten).
              tts_script_check: !(
                window as unknown as { __lvScriptCheckOff?: boolean }
              ).__lvScriptCheckOff,
            };
          if (cmd === "plugin:os|locale") return "de-DE";
          if (cmd === "plugin:app|version") return "0.16.0";
          if (cmd.includes("permission")) return true;
          if (cmd === "plugin:event|listen") return ++callback;
          if (cmd === "get_selected_model") return "";
          if (cmd === "meetings_is_recording") return false;
          if (cmd === "tts_server_status")
            return { phase: "stopped", message: null };
          if (cmd === "tts_list_voice_infos") return voices;
          if (cmd === "tts_list_voices") return voices.map((v) => v.id);
          // Nur die eine Stimme hat eine Hoerprobe auf der Platte. Die andere
          // liefert null — genau wie das Backend, wenn noch nichts erzeugt
          // wurde.
          if (cmd === "tts_voice_demo_cached")
            return args?.voiceId === "erzaehlerin"
              ? {
                  wav_path: "C:/demo/erzaehlerin.wav",
                  transcript: "Probesatz.",
                }
              : null;
          // Ein Arbeitsblatt mit einer erzeugten Aufnahme und einer Datei,
          // die keine ist — nur die eine darf einen Abspielknopf bekommen.
          if (cmd === "pages_list") return [{ id: "p1", title: "Der Sturm" }];
          if (cmd === "books_list")
            return (window as unknown as { __books?: unknown[] }).__books ?? [];
          if (cmd === "books_templates")
            return [
              {
                id: "geschichte",
                name: "Geschichte (Vorlesen)",
                body: "Schreibe: {{prompt}}",
                builtin: true,
                modified: false,
              },
            ];
          if (cmd === "books_create") {
            const book = {
              id: "book_1",
              title: args?.title,
              created_ms: 1,
              page_ids: [],
              characters: [],
              language: "de",
            };
            (window as unknown as { __books?: unknown[] }).__books = [book];
            return book;
          }
          if (cmd === "books_memory_read")
            return ["welt", "figuren", "verlauf", "stil"].map((kind) => ({
              kind,
              text: "",
            }));
          if (cmd === "books_generate")
            return {
              title: "Der Drache lernt schwimmen",
              script: "<Erzählerin> Es war einmal ein Drache.",
              memory: {
                verlauf: "Der Drache fiel ins Wasser.",
                figuren: "",
                welt: "",
              },
            };
          if (cmd === "pages_create")
            return {
              id: "p2",
              title: args?.title,
              modified_ms: 0,
              preview: "",
            };
          if (cmd === "page_state_save") return null;
          if (
            cmd === "books_add_page" ||
            cmd === "books_memory_write" ||
            cmd === "books_update"
          )
            return cmd === "books_add_page"
              ? {
                  id: "book_1",
                  title: "Drachen",
                  created_ms: 1,
                  page_ids: ["p2"],
                  characters: [],
                  language: "de",
                }
              : null;
          if (cmd === "pages_export_preview")
            return {
              title: "Der Sturm",
              files: ["notizen.txt"],
              voices: [
                {
                  id: "erzaehlerin",
                  display_name: "Erzählerin",
                  present: true,
                },
                { id: "halb", display_name: "Halb", present: false },
              ],
              rights_confirmed: false,
            };
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
          // Die Herkunft der Aufnahme: Text, Stimme, Zeitpunkt.
          if (cmd === "page_audio_note")
            return args?.name === "Der-Sturm_2026-09-08_1405.wav"
              ? {
                  text: "Es war eine dunkle Nacht. Niemand sprach.",
                  voice: "erzaehlerin",
                  seed: 42,
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
            cmd === "tts_list_voices" ||
            cmd === "llm_ps" ||
            cmd === "tts_reading_list"
          )
            return [];
          if (
            cmd.includes("history") ||
            cmd.includes("models") ||
            cmd.includes("devices") ||
            cmd === "meetings_list"
          )
            return [];
          // Eine geladene Piper-Stimme und die Laufzeit; die zweite Stimme
          // ist noch nicht geladen und darf nicht zur Auswahl stehen.
          // Der Seitenstand traegt die Stimme je Reiter -- was gespeichert wird, zaehlt.
          if (cmd === "page_state_save") {
            (window as unknown as { savedPageState?: unknown }).savedPageState =
              args?.state;
            return null;
          }
          if (cmd === "tts_tidy_text")
            return "Sauberer Text ohne Seitenzahlen.";
          if (cmd === "tts_list_downloads")
            return [
              {
                id: "piper-runtime",
                kind: "runtime",
                name: "Piper",
                description: "",
                language: null,
                size_mb: 20,
                is_downloaded: true,
                is_downloading: false,
              },
              {
                id: "de_DE-thorsten-medium",
                kind: "voice",
                name: "Thorsten (Deutsch)",
                description: "",
                language: "de",
                size_mb: 60,
                is_downloaded: true,
                is_downloading: false,
              },
              {
                id: "en_US-amy-medium",
                kind: "voice",
                name: "Amy (English)",
                description: "",
                language: "en",
                size_mb: 60,
                is_downloaded: false,
                is_downloading: false,
              },
            ];
          // Was die App wirklich speichern will — sonst prueft der Test nur,
          // dass ein Auswahlfeld umspringt.
          if (cmd === "change_tts_engine_setting") {
            (window as unknown as { savedEngine?: unknown }).savedEngine =
              args?.value;
            settings.tts_engine = args?.value as string;
            return null;
          }
          if (cmd === "change_tts_piper_voice_setting") {
            (
              window as unknown as { savedPiperVoice?: unknown }
            ).savedPiperVoice = args?.value;
            return null;
          }
          if (cmd === "get_custom_sounds") return { start: false, stop: false };
          if (cmd === "tts_speak_text_from") {
            (window as unknown as { spokenFrom?: unknown[] }).spokenFrom = [
              ...((window as unknown as { spokenFrom?: unknown[] })
                .spokenFrom ?? []),
              args,
            ];
            return null;
          }
          if (cmd === "tts_prewarm") return null;
          // Was wirklich zum Vorlesen geschickt wird — der Dialog darf das
          // Vorlesen aufhalten, aber nicht verhindern.
          if (cmd === "tts_speak_text") {
            (window as unknown as { spokenTexts?: unknown[] }).spokenTexts = [
              ...((window as unknown as { spokenTexts?: unknown[] })
                .spokenTexts ?? []),
              args?.text,
            ];
            return null;
          }
          return null;
        },
      },
    });
  });
});

// Ein langer Text (~30 Absaetze) und die Palette mit Filter "Alle" (~95 Tags):
// genau der Fall, in dem der Klappbereich frueher den Editor verdraengte.
const LANGER_TEXT = Array.from(
  { length: 30 },
  (_, index) =>
    `Absatz ${index + 1}: Es war einmal ein sehr langer Text, der den Editor füllt.`,
).join("\n\n");

async function openPaletteAlle(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  const editor = page.locator("textarea").first();
  await editor.fill(LANGER_TEXT);
  const details = page.locator(".tts-editor details.workspace-disclosure");
  await details.locator("summary").click();
  await page.getByRole("tab", { name: "Alle" }).click();
  await expect(details).toHaveJSProperty("open", true);
  return { editor, details };
}

async function box(locator: import("@playwright/test").Locator) {
  const b = await locator.boundingBox();
  if (!b) throw new Error("kein Rahmen");
  return b;
}

for (const size of [
  { width: 1920, height: 1050 },
  { width: 1366, height: 768 },
]) {
  test(`palette opened with "Alle" scrolls and never covers the editor at ${size.width}x${size.height}`, async ({
    page,
  }) => {
    await page.setViewportSize(size);
    const { editor, details } = await openPaletteAlle(page);
    const fill = page.locator(".tts-editor__fill");

    // Der Klappbereich ist höhenbegrenzt und scrollt in sich.
    const scroll = await details.evaluate((el) => ({
      scrollHeight: el.scrollHeight,
      clientHeight: el.clientHeight,
    }));
    expect(scroll.scrollHeight).toBeGreaterThan(scroll.clientHeight);

    // Der Editor behält seine Mindesthöhe und die textarea läuft nicht über
    // ihren Kasten hinaus.
    const fillBox = await box(fill);
    const editorBox = await box(editor);
    const detailsBox = await box(details);
    const summaryBox = await box(details.locator("summary"));
    expect(fillBox.height).toBeGreaterThanOrEqual(160);
    expect(editorBox.y + editorBox.height).toBeLessThanOrEqual(
      fillBox.y + fillBox.height + 1,
    );

    // Kasten und Klappbereich überschneiden sich nicht (y-Intervalle), die
    // Überschrift liegt unterhalb des Editors.
    expect(fillBox.y + fillBox.height).toBeLessThanOrEqual(detailsBox.y + 1);
    expect(summaryBox.y).toBeGreaterThanOrEqual(fillBox.y + fillBox.height - 1);

    // Höchstens ~40 % der Editorspalte (etwas Luft für Rahmen/Rundung).
    const column = await box(page.locator(".tts-editor"));
    expect(detailsBox.height).toBeLessThanOrEqual(column.height * 0.42);

    // Aufnahme nur auf Anforderung (SCREENS_DIR), nie im normalen Lauf --
    // sonst ueberschreibt jeder Testlauf versionierte PNGs.
    if (process.env.SCREENS_DIR) {
      await page.screenshot({
        path: path.resolve(
          process.env.SCREENS_DIR,
          `palette-klappbereich-${size.width}.png`,
        ),
        animations: "disabled",
      });
    }
  });
}

test("suchfeld and filter row stay visible while the palette scrolls", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1920, height: 1050 });
  const { details } = await openPaletteAlle(page);
  await details.evaluate((el) => {
    el.scrollTop = el.scrollHeight;
  });
  const search = details.getByPlaceholder("Tags durchsuchen…");
  await expect(search).toBeInViewport();
  const detailsBox = await box(details);
  const searchBox = await box(search);
  // Unterhalb der klebenden Überschrift, nicht darunter versteckt.
  const summaryBox = await box(details.locator("summary"));
  expect(searchBox.y).toBeGreaterThanOrEqual(
    summaryBox.y + summaryBox.height - 1,
  );
  expect(searchBox.y + searchBox.height).toBeLessThanOrEqual(
    detailsBox.y + detailsBox.height,
  );
});

test("clicking a tag still inserts it into the text", async ({ page }) => {
  await page.setViewportSize({ width: 1366, height: 768 });
  const { editor, details } = await openPaletteAlle(page);
  await editor.evaluate((el) => {
    (el as HTMLTextAreaElement).setSelectionRange(0, 0);
  });
  // Ein Chip weit unten in der Liste: erst hinscrollen, dann klicken.
  const tagButton = details.getByRole("button", { name: "Flüstern" }).first();
  await tagButton.scrollIntoViewIfNeeded();
  await tagButton.click();
  await expect(editor).toHaveValue(/\[[^\]]+\]/);
});

// P7 (Goal ui-vorlesen-kompakt): Palette folgt dem App-Standard und ist kompakt.
test("palette filter row is one line, neutral, and uses a scale-conform legend", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1366, height: 768 });
  const { details } = await openPaletteAlle(page);

  // Einzeilig: alle Filter liegen auf gleicher Hoehe (A24).
  const tabs = details.getByRole("tab");
  expect(await tabs.count()).toBeGreaterThanOrEqual(10);
  const tops = await tabs.evaluateAll((els) =>
    els.map((el) => el.getBoundingClientRect().top),
  );
  expect(Math.max(...tops) - Math.min(...tops)).toBeLessThanOrEqual(2);

  // Aktiver Filter ist neutral, nicht gelb (A11).
  const active = details.getByRole("tab", { name: "Alle" });
  const colors = await active.evaluate((el) => {
    const probe = document.createElement("span");
    probe.style.backgroundColor = "var(--color-logo-primary)";
    document.body.appendChild(probe);
    const accent = getComputedStyle(probe).backgroundColor;
    probe.remove();
    return { bg: getComputedStyle(el).backgroundColor, accent };
  });
  expect(colors.bg).not.toBe(colors.accent);

  // Keine Schrift unter 11 px in der Palette (A14/A15), Legende mit
  // Skalen-Zeilenhoehe.
  const smallest = await details.evaluate((root) => {
    let min = Infinity;
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    while (walker.nextNode()) {
      const node = walker.currentNode;
      if (!node.textContent?.trim() || !node.parentElement) continue;
      min = Math.min(
        min,
        parseFloat(getComputedStyle(node.parentElement).fontSize),
      );
    }
    return min;
  });
  expect(smallest).toBeGreaterThanOrEqual(11);
  const summarySize = await details
    .locator("summary")
    .evaluate((el) => parseFloat(getComputedStyle(el).fontSize));
  expect(summarySize).toBeCloseTo(13.125, 1);
  const legend = await details.getByTestId("tag-legend").evaluate((el) => {
    const style = getComputedStyle(el);
    return {
      size: parseFloat(style.fontSize),
      line: parseFloat(style.lineHeight),
    };
  });
  expect(legend.line).toBeLessThanOrEqual(legend.size * 1.5);

  // "Spezial" traegt nicht mehr das Sparkles-Symbol (A05).
  const special = details.getByRole("tab", { name: "Spezial" });
  await expect(special.locator("svg.lucide-sparkles")).toHaveCount(0);
  await expect(special.locator("svg")).toHaveCount(1);
});

test("palette tags are compact and the favorite star has a 24px hit area", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1366, height: 768 });
  const { details } = await openPaletteAlle(page);

  // Zeilenabstand der Tags (A23): y-Spruenge zwischen den Zeilen.
  const rowTops = await details
    .locator("button[title]:not([aria-label])")
    .evaluateAll((els) => {
      const tops = els.map((el) => Math.round(el.getBoundingClientRect().top));
      return [...new Set(tops)].sort((a, b) => a - b);
    });
  expect(rowTops.length).toBeGreaterThan(3);
  const gaps = rowTops.slice(1).map((y, i) => y - rowTops[i]);
  // Jeder Zeilensprung, der die Tag-Raster betrifft, bleibt kompakt.
  expect(Math.min(...gaps)).toBeGreaterThanOrEqual(24);
  expect(Math.max(...gaps.slice(0, 3))).toBeLessThanOrEqual(34);
  const chip = details.locator("button[title]:not([aria-label])").first();
  expect((await box(chip)).height).toBeGreaterThanOrEqual(24);

  // Favoriten-Stern: Klickflaeche mindestens 24x24 (A12), Symbol >= 12 px.
  const star = details
    .getByRole("button", { name: /Favorit|favorite/i })
    .first();
  await star.scrollIntoViewIfNeeded();
  const starBox = await box(star);
  expect(starBox.width).toBeGreaterThanOrEqual(24);
  expect(starBox.height).toBeGreaterThanOrEqual(24);
  const icon = await box(star.locator("svg"));
  expect(icon.width).toBeGreaterThanOrEqual(12);
});

// P9: Die einzeilige Filterleiste zeigt am Rand, dass waagrecht mehr kommt
// (weicher Verlauf), und nur solange dort noch Reiter verborgen sind.
test("palette filter row hints at hidden tabs with an edge fade", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1366, height: 768 });
  const { details } = await openPaletteAlle(page);
  const list = details.getByRole("tablist");
  const fadeStart = details.getByTestId("tag-tabs-fade-start");
  const fadeEnd = details.getByTestId("tag-tabs-fade-end");

  const overflowing = await list.evaluate(
    (el) => el.scrollWidth > el.clientWidth + 1,
  );
  expect(overflowing, "Leiste passt schon: kein Test moeglich").toBe(true);

  // Anfang: rechts Hinweis, links keiner.
  await expect(fadeEnd).toHaveCSS("opacity", "1");
  await expect(fadeStart).toHaveCSS("opacity", "0");
  // Der Hinweis fängt keine Klicks ab und verschiebt nichts.
  await expect(fadeEnd).toHaveCSS("pointer-events", "none");
  const listBox = await list.boundingBox();
  const endBox = await fadeEnd.boundingBox();
  expect(endBox!.x + endBox!.width).toBeLessThanOrEqual(
    listBox!.x + listBox!.width + 1,
  );

  // Ans Ende scrollen: rechts weg, links da.
  await list.evaluate((el) => {
    el.scrollLeft = el.scrollWidth;
  });
  await expect(fadeEnd).toHaveCSS("opacity", "0");
  await expect(fadeStart).toHaveCSS("opacity", "1");
  expect(
    await list.evaluate((el) => el.scrollLeft),
    "Leiste sitzt am Ende",
  ).toBeGreaterThan(0);
});

for (const size of [
  { width: 1920, height: 1050, file: "palette-kompakt-1920.png" },
  { width: 1366, height: 768, file: "palette-kompakt-1366.png" },
]) {
  test(`p7 screenshot palette alle ${size.width}x${size.height}`, async ({
    page,
  }) => {
    test.skip(
      !process.env.SCREENS_DIR,
      "nur mit SCREENS_DIR (Aufnahme, kein Verhaltenstest)",
    );
    await page.setViewportSize(size);
    await openPaletteAlle(page);
    await page.screenshot({
      path: path.resolve(process.env.SCREENS_DIR!, size.file),
      animations: "disabled",
    });
    await expect(page.getByRole("tab", { name: "Alle" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });
}
