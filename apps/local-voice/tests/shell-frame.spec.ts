import { test, expect, type Page } from "@playwright/test";
import { diagnoseAn, geheZu, oeffneModellwahl, oeffneProfilmenue } from "./profilmenue";

// Suite-Rahmen nach ADR-0027 (Variante A): schlanke Fussleiste (24 px) mit
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
    const listeners = new Map<number, { event: string; handler: number }>();
    let callback = 0;
    const calls: [string, unknown][] = [];
    (window as unknown as { __calls: typeof calls }).__calls = calls;
    const updateState = window as unknown as {
      __LV_REC?: boolean;
      __LV_ONLINE?: boolean;
      __LV_ONLINE_VERSION?: string;
      __LV_INSTALL_FAIL?: boolean;
      __LV_UPDATED?: string;
      __lvEmit: (event: string, payload: unknown) => void;
    };
    updateState.__lvEmit = (event, payload) => {
      for (const [id, entry] of listeners) {
        if (entry.event === event) {
          (callbacks.get(entry.handler) as (value: unknown) => void)?.({ event, id, payload });
        }
      }
    };
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
          if (cmd === "get_app_settings" || cmd === "get_default_settings") return { ...settings, update_checks_enabled: updateState.__LV_ONLINE ?? false };
          if (cmd === "plugin:os|locale") return "de-DE";
          if (cmd === "plugin:app|version") return "0.21.30";
          if (cmd === "local_update_check") {
            const found = (window as unknown as { __LV_UPDATE?: Record<string, unknown> }).__LV_UPDATE;
            const version = localStorage.getItem("test.localVersion");
            return found ? version ? { ...found, version, notes: "- Neuer" } : found : null;
          }
          if (cmd.includes("permission")) return true;
          if (cmd === "plugin:event|listen") {
            const id = ++callback;
            listeners.set(id, { event: args?.event as string, handler: args?.handler as number });
            return id;
          }
          if (cmd === "plugin:event|unlisten") { listeners.delete(args?.eventId as number); return null; }
          if (cmd === "update_gate_status") return { belegt: updateState.__LV_REC ?? false };
          if (cmd === "update_verlauf") return { spaeter: JSON.parse(localStorage.getItem("test.update_spaeter") ?? "null"), aktualisiert_auf: updateState.__LV_UPDATED ?? null };
          if (cmd === "update_spaeter") {
            localStorage.setItem("test.update_spaeter", JSON.stringify(args));
            return null;
          }
          if (cmd === "update_install_begin") {
            calls.push([cmd, args]);
            if (updateState.__LV_REC) throw { code: "belegt", grund: "Diktat" };
            return null;
          }
          if (cmd === "update_install_abort" || cmd === "update_install_commit" || cmd === "plugin:process|restart") { calls.push([cmd, args]); return null; }
          if (cmd === "plugin:updater|check") {
            calls.push([cmd, args]);
            return updateState.__LV_ONLINE_VERSION ? { rid: 1, currentVersion: "0.21.30", version: updateState.__LV_ONLINE_VERSION, body: "- Synthetische Aenderung", rawJson: {} } : null;
          }
          if (cmd === "plugin:updater|download") {
            calls.push([cmd, { rid: args?.rid }]);
            const channel = args?.onEvent as { onmessage: (value: unknown) => void };
            channel.onmessage({ event: "Started", data: { contentLength: 10 } });
            channel.onmessage({ event: "Progress", data: { chunkLength: 10 } });
            channel.onmessage({ event: "Finished" });
            return 2;
          }
          if (cmd === "plugin:updater|install") {
            calls.push([cmd, args]);
            if (updateState.__LV_INSTALL_FAIL) throw new Error("update_test_failed");
            return null;
          }
          if (cmd === "get_selected_model") return "";
          if (cmd === "meetings_is_recording" || cmd === "is_recording")
            return (window as unknown as { __LV_REC?: boolean }).__LV_REC ?? false;
          if (cmd === "local_update_install") { calls.push([cmd, args]); return null; }
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

