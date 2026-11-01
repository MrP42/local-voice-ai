import { test, expect, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";

// Seite „Sprachgespraech“ gegen die Tauri-Attrappe: Infozeile, Text-Chat mit
// Token/Abo-Hinweis, Fussleisten-Menue „Gespraech“, Plus-Menue, Einstellungen.
// `voice_call_chat` bleibt offen, bis der Test ihn mit `__chat.resolve` beendet;
// Ereignisse (`voice-call`) loest `window.__emit` aus.

type Call = { cmd: string; args: Record<string, any> };

// Abnahmebilder nur auf Wunsch (wie die anderen Bildstrecken): ein fester
// Ablagepfad brach den Lauf auf Mac und Windows ab.
const SHOTS = process.env.SCREENS_DIR ?? "";

/** Ergänzungen der Attrappe: Einstellungen, gespeicherte Gesprächskonfiguration, Werkzeuge. */
type Extra = {
  settings?: Record<string, unknown>;
  config?: Record<string, unknown>;
  tools?: Record<string, unknown>[];
  /** Gespeichertes Gespräch, das schon im Verlauf steht (Stufe 3: Zeilen bearbeiten). */
  seed?: {
    title: string;
    entries: { role: string; mode: string; text: string }[];
  };
  /** Gelernte Paare der Attrappe (Stufe 3). */
  pairs?: Record<string, unknown>[];
};

async function install(
  page: Page,
  theme: "light" | "dark" = "light",
  extra: Extra = {},
) {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript((e: Extra) => ((window as any).__extra = e), extra);
  await page.addInitScript((themeName: string) => {
    const w = window as any;
    const extra: Extra = w.__extra ?? {};
    const callbacks = new Map<number, (e: unknown) => void>();
    const listeners: Record<string, number[]> = {};
    const handlerOf = new Map<number, { event: string; handler: number }>();
    let callback = 0;
    const settings: Record<string, any> = {
      onboarding_completed: true,
      app_language: "de",
      theme: themeName,
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
      llm_connections: [
        {
          id: "cli",
          kind: "claude_cli",
          label: "Claude (Abo)",
          base_url: "",
          enabled: true,
        },
      ],
      llm_models: [
        {
          id: "cli:sonnet",
          connection_id: "cli",
          remote_id: "sonnet",
          label: "Claude Sonnet",
          enabled: true,
          tags: [],
        },
      ],
      llm_active_model_id: "cli:sonnet",
      push_to_talk: true,
    };
    Object.assign(settings, extra.settings ?? {});
    // Wie der Kern: ohne freigegebenen Bereich gibt es keine Werkzeuge. Tests mit Werkzeugen, die
    // nichts anderes sagen, geben „Besprechungen“ frei.
    if (
      extra.tools?.length &&
      !("voice_call_knowledge" in (extra.settings ?? {}))
    )
      settings.voice_call_knowledge = ["meetings"];
    w.__settings = settings;
    w.__tools = extra.tools ?? [];
    w.__listed = [] as number[];
    w.__tool = null;
    w.__turnReplies = [] as string[];
    // Gespraechsverlauf (U3): Attrappe der Befehle `conversation*`, im Speicher.
    const convs: Record<string, any>[] = [];
    const entries: Record<string, Record<string, any>[]> = {};
    let seq = 0;
    let pinSeq = 0;
    const find = (id: string) => {
      const c = convs.find((x) => x.id === id);
      if (!c) throw "Das Gespräch gibt es nicht (mehr).";
      return c;
    };
    if (extra.seed) {
      const now = new Date().toISOString();
      const c = {
        id: "conv_seed000000000000000000",
        title: extra.seed.title,
        created_at: now,
        updated_at: now,
        kind: "voice",
        pinned: false,
        archived: false,
      };
      convs.push(c);
      entries[c.id] = extra.seed.entries.map((e, i) => ({
        id: `entry_seed${String(i).padStart(20, "0")}`,
        conversation_id: c.id,
        status: "complete",
        created_at: now,
        ...e,
      }));
    }
    // Lernspeicher (Stufe 3): Paare im Speicher, Antworten der Einrichtung vorgebbar.
    w.__pairs = JSON.parse(JSON.stringify(extra.pairs ?? [])) as Record<
      string,
      any
    >[];
    w.__transcript = "Das habe ich verstanden";
    w.__setupOutcome = {
      reason: "ok",
      matched_percent: 91,
      pairs: [] as Record<string, any>[],
    };
    w.__convs = convs;
    w.__failAppend = false;
    w.__stored = () => Object.values(entries).flat().length;
    w.__calls = [];
    w.__chat = null;
    w.__listening = (event: string) => (listeners[event] ?? []).length;
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
            case "voice_call_start":
              return w.__startHang ? new Promise(() => {}) : null;
            case "voice_call_chat":
              return new Promise((resolve) => {
                w.__chat = { args, resolve };
              });
            case "voice_call_cancel":
              w.__hung?.get(args.requestId)?.("");
              return false;
            case "voice_tools_list": {
              const granted = (
                (settings.voice_call_knowledge ?? []) as string[]
              ).length;
              const list = granted ? w.__tools : [];
              w.__listed.push(list.length);
              return list;
            }
            case "change_voice_call_knowledge_setting":
              settings.voice_call_knowledge = args.areas;
              return null;
            case "change_voice_call_read_transcripts_setting":
              settings.voice_call_read_transcripts = args.enabled;
              return null;
            case "voice_tool_call":
              return new Promise((resolve, reject) => {
                w.__tool = { args, resolve, reject };
              });
            case "voice_call_turn": {
              // Orchestrator (Stufe 2): Antwort aus der Warteschlange `__turnReplies`.
              if (w.__turnHang)
                return new Promise((resolve) => {
                  (w.__hung ??= new Map()).set(args.requestId, resolve);
                });
              const text = (w.__turnReplies as string[]).shift() ?? "";
              w.__emit("voice-call", {
                kind: "delta",
                request_id: args.requestId,
                text,
              });
              w.__emit("voice-call", {
                kind: "usage",
                request_id: args.requestId,
                usage: {
                  prompt_tokens: 100,
                  completion_tokens: 10,
                  usage_event_ids: [],
                  usage_event_id: null,
                  provider_id: "claude_cli",
                  local: false,
                },
              });
              return text;
            }
            case "conversations_list":
              return convs
                .filter((c) => args.includeArchived || !c.archived)
                .sort(
                  (a, b) =>
                    Number(b.pinned) - Number(a.pinned) ||
                    (b.pinned_at ?? 0) - (a.pinned_at ?? 0) ||
                    (a.updated_at < b.updated_at ? 1 : -1),
                )
                .map((c) => {
                  const list = entries[c.id] ?? [];
                  return {
                    id: c.id,
                    title: c.title,
                    updated_at: c.updated_at,
                    pinned: c.pinned,
                    archived: c.archived,
                    entry_count: list.length,
                    last_text: list.length ? list[list.length - 1].text : "",
                  };
                });
            case "conversation_create": {
              const now = new Date().toISOString();
              const c = {
                id: `conv_${String(++seq).padStart(26, "0")}`,
                title: args.title || "Neues Gespräch",
                created_at: now,
                updated_at: now,
                kind: "voice",
                pinned: false,
                archived: false,
              };
              convs.push(c);
              entries[c.id] = [];
              return c;
            }
            case "conversation_get":
              return {
                conversation: find(args.id),
                entries: entries[args.id] ?? [],
              };
            case "conversation_update": {
              const c = find(args.id);
              if (args.title != null) c.title = args.title;
              if (args.pinned != null) c.pinned = args.pinned;
              if (args.archived != null) c.archived = args.archived;
              return c;
            }
            case "conversation_set_pinned": {
              const c = find(args.id);
              c.pinned = args.pinned;
              c.pinned_at = args.pinned ? (c.pinned_at ?? ++pinSeq) : null;
              return c;
            }
            case "llm_set_active_model":
              settings.llm_active_model_id = args.id;
              return null;
            case "llm_set_function_model": {
              const fm = { ...(settings.function_models ?? {}) };
              if (args.modelId)
                fm[args.forFunction] = {
                  model_id: args.modelId,
                  effort: args.effort ?? null,
                };
              else delete fm[args.forFunction];
              settings.function_models = fm;
              return null;
            }
            case "llm_upsert_model": {
              settings.llm_models = settings.llm_models.map((m: any) =>
                m.id === args.model.id ? args.model : m,
              );
              return null;
            }
            case "llm_model_efforts":
              return w.__efforts ?? ["low", "medium", "high"];
            case "conversation_delete": {
              const c = find(args.id);
              convs.splice(convs.indexOf(c), 1);
              delete entries[c.id];
              return null;
            }
            case "conversation_append_entry": {
              if (w.__failAppend) throw "Datenbank gesperrt.";
              const c = find(args.conversationId);
              const stored = {
                ...args.entry,
                id: `entry_${String(++seq).padStart(26, "0")}`,
                conversation_id: c.id,
              };
              entries[c.id].push(stored);
              c.updated_at = new Date().toISOString();
              return stored;
            }
            case "conversation_update_entry": {
              const found = Object.values(entries)
                .flat()
                .find((x) => x.id === args.id);
              if (!found) throw "Den Beitrag gibt es nicht (mehr).";
              found.text = args.text;
              found.status = args.status;
              return found;
            }
            case "speech_learning_feedback":
              return { reason: "ok", learned: 1, suggestions: 0, adopted: 0 };
            case "speech_learning_list":
              return w.__pairs;
            case "speech_learning_set_status": {
              const p = w.__pairs.find((x: any) => x.id === args.id);
              if (!p) throw "Den Eintrag gibt es nicht (mehr).";
              p.status = args.status;
              return p;
            }
            case "speech_learning_delete":
              w.__pairs = w.__pairs.filter((x: any) => x.id !== args.id);
              return null;
            case "speech_learning_clear": {
              const n = w.__pairs.length;
              w.__pairs = [];
              return n;
            }
            case "speech_learning_setup_text":
              return {
                sentences: [
                  "Der Bürgermeister bat die Verwaltung, den Bauantrag zu prüfen.",
                  "Das Ordnungsamt versendet den Gebührenbescheid zur Grundsteuer.",
                  "Außerdem noch einige Begriffe: Amtsblatt.",
                ],
                text: "Der Bürgermeister bat die Verwaltung, den Bauantrag zu prüfen. Das Ordnungsamt versendet den Gebührenbescheid zur Grundsteuer. Außerdem noch einige Begriffe: Amtsblatt.",
              };
            case "speech_learning_setup_prepare":
              return null;
            case "speech_learning_setup_evaluate":
              w.__pairs.push(...w.__setupOutcome.pairs);
              return w.__setupOutcome;
            case "voice_call_transcribe":
              return w.__transcript;
            case "plugin:dialog|open":
              return "C:\\Daten\\Protokoll.pdf";
            case "plugin:fs|stat":
              return { size: 2048, isFile: true, isDirectory: false };
            case "tts_extract_document":
              return "Inhalt der Datei";
            case "meetings_is_recording":
              return false;
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
            case "meetings_list":
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
    // Die Attrappe hat ein Cloud-Modell (Abo) als Antwortmodell: ohne Angabe gilt „frei“, sonst
    // sperrt die Vorgabe „nur lokal“ das Gespräch (eigene Tests prüfen genau das).
    localStorage.setItem(
      "voice-call.config",
      JSON.stringify(
        extra.config ?? { orchestration: { souveraenitaet: "frei" } },
      ),
    );
  }, theme);
}

const calls = (page: Page, cmd: string) =>
  page.evaluate(
    (c) => (window as any).__calls.filter((x: Call) => x.cmd === c) as Call[],
    cmd,
  );

const nav = (page: Page) =>
  page.getByRole("navigation", { name: "Hauptnavigation" });

/** Schmal ist die Navigation ein Drawer hinter dem Menueknopf. */
async function goTo(page: Page, name: string) {
  const menu = page.getByRole("button", { name: "Menü öffnen", exact: true });
  if (await menu.isVisible()) await menu.click();
  await nav(page).getByRole("button", { name, exact: true }).click();
}

async function openPage(page: Page) {
  await page.goto("/");
  await goTo(page, "Sprachgespräch");
  await expect(
    page.getByRole("heading", { name: "Worüber möchtest du sprechen?" }),
  ).toBeVisible();
}

/** Kontextmenü „Gesprächsoptionen“ rechts in der Infozeile. */
async function openOptions(page: Page, item: string) {
  await page.getByRole("button", { name: "Gesprächsoptionen" }).click();
  await page.getByRole("menuitem", { name: item, exact: true }).click();
}
/** Rechte Spalte über das Kontextmenü öffnen. */
const openSettingsPanel = (page: Page) =>
  openOptions(page, "Gesprächseinstellungen");

/** Suchoverlay über die Lupe in der Verlaufsspalte. */
async function openSearch(page: Page) {
  await page.getByRole("button", { name: "Gespräche durchsuchen" }).click();
  const search = page.getByTestId("history-search");
  await expect(search).toBeVisible();
  return search;
}
const permissionChip = (page: Page) => page.getByTestId("permission-chip");

const message = (page: Page) =>
  page.getByRole("textbox", { name: "Nachricht" });

const history = (page: Page) =>
  page.getByRole("navigation", { name: "Gesprächsverlauf" });
const historyItem = (page: Page, title: string) =>
  history(page).getByRole("button", { name: new RegExp(`^${title}`) });
// Der Schalter im Seitenkopf (nicht der Menüpunkt „Verlauf“ der Seitenleiste).
const historyToggle = (page: Page) =>
  page.locator("button[aria-controls][aria-label='Gesprächsverlauf']");
const historyToggleSelector =
  "button[aria-controls][aria-label='Gesprächsverlauf']";
const newChat = (page: Page) =>
  page.getByRole("button", { name: "Neues Gespräch", exact: true });
const emptyTitle = (page: Page) =>
  page.getByRole("heading", { name: "Worüber möchtest du sprechen?" });

/** Schickt eine Nachricht und beantwortet sie mit den gegebenen Saetzen. */
async function exchange(
  page: Page,
  text: string,
  reply: string,
  usage = {
    prompt_tokens: 120,
    completion_tokens: 30,
    usage_event_ids: [] as number[],
    usage_event_id: null,
    provider_id: "claude_cli",
    local: false,
  },
) {
  await page.evaluate(() => ((window as any).__chat = null));
  await message(page).fill(text);
  await message(page).press("Enter");
  await expect
    .poll(() => page.evaluate(() => (window as any).__chat !== null))
    .toBe(true);
  const requestId = await page.evaluate(
    () => (window as any).__chat.args.requestId as string,
  );
  await page.waitForFunction(
    () => (window as any).__listening("voice-call") > 0,
  );
  await page.evaluate(
    ([id, sentence, u]) => {
      const w = window as any;
      w.__emit("voice-call", {
        kind: "sentence",
        request_id: id,
        index: 0,
        text: sentence,
      });
      w.__emit("voice-call", { kind: "done", request_id: id, usage: u });
      w.__chat.resolve(sentence);
    },
    [requestId, reply, usage] as const,
  );
  await expect(page.getByRole("log").getByText(reply)).toBeVisible();
}

test.describe("Sprachgespraech", () => {
  test.beforeEach(async ({ page }) => {
    await install(page);
  });

  test("Infozeile ist eine Zeile mit i-Knopf und Popover", async ({ page }) => {
    await openPage(page);
    const info = page.getByRole("button", {
      name: "Informationen zum Sprachgespräch",
    });
    await expect(info).toBeVisible();
    const row = info.locator("xpath=..");
    const box = await row.boundingBox();
    expect(box!.height).toBeLessThanOrEqual(40);
    await info.click();
    await expect(
      page.getByText("Unterbrechen: einfach weitersprechen"),
    ).toBeVisible();
    await expect(page.getByText("Token Eingabe / Ausgabe")).toBeVisible();
  });

  test("Text senden zeigt Antwort, Token und Abo-Hinweis", async ({ page }) => {
    await openPage(page);
    await exchange(page, "Hallo KI", "Hallo zurück.");
    const sent = await calls(page, "voice_call_chat");
    expect(sent).toHaveLength(1);
    expect(JSON.stringify(sent[0].args.messages)).toContain("Hallo KI");
    await expect(page.getByText("Hallo KI").first()).toBeVisible();
    await expect(page.getByText("150 Token")).toBeVisible();
    await expect(page.getByText("Abo · keine API-Kosten")).toBeVisible();
  });

  test("Fußleiste: Telefon immer sichtbar, Gesprächsmenü nur auf dieser Seite", async ({
    page,
  }) => {
    await openPage(page);
    const status = page.locator(".workspace-status");
    const phone = status.locator("[data-call-button]");
    const menuBtn = status.getByRole("button", {
      name: "Gespräch: Mikrofon, Stimme und Sprachfluss",
    });
    // Kein Wort „Gespräch“ mehr in der Leiste: nur das Telefon, grau ohne Anruf.
    await expect(phone).toBeVisible();
    await expect(phone).toHaveAttribute("data-state", "idle");
    await expect(phone).toHaveAttribute("aria-pressed", "false");
    await expect(status.getByText("Gespräch", { exact: true })).toHaveCount(0);
    await menuBtn.click();
    await expect(page.getByRole("menu", { name: "Gespräch" })).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("menu", { name: "Gespräch" })).toHaveCount(0);
    await goTo(page, "Verlauf");
    await expect(menuBtn).toHaveCount(0);
    await expect(phone).toBeVisible();
  });

  test("Plus-Menü bietet Dateien und deaktiviertes Corporate LLM", async ({
    page,
  }) => {
    await openPage(page);
    await page.getByRole("button", { name: "Hinzufügen" }).click();
    const menu = page.getByRole("menu", { name: "Hinzufügen" });
    await expect(menu).toBeVisible();
    await expect(
      menu.getByRole("menuitem", { name: /Dateien anhängen/ }),
    ).toBeVisible();
    await expect(
      menu.getByRole("menuitem", { name: /Aus Corporate LLM/ }),
    ).toHaveAttribute("aria-disabled", "true");
  });

  test("Neues Gespräch während laufender Antwort verwirft die späte Antwort", async ({
    page,
  }) => {
    await openPage(page);
    await message(page).fill("Erzähl etwas");
    await message(page).press("Enter");
    await expect
      .poll(() => page.evaluate(() => (window as any).__chat !== null))
      .toBe(true);
    const requestId = await page.evaluate(
      () => (window as any).__chat.args.requestId as string,
    );
    await expect(
      page.getByRole("button", { name: "Antwort stoppen" }),
    ).toBeVisible();
    await page.getByRole("button", { name: "Neues Gespräch" }).click();
    await expect
      .poll(async () => (await calls(page, "voice_call_cancel")).length)
      .toBe(1);
    expect((await calls(page, "voice_call_cancel"))[0].args.requestId).toBe(
      requestId,
    );
    // Die Antwort trifft trotzdem noch ein: sie darf nichts mehr schreiben.
    await page.evaluate(
      ([id]) => {
        const w = window as any;
        w.__emit("voice-call", {
          kind: "sentence",
          request_id: id,
          index: 0,
          text: "Späte Antwort.",
        });
        w.__emit("voice-call", {
          kind: "done",
          request_id: id,
          usage: {
            prompt_tokens: 50,
            completion_tokens: 5,
            provider_id: "claude_cli",
            local: false,
          },
        });
        w.__chat.resolve("Späte Antwort.");
      },
      [requestId] as const,
    );
    await expect(
      page.getByRole("heading", { name: "Worüber möchtest du sprechen?" }),
    ).toBeVisible();
    await expect(page.getByText("Späte Antwort.")).toHaveCount(0);
    await expect(page.getByText("55 Token")).toHaveCount(0);
  });

  test("Nachricht senden legt das Gespräch an, der Titel erscheint in der Liste", async ({
    page,
  }) => {
    await openPage(page);
    await expect(history(page)).toBeVisible();
    expect(await calls(page, "conversation_create")).toHaveLength(0);
    await exchange(page, "Hallo KI", "Hallo zurück.");
    await expect(historyItem(page, "Hallo KI")).toBeVisible();
    await expect(historyItem(page, "Hallo KI")).toHaveAttribute(
      "aria-current",
      "true",
    );
    const created = await calls(page, "conversation_create");
    expect(created).toHaveLength(1);
    expect(created[0].args.title).toBe("Hallo KI");
    await expect
      .poll(async () => (await calls(page, "conversation_append_entry")).length)
      .toBe(2);
    const [user, assistant] = (
      await calls(page, "conversation_append_entry")
    ).map((c) => c.args.entry);
    expect(user).toMatchObject({
      role: "user",
      mode: "typed",
      text: "Hallo KI",
      status: "complete",
    });
    expect(assistant).toMatchObject({
      role: "assistant",
      mode: "typed",
      text: "Hallo zurück.",
      status: "complete",
    });
    // Verbrauch steht nur in ext["local-voice-ai"], nicht in Vertragsfeldern.
    expect(assistant.ext["local-voice-ai"]).toMatchObject({
      prompt_tokens: 120,
      completion_tokens: 30,
      provider_id: "claude_cli",
      local: false,
    });
    expect(assistant.usage).toBeUndefined();
    // Eine zweite Nachricht hängt an dasselbe Gespräch an.
    await exchange(page, "Noch eine Frage", "Noch eine Antwort.");
    await expect
      .poll(async () => (await calls(page, "conversation_append_entry")).length)
      .toBe(4);
    expect(await calls(page, "conversation_create")).toHaveLength(1);
  });

  test("Titel wird an der Wortgrenze auf 60 Zeichen gekürzt", async ({
    page,
  }) => {
    await openPage(page);
    await exchange(
      page,
      "Fasse bitte das gestrige Meeting über Budget Termine und Risiken kurz zusammen",
      "Gern.",
    );
    const title = (await calls(page, "conversation_create"))[0].args
      .title as string;
    expect(title.length).toBeLessThanOrEqual(60);
    expect(title.endsWith("…")).toBe(true);
    expect(title).toBe(
      "Fasse bitte das gestrige Meeting über Budget Termine und…",
    );
  });

  test("Neues Gespräch und zurückwechseln zeigt den alten Verlauf", async ({
    page,
  }) => {
    await openPage(page);
    await exchange(page, "Hallo KI", "Hallo zurück.");
    await expect(page.getByText("150 Token")).toBeVisible();
    await newChat(page).click();
    await expect(emptyTitle(page)).toBeVisible();
    await expect(page.getByText("150 Token")).toHaveCount(0);
    await exchange(page, "Zweite Frage", "Zweite Antwort.", {
      prompt_tokens: 40,
      completion_tokens: 10,
      usage_event_ids: [],
      usage_event_id: null,
      provider_id: "claude_cli",
      local: false,
    });
    await expect(historyItem(page, "Zweite Frage")).toBeVisible();
    await expect(historyItem(page, "Hallo KI")).toBeVisible();

    await historyItem(page, "Hallo KI").click();
    const log = page.getByRole("log");
    await expect(log.getByText("Hallo zurück.")).toBeVisible();
    await expect(log.getByText("Zweite Antwort.")).toHaveCount(0);
    await expect(page.getByText("150 Token")).toBeVisible();
    await expect(historyItem(page, "Hallo KI")).toHaveAttribute(
      "aria-current",
      "true",
    );

    // Die KI kennt den geöffneten Verlauf.
    await exchange(page, "Und weiter?", "Weiter so.");
    const sent = await calls(page, "voice_call_chat");
    const last = JSON.stringify(sent[sent.length - 1].args.messages);
    expect(last).toContain("Hallo KI");
    expect(last).toContain("Hallo zurück.");
    expect(last).not.toContain("Zweite Frage");
    // … und der neue Beitrag landet im geöffneten Gespräch.
    await expect
      .poll(async () => (await calls(page, "conversation_append_entry")).length)
      .toBe(6);
    const all = await calls(page, "conversation_append_entry");
    expect(all[4].args.conversationId).toBe(all[0].args.conversationId);
    expect(all[4].args.conversationId).not.toBe(all[2].args.conversationId);
  });

  test("Unterbrochene, schon gespeicherte Antwort bleibt eine Sprechblase und ein Beitrag", async ({
    page,
  }) => {
    await openPage(page);
    await exchange(
      page,
      "Erzähl mir etwas",
      "Volle Antwort, die ich gerade sprach.",
    );
    await expect
      .poll(async () => (await calls(page, "conversation_append_entry")).length)
      .toBe(2);
    // Der Anruf meldet nach der fertigen Antwort, was tatsächlich gesprochen wurde.
    await page.evaluate(() => {
      const engine = (window as any).__voiceCallEngine;
      engine.cb.onAssistant("Volle Antwort", true, true);
    });
    const log = page.getByRole("log");
    await expect(log.getByText("Volle Antwort")).toBeVisible();
    await expect(
      log.getByText("Volle Antwort, die ich gerade sprach."),
    ).toHaveCount(0);
    await expect(log.getByText("Volle Antwort")).toHaveCount(1);
    await expect
      .poll(async () => (await calls(page, "conversation_update_entry")).length)
      .toBe(1);
    const update = (await calls(page, "conversation_update_entry"))[0].args;
    expect(update).toMatchObject({
      text: "Volle Antwort",
      status: "interrupted",
    });
    expect(await calls(page, "conversation_append_entry")).toHaveLength(2);
    // Wieder geöffnet: eine Antwort, als unterbrochen gespeichert.
    await newChat(page).click();
    await historyItem(page, "Erzähl mir etwas").click();
    await expect(log.getByText("Volle Antwort")).toHaveCount(1);
  });

  test("Anhänge werden als Vertragsfeld gespeichert, ohne Dateiinhalt", async ({
    page,
  }) => {
    await openPage(page);
    await page.getByRole("button", { name: "Hinzufügen" }).click();
    await page.getByRole("menuitem", { name: /Dateien anhängen/ }).click();
    await expect(page.getByText("Protokoll.pdf")).toBeVisible();
    await exchange(page, "Fasse das zusammen", "Erledigt.");
    await expect
      .poll(async () => (await calls(page, "conversation_append_entry")).length)
      .toBe(2);
    const user = (await calls(page, "conversation_append_entry"))[0].args.entry;
    expect(user.attachments).toHaveLength(1);
    expect(user.attachments[0]).toMatchObject({
      name: "Protokoll.pdf",
      mime: "application/pdf",
      size_bytes: 2048,
    });
    expect(JSON.stringify(user)).not.toContain("Inhalt der Datei");
  });

  test("Fehlgeschlagenes Speichern wird gemeldet und beim nächsten Beitrag nachgeholt", async ({
    page,
  }) => {
    await openPage(page);
    await page.evaluate(() => ((window as any).__failAppend = true));
    await exchange(page, "Hallo KI", "Hallo zurück.");
    await expect(
      page.getByRole("alert").filter({ hasText: "Speichern im Verlauf" }),
    ).toBeVisible();
    expect(await page.evaluate(() => (window as any).__stored())).toBe(0);
    await page.evaluate(() => ((window as any).__failAppend = false));
    await exchange(page, "Noch eine Frage", "Noch eine Antwort.");
    await expect
      .poll(() => page.evaluate(() => (window as any).__stored()))
      .toBe(4);
    await expect(
      page.getByRole("alert").filter({ hasText: "Speichern im Verlauf" }),
    ).toHaveCount(0);
    const detail = await page.evaluate(async () => {
      const w = window as any;
      const id = w.__convs[0].id;
      return (
        await w.__TAURI_INTERNALS__.invoke("conversation_get", { id })
      ).entries.map((e: any) => e.text);
    });
    expect(detail).toEqual([
      "Hallo KI",
      "Hallo zurück.",
      "Noch eine Frage",
      "Noch eine Antwort.",
    ]);
  });

  test("Archivieren des offenen Gesprächs beginnt ein neues", async ({
    page,
  }) => {
    await openPage(page);
    await exchange(page, "Hallo KI", "Hallo zurück.");
    await page.getByRole("button", { name: "Optionen für „Hallo KI“" }).click();
    await page.getByRole("menuitem", { name: "Archivieren" }).click();
    await expect(emptyTitle(page)).toBeVisible();
    await expect(historyItem(page, "Hallo KI")).toHaveCount(0);
  });

  test("Fokus kehrt nach Menüaktion und Escape in der Löschfrage zur Zeile zurück", async ({
    page,
  }) => {
    await openPage(page);
    await exchange(page, "Hallo KI", "Hallo zurück.");
    await page.getByRole("button", { name: "Optionen für „Hallo KI“" }).click();
    await expect(
      page.getByRole("button", { name: "Optionen für „Hallo KI“" }),
    ).toHaveAttribute("aria-controls", /.+/);
    await page.getByRole("menuitem", { name: "Anheften" }).click();
    await expect(historyItem(page, "Hallo KI")).toBeFocused();

    await page.getByRole("button", { name: "Optionen für „Hallo KI“" }).click();
    // Deaktivierte Einträge bleiben mit den Pfeiltasten erreichbar.
    await page.keyboard.press("End");
    await expect(
      page.getByRole("menuitem", { name: /In Corporate LLM öffnen/ }),
    ).toBeFocused();
    await page.keyboard.press("Home");
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("Enter");
    const confirm = page.getByRole("group", {
      name: "„Hallo KI“ endgültig löschen?",
    });
    await expect(confirm).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(confirm).toHaveCount(0);
    await expect(historyItem(page, "Hallo KI")).toBeFocused();
    expect(await calls(page, "conversation_delete")).toHaveLength(0);
  });

  test("Gespräch umbenennen", async ({ page }) => {
    await openPage(page);
    await exchange(page, "Hallo KI", "Hallo zurück.");
    await page.getByRole("button", { name: "Optionen für „Hallo KI“" }).click();
    await page.getByRole("menuitem", { name: "Umbenennen" }).click();
    const input = page.getByRole("textbox", { name: "Neuer Titel" });
    await expect(input).toBeFocused();
    await input.fill("Mein Plan");
    await input.press("Enter");
    await expect(historyItem(page, "Mein Plan")).toBeVisible();
    await expect(historyItem(page, "Hallo KI")).toHaveCount(0);
    const updates = await calls(page, "conversation_update");
    expect(updates).toHaveLength(1);
    expect(updates[0].args.title).toBe("Mein Plan");
  });

  test("Umbenennen mit Escape ändert nichts", async ({ page }) => {
    await openPage(page);
    await exchange(page, "Hallo KI", "Hallo zurück.");
    await page.getByRole("button", { name: "Optionen für „Hallo KI“" }).click();
    await page.getByRole("menuitem", { name: "Umbenennen" }).click();
    const input = page.getByRole("textbox", { name: "Neuer Titel" });
    await input.fill("Verworfen");
    await input.press("Escape");
    await expect(historyItem(page, "Hallo KI")).toBeVisible();
    expect(await calls(page, "conversation_update")).toHaveLength(0);
  });

  test("Gespräch löschen fragt nach und leert die Ansicht", async ({
    page,
  }) => {
    await openPage(page);
    await exchange(page, "Hallo KI", "Hallo zurück.");
    await page.getByRole("button", { name: "Optionen für „Hallo KI“" }).click();
    await page.getByRole("menuitem", { name: "Löschen" }).click();
    const confirm = page.getByRole("group", {
      name: "„Hallo KI“ endgültig löschen?",
    });
    await expect(confirm).toBeVisible();
    await confirm.getByRole("button", { name: "Abbrechen" }).click();
    await expect(historyItem(page, "Hallo KI")).toBeVisible();
    expect(await calls(page, "conversation_delete")).toHaveLength(0);

    await page.getByRole("button", { name: "Optionen für „Hallo KI“" }).click();
    await page.getByRole("menuitem", { name: "Löschen" }).click();
    await confirm.getByRole("button", { name: "Löschen" }).click();
    await expect(historyItem(page, "Hallo KI")).toHaveCount(0);
    await expect(emptyTitle(page)).toBeVisible();
    expect(await calls(page, "conversation_delete")).toHaveLength(1);
  });

  test("Archivieren blendet aus, Archiv anzeigen holt zurück", async ({
    page,
  }) => {
    await openPage(page);
    await exchange(page, "Hallo KI", "Hallo zurück.");
    await newChat(page).click();
    await exchange(page, "Zweite Frage", "Zweite Antwort.");
    const search = await openSearch(page);
    await search.getByRole("combobox").fill("zweite");
    await expect(
      search.getByRole("option", { name: "Zweite Frage" }),
    ).toBeVisible();
    await expect(search.getByRole("option", { name: "Hallo KI" })).toHaveCount(
      0,
    );
    await page.keyboard.press("Escape");
    await expect(search).toHaveCount(0);

    await page.getByRole("button", { name: "Optionen für „Hallo KI“" }).click();
    await page.getByRole("menuitem", { name: "Archivieren" }).click();
    await expect(historyItem(page, "Hallo KI")).toHaveCount(0);
    await history(page)
      .getByRole("switch", { name: "Archiv anzeigen" })
      .check();
    await expect(historyItem(page, "Hallo KI")).toBeVisible();
    await expect(history(page).getByText("Archiviert")).toBeVisible();
  });

  test("Deaktivierter Eintrag „In Corporate LLM öffnen“ nennt den Grund", async ({
    page,
  }) => {
    await openPage(page);
    await exchange(page, "Hallo KI", "Hallo zurück.");
    await page.getByRole("button", { name: "Optionen für „Hallo KI“" }).click();
    const item = page.getByRole("menuitem", {
      name: /In Corporate LLM öffnen/,
    });
    await expect(item).toHaveAttribute("aria-disabled", "true");
    await expect(item).toContainText("Corporate LLM verbinden");
    await page.keyboard.press("Escape");
    await expect(page.getByRole("menu")).toHaveCount(0);
  });

  test("Verlaufsspalte lässt sich einklappen, die Wahl bleibt", async ({
    page,
  }) => {
    await openPage(page);
    await expect(history(page)).toBeVisible();
    await historyToggle(page).click();
    await expect(history(page)).toHaveCount(0);
    await page.reload();
    await goTo(page, "Sprachgespräch");
    await expect(emptyTitle(page)).toBeVisible();
    await expect(history(page)).toHaveCount(0);
    await historyToggle(page).click();
    await expect(history(page)).toBeVisible();
  });

  test("Schmal liegt der Verlauf als Überlagerung über dem Gespräch", async ({
    browser,
  }) => {
    const ctx = await browser.newContext({
      viewport: { width: 390, height: 844 },
      baseURL: test.info().project.use.baseURL,
    });
    const page = await ctx.newPage();
    await install(page);
    await openPage(page);
    await exchange(page, "Hallo KI", "Hallo zurück.");
    await expect(history(page)).toHaveCount(0);
    await historyToggle(page).click();
    await expect(history(page)).toBeVisible();
    const box = await history(page).boundingBox();
    expect(box!.width).toBeLessThanOrEqual(390 * 0.85 + 1);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
    await newChat(page).click();
    await expect(history(page)).toHaveCount(0);
    await expect(emptyTitle(page)).toBeVisible();
    await ctx.close();
  });

  test("Einstellungs-Panel hat fünf Reiter, Wissen zwischen Erkennung und Modelle", async ({
    page,
  }) => {
    await openPage(page);
    await openSettingsPanel(page);
    const panel = page.getByRole("complementary", {
      name: "Gesprächseinstellungen",
    });
    await expect(panel).toBeVisible();
    await expect(panel.getByRole("tab")).toHaveCount(5);
    const names = ["Gespräch", "Stimme", "Erkennung", "Wissen", "Modelle"];
    for (const n of names)
      await expect(panel.getByRole("tab", { name: n })).toBeVisible();
    await expect(panel.getByRole("tab")).toHaveText(names);
  });
});

