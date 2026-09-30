import { test, expect, type Page } from "@playwright/test";
import {
  minutesErrorCode,
  minutesErrorDetail,
  progressPercent,
} from "../src/lib/meetingMinutes";

// P1k: Protokoll-Reiter mit Vorlagenwahl (auch "Automatisch (nach Inhalt)"),
// Laufzustand ueber Reiterwechsel (B14) und Warnung bei Luecken. Gegen die
// Tauri-Attrappe: jeder Aufruf steht in `window.__calls`, Ereignisse loest
// `window.__emit` aus, der Ausgang von `meetings_generate_minutes` steuert
// `window.__generate`.

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
              return w.__meetings;
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

const emit = (page: Page, event: string, payload: unknown) =>
  page.evaluate(([e, p]) => (window as any).__emit(e, p), [event, payload]);

const openRecordings = async (page: Page) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/");
  await page.getByRole("button", { name: "Aufnahmen", exact: true }).click();
};

const openMinutes = async (page: Page) => {
  await openRecordings(page);
  await page.getByText("Kundentermin Meyer", { exact: true }).click();
  await page.getByRole("tab", { name: "Protokoll", exact: true }).click();
};

const generateButton = (page: Page) =>
  page.getByRole("button", { name: /^(Erzeugen|Neu erzeugen)$/ });

const pickTemplate = async (page: Page, name: string) => {
  await page.locator(".app-select__control").first().click();
  await page.getByRole("option", { name, exact: true }).click();
};

// ---------------------------------------------------------------------------
// Reine Logik (ohne Browser)
// ---------------------------------------------------------------------------

test.describe("Protokoll Logik", () => {
  test("Fehlercodes: ganzer Code, mit Detail, unbekannt", () => {
    expect(minutesErrorCode("minutes_busy")).toBe("minutes_busy");
    expect(minutesErrorCode("no_provider: Kein LLM-Provider")).toBe(
      "no_provider",
    );
    expect(minutesErrorCode("minutes_busy_not")).toBe("llm_failed");
    expect(minutesErrorCode("irgendwas Neues")).toBe("llm_failed");
    expect(minutesErrorDetail("llm_failed: Zeitlimit")).toBe("Zeitlimit");
    expect(minutesErrorDetail("ganz anderer Text")).toBe("ganz anderer Text");
  });

  test("Prozent: unbestimmt bei total 0, sonst gerundet und begrenzt", () => {
    expect(progressPercent(null)).toBeNull();
    expect(progressPercent({ done: 0, total: 0 })).toBeNull();
    expect(progressPercent({ done: 1, total: 4 })).toBe(25);
    expect(progressPercent({ done: 2, total: 3 })).toBe(67);
    expect(progressPercent({ done: 9, total: 4 })).toBe(100);
    expect(progressPercent({ done: -1, total: 4 })).toBe(0);
  });
});

// ---------------------------------------------------------------------------
// Vorlagenwahl im Protokoll-Reiter
// ---------------------------------------------------------------------------

test("Protokoll: Vorlagenwahl bietet die Vorlagen und Automatisch, Erzeugen ruft den Befehl mit der Vorlage", async ({
  page,
}) => {
  await openMinutes(page);
  await page.locator(".app-select__control").first().click();
  for (const name of [
    "Automatisch (nach Inhalt)",
    "Allgemein",
    "Kundengespräch / Vertrieb",
  ]) {
    await expect(page.getByRole("option", { name, exact: true })).toBeVisible();
  }
  await page
    .getByRole("option", { name: "Kundengespräch / Vertrieb", exact: true })
    .click();
  // Die Wahl gilt je Besprechung (geteilt mit den KI-Notizen).
  await expect
    .poll(async () => (await calls(page, "meetings_set_template")).length)
    .toBe(1);
  expect((await calls(page, "meetings_set_template"))[0].args).toEqual({
    meetingId: "m1",
    templateId: "builtin:vertrieb",
  });

  await generateButton(page).click();
  await expect
    .poll(async () => (await calls(page, "meetings_generate_minutes")).length)
    .toBe(1);
  expect((await calls(page, "meetings_generate_minutes"))[0].args).toEqual({
    meetingId: "m1",
    templateId: "builtin:vertrieb",
  });
  await expect(page.getByText("Fertig.")).toBeVisible();
  await expect(generateButton(page)).toBeEnabled();
});

