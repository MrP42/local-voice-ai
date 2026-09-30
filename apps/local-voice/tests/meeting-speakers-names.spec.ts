import fs from "node:fs";
import path from "node:path";
import { test, expect, type Page } from "@playwright/test";

// U8: Sprecher benennen in der neuen Oberfläche. Popover am Transkript mit
// Autovervollständigung aus den Personen, Menüeintrag "Sprecher benennen …" mit
// Dialog, Namensvorschläge aus dem Gesagten (übernehmen/verwerfen, nie von
// selbst) und die Einstellung "Mein Name". Gegen die Tauri-Attrappe: sie führt
// Namen, Personen, Vorschläge und Einstellung selbst aus (wie das Backend) und
// merkt sich jeden Aufruf in `window.__calls`.

type Call = { cmd: string; args: Record<string, any> };

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(() => {
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
    };
    w.__calls = [];
    w.__epoch = 4;
    w.__names = {} as Record<string, string>;
    w.__dismissed = [] as string[];
    w.__played = [];
    // Bekannte Personen (P5d).
    w.__people = [
      { id: "h1", name: "Anna Berg", meeting_count: 4 },
      { id: "h2", name: "André Kaya", meeting_count: 2 },
      { id: "h3", name: "Ben Müller", meeting_count: 1 },
    ].map((p) => ({ email: null, company: null, is_self: false, ...p }));
    // Vorschlag aus dem Gesagten: Sprecher 1 wurde von Sprecher 2 bedankt.
    w.__suggestions = [
      {
        channel: 1,
        speaker_index: 1,
        name: "André",
        confidence: 0.9,
        evidence: [
          {
            segment_index: 2,
            start_ms: 9500,
            quote: "Vielen Dank, André.",
            rule: "thanks",
          },
          {
            segment_index: 3,
            start_ms: 13500,
            quote: "Danke dir, André, das hilft uns.",
            rule: "thanks",
          },
          {
            segment_index: 4,
            start_ms: 18500,
            quote: "André, was meinst du dazu?",
            rule: "address",
          },
        ],
      },
    ];
    const seg = (
      i: number,
      channel: number,
      from: number,
      to: number,
      speaker: number | null,
      text: string,
    ) => ({
      segment_index: i,
      text,
      start_ms: from,
      end_ms: to,
      channel,
      speaker_index: speaker,
    });
    w.__segments = [
      seg(0, 0, 0, 4000, null, "Guten Morgen zusammen, danke fürs Kommen."),
      seg(1, 1, 4000, 9000, 1, "Wir haben das Budget für 2027 geprüft."),
      seg(2, 1, 9500, 13000, 2, "Vielen Dank, André. Der Zeitplan steht."),
      seg(3, 1, 13500, 18000, 1, "Dann schicke ich Ihnen das Angebot."),
      seg(4, 1, 18500, 21000, 3, "Ich ergänze noch die Konditionen."),
    ];
    w.__meetings = [
      {
        id: "m1",
        title: "Kundentermin Meyer",
        status: "ready",
        source: "live",
        started_at: 1790000000,
        ended_at: 1790000600,
        language: "de",
        mic_audio_path: null,
        system_audio_path: null,
        duration_ms: 600000,
        consent_confirmed_at: 1790000000,
        audio_retention_until: null,
        created_at: 1790000000,
        source_path: null,
        deleted_at: null,
      },
    ];
    const selfName = () => (w.__settings.meeting_self_name ?? "").trim();
    // Der dominante Sprecher am Mikrofon (Kanal 0) trägt "Mein Name".
    const dominantMic = () => {
      const ms: Record<number, number> = {};
      for (const s of w.__segments) {
        if (s.channel !== 0 || s.speaker_index === null) continue;
        ms[s.speaker_index] =
          (ms[s.speaker_index] ?? 0) + s.end_ms - s.start_ms;
      }
      const top = Object.entries(ms).sort((a, b) => b[1] - a[1])[0];
      return top ? Number(top[0]) : null;
    };
    const labelOf = (channel: number, index: number) => {
      const named = w.__names[`${channel}:${index}`];
      if (named) return named;
      if (channel === 0 && index === dominantMic() && selfName())
        return selfName();
      return `${channel === 1 ? "Gegenseite" : "Raum"} ${index}`;
    };
    const speakerList = () => {
      const total = w.__segments.reduce(
        (a: number, s: any) => a + s.end_ms - s.start_ms,
        0,
      );
      const ms: Record<string, number> = {};
      for (const s of w.__segments) {
        if (s.speaker_index === null) continue;
        const key = `${s.channel}:${s.speaker_index}`;
        ms[key] = (ms[key] ?? 0) + s.end_ms - s.start_ms;
      }
      return Object.keys(ms)
        .sort()
        .map((key) => {
          const [channel, index] = key.split(":").map(Number);
          return {
            channel,
            speaker_index: index,
            label: labelOf(channel, index),
            display_name: w.__names[key] ?? null,
            human_id: null,
            share_pct: Math.round((ms[key] * 1000) / total) / 10,
          };
        });
    };
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
          w.__calls.push({ cmd, args: JSON.parse(JSON.stringify(args)) });
          switch (cmd) {
            case "get_app_settings":
            case "get_default_settings":
              return w.__settings;
            case "plugin:os|locale":
              return "de-DE";
            case "plugin:app|version":
              return "0.14.0";
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
                listeners[entry.event] = (listeners[entry.event] ?? []).filter(
                  (h) => h !== entry.handler,
                );
                handlerOf.delete(args.eventId as number);
              }
              return null;
            }
            case "get_selected_model":
              return "";
            case "meetings_is_recording":
              return false;
            case "meetings_recording_position":
              return null;
            case "meetings_list":
              return w.__meetings;
            case "meetings_get_segments":
              return w.__segments;
            case "meetings_segment_epoch":
              return w.__epoch;
            case "meetings_get_documents":
            case "action_items_list":
              return [];
            case "meeting_notes_get":
              return {
                meeting_id: args.meetingId,
                blocks: [],
                revision: 0,
                updated_at: 0,
              };
            case "meeting_speakers_list":
              return speakerList();
            case "meeting_speaker_notices":
              return [];
            case "people_list": {
              const q = String(args.query ?? "").toLowerCase();
              return w.__people.filter((p: any) =>
                p.name.toLowerCase().includes(q),
              );
            }
            case "meeting_participants":
              // Benannte Sprecher sind Teilnehmende (P5d).
              return [...new Set(Object.values(w.__names))].map(
                (name: any) => ({
                  human_id: `h-${name}`,
                  name,
                  email: null,
                  company: null,
                  role: "speaker",
                  source: "speaker",
                  is_self: false,
                  meeting_count: 1,
                }),
              );
            case "meeting_speaker_suggestions":
              return w.__suggestions.filter(
                (s: any) =>
                  !w.__names[`${s.channel}:${s.speaker_index}`] &&
                  !w.__dismissed.includes(
                    `${s.channel}:${s.speaker_index}:${s.name}`,
                  ),
              );
            case "meeting_speaker_suggestion_dismiss":
              w.__dismissed.push(
                `${args.channel}:${args.speakerIndex}:${args.name}`,
              );
              return null;
            case "meeting_speaker_rename": {
              const key = `${args.channel}:${args.speakerIndex}`;
              const name = (args.name ?? "").trim();
              if (name === "") delete w.__names[key];
              else w.__names[key] = name;
              return speakerList().find(
                (s: any) =>
                  s.channel === args.channel &&
                  s.speaker_index === args.speakerIndex,
              );
            }
            case "meeting_speaker_merge": {
              const from = `${args.channel}:${args.from}`;
              const into = `${args.channel}:${args.into}`;
              for (const s of w.__segments) {
                if (s.channel === args.channel && s.speaker_index === args.from)
                  s.speaker_index = args.into;
              }
              if (w.__names[from] && !w.__names[into])
                w.__names[into] = w.__names[from];
              delete w.__names[from];
              return null;
            }
            case "meeting_segment_set_speaker": {
              const s = w.__segments.find(
                (x: any) => x.segment_index === args.segmentIndex,
              );
              if (!s) throw "segment_not_found";
              s.speaker_index = args.speakerIndex;
              return null;
            }
            case "change_meeting_self_name_setting":
              w.__settings.meeting_self_name = args.name;
              return null;
            case "tts_server_status":
              return { phase: "stopped", message: null };
            case "get_custom_sounds":
              return { start: false, stop: false };
            case "pages_list":
            case "page_files":
            case "tts_list_voices":
            case "tts_list_voice_infos":
            case "llm_ps":
            case "tts_reading_list":
            case "tts_list_downloads":
            case "llm_local_list":
            case "meeting_templates_list":
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
  });
});

