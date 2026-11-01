import { test, expect, type Page } from "@playwright/test";

// Vorlesen mit mehreren Sprechern über einen Anbieter (Phonon zuerst, jeder Anbieter
// mit `voice_switching`): die Stimmen des Anbieters sind die Sprecher im Skript --
// Dropdown "Skript mit Sprechern", Sprecher einfügen (Name, weiblich/männlich),
// Skript-Prüfung, Sprecher tauschen. Stil-Tags `[…]` gibt es dort nicht (Phonon
// setzt sie nicht um): die Palette bietet nur Pausen an, Auto-Tagging ist aus.
// Fish bleibt, wie es war (Regression am Ende).
//
// Die Attrappe führt `tts_provider_speakers` wie das Backend: dieselben Namen, die der
// Parser schaltet. `calls` zeigt, was die Oberfläche wirklich angefragt hat.

type Cfg = {
  engine: string;
  /** Gewählte Stimme des Anbieters (leer = Skript mit Sprechern). */
  chosen?: string | null;
  /** Der Anbieter wechselt die Stimme je Satz (Vorgabe: ja). */
  switching?: boolean;
  /** Zwei Seiten; die zweite hat (alt) eine eigene Stimme gespeichert. */
  twoPages?: boolean;
  /** Seite 2 enthält einen Sprecher-Marker (<Nele>). */
  markers?: boolean;
};

