import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { test, expect, type Page } from "@playwright/test";

// Ausschließlich erfundener Inhalt; das Emoji liegt vor allen Änderungen.
const BASIS = "😀 Fiktiva sagt: „Die ﬁktive Idee...“";
const ERGEBNIS = '😀 Fiktiva sagt: "Die fiktive Idee…"';
const REGELN = ["ligaturen", "anfuehrungszeichen", "auslassung"] as const;
const SHOTS = process.env.AUFBEREITEN_SHOTS === "1";
const SHOT_DIR = process.env.LV_SHOT_DIR ?? "docs/vorschau/aufbereiten-a2";
type Theme = "light" | "dark";

async function bruecke(page: Page, theme: Theme = "light") {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.emulateMedia({ colorScheme: theme });
  await page.addInitScript(
    ({ theme }) => {
      const callbacks = new Map<number, unknown>();
      let callback = 0;
      const calls: string[] = [];
      const zustand = {
        leer: false,
        veraltet: false,
        entwürfe: 0,
        original: null as string | null,
      };
      Object.assign(window, { __calls: calls, __aufbereiten: zustand });
      const settings = {
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
        tts_voice: null,
        tts_engine: "piper",
        tts_piper_voice: null,
        tts_script_check: true,
      };
      type Aenderung = {
        id: string;
        art: { regel: string };
        bereich: [number, number];
        alt: string;
        neu: string;
        grund_code: string;
        zaehler: null;
        gruppe: string;
        status: string;
      };
      let basis = "";
      let bereiche: [number, number][] = [];
      let entwurf = {
        id: "entwurf-0",
        status: "entwurf",
        basis: { sha256: "x", laenge: 0 },
        aenderungen: [] as Aenderung[],
      };
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
            calls.push(cmd);
            switch (cmd) {
              case "aufbereitung_entwurf_anlegen": {
                basis = String(args?.text);
                const regeln = args?.regeln as string[];
                const kandidaten = [
                  { regel: "ligaturen", alt: "ﬁ", neu: "fi" },
                  { regel: "anfuehrungszeichen", alt: "„", neu: '"' },
                  { regel: "anfuehrungszeichen", alt: "“", neu: '"' },
                  { regel: "auslassung", alt: "...", neu: "…" },
                ]
                  .filter((a) => !zustand.leer && regeln.includes(a.regel))
                  .map((a) => ({ ...a, start: basis.indexOf(a.alt) }))
                  .filter((a) => a.start >= 0)
                  .sort((a, b) => a.start - b.start);
                bereiche = kandidaten.map((a) => [
                  a.start,
                  a.start + a.alt.length,
                ]);
                const encoder = new TextEncoder();
                entwurf = {
                  id: `entwurf-${++zustand.entwürfe}`,
                  status: "entwurf",
                  basis: { sha256: "x", laenge: basis.length },
                  aenderungen: kandidaten.map((a, i) => ({
                    id: `aenderung-${i}`,
                    art: { regel: a.regel },
                    bereich: [
                      encoder.encode(basis.slice(0, a.start)).length,
                      encoder.encode(basis.slice(0, a.start + a.alt.length))
                        .length,
                    ],
                    alt: a.alt,
                    neu: a.neu,
                    grund_code: a.regel,
                    zaehler: null,
                    gruppe: `${a.regel}#1`,
                    status: "offen",
                  })),
                };
                return structuredClone(entwurf);
              }
              case "aufbereitung_utf16_bereiche":
                return structuredClone(bereiche);
              case "aufbereitung_entscheiden":
                entwurf.aenderungen = entwurf.aenderungen.map((a) =>
                  a.gruppe === args?.gruppe
                    ? { ...a, status: String(args?.entscheidung) }
                    : a,
                );
                return structuredClone(entwurf);
              case "aufbereitung_uebernehmen": {
                if (zustand.veraltet) {
                  zustand.veraltet = false;
                  entwurf.status = "veraltet";
                  return {
                    text: null,
                    aufbereitung: structuredClone(entwurf),
                    warnung: null,
                  };
                }
                zustand.original ??= basis;
                let text = basis;
                // Rückwärts ersetzen, damit die UTF-16-Grenzen gültig bleiben.
                for (let i = entwurf.aenderungen.length - 1; i >= 0; i--) {
                  const a = entwurf.aenderungen[i];
                  if (a.status !== "angenommen") continue;
                  const [start, ende] = bereiche[i];
                  text = text.slice(0, start) + a.neu + text.slice(ende);
                }
                entwurf.status = "aktiv";
                return {
                  text,
                  aufbereitung: structuredClone(entwurf),
                  warnung: null,
                };
              }
              case "aufbereitung_rueckgaengig":
                return { text: basis, original: false, warnung: null };
              case "aufbereitung_original":
                return zustand.original;
              case "aufbereitung_liste":
                return [];
              case "page_state_save":
                sessionStorage.setItem(`__ps_${args?.id}`, String(args?.state));
                return null;
              case "page_state_load":
                return sessionStorage.getItem(`__ps_${args?.id}`);
              case "get_app_settings":
              case "get_default_settings":
                return settings;
              case "plugin:os|locale":
                return "de-DE";
              case "plugin:app|version":
                return "0.16.0";
              case "plugin:event|listen":
                return ++callback;
              case "get_selected_model":
                return "";
              case "meetings_is_recording":
                return false;
              case "tts_server_status":
                return { phase: "stopped", message: null };
              case "pages_list":
                return [{ id: "p1", title: "Fiktivas Idee" }];
              case "books_list":
              case "library_list":
              case "books_templates":
              case "books_memory_read":
              case "page_files":
              case "tts_list_downloads":
              case "tts_list_voices":
              case "tts_list_voice_infos":
              case "llm_ps":
              case "tts_reading_list":
              case "meetings_list":
                return [];
              case "page_dir":
                return "C:/projects/p1";
              case "get_custom_sounds":
                return { start: false, stop: false };
            }
            if (cmd.includes("permission")) return true;
            if (
              cmd.includes("history") ||
              cmd.includes("models") ||
              cmd.includes("devices")
            )
              return [];
            return null;
          },
        },
      });
    },
    { theme },
  );
}

