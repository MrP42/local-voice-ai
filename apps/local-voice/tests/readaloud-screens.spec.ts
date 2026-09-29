import { test, expect, type Page } from "@playwright/test";
import * as fs from "node:fs";
import * as path from "node:path";

// Vorher/Nachher-Aufnahmen der Vorlesen-Seite (Goal ui-vorlesen-kompakt, P1).
// Kein Verhaltenstest: laeuft nur mit SCREENS_DIR und legt dort vier PNGs
// plus je eine Messwert-Datei ab. Dieselbe Spec erzeugt nach dem Umbau die
// Nachher-Bilder -- deshalb greift sie nur ueber Dinge zu, die den Umbau
// ueberleben: Navigationsknopf "Vorlesen", Klappbereich "Ausdruck &
// Sprechstil", Filter "Alle", Dateiname in der Dateiliste. Keine Knoepfe
// der Bedienspalte.
//
//   SCREENS_DIR=../../koordination/ui-vorlesen-kompakt/screens/vorher \
//   LV_DEV_PORT=1611 pnpm exec playwright test readaloud-screens --reporter=line

const SCREENS_DIR = process.env.SCREENS_DIR;
test.skip(!SCREENS_DIR, "nur mit SCREENS_DIR (Aufnahmen, kein Verhaltenstest)");

const PAGE_TITLE = "CASE-GESPRÄCH IE2S · 28.09.2026";
const FILE_STEM = "CASE-GESPRÄCH-IE2S-·-28.09.2026";
const FILES = [
  { name: `${FILE_STEM}_2026-09-28_1736.wav`, mb: 34.6 },
  { name: `${FILE_STEM}_2026-09-28_1712.wav`, mb: 33.1 },
  { name: `${FILE_STEM}_2026-09-28_1649.wav`, mb: 31.8 },
  { name: `${FILE_STEM}-Zusammenfassung_2026-09-28_1631.wav`, mb: 30.2 },
  { name: `${FILE_STEM}-EN_2026-09-28_1604.wav`, mb: 32.7 },
  { name: `${FILE_STEM}_2026-09-28_1540.wav`, mb: 35.0 },
];

// ~30 Absaetze: genug, dass der Editor scrollt und die Palette ihn
// wirklich verdraengen kann.
const TOPICS = [
  "Ausgangslage beim Kunden",
  "Energiefluss im Werk",
  "Lastspitzen am Vormittag",
  "Speicher und Eigenverbrauch",
  "Netzentgelte und Leistungspreis",
  "Datenlage aus den Zählern",
  "Priorisierung der Maßnahmen",
  "Wirtschaftlichkeit",
  "Risiken der Umsetzung",
  "Nächste Schritte",
];
const TEXT = Array.from({ length: 30 }, (_, index) => {
  const topic = TOPICS[index % TOPICS.length];
  const tag = index % 7 === 3 ? "[Entspannt] " : "";
  return (
    `${tag}${index + 1}. ${topic}: Im Gespräch ging es zunächst darum, welche ` +
    `Annahmen belastbar sind und welche wir erst noch prüfen müssen. Der ` +
    `Kunde nannte dazu konkrete Zahlen aus dem letzten Quartal, die wir ` +
    `später gegen die Messreihen halten.`
  );
}).join("\n\n");

