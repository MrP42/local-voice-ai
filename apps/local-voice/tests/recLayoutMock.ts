import type { Page } from "@playwright/test";

/**
 * Tauri-Attrappe und Helfer fuer die Aufnahmen-Oberflaeche:
 * `meeting-layout.spec.ts` (Spalten, Griffe, eine Scrollbar, Persistenz) und
 * `meeting-screens.spec.ts` (Nachher-Bilder, nur mit LVA_SCREENSHOTS).
 * Eine Besprechung laeuft (m3, Fortschritt per `emit`), eine ist importiert
 * (m2, mit Audio, Protokoll und 60 Segmenten), der Rest ist fertig.
 *
 * Seit M4 (`meeting-header.spec.ts`) schreibt die Attrappe alle Aufrufe mit
 * (`calls`) und kennt die Befehle aus Startdialog, Menue und Kopf (Start,
 * Umbenennen, Loeschen, Ordner, Import, Neu-Transkription, Neu-Erzeugen).
 */

export const VIEWPORTS = {
  "1920": { width: 1920, height: 1050 },
  "1366": { width: 1366, height: 768 },
  "900": { width: 900, height: 700 },
  "480": { width: 480, height: 800 },
} as const;

const T0 = 1_790_000_000;
const meeting = (
  id: string,
  title: string,
  over: Record<string, unknown> = {},
) => ({
  id,
  title,
  status: "ready",
  source: "recording",
  started_at: T0,
  ended_at: T0 + 3600,
  language: "de",
  mic_audio_path: null,
  system_audio_path: null,
  duration_ms: 3_540_000,
  consent_confirmed_at: T0,
  audio_retention_until: null,
  created_at: T0,
  source_path: null,
  description: null,
  deleted_at: null,
  ...over,
});

export const TITLE_M1 = "Jour fixe Vertrieb KW 40";
export const TITLE_M2 = "Kundentermin Stadtwerke Kaiserslautern – Lastgang";
export const TITLE_M3 = "Projektreview Netzpuls Sprint 12";

export const MEETINGS = [
  meeting("m1", TITLE_M1, {
    started_at: T0 + 5 * 86400,
    duration_ms: 2_280_000,
  }),
  meeting("m2", TITLE_M2, {
    started_at: T0 + 4 * 86400,
    source: "import",
    source_path: "C:/Users/patrick/Aufnahmen/SWK-Lastgang-2026-09-28.m4a",
    mic_audio_path: "C:/Users/patrick/Aufnahmen/SWK-Lastgang-2026-09-28.m4a",
    audio_retention_until: T0 + 34 * 86400,
    duration_ms: 3_540_000,
  }),
  meeting("m3", TITLE_M3, {
    started_at: T0 + 3 * 86400,
    status: "processing",
    source: "import",
    source_path: "C:/Users/patrick/Videos/Netzpuls-Review-Sprint12.mp4",
    duration_ms: 4_200_000,
  }),
  meeting("m4", "Elternabend Klasse 3b", {
    started_at: T0 + 2 * 86400,
    duration_ms: 5_100_000,
  }),
  meeting("m5", "Steuerberatung Jahresabschluss 2025", {
    started_at: T0 + 1 * 86400,
    duration_ms: 1_980_000,
  }),
  meeting("m6", "Podcast-Interview „KI im Mittelstand“", {
    started_at: T0,
    source: "import",
    source_path: "C:/Users/patrick/Podcast/Folge-17-KI-Mittelstand.wav",
    duration_ms: 2_940_000,
  }),
];

const folder = (id: string, name: string, count: number) => ({
  id,
  name,
  color: null,
  sort: Number(id.slice(1)),
  meeting_count: count,
  created_at: 1,
  updated_at: 1,
});
export const FOLDERS = [
  folder("f1", "Privat", 2),
  folder("f2", "Geschäftlich", 3),
  folder("f3", "Kunde Stadtwerke", 1),
  folder("f4", "Podcast", 1),
];

