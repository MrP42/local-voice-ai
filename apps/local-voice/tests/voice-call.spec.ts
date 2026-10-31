import { test, expect, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";

// Seite „Sprachgespraech“ gegen die Tauri-Attrappe: Infozeile, Text-Chat mit
// Token/Abo-Hinweis, Fussleisten-Menue „Gespraech“, Plus-Menue, Einstellungen.
// `voice_call_chat` bleibt offen, bis der Test ihn mit `__chat.resolve` beendet;
// Ereignisse (`voice-call`) loest `window.__emit` aus.

type Call = { cmd: string; args: Record<string, any> };

const SHOTS =
  process.env.LV_SHOTS_DIR ?? "/mnt/project-files/sprachgespraech/umsetzung";

/** Ergänzungen der Attrappe: Einstellungen, gespeicherte Gesprächskonfiguration, Werkzeuge. */
type Extra = {
  settings?: Record<string, unknown>;
  config?: Record<string, unknown>;
  tools?: Record<string, unknown>[];
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
    w.__settings = settings;
    w.__tools = extra.tools ?? [];
    w.__tool = null;
    w.__turnReplies = [] as string[];
    // Gespraechsverlauf (U3): Attrappe der Befehle `conversation*`, im Speicher.
    const convs: Record<string, any>[] = [];
    const entries: Record<string, Record<string, any>[]> = {};
    let seq = 0;
    const find = (id: string) => {
      const c = convs.find((x) => x.id === id);
      if (!c) throw "Das Gespräch gibt es nicht (mehr).";
      return c;
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
            case "voice_call_chat":
              return new Promise((resolve) => {
                w.__chat = { args, resolve };
              });
            case "voice_call_cancel":
              w.__hung?.get(args.requestId)?.("");
              return false;
            case "voice_tools_list":
              return w.__tools;
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

const message = (page: Page) =>
  page.getByRole("textbox", { name: "Nachricht" });

const history = (page: Page) =>
  page.getByRole("navigation", { name: "Gesprächsverlauf" });
const historyItem = (page: Page, title: string) =>
  history(page).getByRole("button", { name: new RegExp(`^${title}`) });
// Der Schalter im Seitenkopf (nicht der Menüpunkt „Verlauf“ der Seitenleiste).
const historyToggle = (page: Page) =>
  page.locator("button[aria-controls][aria-label='Gesprächsverlauf']");
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

  test("Fussleiste zeigt Gespräch nur auf dieser Seite", async ({ page }) => {
    await openPage(page);
    const trigger = page.getByRole("button", { name: /^Gespräch$/ }).last();
    const footerBtn = page.locator(".workspace-status").getByRole("button", {
      name: /Gespräch/,
    });
    await expect(footerBtn).toBeVisible();
    await footerBtn.click();
    await expect(page.getByRole("menu", { name: "Gespräch" })).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("menu", { name: "Gespräch" })).toHaveCount(0);
    void trigger;
    await goTo(page, "Verlauf");
    await expect(footerBtn).toHaveCount(0);
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

  test("Archivieren blendet aus, Archiv anzeigen holt zurück; Suche filtert", async ({
    page,
  }) => {
    await openPage(page);
    await exchange(page, "Hallo KI", "Hallo zurück.");
    await newChat(page).click();
    await exchange(page, "Zweite Frage", "Zweite Antwort.");
    await history(page)
      .getByRole("searchbox", { name: "Gespräche durchsuchen" })
      .fill("zweite");
    await expect(historyItem(page, "Zweite Frage")).toBeVisible();
    await expect(historyItem(page, "Hallo KI")).toHaveCount(0);
    await history(page)
      .getByRole("searchbox", { name: "Gespräche durchsuchen" })
      .fill("");

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

  test("Einstellungs-Panel hat vier Reiter", async ({ page }) => {
    await openPage(page);
    await page.getByRole("button", { name: "Gesprächseinstellungen" }).click();
    const panel = page.getByRole("complementary", {
      name: "Gesprächseinstellungen",
    });
    await expect(panel).toBeVisible();
    await expect(panel.getByRole("tab")).toHaveCount(4);
    for (const n of ["Gespräch", "Stimme", "Erkennung", "Modelle"])
      await expect(panel.getByRole("tab", { name: n })).toBeVisible();
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

    await page.getByRole("button", { name: "Gesprächseinstellungen" }).click();
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
    await page.getByRole("button", { name: "Gesprächseinstellungen" }).click();
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
    // Transkripte im Sprachgespräch: Standard aus, Wahl geht an den Kern.
    const transcripts = panel.getByRole("switch", {
      name: "Transkripte im Sprachgespräch lesen",
    });
    await expect(transcripts).not.toBeChecked();
    await transcripts.click();
    await expect
      .poll(async () => {
        const c = await calls(
          page,
          "change_voice_call_read_transcripts_setting",
        );
        return c.map((x) => x.args.enabled);
      })
      .toEqual([true]);
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
      await page
        .getByRole("button", { name: "Gesprächseinstellungen" })
        .click();
      await snap("einstellungen-1440");
    });
    await shoot("dark", big, async (_p, snap) => snap("dunkel-1440"));
    await shoot("light", { width: 390, height: 844 }, async (_p, snap) =>
      snap("hell-390"),
    );

    // Verlauf (U3): mehrere Gespräche, eines angeheftet.
    const moreHistory = async (page: Page, narrow = false) => {
      const newChat = page.getByRole("button", {
        name: "Neues Gespräch",
        exact: true,
      });
      await newChat.click();
      await exchange(page, "Rezept für Linsensuppe", "Gern, hier ein Rezept.");
      await newChat.click();
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
    const dir = "/mnt/project-files/sprachgespraech/stufe2/";
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
      await page
        .getByRole("button", { name: "Gesprächseinstellungen" })
        .click();
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
});
