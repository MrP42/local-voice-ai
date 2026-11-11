import { test, expect, type Page } from "@playwright/test";
import { geheZu } from "./profilmenue";

// Einstellungen › Allgemein › „Netzwerk“ (lva#496): Der Knopf prüft die
// eingerichteten Gegenstellen, zeigt je Zeile erreichbar / eingeschränkt /
// nicht erreichbar als Wort UND Symbol samt Auswirkung, und der Bericht für die
// IT lässt sich kopieren. Nichts läuft von selbst. Die Tauri-Brücke ist die
// schlanke Attrappe aus library-layout-settings.spec.ts.

const REPORT =
  "Local Voice – Netzwerkprüfung\nApp-Version: 0.24.1\n1. [nicht erreichbar] Geräteabgleich – hub.firma.example:443\n";

const ROWS = [
  {
    category: "sync_hub",
    host: "hub.firma.example",
    port: 443,
    tls: true,
    state: "unreachable",
    reason: "dns_failed",
    dns_ms: null,
    tls_ms: null,
    http_status: null,
    total_ms: 41,
    issuer: null,
  },
  {
    category: "update",
    host: "github.com",
    port: 443,
    tls: true,
    state: "limited",
    reason: "tls_inspection",
    dns_ms: 12,
    tls_ms: 80,
    http_status: 200,
    total_ms: 140,
    issuer: "Zscaler Root CA (Zscaler Inc.)",
  },
  {
    category: "llm",
    host: "api.openai.com",
    port: 443,
    tls: true,
    state: "reachable",
    reason: null,
    dns_ms: 9,
    tls_ms: 55,
    http_status: 421,
    total_ms: 120,
    issuer: null,
  },
];

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(() => {
    const callbacks = new Map<number, unknown>();
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
      tts_engine: "fish",
      tts_piper_voice: null,
      tts_export_format: "wav",
    };
    const voices = [
      {
        id: "patrick",
        meta: { display_name: "Patrick", tags: [] },
        origin: "recorded",
        avatar_path: null,
      },
    ];
    const w = window as unknown as {
      __nzCalls?: string[];
      __nzResult?: unknown;
      __nzError?: string;
      __nzDelay?: number;
    };
    w.__nzCalls = [];
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
        invoke: async (cmd: string) => {
          if (cmd === "get_app_settings" || cmd === "get_default_settings")
            return settings;
          if (cmd === "plugin:os|locale") return "de-DE";
          if (cmd === "plugin:app|version") return "0.24.1";
          if (cmd.includes("permission")) return true;
          if (cmd === "plugin:event|listen") return ++callback;
          if (cmd === "get_selected_model") return "";
          if (cmd === "meetings_is_recording") return false;
          if (cmd === "tts_server_status")
            return { phase: "stopped", message: null };
          if (cmd === "library_layout_status")
            return {
              layout: 2,
              phase: null,
              probleme: [],
              hinweise: [],
              zu_verschieben: 0,
              platz_benoetigt: 0,
              platz_frei: 50_000_000_000,
            };
          if (cmd === "library_backup_list") return [];
          if (cmd === "tts_list_voice_infos") return voices;
          if (cmd === "tts_list_voices") return voices.map((v) => v.id);
          if (cmd === "netzcheck_run") {
            w.__nzCalls!.push(cmd);
            // Dauert einen Moment, damit der Zustand „Prüfung läuft“ sichtbar wird.
            await new Promise((r) => setTimeout(r, w.__nzDelay ?? 250));
            if (w.__nzError) throw w.__nzError;
            return w.__nzResult;
          }
          if (
            cmd === "llm_ps" ||
            cmd === "tts_reading_list" ||
            cmd.includes("history") ||
            cmd.includes("models") ||
            cmd.includes("devices") ||
            cmd === "meetings_list" ||
            cmd === "tts_list_downloads"
          )
            return [];
          if (cmd === "get_custom_sounds") return { start: false, stop: false };
          return null;
        },
      },
    });
  });
});

const openApp = async (page: Page, result: unknown) => {
  await page.addInitScript((r) => {
    (window as unknown as { __nzResult: unknown }).__nzResult = r;
  }, result);
  await page.goto("/");
  await geheZu(page, "Einstellungen");
  await page.getByRole("tab", { name: "Allgemein" }).click();
  await expect(page.getByTestId("network-check")).toBeVisible();
};

const withRows = {
  rows: ROWS,
  proxy: {
    entries: [
      {
        source: "system",
        scope: "https",
        endpoint: "proxy.firma.example:3128",
      },
    ],
    pac: false,
  },
  report: REPORT,
};

