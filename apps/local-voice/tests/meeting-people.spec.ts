import { test, expect, type Page } from "@playwright/test";
import {
  initials,
  orderParticipants,
  peopleErrorKey,
} from "../src/lib/meetingPeople";

// M5-P5d/P5e: Personen (Chips, Popover, "Personen verwalten", Personenfilter der
// Liste, Fragen zu einer Person) und der Pre-Meeting-Brief ("Vorbereiten" an
// der Terminkarte und aus dem Hinweisfenster) gegen die Tauri-Attrappe. Die
// Attrappe fuehrt Umbenennen und Zusammenfuehren selbst aus und merkt sich
// jeden Aufruf in `window.__calls`; `window.__emit` loest Ereignisse aus.

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

type Call = { cmd: string; args: Record<string, any> };

const NOW = Date.parse("2026-09-29T09:00:00+02:00");
const START = Date.parse("2026-09-29T14:00:00+02:00");

const calls = (page: Page, cmd: string) =>
  page.evaluate(
    (c) => (window as any).__calls.filter((x: Call) => x.cmd === c) as Call[],
    cmd,
  );

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.clock.setFixedTime(NOW);
  await page.addInitScript(
    ({ start }) => {
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
        post_process_provider_id: "local",
        post_process_providers: [
          { id: "local", label: "Lokal", base_url: "http://127.0.0.1:0/v1" },
        ],
        post_process_prompts: [],
        custom_words: [],
        post_process_models: {},
        post_process_api_keys: {},
        push_to_talk: true,
        meeting_capture_system: true,
        meeting_reminder_lead_s: 60,
        meeting_reminder_all_events: false,
      };
      const meeting = (id: string, title: string, started: number) => ({
        id,
        title,
        status: "ready",
        source: "live",
        started_at: started,
        ended_at: started + 600,
        language: "de",
        mic_audio_path: null,
        system_audio_path: null,
        duration_ms: 600000,
        consent_confirmed_at: started,
        audio_retention_until: null,
        created_at: started,
        source_path: null,
        deleted_at: null,
      });
      w.__calls = [];
      w.__meetings = [
        meeting("m1", "Kundentermin Meyer", 1790000000),
        meeting("m2", "Teamrunde", 1790100000),
      ];
      // Personen: Anna nahm an beiden Besprechungen teil, Bernd an einer.
      w.__people = [
        {
          id: "h1",
          name: "Anna Berg",
          email: "anna@firma.de",
          company: "firma.de",
          is_self: false,
          meeting_count: 2,
        },
        {
          id: "h2",
          name: "Bernd Alt",
          email: "bernd@kunde.de",
          company: "kunde.de",
          is_self: false,
          meeting_count: 1,
        },
        {
          id: "h3",
          name: "Bernd Altmann",
          email: null,
          company: null,
          is_self: false,
          meeting_count: 1,
        },
        {
          id: "h9",
          name: "Patrick Wolff",
          email: "ich@wolff.de",
          company: "wolff.de",
          is_self: true,
          meeting_count: 2,
        },
      ];
      const part = (id: string, role: string, source: string) => {
        const p = w.__people.find((x: any) => x.id === id);
        return {
          human_id: id,
          name: p.name,
          email: p.email,
          company: p.company,
          role,
          source,
          is_self: p.is_self,
          meeting_count: p.meeting_count,
        };
      };
      w.__participants = {
        m1: [
          part("h1", "organizer", "calendar"),
          part("h2", "attendee", "calendar"),
          part("h9", "attendee", "calendar"),
        ],
        m2: [],
      } as Record<string, any[]>;
      w.__upcoming = [
        {
          source_id: "ics-1",
          all_day: false,
          cancelled: false,
          location: null,
          join_url: null,
          description: null,
          attendees: [
            {
              email: "anna@firma.de",
              name: "Anna Berg",
              organizer: true,
              is_self: false,
              partstat: null,
            },
            {
              email: "bernd@kunde.de",
              name: "Bernd Alt",
              organizer: false,
              is_self: false,
              partstat: null,
            },
          ],
          key: "ics-1:serie:1",
          uid: "serie",
          title: "Jour fixe Vertrieb",
          starts_at: start,
          ends_at: start + 3_600_000,
        },
      ];
      w.__brief = {
        event_key: "ics-1:serie:1",
        event_title: "Jour fixe Vertrieb",
        names: ["Anna Berg", "Bernd Alt"],
        shared_meetings: 2,
        filter: {
          meeting_ids: ["m2", "m1"],
          folder_id: null,
          person: null,
          person_id: null,
          from: null,
          to: null,
          event_uid: "serie",
        },
        recipe_id: "builtin:vorbereitung-termin",
        recipe_var: "teilnehmende",
        recipe_value: "Anna Berg, Bernd Alt",
        thread_id: null,
      };
      w.__pendingBrief = null;
      w.__threadMessages = {};
      w.__threads = [];
      w.__askAnswer = null;
      w.__recipes = [
        {
          id: "builtin:vorbereitung-termin",
          title: "Vorbereitung auf den Termin mit {{teilnehmende}}",
          builtin: true,
          spec: {
            version: 1,
            prompt: "Bereite mich auf den Termin mit {{teilnehmende}} vor.",
            variables: [
              {
                name: "teilnehmende",
                label: "Teilnehmende",
                kind: "text",
                required: true,
                default: null,
              },
            ],
            scope: "global",
            live_ok: false,
          },
          updated_at: 0,
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
                  listeners[entry.event] = (
                    listeners[entry.event] ?? []
                  ).filter((h) => h !== entry.handler);
                  handlerOf.delete(args.eventId as number);
                }
                return null;
              }
              case "meetings_is_recording":
                return false;
              case "meetings_recording_position":
                return null;
              case "meetings_list":
                return w.__meetings;
              case "meetings_search": {
                const f = args.filter;
                const list = w.__meetings.filter(
                  (m: any) =>
                    !f.person_id ||
                    (w.__participants[m.id] ?? []).some(
                      (p: any) => p.human_id === f.person_id,
                    ),
                );
                return {
                  items: list.map((m: any) => ({
                    meeting: m,
                    snippet: null,
                    hit_source: null,
                  })),
                  total: list.length,
                  truncated: false,
                };
              }
              case "meeting_folders_list":
                return [];
              case "meetings_get_segments":
                return [
                  {
                    segment_index: 0,
                    text: "Guten Morgen.",
                    start_ms: 0,
                    end_ms: 3000,
                    channel: 0,
                    speaker_index: null,
                  },
                ];
              case "meetings_segment_epoch":
                return 1;
              case "meetings_get_documents":
              case "action_items_list":
              case "meeting_speakers_list":
              case "meeting_speaker_notices":
                return [];
              case "meeting_notes_get":
                return {
                  meeting_id: args.meetingId,
                  blocks: [],
                  revision: 0,
                  updated_at: 0,
                };
              // Personen
              case "meeting_participants":
                return w.__participants[args.meetingId] ?? [];
              case "people_list": {
                const q = (args.query ?? "").toLowerCase();
                return w.__people.filter(
                  (p: any) => !q || p.name.toLowerCase().includes(q),
                );
              }
              case "people_get": {
                const p = w.__people.find((x: any) => x.id === args.id);
                if (!p) throw "person_not_found";
                return {
                  ...p,
                  other_emails: [],
                  recent_meetings: [],
                };
              }
              case "people_update": {
                const p = w.__people.find((x: any) => x.id === args.id);
                if (!p) throw "person_not_found";
                if (!String(args.name).trim()) throw "person_name_invalid";
                if (
                  args.email &&
                  w.__people.some(
                    (x: any) => x.id !== args.id && x.email === args.email,
                  )
                )
                  throw "person_email_taken";
                p.name = String(args.name).trim();
                if (args.email !== null) p.email = args.email || null;
                for (const list of Object.values(w.__participants) as any[]) {
                  for (const row of list)
                    if (row.human_id === args.id) {
                      row.name = p.name;
                      row.email = p.email;
                    }
                }
                return null;
              }
              case "people_merge": {
                const gone = w.__people.find((x: any) => x.id === args.gone);
                const keep = w.__people.find((x: any) => x.id === args.keep);
                if (!gone || !keep) throw "person_not_found";
                keep.meeting_count = Math.max(
                  keep.meeting_count,
                  gone.meeting_count,
                );
                w.__people = w.__people.filter((x: any) => x.id !== args.gone);
                for (const key of Object.keys(w.__participants)) {
                  const rows = w.__participants[key];
                  const seen = new Set<string>();
                  w.__participants[key] = rows
                    .map((row: any) =>
                      row.human_id === args.gone
                        ? {
                            ...row,
                            human_id: keep.id,
                            name: keep.name,
                            email: keep.email,
                          }
                        : row,
                    )
                    .filter((row: any) => {
                      if (seen.has(row.human_id)) return false;
                      seen.add(row.human_id);
                      return true;
                    });
                }
                return null;
              }
              // Brief
              case "people_brief_info":
                if (args.eventKey !== w.__brief.event_key)
                  throw "calendar_event_not_found";
                return w.__brief;
              case "people_brief_open":
                w.__pendingBrief = args.eventKey;
                return null;
              case "people_brief_pending": {
                const key = w.__pendingBrief;
                w.__pendingBrief = null;
                return key;
              }
              // Chat
              case "meeting_chat_ask":
                return (
                  w.__askAnswer ?? {
                    thread_id: "t-brief",
                    message_id: "a1",
                    text: "Der Brief.",
                    citations: [],
                    coverage: {
                      meetings_in_scope: 2,
                      meetings_with_hits: 2,
                      meetings_read: 2,
                      excerpts_read: 4,
                      lexical_only: true,
                      rounds: 1,
                      truncated: false,
                      live: false,
                      dropped_citations: 0,
                      cpu_limited: false,
                    },
                    not_found: false,
                    uncited: false,
                  }
                );
              case "meeting_chat_cancel":
                return true;
              case "meeting_chat_threads":
                return w.__threads;
              case "meeting_chat_thread":
                return w.__threadMessages[args.threadId] ?? [];
              case "chat_recipes_list":
                return w.__recipes;
              // Kalender
              case "calendar_sources_list":
                return [
                  {
                    id: "ics-1",
                    kind: "ics",
                    label: "Outlook",
                    account_hint: "outlook.office365.com",
                    enabled: true,
                    has_attendee_data: true,
                    last_sync_at: 1,
                    last_ok_at: 1,
                    last_error: null,
                    event_count: 1,
                  },
                ];
              case "calendar_upcoming":
                return w.__upcoming;
              case "calendar_suggest_event":
                return null;
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
    { start: START },
  );
});

