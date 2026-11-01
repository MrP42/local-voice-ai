import { expect, test, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { geheZu } from "./profilmenue";

// One-Click-Einrichtung von Phonon im Seitenblatt (Modelle › Vorlesen › Anbieter):
// Sprachwahl, Lizenz, Hugging-Face-Freigabe per Zwischenablage, Reihenfolge der
// Downloads, Fortschritt, macOS ohne Tresor (nur Englisch vorausgewählt), Klonen gesperrt.

const PROVIDERS = JSON.parse(
  readFileSync(
    resolve(process.cwd(), "tests", "fixtures", "tts-providers-real.json"),
    "utf8",
  ),
) as unknown[];

const row = (over: Record<string, unknown>) => ({
  id: "phonon-runtime",
  kind: "runtime",
  name: "Phonon program ptts (Windows x64)",
  description: "ptts",
  language: null,
  size_mb: 7,
  size_bytes: 7_766_316,
  is_downloaded: false,
  is_downloading: false,
  is_usable: false,
  unsupported_reason: null,
  license: "MIT OR Apache-2.0",
  license_url: "https://example.org/mit",
  license_non_commercial: false,
  engine: "phonon",
  gated: false,
  requires: null,
  source: "https://github.com/gradium-ai/xn-ptts/releases/tag/v0.4.2",
  sha256: null,
  license_note: null,
  bundled_voices: 0,
  bundled_voices_present: 0,
  ...over,
});
const DOWNLOADS = [
  row({}),
  row({
    id: "phonon-40m-de",
    kind: "model",
    name: "Phonon 40m (German, q8)",
    language: "de",
    size_bytes: 75_158_617 + 13 * 256_136,
    bundled_voices: 13,
    license: "CC-BY-NC-4.0",
    license_url: "https://huggingface.co/gradium/phonon-40m-de",
    license_non_commercial: true,
    gated: true,
    requires: "phonon-runtime",
    source:
      "hf://gradium/phonon-40m-de@79cfc7b47fcfc0b666657b981eb366415023a862",
  }),
  row({
    id: "phonon-31m-en",
    kind: "model",
    name: "Phonon 31m (English, q8)",
    language: "en",
    size_bytes: 65_116_929 + 37 * 256_136,
    bundled_voices: 37,
    license: "Apache-2.0",
    license_url: "https://huggingface.co/gradium/phonon-31m-en",
    requires: "phonon-runtime",
    source:
      "hf://gradium/phonon-31m-en@92741c5b07d085ff840e26fea8ce43110b87852f",
  }),
];

interface Opts {
  os?: "windows" | "macos";
  access?: boolean;
  /** Herkunft eines vorhandenen Zugangs (Standard: in der App abgelegt). */
  accessSource?: "app" | "system";
  verify?: "granted" | "denied" | "unreachable";
  /** Downloads bleiben hängen (Fortschritt/Abbrechen prüfen). */
  hang?: boolean;
  theme?: "light" | "dark";
  /** Schon eingerichtet (alle Downloads da, freigegeben). */
  installed?: boolean;
  /** Nur diese Downloads sind da (Anbieter freigegeben), z. B. Programm und Deutsch. */
  installedIds?: string[];
}

async function setup(page: Page, opts: Opts = {}) {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(
    (arg: { opts: Opts; providers: any[]; downloads: any[] }) => {
      const { opts, providers, downloads } = arg;
      const os = opts.os ?? "windows";
      const callbacks = new Map<number, (e: unknown) => void>();
      const listeners: Record<string, number[]> = {};
      let callback = 0;
      const w = window as unknown as Record<string, any>;
      w.__calls = [] as unknown[];
      w.__emit = (event: string, payload: unknown) => {
        for (const id of listeners[event] ?? [])
          callbacks.get(id)?.({ event, id, payload });
      };
      let access = opts.access === true;
      if (opts.installed || opts.installedIds) {
        for (const d of downloads) {
          if (opts.installedIds && !opts.installedIds.includes(d.id)) continue;
          d.is_downloaded = true;
          d.is_usable = true;
          d.bundled_voices_present = d.bundled_voices;
        }
      }
      const phonon = providers.find((p) => p.descriptor.id === "phonon");
      if (opts.installed || opts.installedIds) {
        Object.assign(phonon, {
          enabled: true,
          license_acked: true,
          blocked: null,
        });
        phonon.health = {
          ...phonon.health,
          available: true,
          reason: null,
          hint: null,
          version: "0.4.2",
        };
      }
      const settings: Record<string, unknown> = {
        onboarding_completed: true,
        app_language: "de",
        theme: opts.theme ?? "light",
        show_whats_new_on_update: false,
        debug_mode: false,
        selected_model: "",
        bindings: {},
        post_process_providers: [],
        post_process_prompts: [],
        custom_words: [],
        post_process_models: {},
        post_process_api_keys: {},
        push_to_talk: true,
        tts_voice: null,
        tts_engine: "piper",
        tts_piper_voice: null,
      };
      Object.assign(window, {
        __TAURI_OS_PLUGIN_INTERNALS__: {
          platform: os,
          os_type: os,
          family: os === "windows" ? "windows" : "unix",
          arch: "x86_64",
          version: "1",
          eol: "\n",
        },
        __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
        __TAURI_INTERNALS__: {
          metadata: {
            currentWindow: { label: "main" },
            currentWebview: { label: "main" },
          },
          transformCallback: (fn: (e: unknown) => void) => {
            callbacks.set(++callback, fn);
            return callback;
          },
          unregisterCallback: (id: number) => callbacks.delete(id),
          convertFileSrc: (path: string) => path,
          invoke: async (cmd: string, args?: Record<string, any>) => {
            (w.__calls as unknown[]).push([cmd, args]);
            if (cmd === "get_app_settings" || cmd === "get_default_settings")
              return settings;
            if (cmd === "plugin:os|locale") return "de-DE";
            if (cmd === "plugin:app|version") return "0.23.0";
            if (cmd.includes("permission")) return true;
            if (cmd === "plugin:event|listen") {
              (listeners[args?.event as string] ??= []).push(
                args?.handler as number,
              );
              return args?.handler;
            }
            if (cmd === "tts_server_status")
              return { phase: "stopped", message: null };
            if (cmd === "tts_runtime_status") return null;
            if (cmd === "tts_list_downloads")
              return JSON.parse(JSON.stringify(downloads));
            if (cmd === "tts_providers_list")
              return JSON.parse(JSON.stringify(providers));
            if (cmd === "tts_provider_health") return phonon.health;
            if (cmd === "tts_provider_get_config")
              return args?.id === "phonon"
                ? JSON.stringify({
                    exe_path: "",
                    model: "",
                    quant: "q8",
                    lang: "de",
                    voice: null,
                    threads: null,
                    hf_home: null,
                    allow_network: false,
                  })
                : null;
            if (cmd === "tts_provider_set_enabled") {
              phonon.enabled = args?.enabled === true;
              return null;
            }
            if (cmd === "tts_provider_acknowledge_license") {
              phonon.license_acked = true;
              phonon.blocked = null;
              return { fingerprint: args?.fingerprint, acknowledged_at: "x" };
            }
            if (cmd === "tts_hf_access_status")
              return {
                present: access,
                source: access ? (opts.accessSource ?? "app") : null,
                problem: null,
              };
            if (cmd === "tts_hf_access_from_clipboard") {
              access = true;
              return { present: true, source: "app", problem: null };
            }
            if (cmd === "tts_hf_access_verify")
              return access ? (opts.verify ?? "granted") : "missing";
            if (cmd === "tts_hf_access_delete") {
              access = false;
              return { present: false, source: null, problem: null };
            }
            if (cmd === "tts_download_model") {
              if (opts.hang) return new Promise(() => {});
              const d = downloads.find((x) => x.id === args?.id);
              if (d) {
                d.is_downloaded = true;
                d.is_usable = true;
                d.bundled_voices_present = d.bundled_voices;
              }
              setTimeout(
                () => w.__emit("model-download-complete", args?.id),
                0,
              );
              return null;
            }
            if (cmd === "phonon_gradium_key_status")
              return {
                supported: os === "windows",
                present: false,
                problem: null,
              };
            if (cmd === "phonon_own_voices_list")
              return {
                voices: [],
                // Wie das Backend: genau die installierten Phonon-Modelle.
                models: downloads
                  .filter(
                    (d: {
                      kind: string;
                      engine: string;
                      is_downloaded: boolean;
                    }) =>
                      d.engine === "phonon" &&
                      d.kind === "model" &&
                      d.is_downloaded,
                  )
                  .map((d: { id: string }) => d.id),
              };
            if (
              cmd.includes("history") ||
              cmd.includes("models") ||
              cmd.includes("devices") ||
              cmd === "meetings_list" ||
              cmd === "tts_list_voices" ||
              cmd === "tts_list_voice_infos" ||
              cmd === "tts_reading_list"
            )
              return [];
            return null;
          },
        },
      });
      try {
        localStorage.setItem("lva.ui.models.area", "voices");
        localStorage.setItem("lva.ui.models.voices.view", "providers");
      } catch {
        /* egal */
      }
    },
    {
      opts,
      providers: JSON.parse(JSON.stringify(PROVIDERS)),
      downloads: JSON.parse(JSON.stringify(DOWNLOADS)),
    },
  );
}

const calls = (page: Page) =>
  page.evaluate(
    () =>
      (window as unknown as { __calls: [string, Record<string, any>][] })
        .__calls,
  );
const named = async (page: Page, cmd: string) =>
  (await calls(page)).filter(([c]) => c === cmd);

/** WCAG-Kontrast zwischen Text- und Hintergrundfarbe eines Elements (sofort, ohne Warten). */
async function contrastOf(loc: import("@playwright/test").Locator) {
  return loc.evaluate((el) => {
    const c = getComputedStyle(el);
    const lum = (rgb: string) => {
      const [r, g, b] = (rgb.match(/[\d.]+/g) ?? []).slice(0, 3).map((v) => {
        const x = Number(v) / 255;
        return x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4;
      });
      return 0.2126 * r + 0.7152 * g + 0.0722 * b;
    };
    const a = lum(c.color);
    const b = lum(c.backgroundColor);
    return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
  });
}

async function openSetup(page: Page) {
  await page.goto("/");
  await geheZu(page, "Modelle");
  await page.getByTestId("tts-provider-setup-phonon").click();
  const setupPanel = page.getByTestId("phonon-setup");
  await expect(setupPanel).toBeVisible();
  return setupPanel;
}

test("Windows: ein Klick lädt Programm, Deutsch und Englisch mit allen Stimmen, gibt frei und bestätigt die Lizenz", async ({
  page,
}) => {
  await setup(page);
  const panel = await openSetup(page);
  await expect(panel).toContainText("Programm ptts");
  await expect(panel).toContainText("13 Stimmen");
  await expect(panel).toContainText("37 Stimmen");
  const de = page.getByTestId("phonon-setup-choose-phonon-40m-de");
  const en = page.getByTestId("phonon-setup-choose-phonon-31m-en");
  await expect(de).toBeChecked();
  await expect(en).toBeChecked();
  // Keine Pfade, kein Textfeld.
  await expect(panel.locator("input[type=text], textarea")).toHaveCount(0);

  const start = page.getByTestId("phonon-setup-start");
  await expect(start).toBeDisabled();
  await expect(page.getByTestId("phonon-setup-why")).toContainText("Lizenz");
  await page.getByTestId("phonon-setup-license").check();
  await expect(page.getByTestId("phonon-setup-why")).toContainText("Freigabe");

  // Zwischenablage nur auf Klick; danach wird der Zugang geprüft.
  expect(await named(page, "tts_hf_access_from_clipboard")).toHaveLength(0);
  await page.getByTestId("phonon-setup-access-paste").click();
  await expect(page.getByTestId("phonon-setup-access")).toContainText(
    "Freigabe geprüft",
  );
  const fromClipboard = await named(page, "tts_hf_access_from_clipboard");
  expect(fromClipboard).toEqual([
    ["tts_hf_access_from_clipboard", { allowFile: false }],
  ]);
  expect(await named(page, "tts_hf_access_verify")).toEqual([
    ["tts_hf_access_verify", { modelId: "phonon-40m-de" }],
  ]);

  await expect(start).toBeEnabled();
  await expect(start).toContainText(/Einrichten \(1\d\d MB\)/);
  // Primärknopf: Weiß auf Markenfarbe mindestens 4,5:1, ohne Überblendung aus dem Sperrzustand.
  expect(await contrastOf(start)).toBeGreaterThanOrEqual(4.5);
  await start.click();
  await expect(page.getByTestId("phonon-setup-status")).toContainText(
    "50 Stimmen",
  );
  const order = (await calls(page))
    .map(([c, a]) =>
      c === "tts_download_model"
        ? a.id
        : c === "tts_provider_set_enabled" ||
            c === "tts_provider_acknowledge_license"
          ? c
          : null,
    )
    .filter(Boolean);
  expect(order).toEqual([
    "tts_provider_set_enabled",
    "phonon-runtime",
    "phonon-40m-de",
    "phonon-31m-en",
    "tts_provider_acknowledge_license",
  ]);
});

test("Nur Englisch: Deutsch abwählen braucht keine Freigabe, Größe und Stimmenzahl passen sich an", async ({
  page,
}) => {
  await setup(page);
  const panel = await openSetup(page);
  const start = page.getByTestId("phonon-setup-start");
  const before = await start.textContent();
  await page.getByTestId("phonon-setup-choose-phonon-40m-de").uncheck();
  await expect(page.getByTestId("phonon-setup-access")).toHaveCount(0);
  await expect(start).not.toHaveText(before ?? "");
  await page.getByTestId("phonon-setup-license").check();
  await start.click();
  await expect(page.getByTestId("phonon-setup-status")).toContainText(
    "37 Stimmen",
  );
  const ids = (await named(page, "tts_download_model")).map(([, a]) => a.id);
  expect(ids).toEqual(["phonon-runtime", "phonon-31m-en"]);
  expect(await named(page, "tts_hf_access_from_clipboard")).toHaveLength(0);
  await expect(panel).toHaveAttribute("data-complete", "true");
});

test("Freigabe abgelehnt: verständliche Meldung, Einrichten bleibt gesperrt", async ({
  page,
}) => {
  await setup(page, { verify: "denied" });
  await openSetup(page);
  await page.getByTestId("phonon-setup-license").check();
  await page.getByTestId("phonon-setup-access-paste").click();
  await expect(
    page.getByTestId("phonon-setup-access").getByRole("alert"),
  ).toContainText("verweigert");
  await expect(page.getByTestId("phonon-setup-start")).toBeDisabled();
});

test("Fortschritt als progressbar, Abbrechen ruft den Abbruch", async ({
  page,
}) => {
  await setup(page, { hang: true });
  await openSetup(page);
  await page.getByTestId("phonon-setup-choose-phonon-40m-de").uncheck();
  await page.getByTestId("phonon-setup-license").check();
  await page.getByTestId("phonon-setup-start").click();
  await expect(page.getByTestId("phonon-setup-status")).toContainText(
    "Programm ptts",
  );
  await page.evaluate(() =>
    (window as unknown as { __emit: (e: string, p: unknown) => void }).__emit(
      "model-download-progress",
      {
        model_id: "phonon-runtime",
        downloaded: 40,
        total: 100,
        percentage: 40,
      },
    ),
  );
  const bar = page.getByRole("progressbar", { name: /Programm ptts/ });
  await expect(bar).toHaveAttribute("aria-valuenow", "40");
  await page.getByTestId("phonon-setup-cancel-phonon-runtime").click();
  expect(await named(page, "tts_cancel_download")).toEqual([
    ["tts_cancel_download", { id: "phonon-runtime" }],
  ]);
});

test("macOS ohne Tresor: nur Englisch vorausgewählt; Deutsch erst nach Zustimmung zur Klartext-Datei; Klonen gesperrt mit Grund", async ({
  page,
}) => {
  await setup(page, { os: "macos" });
  await openSetup(page);
  const de = page.getByTestId("phonon-setup-choose-phonon-40m-de");
  await expect(de).not.toBeChecked();
  await expect(de).toBeDisabled();
  await expect(
    page.getByTestId("phonon-setup-choose-phonon-31m-en"),
  ).toBeChecked();
  await expect(
    page.getByTestId("phonon-setup-mac-hint-phonon-40m-de"),
  ).toContainText("Klartext");
  await page.getByTestId("phonon-setup-access-file").check();
  await expect(de).toBeEnabled();
  await de.check();
  await page.getByTestId("phonon-setup-access-paste").click();
  expect(await named(page, "tts_hf_access_from_clipboard")).toEqual([
    ["tts_hf_access_from_clipboard", { allowFile: true }],
  ]);

  // Eigene Stimmen: Klonen ist ohne Tresor gesperrt, der Grund steht daneben.
  await page.getByTestId("phonon-own-voices-toggle").click();
  await expect(page.getByTestId("phonon-create-button")).toBeDisabled();
  await expect(page.getByTestId("phonon-clone-blocked")).toContainText(
    "Schlüsselbund",
  );
  await expect(page.getByTestId("phonon-import-button")).toBeEnabled();
});

test("Bilder für die Abnahme (nur mit SCREENS_DIR)", async ({ page }) => {
  const dir = process.env.SCREENS_DIR;
  test.skip(!dir, "nur mit SCREENS_DIR");
  for (const theme of ["light", "dark"] as const) {
    const hell = theme === "light" ? "hell" : "dunkel";
    await page.setViewportSize({ width: 1440, height: 1000 });
    await setup(page, { theme });
    await page.goto("/");
    await geheZu(page, "Modelle");
    await expect(page.getByTestId("voice-providers-view")).toBeVisible();
    await page.screenshot({ path: `${dir}/01-anbieterliste-${hell}.png` });
    await page.getByTestId("tts-provider-setup-phonon").click();
    await expect(page.getByTestId("phonon-setup")).toBeVisible();
    await page.screenshot({ path: `${dir}/02-einrichten-${hell}.png` });
    await page.getByTestId("phonon-setup-license").check();
    await page.getByTestId("phonon-setup-access-paste").click();
    await expect(page.getByTestId("phonon-setup-start")).toBeEnabled();
    expect(
      await contrastOf(page.getByTestId("phonon-setup-start")),
    ).toBeGreaterThanOrEqual(4.5);
    await page.screenshot({ path: `${dir}/03-freigabe-geprueft-${hell}.png` });
  }
});

test("Bilder: laufend, fertig, macOS, schmal (nur mit SCREENS_DIR)", async ({
  page,
  browser,
}) => {
  test.setTimeout(240_000);
  const dir = process.env.SCREENS_DIR;
  test.skip(!dir, "nur mit SCREENS_DIR");
  await page.setViewportSize({ width: 1440, height: 1000 });
  await setup(page, { hang: true });
  await openSetup(page);
  await page.getByTestId("phonon-setup-choose-phonon-40m-de").uncheck();
  await page.getByTestId("phonon-setup-license").check();
  await page.getByTestId("phonon-setup-start").click();
  await page.evaluate(() =>
    (window as unknown as { __emit: (e: string, p: unknown) => void }).__emit(
      "model-download-progress",
      {
        model_id: "phonon-runtime",
        downloaded: 4_200_000,
        total: 7_766_316,
        percentage: 54,
      },
    ),
  );
  await expect(page.getByRole("progressbar")).toBeVisible();
  await page.screenshot({ path: `${dir}/04-laeuft-hell.png` });

  for (const theme of ["light", "dark"] as const) {
    const ctx = await browser.newContext({
      viewport: { width: 1440, height: 1000 },
    });
    const p = await ctx.newPage();
    await setup(p, { installed: true, theme });
    await p.goto("/");
    await geheZu(p, "Modelle");
    await p.getByTestId("tts-provider-open-phonon").click();
    await expect(p.getByTestId("phonon-setup")).toHaveAttribute(
      "data-complete",
      "true",
    );
    await p.screenshot({
      path: `${dir}/05-bereit-${theme === "light" ? "hell" : "dunkel"}.png`,
    });
    if (theme === "light") {
      await p.getByTestId("tts-provider-advanced-phonon").click();
      await p.getByTestId("phonon-own-voices-toggle").click();
      await p.getByTestId("voice-provider-sheet").evaluate((el) => {
        const body = el.closest(".fui-DrawerBody") ?? el.parentElement;
        body?.scrollTo(0, 99999);
      });
      await p.screenshot({
        path: `${dir}/06-erweitert-eigene-stimmen-hell.png`,
      });
    }
    await ctx.close();
  }

  const mac = await browser.newContext({
    viewport: { width: 1440, height: 1000 },
  });
  const m = await mac.newPage();
  await setup(m, { os: "macos" });
  await openSetup(m);
  await m.screenshot({ path: `${dir}/07-macos-nur-englisch-hell.png` });
  await mac.close();
});

test("Bild: schmal 390 px ohne waagerechte Scrollleiste (nur mit SCREENS_DIR)", async ({
  page,
}) => {
  const dir = process.env.SCREENS_DIR;
  test.skip(!dir, "nur mit SCREENS_DIR");
  await setup(page);
  await openSetup(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: `${dir}/08-schmal-390-einrichten.png` });
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth > window.innerWidth,
  );
  expect(overflow).toBe(false);
});

