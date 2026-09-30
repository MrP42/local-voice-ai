import { test, expect, type Page } from "@playwright/test";
import {
  formatCoverage,
  missingValues,
  recipeFits,
  recipePlaceholders,
  recipeValues,
  slashQuery,
  splitAnswer,
  splitTemplate,
  chatErrorCode,
  chatErrorKey,
} from "../src/lib/meetingChat";

// M4-P4e: Chat-Oberflaeche (Seitenleiste im Detail, Live-Zeile, "Alle
// Besprechungen fragen") gegen die Tauri-Attrappe. Jeder Aufruf landet in
// `window.__calls`; `meeting_chat_ask` bleibt offen, bis der Test ihn mit
// `__pending.resolve/reject` beendet, Ereignisse loest `window.__emit` aus.

type Call = { cmd: string; args: Record<string, any> };

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
        {
          id: "local",
          label: "Lokal",
          base_url: "http://127.0.0.1:0/v1",
        },
      ],
      post_process_prompts: [],
      custom_words: [],
      post_process_models: {},
      post_process_api_keys: {},
      push_to_talk: true,
    };
    const meeting = (id: string, title: string, started = 1790000000) => ({
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
    const folder = (id: string, name: string, count = 0) => ({
      id,
      name,
      color: null,
      sort: 1,
      meeting_count: count,
      created_at: 1,
      updated_at: 1,
    });
    const recipe = (
      key: string,
      title: string,
      scope: string,
      live_ok: boolean,
      variables: any[],
      prompt: string,
      builtin = true,
    ) => ({
      id: builtin ? `builtin:${key}` : key,
      title,
      builtin,
      spec: { version: 1, prompt, variables, scope, live_ok },
      updated_at: 0,
    });
    w.__settings = settings;
    w.__calls = [];
    w.__position = null;
    w.__nullChat = false;
    w.__meetings = [
      meeting("m1", "Kundentermin Meyer"),
      meeting("m2", "Teamrunde", 1790100000),
    ];
    w.__folders = [folder("f1", "Vertrieb", 1)];
    w.__assigned = { m1: ["f1"], m2: [] } as Record<string, string[]>;
    w.__segments = Array.from({ length: 15 }, (_, i) => ({
      segment_index: i,
      text: i === 12 ? "Der Preis ist uns zu hoch." : `Satz Nummer ${i}.`,
      start_ms: i === 12 ? 195_000 : i * 15_000,
      end_ms: i === 12 ? 200_000 : i * 15_000 + 5_000,
      channel: 0,
      speaker_index: null,
    }));
    w.__threads = [];
    w.__threadMessages = {};
    w.__pending = null;
    w.__recipes = [
      recipe(
        "was-verpasst",
        "Was habe ich verpasst?",
        "meeting",
        true,
        [],
        "Was wurde besprochen?",
      ),
      recipe(
        "was-fragen",
        "Was sollte ich jetzt fragen?",
        "meeting",
        true,
        [],
        "Welche Fragen?",
      ),
      recipe(
        "follow-up-mail",
        "Follow-up-E-Mail an {{empfaenger}}",
        "meeting",
        false,
        [
          {
            name: "empfaenger",
            label: "Empfänger",
            kind: "text",
            required: true,
            default: null,
          },
        ],
        "Schreibe an {{empfaenger}}.",
      ),
      recipe(
        "entscheidungen-ordner",
        "Entscheidungen im Ordner {{folder}}",
        "global",
        false,
        [
          {
            name: "folder",
            label: "Ordner",
            kind: "folder",
            required: true,
            default: null,
          },
        ],
        "Entscheidungen im Ordner {{folder}}?",
      ),
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
            case "meetings_is_recording":
              return w.__position !== null;
            case "meetings_recording_position":
              return w.__position;
            case "meetings_list":
              return w.__meetings;
            case "meetings_search": {
              const f = args.filter;
              const list = w.__meetings.filter(
                (m: any) =>
                  !f.folder_id ||
                  (w.__assigned[m.id] ?? []).includes(f.folder_id),
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
              return w.__nullChat ? null : w.__folders;
            case "meetings_get_segments":
              return args.meetingId === "m-live" ? [] : w.__segments;
            case "meetings_segment_epoch":
              return 1;
            case "meetings_get_documents":
              return [];
            case "meeting_notes_get":
              return {
                meeting_id: args.meetingId,
                blocks: [
                  {
                    id: "B1",
                    kind: "paragraph",
                    text: "Preis nachverhandeln",
                    at_ms: null,
                    checked: false,
                  },
                ],
                revision: 1,
                updated_at: 0,
              };
            // M4-P4e: Chat
            case "meeting_chat_ask":
              return new Promise((resolve, reject) => {
                w.__pending = { req: args.req, resolve, reject };
              });
            case "meeting_chat_cancel": {
              const pending = w.__pending;
              w.__pending = null;
              if (pending) setTimeout(() => pending.reject("cancelled"), 10);
              return true;
            }
            case "meeting_chat_threads":
              return w.__nullChat ? null : w.__threads;
            case "meeting_chat_thread":
              return w.__nullChat
                ? null
                : (w.__threadMessages[args.threadId] ?? []);
            case "meeting_chat_thread_delete":
              w.__threads = w.__threads.filter(
                (x: any) => x.id !== args.threadId,
              );
              return null;
            case "chat_recipes_list":
              return w.__nullChat ? null : w.__recipes;
            case "chat_recipes_duplicate": {
              const base = w.__recipes.find((r: any) => r.id === args.id);
              const copy = {
                ...base,
                id: `own-${w.__recipes.length}`,
                title: `${base.title} (Kopie)`,
                builtin: false,
              };
              w.__recipes = [...w.__recipes, copy];
              return copy;
            }
            case "chat_recipes_save": {
              const saved = {
                id: args.id ?? `own-${w.__recipes.length}`,
                title: args.title,
                builtin: false,
                spec: args.spec,
                updated_at: 2,
              };
              w.__recipes = args.id
                ? w.__recipes.map((r: any) => (r.id === args.id ? saved : r))
                : [...w.__recipes, saved];
              return saved;
            }
            case "chat_recipes_delete":
              w.__recipes = w.__recipes.filter((r: any) => r.id !== args.id);
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

// ---------------------------------------------------------------------------
// Helfer
// ---------------------------------------------------------------------------

const calls = (page: Page, cmd: string) =>
  page.evaluate(
    (c) => (window as any).__calls.filter((x: Call) => x.cmd === c) as Call[],
    cmd,
  );

const coverage = (over: Record<string, unknown> = {}) => ({
  meetings_in_scope: 1,
  meetings_with_hits: 1,
  meetings_read: 1,
  excerpts_read: 3,
  lexical_only: false,
  rounds: 1,
  truncated: false,
  live: false,
  dropped_citations: 0,
  cpu_limited: false,
  ...over,
});

const citation = (over: Record<string, unknown> = {}) => ({
  n: 1,
  meeting_id: "m1",
  meeting_title: "Kundentermin Meyer",
  started_at: 1790000000,
  source: "transcript",
  epoch: 1,
  segment_index: 12,
  start_ms: 195_000,
  ref_key: null,
  quote: "Der Preis ist uns zu hoch.",
  ...over,
});

const answer = (over: Record<string, unknown> = {}) => ({
  thread_id: "t1",
  message_id: "msg1",
  text: "Der Kunde findet den Preis zu hoch [1].",
  citations: [citation()],
  coverage: coverage(),
  not_found: false,
  uncited: false,
  provider_local: true,
  ...over,
});

/** Wartet auf den offenen `meeting_chat_ask` und liefert dessen Anfrage. */
const pendingRequest = async (page: Page) => {
  await expect
    .poll(() => page.evaluate(() => (window as any).__pending !== null))
    .toBe(true);
  return page.evaluate(() => (window as any).__pending.req);
};

const emitChat = (page: Page, payload: Record<string, unknown>) =>
  page.evaluate(
    (p) => (window as any).__emit("meeting-chat-event", p),
    payload,
  );

const resolveAsk = (page: Page, value: unknown) =>
  page.evaluate((v) => {
    const w = window as any;
    const pending = w.__pending;
    w.__pending = null;
    pending.resolve(v);
  }, value);

const rejectAsk = (page: Page, error: string) =>
  page.evaluate((e) => {
    const w = window as any;
    const pending = w.__pending;
    w.__pending = null;
    pending.reject(e);
  }, error);

const openRecordings = async (page: Page) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  await page.goto("/");
  await page.getByRole("button", { name: "Aufnahmen", exact: true }).click();
  await expect(page.getByText("Kundentermin Meyer")).toBeVisible();
};

const openDetail = async (page: Page) => {
  await openRecordings(page);
  await page.getByText("Kundentermin Meyer", { exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Fragen", exact: true }),
  ).toBeVisible();
};

const panel = (page: Page) => page.getByTestId("chat-panel");
const questionBox = (page: Page) =>
  panel(page).getByRole("textbox", { name: "Frage", exact: true });

/** Audio-Wiedergabe nur mitschreiben: Position beim play() = Sprungziel. */
const withAudio = (page: Page) =>
  page.addInitScript(() => {
    const w = window as any;
    w.__played = [];
    const times = new WeakMap<object, number>();
    Object.defineProperty(HTMLMediaElement.prototype, "currentTime", {
      configurable: true,
      get() {
        return times.get(this) ?? 0;
      },
      set(v: number) {
        times.set(this, v);
      },
    });
    HTMLMediaElement.prototype.play = function () {
      w.__played.push(times.get(this) ?? 0);
      return Promise.resolve();
    };
    HTMLMediaElement.prototype.pause = function () {};
    w.__meetings[0].mic_audio_path = "C:/audio/m1-mic.wav";
    w.__meetings[1].mic_audio_path = "C:/audio/m2-mic.wav";
  });

// ---------------------------------------------------------------------------
// Reine Logik
// ---------------------------------------------------------------------------

test.describe("meetingChat.ts (rein)", () => {
  test("splitAnswer: Marker, Mehrfachzitate, unbekannte Nummern bleiben Text", () => {
    expect(splitAnswer("A [1]. B [2][3], C [1, 2].")).toEqual([
      { kind: "text", text: "A " },
      { kind: "cite", n: 1 },
      { kind: "text", text: ". B " },
      { kind: "cite", n: 2 },
      { kind: "cite", n: 3 },
      { kind: "text", text: ", C " },
      { kind: "cite", n: 1 },
      { kind: "cite", n: 2 },
      { kind: "text", text: "." },
    ]);
    expect(splitAnswer("Liste [7] und [1].", new Set([1]))).toEqual([
      { kind: "text", text: "Liste [7] und " },
      { kind: "cite", n: 1 },
      { kind: "text", text: "." },
    ]);
    expect(splitAnswer("")).toEqual([]);
  });

  test("formatCoverage: eine Besprechung, global, Hinweise", () => {
    const t = (key: string, o?: Record<string, unknown>) =>
      `${key.split(".").pop()}${o ? JSON.stringify(o) : ""}`;
    expect(formatCoverage(coverage() as any, t, { global: false })).toBe(
      'read{"excerpts":"excerpts{\\"count\\":3}"}',
    );
    const global = formatCoverage(
      coverage({
        meetings_in_scope: 132,
        meetings_with_hits: 20,
        meetings_read: 6,
        excerpts_read: 11,
        lexical_only: true,
        cpu_limited: true,
        rounds: 2,
        dropped_citations: 1,
      }) as any,
      t,
      { global: true },
    );
    expect(global).toContain('searched{"count":132}');
    expect(global).toContain('unread{"count":14}');
    expect(global).toContain("secondRound");
    expect(global).toContain("lexicalOnly");
    expect(global).toContain("cpuLimited");
    expect(global).toContain('dropped{"count":1}');
  });

  test("Recipes: Platzhalter, Pflichtwerte, Scope, Slash-Menü, Fehlercodes", () => {
    const spec = {
      version: 1,
      prompt: "Aufgaben von {{person}} seit {{date_from}}, nochmal {{person}}",
      variables: [
        { name: "date_from", label: "Seit", kind: "date_from" as const },
        { name: "person", label: "Person", kind: "person" as const },
        {
          name: "extra",
          label: "Extra",
          kind: "text" as const,
          required: false,
          default: "x",
        },
      ],
      scope: "global" as const,
      live_ok: false,
    };
    expect(recipePlaceholders(spec).map((v) => v.name)).toEqual([
      "person",
      "date_from",
      "extra",
    ]);
    expect(missingValues(spec, { person: "Anna" }).map((v) => v.name)).toEqual([
      "date_from",
    ]);
    expect(
      recipeValues(spec, { person: " Anna ", date_from: "2026-09-01" }),
    ).toEqual({ person: "Anna", date_from: "2026-09-01", extra: "x" });
    expect(recipeFits(spec, { global: true, live: false })).toBe(true);
    expect(recipeFits(spec, { global: false, live: false })).toBe(false);
    expect(
      recipeFits({ ...spec, scope: "meeting" }, { global: false, live: true }),
    ).toBe(false);
    expect(splitTemplate("An {{empfaenger}}!")).toEqual([
      { kind: "text", text: "An " },
      { kind: "var", name: "empfaenger" },
      { kind: "text", text: "!" },
    ]);
    expect(slashQuery("/Foll")).toBe("foll");
    expect(slashQuery("Frage")).toBeNull();
    expect(chatErrorCode("recipe_invalid: needs_meeting")).toBe(
      "recipe_invalid",
    );
    expect(chatErrorKey("no_provider")).toBe(
      "meetings.chat.errors.no_provider",
    );
    expect(chatErrorKey("komisch")).toBe("meetings.chat.errors.unknown");
  });
});

// ---------------------------------------------------------------------------
// Besprechung: Seitenleiste
// ---------------------------------------------------------------------------

test.describe("Chat in der Besprechung", () => {
  test("Strg+J öffnet die Seitenleiste, Tabs bleiben sichtbar, Deltas und Stufen rendern", async ({
    page,
  }) => {
    await openDetail(page);
    await expect(panel(page)).toHaveCount(0);
    await page.keyboard.press("Control+j");
    await expect(panel(page)).toBeVisible();
    // Der Chat steht im Reiter "Fragen" rechts; das Transkript ist ein
    // Reiter daneben und bleibt mit seinem Zustand eingehaengt.
    await expect(
      page.getByRole("tab", { name: "Transkript", exact: true }),
    ).toBeVisible();
    await expect(page.locator('[data-segment-index="12"]')).toBeAttached();

    await questionBox(page).fill("Was hält der Kunde vom Preis?");
    await questionBox(page).press("Enter");
    const req = await pendingRequest(page);
    expect(req.scope).toEqual({ kind: "meeting", meeting_id: "m1" });
    expect(req.thread_id).toBeNull();
    expect(req.question).toBe("Was hält der Kunde vom Preis?");
    expect(req.recipe).toBeNull();
    expect(req.request_id).toMatch(/^chat-/);
    await expect(
      panel(page).getByText("Was hält der Kunde vom Preis?"),
    ).toBeVisible();

    await emitChat(page, {
      kind: "stage",
      request_id: req.request_id,
      stage: "searching",
      round: 1,
    });
    await expect(page.getByTestId("chat-stage")).toHaveText(
      "Suche passende Stellen …",
    );
    // Fremde Anfrage: wird ignoriert.
    await emitChat(page, {
      kind: "delta",
      request_id: "fremd",
      text: "FALSCH",
    });
    await emitChat(page, {
      kind: "stage",
      request_id: req.request_id,
      stage: "answering",
      round: 1,
    });
    await emitChat(page, {
      kind: "delta",
      request_id: req.request_id,
      text: "Der Kunde findet ",
    });
    await emitChat(page, {
      kind: "delta",
      request_id: req.request_id,
      text: "den Preis zu hoch.",
    });
    await expect(page.getByTestId("chat-streaming")).toHaveText(
      "Der Kunde findet den Preis zu hoch.",
    );
    await expect(page.getByTestId("chat-stage")).toHaveText("Antworte …");
    await expect(panel(page).getByText("FALSCH")).toHaveCount(0);

    // Zweite Runde verwirft den bisherigen Text.
    await emitChat(page, {
      kind: "stage",
      request_id: req.request_id,
      stage: "reading",
      round: 2,
    });
    await expect(page.getByTestId("chat-streaming")).toHaveCount(0);
    await expect(page.getByTestId("chat-stage")).toHaveText(
      "Zweiter Anlauf mit weiteren Stellen …",
    );

    await resolveAsk(page, answer());
    await expect(page.getByTestId("chat-stage")).toHaveCount(0);
    const message = panel(page).getByTestId("chat-answer").last();
    await expect(message).toContainText("Der Kunde findet den Preis zu hoch");
    await expect(message.getByTestId("citation-chip")).toHaveCount(1);
    await expect(message.getByTestId("coverage-note")).toHaveText(
      "Gelesen: 3 Stellen.",
    );
    // Folgefrage bleibt im Verlauf.
    await questionBox(page).fill("Und der Termin?");
    await panel(page).getByRole("button", { name: "Senden" }).click();
    expect((await pendingRequest(page)).thread_id).toBe("t1");
  });

  test("Knopf „Fragen“ öffnet und schließt die Seitenleiste", async ({
    page,
  }) => {
    await openDetail(page);
    await page.getByRole("button", { name: "Fragen", exact: true }).click();
    await expect(panel(page)).toBeVisible();
    await panel(page).getByRole("button", { name: "Chat schließen" }).click();
    // Zurueck auf den Reiter Transkript: der Chat bleibt nur verborgen.
    await expect(panel(page)).toBeHidden();
    await expect(
      page.getByRole("tab", { name: "Transkript", exact: true }),
    ).toHaveAttribute("aria-selected", "true");
  });

  test("Zitat-Chip: Tooltip, Sprung ins Transkript, Segment markiert, playAt ab 195 s", async ({
    page,
  }) => {
    await withAudio(page);
    await openDetail(page);
    // Vom Notizen-Tab aus: der Sprung muss den Tab wechseln.
    await page.getByRole("tab", { name: "Notizen", exact: true }).click();
    await page.keyboard.press("Control+j");
    await questionBox(page).fill("Preis?");
    await questionBox(page).press("Enter");
    await pendingRequest(page);
    await resolveAsk(page, answer());

    const chip = panel(page).getByTestId("citation-chip").first();
    await expect(chip).toHaveText("1");
    await expect(chip).toHaveAttribute("aria-label", /Beleg 1/);
    await chip.hover();
    const tip = page.getByRole("tooltip");
    await expect(tip).toContainText("Kundentermin Meyer");
    await expect(tip).toContainText("03:15");
    await expect(tip).toContainText("Der Preis ist uns zu hoch.");

    await chip.click();
    const row = page.locator('[data-segment-index="12"]');
    await expect(row).toBeVisible();
    await expect(row).toHaveAttribute("data-highlighted", "true");
    await expect
      .poll(() => page.evaluate(() => (window as any).__played))
      .toEqual([195]);
    // Das Transkript steht im selben Bereich wie der Chat: der Chat bleibt
    // mit seinen Nachrichten eingehaengt, nur der Reiter wechselt.
    await expect(panel(page)).toBeAttached();
    await expect(
      page.getByRole("tab", { name: "Transkript", exact: true }),
    ).toHaveAttribute("aria-selected", "true");
  });

  test("Notizen-Zitat öffnet den Tab Notizen und markiert den Block", async ({
    page,
  }) => {
    await openDetail(page);
    await page.keyboard.press("Control+j");
    await questionBox(page).fill("Was steht in meinen Notizen?");
    await questionBox(page).press("Enter");
    await pendingRequest(page);
    await resolveAsk(
      page,
      answer({
        text: "Preis nachverhandeln [1].",
        citations: [
          citation({
            source: "user_notes",
            segment_index: null,
            start_ms: null,
            ref_key: "B1",
            quote: "Preis nachverhandeln",
          }),
        ],
      }),
    );
    await panel(page).getByTestId("citation-chip").first().click();
    await expect(
      page.getByRole("tab", { name: "Notizen", exact: true }),
    ).toHaveAttribute("aria-selected", "true");
    await expect(page.locator('[data-block-id="B1"]')).toHaveAttribute(
      "data-highlighted",
      "true",
    );
  });

  test("not_found zeigt den Hinweis, uncited „ohne Beleg“", async ({
    page,
  }) => {
    await openDetail(page);
    await page.keyboard.press("Control+j");
    await questionBox(page).fill("Wer hat gewonnen?");
    await questionBox(page).press("Enter");
    await pendingRequest(page);
    await resolveAsk(
      page,
      answer({ text: "", citations: [], not_found: true }),
    );
    await expect(panel(page).getByTestId("chat-not-found")).toHaveText(
      "Dazu steht in den durchsuchten Besprechungen nichts.",
    );

    await questionBox(page).fill("Und sonst?");
    await questionBox(page).press("Enter");
    await pendingRequest(page);
    await resolveAsk(
      page,
      answer({ text: "Alles gut.", citations: [], uncited: true }),
    );
    await expect(panel(page).getByTestId("chat-uncited")).toHaveText(
      "ohne Beleg",
    );
  });

  test("Abbrechen ruft meeting_chat_cancel mit der request_id", async ({
    page,
  }) => {
    await openDetail(page);
    await page.keyboard.press("Control+j");
    await questionBox(page).fill("Lange Frage");
    await questionBox(page).press("Enter");
    const req = await pendingRequest(page);
    await emitChat(page, {
      kind: "delta",
      request_id: req.request_id,
      text: "Halb",
    });
    await panel(page).getByRole("button", { name: "Abbrechen" }).click();
    const cancel = await calls(page, "meeting_chat_cancel");
    expect(cancel).toHaveLength(1);
    expect(cancel[0].args).toEqual({ requestId: req.request_id });
    await expect(panel(page).getByTestId("chat-notice")).toHaveText(
      "Abgebrochen – nichts gespeichert.",
    );
    await expect(page.getByTestId("chat-streaming")).toHaveCount(0);
    await expect(
      panel(page).getByRole("button", { name: "Senden" }),
    ).toBeVisible();
  });

  test("failed-Code wird übersetzt", async ({ page }) => {
    await openDetail(page);
    await page.keyboard.press("Control+j");
    await questionBox(page).fill("Frage");
    await questionBox(page).press("Enter");
    const req = await pendingRequest(page);
    await emitChat(page, {
      kind: "failed",
      request_id: req.request_id,
      code: "no_provider",
    });
    await rejectAsk(page, "no_provider");
    await expect(panel(page).getByRole("alert")).toHaveText(
      "Kein KI-Anbieter eingerichtet (Einstellungen → Nachbearbeitung).",
    );
  });

  test("Externer Anbieter: gelbe Leiste und einmalige Bestätigung je Anbieter", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      const s = (window as any).__settings;
      s.post_process_provider_id = "openai";
      s.post_process_providers = [
        {
          id: "openai",
          label: "OpenAI",
          base_url: "https://api.openai.com/v1",
        },
      ];
    });
    await openDetail(page);
    await page.keyboard.press("Control+j");
    await expect(page.getByTestId("chat-remote-bar")).toHaveText(
      "Antworten erzeugt OpenAI. Ausschnitte aus deinen Besprechungen werden dorthin übertragen.",
    );
    await questionBox(page).fill("Preis?");
    await questionBox(page).press("Enter");
    const dialog = page.getByRole("dialog");
    await expect(dialog).toContainText("Ausschnitte an OpenAI senden?");
    expect(await calls(page, "meeting_chat_ask")).toHaveLength(0);
    await dialog.getByRole("button", { name: "Senden" }).click();
    await pendingRequest(page);
    await resolveAsk(page, answer({ provider_local: false }));

    await questionBox(page).fill("Noch eine");
    await questionBox(page).press("Enter");
    await pendingRequest(page);
    await expect(page.getByRole("dialog")).toHaveCount(0);
    expect(await calls(page, "meeting_chat_ask")).toHaveLength(2);
  });

  test("Recipe per „/“: Variable als Inline-Chip, Pflichtwert vor dem Senden", async ({
    page,
  }) => {
    await openDetail(page);
    await page.keyboard.press("Control+j");
    await questionBox(page).fill("/follow");
    const menu = page.getByRole("listbox", { name: "Recipe wählen" });
    await expect(menu).toBeVisible();
    // Global-Recipes passen nicht zu einer Besprechung.
    await expect(
      menu.getByRole("option", { name: /Entscheidungen/ }),
    ).toHaveCount(0);
    await questionBox(page).press("Enter");
    const chip = panel(page).getByTestId("recipe-chip");
    await expect(chip).toContainText("Follow-up-E-Mail an");
    const variable = chip.getByRole("textbox", { name: "Empfänger" });
    await expect(variable).toBeVisible();
    await expect(questionBox(page)).toHaveValue("");

    await panel(page).getByRole("button", { name: "Senden" }).click();
    await expect(panel(page).getByRole("alert")).toHaveText(
      "Bitte „Empfänger“ ausfüllen.",
    );
    expect(await calls(page, "meeting_chat_ask")).toHaveLength(0);

    await variable.fill("Frau Meyer");
    await panel(page).getByRole("button", { name: "Senden" }).click();
    const req = await pendingRequest(page);
    expect(req.recipe).toEqual({
      recipe_id: "builtin:follow-up-mail",
      values: { empfaenger: "Frau Meyer" },
    });
    expect(req.question).toBe("");
  });

  test("Knopfleiste: Recipe ohne Variablen fragt sofort", async ({ page }) => {
    await openDetail(page);
    await page.keyboard.press("Control+j");
    await panel(page)
      .getByRole("button", { name: "Was habe ich verpasst?" })
      .click();
    const req = await pendingRequest(page);
    expect(req.recipe).toEqual({
      recipe_id: "builtin:was-verpasst",
      values: {},
    });
  });

  test("Verläufe je Scope: Liste, Laden, Löschen", async ({ page }) => {
    await page.addInitScript(() => {
      const w = window as any;
      w.__threads = [
        {
          id: "t-alt",
          scope_json: "{}",
          meeting_id: "m1",
          title: "Preisfrage",
          message_count: 2,
          created_at: 1,
          updated_at: 2,
        },
      ];
      w.__threadMessages = {
        "t-alt": [
          {
            id: "u1",
            role: "user",
            text: "Was kostet es?",
            citations: [],
            coverage: null,
            not_found: false,
            uncited: false,
            created_at: 1,
          },
          {
            id: "a1",
            role: "assistant",
            text: "Zu viel [1].",
            citations: [
              {
                n: 1,
                meeting_id: "m1",
                meeting_title: "Kundentermin Meyer",
                started_at: 1790000000,
                source: "transcript",
                epoch: 1,
                segment_index: 12,
                start_ms: 195000,
                ref_key: null,
                quote: "Der Preis ist uns zu hoch.",
              },
            ],
            coverage: {
              meetings_in_scope: 1,
              meetings_with_hits: 1,
              meetings_read: 1,
              excerpts_read: 1,
              lexical_only: true,
              rounds: 1,
              truncated: false,
              live: false,
              dropped_citations: 0,
              cpu_limited: false,
            },
            not_found: false,
            uncited: false,
            created_at: 2,
          },
        ],
      };
    });
    await openDetail(page);
    await page.keyboard.press("Control+j");
    const threads = await calls(page, "meeting_chat_threads");
    expect(threads.at(-1)!.args).toEqual({
      scope: { kind: "meeting", meeting_id: "m1" },
    });
    await panel(page)
      .getByRole("button", { name: /Preisfrage/ })
      .click();
    await expect(panel(page).getByText("Was kostet es?")).toBeVisible();
    await expect(panel(page).getByTestId("coverage-note")).toHaveText(
      "Gelesen: 1 Stelle. Nur Stichwortsuche.",
    );
    await questionBox(page).fill("Weiter");
    await questionBox(page).press("Enter");
    expect((await pendingRequest(page)).thread_id).toBe("t-alt");
    await resolveAsk(page, answer({ thread_id: "t-alt" }));

    await panel(page)
      .getByRole("button", { name: "Verlauf löschen" })
      .first()
      .click();
    expect(
      (await calls(page, "meeting_chat_thread_delete")).at(-1)!.args,
    ).toEqual({ threadId: "t-alt" });
    await expect(panel(page).getByText("Was kostet es?")).toHaveCount(0);
  });

  test("Backend ohne Antwort (null): Seitenleiste bleibt bedienbar", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      (window as any).__nullChat = true;
    });
    await openDetail(page);
    await page.keyboard.press("Control+j");
    await expect(questionBox(page)).toBeVisible();
    await questionBox(page).fill("Frage");
    await questionBox(page).press("Enter");
    await pendingRequest(page);
    await resolveAsk(page, null);
    await expect(questionBox(page)).toBeEnabled();
  });
});

