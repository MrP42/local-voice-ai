import { test, expect, type Page } from "@playwright/test";
import { geheZu, oeffneModellwahl, oeffneProfilmenue } from "./profilmenue";

// Suite-Rahmen nach ADR-0026 (Variante A): schlanke Fussleiste (24 px) mit
// Telefon, Mikrofon als Symbol und Speicher-Saeulen; Titelleiste nur mit
// Navigation, App und Fensterknoepfen; Profilkreis unten links mit Menue
// (Kontingent, Benachrichtigungen, Modelle, Integrationen, Einstellungen, Hilfe).
// Schreibt Abnahmebilder nach LVA_SHOTS (Standard test-results/shell).
const SHOTS = process.env.LVA_SHOTS ?? "test-results/shell";

async function setup(page: Page, theme: "light" | "dark") {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript((THEME) => {
    const callbacks = new Map<number, unknown>();
    let callback = 0;
    const calls: [string, unknown][] = [];
    (window as unknown as { __calls: typeof calls }).__calls = calls;
    const settings: Record<string, unknown> = {
      onboarding_completed: true,
      app_language: "de",
      theme: THEME,
      show_whats_new_on_update: false,
      debug_mode: false,
      selected_model: "",
      memory_guard_enabled: true,
      memory_guard_percent: 90,
      bindings: {
        transcribe: { id: "transcribe", name: "Diktat", description: "", current_binding: "Ctrl+Space", default_binding: "Ctrl+Space" },
      },
      post_process_providers: [],
      post_process_prompts: [],
      custom_words: [],
      post_process_models: {},
      post_process_api_keys: {},
      llm_connections: [{ id: "local", kind: "local", label: "In der App", base_url: "http://127.0.0.1:0/v1", enabled: true }],
      llm_models: [{ id: "local:llm-qwen3-4b-q4", connection_id: "local", remote_id: "llm-qwen3-4b-q4", label: "Qwen3 4B", enabled: true, tags: [] }],
      llm_active_model_id: "local:llm-qwen3-4b-q4",
      push_to_talk: true,
    };
    Object.assign(window, {
      __TAURI_OS_PLUGIN_INTERNALS__: { platform: "windows", os_type: "windows", family: "windows", arch: "x86_64", version: "10.0.26200", eol: "\r\n" },
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
      __TAURI_INTERNALS__: {
        metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
        transformCallback: (fn: unknown) => { callbacks.set(++callback, fn); return callback; },
        unregisterCallback: (id: number) => callbacks.delete(id),
        convertFileSrc: (path: string) => path,
        invoke: async (cmd: string, args?: Record<string, unknown>) => {
          if (cmd.startsWith("change_memory_guard") || cmd === "release_memory") calls.push([cmd, args]);
          if (cmd === "change_memory_guard_enabled_setting") { settings.memory_guard_enabled = args?.enabled; return null; }
          if (cmd === "change_memory_guard_percent_setting") { settings.memory_guard_percent = args?.percent; return null; }
          if (cmd === "release_memory") return 1280;
          if (cmd === "get_app_settings" || cmd === "get_default_settings") return settings;
          if (cmd === "plugin:os|locale") return "de-DE";
          if (cmd === "plugin:app|version") return "0.21.30";
          if (cmd.includes("permission")) return true;
          if (cmd === "plugin:event|listen") return ++callback;
          if (cmd === "get_selected_model") return "";
          if (cmd === "meetings_is_recording") return false;
          if (cmd === "tts_server_status") return { phase: "stopped", owns_server: false, message: null, server_up: false };
          if (cmd === "compliance_status")
            return { level: "green", profile: "eu", active_model: "Qwen3 4B", active: null, checks: [], plaintext_keys: 0 };
          if (cmd === "llm_local_status") return { phase: "ready", model_id: "llm-qwen3-4b-q4", backend: "vulkan", port: 18477, message: null };
          if (cmd === "system_memory")
            return { ram_total_mb: 32403, ram_used_mb: 20172, app_ram_mb: 6451, app_gpu_mb: 2200, gpus: [
              { name: "RTX 4070", budget_mb: 12288, used_mb: 5940, dedicated_mb: 12288, shared: false },
            ] };
          if (["tts_list_voices", "tts_list_voice_infos", "llm_ps", "tts_reading_list", "pages_list", "page_files", "tts_list_downloads", "llm_local_list"].includes(cmd)) return [];
          if (cmd.includes("history") || cmd.includes("models") || cmd.includes("devices") || cmd === "meetings_list") return [];
          if (cmd === "get_custom_sounds") return { start: false, stop: false };
          return null;
        },
      },
    });
  }, theme);
}

