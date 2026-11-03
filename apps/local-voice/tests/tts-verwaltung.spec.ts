import { test, expect, type Page } from "@playwright/test";
import { readFileSync, mkdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { geheZu } from "./profilmenue";

// Vorlesen unter Modelle (Variante B): Umschalter Stimmen/Anbieter, Filterspalte
// mit Zaehlern, Karten- und Listenansicht ueber alle Anbieter, Hoerprobe und
// Auswahl; Anbieter als Karten mit den bisherigen Einstellungen im Seitenblatt.
// Anbieterdaten: die echten Daten aus `fixtures/tts-providers-real.json`, nur
// der Zustand wird je Test abgewandelt. Bilder nur mit SCREENS_DIR.

type Cfg = { engine?: string; phononReady?: boolean };

const REAL = JSON.parse(
  readFileSync(
    resolve(process.cwd(), "tests", "fixtures", "tts-providers-real.json"),
    "utf8",
  ),
) as unknown[];

async function setup(page: Page, cfg: Cfg = {}, theme = "light") {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.addInitScript(
    (arg: { cfg: Cfg; real: unknown[]; theme: string }) => {
      const { cfg, real, theme } = arg;
      const callbacks = new Map<number, unknown>();
      let callback = 0;
      const w = window as unknown as Record<string, any>;
      w.__calls = [] as unknown[];
      const entries = JSON.parse(JSON.stringify(real)) as any[];
      const find = (id: unknown) => entries.find((e) => e.descriptor.id === id);
      const recompute = (e: any) => {
        if (e.descriptor.opt_in && !e.enabled)
          e.blocked = {
            kind: "license_blocked",
            retryable: false,
            message: "Der Anbieter ist nicht freigeschaltet.",
          };
        else if (e.license_needs_ack && !e.license_acked)
          e.blocked = {
            kind: "license_blocked",
            retryable: false,
            message: "Lizenz nicht bestätigt.",
          };
        else e.blocked = null;
      };
      if (cfg.phononReady) {
        const p = find("phonon");
        p.enabled = true;
        p.license_acked = true;
        p.health = {
          ...p.health,
          available: true,
          reason: null,
          hint: null,
          version: "0.4.2",
        };
        recompute(p);
      }
      entries.forEach((e) => {
        e.visible_voices = e.visible_voices ?? [];
        e.selected = false;
      });
      const settings: Record<string, any> = {
        onboarding_completed: true,
        app_language: "de",
        theme,
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
        tts_voice: "anton",
        tts_engine: cfg.engine ?? "fish",
        tts_piper_voice: "de_DE-thorsten-medium",
        tts_fish_dir: "C:\\AI\\fish-speech",
        tts_providers: { voices: {}, voice_names: {} },
      };
      const dl = (over: Record<string, unknown>) => ({
        kind: "voice",
        name: "",
        description: "",
        language: "de",
        size_mb: 60,
        is_downloaded: false,
        is_downloading: false,
        is_usable: false,
        unsupported_reason: null,
        license: "CC0-1.0",
        license_url: null,
        license_non_commercial: false,
        engine: "",
        size_bytes: 0,
        gated: false,
        requires: null,
        source: null,
        sha256: null,
        license_note: null,
        ...over,
      });
      const downloads = [
        dl({
          id: "piper-runtime",
          kind: "runtime",
          name: "Piper",
          language: null,
          size_mb: 20,
          is_downloaded: true,
          is_usable: true,
        }),
        dl({
          id: "de_DE-thorsten-medium",
          name: "Thorsten (Deutsch)",
          description: "Klare, ruhige Stimme.",
          is_downloaded: true,
          is_usable: true,
        }),
        dl({
          id: "de_DE-thorsten-high",
          name: "Thorsten (hoch)",
          description: "Beste Piper-Qualität, größter Download.",
          size_mb: 110,
          license_non_commercial: true,
        }),
        dl({
          id: "en_US-amy-medium",
          name: "Amy (Englisch)",
          description: "Amerikanisches Englisch.",
          language: "en",
          size_mb: 63,
        }),
        dl({
          id: "de_DE-kerstin-low",
          name: "Kerstin (niedrig)",
          description: "Schnell und klein.",
          size_mb: 28,
        }),
      ];
      const fishInfo = (id: string, name: string, language: string) => ({
        id,
        meta: { display_name: name, color: "#3b82f6", language },
        origin: "user",
        avatar_path: null,
      });
      const phononVoices = [
        {
          id: "anna",
          label: "Anna",
          language: "de",
          gender: "female",
          license: null,
        },
        {
          id: "nele",
          label: "Nele – Rounded, open tone with gentle lift at the ends…",
          language: "de",
          gender: "female",
          license: null,
        },
        {
          id: "ben",
          label: "Ben",
          language: "en",
          gender: "male",
          license: null,
        },
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
          invoke: async (cmd: string, args?: Record<string, any>) => {
            w.__calls.push([cmd, args]);
            const copy = <T>(x: T): T => JSON.parse(JSON.stringify(x));
            const sel = () => {
              entries.forEach((e) => {
                e.selected =
                  settings.tts_engine === e.descriptor.id ||
                  settings.tts_engine === `provider:${e.descriptor.id}`;
              });
            };
            if (cmd === "get_app_settings" || cmd === "get_default_settings")
              return copy(settings);
            if (cmd === "plugin:os|locale") return "de-DE";
            if (cmd === "plugin:app|version") return "0.21.19";
            if (cmd.includes("permission")) return true;
            if (cmd === "plugin:event|listen") return ++callback;
            if (cmd === "get_selected_model") return "";
            if (cmd === "meetings_is_recording") return false;
            if (cmd === "tts_server_status")
              return { phase: "stopped", message: null };
            if (cmd === "tts_runtime_status")
              return {
                platform: "windows-x64",
                piper: { supported: true, ready: true, missing: [] },
                fish: { supported: true, ready: true, missing: [] },
                fish_dir: "C:\\AI\\fish-speech",
              };
            if (cmd === "tts_list_downloads") return copy(downloads);
            if (cmd === "tts_list_voices") return ["anton", "lea"];
            if (cmd === "tts_list_voice_infos")
              return [
                fishInfo("anton", "Anton", "de"),
                fishInfo("lea", "Lea", "de"),
              ];
            if (cmd === "change_tts_engine_setting")
              settings.tts_engine = args?.value;
            if (cmd === "change_tts_voice_setting")
              settings.tts_voice = args?.value;
            if (cmd === "change_tts_piper_voice_setting")
              settings.tts_piper_voice = args?.value;
            if (cmd === "tts_download_model") {
              const d = downloads.find((x) => x.id === args?.modelId);
              if (d) {
                d.is_downloaded = true;
                d.is_usable = true;
              }
              return null;
            }
            if (cmd === "pages_list") return [{ id: "p1", title: "Seite" }];
            if (cmd === "get_custom_sounds")
              return { start: false, stop: false };
            if (cmd === "tts_providers_list") {
              sel();
              return copy(entries);
            }
            if (cmd === "tts_provider_health")
              return copy(find(args?.id).health);
            if (cmd === "tts_provider_voice_samples")
              return args?.id === "phonon"
                ? [
                    { voice_id: "anna", wav_path: "C:/proben/anna.wav" },
                    { voice_id: "nele", wav_path: "C:/proben/nele.wav" },
                    { voice_id: "ben", wav_path: null },
                  ]
                : [];
            // Eigene Namen wie im Backend (`apply_voice_names`): Name vor " – " ersetzen.
            const named = (pid: string, list: any[]) =>
              list.map((v) => {
                const own = settings.tts_providers.voice_names[pid]?.[v.id];
                if (!own) return v;
                const rest = String(v.label).split(" – ").slice(1).join(" – ");
                return { ...v, label: rest ? `${own} – ${rest}` : own };
              });
            if (cmd === "tts_provider_set_voice_name") {
              const pid = String(args?.id);
              const names = (settings.tts_providers.voice_names[pid] ??= {});
              const name = String(args?.name ?? "").trim();
              if (name.includes("<") || name.includes(":"))
                throw {
                  kind: "bad_request",
                  retryable: false,
                  message: "Der Name darf keine Zeichen < > : enthalten.",
                };
              if (!name || name === args?.original) delete names[args?.voice];
              else names[args?.voice] = name;
              if (Object.keys(names).length === 0)
                delete settings.tts_providers.voice_names[pid];
              return null;
            }
            if (cmd === "tts_set_voice_meta") return null;
            if (cmd === "tts_provider_voices") {
              const e = find(args?.id);
              if (e.blocked) throw e.blocked;
              return e.descriptor.id === "phonon"
                ? copy(named("phonon", phononVoices))
                : [];
            }
            if (cmd === "tts_provider_voice_options") {
              const out: unknown[] = [];
              for (const e of entries) {
                if (["fish", "piper"].includes(e.descriptor.id) || e.blocked)
                  continue;
                if (e.descriptor.id !== "phonon") continue;
                for (const v of [
                  {
                    id: "default",
                    label: "Standardstimme des Modells",
                    language: "de",
                    gender: null,
                    license: null,
                  },
                  ...phononVoices.filter((x) => x.id !== "ben"),
                ])
                  out.push({
                    provider_id: e.descriptor.id,
                    provider_label: e.descriptor.label,
                    voice_switching: false,
                    voice: named(e.descriptor.id, [v])[0],
                  });
              }
              return out;
            }
            if (cmd === "tts_provider_get_config") {
              if (args?.id === "phonon")
                return JSON.stringify({
                  exe_path: "ptts",
                  model: "hf://example/phonon-31m-en",
                  quant: "q8",
                  lang: "de",
                  voice: null,
                  threads: null,
                  hf_home: null,
                  allow_network: false,
                });
              throw {
                kind: "bad_request",
                retryable: false,
                message: "Dieser Anbieter hat keine Konfiguration.",
              };
            }
            if (cmd === "tts_provider_set_enabled") {
              const e = find(args?.id);
              e.enabled = args?.enabled === true;
              recompute(e);
              return null;
            }
            if (cmd === "tts_provider_acknowledge_license") {
              const e = find(args?.id);
              e.license_acked = true;
              recompute(e);
              return {
                provider_id: e.descriptor.id,
                fingerprint: e.fingerprint,
                acked_at: "2026-10-26T10:00:00Z",
              };
            }
            if (cmd === "tts_provider_select") {
              const e = find(args?.id);
              if (e.blocked) throw e.blocked;
              settings.tts_engine =
                e.descriptor.id === "piper" || e.descriptor.id === "fish"
                  ? e.descriptor.id
                  : `provider:${e.descriptor.id}`;
              if (args?.voice)
                settings.tts_providers.voices[e.descriptor.id] = args.voice;
              else delete settings.tts_providers.voices[e.descriptor.id];
              return null;
            }
            if (cmd === "plugin:dialog|ask") return true;
            if (
              cmd.includes("history") ||
              cmd.includes("models") ||
              cmd.includes("devices") ||
              cmd === "meetings_list" ||
              cmd === "llm_ps" ||
              cmd === "tts_reading_list" ||
              cmd === "page_files"
            )
              return [];
            return null;
          },
        },
      });
      try {
        localStorage.setItem("lva.ui.models.area", "voices");
      } catch {
        /* egal */
      }
    },
    { cfg, real: REAL, theme },
  );
}

