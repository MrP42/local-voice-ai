import { test, expect, type Locator, type Page } from "@playwright/test";

// P8a: Fortschritt, Pause und Stopp der Verarbeitung einer Besprechung gegen
// die Tauri-Attrappe. Die Attrappe spielt das Backend nicht nach: Befehle
// (`meetings_job_pause` ...) werden nur mitgeschrieben, die Antworten des
// Backends kommen als `window.__emit("meeting-event", ...)` -- genau der Weg, auf
// dem sie auch in der App eintreffen. Der Zustand beim Oeffnen kommt aus
// `meetings_progress_list` (`window.__progressList`).

type Call = { cmd: string; args: Record<string, any> };

const MEETING = {
  id: "m1",
  title: "Jour Fixe Vertrieb",
  status: "processing",
  source: "import",
  started_at: 1790000000,
  ended_at: null,
  language: "de",
  mic_audio_path: null,
  system_audio_path: null,
  duration_ms: 4_200_000,
  consent_confirmed_at: 1790000000,
  audio_retention_until: null,
  created_at: 1790000000,
  source_path: "C:/Import/Jour Fixe.mp4",
  deleted_at: null,
};

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript((meeting) => {
    const w = window as any;
    const callbacks = new Map<number, (e: unknown) => void>();
    const listeners: Record<string, number[]> = {};
    const handlerOf = new Map<number, { event: string; handler: number }>();
    let callback = 0;
    w.__settings = {
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
    };
    w.__calls = [];
    w.__meetings = [{ ...meeting }];
    w.__progressList = [];
    w.__minutesState = {
      running: false,
      progress: null,
      cancelling: false,
      started_at: null,
    };
    w.__jobError = null;
    w.__segments = Array.from({ length: 40 }, (_, i) => ({
      segment_index: i,
      text: `Segment ${i} mit etwas mehr Text, damit die Zeile eine sinnvolle Höhe hat.`,
      start_ms: i * 10_000,
      end_ms: i * 10_000 + 9_000,
      channel: 2,
      speaker_index: null,
    }));
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
        invoke: async (cmd: string, args: Record<string, any> = {}) => {
          w.__calls.push({ cmd, args: JSON.parse(JSON.stringify(args)) });
          switch (cmd) {
            case "get_app_settings":
            case "get_default_settings":
              return w.__settings;
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
            case "meetings_is_recording":
              return false;
            case "meetings_recording_position":
              return null;
            case "meetings_list":
              return w.__meetings;
            case "meetings_get_segments":
              return w.__segments;
            case "meetings_segment_epoch":
              return 0;
            case "meetings_progress_list":
              return w.__progressList;
            case "meetings_minutes_state":
              return w.__minutesState;
            case "meetings_job_pause":
            case "meetings_job_resume":
            case "meetings_job_stop":
            case "meetings_continue":
              if (w.__jobError) throw w.__jobError;
              return null;
            case "meetings_get_documents":
            case "action_items_list":
            case "meeting_speakers_list":
            case "meeting_speaker_notices":
            case "meeting_participants":
              return [];
            case "meeting_notes_get":
              return {
                meeting_id: args.meetingId,
                blocks: [],
                revision: 0,
                updated_at: 0,
              };
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
  }, MEETING);
});

const calls = (page: Page, cmd: string) =>
  page.evaluate(
    (c) => (window as any).__calls.filter((x: Call) => x.cmd === c) as Call[],
    cmd,
  );

const emit = (page: Page, payload: Record<string, unknown>) =>
  page.evaluate((p) => (window as any).__emit("meeting-event", p), payload);

/** 70 Minuten Audio, davon 42 % (29:24) verarbeitet, 12:30 Laufzeit, ~24 Min. Rest. */
const progress = (over: Record<string, unknown> = {}) => ({
  kind: "progress",
  meeting_id: "m1",
  phase: "transcription",
  done: 1_764_000,
  total: 4_200_000,
  elapsed_ms: 750_000,
  eta_ms: 1_440_000,
  state: "running",
  pausable: true,
  ...over,
});

const snapshot = (over: Record<string, unknown> = {}) => {
  const { kind: _kind, ...rest } = progress(over);
  return rest;
};

const setMeetings = (page: Page, patch: Record<string, unknown>) =>
  page.evaluate((p) => {
    const w = window as any;
    w.__meetings = w.__meetings.map((m: any) => ({ ...m, ...p }));
  }, patch);

