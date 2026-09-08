import { test, expect } from "@playwright/test";

// Die Stimmenliste mit echten Eintraegen: eine Stimme mit vorhandener
// Hoerprobe, eine ohne. Der Unterschied ist der Punkt — die eine ist sofort
// hoerbar, die andere kostet einen Start der Sprach-Engine, und das muss man
// sehen, bevor man klickt.
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
          if (cmd === "pages_list")
            return [{ id: "p1", title: "Der Sturm" }];
          if (cmd === "page_files")
            return [
              { name: "Der-Sturm_2026-09-08_1405.wav", size: 120, modified_ms: 0 },
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
            (window as unknown as { savedPiperVoice?: unknown }).savedPiperVoice =
              args?.value;
            return null;
          }
          if (cmd === "get_custom_sounds") return { start: false, stop: false };
          return null;
        },
      },
    });
  });
});

async function openVoices(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await page
    .getByText("Stimmen anhören & verwalten")
    .click();
}

test("a voice with a stored sample plays without starting the engine", async ({
  page,
}) => {
  await openVoices(page);

  const row = page.locator(".py-1", { hasText: "Erzählerin" }).first();
  // Der Player steht da, ohne dass jemand geklickt hat.
  await expect(row.locator("audio")).toHaveCount(1);
  await expect(row.getByText("Probesatz.")).toBeVisible();
  // Und kein Knopf, der eine Erzeugung anbietet.
  await expect(
    row.getByRole("button", { name: "Hörprobe erzeugen" }),
  ).toHaveCount(0);
});

test("a voice without a sample says what the click will cost", async ({
  page,
}) => {
  await openVoices(page);

  const row = page.locator(".py-1", { hasText: "Leo Lausemaus" }).first();
  await expect(row.locator("audio")).toHaveCount(0);
  await expect(
    row.getByRole("button", { name: "Hörprobe erzeugen" }),
  ).toBeVisible();
  await expect(row.getByText(/startet einmalig die Sprach-Engine/)).toBeVisible();
});

test("a generated recording can be played from the file list", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Vorlesen", exact: true }).click();

  const audioRow = page
    .locator("div", { hasText: /^Der-Sturm_2026-09-08_1405\.wav/ })
    .last();
  const listen = audioRow.getByRole("button", { name: "Anhören" });
  await expect(listen).toBeVisible();

  // Der Player entsteht erst auf Wunsch — sonst laege unter jeder Datei einer.
  const filesArea = page.locator(".tts-workspace__files");
  await expect(filesArea.locator("audio")).toHaveCount(0);
  await listen.click();
  await expect(filesArea.locator("audio")).toHaveCount(1);

  // Eine Textdatei bekommt keinen Abspielknopf.
  const textRow = page.locator("div", { hasText: /^notizen\.txt/ }).last();
  await expect(textRow.getByRole("button", { name: "Anhören" })).toHaveCount(0);
});

test("a recording carries its origin and hands the text back to the editor", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Vorlesen", exact: true }).click();

  const filesArea = page.locator(".tts-workspace__files");
  await filesArea.getByRole("button", { name: "Anhören" }).click();

  // Wer sie gesprochen hat und woraus sie entstand.
  await expect(
    filesArea.getByText("Es war eine dunkle Nacht.").first(),
  ).toBeVisible();
  await expect(filesArea.getByText(/erzaehlerin/).first()).toBeVisible();

  // Und der Text kommt zurueck in den Editor — Grundlage jeder Korrektur.
  const editor = page.getByPlaceholder("Text zum Vorlesen eingeben oder einfügen…");
  await expect(editor).toHaveValue("");
  await filesArea.getByRole("button", { name: "Text übernehmen" }).click();
  await expect(editor).toHaveValue("Es war eine dunkle Nacht. Niemand sprach.");
});

test("the spoken line is highlighted while the recording plays", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Vorlesen", exact: true }).click();

  const filesArea = page.locator(".tts-workspace__files");
  await filesArea.getByRole("button", { name: "Anhören" }).click();

  const first = filesArea.getByRole("listitem").filter({
    hasText: "Es war eine dunkle Nacht.",
  });
  const second = filesArea.getByRole("listitem").filter({
    hasText: "Niemand sprach.",
  });

  // Am Anfang steht der erste Satz — der zweite noch nicht.
  await expect(first).toHaveAttribute("aria-current", "true");
  await expect(second).not.toHaveAttribute("aria-current", "true");

  // Der Sprecher steht bei seinem Satz: ein Hoerspiel wechselt sie.
  await expect(second).toContainText("leo-lausemaus");

  // Drei Sekunden hinein gehoert die Hervorhebung dem zweiten Satz. Die
  // Audiodatei selbst gibt es im Test nicht, deshalb wird das Ereignis
  // gesendet, das ihre Wiedergabe ausloesen wuerde.
  await filesArea.locator("audio").evaluate((element) => {
    Object.defineProperty(element, "currentTime", { value: 3, writable: true });
    element.dispatchEvent(new Event("timeupdate", { bubbles: true }));
  });
  await expect(second).toHaveAttribute("aria-current", "true");
  await expect(first).not.toHaveAttribute("aria-current", "true");
});

