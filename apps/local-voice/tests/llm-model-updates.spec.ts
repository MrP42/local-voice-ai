import { test, expect, type Page } from "@playwright/test";

// Neue Modelle der Anbieter: Pruefung nach dem Start (Backend entscheidet, was
// uebernommen wird), Dialog mit den drei Antworten, Schalter in den
// Einstellungen und die Kennzeichnung "neu" in der Modellliste einer Verbindung.
type Update = {
  connection_id: string;
  connection_label: string;
  remote_id: string;
  replaces: string | null;
  replaces_remote_id: string | null;
  replaces_active: boolean;
};

const haiku: Update = {
  connection_id: "claude",
  connection_label: "Claude-Abo",
  remote_id: "claude-haiku-5-5",
  replaces: "claude:claude-haiku-4-5-20251001",
  replaces_remote_id: "claude-haiku-4-5-20251001",
  replaces_active: true,
};

async function setup(
  page: Page,
  check: { mode: string; applied: Update[]; pending: Update[] },
  history: unknown[] = [],
) {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.clock.install();
  await page.addInitScript(
    ({ check, history }) => {
      const callbacks = new Map<number, unknown>();
      let callback = 0;
      const w = window as unknown as Record<string, unknown>;
      w.__calls = [] as unknown[];
      w.__listeners = {} as Record<string, number[]>;
      w.__emit = (name: string, payload: unknown) =>
        ((w.__listeners as Record<string, number[]>)[name] ?? []).forEach((id) =>
          (callbacks.get(id) as (e: unknown) => void)?.({ event: name, id: 0, payload }),
        );
      const settings: Record<string, unknown> = {
        onboarding_completed: true,
        app_language: "de",
        theme: "light",
        show_whats_new_on_update: false,
        debug_mode: false,
        selected_model: "",
        bindings: {
          transcribe: { id: "transcribe", name: "Diktat", description: "", current_binding: "Ctrl+Space", default_binding: "Ctrl+Space" },
        },
        post_process_providers: [],
        post_process_prompts: [],
        custom_words: [],
        post_process_models: {},
        post_process_api_keys: {},
        llm_connections: [
          { id: "ol", kind: "ollama", label: "Ollama", base_url: "http://127.0.0.1:11434/v1", enabled: true },
        ],
        llm_models: [],
        llm_active_model_id: null,
        llm_auto_update_models: check.mode,
        llm_model_history: history,
      };
      Object.assign(window, {
        __TAURI_OS_PLUGIN_INTERNALS__: { platform: "windows", os_type: "windows", family: "windows", arch: "x86_64", version: "10.0.26200", eol: "\r\n" },
        __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
        __TAURI_INTERNALS__: {
          metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
          transformCallback: (fn: unknown) => {
            callbacks.set(++callback, fn);
            return callback;
          },
          unregisterCallback: (id: number) => callbacks.delete(id),
          convertFileSrc: (path: string) => path,
          invoke: async (cmd: string, args?: Record<string, unknown>) => {
            (w.__calls as unknown[]).push([cmd, args]);
            if (cmd === "get_app_settings" || cmd === "get_default_settings") return JSON.parse(JSON.stringify(settings));
            if (cmd === "plugin:os|locale") return "de-DE";
            if (cmd === "plugin:app|version") return "0.21.10";
            if (cmd.includes("permission")) return true;
            if (cmd === "plugin:event|listen") {
              (w.__listeners as Record<string, number[]>)[String(args?.event)] ??= [];
              (w.__listeners as Record<string, number[]>)[String(args?.event)].push(Number(args?.handler));
              return ++callback;
            }
            if (cmd === "get_selected_model") return "";
            if (cmd === "meetings_is_recording") return false;
            if (cmd === "llm_check_new_models") return check;
            if (cmd === "llm_answer_model_updates") return args?.answer === "never" ? [] : check.pending;
            if (cmd === "llm_set_auto_update_models") {
              settings.llm_auto_update_models = args?.mode;
              return null;
            }
            if (cmd === "llm_list_remote_models") return ["qwen3:8b", "qwen9:1b"];
            if (cmd === "get_custom_sounds") return { start: false, stop: false };
            if (cmd.includes("history") || cmd.includes("models") || cmd.includes("devices") || cmd === "meetings_list") return [];
            return null;
          },
        },
      });
    },
    { check, history },
  );
}

const calls = (page: Page) =>
  page.evaluate(() => (window as unknown as { __calls: [string, unknown][] }).__calls);

const afterStart = async (page: Page) => {
  await page.goto("/");
  await page.clock.fastForward(25_000);
  // Die Antwort des Backends kommt in Echtzeit; die Uhr danach noch einmal laufen
  // lassen, damit Dialog und Hinweis (Animation) sichtbar werden.
  await page.waitForTimeout(300);
  await page.clock.fastForward(1_000);
};

test("fragen: der Dialog nennt das neue Modell und das, was es ersetzt", async ({ page }) => {
  await setup(page, { mode: "ask", applied: [], pending: [haiku] });
  await afterStart(page);
  const list = page.getByTestId("model-update-list");
  await expect(list).toContainText("Claude Haiku 5.5");
  await expect(list).toContainText("ersetzt das aktive Modell Claude Haiku 4.5");
  await expect(page.getByRole("button", { name: "Ja, immer" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Nur dieses Mal" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Nein, ich wähle selbst" })).toBeVisible();
});

