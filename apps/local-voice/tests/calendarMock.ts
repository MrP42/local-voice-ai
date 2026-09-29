import type { Page } from "@playwright/test";

/**
 * Tauri-Attrappe fuer die Kalender-Specs (M5-P5b): `meeting-calendar.spec.ts`
 * (Hauptfenster) und `meeting-prompt.spec.ts` (Hinweisfenster). Jeder Aufruf
 * landet in `window.__calls`; `window.__emit(event, payload)` loest Ereignisse aus.
 * Der Zustand steht in `window.__*` und laesst sich pro Test ueberschreiben
 * (`page.addInitScript` NACH `installTauriMock`).
 */
export type Call = { cmd: string; args: Record<string, unknown> };

export const installTauriMock = async (
  page: Page,
  windowLabel: "main" | "meeting_prompt" = "main",
) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript((label) => {
    const w = window as any;
    const callbacks = new Map<number, (e: unknown) => void>();
    const listeners: Record<string, number[]> = {};
    const handlerOf = new Map<number, { event: string; handler: number }>();
    let callback = 0;
    const settings: Record<string, unknown> = {
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
      meeting_reminder_lead_s: 60,
      meeting_reminder_all_events: false,
      // M5-P5f: Microsoft-Anmeldung (Client-ID des Nutzers, keine eingebaute)
      calendar_graph_client_id: null,
      calendar_graph_tenant: null,
    };
    w.__settings = settings;
    w.__calls = [];
    w.__sources = [];
    w.__upcoming = [];
    w.__suggest = null;
    w.__prompt = null;
    w.__addError = null;
    w.__startError = null;
    // M5-P5e: Brief-Zuschnitt, den `people_brief_info` liefert (null = Fehler).
    w.__brief = null;
    w.__addDelay = 0;
    // M5-P5f: Microsoft-Anmeldung. `__graphWait` haelt die Anmeldung offen, bis
    // "Abbrechen" `calendar_graph_cancel_sign_in` ruft; `__graphError` laesst sie scheitern.
    w.__graphWait = false;
    w.__graphError = null;
    w.__graphCancel = null;
    w.__meeting = {
      id: "m-neu",
      title: "Neu",
      status: "recording",
      source: "recording",
      started_at: 1790000000,
      ended_at: null,
      language: "de",
      mic_audio_path: null,
      system_audio_path: null,
      duration_ms: null,
      consent_confirmed_at: 1790000000,
      audio_retention_until: null,
      created_at: 1790000000,
      source_path: null,
    };
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
          currentWindow: { label },
          currentWebview: { label },
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
            case "meetings_is_recording":
              return false;
            case "meetings_recording_position":
              return null;
            case "meetings_list":
              return [];
            case "meetings_start":
              return { ...w.__meeting, title: args.title };
            case "meetings_start_from_event":
              if (w.__startError) throw w.__startError;
              return { ...w.__meeting, title: args.title ?? "aus Termin" };
            case "meetings_stop":
              return "m-neu";
            case "meetings_set_template":
              return null;
            case "meeting_templates_list":
              return [
                {
                  id: "builtin:allgemein",
                  title: "Allgemein",
                  builtin: true,
                  spec: { version: 1, context: "", sections: [] },
                  updated_at: 1,
                },
              ];
            // Kalender
            case "calendar_sources_list":
              return w.__sources;
            case "calendar_source_add_ics": {
              if (w.__addDelay) {
                await new Promise((r) => setTimeout(r, w.__addDelay));
              }
              if (w.__addError) throw w.__addError;
              const source = {
                id: `ics-${w.__sources.length + 1}`,
                kind: "ics",
                label: args.label || "Kalender",
                account_hint: "outlook.office365.com",
                enabled: true,
                has_attendee_data: true,
                last_sync_at: 1790000000000,
                last_ok_at: 1790000000000,
                last_error: null,
                event_count: 23,
              };
              w.__sources = [...w.__sources, source];
              return source;
            }
            case "calendar_source_remove":
              w.__sources = w.__sources.filter((s: any) => s.id !== args.id);
              return null;
            case "change_calendar_graph_client_id_setting":
              settings.calendar_graph_client_id = args.clientId;
              return null;
            case "change_calendar_graph_tenant_setting":
              settings.calendar_graph_tenant = args.tenant;
              return null;
            case "calendar_graph_sign_in": {
              if (w.__graphWait) {
                await new Promise((_resolve, reject) => {
                  w.__graphCancel = () =>
                    reject("Die Anmeldung wurde abgebrochen.");
                });
              }
              if (w.__graphError) throw w.__graphError;
              const source = {
                id: "graph-1",
                kind: "graph",
                label: "Outlook / Microsoft 365",
                account_hint: "anna.berg@firma.de",
                enabled: true,
                has_attendee_data: true,
                last_sync_at: 1790000000000,
                last_ok_at: 1790000000000,
                last_error: null,
                event_count: 12,
              };
              w.__sources = [...w.__sources, source];
              return source;
            }
            case "calendar_graph_cancel_sign_in":
              if (w.__graphCancel) w.__graphCancel();
              return true;
            case "calendar_graph_sign_out":
              w.__sources = w.__sources.filter((s: any) => s.id !== args.id);
              return null;
            case "calendar_sync_now":
              w.__sources = w.__sources.map((s: any) => ({
                ...s,
                last_error: null,
                last_ok_at: 1790003600000,
              }));
              return w.__sources;
            case "calendar_upcoming":
              return w.__upcoming;
            case "calendar_suggest_event":
              return w.__suggest;
            case "calendar_open_join_url":
              return null;
            case "change_meeting_reminder_lead_setting":
              settings.meeting_reminder_lead_s = args.seconds;
              return null;
            case "change_meeting_reminder_all_events_setting":
              settings.meeting_reminder_all_events = args.enabled;
              return null;
            case "change_meeting_capture_system_setting":
              settings.meeting_capture_system = args.enabled;
              return null;
            // Brief (P5e)
            case "people_brief_info":
              if (!w.__brief) throw "calendar_event_not_found";
              return w.__brief;
            case "people_brief_open":
              return null;
            // Hinweisfenster
            case "meeting_prompt_current":
              return w.__prompt;
            case "meeting_prompt_ready":
            case "meeting_prompt_dismiss":
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
  }, windowLabel);
};

export const calls = (page: Page, cmd: string) =>
  page.evaluate(
    (c) => (window as any).__calls.filter((x: Call) => x.cmd === c) as Call[],
    cmd,
  );

/** Ein Termin wie ihn `calendar_upcoming` liefert; Zeiten in ms UTC. */
export const calEvent = (over: Record<string, unknown> = {}) => ({
  key: "ics-1:uid-1:1790589600000",
  source_id: "ics-1",
  uid: "uid-1",
  title: "Jour fixe Vertrieb",
  starts_at: 1790589600000,
  ends_at: 1790593200000,
  all_day: false,
  cancelled: false,
  location: null,
  join_url: null,
  description: null,
  attendees: [
    {
      email: "anna@example.org",
      name: "Anna Berg",
      organizer: true,
      is_self: false,
      partstat: null,
    },
    {
      email: "bernd@example.org",
      name: "Bernd Alt",
      organizer: false,
      is_self: false,
      partstat: null,
    },
    {
      email: "clara@example.org",
      name: null,
      organizer: false,
      is_self: false,
      partstat: null,
    },
  ],
  ...over,
});