const calls = (page: Page, cmd: string) =>
  page.evaluate(
    (c) => (window as any).__calls.filter((x: Call) => x.cmd === c) as Call[],
    cmd,
  );

/** Wiedergabe nur mitschreiben: die Position beim play() ist das Sprungziel. */
const withAudio = (page: Page) =>
  page.addInitScript(() => {
    const w = window as any;
    const times = new WeakMap<object, number>();
    Object.defineProperty(HTMLMediaElement.prototype, "currentTime", {
      configurable: true,
      get() {
        return times.get(this) ?? 0;
      },
      set(v: number) {
        times.set(this, v);
      },
    });
    HTMLMediaElement.prototype.play = function () {
      w.__played.push(times.get(this) ?? 0);
      return Promise.resolve();
    };
    HTMLMediaElement.prototype.pause = function () {};
    w.__meetings[0].mic_audio_path = "C:/audio/m1-mic.wav";
    w.__meetings[0].system_audio_path = "C:/audio/m1-system.wav";
  });

const openDetail = async (page: Page) => {
  await page.setViewportSize({ width: 1280, height: 1000 });
  await page.goto("/");
  await page.getByRole("button", { name: "Aufnahmen", exact: true }).click();
  await page.getByText("Kundentermin Meyer", { exact: true }).click();
  await expect(page.locator('[data-segment-index="1"]')).toBeVisible();
};

