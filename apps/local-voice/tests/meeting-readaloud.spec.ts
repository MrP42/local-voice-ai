import { test, expect, type Page } from "@playwright/test";
import { markdownToSpeech } from "../src/lib/speechText";

// Aufnahmen: "Vorlesen" fuer erzeugte Inhalte (Protokoll, Dokumente) ueber den vorhandenen
// Befehl `tts_speak_text`, und der Status "Fertig" erscheint nirgends mehr (nur Laufendes
// und Fehler). Gegen die Tauri-Attrappe: Aufrufe in `window.__calls`, `tts_speak_text`
// haengt bis `window.__speakEnd()` oder `tts_cancel` (wie die echte Wiedergabe).

type Call = { cmd: string; args: Record<string, unknown> };

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(() => {
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
    };
    const spec = {
      version: 1,
      context: "Zweck",
      sections: [
        { id: "a", title: "A", instruction: "", kind: "text" },
        { id: "b", title: "B", instruction: "", kind: "tasks" },
      ],
    };
    w.__settings = settings;
    w.__calls = [];
    w.__template = null; // gemerkte Wahl der Besprechung
    w.__auto = null; // was die Automatik zuletzt gewaehlt hat
    w.__meta = null; // Anzeigedaten des juengsten Protokolls
    w.__runState = {
      running: false,
      progress: null,
      cancelling: false,
      started_at: null,
    };
    // "ok" | "pending" | "busy" | Fehlertext
    w.__generate = "ok";
    w.__release = null; // beendet ein haengendes Erzeugen
    w.__documents = [];
    w.__templates = [
      {
        id: "builtin:allgemein",
        title: "Allgemein",
        builtin: true,
        spec,
        updated_at: 1,
      },
      {
        id: "builtin:vertrieb",
        title: "Kundengespräch / Vertrieb",
        builtin: true,
        spec,
        updated_at: 1,
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
    w.__minutesDoc = (version: number, body: string) => ({
      id: `p${version}`,
      meeting_id: "m1",
      kind: "minutes",
      body_format: "markdown@1",
      body,
      version,
      created_at: 1790000900 + version,
      template_id: null,
      updated_at: 300 + version,
    });
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
              return "0.14.0";
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
                listeners[entry.event] = (listeners[entry.event] ?? []).filter(
                  (h) => h !== entry.handler,
                );
                handlerOf.delete(args.eventId as number);
              }
              return null;
            }
            case "get_selected_model":
              return "";
            case "meetings_start":
              return { ...w.__meetings[0], id: "m-neu", status: "recording" };
            case "meetings_is_recording":
              return false;
            case "meetings_recording_position":
              return null;
            case "meetings_list":
              return [...(w.__extraMeetings ?? []), ...w.__meetings];
            case "meetings_get_segments":
              return [];
            case "meetings_get_documents":
              return w.__documents;
            case "meetings_minutes_file":
              return null;
            case "meetings_get_template":
              return w.__template;
            case "meetings_set_template":
              w.__template = args.templateId;
              return null;
            case "meetings_get_auto_template":
              return w.__auto;
            case "meetings_minutes_meta":
              return w.__meta;
            case "meetings_minutes_state":
              return w.__runState;
            case "meetings_generate_minutes": {
              const mode = w.__generate as string;
              if (mode === "busy") throw "minutes_busy";
              if (mode === "pending") {
                w.__runState = {
                  running: true,
                  progress: { phase: "write", done: 1, total: 4 },
                  cancelling: false,
                  started_at: 1790001000,
                };
                await new Promise<void>((resolve) => {
                  w.__release = resolve;
                });
              } else if (mode !== "ok") {
                throw mode;
              }
              w.__documents = [
                ...w.__documents,
                w.__minutesDoc(1, "# Protokoll: Kundentermin Meyer\n\nFertig."),
              ];
              return w.__documents[w.__documents.length - 1];
            }
            case "meeting_notes_get":
              // Die Notizen sind der erste Reiter der Arbeitsflaeche.
              return {
                meeting_id: args.meetingId,
                blocks: [],
                revision: 0,
                updated_at: 0,
              };
            case "meeting_templates_list":
              return w.__templates;
            case "plugin:dialog|save":
              return null;
            case "tts_speak_text":
              w.__speaking = true;
              // Der Befehl kehrt erst nach der Wiedergabe zurueck.
              await new Promise<void>((resolve) => {
                w.__speakEnd = resolve;
              });
              w.__speaking = false;
              if (w.__speakError) throw w.__speakError;
              return null;
            case "tts_cancel":
              w.__speakEnd?.();
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

const calls = (page: Page, cmd: string) =>
  page.evaluate(
    (c) => (window as any).__calls.filter((x: Call) => x.cmd === c) as Call[],
    cmd,
  );

const openRecordings = async (page: Page) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/");
  await page.getByRole("button", { name: "Aufnahmen", exact: true }).click();
};