async function setup(page: Page, cfg: Cfg) {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.setViewportSize({ width: 1280, height: 1200 });
  await page.addInitScript((cfg: Cfg) => {
    const callbacks = new Map<number, unknown>();
    let callback = 0;
    const w = window as unknown as Record<string, unknown>;
    w.__calls = [] as unknown[];
    const switching = cfg.switching !== false;
    const settings: Record<string, unknown> = {
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
      tts_engine: cfg.engine,
      tts_voice: null,
      tts_piper_voice: null,
      tts_translate_lang: "English",
      tts_script_check: true,
      tts_providers: {
        enabled: ["phonon"],
        license_acks: [],
        allow_remote: [],
        voices: cfg.chosen ? { phonon: cfg.chosen } : {},
        visible_voices: {},
      },
    };
    const voice = (id: string, name: string, language: string) => ({
      id,
      meta: { display_name: name, color: "teal", language, tags: [] },
      origin: "recorded",
      avatar_path: null,
    });
    // Fish-Stimmen: unter Phonon dürfen sie KEINE Sprecher im Skript sein.
    const fish = [
      voice("erzaehlerin", "Erzählerin", "de-DE"),
      voice("leo", "Leo", "de-DE"),
    ];
    const phononVoice = (id: string, label: string, gender: string | null) => ({
      provider_id: "phonon",
      provider_label: "Phonon",
      voice_switching: switching,
      voice: { id, label, language: "de", gender, license: null },
    });
    const options = [
      phononVoice("default", "Standardstimme des Modells", null),
      phononVoice("nele-id", "Nele – Rounded, open tone", "female"),
      phononVoice("anton-id", "Anton – Warm baritone", "male"),
    ];
    // Die Sprecher, die der Parser kennt: ohne Standardstimme, Name vor " – ".
    const speakers = switching
      ? [
          {
            id: "nele-id",
            name: "Nele",
            aliases: ["Nele – Rounded, open tone"],
            gender: "female",
            language: "de",
          },
          {
            id: "anton-id",
            name: "Anton",
            aliases: ["Anton – Warm baritone"],
            gender: "male",
            language: "de",
          },
        ]
      : [];
    const caps = (styleTags: boolean, voiceSwitching: boolean) => ({
      languages: ["de"],
      streaming: "sentence",
      formats: ["wav"],
      sample_rates: [24000],
      cloning: false,
      devices: ["cpu"],
      platforms: ["windows-x64"],
      style_tags: styleTags,
      voice_switching: voiceSwitching,
    });
    const entry = (id: string, label: string, c: unknown) => ({
      descriptor: { id, label, tier: "experimental", opt_in: true, caps: c },
    });
    const listeners: Record<string, number[]> = {};
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
          if (cmd === "plugin:app|version") return "0.21.22";
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
          if (cmd === "tts_list_voice_infos")
            return JSON.parse(JSON.stringify(fish));
          if (cmd === "tts_list_voices") return fish.map((v) => v.id);
          if (cmd === "tts_providers_list")
            return [
              entry("fish", "Fish Speech", caps(true, true)),
              entry("phonon", "Phonon", caps(false, switching)),
            ];
          if (cmd === "tts_provider_voice_options") return options;
          // Hoerproben: Standardstimme und Nele fertig, Anton entsteht noch.
          if (cmd === "tts_provider_voice_samples")
            return [
              { voice_id: "default", wav_path: "C:/proben/default.wav" },
              { voice_id: "nele-id", wav_path: "C:/proben/nele.wav" },
              { voice_id: "anton-id", wav_path: null },
            ];
          if (cmd === "tts_voice_demo_cached") return null;
          if (cmd === "tts_voice_demo")
            return {
              wav_path: `C:/proben/${String(args?.voiceId)}-demo.wav`,
              transcript: "Probe",
            };
          if (cmd === "tts_provider_speakers")
            return cfg.engine === "provider:phonon" ? speakers : [];
          // Wie `select_in`: Engine und Stimme werden gespeichert.
          if (cmd === "tts_provider_select") {
            const voices = (settings.tts_providers as { voices: object })
              .voices as Record<string, string>;
            settings.tts_engine = `provider:${String(args?.id)}`;
            if (args?.voice) voices[String(args?.id)] = String(args?.voice);
            else delete voices[String(args?.id)];
            return null;
          }
          if (cmd === "pages_list")
            return cfg.twoPages
              ? [
                  { id: "p1", title: "Podcast" },
                  { id: "p2", title: "Hörbuch" },
                ]
              : [{ id: "p1", title: "Podcast" }];
          if (cmd === "page_state_load")
            return args?.id === "p2"
              ? JSON.stringify({
                  text: cfg.markers ? "<Nele> Zweite Seite." : "Zweite Seite.",
                  tab: "original",
                  voices: { original: "provider:phonon:anton-id" },
                })
              : null;
          if (cmd === "page_dir") return "C:\\projects\\p1";
          if (cmd === "tts_translate")
            return "<Nele> Good day.\n<Anton> Hello everyone.";
          if (cmd === "tts_cached_translation") return null;
          if (cmd === "tts_list_downloads") return [];
          if (
            cmd === "llm_ps" ||
            cmd === "tts_reading_list" ||
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
  }, cfg);
}

const calls = (page: Page, cmd?: string) =>
  page.evaluate(
    (name) =>
      (window as unknown as { __calls: [string, unknown][] }).__calls.filter(
        ([c]) => !name || c === name,
      ),
    cmd,
  );

async function gotoReadAloud(page: Page) {
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  const select = page.getByTestId("voice-select");
  await expect(select).toBeVisible();
  return select;
}

const editorOf = (page: Page) => page.locator("textarea").first();

/** Der aufklappbare Block "Schreibregeln: Sprecher & Tags" (nicht der Hilfetext rechts). */
const writingRules = (page: Page) =>
  page.locator("details", {
    has: page.locator("summary", { hasText: "Schreibregeln" }),
  });

/** Kontextmenü an der Schreibmarke öffnen (Menü schließt bei Scroll: per JS klicken). */
async function openSpeakerMenu(page: Page) {
  const editor = editorOf(page);
  await editor.click();
  await editor.dispatchEvent("contextmenu", { clientX: 200, clientY: 200 });
  await page
    .getByRole("menuitem", { name: "Sprecher einfügen" })
    .evaluate((el) => (el as HTMLButtonElement).click());
}

/** Kontextmenü öffnen und „Tag einfügen“ aufklappen (die Palette unter dem Text
 *  gibt es seit lva#148 nicht mehr; Tags kommen über das Kontextmenü). */
async function openTagMenu(page: Page) {
  const editor = editorOf(page);
  await editor.click();
  await editor.dispatchEvent("contextmenu", { clientX: 200, clientY: 200 });
  await page
    .getByRole("menuitem", { name: "Tag einfügen" })
    .evaluate((el) => (el as HTMLButtonElement).click());
}

async function pickMenuItem(page: Page, testId: string) {
  await page.getByTestId("tts-action-menu").click();
  await page.getByTestId(testId).click();
}

test("Phonon: das Dropdown nennt Skript mit Sprechern und die Stimmen als Einträge", async ({
  page,
}) => {
  await setup(page, { engine: "provider:phonon" });
  const select = await gotoReadAloud(page);
  // Ohne gewählte Stimme gilt "Skript mit Sprechern" -- wie bei Fish.
  await expect(select).toContainText("Phonon · Skript mit Sprechern");
  await expect(page.getByTestId("voice-mode-hint")).toContainText(
    "Die Sprecher im Skript lesen ihre Zeilen",
  );
  await select.getByRole("combobox").click();
  const option = (name: RegExp) => page.getByRole("option", { name });
  await expect(option(/Phonon · Nele/)).toBeVisible();
  await expect(option(/Phonon · Anton/)).toBeVisible();
  await expect(option(/Phonon · Skript mit Sprechern/)).toBeVisible();
  // Die Standardstimme heißt jetzt "Skript mit Sprechern", nicht mehr "Standardstimme".
  await expect(option(/Standardstimme des Modells/)).toHaveCount(0);
});

test("Phonon: eine gewählte Stimme liest alles, die Sprecher im Skript werden übergangen", async ({
  page,
}) => {
  await setup(page, { engine: "provider:phonon", chosen: "nele-id" });
  const select = await gotoReadAloud(page);
  await expect(select).toContainText("Phonon · Nele");
  await expect(page.getByTestId("voice-mode-hint")).toContainText(
    "Diese Stimme liest den ganzen Text",
  );
  // Zurück zu "Skript mit Sprechern": Anbieter und KEINE Stimme gehen ans Backend.
  await select.getByRole("combobox").click();
  await page
    .getByRole("option", { name: /Phonon · Skript mit Sprechern/ })
    .click();
  await expect
    .poll(async () => (await calls(page, "tts_provider_select")).length)
    .toBe(1);
  expect((await calls(page, "tts_provider_select"))[0][1]).toEqual({
    id: "phonon",
    voice: null,
  });
  await expect(page.getByTestId("voice-mode-hint")).toContainText(
    "Die Sprecher im Skript lesen ihre Zeilen",
  );
});

test("Phonon: Sprecher einfügen listet die Phonon-Stimmen mit Geschlecht, nicht die von Fish", async ({
  page,
}) => {
  await setup(page, { engine: "provider:phonon" });
  await gotoReadAloud(page);
  const editor = editorOf(page);
  await editor.fill("Es war einmal.");
  await openSpeakerMenu(page);
  const nele = page.getByRole("menuitem", { name: /Nele/ });
  const anton = page.getByRole("menuitem", { name: /Anton/ });
  await expect(nele).toContainText("weiblich");
  await expect(anton).toContainText("männlich");
  await expect(page.getByRole("menuitem", { name: /Erzählerin/ })).toHaveCount(
    0,
  );
  await expect(page.getByRole("menuitem", { name: /Leo/ })).toHaveCount(0);
  await nele.evaluate((el) => (el as HTMLButtonElement).click());
  await expect(editor).toHaveValue(/<Nele> /);
  // Die Oberfläche hat die Liste vom Backend geholt (dieselbe, die der Parser benutzt).
  expect((await calls(page, "tts_provider_speakers")).length).toBeGreaterThan(
    0,
  );
});

test("Phonon: die Skript-Prüfung kennt die Phonon-Stimmen und nicht die von Fish", async ({
  page,
}) => {
  await setup(page, { engine: "provider:phonon" });
  await gotoReadAloud(page);
  const editor = editorOf(page);
  await editor.fill(
    "<Nele> Guten Tag.\n<Anton:leise> Hallo.\n<Erzählerin> Fremd.\n[flüsternd und müde] Frei.",
  );
  await pickMenuItem(page, "script-check-run");
  const panel = page.getByTestId("script-check");
  // Nur die Erzählerin (Fish) ist unbekannt; Nele und Anton gelten.
  await expect(panel).toContainText("1 Befund im Skript");
  await expect(page.getByTestId("script-finding")).toContainText(
    "Sprecher „Erzählerin“",
  );
  // Ohne Stil-Tags gibt es keine Tag-Vorschläge: das freie Tag ist kein Hinweis.
  await expect(panel).not.toContainText("Hinweis");
  await expect(page.locator("[data-finding]")).toHaveCount(1);
});

test("Phonon: die ganze Beschriftung trifft die Stimme, mit Vorschlag für den Namen", async ({
  page,
}) => {
  await setup(page, { engine: "provider:phonon" });
  await gotoReadAloud(page);
  await editorOf(page).fill("<Nele – Rounded, open tone> Guten Tag.");
  await pickMenuItem(page, "script-check-run");
  const card = page.getByTestId("script-finding");
  // Ein Hinweis, kein Fehler: der Marker trifft Nele, ist aber anders geschrieben.
  await expect(card).toContainText(
    "ist anders geschrieben als der Anzeigename",
  );
  await expect(
    card.getByTestId("script-finding-recommendation").first(),
  ).toHaveText("Nele");
  await expect(page.locator("[data-finding]")).toHaveCount(0);
});

test("Phonon: Sprecher tauschen bietet die Phonon-Stimmen an", async ({
  page,
}) => {
  await setup(page, { engine: "provider:phonon" });
  await gotoReadAloud(page);
  await editorOf(page).fill("<Nele> Guten Tag.\n<Anton> Hallo zusammen.");
  await page.getByRole("tab", { name: "Übersetzung" }).click();
  await page.getByTestId("tts-action-translate").click();
  // Die Übersetzung hat die Marken behalten: der Dialog zeigt Nele und Anton.
  const list = page.getByTestId("speaker-swap-list");
  await expect(list).toBeVisible();
  const forNele = page.getByTestId("speaker-swap-nele-id");
  await expect(forNele).toBeVisible();
  const labels = await forNele.locator("option").allTextContents();
  expect(labels).toContain("Nele · weiblich · de");
  expect(labels).toContain("Anton · männlich · de");
  expect(labels.join("|")).not.toContain("Erzählerin");
});

test("Phonon: die Palette bietet nur Pausen an, Auto-Tagging ist ausgegraut", async ({
  page,
}) => {
  await setup(page, { engine: "provider:phonon" });
  await gotoReadAloud(page);
  await editorOf(page).fill("Ein Satz.");
  await openTagMenu(page);
  await expect(page.getByTestId("tag-menu-pauses-only")).toContainText(
    "Phonon setzt keine Stil-Tags um",
  );
  // Pausen sind da, Gefühle nicht.
  await expect(
    page.getByRole("menuitem", { name: /Pause/ }).first(),
  ).toBeVisible();
  await expect(page.getByRole("menuitem", { name: /Flüsternd/ })).toHaveCount(0);
  await page.keyboard.press("Escape");

  // Die Schreibregeln erklären nur Pausen, keine Emotions-Tags.
  const rules = writingRules(page);
  await rules.locator("summary").click();
  await expect(rules).toContainText("Phonon setzt keine Emotions-Tags um");
  await expect(rules).not.toContainText("[whisper]");
  await expect(rules).toContainText("[short pause]");

  await page.getByTestId("tts-action-menu").click();
  const autotag = page.getByTestId("autotag-open");
  await expect(autotag).toBeDisabled();
  await expect(autotag).toHaveAttribute(
    "title",
    /Phonon setzt keine Stil-Tags um/,
  );
});

test("Phonon ohne Stimmwechsel: keine Sprecher im Skript", async ({ page }) => {
  await setup(page, { engine: "provider:phonon", switching: false });
  const select = await gotoReadAloud(page);
  // Ohne Stimmwechsel ist die Standardstimme einfach die Standardstimme.
  await expect(select).not.toContainText("Skript mit Sprechern");
  await expect(page.getByTestId("voice-mode-hint")).toContainText(
    "Diese Stimme liest den ganzen Text",
  );
  const editor = editorOf(page);
  await editor.fill("<Nele> Hallo.");
  await openSpeakerMenu(page);
  await expect(page.getByRole("menuitem", { name: /Nele/ })).toHaveCount(0);
});

test("Fish bleibt, wie es war: Sprecher sind die aufgenommenen Stimmen, volle Palette", async ({
  page,
}) => {
  await setup(page, { engine: "fish" });
  const select = await gotoReadAloud(page);
  await expect(select).toContainText("Skript mit Sprechern");
  await expect(select).not.toContainText("Phonon · Skript mit Sprechern");
  const editor = editorOf(page);
  await editor.fill("Es war einmal.");
  await openSpeakerMenu(page);
  await expect(
    page.getByRole("menuitem", { name: /Erzählerin/ }),
  ).toBeVisible();
  await expect(page.getByRole("menuitem", { name: /Nele/ })).toHaveCount(0);
  // Fish: keine Geschlechtsangabe, keine Anfrage an die Anbieter-Sprecher.
  await expect(page.getByTestId("speaker-gender")).toHaveCount(0);
  expect(await calls(page, "tts_provider_speakers")).toHaveLength(0);
  // Palette ganz, Auto-Tagging frei.
  await page.keyboard.press("Escape");
  await openTagMenu(page);
  await expect(page.getByTestId("tag-menu-pauses-only")).toHaveCount(0);
  await expect(
    page.getByRole("menuitem", { name: /Flüsternd/ }).first(),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  const rules = writingRules(page);
  await rules.locator("summary").click();
  await expect(rules).toContainText("Emotions- und Effekt-Tags stehen");
  await expect(rules).toContainText("[whisper] Komm näher");
  await page.getByTestId("tts-action-menu").click();
  await expect(page.getByTestId("autotag-open")).toBeEnabled();
});

test("Bild: Vorlesen mit Phonon-Sprechern (nur mit SCREENS_DIR)", async ({
  page,
}) => {
  const dir = process.env.SCREENS_DIR;
  test.skip(!dir, "nur mit SCREENS_DIR");
  await setup(page, { engine: "provider:phonon" });
  const select = await gotoReadAloud(page);
  const editor = editorOf(page);
  await editor.fill(
    "<Nele> Guten Morgen, Anton. Hast du gut geschlafen?\n<Anton> Ja, danke. Und du?\n<Nele> Wunderbar. Wollen wir anfangen?\n<Erzählerin> Fremd.",
  );
  await page.waitForTimeout(400);
  await page.screenshot({ path: `${dir}/phonon-sprecher-seite.png` });
  await select.getByRole("combobox").click();
  await page.waitForTimeout(200);
  await page.screenshot({ path: `${dir}/phonon-sprecher-dropdown.png` });
  await page.keyboard.press("Escape");
  await expect(page.getByRole("option").first()).toHaveCount(0);
  // Mit echtem Rechtsklick (nicht per Ereignis): das Menue sitzt dann an der Maus.
  await editor.click({ button: "right", position: { x: 300, y: 50 } });
  await page
    .getByRole("menuitem", { name: "Sprecher einfügen" })
    .evaluate((el) => (el as HTMLButtonElement).click());
  await expect(page.getByRole("menuitem", { name: /Nele/ })).toBeVisible();
  await page.screenshot({ path: `${dir}/phonon-sprecher-menue.png` });
});

test("Phonon: Reinhoeren im Dropdown spielt die Hoerprobe, ohne die Stimme zu waehlen", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const w = window as unknown as { __played: string[] };
    w.__played = [];
    HTMLMediaElement.prototype.play = function () {
      w.__played.push(this.getAttribute("src") ?? this.src);
      return Promise.resolve();
    };
  });
  await setup(page, { engine: "provider:phonon" });
  const select = await gotoReadAloud(page);
  await select.getByRole("combobox").click();
  const nele = page.getByTestId("voice-sample-provider:phonon:nele-id");
  await expect(nele).toBeVisible();
  await expect(
    page.getByTestId("voice-sample-provider:phonon:anton-id"),
  ).toHaveAttribute("aria-disabled", "true");
  await page.screenshot({
    path: "test-results/phonon-hoerproben-dropdown.png",
  });
  await nele.click();
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { __played: string[] }).__played,
      ),
    )
    .toEqual(["C:/proben/nele.wav"]);
  // Die Liste bleibt offen, und gewaehlt wurde nichts.
  await expect(
    page.getByRole("option", { name: /Phonon · Anton/ }),
  ).toBeVisible();
  expect(await calls(page, "tts_provider_select")).toHaveLength(0);
  await expect(select).toContainText("Phonon · Skript mit Sprechern");
});

