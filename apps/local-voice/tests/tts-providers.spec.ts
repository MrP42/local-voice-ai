import { test, expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

// Goal tts-provider, Paket P4: Gruppe "Sprachausgabe-Anbieter" im Reiter
// Ausgabe. Liste mit Stufe/Zustand/Gerät/Lizenz, Freigabe-Ablauf (Aktivieren,
// Lizenzdialog mit Fingerabdruck, Fernzugriff), gesperrte Anbieter nennen den
// Grund, Probestimme mit Ereignissen (auch `started` vor der Antwort) und
// Abbruch. Dazu: das Stimmen-Dropdown der Vorlesen-Seite bleibt heil, wenn
// `tts_engine = provider:<id>` gilt. Die Attrappe führt den Zustand der
// Anbieter selbst (Freigaben wirken wie im Backend).
//
// Anbieterdaten: `tests/fixtures/tts-providers-real.json` ist die UNVERAENDERTE
// Ausgabe von `local-voice-ai.exe --tts-providers --json` (26.10.2026). Die
// Attrappe kopiert sie und aendert je Test nur ZUSTAND (enabled, license_acked,
// remote, health.available/reason/hint/version, Fingerabdruck), nie Geraet,
// Streaming-Art oder Lizenzangaben. Echte Werte: Phonon = CPU, satzweise;
// Piper und Phonon sind nicht lizenzverifiziert; echtes Streaming hat nur
// openai-speech (Art "chunk").

type Over = {
  enabled?: boolean;
  license_acked?: boolean;
  remote?: boolean;
  fingerprint?: string;
  health?: Record<string, unknown>;
};
type Cfg = { engine: string; over?: Record<string, Over> };
type Arg = { cfg: Cfg; real: unknown[] };

const REAL = JSON.parse(
  readFileSync(
    resolve(process.cwd(), "tests", "fixtures", "tts-providers-real.json"),
    "utf8",
  ),
) as unknown[];

async function setup(page: Page, cfg: Cfg) {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.setViewportSize({ width: 1100, height: 1500 });
  const initArg: Arg = { cfg, real: REAL };
  await page.addInitScript((arg: Arg) => {
    const { cfg, real } = arg;
    const callbacks = new Map<number, unknown>();
    let callback = 0;
    const w = window as unknown as Record<string, unknown>;
    w.__calls = [] as unknown[];
    const listeners: Record<string, number[]> = {};
    w.__emit = (name: string, payload: unknown) =>
      (listeners[name] ?? []).forEach((id) =>
        (callbacks.get(id) as (e: unknown) => void)?.({
          event: name,
          id: 0,
          payload,
        }),
      );

    // Echte Anbieterdaten kopieren, nur Zustand abwandeln.
    const entries = (JSON.parse(JSON.stringify(real)) as any[]).map((e) => {
      const o = cfg.over?.[e.descriptor.id] ?? {};
      const { health: healthOver, ...state } = o;
      Object.assign(e, state);
      if (healthOver) Object.assign(e.health, healthOver);
      e.selected =
        cfg.engine === e.descriptor.id ||
        cfg.engine === `provider:${e.descriptor.id}`;
      return e;
    });
    // Wie das Backend: aus Freigaben folgt die Sperre.
    const recompute = (e: any) => {
      if (e.descriptor.opt_in && !e.enabled)
        e.blocked = {
          kind: "license_blocked",
          retryable: false,
          message: "Der Anbieter ist nicht freigeschaltet.",
        };
      else if (e.license_needs_ack && !e.license_acked)
        e.blocked = {
          kind: "license_blocked",
          retryable: false,
          message: "Lizenz nicht bestätigt.",
        };
      else if (e.remote && !e.allow_remote)
        e.blocked = {
          kind: "privacy_blocked",
          retryable: false,
          message: "Fernzugriff nicht erlaubt.",
        };
      else e.blocked = null;
    };
    entries.forEach((e) => {
      if (cfg.over?.[e.descriptor.id]) recompute(e);
    });
    w.__entries = entries;
    w.__previewReject = null;
    w.__previewEarly = true;
    const configs: Record<string, unknown> = {
      phonon: {
        exe_path: "ptts",
        model: "hf://example/phonon-31m-en",
        quant: "q8",
        lang: "de",
        voice: null,
        threads: null,
        hf_home: null,
        allow_network: false,
      },
      "openai-speech": {
        id: "openai-speech",
        label: "OpenAI-kompatibler Sprachserver",
        base_url: "http://example.org:8880/v1",
        model: "tts-1",
        voice: null,
        sample_rate: 24000,
        languages: ["und"],
        allow_remote: false,
        api_key_env: null,
        license: null,
      },
    };
    const err = (kind: string, message: string) => ({
      kind,
      retryable: false,
      message,
    });
    const find = (id: unknown) =>
      entries.find((e) => e.descriptor.id === id) as any;
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
      post_process_providers: [],
      post_process_prompts: [],
      custom_words: [],
      post_process_models: {},
      post_process_api_keys: {},
      push_to_talk: true,
      tts_voice: null,
      tts_engine: cfg.engine,
      tts_piper_voice: null,
      tts_fish_dir: "C:\\AI\\fish-speech",
    };
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
          (w.__calls as unknown[]).push([cmd, args]);
          const copy = <T>(x: T): T => JSON.parse(JSON.stringify(x));
          if (cmd === "get_app_settings" || cmd === "get_default_settings")
            return copy(settings);
          if (cmd === "plugin:os|locale") return "de-DE";
          if (cmd === "plugin:app|version") return "0.21.19";
          if (cmd.includes("permission")) return true;
          if (cmd === "plugin:event|listen") {
            const name = String(args?.event);
            (listeners[name] ??= []).push(Number(args?.handler));
            return ++callback;
          }
          if (cmd === "get_selected_model") return "";
          if (cmd === "meetings_is_recording") return false;
          if (cmd === "tts_server_status")
            return { phase: "stopped", message: null };
          if (cmd === "tts_runtime_status")
            return {
              platform: "windows-x64",
              piper: { supported: true, ready: true, missing: [] },
              fish: { supported: true, ready: true, missing: [] },
              fish_dir: "C:\\AI\\fish-speech",
            };
          if (
            cmd === "tts_list_voices" ||
            cmd === "tts_list_voice_infos" ||
            cmd === "tts_list_downloads"
          )
            return [];
          if (cmd === "pages_list") return [{ id: "p1", title: "Seite" }];
          if (cmd === "get_custom_sounds") return { start: false, stop: false };

          // --- Anbieter ---
          if (cmd === "tts_providers_list") return copy(entries);
          if (cmd === "tts_provider_health") {
            const e = find(args?.id);
            e.health = {
              ...e.health,
              checked_ms: (e.health.checked_ms ?? 0) + 1,
            };
            return copy(e.health);
          }
          if (cmd === "tts_provider_voices") {
            const e = find(args?.id);
            if (e.blocked) throw e.blocked;
            const all = [
              {
                id: "anna",
                label: "Anna",
                language: "de",
                gender: "female",
                license: null,
              },
              {
                id: "ben",
                label: "Ben",
                language: "en",
                gender: "male",
                license: {
                  code: "MIT",
                  weights: "CC BY-NC 4.0",
                  non_commercial: true,
                  verified: false,
                  source_url: "",
                  verified_on: null,
                  note: "",
                },
              },
            ];
            const lang = args?.language as string | null;
            return copy(lang ? all.filter((v) => v.language === lang) : all);
          }
          if (cmd === "tts_provider_get_config") {
            const c = configs[String(args?.id)];
            if (!c)
              throw err(
                "bad_request",
                "Dieser Anbieter hat keine Konfiguration.",
              );
            return JSON.stringify(c);
          }
          if (cmd === "tts_provider_set_config") {
            const parsed = JSON.parse(String(args?.json));
            if (String(parsed.base_url ?? "").startsWith("ftp:"))
              throw err(
                "invalid_config",
                "Die Adresse muss mit http:// oder https:// beginnen.",
              );
            configs[String(args?.id)] = parsed;
            return null;
          }
          if (cmd === "tts_provider_set_enabled") {
            const e = find(args?.id);
            e.enabled = args?.enabled === true;
            recompute(e);
            return null;
          }
          if (cmd === "tts_provider_acknowledge_license") {
            const e = find(args?.id);
            if (args?.fingerprint !== e.fingerprint)
              throw err(
                "bad_request",
                "Modell oder Lizenzangaben haben sich geändert.",
              );
            e.license_acked = true;
            recompute(e);
            return {
              provider_id: e.descriptor.id,
              fingerprint: e.fingerprint,
              acked_at: "2026-10-26T10:00:00Z",
            };
          }
          if (cmd === "tts_provider_set_allow_remote") {
            const e = find(args?.id);
            e.allow_remote = args?.allow === true;
            recompute(e);
            return null;
          }
          if (cmd === "tts_provider_select") {
            const e = find(args?.id);
            if (e.blocked) throw e.blocked;
            entries.forEach((x: any) => (x.selected = false));
            e.selected = true;
            e.voice = (args?.voice as string | null) ?? null;
            settings.tts_engine =
              e.descriptor.id === "fish" || e.descriptor.id === "piper"
                ? e.descriptor.id
                : `provider:${e.descriptor.id}`;
            return null;
          }
          if (cmd === "tts_provider_preview") {
            if (w.__previewReject) throw w.__previewReject;
            const requestId = "req-1";
            // `started` kommt VOR der Antwort des Befehls.
            if (w.__previewEarly)
              (w.__emit as (n: string, p: unknown) => void)(
                "tts-provider-preview",
                {
                  request_id: requestId,
                  phase: "started",
                  ttfa_ms: null,
                  error_kind: null,
                  message: null,
                },
              );
            return requestId;
          }
          if (cmd === "tts_provider_cancel") return true;
          if (
            cmd.includes("history") ||
            cmd.includes("models") ||
            cmd.includes("devices") ||
            cmd === "meetings_list" ||
            cmd === "llm_ps" ||
            cmd === "tts_reading_list" ||
            cmd === "page_files"
          )
            return [];
          return null;
        },
      },
    });
    try {
      localStorage.setItem("lva.ui.settings.tab", "output");
    } catch {
      /* egal */
    }
  }, initArg);
}