test("Tastatur und Screenreader: Sprachen per Leertaste, Labels, Status als Live-Region, Escape gibt den Fokus zurück", async ({
  page,
}) => {
  await setup(page, { access: true });
  await openSetup(page);
  const en = page.getByTestId("phonon-setup-choose-phonon-31m-en");
  // Echte Checkbox mit zugeordnetem Label (for/id): per Name erreichbar.
  await expect(
    page.getByRole("checkbox", { name: /Englisch · Phonon 31m/ }),
  ).toBeVisible();
  await en.focus();
  await page.keyboard.press("Space");
  await expect(en).not.toBeChecked();
  await page.keyboard.press("Space");
  await expect(en).toBeChecked();
  const license = page.getByTestId("phonon-setup-license");
  await license.focus();
  await page.keyboard.press("Space");
  await expect(license).toBeChecked();
  await expect(page.getByTestId("phonon-setup-status")).toHaveAttribute(
    "aria-live",
    "polite",
  );
  // Vorhandener Zugang wird ohne Klick einmal geprüft.
  await expect(page.getByTestId("phonon-setup-access")).toHaveAttribute(
    "data-check",
    "granted",
  );
  const start = page.getByTestId("phonon-setup-start");
  await start.focus();
  await expect(start).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("phonon-setup")).toHaveCount(0);
  await expect(page.getByTestId("tts-provider-setup-phonon")).toBeFocused();
});

