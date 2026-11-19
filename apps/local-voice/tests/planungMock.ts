import type { Page } from "@playwright/test";

// Tauri-Attrappe der Seite „Planung“ (lva#534 P2): die Befehle besprechungsplanung_* mit
// synthetischen Daten (example.org). Die Regelprüfung der Attrappe ist ein verkleinerter Nachbau
// der des Crates (rules.rs); die Seite zeigt nur, was die Attrappe zurückgibt.

export const T0 = 1_793_000_000_000;
const MIN = 60_000;

export interface MockPlan {
  id: string;
  meeting_id: string | null;
  title: string;
  goal_type: string;
  goal_text: string;
  expected_result: string | null;
  duration_min: number;
  classification: string;
  agenda: { minutes: number; topic: string; owner: string | null }[];
  roles: Record<string, string | null>;
  materials: { label: string; url: string | null; due_date: string | null }[];
  hints: Record<string, unknown>;
  transcription_planned: boolean;
  minutes_template_id: string | null;
  status: string;
  created_at: number;
  updated_at: number;
}

export function plan(
  id: string,
  title: string,
  extra: Partial<MockPlan> = {},
): MockPlan {
  return {
    id,
    meeting_id: null,
    title,
    goal_type: "entscheiden",
    goal_text: "Wir entscheiden über den Pilotstart.",
    expected_result: "Entscheidung mit Termin",
    duration_min: 25,
    classification: "intern",
    agenda: [
      { minutes: 5, topic: "Stand", owner: "A. Beispiel" },
      { minutes: 15, topic: "Optionen", owner: null },
    ],
    roles: {
      moderation: "A. Beispiel",
      ergebnissicherung: null,
      entscheidung: null,
    },
    materials: [],
    hints: {
      weiterleiten_erlaubt: null,
      barrierefreiheit: null,
      ergebnisfrist: null,
      ablageort: null,
    },
    transcription_planned: false,
    minutes_template_id: null,
    status: "entwurf",
    created_at: T0 - 86_400_000,
    updated_at: T0 - 3_600_000,
    ...extra,
  };
}

export function link(id: string, extra: Record<string, unknown> = {}) {
  return {
    plan_id: id,
    adapter: "ics_marker",
    source_id: "src1",
    remote_id: null,
    ical_uid: `${id}@example.org`,
    change_key: null,
    starts_at: T0 + 86_400_000,
    ends_at: T0 + 86_400_000 + 25 * MIN,
    location: "Raum 1.02",
    join_url: null,
    body_hash: null,
    last_seen_at: T0,
    last_change_at: T0,
    sync_state: "ok",
    verschiebung_offen: false,
    block_konflikt: false,
    abgesagt: false,
    instanz: null,
    fremd_beobachtet: false,
    ...extra,
  };
}

export function demoDaten() {
  const plaene = [
    plan("p_entwurf", "Abstimmung Pilotstart", {
      minutes_template_id: "vorlage-1",
    }),
    plan("p_uebergeben", "Jour fixe Betrieb", { status: "uebergeben" }),
    plan("p_verknuepft", "Planung Quartal 1", {
      status: "verknuepft",
      transcription_planned: true,
      duration_min: 50,
    }),
    plan("p_verschoben", "Review Datenschutz", { status: "verschoben" }),
    plan("p_konflikt", "Workshop Rollen", { status: "verknuepft" }),
    plan("p_abgesagt", "Kickoff Archiv", { status: "abgesagt" }),
    plan("p_fertig", "Retrospektive Oktober", {
      status: "durchgefuehrt",
      transcription_planned: true,
    }),
  ];
  const links: Record<string, unknown> = {
    p_uebergeben: null,
    p_verknuepft: link("p_verknuepft", {
      starts_at: T0 + 2 * 86_400_000,
      ends_at: T0 + 2 * 86_400_000 + 50 * MIN,
    }),
    p_verschoben: link("p_verschoben", {
      sync_state: "verschoben",
      verschiebung_offen: true,
      starts_at: T0 + 3 * 86_400_000,
      ends_at: T0 + 3 * 86_400_000 + 25 * MIN,
    }),
    p_konflikt: link("p_konflikt", {
      sync_state: "konflikt",
      block_konflikt: true,
      starts_at: T0 + 4 * 86_400_000,
      ends_at: T0 + 4 * 86_400_000 + 25 * MIN,
    }),
    p_abgesagt: link("p_abgesagt", { sync_state: "abgesagt", abgesagt: true }),
    p_fertig: link("p_fertig", {
      starts_at: T0 - 7 * 86_400_000,
      ends_at: T0 - 7 * 86_400_000 + 25 * MIN,
    }),
  };
  const consents: Record<string, unknown[]> = {
    p_verknuepft: [
      {
        plan_id: "p_verknuepft",
        person: "a@example.org",
        state: "zugestimmt",
        channel: "einladung",
        recorded_at: T0 - 3_600_000,
      },
      {
        plan_id: "p_verknuepft",
        person: "b@example.org",
        state: "offen",
        channel: null,
        recorded_at: null,
      },
      {
        plan_id: "p_verknuepft",
        person: "c@example.org",
        state: "abgelehnt",
        channel: "mail",
        recorded_at: T0 - 7_200_000,
      },
    ],
  };
  return { plaene, links, consents };
}

