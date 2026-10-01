import { test, expect, type Page } from "@playwright/test";
import { exportFileName } from "../src/lib/meetingExport";

// M6-P6d (AK10 Teil 4): Export-Oberfläche gegen die Tauri-Attrappe. Jeder
// Aufruf landet in `window.__calls`; der Speichern-Dialog liefert einen Pfad
// mit der Endung des angebotenen Filters, `meetings_export` und
// `meetings_copy_formatted` antworten, wie `__exportResult` / `__copyResult`
// es vorgeben.

type Call = { cmd: string; args: Record<string, any> };

const ALL = {
  notes: true,
  ai_notes: true,
  minutes: true,
  transcript: true,
  participants: true,
};

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(() => {
    const w = window as any;
    const callbacks = new Map<number, (e: unknown) => void>();
    let callback = 0;
    const settings: Record<string, any> = {
      onboarding_completed: true,
      app_language: "de",
      theme: "light",
      show_whats_new_on_update: false,
      debug_mode: false,
      selected_model: "",
      bindings: {},
      post_process_provider_id: "local",
      post_process_providers: [
        { id: "local", label: "Lokal", base_url: "http://127.0.0.1:0/v1" },
      ],
      post_process_prompts: [],
      custom_words: [],
      post_process_models: {},
      post_process_api_keys: {},
      push_to_talk: true,
      meeting_self_emails: [],
    };
    w.__settings = settings;
    w.__calls = [];
    w.__exportResult = { ok: true };
    w.__copyResult = { ok: true };
    w.__integrations = []; // A6: Ziele fuer „Ablegen in“
    w.__placeResult = { ok: true };
    w.__vaultResult = "created";
    w.__savePath = null; // null: Pfad aus dem Filter bilden
    w.__meetings = [
      {
        id: "m1",
        title: "Kundentermin: Meyer/Nord",
        status: "ready",
        source: "live",
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
        deleted_at: null,
      },
    ];
    w.__segments = Array.from({ length: 5 }, (_, i) => ({
      segment_index: i,
      text: `Satz Nummer ${i}.`,
      start_ms: i * 15_000,
      end_ms: i * 15_000 + 5_000,
      channel: 0,
      speaker_index: null,
    }));

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
        invoke: async (cmd: string, args: Record<string, any> = {}) => {
          w.__calls.push({ cmd, args: JSON.parse(JSON.stringify(args)) });
          switch (cmd) {
            case "get_app_settings":
            case "get_default_settings":
              return settings;
            case "plugin:os|locale":
              return "de-DE";
            case "plugin:app|version":
              return "0.14.0";
            case "plugin:event|listen":
              return ++callback;
            case "plugin:event|unlisten":
              return null;
            case "meetings_is_recording":
              return false;
            case "meetings_recording_position":
              return null;
            case "meetings_list":
              return w.__meetings;
            case "meetings_search":
              return {
                items: w.__meetings.map((m: any) => ({
                  meeting: m,
                  snippet: null,
                  hit_source: null,
                })),
                total: w.__meetings.length,
                truncated: false,
              };
            case "meeting_folders_list":
              return [];
            case "meetings_get_segments":
              return w.__segments;
            case "meetings_segment_epoch":
              return 1;
            case "meetings_get_documents":
              return [];
            case "meeting_notes_get":
              return {
                meeting_id: args.meetingId,
                blocks: [],
                revision: 1,
                updated_at: 0,
              };
            case "plugin:dialog|save": {
              if (w.__savePath !== null) return w.__savePath;
              const ext = args.options?.filters?.[0]?.extensions?.[0] ?? "bin";
              return `C:\\Temp\\Export.${ext}`;
            }
            case "meetings_export": {
              const r = w.__exportResult;
              if (r.ok) return null;
              throw r.error;
            }
            case "meetings_copy_formatted": {
              const r = w.__copyResult;
              if (r.ok) return null;
              throw r.error;
            }
            case "integrations_list":
              return w.__integrations;
            case "integration_export_to_folder": {
              const r = w.__placeResult;
              if (!r.ok) throw r.error;
              return {
                rel: `Protokolle/Kundentermin.${args.format}`,
                bytes: 100,
              };
            }
            case "integration_save_to_vault": {
              const r = w.__placeResult;
              if (!r.ok) throw r.error;
              return {
                rel: "00_inbox/2026-10-01 Kundentermin.md",
                result: w.__vaultResult,
              };
            }
            case "get_selected_model":
              return "";
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
            case "meeting_templates_list":
            case "action_items_list":
            case "chat_recipes_list":
            case "meeting_chat_threads":
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
  });
});