test("Reinhören gibt es auch für Stimmen ohne Vorratsprobe (Fish)", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const w = window as unknown as { __played: string[] };
    w.__played = [];
    HTMLMediaElement.prototype.play = function () {
      w.__played.push(this.getAttribute("src") ?? this.src);
      return Promise.resolve();
    };
  });
  await setup(page, { engine: "provider:phonon" });
  const select = await gotoReadAloud(page);
  await select.getByRole("combobox").click();
  await page.getByTestId("voice-sample-leo").click();
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { __played: string[] }).__played,
      ),
    )
    .toEqual(["C:/proben/leo-demo.wav"]);
  expect(await calls(page, "tts_provider_select")).toHaveLength(0);
});

/** Caret in den Text setzen und das Kontextmenü dort öffnen. */
async function contextMenuAt(page: Page, caret: number) {
  const editor = editorOf(page);
  await editor.evaluate((el, at) => {
    const ta = el as HTMLTextAreaElement;
    ta.focus();
    ta.setSelectionRange(at, at);
    // Echtes Mausereignis mit Koordinaten (wie ein Rechtsklick): sonst steht
    // das Menü ohne Position unsichtbar am Rand.
    ta.dispatchEvent(
      new MouseEvent("contextmenu", {
        bubbles: true,
        cancelable: true,
        clientX: 520,
        clientY: 180,
      }),
    );
  }, caret);
}