test("nichts läuft von selbst; der Knopf startet die Prüfung", async ({
  page,
}) => {
  await openApp(page, withRows);
  await page.waitForTimeout(600);
  expect(
    await page.evaluate(
      () => (window as unknown as { __nzCalls: string[] }).__nzCalls.length,
    ),
  ).toBe(0);
  await expect(page.getByTestId("network-check-row")).toHaveCount(0);

  const run = page.getByTestId("network-check-run");
  await run.click();
  await expect(run).toBeDisabled();
  await expect(page.getByTestId("network-check-status")).toContainText(
    "Prüfung läuft",
  );
  await expect(page.getByTestId("network-check-row")).toHaveCount(3);
  await expect(run).toBeEnabled();
  expect(
    await page.evaluate(
      () => (window as unknown as { __nzCalls: string[] }).__nzCalls.length,
    ),
  ).toBe(1);
});

test("Zeilen nennen Zustand als Wort, Auswirkung, Grund und Proxy", async ({
  page,
}) => {
  await openApp(page, withRows);
  await page.getByTestId("network-check-run").click();
  const rows = page.getByTestId("network-check-row");
  await expect(rows).toHaveCount(3);

  const down = rows.nth(0);
  await expect(down).toHaveAttribute("data-state", "unreachable");
  await expect(down).toContainText("Nicht erreichbar");
  await expect(down).toContainText("hub.firma.example:443");
  await expect(down).toContainText(
    "Der Geräteabgleich funktioniert nicht; Änderungen bleiben auf diesem Gerät.",
  );
  await expect(down).toContainText("Namensauflösung fehlgeschlagen");

  const limited = rows.nth(1);
  await expect(limited).toContainText("Eingeschränkt");
  await expect(limited).toContainText("Zscaler Root CA (Zscaler Inc.)");
  await expect(limited).toContainText("TLS-aufbrechenden Proxy");

  await expect(rows.nth(2)).toContainText("Erreichbar");

  await expect(page.getByTestId("network-check-status")).toContainText(
    "1 erreichbar, 1 eingeschränkt, 1 nicht erreichbar",
  );
  await expect(page.getByTestId("network-check-proxy")).toContainText(
    "proxy.firma.example:3128",
  );
});

test("Bericht lässt sich anzeigen und kopieren", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await openApp(page, withRows);
  await page.getByTestId("network-check-run").click();
  await expect(page.getByTestId("network-check-row")).toHaveCount(3);

  await page.getByText("Bericht anzeigen").click();
  await expect(page.getByTestId("network-check-report")).toContainText(
    "hub.firma.example:443",
  );

  await page.getByTestId("network-check-copy").click();
  await expect(page.locator("[data-sonner-toast]")).toContainText(
    "Bericht kopiert",
  );
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(
    REPORT,
  );
});

test("ohne eingerichtete Gegenstellen sagt die Prüfung es", async ({
  page,
}) => {
  await openApp(page, {
    rows: [],
    proxy: { entries: [], pac: false },
    report: REPORT,
  });
  await page.getByTestId("network-check-run").click();
  await expect(page.getByTestId("network-check-status")).toContainText(
    "keine Gegenstellen eingerichtet",
  );
  await expect(page.getByTestId("network-check-proxy")).toContainText(
    "Kein System-Proxy erkannt",
  );
});

test("Tastatur: Knopf per Fokus und Eingabetaste, Ergebnisliste benannt", async ({
  page,
}) => {
  await openApp(page, withRows);
  await page.getByTestId("network-check-run").focus();
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("network-check-row")).toHaveCount(3);
  await expect(
    page.getByRole("list", { name: "Ergebnisse der Netzwerkprüfung" }),
  ).toBeVisible();
  await expect(page.getByTestId("network-check-status")).toHaveAttribute(
    "aria-live",
    "polite",
  );
  // Der Bericht ist per Tastatur erreichbar (scrollbarer Bereich im Fokus).
  await page.getByText("Bericht anzeigen").focus();
  await page.keyboard.press("Enter");
  await page.keyboard.press("Tab");
  await expect(page.getByTestId("network-check-report")).toBeFocused();
});