async function openEditor(page: Page, text = BASIS, theme: Theme = "light") {
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  // Schmale Fenster: Navigation im Drawer („Menü öffnen“).
  if ((page.viewportSize()?.width ?? 1280) < 768) {
    await page.getByRole("button", { name: "Menü öffnen" }).click();
  }
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  const editor = page.locator("textarea").first();
  await editor.fill(text);
  await expect(page.getByTestId("editor-menu")).toBeVisible();
  return editor;
}

async function oeffnen(page: Page) {
  await page.getByTestId("editor-menu-cleanup").click();
  await page.getByTestId("editor-menu-aufbereiten").click();
  const dialog = page.getByTestId("aufbereiten-dialog");
  await expect(dialog).toBeVisible();
  // Der Fußbereich liegt außerhalb des Testid-Containers.
  await expect(page.getByTestId("aufbereiten-uebernehmen")).toBeEnabled();
  return dialog;
}

async function modus(page: Page, werte: { leer?: boolean; veraltet?: boolean }) {
  await page.evaluate((werte) => {
    const w = window as unknown as {
      __aufbereiten: { leer: boolean; veraltet: boolean };
    };
    Object.assign(w.__aufbereiten, werte);
  }, werte);
}

async function status(page: Page, regel: string, wert: string) {
  await expect(
    page.getByTestId(`aufbereiten-gruppe-${regel}`).locator("p").first(),
  ).toContainText(wert);
}

async function alleAnnehmen(page: Page) {
  await page.getByTestId("aufbereiten-alle-annehmen").click();
  for (const regel of REGELN) await status(page, regel, "Angenommen");
  await expect(page.getByTestId("aufbereiten-uebernehmen")).toBeEnabled();
}

async function anwenden(page: Page) {
  await alleAnnehmen(page);
  await page.getByTestId("aufbereiten-uebernehmen").click();
  await expect(page.getByTestId("aufbereiten-dialog")).toBeHidden();
  await expect(page.locator("textarea").first()).toHaveValue(ERGEBNIS);
}

// Gegenproben: Ohne die jeweilige UI-Wirkung schlagen diese Erwartungen fehl;
// insbesondere werden Ergebnis und Vergleich unabhängig von der Attrappe geprüft.
test("Menüeintrag öffnet Vorschläge mit Regelgruppen und Anzahl", async ({
  page,
}) => {
  await bruecke(page);
  await openEditor(page);
  const dialog = await oeffnen(page);
  await expect(dialog.getByTestId("aufbereiten-gruppe-ligaturen")).toContainText(
    "Ligaturen auflösen · 1 Änderungen, 1 Gruppen · Offen",
  );
  await expect(
    dialog.getByTestId("aufbereiten-gruppe-anfuehrungszeichen"),
  ).toContainText("Anführungszeichen · 2 Änderungen, 1 Gruppen · Offen");
  await expect(dialog.getByTestId("aufbereiten-gruppe-auslassung")).toContainText(
    "Auslassungspunkte · 1 Änderungen, 1 Gruppen · Offen",
  );
});