/** Einen Menüeintrag per JS klicken (das Menü schließt bei Scroll). */
const clickIn = (page: Page, testId: string) =>
  page.getByTestId(testId).evaluate((el) => (el as HTMLButtonElement).click());

const pickSpeaker = (page: Page, name: RegExp) =>
  page
    .getByRole("menu")
    .getByRole("menuitem", { name })
    .evaluate((el) => (el as HTMLButtonElement).click());

const SCRIPT = [
  "<Erzählerin> Hallo.",
  "<Leo> Hi.",
  "<Erzählerin:leise> Tschüss.",
].join("\n");

test("Kontextmenü: Sprecher ändern an allen Stellen mit demselben Namen (Stil bleibt)", async ({
  page,
}) => {
  await setup(page, { engine: "fish" });
  await gotoReadAloud(page);
  await editorOf(page).fill(SCRIPT);
  await contextMenuAt(page, 3);
  await clickIn(page, "menu-speaker-change");
  // Der aktuelle Sprecher steht nicht zur Wahl.
  const menu = page.getByRole("menu");
  await expect(menu.getByRole("menuitem", { name: /Leo/ })).toBeVisible();
  await expect(menu.getByRole("menuitem", { name: /^Erzählerin/ })).toHaveCount(
    0,
  );
  await pickSpeaker(page, /Leo/);
  await expect(page.getByTestId("menu-speaker-change-all")).toContainText(
    "Alle 2 Stellen „Erzählerin“",
  );
  await page.screenshot({ path: "test-results/sprecher-aendern-menue.png" });
  await clickIn(page, "menu-speaker-change-all");
  await expect(editorOf(page)).toHaveValue(
    ["<Leo> Hallo.", "<Leo> Hi.", "<Leo:leise> Tschüss."].join("\n"),
  );
});

