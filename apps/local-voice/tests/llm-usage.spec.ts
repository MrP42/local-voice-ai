import { test, expect } from "@playwright/test";

// Verbrauch und Budget: die Uebersicht zeigt, was das Backend gebucht hat,
// die Fussleiste warnt ab 80 % des Monatsbudgets -- beides aus Zahlen, die
// der Mock liefert, nicht aus Annahmen der Oberflaeche.
const setup =
  (budget: { spent: number; limit: number; enforced: boolean } | null) =>
  async ({ page }: { page: import("@playwright/test").Page }) => {
    page.on("pageerror", (error) => {
      throw error;
    });
    await page.addInitScript((budget) => {
      const callbacks = new Map<number, unknown>();
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
        post_process_providers: [
          {
            id: "openai",
            label: "OpenAI",
            base_url: "https://api.openai.com/v1",
            allow_base_url_edit: false,
            models_endpoint: null,
            supports_structured_output: true,
          },
        ],
        post_process_prompts: [],
        custom_words: [],
        post_process_models: {},
        post_process_api_keys: {},
        llm_connections: [
          {
            id: "openai-1",
            kind: "openai",
            label: "OpenAI",
            base_url: "https://api.openai.com/v1",
            enabled: true,
            monthly_budget_usd: budget ? budget.limit : null,
            budget_enforced: budget?.enforced ?? false,
          },
        ],
        llm_models: [
          {
            id: "openai-1:gpt-4.1-mini",
            connection_id: "openai-1",
            remote_id: "gpt-4.1-mini",
            label: "gpt-4.1-mini",
            enabled: true,
            tags: [],
          },
        ],
        llm_active_model_id: "openai-1:gpt-4.1-mini",
        push_to_talk: true,
      };
      const saved: Record<string, unknown> = {};
      (window as unknown as { saved: Record<string, unknown> }).saved = saved;
      Object.assign(window, {
        __TAURI_OS_PLUGIN_INTERNALS__: {
          platform: "windows",
          os_type: "windows",
          family: "windows",
          arch: "x86_64",
          version: "10.0.26200",
          eol: "\r\n",
        },
        __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
        __TAURI_INTERNALS__: {
          metadata: {
            currentWindow: { label: "main" },
            currentWebview: { label: "main" },
          },
          transformCallback: (fn: unknown) => {
            callbacks.set(++callback, fn);
            return callback;
          },
          unregisterCallback: (id: number) => callbacks.delete(id),
          convertFileSrc: (path: string) => path,
          invoke: async (cmd: string, args?: Record<string, unknown>) => {
            if (cmd === "get_app_settings" || cmd === "get_default_settings")
              return settings;
            if (cmd === "plugin:os|locale") return "de-DE";
            if (cmd === "plugin:app|version") return "0.16.7";
            if (cmd.includes("permission")) return true;
            if (cmd === "plugin:event|listen") return ++callback;
            if (cmd === "get_selected_model") return "";
            if (cmd === "meetings_is_recording") return false;
            if (cmd === "tts_server_status")
              return { phase: "stopped", message: null };
            if (cmd === "llm_local_status")
              return {
                phase: "stopped",
                model_id: null,
                backend: null,
                port: null,
                message: null,
              };
            if (cmd === "system_memory")
              return { ram_total_mb: 0, ram_used_mb: 0, gpus: [] };
            if (cmd === "usage_budget_states")
              return budget
                ? [
                    {
                      connection_id: "openai-1",
                      spent_micro: budget.spent * 1_000_000,
                      limit_micro: budget.limit * 1_000_000,
                      ratio: budget.spent / budget.limit,
                      enforced: budget.enforced,
                    },
                  ]
                : [];
            if (cmd === "usage_summary") {
              saved.range = args?.range;
              return {
                range: args?.range,
                calls: 12,
                failed: 1,
                prompt_tokens: 48_200,
                completion_tokens: 6_100,
                cost_micro: 29_040,
                by_model: [
                  {
                    key: "openai-1:gpt-4.1-mini",
                    label: "gpt-4.1-mini · OpenAI",
                    calls: 12,
                    prompt_tokens: 48_200,
                    completion_tokens: 6_100,
                    cost_micro: 29_040,
                  },
                ],
                by_purpose: [
                  {
                    key: "post_process",
                    label: "post_process",
                    calls: 9,
                    prompt_tokens: 40_000,
                    completion_tokens: 5_000,
                    cost_micro: 24_000,
                  },
                  {
                    key: "translation",
                    label: "translation",
                    calls: 3,
                    prompt_tokens: 8_200,
                    completion_tokens: 1_100,
                    cost_micro: 5_040,
                  },
                ],
                by_day: [
                  {
                    key: "2026-09-12",
                    label: "2026-09-12",
                    calls: 5,
                    prompt_tokens: 20_000,
                    completion_tokens: 2_000,
                    cost_micro: 10_000,
                  },
                  {
                    key: "2026-09-13",
                    label: "2026-09-13",
                    calls: 7,
                    prompt_tokens: 28_200,
                    completion_tokens: 4_100,
                    cost_micro: 19_040,
                  },
                ],
              };
            }
            if (cmd === "usage_clear") {
              saved.cleared = true;
              return null;
            }
            if (cmd === "llm_upsert_connection") {
              saved.connection = args?.connection;
              return null;
            }
            if (
              cmd === "tts_list_voices" ||
              cmd === "tts_list_voice_infos" ||
              cmd === "llm_ps" ||
              cmd === "tts_reading_list" ||
              cmd === "pages_list" ||
              cmd === "page_files" ||
              cmd === "tts_list_downloads" ||
              cmd === "llm_local_list"
            )
              return [];
            if (
              cmd.includes("history") ||
              cmd.includes("models") ||
              cmd.includes("devices") ||
              cmd === "meetings_list"
            )
              return [];
            if (cmd === "get_custom_sounds")
              return { start: false, stop: false };
            return null;
          },
        },
      });
    }, budget);
  };