export const LINES = [
  [1, "Guten Morgen zusammen, schön, dass es heute so kurzfristig klappt."],
  [0, "Gerne. Wir wollten uns ja den Lastgang vom letzten Quartal ansehen."],
  [1, "Genau. Die Spitzen liegen fast immer zwischen halb acht und neun."],
  [
    0,
    "Das passt zu dem, was wir aus den Zählerdaten sehen, vor allem montags.",
  ],
  [
    1,
    "Können wir die Spitzen mit dem Speicher glätten, ohne Leistung zuzukaufen?",
  ],
  [
    0,
    "Teilweise. Mit 400 Kilowattstunden kappen wir etwa ein Drittel der Spitzen.",
  ],
  [1, "Und was bedeutet das für den Leistungspreis im nächsten Jahr?"],
  [0, "Grob gerechnet sinkt er um zwölf bis fünfzehn Prozent, je nach Tarif."],
  [
    1,
    "Das klingt gut, aber ich brauche das schriftlich für die Geschäftsführung.",
  ],
  [
    0,
    "Kein Problem, ich schicke bis Freitag eine Kurzfassung mit den Annahmen.",
  ],
] as const;

export const SEGMENTS = Array.from({ length: 60 }, (_, i) => {
  const [channel, text] = LINES[i % LINES.length];
  return {
    segment_index: i,
    text,
    start_ms: i * 29_000 + 4_000,
    end_ms: i * 29_000 + 28_000,
    channel,
    speaker_index: null,
  };
});

const MINUTES_BODY = [
  "# Protokoll: Kundentermin Stadtwerke Kaiserslautern",
  "",
  "**Datum:** 28.09.2026 · **Dauer:** 59 Min. · **Teilnehmende:** Patrick Wolff, Frau Becker (SWK)",
  "",
  "## Anlass",
  "Auswertung des Lastgangs Q3 und Möglichkeiten zur Glättung der Lastspitzen.",
  "",
  "## Ergebnisse",
  "- Lastspitzen liegen werktags zwischen 07:30 und 09:00 Uhr, montags am höchsten.",
  "- Ein Speicher mit 400 kWh kappt rund ein Drittel der Spitzen.",
  "- Erwartete Senkung des Leistungspreises: 12–15 %.",
  "",
  "## Aufgaben",
  "- [ ] Kurzfassung mit Annahmen an Frau Becker – Patrick, bis Freitag",
  "- [ ] Tarifblatt 2027 anfordern – SWK",
].join("\n");

/** 70 Minuten Audio, 42 % verarbeitet (P8a), ~24 Min. Rest. */
export const PROGRESS = {
  kind: "progress",
  meeting_id: "m3",
  phase: "transcription",
  done: 1_764_000,
  total: 4_200_000,
  elapsed_ms: 750_000,
  eta_ms: 1_440_000,
  state: "running",
  pausable: true,
};

/**
 * Attrappe einbauen. `recording`: eine Aufnahme (m1) laeuft schon beim Start.
 * Die Seite "Aufnahmen" ist per Navigation erreichbar (`openRecordings`).
 */
