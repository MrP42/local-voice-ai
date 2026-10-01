import path from "node:path";
import { test, expect, type Page } from "@playwright/test";

// M3-P3c: Sprecher benennen, zusammenführen, umhängen (Detailansicht),
// "Mehrere Personen am Mikrofon" (RecorderCard), die Einstellungszeile und
// die Hinweise aus dem Bericht der Sprechertrennung, gegen die Tauri-Attrappe.
// Die Attrappe führt Namen, Zusammenführen und Umhängen selbst aus (wie das
// Backend) und merkt sich jeden Aufruf in `window.__calls`; Ereignisse löst
// `window.__emit` aus.

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
    w.__notices = [];
    w.__names = {} as Record<string, string>;
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
      seg(2, 1, 9500, 13000, 2, "Der Zeitplan steht, die Freigabe fehlt noch."),
      seg(3, 1, 13500, 18000, 1, "Dann schicke ich Ihnen das neue Angebot."),
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
    // Wie `SpeakerDirectory`: Name, sonst "Gegenseite n".
    const labelOf = (channel: number, index: number) =>
      w.__names[`${channel}:${index}`] ??
      `${channel === 1 ? "Gegenseite" : "Person"} ${index}`;
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
            case "meetings_start":
              return { ...w.__meetings[0], id: "m-neu", status: "recording" };
            case "meetings_get_segments":
              return w.__segments;
            case "meetings_segment_epoch":
              return w.__epoch;
            case "meetings_get_documents":
            case "action_items_list":
              return [];
            case "meeting_notes_get":
              // Die Notizen sind der erste Reiter der Arbeitsflaeche.
              return {
                meeting_id: args.meetingId,
                blocks: [],
                revision: 0,
                updated_at: 0,
              };
            case "meeting_speakers_list":
              return speakerList();
            case "meeting_speaker_notices":
              return w.__notices;
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
              if (args.epoch !== w.__epoch) throw "stale_epoch";
              const s = w.__segments.find(
                (x: any) => x.segment_index === args.segmentIndex,
              );
              if (!s) throw "segment_not_found";
              s.speaker_index = args.speakerIndex;
              return null;
            }
            case "meetings_set_diarize_mic":
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

const openDetail = async (page: Page) => {
  await page.setViewportSize({ width: 1280, height: 1000 });
  await page.goto("/");
  await page.getByRole("button", { name: "Aufnahmen", exact: true }).click();
  await page.getByText("Kundentermin Meyer", { exact: true }).click();
  await expect(page.locator('[data-segment-index="1"]')).toBeVisible();
};

const row = (page: Page, index: number) =>
  page.locator(`[data-segment-index="${index}"]`);

/** Das Label in der Zeile: Knopf (Sprecher) oder Text ("Ich"). */
const who = (page: Page, index: number) =>
  row(page, index).locator("span.truncate").first();

const popover = (page: Page) => page.getByTestId("speaker-popover");

const openSpeaker = async (page: Page, index: number) => {
  await row(page, index).getByTestId("speaker-label").click();
  await expect(popover(page)).toBeVisible();
};