type Call = [string, Record<string, unknown> | undefined];
const calls = (page: Page, cmd: string) =>
  page.evaluate(
    (c) =>
      ((window as unknown as { __calls: Call[] }).__calls ?? []).filter(
        ([name]) => name === c,
      ),
    cmd,
  );

const openManager = async (page: Page) => {
  await page.goto("/");
  await geheZu(page, "Modelle");
  await expect(page.getByTestId("voice-manager")).toBeVisible();
  await expect(page.getByTestId("voice-gallery")).toBeVisible();
};

const noHorizontalScroll = (page: Page) =>
  page.evaluate(
    () =>
      document.documentElement.scrollWidth <=
      document.documentElement.clientWidth + 1,
  );

test("Stimmen: Karten über alle Anbieter, Zähler, Filter, 'nicht gemessen'", async ({
  page,
}) => {
  await setup(page, { phononReady: true });
  await openManager(page);
  // Fish (Skript + 2), Piper (4), Phonon (default + 3).
  await expect(page.locator("[data-voice-card]")).toHaveCount(11);
  await expect(page.getByTestId("voice-count")).toContainText("11 von 11");
  await expect(page.locator("[data-active-name]")).toHaveText("Anton");
  // Es gibt keine Qualitäts-/Aufwandsangaben: ehrlich "nicht gemessen".
  await expect(
    page.locator("[data-voice-card='piper:de_DE-thorsten-medium']"),
  ).toContainText("nicht gemessen");
  // Anbieterfilter mit Zähler.
  const piper = page.locator("[data-facet='providers'] label", {
    hasText: "Piper",
  });
  await expect(piper).toContainText("4");
  await piper.locator("input").check();
  await expect(page.locator("[data-voice-card]")).toHaveCount(4);
  await page.getByTestId("voice-filter-installed").check();
  await expect(page.locator("[data-voice-card]")).toHaveCount(1);
  await page.getByTestId("voice-filter-reset").click();
  await expect(page.locator("[data-voice-card]")).toHaveCount(11);
});