export const installRecMock = async (
  page: Page,
  options: { recording?: boolean } = {},
) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(
    ({ meetings, folders, segments, minutesBody, recording }) => {
      const w = window as any;
      const callbacks = new Map<number, (e: unknown) => void>();
      const listeners: Record<string, number[]> = {};
      const handlerOf = new Map<number, { event: string; handler: number }>();
      let callback = 0;
      w.__settings = {
        onboarding_completed: true,
        app_language: "de",
        theme: "light",
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
        meeting_capture_system: true,
      };
      w.__meetings = meetings;
      w.__folders = folders;
      w.__segments = segments;
      w.__recording = recording;
      w.__position = recording
        ? { meeting_id: "m1", position_ms: 754_000 }
        : null;
      w.__progressList = [];
      w.__calls = [];
      w.__participants = {};
      // G4: erkannte Sprecher (je Besprechung), Vorlage und Protokoll-Ablage
      // der Besprechung. Bekannte Personen und `meetings_update_metadata` kommen
      // aus `recLiveMock` (`__people`).
      w.__speakers = {};
      w.__template = null;
      w.__minutesFile = null;
      w.__minutesMeta = null;
      w.__folderMap = { m2: ["f2", "f3"] };
      w.__pick = null;
      w.__templates = [
        {
          id: "builtin:allgemein",
          title: "Allgemein",
          builtin: true,
          spec: { version: 1, context: "Zweck", sections: [] },
          updated_at: 1,
        },
        {
          id: "builtin:vertrieb",
          title: "Kundengespräch / Vertrieb",
          builtin: true,
          spec: { version: 1, context: "Zweck", sections: [] },
          updated_at: 1,
        },
      ];
      w.__documents = [
        {
          id: "p1",
          meeting_id: "m2",
          kind: "minutes",
          body_format: "markdown@1",
          body: minutesBody,
          version: 1,
          created_at: 1790000900,
          template_id: null,
          updated_at: 301,
        },
      ];
      w.__emit = (event: string, payload: unknown) =>
        (listeners[event] ?? []).forEach((h) =>
          callbacks.get(h)?.({ event, id: 0, payload }),
        );
      Object.assign(window, {
        __TAURI_OS_PLUGIN_INTERNALS__: {
          platform: "windows",
          os_type: "windows",
          family: "windows",
          arch: "x86_64",
          version: "11",
          eol: "\r\n",
        },
        __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
        __TAURI_INTERNALS__: {
          metadata: {
            currentWindow: { label: "main" },
            currentWebview: { label: "main" },
          },
          transformCallback: (fn: (e: unknown) => void) => {
            callbacks.set(++callback, fn);
            return callback;
          },
          unregisterCallback: (id: number) => callbacks.delete(id),
          convertFileSrc: (p: string) => p,
          invoke: async (cmd: string, args: Record<string, any> = {}) => {
            if (!cmd.startsWith("plugin:event|")) w.__calls.push({ cmd, args });
            switch (cmd) {
              case "meetings_start":
                return {
                  ...w.__meetings[0],
                  id: "m-neu",
                  title: args.title,
                  status: "recording",
                };
              case "meetings_rename":
                w.__meetings = w.__meetings.map((m: any) =>
                  m.id === args.meetingId ? { ...m, title: args.title } : m,
                );
                return null;
              case "meetings_delete":
                w.__meetings = w.__meetings.filter(
                  (m: any) => m.id !== args.meetingId,
                );
                return null;
              case "meetings_set_folders":
                w.__folderMap[args.meetingId] = args.folderIds;
                return null;
              case "plugin:dialog|open":
                return w.__pick;
              case "meeting_templates_list":
                return w.__templates;
              case "meeting_participants":
                return w.__participants[args.meetingId] ?? [];
              case "meeting_speakers_list":
                return w.__speakers[args.meetingId] ?? [];
              case "meeting_speaker_rename": {
                const list = w.__speakers[args.meetingId] ?? [];
                const sp = list.find(
                  (x: any) =>
                    x.channel === args.channel &&
                    x.speaker_index === args.speakerIndex,
                );
                if (sp) {
                  sp.display_name = args.name;
                  sp.label = args.name ?? sp.label;
                }
                let person = (w.__people ??= []).find(
                  (p: any) => p.name === args.name,
                );
                if (!person && args.name) {
                  person = {
                    id: `h-neu-${w.__people.length}`,
                    name: args.name,
                    email: null,
                    company: null,
                    is_self: false,
                    meeting_count: 1,
                  };
                  w.__people.push(person);
                }
                const have = w.__participants[args.meetingId] ?? [];
                if (
                  person &&
                  !have.some((p: any) => p.human_id === person.id)
                ) {
                  w.__participants[args.meetingId] = [
                    ...have,
                    {
                      human_id: person.id,
                      name: person.name,
                      email: person.email,
                      company: person.company,
                      role: "speaker",
                      source: "speaker",
                      is_self: false,
                      meeting_count: 1,
                    },
                  ];
                }
                return sp;
              }
              case "meetings_minutes_file":
                return w.__minutesFile;
              case "meetings_minutes_meta":
                return w.__minutesMeta;
              case "meetings_get_template":
                return w.__template;
              case "meetings_set_template":
                w.__template = args.templateId;
                return null;
              case "get_app_settings":
              case "get_default_settings":
                return w.__settings;
              case "plugin:os|locale":
                return "de-DE";
              case "plugin:app|version":
                return "0.20.9";
              case "plugin:event|listen": {
                (listeners[args.event as string] ??= []).push(
                  args.handler as number,
                );
                const eventId = ++callback;
                handlerOf.set(eventId, {
                  event: args.event as string,
                  handler: args.handler as number,
                });
                return eventId;
              }
              case "plugin:event|unlisten": {
                const entry = handlerOf.get(args.eventId as number);
                if (entry) {
                  listeners[entry.event] = (
                    listeners[entry.event] ?? []
                  ).filter((h) => h !== entry.handler);
                  handlerOf.delete(args.eventId as number);
                }
                return null;
              }
              case "get_selected_model":
                return "";
              case "meetings_is_recording":
                return w.__recording;
              case "meetings_recording_position":
                return w.__position;
              case "meetings_list":
                return w.__meetings;
              case "meetings_search":
                return {
                  items: w.__meetings.map((m: any) => ({
                    meeting: m,
                    snippet: null,
                    hit_source: null,
                  })),
                  truncated: false,
                  total: w.__meetings.length,
                };
              case "meeting_folders_list":
                return w.__folders;
              case "meetings_get_folders":
                return w.__folderMap[args.meetingId] ?? [];
              case "meetings_get_segments":
                return args.meetingId === "m1" && w.__recording
                  ? []
                  : w.__segments;
              case "meetings_segment_epoch":
                return 1;
              case "meetings_progress_list":
                return w.__progressList;
              case "meetings_minutes_state":
                return {
                  running: false,
                  progress: null,
                  cancelling: false,
                  started_at: null,
                };
              case "meetings_get_documents":
                return w.__documents.filter(
                  (d: any) => d.meeting_id === args.meetingId,
                );
              case "meeting_notes_get":
                return {
                  meeting_id: args.meetingId,
                  blocks: [],
                  revision: 0,
                  updated_at: 0,
                };
              case "meeting_notes_save":
                return { revision: 1, updated_at: 1 };
              case "tts_server_status":
                return { phase: "stopped", message: null };
              case "get_custom_sounds":
                return { start: false, stop: false };
              case "action_items_list":
              case "meeting_speaker_notices":
              case "chat_recipes_list":
              case "meeting_chat_threads":
              case "people_list":
              case "calendar_upcoming":
              case "calendar_sources_list":
              case "pages_list":
              case "page_files":
              case "tts_list_voices":
              case "tts_list_voice_infos":
              case "llm_ps":
              case "tts_reading_list":
              case "tts_list_downloads":
              case "llm_local_list":
                return [];
            }
            if (
              cmd.includes("permission") ||
              cmd.includes("history") ||
              cmd.includes("models") ||
              cmd.includes("devices")
            )
              return cmd.includes("permission") ? true : [];
            return null;
          },
        },
      });
    },
    {
      meetings: MEETINGS,
      folders: FOLDERS,
      segments: SEGMENTS,
      minutesBody: MINUTES_BODY,
      recording: options.recording ?? false,
    },
  );
};