const openPolishTab = async (page: import("@playwright/test").Page) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/");
  await page
    .getByRole("button", { name: "Einstellungen", exact: true })
    .last()
    .click();
  await page
    .getByRole("tab", { name: "KI-Textverbesserung", exact: true })
    .click();
};

test.describe("usage overview", () => {
  test.beforeEach(setup(null));

  test("shows totals, models and purposes as the backend booked them", async ({
    page,
  }) => {
    await openPolishTab(page);
    const overview = page.locator("[data-usage-overview]");
    await overview.scrollIntoViewIfNeeded();
    const totals = overview.locator("[data-usage-totals]");
    await expect(totals).toContainText("12");
    await expect(totals).toContainText("48.200");
    await expect(totals).toContainText("0,03 USD");
    await expect(totals).toContainText("1 fehlgeschlagen");
    await expect(overview.locator("[data-usage-table=by-model]")).toContainText(
      "gpt-4.1-mini · OpenAI",
    );
    // Zwecke werden uebersetzt, nicht als Schluessel gezeigt.
    const purposes = overview.locator("[data-usage-table=by-purpose]");
    await expect(purposes).toContainText("Diktat verbessern");
    await expect(purposes).toContainText("Übersetzung");
    await expect(purposes).not.toContainText("post_process");
    // Zwei Tage -> zwei Balken.
    await expect(overview.locator("[data-usage-days] > div")).toHaveCount(2);
  });

  test("the range switch asks the backend for that range", async ({ page }) => {
    await openPolishTab(page);
    const overview = page.locator("[data-usage-overview]");
    await overview.scrollIntoViewIfNeeded();
    await overview.getByRole("tab", { name: "Heute" }).click();
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (window as unknown as { saved: Record<string, unknown> }).saved
              .range,
        ),
      )
      .toBe("today");
  });

  test("a budget is saved on the connection", async ({ page }) => {
    await openPolishTab(page);
    // Verbindung aufklappen und Budget setzen.
    await page.getByRole("button", { name: "OpenAI", exact: true }).click();
    const field = page
      .locator("[data-budget-row]")
      .getByLabel("Monatsbudget (USD)");
    await field.fill("12,50");
    await field.blur();
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (
              (window as unknown as { saved: Record<string, unknown> }).saved
                .connection as { monthly_budget_usd?: number } | undefined
            )?.monthly_budget_usd,
        ),
      )
      .toBe(12.5);
  });
});

test.describe("footer budget light", () => {
  test("warns in yellow at 85 % of the monthly budget", async ({ page }) => {
    await setup({ spent: 8.5, limit: 10, enforced: false })({ page });
    await page.setViewportSize({ width: 1280, height: 800 });
    await page.goto("/");
    const selector = page.locator("[data-llm-selector]");
    await expect(selector.locator("div.rounded-full")).toHaveClass(
      /bg-yellow-400/,
    );
    await expect(selector.locator("[data-llm-budget]")).toHaveText("85%");
  });

  test("turns red once a hard budget is exhausted", async ({ page }) => {
    await setup({ spent: 10, limit: 10, enforced: true })({ page });
    await page.setViewportSize({ width: 1280, height: 800 });
    await page.goto("/");
    const selector = page.locator("[data-llm-selector]");
    await expect(selector.locator("div.rounded-full")).toHaveClass(
      /bg-red-400/,
    );
    await expect(selector).toHaveAttribute("title", /ausgeschöpft/);
  });

  test("stays green without a budget", async ({ page }) => {
    await setup(null)({ page });
    await page.setViewportSize({ width: 1280, height: 800 });
    await page.goto("/");
    const selector = page.locator("[data-llm-selector]");
    await expect(selector.locator("div.rounded-full")).toHaveClass(
      /bg-green-400/,
    );
    await expect(selector.locator("[data-llm-budget]")).toHaveCount(0);
  });
});