test("Fussleiste: 24 px, Modelle des Moduls links, Version ganz rechts", async ({ page }) => {
  await setup(page, "light");
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  // lva#394: Start ist der Chat (Antwortmodell im Chatkopf); die Fußleiste mit
  // Sprach- und KI-Modell zeigt eine Seite ohne eigene Modellwahl, z. B. Verlauf.
  await page
    .getByRole("navigation", { name: "Hauptnavigation" })
    .getByRole("button", { name: "Verlauf", exact: true })
    .click();
  const bar = page.locator(".lv-statusbar");
  expect((await bar.boundingBox())!.height).toBe(24);
  expect(await bar.evaluate((el) => getComputedStyle(el).fontSize)).toBe("12px");
  for (const b of await bar.locator("button.lv-status-btn").all())
    expect((await b.boundingBox())!.height).toBe(22);

  // Patrick 02.11. (löst ADR-0027 E7 ab): die Modelle des geöffneten Moduls
  // stehen links unten. Patrick 05.11. (#281): die Version immer ganz rechts.
  await expect(bar.getByTestId("fuss-modelle")).toHaveAttribute("data-modul", "history");
  await expect(bar.locator("[data-llm-selector]")).toHaveCount(1);
  await expect(bar.locator("[data-model-status]")).toHaveCount(1);
  const version = bar.getByTestId("footer-version");
  await expect(version).toHaveText("v0.21.30");
  await expect(version).toHaveAccessibleName("v0.21.30, Nach Updates suchen");
  await expect(version).toHaveAttribute("title", "Nach Updates suchen");
  const barBox = (await bar.boundingBox())!;
  const versionBox = (await version.boundingBox())!;
  expect(barBox.x + barBox.width - (versionBox.x + versionBox.width)).toBeLessThan(16);

  // Mikrofon: nur Symbol und Pfeil, Name im Tooltip
  const mic = bar.locator("[data-mic-selector]");
  await expect(mic).toHaveAttribute("aria-label", /Mikrofon: Systemvorgabe/);
  await expect(mic).toHaveText("");

  // Codex 01.11.: rechts Mikrofon und Telefon; Speicher/GPU und Fish-Server
  // nur als bewusst eingeschaltete Diagnose, einklappbar. Das Schild steht
  // immer da, ganz rechts vor der Version (Patrick 08.11.2026).
  await expect(bar.locator("[data-resource-meter]")).toHaveCount(0);
  await expect(bar.locator("[data-compliance-shield]")).toBeVisible();
  await expect(bar.locator("[data-fish-server]")).toHaveCount(0);
  await expect(bar.getByTestId("statusbar-diagnose")).toHaveCount(0);
  const x = async (sel: string) => (await bar.locator(sel).first().boundingBox())!.x;
  expect(await x("[data-mic-selector]")).toBeLessThan(await x(".lv-call-group"));
  // Patrick 08.11.2026: Schild ganz rechts, nach dem Telefon und direkt vor der Version.
  expect(await x(".lv-call-group")).toBeLessThan(await x("[data-compliance-shield]"));
  expect(await x("[data-compliance-shield]")).toBeLessThan(versionBox.x);

  // Profilmenü ▸ Hilfe ▸ Diagnose: Knopf erscheint, ausgeklappt mit Speicher
  await oeffneProfilmenue(page);
  const menue = page.locator(".wai-konto-menue");
  await menue.getByRole("button", { name: "Hilfe", exact: true }).click();
  const schalter = menue.locator('[data-profilmenue="diagnose"]');
  await schalter.click();
  await expect(schalter).toHaveAttribute("aria-pressed", "true");
  await page.keyboard.press("Escape");
  const diagnose = bar.getByTestId("statusbar-diagnose");
  await expect(diagnose).toHaveAttribute("aria-expanded", "true");
  await expect(bar.locator("[data-resource-meter]")).toBeVisible();
  // Einklappen behält nur den Knopf
  await diagnose.click();
  await expect(diagnose).toHaveAttribute("aria-expanded", "false");
  await expect(bar.locator("[data-resource-meter]")).toHaveCount(0);
  await expect(bar.locator("[data-compliance-shield]")).toBeVisible();
  await expect(bar.locator("[data-fish-server]")).toHaveCount(0);
  // Eingeklappt bleibt eingeklappt (auch nach Neustart), Ausklappen holt alles zurück
  await page.reload();
  await expect(bar.getByTestId("statusbar-diagnose")).toHaveAttribute("aria-expanded", "false");
  await expect(bar.locator("[data-resource-meter]")).toHaveCount(0);
  await bar.getByTestId("statusbar-diagnose").click();
  await expect(bar.locator("[data-resource-meter]")).toBeVisible();
  await expect(bar.locator("[data-compliance-shield]")).toBeVisible();
  // Abschalten im Menü entfernt auch den Knopf
  await oeffneProfilmenue(page);
  await menue.getByRole("button", { name: "Hilfe", exact: true }).click();
  await menue.locator('[data-profilmenue="diagnose"]').click();
  await page.keyboard.press("Escape");
  await expect(bar.getByTestId("statusbar-diagnose")).toHaveCount(0);
  await expect(bar.locator("[data-resource-meter]")).toHaveCount(0);
});