const MINUTES = [
  "# Protokoll: Kundentermin Meyer",
  "",
  "## Beschlüsse",
  "",
  "- **Angebot** bis Freitag senden [F3]",
  "- Termin im [Kalender](https://example.org) eintragen",
  "",
  "| Wer | Was |",
  "| --- | --- |",
  "| Anna | Angebot |",
].join("\n");

const SPOKEN =
  "Protokoll: Kundentermin Meyer.\n\nBeschlüsse.\n\nAngebot bis Freitag senden.\nTermin im Kalender eintragen.\n\nWer, Was.\nAnna, Angebot.";

const withMinutes = (page: Page) =>
  page.addInitScript((body) => {
    const w = window as any;
    w.__documents = [w.__minutesDoc(1, body)];
  }, MINUTES);

const openMinutes = async (page: Page) => {
  await openRecordings(page);
  await page.getByText("Kundentermin Meyer", { exact: true }).click();
  await page.getByRole("tab", { name: "Protokoll", exact: true }).click();
};

test.describe("Vorlesen in den Aufnahmen", () => {
  test("Markdown wird zu Klartext: ohne Zeichen, Belege und Tabellenstriche", () => {
    expect(markdownToSpeech(MINUTES)).toBe(SPOKEN);
    expect(markdownToSpeech("")).toBe("");
    expect(markdownToSpeech("**Nur** _fett_ und `Code`")).toBe(
      "Nur fett und Code",
    );
    expect(markdownToSpeech("1. Erstens\n2. Zweitens.\n- [x] Erledigt")).toBe(
      "Erstens.\nZweitens.\nErledigt.",
    );
    expect(markdownToSpeech("snake_case_name bleibt, 2 * 3 auch")).toBe(
      "snake_case_name bleibt, 2 * 3 auch",
    );
  });

  test("Vorlesen-Knopf liest Protokoll vor und stoppt", async ({ page }) => {
    await withMinutes(page);
    await openMinutes(page);
    const speak = page.getByTestId("minutes-readaloud");
    await expect(speak).toHaveAttribute("aria-label", "Vorlesen");

    await speak.click();
    // Der vorhandene Vorlese-Befehl bekommt den angezeigten Text als Klartext.
    await expect
      .poll(async () => (await calls(page, "tts_speak_text")).length)
      .toBe(1);
    expect((await calls(page, "tts_speak_text"))[0].args).toEqual({
      text: SPOKEN,
    });
    // Waehrend der Wiedergabe: Stopp.
    await expect(speak).toHaveAttribute("aria-label", "Vorlesen beenden");
    await expect(speak).toHaveAttribute("aria-pressed", "true");

    await speak.click();
    await expect
      .poll(async () => (await calls(page, "tts_cancel")).length)
      .toBe(1);
    await expect(speak).toHaveAttribute("aria-label", "Vorlesen");
    // Kein Fehler-Toast nach eigenem Stopp.
    await expect(page.locator("[data-sonner-toast]")).toHaveCount(0);

    // Zweiter Lauf, diesmal bis zum Ende: der Knopf springt von selbst zurueck.
    await speak.click();
    await expect(speak).toHaveAttribute("aria-label", "Vorlesen beenden");
    await page.evaluate(() => (window as any).__speakEnd());
    await expect(speak).toHaveAttribute("aria-label", "Vorlesen");
    expect((await calls(page, "tts_speak_text")).length).toBe(2);
  });

  test("Reiterwechsel beendet die Wiedergabe; ein Fehler des Vorlesens steht als Hinweis da", async ({
    page,
  }) => {
    await withMinutes(page);
    await openMinutes(page);
    const speak = page.getByTestId("minutes-readaloud");
    await speak.click();
    await expect(speak).toHaveAttribute("aria-label", "Vorlesen beenden");
    await page.getByRole("tab", { name: "Transkript", exact: true }).click();
    await expect
      .poll(async () => (await calls(page, "tts_cancel")).length)
      .toBe(1);

    await page.getByRole("tab", { name: "Protokoll", exact: true }).click();
    await page.evaluate(() => {
      (window as any).__speakError = "Der Sprachserver läuft nicht";
    });
    await page.getByTestId("minutes-readaloud").click();
    await page.evaluate(() => (window as any).__speakEnd());
    await expect(page.locator("[data-sonner-toast]")).toContainText(
      "Vorlesen fehlgeschlagen: Der Sprachserver läuft nicht",
    );
  });

  test("ohne Protokoll gibt es keinen Vorlesen-Knopf", async ({ page }) => {
    await openMinutes(page);
    await expect(page.getByTestId("minutes-empty")).toBeVisible();
    await expect(page.getByTestId("minutes-readaloud")).toHaveCount(0);
  });

  test("Screenshot: Detailansicht mit Vorlesen-Knopf", async ({ page }) => {
    const dir = process.env.SCREENS_DIR;
    test.skip(!dir, "SCREENS_DIR nicht gesetzt");
    await withMinutes(page);
    await openMinutes(page);
    await expect(page.getByTestId("minutes-readaloud")).toBeVisible();
    await page.screenshot({ path: `${dir}/aufnahmen-vorlesen.png` });
    await page.getByTestId("minutes-readaloud").click();
    await expect(page.getByTestId("minutes-readaloud")).toHaveAttribute(
      "aria-label",
      "Vorlesen beenden",
    );
    await page.screenshot({ path: `${dir}/aufnahmen-vorlesen-laeuft.png` });
  });
});