const callsOf = (page: Page) =>
  page.evaluate(() => (window as unknown as { __calls: [string, unknown][] }).__calls);

test("Fussleiste: 24 px, schlank ohne Modellwahl und Version, Reihenfolge rechts", async ({ page }) => {
  await setup(page, "light");
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  const bar = page.locator(".lv-statusbar");
  expect((await bar.boundingBox())!.height).toBe(24);
  expect(await bar.evaluate((el) => getComputedStyle(el).fontSize)).toBe("12px");
  for (const b of await bar.locator("button.lv-status-btn").all())
    expect((await b.boundingBox())!.height).toBe(22);

  // ADR-0026 E7: Modellwahl und Version stehen nicht mehr in der Fussleiste.
  await expect(bar.locator("[data-llm-selector]")).toHaveCount(0);
  await expect(bar.locator("[data-model-status]")).toHaveCount(0);
  await expect(bar.locator("[data-testid=footer-version]")).toHaveCount(0);

  // Mikrofon: nur Symbol und Pfeil, Name im Tooltip
  const mic = bar.locator("[data-mic-selector]");
  await expect(mic).toHaveAttribute("aria-label", /Mikrofon: Systemvorgabe/);
  await expect(mic).toHaveText("");

  // Reihenfolge rechts: Mikrofon vor Speicher
  const x = async (sel: string) => (await bar.locator(sel).first().boundingBox())!.x;
  expect(await x("[data-mic-selector]")).toBeLessThan(await x("[data-resource-meter]"));
});

test("Profilmenü ▸ Modelle: Rollen-Tooltips und Modellwahl", async ({ page }) => {
  await setup(page, "light");
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await oeffneModellwahl(page);
  const modelle = page.getByTestId("profilmenue-modelle");
  await modelle.locator("[data-model-status]").hover();
  const tip = page.getByRole("tooltip");
  await expect(tip.locator("strong")).toHaveText("Spracherkennung");
  await expect(tip).toContainText("Wandelt Gesprochenes in Text um");
  await modelle.locator("[data-llm-selector]").hover();
  await expect(page.getByRole("tooltip").locator("strong")).toHaveText("KI-Modell");
  await expect(page.getByRole("tooltip")).toContainText("Qwen3 4B");
});

test("Speicher-Popover: Verteilung, freigeben, Speicherschutz", async ({ page }) => {
  await setup(page, "light");
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  const meter = page.locator("[data-resource-meter]");
  await expect(meter.locator(".lv-meter__bar")).toHaveCount(2);
  // Schutzgrenze als Linie nur an der RAM-Saeule, bei 90 %
  await expect(meter.locator("[data-meter-col=ram] .lv-meter__limit")).toHaveCount(1);
  await expect(meter.locator("[data-meter-col=gpu] .lv-meter__limit")).toHaveCount(0);
  await meter.click();
  const pop = page.locator("[data-memory-popover]");
  await expect(pop).toContainText("Arbeitsspeicher");
  await expect(pop).toContainText("19,7 von 31,6 GB (62 %)");
  await expect(pop).toContainText("Grafikspeicher");
  await expect(pop).toContainText("RTX 4070 · Treiberbudget");
  await expect(pop).toContainText("Die App lädt kein Modell, das die Grenze überschreiten würde, und entlädt vorher ungenutzte.");
  await expect(pop.locator("[data-memory-guard-switch]")).toBeChecked();
  await expect(pop.locator("[data-memory-guard-slider]")).toHaveAttribute("min", "60");
  await expect(pop.locator("[data-memory-guard-warning]")).toHaveCount(0);
  await page.screenshot({ path: `${SHOTS}/light-1440-speicher.png`, animations: "disabled" });

  await pop.locator("[data-release-memory]").click();
  await expect(pop).toContainText("1280 MB freigegeben");
  expect((await callsOf(page)).some(([c]) => c === "release_memory")).toBe(true);

  await pop.locator("[data-memory-guard-switch]").uncheck();
  await expect(pop.locator("[data-memory-guard-warning]")).toContainText(
    "Ohne Speicherschutz können große Modelle den Arbeitsspeicher füllen. Der PC kann dann stocken, einfrieren oder Programme beenden.",
  );
  expect(await callsOf(page)).toContainEqual(["change_memory_guard_enabled_setting", { enabled: false }]);
  await expect(meter.locator(".lv-meter__limit")).toHaveCount(0);
});

