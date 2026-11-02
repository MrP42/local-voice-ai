import { test, expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { typesToShow } from "../src/components/settings/meetings/documents/docTypes";
import { geheZu } from "./profilmenue";

// Dokumenttypen neben dem Protokoll (Entwurf 2026-10-13, Etappe 1): Typleiste im
// Reiter, "Neues Dokument" mit den zehn Typen, Erzeugen mit eigenem Befehl und
// eigenem Ereignis, Follow-up oeffnet den vorhandenen Dialog. Gegen die
// Tauri-Attrappe: jeder Aufruf steht in `window.__calls`, Ereignisse loest
// `window.__emit` aus, `window.__generate` steuert den Ausgang des Erzeugens.

type Call = { cmd: string; args: Record<string, unknown> };

const TYPES = [
  ["protokoll", "Protokoll", "minutes", "minutes", "2–6 Seiten"],
  [
    "management_summary",
    "Management Summary",
    "document",
    "doc:management_summary",
    "höchstens 1 Seite",
  ],
  [
    "kurzfassung",
    "Zusammenfassung",
    "document",
    "doc:kurzfassung",
    "höchstens ½ Seite",
  ],
  [
    "entscheidungen_aufgaben",
    "Entscheidungen und Aufgaben",
    "document",
    "doc:entscheidungen_aufgaben",
    "nur Listen",
  ],
  ["followup_mail", "Follow-up-Mail", "followup", "", "höchstens 250 Wörter"],
  [
    "statusbericht",
    "Statusbericht",
    "document",
    "doc:statusbericht",
    "1–2 Seiten",
  ],
  [
    "briefing_abwesende",
    "Briefing für Abwesende",
    "document",
    "doc:briefing_abwesende",
    "höchstens 1 Seite",
  ],
  ["faq", "Fragen und Antworten", "document", "doc:faq", "nach Bedarf"],
  [
    "lernzusammenfassung",
    "Lernzusammenfassung",
    "document",
    "doc:lernzusammenfassung",
    "1–2 Seiten",
  ],
  [
    "vortragsgrundlage",
    "Vortragsgrundlage",
    "document",
    "doc:vortragsgrundlage",
    "1–2 Seiten",
  ],
].map(([key, title, engine, kind, length]) => ({
  key,
  title,
  engine,
  kind,
  length,
  audience:
    key === "management_summary" ? "Entscheider, nicht anwesend" : "alle",
  depth: "gering",
  sections: ["A", "B"],
}));

const SAMPLE =
  process.env.DOC_SAMPLE && readFileSync(process.env.DOC_SAMPLE, "utf-8");

const SUMMARY_BODY =
  SAMPLE ||
  "# Management Summary: Kundentermin Meyer\n\n**Datum:** 2026-10-12 · **Dauer:** 10 min · **Für:** Entscheider, nicht anwesend\n\n## Anlass und Ziel\n\nEntscheidung über den Go-Live.\n\n## Kernergebnisse\n\n- Go-Live am 1. September\n- Budget bleibt unverändert\n";

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(
    ({ types, summaryBody }) => {
      const w = window as any;
      const callbacks = new Map<number, (e: unknown) => void>();
      const listeners: Record<string, number[]> = {};
      const handlerOf = new Map<number, { event: string; handler: number }>();
      let callback = 0;
      const settings = {
        onboarding_completed: true,
        app_language: "de",
        theme: "light",
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
        document_letterhead_id: "neutral",
      };
      w.__settings = settings;
      w.__savePath = null; // Antwort des Speichern-Dialogs
      const colors = {
        akzent: "#FFDD00",
        akzent_text: "#111418",
        text: "#1F2937",
        titel: "#111418",
        gedaempft: "#626976",
        linie: "#E1E6EC",
      };
      const builtin = (id: string, name: string) => ({
        schema: "lva.briefkopf/1",
        id,
        name,
        builtin: true,
        firma: { name: id === "neutral" ? "" : name, zeilen: [] },
        logo: null,
        farben: colors,
        fusszeile: { zeilen: [], seitenzahl: true },
      });
      w.__letterheads = [
        builtin("neutral", "Neutral"),
        builtin("wai", "Wolff Applied AI"),
        builtin("ingenieurbuero", "Ingenieurbüro Wolff"),
      ];
      w.__calls = [];
      w.__generate = "ok";
      w.__release = null;
      w.__runState = {
        running: false,
        progress: null,
        cancelling: false,
        started_at: null,
      };
      w.__documents = [
        {
          id: "p1",
          meeting_id: "m1",
          kind: "minutes",
          body_format: "markdown@1",
          body: "# Protokoll: Kundentermin Meyer\n\nAusführlich.",
          version: 1,
          created_at: 1790000901,
          template_id: null,
          updated_at: 301,
        },
      ];
      w.__meetings = [
        {
          id: "m1",
          title: "Kundentermin Meyer",
          status: "ready",
          source: "recording",
          started_at: 1790000000,
          ended_at: 1790000600,
          language: "de",
          mic_audio_path: null,
          system_audio_path: null,
          duration_ms: 600000,
          consent_confirmed_at: 1790000000,
          audio_retention_until: null,
          created_at: 1790000000,
          source_path: null,
        },
      ];
      w.__emit = (event: string, payload: unknown) =>
        (listeners[event] ?? []).forEach((h) =>
          callbacks.get(h)?.({ event, id: 0, payload }),
        );
      Object.assign(window, {
        __TAURI_OS_PLUGIN_INTERNALS__: {
          platform: "windows",
          os_type: "windows",
          family: "windows",
          arch: "x86_64",
          version: "11",
          eol: "\r\n",
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
          convertFileSrc: (p: string) => p,
          invoke: async (cmd: string, args: Record<string, unknown> = {}) => {
            w.__calls.push({ cmd, args });
            switch (cmd) {
              case "get_app_settings":
              case "get_default_settings":
                return settings;
              case "plugin:os|locale":
                return "de-DE";
              case "plugin:app|version":
                return "0.21.15";
              case "plugin:event|listen": {
                (listeners[args.event as string] ??= []).push(
                  args.handler as number,
                );
                const eventId = ++callback;
                handlerOf.set(eventId, {
                  event: args.event as string,
                  handler: args.handler as number,
                });
                return eventId;
              }
              case "plugin:event|unlisten": {
                const entry = handlerOf.get(args.eventId as number);
                if (entry) {
                  listeners[entry.event] = (
                    listeners[entry.event] ?? []
                  ).filter((h) => h !== entry.handler);
                  handlerOf.delete(args.eventId as number);
                }
                return null;
              }
              case "get_selected_model":
                return "";
              case "meetings_is_recording":
                return false;
              case "meetings_recording_position":
                return null;
              case "meetings_list":
                return w.__meetings;
              case "meetings_get_segments":
                return [];
              case "meetings_get_documents":
                return w.__documents;
              case "meetings_minutes_file":
                return null;
              case "meetings_minutes_meta":
              case "meetings_document_meta":
                return null;
              case "meetings_minutes_state":
                return w.__runState;
              case "meetings_document_types":
                return types;
              case "meetings_generate_document": {
                const mode = w.__generate as string;
                if (mode === "pending") {
                  w.__runState = {
                    running: true,
                    progress: { phase: "write", done: 0, total: 1 },
                    cancelling: false,
                    started_at: 1790001000,
                  };
                  await new Promise<void>((resolve) => {
                    w.__release = resolve;
                  });
                  w.__runState = { ...w.__runState, running: false };
                } else if (mode !== "ok") {
                  throw mode;
                }
                const doc = {
                  id: `d${w.__documents.length + 1}`,
                  meeting_id: "m1",
                  kind: `doc:${args.docType}`,
                  body_format: "markdown@1",
                  body: summaryBody,
                  version: 1,
                  created_at: 1790001900,
                  template_id: `doctype:${args.docType}`,
                  updated_at: 400,
                };
                w.__documents = [...w.__documents, doc];
                return doc;
              }
              case "meeting_followup_draft":
                throw "followup_no_content";
              case "meeting_notes_get":
                return {
                  meeting_id: args.meetingId,
                  blocks: [],
                  revision: 0,
                  updated_at: 0,
                };
              case "meeting_templates_list":
                return [];
              case "plugin:dialog|save":
                return w.__savePath;
              case "meetings_export_document_as":
              case "letterhead_export_sample":
                return null;
              case "letterheads_list":
                return w.__letterheads;
              case "letterhead_save": {
                const lh = { ...(args.letterhead as any), builtin: false };
                w.__letterheads = [
                  ...w.__letterheads.filter((l: any) => l.id !== lh.id),
                  lh,
                ];
                return null;
              }
              case "change_document_letterhead_setting":
                settings.document_letterhead_id = args.id as string;
                return null;
              case "tts_server_status":
                return { phase: "stopped", message: null };
              case "get_custom_sounds":
                return { start: false, stop: false };
              case "pages_list":
              case "page_files":
              case "tts_list_voices":
              case "tts_list_voice_infos":
              case "llm_ps":
              case "tts_reading_list":
              case "tts_list_downloads":
              case "llm_local_list":
              case "action_items_list":
              case "integrations_list":
                return w.__integrations ?? [];
              case "presenter_send":
                return {
                  folder: "pakete/2026-10-04-kundentermin-meyer-m1",
                  files: 6,
                  replaced: 0,
                };
              case "provenance_get":
                return [];
            }
            if (
              cmd.includes("permission") ||
              cmd.includes("history") ||
              cmd.includes("models") ||
              cmd.includes("devices")
            )
              return cmd.includes("permission") ? true : [];
            return null;
          },
        },
      });
      localStorage.setItem("lva.ui.settings.tab", "dictation");
    },
    { types: TYPES, summaryBody: SUMMARY_BODY },
  );
});