test("Listenansicht zeigt dieselben Stimmen; die Ansicht bleibt gemerkt", async ({
  page,
}) => {
  await setup(page, { phononReady: true });
  await openManager(page);
  await page.getByTestId("voice-layout-list").click();
  await expect(page.locator("[data-voice-row]")).toHaveCount(11);
  await page.reload();
  await expect(page.getByTestId("voice-list")).toBeVisible();
});

test("Auswahl: Piper-Stimme verwenden schaltet Engine und Stimme", async ({
  page,
}) => {
  await setup(page);
  await openManager(page);
  await page
    .locator("[data-voice-card='piper:de_DE-thorsten-medium']")
    .getByTestId("voice-use-piper:de_DE-thorsten-medium")
    .click();
  await expect
    .poll(
      async () => (await calls(page, "change_tts_piper_voice_setting")).length,
    )
    .toBeGreaterThan(0);
  expect(
    (await calls(page, "change_tts_piper_voice_setting"))[0][1],
  ).toMatchObject({
    value: "de_DE-thorsten-medium",
  });
  expect((await calls(page, "change_tts_engine_setting"))[0][1]).toMatchObject({
    value: "piper",
  });
  await expect(page.locator("[data-active-name]")).toHaveText(
    "Thorsten (mittlere Qualität)",
  );
});