const row = (page: Page, index: number) =>
  page.locator(`[data-segment-index="${index}"]`);
const who = (page: Page, index: number) =>
  row(page, index).locator("span.truncate").first();
const popover = (page: Page) => page.getByTestId("speaker-popover");
const dialog = (page: Page) => page.getByTestId("speakers-dialog");
const hints = (page: Page) => page.getByTestId("name-suggestion");

const openSpeaker = async (page: Page, index: number) => {
  await row(page, index).getByTestId("speaker-label").click();
  await expect(popover(page)).toBeVisible();
};

/** Menü ☰ -> Eintrag (per JS geklickt: das Menü schließt beim Scrollen). */
const menuItem = async (page: Page, testId: string) => {
  await page.getByTestId("meeting-menu").click();
  await page.getByTestId(testId).evaluate((el) => (el as HTMLElement).click());
};

test.describe("Popover am Transkript", () => {
  test("Autovervollständigung: bekannte Personen erscheinen beim Tippen, ein Klick benennt mit genau diesem Namen", async ({
    page,
  }) => {
    await openDetail(page);
    await openSpeaker(page, 1);
    const input = page.getByTestId("speaker-name-input");
    await input.fill("an");
    const matches = popover(page).getByTestId("person-match");
    await expect(matches).toHaveText(["Anna Berg", "André Kaya"]);
    // Groß-/Kleinschreibung und Akzente zählen nicht.
    await input.fill("ANDRE");
    await expect(matches).toHaveText(["André Kaya"]);
    await input.fill("Anna");
    await expect(matches).toHaveText(["Anna Berg"]);
    await matches.first().click();

    await expect(popover(page)).toBeHidden();
    const rename = await calls(page, "meeting_speaker_rename");
    expect(rename).toHaveLength(1);
    expect(rename[0].args).toEqual({
      meetingId: "m1",
      channel: 1,
      speakerIndex: 1,
      name: "Anna Berg",
    });
    // Der Name gilt für alle Sätze dieses Sprechers.
    await expect(who(page, 1)).toHaveText("Anna Berg");
    await expect(who(page, 3)).toHaveText("Anna Berg");
    await expect(who(page, 2)).toHaveText("Gegenseite 2");
  });

  test("ein neuer Name geht auch: ohne Treffer keine Liste, Enter speichert wie getippt", async ({
    page,
  }) => {
    await openDetail(page);
    await openSpeaker(page, 2);
    const input = page.getByTestId("speaker-name-input");
    await input.fill("Wiebke Neu");
    await expect(popover(page).getByTestId("person-matches")).toHaveCount(0);
    await input.press("Enter");
    await expect(who(page, 2)).toHaveText("Wiebke Neu");
    expect((await calls(page, "meeting_speaker_rename"))[0].args.name).toBe(
      "Wiebke Neu",
    );
  });

  test("die drei Wege stehen im Popover: umbenennen, zusammenführen, nur diesen Satz zuordnen", async ({
    page,
  }) => {
    await openDetail(page);
    await openSpeaker(page, 2);
    await expect(popover(page)).toContainText("Umbenennen");
    await expect(popover(page).getByTestId("speaker-merge")).toContainText(
      "Mit anderem Sprecher zusammenführen",
    );
    await expect(popover(page).getByTestId("speaker-move")).toContainText(
      "Nur diesen Satz zuordnen",
    );
  });

  test("der Vorschlag für diesen Sprecher steht im Popover und lässt sich übernehmen", async ({
    page,
  }) => {
    await openDetail(page);
    // Sprecher 2 hat keinen Vorschlag.
    await openSpeaker(page, 2);
    await expect(page.getByTestId("speaker-suggestion")).toHaveCount(0);
    await page.keyboard.press("Escape");
    await openSpeaker(page, 1);
    await expect(page.getByTestId("speaker-suggestion")).toContainText(
      "Vermutlich André (3 Belege im Gespräch)",
    );
    await page.getByTestId("speaker-suggestion-accept").click();
    await expect(who(page, 1)).toHaveText("André");
    expect((await calls(page, "meeting_speaker_rename"))[0].args).toEqual({
      meetingId: "m1",
      channel: 1,
      speakerIndex: 1,
      name: "André",
    });
  });
});