test("Protokoll: die gemerkte Wahl der Besprechung steht schon im Wähler und geht mit", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as any).__template = "builtin:vertrieb";
  });
  await openMinutes(page);
  await expect(page.locator(".app-select__single-value").first()).toHaveText(
    "Kundengespräch / Vertrieb",
  );
  // Eine Nutzerwahl braucht keine Automatik: kein Hinweis "Automatisch".
  await expect(page.getByTestId("template-auto-note")).toHaveCount(0);
  await generateButton(page).click();
  await expect
    .poll(async () => (await calls(page, "meetings_generate_minutes")).length)
    .toBe(1);
  expect(
    ((await calls(page, "meetings_generate_minutes"))[0].args as any)
      .templateId,
  ).toBe("builtin:vertrieb");
});

test("Protokoll: Automatisch zeigt nach dem Erzeugen, welche Vorlage gewählt wurde", async ({
  page,
}) => {
  await openMinutes(page);
  await pickTemplate(page, "Automatisch (nach Inhalt)");
  await expect
    .poll(async () => (await calls(page, "meetings_set_template")).length)
    .toBe(1);
  expect(
    ((await calls(page, "meetings_set_template"))[0].args as any).templateId,
  ).toBe("auto");
  // Vor dem ersten Lauf: noch nichts gewählt, der Hinweis sagt wann.
  const note = page.getByTestId("template-auto-note");
  await expect(note).toHaveText("Wird beim Erzeugen nach dem Inhalt gewählt.");
  await expect(note).toHaveAttribute("data-state", "pending");

  // Das Backend wählt beim Erzeugen "Kundengespräch / Vertrieb" und meldet das Ende.
  await page.evaluate(() => {
    const w = window as any;
    w.__auto = {
      template_id: "builtin:vertrieb",
      title: "Kundengespräch / Vertrieb",
      reason: "Angebot und Budget des Kunden",
      outcome: "model",
    };
    w.__meta = {
      document_id: "p1",
      template_id: "builtin:vertrieb",
      template_title: "Kundengespräch / Vertrieb",
      auto: w.__auto,
      incomplete: false,
      gaps: [],
      chunks_total: 1,
      chunks_split: 0,
    };
  });
  await generateButton(page).click();
  await expect
    .poll(async () => (await calls(page, "meetings_generate_minutes")).length)
    .toBe(1);
  expect(
    ((await calls(page, "meetings_generate_minutes"))[0].args as any)
      .templateId,
  ).toBe("auto");
  await emit(page, "minutes-event", {
    kind: "done",
    meeting_id: "m1",
    document_id: "p1",
  });
  await expect(note).toHaveText("Automatisch: Kundengespräch / Vertrieb");
  await expect(note).toHaveAttribute("data-state", "model");
  await expect(note).toHaveAttribute("data-template-id", "builtin:vertrieb");
  await expect(note).toHaveAttribute(
    "title",
    "Begründung: Angebot und Budget des Kunden",
  );
  await expect(page.getByTestId("minutes-created-with")).toHaveText(
    "Erzeugt mit der Vorlage: Kundengespräch / Vertrieb (automatisch gewählt)",
  );
  // Jederzeit änderbar: die Wahl "Allgemein" ersetzt die Automatik.
  await pickTemplate(page, "Allgemein");
  await expect(note).toHaveCount(0);
});

test("Protokoll: eine unklare Automatik sagt, dass die Standardvorlage gilt", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const w = window as any;
    w.__template = "auto";
    w.__auto = {
      template_id: "builtin:allgemein",
      title: "Allgemein",
      reason: "",
      outcome: "uncertain",
    };
  });
  await openMinutes(page);
  await expect(page.locator(".app-select__single-value").first()).toHaveText(
    "Automatisch (nach Inhalt)",
  );
  await expect(page.getByTestId("template-auto-note")).toContainText(
    "Automatisch: Allgemein – Nicht eindeutig – es gilt die Standardvorlage.",
  );
});

// ---------------------------------------------------------------------------
// Laufzustand (B14)
// ---------------------------------------------------------------------------