test("Annehmen und Ablehnen setzen den Status je Regel und für alle", async ({
  page,
}) => {
  await bruecke(page);
  await openEditor(page);
  await oeffnen(page);
  for (const regel of REGELN) {
    await page.getByTestId(`aufbereiten-annehmen-${regel}`).click();
    await status(page, regel, "Angenommen");
    await page.getByTestId(`aufbereiten-ablehnen-${regel}`).click();
    await status(page, regel, "Abgelehnt");
  }
  await alleAnnehmen(page);
  await page.getByTestId("aufbereiten-alle-ablehnen").click();
  for (const regel of REGELN) await status(page, regel, "Abgelehnt");
});

test("Übernehmen ersetzt den Editorinhalt und Rückgängig stellt den Vortext her", async ({
  page,
}) => {
  await bruecke(page);
  const editor = await openEditor(page);
  await oeffnen(page);
  await anwenden(page);
  // Die Leiste des Editors hat ebenfalls „Rückgängig“: nur den Hinweis (Toast) meinen.
  const undo = page
    .locator("[data-sonner-toast]")
    .getByRole("button", { name: "Rückgängig", exact: true });
  await expect(undo).toBeVisible();
  await undo.click();
  await expect(editor).toHaveValue(BASIS);
});

test("Veralteter Entwurf wird neu berechnet und verändert den Editor nicht", async ({
  page,
}) => {
  await bruecke(page);
  const editor = await openEditor(page);
  await modus(page, { veraltet: true });
  await oeffnen(page);
  await alleAnnehmen(page);
  await page.getByTestId("aufbereiten-uebernehmen").click();
  await expect(page.getByTestId("aufbereiten-veraltet")).toBeVisible();
  await expect(page.getByTestId("aufbereiten-uebernehmen")).toBeDisabled();
  await expect(editor).toHaveValue(BASIS);
  const vorher = await page.evaluate(
    () =>
      (window as unknown as { __aufbereiten: { entwürfe: number } })
        .__aufbereiten.entwürfe,
  );
  await page.getByTestId("aufbereiten-neu-berechnen").click();
  await expect(page.getByTestId("aufbereiten-veraltet")).toBeHidden();
  await expect(page.getByTestId("aufbereiten-uebernehmen")).toBeEnabled();
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { __aufbereiten: { entwürfe: number } })
          .__aufbereiten.entwürfe,
    ),
  ).toBeGreaterThan(vorher);
  for (const regel of REGELN) await status(page, regel, "Offen");
  await expect(editor).toHaveValue(BASIS);
});

test("Original wiederherstellen setzt den erhaltenen Originaltext", async ({
  page,
}) => {
  await bruecke(page);
  const editor = await openEditor(page);
  await oeffnen(page);
  await expect(page.getByTestId("aufbereiten-original")).toHaveCount(0);
  await anwenden(page);
  await oeffnen(page);
  await page.getByTestId("aufbereiten-original").click();
  await expect(page.getByTestId("aufbereiten-dialog")).toBeHidden();
  await expect(editor).toHaveValue(BASIS);
});

test("Tab erreicht die Knöpfe und Escape schließt den Dialog", async ({
  page,
}) => {
  await bruecke(page);
  await openEditor(page);
  const dialog = await oeffnen(page);
  // Vom letzten Regelschalter führt die natürliche Tab-Reihenfolge zu Knöpfen.
  await dialog.getByTestId("aufbereiten-regel-listen").focus();
  await page.keyboard.press("Tab");
  await expect(dialog.getByTestId("aufbereiten-alle-annehmen")).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(dialog.getByTestId("aufbereiten-alle-ablehnen")).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
});