test.describe("Menü: Sprecher benennen …", () => {
  test("der Dialog zeigt alle Sprecher mit Anzahl Sätze; Name speichern gilt überall", async ({
    page,
  }) => {
    await openDetail(page);
    await menuItem(page, "menu-speakers");
    await expect(dialog(page)).toBeVisible();
    const rows = dialog(page).getByTestId("speakers-dialog-row");
    await expect(rows).toHaveCount(3);
    await expect(rows.nth(0)).toContainText("Gegenseite 1");
    await expect(rows.nth(0)).toContainText("2 Sätze");
    await expect(rows.nth(1)).toContainText("1 Satz");
    // Ohne Audio gibt es keine Hörprobe.
    await expect(
      dialog(page).getByTestId("speakers-dialog-listen"),
    ).toHaveCount(0);

    const input = rows.nth(1).getByTestId("speakers-dialog-input");
    const save = rows.nth(1).getByTestId("speakers-dialog-save");
    await expect(save).toBeDisabled();
    await input.fill("Ben Müller");
    await save.click();
    await expect(who(page, 2)).toHaveText("Ben Müller");
    expect((await calls(page, "meeting_speaker_rename"))[0].args).toEqual({
      meetingId: "m1",
      channel: 1,
      speakerIndex: 2,
      name: "Ben Müller",
    });
    // Der Dialog zeigt den neuen Namen als Zeilenüberschrift.
    await expect(rows.nth(1)).toContainText("Ben Müller");
    // Auch die Kopfzeile kennt die Person jetzt.
    await expect(page.getByTestId("participant-chip").first()).toHaveAttribute(
      "title",
      /^Ben Müller /,
    );
  });

  test("im Dialog: Auswahl einer bekannten Person benennt sofort mit deren Namen", async ({
    page,
  }) => {
    await openDetail(page);
    await menuItem(page, "menu-speakers");
    const rows = dialog(page).getByTestId("speakers-dialog-row");
    await rows.nth(2).getByTestId("speakers-dialog-input").fill("anna");
    await rows.nth(2).getByTestId("person-match").first().click();
    await expect(who(page, 4)).toHaveText("Anna Berg");
    expect((await calls(page, "meeting_speaker_rename"))[0].args.name).toBe(
      "Anna Berg",
    );
  });

  test("Hörprobe springt zum längsten Satz des Sprechers", async ({ page }) => {
    await withAudio(page);
    await openDetail(page);
    await menuItem(page, "menu-speakers");
    const rows = dialog(page).getByTestId("speakers-dialog-row");
    await expect(
      rows.nth(0).getByTestId("speakers-dialog-listen"),
    ).toBeVisible();
    // Sprecher 1: Sätze ab 4 s (5 s) und 13,5 s (4,5 s): der längere beginnt bei 4 s.
    await rows.nth(0).getByTestId("speakers-dialog-listen").click();
    await expect
      .poll(() => page.evaluate(() => (window as any).__played))
      .toEqual([4]);
    await rows.nth(1).getByTestId("speakers-dialog-listen").click();
    await expect
      .poll(() => page.evaluate(() => (window as any).__played))
      .toEqual([4, 9.5]);
  });

  test("Menüeintrag ist ohne erkannte Sprecher gesperrt", async ({ page }) => {
    await page.addInitScript(() => {
      const w = window as any;
      for (const s of w.__segments) s.speaker_index = null;
    });
    await openDetail(page);
    await page.getByTestId("meeting-menu").click();
    await expect(page.getByTestId("menu-speakers")).toBeDisabled();
  });
});