test("Kontextmenü: Sprecher ändern nur an dieser Stelle, auch bei einem gültigen Namen", async ({
  page,
}) => {
  await setup(page, { engine: "fish" });
  await gotoReadAloud(page);
  await editorOf(page).fill(SCRIPT);
  // Caret in der dritten Zeile, im Marker <Erzählerin:leise>.
  await contextMenuAt(page, SCRIPT.lastIndexOf("<Erz") + 2);
  await clickIn(page, "menu-speaker-change");
  await pickSpeaker(page, /Leo/);
  await clickIn(page, "menu-speaker-change-here");
  await expect(editorOf(page)).toHaveValue(
    ["<Erzählerin> Hallo.", "<Leo> Hi.", "<Leo:leise> Tschüss."].join("\n"),
  );
});

test("Kontextmenü: unbekannter Sprecher lässt sich umhängen, außerhalb eines Markers fehlt der Eintrag", async ({
  page,
}) => {
  await setup(page, { engine: "fish" });
  await gotoReadAloud(page);
  await editorOf(page).fill(
    ["<Erzähler> Eins.", "<Erzähler> Zwei."].join("\n"),
  );
  await contextMenuAt(page, 14);
  await expect(page.getByTestId("menu-speaker-change")).toHaveCount(0);
  await page.keyboard.press("Escape");
  await contextMenuAt(page, 2);
  await expect(page.getByTestId("menu-speaker-change")).toContainText(
    "Sprecher „Erzähler“ ändern",
  );
  await clickIn(page, "menu-speaker-change");
  await pickSpeaker(page, /Erzählerin/);
  await clickIn(page, "menu-speaker-change-all");
  await expect(editorOf(page)).toHaveValue(
    ["<Erzählerin> Eins.", "<Erzählerin> Zwei."].join("\n"),
  );
});

