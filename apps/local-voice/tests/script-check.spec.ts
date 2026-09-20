import { test, expect } from "@playwright/test";

// Skript-Pruefung im Vorlesen-Editor. Die Tauri-Bruecke ist dieselbe
// Attrappe wie in voices.spec.ts (zwei benannte Stimmen plus achtzehn
// generische); zusaetzlich merkt sie sich, ob und womit vorgelesen wurde.
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
            return settings;
          if (cmd === "plugin:os|locale") return "de-DE";
          if (cmd === "plugin:app|version") return "0.16.0";
          if (cmd.includes("permission")) return true;
          if (cmd === "plugin:event|listen") return ++callback;
          if (cmd === "get_selected_model") return "";
          if (cmd === "meetings_is_recording") return false;
          if (cmd === "tts_server_status")
            return { phase: "stopped", message: null };
          if (cmd === "tts_list_voice_infos") return voices;
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

const SCRIPT =
  "<Erzählerin> Es war einmal.\n<Bob> Wer bin ich?\n[mysterious] Ein Tag, das Fish nicht kennt.\n<Bob:leise> Und nochmal Bob.";

async function openEditorWithScript(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  const editor = page.locator("textarea").first();
  await editor.fill(SCRIPT);
  return editor;
}

test("an unknown speaker and an unknown tag show up as findings", async ({
  page,
}) => {
  await openEditorWithScript(page);
  const panel = page.getByTestId("script-check");
  await expect(panel).toContainText("3 Befunde im Skript");
  const rows = page.getByTestId("script-finding");
  await expect(rows).toHaveCount(3);
  await expect(rows.nth(0)).toContainText("Zeile 2:");
  await expect(rows.nth(0)).toContainText("Sprecher „Bob“ hat keine Stimme");
  await expect(rows.nth(1)).toContainText("Zeile 3:");
  await expect(rows.nth(1)).toContainText("Tag „mysterious“");
  await expect(rows.nth(2)).toContainText("Zeile 4:");
  // Die bekannte Erzaehlerin ist kein Befund. Und im Text selbst sind die
  // Stellen unterstrichen plus je eine Marke am Rand.
  await expect(
    rows.getByRole("button", { name: /Sprecher „Erzählerin“/ }),
  ).toHaveCount(0);
  await expect(page.locator("[data-finding]")).toHaveCount(3);
  await expect(page.locator("[data-finding-mark]")).toHaveCount(3);
});

test("clicking a finding jumps to the spot and selects it", async ({
  page,
}) => {
  const editor = await openEditorWithScript(page);
  await page
    .getByTestId("script-finding")
    .nth(1)
    .getByRole("button", { name: /Tag „mysterious“/ })
    .click();
  await expect(editor).toBeFocused();
  const start = SCRIPT.indexOf("[mysterious]");
  expect(
    await editor.evaluate((el) => [
      (el as HTMLTextAreaElement).selectionStart,
      (el as HTMLTextAreaElement).selectionEnd,
    ]),
  ).toEqual([start, start + "[mysterious]".length]);
});

test("replace everywhere swaps every occurrence and keeps the style", async ({
  page,
}) => {
  const editor = await openEditorWithScript(page);
  const bobRow = page.getByTestId("script-finding").nth(0);
  await bobRow.getByTestId("script-finding-replacement").click();
  await page.getByText("Leo Lausemaus", { exact: true }).click();
  await bobRow.getByRole("button", { name: "Überall im Text" }).click();
  await expect(editor).toHaveValue(
    "<Erzählerin> Es war einmal.\n<Leo Lausemaus> Wer bin ich?\n[mysterious] Ein Tag, das Fish nicht kennt.\n<Leo Lausemaus:leise> Und nochmal Bob.",
  );
  await expect(page.getByTestId("script-check")).toContainText(
    "1 Befund im Skript",
  );
  // Das Tag hier entfernen — danach ist das Skript sauber.
  await page
    .getByTestId("script-finding")
    .first()
    .getByRole("button", { name: "Entfernen hier" })
    .click();
  await expect(editor).toHaveValue(
    "<Erzählerin> Es war einmal.\n<Leo Lausemaus> Wer bin ich?\n Ein Tag, das Fish nicht kennt.\n<Leo Lausemaus:leise> Und nochmal Bob.",
  );
  await expect(page.getByTestId("script-check-clean")).toBeVisible();
});

test("reading with findings asks first and then reads anyway", async ({
  page,
}) => {
  await openEditorWithScript(page);
  await page
    .getByRole("button", { name: "Vorlesen", exact: true })
    .last()
    .click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("3 Befunde – trotzdem fortfahren?");
  // Nichts wurde geschickt, solange der Dialog offen ist.
  expect(
    await page.evaluate(
      () => (window as unknown as { spokenTexts?: unknown[] }).spokenTexts,
    ),
  ).toBeUndefined();
  await page.getByTestId("script-check-proceed").click();
  await expect(dialog).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { spokenTexts?: unknown[] }).spokenTexts,
      ),
    )
    .toEqual([SCRIPT]);
});