const openRecordings = async (page: Page) => {
  await page.setViewportSize({ width: 1280, height: 1000 });
  await page.goto("/");
  await page.getByRole("button", { name: "Aufnahmen", exact: true }).click();
};

const openDetail = async (page: Page) => {
  await openRecordings(page);
  await page.getByText("Kundentermin Meyer", { exact: true }).click();
  await expect(page.getByTestId("participant-chips")).toBeVisible();
};

const chip = (page: Page, name: string) =>
  page.getByTestId("participant-chip").filter({ hasText: name });

// ---------------------------------------------------------------------------
// Reine Logik
// ---------------------------------------------------------------------------

test.describe("Personen Logik", () => {
  test("Fehlercodes werden übersetzt, Unbekanntes wird allgemein", () => {
    expect(peopleErrorKey("person_email_taken")).toBe(
      "meetings.people.errors.emailTaken",
    );
    expect(peopleErrorKey("person_not_found: x")).toBe(
      "meetings.people.errors.notFound",
    );
    expect(peopleErrorKey("kaputt")).toBe("meetings.people.errors.failed");
  });

  test("Initialen und Reihenfolge (ich zuletzt)", () => {
    expect(initials("Anna Berg")).toBe("AB");
    expect(initials("  clara  ")).toBe("C");
    expect(initials("")).toBe("");
    const list = [
      { n: "ich", is_self: true },
      { n: "a", is_self: false },
      { n: "b", is_self: false },
    ];
    expect(orderParticipants(list).map((p) => p.n)).toEqual(["a", "b", "ich"]);
  });
});