test("Die Stimme gehört zur Seite: ein Seitenwechsel holt die Stimme der neuen Seite", async ({
  page,
}) => {
  await setup(page, { engine: "provider:phonon", twoPages: true });
  const select = await gotoReadAloud(page);
  await select.getByRole("combobox").click();
  await page.getByRole("option", { name: /Phonon · Nele/ }).click();
  await expect(select).toContainText("Phonon · Nele");
  const selects = () => calls(page, "tts_provider_select");
  await expect.poll(async () => (await selects()).length).toBe(1);

  // Seite 2 hat Anton gespeichert: mit der Seite wechselt die Stimme.
  await page
    .getByText("Hörbuch", { exact: true })
    .click({ position: { x: 4, y: 4 } });
  // Eine Seite mit Text öffnet in der Leseansicht (lva#148).
  await expect(page.getByTestId("reader-flow")).toContainText("Zweite Seite.");
  await expect(select).toContainText("Phonon · Anton");
  await expect.poll(async () => (await selects()).length).toBe(2);
});

test("Rechtsklick auf eine Seite öffnet ein Fluent-Menü statt des Browsermenüs", async ({
  page,
}) => {
  await setup(page, { engine: "provider:phonon", twoPages: true });
  await gotoReadAloud(page);
  await page
    .getByText("Hörbuch", { exact: true })
    .click({ button: "right", position: { x: 4, y: 4 } });
  const menu = page.getByRole("menu", { name: "Seiten" });
  await expect(menu).toBeVisible();
  await expect(
    menu.getByRole("menuitem", { name: "Umbenennen" }),
  ).toBeVisible();
  await expect(menu.getByRole("menuitem", { name: /löschen/i })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);
});

