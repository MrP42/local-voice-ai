import { test, expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

// Eigene Phonon-Stimmen (Einstellungen > Ausgabe > Sprachausgabe-Anbieter > Phonon):
// importieren, mit Gradium erstellen (Datei, Mikrofon, vorhandene Fish-Stimme),
// entfernen, Gradium-Schlüssel. Die Attrappe führt den Zustand der Stimmen und des
// Schlüssels selbst und zeichnet jeden Befehl auf (`__calls`): geprüft wird, WELCHE
// Befehle mit WELCHEN Argumenten gehen — vor allem, dass `phonon_voice_clone` (der
// einzige Netzzugriff) nie ohne Häkchen und Hinweis aufgerufen wird. Es gibt keinen
// echten Aufruf an api.gradium.ai.

type Arg = { real: unknown[]; key: boolean; fish: unknown[] };

const REAL = JSON.parse(
  readFileSync(
    resolve(process.cwd(), "tests", "fixtures", "tts-providers-real.json"),
    "utf8",
  ),
) as unknown[];

const SCREENS = process.env.SCREENS_DIR;

const FISH = [
  {
    id: "erzaehler",
    meta: { display_name: "Erzähler", color: "teal" },
    origin: { kind: "recording" },
    avatar_path: null,
  },
  {
    id: "anna",
    meta: { display_name: "Anna", color: "rose" },
    origin: { kind: "recording" },
    avatar_path: null,
  },
];

async function setup(
  page: Page,
  opts: { key?: boolean; fish?: unknown[] } = {},
) {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.setViewportSize({ width: 1100, height: 1500 });
  const initArg: Arg = {
    real: REAL,
    key: opts.key ?? true,
    fish: opts.fish ?? FISH,
  };
  await page.addInitScript((arg: Arg) => {
    const callbacks = new Map<number, unknown>();
    let callback = 0;
    const w = window as unknown as Record<string, any>;
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
    w.__voicesChanged = 0;
    window.addEventListener("lv-voices-changed", () => {
      w.__voicesChanged += 1;
    });

    const entries = (JSON.parse(JSON.stringify(arg.real)) as any[]).map((e) => {
      e.visible_voices = e.visible_voices ?? [];
      e.selected = e.descriptor.id === "fish";
      e.blocked = null;
      if (e.descriptor.id === "phonon") {
        e.enabled = true;
        e.license_acked = true;
      }
      return e;
    });

    // Zustand der eigenen Stimmen und des Schlüssels.
    w.__own = {
      voices: [] as any[],
      models: ["phonon-40m-de"],
      languages: ["en", "fr", "de", "es", "pt"],
      busy: false,
    };
    w.__key = { supported: true, present: arg.key, problem: null };
    w.__fish = arg.fish;
    w.__picks = [] as unknown[];
    // Verhalten von `phonon_voice_clone`: ok | unauthorized | slow
    w.__cloneMode = "ok";
    w.__release = null as null | ((v: string) => void);
    const err = (kind: string, message: string) => ({ kind, message });
    const idOf = (name: string) =>
      name
        .trim()
        .replace(/ü/g, "ue")
        .replace(/ä/g, "ae")
        .replace(/ö/g, "oe")
        .replace(/[^A-Za-z0-9_]+/g, "-");
    const add = (
      name: string,
      origin: string,
      extra: Record<string, unknown>,
    ) => {
      const voice = {
        id: idOf(name),
        name,
        origin,
        voice_id: "",
        language: "",
        created_at: "2026-10-27T10:00:00Z",
        consent_at: null,
        source_note: null,
        license: "eigene/unbekannt",
        verified: false,
        in_models: ["phonon-40m-de"],
        ...extra,
      };
      w.__own.voices = [
        ...w.__own.voices.filter((v: any) => v.id !== voice.id),
        voice,
      ];
      return voice;
    };

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
      tts_engine: "fish",
      tts_piper_voice: null,
      tts_fish_dir: "C:\\AI\\fish-speech",
      tts_providers: { voices: {} },
    };
    const configs: Record<string, unknown> = {
      phonon: {
        exe_path: "ptts",
        model: "C:\\Daten\\tts\\phonon\\models\\phonon-40m-de",
        quant: "q8",
        lang: "de",
        voice: null,
        threads: null,
        hf_home: null,
        allow_network: false,
      },
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
        invoke: async (cmd: string, args?: Record<string, any>) => {
          (w.__calls as unknown[]).push([cmd, args]);
          const copy = <T>(x: T): T => JSON.parse(JSON.stringify(x));
          if (cmd === "get_app_settings" || cmd === "get_default_settings")
            return copy(settings);
          if (cmd === "plugin:os|locale") return "de-DE";
          if (cmd === "plugin:app|version") return "0.21.22";
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
          if (cmd === "tts_list_voice_infos") return copy(w.__fish);
          if (cmd === "tts_list_voices" || cmd === "tts_list_downloads")
            return [];
          if (cmd === "pages_list") return [{ id: "p1", title: "Seite" }];
          if (cmd === "get_custom_sounds") return { start: false, stop: false };

          // --- Anbieter (nur, was die Phonon-Einstellungen brauchen) ---
          if (cmd === "tts_providers_list") return copy(entries);
          if (cmd === "tts_provider_health") {
            const e = entries.find((x: any) => x.descriptor.id === args?.id);
            return copy(e.health);
          }
          if (cmd === "tts_provider_voices") return [];
          if (cmd === "tts_provider_get_config")
            return JSON.stringify(configs[String(args?.id)]);
          if (cmd === "tts_provider_inspect_model_folder")
            return { is_dir: true, has_f32: false, has_q8: true };
          if (cmd === "tts_provider_voice_options") return [];
          if (cmd === "plugin:dialog|open") {
            const picks = w.__picks as unknown[];
            return picks.length > 0 ? picks.shift() : null;
          }

          // --- Eigene Phonon-Stimmen ---
          if (cmd === "phonon_own_voices_list") return copy(w.__own);
          if (cmd === "phonon_own_voice_import") {
            const path = String(args?.path ?? "");
            if (path.includes("kaputt"))
              throw err(
                "invalid_file",
                "Das ist keine safetensors-Datei (Kopf unlesbar).",
              );
            const id = idOf(String(args?.name));
            if (
              !args?.overwrite &&
              w.__own.voices.some((v: any) => v.id === id)
            )
              throw err("exists", "Es gibt schon eine eigene Stimme.");
            return copy(add(String(args?.name), "import", {}));
          }
          if (cmd === "phonon_own_voice_remove") {
            if (!w.__own.voices.some((v: any) => v.id === args?.id))
              throw err("not_own", "keine eigene Stimme");
            w.__own.voices = w.__own.voices.filter(
              (v: any) => v.id !== args?.id,
            );
            return null;
          }
          if (cmd === "phonon_voice_clone") {
            const req = args?.request;
            if (!req?.consent)
              throw err("consent_required", "Ohne Einwilligung geht nichts.");
            const phase = (p: string) =>
              w.__emit("phonon-voice-clone-phase", p);
            phase("uploading");
            if (w.__cloneMode === "unauthorized")
              throw err(
                "unauthorized",
                "Gradium lehnt den API-Schlüssel ab (HTTP 401). Schlüssel in den Phonon-Einstellungen prüfen oder neu eintragen.",
              );
            if (w.__cloneMode === "slow") {
              const outcome = await new Promise<string>((resolve) => {
                w.__release = resolve;
              });
              if (outcome === "cancel") throw err("cancelled", "Abgebrochen.");
            }
            phase("waiting");
            phase("downloading");
            const id = idOf(String(req.name));
            if (!req.overwrite && w.__own.voices.some((v: any) => v.id === id))
              throw err("exists", "Es gibt schon eine eigene Stimme.");
            return copy(
              add(String(req.name), "gradium-clone", {
                voice_id: "vox_9",
                language: req.language,
                consent_at: "2026-10-27T10:05:00Z",
                source_note:
                  req.source.kind === "fish"
                    ? `Fish-Stimme: ${req.source.voice_id}`
                    : req.source.kind === "recording"
                      ? "Mikrofonaufnahme"
                      : "Audiodatei",
              }),
            );
          }
          if (cmd === "phonon_voice_clone_cancel") {
            w.__release?.("cancel");
            return null;
          }
          if (cmd === "phonon_gradium_key_status") return copy(w.__key);
          if (cmd === "phonon_gradium_key_set") {
            if (String(args?.key).trim().length < 8)
              throw err("key_store", "Der Schlüssel sieht nicht richtig aus.");
            w.__key = { ...w.__key, present: true };
            return copy(w.__key);
          }
          if (cmd === "phonon_gradium_key_delete") {
            w.__key = { ...w.__key, present: false };
            return copy(w.__key);
          }
          if (cmd === "phonon_voice_record_start") return null;
          if (cmd === "phonon_voice_record_stop") return { seconds: 9.4 };
          if (cmd === "phonon_voice_record_discard") return null;

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
const pick = (page: Page, ...paths: (string | null)[]) =>
  page.evaluate((p) => {
    (window as unknown as { __picks: unknown[] }).__picks.push(...p);
  }, paths);
const setMode = (page: Page, mode: string) =>
  page.evaluate((m) => {
    (window as unknown as { __cloneMode: string }).__cloneMode = m;
  }, mode);

const openPhonon = async (page: Page) => {
  // Die Anbieter stehen unter Modelle > Vorlesen (Ansicht "Anbieter"), ihre Einstellungen im Seitenblatt.
  await page.addInitScript(() => {
    try {
      localStorage.setItem("lva.ui.models.area", "voices");
      localStorage.setItem("lva.ui.models.voices.view", "providers");
    } catch {
      /* egal */
    }
  });
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Modelle", exact: true })
    .click();
  await page.getByTestId("tts-provider-open-phonon").click();
  await expect(page.getByTestId("phonon-own-voices")).toBeVisible();
  // „Eigene Stimmen“ ist im Seitenblatt eingeklappt.
  await page.getByTestId("phonon-own-voices-toggle").click();
};

const shot = async (page: Page, name: string) => {
  if (SCREENS) await page.screenshot({ path: `${SCREENS}/${name}.png` });
};

// ---------------------------------------------------------------------------

test("Import: Datei wählen, Name vergeben, Stimme erscheint; Ersetzen fragt zurück", async ({
  page,
}) => {
  await setup(page);
  await openPhonon(page);
  await expect(page.getByTestId("phonon-own-empty")).toBeVisible();

  await pick(page, "C:\\Stimmen\\Meine_Stimme.safetensors");
  await page.getByTestId("phonon-import-button").click();
  const dialog = page.getByTestId("phonon-import-dialog");
  await expect(dialog).toBeVisible();
  // Nur der Dateiname steht da, nicht der Ordner.
  await expect(page.getByTestId("phonon-import-file")).toHaveText(
    "Datei: Meine_Stimme.safetensors",
  );
  await expect(page.getByTestId("phonon-import-name")).toHaveValue(
    "Meine Stimme",
  );
  // Der Import geht nicht ins Netz und sagt es.
  await expect(page.locator(".lv-dialog")).toContainText(
    "Es geht nichts ins Netz",
  );
  await shot(page, "phonon-import");
  await page.getByTestId("phonon-import-run").click();

  const row = page.getByTestId("phonon-own-voice-Meine-Stimme");
  await expect(row).toBeVisible();
  await expect(row).toContainText("importiert");
  await expect(row).toContainText("Lizenz: eigene/unbekannt, nicht geprüft");
  await expect(row).toContainText("liegt in: 40m-de");
  await expect(page.getByTestId("phonon-own-notice")).toContainText(
    "Die Stimme „Meine Stimme“ wurde importiert.",
  );
  const imports = await calls(page, "phonon_own_voice_import");
  expect(imports).toHaveLength(1);
  expect(imports[0][1]).toEqual({
    path: "C:\\Stimmen\\Meine_Stimme.safetensors",
    name: "Meine Stimme",
    overwrite: false,
  });
  // Andere Stimmenlisten laden neu.
  expect(
    await page.evaluate(
      () => (window as unknown as { __voicesChanged: number }).__voicesChanged,
    ),
  ).toBeGreaterThan(0);

  // Derselbe Name noch einmal: Rückfrage, dann Ersetzen mit Zustimmung.
  await pick(page, "C:\\Stimmen\\andere.safetensors");
  await page.getByTestId("phonon-import-button").click();
  await page.getByTestId("phonon-import-name").fill("Meine Stimme");
  await page.getByTestId("phonon-import-run").click();
  await expect(page.getByTestId("phonon-import-overwrite")).toContainText(
    "Es gibt schon eine eigene Stimme „Meine Stimme“",
  );
  await expect(page.getByTestId("phonon-import-run")).toHaveText("Ersetzen");
  expect((await calls(page, "phonon_own_voice_import"))[1][1]).toMatchObject({
    overwrite: false,
  });
  await page.getByTestId("phonon-import-run").click();
  await expect(page.getByTestId("phonon-import-dialog")).toHaveCount(0);
  const again = await calls(page, "phonon_own_voice_import");
  expect(again).toHaveLength(3);
  expect(again[2][1]).toMatchObject({ name: "Meine Stimme", overwrite: true });
  await expect(page.locator("[data-testid^='phonon-own-voice-']")).toHaveCount(
    1,
  );
});

test("Import: eine ungültige Datei bleibt im Dialog stehen und legt nichts an", async ({
  page,
}) => {
  await setup(page);
  await openPhonon(page);
  await pick(page, "C:\\Stimmen\\kaputt.safetensors");
  await page.getByTestId("phonon-import-button").click();
  await page.getByTestId("phonon-import-run").click();
  await expect(page.getByTestId("phonon-import-error")).toContainText(
    "keine safetensors-Datei",
  );
  await expect(page.getByTestId("phonon-import-dialog")).toBeVisible();
  await expect(page.getByTestId("phonon-own-empty")).toBeVisible();
  // Abbrechen schliesst; nichts ist angelegt.
  await page.getByRole("button", { name: "Abbrechen" }).click();
  await expect(page.getByTestId("phonon-import-dialog")).toHaveCount(0);
  await expect(page.getByTestId("phonon-own-empty")).toBeVisible();
  // Ein Abbruch der Dateiwahl öffnet nichts.
  await page.getByTestId("phonon-import-button").click();
  await expect(page.getByTestId("phonon-import-dialog")).toHaveCount(0);
});

test("Erstellen: Hinweis steht da, ohne Häkchen kein Aufruf, mit Häkchen genau einer", async ({
  page,
}) => {
  await setup(page);
  await openPhonon(page);
  await pick(page, "C:\\Aufnahmen\\Sprecher_Eins.wav");
  await page.getByTestId("phonon-create-button").click();
  const dialog = page.getByTestId("phonon-clone-dialog");
  await expect(dialog).toBeVisible();

  // Der Pflicht-Hinweis ist sichtbar, bevor irgendetwas gewählt ist.
  const notice = page.getByTestId("phonon-clone-notice");
  await expect(notice).toContainText(
    "Die Aufnahme (ca. 10 s) wird an Gradium (api.gradium.ai) gesendet und dort in Ihrem Konto gespeichert.",
  );
  await expect(page.getByTestId("phonon-clone-run")).toBeDisabled();

  await page.getByTestId("phonon-clone-file-choose").click();
  await expect(page.getByTestId("phonon-clone-file")).toHaveText(
    "Sprecher_Eins.wav",
  );
  await expect(page.getByTestId("phonon-clone-name")).toHaveValue(
    "Sprecher Eins",
  );
  // Name und Datei da, aber ohne Einwilligung bleibt der Knopf gesperrt.
  await expect(page.getByTestId("phonon-clone-run")).toBeDisabled();
  expect(await calls(page, "phonon_voice_clone")).toHaveLength(0);

  // Startsekunde: Komma wird verstanden, Unsinn sperrt.
  const start = page.getByTestId("phonon-clone-start");
  await start.fill("abc");
  await page.getByTestId("phonon-clone-consent").check();
  await expect(page.getByTestId("phonon-clone-run")).toBeDisabled();
  await start.fill("2,5");
  await expect(page.getByTestId("phonon-clone-run")).toBeEnabled();
  await shot(page, "phonon-clone-dialog");

  await page.getByTestId("phonon-clone-run").click();
  await expect(page.getByTestId("phonon-clone-done")).toContainText(
    "Die Stimme „Sprecher Eins“ ist angelegt",
  );
  const clones = await calls(page, "phonon_voice_clone");
  expect(clones).toHaveLength(1);
  expect(clones[0][1]).toEqual({
    request: {
      source: { kind: "file", path: "C:\\Aufnahmen\\Sprecher_Eins.wav" },
      name: "Sprecher Eins",
      language: "de",
      start_s: 2.5,
      consent: true,
      overwrite: false,
    },
  });
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Schließen" })
    .last()
    .click();
  const row = page.getByTestId("phonon-own-voice-Sprecher-Eins");
  await expect(row).toContainText("mit Gradium erstellt");
  await expect(row).toContainText("Audiodatei");
  await shot(page, "phonon-own-voices");
});

test("Erstellen: eine andere Sprache geht mit; der Schlüssel fehlt: Hinweis, Knopf gesperrt", async ({
  page,
}) => {
  await setup(page, { key: false });
  await openPhonon(page);
  await pick(page, "C:\\Aufnahmen\\a.wav");
  await page.getByTestId("phonon-create-button").click();
  await page.getByTestId("phonon-clone-file-choose").click();
  await page.getByTestId("phonon-clone-consent").check();
  // Ohne Schlüssel nie: Hinweis statt Knopf.
  await expect(page.getByTestId("phonon-clone-nokey")).toContainText(
    "Gradium-API-Schlüssel",
  );
  await expect(page.getByTestId("phonon-clone-run")).toBeDisabled();
  expect(await calls(page, "phonon_voice_clone")).toHaveLength(0);
  await page.getByRole("button", { name: "Abbrechen" }).click();

  // Schlüssel eintragen, dann geht es; Sprache Englisch.
  await page
    .getByTestId("phonon-key-input")
    .fill("gsk-geheimer-test-schluessel");
  await page.getByTestId("phonon-key-save").click();
  await expect(page.getByTestId("phonon-key-status")).toHaveText(
    "Ein Schlüssel ist gespeichert.",
  );
  await pick(page, "C:\\Aufnahmen\\a.wav");
  await page.getByTestId("phonon-create-button").click();
  await page.getByTestId("phonon-clone-file-choose").click();
  await page.getByLabel("Sprache der Aufnahme").click();
  await page.getByRole("option", { name: "Englisch" }).click();
  await page.getByTestId("phonon-clone-consent").check();
  await page.getByTestId("phonon-clone-run").click();
  await expect(page.getByTestId("phonon-clone-done")).toBeVisible();
  const clones = await calls(page, "phonon_voice_clone");
  expect(clones).toHaveLength(1);
  expect(
    (clones[0][1] as { request: { language: string } }).request.language,
  ).toBe("en");
});

test("Schlüssel: eintragen, ändern, löschen; er steht nie wieder in der Oberfläche", async ({
  page,
}) => {
  await setup(page, { key: false });
  await openPhonon(page);
  const status = page.getByTestId("phonon-key-status");
  await expect(status).toHaveText("Kein Schlüssel gespeichert.");
  await expect(page.getByTestId("phonon-key")).toContainText(
    "nur an api.gradium.ai gesendet",
  );
  await expect(page.getByTestId("phonon-key")).toContainText(
    "Beim Speichern geht nichts ins Netz",
  );

  const input = page.getByTestId("phonon-key-input");
  await expect(input).toHaveAttribute("type", "password");
  // Zu kurz: kein Speichern.
  await input.fill("kurz");
  await expect(page.getByTestId("phonon-key-save")).toBeDisabled();
  const secret = "gsk-sehr-geheim-0123456789";
  await input.fill(secret);
  await page.getByTestId("phonon-key-save").click();
  await expect(status).toHaveText("Ein Schlüssel ist gespeichert.");
  await expect(page.getByTestId("phonon-key-saved")).toBeVisible();
  // Das Feld ist leer, der Schlüssel steht nirgends im Dokument.
  await expect(input).toHaveValue("");
  expect(await page.content()).not.toContain(secret);
  expect(await calls(page, "phonon_gradium_key_set")).toEqual([
    ["phonon_gradium_key_set", { key: secret }],
  ]);
  // Und keine der Einstellungen hat ihn je bekommen.
  expect(
    (await calls(page, "set_app_settings")).concat(
      await calls(page, "change_setting"),
    ),
  ).toHaveLength(0);

  // Ändern.
  await expect(page.getByTestId("phonon-key-save")).toHaveText("Ändern");
  await input.fill("gsk-zweiter-schluessel-99");
  await page.getByTestId("phonon-key-save").click();
  await expect(page.getByTestId("phonon-key-saved")).toBeVisible();
  // Löschen.
  await page.getByTestId("phonon-key-delete").click();
  await expect(status).toHaveText("Kein Schlüssel gespeichert.");
  await expect(page.getByTestId("phonon-key-delete")).toHaveCount(0);
});

test("Fish-Stimmen: jede braucht ihre eigene Einwilligung", async ({
  page,
}) => {
  await setup(page);
  await openPhonon(page);
  const fish = page.getByTestId("phonon-fish-voices");
  await expect(fish).toContainText("Lokale Fish-Speech-Stimmen übernehmen");
  await expect(fish).toContainText(
    "Jede Stimme braucht ihre eigene Einwilligung.",
  );
  await expect(page.getByTestId("phonon-fish-erzaehler")).toContainText(
    "Erzähler",
  );
  await expect(page.getByTestId("phonon-fish-anna")).toContainText("Anna");

  // Erste Stimme: Quelle Fish ist vorgewählt, Name aus der Fish-Stimme.
  await page.getByTestId("phonon-fish-take-erzaehler").click();
  await expect(page.getByTestId("phonon-clone-source-fish")).toBeChecked();
  await expect(page.getByTestId("phonon-clone-name")).toHaveValue("Erzähler");
  await expect(page.getByTestId("phonon-clone-notice")).toContainText(
    "api.gradium.ai",
  );
  await page.getByTestId("phonon-clone-consent").check();
  await page.getByTestId("phonon-clone-run").click();
  await expect(page.getByTestId("phonon-clone-done")).toBeVisible();
  let clones = await calls(page, "phonon_voice_clone");
  expect(clones).toHaveLength(1);
  expect(
    (clones[0][1] as { request: Record<string, unknown> }).request,
  ).toMatchObject({
    source: { kind: "fish", voice_id: "erzaehler" },
    name: "Erzähler",
    consent: true,
  });
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Schließen" })
    .last()
    .click();
  await expect(page.getByTestId("phonon-own-voice-Erzaehler")).toContainText(
    "Fish-Stimme: erzaehler",
  );

  // Zweite Stimme: das Häkchen ist wieder leer, der Knopf gesperrt.
  await page.getByTestId("phonon-fish-take-anna").click();
  await expect(page.getByTestId("phonon-clone-name")).toHaveValue("Anna");
  await expect(page.getByTestId("phonon-clone-consent")).not.toBeChecked();
  await expect(page.getByTestId("phonon-clone-run")).toBeDisabled();
  await page.getByTestId("phonon-clone-consent").check();
  await expect(page.getByTestId("phonon-clone-run")).toBeEnabled();
  await page.getByRole("button", { name: "Abbrechen" }).click();
  clones = await calls(page, "phonon_voice_clone");
  expect(clones).toHaveLength(1);
});

test("Mikrofon: aufnehmen, beenden, erstellen; Schließen verwirft die Aufnahme", async ({
  page,
}) => {
  await setup(page);
  await openPhonon(page);
  await page.getByTestId("phonon-create-button").click();
  await page.getByTestId("phonon-clone-source-recording").check();
  // Ohne Aufnahme ist nichts wählbar.
  await page.getByTestId("phonon-clone-name").fill("Mikro");
  await page.getByTestId("phonon-clone-consent").check();
  await expect(page.getByTestId("phonon-clone-run")).toBeDisabled();

  await page.getByTestId("phonon-clone-record-start").click();
  await expect(page.getByTestId("phonon-clone-record-status")).toContainText(
    "Aufnahme läuft",
  );
  // Während der Aufnahme kein Erstellen.
  await expect(page.getByTestId("phonon-clone-run")).toBeDisabled();
  await page.getByTestId("phonon-clone-record-stop").click();
  await expect(page.getByTestId("phonon-clone-record-status")).toHaveText(
    "Aufnahme bereit: 9.4 s",
  );
  // Eine neue Aufnahme ist eine neue Quelle: die Einwilligung geht zurück.
  await expect(page.getByTestId("phonon-clone-consent")).not.toBeChecked();
  await page.getByTestId("phonon-clone-consent").check();
  await page.getByTestId("phonon-clone-run").click();
  await expect(page.getByTestId("phonon-clone-done")).toBeVisible();
  const clones = await calls(page, "phonon_voice_clone");
  expect(
    (clones[0][1] as { request: Record<string, unknown> }).request,
  ).toMatchObject({ source: { kind: "recording" }, start_s: 0, consent: true });
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Schließen" })
    .last()
    .click();

  // Zweiter Dialog: aufnehmen und schließen verwirft.
  await page.getByTestId("phonon-create-button").click();
  await page.getByTestId("phonon-clone-source-recording").check();
  await page.getByTestId("phonon-clone-record-start").click();
  await page.getByTestId("phonon-clone-record-stop").click();
  await page.getByRole("button", { name: "Abbrechen" }).click();
  expect(
    (await calls(page, "phonon_voice_record_discard")).length,
  ).toBeGreaterThan(0);
});

test("Fehler von Gradium: Meldung im Dialog, nichts angelegt, erneut möglich", async ({
  page,
}) => {
  await setup(page);
  await openPhonon(page);
  await setMode(page, "unauthorized");
  await pick(page, "C:\\Aufnahmen\\x.wav");
  await page.getByTestId("phonon-create-button").click();
  await page.getByTestId("phonon-clone-file-choose").click();
  await page.getByTestId("phonon-clone-consent").check();
  await page.getByTestId("phonon-clone-run").click();
  await expect(page.getByTestId("phonon-clone-error")).toContainText(
    "Gradium lehnt den API-Schlüssel ab (HTTP 401)",
  );
  await expect(page.getByTestId("phonon-clone-dialog")).toBeVisible();
  await expect(page.getByTestId("phonon-clone-run")).toBeEnabled();
  await expect(page.locator("[data-testid^='phonon-own-voice-']")).toHaveCount(
    0,
  );
  // Zweiter Versuch gelingt.
  await setMode(page, "ok");
  await page.getByTestId("phonon-clone-run").click();
  await expect(page.getByTestId("phonon-clone-done")).toBeVisible();
  await expect(page.getByTestId("phonon-clone-error")).toHaveCount(0);
});

test("Erstellen läuft: Stufen sind sichtbar, Abbrechen beendet ohne Fehlermeldung", async ({
  page,
}) => {
  await setup(page);
  await openPhonon(page);
  await setMode(page, "slow");
  await pick(page, "C:\\Aufnahmen\\x.wav");
  await page.getByTestId("phonon-create-button").click();
  await page.getByTestId("phonon-clone-file-choose").click();
  await page.getByTestId("phonon-clone-consent").check();
  await page.getByTestId("phonon-clone-run").click();
  await expect(page.getByTestId("phonon-clone-phase")).toContainText(
    "Die Aufnahme wird an Gradium übertragen",
  );
  // Während des Laufs: nur Abbrechen.
  await expect(page.getByTestId("phonon-clone-run")).toHaveCount(0);
  await page.getByTestId("phonon-clone-cancel").click();
  await expect(page.getByTestId("phonon-clone-run")).toBeVisible();
  await expect(page.getByTestId("phonon-clone-error")).toHaveCount(0);
  expect(await calls(page, "phonon_voice_clone_cancel")).toHaveLength(1);
  await expect(page.locator("[data-testid^='phonon-own-voice-']")).toHaveCount(
    0,
  );
});

test("Entfernen: mit Rückfrage; bei einer erstellten Stimme der Hinweis auf das Gradium-Konto", async ({
  page,
}) => {
  await setup(page);
  await openPhonon(page);
  await pick(page, "C:\\Stimmen\\Alt.safetensors");
  await page.getByTestId("phonon-import-button").click();
  await page.getByTestId("phonon-import-run").click();
  await expect(page.getByTestId("phonon-own-voice-Alt")).toBeVisible();

  await page.getByTestId("phonon-own-remove-Alt").click();
  await expect(
    page.locator(".lv-dialog").getByText("Eigene Stimme „Alt“ entfernen?"),
  ).toBeVisible();
  // Importierte Stimme: kein Hinweis auf Gradium.
  await expect(page.locator(".lv-dialog")).not.toContainText("Gradium-Konto");
  await page.getByRole("button", { name: "Abbrechen" }).click();
  expect(await calls(page, "phonon_own_voice_remove")).toHaveLength(0);

  await page.getByTestId("phonon-own-remove-Alt").click();
  await page.getByTestId("phonon-own-remove-confirm").click();
  await expect(page.getByTestId("phonon-own-voice-Alt")).toHaveCount(0);
  expect(await calls(page, "phonon_own_voice_remove")).toEqual([
    ["phonon_own_voice_remove", { id: "Alt" }],
  ]);
  await expect(page.getByTestId("phonon-own-notice")).toContainText(
    "wurde entfernt",
  );

  // Erstellte Stimme: der Dialog nennt die Kopie bei Gradium.
  await pick(page, "C:\\Aufnahmen\\g.wav");
  await page.getByTestId("phonon-create-button").click();
  await page.getByTestId("phonon-clone-file-choose").click();
  await page.getByTestId("phonon-clone-consent").check();
  await page.getByTestId("phonon-clone-run").click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Schließen" })
    .last()
    .click();
  await page.getByTestId("phonon-own-remove-g").click();
  await expect(page.locator(".lv-dialog")).toContainText(
    "bleibt in Ihrem Gradium-Konto bestehen",
  );
});

test("Ohne Phonon-Modell: Hinweis; die Stimmen werden trotzdem verwaltet", async ({
  page,
}) => {
  await setup(page);
  await page.addInitScript(() => {
    (window as unknown as { __own: { models: string[] } }).__own.models = [];
  });
  await openPhonon(page);
  await expect(page.getByTestId("phonon-own-no-model")).toContainText(
    "kein Phonon-Modell installiert",
  );
  await pick(page, "C:\\Stimmen\\Fruehstart.safetensors");
  await page.getByTestId("phonon-import-button").click();
  await page.getByTestId("phonon-import-run").click();
  await expect(page.getByTestId("phonon-own-voice-Fruehstart")).toBeVisible();
});
