import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { test, expect, type Page } from "@playwright/test";
import { installLibraryTranscriptionMock } from "./libraryTranscriptionMock";

// lva#943: Dialog „Aussprache“ (Lexikon je Buch und global). Nur erfundene Inhalte.
const TEXT =
  "Fiktivia arbeitet bei GE an einem Workflow. Das Meeting mit Microsoft beginnt um 9 Uhr.";
const SHOTS = process.env.LEXIKON_SHOTS === "1";
const SHOT_DIR = process.env.LV_SHOT_DIR ?? "docs/vorschau/aussprache-lexikon";
type Theme = "light" | "dark";
type Lex = {
  kontexte: { buch_id: string | null; seite_id: string | null }[];
  gespeichert: { ziel: string; eintraege: unknown[] }[];
  vorschlagTexte: number[];
};

async function bruecke(page: Page, theme: Theme = "light") {
  await installLibraryTranscriptionMock(page);
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.emulateMedia({ colorScheme: theme });
  await page.addInitScript(
    ({ theme }) => {
      const callbacks = new Map<number, unknown>();
      let callback = 0;
      const calls: string[] = [];
      const kontexte: unknown[] = [];
      const gespeichert: unknown[] = [];
      const vorschlagTexte: number[] = [];
      const lexika: Record<string, unknown[]> = {
        "buch:b1": [
          { wort: "Microsoft", aussprache: "Maikrosoft", gross_klein: false },
        ],
        global: [
          { wort: "Meeting", aussprache: "Mieting", gross_klein: false },
          { wort: "Team", aussprache: "Tiem", gross_klein: false },
        ],
      };
      // 0,05 s Stille, 8 kHz, 8 Bit: ein gültiges WAV für die Probe.
      const n = 400;
      const wav = new Uint8Array(44 + n);
      const dv = new DataView(wav.buffer);
      const str = (o: number, s: string) =>
        [...s].forEach((c, i) => dv.setUint8(o + i, c.charCodeAt(0)));
      str(0, "RIFF");
      dv.setUint32(4, 36 + n, true);
      str(8, "WAVEfmt ");
      dv.setUint32(16, 16, true);
      dv.setUint16(20, 1, true);
      dv.setUint16(22, 1, true);
      dv.setUint32(24, 8000, true);
      dv.setUint32(28, 8000, true);
      dv.setUint16(32, 1, true);
      dv.setUint16(34, 8, true);
      str(36, "data");
      dv.setUint32(40, n, true);
      wav.fill(128, 44);
      const buch = {
        id: "b1",
        title: "Fiktivas Idee",
        subtitle: null,
        authors: ["Fiktiva Muster"],
        language: "de",
        isbn: null,
        publisher: null,
        publish_date: null,
        pages: null,
        description: null,
        metadata_source: "manual",
        cover_source: "generated",
        text: { page_id: "p1", origin: "document", source_name: null },
        audio: null,
        added_ms: 0,
        finished: false,
        cover_path: null,
        audio_path: null,
        schema: 2,
        editions: [],
        edition_tombstones: [],
        alignments: [],
        aussprache_lexikon: [],
        audio_paths: [],
      };
      Object.assign(window, {
        __lexikon: { kontexte, gespeichert, vorschlagTexte, lexika },
      });
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
            if (cmd.startsWith("library_transcription_"))
              return (window as any).__libraryTranscriptionMock(cmd, args);
            calls.push(cmd);
            switch (cmd) {
              case "lexikon_kontext":
                kontexte.push(args?.kontext);
                return null;
              case "lexikon_laden": {
                const ziel = args?.ziel as { art: string; id?: string };
                return structuredClone(
                  lexika[
                    ziel.art === "global" ? "global" : `${ziel.art}:${ziel.id}`
                  ] ?? [],
                );
              }
              case "lexikon_speichern": {
                const ziel = args?.ziel as { art: string; id?: string };
                const k =
                  ziel.art === "global" ? "global" : `${ziel.art}:${ziel.id}`;
                lexika[k] = structuredClone(args?.eintraege) as never;
                gespeichert.push({ ziel: k, eintraege: args?.eintraege });
                return null;
              }
              case "lexikon_vorschlaege":
                vorschlagTexte.push(String(args?.dokument).length);
                return {
                  vorschlaege: [
                    {
                      wort: "Workflow",
                      aussprache: "Wörkflo",
                      gross_klein: false,
                    },
                    { wort: "GE", aussprache: "Dschi-Ie", gross_klein: true },
                    {
                      wort: "Fiktivia",
                      aussprache: "Fiktiwia",
                      gross_klein: false,
                    },
                  ],
                  woerterbuch_verfuegbar: false,
                };
              case "tts_wort_probe":
                return Array.from(wav);
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
              case "library_list":
                return [buch];
              case "pages_list":
                return [{ id: "p1", title: "Fiktivas Idee" }];
              case "books_list":
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

async function openEditor(page: Page, theme: Theme = "light") {
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  if ((page.viewportSize()?.width ?? 1280) < 768) {
    await page.getByRole("button", { name: "Menü öffnen" }).click();
  }
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  const editor = page.locator("textarea").first();
  await editor.fill(TEXT);
  return editor;
}

const lex = (page: Page) =>
  page.evaluate(() => (window as unknown as { __lexikon: Lex }).__lexikon);

async function oeffnen(page: Page) {
  await page.getByTestId("tts-action-menu").first().click();
  await page.getByTestId("lexikon-open").click();
  const dialog = page.getByTestId("lexikon-dialog");
  await expect(dialog).toBeVisible();
  await expect(page.getByTestId("lexikon-wort-0")).toBeVisible();
  return dialog;
}

test("Kontext: Buch der Seite geht vor dem Vorlesen an den Kern", async ({
  page,
}) => {
  await bruecke(page);
  await openEditor(page);
  await expect
    .poll(async () => (await lex(page)).kontexte.at(-1))
    .toEqual({ buch_id: "b1", seite_id: "p1" });
});

test("Menü öffnet Buchliste, Wechsel zu „Alle Texte“ zeigt die globale Liste", async ({
  page,
}) => {
  await bruecke(page);
  await openEditor(page);
  const dialog = await oeffnen(page);
  await expect(page.getByTestId("lexikon-wort-0")).toHaveValue("Microsoft");
  await dialog.getByRole("tab", { name: /Alle Texte/ }).click();
  await expect(page.getByTestId("lexikon-wort-0")).toHaveValue("Meeting");
  await expect(page.getByTestId("lexikon-wort-1")).toHaveValue("Team");
  await expect(page.getByTestId("lexikon-speichern")).toBeDisabled();
});

test("Ungültige Zeile blockiert, gültige wird ins Buch gespeichert", async ({
  page,
}) => {
  await bruecke(page);
  await openEditor(page);
  await oeffnen(page);
  await page.getByTestId("lexikon-hinzu").click();
  await page.getByTestId("lexikon-wort-1").fill("zwei Wörter");
  await expect(page.getByText("Nur ein einzelnes Wort")).toBeVisible();
  await page.getByTestId("lexikon-speichern").click();
  expect((await lex(page)).gespeichert).toEqual([]);
  await page.getByTestId("lexikon-wort-1").fill("GE");
  await page.getByTestId("lexikon-aussprache-1").fill("Dschi-Ie");
  await page.getByTestId("lexikon-gross-klein-1").check();
  await page.getByTestId("lexikon-speichern").click();
  await expect(page.getByTestId("lexikon-dialog")).toBeHidden();
  expect((await lex(page)).gespeichert).toEqual([
    {
      ziel: "buch:b1",
      eintraege: [
        { wort: "Microsoft", aussprache: "Maikrosoft", gross_klein: false },
        { wort: "GE", aussprache: "Dschi-Ie", gross_klein: true },
      ],
    },
  ]);
});

test("KI-Vorschläge: nur gewählte werden übernommen, erst Speichern schreibt", async ({
  page,
}) => {
  await bruecke(page);
  await openEditor(page);
  await oeffnen(page);
  await page.getByTestId("lexikon-vorschlagen").click();
  const box = page.getByTestId("lexikon-vorschlaege");
  await expect(box).toContainText("Workflow");
  await expect(box).toContainText("nicht der Text");
  await page.getByTestId("lexikon-vorschlag-2").uncheck();
  await page.getByTestId("lexikon-uebernehmen").click();
  await expect(page.getByTestId("lexikon-wort-2")).toHaveValue("GE");
  expect((await lex(page)).gespeichert).toEqual([]);
  await page.getByTestId("lexikon-speichern").click();
  await expect(page.getByTestId("lexikon-dialog")).toBeHidden();
  const g = (await lex(page)).gespeichert;
  expect(g).toHaveLength(1);
  expect((g[0].eintraege as { wort: string }[]).map((e) => e.wort)).toEqual([
    "Microsoft",
    "Workflow",
    "GE",
  ]);
});

test("Markiertes Wort wird als neue Zeile vorbelegt", async ({ page }) => {
  await bruecke(page);
  const editor = await openEditor(page);
  await editor.evaluate((el: HTMLTextAreaElement) => {
    const i = el.value.indexOf("Workflow");
    el.focus();
    el.setSelectionRange(i, i + "Workflow".length);
  });
  // Textarea-Auswahl liefert getSelection() nicht: Markierung im Dokument nachbilden.
  await page.evaluate(() => {
    const span = document.createElement("span");
    span.textContent = "Workflow";
    document.body.appendChild(span);
    const r = document.createRange();
    r.selectNodeContents(span);
    const s = window.getSelection();
    s?.removeAllRanges();
    s?.addRange(r);
  });
  await oeffnen(page);
  await expect(page.getByTestId("lexikon-wort-1")).toHaveValue("Workflow");
  await expect(page.getByTestId("lexikon-aussprache-1")).toBeFocused();
});

async function aufnahme(
  page: Page,
  zustand: string,
  breite: number,
  theme: Theme,
) {
  if (!SHOTS) return;
  mkdirSync(SHOT_DIR, { recursive: true });
  const name = theme === "light" ? "hell" : "dunkel";
  await page.screenshot({
    path: join(SHOT_DIR, `${zustand}-${breite}-${name}.png`),
  });
}

if (SHOTS) {
  for (const theme of ["light", "dark"] as const) {
    for (const breite of [1440, 768, 390]) {
      const name = theme === "light" ? "hell" : "dunkel";
      test(`Vorschau Aussprache ${breite} ${name}`, async ({ page }) => {
        await bruecke(page, theme);
        await page.setViewportSize({
          width: breite,
          height: breite === 1440 ? 900 : breite === 768 ? 1024 : 844,
        });
        await openEditor(page, theme);
        await page.addStyleTag({
          content:
            "*,*::before,*::after{animation-duration:0s!important;" +
            "animation-delay:0s!important;transition-duration:0s!important;" +
            "transition-delay:0s!important}",
        });
        await oeffnen(page);
        await aufnahme(page, "1-buch", breite, theme);
        await page.getByTestId("lexikon-vorschlagen").click();
        await expect(page.getByTestId("lexikon-vorschlaege")).toBeVisible();
        await aufnahme(page, "2-vorschlaege", breite, theme);
        await page.getByTestId("lexikon-uebernehmen").click();
        await page.getByTestId("lexikon-hinzu").click();
        await page.getByTestId("lexikon-wort-4").fill("zwei Wörter");
        await page.getByTestId("lexikon-aussprache-4").click();
        await aufnahme(page, "3-bearbeiten-fehler", breite, theme);
        await page.getByRole("tab", { name: /Alle Texte/ }).click();
        await aufnahme(page, "4-alle-texte", breite, theme);
      });
    }
  }
}