const openList = async (page: Page) => {
  await page.setViewportSize({ width: 1280, height: 1000 });
  await page.goto("/");
  await page.getByRole("button", { name: "Aufnahmen", exact: true }).click();
  // Die Liste steht immer links; eine gemerkte Auswahl zeigt den Titel auch
  // in der Arbeitsflaeche (daher der erste Treffer).
  await expect(
    page.getByText("Jour Fixe Vertrieb", { exact: true }).first(),
  ).toBeVisible();
};

const openDetail = async (page: Page) => {
  await openList(page);
  await page.getByText("Jour Fixe Vertrieb", { exact: true }).first().click();
  await expect(page.locator('[data-segment-index="0"]')).toBeVisible();
};

const panel = (page: Page) => page.getByTestId("job-panel");

test.describe("Besprechungsliste", () => {
  test("ohne Fortschritt bleibt der Chip, mit Fortschritt steht ein Balken mit Prozent und Restdauer da", async ({
    page,
  }) => {
    await openList(page);
    const row = page.locator('[data-meeting-id="m1"]');
    // Noch kein Ereignis (etwa ein wartender Auftrag): der Chip wie bisher.
    await expect(row.getByText("Wird verarbeitet")).toBeVisible();
    await expect(row.getByTestId("job-bar")).toHaveCount(0);

    await emit(page, progress());
    const bar = row.getByTestId("job-bar");
    await expect(bar).toBeVisible();
    await expect(bar).toContainText("Transkription");
    await expect(bar.getByTestId("job-bar-percent")).toHaveText("42 %");
    await expect(bar.getByTestId("job-bar-note")).toHaveText(
      "noch ca. 24 Min.",
    );
    await expect(bar.getByRole("progressbar")).toHaveAttribute(
      "aria-valuenow",
      "42",
    );
    await expect(row.getByText("Wird verarbeitet")).toHaveCount(0);

    // Ein Klick auf den Balken öffnet die Besprechung nicht (er ist keine Zeile).
    await bar.click();
    await expect(page.getByTestId("job-panel")).toHaveCount(0);

    // Fertig: Balken weg, Chip "Fertig".
    await setMeetings(page, { status: "ready" });
    await emit(page, {
      kind: "state",
      meeting_id: "m1",
      status: "ready",
      paused: false,
    });
    await expect(row.getByTestId("job-bar")).toHaveCount(0);
    await expect(row.getByText("Fertig", { exact: true })).toBeVisible();
  });

  test("pausiert und gestoppt: die Liste sagt es, ein abgebrochener Import trägt den Chip Abgebrochen", async ({
    page,
  }) => {
    await openList(page);
    const row = page.locator('[data-meeting-id="m1"]');
    await emit(page, progress({ state: "paused" }));
    await expect(row.getByTestId("job-bar-note")).toHaveText("Pausiert");
    // Pausiert: keine Restdauer.
    await expect(row.getByTestId("job-bar")).not.toContainText("noch ca.");

    await setMeetings(page, { status: "cancelled" });
    await emit(page, {
      kind: "state",
      meeting_id: "m1",
      status: "cancelled",
      paused: false,
    });
    await emit(page, {
      kind: "job_ended",
      meeting_id: "m1",
      phase: "transcription",
      stopped: true,
    });
    await expect(row.getByText("Abgebrochen", { exact: true })).toBeVisible();
    await expect(row.getByTestId("job-bar")).toHaveCount(0);
  });
});