test("Protokoll: Reiterwechsel waehrend des Laufs behaelt Sperre und Fortschritt, kein zweiter Start", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as any).__generate = "pending";
  });
  await openMinutes(page);
  await generateButton(page).click();
  await expect(page.getByTestId("minutes-running")).toBeVisible();
  await expect(generateButton(page)).toBeDisabled();
  await expect(
    page.getByRole("progressbar", { name: "Fortschritt des Protokolls" }),
  ).toBeVisible();

  // Reiter verlassen und zurueck: das Backend laeuft weiter, der Reiter fragt nach.
  await page.getByRole("tab", { name: "Notizen", exact: true }).click();
  await expect(page.getByTestId("minutes-running")).toHaveCount(0);
  await page.getByRole("tab", { name: "Protokoll", exact: true }).click();
  await expect(page.getByTestId("minutes-running")).toBeVisible();
  await expect(generateButton(page)).toBeDisabled();
  // Der Zustand kam vom Backend, mit dem Stand des Laufs: Schritt 1 von 4 = 25 %.
  await expect(page.getByTestId("minutes-percent")).toContainText("25 %");
  await expect(page.getByTestId("minutes-phase")).toHaveText(
    "Transkript wird ausgewertet …",
  );
  expect((await calls(page, "meetings_minutes_state")).length).toBeGreaterThan(
    1,
  );
  // Der Knopf ist gesperrt: es gab nur den einen Start.
  expect(await calls(page, "meetings_generate_minutes")).toHaveLength(1);

  // Fortschrittsereignisse: die eigene Besprechung zaehlt, fremde nicht.
  await emit(page, "minutes-event", {
    kind: "progress",
    meeting_id: "andere",
    phase: "write",
    done: 4,
    total: 4,
  });
  await expect(page.getByTestId("minutes-percent")).toContainText("25 %");
  await emit(page, "minutes-event", {
    kind: "progress",
    meeting_id: "m1",
    phase: "merge",
    done: 3,
    total: 4,
  });
  await expect(page.getByTestId("minutes-percent")).toContainText("75 %");
  await expect(page.getByTestId("minutes-phase")).toHaveText(
    "Teile werden zusammengeführt …",
  );
  await expect(
    page.getByRole("progressbar", { name: "Fortschritt des Protokolls" }),
  ).toHaveAttribute("aria-valuenow", "75");
  // Auch die Vorlagenwahl ist waehrend des Laufs gesperrt.
  await expect(page.locator(".app-select__control").first()).toHaveClass(
    /--is-disabled/,
  );

  // Ende: Sperre weg, Protokoll da.
  await page.evaluate(() => {
    const w = window as any;
    w.__runState = {
      running: false,
      progress: null,
      cancelling: false,
      started_at: null,
    };
    w.__release?.();
  });
  await emit(page, "minutes-event", {
    kind: "done",
    meeting_id: "m1",
    document_id: "p1",
  });
  await expect(page.getByTestId("minutes-running")).toHaveCount(0);
  await expect(page.getByText("Fertig.")).toBeVisible();
  await expect(generateButton(page)).toBeEnabled();
});

test("Protokoll: ein Lauf, der schon besteht, sperrt den Knopf beim Öffnen", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as any).__runState = {
      running: true,
      progress: { phase: "template", done: 0, total: 0 },
      cancelling: false,
      started_at: 1790001000,
    };
  });
  await openMinutes(page);
  await expect(generateButton(page)).toBeDisabled();
  await expect(page.getByTestId("minutes-running")).toBeVisible();
  // Unbestimmt (Vorlagenwahl): kein Prozentwert, aber die Phase.
  await expect(page.getByTestId("minutes-phase")).toHaveText(
    "Vorlage wird gewählt …",
  );
  await expect(page.getByTestId("minutes-percent")).toHaveText("");
  expect(await calls(page, "meetings_generate_minutes")).toHaveLength(0);
});

test("Protokoll: ein abgewiesener zweiter Start ist kein Fehler, der laufende Lauf bleibt sichtbar", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const w = window as any;
    w.__generate = "busy";
    w.__runState = {
      running: false,
      progress: null,
      cancelling: false,
      started_at: null,
    };
  });
  await openMinutes(page);
  // Zwischen Anzeige und Klick hat ein anderer Reiter den Lauf gestartet.
  await page.evaluate(() => {
    (window as any).__runState = {
      running: true,
      progress: { phase: "write", done: 2, total: 4 },
      cancelling: false,
      started_at: 1790001000,
    };
  });
  await generateButton(page).click();
  await expect(page.getByTestId("minutes-running")).toBeVisible();
  await expect(generateButton(page)).toBeDisabled();
  await expect(page.getByTestId("minutes-percent")).toContainText("50 %");
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(page.getByText("wird schon ein Protokoll erzeugt")).toHaveCount(
    0,
  );
});

test("Protokoll: Fehlercodes erscheinen als verständlicher Text, der Knopf ist wieder frei", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as any).__generate = "no_provider: Kein LLM-Provider konfiguriert";
  });
  await openMinutes(page);
  await generateButton(page).click();
  await expect(
    page.getByText(
      "Kein Sprachmodell eingerichtet (Einstellungen → Nachbearbeitung).",
    ),
  ).toBeVisible();
  await expect(generateButton(page)).toBeEnabled();
  await expect(page.getByTestId("minutes-running")).toHaveCount(0);

  // Ereignis eines fehlgeschlagenen Laufs aus einem anderen Reiter.
  await emit(page, "minutes-event", {
    kind: "failed",
    meeting_id: "m1",
    code: "memory_low",
  });
  await expect(page.getByText("Zu wenig freier Arbeitsspeicher")).toBeVisible();
  // Ein Stopp ist kein Fehler.
  await emit(page, "minutes-event", {
    kind: "failed",
    meeting_id: "m1",
    code: "minutes_cancelled",
  });
  await expect(page.getByTestId("minutes-running")).toHaveCount(0);
});