const calls = (page: Page, cmd: string) =>
  page.evaluate(
    (c) => (window as any).__calls.filter((x: Call) => x.cmd === c) as Call[],
    cmd,
  );

const emit = (page: Page, event: string, payload: unknown) =>
  page.evaluate(([e, p]) => (window as any).__emit(e, p), [event, payload]);

const openDocuments = async (page: Page) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/");
  await page.getByRole("button", { name: "Aufnahmen", exact: true }).click();
  await page.getByText("Kundentermin Meyer", { exact: true }).click();
  await page.getByRole("tab", { name: "Protokoll", exact: true }).click();
  await expect(page.getByTestId("documents-panel")).toBeVisible();
};

/** Eintrag im Menue "Neues Dokument" (per JS geklickt, das Menue haengt im Portal). */
const pickType = async (page: Page, key: string) => {
  await page.getByTestId("doc-new").click();
  await expect(page.getByRole("menu")).toBeVisible();
  await page
    .getByTestId(`doc-type-${key}`)
    .evaluate((el) => (el as HTMLElement).click());
};

test("typesToShow: Protokoll immer, vorhandene und gewaehlte Typen, nie Follow-up", () => {
  const info = TYPES as unknown as Parameters<typeof typesToShow>[0];
  const keys = (sel: string, kinds: string[]) =>
    typesToShow(info, kinds, sel).map((t) => t.key);
  expect(keys("protokoll", [])).toEqual(["protokoll"]);
  expect(keys("faq", ["doc:kurzfassung"])).toEqual([
    "protokoll",
    "kurzfassung",
    "faq",
  ]);
  expect(keys("followup_mail", [])).toEqual(["protokoll"]);
});