test.describe("Statusbereich der Detailansicht", () => {
  test("Balken mit Prozent, verarbeiteter Audiodauer, Laufzeit, Restdauer und Phase", async ({
    page,
  }) => {
    await openDetail(page);
    await emit(page, progress());
    await expect(panel(page)).toBeVisible();
    await expect(page.getByTestId("job-phase")).toHaveText("Transkription");
    await expect(page.getByTestId("job-percent")).toHaveText("42 %");
    await expect(page.getByTestId("job-amount")).toHaveText(
      "29:24 von 70:00 Audio",
    );
    await expect(page.getByTestId("job-elapsed")).toHaveText(
      /^Laufzeit 12:3\d$/,
    );
    await expect(page.getByTestId("job-eta")).toHaveText("noch ca. 24 Min.");
    await expect(panel(page).getByRole("progressbar")).toHaveAttribute(
      "aria-valuenow",
      "42",
    );

    // Der Balken wächst mit den Ereignissen.
    await emit(page, progress({ done: 3_150_000, eta_ms: 600_000 }));
    await expect(page.getByTestId("job-percent")).toHaveText("75 %");
    await expect(page.getByTestId("job-eta")).toHaveText("noch ca. 10 Min.");
  });

  test("in der Anlaufzeit steht keine erfundene Restdauer, danach ja", async ({
    page,
  }) => {
    await openDetail(page);
    await emit(
      page,
      progress({ done: 60_000, elapsed_ms: 3_000, eta_ms: null }),
    );
    await expect(page.getByTestId("job-eta")).toHaveText(
      "Restdauer wird berechnet …",
    );
    await emit(
      page,
      progress({ done: 300_000, elapsed_ms: 9_000, eta_ms: 200_000 }),
    );
    await expect(page.getByTestId("job-eta")).toHaveText("noch ca. 4 Min.");
  });

  test("die Sprechertrennung hat keinen Prozentwert und lässt sich nicht pausieren", async ({
    page,
  }) => {
    await openDetail(page);
    await emit(
      page,
      progress({ phase: "speakers", done: 0, eta_ms: null, pausable: false }),
    );
    await expect(page.getByTestId("job-phase")).toHaveText("Sprecher");
    await expect(page.getByTestId("job-percent")).toHaveCount(0);
    await expect(panel(page).getByRole("progressbar")).not.toHaveAttribute(
      "aria-valuenow",
      /.+/,
    );
    const pause = page.getByTestId("job-pause");
    await expect(pause).toBeDisabled();
    // Der Grund steht im Tooltip (Name + Kurzerklaerung), auch am gesperrten Knopf.
    const box = (await pause.boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await expect(page.getByRole("tooltip")).toContainText(/nicht pausieren/);
    // Stoppen geht in jeder Phase.
    await expect(page.getByTestId("job-stop")).toBeEnabled();
  });

  test("Protokoll: der Zustand kommt beim Öffnen aus dem Backend und überlebt einen Reiterwechsel", async ({
    page,
  }) => {
    await page.addInitScript(
      (snap) => {
        const w = window as any;
        w.__progressList = [snap];
        // Dasselbe sagt das Protokoll-Backend über `meetings_minutes_state` (P1k).
        w.__minutesState = {
          running: true,
          progress: { phase: "write", done: 2, total: 6 },
          cancelling: false,
          started_at: 1790001000,
        };
      },
      snapshot({
        phase: "minutes",
        done: 2,
        total: 6,
        eta_ms: 90_000,
        pausable: false,
      }),
    );
    await openDetail(page);
    // Ohne ein einziges Ereignis: der Auftrag steht im Statusbereich.
    await expect(panel(page)).toBeVisible();
    await expect(page.getByTestId("job-phase")).toHaveText("Protokoll");
    await expect(page.getByTestId("job-amount")).toHaveText("Schritt 2 von 6");
    await expect(page.getByTestId("job-eta")).toHaveText("noch ca. 2 Min.");
    // Das Protokoll lässt sich nicht pausieren, nur stoppen.
    await expect(page.getByTestId("job-pause")).toBeDisabled();
    await expect(page.getByTestId("job-stop")).toBeEnabled();

    await page.getByRole("tab", { name: "Protokoll", exact: true }).click();
    const generate = page.getByRole("button", {
      name: "Erzeugen",
      exact: true,
    });
    await expect(generate).toBeDisabled();
    await expect(page.getByTestId("minutes-running")).toBeVisible();

    // Reiter weg und zurück: der Laufzustand ist nicht verloren, der Knopf bleibt gesperrt.
    await page.getByRole("tab", { name: "Notizen", exact: true }).click();
    await expect(panel(page)).toBeVisible();
    await page.getByRole("tab", { name: "Protokoll", exact: true }).click();
    await expect(
      page.getByRole("button", { name: "Erzeugen", exact: true }),
    ).toBeDisabled();
    await expect(page.getByTestId("minutes-running")).toBeVisible();
    await expect(panel(page)).toBeVisible();

    // Ende des Auftrags: das Backend meldet das Ende, der Knopf ist frei, das Panel weg.
    await page.evaluate(() => {
      const w = window as any;
      w.__minutesState = {
        running: false,
        progress: null,
        cancelling: false,
        started_at: null,
      };
      w.__emit("minutes-event", {
        kind: "done",
        meeting_id: "m1",
        document_id: "d1",
      });
    });
    await emit(page, {
      kind: "job_ended",
      meeting_id: "m1",
      phase: "minutes",
      stopped: false,
    });
    await expect(
      page.getByRole("button", { name: "Erzeugen", exact: true }),
    ).toBeEnabled();
    await expect(panel(page)).toHaveCount(0);
  });

  test("Protokoll: Stoppen im Statusbereich fragt zurück und ruft den Stopp-Befehl der Besprechung", async ({
    page,
  }) => {
    await page.addInitScript(
      (snap) => {
        (window as any).__progressList = [snap];
      },
      snapshot({
        phase: "minutes",
        done: 0,
        total: 0,
        eta_ms: null,
        pausable: false,
      }),
    );
    await openDetail(page);
    // Unbekannte Größe: unbestimmter Balken, aber Stoppen geht.
    await expect(page.getByTestId("job-phase")).toHaveText("Protokoll");
    await expect(page.getByTestId("job-percent")).toHaveCount(0);
    await page.getByTestId("job-stop").click();
    await expect(page.getByRole("dialog")).toContainText(
      "Die Erzeugung wird abgebrochen",
    );
    await page.getByTestId("job-stop-confirm").click();
    expect(await calls(page, "meetings_job_stop")).toEqual([
      { cmd: "meetings_job_stop", args: { meetingId: "m1" } },
    ]);
  });

  test("KI-Notizen: ein laufender Auftrag sperrt den Knopf auch nach einem Reiterwechsel", async ({
    page,
  }) => {
    await page.addInitScript(
      (snap) => {
        (window as any).__progressList = [snap];
      },
      snapshot({ phase: "notes", done: 2, total: 5, eta_ms: 60_000 }),
    );
    await setMeetingsLater(page, { status: "ready" });
    await openDetail(page);
    await expect(page.getByTestId("job-phase")).toHaveText("KI-Notizen");
    await page.getByRole("tab", { name: "Notizen", exact: true }).click();
    await page.getByRole("tab", { name: "KI-Notizen", exact: true }).click();
    await expect(
      page.getByRole("button", { name: /KI-Notizen (neu )?erzeugen/ }),
    ).toBeDisabled();
  });
});

/** Setzt die Attrappen-Besprechung vor dem ersten Laden (Init-Skript, nach dem der Test-Setup). */
const setMeetingsLater = async (
  page: Page,
  patch: Record<string, unknown> = {},
) => {
  await page.addInitScript((p) => {
    const w = window as any;
    w.__meetings = w.__meetings.map((m: any) => ({ ...m, ...p }));
  }, patch);
};

test.describe("Pause und Fortsetzen", () => {
  test("der Pausenknopf ruft den Befehl, zeigt Pausiert und wird zu Fortsetzen", async ({
    page,
  }) => {
    await openDetail(page);
    await emit(page, progress());
    const pause = page.getByTestId("job-pause");
    await expect(pause).toBeEnabled();
    // Symbolknopf: der Name steht im aria-label.
    await expect(pause).toHaveAttribute("aria-label", "Pausieren");

    await pause.click();
    expect(await calls(page, "meetings_job_pause")).toEqual([
      { cmd: "meetings_job_pause", args: { meetingId: "m1" } },
    ]);
    // Das Backend bestätigt: erst "Pause wird eingelegt", dann "Pausiert".
    await emit(page, progress({ state: "pausing" }));
    await expect(page.getByTestId("job-state")).toHaveText(
      "Pause wird eingelegt …",
    );
    await expect(page.getByTestId("job-resume")).toHaveAttribute(
      "aria-label",
      "Fortsetzen",
    );
    await emit(page, progress({ state: "paused" }));
    await expect(page.getByTestId("job-state")).toHaveText("Pausiert");
    await expect(page.getByTestId("job-eta")).toHaveCount(0);

    await page.getByTestId("job-resume").click();
    expect(await calls(page, "meetings_job_resume")).toEqual([
      { cmd: "meetings_job_resume", args: { meetingId: "m1" } },
    ]);
    await emit(page, progress({ state: "running" }));
    await expect(page.getByTestId("job-pause")).toHaveAttribute(
      "aria-label",
      "Pausieren",
    );
    await expect(page.getByTestId("job-state")).toHaveCount(0);
  });

  test("ein Fehler des Befehls steht im Panel, der Zustand bleibt", async ({
    page,
  }) => {
    await openDetail(page);
    await emit(page, progress());
    await page.evaluate(() => ((window as any).__jobError = "no_job"));
    await page.getByTestId("job-pause").click();
    await expect(page.getByTestId("job-error")).toContainText(
      "Die Verarbeitung ist inzwischen beendet.",
    );
    await expect(page.getByTestId("job-pause")).toBeEnabled();
  });
});

test.describe("Stoppen", () => {
  test("Rückfrage im Dialog: Abbrechen tut nichts, Stoppen ruft den Befehl genau einmal", async ({
    page,
  }) => {
    await openDetail(page);
    await emit(page, progress());
    await page.getByTestId("job-stop").click();
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog).toContainText("Verarbeitung stoppen?");
    await expect(dialog).toContainText(
      "bereits transkribierten Abschnitte bleiben erhalten",
    );

    // Abbrechen: Dialog zu, kein Befehl.
    await page.getByTestId("job-stop-cancel").click();
    await expect(dialog).toBeHidden();
    expect(await calls(page, "meetings_job_stop")).toHaveLength(0);

    // Bestätigen: der Befehl geht einmal hinaus.
    await page.getByTestId("job-stop").click();
    await page.getByTestId("job-stop-confirm").click();
    await expect(dialog).toBeHidden();
    expect(await calls(page, "meetings_job_stop")).toEqual([
      { cmd: "meetings_job_stop", args: { meetingId: "m1" } },
    ]);

    // Das Backend meldet "Wird gestoppt": beide Knöpfe sind gesperrt.
    await emit(page, progress({ state: "stopping" }));
    await expect(page.getByTestId("job-state")).toHaveText("Wird gestoppt …");
    await expect(page.getByTestId("job-stop")).toBeDisabled();
    await expect(page.getByTestId("job-pause")).toBeDisabled();
  });

  test("nach dem Stopp: Status Abgebrochen, Segmente bleiben, Fortsetzen ruft den Befehl", async ({
    page,
  }) => {
    await setMeetingsLater(page, { mic_audio_path: "C:/Import/import.wav" });
    await openDetail(page);
    await emit(page, progress({ state: "stopping" }));
    // Ende: Zustand `cancelled`, der Auftrag ist weg.
    await setMeetings(page, { status: "cancelled" });
    await emit(page, {
      kind: "state",
      meeting_id: "m1",
      status: "cancelled",
      paused: false,
    });
    await emit(page, {
      kind: "job_ended",
      meeting_id: "m1",
      phase: "transcription",
      stopped: true,
    });

    await expect(panel(page)).toHaveCount(0);
    const cancelled = page.getByTestId("cancelled-panel");
    await expect(cancelled).toBeVisible();
    await expect(cancelled).toContainText("Verarbeitung abgebrochen");
    await expect(cancelled).toContainText(
      "Es sind 40 Abschnitte transkribiert",
    );
    // Die schon transkribierten Segmente stehen noch da.
    await expect(page.locator('[data-segment-index="39"]')).toBeVisible();
    // "Neu transkribieren" bleibt erreichbar (Audio ist da).
    await expect(
      page.getByText("Neu transkribieren", { exact: false }).first(),
    ).toBeVisible();

    await page.getByTestId("job-continue").click();
    expect(await calls(page, "meetings_continue")).toEqual([
      { cmd: "meetings_continue", args: { meetingId: "m1" } },
    ]);
    // Das Backend setzt wieder auf "processing" und meldet Fortschritt.
    await setMeetings(page, { status: "processing" });
    await emit(page, {
      kind: "state",
      meeting_id: "m1",
      status: "processing",
      paused: false,
    });
    await emit(page, progress({ done: 2_000_000, eta_ms: 900_000 }));
    await expect(page.getByTestId("cancelled-panel")).toHaveCount(0);
    await expect(panel(page)).toBeVisible();
  });

  test("Fortsetzen ohne Aufnahme auf der Platte gibt es nicht", async ({
    page,
  }) => {
    await setMeetingsLater(page, { status: "cancelled" });
    await openDetail(page);
    await expect(page.getByTestId("cancelled-panel")).toBeVisible();
    await expect(page.getByTestId("job-continue")).toHaveCount(0);
  });

  test("Fortsetzen: Fehlercode des Backends wird übersetzt angezeigt", async ({
    page,
  }) => {
    await setMeetingsLater(page, {
      status: "cancelled",
      mic_audio_path: "C:/Import/import.wav",
    });
    await openDetail(page);
    await page.evaluate(() => ((window as any).__jobError = "audio_missing"));
    await page.getByTestId("job-continue").click();
    await expect(page.getByTestId("job-continue-error")).toContainText(
      "Die Aufzeichnung liegt nicht mehr auf der Platte",
    );
  });
});