// ---------------------------------------------------------------------------
// Recipes verwalten
// ---------------------------------------------------------------------------

test("RecipeManagerDialog: Builtins schreibgeschützt, Duplizieren, Bearbeiten", async ({
  page,
}) => {
  await openDetail(page);
  await page.keyboard.press("Control+j");
  await panel(page).getByRole("button", { name: "Recipes verwalten" }).click();
  const dialog = page.getByRole("dialog");
  const builtinRow = dialog.locator('[data-recipe-id="builtin:was-verpasst"]');
  await expect(builtinRow).toContainText("Mitgeliefert");
  await expect(
    builtinRow.getByRole("button", { name: "Bearbeiten" }),
  ).toHaveCount(0);
  await builtinRow.getByRole("button", { name: "Duplizieren" }).click();
  expect((await calls(page, "chat_recipes_duplicate")).at(-1)!.args).toEqual({
    id: "builtin:was-verpasst",
  });
  const copy = dialog.locator('[data-recipe-id="own-4"]');
  await expect(copy).toContainText("Was habe ich verpasst? (Kopie)");
  await copy.getByRole("button", { name: "Bearbeiten" }).click();
  await dialog.getByRole("textbox", { name: "Titel" }).fill("Kurz verpasst");
  await dialog
    .getByRole("textbox", { name: "Anweisung" })
    .fill("Was sagte {{person}}?");
  await dialog.getByRole("button", { name: "Variable hinzufügen" }).click();
  await dialog.getByRole("textbox", { name: "Name" }).fill("person");
  await dialog.getByRole("textbox", { name: "Beschriftung" }).fill("Person");
  await dialog.getByRole("button", { name: "Speichern" }).click();
  const save = (await calls(page, "chat_recipes_save")).at(-1)!;
  expect(save.args.id).toBe("own-4");
  expect(save.args.title).toBe("Kurz verpasst");
  expect(save.args.spec.prompt).toBe("Was sagte {{person}}?");
  expect(save.args.spec.variables).toEqual([
    {
      name: "person",
      label: "Person",
      kind: "text",
      required: true,
      default: null,
    },
  ]);
  await expect(dialog.locator('[data-recipe-id="own-4"]')).toContainText(
    "Kurz verpasst",
  );
});