for (const [label, answer] of [
  ["Ja, immer", "always"],
  ["Nur dieses Mal", "once"],
  ["Nein, ich wähle selbst", "never"],
] as const) {
  test(`Antwort "${label}" geht als ${answer} ans Backend und schliesst den Dialog`, async ({ page }) => {
    await setup(page, { mode: "ask", applied: [], pending: [haiku] });
    await afterStart(page);
    await page.getByRole("button", { name: label }).click();
    await expect(page.getByTestId("model-update-list")).toHaveCount(0);
    const sent = (await calls(page)).filter(([c]) => c === "llm_answer_model_updates");
    expect(sent).toEqual([["llm_answer_model_updates", { answer }]]);
  });
}

test("automatisch: kein Dialog, nur ein Hinweis", async ({ page }) => {
  await setup(page, { mode: "on", applied: [haiku], pending: [] });
  await afterStart(page);
  await expect(page.getByText("Neues Sprachmodell übernommen")).toBeVisible();
  await expect(page.getByText("Claude Haiku 5.5 ersetzt Claude Haiku 4.5")).toBeVisible();
  await expect(page.getByTestId("model-update-list")).toHaveCount(0);
});

test("nein: kein Dialog und kein Hinweis", async ({ page }) => {
  await setup(page, { mode: "off", applied: [], pending: [] });
  await afterStart(page);
  await expect(page.getByTestId("model-update-list")).toHaveCount(0);
  await expect(page.getByText("Neues Sprachmodell übernommen")).toHaveCount(0);
  const checked = (await calls(page)).filter(([c]) => c === "llm_check_new_models");
  expect(checked.length).toBeGreaterThan(0);
});

test("Schliessen ohne Antwort schickt nichts ans Backend", async ({ page }) => {
  await setup(page, { mode: "ask", applied: [], pending: [haiku] });
  await afterStart(page);
  await page.getByRole("button", { name: "Später fragen" }).first().click();
  await expect(page.getByTestId("model-update-list")).toHaveCount(0);
  const sent = (await calls(page)).filter(([c]) => c === "llm_answer_model_updates");
  expect(sent).toEqual([]);
});

test("der Schalter in den Einstellungen zeigt den Stand und ruft das Backend", async ({ page }) => {
  await setup(page, { mode: "ask", applied: [], pending: [] });
  await page.goto("/");
  await page.getByRole("button", { name: "Einstellungen", exact: true }).first().click();
  await page.getByRole("tab", { name: "KI-Modelle & Anbieter", exact: true }).click();
  const select = page.getByTestId("auto-update-models");
  await expect(select).toHaveValue("ask");
  await select.selectOption("off");
  const sent = (await calls(page)).filter(([c]) => c === "llm_set_auto_update_models");
  expect(sent).toEqual([["llm_set_auto_update_models", { mode: "off" }]]);
});

test("in der Modellliste einer Verbindung steht neu hinter neuen, nicht freigegebenen Modellen", async ({ page }) => {
  await setup(page, { mode: "off", applied: [], pending: [] }, [
    { key: "ol:qwen9:1b", status: "new", at: 1 },
    { key: "ol:qwen3:8b", status: "seen", at: 1 },
  ]);
  await page.goto("/");
  await page.getByRole("button", { name: "Einstellungen", exact: true }).first().click();
  await page.getByRole("tab", { name: "KI-Modelle & Anbieter", exact: true }).click();
  await page.getByRole("button", { name: "Ollama", exact: true }).first().click();
  await page.getByRole("button", { name: "Modelle laden" }).click();
  await expect(page.locator('[data-new-model="qwen9:1b"]')).toContainText("neu");
  await expect(page.locator('[data-new-model="qwen3:8b"]')).toHaveCount(0);
});

test("die Codex-CLI aktualisiert sich selbst: Hinweise vom Start bis zum Ende", async ({ page }) => {
  await setup(page, { mode: "off", applied: [], pending: [] });
  await page.goto("/");
  await page.waitForFunction(
    () => ((window as unknown as { __listeners: Record<string, number[]> }).__listeners["cli-update"] ?? []).length > 0,
  );
  const emit = (state: string, version: string | null, message: string | null) =>
    page.evaluate(
      ([state, version, message]) =>
        (window as unknown as { __emit: (n: string, p: unknown) => void }).__emit("cli-update", {
          cli: "codex",
          state,
          version,
          message,
        }),
      [state, version, message],
    ).then(() => page.clock.fastForward(500));
  await emit("started", null, null);
  await expect(page.getByText("Die Codex-CLI ist zu alt für dieses Modell")).toBeVisible();
  await emit("done", "codex-cli 0.162.1", null);
  await expect(page.getByText("Codex-CLI aktualisiert (codex-cli 0.162.1)")).toBeVisible();
  await expect(page.getByText("Bitte den Vorgang jetzt noch einmal starten.")).toBeVisible();
  await emit("manual", null, "Node.js wurde nicht gefunden.");
  await expect(page.getByText("von Hand aktualisiert werden")).toBeVisible();
  await expect(page.getByText("Node.js wurde nicht gefunden.")).toBeVisible();
});