test("Sprecher auf allen Seiten ersetzen: Vorschau zählt, Anwenden schreibt Seiten und offenen Text", async ({
  page,
}) => {
  await setup(page, {
    engine: "provider:phonon",
    twoPages: true,
    markers: true,
  });
  const select = await gotoReadAloud(page);
  await editorOf(page).fill("<Nele> Eins.\n<Anton> Zwei.");
  await select.click({ button: "right" });
  await page.getByTestId("voice-menu-replace").click();
  const from = page.getByTestId("replace-speaker-from");
  const to = page.getByTestId("replace-speaker-to");
  await from.selectOption({ label: "Nele" });
  await to.selectOption({ label: "Anton" });
  await expect(page.getByTestId("replace-speaker-preview")).toContainText(
    "2 Marker",
  );
  await page.getByTestId("replace-speaker-apply").click();
  await expect(page.getByTestId("replace-speaker-preview")).toContainText(
    "Fertig",
  );
  await page.getByRole("button", { name: "Schließen" }).first().click();
  await expect(editorOf(page)).toHaveValue("<Anton> Eins.\n<Anton> Zwei.");
  const saves = await calls(page, "page_state_save");
  const other = saves.find((c) => c[1]?.id === "p2");
  expect(JSON.parse(String(other?.[1]?.state)).text).toBe(
    "<Anton> Zweite Seite.",
  );
});