/** Ereignis des Backends ausloesen. */
export const emitMeeting = (page: Page, payload: Record<string, unknown>) =>
  page.evaluate((p) => (window as any).__emit("meeting-event", p), payload);

/** Seite "Aufnahmen" ueber die Navigation oeffnen (Fenstergroesse vorher). */
export const openRecordings = async (
  page: Page,
  width: number,
  height: number,
) => {
  await page.setViewportSize({ width, height });
  await page.goto("/");
  await goToRecordings(page);
};

export const goToRecordings = async (page: Page) => {
  const nav = page.getByRole("button", { name: "Aufnahmen", exact: true });
  if (!(await nav.isVisible())) {
    // Schmales Fenster: Menue ggf. zuerst aufklappen.
    const toggle = page.getByRole("button", { name: /Menü/ }).first();
    if (await toggle.isVisible()) await toggle.click();
  }
  await nav.click();
  await page.getByTestId("rec-content").waitFor();
};

/** Besprechung in der Liste waehlen (die Liste ist ggf. in der Schublade). */
export const pickMeeting = async (page: Page, id: string) => {
  const row = page.locator(`[data-meeting-id="${id}"]`);
  if (!(await row.isVisible())) {
    // Mittel: Leiste mit Knopf; schmal: Knopf im Seitenkopf.
    const rail = page.getByTestId("sessions-expand");
    await (
      (await rail.count()) > 0 ? rail : page.getByTestId("sessions-open")
    ).click();
  }
  await row.click();
};