export async function installMock(
  page: Page,
  opts: { theme?: "light" | "dark"; leer?: boolean } = {},
) {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(
    ({ daten, theme }) => {
      // Die Vorschau liegt in einem Rahmen ohne Rechte (`sandbox=""`): dort gibt es weder
      // localStorage noch die Attrappe, und der Initialisierungsskript läuft auch dort.
      if (window.top !== window) return;
      const callbacks = new Map<number, unknown>();
      let callback = 0;
      const settings = {
        onboarding_completed: true,
        app_language: "de",
        theme,
        show_whats_new_on_update: false,
        debug_mode: false,
        selected_model: "",
        bindings: {},
        post_process_providers: [],
        post_process_prompts: [],
        custom_words: [],
        post_process_models: {},
        post_process_api_keys: {},
        push_to_talk: true,
        tts_engine: "piper",
      };
      const w = window as unknown as Record<string, any>;
      w.__calls = [] as unknown[];
      w.__daten = daten;
      // Tests halten einzelne Antworten an oder lassen sie ohne Inhaltsdaten fehlschlagen.
      w.__steuerung = {};
      w.__freigaben = {};
      window.localStorage.setItem("lva.ui.section", "planung");

      const pruefen = (input: any, beginn: any) => {
        const h: { code: string; stufe: string; text: string }[] = [];
        const add = (code: string, stufe: string) =>
          h.push({ code, stufe, text: code });
        if (!String(input.title).trim()) add("titel_leer", "fehler");
        if (!String(input.goal_text).trim()) add("ziel_fehlt", "warnung");
        if (!String(input.expected_result ?? "").trim())
          add("ergebnis_fehlt", "info");
        if (input.duration_min === 0) add("dauer_null", "fehler");
        else if (![25, 50].includes(input.duration_min))
          add("dauer_unueblich", "warnung");
        if (input.agenda.length === 0) add("agenda_leer", "warnung");
        else {
          if (input.agenda.some((a: any) => !String(a.topic).trim()))
            add("agenda_punkt_leer", "fehler");
          const summe = input.agenda.reduce(
            (s: number, a: any) => s + a.minutes,
            0,
          );
          if (summe > input.duration_min && input.duration_min > 0)
            add("agenda_zu_lang", "warnung");
        }
        if (input.transcription_planned)
          add("transkription_einwilligung", "info");
        if (beginn && beginn.minute !== 0 && beginn.minute !== 30)
          add("beginn_krumm", "info");
        return h;
      };
      const vorschau = (input: any, id: string | null) => {
        const marke = `lva-plan:${id ?? "vorschau"}`;
        const text = [
          "— Besprechungsplanung —",
          input.title,
          `Ziel: ${input.goal_text}`,
          `Dauer: ${input.duration_min} Minuten`,
          "",
          marke,
        ].join("\n");
        return {
          html: `<div style="font-family:'Segoe UI',sans-serif"><p><b>${input.title}</b></p><p>Ziel: ${input.goal_text}</p><p>${marke}</p></div>`,
          text,
          block_hash: "h",
        };
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
          convertFileSrc: (path: string) => `asset://${path}`,
          invoke: async (cmd: string, args?: Record<string, any>) => {
            (w.__calls as unknown[]).push([cmd, args]);
            const steuerung = w.__steuerung[cmd];
            if (steuerung?.pause) {
              await new Promise<void>((resolve) => {
                (w.__freigaben[cmd] ??= []).push(resolve);
              });
            }
            if (steuerung?.fehler) throw new Error("mock_abruf_fehlgeschlagen");
            const d = w.__daten as {
              plaene: any[];
              links: Record<string, any>;
              consents: Record<string, any[]>;
            };
            if (cmd === "get_app_settings" || cmd === "get_default_settings")
              return JSON.parse(JSON.stringify(settings));
            if (cmd === "plugin:os|locale") return "de-DE";
            if (cmd === "plugin:app|version") return "0.23.0";
            if (cmd.includes("permission")) return true;
            if (cmd === "plugin:event|listen") return Number(args?.handler);
            if (cmd === "plugin:event|unlisten") return null;
            if (cmd === "besprechungsplanung_liste")
              return JSON.parse(JSON.stringify(d.plaene));
            if (cmd === "besprechungsplanung_verknuepfung")
              return JSON.parse(JSON.stringify(d.links[args?.id] ?? null));
            if (cmd === "besprechungsplanung_einwilligungen")
              return JSON.parse(JSON.stringify(d.consents[args?.id] ?? []));
            if (cmd === "besprechungsplanung_einwilligung_setzen") {
              const c = args?.einwilligung;
              const liste = (d.consents[c.plan_id] ??= []);
              const index = liste.findIndex((alt) => alt.person === c.person);
              if (index < 0) liste.push({ ...c });
              else liste[index] = { ...c };
              return null;
            }
            if (cmd === "besprechungsplanung_pruefen")
              return pruefen(args?.input, args?.beginn);
            if (cmd === "besprechungsplanung_vorschau")
              return vorschau(args?.input, args?.id);
            if (cmd === "besprechungsplanung_anlegen") {
              const p = {
                ...args?.input,
                id: `neu${d.plaene.length}`,
                meeting_id: null,
                status: "entwurf",
                created_at: Date.now(),
                updated_at: Date.now(),
              };
              d.plaene.push(p);
              return p;
            }
            if (cmd === "besprechungsplanung_aendern") {
              const i = d.plaene.findIndex((p) => p.id === args?.id);
              d.plaene[i] = {
                ...d.plaene[i],
                ...args?.input,
                updated_at: Date.now(),
              };
              return d.plaene[i];
            }
            if (cmd === "besprechungsplanung_einladung") {
              const p = d.plaene.find((x) => x.id === args?.id);
              p.status = "uebergeben";
              return vorschau(p, p.id);
            }
            if (cmd === "besprechungsplanung_loeschen") {
              d.plaene = d.plaene.filter((p) => p.id !== args?.id);
              return null;
            }
            if (cmd === "besprechungsplanung_verschiebung_bestaetigen") {
              const l = d.links[args?.id];
              l.verschiebung_offen = false;
              l.sync_state = "ok";
              d.plaene.find((p) => p.id === args?.id).status = "verknuepft";
              return l;
            }
            if (cmd === "besprechungsplanung_abgleichen")
              return {
                geprueft: 4,
                geaendert: 1,
                ereignisse: [],
                mehrdeutig: [],
                uebersprungen: 0,
                fehler: [],
              };
            if (cmd === "tts_server_status")
              return { phase: "stopped", message: null };
            if (cmd === "get_custom_sounds")
              return { start: false, stop: false };
            if (cmd === "meetings_is_recording") return false;
            if (cmd === "get_selected_model") return "";
            if (cmd.endsWith("_list") || cmd === "llm_ps") return [];
            if (
              cmd.includes("models") ||
              cmd.includes("history") ||
              cmd.includes("devices")
            )
              return [];
            return null;
          },
        },
      });
    },
    {
      daten: opts.leer ? { plaene: [], links: {}, consents: {} } : demoDaten(),
      theme: opts.theme ?? "light",
    },
  );
}
