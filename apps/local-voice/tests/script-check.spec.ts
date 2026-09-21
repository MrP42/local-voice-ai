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
            return {
              ...settings,
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
  // Unterstreichung kommt von selbst (120 ms Debounce); das Korrekturpanel
  // erst per Knopf -- wie in Office.
  await page.getByTestId("script-check-run").click();
  return editor;
}

test("findings are grouped: one problem with all its spots at a time", async ({
  page,
}) => {
  await openEditorWithScript(page);
  const panel = page.getByTestId("script-check");
  // Drei Stellen, aber nur zwei Probleme: Bob (2x) und mysterious (1x).
  await expect(panel).toContainText("3 Befunde im Skript");
  await expect(panel).toContainText("2 Gruppen");
  await expect(panel.getByTestId("script-check-group-position")).toHaveText(
    "1 von 2",
  );
  const card = page.getByTestId("script-finding");
  await expect(card).toHaveCount(1);
  await expect(card).toContainText("Sprecher „Bob“ (2 Stellen)");
  await expect(card).toContainText("Stelle 1 von 2");
  await expect(card.getByRole("button", { name: "Z. 2" })).toBeVisible();
  await expect(card.getByRole("button", { name: "Z. 4" })).toBeVisible();
  // Die bekannte Erzaehlerin ist kein Befund; im Text sind alle drei
  // Stellen unterstrichen plus je eine Marke am Rand.
  await expect(panel).not.toContainText("Erzählerin");
  await expect(page.locator("[data-finding]")).toHaveCount(3);
  await expect(page.locator("[data-finding-mark]")).toHaveCount(3);
  // Naechste Gruppe: das Tag.
  await panel.getByRole("button", { name: "Nächste Gruppe" }).click();
  await expect(card).toContainText("Tag „mysterious“ (1 Stelle)");
});

test("clicking a spot jumps to it and selects it", async ({ page }) => {
  const editor = await openEditorWithScript(page);
  const panel = page.getByTestId("script-check");
  await panel.getByRole("button", { name: "Nächste Gruppe" }).click();
  await page
    .getByTestId("script-finding")
    .getByRole("button", { name: "Z. 3" })
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

test("replace all swaps every spot of the group and keeps the style", async ({
  page,
}) => {
  const editor = await openEditorWithScript(page);
  const card = page.getByTestId("script-finding");
  await card.getByTestId("script-finding-replacement").click();
  await page.getByText("Leo Lausemaus", { exact: true }).click();
  await card.getByRole("button", { name: "Alle 2 ersetzen" }).click();
  await expect(editor).toHaveValue(
    "<Erzählerin> Es war einmal.\n<Leo Lausemaus> Wer bin ich?\n[mysterious] Ein Tag, das Fish nicht kennt.\n<Leo Lausemaus:leise> Und nochmal Bob.",
  );
  await expect(page.getByTestId("script-check")).toContainText(
    "1 Befund im Skript",
  );
  // Das Tag entfernen (einzige Stelle) — danach ist das Skript sauber.
  await card.getByRole("button", { name: "Nur diese entfernen" }).click();
  await expect(editor).toHaveValue(
    "<Erzählerin> Es war einmal.\n<Leo Lausemaus> Wer bin ich?\n Ein Tag, das Fish nicht kennt.\n<Leo Lausemaus:leise> Und nochmal Bob.",
  );
  // Sauber: das Panel bleibt offen und meldet es, keine Unterstreichung,
  // kein Zaehler am Knopf.
  await expect(page.getByTestId("script-check-clean")).toBeVisible();
  await expect(page.locator("[data-finding]")).toHaveCount(0);
  await expect(page.getByTestId("script-check-badge")).toHaveCount(0);
});

test("a recommendation fixes all spots with one click", async ({ page }) => {
  const editor = await openEditorWithScript(page);
  // "Erzahlerin" (Tippfehler, ohne Umlaut) -> Empfehlung "Erzählerin".
  await editor.fill(
    "<Erzahlerin> Hallo.\n<Erzahlerin:leise> Psst.\n[calm] Ruhig.",
  );
  await page.getByTestId("script-check-run").click();
  const card = page.getByTestId("script-finding");
  await expect(card).toContainText("Sprecher „Erzahlerin“ (2 Stellen)");
  const rec = card.getByTestId("script-finding-recommendation");
  await expect(rec.first()).toHaveText("Erzählerin");
  await rec.first().click();
  await expect(editor).toHaveValue(
    "<Erzählerin> Hallo.\n<Erzählerin:leise> Psst.\n[calm] Ruhig.",
  );
  // "calm" ist ein Alias von "relaxed": das wird empfohlen, nicht die
  // ganze Liste.
  await expect(card).toContainText("Tag „calm“ (1 Stelle)");
  await expect(
    card.getByTestId("script-finding-recommendation").first(),
  ).toContainText("[relaxed]");
});

test("the check button runs even when the automatic check is off", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as unknown as { __lvScriptCheckOff: boolean }).__lvScriptCheckOff =
      true;
  });
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  const editor = page.locator("textarea").first();
  await editor.fill(SCRIPT);
  // Automatik aus: keine Unterstreichung, kein Panel, obwohl Befunde da sind.
  await expect(page.getByTestId("script-check")).toHaveCount(0);
  await expect(page.locator("[data-finding]")).toHaveCount(0);
  await page.getByTestId("script-check-run").click();
  await expect(page.getByTestId("script-check")).toContainText(
    "3 Befunde im Skript",
  );
  await expect(page.locator("[data-finding]")).toHaveCount(3);
  await expect(editor).toBeFocused();
});