test("Doppelklick auf Einrichten startet genau einen Lauf", async ({
  page,
}) => {
  await setup(page, { access: true });
  await openSetup(page);
  await page.getByTestId("phonon-setup-license").check();
  await expect(page.getByTestId("phonon-setup-access")).toHaveAttribute(
    "data-check",
    "granted",
  );
  await page.getByTestId("phonon-setup-start").dblclick();
  await expect(page.getByTestId("phonon-setup-status")).toContainText(
    "50 Stimmen",
  );
  const ids = (await named(page, "tts_download_model")).map(([, a]) => a.id);
  expect(ids).toEqual(["phonon-runtime", "phonon-40m-de", "phonon-31m-en"]);
});

test("Zugang aus dem System (hf auth): kein „Zugang entfernen“, stattdessen Hinweis", async ({
  page,
}) => {
  await setup(page, { access: true, accessSource: "system" });
  await openSetup(page);
  await expect(page.getByTestId("phonon-setup-access")).toHaveAttribute(
    "data-check",
    "granted",
  );
  await expect(page.getByTestId("phonon-setup-access-remove")).toHaveCount(0);
  await expect(page.getByTestId("phonon-setup-access-system")).toContainText(
    "hf auth",
  );
});

test("Deutsch schon da: Englisch ist vorgewählt und wird mit einem Klick nachgeladen", async ({
  page,
}) => {
  await setup(page, { installedIds: ["phonon-runtime", "phonon-40m-de"] });
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Modelle", exact: true })
    .click();
  await page.getByTestId("tts-provider-open-phonon").click();
  const panel = page.getByTestId("phonon-setup");
  await expect(panel).toHaveAttribute("data-complete", "false");
  await expect(
    page.getByTestId("phonon-setup-choose-phonon-31m-en"),
  ).toBeChecked();
  // Englisch ist Apache-2.0 und nicht gesperrt: keine Lizenzbestätigung, kein Zugang nötig.
  await expect(page.getByTestId("phonon-setup-license")).toHaveCount(0);
  await expect(page.getByTestId("phonon-setup-access")).toHaveCount(0);
  await page.getByTestId("phonon-setup-start").click();
  const ids = (await named(page, "tts_download_model")).map(([, a]) => a.id);
  expect(ids).toEqual(["phonon-31m-en"]);
  await expect(panel).toHaveAttribute("data-complete", "true");
});

test("Eingerichtet: Status, eigene Stimmen und Aktionen zeigen denselben Stand", async ({
  page,
}) => {
  await setup(page, { installed: true });
  await page.goto("/");
  await geheZu(page, "Modelle");
  await page.getByTestId("tts-provider-open-phonon").click();
  await expect(page.getByTestId("phonon-setup")).toHaveAttribute(
    "data-complete",
    "true",
  );
  await page.getByTestId("phonon-own-voices-toggle").click();
  const own = page.getByTestId("phonon-own-voices");
  await expect(own).not.toContainText("kein Phonon-Modell installiert");
  await expect(
    own.getByRole("button", { name: /Stimmdatei importieren/ }),
  ).toBeEnabled();
});
