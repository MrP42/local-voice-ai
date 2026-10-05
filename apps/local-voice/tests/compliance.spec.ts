import { test, expect } from "@playwright/test";

// Regelwerk und Schild (06.10.2026): Cloud-Modelle tragen Wolke und Flagge,
// gesperrte Modelle sind in der Auswahl nicht waehlbar, das Schild unten rechts
// zeigt die Ampel und erklaert sie auf Klick.
test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(() => {
    const callbacks = new Map<number, unknown>();
    let callback = 0;
    const settings: Record<string, unknown> = {
      onboarding_completed: true,
      app_language: "de",
      theme: "light",
      show_whats_new_on_update: false,
      debug_mode: false,
      selected_model: "",
      bindings: { transcribe: { id: "transcribe", name: "Diktat", description: "", current_binding: "Ctrl+Space", default_binding: "Ctrl+Space" } },
      post_process_providers: [],
      post_process_prompts: [],
      custom_words: [],
      post_process_models: {},
      post_process_api_keys: {},
      compliance_profile: "eu",
      llm_connections: [
        { id: "local", kind: "local", label: "In der App", base_url: "http://127.0.0.1:0/v1", enabled: true },
        { id: "claude", kind: "claude_cli", label: "Claude-Abo", base_url: "cli://claude", enabled: true },
        { id: "bedrock", kind: "bedrock_mantle", label: "Bedrock", base_url: "https://bedrock-mantle.eu-central-1.api.aws/v1", enabled: true },
      ],
      llm_models: [
        { id: "local:qwen", connection_id: "local", remote_id: "qwen", label: "Qwen3 4B", enabled: true, tags: [] },
        { id: "claude:sonnet", connection_id: "claude", remote_id: "sonnet", label: "sonnet", enabled: true, tags: [] },
        { id: "bedrock:claude", connection_id: "bedrock", remote_id: "claude", label: "Claude (Bedrock)", enabled: true, tags: [] },
      ],
      llm_active_model_id: "local:qwen",
      push_to_talk: true,
    };
    const saved: Record<string, unknown> = {};
    const status = {
      level: "green",
      profile: "eu",
      active_model: "Qwen3 4B",
      active: { verdict: "allowed", cloud: false, countries: [], reasons: ["local"], sources: [], checked: null },
      checks: [
        { id: "profile_eu", level: "green", detail: null },
        { id: "model_local", level: "green", detail: "Qwen3 4B" },
        { id: "keys_protected", level: "green", detail: null },
      ],
    };
    const assessments = [
      { model_id: "local:qwen", assessment: status.active },
      { model_id: "claude:sonnet", assessment: { verdict: "blocked", cloud: true, countries: ["US"], reasons: ["no_dpa", "third_country_no_safeguard"], sources: [], checked: "2026-10-06" } },
      { model_id: "bedrock:claude", assessment: { verdict: "allowed", cloud: true, countries: ["DE"], reasons: ["eu_ok"], sources: [], checked: "2026-10-06" } },
    ];
    Object.assign(window, { saved, __status: status });
    Object.assign(window, {
      __TAURI_OS_PLUGIN_INTERNALS__: { platform: "windows", os_type: "windows", family: "windows", arch: "x86_64", version: "10.0.26200", eol: "\r\n" },
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
      __TAURI_INTERNALS__: {
        metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
        transformCallback: (fn: unknown) => { callbacks.set(++callback, fn); return callback; },
        unregisterCallback: (id: number) => callbacks.delete(id),
        convertFileSrc: (path: string) => path,
        invoke: async (cmd: string, args?: Record<string, unknown>) => {
          if (cmd === "get_app_settings" || cmd === "get_default_settings") return settings;
          if (cmd === "plugin:os|locale") return "de-DE";
          if (cmd === "plugin:app|version") return "0.21.3";
          if (cmd.includes("permission")) return true;
          if (cmd === "plugin:event|listen") return ++callback;
          if (cmd === "get_selected_model") return "";
          if (cmd === "meetings_is_recording") return false;
          if (cmd === "tts_server_status") return { phase: "stopped", message: null };
          if (cmd === "llm_local_status") return { phase: "ready", model_id: "qwen", backend: "cuda", port: 1, message: null };
          if (cmd === "compliance_status") return JSON.parse(JSON.stringify((window as unknown as { __status: unknown }).__status));
          if (cmd === "compliance_assess_models") return JSON.parse(JSON.stringify(assessments));
          if (cmd === "compliance_set_profile") { saved.profile = args?.profile; settings.compliance_profile = args?.profile; return null; }
          if (cmd === "llm_set_active_model") { saved.active = args?.id; settings.llm_active_model_id = args?.id; return null; }
          if (cmd === "system_memory") return { ram_total_mb: 65536, ram_used_mb: 1000, gpus: [] };
          if (cmd.includes("history") || cmd.includes("models") || cmd.includes("devices") || cmd.includes("list") || cmd === "meetings_list") return [];
          if (cmd === "get_custom_sounds") return { start: false, stop: false };
          return null;
        },
      },
    });
  });
});

test("cloud models carry cloud and flag, blocked ones cannot be chosen", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto("/");
  await page.locator("[data-llm-selector]").click();
  const blocked = page.locator('[data-llm-option="claude:sonnet"]');
  await expect(blocked.locator("[data-model-badges]")).toHaveAttribute("data-verdict", "blocked");
  await expect(blocked.locator('[data-flag="US"]')).toBeVisible();
  await expect(blocked).toHaveAttribute("aria-disabled", "true");
  await blocked.click();
  expect(await page.evaluate(() => (window as unknown as { saved: Record<string, unknown> }).saved.active)).toBeUndefined();

  const eu = page.locator('[data-llm-option="bedrock:claude"]');
  await expect(eu.locator('[data-flag="DE"]')).toBeVisible();
  await expect(eu.locator("[data-model-badges]")).toHaveAttribute("data-cloud", "true");
  await eu.click();
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { saved: Record<string, unknown> }).saved.active))
    .toBe("bedrock:claude");
});

test("the shield is green when all is well and explains itself on click", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto("/");
  const shield = page.locator("[data-compliance-shield]");
  await expect(shield).toHaveAttribute("data-compliance-shield", "green");
  await shield.click();
  const panel = page.locator("[data-compliance-panel]");
  await expect(panel).toContainText("Sicher und regelkonform");
  await expect(panel.locator('[data-check="profile_eu"]')).toContainText("EU-KI-VO");
  await expect(panel.locator('[data-check="model_local"]')).toContainText("Qwen3 4B");
});

test("the shield turns red on a blocked call and says why", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.addInitScript(() => {
    const w = window as unknown as { __status: Record<string, unknown> };
    // Wird nach dem Basis-Skript ausgefuehrt: Status ueberschreiben.
    setTimeout(() => {
      w.__status.level = "red";
      (w.__status.checks as unknown[]).push({ id: "blocked_calls", level: "red", detail: "2 × Claude-Abo" });
    }, 0);
  });
  await page.goto("/");
  const shield = page.locator('[data-compliance-shield="red"]');
  await expect(shield).toBeVisible();
  await shield.click();
  await expect(page.locator('[data-check="blocked_calls"]')).toContainText("2 × Claude-Abo");
  await expect(page.locator("[data-compliance-panel]")).toContainText("Verstoß gegen das Regelwerk");
});