// Stufe 2: Orchestrator (Denker und lesende LVA-Werkzeuge). Der Sprecher ist hier ein lokales
// Modell; die Antworten kommen aus `__turnReplies`, Werkzeugaufrufe laufen über `__tool`.
const LOCAL_SPEAKER = {
  llm_connections: [
    {
      id: "loc",
      kind: "local",
      label: "Lokal",
      base_url: "http://127.0.0.1:0/v1",
      enabled: true,
    },
    {
      id: "cli",
      kind: "claude_cli",
      label: "Claude (Abo)",
      base_url: "",
      enabled: true,
    },
  ],
  llm_models: [
    {
      id: "loc:gemma",
      connection_id: "loc",
      remote_id: "gemma",
      label: "Gemma lokal",
      enabled: true,
      tags: [],
    },
    {
      id: "cli:sonnet",
      connection_id: "cli",
      remote_id: "sonnet",
      label: "Claude Sonnet",
      enabled: true,
      tags: [],
    },
  ],
  llm_active_model_id: "loc:gemma",
};

const MEETINGS_TOOL = {
  name: "meetings_list",
  title: "Besprechungen auflisten",
  description: "Listet fertige Besprechungen.",
  parameters_json: JSON.stringify({
    type: "object",
    properties: { limit: { type: "integer", minimum: 1, maximum: 50 } },
    additionalProperties: false,
  }),
  capability: "meeting.read",
};