// ---------------------------------------------------------------------------
// Status: "Fertig" bleibt weg
// ---------------------------------------------------------------------------

const meeting = (id: string, title: string, status: string) => ({
  id,
  title,
  status,
  source: "recording",
  started_at: 1790100000,
  ended_at: 1790100600,
  language: "de",
  mic_audio_path: null,
  system_audio_path: null,
  duration_ms: 600000,
  consent_confirmed_at: 1790100000,
  audio_retention_until: null,
  created_at: 1790100000,
  source_path: null,
});

test.describe("Status nur bei Laufendem und Fehlern", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(
      (extra) => {
        (window as any).__extraMeetings = extra;
      },
      [
        meeting("m-fail", "Fehlerhafte Aufnahme", "failed"),
        meeting("m-proc", "Laufende Aufnahme", "processing"),
        meeting("m-wait", "Wartende Aufnahme", "queued"),
      ],
    );
    await openRecordings(page);
  });

  const row = (page: Page, id: string) =>
    page.locator(`[data-meeting-id="${id}"]`);

  test("Liste: fertig ohne Hinweis, Fehler deutlich rot, laufend und wartend sichtbar", async ({
    page,
  }) => {
    await expect(row(page, "m1")).toBeVisible();
    await expect(row(page, "m1")).not.toContainText("Fertig");
    await expect(row(page, "m1").getByTestId("row-status")).toHaveCount(0);
    await expect(page.getByText("Fertig", { exact: true })).toHaveCount(0);

    const failed = row(page, "m-fail").getByTestId("row-status");
    await expect(failed).toHaveText("Fehlgeschlagen");
    await expect(failed).toHaveClass(/text-red-500/);
    await expect(row(page, "m-proc").getByTestId("row-status")).toHaveText(
      "Wird verarbeitet",
    );
    await expect(row(page, "m-wait").getByTestId("row-status")).toHaveText(
      "Wartet",
    );
  });

  test("Kopf der Detailansicht: fertig ohne Chip, Fehler und Verarbeitung mit Chip", async ({
    page,
  }) => {
    await page.getByText("Kundentermin Meyer", { exact: true }).click();
    await expect(page.getByTestId("rec-detail-head")).toBeVisible();
    await expect(page.getByTestId("status-chip")).toHaveCount(0);
    await expect(page.getByTestId("rec-detail-head")).not.toContainText(
      "Fertig",
    );
    // Die anderen Chips bleiben.
    await expect(page.getByTestId("source-chip")).toBeVisible();
    await expect(page.getByTestId("duration-chip")).toBeVisible();

    await page.getByText("Fehlerhafte Aufnahme", { exact: true }).click();
    const chip = page.getByTestId("status-chip");
    await expect(chip).toHaveText("Fehlgeschlagen");
    await expect(chip).toHaveClass(/text-red-500/);

    await page.getByText("Laufende Aufnahme", { exact: true }).click();
    await expect(page.getByTestId("status-chip")).toHaveText(
      "Wird verarbeitet",
    );
  });
});