// ---------------------------------------------------------------------------
// Liste: alle / Auswahl fragen
// ---------------------------------------------------------------------------

test.describe("Chat über viele Besprechungen", () => {
  test("„Alle Besprechungen fragen“ übernimmt den Ordner als Scope-Chip", async ({
    page,
  }) => {
    await openRecordings(page);
    await page
      .getByRole("group", { name: "Ordner" })
      .getByRole("button", { name: /Vertrieb/ })
      .click();
    await expect(page.getByText("Teamrunde")).toHaveCount(0);
    await page
      .getByRole("button", { name: "Alle Besprechungen fragen" })
      .click();
    await expect(panel(page)).toBeVisible();
    const chips = panel(page).getByRole("group", { name: "Eingrenzung" });
    await expect(chips).toContainText("Ordner: Vertrieb");
    await questionBox(page).fill("Welche Einwände gab es?");
    await questionBox(page).press("Enter");
    const req = await pendingRequest(page);
    expect(req.scope).toEqual({
      kind: "global",
      filter: {
        meeting_ids: null,
        folder_id: "f1",
        person: null,
        person_id: null,
        from: null,
        to: null,
        event_uid: null,
      },
    });
    await resolveAsk(
      page,
      answer({
        coverage: coverage({
          meetings_in_scope: 12,
          meetings_with_hits: 5,
          meetings_read: 3,
          excerpts_read: 7,
        }),
      }),
    );
    await expect(panel(page).getByTestId("coverage-note")).toHaveText(
      "Durchsucht: 12 Besprechungen. Gelesen: 3 Besprechungen, 7 Stellen. 2 weitere mit Treffern nicht gelesen – Frage eingrenzen.",
    );

    // Chip entfernen: Scope wird "alle", neuer Verlauf.
    await chips
      .getByRole("button", { name: "Ordner: Vertrieb entfernen" })
      .click();
    await expect(chips).toContainText("Alle Besprechungen");
    await questionBox(page).fill("Und insgesamt?");
    await questionBox(page).press("Enter");
    const next = await pendingRequest(page);
    expect(next.scope.filter.folder_id).toBeNull();
    expect(next.thread_id).toBeNull();
  });

  test("Global-Recipe mit Ordner-Variable: Ordner als Chip wählen", async ({
    page,
  }) => {
    await openRecordings(page);
    await page
      .getByRole("button", { name: "Alle Besprechungen fragen" })
      .click();
    // Meeting-Recipes passen nicht, das Global-Recipe schon.
    await expect(
      panel(page).getByRole("button", { name: /Was habe ich verpasst/ }),
    ).toHaveCount(0);
    await panel(page)
      .getByRole("button", { name: "Entscheidungen im Ordner Ordner" })
      .click();
    const chip = panel(page).getByTestId("recipe-chip");
    const folderChoice = chip.getByRole("radiogroup", { name: "Ordner" });
    await folderChoice.getByRole("radio", { name: "Vertrieb" }).click();
    await expect(
      folderChoice.getByRole("radio", { name: "Vertrieb" }),
    ).toHaveAttribute("aria-checked", "true");
    await panel(page).getByRole("button", { name: "Senden" }).click();
    const req = await pendingRequest(page);
    expect(req.recipe).toEqual({
      recipe_id: "builtin:entscheidungen-ordner",
      values: { folder: "f1" },
    });
    await expect(panel(page).getByTestId("recipe-chip")).toHaveCount(0);
  });

  test("Auswahlmodus → „Auswahl fragen“; Zitat öffnet erst die Besprechung", async ({
    page,
  }) => {
    await withAudio(page);
    await openRecordings(page);
    await page.getByRole("button", { name: "Auswählen", exact: true }).click();
    await page
      .getByRole("checkbox", { name: "Kundentermin Meyer auswählen" })
      .check();
    await page.getByRole("checkbox", { name: "Teamrunde auswählen" }).check();
    await expect(page.getByText("2 ausgewählt")).toBeVisible();
    await page.getByRole("button", { name: "Auswahl fragen" }).click();
    const chips = panel(page).getByRole("group", { name: "Eingrenzung" });
    await expect(chips).toContainText("2 Besprechungen");
    await questionBox(page).fill("Was ist offen?");
    await questionBox(page).press("Enter");
    const req = await pendingRequest(page);
    expect(req.scope.kind).toBe("global");
    expect(req.scope.filter.meeting_ids).toEqual(["m1", "m2"]);
    await resolveAsk(
      page,
      answer({
        text: "Preis [1].",
        citations: [citation({ meeting_id: "m2", meeting_title: "Teamrunde" })],
        coverage: coverage({ meetings_in_scope: 2 }),
      }),
    );
    await panel(page).getByTestId("citation-chip").first().click();
    // Die Detailansicht der Besprechung m2 ist offen; der Chat bleibt mit
    // seinen Nachrichten eingehaengt und steht im Reiter "Fragen" bereit.
    await expect(
      page.getByRole("heading", { name: "Teamrunde" }),
    ).toBeVisible();
    await expect(page.locator('[data-segment-index="12"]')).toHaveAttribute(
      "data-highlighted",
      "true",
    );
    await expect
      .poll(() => page.evaluate(() => (window as any).__played))
      .toEqual([195]);
    await expect(panel(page)).toBeAttached();
    await page.getByRole("tab", { name: "Fragen", exact: true }).click();
    await expect(panel(page)).toBeVisible();
    await expect(panel(page).getByText("Was ist offen?")).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// Aufnahme: Live-Zeile
// ---------------------------------------------------------------------------

test("Live: eingeklappte Zeile unter dem Notizblock, Recipe „Was habe ich verpasst?“", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as any).__position = { meeting_id: "m-live", position_ms: 60000 };
  });
  await page.setViewportSize({ width: 1400, height: 900 });
  await page.goto("/");
  await page.getByRole("button", { name: "Aufnahmen", exact: true }).click();
  await expect(page.getByTestId("live-notes-pad")).toBeVisible();
  const row = page.getByTestId("live-chat-row");
  await expect(row).toContainText("Frage zur laufenden Besprechung");
  await expect(panel(page)).toHaveCount(0);
  await row.getByRole("button", { name: "Was habe ich verpasst?" }).click();
  await expect(panel(page)).toBeVisible();
  const req = await pendingRequest(page);
  expect(req.scope).toEqual({ kind: "meeting", meeting_id: "m-live" });
  expect(req.recipe).toEqual({ recipe_id: "builtin:was-verpasst", values: {} });
  await resolveAsk(
    page,
    answer({ coverage: coverage({ live: true, excerpts_read: 4 }) }),
  );
  await expect(panel(page).getByTestId("coverage-note")).toHaveText(
    "Live-Mitschrift. Gelesen: 4 Stellen.",
  );
  // Nur live-taugliche Recipes in der Knopfleiste.
  await expect(
    panel(page).getByRole("button", { name: /Follow-up/ }),
  ).toHaveCount(0);
  await row.getByRole("button", { name: "Chat einklappen" }).click();
  await expect(panel(page)).toHaveCount(0);
});

