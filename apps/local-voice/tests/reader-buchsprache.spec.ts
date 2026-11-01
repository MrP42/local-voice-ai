import { test, expect, type Page } from "@playwright/test";

// Buchsprache und Stimmen (Patrick 01.11. 23:08). Attrappe aus
// readaloud-provider-speakers.spec.ts, ergänzt um englische Phonon-Stimmen.
//
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
  /** Arbeitsstand der Seite p1 (JSON), sonst leer. */
  state?: string;
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
    const en = (id: string, label: string) => ({
      ...phononVoice(id, label, null),
      voice: { id, label, language: "en", gender: null, license: null },
    });
    const options = [
      phononVoice("default", "Standardstimme des Modells", null),
      phononVoice("nele-id", "Nele – Rounded, open tone", "female"),
      phononVoice("anton-id", "Anton – Warm baritone", "male"),
      en("en:default", "Standardstimme des Modells (en)"),
      en("en:Zoey", "Zoey"),
      en("en:Mark", "Mark"),
      ...Array.from({ length: 30 }, (_, i) =>
        phononVoice(`de-${i}`, `Stimme ${String(i).padStart(2, "0")}`, null),
      ),
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
          if (cmd === "page_state_load" && args?.id === "p1")
            return cfg.state ?? null;
          if (cmd === "library_list") return [];
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

const ENGLISH = Array.from(
  { length: 20 },
  (_, i) =>
    `Paragraph ${i + 1}: The distinction between fast and slow thinking has been explored by many psychologists over the last twenty-five years, and it is the secret author of many of the choices that you make.`,
).join("\n\n");

const saved = async (page: Page) => {
  const all = (await calls(page, "page_state_save")) as [
    string,
    { id: string; state: string },
  ][];
  const last = all.filter(([, a]) => a.id === "p1").pop();
  return last ? JSON.parse(last[1].state) : null;
};

test("englisches Buch: Sprache erkannt und gespeichert, englische Standardstimme", async ({ page }) => {
  await setup(page, {
    engine: "provider:phonon",
    state: JSON.stringify({
      text: ENGLISH,
      tab: "original",
      voices: { original: "provider:phonon:default" },
    }),
  });
  await gotoReadAloud(page);
  await expect
    .poll(async () =>
      ((await calls(page, "tts_provider_select")) as [string, { id: string; voice: string | null }][]).map(
        ([, a]) => a.voice,
      ),
    )
    .toContain("en:default");
  await expect.poll(async () => (await saved(page))?.lang).toBe("en");
  await expect
    .poll(async () => (await saved(page))?.voices?.original)
    .toBe("provider:phonon:en:default");
});

test("von Hand gewählte Stimme bleibt, auch wenn sie nicht zur Buchsprache passt", async ({ page }) => {
  await setup(page, {
    engine: "provider:phonon",
    chosen: "nele-id",
    state: JSON.stringify({
      text: ENGLISH,
      tab: "original",
      voices: { original: "provider:phonon:nele-id" },
      voicesChosen: { original: true },
    }),
  });
  await gotoReadAloud(page);
  await expect.poll(async () => (await saved(page))?.lang).toBe("en");
  await page.waitForTimeout(500);
  expect(await calls(page, "tts_provider_select")).toHaveLength(0);
});

test("Stimmenauswahl: Sprachfilter, standardmäßig passend zur Buchsprache", async ({ page }) => {
  await setup(page, {
    engine: "provider:phonon",
    state: JSON.stringify({ text: ENGLISH, tab: "original", voices: {} }),
  });
  const select = await gotoReadAloud(page);
  await select.locator(".app-select__control").click();
  const menu = page.locator(".app-select__menu");
  await expect(menu.getByTestId("voice-lang-filter-auto")).toHaveAttribute("aria-pressed", "true");
  await expect(menu.getByTestId("voice-lang-filter-auto")).toHaveText("Passend (Englisch)");
  await expect(menu.getByRole("option", { name: /Zoey/ })).toBeVisible();
  await expect(menu.getByRole("option", { name: /Nele/ })).toHaveCount(0);
  // Alle: auch die deutschen; ein Klick auf den Filter schließt die Liste nicht.
  await menu.getByTestId("voice-lang-filter-all").click();
  await expect(menu).toBeVisible();
  await expect(menu.getByRole("option", { name: /Nele/ })).toHaveCount(1);
  // Deutsch: nur deutsche (und die gewählte).
  await menu.getByTestId("voice-lang-filter-de").click();
  await expect(menu.getByTestId("voice-lang-filter-en")).toHaveCount(0);
  await expect(menu.getByRole("option", { name: /Mark/ })).toHaveCount(0);
  await expect(menu.getByRole("option", { name: /Nele/ })).toHaveCount(1);
});

test("Stimmenauswahl öffnet wieder an der zuletzt gescrollten Stelle", async ({ page }) => {
  await setup(page, {
    engine: "provider:phonon",
    state: JSON.stringify({ text: ENGLISH, tab: "original", voices: {} }),
  });
  const select = await gotoReadAloud(page);
  await select.locator(".app-select__control").click();
  await page.getByTestId("voice-lang-filter-all").click();
  const list = page.locator(".app-select__menu-list");
  await list.evaluate((el) => {
    el.scrollTop = 400;
    el.dispatchEvent(new Event("scroll"));
  });
  const top = await list.evaluate((el) => el.scrollTop);
  expect(top).toBeGreaterThan(100);
  await page.keyboard.press("Escape");
  await expect(list).toHaveCount(0);
  await select.locator(".app-select__control").click();
  await expect
    .poll(() => page.locator(".app-select__menu-list").evaluate((el) => el.scrollTop))
    .toBe(top);
});