test.describe("Sprecher im Transkript", () => {
  test("die Zeilen zeigen die Sprecher, ohne Trennung bleibt der Kanalname", async ({
    page,
  }) => {
    await openDetail(page);
    await expect(who(page, 0)).toHaveText("Ich");
    await expect(row(page, 0).getByTestId("speaker-label")).toHaveCount(0);
    await expect(who(page, 1)).toHaveText("Gegenseite 1");
    await expect(who(page, 2)).toHaveText("Gegenseite 2");
    await expect(who(page, 3)).toHaveText("Gegenseite 1");
    expect((await calls(page, "meeting_speakers_list"))[0].args).toEqual({
      meetingId: "m1",
    });
  });

  test("Umbenennen: Klick auf das Label, Name speichern, Popover schließt", async ({
    page,
  }) => {
    await openDetail(page);
    await openSpeaker(page, 1);
    await expect(popover(page)).toContainText("Gegenseite 1");
    // Anteil: 5 s + 4,5 s von 19,5 s Gesamtzeit = 48,7 %.
    await expect(popover(page)).toContainText("Redeanteil 49 %");
    const input = page.getByTestId("speaker-name-input");
    await expect(input).toBeFocused();
    await input.fill("  Anna Berg ");
    await input.press("Enter");
    await expect(popover(page)).toBeHidden();
    await expect(who(page, 1)).toHaveText("Anna Berg");

    const rename = await calls(page, "meeting_speaker_rename");
    expect(rename).toHaveLength(1);
    expect(rename[0].args).toEqual({
      meetingId: "m1",
      channel: 1,
      speakerIndex: 1,
      name: "Anna Berg",
    });
    // Das Feld zeigt beim erneuten Öffnen den gespeicherten Namen.
    await openSpeaker(page, 1);
    await expect(page.getByTestId("speaker-name-input")).toHaveValue(
      "Anna Berg",
    );
    await page.keyboard.press("Escape");
    await expect(popover(page)).toBeHidden();
  });

  test("der Name gilt für alle Segmente dieses Sprechers, andere bleiben", async ({
    page,
  }) => {
    await openDetail(page);
    await openSpeaker(page, 3);
    await page.getByTestId("speaker-name-input").fill("Anna Berg");
    await page.getByTestId("speaker-save").click();
    await expect(who(page, 1)).toHaveText("Anna Berg");
    await expect(who(page, 3)).toHaveText("Anna Berg");
    await expect(who(page, 2)).toHaveText("Gegenseite 2");
    await expect(who(page, 4)).toHaveText("Gegenseite 3");
    await expect(who(page, 0)).toHaveText("Ich");

    // Namen entfernen: zurück zum Standardlabel, in allen Segmenten.
    await openSpeaker(page, 1);
    await page.getByRole("button", { name: "Namen entfernen" }).click();
    await expect(who(page, 1)).toHaveText("Gegenseite 1");
    await expect(who(page, 3)).toHaveText("Gegenseite 1");
    const rename = await calls(page, "meeting_speaker_rename");
    expect(rename[1].args.name).toBeNull();
  });

  test("Zusammenführen: mit Rückfrage, alle Segmente gehen an den Zielsprecher", async ({
    page,
  }) => {
    await openDetail(page);
    await openSpeaker(page, 2);
    await page.getByTestId("speaker-merge-1").click();
    await expect(page.getByTestId("speaker-merge-confirm")).toContainText(
      "„Gegenseite 2“ geht in „Gegenseite 1“ auf",
    );
    // Abbrechen ändert nichts.
    await page.getByRole("button", { name: "Abbrechen" }).click();
    expect(await calls(page, "meeting_speaker_merge")).toHaveLength(0);
    await page.getByTestId("speaker-merge-1").click();
    await page.getByTestId("speaker-merge-do").click();
    await expect(popover(page)).toBeHidden();

    expect((await calls(page, "meeting_speaker_merge"))[0].args).toEqual({
      meetingId: "m1",
      channel: 1,
      from: 2,
      into: 1,
    });
    await expect(who(page, 2)).toHaveText("Gegenseite 1");
    await expect(who(page, 1)).toHaveText("Gegenseite 1");
    await expect(who(page, 4)).toHaveText("Gegenseite 3");
    // Die Sprecherliste wurde neu geladen: Gegenseite 2 gibt es nicht mehr.
    await openSpeaker(page, 4);
    await expect(page.getByTestId("speaker-merge-2")).toHaveCount(0);
    await expect(page.getByTestId("speaker-merge-1")).toBeVisible();
  });

  test("Segment umhängen: zu einem anderen oder zu einem neuen Sprecher", async ({
    page,
  }) => {
    await openDetail(page);
    await openSpeaker(page, 3);
    await page.getByTestId("speaker-move-2").click();
    await expect(popover(page)).toBeHidden();
    await expect(who(page, 3)).toHaveText("Gegenseite 2");
    // Nur dieses Segment: das andere Segment von Sprecher 1 bleibt.
    await expect(who(page, 1)).toHaveText("Gegenseite 1");
    let set = await calls(page, "meeting_segment_set_speaker");
    expect(set[0].args).toEqual({
      meetingId: "m1",
      segmentIndex: 3,
      epoch: 4,
      speakerIndex: 2,
    });

    // Neuer Sprecher: die nächste freie Nummer im Kanal (4).
    await openSpeaker(page, 1);
    await page.getByTestId("speaker-move-new").click();
    await expect(who(page, 1)).toHaveText("Gegenseite 4");
    set = await calls(page, "meeting_segment_set_speaker");
    expect(set[1].args).toMatchObject({ segmentIndex: 1, speakerIndex: 4 });
  });

  test("veraltete Epoche: Fehlermeldung im Popover, nichts ändert sich", async ({
    page,
  }) => {
    await openDetail(page);
    await page.evaluate(() => {
      (window as any).__epoch = 5; // Neu-Transkription lief dazwischen
    });
    await openSpeaker(page, 3);
    await page.getByTestId("speaker-move-2").click();
    await expect(popover(page).getByRole("alert")).toContainText(
      "Das Transkript wurde inzwischen ersetzt",
    );
    await expect(who(page, 3)).toHaveText("Gegenseite 1");
  });

  test("Änderungen von außen (Ereignis) laden Segmente und Namen neu", async ({
    page,
  }) => {
    await openDetail(page);
    await page.evaluate(() => {
      const w = window as any;
      w.__names["1:2"] = "Ben Kaya";
      w.__emit("speakers-changed", { meeting_id: "m1" });
    });
    await expect(who(page, 2)).toHaveText("Ben Kaya");
    // Ereignis einer anderen Besprechung wird ignoriert.
    await page.evaluate(() => {
      const w = window as any;
      w.__names["1:2"] = "Nicht meins";
      w.__emit("speakers-changed", { meeting_id: "andere" });
    });
    await page.waitForTimeout(200);
    await expect(who(page, 2)).toHaveText("Ben Kaya");
  });

  test("Hinweise aus dem Bericht stehen dezent unter der Werkzeugleiste", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      (window as any).__notices = ["model_missing", "mic_without_aec"];
    });
    await openDetail(page);
    const notices = page.getByTestId("speaker-notices");
    await expect(notices).toContainText("nicht installiert");
    await expect(notices.locator("[data-notice=mic_without_aec]")).toContainText(
      "ohne Echo-Unterdrückung",
    );
  });

  test("ohne Hinweise erscheint kein Hinweisblock", async ({ page }) => {
    await openDetail(page);
    await expect(page.getByTestId("speaker-notices")).toHaveCount(0);
  });
});