test("with automatic check on, the text is underlined before any click", async ({
  page,
}) => {
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  const editor = page.locator("textarea").first();
  await editor.fill(SCRIPT);
  await expect(page.locator("[data-finding]")).toHaveCount(3);
  await expect(page.getByTestId("script-check-badge")).toHaveText("3");
  // Kein Panel, bis man es anfordert; Schliessen nimmt es wieder weg.
  await expect(page.getByTestId("script-check")).toHaveCount(0);
  await page.getByTestId("script-check-run").click();
  await expect(page.getByTestId("script-check")).toBeVisible();
  await page.getByTestId("script-check-close").click();
  await expect(page.getByTestId("script-check")).toHaveCount(0);
  await expect(page.locator("[data-finding]")).toHaveCount(3);
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

test("history: undo and redo cover typing and replace-all", async ({
  page,
}) => {
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  const editor = page.locator("textarea").first();
  await editor.fill("<Bob> Hallo Bob.\n<Bob:leise> Psst.");
  // Ueberall ersetzen aus dem Kontextmenue: Wort unter dem Caret.
  await editor.click();
  await editor.evaluate((el) => {
    const ta = el as HTMLTextAreaElement;
    const pos = ta.value.indexOf("Bob.");
    ta.setSelectionRange(pos, pos);
  });
  await editor.dispatchEvent("contextmenu", { clientX: 200, clientY: 200 });
  // Per JS klicken: Playwrights Scroll-ins-Bild loest das Scroll-Ereignis
  // aus, auf das das Menue mit Schliessen reagiert.
  await page
    .getByTestId("menu-replace-all")
    .evaluate((el) => (el as HTMLButtonElement).click());
  await expect(page.getByTestId("replace-all-count")).toContainText(
    "3 Treffer",
  );
  await page.getByTestId("replace-all-replacement").fill("Leo");
  await page.getByTestId("replace-all-run").click();
  await expect(editor).toHaveValue("<Leo> Hallo Leo.\n<Leo:leise> Psst.");
  // Historie: ein Schritt zurueck, einer vor -- per Knopf und per Tastatur.
  await page.getByTestId("history-undo").click();
  await expect(editor).toHaveValue("<Bob> Hallo Bob.\n<Bob:leise> Psst.");
  await page.getByTestId("history-redo").click();
  await expect(editor).toHaveValue("<Leo> Hallo Leo.\n<Leo:leise> Psst.");
  await editor.press("Control+z");
  await expect(editor).toHaveValue("<Bob> Hallo Bob.\n<Bob:leise> Psst.");
  await editor.press("Control+y");
  await expect(editor).toHaveValue("<Leo> Hallo Leo.\n<Leo:leise> Psst.");
});

test("auto-tagging opens its dialog first", async ({ page }) => {
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await page.locator("textarea").first().fill("Ein Satz.");
  await page.getByTestId("autotag-open").click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("Auto-Tagging");
  await expect(dialog.getByText("Sparsam")).toBeVisible();
  await expect(page.getByTestId("autotag-run")).toBeEnabled();
});

test("the context menu reads from here or only this sentence", async ({
  page,
}) => {
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  const editor = page.locator("textarea").first();
  await editor.fill("Erster Satz hier. Zweiter Satz da. Dritter Satz dort.");
  await editor.click();
  await editor.evaluate((el) => {
    const ta = el as HTMLTextAreaElement;
    const pos = ta.value.indexOf("Zweiter") + 3;
    ta.setSelectionRange(pos, pos);
  });
  await editor.dispatchEvent("contextmenu", { clientX: 200, clientY: 200 });
  await page
    .getByTestId("menu-speak-one")
    .evaluate((el) => (el as HTMLButtonElement).click());
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { spokenFrom?: unknown[] }).spokenFrom,
      ),
    )
    .toEqual([
      {
        text: "Erster Satz hier. Zweiter Satz da. Dritter Satz dort.",
        charOffset: "Erster Satz hier. Zweiter Satz da.".indexOf("Zweiter") + 3,
        onlyOne: true,
      },
    ]);
  await editor.dispatchEvent("contextmenu", { clientX: 200, clientY: 200 });
  await page
    .getByTestId("menu-speak-from")
    .evaluate((el) => (el as HTMLButtonElement).click());
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as { spokenFrom?: { onlyOne: boolean }[] })
            .spokenFrom?.length,
      ),
    )
    .toBe(2);
});