// ---------------------------------------------------------------------------
// Personen (P5d)
// ---------------------------------------------------------------------------

test.describe("Personen", () => {
  test("Teilnehmenden-Chips: Organisator vorn, „ich“ am Ende und gekennzeichnet", async ({
    page,
  }) => {
    await openDetail(page);
    const chips = page.getByTestId("participant-chip");
    await expect(chips).toHaveCount(3);
    await expect(chips.nth(0)).toContainText("Anna Berg");
    await expect(chips.nth(0)).toHaveAttribute("data-role", "organizer");
    await expect(chips.nth(1)).toContainText("Bernd Alt");
    await expect(chips.nth(2)).toContainText("Patrick Wolff");
    await expect(chips.nth(2)).toContainText("(ich)");
    expect((await calls(page, "meeting_participants"))[0].args).toEqual({
      meetingId: "m1",
    });
  });

  test("Besprechung ohne Teilnehmende zeigt keine Chipzeile", async ({
    page,
  }) => {
    await openRecordings(page);
    await page.getByText("Teamrunde", { exact: true }).click();
    await expect(page.locator('[data-segment-index="0"]')).toBeVisible();
    await expect(page.getByTestId("participant-chips")).toHaveCount(0);
  });

  test("Klick auf einen Chip: Popover mit E-Mail, Firma und Besprechungen", async ({
    page,
  }) => {
    await openDetail(page);
    await chip(page, "Anna Berg").click();
    const pop = page.getByTestId("person-popover");
    await expect(pop.getByTestId("person-name")).toHaveText("Anna Berg");
    await expect(pop).toContainText("Organisator");
    await expect(pop).toContainText("Aus dem Kalender");
    await expect(pop.getByTestId("person-email")).toHaveText("anna@firma.de");
    await expect(pop.getByTestId("person-company")).toHaveText("firma.de");
    await expect(pop.getByTestId("person-meetings")).toHaveText(
      "2 Besprechungen mit Anna Berg",
    );
    // Das Popover steht sichtbar unter dem Chip, nicht ausserhalb des Fensters.
    await expect
      .poll(async () => (await pop.boundingBox())?.y ?? -1)
      .toBeGreaterThan(0);
    const box = (await pop.boundingBox())!;
    const chipBox = (await chip(page, "Anna Berg").boundingBox())!;
    expect(box.y).toBeGreaterThanOrEqual(chipBox.y + chipBox.height);
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.y + box.height).toBeLessThanOrEqual(1000);
    await page.keyboard.press("Escape");
    await expect(pop).toBeHidden();
    // Eine Person ohne Adresse sagt das.
    await chip(page, "Bernd Alt").click();
    await expect(page.getByTestId("person-meetings")).toHaveText(
      "1 Besprechung mit Bernd Alt",
    );
  });

  test("„Besprechungen mit …“ grenzt die Liste ein (Personen-Chip, person_id)", async ({
    page,
  }) => {
    await openDetail(page);
    await chip(page, "Bernd Alt").click();
    await page.getByTestId("person-meetings").click();
    // Die Liste links zeigt den Chip "Person: Bernd Alt"; die Detailansicht
    // bleibt daneben offen.
    const list = page.getByTestId("rec-sessions");
    await expect(page.getByTestId("person-filter-chip")).toContainText(
      "Person: Bernd Alt",
    );
    await expect(
      list.getByText("Kundentermin Meyer", { exact: true }),
    ).toBeVisible();
    await expect(list.getByText("Teamrunde", { exact: true })).toHaveCount(0);
    const search = await calls(page, "meetings_search");
    expect(search[search.length - 1].args.filter).toMatchObject({
      person_id: "h2",
    });
    // Chip entfernen: alle Besprechungen wieder.
    await page.getByTestId("person-filter-remove").click();
    await expect(page.getByTestId("person-filter-chip")).toHaveCount(0);
    await expect(list.getByText("Teamrunde", { exact: true })).toBeVisible();
  });

  test("„Fragen“ öffnet den Chat über diese Person (person_id im Scope)", async ({
    page,
  }) => {
    await openDetail(page);
    await chip(page, "Anna Berg").click();
    await page.getByTestId("person-ask").click();
    const panel = page.getByTestId("chat-panel");
    await expect(panel).toBeVisible();
    await expect(panel).toContainText("Person: Anna Berg");
    const threads = await calls(page, "meeting_chat_threads");
    expect(threads[threads.length - 1].args.scope).toMatchObject({
      kind: "global",
      filter: { person_id: "h1" },
    });
  });

  test("Personen verwalten: Umbenennen aktualisiert die Chips", async ({
    page,
  }) => {
    await openDetail(page);
    await chip(page, "Bernd Alt").click();
    await page.getByTestId("person-manage").click();
    const dialog = page.getByTestId("people-dialog");
    await expect(dialog.getByTestId("people-row")).toHaveCount(4);
    await dialog.getByTestId("people-search").fill("bernd");
    await expect(dialog.getByTestId("people-row")).toHaveCount(2);
    await dialog.getByTestId("people-row").first().click();
    await dialog.getByTestId("person-name-input").fill("Bernd Alt-Meyer");
    await dialog.getByTestId("person-save").click();
    await expect(dialog.getByTestId("people-status")).toHaveText(
      "Gespeichert.",
    );
    expect((await calls(page, "people_update"))[0].args).toEqual({
      id: "h2",
      name: "Bernd Alt-Meyer",
      email: "bernd@kunde.de",
    });
    // Schliessen: die Chip-Zeile der Besprechung zeigt den neuen Namen.
    await page.getByRole("button", { name: "Schließen" }).last().click();
    await expect(chip(page, "Bernd Alt-Meyer")).toBeVisible();
  });

  test("Personen verwalten: Fehler (Adresse vergeben) erscheinen im Dialog", async ({
    page,
  }) => {
    await openDetail(page);
    await chip(page, "Bernd Alt").click();
    await page.getByTestId("person-manage").click();
    const dialog = page.getByTestId("people-dialog");
    await dialog.getByTestId("people-row").nth(1).click();
    await dialog.getByTestId("person-email-input").fill("anna@firma.de");
    await dialog.getByTestId("person-save").click();
    await expect(dialog.getByRole("alert")).toContainText(
      "Diese Adresse gehört einer anderen Person",
    );
  });

  test("Zusammenführen: Rückfrage, dann people_merge(keep, gone); Abbrechen ändert nichts", async ({
    page,
  }) => {
    await openDetail(page);
    await chip(page, "Anna Berg").click();
    await page.getByTestId("person-manage").click();
    const dialog = page.getByTestId("people-dialog");
    // "Bernd Altmann" (ohne Adresse) geht in "Bernd Alt" auf.
    await dialog.getByTestId("people-search").fill("altmann");
    await expect(dialog.getByTestId("people-row")).toHaveCount(1);
    await dialog.getByTestId("people-row").first().click();
    await expect(dialog.getByTestId("merge-start")).toBeDisabled();
    await dialog.getByTestId("merge-target").click();
    await page.getByText("Bernd Alt (bernd@kunde.de)", { exact: true }).click();
    await dialog.getByTestId("merge-start").click();
    await expect(dialog.getByTestId("merge-confirm")).toContainText(
      "„Bernd Altmann“ in „Bernd Alt“ zusammenführen?",
    );
    await dialog.getByRole("button", { name: "Abbrechen" }).click();
    expect(await calls(page, "people_merge")).toHaveLength(0);
    await dialog.getByTestId("merge-start").click();
    await dialog.getByTestId("merge-do").click();
    await expect(dialog.getByTestId("people-status")).toHaveText(
      "Zusammengeführt.",
    );
    expect((await calls(page, "people_merge"))[0].args).toEqual({
      keep: "h2",
      gone: "h3",
    });
    // Danach zeigt der Dialog die verbleibende Person.
    await expect(dialog.getByTestId("person-name-input")).toHaveValue(
      "Bernd Alt",
    );
  });
});