test("Laden: nur dort angeboten, wo es eine Funktion gibt (Piper); Anbieterstimmen ohne Laden", async ({
  page,
}) => {
  await setup(page, { phononReady: true });
  await openManager(page);
  const card = page.locator("[data-voice-card='piper:en_US-amy-medium']");
  await expect(card).toContainText("nicht geladen");
  await card.getByTestId("voice-load-piper:en_US-amy-medium").click();
  expect((await calls(page, "tts_download_model"))[0][1]).toMatchObject({
    id: "en_US-amy-medium",
  });
  // Phonon-Stimmen haben keinen Lade-Knopf.
  await expect(
    page.locator(
      "[data-voice-card='phonon:anna'] [data-testid^='voice-load-']",
    ),
  ).toHaveCount(0);
  // Hörprobe: fertige Probe spielbar, entstehende gesperrt, ungeladene gesperrt.
  await expect(
    page.getByRole("button", { name: /Reinhören: Anna/ }),
  ).toBeEnabled();
  await expect(
    page.getByRole("button", { name: /Hörprobe von Ben wird erstellt/ }),
  ).toBeDisabled();
  await expect(
    page.locator(
      "[data-voice-card='piper:de_DE-kerstin-low'] button[disabled]",
    ),
  ).toHaveCount(1);
});