test("the end of the read-aloud page can actually be reached", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 520 });
  await page.goto("/");
  await page.getByRole("button", { name: "Vorlesen", exact: true }).click();
  await page.getByText("Stimmen anhören & verwalten").click();

  // Ans Ende scrollen wie ein Mensch mit dem Mausrad: bis der Container
  // nicht mehr weiter kann.
  const main = page.getByRole("main");
  await main.evaluate((element) => {
    element.scrollTop = element.scrollHeight;
  });

  // Der Stimmwechsler ist das letzte Element der Seite. Er muss danach
  // vollstaendig sichtbar sein und darf nicht unter der Statusleiste liegen.
  const last = page.getByText("Stimmwechsler", { exact: true });
  await expect(last).toBeInViewport();
  const box = (await last.boundingBox())!;
  const viewport = page.viewportSize()!;
  expect(box.y + box.height).toBeLessThanOrEqual(viewport.height);

  // Und der Container ist wirklich am Ende — sonst haette das Scrollen
  // vorzeitig gestoppt.
  const rest = await main.evaluate(
    (element) => element.scrollHeight - element.scrollTop - element.clientHeight,
  );
  expect(rest).toBeLessThanOrEqual(1);
});

test("the wheel scrolls the page even when it sits over a player", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 520 });
  await page.goto("/");
  await page.getByRole("button", { name: "Vorlesen", exact: true }).click();
  await page.getByText("Stimmen anhören & verwalten").click();

  const main = page.getByRole("main");
  const slider = page.locator('input[type="range"]:visible').first();
  await slider.scrollIntoViewIfNeeded();
  const before = await main.evaluate((element) => element.scrollTop);
  const value = await slider.inputValue();

  await slider.hover();
  await page.mouse.wheel(0, 400);
  await page.waitForTimeout(200);

  const after = await main.evaluate((element) => element.scrollTop);
  expect(after, "Rad ueber dem Regler muss die Seite scrollen").toBeGreaterThan(
    before,
  );
  expect(await slider.inputValue(), "und den Regler nicht verstellen").toBe(
    value,
  );
});

test("the read-aloud engine can be switched to Piper and the choice is saved", async ({
  page,
}) => {
  await page.goto("/");
  await page
    .getByRole("button", { name: "Einstellungen", exact: true })
    .last()
    .click();
  await page.getByRole("tab", { name: "Vorlesen", exact: true }).click();

  const engine = page.getByText("Engine fürs Vorlesen", { exact: true });
  await engine.scrollIntoViewIfNeeded();
  await expect(engine).toBeVisible();

  // Solange Fish laeuft, ist die Piper-Stimme kein Thema.
  await expect(page.getByText("Piper-Stimme", { exact: true })).toHaveCount(0);

  const select = page.locator(".w-48").first();
  await select.click();
  await page.getByText("Piper (CPU, sofort bereit)").click();

  // Gespeichert, nicht nur angezeigt.
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { savedEngine?: string }).savedEngine,
      ),
    )
    .toBe("piper");

  // Und erst jetzt fragt die Oberflaeche nach der Stimme.
  await expect(page.getByText("Piper-Stimme", { exact: true })).toBeVisible();

  // Zur Auswahl steht genau die geladene Stimme — eine noch nicht geladene
  // waere ein Versprechen, das die Wiedergabe nicht halten kann.
  const voiceSelect = page.locator(".w-48").nth(1);
  await voiceSelect.click();
  await expect(page.getByText("Thorsten (Deutsch)")).toBeVisible();
  await expect(page.getByText("Amy (English)")).toHaveCount(0);
});

test("the menu collapses to icons and stays that way", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto("/");

  const nav = page.locator(".workspace-nav");
  const label = page.getByRole("button", { name: "Vorlesen", exact: true });

  // Beim ersten Start offen: wer die App nicht kennt, soll lesen koennen.
  await expect(label).toBeVisible();
  const wide = (await nav.boundingBox())!.width;

  await page.getByRole("button", { name: "Menü einklappen" }).click();
  await expect(nav).toHaveAttribute("data-collapsed", "true");
  const narrow = (await nav.boundingBox())!.width;
  expect(narrow).toBeLessThan(wide);
  // Sichtbar bleibt nur das Symbol — der Name aber erreichbar: als Tooltip
  // und fuer Screenreader. Ein Menue aus stummen Bildchen waere kein
  // Fortschritt.
  await expect(label.locator("span")).toBeHidden();
  await expect(label).toBeVisible();
  await expect(label).toHaveAttribute("title", "Vorlesen");

  // Und der Zustand ueberlebt den Neustart.
  await page.reload();
  await expect(page.locator(".workspace-nav")).toHaveAttribute(
    "data-collapsed",
    "true",
  );

  await page.getByRole("button", { name: "Menü ausklappen" }).click();
  await expect(page.locator(".workspace-nav")).toHaveAttribute(
    "data-collapsed",
    "false",
  );
});

test("the sidebar no longer repeats the logo", async ({ page }) => {
  await page.goto("/");
  // Die Kopfzeile des Fensters traegt das Logo bereits; ein zweites kostete
  // nur die oberste Zeile des Menues.
  await expect(page.locator(".workspace-nav__brand")).toHaveCount(0);
});