// ---------------------------------------------------------------------------
// Warnung bei Luecken (B12)
// ---------------------------------------------------------------------------

test("Protokoll: fehlende Teile des Transkripts stehen als sichtbare Warnung da", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const w = window as any;
    w.__documents = [
      w.__minutesDoc(1, "# Protokoll: Kundentermin Meyer\n\nTeilweise."),
    ];
    w.__meta = {
      document_id: "p1",
      template_id: "builtin:allgemein",
      template_title: "Allgemein",
      auto: null,
      incomplete: true,
      gaps: ["12:00-24:30", "40:00-41:10"],
      chunks_total: 5,
      chunks_split: 1,
    };
  });
  await openMinutes(page);
  await expect(page.getByTestId("minutes-incomplete")).toHaveText(
    "Das Protokoll ist unvollständig: Die Besprechung bei 12:00-24:30, 40:00-41:10 konnte nicht ausgewertet werden. Das vollständige Transkript bleibt erhalten.",
  );
  await expect(page.getByTestId("minutes-created-with")).toHaveText(
    "Erzeugt mit der Vorlage: Allgemein",
  );
});

test("Protokoll: ein vollständiges Protokoll zeigt keine Warnung", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const w = window as any;
    w.__documents = [w.__minutesDoc(1, "# Protokoll: X\n\nGanz.")];
    w.__meta = {
      document_id: "p1",
      template_id: "builtin:allgemein",
      template_title: "Allgemein",
      auto: null,
      incomplete: false,
      gaps: [],
      chunks_total: 1,
      chunks_split: 0,
    };
  });
  await openMinutes(page);
  await expect(page.getByText("Ganz.")).toBeVisible();
  await expect(page.getByTestId("minutes-incomplete")).toHaveCount(0);
});

// ---------------------------------------------------------------------------
// Automatisch bei neuen Aufnahmen
// ---------------------------------------------------------------------------

test.describe("Automatisch bei neuen Aufnahmen", () => {
  // Die Vorlagenwahl steht seit M4 im Startdialog.
  const openStart = async (page: Page) => {
    await page.getByRole("button", { name: "Aufnahme starten" }).click();
    await expect(page.getByTestId("record-template")).toBeVisible();
  };
  const startNow = async (page: Page) => {
    await page
      .getByRole("button", { name: "Alle Beteiligten haben zugestimmt" })
      .click();
  };

  test("Der Startdialog bietet Automatisch, die Wahl geht an die Besprechung", async ({
    page,
  }) => {
    await openRecordings(page);
    await openStart(page);
    const picker = page.getByTestId("record-template");
    await picker.locator(".app-select__control").click();
    await page
      .getByRole("option", { name: "Automatisch (nach Inhalt)", exact: true })
      .click();
    // Vor der Klassifikation gibt es nichts zu zeigen: nur der Hinweis, wann sie kommt.
    await expect(picker.getByTestId("template-auto-note")).toBeVisible();
    await startNow(page);
    await expect
      .poll(async () => (await calls(page, "meetings_set_template")).length)
      .toBe(1);
    expect((await calls(page, "meetings_set_template"))[0].args).toEqual({
      meetingId: "m-neu",
      templateId: "auto",
    });
  });

  test("Als Standard eingestellt gilt Automatisch, solange der Nutzer nichts anderes wählt", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      (window as any).__settings.meeting_default_template_id = "auto";
    });
    await openRecordings(page);
    await openStart(page);
    await expect(
      page.getByTestId("record-template").locator(".app-select__single-value"),
    ).toHaveText("Automatisch (nach Inhalt)");
    // Nutzerwahl hat Vorrang vor der Standardeinstellung.
    await page
      .getByTestId("record-template")
      .locator(".app-select__control")
      .click();
    await page
      .getByRole("option", { name: "Kundengespräch / Vertrieb", exact: true })
      .click();
    await startNow(page);
    await expect
      .poll(async () => (await calls(page, "meetings_set_template")).length)
      .toBe(1);
    expect(
      ((await calls(page, "meetings_set_template"))[0].args as any).templateId,
    ).toBe("builtin:vertrieb");
  });
});