test("Anbieter: Karten mit Zustand; Seitenblatt enthält die bisherigen Einstellungen; Freigabe-Ablauf", async ({
  page,
}) => {
  await setup(page);
  await openManager(page);
  await page.getByTestId("voice-view-providers").click();
  await expect(page.locator("[data-provider-card]")).toHaveCount(5);
  await expect(page.locator("[data-provider-card='fish']")).toContainText(
    "Bereit",
  );
  // Phonon ist noch nicht eingerichtet: eine Hauptaktion „Einrichten“ statt Schalter.
  await expect(page.locator("[data-provider-card='phonon']")).toContainText(
    "Nicht eingerichtet",
  );
  await expect(page.getByTestId("tts-provider-card-enable-phonon")).toHaveCount(0);
  await page.getByTestId("tts-provider-setup-phonon").click();
  const sheet = page.getByTestId("voice-provider-sheet");
  await expect(sheet).toBeVisible();
  // Aktivieren im Blatt führt weiterhin über den Lizenzdialog.
  await sheet.getByTestId("tts-provider-enable-phonon").click();
  await expect(page.getByTestId("tts-provider-license-dialog")).toBeVisible();
  await page.getByTestId("tts-provider-license-checkbox").check();
  await page.getByTestId("tts-provider-license-confirm").click();
  await expect(page.getByTestId("tts-provider-license-dialog")).toHaveCount(0);
  await expect(sheet.getByTestId("tts-provider-phonon")).toBeVisible();
  await expect(sheet.getByTestId("tts-provider-details-phonon")).toBeVisible();
  await page.getByTestId("voice-provider-sheet-close").click();
  await expect(sheet).toHaveCount(0);
  // Piper: das Seitenblatt trägt auch die Downloads (Stimmen, Programm).
  await page.getByTestId("tts-provider-open-piper").click();
  await expect(
    page
      .getByTestId("voice-provider-sheet")
      .locator("[data-models-section='voices']"),
  ).toBeVisible();
});

test("Einstellungen › Ausgabe verweist auf die Anbieter unter Modelle", async ({
  page,
}) => {
  await setup(page);
  await page.addInitScript(() =>
    localStorage.setItem("lva.ui.settings.tab", "output"),
  );
  await page.goto("/");
  await geheZu(page, "Einstellungen");
  await expect(page.getByTestId("tts-providers-open-models")).toBeVisible();
  await expect(page.getByTestId("tts-provider-fish")).toHaveCount(0);
  await page.getByTestId("tts-providers-open-models").click();
  await expect(page.getByTestId("voice-providers-view")).toBeVisible();
});

test("Seitenblatt: Escape gibt den Fokus an den Öffner zurück; Umschalter ist ein Tab-Umschalter; Downloads-Knopf", async ({
  page,
}) => {
  await setup(page);
  await openManager(page);
  const tabs = page.getByRole("tablist");
  await expect(tabs.getByRole("tab", { name: /Stimmen/ })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await page.getByTestId("voice-view-providers").click();
  await expect(page.locator("#voice-view-panel")).toHaveAttribute(
    "aria-labelledby",
    "voice-view-tab-providers",
  );
  const opener = page.getByTestId("tts-provider-open-piper");
  await expect(opener).toBeVisible();
  await opener.focus();
  await opener.click();
  await expect(page.getByTestId("voice-provider-sheet")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("voice-provider-sheet")).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(() => document.activeElement?.getAttribute("data-testid")),
    )
    .toBe("tts-provider-open-piper");
});