test.describe("Namensvorschläge", () => {
  test("der Hinweis nennt Sprecher, Name und Belege; ohne Klick geschieht nichts", async ({
    page,
  }) => {
    await openDetail(page);
    await expect(hints(page)).toHaveCount(1);
    await expect(page.getByTestId("name-suggestion-text")).toHaveText(
      "Gegenseite 1 ist vermutlich André (3 Belege)",
    );
    await page.waitForTimeout(300);
    expect(await calls(page, "meeting_speaker_rename")).toHaveLength(0);
    await expect(who(page, 1)).toHaveText("Gegenseite 1");

    // Belege: Sätze mit Zeit.
    await page.getByTestId("name-suggestion-evidence-toggle").click();
    const evidence = page.getByTestId("name-suggestion-evidence");
    await expect(evidence).toContainText("0:09");
    await expect(evidence).toContainText("„Vielen Dank, André.“");
    await expect(evidence).toContainText("„André, was meinst du dazu?“");
  });

  test("Übernehmen benennt den Sprecher und der Hinweis verschwindet", async ({
    page,
  }) => {
    await openDetail(page);
    await page.getByTestId("name-suggestion-accept").click();
    await expect(who(page, 1)).toHaveText("André");
    await expect(who(page, 3)).toHaveText("André");
    await expect(hints(page)).toHaveCount(0);
    expect((await calls(page, "meeting_speaker_rename"))[0].args).toEqual({
      meetingId: "m1",
      channel: 1,
      speakerIndex: 1,
      name: "André",
    });
    await expect(page.getByTestId("participant-chip").first()).toHaveAttribute(
      "title",
      /^André /,
    );
  });

  test("Verwerfen entfernt den Hinweis dauerhaft und benennt nichts", async ({
    page,
  }) => {
    await openDetail(page);
    await page.getByTestId("name-suggestion-dismiss").click();
    await expect(hints(page)).toHaveCount(0);
    expect(
      (await calls(page, "meeting_speaker_suggestion_dismiss"))[0].args,
    ).toEqual({
      meetingId: "m1",
      channel: 1,
      speakerIndex: 1,
      name: "André",
    });
    expect(await calls(page, "meeting_speaker_rename")).toHaveLength(0);
    await expect(who(page, 1)).toHaveText("Gegenseite 1");
    // Eine Änderung von außen lädt neu: der verworfene Vorschlag kommt nicht wieder.
    await page.evaluate(() =>
      (window as any).__emit("speakers-changed", { meeting_id: "m1" }),
    );
    await page.waitForTimeout(300);
    await expect(hints(page)).toHaveCount(0);
  });

  test("auch im Dialog lässt sich der Vorschlag übernehmen oder verwerfen", async ({
    page,
  }) => {
    await openDetail(page);
    await menuItem(page, "menu-speakers");
    const suggestion = dialog(page).getByTestId("speakers-dialog-suggestion");
    await expect(suggestion).toHaveCount(1);
    await expect(suggestion).toContainText("Vermutlich André (3 Belege");
    await suggestion.getByTestId("speakers-dialog-suggestion-accept").click();
    await expect(who(page, 1)).toHaveText("André");
    await expect(suggestion).toHaveCount(0);
  });

  test("während einer wachsenden Aufnahme wird nichts vorgeschlagen", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      (window as any).__meetings[0].status = "processing";
    });
    await openDetail(page);
    await page.waitForTimeout(300);
    await expect(hints(page)).toHaveCount(0);
    expect(await calls(page, "meeting_speaker_suggestions")).toHaveLength(0);
  });
});