// ---------------------------------------------------------------------------
// Abnahme-Screenshot
// ---------------------------------------------------------------------------

test("Screenshot für die Abnahme", async ({ page }) => {
  test.skip(!process.env.P4E_SCREENSHOT, "nur auf Anforderung");
  await withAudio(page);
  await openDetail(page);
  await page.keyboard.press("Control+j");
  await questionBox(page).fill("Was hält der Kunde vom Preis?");
  await questionBox(page).press("Enter");
  await pendingRequest(page);
  await resolveAsk(
    page,
    answer({
      text: "Der Kunde findet den Preis zu hoch [1] und will bis Freitag ein neues Angebot [2].",
      citations: [
        citation(),
        citation({
          n: 2,
          segment_index: 13,
          start_ms: 195_000 + 30_000,
          quote: "Schicken Sie uns bis Freitag ein neues Angebot.",
        }),
      ],
      coverage: coverage({ excerpts_read: 6, lexical_only: true }),
    }),
  );
  await panel(page).getByTestId("citation-chip").first().click();
  await page.getByTestId("citation-chip").nth(1).hover();
  await page.waitForTimeout(300);
  await page.screenshot({ path: process.env.P4E_SCREENSHOT!, fullPage: false });
});

// ---------------------------------------------------------------------------
// B6: Beleg-Tooltip verdeckt den Antworttext nicht
// ---------------------------------------------------------------------------