test("Titelleiste ohne Werkzeuge, Profilmenü unten links (ADR-0026)", async ({ page }) => {
  await setup(page, "light");
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  const bar = page.locator(".lv-titlebar");
  await expect(bar.locator(".lv-titlebar__name")).toHaveText("Local Voice AI");
  expect(await bar.innerText()).not.toMatch(/WAI/);
  // E3: keine Suche, Glocke, Hilfe, Zahnrad, Profil, Rückgängig in der Titelleiste
  await expect(bar.getByRole("searchbox")).toHaveCount(0);
  await expect(bar.getByRole("button", { name: /Benachrichtigungen|Hilfe|Einstellungen|Profil|Rückgängig/ })).toHaveCount(0);

  // E1: Profilkreis unten in der Navigation
  const nav = page.getByRole("navigation", { name: "Hauptnavigation" });
  const kreis = nav.getByRole("button", { name: /^Profil:/ });
  const navBox = (await nav.boundingBox())!;
  const kreisBox = (await kreis.boundingBox())!;
  expect(kreisBox.y + kreisBox.height).toBeGreaterThan(navBox.y + navBox.height - 60);
  // Modelle, Integrationen, Einstellungen sind keine Navigationseinträge mehr
  for (const name of ["Modelle", "Integrationen", "Einstellungen"])
    await expect(nav.getByRole("button", { name, exact: true })).toHaveCount(0);

  // E2: Menü mit festen Einträgen
  await oeffneProfilmenue(page);
  const menue = page.locator(".wai-konto-menue");
  await expect(menue).toContainText("Lokales Profil");
  for (const name of ["Verbleibendes Kontingent", "Benachrichtigungen", "Modelle", "Integrationen", "Einstellungen", "Hilfe"])
    await expect(menue.getByRole("button", { name, exact: true })).toBeVisible();
  await page.screenshot({ path: `${SHOTS}/light-1440-profilmenue.png`, animations: "disabled" });

  // Benachrichtigungen: Unterebene mit Zurück
  await menue.getByRole("button", { name: "Benachrichtigungen", exact: true }).click();
  await expect(menue).toContainText("Keine Benachrichtigungen");
  await menue.getByRole("button", { name: "Zurück", exact: true }).click();

  // Hilfe ▸ Über
  await menue.getByRole("button", { name: "Hilfe", exact: true }).click();
  await expect(menue.getByRole("button", { name: "Hilfe zu dieser Seite", exact: true })).toBeVisible();
  await page.screenshot({ path: `${SHOTS}/light-1440-profilmenue-hilfe.png`, animations: "disabled" });
  await menue.getByRole("button", { name: "Über Local Voice AI", exact: true }).click();
  const about = page.getByRole("dialog");
  await expect(about).toContainText("WAI Local Voice AI");
  await expect(about).toContainText("Version 0.21.30");
  await page.keyboard.press("Escape");

  // Einstellungen über das Menü, dann Strg+, von überall
  await oeffneProfilmenue(page);
  await page.locator(".wai-konto-menue").getByRole("button", { name: "Einstellungen", exact: true }).click();
  await expect(page.locator("#page-title")).toHaveText("Einstellungen");
  await nav.getByRole("button", { name: "Verlauf", exact: true }).click();
  await expect(page.locator("#page-title")).toHaveText("Verlauf");
  const isMac = await page.evaluate(() => /Mac|iPhone|iPad/.test(navigator.platform));
  await page.keyboard.press(isMac ? "Meta+," : "Control+,");
  await expect(page.locator("#page-title")).toHaveText("Einstellungen");
});