// ---------------------------------------------------------------------------
// Helfer
// ---------------------------------------------------------------------------

const calls = (page: Page, cmd: string) =>
  page.evaluate(
    (c) => (window as any).__calls.filter((x: Call) => x.cmd === c) as Call[],
    cmd,
  );

const setResult = (page: Page, key: string, value: Record<string, unknown>) =>
  page.evaluate(([k, v]) => ((window as any)[k as string] = v), [key, value]);

const toasts = (page: Page) => page.locator("[data-sonner-toast]");

const openExport = async (page: Page) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  await page.goto("/");
  await page.getByRole("button", { name: "Aufnahmen", exact: true }).click();
  await page.getByText("Kundentermin: Meyer/Nord", { exact: true }).click();
  await page.getByTestId("export-open").click();
  await expect(page.getByTestId("export-dialog")).toBeVisible();
};

// ---------------------------------------------------------------------------
// Reine Logik
// ---------------------------------------------------------------------------

test("Dateiname: verbotene Zeichen ersetzt, Endung angehängt, Leertitel gefüllt", () => {
  expect(exportFileName('Kunde: A/B "Nord"', "pdf")).toBe(
    "Kunde_ A_B _Nord_.pdf",
  );
  expect(exportFileName("  ", "srt")).toBe("besprechung.srt");
});

// ---------------------------------------------------------------------------
// Formate
// ---------------------------------------------------------------------------

test("Jedes Format öffnet den Speichern-Dialog und ruft meetings_export mit Pfad und Endung", async ({
  page,
}) => {
  await openExport(page);
  const formats = [
    ["docx", "Word"],
    ["txt", "TXT"],
    ["md", "Markdown"],
    ["html", "HTML"],
    ["pdf", "PDF"],
    ["srt", "SRT"],
    ["vtt", "VTT"],
    ["json", "JSON"],
  ] as const;
  await expect(
    page.getByTestId("export-dialog").getByRole("button"),
  ).toHaveCount(formats.length + 1); // + „Formatiert kopieren“

  for (const [ext, label] of formats) {
    await page.getByRole("button", { name: label, exact: true }).click();
    await expect
      .poll(async () => (await calls(page, "meetings_export")).length)
      .toBe(formats.findIndex(([e]) => e === ext) + 1);
    await expect(
      toasts(page).filter({
        hasText: `Exportiert nach C:\\Temp\\Export.${ext}`,
      }),
    ).toHaveCount(1);
  }

  const saves = await calls(page, "plugin:dialog|save");
  expect(saves.map((c) => c.args.options.filters[0].extensions)).toEqual(
    formats.map(([ext]) => [ext]),
  );
  // Vorschlag: bereinigter Titel + Endung, Filtername = Formatname.
  expect(saves[0].args.options.defaultPath).toBe(
    "Kundentermin_ Meyer_Nord.docx",
  );
  expect(saves[4].args.options.defaultPath).toBe(
    "Kundentermin_ Meyer_Nord.pdf",
  );
  expect(saves[4].args.options.filters[0].name).toBe("PDF");

  const exports = await calls(page, "meetings_export");
  expect(exports.map((c) => c.args)).toEqual(
    formats.map(([ext]) => ({
      meetingId: "m1",
      path: `C:\\Temp\\Export.${ext}`,
      parts: ALL,
    })),
  );
});

test("Abbruch im Speichern-Dialog: kein Export, keine Meldung", async ({
  page,
}) => {
  await openExport(page);
  await page.evaluate(() => {
    (window as any).__savePath = undefined; // Dialog abgebrochen
  });
  await page.getByRole("button", { name: "JSON", exact: true }).click();
  await expect
    .poll(async () => (await calls(page, "plugin:dialog|save")).length)
    .toBe(1);
  expect(await calls(page, "meetings_export")).toHaveLength(0);
  await expect(toasts(page)).toHaveCount(0);
});

test("Fehler beim Export erscheint als Toast; PDF nennt den HTML-Rückfall", async ({
  page,
}) => {
  await openExport(page);
  await setResult(page, "__exportResult", {
    ok: false,
    error: "Datei ist gesperrt",
  });
  await page.getByRole("button", { name: "Word", exact: true }).click();
  await expect(toasts(page).last()).toContainText(
    "Export fehlgeschlagen: Datei ist gesperrt",
  );
  await expect(toasts(page).last()).not.toContainText("Alternative");

  await setResult(page, "__exportResult", {
    ok: false,
    error: "pdf_unavailable: WebView2 fehlt",
  });
  await page.getByRole("button", { name: "PDF", exact: true }).click();
  await expect(
    toasts(page).filter({ hasText: "pdf_unavailable" }),
  ).toContainText("Alternative: als HTML speichern");
});