export interface ScrollReport {
  /** Scrollt die ganze Seite? */
  documentScrollHeight: number;
  innerHeight: number;
  /** Hauptbereich: scrollt er? */
  mainOverflow: boolean;
  /** Flaechen mit echtem Ueberlauf (overflow auto/scroll und Inhalt > Hoehe). */
  scrollers: { id: string; scrollHeight: number; clientHeight: number }[];
  /** Flaechen mit Ueberlauf, die eine andere scrollende Flaeche enthalten. */
  nested: { inner: string; outer: string }[];
}

/**
 * Wo scrollt was? Eine Flaeche zaehlt als scrollend, wenn sie overflow-y
 * `auto`/`scroll` hat UND wirklich ueberlaeuft. Verschachtelt heisst: eine
 * solche Flaeche hat einen Vorfahren, der ebenfalls so scrollt.
 */
export const scrollReport = (page: Page): Promise<ScrollReport> =>
  page.evaluate(() => {
    const isScroller = (el: Element) => {
      const s = getComputedStyle(el);
      return (
        /(auto|scroll)/.test(s.overflowY) &&
        el.scrollHeight > el.clientHeight + 1
      );
    };
    const name = (el: Element) =>
      (el as HTMLElement).dataset.testid ??
      (el.id ||
        `${el.tagName.toLowerCase()}.${(el.getAttribute("class") ?? "").slice(0, 40)}`);
    const scrollers = Array.from(document.querySelectorAll("*")).filter(
      isScroller,
    );
    const nested: { inner: string; outer: string }[] = [];
    for (const el of scrollers) {
      let p = el.parentElement;
      while (p) {
        if (isScroller(p)) {
          nested.push({ inner: name(el), outer: name(p) });
          break;
        }
        p = p.parentElement;
      }
    }
    const main = document.getElementById("workspace-content")!;
    return {
      documentScrollHeight: document.scrollingElement!.scrollHeight,
      innerHeight,
      mainOverflow: main.scrollHeight > main.clientHeight + 1,
      scrollers: scrollers.map((el) => ({
        id: name(el),
        scrollHeight: el.scrollHeight,
        clientHeight: el.clientHeight,
      })),
      nested,
    };
  });

export interface Call {
  cmd: string;
  args: Record<string, unknown>;
}

/** Aufrufe eines Befehls, in Reihenfolge (nur seit dem Laden der Seite). */
export const calls = (page: Page, cmd: string): Promise<Call[]> =>
  page.evaluate(
    (c) => ((window as any).__calls as Call[]).filter((x) => x.cmd === c),
    cmd,
  );
