import { test, expect, type Page } from "@playwright/test";
import * as fs from "node:fs";
import * as path from "node:path";

// Ist-Aufnahmen der Seite "Aufnahmen" (Goal aufnahmen-ui, M1/U1).
// Kein Verhaltenstest: laeuft nur mit SCREENS_DIR und legt dort PNGs plus
// Messwerte (JSON) ab. Liegt bewusst unter koordination/, weil U1 keinen
// Produktcode anfasst; M2 kann die Spec nach tests/ uebernehmen und dieselben
// Szenen als Nachher-Bilder erzeugen.
//
// Aufruf (aus apps/local-voice, Config liegt im Scratchpad):
//   SCREENS_DIR=../../koordination/aufnahmen-ui/screens \
//   NODE_PATH=$PWD/node_modules pnpm exec playwright test \
//     --config <scratchpad>/u1_pw.config.mjs --reporter=line

const SCREENS_DIR = process.env.SCREENS_DIR;
test.skip(!SCREENS_DIR, "nur mit SCREENS_DIR (Aufnahmen, kein Verhaltenstest)");

const VIEWPORTS = {
  "1920": { width: 1920, height: 1050 },
  "1366": { width: 1366, height: 768 },
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
  deleted_at: null,
  ...over,
});

const MEETINGS = [
  meeting("m1", "Jour fixe Vertrieb KW 40", {
    started_at: T0 + 5 * 86400,
    duration_ms: 2_280_000,
  }),
  meeting("m2", "Kundentermin Stadtwerke Kaiserslautern – Lastgang", {
    started_at: T0 + 4 * 86400,
    source: "import",
    source_path: "C:/Users/patrick/Aufnahmen/SWK-Lastgang-2026-09-28.m4a",
    mic_audio_path: "C:/Users/patrick/Aufnahmen/SWK-Lastgang-2026-09-28.m4a",
    audio_retention_until: T0 + 34 * 86400,
    duration_ms: 3_540_000,
  }),
  meeting("m3", "Projektreview Netzpuls Sprint 12", {
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
const FOLDERS = [
  folder("f1", "Privat", 2),
  folder("f2", "Geschäftlich", 3),
  folder("f3", "Kunde Stadtwerke", 1),
  folder("f4", "Podcast", 1),
];

const LINES = [
  [1, "Guten Morgen zusammen, schön, dass es heute so kurzfristig klappt."],
  [0, "Gerne. Wir wollten uns ja den Lastgang vom letzten Quartal ansehen."],
  [1, "Genau. Die Spitzen liegen fast immer zwischen halb acht und neun."],
  [0, "Das passt zu dem, was wir aus den Zählerdaten sehen, vor allem montags."],
  [1, "Können wir die Spitzen mit dem Speicher glätten, ohne Leistung zuzukaufen?"],
  [0, "Teilweise. Mit 400 Kilowattstunden kappen wir etwa ein Drittel der Spitzen."],
  [1, "Und was bedeutet das für den Leistungspreis im nächsten Jahr?"],
  [0, "Grob gerechnet sinkt er um zwölf bis fünfzehn Prozent, je nach Tarif."],
  [1, "Das klingt gut, aber ich brauche das schriftlich für die Geschäftsführung."],
  [0, "Kein Problem, ich schicke bis Freitag eine Kurzfassung mit den Annahmen."],
] as const;
const SEGMENTS = Array.from({ length: 60 }, (_, i) => {
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

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(
    ({ meetings, folders, segments, minutesBody }) => {
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
      w.__recording = w.__recording ?? false;
      w.__position = w.__position ?? null;
      w.__progressList = w.__progressList ?? [];
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
            switch (cmd) {
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
                return args.meetingId === "m2" ? ["f2", "f3"] : [];
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
              case "meeting_speakers_list":
              case "meeting_speaker_notices":
              case "meeting_participants":
              case "meeting_templates_list":
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
      localStorage.setItem("lva.ui.settings.tab", "dictation");
    },
    {
      meetings: MEETINGS,
      folders: FOLDERS,
      segments: SEGMENTS,
      minutesBody: MINUTES_BODY,
    },
  );
});

const emit = (page: Page, payload: Record<string, unknown>) =>
  page.evaluate((p) => (window as any).__emit("meeting-event", p), payload);

/** 70 Minuten Audio, 42 % verarbeitet (P8a), ~24 Min. Rest. */
const PROGRESS = {
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

/** Messwerte: scrollt die Seite, welche Flaechen scrollen, wo steht was. */
const measure = (page: Page) =>
  page.evaluate(() => {
    const main = document.getElementById("workspace-content")!;
    const scrollables = Array.from(document.querySelectorAll("*"))
      .filter((el) => {
        const s = getComputedStyle(el);
        return (
          /(auto|scroll)/.test(s.overflowY) &&
          el.scrollHeight > el.clientHeight + 1
        );
      })
      .map((el) => {
        const r = el.getBoundingClientRect();
        return {
          el:
            (el as HTMLElement).dataset.testid ??
            el.id ??
            el.tagName.toLowerCase(),
          cls: (el.getAttribute("class") ?? "").slice(0, 80),
          scrollHeight: el.scrollHeight,
          clientHeight: el.clientHeight,
          top: Math.round(r.top),
          height: Math.round(r.height),
        };
      });
    const box = (sel: string) => {
      const el = document.querySelector(sel);
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return {
        top: Math.round(r.top),
        left: Math.round(r.left),
        width: Math.round(r.width),
        height: Math.round(r.height),
      };
    };
    const tabButton = Array.from(document.querySelectorAll("button")).find(
      (b) => b.textContent?.trim() === "Transkript",
    );
    const tabTop = tabButton
      ? Math.round(tabButton.getBoundingClientRect().top)
      : null;
    return {
      viewport: { w: innerWidth, h: innerHeight },
      document: {
        scrollHeight: document.scrollingElement!.scrollHeight,
        innerHeight,
      },
      main: {
        scrollHeight: main.scrollHeight,
        clientHeight: main.clientHeight,
        pageScrolls: main.scrollHeight > main.clientHeight + 1,
      },
      scrollables,
      tabTop,
      boxes: {
        jobPanel: box("[data-testid=job-panel]"),
        transcript: box("[data-testid=transcript-scroll]"),
        notesPad: box("[data-testid=live-notes-pad]"),
      },
    };
  });

const shoot = async (page: Page, name: string, fullPage = false) => {
  await page.waitForTimeout(250);
  await page.screenshot({
    path: path.join(SCREENS_DIR!, `${name}.png`),
    animations: "disabled",
    fullPage,
  });
  const m = await measure(page);
  fs.writeFileSync(
    path.join(SCREENS_DIR!, `${name}.json`),
    JSON.stringify(m, null, 2),
  );
  return m;
};

const openRecordings = async (page: Page, w: number, h: number) => {
  await page.setViewportSize({ width: w, height: h });
  await page.goto("/");
  const nav = page.getByRole("button", { name: "Aufnahmen", exact: true });
  if (!(await nav.isVisible())) {
    // Schmales Fenster: Menue ggf. zuerst aufklappen.
    const toggle = page.getByRole("button", { name: /Menü/ }).first();
    if (await toggle.isVisible()) await toggle.click();
  }
  await nav.click();
};

for (const [label, vp] of Object.entries(VIEWPORTS)) {
  test.describe(`Ist ${label}`, () => {
    test(`liste ${label}`, async ({ page }) => {
      await openRecordings(page, vp.width, vp.height);
      await expect(
        page.getByText("Jour fixe Vertrieb KW 40", { exact: true }),
      ).toBeVisible();
      await emit(page, PROGRESS);
      await shoot(page, `ist-liste-${label}`);
      // Wie weit reicht die Seite? Ganzseitig im Hauptbereich messen.
      const total = await page.evaluate(() => {
        const main = document.getElementById("workspace-content")!;
        return main.scrollHeight;
      });
      fs.writeFileSync(
        path.join(SCREENS_DIR!, `ist-liste-${label}-hoehe.json`),
        JSON.stringify({ mainScrollHeight: total }, null, 2),
      );
    });

    test(`live ${label}`, async ({ page }) => {
      await page.addInitScript(() => {
        const w = window as any;
        w.__recording = true;
        w.__position = { meeting_id: "m1", position_ms: 754_000 };
      });
      await openRecordings(page, vp.width, vp.height);
      await expect(page.getByTestId("live-notes-pad")).toBeVisible();
      await emit(page, {
        kind: "state",
        meeting_id: "m1",
        status: "recording",
        paused: false,
      });
      await page.waitForTimeout(200);
      await emit(page, {
        kind: "segments",
        meeting_id: "m1",
        appended: SEGMENTS.slice(0, 14),
      });
      await expect(page.getByText(LINES[3][1]).first()).toBeVisible();
      await page.getByTestId("note-starter").click();
      await page.keyboard.type("Lastspitzen 7:30–9:00, montags am höchsten", {
        delay: 2,
      });
      await page.keyboard.press("Enter");
      await page.keyboard.type("[ ] Kurzfassung bis Freitag", { delay: 2 });
      await shoot(page, `ist-live-${label}`);
    });

    test(`detail ${label}`, async ({ page }) => {
      await openRecordings(page, vp.width, vp.height);
      await page
        .getByText("Kundentermin Stadtwerke Kaiserslautern – Lastgang", {
          exact: true,
        })
        .click();
      await expect(page.locator('[data-segment-index="0"]')).toBeVisible();
      await shoot(page, `ist-detail-${label}`);
    });

    test(`fortschritt ${label}`, async ({ page }) => {
      await openRecordings(page, vp.width, vp.height);
      await page
        .getByText("Projektreview Netzpuls Sprint 12", { exact: true })
        .click();
      await expect(page.locator('[data-segment-index="0"]')).toBeVisible();
      await emit(page, PROGRESS);
      await expect(page.getByTestId("job-panel")).toBeVisible();
      await shoot(page, `ist-fortschritt-${label}`);
    });
  });
}

test("Ist 1366: Reiter Protokoll und Notizen, Chat offen (1920)", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  await page
    .getByText("Kundentermin Stadtwerke Kaiserslautern – Lastgang", {
      exact: true,
    })
    .click();
  await expect(page.locator('[data-segment-index="0"]')).toBeVisible();
  await page.getByRole("button", { name: "Protokoll", exact: true }).click();
  await page.waitForTimeout(300);
  await shoot(page, "ist-protokoll-1366");
  await page.getByRole("button", { name: "Notizen", exact: true }).click();
  await page.waitForTimeout(300);
  await shoot(page, "ist-notizen-1366");
  // Ganze Seite: wie lang die Detailansicht wirklich ist.
  await page.getByRole("button", { name: "Transkript", exact: true }).click();
  await page.setViewportSize({ width: 1920, height: 1050 });
  await page.getByRole("button", { name: /Fragen/ }).first().click();
  await page.waitForTimeout(400);
  await shoot(page, "ist-chat-1920");
});