// ---------------------------------------------------------------------------
// Teile
// ---------------------------------------------------------------------------

test("Teile-Häkchen gehen an meetings_export und meetings_copy_formatted", async ({
  page,
}) => {
  await openExport(page);
  const dialog = page.getByTestId("export-dialog");
  for (const name of ["Meine Notizen", "Transkript", "Teilnehmende"]) {
    await expect(dialog.getByLabel(name, { exact: true })).toBeChecked();
  }
  await dialog.getByLabel("Transkript", { exact: true }).uncheck();
  await dialog.getByLabel("Teilnehmende", { exact: true }).uncheck();

  await page.getByRole("button", { name: "Markdown", exact: true }).click();
  await expect
    .poll(async () => (await calls(page, "meetings_export")).length)
    .toBe(1);
  await page.getByTestId("export-copy").click();
  await expect
    .poll(async () => (await calls(page, "meetings_copy_formatted")).length)
    .toBe(1);

  const parts = {
    notes: true,
    ai_notes: true,
    minutes: true,
    transcript: false,
    participants: false,
  };
  expect((await calls(page, "meetings_export"))[0].args).toEqual({
    meetingId: "m1",
    path: "C:\\Temp\\Export.md",
    parts,
  });
  expect((await calls(page, "meetings_copy_formatted"))[0].args).toEqual({
    meetingId: "m1",
    parts,
  });
});

test("Ohne gewählten Teil sind alle Knöpfe gesperrt und ein Hinweis erscheint", async ({
  page,
}) => {
  await openExport(page);
  const dialog = page.getByTestId("export-dialog");
  for (const name of [
    "Meine Notizen",
    "KI-Notizen",
    "Protokoll",
    "Transkript",
    "Teilnehmende",
  ]) {
    await dialog.getByLabel(name, { exact: true }).uncheck();
  }
  await expect(
    dialog.getByText("Wähle mindestens einen Teil aus."),
  ).toBeVisible();
  await expect(page.getByTestId("export-docx")).toBeDisabled();
  await expect(page.getByTestId("export-srt")).toBeDisabled();
  await expect(page.getByTestId("export-copy")).toBeDisabled();
  await dialog.getByLabel("Protokoll", { exact: true }).check();
  await expect(page.getByTestId("export-copy")).toBeEnabled();
});

// ---------------------------------------------------------------------------
// Zwischenablage
// ---------------------------------------------------------------------------

test("„Formatiert kopieren“ ruft meetings_copy_formatted und meldet Erfolg", async ({
  page,
}) => {
  await openExport(page);
  await page.getByRole("button", { name: "Formatiert kopieren" }).click();
  await expect(toasts(page).last()).toContainText(
    "Formatiert in die Zwischenablage kopiert.",
  );
  const copies = await calls(page, "meetings_copy_formatted");
  expect(copies).toHaveLength(1);
  expect(copies[0].args).toEqual({ meetingId: "m1", parts: ALL });
  // Kein Datei-Export nebenbei.
  expect(await calls(page, "meetings_export")).toHaveLength(0);
  expect(await calls(page, "plugin:dialog|save")).toHaveLength(0);
});

test("„Formatiert kopieren“: Fehler erscheint als Toast", async ({ page }) => {
  await openExport(page);
  await setResult(page, "__copyResult", {
    ok: false,
    error: "Zwischenablage belegt",
  });
  await page.getByRole("button", { name: "Formatiert kopieren" }).click();
  await expect(toasts(page).last()).toContainText(
    "Kopieren fehlgeschlagen: Zwischenablage belegt",
  );
});

test("Export-Dialog (Screenshot)", async ({ page }) => {
  test.skip(
    !process.env.LVA_SCREENSHOTS,
    "Abnahme-Bild nur mit LVA_SCREENSHOTS=1",
  );
  await openExport(page);
  await page.getByTestId("export-dialog").getByLabel("Transkript").uncheck();
  await page.screenshot({
    path: "../../koordination/granola-besprechungen/abnahme/p6d-export.png",
    animations: "disabled",
  });
});

// ---------------------------------------------------------------------------
// A6: Ablegen in einen Ordner oder Obsidian-Vault
// ---------------------------------------------------------------------------