// ---------------------------------------------------------------------------
// Pre-Meeting-Brief (P5e)
// ---------------------------------------------------------------------------

test.describe("Vorbereiten", () => {
  test("Vorbereiten an der Terminkarte: meeting_chat_ask mit Recipe-ID und meeting_ids", async ({
    page,
  }) => {
    await openRecordings(page);
    const button = page.getByTestId("upcoming-brief");
    await expect(button).toBeEnabled();
    await expect(button).toHaveAttribute("data-shared", "2");
    await button.click();
    await expect
      .poll(async () => (await calls(page, "meeting_chat_ask")).length)
      .toBe(1);
    const req = (await calls(page, "meeting_chat_ask"))[0].args.req;
    expect(req.recipe).toEqual({
      recipe_id: "builtin:vorbereitung-termin",
      values: { teilnehmende: "Anna Berg, Bernd Alt" },
    });
    expect(req.scope.kind).toBe("global");
    expect(req.scope.filter.meeting_ids).toEqual(["m2", "m1"]);
    expect(req.scope.filter.event_uid).toBe("serie");
    expect(req.thread_id).toBeNull();
    // Die Frage steht mit den Namen im Verlauf, die Antwort darunter.
    const panel = page.getByTestId("chat-panel");
    await expect(panel).toContainText(
      "Vorbereitung auf den Termin mit Anna Berg, Bernd Alt",
    );
    await expect(panel).toContainText("Der Brief.");
    expect((await calls(page, "people_brief_info"))[0].args).toEqual({
      eventKey: "ics-1:serie:1",
    });
  });

  test("ohne gemeinsame Besprechungen ist der Knopf deaktiviert und sagt warum", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      const w = window as any;
      // Nach dem Laden der Attrappe (addInitScript-Reihenfolge) ueberschreiben.
      w.__brief = { ...w.__brief, shared_meetings: 0 };
      w.__brief.filter = { ...w.__brief.filter, meeting_ids: [] };
    });
    await openRecordings(page);
    const button = page.getByTestId("upcoming-brief");
    await expect(button).toBeDisabled();
    await expect(button).toHaveAttribute("data-shared", "0");
    await expect(button.locator("xpath=..")).toHaveAttribute(
      "title",
      "Keine früheren Besprechungen mit diesen Teilnehmenden.",
    );
    expect(await calls(page, "meeting_chat_ask")).toHaveLength(0);
  });

  test("zweiter Klick öffnet den gespeicherten Brief statt neu zu fragen", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      const w = window as any;
      w.__brief = { ...w.__brief, thread_id: "t-alt" };
      w.__threadMessages = {
        "t-alt": [
          {
            id: "u0",
            role: "user",
            text: "Vorbereitung auf den Termin mit Anna Berg, Bernd Alt",
            citations: [],
            coverage: null,
            not_found: false,
            uncited: false,
            created_at: 1,
          },
          {
            id: "a0",
            role: "assistant",
            text: "Gespeicherter Brief von gestern.",
            citations: [],
            coverage: null,
            not_found: false,
            uncited: false,
            created_at: 2,
          },
        ],
      };
    });
    await openRecordings(page);
    const button = page.getByTestId("upcoming-brief");
    await expect(button).toHaveAttribute("data-saved", "true");
    await expect(button.locator("xpath=..")).toHaveAttribute(
      "title",
      "Gespeicherten Brief öffnen",
    );
    await button.click();
    await expect(page.getByTestId("chat-panel")).toContainText(
      "Gespeicherter Brief von gestern.",
    );
    expect(
      (await calls(page, "meeting_chat_thread")).map((c) => c.args.threadId),
    ).toContain("t-alt");
    expect(await calls(page, "meeting_chat_ask")).toHaveLength(0);
  });

  test("„Vorbereiten“ im Hinweisfenster: das Hauptfenster holt den Termin und öffnet den Brief", async ({
    page,
  }) => {
    await openRecordings(page);
    // Das Backend hat den Wunsch gemerkt und meldet ihn dem Hauptfenster.
    await page.evaluate(() => {
      const w = window as any;
      w.__pendingBrief = "ics-1:serie:1";
      w.__emit("brief-request-event", { event_key: "ics-1:serie:1" });
    });
    await expect
      .poll(async () => (await calls(page, "meeting_chat_ask")).length)
      .toBe(1);
    const req = (await calls(page, "meeting_chat_ask"))[0].args.req;
    expect(req.recipe.recipe_id).toBe("builtin:vorbereitung-termin");
    expect(req.scope.filter.meeting_ids).toEqual(["m2", "m1"]);
    // Der Wunsch ist abgeholt: ein zweites Ereignis fragt nicht noch einmal.
    await page.evaluate(() =>
      (window as any).__emit("brief-request-event", {
        event_key: "ics-1:serie:1",
      }),
    );
    await page.waitForTimeout(200);
    expect(await calls(page, "meeting_chat_ask")).toHaveLength(1);
  });

  test("ein beim Start schon gemerkter Wunsch öffnet den Brief nach dem Laden der Seite", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      (window as any).__pendingBrief = "ics-1:serie:1";
    });
    await openRecordings(page);
    await expect
      .poll(async () => (await calls(page, "meeting_chat_ask")).length)
      .toBe(1);
  });
});