const orchestration = (patch: Record<string, unknown>) => ({
  orchestration: {
    denkerModelId: "",
    souveraenitaet: "nur_lokal",
    useTools: true,
    ...patch,
  },
});

const replies = (page: Page, list: string[]) =>
  page.evaluate((l) => ((window as any).__turnReplies = [...l]), list);

const send = async (page: Page, text: string) => {
  await message(page).fill(text);
  await message(page).press("Enter");
};

test.describe("Sprachgespraech Orchestrator", () => {
  test("Werkzeugaufruf erscheint als Statuszeile, danach kommt die Antwort", async ({
    page,
  }) => {
    await install(page, "light", {
      settings: LOCAL_SPEAKER,
      config: orchestration({}),
      tools: [MEETINGS_TOOL],
    });
    await openPage(page);
    await replies(page, [
      "\u27e6werkzeug meetings_list {}\u27e7",
      "Gestern gab es zwei Besprechungen.",
    ]);
    await send(page, "Welche Besprechungen hatte ich gestern?");

    await expect
      .poll(() => page.evaluate(() => (window as any).__tool !== null))
      .toBe(true);
    const status = page.getByTestId("call-status");
    await expect(status).toHaveAttribute("aria-live", "polite");
    await expect(status).toHaveText("Schaue in Besprechungen nach …");

    await page.evaluate(() =>
      (window as any).__tool.resolve(
        JSON.stringify({ meetings: [{ title: "Wochenplanung" }] }),
      ),
    );
    await expect(
      page.getByRole("log").getByText("Gestern gab es zwei Besprechungen."),
    ).toBeVisible();
    await expect(status).toHaveText("");

    const tool = await calls(page, "voice_tool_call");
    expect(tool).toHaveLength(1);
    expect(tool[0].args.name).toBe("meetings_list");
    expect(tool[0].args.args).toBe("{}");
    const turns = await calls(page, "voice_call_turn");
    expect(turns).toHaveLength(2);
    expect(turns.every((c) => c.args.modelId === "loc:gemma")).toBe(true);
    expect(JSON.stringify(turns[1].args.messages)).toContain(
      "Daten aus Werkzeug meetings_list",
    );
    expect(await calls(page, "voice_call_chat")).toHaveLength(0);
    // Beide Durchgänge laufen in die Infozeile ein.
    await expect(page.getByText("220 Token")).toBeVisible();
  });

  test("Modell je Funktion: der Sprecher folgt function_models.voice_speaker", async ({
    page,
  }) => {
    // Standardmodell ist Sonnet (Protokoll usw.), fürs Gespräch gilt Gemma lokal.
    await install(page, "light", {
      settings: {
        ...LOCAL_SPEAKER,
        llm_active_model_id: "cli:sonnet",
        function_models: { voice_speaker: { model_id: "loc:gemma", effort: null } },
      },
      config: orchestration({
        denkerModelId: "cli:sonnet",
        souveraenitaet: "frei",
        useTools: false,
      }),
    });
    await openPage(page);
    await replies(page, [
      '\u27e6werkzeug denker_fragen {"frage":"Was ist ein Qubit?"}\u27e7',
      "Ein Qubit ist die Recheneinheit eines Quantencomputers.",
    ]);
    await send(page, "Was ist ein Qubit?");
    await expect(
      page
        .getByRole("log")
        .getByText("Ein Qubit ist die Recheneinheit eines Quantencomputers."),
    ).toBeVisible();
    const turns = await calls(page, "voice_call_turn");
    expect(turns.map((c) => [c.args.modelId, c.args.forFunction])).toEqual([
      ["loc:gemma", "voice_speaker"],
      ["cli:sonnet", null],
    ]);
  });

  test("Denker-Übergabe: zweites Modell antwortet, beide Verbräuche zählen", async ({
    page,
  }) => {
    await install(page, "light", {
      settings: LOCAL_SPEAKER,
      config: orchestration({
        denkerModelId: "cli:sonnet",
        souveraenitaet: "frei",
        useTools: false,
      }),
    });
    await openPage(page);
    await replies(page, [
      '\u27e6werkzeug denker_fragen {"frage":"Was ist ein Qubit?"}\u27e7',
      "Ein Qubit ist die Recheneinheit eines Quantencomputers.",
    ]);
    await send(page, "Was ist ein Qubit?");
    await expect(
      page
        .getByRole("log")
        .getByText("Ein Qubit ist die Recheneinheit eines Quantencomputers."),
    ).toBeVisible();
    const turns = await calls(page, "voice_call_turn");
    expect(turns.map((c) => c.args.modelId)).toEqual([
      "loc:gemma",
      "cli:sonnet",
    ]);
    expect(await calls(page, "voice_call_chat")).toHaveLength(0);
    expect(await calls(page, "voice_tools_list")).toHaveLength(0);
    await expect(page.getByText("220 Token")).toBeVisible();
  });

  test("„nur lokal“ sperrt einen Cloud-Denker und sagt es", async ({
    page,
  }) => {
    await install(page, "light", {
      settings: LOCAL_SPEAKER,
      config: orchestration({
        denkerModelId: "cli:sonnet",
        souveraenitaet: "nur_lokal",
      }),
      tools: [MEETINGS_TOOL],
    });
    await openPage(page);
    await replies(page, ["Das weiß ich auch so."]);
    await send(page, "Wie spät ist es ungefähr?");
    await expect(
      page.getByRole("log").getByText("Das weiß ich auch so."),
    ).toBeVisible();
    await expect(page.getByTestId("call-notice")).toContainText(
      "Das Denker-Modell ist bei dieser Souveränität nicht zulässig und wird nicht genutzt.",
    );
    const turns = await calls(page, "voice_call_turn");
    expect(turns).toHaveLength(1);
    expect(turns[0].args.modelId).toBe("loc:gemma");
    // Der Denker wird dem Sprecher nicht einmal angeboten.
    expect(JSON.stringify(turns[0].args.messages)).not.toContain(
      "denker_fragen",
    );

    await openSettingsPanel(page);
    const panel = page.getByRole("complementary", {
      name: "Gesprächseinstellungen",
    });
    await panel.getByRole("tab", { name: "Modelle" }).click();
    await expect(
      panel.getByText(
        "Bei „nur lokal“ wird dieses Cloud-Modell nicht genutzt.",
      ),
    ).toBeVisible();
    await expect(panel.getByRole("radio", { name: "Nur lokal" })).toBeChecked();
  });

  test("Gesperrter Sprecher: kein Gespräch, Alarm mit Knopf zu den Einstellungen", async ({
    page,
  }) => {
    // Auch ohne Werkzeuge und Denker: das Cloud-Modell bekommt bei „nur lokal“ nichts.
    await install(page, "light", {
      config: orchestration({ useTools: false }),
      tools: [MEETINGS_TOOL],
    });
    await openPage(page);
    await send(page, "Hallo KI");
    const alert = page.getByRole("alert").filter({
      hasText: "kein Gespräch möglich",
    });
    await expect(alert).toBeVisible();
    expect(await calls(page, "voice_call_chat")).toHaveLength(0);
    expect(await calls(page, "voice_call_turn")).toHaveLength(0);
    expect(await calls(page, "voice_tool_call")).toHaveLength(0);
    await alert.getByRole("button", { name: "Einstellungen öffnen" }).click();
    const panel = page.getByRole("complementary", {
      name: "Gesprächseinstellungen",
    });
    await expect(panel.getByRole("tab", { name: "Modelle" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    // Derselbe Hinweis in den Einstellungen.
    await expect(
      panel.getByRole("alert").filter({ hasText: "nicht zulässig" }),
    ).toBeVisible();
  });

  test("Ein Fehler des Werkzeugs geht als Text zurück und wird gesprochen", async ({
    page,
  }) => {
    await install(page, "light", {
      settings: LOCAL_SPEAKER,
      config: orchestration({}),
      tools: [MEETINGS_TOOL],
    });
    await openPage(page);
    await replies(page, [
      "\u27e6werkzeug meetings_list {}\u27e7",
      "Das hat leider nicht geklappt.",
    ]);
    await send(page, "Welche Besprechungen gibt es?");
    await expect
      .poll(() => page.evaluate(() => (window as any).__tool !== null))
      .toBe(true);
    await page.evaluate(() =>
      (window as any).__tool.reject("Die Datenbank ist gesperrt."),
    );
    await expect(
      page.getByRole("log").getByText("Das hat leider nicht geklappt."),
    ).toBeVisible();
    const turns = await calls(page, "voice_call_turn");
    expect(JSON.stringify(turns[1].args.messages)).toContain(
      "Die Datenbank ist gesperrt.",
    );
    await expect(page.getByTestId("call-status")).toHaveText("");
  });

  test("Wächter: hängendes Modell wird abgebrochen und die Meldung gesprochen", async ({
    page,
  }) => {
    await page.clock.install();
    await install(page, "light", {
      settings: LOCAL_SPEAKER,
      config: orchestration({ useTools: false, denkerModelId: "loc:gemma" }),
    });
    await openPage(page);
    await page.evaluate(() => ((window as any).__turnHang = true));
    await send(page, "Hallo?");
    await expect
      .poll(async () => (await calls(page, "voice_call_turn")).length)
      .toBe(1);
    await page.clock.fastForward(21_000);
    await expect(
      page.getByRole("log").getByText("Das Modell antwortet gerade nicht."),
    ).toBeVisible();
    const cancels = await calls(page, "voice_call_cancel");
    const turn = (await calls(page, "voice_call_turn"))[0];
    expect(cancels.some((c) => c.args.requestId === turn.args.requestId)).toBe(
      true,
    );
  });

  test("Wächter: hängendes Werkzeug liefert eine Meldung an das Modell", async ({
    page,
  }) => {
    await page.clock.install();
    await install(page, "light", {
      settings: LOCAL_SPEAKER,
      config: orchestration({}),
      tools: [MEETINGS_TOOL],
    });
    await openPage(page);
    await replies(page, [
      "\u27e6werkzeug meetings_list {}\u27e7",
      "Das Werkzeug war zu langsam.",
    ]);
    await send(page, "Welche Besprechungen gibt es?");
    await expect
      .poll(() => page.evaluate(() => (window as any).__tool !== null))
      .toBe(true);
    await page.clock.fastForward(11_000);
    await expect(
      page.getByRole("log").getByText("Das Werkzeug war zu langsam."),
    ).toBeVisible();
    const turns = await calls(page, "voice_call_turn");
    expect(JSON.stringify(turns[1].args.messages)).toContain(
      "nicht rechtzeitig",
    );
  });

  test("Jeder Durchgang nennt Modellkennung und Souveränität ausdrücklich", async ({
    page,
  }) => {
    await install(page, "light", {
      settings: LOCAL_SPEAKER,
      config: orchestration({}),
      tools: [MEETINGS_TOOL],
    });
    await openPage(page);
    await replies(page, ["Hallo, wie kann ich helfen?"]);
    await send(page, "Hallo");
    await expect(
      page.getByRole("log").getByText("Hallo, wie kann ich helfen?"),
    ).toBeVisible();
    const turns = await calls(page, "voice_call_turn");
    expect(turns[0].args.modelId).toBe("loc:gemma");
    expect(turns[0].args.stufe).toBe("nur_lokal");
  });

  test("Ohne Denker und ohne Werkzeuge bleibt der alte Weg", async ({
    page,
  }) => {
    await install(page, "light", {
      settings: LOCAL_SPEAKER,
      config: orchestration({ useTools: false }),
      tools: [MEETINGS_TOOL],
    });
    await openPage(page);
    await exchange(page, "Hallo KI", "Hallo zurück.");
    expect(await calls(page, "voice_call_chat")).toHaveLength(1);
    expect(await calls(page, "voice_call_turn")).toHaveLength(0);
    expect(await calls(page, "voice_tools_list")).toHaveLength(0);
    await expect(page.getByTestId("call-status")).toHaveText("");
  });

  test("Einstellungen: Vorgaben und Speichern der Orchestrator-Wahl", async ({
    page,
  }) => {
    await install(page, "light", {
      settings: LOCAL_SPEAKER,
      config: orchestration({}),
    });
    await openPage(page);
    await openSettingsPanel(page);
    const panel = page.getByRole("complementary", {
      name: "Gesprächseinstellungen",
    });
    await panel.getByRole("tab", { name: "Modelle" }).click();
    for (const level of ["Nur lokal", "EU", "Frei"])
      await expect(panel.getByRole("radio", { name: level })).toBeVisible();
    await expect(panel.getByRole("radio", { name: "Nur lokal" })).toBeChecked();
    const tools = panel.getByRole("switch", { name: "LVA-Werkzeuge nutzen" });
    await expect(tools).toBeChecked();
    await expect(panel.getByText("Aus", { exact: true })).toBeVisible();
    // Der Denker-Hinweis ist mit dem Feld verbunden.
    const denker = panel.getByRole("combobox", { name: "Denker-Modell" });
    const describedBy = await denker.getAttribute("aria-describedby");
    expect(describedBy).toBeTruthy();
    await expect(page.locator(`[id="${describedBy}"]`)).toContainText(
      "zweites, stärkeres Modell",
    );
    await panel.getByRole("radio", { name: "EU" }).click();
    await tools.click();
    const saved = await page.evaluate(() =>
      JSON.parse(localStorage.getItem("voice-call.config") ?? "{}"),
    );
    expect(saved.orchestration).toEqual({
      denkerModelId: "",
      souveraenitaet: "eu",
      useTools: false,
    });
  });
});

const SEED = {
  title: "Grundsteuer klären",
  entries: [
    { role: "user", mode: "spoken", text: "Ich zahle die Grund Steuer morgen" },
    { role: "assistant", mode: "typed", text: "Verstanden, bis morgen." },
    { role: "user", mode: "typed", text: "Getippte Nachricht" },
  ],
};

const PAIRS = [
  {
    id: 1,
    bereich: "standard",
    gehoert: "Grund Steuer",
    korrigiert: "Grundsteuer",
    anzahl: 3,
    zuletzt: "2026-11-01T09:00:00.000Z",
    status: "vorschlag",
  },
  {
    id: 2,
    bereich: "standard",
    gehoert: "Wiederspruch",
    korrigiert: "Widerspruch",
    anzahl: 2,
    zuletzt: "2026-11-01T08:00:00.000Z",
    status: "uebernommen",
  },
  {
    id: 3,
    bereich: "standard",
    gehoert: "Gemeinde Rat",
    korrigiert: "Gemeinderat",
    anzahl: 1,
    zuletzt: "2026-10-31T08:00:00.000Z",
    status: "beobachtet",
  },
];

/** Öffnet das gespeicherte Gespräch aus der Attrappe. */
async function openSeed(page: Page) {
  await openPage(page);
  await historyItem(page, SEED.title).click();
  await expect(
    page.getByRole("log").getByText("Ich zahle die Grund Steuer morgen"),
  ).toBeVisible();
}

const editButton = (page: Page) =>
  page.getByRole("button", { name: "Erkannten Text bearbeiten" });

/** Wartet, bis laufende Übergänge fertig sind (z. B. der Reiter-Indikator). */
async function settled(page: Page) {
  await page.waitForFunction(() =>
    document.getAnimations().every((a) => a.playState !== "running"),
  );
}

/** Gesprächseinstellungen, Reiter Erkennung. */
async function openRecognition(page: Page) {
  await openSettingsPanel(page);
  const panel = page.getByRole("complementary", {
    name: "Gesprächseinstellungen",
  });
  const tab = panel.getByRole("tab", { name: "Erkennung" });
  await tab.click();
  await expect(tab).toHaveAttribute("aria-selected", "true");
  await settled(page);
  return panel;
}

/** Ein Mikrofon aus einem Ton, damit die Aufnahme der Einrichtung Daten bekommt. */
async function fakeMicrophone(page: Page) {
  await page.addInitScript(() => {
    navigator.mediaDevices.getUserMedia = async () => {
      const c = new AudioContext();
      const o = c.createOscillator();
      const d = c.createMediaStreamDestination();
      o.connect(d);
      o.start();
      return d.stream;
    };
  });
}

test.describe("Sprachgespraech Stufe 3: Lernen aus Korrekturen", () => {
  test("Zeile bearbeiten: Enter speichert den Beitrag, Feedback nur bei eingeschaltetem Lernen", async ({
    page,
  }) => {
    await install(page, "light", {
      seed: SEED,
      settings: { speech_learning_enabled: true },
    });
    await openSeed(page);
    // Nur die eigene erkannte Zeile hat den Stift (nicht die Antwort, nicht getippter Text).
    await expect(editButton(page)).toHaveCount(1);
    await editButton(page).click();
    const field = page.getByRole("textbox", {
      name: "Erkannten Text bearbeiten",
    });
    await expect(field).toBeFocused();
    await expect(field).toHaveValue("Ich zahle die Grund Steuer morgen");
    await field.fill("Ich zahle die Grundsteuer morgen");
    await field.press("Enter");
    await expect(field).toHaveCount(0);
    await expect(
      page.getByRole("log").getByText("Ich zahle die Grundsteuer morgen"),
    ).toBeVisible();
    await expect
      .poll(async () => (await calls(page, "conversation_update_entry")).length)
      .toBe(1);
    const [update] = await calls(page, "conversation_update_entry");
    expect(update.args).toMatchObject({
      text: "Ich zahle die Grundsteuer morgen",
      status: "complete",
    });
    await expect
      .poll(async () => (await calls(page, "speech_learning_feedback")).length)
      .toBe(1);
    const [feedback] = await calls(page, "speech_learning_feedback");
    expect(feedback.args).toEqual({
      bereich: "standard",
      erkannt: "Ich zahle die Grund Steuer morgen",
      korrigiert: "Ich zahle die Grundsteuer morgen",
    });
  });

  test("Zweite Bearbeitung derselben Zeile meldet weiter den erkannten Originaltext", async ({
    page,
  }) => {
    await install(page, "light", {
      seed: SEED,
      settings: { speech_learning_enabled: true },
    });
    await openSeed(page);
    const field = page.getByRole("textbox", {
      name: "Erkannten Text bearbeiten",
    });
    await editButton(page).click();
    await field.fill("Ich zahle die Grundsteuer morgen");
    await field.press("Enter");
    await expect(field).toHaveCount(0);
    await editButton(page).click();
    await field.fill("Ich zahle die Grundsteuer heute");
    await field.press("Enter");
    await expect(
      page.getByRole("log").getByText("Ich zahle die Grundsteuer heute"),
    ).toBeVisible();
    await expect
      .poll(async () => (await calls(page, "conversation_update_entry")).length)
      .toBe(2);
    for (const c of await calls(page, "speech_learning_feedback")) {
      expect(c.args.erkannt).toBe("Ich zahle die Grund Steuer morgen");
    }
  });

  test("Zeile bearbeiten ohne Lernen: der Beitrag wird gespeichert, es geht kein Feedback raus", async ({
    page,
  }) => {
    await install(page, "light", { seed: SEED });
    await openSeed(page);
    await editButton(page).click();
    const field = page.getByRole("textbox", {
      name: "Erkannten Text bearbeiten",
    });
    await field.fill("Ich zahle die Grundsteuer morgen");
    await field.press("Enter");
    await expect
      .poll(async () => (await calls(page, "conversation_update_entry")).length)
      .toBe(1);
    await page.waitForTimeout(300);
    expect(await calls(page, "speech_learning_feedback")).toHaveLength(0);
  });

  test("Esc bricht ab, unveränderter oder leerer Text speichert nichts", async ({
    page,
  }) => {
    await install(page, "light", {
      seed: SEED,
      settings: { speech_learning_enabled: true },
    });
    await openSeed(page);
    await editButton(page).click();
    let field = page.getByRole("textbox", {
      name: "Erkannten Text bearbeiten",
    });
    await field.fill("etwas ganz anderes");
    await field.press("Escape");
    await expect(field).toHaveCount(0);
    await expect(
      page.getByRole("log").getByText("Ich zahle die Grund Steuer morgen"),
    ).toBeVisible();
    // Unverändert bestätigen und leeren Text bestätigen.
    await editButton(page).click();
    field = page.getByRole("textbox", { name: "Erkannten Text bearbeiten" });
    await field.press("Enter");
    await editButton(page).click();
    field = page.getByRole("textbox", { name: "Erkannten Text bearbeiten" });
    await field.fill("   ");
    await field.press("Enter");
    await page.waitForTimeout(300);
    expect(await calls(page, "conversation_update_entry")).toHaveLength(0);
    expect(await calls(page, "speech_learning_feedback")).toHaveLength(0);
    await expect(
      page.getByRole("log").getByText("Ich zahle die Grund Steuer morgen"),
    ).toBeVisible();
  });

  test("Einstellungen Erkennung: Schalter standardmäßig aus, Liste mit Übernehmen, Ablehnen, Löschen", async ({
    page,
  }) => {
    await install(page, "light", { pairs: PAIRS });
    await openPage(page);
    const panel = await openRecognition(page);
    const learn = panel.getByRole("switch", { name: "Aus Korrekturen lernen" });
    const auto = panel.getByRole("switch", {
      name: "Vorschläge automatisch übernehmen",
    });
    await expect(learn).not.toBeChecked();
    await expect(auto).not.toBeChecked();
    await expect(auto).toBeDisabled();
    await expect(panel.getByText("Datenschutz", { exact: true })).toBeVisible();
    await learn.click();
    await expect
      .poll(async () =>
        (await calls(page, "change_speech_learning_enabled_setting")).map(
          (c) => c.args.enabled,
        ),
      )
      .toEqual([true]);
    await expect(auto).toBeEnabled();
    await auto.click();
    await expect
      .poll(async () =>
        (await calls(page, "change_speech_learning_auto_apply_setting")).map(
          (c) => c.args.enabled,
        ),
      )
      .toEqual([true]);

    const rows = panel.getByTestId("learned-pair");
    await expect(rows).toHaveCount(3);
    await expect(rows.nth(0)).toContainText("Grund Steuer");
    await expect(rows.nth(0)).toContainText("Grundsteuer");
    await expect(rows.nth(0)).toContainText("3×");
    await expect(rows.nth(0)).toContainText("Vorschlag");
    await expect(rows.nth(1)).toContainText("Übernommen");
    await expect(rows.nth(2)).toContainText("Beobachtet");

    await panel
      .getByRole("button", { name: "„Grund Steuer → Grundsteuer“ übernehmen" })
      .click();
    await expect(rows.nth(0)).toContainText("Übernommen");
    expect((await calls(page, "speech_learning_set_status"))[0].args).toEqual({
      id: 1,
      status: "uebernommen",
    });
    await panel
      .getByRole("button", { name: "„Gemeinde Rat → Gemeinderat“ ablehnen" })
      .click();
    await expect(rows.nth(2)).toContainText("Abgelehnt");
    await panel
      .getByRole("button", { name: "„Wiederspruch → Widerspruch“ löschen" })
      .click();
    await expect(rows).toHaveCount(2);
    expect((await calls(page, "speech_learning_delete"))[0].args).toEqual({
      id: 2,
    });
  });

  test("Alles löschen verlangt eine Bestätigung", async ({ page }) => {
    await install(page, "light", { pairs: PAIRS });
    await openPage(page);
    const panel = await openRecognition(page);
    await expect(panel.getByTestId("learned-pair")).toHaveCount(3);
    await panel.getByRole("button", { name: "Alles löschen" }).click();
    const dialog = page.getByRole("dialog", { name: "Alles löschen?" });
    await expect(dialog).toBeVisible();
    await dialog.getByRole("button", { name: "Abbrechen" }).last().click();
    await expect(dialog).toHaveCount(0);
    expect(await calls(page, "speech_learning_clear")).toHaveLength(0);
    await panel.getByRole("button", { name: "Alles löschen" }).click();
    await page
      .getByRole("dialog", { name: "Alles löschen?" })
      .getByRole("button", { name: "Endgültig löschen" })
      .click();
    await expect(panel.getByText("Noch nichts gelernt.")).toBeVisible();
    expect(await calls(page, "speech_learning_clear")).toHaveLength(1);
    expect((await calls(page, "speech_learning_clear"))[0].args).toEqual({
      bereich: "standard",
    });
  });

  test("Einrichtung durch Vorlesen: Text, Aufnahme, Auswertung, Vorschläge", async ({
    page,
  }) => {
    await fakeMicrophone(page);
    await install(page, "light");
    await page.addInitScript(() => {
      const w = window as any;
      w.__transcript = "Der Bürgermeister bat die Verwaltung";
      w.__setupOutcome = {
        reason: "ok",
        matched_percent: 91,
        pairs: [
          {
            id: 11,
            bereich: "standard",
            gehoert: "Grund Steuer",
            korrigiert: "Grundsteuer",
            anzahl: 2,
            zuletzt: "2026-11-01T09:00:00.000Z",
            status: "vorschlag",
          },
          {
            id: 12,
            bereich: "standard",
            gehoert: "Amtsplatt",
            korrigiert: "Amtsblatt",
            anzahl: 2,
            zuletzt: "2026-11-01T09:00:00.000Z",
            status: "vorschlag",
          },
        ],
      };
    });
    await openPage(page);
    const panel = await openRecognition(page);
    await panel.getByRole("button", { name: "Erkennung einrichten" }).click();
    const dialog = page.getByRole("dialog", { name: "Erkennung einrichten" });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByTestId("setup-text")).toContainText(
      "Das Ordnungsamt versendet den Gebührenbescheid zur Grundsteuer.",
    );
    expect(await calls(page, "speech_learning_setup_prepare")).toHaveLength(1);
    await dialog.getByRole("button", { name: "Aufnahme starten" }).click();
    await expect(dialog.getByText(/Aufnahme läuft/)).toBeVisible();
    await page.waitForTimeout(1500);
    await dialog.getByRole("button", { name: "Fertig" }).click();
    await expect(dialog.getByText(/91 % der Wörter/)).toBeVisible();
    const [transcribe] = await calls(page, "voice_call_transcribe");
    expect(transcribe.args.pcm16B64.length).toBeGreaterThan(1000);
    const [evaluate] = await calls(page, "speech_learning_setup_evaluate");
    expect(evaluate.args.bereich).toBe("standard");
    expect(evaluate.args.transcript).toBe(
      "Der Bürgermeister bat die Verwaltung",
    );
    expect(evaluate.args.reference).toContain(
      "Gebührenbescheid zur Grundsteuer",
    );
    const rows = dialog.getByTestId("learned-pair");
    await expect(rows).toHaveCount(2);
    await dialog
      .getByRole("button", { name: "„Amtsplatt → Amtsblatt“ übernehmen" })
      .click();
    await dialog
      .getByRole("button", { name: "„Grund Steuer → Grundsteuer“ ablehnen" })
      .click();
    await expect(rows.nth(0)).toContainText("Abgelehnt");
    await expect(rows.nth(1)).toContainText("Übernommen");
    const set = await calls(page, "speech_learning_set_status");
    expect(set.map((c) => c.args)).toEqual([
      { id: 12, status: "uebernommen" },
      { id: 11, status: "abgelehnt" },
    ]);
    await dialog.getByRole("button", { name: "Schließen" }).last().click();
    await expect(dialog).toHaveCount(0);
    // Die Liste im Reiter zeigt die Ergebnisse.
    await expect(panel.getByTestId("learned-pair")).toHaveCount(2);
  });

  test("Einrichtung: passt das Vorgelesene nicht, kommt ein Hinweis statt Vorschlägen", async ({
    page,
  }) => {
    await fakeMicrophone(page);
    await install(page, "light");
    await page.addInitScript(() => {
      (window as any).__setupOutcome = {
        reason: "rewrite",
        matched_percent: 12,
        pairs: [],
      };
    });
    await openPage(page);
    const panel = await openRecognition(page);
    await panel.getByRole("button", { name: "Erkennung einrichten" }).click();
    const dialog = page.getByRole("dialog", { name: "Erkennung einrichten" });
    await dialog.getByRole("button", { name: "Aufnahme starten" }).click();
    await page.waitForTimeout(1500);
    await dialog.getByRole("button", { name: "Fertig" }).click();
    await expect(dialog.getByText(/passt nicht zum Text \(12 %/)).toBeVisible();
    await expect(dialog.getByTestId("learned-pair")).toHaveCount(0);
    await dialog.getByRole("button", { name: "Noch einmal" }).click();
    await expect(
      dialog.getByRole("button", { name: "Aufnahme starten" }),
    ).toBeVisible();
  });

  test("Einrichtung: doppelter Start öffnet nur ein Mikrofon, Schließen gibt es frei", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      const w = window as any;
      w.__mic = { opened: 0, stopped: 0 };
      navigator.mediaDevices.getUserMedia = async () => {
        w.__mic.opened += 1;
        await new Promise((r) => setTimeout(r, 200));
        const c = new AudioContext();
        const o = c.createOscillator();
        const d = c.createMediaStreamDestination();
        o.connect(d);
        o.start();
        for (const t of d.stream.getTracks()) {
          const stop = t.stop.bind(t);
          t.stop = () => {
            w.__mic.stopped += 1;
            stop();
          };
        }
        return d.stream;
      };
    });
    await install(page, "light");
    await openPage(page);
    const panel = await openRecognition(page);
    await panel.getByRole("button", { name: "Erkennung einrichten" }).click();
    const dialog = page.getByRole("dialog", { name: "Erkennung einrichten" });
    const start = dialog.getByRole("button", { name: "Aufnahme starten" });
    await expect(start).toBeVisible();
    await start.dblclick();
    await expect(dialog.getByText(/Aufnahme läuft/)).toBeVisible();
    const mic = () => page.evaluate(() => (window as any).__mic);
    expect((await mic()).opened).toBe(1);
    await dialog.getByRole("button", { name: "Abbrechen" }).click();
    await expect.poll(async () => (await mic()).stopped).toBeGreaterThan(0);
  });

  test("Einrichtung: Abbrechen während der Aufnahme wertet nichts aus", async ({
    page,
  }) => {
    await fakeMicrophone(page);
    await install(page, "light");
    await openPage(page);
    const panel = await openRecognition(page);
    await panel.getByRole("button", { name: "Erkennung einrichten" }).click();
    const dialog = page.getByRole("dialog", { name: "Erkennung einrichten" });
    await dialog.getByRole("button", { name: "Aufnahme starten" }).click();
    await expect(dialog.getByText(/Aufnahme läuft/)).toBeVisible();
    await dialog.getByRole("button", { name: "Abbrechen" }).click();
    await expect(dialog).toHaveCount(0);
    await page.waitForTimeout(300);
    expect(await calls(page, "voice_call_transcribe")).toHaveLength(0);
    expect(await calls(page, "speech_learning_setup_evaluate")).toHaveLength(0);
  });
});