test.describe("Aufnahme und Einstellung", () => {
  // Die Optionen stehen seit M4 im Startdialog.
  const openStart = (page: Page) =>
    page.getByRole("button", { name: "Aufnahme starten" }).click();
  const confirmStart = (page: Page) =>
    page
      .getByRole("button", { name: "Alle Beteiligten haben zugestimmt" })
      .click();
  const startNow = async (page: Page) => {
    await openStart(page);
    await confirmStart(page);
  };

  test("Häkchen „Mehrere Personen am Mikrofon“ setzt diarize_mic der neuen Besprechung", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1280, height: 1000 });
    await page.goto("/");
    await page.getByRole("button", { name: "Aufnahmen", exact: true }).click();
    await openStart(page);
    const box = page.getByTestId("diarize-mic");
    await expect(box).not.toBeChecked();
    // ToggleSwitch: unsichtbares Eingabefeld unter dem sichtbaren Schalter (force).
    await box.check({ force: true });
    await confirmStart(page);
    await expect
      .poll(async () => (await calls(page, "meetings_set_diarize_mic")).length)
      .toBe(1);
    expect((await calls(page, "meetings_set_diarize_mic"))[0].args).toEqual({
      meetingId: "m-neu",
      enabled: true,
    });
  });

  test("ohne Häkchen wird diarize_mic nicht gesetzt", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1280, height: 1000 });
    await page.goto("/");
    await page.getByRole("button", { name: "Aufnahmen", exact: true }).click();
    await startNow(page);
    await expect
      .poll(async () => (await calls(page, "meetings_start")).length)
      .toBe(1);
    expect(await calls(page, "meetings_set_diarize_mic")).toHaveLength(0);
  });

  test("Präsenz (ohne Systemton): das Mikrofon wird ohnehin getrennt, kein Häkchen", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      (window as any).__settings.meeting_capture_system = false;
    });
    await page.setViewportSize({ width: 1280, height: 1000 });
    await page.goto("/");
    await page.getByRole("button", { name: "Aufnahmen", exact: true }).click();
    await openStart(page);
    await expect(page.getByTestId("capture-system")).not.toBeChecked();
    await expect(page.getByTestId("diarize-mic")).toHaveCount(0);
  });

  test("Einstellungszeile „Sprecher automatisch trennen“ schaltet auto/off", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1280, height: 1000 });
    await page.goto("/");
    await page
      .getByRole("navigation")
      .getByRole("button", { name: "Einstellungen", exact: true })
      .click();
    await expect(page.getByText("Sprecher automatisch trennen")).toBeVisible();
    const toggle = page
      .getByText("Sprecher automatisch trennen")
      .locator("xpath=ancestor::div[.//input[@type='checkbox']][1]")
      .locator("input[type=checkbox]");
    await expect(toggle).toBeChecked();
    await toggle.evaluate((el) => (el as HTMLInputElement).click());
    await expect
      .poll(
        async () =>
          (await calls(page, "change_meeting_diarization_setting")).length,
      )
      .toBe(1);
    expect(
      (await calls(page, "change_meeting_diarization_setting"))[0].args,
    ).toEqual({ mode: "off" });
  });
});

// Abnahmebild (P3c): nur mit `P3C_SCREENSHOT=1`, damit volle Läufe die Datei
// nicht überschreiben.
test("Abnahmebild: benannte Sprecher, Popover offen", async ({ page }) => {
  test.skip(!process.env.P3C_SCREENSHOT, "nur mit P3C_SCREENSHOT=1");
  await page.addInitScript(() => {
    const w = window as any;
    w.__names = { "1:1": "Anna Berg", "1:2": "Ben Kaya" };
    w.__notices = ["mic_without_aec"];
  });
  await openDetail(page);
  await expect(who(page, 1)).toHaveText("Anna Berg");
  await openSpeaker(page, 2);
  await page.waitForTimeout(150);
  const shot = path.resolve(
    process.cwd(),
    "../../koordination/granola-besprechungen/abnahme/p3c-sprecher.png",
  );
  await page.screenshot({ path: shot, animations: "disabled" });
});