test("Bei 390 Pixeln sind die Entscheidungsknöpfe mindestens 44 Pixel hoch", async ({
  page,
}) => {
  await bruecke(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await openEditor(page);
  await oeffnen(page);
  for (const id of [
    ...REGELN.flatMap((regel) => [
      `aufbereiten-annehmen-${regel}`,
      `aufbereiten-ablehnen-${regel}`,
    ]),
    "aufbereiten-uebernehmen",
  ]) {
    const button = page.getByTestId(id);
    await expect(button).toBeVisible();
    const box = await button.boundingBox();
    expect(box, id).not.toBeNull();
    expect(box!.height, id).toBeGreaterThanOrEqual(44);
  }
});

test("UTF-16-Grenzen zeigen Änderungen hinter einem Emoji an der richtigen Stelle", async ({
  page,
}) => {
  await bruecke(page);
  await openEditor(page);
  const dialog = await oeffnen(page);
  // Die sr-only-Präfixe gehören zum Textinhalt der ins-/del-Elemente.
  await expect(dialog.locator("del")).toHaveText([
    "entfernt „",
    "entfernt ﬁ",
    "entfernt ...",
    "entfernt “",
  ]);
  await expect(dialog.locator("ins")).toHaveText([
    'eingefügt "',
    "eingefügt fi",
    "eingefügt …",
    'eingefügt "',
  ]);
  // Zusätzlich den Kontext prüfen: Ein verschobener Schnitt verliert Zeichen.
  const vergleich = await dialog.locator("del").first().evaluate((el) => {
    const flaeche = el.parentElement?.parentElement;
    if (!flaeche) throw new Error("Vergleich fehlt");
    const kopie = flaeche.cloneNode(true) as HTMLElement;
    kopie.querySelectorAll(".sr-only, del").forEach((knoten) => knoten.remove());
    return kopie.textContent;
  });
  expect(vergleich).toBe(ERGEBNIS);
});

test("Ein leerer Entwurf zeigt Keine Vorschläge", async ({ page }) => {
  await bruecke(page);
  await openEditor(page);
  await modus(page, { leer: true });
  const dialog = await oeffnen(page);
  await expect(dialog.getByTestId("aufbereiten-leer")).toHaveText(
    "Keine Vorschläge.",
  );
  await expect(
    dialog.locator('[data-testid^="aufbereiten-gruppe-"]'),
  ).toHaveCount(0);
  await expect(dialog.locator("ins, del")).toHaveCount(0);
});

async function aufnahme(page: Page, zustand: string, breite: number, theme: Theme) {
  if (!SHOTS) return;
  mkdirSync(SHOT_DIR, { recursive: true });
  const name = theme === "light" ? "hell" : "dunkel";
  await page.screenshot({
    path: join(SHOT_DIR, `${zustand}-${breite}-${name}.png`),
  });
}

// Ohne Aufnahmevariable werden weder zusätzliche Browserfälle noch PNGs erzeugt.
if (SHOTS) {
  for (const theme of ["light", "dark"] as const) {
    for (const breite of [1440, 768, 390]) {
      const name = theme === "light" ? "hell" : "dunkel";
      test(`Vorschau mit ${breite} Pixeln in ${name}`, async ({ page }) => {
        await bruecke(page, theme);
        await page.setViewportSize({
          width: breite,
          height: breite === 1440 ? 900 : breite === 768 ? 1024 : 844,
        });
        const editor = await openEditor(page, BASIS, theme);
        await page.addStyleTag({
          content:
            "*,*::before,*::after{animation-duration:0s!important;" +
            "animation-delay:0s!important;transition-duration:0s!important;" +
            "transition-delay:0s!important}",
        });
        await modus(page, { leer: true });
        await oeffnen(page);
        await expect(page.getByTestId("aufbereiten-leer")).toBeVisible();
        await aufnahme(page, "1-keine-vorschlaege", breite, theme);
        await page.keyboard.press("Escape");
        await expect(page.getByTestId("aufbereiten-dialog")).toBeHidden();
        await modus(page, { leer: false });
        await oeffnen(page);
        await aufnahme(page, "2-vorschlaege", breite, theme);
        await page.getByTestId("aufbereiten-annehmen-ligaturen").click();
        await status(page, "ligaturen", "Angenommen");
        await page.getByTestId("aufbereiten-ablehnen-anfuehrungszeichen").click();
        await status(page, "anfuehrungszeichen", "Abgelehnt");
        await status(page, "auslassung", "Offen");
        await aufnahme(page, "3-teilannahme", breite, theme);
        await modus(page, { veraltet: true });
        await page.getByTestId("aufbereiten-uebernehmen").click();
        await expect(page.getByTestId("aufbereiten-veraltet")).toBeVisible();
        await expect(editor).toHaveValue(BASIS);
        await aufnahme(page, "4-veraltet", breite, theme);
        await page.getByTestId("aufbereiten-neu-berechnen").click();
        await expect(page.getByTestId("aufbereiten-veraltet")).toBeHidden();
        await expect(page.getByTestId("aufbereiten-uebernehmen")).toBeEnabled();
        await anwenden(page);
        await expect(
          page.getByRole("button", { name: "Rückgängig", exact: true }),
        ).toBeVisible();
        await aufnahme(page, "5-uebernommen", breite, theme);
        await oeffnen(page);
        await page.getByTestId("aufbereiten-original").click();
        await expect(page.getByTestId("aufbereiten-dialog")).toBeHidden();
        await expect(editor).toHaveValue(BASIS);
        await aufnahme(page, "6-original-wiederhergestellt", breite, theme);
      });
    }
  }
}