const VIEWPORTS = {
  "1920": { width: 1920, height: 1050 },
  "1366": { width: 1366, height: 768 },
} as const;

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(
    ({ title, text, files }) => {
      const callbacks = new Map<number, unknown>();
      let callback = 0;
      const now = Date.now();
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
        tts_script_check: true,
      };
      const voices = [
        {
          id: "patrick",
          meta: { display_name: "Patrick", tags: [] },
          origin: "recorded",
          avatar_path: null,
        },
        {
          id: "erzaehlerin",
          meta: { display_name: "Erzählerin", tags: [] },
          origin: "recorded",
          avatar_path: null,
        },
      ];
      const pages = [
        {
          id: "p1",
          title,
          modified_ms: now - 20 * 60_000,
          preview: text.slice(0, 80),
        },
        {
          id: "p2",
          title: "Angebot Netzpuls – Entwurf",
          modified_ms: now - 26 * 3_600_000,
          preview: "Sehr geehrte Damen und Herren, anbei unser Angebot",
        },
        {
          id: "p3",
          title: "Hörspiel: Der Sturm",
          modified_ms: now - 5 * 86_400_000,
          preview: "<Erzählerin> Es war eine dunkle Nacht.",
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
          convertFileSrc: (p: string) => p,
          invoke: async (cmd: string, args?: Record<string, unknown>) => {
            if (cmd === "get_app_settings" || cmd === "get_default_settings")
              return settings;
            if (cmd === "plugin:os|locale") return "de-DE";
            if (cmd === "plugin:app|version") return "0.20.3";
            if (cmd.includes("permission")) return true;
            if (cmd === "plugin:event|listen") return ++callback;
            if (cmd === "get_selected_model") return "";
            if (cmd === "meetings_is_recording") return false;
            if (cmd === "tts_server_status")
              return { phase: "stopped", message: null };
            if (cmd === "tts_list_voice_infos") return voices;
            if (cmd === "tts_list_voices") return voices.map((v) => v.id);
            if (cmd === "pages_list") return pages;
            if (cmd === "page_state_load")
              return args?.id === "p1"
                ? JSON.stringify({
                    text,
                    summary: "",
                    sourceUrl: "",
                    tab: "original",
                  })
                : JSON.stringify({ text: "", tab: "original" });
            if (cmd === "page_state_save") return null;
            if (cmd === "page_dir") return "C:/projects/p1";
            if (cmd === "page_files")
              return files.map((file, index) => ({
                name: file.name,
                size: Math.round(file.mb * 1024 * 1024),
                modified_ms: now - index * 1_500_000,
              }));
            if (cmd === "page_audio_note")
              return {
                text: text.slice(0, 400),
                voice: "patrick",
                seed: 42,
                created_ms: now - 20 * 60_000,
                segments: text
                  .split("\n\n")
                  .slice(0, 6)
                  .map((segment, index) => ({
                    text: segment,
                    voice: "patrick",
                    start_ms: index * 9000,
                    end_ms: (index + 1) * 9000,
                  })),
              };
            if (cmd === "books_list") return [];
            if (cmd === "llm_ps" || cmd === "tts_reading_list") return [];
            if (cmd === "tts_list_downloads") return [];
            if (cmd === "get_custom_sounds")
              return { start: false, stop: false };
            if (
              cmd.includes("history") ||
              cmd.includes("models") ||
              cmd.includes("devices") ||
              cmd === "meetings_list"
            )
              return [];
            return null;
          },
        },
      });
    },
    { title: PAGE_TITLE, text: TEXT, files: FILES },
  );
});

async function openReadAloud(page: Page, viewport: keyof typeof VIEWPORTS) {
  await page.setViewportSize(VIEWPORTS[viewport]);
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  // Seite geladen: Text im Editor und die erste Datei in der Liste.
  await expect(page.locator("textarea").first()).toHaveValue(TEXT);
  await expect(fileRow(page, FILES[0].name)).toBeVisible();
  // Schriften und Chips setzen sich; ohne Pause wandert das Layout noch.
  await page.waitForTimeout(400);
}

/** Zeile der Dateiliste: ueber den vollen Namen (Text oder Tooltip). */
function fileRow(page: Page, name: string) {
  return page
    .locator("div", { hasText: name })
    .or(page.locator("div", { has: page.locator(`[title="${name}"]`) }))
    .last();
}

/**
 * Messwerte ohne feste Selektoren: jedes sichtbare Bedienelement mit Box,
 * Schriftgroesse und Symbolgroesse, zugeordnet zur Spalte, in der es steht.
 */