test("der Reiter zeigt das Protokoll und bietet alle zehn Typen an", async ({
  page,
}) => {
  await openDocuments(page);
  const bar = page.getByTestId("doc-type-bar");
  await expect(bar.getByRole("button", { name: "Protokoll" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(page.getByTestId("minutes-doc")).toContainText("Ausführlich.");
  // Ohne Presenter-Integration kein Senden-Knopf.
  await expect(page.getByTestId("presenter-send")).toHaveCount(0);

  await page.getByTestId("doc-new").click();
  const menu = page.getByRole("menu");
  await expect(menu.getByRole("menuitem")).toHaveCount(10);
  await expect(menu.getByTestId("doc-type-management_summary")).toContainText(
    "höchstens 1 Seite",
  );
  await expect(menu.getByTestId("doc-type-vortragsgrundlage")).toBeVisible();
});

test("Management Summary erzeugen: eigener Befehl, Chip bleibt, Protokoll unberuehrt", async ({
  page,
}) => {
  await openDocuments(page);
  await pickType(page, "management_summary");

  const chip = page.getByTestId("doc-chip-management_summary");
  await expect(chip).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByTestId("minutes-empty")).toContainText(
    "Entscheider, nicht anwesend",
  );

  await page.getByTestId("minutes-generate").click();
  await expect(page.getByTestId("minutes-doc")).toContainText(
    "Management Summary",
  );
  const generated = await calls(page, "meetings_generate_document");
  expect(generated).toHaveLength(1);
  expect(generated[0].args).toMatchObject({
    meetingId: "m1",
    docType: "management_summary",
  });
  expect(await calls(page, "meetings_generate_minutes")).toHaveLength(0);

  // Zurueck zum Protokoll: dessen Text, der Chip der Summary bleibt sichtbar.
  await page.getByTestId("doc-chip-protokoll").click();
  await expect(page.getByTestId("minutes-doc")).toContainText("Ausführlich.");
  await expect(chip).toBeVisible();
});

test("Fortschritt kommt ueber das Dokument-Ereignis, nicht ueber das des Protokolls", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as any).__generate = "pending";
  });
  await openDocuments(page);
  await pickType(page, "kurzfassung");
  await page.getByTestId("minutes-generate").click();
  await expect(page.getByTestId("minutes-running")).toContainText(
    "„Zusammenfassung“ wird erzeugt",
  );

  // Ein Ereignis eines anderen Typs aendert nichts ...
  await emit(page, "document-event", {
    kind: "progress",
    meeting_id: "m1",
    doc_type: "faq",
    phase: "merge",
    done: 3,
    total: 4,
  });
  // ... das eigene setzt den Fortschritt.
  await emit(page, "document-event", {
    kind: "progress",
    meeting_id: "m1",
    doc_type: "kurzfassung",
    phase: "write",
    done: 1,
    total: 2,
  });
  await expect(page.getByTestId("minutes-percent")).toContainText("50");

  await page.evaluate(() => (window as any).__release());
  await emit(page, "document-event", {
    kind: "done",
    meeting_id: "m1",
    doc_type: "kurzfassung",
    document_id: "d2",
  });
  await expect(page.getByTestId("minutes-running")).toHaveCount(0);
  await expect(page.getByTestId("doc-chip-kurzfassung")).toBeVisible();
});