test.describe("Automatisch mitscrollen", () => {
  const metrics = (box: Locator) =>
    box.evaluate((el) => ({
      top: el.scrollTop,
      distance: el.scrollHeight - el.scrollTop - el.clientHeight,
      scrollable: el.scrollHeight > el.clientHeight,
    }));

  const appendSegments = (page: Page, from: number, count: number) =>
    emit(page, {
      kind: "segments",
      meeting_id: "m1",
      appended: Array.from({ length: count }, (_, i) => ({
        segment_index: from + i,
        text: `Neues Segment ${from + i} mit etwas mehr Text, damit die Zeile Platz braucht.`,
        start_ms: (from + i) * 10_000,
        end_ms: (from + i) * 10_000 + 9_000,
        channel: 2,
        speaker_index: null,
      })),
    });

  const atEnd = (box: Locator) => async () => (await metrics(box)).distance < 4;

  test("folgt neuen Segmenten, pausiert beim Hochscrollen und setzt am Ende fort", async ({
    page,
  }) => {
    await openDetail(page);
    await emit(page, progress());
    const box = page.getByTestId("transcript-scroll");
    const toggle = page.getByTestId("autoscroll-toggle");
    await expect(toggle).toBeChecked(); // Standard: an
    expect((await metrics(box)).scrollable).toBe(true);
    await expect.poll(atEnd(box)).toBe(true);

    // Neue Segmente: der Blick bleibt am Ende.
    await appendSegments(page, 40, 10);
    await expect(page.locator('[data-segment-index="49"]')).toBeAttached();
    await expect.poll(atEnd(box)).toBe(true);

    // Der Nutzer blättert hoch: das Mitscrollen pausiert, die Ansicht bleibt stehen.
    await box.evaluate((el) => (el.scrollTop = 0));
    await expect(page.getByTestId("autoscroll-paused")).toBeVisible();
    await appendSegments(page, 50, 5);
    await expect(page.locator('[data-segment-index="54"]')).toBeAttached();
    expect((await metrics(box)).top).toBeLessThan(4);

    // Wieder ans Ende: es folgt wieder.
    await box.evaluate((el) => (el.scrollTop = el.scrollHeight));
    await expect(page.getByTestId("autoscroll-paused")).toBeHidden();
    await appendSegments(page, 55, 5);
    await expect(page.locator('[data-segment-index="59"]')).toBeAttached();
    await expect.poll(atEnd(box)).toBe(true);
  });

  test("der Schalter: aus = kein Mitscrollen, an = springt ans Ende; die Wahl bleibt nach einem Neuladen", async ({
    page,
  }) => {
    await openDetail(page);
    await emit(page, progress());
    const box = page.getByTestId("transcript-scroll");
    const toggle = page.getByTestId("autoscroll-toggle");
    await expect.poll(atEnd(box)).toBe(true);

    await toggle.uncheck();
    const before = (await metrics(box)).top;
    await appendSegments(page, 40, 10);
    await expect(page.locator('[data-segment-index="49"]')).toBeAttached();
    expect((await metrics(box)).top).toBe(before); // steht still
    expect((await metrics(box)).distance).toBeGreaterThan(50);

    // Der Schalter holt das Mitscrollen zurück.
    await toggle.check();
    await expect.poll(atEnd(box)).toBe(true);

    // Gemerkt: aus, Neuladen, noch aus.
    await toggle.uncheck();
    await page.reload();
    await openDetail(page);
    await emit(page, progress());
    await expect(page.getByTestId("autoscroll-toggle")).not.toBeChecked();
  });

  test("ohne laufende Verarbeitung gibt es den Schalter nicht", async ({
    page,
  }) => {
    await setMeetingsLater(page, { status: "ready" });
    await openDetail(page);
    await expect(page.getByTestId("autoscroll-toggle")).toHaveCount(0);
  });
});