test.describe("Mein Name", () => {
  const settingsNav = (page: Page) =>
    page
      .getByRole("navigation")
      .getByRole("button", { name: "Einstellungen", exact: true });

  test("ohne Namen bleibt es bei „Ich“", async ({ page }) => {
    await openDetail(page);
    await expect(who(page, 0)).toHaveText("Ich");
  });

  test("der Kanal „Ich“ trägt den gesetzten Namen", async ({ page }) => {
    await page.addInitScript(() => {
      (window as any).__settings.meeting_self_name = "Patrick Wolff";
    });
    await openDetail(page);
    await expect(who(page, 0)).toHaveText("Patrick Wolff");
    await expect(who(page, 1)).toHaveText("Gegenseite 1");
  });

  test("der dominante Sprecher am getrennten Mikrofon trägt den Namen", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      const w = window as any;
      w.__settings.meeting_self_name = "Patrick Wolff";
      w.__segments.push(
        {
          segment_index: 5,
          text: "Ich erkläre kurz den Stand.",
          start_ms: 22000,
          end_ms: 32000,
          channel: 0,
          speaker_index: 1,
        },
        {
          segment_index: 6,
          text: "Kurze Ergänzung von mir.",
          start_ms: 32500,
          end_ms: 34000,
          channel: 0,
          speaker_index: 2,
        },
      );
    });
    await openDetail(page);
    await expect(who(page, 5)).toHaveText("Patrick Wolff");
    await expect(who(page, 6)).toHaveText("Raum 2");
    // Ein von Hand vergebener Name gewinnt.
    await openSpeaker(page, 5);
    await page.getByTestId("speaker-name-input").fill("Ben Müller");
    await page.getByTestId("speaker-save").click();
    await expect(who(page, 5)).toHaveText("Ben Müller");
  });

  test("Einstellung: Name eintragen speichert beim Verlassen, danach steht er im Transkript", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1280, height: 1000 });
    await page.goto("/");
    await settingsNav(page).click();
    const input = page.getByTestId("self-name-input");
    await expect(input).toBeVisible();
    await expect(input).toHaveValue("");
    await input.fill("  Patrick Wolff ");
    await input.press("Enter");
    await expect
      .poll(
        async () =>
          (await calls(page, "change_meeting_self_name_setting")).length,
      )
      .toBe(1);
    expect(
      (await calls(page, "change_meeting_self_name_setting"))[0].args,
    ).toEqual({ name: "Patrick Wolff" });

    await page.getByRole("button", { name: "Aufnahmen", exact: true }).click();
    await page.getByText("Kundentermin Meyer", { exact: true }).click();
    await expect(who(page, 0)).toHaveText("Patrick Wolff");

    // Wieder leeren: zurück zu „Ich“.
    await settingsNav(page).click();
    await input.fill("");
    await input.press("Enter");
    await expect
      .poll(
        async () =>
          (await calls(page, "change_meeting_self_name_setting")).length,
      )
      .toBe(2);
    expect(
      (await calls(page, "change_meeting_self_name_setting"))[1].args,
    ).toEqual({ name: null });
    // Die Besprechung bleibt geöffnet.
    await page.getByRole("button", { name: "Aufnahmen", exact: true }).click();
    await expect(who(page, 0)).toHaveText("Ich");
  });

  test("ein unveränderter Name löst keinen Speicheraufruf aus", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1280, height: 1000 });
    await page.goto("/");
    await settingsNav(page).click();
    const input = page.getByTestId("self-name-input");
    await input.focus();
    await input.blur();
    await page.waitForTimeout(200);
    expect(await calls(page, "change_meeting_self_name_setting")).toHaveLength(
      0,
    );
  });
});

