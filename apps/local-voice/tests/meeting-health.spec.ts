import { test, expect, type Page } from "@playwright/test";

// M2-P2e: Warnleiste des Ausfallwaechters auf der Aufnahmeseite. Die Tauri-
// Attrappe meldet eine laufende Aufnahme; `MeetingEvent::Health` loest
// `window.__emit("meeting-event", ...)` aus.

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
      meeting_capture_system: true,
    };
    w.__recording = true;
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
            case "meetings_is_recording":
              return w.__recording;
            case "meetings_stop":
              w.__recording = false;
              return "m1";
            case "meetings_recording_position":
              return { meeting_id: "m1", position_ms: 1000 };
            case "meetings_list":
            case "meetings_get_segments":
            case "meetings_get_documents":
            case "meeting_templates_list":
            case "pages_list":
            case "page_files":
            case "tts_list_voices":
            case "tts_list_voice_infos":
            case "llm_ps":
            case "tts_reading_list":
            case "tts_list_downloads":
            case "llm_local_list":
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

const health = (page: Page, channel: number, state: string) =>
  page.evaluate(
    ([c, s]) =>
      (window as any).__emit("meeting-event", {
        kind: "health",
        meeting_id: "m1",
        channel: c,
        state: s,
      }),
    [channel, state] as const,
  );

const openRecording = async (page: Page) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/");
  await page.getByRole("button", { name: "Aufnahmen", exact: true }).click();
  // Aufnahme laeuft: der Stopp-Knopf ist da, der Hoerer ist angemeldet.
  await expect(page.getByRole("button", { name: "Beenden" })).toBeVisible();
};

const warnings = (page: Page) => page.getByTestId("health-warning");

test("Health Silent zeigt die Leiste, Recovered nimmt sie weg", async ({
  page,
}) => {
  await openRecording(page);
  await expect(warnings(page)).toHaveCount(0);

  await health(page, 0, "silent");
  await expect(warnings(page)).toHaveCount(1);
  await expect(warnings(page).first()).toContainText(
    "Mikrofon liefert seit 30 s kein Signal",
  );

  await health(page, 0, "recovered");
  await expect(warnings(page)).toHaveCount(0);
});

test("ein Wechsel ersetzt den Text des Kanals, Systemton steht neben dem Mikrofon", async ({
  page,
}) => {
  await openRecording(page);
  await health(page, 0, "no_data");
  await expect(warnings(page).first()).toContainText(
    "Mikrofon liefert keine Daten",
  );
  await health(page, 0, "digital_zero");
  await expect(warnings(page)).toHaveCount(1);
  await expect(warnings(page).first()).toContainText("digitale Stille");

  await health(page, 1, "no_data");
  await expect(warnings(page)).toHaveCount(2);
  await health(page, 1, "recovered");
  await expect(warnings(page)).toHaveCount(1);
  await health(page, 0, "recovered");
  await expect(warnings(page)).toHaveCount(0);
});

test("Ueberlauf beider Kanaele erscheint einmal, Recovered des einen laesst den anderen stehen", async ({
  page,
}) => {
  await openRecording(page);
  await health(page, 0, "queue_overflow");
  await health(page, 1, "queue_overflow");
  await expect(warnings(page)).toHaveCount(1);
  await expect(warnings(page).first()).toContainText("kommt nicht hinterher");
  await health(page, 0, "recovered");
  await expect(warnings(page)).toHaveCount(1);
  await health(page, 1, "recovered");
  await expect(warnings(page)).toHaveCount(0);
});

test("fehlende Spracherkennung und Systemton-Ausfall bleiben trotz Recovered stehen", async ({
  page,
}) => {
  await openRecording(page);
  await health(page, 0, "vad_unavailable");
  await health(page, 1, "loopback_died");
  await expect(warnings(page)).toHaveCount(2);
  await health(page, 0, "silent");
  await expect(warnings(page)).toHaveCount(3);
  await health(page, 0, "recovered");
  await expect(warnings(page)).toHaveCount(2);
  await expect(page.getByTestId("health-warnings")).toContainText(
    "Systemton ist ausgefallen",
  );
  await expect(page.getByTestId("health-warnings")).toContainText(
    "Spracherkennung (VAD) ist nicht verfügbar",
  );
});

test("nach dem Beenden verschwinden alle Warnungen", async ({ page }) => {
  await openRecording(page);
  await health(page, 0, "clipping");
  await expect(warnings(page)).toHaveCount(1);
  await page.getByRole("button", { name: "Beenden" }).click();
  await expect(page.getByRole("button", { name: "Beenden" })).toHaveCount(0);
  await expect(warnings(page)).toHaveCount(0);
});