const target = (
  id: string,
  kind: string,
  label: string,
  extra: Record<string, unknown> = {},
) => ({
  integration: {
    id,
    kind,
    label,
    enabled: true,
    direction: "both",
    ...extra,
  },
});

const setTargets = (page: Page, list: unknown[]) =>
  page.evaluate((l) => ((window as any).__integrations = l), list);

test("Ablegen in: Ordner und Vault erscheinen, ausgeschaltete und lesende nicht", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as any).__integrations = [];
  });
  await openExport(page);
  await expect(page.getByTestId("export-place")).toHaveCount(0);
  await page.getByRole("button", { name: "Schließen" }).first().click();
  await setTargets(page, [
    target("f1", "folder", "Ablage Berichte"),
    target("v1", "obsidian", "AI-OS Vault"),
    target("f2", "folder", "Ausgeschaltet", { enabled: false }),
    target("f3", "folder", "Nur lesen", { direction: "read" }),
    target("y1", "youtube", "YouTube"),
  ]);
  await page.getByTestId("export-open").click();
  const rows = page.getByTestId("export-place-target");
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0)).toContainText("Ordner „Ablage Berichte“");
  await expect(rows.nth(1)).toContainText("Als Notiz in „AI-OS Vault“");
  await expect(rows.nth(0).getByRole("button")).toHaveText([
    "Markdown",
    "Word",
    "PDF",
  ]);
});

test("Ablegen in einen Ordner: Format und gewählte Teile gehen ans Backend, der Pfad steht in der Meldung", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as any).__integrations = [
      {
        integration: {
          id: "f1",
          kind: "folder",
          label: "Ablage Berichte",
          enabled: true,
          direction: "both",
        },
      },
    ];
  });
  await openExport(page);
  await page
    .getByTestId("export-dialog")
    .getByLabel("Transkript", { exact: true })
    .uncheck();
  await page.getByTestId("place-f1-docx").click();
  await expect
    .poll(
      async () => (await calls(page, "integration_export_to_folder")).length,
    )
    .toBe(1);
  const [call] = await calls(page, "integration_export_to_folder");
  expect(call.args).toEqual({
    id: "f1",
    meetingId: "m1",
    format: "docx",
    parts: { ...ALL, transcript: false },
  });
  await expect(toasts(page).last()).toContainText(
    "Abgelegt in „Ablage Berichte“: Protokolle/Kundentermin.docx",
  );
  // Kein Speichern-Dialog: das Ziel steht fest.
  expect(await calls(page, "plugin:dialog|save")).toHaveLength(0);
});

test("Notiz im Vault: angelegt, aktualisiert und unverändert werden unterschieden", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as any).__integrations = [
      {
        integration: {
          id: "v1",
          kind: "obsidian",
          label: "AI-OS Vault",
          enabled: true,
          direction: "both",
        },
      },
    ];
  });
  await openExport(page);
  for (const [result, text] of [
    [
      "created",
      "Notiz angelegt in „AI-OS Vault“: 00_inbox/2026-10-01 Kundentermin.md",
    ],
    ["updated", "Notiz aktualisiert in „AI-OS Vault“"],
    ["unchanged", "ist schon aktuell"],
  ] as const) {
    await page.evaluate((r) => ((window as any).__vaultResult = r), result);
    await page.getByTestId("place-v1-vault").click();
    await expect(toasts(page).filter({ hasText: text })).toHaveCount(1);
  }
  const saves = await calls(page, "integration_save_to_vault");
  expect(saves).toHaveLength(3);
  expect(saves[0].args).toEqual({ id: "v1", meetingId: "m1", parts: ALL });
});

test("Ablegen scheitert verständlich: Code wird übersetzt, Klartext bleibt stehen", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as any).__integrations = [
      {
        integration: {
          id: "f1",
          kind: "folder",
          label: "Ablage",
          enabled: true,
          direction: "both",
        },
      },
    ];
  });
  await openExport(page);
  await setResult(page, "__placeResult", {
    ok: false,
    error: "folder_path_escape",
  });
  await page.getByTestId("place-f1-md").click();
  await expect(
    toasts(page).filter({
      hasText: "Ablegen nicht möglich: Der Unterordner ist nicht zulässig",
    }),
  ).toHaveCount(1);
  await setResult(page, "__placeResult", {
    ok: false,
    error: "Der Ordner wurde nicht gefunden.",
  });
  await page.getByTestId("place-f1-pdf").click();
  await expect(
    toasts(page).filter({
      hasText: "Ablegen nicht möglich: Der Ordner wurde nicht gefunden.",
    }),
  ).toHaveCount(1);
});