test("Schmal (390 px): keine waagerechte Rollleiste, Bedienziele mindestens 44 px, Filter hinter Knopf", async ({
  page,
}) => {
  await setup(page, { phononReady: true });
  await openManager(page);
  await page.setViewportSize({ width: 390, height: 900 });
  await expect(page.getByTestId("voice-filter-panel")).toBeHidden();
  expect(await noHorizontalScroll(page)).toBe(true);
  await page.getByTestId("voice-filter-toggle").click();
  await expect(page.getByTestId("voice-filter-panel")).toBeVisible();
  await page.getByTestId("voice-filter-toggle").click();
  const small = await page.evaluate(() => {
    const out: string[] = [];
    const sel =
      "[data-voice-card] button, [data-voice-card] [role='button'], [data-testid='voice-view-switch'] button, [data-testid='voice-filter-toggle']";
    document.querySelectorAll<HTMLElement>(sel).forEach((el) => {
      const r = el.getBoundingClientRect();
      if (r.width === 0) return;
      if (r.height < 43.5 || r.width < 43.5)
        out.push(
          `${el.getAttribute("aria-label") ?? el.textContent}: ${Math.round(r.width)}x${Math.round(r.height)}`,
        );
    });
    return out;
  });
  expect(small).toEqual([]);
  await page.getByTestId("voice-view-providers").click();
  await expect(page.getByTestId("voice-providers-view")).toBeVisible();
  expect(await noHorizontalScroll(page)).toBe(true);
  await page.getByTestId("tts-provider-open-phonon").click();
  await expect(page.getByTestId("voice-provider-sheet")).toBeVisible();
  expect(await noHorizontalScroll(page)).toBe(true);
});

test("Umbenennen: eigener Name einer Anbieter-Stimme, Kennung bleibt, zurück zum Original", async ({
  page,
}) => {
  await setup(page, { phononReady: true });
  await openManager(page);
  const card = page.locator("[data-voice-card='phonon:nele']");
  await expect(card.locator("h3")).toHaveText("Nele");
  await page.getByTestId("voice-rename-phonon:nele").click();
  const dialog = page.getByTestId("voice-rename-dialog");
  await expect(dialog).toBeVisible();
  await expect(page.getByTestId("voice-rename-id")).toContainText("nele");
  // Ungültige Zeichen: Hinweis, Speichern gesperrt.
  await page.getByTestId("voice-rename-input").fill("<Nele>");
  await expect(page.getByTestId("voice-rename-error")).toBeVisible();
  await expect(page.getByTestId("voice-rename-save")).toBeDisabled();
  // Der Name einer anderen Stimme desselben Anbieters (auch mit Umlaut-Schreibweise): gesperrt.
  await page.getByTestId("voice-rename-input").fill("anna");
  await expect(page.getByTestId("voice-rename-error")).toContainText(
    "andere Stimme",
  );
  await expect(page.getByTestId("voice-rename-save")).toBeDisabled();
  await page.getByTestId("voice-rename-input").fill("Nelé  Groß");
  await page.getByTestId("voice-rename-input").press("Enter");
  await expect(dialog).toHaveCount(0);
  const set = await calls(page, "tts_provider_set_voice_name");
  expect(set[0][1]).toMatchObject({
    id: "phonon",
    voice: "nele",
    name: "Nelé Groß",
    original: "Nele",
  });
  // Katalog und Vorlesen-Auswahl zeigen den neuen Namen, die Kennung bleibt.
  await expect(card.locator("h3")).toHaveText("Nelé Groß");
  await expect(page.locator("[data-voice-card='phonon:nele']")).toHaveCount(1);
  // Im Seitenblatt des Anbieters: Liste "Im Vorlesen anzeigen" mit Stift, zurück zum Original.
  await page.getByTestId("voice-view-providers").click();
  await page.getByTestId("tts-provider-open-phonon").click();
  const row = page
    .getByTestId("tts-provider-visible-list-phonon")
    .locator("[data-voice-id='nele']");
  await expect(row).toContainText("Nelé Groß");
  await page.getByTestId("tts-provider-rename-nele").click();
  await page.getByTestId("voice-rename-reset").click();
  await expect(row).toContainText("Nele");
  await expect(row).not.toContainText("Nelé");
  const last = (await calls(page, "tts_provider_set_voice_name")).at(-1);
  expect(last?.[1]).toMatchObject({ voice: "nele", name: null });
});