test("Modulkopf: ⓘ nur Kurzbeschreibung, Suche je Modul, kein Hilfe-Knopf", async ({ page }) => {
  await setup(page, "light");
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  const nav = page.getByRole("navigation", { name: "Hauptnavigation" });
  const isMac = await page.evaluate(() => /Mac|iPhone|iPad/.test(navigator.platform));

  // Verlauf: Suche an, Strg+K springt hinein
  await nav.getByRole("button", { name: "Verlauf", exact: true }).click();
  const kopf = page.locator(".page-shell__head");
  await expect(kopf.locator("#page-title")).toHaveText("Verlauf");
  await expect(page.locator(".page-shell__description")).toHaveCount(0);
  await kopf.getByTestId("page-info").hover();
  await expect(page.getByRole("tooltip")).toBeVisible();
  await page.keyboard.press(isMac ? "Meta+k" : "Control+k");
  await expect(kopf.getByRole("searchbox")).toBeFocused();
  // E4: kein Hilfe-Knopf auf der Seite, F1 öffnet die Seitenhilfe
  await expect(page.getByRole("button", { name: "Hilfe zu dieser Seite" })).toHaveCount(0);
  await page.keyboard.press("F1");
  await expect(page.locator(".lv-helpdock__panel")).toBeVisible();
  await page.screenshot({ path: `${SHOTS}/light-1440-verlauf-suche-hilfe.png`, animations: "disabled" });
  await page.keyboard.press("F1");
  await expect(page.locator(".lv-helpdock__panel")).toHaveCount(0);

  // Vorlesen: Suche aus, kein Strg+K-Feld
  await nav.getByRole("button", { name: "Vorlesen", exact: true }).click();
  await expect(page.locator("#page-title")).toHaveText("Vorlesen");
  await expect(page.locator(".page-shell__head").getByRole("searchbox")).toHaveCount(0);
});

for (const theme of ["dark", "light"] as const) {
  test(`Abnahmebild Rahmen (${theme})`, async ({ page }) => {
    await setup(page, theme);
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto("/");
    await page.locator("[data-resource-meter]").click();
    await expect(page.locator("[data-memory-popover]")).toBeVisible();
    await page.screenshot({ path: `${SHOTS}/${theme}-1440-rahmen.png`, animations: "disabled" });
    await page.keyboard.press("Escape");
    // Abnahmebilder ADR-0026: Profilmenü ▸ Modelle und Vorlesen mit Modulkopf
    await oeffneModellwahl(page);
    await page.screenshot({ path: `${SHOTS}/${theme}-1440-profilmenue-modelle.png`, animations: "disabled" });
    await page.keyboard.press("Escape");
    await page.getByRole("navigation", { name: "Hauptnavigation" }).getByRole("button", { name: "Vorlesen", exact: true }).click();
    await expect(page.locator("#page-title")).toHaveText("Vorlesen");
    await page.screenshot({ path: `${SHOTS}/${theme}-1440-vorlesen.png`, animations: "disabled" });
    await page.getByRole("navigation", { name: "Hauptnavigation" }).getByRole("button", { name: "Start", exact: true }).click();
    await page.setViewportSize({ width: 390, height: 844 });
    await page.screenshot({ path: `${SHOTS}/${theme}-390-rahmen.png`, animations: "disabled" });
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    expect(overflow).toBeLessThanOrEqual(0);
  });
}

// Seitenhilfe: auf allen Seiten dieselbe volle Spalte ganz rechts im Dock;
// bei Vorlesen bleibt die Dateiliste, wo sie ist.
for (const theme of ["light", "dark"] as const) {
  test(`Seitenhilfe dockt rechts an (${theme})`, async ({ page }) => {
    await setup(page, theme);
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto("/");
    const nav = page.getByRole("navigation", { name: "Hauptnavigation" });
    for (const [label, id] of [["Einstellungen", "einstellungen"], ["Vorlesen", "vorlesen"], ["Aufnahmen", "aufnahmen"]] as const) {
      if (label === "Einstellungen") await geheZu(page, "Einstellungen");
      else await nav.getByRole("button", { name: label, exact: true }).click();
      await expect(page.locator("#page-title")).toHaveText(label);
      const dock = page.locator(".lv-helpdock");
      if (id === "vorlesen") await page.evaluate(() => localStorage.removeItem("lva.ui.tts.helpOpen"));
      // ADR-0026 E4: Hilfe nur über F1 bzw. Profilmenü ▸ Hilfe
      await page.keyboard.press("F1");
      const panel = dock.locator(".lv-helpdock__panel");
      await expect(panel).toBeVisible();
      const box = (await panel.boundingBox())!;
      expect(box.x + box.width).toBeGreaterThan(1440 - 2);
      // Die Hilfe sitzt nicht in der zentrierten Spalte
      await expect(page.locator("#workspace-content .lv-helpdock__panel")).toHaveCount(0);
      if (id === "vorlesen") await expect(page.getByTestId("tts-files")).toBeVisible();
      await page.screenshot({ path: `${SHOTS}/hilfe-${id}-${theme}.png`, animations: "disabled" });
      await page.getByRole("button", { name: "Hilfe schließen" }).click();
      await expect(panel).toHaveCount(0);
    }
  });
}