test("Fußleiste ▸ Modelle: Rollen-Tooltips und Modellwahl", async ({ page }) => {
  await setup(page, "light");
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  // lva#394: Start ist der Chat (Antwortmodell im Chatkopf); die Fußleiste mit
  // Sprach- und KI-Modell zeigt eine Seite ohne eigene Modellwahl, z. B. Verlauf.
  await page
    .getByRole("navigation", { name: "Hauptnavigation" })
    .getByRole("button", { name: "Verlauf", exact: true })
    .click();
  await oeffneModellwahl(page);
  const modelle = page.getByTestId("fuss-modelle");
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
  await diagnoseAn(page);
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

test("Titelleiste ohne Werkzeuge, Profilmenü unten links (ADR-0027)", async ({ page }) => {
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
  for (const name of ["Benachrichtigungen", "Modelle", "Integrationen", "Einstellungen", "Hilfe"])
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

test("Modulwerkzeuge in der Titelleiste: ⓘ nur Kurzbeschreibung, Suche je Modul, kein Hilfe-Knopf", async ({ page }) => {
  await setup(page, "light");
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  const nav = page.getByRole("navigation", { name: "Hauptnavigation" });
  const isMac = await page.evaluate(() => /Mac|iPhone|iPad/.test(navigator.platform));

  // Verlauf: Suche an, Strg+K springt hinein
  await nav.getByRole("button", { name: "Verlauf", exact: true }).click();
  // Keine Titelzeile im Modul (Patrick 02.11.): ⓘ und Suche stehen in der
  // Titelleiste, der Name nur noch unsichtbar als h1 und in der Navigation.
  const kopf = page.locator(".lv-titlebar .page-shell__head");
  await expect(kopf).toBeVisible();
  await expect(page.locator("#page-title")).toHaveText("Verlauf");
  await expect(page.locator("#page-title")).toHaveClass(/sr-only/);
  await expect(page.locator(".page-shell__description")).toHaveCount(0);
  // Titel-ⓘ: nur die Kurzbeschreibung, als Beschreibung am Knopf verknüpft
  const info = kopf.getByTestId("page-info");
  await info.hover();
  const tip = page.getByRole("tooltip");
  await expect(tip).toHaveText("Texte wiederfinden, kopieren und Aufnahmen anhören.");
  await expect(info).toHaveAccessibleDescription("Texte wiederfinden, kopieren und Aufnahmen anhören.");
  await expect(tip.locator("a, button")).toHaveCount(0);
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

test("Keine Titelzeile im Modul: Inhalt beginnt direkt unter der Titelleiste", async ({ page }) => {
  await setup(page, "light");
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  const nav = page.getByRole("navigation", { name: "Hauptnavigation" });
  for (const label of ["Verlauf", "Aufnahmen", "Vorlesen", "Bibliothek"]) {
    await nav.getByRole("button", { name: label, exact: true }).click();
    const titel = page.getByRole("heading", { level: 1, name: label, exact: true });
    await expect(titel).toHaveCount(1);
    // Nur für Screenreader: 1 px, ohne eigene Zeile.
    const box = (await titel.boundingBox())!;
    expect(box.height).toBeLessThanOrEqual(1);
    // Im Inhalt steht keine Werkzeugzeile mehr; sie liegt in der Titelleiste.
    await expect(page.locator(".lv-content .page-shell__head")).toHaveCount(0);
  }
});

for (const theme of ["dark", "light"] as const) {
  test(`Abnahmebild Rahmen (${theme})`, async ({ page }) => {
    await setup(page, theme);
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto("/");
    // Standard (Codex 01.11.): Statusleiste nur Mikrofon und Telefon.
    await page.screenshot({ path: `${SHOTS}/${theme}-1440-rahmen.png`, animations: "disabled" });
    // Modelle je Modul stehen links in der Fußleiste (Patrick 02.11.).
    await oeffneModellwahl(page);
    const leiste = (await page.locator(".lv-statusbar").boundingBox())!;
    for (const knopf of await page.getByTestId("fuss-modelle").locator("button.lv-status-btn").all()) {
      const b = (await knopf.boundingBox())!;
      expect(b.y).toBeGreaterThanOrEqual(leiste.y - 1);
      expect(b.y + b.height).toBeLessThanOrEqual(leiste.y + leiste.height + 1);
    }
    await page.screenshot({ path: `${SHOTS}/${theme}-1440-fussleiste-modelle.png`, animations: "disabled" });
    await page.getByRole("navigation", { name: "Hauptnavigation" }).getByRole("button", { name: "Vorlesen", exact: true }).click();
    await expect(page.locator("#page-title")).toHaveText("Vorlesen");
    await page.screenshot({ path: `${SHOTS}/${theme}-1440-vorlesen.png`, animations: "disabled" });
    await page.getByRole("navigation", { name: "Hauptnavigation" }).getByRole("button", { name: "Start", exact: true }).click();
    await page.setViewportSize({ width: 390, height: 844 });
    // Auch schmal: Version sichtbar ganz rechts, auf der Höhe der einzeiligen Leiste.
    const schmal = (await page.locator(".lv-statusbar").boundingBox())!;
    const v = (await page.locator(".lv-statusbar").getByTestId("footer-version").boundingBox())!;
    expect(v.y).toBeGreaterThanOrEqual(schmal.y - 1);
    expect(v.y + v.height).toBeLessThanOrEqual(schmal.y + schmal.height + 1);
    expect(v.x + v.width).toBeLessThanOrEqual(390);
    expect(v.x + v.width).toBeGreaterThan(390 - 16);
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
      // ADR-0027 E4: Hilfe nur über F1 bzw. Profilmenü ▸ Hilfe
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

// Version in der Fußleiste (#281, Codex zu lva#282): Fokus sichtbar,
// Update-Angebot mit ↑, Touchziel ≥ 44 px. Bilder nach LVA_SHOTS.
const fussClip = async (page: Page) => {
  const b = (await page.locator(".lv-statusbar").boundingBox())!;
  const w = page.viewportSize()!.width;
  const breite = Math.min(w, 700);
  return { x: w - breite, y: Math.max(0, b.y - 8), width: breite, height: b.height + 8 };
};
const angebot = (page: Page) =>
  page.addInitScript(() => {
    (window as unknown as { __LV_UPDATE: unknown }).__LV_UPDATE = {
      version: "0.23.0", path: "C:/Updates/local-voice_0.23.0.exe", file_name: "local-voice_0.23.0.exe", notes: "- Version wieder rechts",
    };
  });

for (const theme of ["light", "dark"] as const) {
  test(`Fußleiste ▸ Version: Fokus und Update-Angebot (${theme})`, async ({ page }) => {
    await setup(page, theme);
    await angebot(page);
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto("/");
    // Das Angebot meldet sich einmal von selbst; „Später“ lässt es in der Leiste stehen.
    await page.getByRole("button", { name: "Später" }).first().click();
    const version = page.locator(".lv-statusbar").getByTestId("footer-version");
    await expect(version).toContainText("v0.21.30");
    await expect(version).toContainText("↑");
    await expect(version).toHaveAccessibleName("v0.21.30, Version 0.23.0 ist verfügbar – klicken zum Installieren");
    await expect(version).toHaveAttribute("title", "Version 0.23.0 ist verfügbar – klicken zum Installieren");
    // Hervorgehoben: Akzentschrift statt der Farbe der Nachbarknöpfe.
    const farbe = await version.evaluate((el) => getComputedStyle(el).color);
    const mikro = await page.locator(".lv-statusbar [data-mic-selector]").evaluate((el) => getComputedStyle(el).color);
    expect(farbe).not.toBe(mikro);
    await page.screenshot({ path: `${SHOTS}/version-angebot-${theme}-1440.png`, clip: await fussClip(page), animations: "disabled" });

    // Tastaturfokus: sichtbarer Rahmen um die Version.
    await page.keyboard.press("Shift");
    await version.focus();
    expect(await version.evaluate((el) => getComputedStyle(el).outlineStyle)).toBe("solid");
    await page.screenshot({ path: `${SHOTS}/version-fokus-${theme}-1440.png`, clip: await fussClip(page), animations: "disabled" });

    await page.setViewportSize({ width: 390, height: 844 });
    const v = (await version.boundingBox())!;
    expect(v.x + v.width).toBeLessThanOrEqual(390);
    await page.screenshot({ path: `${SHOTS}/version-angebot-${theme}-390.png`, clip: await fussClip(page), animations: "disabled" });
  });
}

test.describe("Fußleiste ▸ Version mit Touch", () => {
  test.use({ hasTouch: true, isMobile: true, viewport: { width: 390, height: 844 } });
  for (const theme of ["light", "dark"] as const) {
    test(`Touchziel ≥ 44 px (${theme})`, async ({ page }) => {
      await setup(page, theme);
      await page.goto("/");
      expect(await page.evaluate(() => matchMedia("(pointer: coarse)").matches)).toBe(true);
      const version = page.locator(".lv-statusbar").getByTestId("footer-version");
      await expect(version).toHaveText("v0.21.30");
      const b = (await version.boundingBox())!;
      console.log(`touchziel ${theme} 390: ${b.width.toFixed(1)} x ${b.height.toFixed(1)} px`);
      expect(b.height).toBeGreaterThanOrEqual(44);
      expect(b.width).toBeGreaterThanOrEqual(44);
      expect(b.x + b.width).toBeLessThanOrEqual(390);
      await page.screenshot({ path: `${SHOTS}/version-touch-${theme}-390.png`, clip: await fussClip(page), animations: "disabled" });
    });
  }
});

// lva#869 (Hotfix 0.29.1): während einer Aufnahme geht das Update-Fenster nicht von selbst auf,
// und „Installieren“ startet den Installer erst, wenn die Aufnahme endet.
test("Update-Angebot: kein Fenster und keine Installation während einer Aufnahme", async ({ page }) => {
  await setup(page, "light");
  await angebot(page);
  await page.addInitScript(() => {
    (window as unknown as { __LV_REC: boolean }).__LV_REC = true;
  });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  const version = page.locator(".lv-statusbar").getByTestId("footer-version");
  await expect(version).toContainText("↑");
  await expect(page.getByTestId("update-install")).toHaveCount(0);

  // Auf Wunsch öffnet es sich trotzdem; „Installieren“ wartet aber auf das Ende der Aufnahme.
  await version.click();
  await page.getByTestId("update-install").click();
  await page.waitForTimeout(1500);
  expect((await callsOf(page)).filter(([c]) => c === "local_update_install")).toHaveLength(0);
  await page.evaluate(() => {
    (window as unknown as { __LV_REC: boolean }).__LV_REC = false;
    (window as unknown as { __lvEmit: (event: string, payload: unknown) => void }).__lvEmit("lv-update-gate", { belegt: false });
  });
  await expect
    .poll(async () => (await callsOf(page)).filter(([c]) => c === "local_update_install").length, { timeout: 10_000 })
    .toBe(1);
});

test("Update H3: 30-Minuten-Pruefung, Fokusfrist und Aufwachen", async ({ page }) => {
  await setup(page, "light");
  await page.addInitScript(() => { Object.assign(window, { __LV_ONLINE: true }); });
  await page.clock.install();
  await page.goto("/");
  const checks = async () => (await callsOf(page)).filter(([c]) => c === "plugin:updater|check").length;
  await expect.poll(checks).toBe(1);
  await page.clock.runFor(29 * 60_000);
  expect(await checks()).toBe(1);
  await page.clock.runFor(60_000);
  await expect.poll(checks).toBe(2);
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  expect(await checks()).toBe(2);
  await page.clock.runFor(5 * 60_000);
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect.poll(checks).toBe(3);
  await page.clock.fastForward(3 * 60_000);
  await expect.poll(checks).toBe(4);
});

test("Update H3: Spaeter ueberlebt Neustart, neuere Version fragt sofort", async ({ page }) => {
  await setup(page, "light");
  await angebot(page);
  await page.clock.install();
  await page.goto("/");
  await page.getByRole("button", { name: "Später", exact: true }).first().click();
  await expect.poll(() => page.evaluate(() => !!localStorage.getItem("test.update_spaeter"))).toBe(true);
  await page.reload();
  await expect(page.getByTestId("footer-version")).toContainText("↑");
  await expect(page.getByTestId("update-install")).toHaveCount(0);
  // Manuell bleibt dieselbe Version erreichbar.
  await page.getByTestId("footer-version").click();
  await expect(page.getByTestId("update-install")).toBeVisible();
  await page.getByRole("button", { name: "Später", exact: true }).first().click();
  await page.evaluate(() => localStorage.setItem("test.localVersion", "0.24.0"));
  await page.reload();
  await expect(page.getByTestId("update-install")).toBeVisible();
  await expect(page.getByTestId("update-notes")).toContainText("Neuer");
});

test("Update H3: Aufschub endet nach vier Stunden", async ({ page }) => {
  await setup(page, "light");
  await angebot(page);
  const now = Date.now();
  await page.clock.install({ time: now });
  await page.clock.pauseAt(now + 1000);
  await page.goto("/");
  await page.getByRole("button", { name: "Später", exact: true }).first().click();
  await expect.poll(() => page.evaluate(() => !!localStorage.getItem("test.update_spaeter"))).toBe(true);
  const bis = await page.evaluate(() => JSON.parse(localStorage.getItem("test.update_spaeter")!).bis as number);
  await page.clock.setSystemTime(bis - 60_000);
  await page.reload();
  await expect(page.getByTestId("footer-version")).toContainText("↑");
  await expect(page.getByTestId("update-install")).toHaveCount(0);
  // Ohne Onlinepruefungen und ohne erneutes Laden muss allein die Frist wecken.
  // Gegenprobe: ohne Aufschub-Timer bleibt update-install auch nach der Frist aus.
  await page.clock.runFor(59_999);
  await expect(page.getByTestId("update-install")).toHaveCount(0);
  await page.clock.runFor(1);
  await expect(page.getByTestId("update-install")).toBeVisible();
});

test("Update H3: Installation wartet bei verlorenem Frei-Event hoechstens 30 Sekunden", async ({ page }) => {
  await setup(page, "light");
  await angebot(page);
  await page.addInitScript(() => { Object.assign(window, { __LV_REC: true }); });
  const now = Date.now();
  await page.clock.install({ time: now });
  await page.clock.pauseAt(now + 1000);
  await page.goto("/");
  await page.getByTestId("footer-version").click();
  await page.getByTestId("update-install").click();
  await expect.poll(async () => (await callsOf(page)).filter(([c]) => c === "update_install_begin").length).toBe(1);
  // Gegenprobe: ohne Timeout-Nachpruefung kommt hier nie local_update_install.
  await page.evaluate(() => { Object.assign(window, { __LV_REC: false }); });
  await page.clock.runFor(29_999);
  expect((await callsOf(page)).some(([c]) => c === "local_update_install")).toBe(false);
  await page.clock.runFor(1);
  await expect.poll(async () => (await callsOf(page)).filter(([c]) => c === "local_update_install").length).toBe(1);
});

test("Update H3: vorgemerktes Angebot erscheint beim Frei-Event", async ({ page }) => {
  await setup(page, "light");
  await angebot(page);
  await page.addInitScript(() => { Object.assign(window, { __LV_REC: true }); });
  await page.goto("/");
  await expect(page.getByTestId("footer-version")).toContainText("↑");
  await expect(page.getByTestId("update-install")).toHaveCount(0);
  await page.evaluate(() => {
    const state = window as unknown as { __LV_REC: boolean; __lvEmit: (event: string, payload: unknown) => void };
    state.__LV_REC = false;
    state.__lvEmit("lv-update-gate", { belegt: false });
  });
  await expect(page.getByTestId("update-install")).toBeVisible();
});

for (const fail of [false, true]) {
  test(`Update H3: Download vor Gate, begin vor install, Fehlerfreigabe ${fail}`, async ({ page }) => {
    await setup(page, "light");
    await page.addInitScript((fail) => { Object.assign(window, { __LV_ONLINE: true, __LV_ONLINE_VERSION: "0.31.0", __LV_REC: true, __LV_INSTALL_FAIL: fail }); }, fail);
    await page.goto("/");
    await expect(page.getByTestId("footer-version")).toContainText("↑");
    await page.getByTestId("footer-version").click();
    await page.getByTestId("update-install").click();
    await expect.poll(async () => (await callsOf(page)).some(([c]) => c === "update_install_begin")).toBe(true);
    expect((await callsOf(page)).some(([c]) => c === "plugin:updater|install")).toBe(false);
    await page.evaluate(() => {
      const state = window as unknown as { __LV_REC: boolean; __lvEmit: (event: string, payload: unknown) => void };
      state.__LV_REC = false;
      state.__lvEmit("lv-update-gate", { belegt: false });
    });
    await expect.poll(async () => (await callsOf(page)).some(([c]) => c === (fail ? "update_install_abort" : "plugin:process|restart"))).toBe(true);
    const calls = (await callsOf(page)).map(([c]) => c);
    expect(calls.indexOf("plugin:updater|download")).toBeLessThan(calls.indexOf("update_install_begin"));
    expect(calls.lastIndexOf("update_install_begin")).toBeLessThan(calls.indexOf("update_install_commit"));
    expect(calls.indexOf("update_install_commit")).toBeLessThan(calls.indexOf("plugin:updater|install"));
    expect(calls.includes("plugin:process|restart")).toBe(!fail);
  });
}

test("Update H3: geaenderte Version verlangt neue Bestaetigung", async ({ page }) => {
  await setup(page, "light");
  await page.addInitScript(() => { Object.assign(window, { __LV_ONLINE: true, __LV_ONLINE_VERSION: "0.31.0" }); });
  await page.goto("/");
  await expect(page.getByTestId("update-install")).toBeVisible();
  await page.evaluate(() => { (window as unknown as { __LV_ONLINE_VERSION: string }).__LV_ONLINE_VERSION = "0.32.0"; });
  await page.getByTestId("update-install").click();
  await expect(page.getByRole("dialog")).toContainText("0.32.0");
  expect((await callsOf(page)).some(([c]) => c === "plugin:updater|download" || c === "update_install_begin")).toBe(false);
});

test("Update H3: aktualisierte Version steht fuer die Anzeige bereit", async ({ page }) => {
  await setup(page, "light");
  await page.addInitScript(() => {
    Object.assign(window, { __LV_UPDATED: "0.30.0" });
    window.addEventListener("lv-update-installed", (event) => {
      Object.assign(window, { __installedDetail: (event as CustomEvent).detail });
    });
  });
  await page.goto("/");
  await expect(page.locator('[data-aktualisiert-auf="0.30.0"]')).toHaveCount(1);
  expect(await page.evaluate(() => (window as unknown as { __installedDetail: unknown }).__installedDetail)).toEqual({ version: "0.30.0" });
});