test("Umbenennen: Piper-Stimme bekommt einen eigenen Namen", async ({
  page,
}) => {
  await setup(page);
  await openManager(page);
  const card = page.locator("[data-voice-card='piper:de_DE-thorsten-medium']");
  await page.getByTestId("voice-rename-piper:de_DE-thorsten-medium").click();
  await page.getByTestId("voice-rename-input").fill("Thorsten ruhig");
  await page.getByTestId("voice-rename-save").click();
  await expect(card.locator("h3")).toHaveText("Thorsten ruhig");
  expect(
    (await calls(page, "tts_provider_set_voice_name"))[0][1],
  ).toMatchObject({ id: "piper", voice: "de_DE-thorsten-medium" });
});

test("Umbenennen: eigene Fish-Stimme ändert ihren Anzeigenamen", async ({
  page,
}) => {
  await setup(page);
  await openManager(page);
  await page.getByTestId("voice-rename-fish:lea").click();
  await page.getByTestId("voice-rename-input").fill("Léa");
  await page.getByTestId("voice-rename-save").click();
  await expect(page.getByTestId("voice-rename-dialog")).toHaveCount(0);
  const meta = await calls(page, "tts_set_voice_meta");
  expect(meta[0][1]).toMatchObject({
    id: "lea",
    meta: { display_name: "Léa" },
  });
});