type Call = [string, Record<string, unknown> | undefined];
const calls = (page: Page, cmd: string) =>
  page.evaluate(
    (c) =>
      ((window as unknown as { __calls: Call[] }).__calls ?? []).filter(
        ([name]) => name === c,
      ),
    cmd,
  );
const emit = (page: Page, payload: Record<string, unknown>) =>
  page.evaluate(
    (p) =>
      (window as unknown as { __emit: (n: string, p: unknown) => void }).__emit(
        "tts-provider-preview",
        p,
      ),
    payload,
  );

const openGroup = async (page: Page) => {
  await page.goto("/");
  await page
    .getByRole("button", { name: "Einstellungen", exact: true })
    .click();
  await expect(page.getByTestId("tts-provider-fish")).toBeVisible();
};

test("Liste zeigt Stufe, Zustand, Gerät, Version, Streaming, Klonen und Lizenz je Anbieter", async ({
  page,
}) => {
  await setup(page, {
    engine: "fish",
    over: { piper: { health: { version: "2023.11" } } },
  });
  await openGroup(page);

  const fish = page.getByTestId("tts-provider-fish");
  await expect(fish).toContainText("Fish Speech");
  await expect(page.getByTestId("tts-provider-tier-fish")).toHaveText(
    "Unterstützt",
  );
  await expect(page.getByTestId("tts-provider-selected-fish")).toHaveText(
    "Liest vor",
  );
  await expect(page.getByTestId("tts-provider-device-fish")).toContainText(
    "CUDA",
  );
  await expect(page.getByTestId("tts-provider-cloning-fish")).toHaveText("ja");
  // Fish: Lizenz NICHT verifiziert, deutlich benannt.
  await expect(page.getByTestId("tts-provider-verified-fish")).toContainText(
    "nicht verifiziert",
  );

  const piper = page.getByTestId("tts-provider-piper");
  await expect(piper).toContainText("2023.11");
  await expect(piper).toContainText("satzweise");
  await expect(page.getByTestId("tts-provider-cloning-piper")).toHaveText(
    "nein",
  );
  // Echte Daten: auch Piper ist (noch) nicht lizenzverifiziert.
  await expect(page.getByTestId("tts-provider-verified-piper")).toContainText(
    "nicht verifiziert",
  );
  await expect(page.getByTestId("tts-provider-device-piper")).toContainText(
    "CPU",
  );

  const phonon = page.getByTestId("tts-provider-phonon");
  await expect(page.getByTestId("tts-provider-tier-phonon")).toHaveText(
    "Experimentell",
  );
  // Echte Daten: Phonon läuft auf der CPU und liefert satzweise.
  await expect(phonon).toContainText("satzweise");
  await expect(phonon).not.toContainText("echtes Streaming");
  await expect(page.getByTestId("tts-provider-device-phonon")).toContainText(
    "CPU",
  );
  await expect(
    page.getByTestId("tts-provider-device-phonon"),
  ).not.toContainText("CUDA");
  // Echtes Streaming (Art "chunk") hat nur der OpenAI-kompatible Sprachserver.
  await expect(page.getByTestId("tts-provider-openai-speech")).toContainText(
    "echtes Streaming",
  );
  await expect(
    page.getByTestId("tts-provider-noncommercial-phonon"),
  ).toHaveText("nichtkommerziell");
  await expect(page.getByTestId("tts-provider-selected-phonon")).toHaveCount(0);

  // Nicht verfügbar: Grund und Hinweis aus der Gesundheitsprüfung.
  const reason = page.getByTestId("tts-provider-reason-ollama-speech");
  await expect(reason).toContainText("Ollama ist nicht erreichbar");
  await expect(reason).toContainText("Ollama starten oder die Adresse prüfen.");
  await expect(
    page.getByTestId("tts-provider-health-ollama-speech"),
  ).toHaveText("Nicht bereit");

  // Fish/Piper ohne Opt-in: kein Schalter. Phonon: Schalter.
  await expect(page.getByTestId("tts-provider-enable-fish")).toHaveCount(0);
  await expect(page.getByTestId("tts-provider-enable-phonon")).toHaveCount(1);

  // Erneut prüfen ruft die Gesundheit dieses Anbieters.
  await page.getByTestId("tts-provider-recheck-piper").click();
  await expect
    .poll(async () => (await calls(page, "tts_provider_health")).length)
    .toBe(1);
  expect((await calls(page, "tts_provider_health"))[0][1]).toEqual({
    id: "piper",
  });
});