test("Follow-up-Mail oeffnet den vorhandenen Dialog und legt keinen Chip an", async ({
  page,
}) => {
  await openDocuments(page);
  await pickType(page, "followup_mail");
  await expect
    .poll(async () => (await calls(page, "meeting_followup_draft")).length)
    .toBe(1);
  await expect(page.getByTestId("doc-chip-followup_mail")).toHaveCount(0);
});

test("Bild fuer die Abnahme (nur mit SCREENS_DIR)", async ({ page }) => {
  const dir = process.env.SCREENS_DIR;
  test.skip(!dir, "nur mit SCREENS_DIR");
  await openDocuments(page);
  await pickType(page, "management_summary");
  await page.getByTestId("minutes-generate").click();
  await expect(page.getByTestId("minutes-doc")).toContainText(
    "Management Summary",
  );
  await page.screenshot({ path: `${dir}/dokumente-management-summary.png` });
  await page.getByTestId("doc-new").click();
  await expect(page.getByRole("menu")).toBeVisible();
  await page.screenshot({ path: `${dir}/dokumente-neues-dokument.png` });
});

test("Export: Word/PDF ueber das gespeicherte Dokument mit Briefkopf", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as any).__savePath = "C:/Ablage/Kundentermin Meyer.pdf";
  });
  await openDocuments(page);
  await page.getByTestId("minutes-export").click();
  await expect
    .poll(async () => (await calls(page, "meetings_export_document_as")).length)
    .toBe(1);
  const [call] = await calls(page, "meetings_export_document_as");
  expect(call.args).toEqual({
    documentId: "p1",
    path: "C:/Ablage/Kundentermin Meyer.pdf",
  });
  const [dialog] = await calls(page, "plugin:dialog|save");
  const names = (
    (dialog.args.options as { filters: { name: string }[] }).filters ?? []
  ).map((f) => f.name);
  expect(names).toEqual(["Word", "PDF", "PowerPoint", "Text", "Markdown"]);
  // Der alte Weg (nur Text) wird fuer Protokoll/Dokumente nicht mehr benutzt.
  expect(await calls(page, "meetings_export_document")).toHaveLength(0);
});