test.describe("Der Name gilt überall", () => {
  test("Transkript, Kopfzeile, Dialog und Popover zeigen denselben Namen", async ({
    page,
  }) => {
    await openDetail(page);
    await openSpeaker(page, 3);
    await page.getByTestId("speaker-name-input").fill("Anna Berg");
    await page.getByTestId("speaker-save").click();

    await expect(who(page, 1)).toHaveText("Anna Berg");
    await expect(who(page, 3)).toHaveText("Anna Berg");
    await expect(page.getByTestId("participant-chip").first()).toHaveAttribute(
      "title",
      /^Anna Berg /,
    );
    // Die Namensvorschläge sprechen den Sprecher mit dem neuen Namen an
    // (dieser Sprecher ist jetzt benannt: kein Vorschlag mehr für ihn).
    await expect(hints(page)).toHaveCount(0);
    await menuItem(page, "menu-speakers");
    await expect(
      dialog(page).getByTestId("speakers-dialog-row").first(),
    ).toContainText("Anna Berg");
    await page.keyboard.press("Escape");
    await openSpeaker(page, 1);
    await expect(popover(page)).toContainText("Anna Berg");
    await expect(page.getByTestId("speaker-name-input")).toHaveValue(
      "Anna Berg",
    );
  });
});

// Abnahmebild (U8): nur mit `U8_SCREENSHOT=1`, damit volle Läufe nichts überschreiben.
test("Abnahmebild: Namensvorschlag, Popover und Dialog", async ({ page }) => {
  test.skip(!process.env.U8_SCREENSHOT, "nur mit U8_SCREENSHOT=1");
  await openDetail(page);
  await page.getByTestId("name-suggestion-evidence-toggle").click();
  await openSpeaker(page, 2);
  await page.getByTestId("speaker-name-input").fill("an");
  await page.waitForTimeout(150);
  const dir =
    process.env.U8_SHOT_DIR ??
    path.resolve(process.cwd(), "../../koordination/abnahme");
  fs.mkdirSync(dir, { recursive: true });
  await page.screenshot({
    path: path.join(dir, "u8-popover.png"),
    animations: "disabled",
  });
  await page.keyboard.press("Escape");
  await menuItem(page, "menu-speakers");
  await page.waitForTimeout(150);
  await page.screenshot({
    path: path.join(dir, "u8-dialog.png"),
    animations: "disabled",
  });
});
