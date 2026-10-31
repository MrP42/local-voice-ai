import { test, expect, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";

// Seite „Sprachgespraech“ gegen die Tauri-Attrappe: Infozeile, Text-Chat mit
// Token/Abo-Hinweis, Fussleisten-Menue „Gespraech“, Plus-Menue, Einstellungen.
// `voice_call_chat` bleibt offen, bis der Test ihn mit `__chat.resolve` beendet;
// Ereignisse (`voice-call`) loest `window.__emit` aus.

type Call = { cmd: string; args: Record<string, any> };

const SHOTS = "/mnt/project-files/sprachgespraech/umsetzung";

async function install(page: Page, theme: "light" | "dark" = "light") {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript((themeName: string) => {
    const w = window as any;
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
    w.__settings = settings;
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
  });
});