const openOutput = async (page: Page) => {
  await page.setViewportSize({ width: 1280, height: 1000 });
  await page.goto("/");
  await geheZu(page, "Einstellungen");
  await page.getByRole("tab", { name: "Ausgabe", exact: true }).click();
  await expect(page.getByTestId("documents-settings")).toBeVisible();
};

test("Einstellungen > Ausgabe > Dokumente: eigenen Briefkopf anlegen und bearbeiten", async ({
  page,
}) => {
  await openOutput(page);
  await expect(page.getByText("Briefkopf für Word und PDF")).toBeVisible();
  // Mitgeliefert: kein Formular, nur "Als eigenen Briefkopf anlegen".
  await expect(page.getByTestId("letterhead-form")).toHaveCount(0);
  await page.getByTestId("letterhead-copy").click();

  const [saved] = await calls(page, "letterhead_save");
  const copy = saved.args.letterhead as { id: string; name: string };
  expect(copy).toMatchObject({
    id: "kopie-von-neutral",
    name: "Kopie von Neutral",
  });
  await expect
    .poll(
      async () =>
        (await calls(page, "change_document_letterhead_setting")).at(-1)?.args,
    )
    .toEqual({ id: "kopie-von-neutral" });

  const form = page.getByTestId("letterhead-form");
  await expect(form).toBeVisible();
  await page.getByTestId("letterhead-firm").fill("Muster GmbH");
  // Zeilenumbrueche lassen sich tippen; leere Zeilen fallen erst beim Speichern weg.
  const typed = ["Musterstraße 1", "", "12345 Musterstadt "].join("\n");
  await page.getByTestId("letterhead-lines").fill(typed);
  await expect(page.getByTestId("letterhead-lines")).toHaveValue(typed);
  await page.getByTestId("letterhead-save").click();
  await expect
    .poll(async () => (await calls(page, "letterhead_save")).length)
    .toBe(2);
  const last = (await calls(page, "letterhead_save")).at(-1)!.args
    .letterhead as { firma: { name: string; zeilen: string[] } };
  expect(last.firma.name).toBe("Muster GmbH");
  expect(last.firma.zeilen).toEqual(["Musterstraße 1", "12345 Musterstadt"]);

  const dir = process.env.SCREENS_DIR;
  if (dir) {
    await page
      .getByTestId("documents-settings")
      .screenshot({ path: `${dir}/einstellungen-dokumente.png` });
  }
});

test("An WAI Presenter senden: nur mit Integration, gewaehlte Dokumente und Formate", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as any).__integrations = [
      {
        integration: {
          id: "int-p",
          kind: "presenter",
          label: "WAI Presenter",
          enabled: true,
        },
      },
    ];
  });
  await openDocuments(page);
  await page.getByTestId("presenter-send").click();
  const dialog = page.getByTestId("presenter-send-dialog");
  await expect(dialog).toBeVisible();
  await expect(page.getByTestId("presenter-doc-protokoll")).toBeChecked();
  await expect(page.getByTestId("presenter-format-pdf")).toBeChecked();
  await page.getByTestId("presenter-format-pptx").check();
  await page.getByTestId("presenter-send-submit").click();
  await expect(page.getByTestId("presenter-send-done")).toContainText(
    "pakete/2026-10-04-kundentermin-meyer-m1",
  );
  const [call] = await calls(page, "presenter_send");
  expect(call.args).toEqual({
    integrationId: "int-p",
    meetingId: "m1",
    docTypes: ["protokoll"],
    formats: ["pdf", "pptx"],
  });
  const dir = process.env.SCREENS_DIR;
  if (dir) {
    await page.screenshot({ path: `${dir}/presenter-senden.png` });
  }
});