test("Aktivieren führt über den Lizenzdialog (Fingerabdruck) bis zur Freigabe", async ({
  page,
}) => {
  await setup(page, { engine: "fish" });
  await openGroup(page);

  // Vor der Freigabe: gesperrt, Grund steht da, Auswahl nicht möglich.
  const blocked = page.getByTestId("tts-provider-blocked-phonon");
  await expect(blocked).toContainText("nicht aktiviert");

  await page.getByTestId("tts-provider-enable-phonon").click();
  await expect
    .poll(async () => (await calls(page, "tts_provider_set_enabled")).length)
    .toBe(1);
  expect((await calls(page, "tts_provider_set_enabled"))[0][1]).toEqual({
    id: "phonon",
    enabled: true,
  });

  // Lizenzdialog mit Zusammenfassung, Quell-Link, ausdrücklicher Bestätigung.
  const dialog = page.getByTestId("tts-provider-license-dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText("Phonon 40m: CC BY-NC 4.0");
  await expect(dialog).toContainText("nichtkommerzielle Nutzung");
  await expect(dialog).toContainText("nicht verifiziert");
  await expect(dialog.getByTestId("tts-provider-license-source")).toBeVisible();
  const confirm = page.getByTestId("tts-provider-license-confirm");
  await expect(confirm).toBeDisabled();
  await page.getByTestId("tts-provider-license-checkbox").check();
  await expect(confirm).toBeEnabled();
  await confirm.click();

  await expect
    .poll(
      async () =>
        (await calls(page, "tts_provider_acknowledge_license")).length,
    )
    .toBe(1);
  expect((await calls(page, "tts_provider_acknowledge_license"))[0][1]).toEqual(
    {
      id: "phonon",
      fingerprint: "53dccea71bf676cb6875f3bf9be8f058",
    },
  );
  await expect(dialog).toHaveCount(0);
  // Freigegeben: keine Sperre mehr.
  await expect(page.getByTestId("tts-provider-blocked-phonon")).toHaveCount(0);
  // Kein Fernzugriffs-Dialog für einen lokalen Anbieter.
  await expect(page.getByTestId("tts-provider-remote-dialog")).toHaveCount(0);
});

test("Fernzugriff: Aktivieren eines entfernten Anbieters verlangt die Freigabe mit Warnung", async ({
  page,
}) => {
  // Zustand: Lizenz schon bestätigt, Server steht auf einem fremden Rechner.
  await setup(page, {
    engine: "fish",
    over: { "openai-speech": { license_acked: true, remote: true } },
  });
  await openGroup(page);

  await page.getByTestId("tts-provider-enable-openai-speech").click();
  // Lizenz ist bestätigt: kein Lizenzdialog, aber Fernzugriff.
  const dialog = page.getByTestId("tts-provider-remote-dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText("verlässt dann diesen Rechner");
  await page.getByTestId("tts-provider-remote-confirm").click();
  await expect
    .poll(
      async () => (await calls(page, "tts_provider_set_allow_remote")).length,
    )
    .toBe(1);
  expect((await calls(page, "tts_provider_set_allow_remote"))[0][1]).toEqual({
    id: "openai-speech",
    allow: true,
  });
  await expect(dialog).toHaveCount(0);
  await expect(
    page.getByTestId("tts-provider-blocked-openai-speech"),
  ).toHaveCount(0);
  await expect(
    page.getByTestId("tts-provider-remote-allowed-openai-speech"),
  ).toBeVisible();
  // Zurücknehmen.
  await page.getByTestId("tts-provider-revoke-remote-openai-speech").click();
  await expect(
    page.getByTestId("tts-provider-blocked-openai-speech"),
  ).toContainText("außerhalb dieses Rechners");
});

test("Veraltete Lizenzbestätigung (bad_request) bleibt im Dialog und lädt die Liste neu", async ({
  page,
}) => {
  await setup(page, { engine: "fish" });
  await openGroup(page);
  await page.getByTestId("tts-provider-enable-phonon").click();
  const dialog = page.getByTestId("tts-provider-license-dialog");
  await expect(dialog).toBeVisible();
  // Der Stand ändert sich zwischen Anzeige und Klick.
  await page.evaluate(() => {
    const e = (
      window as unknown as {
        __entries: { descriptor: { id: string }; fingerprint: string }[];
      }
    ).__entries.find((x) => x.descriptor.id === "phonon")!;
    e.fingerprint = "0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f";
  });
  await page.getByTestId("tts-provider-license-checkbox").check();
  const listCallsBefore = (await calls(page, "tts_providers_list")).length;
  await page.getByTestId("tts-provider-license-confirm").click();
  await expect(dialog).toContainText("haben sich geändert");
  await expect
    .poll(async () => (await calls(page, "tts_providers_list")).length)
    .toBeGreaterThan(listCallsBefore);
  // Neu bestätigen mit dem neuen Fingerabdruck.
  await expect(page.getByTestId("tts-provider-license-confirm")).toBeDisabled();
  await page.getByTestId("tts-provider-license-checkbox").check();
  await page.getByTestId("tts-provider-license-confirm").click();
  await expect(dialog).toHaveCount(0);
  const acks = await calls(page, "tts_provider_acknowledge_license");
  expect(acks[acks.length - 1][1]).toEqual({
    id: "phonon",
    fingerprint: "0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f",
  });
});

test("Gesperrter Anbieter nennt, was fehlt, und lässt sich nicht als Stimme wählen", async ({
  page,
}) => {
  await setup(page, { engine: "fish" });
  await openGroup(page);

  // Lizenz fehlt (aktiviert, aber nicht bestätigt): Weg zur Bestätigung.
  await page.evaluate(() => {
    const e = (
      window as unknown as {
        __entries: {
          descriptor: { id: string };
          enabled: boolean;
          blocked: unknown;
        }[];
      }
    ).__entries.find((x) => x.descriptor.id === "phonon")!;
    e.enabled = true;
    e.blocked = {
      kind: "license_blocked",
      retryable: false,
      message: "Lizenz nicht bestätigt.",
    };
  });
  await page.getByTestId("tts-providers-recheck-all").click();
  const blocked = page.getByTestId("tts-provider-blocked-phonon");
  await expect(blocked).toContainText("Lizenz ist noch nicht bestätigt");
  await expect(
    page.getByTestId("tts-provider-open-license-phonon"),
  ).toBeVisible();

  // Entfernter Anbieter ohne Opt-in: zuerst fehlt die Aktivierung.
  const remote = page.getByTestId("tts-provider-blocked-openai-speech");
  await expect(remote).toHaveAttribute("data-kind", "license_blocked");
  await expect(remote).toContainText("nicht aktiviert");

  // Aufgeklappt: Stimmen sind erst nach Freigabe abrufbar, Auswahl und Probe aus.
  await page.getByTestId("tts-provider-toggle-phonon").click();
  const details = page.getByTestId("tts-provider-details-phonon");
  await expect(details).toContainText("erst nach der Freigabe abrufbar");
  await expect(
    page.getByTestId("tts-provider-preview-play-phonon"),
  ).toBeDisabled();
  expect(await calls(page, "tts_provider_voices")).toHaveLength(0);
});

test("Einstellungen: Felder aus der Konfiguration, Freigaben nicht als Feld, Validierungsfehler sichtbar", async ({
  page,
}) => {
  await setup(page, { engine: "fish" });
  await openGroup(page);
  await page.getByTestId("tts-provider-toggle-openai-speech").click();
  const cfg = page.getByTestId("tts-provider-config-openai-speech");
  await expect(
    cfg.getByTestId("tts-provider-openai-speech-base_url"),
  ).toHaveValue("http://example.org:8880/v1");
  await expect(
    cfg.getByTestId("tts-provider-openai-speech-sample_rate"),
  ).toHaveValue("24000");
  // Freigaben und Identität sind keine Eingaben.
  await expect(
    cfg.getByTestId("tts-provider-openai-speech-allow_remote"),
  ).toHaveCount(0);
  await expect(cfg.getByTestId("tts-provider-openai-speech-id")).toHaveCount(0);

  await cfg
    .getByTestId("tts-provider-openai-speech-base_url")
    .fill("ftp://example.org");
  await page.getByTestId("tts-provider-config-save-openai-speech").click();
  await expect(
    page.getByTestId("tts-provider-config-error-openai-speech"),
  ).toContainText("Die Adresse muss mit http:// oder https:// beginnen.");

  await cfg
    .getByTestId("tts-provider-openai-speech-base_url")
    .fill("http://127.0.0.1:8880/v1");
  await cfg.getByTestId("tts-provider-openai-speech-sample_rate").fill("22050");
  await page.getByTestId("tts-provider-config-save-openai-speech").click();
  await expect(cfg).toContainText("Gespeichert.");
  const saves = await calls(page, "tts_provider_set_config");
  const last = saves[saves.length - 1][1] as { id: string; json: string };
  expect(last.id).toBe("openai-speech");
  const sent = JSON.parse(last.json);
  expect(sent.base_url).toBe("http://127.0.0.1:8880/v1");
  expect(sent.sample_rate).toBe(22050);
  expect(sent.languages).toEqual(["und"]);

  // Anbieter ohne Konfiguration (Piper): freundlicher Hinweis statt Fehler.
  await page.getByTestId("tts-provider-toggle-piper").click();
  await expect(page.getByTestId("tts-provider-config-piper")).toContainText(
    "keine eigenen Einstellungen",
  );
});

test("Stimme und Sprache wählen, als Vorlese-Stimme verwenden", async ({
  page,
}) => {
  await setup(page, { engine: "fish" });
  await openGroup(page);
  await page.getByTestId("tts-provider-toggle-piper").click();
  const voices = page.getByTestId("tts-provider-voices-piper");
  await expect(voices).toBeVisible();
  // (Im Entwicklungsmodus mountet React Effekte doppelt: nach Sprache zählen.)
  const voiceCalls = async (language: string | null) =>
    (await calls(page, "tts_provider_voices")).filter(
      ([, a]) => a?.language === language,
    );
  await expect
    .poll(async () => (await voiceCalls(null)).length)
    .toBeGreaterThanOrEqual(1);
  expect((await voiceCalls(null))[0][1]).toEqual({
    id: "piper",
    language: null,
  });

  await voices.getByLabel("Sprache", { exact: true }).click();
  await page.getByRole("option", { name: "Englisch" }).click();
  await expect
    .poll(async () => (await voiceCalls("en")).length)
    .toBeGreaterThanOrEqual(1);
  expect((await voiceCalls("en"))[0][1]).toEqual({
    id: "piper",
    language: "en",
  });

  await voices.getByLabel("Stimme", { exact: true }).click();
  await page.getByRole("option", { name: /Ben/ }).click();
  await expect(voices).toContainText("CC BY-NC 4.0");
  await page.getByTestId("tts-provider-use-piper").click();
  await expect
    .poll(async () => (await calls(page, "tts_provider_select")).length)
    .toBe(1);
  expect((await calls(page, "tts_provider_select"))[0][1]).toEqual({
    id: "piper",
    voice: "ben",
  });
  await expect(page.getByTestId("tts-provider-selected-piper")).toBeVisible();
  await expect(page.getByTestId("tts-provider-selected-fish")).toHaveCount(0);
});

test("Probestimme: Phase, Zeit bis zum ersten Ton, Abbrechen (started kommt vor der Antwort)", async ({
  page,
}) => {
  await setup(page, { engine: "fish" });
  await openGroup(page);
  await page.getByTestId("tts-provider-toggle-piper").click();
  const text = page.getByTestId("tts-provider-preview-text-piper");
  await expect(text).toHaveValue(/Guten Tag\. Dies ist ein kurzer Probesatz/);
  await page.getByTestId("tts-provider-preview-play-piper").click();

  await expect
    .poll(async () => (await calls(page, "tts_provider_preview")).length)
    .toBe(1);
  expect((await calls(page, "tts_provider_preview"))[0][1]).toMatchObject({
    id: "piper",
    voice: "en_US-ryan-high", // echte Daten: Piper hat bereits eine Stimme gewählt
  });
  const status = page.getByTestId("tts-provider-preview-status-piper");
  await expect(status).toHaveAttribute("data-phase", "started");

  await emit(page, {
    request_id: "req-1",
    phase: "first_audio",
    ttfa_ms: 187,
    error_kind: null,
    message: null,
  });
  await expect(status).toContainText("erster Ton nach 187 ms");
  // Ereignisse fremder Aufträge zählen nicht.
  await emit(page, {
    request_id: "req-other",
    phase: "done",
    ttfa_ms: null,
    error_kind: null,
    message: null,
  });
  await expect(status).toHaveAttribute("data-phase", "first_audio");

  await page.getByTestId("tts-provider-preview-cancel-piper").click();
  await expect
    .poll(async () => (await calls(page, "tts_provider_cancel")).length)
    .toBe(1);
  expect((await calls(page, "tts_provider_cancel"))[0][1]).toEqual({
    requestId: "req-1",
  });
  await emit(page, {
    request_id: "req-1",
    phase: "cancelled",
    ttfa_ms: null,
    error_kind: null,
    message: null,
  });
  await expect(status).toContainText("Abgebrochen.");
  await expect(
    page.getByTestId("tts-provider-preview-cancel-piper"),
  ).toHaveCount(0);
  // Wieder bedienbar.
  await expect(
    page.getByTestId("tts-provider-preview-play-piper"),
  ).toBeEnabled();
});

test("Probestimme: Fehler beim Start (bad_request: Vorlesen läuft) und Fehler-Ereignis sind verständlich", async ({
  page,
}) => {
  await setup(page, { engine: "fish" });
  await openGroup(page);
  await page.getByTestId("tts-provider-toggle-piper").click();
  await page.evaluate(() => {
    (window as unknown as { __previewReject: unknown }).__previewReject = {
      kind: "bad_request",
      retryable: false,
      message:
        "Das Vorlesen läuft gerade. Erst anhalten, dann die Probestimme abspielen.",
    };
  });
  await page.getByTestId("tts-provider-preview-play-piper").click();
  const error = page.getByTestId("tts-provider-preview-error-piper");
  await expect(error).toContainText("Anfrage abgelehnt");
  await expect(error).toContainText("Das Vorlesen läuft gerade.");
  await expect(
    page.getByTestId("tts-provider-preview-cancel-piper"),
  ).toHaveCount(0);

  // Fehler während der Wiedergabe kommt als Ereignis.
  await page.evaluate(() => {
    (window as unknown as { __previewReject: unknown }).__previewReject = null;
  });
  await page.getByTestId("tts-provider-preview-play-piper").click();
  await expect(
    page.getByTestId("tts-provider-preview-status-piper"),
  ).toHaveAttribute("data-phase", "started");
  await emit(page, {
    request_id: "req-1",
    phase: "error",
    ttfa_ms: null,
    error_kind: "timeout",
    message: "Der Anbieter hat nicht rechtzeitig geantwortet.",
  });
  await expect(error).toContainText("Zeitüberschreitung");
  await expect(error).toContainText("nicht rechtzeitig");
});

test("Probestimme: started und first_audio vor der Befehlsantwort werden zugeordnet", async ({
  page,
}) => {
  await setup(page, { engine: "fish" });
  await openGroup(page);
  await page.getByTestId("tts-provider-toggle-piper").click();
  await page.evaluate(() => {
    const w = window as unknown as {
      __emit: (n: string, p: unknown) => void;
      __previewEarly: boolean;
    };
    // Der Befehl liefert jetzt nach beiden Ereignissen: wir senden first_audio
    // gleich im Anschluss an `started` aus der Attrappe heraus.
    const original = w.__emit;
    w.__emit = (name, payload) => {
      original(name, payload);
      const p = payload as { phase?: string; request_id?: string };
      if (name === "tts-provider-preview" && p.phase === "started")
        original(name, {
          request_id: p.request_id,
          phase: "first_audio",
          ttfa_ms: 95,
          error_kind: null,
          message: null,
        });
    };
  });
  await page.getByTestId("tts-provider-preview-play-piper").click();
  const status = page.getByTestId("tts-provider-preview-status-piper");
  await expect(status).toHaveAttribute("data-phase", "first_audio");
  await expect(status).toContainText("95 ms");
});

test("Vorlesen-Seite: tts_engine = provider:phonon zeigt benannten Eintrag statt Fish und stürzt nicht ab", async ({
  page,
}) => {
  await setup(page, { engine: "provider:phonon" });
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  const select = page.getByTestId("voice-select");
  await expect(select).toBeVisible();
  await expect(select).toContainText("Anbieter: Phonon");
  await expect(select).not.toContainText("Skript mit Sprechern");
  const hint = page.getByTestId("voice-provider-hint");
  await expect(hint).toContainText(
    "Anbieter: Phonon (ptts) (Einstellungen → Sprachausgabe-Anbieter)",
  );
  // Kein Fish-"nicht eingerichtet"-Hinweis.
  await expect(page.getByTestId("voice-setup-hint")).toHaveCount(0);
  // Der Link führt in die Gruppe.
  await page.getByTestId("voice-provider-open").click();
  await expect(page.getByTestId("tts-provider-phonon")).toBeVisible();
  await expect(page.getByTestId("tts-provider-selected-phonon")).toBeVisible();
});

test("Vorlesen-Seite: bei Fish bleibt alles wie zuvor (kein Anbieter-Hinweis)", async ({
  page,
}) => {
  await setup(page, { engine: "fish" });
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await expect(page.getByTestId("voice-select")).toContainText(
    "Skript mit Sprechern",
  );
  await expect(page.getByTestId("voice-provider-hint")).toHaveCount(0);
});

test("Bild der Gruppe (nur mit SCREENS_DIR)", async ({ page }) => {
  const dir = process.env.SCREENS_DIR;
  test.skip(!dir, "nur mit SCREENS_DIR");
  // Zustand: Phonon läuft (Version 0.4.2, bereit); Gerät, Streaming und
  // Lizenzangaben bleiben die der echten Daten (CPU, satzweise, nicht verifiziert).
  await setup(page, {
    engine: "provider:phonon",
    over: {
      phonon: {
        health: {
          available: true,
          reason: null,
          hint: null,
          version: "0.4.2",
        },
      },
    },
  });
  // Hoch genug, damit die ganze Gruppe ohne Rollen auf ein Bild passt.
  await page.setViewportSize({ width: 1100, height: 2700 });
  await openGroup(page);
  // Phonon freigeben, aufklappen, Probestimme laufen lassen.
  await page.getByTestId("tts-provider-enable-phonon").click();
  await page.getByTestId("tts-provider-license-checkbox").check();
  await page.getByTestId("tts-provider-license-confirm").click();
  await expect(page.getByTestId("tts-provider-license-dialog")).toHaveCount(0);
  await page.getByTestId("tts-provider-toggle-phonon").click();
  await expect(page.getByTestId("tts-provider-voices-phonon")).toBeVisible();
  await page.getByTestId("tts-provider-preview-play-phonon").click();
  await emit(page, {
    request_id: "req-1",
    phase: "first_audio",
    ttfa_ms: 187,
    error_kind: null,
    message: null,
  });
  await expect(
    page.getByTestId("tts-provider-preview-status-phonon"),
  ).toContainText("187 ms");
  const group = page.locator('[data-settings-anchor="tts-providers"]');
  await group.screenshot({
    path: `${dir}/tts-anbieter-gruppe.png`,
    animations: "disabled",
  });

  // Lizenzdialog (zweiter Anbieter).
  await page.getByTestId("tts-provider-enable-ollama-speech").click();
  await expect(page.getByTestId("tts-provider-license-dialog")).toBeVisible();
  await page.getByRole("dialog").screenshot({
    path: `${dir}/tts-anbieter-lizenzdialog.png`,
    animations: "disabled",
  });
});