test("läuft schon eine Prüfung, steht das in der Statuszeile und nicht „Fehler“", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as unknown as { __nzError: string }).__nzError = "busy";
  });
  await openApp(page, withRows);
  await page.getByTestId("network-check-run").click();
  await expect(page.locator("[data-sonner-toast]")).toContainText(
    "läuft bereits eine Prüfung",
  );
  const status = page.getByTestId("network-check-status");
  await expect(status).toContainText("Es läuft bereits eine Prüfung.");
  await expect(status).not.toContainText("konnte nicht durchgeführt werden");
  await expect(page.getByTestId("network-check-row")).toHaveCount(0);
});

test("ein anderer Fehler des Befehls zeigt die Fehlermeldung in der Statuszeile", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as unknown as { __nzError: string }).__nzError = "kaputt";
  });
  await openApp(page, withRows);
  await page.getByTestId("network-check-run").click();
  await expect(page.locator("[data-sonner-toast]")).toContainText(
    "konnte nicht durchgeführt werden",
  );
  const status = page.getByTestId("network-check-status");
  await expect(status).toContainText("konnte nicht durchgeführt werden");
  await expect(status).not.toContainText("läuft bereits");
  await expect(page.getByTestId("network-check-row")).toHaveCount(0);
});

test("ein neuer Lauf leert das alte Ergebnis, auch wenn er scheitert", async ({
  page,
}) => {
  await openApp(page, withRows);
  const w = () =>
    page.evaluate(() => {
      const x = window as unknown as { __nzDelay?: number };
      x.__nzDelay = 1500;
    });
  const run = page.getByTestId("network-check-run");
  await run.click();
  await expect(page.getByTestId("network-check-row")).toHaveCount(3);
  await expect(page.getByTestId("network-check-proxy")).toBeVisible();

  // Zweiter Lauf: Das alte Ergebnis verschwindet sofort, nicht erst bei der Antwort.
  await w();
  await page.evaluate(() => {
    (window as unknown as { __nzError: string }).__nzError = "kaputt";
  });
  await run.click();
  await expect(page.getByTestId("network-check-status")).toContainText(
    "Prüfung läuft",
  );
  await expect(page.getByTestId("network-check-row")).toHaveCount(0);
  await expect(page.getByTestId("network-check-proxy")).toHaveCount(0);
  await expect(page.getByTestId("network-check-copy")).toHaveCount(0);

  // Der Lauf scheitert: es bleibt kein veralteter Bericht stehen.
  await expect(page.getByTestId("network-check-status")).toContainText(
    "konnte nicht durchgeführt werden",
  );
  await expect(page.getByTestId("network-check-row")).toHaveCount(0);
  await expect(page.getByTestId("network-check-report")).toHaveCount(0);
});

test("Zertifikatsfehler nennt den Aussteller oder sagt „nicht ermittelbar“", async ({
  page,
}) => {
  const base = {
    category: "llm",
    host: "api.firma.example",
    port: 443,
    tls: true,
    state: "unreachable",
    reason: "tls_certificate",
    dns_ms: 8,
    tls_ms: null,
    http_status: null,
    total_ms: 90,
  };
  await openApp(page, {
    rows: [
      { ...base, issuer: "Firma Inspektion Root CA (Firma GmbH)" },
      { ...base, host: "tts.firma.example", issuer: null },
      {
        ...base,
        host: "hub.firma.example",
        reason: "dns_failed",
        issuer: null,
      },
    ],
    proxy: { entries: [], pac: false },
    report: REPORT,
  });
  await page.getByTestId("network-check-run").click();
  const rows = page.getByTestId("network-check-row");
  await expect(rows).toHaveCount(3);
  await expect(rows.nth(0)).toContainText(
    "Aussteller des Zertifikats: Firma Inspektion Root CA (Firma GmbH)",
  );
  await expect(rows.nth(1)).toContainText(
    "Aussteller des Zertifikats: nicht ermittelbar",
  );
  await expect(rows.nth(2)).not.toContainText("Aussteller");
});

test("HTTP-5xx zeigt den übersetzten Grund, nicht den Schlüssel", async ({
  page,
}) => {
  await openApp(page, {
    rows: [
      {
        category: "llm",
        host: "api.firma.example",
        port: 443,
        tls: true,
        state: "limited",
        reason: "http_5xx",
        dns_ms: 8,
        tls_ms: 40,
        http_status: 502,
        total_ms: 90,
        issuer: null,
      },
    ],
    proxy: { entries: [], pac: false },
    report: REPORT,
  });
  await page.getByTestId("network-check-run").click();
  const row = page.getByTestId("network-check-row");
  await expect(row).toContainText(
    "Server oder Proxy meldet einen Fehler (HTTP 5xx)",
  );
  await expect(row).not.toContainText("http_5xx");
});