test("Beleg-Tooltip überdeckt den Antworttext nicht (B6)", async ({ page }) => {
  await withAudio(page);
  await openDetail(page);
  await page.keyboard.press("Control+j");
  await questionBox(page).fill("Was hält der Kunde vom Preis?");
  await questionBox(page).press("Enter");
  await pendingRequest(page);
  await resolveAsk(
    page,
    answer({
      text: "Der Kunde findet den Preis zu hoch [1] und will bis Freitag ein neues Angebot [2].",
      citations: [
        citation(),
        citation({
          n: 2,
          segment_index: 13,
          start_ms: 225_000,
          quote: "Schicken Sie uns bis Freitag ein neues Angebot.",
        }),
      ],
    }),
  );
  const text = panel(page).locator("[data-citation-scope]").first();
  const chips = panel(page).getByTestId("citation-chip");

  const boxesAfterHover = async (index: number) => {
    await chips.nth(index).hover();
    const tip = page.getByRole("tooltip");
    await expect(tip).toBeVisible();
    // Die Sprechblase blendet ein (150 ms) und misst sich beim ersten Bild.
    await page.waitForTimeout(300);
    const tipBox = (await tip.locator("xpath=..").boundingBox())!;
    const textBox = (await text.boundingBox())!;
    return { tipBox, textBox };
  };
  const overlaps = (
    a: { x: number; y: number; width: number; height: number },
    b: { x: number; y: number; width: number; height: number },
  ) =>
    a.x < b.x + b.width &&
    b.x < a.x + a.width &&
    a.y < b.y + b.height &&
    b.y < a.y + a.height;

  // Breites Fenster: neben dem Chat-Bereich.
  for (const index of [0, 1]) {
    const { tipBox, textBox } = await boxesAfterHover(index);
    expect(overlaps(tipBox, textBox)).toBe(false);
    expect(tipBox.x).toBeGreaterThanOrEqual(0);
    expect(tipBox.x + tipBox.width).toBeLessThanOrEqual(1400);
  }
  if (process.env.LVA_SCREENSHOTS) {
    await page.screenshot({
      path: "../../koordination/granola-besprechungen/abnahme/b6-tooltip.png",
      animations: "disabled",
    });
  }

  // Schmales Fenster: kein Platz daneben, also darunter oder darüber.
  await page.setViewportSize({ width: 620, height: 900 });
  await page.mouse.move(0, 0);
  const narrow = await boxesAfterHover(1);
  expect(overlaps(narrow.tipBox, narrow.textBox)).toBe(false);
});