test("Bilder: Stimme umbenennen (nur mit SCREENS_DIR)", async ({ browser }) => {
  const dir = process.env.SCREENS_DIR;
  test.skip(!dir, "nur mit SCREENS_DIR");
  test.setTimeout(240_000);
  mkdirSync(dir as string, { recursive: true });
  const masse: string[] = [];
  for (const theme of ["light", "dark"] as const) {
    const label = theme === "light" ? "hell" : "dunkel";
    for (const width of [1440, 390]) {
      const ctx = await browser.newContext();
      const p = await ctx.newPage();
      await setup(p, { phononReady: true }, theme);
      await p.goto("/");
      await geheZu(p, "Modelle");
      await p.setViewportSize({ width, height: width === 1440 ? 1000 : 900 });
      await expect(p.getByTestId("voice-gallery")).toBeVisible();
      // Touchziel des Stifts auf der Karte messen (Beleg fuer >= 44 px unter 768 px).
      const box = async (testId: string) => {
        const b = await p.getByTestId(testId).boundingBox();
        return b ? `${Math.round(b.width)}x${Math.round(b.height)}` : "fehlt";
      };
      masse.push(
        `${label} ${width}px Karte  Stift voice-rename-phonon:nele: ${await box("voice-rename-phonon:nele")}`,
      );
      await p.getByTestId("voice-rename-phonon:nele").click();
      // Doppelname: Fehlermeldung, Speichern gesperrt.
      await p.getByTestId("voice-rename-input").fill("Anna");
      await expect(p.getByTestId("voice-rename-save")).toBeDisabled();
      await p.waitForTimeout(300);
      await p.screenshot({
        path: `${dir}/umbenennen-5-doppelname-${label}-${width}.png`,
      });
      await p.getByTestId("voice-rename-input").fill("Nelé");
      await p.waitForTimeout(300);
      await p.screenshot({
        path: `${dir}/umbenennen-1-dialog-${label}-${width}.png`,
      });
      await p.getByTestId("voice-rename-save").click();
      await expect(p.getByTestId("voice-rename-dialog")).toHaveCount(0);
      await p.waitForTimeout(300);
      await p.screenshot({
        path: `${dir}/umbenennen-2-karte-${label}-${width}.png`,
      });
      await p.getByTestId("voice-view-providers").click();
      await p.getByTestId("tts-provider-open-phonon").click();
      const row = p
        .getByTestId("tts-provider-visible-list-phonon")
        .locator("[data-voice-id='nele']");
      await row.scrollIntoViewIfNeeded();
      await p.waitForTimeout(400);
      masse.push(
        `${label} ${width}px Liste  Stift tts-provider-rename-nele: ${await box("tts-provider-rename-nele")}`,
      );
      await p.screenshot({
        path: `${dir}/umbenennen-3-liste-${label}-${width}.png`,
      });
      await p.getByTestId("tts-provider-rename-nele").click();
      await p.getByTestId("voice-rename-input").fill("Anna");
      await expect(p.getByTestId("voice-rename-save")).toBeDisabled();
      await p.waitForTimeout(300);
      await p.screenshot({
        path: `${dir}/umbenennen-6-doppelname-liste-${label}-${width}.png`,
      });
      await p.getByTestId("voice-rename-input").fill("Nelé");
      await p.waitForTimeout(300);
      await p.screenshot({
        path: `${dir}/umbenennen-4-zuruecksetzen-${label}-${width}.png`,
      });
      await ctx.close();
    }
  }
  writeFileSync(
    `${dir}/touchziele.txt`,
    [
      "Touchziele der Stifte (boundingBox, Breite x Hoehe in CSS-px).",
      "Unter 768 px (Touch) gilt >= 44 px; ab 768 px (Maus) kompakter wie der Knopf Anbieter oeffnen daneben.",
      ...masse,
      "",
    ].join("\n"),
  );
});

test("Bilder der Verwaltung (nur mit SCREENS_DIR)", async ({ browser }) => {
  const dir = process.env.SCREENS_DIR;
  test.skip(!dir, "nur mit SCREENS_DIR");
  test.setTimeout(240_000);
  mkdirSync(dir as string, { recursive: true });
  for (const theme of ["light", "dark"] as const) {
    const label = theme === "light" ? "hell" : "dunkel";
    for (const width of [1440, 390]) {
      const ctx = await browser.newContext();
      const p = await ctx.newPage();
      await setup(p, { phononReady: true }, theme);
      await p.goto("/");
      await geheZu(p, "Modelle");
      await p.setViewportSize({ width, height: width === 1440 ? 1000 : 900 });
      await expect(p.getByTestId("voice-gallery")).toBeVisible();
      const s = (n: string) =>
        p.screenshot({
          path: `${dir}/b-${n}-${label}-${width}.png`,
          fullPage: true,
        });
      await s("stimmen");
      await p.getByTestId("voice-layout-list").click();
      await s("liste");
      await p.getByTestId("voice-layout-cards").click();
      if (width === 390) {
        await p.getByTestId("voice-filter-toggle").click();
        await s("filter");
        await p.getByTestId("voice-filter-toggle").click();
      }
      await p.getByTestId("voice-view-providers").click();
      await s("anbieter");
      await p.getByTestId("tts-provider-open-phonon").click();
      await expect(p.getByTestId("voice-provider-sheet")).toBeVisible();
      await p.waitForTimeout(400);
      await p.screenshot({
        path: `${dir}/b-seitenblatt-${label}-${width}.png`,
      });
      await ctx.close();
    }
  }
});
