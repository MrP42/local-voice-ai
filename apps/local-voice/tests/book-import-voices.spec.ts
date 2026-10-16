import { test, expect, type Page } from "@playwright/test";
import { join } from "node:path";

// Belege (optional): LV_SHOT_DIR setzen, sonst Testausgabe-Ordner.
const shot = (name: string) =>
  join(process.env.LV_SHOT_DIR ?? "test-results", name);

// Seit der kompakten Bedienspalte wohnen Skript pruefen, Werkstatt und
// Auto-Tagging hinter dem Menue (Hamburger): erst oeffnen, dann waehlen.
async function pickMenuItem(page: Page, testId: string) {
  await page.getByTestId("tts-action-menu").click();
  await page.getByTestId(testId).click();
}

// Buchimport in der Skript-Werkstatt: die Rueckfrage "Stimmen mit einspielen?"
// ist ein eigener Dialog (kein window.confirm). Tauri-Bruecke wie in
// script-check.spec.ts, ergaenzt um Dateiauswahl, books_inspect, books_import.
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
          if (cmd === "plugin:dialog|open") return "C:/buch/drache.lvbook";
          if (cmd === "books_inspect")
            return {
              title: "Drache",
              pages: 2,
              voices: [
                { id: "neu-1", display_name: "Neue Stimme", present: false },
              ],
              rights_confirmed: true,
            };
          if (cmd === "books_import") {
            (window as unknown as { __imports?: unknown[] }).__imports = [
              ...((window as unknown as { __imports?: unknown[] }).__imports ??
                []),
              args,
            ];
            return {
              id: "book_imp",
              title: "Drache",
              created_ms: 1,
              page_ids: [],
              characters: [],
              language: "de",
            };
          }
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

const imports = (page: Page) =>
  page.evaluate(
    () => (window as unknown as { __imports?: unknown[] }).__imports ?? [],
  );

async function openAsk(page: Page) {
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await pickMenuItem(page, "workshop-open");
  const outer = page.getByRole("dialog", { name: "Skript-Werkstatt" });
  await expect(outer).toBeVisible();
  await outer.getByRole("button", { name: "Buch einspielen…" }).click();
  const ask = page.getByRole("dialog", { name: "Stimmen mit einspielen?" });
  await expect(ask).toBeVisible();
  return { outer, ask };
}

test("book import asks about voices in a dialog and imports with voices", async ({
  page,
}) => {
  const { ask } = await openAsk(page);
  await expect(ask).toContainText("1 neue Stimme");
  await expect(
    ask.getByRole("button", { name: "Mit Stimmen einspielen" }),
  ).toBeVisible();
  await expect(
    ask.getByRole("button", { name: "Ohne Stimmen importieren" }),
  ).toBeVisible();
  await expect(ask.getByRole("button", { name: "Abbrechen" })).toBeVisible();
  await page.screenshot({ path: shot("buchimport-frage.png") });
  await ask.getByRole("button", { name: "Mit Stimmen einspielen" }).click();
  await expect(ask).toHaveCount(0);
  await expect.poll(() => imports(page)).toHaveLength(1);
  expect((await imports(page))[0]).toMatchObject({ importVoices: true });
});

test("book import without voices", async ({ page }) => {
  const { ask } = await openAsk(page);
  await ask.getByRole("button", { name: "Ohne Stimmen importieren" }).click();
  await expect.poll(() => imports(page)).toHaveLength(1);
  expect((await imports(page))[0]).toMatchObject({ importVoices: false });
});

test("cancel aborts the import and keeps the workshop open", async ({
  page,
}) => {
  const { outer, ask } = await openAsk(page);
  await ask.getByRole("button", { name: "Abbrechen" }).click();
  await expect(ask).toHaveCount(0);
  await expect(outer).toBeVisible();
  expect(await imports(page)).toHaveLength(0);
});

test("Escape closes only the inner dialog; focus is visible by keyboard", async ({
  page,
}) => {
  const { outer, ask } = await openAsk(page);
  // Tastaturfokus liegt im Dialog und ist sichtbar (Fokusring).
  const focused = page.locator(":focus");
  // Der Fokus zieht beim Oeffnen in den inneren Dialog (nach einem Frame).
  await expect
    .poll(() =>
      page.evaluate(() =>
        Boolean(
          document.activeElement
            ?.closest('[role="dialog"]')
            ?.textContent?.includes("Stimmen mit einspielen?"),
        ),
      ),
    )
    .toBe(true);
  await page.keyboard.press("Tab");
  const ring = await focused.evaluate((el) => {
    const s = getComputedStyle(el);
    return {
      outline: s.outlineStyle !== "none" && parseFloat(s.outlineWidth) > 0,
      shadow: s.boxShadow !== "none",
    };
  });
  expect(ring.outline || ring.shadow).toBe(true);
  await page.screenshot({ path: shot("buchimport-fokus.png") });
  await page.keyboard.press("Escape");
  await expect(ask).toHaveCount(0);
  await expect(outer).toBeVisible();
  expect(await imports(page)).toHaveLength(0);
});
