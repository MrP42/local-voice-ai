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
  /** Gewaehlte Vorlese-Stimme des Anbieters. */
  voice?: string | null;
  /** Stimmen im Vorlesen-Dropdown (leer = alle). */
  visible_voices?: string[];
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
      e.visible_voices = e.visible_voices ?? [];
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
    w.__configs = configs;
    // Dateiwahl: die naechste Antwort steht in `__picks` (null = abgebrochen).
    w.__picks = [] as unknown[];
    // Was im Modellordner liegt (Pfad -> Antwort); sonst: f32 vorhanden.
    w.__modelInfo = {} as Record<string, unknown>;
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
      tts_providers: { voices: {} as Record<string, string> },
    };
    entries.forEach((e: any) => {
      if (e.voice)
        (settings.tts_providers as any).voices[e.descriptor.id] = e.voice;
    });
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
          // Hoerproben: Anna und Nele fertig, Ben entsteht noch.
          if (cmd === "tts_provider_voice_samples")
            return args?.id === "phonon"
              ? [
                  { voice_id: "anna", wav_path: "C:/proben/anna.wav" },
                  { voice_id: "nele", wav_path: "C:/proben/nele.wav" },
                  { voice_id: "ben", wav_path: null },
                ]
              : [];
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
                id: "nele",
                label:
                  "Nele – Rounded, open tone with gentle lift at the ends…",
                language: "de",
                gender: "female",
                license: {
                  code: "apache-2.0",
                  weights: "apache-2.0",
                  non_commercial: false,
                  verified: false,
                  source_url: "https://huggingface.co/gradium/phonon-voices",
                  verified_on: null,
                  note: "",
                },
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
            // Fuer Bilder: mehr Stimmen (`__voicesExtra`).
            all.push(...((w.__voicesExtra as typeof all | undefined) ?? []));
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
          if (cmd === "plugin:dialog|open") {
            const picks = w.__picks as unknown[];
            return picks.length > 0 ? picks.shift() : null;
          }
          if (cmd === "tts_provider_inspect_model_folder") {
            const known = (w.__modelInfo as Record<string, unknown>)[
              String(args?.path)
            ];
            return known ?? { is_dir: true, has_f32: true, has_q8: false };
          }
          if (cmd === "tts_provider_set_config") {
            const parsed = JSON.parse(String(args?.json));
            // Wie PhononConfig::validate: erstes Problem, Feld zuerst genannt.
            if (String(args?.id) === "phonon") {
              if (String(parsed.exe_path ?? "").trim() === "")
                throw err(
                  "invalid_config",
                  "exe_path fehlt: Pfad zur ptts-Programmdatei eintragen",
                );
              if (String(parsed.model ?? "").trim() === "")
                throw err(
                  "invalid_config",
                  "model fehlt: lokalen Modellordner oder hf://OWNER/MODEL@COMMIT angeben",
                );
              if (parsed.threads !== null && parsed.threads > 256)
                throw err(
                  "invalid_config",
                  `threads ${parsed.threads} liegt nicht in 1 bis 256`,
                );
            }
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
            const stored = (settings.tts_providers as any).voices;
            if (e.voice) stored[e.descriptor.id] = e.voice;
            else delete stored[e.descriptor.id];
            settings.tts_engine =
              e.descriptor.id === "fish" || e.descriptor.id === "piper"
                ? e.descriptor.id
                : `provider:${e.descriptor.id}`;
            return null;
          }
          if (cmd === "tts_provider_set_visible_voices") {
            const e = find(args?.id);
            e.visible_voices = [...((args?.voices as string[]) ?? [])];
            return null;
          }
          if (cmd === "tts_provider_voice_options") {
            // Wie das Backend: freigegebene Anbieter ausser Fish/Piper, nur die gewaehlten Stimmen.
            const out: unknown[] = [];
            for (const e of entries as any[]) {
              const tier = e.descriptor.tier;
              if (["fish", "piper"].includes(e.descriptor.id)) continue;
              if (tier !== "supported" && tier !== "experimental") continue;
              if (e.blocked) continue;
              const listed = (await (
                (w.__TAURI_INTERNALS__ as any).invoke as any
              )("tts_provider_voices", {
                id: e.descriptor.id,
                language: null,
              })) as any[];
              const voices = [
                {
                  id: "default",
                  label: "Standardstimme des Modells",
                  language: "de",
                  gender: null,
                  license: null,
                },
                ...listed,
              ];
              for (const v of voices) {
                const shown =
                  v.id === "default" ||
                  v.id === e.voice ||
                  e.visible_voices.length === 0 ||
                  e.visible_voices.includes(v.id);
                if (shown)
                  out.push({
                    provider_id: e.descriptor.id,
                    provider_label: e.descriptor.label,
                    voice: v,
                  });
              }
            }
            return copy(out);
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
  // Fish: Lizenz belegt (Fish Audio Research License) und nichtkommerziell.
  await expect(page.getByTestId("tts-provider-verified-fish")).toContainText(
    "verifiziert am 2026-10-26",
  );
  await expect(page.getByTestId("tts-provider-fish")).toContainText(
    "nichtkommerziell",
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
    "Anbieter: Phonon (Einstellungen → Sprachausgabe-Anbieter)",
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

// ---------------------------------------------------------------------------
// Phonon-Einstellungsmaske (Durchsuchen, Feldfehler, q8, Stand, Warnhinweis)
// ---------------------------------------------------------------------------

// addInitScript laeuft in der Reihenfolge der Anmeldung: nach `setup` anmelden.
const phonon = (page: Page, suffix: string) =>
  page.getByTestId(`tts-provider-phonon-${suffix}`);

test("Phonon: Durchsuchen wählt Programmdatei (Datei) und Modell (Ordner), q8 wird vorbelegt, Speichern gilt erst danach", async ({
  page,
}) => {
  await setup(page, { engine: "fish" });
  await page.addInitScript(() => {
    const w = window as unknown as {
      __configs: Record<string, any>;
      __picks: unknown[];
      __modelInfo: Record<string, unknown>;
    };
    Object.assign(w.__configs.phonon, {
      exe_path: "",
      model: "",
      quant: null,
    });
    w.__picks.push(
      "D:\\phonon\\bin\\ptts.exe",
      "D:\\phonon\\models\\phonon-40m-de",
    );
    w.__modelInfo["D:\\phonon\\models\\phonon-40m-de"] = {
      is_dir: true,
      has_f32: false,
      has_q8: true,
    };
  });
  await openGroup(page);
  await page.getByTestId("tts-provider-toggle-phonon").click();
  const cfg = page.getByTestId("tts-provider-config-phonon");
  await expect(phonon(page, "exe_path")).toHaveValue("");
  // Noch nichts geaendert: kein Hinweis auf ungespeicherte Aenderungen.
  await expect(
    page.getByTestId("tts-provider-config-dirty-phonon"),
  ).toHaveCount(0);

  // Programmdatei: Dateiwahl.
  await phonon(page, "exe_path-browse").click();
  await expect(phonon(page, "exe_path")).toHaveValue(
    "D:\\phonon\\bin\\ptts.exe",
  );
  // Modell: Ordnerwahl.
  await phonon(page, "model-browse").click();
  await expect(phonon(page, "model")).toHaveValue(
    "D:\\phonon\\models\\phonon-40m-de",
  );
  const dialogs = (await calls(page, "plugin:dialog|open")).map(
    ([, a]) => (a as { options: { directory: boolean } }).options.directory,
  );
  expect(dialogs).toEqual([false, true]);

  // Im Ordner liegt nur model.q8.gguf: q8 ist vorbelegt und erklaert.
  await expect(
    cfg.getByRole("combobox", { name: "Quantisierung" }),
  ).toBeVisible();
  await expect(cfg).toContainText("q8 eingetragen");
  await expect(
    page.getByTestId("tts-provider-config-dirty-phonon"),
  ).toContainText("Ungespeicherte Änderungen");
  // Bis hierher wurde nichts gespeichert.
  expect(await calls(page, "tts_provider_set_config")).toHaveLength(0);

  await page.getByTestId("tts-provider-config-save-phonon").click();
  await expect(cfg).toContainText("Gespeichert.");
  await expect(
    page.getByTestId("tts-provider-config-dirty-phonon"),
  ).toHaveCount(0);
  const saves = await calls(page, "tts_provider_set_config");
  expect(saves).toHaveLength(1);
  const sent = JSON.parse((saves[0][1] as { json: string }).json);
  expect(sent).toMatchObject({
    exe_path: "D:\\phonon\\bin\\ptts.exe",
    model: "D:\\phonon\\models\\phonon-40m-de",
    quant: "q8",
  });

  // Abbrechen im Dialog aendert nichts.
  await phonon(page, "model-browse").click();
  await expect(phonon(page, "model")).toHaveValue(
    "D:\\phonon\\models\\phonon-40m-de",
  );
});

test("Phonon: q8-Hinweis bei getipptem Modellordner (nur q8 vorhanden) mit Knopf", async ({
  page,
}) => {
  await setup(page, { engine: "fish" });
  await page.addInitScript(() => {
    const w = window as unknown as {
      __configs: Record<string, any>;
      __modelInfo: Record<string, unknown>;
    };
    Object.assign(w.__configs.phonon, {
      exe_path: "ptts",
      model: "D:\\nur-q8",
      quant: null,
    });
    w.__modelInfo["D:\\nur-q8"] = {
      is_dir: true,
      has_f32: false,
      has_q8: true,
    };
  });
  await openGroup(page);
  await page.getByTestId("tts-provider-toggle-phonon").click();
  const note = page.getByTestId("tts-provider-quant-note-phonon");
  await expect(note).toContainText("nur model.q8.gguf");
  await page.getByTestId("tts-provider-quant-use-phonon").click();
  await expect(note).toHaveCount(0);
  await page.getByTestId("tts-provider-config-save-phonon").click();
  const saves = await calls(page, "tts_provider_set_config");
  expect(JSON.parse((saves[0][1] as { json: string }).json).quant).toBe("q8");
});

test("Phonon: Fehler stehen am Feld, nichts wird gespeichert, die Anzeige oben erklärt den gespeicherten Stand", async ({
  page,
}) => {
  await setup(page, { engine: "fish" });
  await page.addInitScript(() => {
    const w = window as unknown as { __configs: Record<string, any> };
    Object.assign(w.__configs.phonon, { exe_path: "", model: "" });
  });
  await openGroup(page);

  // Oben: der GESPEICHERTE Stand, mit Verweis auf die Felder.
  const reason = page.getByTestId("tts-provider-reason-phonon");
  await expect(reason).toContainText("Gespeicherter Stand:");
  await expect(reason).toContainText("exe_path fehlt");
  await expect(
    page.getByTestId("tts-provider-reason-fix-phonon"),
  ).toContainText("erst danach gilt die Änderung");
  // Der Verweis oeffnet die Felder.
  await page.getByTestId("tts-provider-reason-open-phonon").click();
  await expect(page.getByTestId("tts-provider-config-phonon")).toBeVisible();
  await expect(page.getByTestId("tts-provider-reason-open-phonon")).toHaveCount(
    0,
  );

  // Nur die Programmdatei eingetragen, Modell leer: beide Pflichtfelder pruefen,
  // der Fehler steht am Modell, nichts wird gespeichert.
  await phonon(page, "exe_path").fill("C:\\ptts\\ptts.exe");
  await page.getByTestId("tts-provider-config-save-phonon").click();
  const modelError = page.getByTestId("tts-provider-field-error-phonon-model");
  await expect(modelError).toContainText("Pflichtfeld");
  await expect(
    page.getByTestId("tts-provider-field-error-phonon-exe_path"),
  ).toHaveCount(0);
  await expect(
    page.getByTestId("tts-provider-config-notsaved-phonon"),
  ).toContainText("Nicht gespeichert");
  await expect(phonon(page, "model")).toHaveAttribute("aria-invalid", "true");
  expect(await calls(page, "tts_provider_set_config")).toHaveLength(0);

  // Beide leer: beide Felder melden sich auf einmal.
  await phonon(page, "exe_path").fill("");
  await page.getByTestId("tts-provider-config-save-phonon").click();
  await expect(
    page.getByTestId("tts-provider-field-error-phonon-exe_path"),
  ).toContainText("Pflichtfeld");
  await expect(modelError).toContainText("Pflichtfeld");

  // Tippen raeumt den Fehler dieses Feldes weg.
  await phonon(page, "model").fill("D:\\modell");
  await expect(modelError).toHaveCount(0);

  // Fehlermeldung des Backends wird dem Feld zugeordnet (erstes Wort = Feld).
  await phonon(page, "exe_path").fill("ptts");
  await phonon(page, "threads").fill("999");
  await page.getByTestId("tts-provider-config-save-phonon").click();
  await expect(
    page.getByTestId("tts-provider-field-error-phonon-threads"),
  ).toContainText("threads 999 liegt nicht in 1 bis 256");
  await expect(
    page.getByTestId("tts-provider-config-error-phonon"),
  ).toHaveCount(0);
  expect(await calls(page, "tts_provider_set_config")).toHaveLength(1);
  await expect(
    page.getByTestId("tts-provider-config-dirty-phonon"),
  ).toBeVisible();
});

test("Gewählter Anbieter, der nicht bereit ist: Warnhinweis ohne Rückfall auf Fish oder Piper", async ({
  page,
}) => {
  // Phonon gewaehlt, aber nicht bereit (echte Daten: exe_path fehlt) und gesperrt.
  await setup(page, { engine: "provider:phonon" });
  await openGroup(page);
  const warn = page.getByTestId("tts-provider-selected-warning-phonon");
  await expect(page.getByTestId("tts-provider-selected-phonon")).toBeVisible();
  await expect(warn).toContainText("Liest derzeit nicht");
  await expect(warn).toContainText("nicht auf Fish oder Piper zurück");
  await expect(warn).toHaveAttribute("data-reason", "blocked");

  // Freigegeben, aber nicht bereit: anderer Grund, derselbe Hinweis auf den Fehler.
  await page.getByTestId("tts-provider-enable-phonon").click();
  await page.getByTestId("tts-provider-license-checkbox").check();
  await page.getByTestId("tts-provider-license-confirm").click();
  await expect(page.getByTestId("tts-provider-license-dialog")).toHaveCount(0);
  await expect(warn).toHaveAttribute("data-reason", "unavailable");
  await expect(warn).toContainText("bricht mit dieser Fehlermeldung ab");

  // Bereit: kein Warnhinweis.
  await page.evaluate(() => {
    const e = (
      window as unknown as {
        __entries: {
          descriptor: { id: string };
          health: Record<string, unknown>;
        }[];
      }
    ).__entries.find((x) => x.descriptor.id === "phonon")!;
    e.health = { ...e.health, available: true, reason: null, hint: null };
  });
  await page.getByTestId("tts-providers-recheck-all").click();
  await expect(warn).toHaveCount(0);
});

test("Phonon-Stimmen: Geschlecht in der Auswahl, Lizenz der Stimme mit Quelle", async ({
  page,
}) => {
  await setup(page, {
    engine: "fish",
    over: { phonon: { enabled: true, license_acked: true } },
  });
  await openGroup(page);
  await page.getByTestId("tts-provider-toggle-phonon").click();
  const voices = page.getByTestId("tts-provider-voices-phonon");
  await voices.getByLabel("Stimme", { exact: true }).click();
  await expect(
    page.getByRole("option", { name: /Nele – Rounded.*weiblich · Deutsch/ }),
  ).toBeVisible();
  await page.getByRole("option", { name: /Nele/ }).click();
  await expect(voices).toContainText("Stimme: Lizenz apache-2.0");
  await expect(voices).not.toContainText("nichtkommerziell");
  await expect(voices.getByTestId("tts-provider-license-source")).toBeVisible();
});

// ---- Sichtbare Stimmen im Vorlesen-Dropdown (27.10.2026) --------------------------------

const phononVisible = (page: Page) =>
  page.getByTestId("tts-provider-visible-phonon");

test("Phonon-Maske: Liste aller Stimmen mit Haken, Alle/Keine, gewählte Stimme bleibt gesetzt", async ({
  page,
}) => {
  await setup(page, {
    engine: "fish",
    over: { phonon: { enabled: true, license_acked: true } },
  });
  await openGroup(page);
  await page.getByTestId("tts-provider-toggle-phonon").click();
  const list = page.getByTestId("tts-provider-visible-list-phonon");
  await expect(list).toBeVisible();

  // Alle benannten Stimmen (die Standardstimme steht nicht in der Liste), alle angehakt.
  await expect(list.locator("[data-voice-id]")).toHaveCount(3);
  await expect(list.locator('[data-voice-id="default"]')).toHaveCount(0);
  const nele = list.locator('[data-voice-id="nele"]');
  await expect(nele).toContainText("Nele");
  await expect(nele).toContainText("weiblich");
  await expect(nele).toContainText("Rounded, open tone");
  await expect(list.locator('[data-voice-id="ben"]')).toContainText("männlich");
  for (const id of ["anna", "nele", "ben"])
    await expect(
      page.getByTestId(`tts-provider-visible-check-${id}`),
    ).toBeChecked();
  await expect(
    page.getByTestId("tts-provider-visible-count-phonon"),
  ).toHaveText("3 von 3 sichtbar");

  // Eine Stimme abwählen: die Auswahl der übrigen wird gespeichert.
  await page
    .getByTestId("tts-provider-visible-check-ben")

    .uncheck();
  await expect
    .poll(
      async () => (await calls(page, "tts_provider_set_visible_voices")).length,
    )
    .toBe(1);
  expect((await calls(page, "tts_provider_set_visible_voices"))[0][1]).toEqual({
    id: "phonon",
    voices: ["anna", "nele"],
  });
  await expect(
    page.getByTestId("tts-provider-visible-count-phonon"),
  ).toHaveText("2 von 3 sichtbar");

  // Wieder anhaken: alle sichtbar heißt leere Liste (auch für spätere Stimmen).
  await page
    .getByTestId("tts-provider-visible-check-ben")

    .check();
  await expect
    .poll(
      async () => (await calls(page, "tts_provider_set_visible_voices")).length,
    )
    .toBe(2);
  expect((await calls(page, "tts_provider_set_visible_voices"))[1][1]).toEqual({
    id: "phonon",
    voices: [],
  });

  // Keine: nur die Standardstimme in der Liste; Alle: leer.
  await page.getByTestId("tts-provider-visible-none-phonon").click();
  await expect(
    page.getByTestId("tts-provider-visible-count-phonon"),
  ).toHaveText("0 von 3 sichtbar");
  for (const id of ["anna", "nele", "ben"])
    await expect(
      page.getByTestId(`tts-provider-visible-check-${id}`),
    ).not.toBeChecked();
  expect((await calls(page, "tts_provider_set_visible_voices"))[2][1]).toEqual({
    id: "phonon",
    voices: ["default"],
  });
  await page.getByTestId("tts-provider-visible-all-phonon").click();
  await expect(
    page.getByTestId("tts-provider-visible-count-phonon"),
  ).toHaveText("3 von 3 sichtbar");

  // Die gewählte Vorlese-Stimme lässt sich nicht abwählen und bleibt bei "Keine" sichtbar.
  const voices = page.getByTestId("tts-provider-voices-phonon");
  await voices.getByLabel("Stimme", { exact: true }).click();
  await page.getByRole("option", { name: /Nele/ }).click();
  await page.getByTestId("tts-provider-use-phonon").click();
  await expect(page.getByTestId("tts-provider-selected-phonon")).toBeVisible();
  await expect(
    phononVisible(page).getByText("gewählt", { exact: true }),
  ).toBeVisible();
  const neleBox = page.getByTestId("tts-provider-visible-check-nele");
  await expect(neleBox).toBeDisabled();
  await page.getByTestId("tts-provider-visible-none-phonon").click();
  await expect(neleBox).toBeChecked();
  await expect(
    page.getByTestId("tts-provider-visible-count-phonon"),
  ).toHaveText("1 von 3 sichtbar");
});

test("Phonon-Maske: gesperrter Anbieter zeigt keine Auswahl; Fish und Piper haben keine", async ({
  page,
}) => {
  await setup(page, { engine: "fish" });
  await openGroup(page);
  await page.getByTestId("tts-provider-toggle-phonon").click();
  await expect(page.getByTestId("tts-provider-visible-phonon")).toHaveCount(0);
  await page.getByTestId("tts-provider-toggle-piper").click();
  await expect(page.getByTestId("tts-provider-voices-piper")).toBeVisible();
  await expect(page.getByTestId("tts-provider-visible-piper")).toHaveCount(0);
});

const gotoReadAloud = async (page: Page) => {
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  const select = page.getByTestId("voice-select");
  await expect(select).toBeVisible();
  return select;
};

test("Vorlesen-Dropdown: Phonon-Stimmen einzeln, nur die sichtbaren; Wahl setzt Anbieter und Stimme", async ({
  page,
}) => {
  await setup(page, {
    engine: "fish",
    over: {
      phonon: { enabled: true, license_acked: true, visible_voices: ["nele"] },
    },
  });
  const select = await gotoReadAloud(page);
  await select.getByRole("combobox").click();
  const option = (name: RegExp) => page.getByRole("option", { name });
  // Sichtbar: die Standardstimme und die ausgewählte Stimme, mit Anbieter und Geschlecht.
  await expect(option(/Phonon · Nele/)).toBeVisible();
  await expect(
    option(/Phonon · Standardstimme des Modells/),
  ).toBeVisible();
  // Nicht gewählt: fehlen.
  await expect(option(/Phonon · Anna/)).toHaveCount(0);
  await expect(option(/Phonon · Ben/)).toHaveCount(0);
  // Fish bleibt, wie es war.
  await expect(option(/Skript mit Sprechern/)).toBeVisible();

  await option(/Phonon · Nele/).click();
  await expect
    .poll(async () => (await calls(page, "tts_provider_select")).length)
    .toBe(1);
  expect((await calls(page, "tts_provider_select"))[0][1]).toEqual({
    id: "phonon",
    voice: "nele",
  });
  // Die Auswahl zeigt die gewählte Stimme statt eines Sammel-Eintrags.
  await expect(select).toContainText("Phonon · Nele");
});

test("Vorlesen-Dropdown: ohne gespeicherte Auswahl stehen alle Stimmen da", async ({
  page,
}) => {
  await setup(page, {
    engine: "fish",
    over: { phonon: { enabled: true, license_acked: true } },
  });
  const select = await gotoReadAloud(page);
  await select.getByRole("combobox").click();
  for (const name of ["Anna", "Nele", "Ben"])
    await expect(
      page.getByRole("option", {
        name: new RegExp(`Phonon · ${name}`),
      }),
    ).toBeVisible();
});

test("Vorlesen-Dropdown: die gewählte Stimme bleibt sichtbar, auch wenn die Auswahl sie nicht nennt", async ({
  page,
}) => {
  await setup(page, {
    engine: "provider:phonon",
    over: {
      phonon: {
        enabled: true,
        license_acked: true,
        voice: "nele",
        visible_voices: ["anna"],
      },
    },
  });
  const select = await gotoReadAloud(page);
  await expect(select).toContainText("Phonon · Nele");
  await select.getByRole("combobox").click();
  await expect(
    page.getByRole("option", { name: /Phonon · Anna/ }),
  ).toBeVisible();
  await expect(
    page.getByRole("option", { name: /Phonon · Ben/ }),
  ).toHaveCount(0);
});

test("Vorlesen-Dropdown: ohne Freigabe stehen keine Anbieter-Stimmen da", async ({
  page,
}) => {
  await setup(page, { engine: "fish" });
  const select = await gotoReadAloud(page);
  await select.getByRole("combobox").click();
  await expect(
    page.getByRole("option", { name: /Skript mit Sprechern/ }),
  ).toBeVisible();
  await expect(page.getByRole("option", { name: /Phonon/ })).toHaveCount(0);
});

test("Bild der Sichtbarkeitsliste (nur mit SCREENS_DIR)", async ({ page }) => {
  const dir = process.env.SCREENS_DIR;
  test.skip(!dir, "nur mit SCREENS_DIR");
  await setup(page, {
    engine: "provider:phonon",
    over: {
      phonon: {
        enabled: true,
        license_acked: true,
        voice: "nele",
        visible_voices: ["anna", "nele", "jette", "mats", "leon"],
      },
    },
  });
  await page.addInitScript(() => {
    const w = window as unknown as { __voicesExtra: unknown[] };
    const row = (
      id: string,
      label: string,
      gender: string,
      language = "de",
    ) => ({
      id,
      label,
      language,
      gender,
      license: null,
    });
    w.__voicesExtra = [
      row("jette", "Jette – Warm, clear voice with a friendly tone", "female"),
      row("annika", "Annika – Bright and lively, quick delivery", "female"),
      row("mats", "Mats – Calm, grounded and unhurried", "male"),
      row("leon", "Leon – Young, energetic and open", "male"),
      row("svea", "Svea – Soft, thoughtful and intimate", "female"),
      row(
        "Captain-Silas-Blackwater",
        "Captain Silas Blackwater – An old British pirate captain, deep and gravelly",
        null as unknown as string,
        "en",
      ),
    ];
  });
  await page.setViewportSize({ width: 1100, height: 1900 });
  await openGroup(page);
  await page.getByTestId("tts-provider-toggle-phonon").click();
  const voices = page.getByTestId("tts-provider-voices-phonon");
  await expect(
    page.getByTestId("tts-provider-visible-list-phonon"),
  ).toBeVisible();
  await expect(
    page.getByTestId("tts-provider-visible-count-phonon"),
  ).toHaveText("5 von 9 sichtbar");
  await voices.screenshot({
    path: `${dir}/phonon-sichtbare-stimmen.png`,
    animations: "disabled",
  });
});

test("Bild der Phonon-Maske (nur mit SCREENS_DIR)", async ({ page }) => {
  const dir = process.env.SCREENS_DIR;
  test.skip(!dir, "nur mit SCREENS_DIR");
  await setup(page, {
    engine: "provider:phonon",
    over: { phonon: { enabled: true, license_acked: true } },
  });
  await page.addInitScript(() => {
    const w = window as unknown as {
      __configs: Record<string, any>;
      __picks: unknown[];
      __modelInfo: Record<string, unknown>;
    };
    Object.assign(w.__configs.phonon, { exe_path: "", model: "", quant: null });
    w.__picks.push(
      "D:\\lv-build\\phonon\\bin\\ptts.exe",
      "D:\\lv-build\\phonon\\models\\phonon-40m-de",
    );
    w.__modelInfo["D:\\lv-build\\phonon\\models\\phonon-40m-de"] = {
      is_dir: true,
      has_f32: false,
      has_q8: true,
    };
  });
  await page.setViewportSize({ width: 1100, height: 2300 });
  await openGroup(page);
  const row = page.getByTestId("tts-provider-phonon");
  // 1) Gespeicherter Stand unvollstaendig, gewaehlt, nicht bereit.
  await row.screenshot({
    path: `${dir}/phonon-maske-1-stand.png`,
    animations: "disabled",
  });
  // 2) Aufgeklappt, Pflichtfelder leer gespeichert: Fehler am Feld.
  await page.getByTestId("tts-provider-toggle-phonon").click();
  await page.getByTestId("tts-provider-config-save-phonon").click();
  await expect(
    page.getByTestId("tts-provider-field-error-phonon-model"),
  ).toBeVisible();
  await row.screenshot({
    path: `${dir}/phonon-maske-2-fehler.png`,
    animations: "disabled",
  });
  // 3) Durchsuchen: Programm und Modell, q8 vorbelegt, ungespeichert.
  await phonon(page, "exe_path-browse").click();
  await phonon(page, "model-browse").click();
  await expect(
    page.getByTestId("tts-provider-config-dirty-phonon"),
  ).toBeVisible();
  await row.screenshot({
    path: `${dir}/phonon-maske-3-durchsuchen.png`,
    animations: "disabled",
  });
});

test("Phonon-Maske: Reinhoeren spielt die fertige Hoerprobe, eine entstehende ist gesperrt", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const w = window as unknown as { __played: string[] };
    w.__played = [];
    HTMLMediaElement.prototype.play = function () {
      w.__played.push(this.getAttribute("src") ?? this.src);
      return Promise.resolve();
    };
  });
  await setup(page, {
    engine: "fish",
    over: { phonon: { enabled: true, license_acked: true } },
  });
  await openGroup(page);
  await page.getByTestId("tts-provider-toggle-phonon").click();
  const nele = page.getByTestId("tts-provider-sample-nele");
  await expect(nele).toBeVisible();
  await expect(nele).toHaveAttribute("aria-label", "Reinhören: Nele");
  await nele.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as unknown as { __played: string[] }).__played),
    )
    .toEqual(["C:/proben/nele.wav"]);
  // Waehrend sie laeuft, stoppt derselbe Knopf.
  await expect(nele).toHaveAttribute("aria-label", "Hörprobe stoppen: Nele");
  // Ben: die Probe entsteht noch -- Kreisel, gesperrt; fertig meldet sie das Ereignis.
  const ben = page.getByTestId("tts-provider-sample-ben");
  await expect(ben).toBeDisabled();
  await page.evaluate(() =>
    (window as unknown as { __emit: (n: string, p: unknown) => void }).__emit(
      "tts-provider-voice-sample",
      { provider_id: "phonon", voice_id: "ben", wav_path: "C:/proben/ben.wav" },
    ),
  );
  await expect(ben).toBeEnabled();
  await page
    .getByTestId("tts-provider-visible-list-phonon")
    .screenshot({ path: "test-results/phonon-hoerproben-einstellungen.png" });
});