/** Gesprächseinstellungen, Reiter Wissen. */
async function openKnowledge(page: Page) {
  const panel = page.getByRole("complementary", {
    name: "Gesprächseinstellungen",
  });
  if (!(await panel.isVisible())) await openSettingsPanel(page);
  const tab = panel.getByRole("tab", { name: "Wissen" });
  await tab.click();
  await expect(tab).toHaveAttribute("aria-selected", "true");
  await settled(page);
  return panel;
}

const AREA_SWITCHES = [
  "Besprechungen",
  "Transkripte",
  "Diktate",
  "Frühere Gespräche",
  "Termine",
  "Vorlesen",
];

const knowledgeCalls = async (page: Page) =>
  (await calls(page, "change_voice_call_knowledge_setting")).map(
    (c) => c.args.areas as string[],
  );

const DICTATIONS_TOOL = {
  name: "dictations_search",
  title: "Diktate durchsuchen",
  description: "Sucht in den letzten Diktaten.",
  parameters_json: JSON.stringify({
    type: "object",
    properties: { query: { type: "string" } },
    required: ["query"],
    additionalProperties: false,
  }),
  capability: "knowledge.read",
};

test.describe("Sprachgespraech Wissen: Bereiche für das Modell", () => {
  test("Reiter Wissen: alle Schalter aus, Transkripte erst mit Besprechungen, Wahl geht an den Kern", async ({
    page,
  }) => {
    await install(page, "light", { settings: LOCAL_SPEAKER });
    await openPage(page);
    const panel = await openKnowledge(page);
    await expect(
      panel.getByText(
        "Was das Modell im Gespräch nachschlagen darf. Jeder Zugriff wird protokolliert.",
      ),
    ).toBeVisible();
    await expect(panel.getByRole("switch")).toHaveCount(AREA_SWITCHES.length);
    for (const name of AREA_SWITCHES) {
      const sw = panel.getByRole("switch", { name, exact: true });
      await expect(sw).toBeVisible();
      await expect(sw).not.toBeChecked();
    }
    const meetings = panel.getByRole("switch", { name: "Besprechungen" });
    const transcripts = panel.getByRole("switch", { name: "Transkripte" });
    await expect(transcripts).toBeDisabled();
    // Jede Beschreibung hängt am Schalter.
    for (const name of AREA_SWITCHES) {
      const sw = panel.getByRole("switch", { name, exact: true });
      const id = await sw.getAttribute("aria-describedby");
      expect(id, name).toBeTruthy();
      await expect(page.locator(`[id="${id}"]`)).not.toBeEmpty();
    }

    await meetings.click();
    await panel.getByRole("switch", { name: "Termine" }).click();
    await expect
      .poll(() => knowledgeCalls(page))
      .toEqual([["meetings"], ["meetings", "calendar"]]);
    await expect(transcripts).toBeEnabled();
    await transcripts.click();
    await expect
      .poll(async () =>
        (await calls(page, "change_voice_call_read_transcripts_setting")).map(
          (c) => c.args.enabled,
        ),
      )
      .toEqual([true]);
    await expect(transcripts).toBeChecked();
    // Besprechungen aus: Transkripte sind wieder gesperrt und zeigen sich aus.
    await meetings.click();
    await expect(transcripts).toBeDisabled();
    await expect(transcripts).not.toBeChecked();
    expect((await knowledgeCalls(page)).at(-1)).toEqual(["calendar"]);
  });

  test("Berechtigungs-Chip zeigt „kein Wissen“; „Alle Einstellungen …“ öffnet den Reiter Wissen", async ({
    page,
  }) => {
    await install(page, "light", { settings: LOCAL_SPEAKER });
    await openPage(page);
    const chip = permissionChip(page);
    await expect(chip).toHaveText("Frei · kein Wissen");
    await expect(page.getByTestId("knowledge-chip")).toHaveCount(0);
    await chip.click();
    await page.getByRole("menuitem", { name: "Alle Einstellungen …" }).click();
    const panel = page.getByRole("complementary", {
      name: "Gesprächseinstellungen",
    });
    await expect(panel).toBeVisible();
    await expect(panel.getByRole("tab", { name: "Wissen" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await panel.getByRole("switch", { name: "Diktate" }).click();
    await expect(chip).toHaveText("Frei · 1 Bereich");
    await panel.getByRole("switch", { name: "Diktate" }).click();
    await expect(chip).toHaveText("Frei · kein Wissen");
  });

  test("Mit einem freigegebenen Bereich bietet voice_tools_list Werkzeuge an", async ({
    page,
  }) => {
    await install(page, "light", {
      settings: { ...LOCAL_SPEAKER, voice_call_knowledge: [] },
      config: orchestration({}),
      tools: [MEETINGS_TOOL],
    });
    await openPage(page);
    // Standard: nichts frei, der Kern liefert keine Werkzeuge, es gilt der alte Weg.
    await exchange(page, "Hallo", "Hallo zurück.");
    expect(
      await page.evaluate(() => (window as any).__listed as number[]),
    ).toEqual([0]);
    expect(await calls(page, "voice_call_turn")).toHaveLength(0);

    // Bereich freigeben: der nächste Schritt lädt die Werkzeuge neu und bietet sie an.
    const panel = await openKnowledge(page);
    await panel.getByRole("switch", { name: "Besprechungen" }).click();
    await expect.poll(() => knowledgeCalls(page)).toEqual([["meetings"]]);
    await replies(page, [
      "\u27e6werkzeug meetings_list {}\u27e7",
      "Zwei Besprechungen.",
    ]);
    await send(page, "Welche Besprechungen gab es?");
    await expect
      .poll(() => page.evaluate(() => (window as any).__tool !== null))
      .toBe(true);
    expect(
      await page.evaluate(() => (window as any).__listed as number[]),
    ).toEqual([0, 1]);
    await page.evaluate(() => (window as any).__tool.resolve("{}"));
    await expect(
      page.getByRole("log").getByText("Zwei Besprechungen."),
    ).toBeVisible();
    const tool = await calls(page, "voice_tool_call");
    expect(tool.map((c) => c.args.name)).toEqual(["meetings_list"]);
    // Das laufende Gespräch geht mit, damit es den Gesprächswerkzeugen verborgen bleibt.
    expect(typeof tool[0].args.conversationId).toBe("string");
  });

  test("Der Aufruf nennt sich lesbar statt mit dem Werkzeugnamen", async ({
    page,
  }) => {
    await install(page, "light", {
      settings: { ...LOCAL_SPEAKER, voice_call_knowledge: ["dictations"] },
      config: orchestration({}),
      tools: [DICTATIONS_TOOL],
    });
    await openPage(page);
    await replies(page, [
      '\u27e6werkzeug dictations_search {"query":"Grundsteuer"}\u27e7',
      "Du hast gestern darüber diktiert.",
    ]);
    await send(page, "Habe ich etwas zur Grundsteuer diktiert?");
    await expect
      .poll(() => page.evaluate(() => (window as any).__tool !== null))
      .toBe(true);
    const status = page.getByTestId("call-status");
    await expect(status).toHaveText("Durchsuche die Diktate …");
    await expect(status).not.toContainText("dictations_search");
    await page.evaluate(() => (window as any).__tool.resolve("{}"));
    await expect(
      page.getByRole("log").getByText("Du hast gestern darüber diktiert."),
    ).toBeVisible();
  });

  test("System-Hinweis: fehlende Bereiche werden genannt, nichts Fehlendes bleibt unerwähnt", async ({
    page,
  }) => {
    const prompt = async () => {
      const chat = await page.evaluate(
        () => (window as any).__chat.args.messages as { content: string }[],
      );
      return chat[0].content;
    };
    await install(page, "light", { settings: LOCAL_SPEAKER });
    await openPage(page);
    await exchange(page, "Hallo", "Hallo zurück.");
    const none = await prompt();
    expect(none).toContain("Gesprächseinstellungen → Wissen");
    expect(none).toContain("nicht freigegeben");
    // Transkripte hängen an den Besprechungen und werden erst genannt, wenn diese frei sind.
    expect(none).toContain(
      "Besprechungen, Diktate, frühere Gespräche, Termine und Vorlesen-Seiten",
    );
    expect(none).not.toContain("Transkripte");
    expect(none).toContain("Behaupte nie, die App habe solche Daten nicht");
    // Die Anweisung des Nutzers bleibt vorn.
    expect(none.startsWith("Du bist ein Gesprächspartner am Telefon.")).toBe(
      true,
    );
  });

  test("System-Hinweis nennt nur, was fehlt; alles frei: kein Hinweis", async ({
    page,
  }) => {
    await install(page, "light", {
      settings: {
        ...LOCAL_SPEAKER,
        voice_call_knowledge: ["meetings", "calendar"],
      },
    });
    await openPage(page);
    await exchange(page, "Hallo", "Hallo zurück.");
    const chat = () =>
      page.evaluate(
        () => (window as any).__chat.args.messages as { content: string }[],
      );
    const partial = (await chat())[0].content;
    expect(partial).toContain(
      "nicht freigegeben: Transkripte, Diktate, frühere Gespräche und Vorlesen-Seiten.",
    );
    expect(partial).not.toMatch(/freigegeben:[^.]*Termine/);
    // Alles frei, auch Transkripte: kein Hinweis mehr (nach dem Umschalten im Reiter).
    const panel = await openKnowledge(page);
    for (const name of ["Diktate", "Frühere Gespräche", "Vorlesen"])
      await panel.getByRole("switch", { name, exact: true }).click();
    await panel.getByRole("switch", { name: "Transkripte" }).click();
    await expect
      .poll(() =>
        page.evaluate(
          () => (window as any).__settings.voice_call_read_transcripts,
        ),
      )
      .toBe(true);
    await exchange(page, "Und jetzt?", "Jetzt alles.");
    const full = (await chat())[0].content;
    expect(full).not.toContain("Gesprächseinstellungen → Wissen");
    expect(full).not.toContain("nicht freigegeben");
  });
});

test.describe("Screenshots @screenshots", () => {
  const conversation = async (page: Page) => {
    await openPage(page);
    await exchange(page, "Hallo KI", "Hallo zurück.");
    await exchange(
      page,
      "Fasse bitte das gestrige Meeting kurz zusammen.",
      "Gern. Es ging um Budget und Termine.",
    );
  };

  test("hell und dunkel, Menues, Einstellungen, schmal", async ({
    browser,
  }) => {
    test.skip(!SHOTS, "nur fuer Abnahmebilder (SCREENS_DIR)");
    mkdirSync(SHOTS, { recursive: true });
    const shoot = async (
      theme: "light" | "dark",
      size: { width: number; height: number },
      steps: (page: Page, snap: (n: string) => Promise<void>) => Promise<void>,
    ) => {
      const ctx = await browser.newContext({
        viewport: size,
        baseURL: test.info().project.use.baseURL,
      });
      const page = await ctx.newPage();
      await install(page, theme);
      await conversation(page);
      await steps(page, async (n) => {
        await page.screenshot({ path: `${SHOTS}/${n}.png` });
      });
      await ctx.close();
    };
    const big = { width: 1440, height: 900 };
    await shoot("light", big, async (page, snap) => {
      await snap("hell-1440");
      await page
        .locator(".workspace-status")
        .getByRole("button", { name: /Gespräch/ })
        .click();
      await snap("fussmenue-1440");
      await page.keyboard.press("Escape");
      await page.getByRole("button", { name: "Hinzufügen" }).click();
      await snap("plus-1440");
      await page.keyboard.press("Escape");
      await page.mouse.click(700, 300);
      await openSettingsPanel(page);
      await snap("einstellungen-1440");
    });
    await shoot("dark", big, async (_p, snap) => snap("dunkel-1440"));
    await shoot("light", { width: 390, height: 844 }, async (_p, snap) =>
      snap("hell-390"),
    );

    // Verlauf (U3): mehrere Gespräche, eines angeheftet.
    const moreHistory = async (page: Page, narrow = false) => {
      const fresh = () =>
        narrow
          ? openOptions(page, "Neues Gespräch")
          : page
              .getByRole("button", { name: "Neues Gespräch", exact: true })
              .click();
      await fresh();
      await exchange(page, "Rezept für Linsensuppe", "Gern, hier ein Rezept.");
      await fresh();
      await exchange(
        page,
        "Erkläre mir bitte den Unterschied zwischen TCP und UDP ausführlich",
        "TCP ist verbindungsorientiert, UDP nicht.",
      );
      if (narrow) await historyToggle(page).click();
      await page
        .getByRole("button", { name: "Optionen für „Rezept für Linsensuppe“" })
        .click();
      await page.getByRole("menuitem", { name: "Anheften" }).click();
    };
    await shoot("light", big, async (page, snap) => {
      await moreHistory(page);
      await snap("verlauf-hell-1440");
    });
    await shoot("dark", big, async (page, snap) => {
      await moreHistory(page);
      await snap("verlauf-dunkel-1440");
    });
    await shoot("light", { width: 390, height: 844 }, async (page, snap) => {
      await moreHistory(page, true);
      await snap("verlauf-390");
    });
  });

  test("Stufe 2: Einstellungen Modelle und Werkzeug-Statuszeile", async ({
    browser,
  }) => {
    test.skip(!SHOTS, "nur fuer Abnahmebilder (SCREENS_DIR)");
    const dir = `${SHOTS}/stufe2/`;
    mkdirSync(dir, { recursive: true });
    {
      const ctx = await browser.newContext({
        viewport: { width: 1440, height: 900 },
        baseURL: test.info().project.use.baseURL,
      });
      const page = await ctx.newPage();
      await install(page, "light", { config: orchestration({}) });
      await openPage(page);
      await send(page, "Hallo KI");
      await expect(
        page.getByRole("alert").filter({ hasText: "kein Gespräch möglich" }),
      ).toBeVisible();
      await page.screenshot({ path: `${dir}sprecher-gesperrt-1440.png` });
      await ctx.close();
    }
    for (const [name, size] of [
      ["1440", { width: 1440, height: 900 }],
      ["390", { width: 390, height: 844 }],
    ] as const) {
      const ctx = await browser.newContext({
        viewport: size,
        baseURL: test.info().project.use.baseURL,
      });
      const page = await ctx.newPage();
      await install(page, "light", {
        settings: LOCAL_SPEAKER,
        config: orchestration({ denkerModelId: "cli:sonnet" }),
        tools: [MEETINGS_TOOL],
      });
      await openPage(page);
      if (name === "1440") {
        await replies(page, [
          "\u27e6werkzeug meetings_list {}\u27e7",
          "Heute gab es zwei Besprechungen.",
        ]);
        await send(page, "Welche Besprechungen hatte ich heute?");
        await expect(page.getByTestId("call-status")).toHaveText(
          "Schaue in Besprechungen nach …",
        );
        await page.screenshot({ path: `${dir}werkzeug-statuszeile-1440.png` });
      }
      await openSettingsPanel(page);
      await page
        .getByRole("complementary", { name: "Gesprächseinstellungen" })
        .getByRole("tab", { name: "Modelle" })
        .click();
      await page.screenshot({
        path: `${dir}einstellungen-modelle-${name}.png`,
      });
      await ctx.close();
    }
  });
  test("Wissen: Reiter und Chip", async ({ browser }) => {
    test.skip(!SHOTS, "nur fuer Abnahmebilder (SCREENS_DIR)");
    const dir = `${SHOTS}/`;
    mkdirSync(dir, { recursive: true });
    for (const [name, size] of [
      ["1440", { width: 1440, height: 900 }],
      ["390", { width: 390, height: 844 }],
    ] as const) {
      const ctx = await browser.newContext({
        viewport: size,
        baseURL: test.info().project.use.baseURL,
      });
      const page = await ctx.newPage();
      await install(page, "light", {
        settings: {
          voice_call_knowledge: ["meetings", "calendar"],
          voice_call_read_transcripts: true,
        },
      });
      await openPage(page);
      await settled(page);
      // Mit freigegebenem Bereich gibt es keinen Chip; der Reiter zeigt die Schalter.
      await expect(permissionChip(page)).toContainText("2 Bereiche");
      await openKnowledge(page);
      await page.screenshot({ path: `${dir}wissen-reiter-${name}.png` });
      await ctx.close();

      const bare = await browser.newContext({
        viewport: size,
        baseURL: test.info().project.use.baseURL,
      });
      const empty = await bare.newPage();
      await install(empty, "light");
      await openPage(empty);
      await settled(empty);
      await expect(permissionChip(empty)).toBeVisible();
      await empty.screenshot({ path: `${dir}wissen-chip-${name}.png` });
      await permissionChip(empty).click();
      await empty
        .getByRole("menuitem", { name: "Alle Einstellungen …" })
        .click();
      await settled(empty);
      await expect(
        empty
          .getByRole("complementary", { name: "Gesprächseinstellungen" })
          .getByRole("tab", { name: "Wissen" }),
      ).toHaveAttribute("aria-selected", "true");
      await empty.screenshot({ path: `${dir}wissen-chip-offen-${name}.png` });
      await bare.close();
    }
  });
  test("Stufe 3: Reiter Erkennung, Einrichtungsdialog, Zeilenbearbeitung", async ({
    browser,
  }) => {
    test.skip(!SHOTS, "nur fuer Abnahmebilder (SCREENS_DIR)");
    const dir = `${SHOTS}/stufe3/`;
    mkdirSync(dir, { recursive: true });
    for (const [name, size] of [
      ["1440", { width: 1440, height: 900 }],
      ["390", { width: 390, height: 844 }],
    ] as const) {
      const ctx = await browser.newContext({
        viewport: size,
        baseURL: test.info().project.use.baseURL,
      });
      const page = await ctx.newPage();
      await fakeMicrophone(page);
      await install(page, "light", {
        seed: SEED,
        pairs: PAIRS,
        settings: { speech_learning_enabled: true },
      });
      await openPage(page);
      if (name === "390") await historyToggle(page).click();
      await historyItem(page, SEED.title).click();
      await expect(
        page.getByRole("log").getByText("Ich zahle die Grund Steuer morgen"),
      ).toBeVisible();
      await settled(page);
      await editButton(page).first().scrollIntoViewIfNeeded();
      await expect(editButton(page).first()).toBeVisible();
      await page.screenshot({ path: `${dir}stift-${name}.png` });
      await editButton(page).click();
      await page
        .getByRole("textbox", { name: "Erkannten Text bearbeiten" })
        .fill("Ich zahle die Grundsteuer morgen");
      await page.screenshot({ path: `${dir}zeile-bearbeiten-${name}.png` });
      await page.keyboard.press("Escape");
      const panel = await openRecognition(page);
      await expect(panel.getByTestId("learned-pair")).toHaveCount(3);
      await page.screenshot({ path: `${dir}erkennung-${name}.png` });
      await panel
        .getByRole("heading", { name: "Gelernte Begriffe" })
        .scrollIntoViewIfNeeded();
      await panel
        .getByRole("list", { name: "Gelernte Begriffe" })
        .evaluate((el) => el.scrollIntoView({ block: "end" }));
      await page.screenshot({ path: `${dir}erkennung-liste-${name}.png` });
      await panel.getByRole("button", { name: "Erkennung einrichten" }).click();
      const dialog = page.getByRole("dialog", { name: "Erkennung einrichten" });
      await expect(dialog.getByTestId("setup-text")).toBeVisible();
      await page.screenshot({ path: `${dir}einrichtung-${name}.png` });
      await ctx.close();
    }
  });
});

const TWO_MODELS = {
  llm_models: [
    {
      id: "cli:sonnet",
      connection_id: "cli",
      remote_id: "sonnet",
      label: "Claude Sonnet",
      enabled: true,
      tags: [],
    },
    {
      id: "cli:opus",
      connection_id: "cli",
      remote_id: "opus",
      label: "Claude Opus",
      enabled: true,
      tags: [],
    },
  ],
};

test.describe("Sprachgespraech aufgeräumte Oberfläche", () => {
  test("Kein Modul-Rad in der Kopfleiste: Einstellungen nur über das Kontextmenü", async ({
    page,
  }) => {
    await install(page);
    await openPage(page);
    await expect(
      page.getByRole("button", { name: "Gesprächseinstellungen" }),
    ).toHaveCount(0);
    await expect(
      page.getByRole("button", { name: "Gesprächsverlauf" }),
    ).toHaveCount(1);
    await expect(page.getByTestId("call-button")).toHaveCount(1);
  });

  test("Kontextmenü „Gesprächsoptionen“ öffnet die Spalte und den Reiter Wissen", async ({
    page,
  }) => {
    await install(page);
    await openPage(page);
    const panel = page.getByRole("complementary", {
      name: "Gesprächseinstellungen",
    });
    await expect(panel).toHaveCount(0);
    await openOptions(page, "Gesprächseinstellungen");
    await expect(panel).toBeVisible();
    await panel
      .getByRole("button", { name: "Einstellungen schließen" })
      .click();
    await expect(panel).toHaveCount(0);
    await openOptions(page, "Wissen");
    await expect(panel.getByRole("tab", { name: "Wissen" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  test("Kontextmenü: „Neues Gespräch“ legt ein neues Gespräch an", async ({
    page,
  }) => {
    await install(page);
    await openPage(page);
    await exchange(page, "Hallo KI", "Hallo zurück.");
    await openOptions(page, "Neues Gespräch");
    await expect(emptyTitle(page)).toBeVisible();
    await expect(historyItem(page, "Hallo KI")).toBeVisible();
  });

  test("Verlauf-Symbol an der Spalte klappt sie ein; das Symbol in der Infozeile klappt sie aus", async ({
    page,
  }) => {
    await install(page);
    await openPage(page);
    await expect(history(page)).toBeVisible();
    // Eingeklappt: nichts mehr in der Kopfleiste, das Symbol sitzt links in der Infozeile.
    await historyToggle(page).click();
    await expect(history(page)).toHaveCount(0);
    const info = page.getByRole("button", {
      name: "Informationen zum Sprachgespräch",
    });
    const row = info.locator("xpath=..");
    await expect(row.locator(historyToggleSelector)).toBeVisible();
    expect(
      await page.evaluate(() =>
        localStorage.getItem("voice-call.history-open"),
      ),
    ).toBe("0");
    await historyToggle(page).click();
    await expect(history(page)).toBeVisible();
    await expect(history(page).locator(historyToggleSelector)).toBeVisible();
  });

  test("Gesprächsknopf im Chatfeld startet und beendet das Gespräch", async ({
    page,
  }) => {
    await install(page);
    await page.addInitScript(() => ((window as any).__startHang = true));
    await openPage(page);
    const button = page.getByTestId("call-button");
    await expect(button).toHaveAccessibleName("Gespräch beginnen");
    await button.click();
    expect(await calls(page, "voice_call_start")).toHaveLength(1);
    await expect(button).toHaveAccessibleName("Gespräch beenden");
    await button.click();
    await expect(button).toHaveAccessibleName("Gespräch beginnen");
    // Das Mikrofon bleibt als eigener Knopf daneben.
    await expect(
      page.getByRole("button", { name: "Mikrofon stummschalten" }),
    ).toBeVisible();
  });

  test("Berechtigungs-Chip ändert Stufe und Wissensbereiche", async ({
    page,
  }) => {
    await install(page);
    await openPage(page);
    const chip = permissionChip(page);
    await expect(chip).toHaveText("Frei · kein Wissen");
    await chip.click();
    await page.getByRole("menuitemradio", { name: "Nur lokal" }).click();
    await expect(chip).toHaveText("Nur lokal · kein Wissen");
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            JSON.parse(localStorage.getItem("voice-call.config") ?? "{}")
              .orchestration?.souveraenitaet,
        ),
      )
      .toBe("nur_lokal");
    await page.getByRole("menuitemcheckbox", { name: "Besprechungen" }).click();
    await page.getByRole("menuitemcheckbox", { name: "Termine" }).click();
    await expect(chip).toHaveText("Nur lokal · 2 Bereiche");
    expect(
      await page.evaluate(
        () => (window as any).__settings.voice_call_knowledge,
      ),
    ).toEqual(["meetings", "calendar"]);
    // Dieselbe Einstellung wie der Reiter Wissen.
    await page.keyboard.press("Escape");
    await openKnowledge(page);
    const panel = page.getByRole("complementary", {
      name: "Gesprächseinstellungen",
    });
    await expect(
      panel.getByRole("switch", { name: "Besprechungen" }),
    ).toBeChecked();
    await panel.getByRole("switch", { name: "Besprechungen" }).click();
    await expect(chip).toHaveText("Nur lokal · 1 Bereich");
    // Der alte Chip in der Infozeile ist weg.
    await expect(page.getByTestId("knowledge-chip")).toHaveCount(0);
  });

  test("Modellauswahl im Chatfeld schreibt die Funktion voice_speaker, nicht das globale Modell", async ({
    page,
  }) => {
    await install(page, "light", { settings: TWO_MODELS });
    await openPage(page);
    const picker = page.getByTestId("model-picker");
    const speaker = () =>
      page.evaluate(
        () => (window as any).__settings.function_models?.voice_speaker,
      );
    await expect(picker).toContainText("Claude Sonnet 5.5");
    await expect(picker).toContainText(/mittel/i);
    await picker.click();
    await page.getByRole("menuitemradio", { name: "Claude Opus 5.5" }).click();
    await expect(picker).toContainText("Claude Opus 5.5");
    await picker.click();
    await page.getByRole("menuitemradio", { name: "hoch" }).click();
    await expect(picker).toContainText(/hoch/i);
    await expect
      .poll(speaker)
      .toEqual({ model_id: "cli:opus", effort: "high" });
    const sets = await calls(page, "llm_set_function_model");
    expect(sets.at(-1)!.args).toMatchObject({
      forFunction: "voice_speaker",
      modelId: "cli:opus",
      effort: "high",
    });
    // Das globale Modell bleibt: kein Befehl, die Fußleiste zeigt weiter Sonnet.
    expect(await calls(page, "llm_set_active_model")).toHaveLength(0);
    expect(await calls(page, "llm_upsert_model")).toHaveLength(0);
    expect(
      await page.evaluate(() => (window as any).__settings.llm_active_model_id),
    ).toBe("cli:sonnet");
    // Die Wahl steht nur noch in den Einstellungen, nicht mehr in der Gesprächskonfiguration.
    const config = await page.evaluate(() =>
      JSON.parse(localStorage.getItem("voice-call.config") ?? "{}"),
    );
    expect(config.orchestration ?? {}).not.toHaveProperty("speakerModelId");
    // Der Aufruf nennt kein Modell mehr; Rust löst voice_speaker selbst auf.
    await message(page).fill("Hallo");
    await message(page).press("Enter");
    await expect
      .poll(() => page.evaluate(() => (window as any).__chat !== null))
      .toBe(true);
    const chat = (await calls(page, "voice_call_chat")).at(-1)!;
    expect(chat.args).not.toHaveProperty("modelId");
    expect(chat.args).not.toHaveProperty("effort");
  });

  test("Suche: Lupe öffnet das Overlay, Aktionen, letzte Gespräche, Treffer, Tastatur", async ({
    page,
  }) => {
    await install(page);
    await openPage(page);
    await exchange(page, "Hallo KI", "Hallo zurück.");
    await newChat(page).click();
    await exchange(page, "Zweite Frage", "Zweite Antwort.");
    await newChat(page).click();
    // Das Suchfeld oben in der Spalte gibt es nicht mehr.
    await expect(history(page).getByRole("searchbox")).toHaveCount(0);
    const search = await openSearch(page);
    await expect(
      search.getByPlaceholder("Gespräche durchsuchen oder Befehl ausführen"),
    ).toBeFocused();
    await expect(
      search.getByRole("group", { name: "Schnellaktionen" }),
    ).toBeVisible();
    await expect(
      search.getByRole("option", { name: /Neues Gespräch/ }),
    ).toBeVisible();
    await expect(
      search.getByRole("option", { name: "Gespräch beginnen" }),
    ).toBeVisible();
    await expect(
      search.getByRole("group", { name: "Letzte Gespräche" }),
    ).toBeVisible();
    // Filter-Tabs.
    await search.getByRole("tab", { name: "Aktionen" }).click();
    await expect(
      search.getByRole("option", { name: "Zweite Frage" }),
    ).toHaveCount(0);
    await search.getByRole("tab", { name: "Gespräche" }).click();
    await expect(
      search.getByRole("option", { name: "Zweite Frage" }),
    ).toBeVisible();
    await expect(
      search.getByRole("option", { name: /Neues Gespräch/ }),
    ).toHaveCount(0);
    await search.getByRole("tab", { name: "Alle" }).click();
    // Eingabe: Treffer; Pfeil und Enter öffnen das Gespräch.
    await search.getByRole("combobox").fill("hallo");
    await expect(
      search.getByRole("option", { name: "Hallo KI" }),
    ).toBeVisible();
    await expect(
      search.getByRole("option", { name: "Zweite Frage" }),
    ).toHaveCount(0);
    await search.getByRole("combobox").press("ArrowDown");
    await search.getByRole("combobox").press("ArrowUp");
    await search.getByRole("combobox").press("Enter");
    await expect(search).toHaveCount(0);
    await expect(
      page.getByRole("log").getByText("Hallo zurück."),
    ).toBeVisible();
  });

  test("Suche: Strg+Umschalt+K öffnet sie, Strg+K bleibt der App-Suche; Strg+N legt ein neues Gespräch an", async ({
    page,
  }) => {
    await install(page);
    await openPage(page);
    await exchange(page, "Hallo KI", "Hallo zurück.");
    await page.keyboard.press("Control+Shift+K");
    const search = page.getByTestId("history-search");
    await expect(search).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(search).toHaveCount(0);
    await page.keyboard.press("Control+k");
    await expect(search).toHaveCount(0);
    await expect(page.locator("[data-titlebar-search]")).toBeFocused();
    await message(page).click();
    await page.keyboard.press("Control+n");
    await expect(emptyTitle(page)).toBeVisible();
  });

  test("Suche: „Gespräch beginnen“ aus den Schnellaktionen startet das Gespräch", async ({
    page,
  }) => {
    await install(page);
    await page.addInitScript(() => ((window as any).__startHang = true));
    await openPage(page);
    const search = await openSearch(page);
    await search.getByRole("option", { name: "Gespräch beginnen" }).click();
    await expect(search).toHaveCount(0);
    expect(await calls(page, "voice_call_start")).toHaveLength(1);
  });

  test("Anheften: Eintrag steht oben unter „Angeheftet“, Lösen holt ihn zurück", async ({
    page,
  }) => {
    await install(page);
    await openPage(page);
    await exchange(page, "Hallo KI", "Hallo zurück.");
    await newChat(page).click();
    await exchange(page, "Zweite Frage", "Zweite Antwort.");
    const items = history(page).getByRole("listitem");
    await expect(items.first()).toContainText("Zweite Frage");
    await page.getByRole("button", { name: "Optionen für „Hallo KI“" }).click();
    await page.getByRole("menuitem", { name: "Anheften" }).click();
    await expect
      .poll(async () => (await calls(page, "conversation_set_pinned")).length)
      .toBe(1);
    expect(
      (await calls(page, "conversation_set_pinned"))[0].args,
    ).toMatchObject({
      pinned: true,
    });
    const pinned = history(page).getByRole("group", { name: "Angeheftet" });
    await expect(pinned).toBeVisible();
    await expect(pinned.getByRole("listitem")).toHaveCount(1);
    await expect(items.first()).toContainText("Hallo KI");
    await page.getByRole("button", { name: "Optionen für „Hallo KI“" }).click();
    await page.getByRole("menuitem", { name: "Lösen" }).click();
    await expect(
      history(page).getByRole("group", { name: "Angeheftet" }),
    ).toHaveCount(0);
    await expect(items.first()).toContainText("Zweite Frage");
    expect(
      (await calls(page, "conversation_set_pinned"))[1].args,
    ).toMatchObject({
      pinned: false,
    });
  });
});

test.describe("Hauptgespräch per Tastenkürzel", () => {
  const overlayStates = async (page: Page) =>
    (await calls(page, "voice_call_overlay")).map((c) => c.args.state);

  test("Kürzel startet das Hauptgespräch auch auf einer anderen Seite und beendet es wieder", async ({
    page,
  }) => {
    await fakeMicrophone(page);
    await install(page, "light");
    await page.goto("/");
    await goTo(page, "Verlauf");
    await page.waitForFunction(
      () => (window as any).__listening("voice-call-hotkey") > 0,
    );
    const phone = page.locator(".workspace-status [data-call-button]");
    await expect(phone).toHaveAttribute("data-state", "idle");

    await page.evaluate(() =>
      (window as any).__emit("voice-call-hotkey", null),
    );
    await expect(phone).toHaveAttribute("data-state", "active");
    expect(await calls(page, "voice_call_start")).toHaveLength(1);
    // Status wie beim Diktat im Overlay; nichts wird eingefügt.
    await expect.poll(() => overlayStates(page)).toContain("call-listening");
    expect(await calls(page, "paste_text")).toHaveLength(0);

    await page.evaluate(() =>
      (window as any).__emit("voice-call-hotkey", null),
    );
    await expect(phone).toHaveAttribute("data-state", "idle");
    await expect
      .poll(async () => (await overlayStates(page)).at(-1))
      .toBeNull();
  });

  test("Verlassen der Seite beendet das Hauptgespräch nicht, Klick aufs Telefon schon", async ({
    page,
  }) => {
    await fakeMicrophone(page);
    await install(page, "light");
    await openPage(page);
    const phone = page.locator(".workspace-status [data-call-button]");
    await phone.click();
    await expect(phone).toHaveAttribute("data-state", "active");
    await goTo(page, "Verlauf");
    await expect(phone).toHaveAttribute("data-state", "active");
    await phone.click();
    await expect(phone).toHaveAttribute("data-state", "idle");
  });

  test("Kürzel spricht ins Hauptgespräch, nicht ins offene andere Gespräch", async ({
    page,
  }) => {
    await fakeMicrophone(page);
    await install(page, "light", {
      seed: {
        title: "Anderes Thema",
        entries: [
          { role: "user", mode: "typed", text: "Etwas anderes" },
          { role: "assistant", mode: "typed", text: "Gern." },
        ],
      },
    });
    // Das Hauptgespräch ist ein anderes, schon gespeichertes Gespräch.
    const mainId = "conv_seed000000000000000000";
    await page.addInitScript((id) => {
      localStorage.setItem("voice-call.main", id);
    }, mainId);
    await openPage(page);
    await newChat(page).click();
    await page.evaluate(() =>
      (window as any).__emit("voice-call-hotkey", null),
    );
    const phone = page.locator(".workspace-status [data-call-button]");
    await expect(phone).toHaveAttribute("data-state", "active");
    // Das Hauptgespräch ist jetzt offen (sein Verlauf steht im Chat).
    await expect(
      page.getByRole("log").getByText("Etwas anderes"),
    ).toBeVisible();
    expect(
      (await calls(page, "conversation_get")).some((c) => c.args.id === mainId),
    ).toBe(true);
  });
});

test.describe("Hauptgespräch Abnahmebilder @screenshots", () => {
  test("Fußleiste und Overlay, hell und dunkel", async ({ browser }) => {
    test.skip(!SHOTS, "nur fuer Abnahmebilder (SCREENS_DIR)");
    mkdirSync(SHOTS, { recursive: true });
    for (const theme of ["light", "dark"] as const) {
      const ctx = await browser.newContext({
        viewport: { width: 1280, height: 760 },
        baseURL: test.info().project.use.baseURL,
      });
      const page = await ctx.newPage();
      await fakeMicrophone(page);
      await install(page, theme, {
        settings: {
          bindings: {
            voice_call: {
              id: "voice_call",
              name: "Main conversation",
              description: "",
              default_binding: "ctrl+alt+g",
              current_binding: "ctrl+alt+g",
            },
          },
        },
      });
      await page.goto("/");
      await goTo(page, "Verlauf");
      const status = page.locator(".workspace-status");
      const phone = status.locator("[data-call-button]");
      await phone.hover();
      await page.waitForTimeout(600);
      await status.screenshot({ path: `${SHOTS}/fusszeile-${theme}-aus.png` });
      await page.screenshot({ path: `${SHOTS}/seite-${theme}-aus.png` });
      await page.evaluate(() =>
        (window as any).__emit("voice-call-hotkey", null),
      );
      await expect(phone).toHaveAttribute("data-state", "active");
      await page.mouse.move(10, 10);
      await page.waitForTimeout(400);
      await status.screenshot({ path: `${SHOTS}/fusszeile-${theme}-an.png` });
      await status
        .getByRole("button", {
          name: "Gespräch: Mikrofon, Stimme und Sprachfluss",
        })
        .click();
      await page.waitForTimeout(300);
      await page.screenshot({ path: `${SHOTS}/menue-${theme}-an.png` });
      await ctx.close();

      // Overlay: dieselbe Attrappe, eigenes Fenster.
      const octx = await browser.newContext({
        viewport: { width: 260, height: 80 },
        baseURL: test.info().project.use.baseURL,
        colorScheme: theme,
      });
      const ov = await octx.newPage();
      await install(ov, theme);
      await ov.goto("/src/overlay/index.html");
      await ov.waitForFunction(
        () => (window as any).__listening("show-overlay") > 0,
      );
      for (const state of [
        "call-listening",
        "call-thinking",
        "call-speaking",
      ]) {
        await ov.evaluate(
          (s) => (window as any).__emit("show-overlay", s),
          state,
        );
        if (state === "call-listening")
          for (let i = 0; i < 12; i++) {
            await ov.evaluate(
              (k) =>
                (window as any).__emit(
                  "mic-level",
                  Array.from(
                    { length: 16 },
                    (_, j) => Math.abs(Math.sin(k + j)) * 0.9,
                  ),
                ),
              i,
            );
            await ov.waitForTimeout(30);
          }
        await ov.waitForTimeout(500);
        await ov.screenshot({ path: `${SHOTS}/overlay-${theme}-${state}.png` });
      }
      await octx.close();
    }
  });
});

test.describe("Screenshots Oberfläche @screenshots", () => {
  test("aufgeräumte Oberfläche: Leerzustand, Chat, Menüs, Verlauf, Suche, Angeheftet", async ({
    browser,
  }) => {
    test.skip(!SHOTS, "nur fuer Abnahmebilder (SCREENS_DIR)");
    const dir = `${SHOTS}/`;
    mkdirSync(dir, { recursive: true });
    const runs = [
      ["hell-1440", "light", { width: 1440, height: 900 }],
      ["hell-390", "light", { width: 390, height: 844 }],
      ["dunkel-1440", "dark", { width: 1440, height: 900 }],
    ] as const;
    for (const [name, theme, size] of runs) {
      const ctx = await browser.newContext({
        viewport: size,
        baseURL: test.info().project.use.baseURL,
      });
      const page = await ctx.newPage();
      await install(page, theme, {
        settings: {
          ...TWO_MODELS,
          voice_call_knowledge: ["meetings", "calendar"],
        },
        config: { orchestration: { souveraenitaet: "frei" } },
      });
      const narrow = size.width < 600;
      const snap = async (n: string) => {
        await settled(page);
        await page.screenshot({ path: `${dir}${n}-${name}.png` });
      };
      await openPage(page);
      await snap("1-leerzustand");
      await exchange(
        page,
        "Fasse bitte das gestrige Meeting kurz zusammen.",
        "Gern. Es ging um Budget und Termine.",
      );
      await snap("2-laufender-chat");
      await page.getByRole("button", { name: "Gesprächsoptionen" }).click();
      await snap("3-kontextmenue");
      await page.keyboard.press("Escape");
      await permissionChip(page).click();
      await snap("4-berechtigungsmenue");
      await page.keyboard.press("Escape");
      await page.getByTestId("model-picker").click();
      await snap("5-modellmenue");
      await page.keyboard.press("Escape");
      if (narrow) await historyToggle(page).click();
      await newChat(page).click();
      await exchange(page, "Rezept für Linsensuppe", "Gern, hier ein Rezept.");
      if (narrow) await historyToggle(page).click();
      await page
        .getByRole("button", { name: "Optionen für „Rezept für Linsensuppe“" })
        .click();
      await page.getByRole("menuitem", { name: "Anheften" }).click();
      await snap("6-angeheftet");
      const search = await openSearch(page);
      await expect(search).toBeVisible();
      await snap("7-suche");
      await page.keyboard.press("Escape");
      if (narrow) await page.keyboard.press("Escape");
      else await historyToggle(page).click();
      await snap("8-verlauf-eingeklappt");
      await ctx.close();
    }
  });
});