async function measure(page: Page) {
  return page.evaluate(() => {
    const region = (el: Element) => {
      const hit = el.closest(
        ".tts-workspace__pages, .tts-editor, .tts-controls, .tts-workspace__files, nav, header",
      );
      if (!hit) return "sonst";
      if (hit.matches(".tts-workspace__pages")) return "seiten";
      if (hit.matches(".tts-editor")) return "editor";
      if (hit.matches(".tts-controls")) return "bedienung";
      if (hit.matches(".tts-workspace__files")) return "dateien";
      return hit.tagName.toLowerCase();
    };
    const box = (el: Element) => {
      const r = el.getBoundingClientRect();
      return {
        x: Math.round(r.x),
        y: Math.round(r.y),
        w: Math.round(r.width),
        h: Math.round(r.height),
      };
    };
    const visible = (el: Element) => {
      const r = el.getBoundingClientRect();
      if (r.width === 0 || r.height === 0) return false;
      // Chromium legt fuer den Inhalt geschlossener <details> weiter Boxen
      // an -- sichtbar ist dort nur die summary.
      const closed = el.closest("details:not([open])");
      return !closed || !!el.closest("summary");
    };
    const controls = Array.from(
      document.querySelectorAll(
        "main button, main summary, main [role=tab], main input, main textarea, main [role=combobox]",
      ),
    )
      .filter(visible)
      .map((el) => {
        const svg = el.querySelector("svg");
        const cs = getComputedStyle(el);
        return {
          region: region(el),
          tag: el.tagName.toLowerCase(),
          label: (
            el.getAttribute("aria-label") ||
            (el as HTMLElement).innerText ||
            el.getAttribute("placeholder") ||
            ""
          )
            .trim()
            .replace(/\s+/g, " ")
            .slice(0, 50),
          ...box(el),
          fontSize: cs.fontSize,
          fontWeight: cs.fontWeight,
          svg: svg
            ? `${Math.round(svg.getBoundingClientRect().width)}x${Math.round(svg.getBoundingClientRect().height)}`
            : null,
          lucide:
            svg
              ?.getAttribute("class")
              ?.match(/lucide-[a-z0-9-]+/g)
              ?.find((c) => c !== "lucide-icon") ?? null,
        };
      });
    // Schriftgroessen aller sichtbaren Textknoten der Seite (je Spalte).
    const fonts: Record<string, Record<string, number>> = {};
    // Kleinschrift (< 12 px) einzeln, mit Farbe: dort sitzen die Ausreisser.
    const smallText: {
      region: string;
      text: string;
      fontSize: string;
      lineHeight: string;
      color: string;
    }[] = [];
    const walker = document.createTreeWalker(
      document.querySelector("main") ?? document.body,
      NodeFilter.SHOW_TEXT,
    );
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      const parent = node.parentElement;
      if (!parent || !node.textContent?.trim() || !visible(parent)) continue;
      const key = region(parent);
      const cs = getComputedStyle(parent);
      fonts[key] ??= {};
      fonts[key][cs.fontSize] = (fonts[key][cs.fontSize] ?? 0) + 1;
      if (parseFloat(cs.fontSize) < 12 && smallText.length < 60)
        smallText.push({
          region: key,
          text: node.textContent.trim().slice(0, 30),
          fontSize: cs.fontSize,
          lineHeight: cs.lineHeight,
          color: cs.color,
        });
    }
    const columns = Object.fromEntries(
      [
        ".tts-workspace__pages",
        ".tts-editor",
        ".tts-controls",
        ".tts-workspace__files",
      ].map((sel) => {
        const el = document.querySelector(sel);
        return [sel, el ? box(el) : null];
      }),
    );
    const disclosure = Array.from(document.querySelectorAll("details")).find(
      (d) => d.querySelector("summary")?.textContent?.includes("Ausdruck"),
    );
    const textarea = document.querySelector("main textarea");
    return {
      viewport: { w: window.innerWidth, h: window.innerHeight },
      columns,
      editorTextarea: textarea ? box(textarea) : null,
      disclosure: disclosure
        ? {
            ...box(disclosure),
            open: disclosure.open,
            scrollHeight: disclosure.scrollHeight,
            clientHeight: disclosure.clientHeight,
          }
        : null,
      fonts,
      smallText,
      controls,
    };
  });
}

function save(name: string, data: unknown) {
  const dir = path.resolve(SCREENS_DIR!);
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(
    path.join(dir, `${name}.json`),
    JSON.stringify(data, null, 2),
    "utf8",
  );
  return path.join(dir, `${name}.png`);
}

for (const viewport of ["1920", "1366"] as const) {
  test(`uebersicht ${viewport}`, async ({ page }) => {
    await openReadAloud(page, viewport);
    const file = save(`uebersicht-${viewport}`, await measure(page));
    await page.screenshot({ path: file });
  });
}

test("palette alle 1920", async ({ page }) => {
  await openReadAloud(page, "1920");
  await page.getByText("Ausdruck & Sprechstil", { exact: true }).click();
  await page.getByRole("tab", { name: "Alle" }).click();
  await page.waitForTimeout(300);
  const file = save("palette-alle-1920", await measure(page));
  await page.screenshot({ path: file });
});

// Vergleichswerte fuer den App-Standard: dieselbe Messung auf den anderen
// Seiten (nur JSON, keine Bilder).
test("vergleich andere seiten", async ({ page }) => {
  await openReadAloud(page, "1920");
  const result: Record<string, unknown> = {};
  for (const name of ["Verlauf", "Modelle", "Einstellungen"]) {
    await page
      .getByRole("navigation")
      .getByRole("button", { name, exact: true })
      .click();
    await page.waitForTimeout(400);
    result[name] = await measure(page);
  }
  save("vergleich-andere-seiten", result);
});

test("datei abspielen 1920", async ({ page }) => {
  await openReadAloud(page, "1920");
  await fileRow(page, FILES[0].name)
    .getByRole("button", { name: "Anhören" })
    .click();
  await expect(page.locator("audio")).toHaveCount(1);
  await page.waitForTimeout(300);
  const file = save("datei-abspielen-1920", await measure(page));
  await page.screenshot({ path: file });
});
