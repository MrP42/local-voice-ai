import fs from "node:fs";
import { test, expect, type Page } from "@playwright/test";

// #266: Sprecher zusammenführen und wieder trennen (Tabelle "Sprecher &
// Redeanteile"). Fall aus Patricks Bildschirmfoto: "Patrick Wolff" steht
// dreimal da, dazu "Aufnahme" (ohne Sprecher). Gegen eine Tauri-Attrappe, die
// Zusammenführen und Trennen wie das Backend ausführt (Segmente wandern,
// Zusammenführungen sind trennbar, die letzte zuerst).
// Abnahmebilder: docs/vorschau/sprecher-zusammenfuehren/.

// Abnahmebilder: standardmäßig unter test-results (ein Schreiben in den
// beobachteten Quellbaum lädt parallel laufende Seiten neu). Für die Ablage
// unter docs/vorschau: LV_SHOT_DIR=docs/vorschau/sprecher-zusammenfuehren, --workers=1.
const SHOTS =
  process.env.LV_SHOT_DIR ?? "test-results/vorschau/sprecher-zusammenfuehren";

type Call = { cmd: string; args: Record<string, any> };

const install = (page: Page, theme: "light" | "dark") =>
  page.addInitScript((theme) => {
    const w = window as any;
    const callbacks = new Map<number, (e: unknown) => void>();
    const listeners: Record<string, number[]> = {};
    const handlerOf = new Map<number, { event: string; handler: number }>();
    let callback = 0;
    w.__settings = {
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
    };
    w.__calls = [];
    w.__played = [];
    w.__names =
      w.__presetNames ??
      ({
        "2:1": "Peter Wöhrle",
        "2:2": "Patrick Wolff",
        "2:3": "Patrick Wolff",
        "2:4": "Patrick Wolff",
      } as Record<string, string>);
    w.__merges = [] as any[];
    w.__mergeSeq = 0;
    w.__people = [
      { id: "h1", name: "Anna Berg", meeting_count: 4 },
      { id: "h2", name: "Bernd Kaya", meeting_count: 2 },
    ].map((p) => ({ email: null, company: null, is_self: false, ...p }));
    const seg = (
      i: number,
      from: number,
      to: number,
      speaker: number | null,
      text: string,
    ) => ({
      segment_index: i,
      text,
      start_ms: from,
      end_ms: to,
      channel: 2,
      speaker_index: speaker,
    });
    // Import: Peter Wöhrle (1), Patrick dreifach erkannt (2, 3, 4), "Aufnahme".
    w.__segments = [
      seg(0, 0, 30000, 1, "Herzlich willkommen, wir starten mit dem Lagebild."),
      seg(1, 30000, 42000, 2, "Danke, ich ergänze kurz die Lizenzen."),
      seg(2, 42000, 47000, 3, "Wo liegen die Open-Project-Lizenzen?"),
      seg(3, 47000, 50000, 4, "Und die Ablage in Confluence?"),
      seg(4, 50000, 70000, 1, "Das klären wir mit dem Betrieb."),
      seg(5, 70000, 71000, null, "Die Aufnahme läuft."),
      seg(6, 71000, 80000, 2, "Dann nehme ich das als Aufgabe mit."),
    ];
    w.__meetings = [
      {
        id: "m1",
        title: "Komm.ONE",
        status: "ready",
        source: "import",
        started_at: 1793743020,
        ended_at: 1793747450,
        language: "de",
        mic_audio_path: "C:/audio/m1.wav",
        system_audio_path: null,
        duration_ms: 90000,
        consent_confirmed_at: 1793743020,
        audio_retention_until: null,
        created_at: 1793743020,
        source_path: "C:/Aufnahmen/Aufnahme 112.m4a",
        deleted_at: null,
      },
    ];
    const defaultLabel = (index: number) =>
      index === 0 ? "Aufnahme" : `Person ${index}`;
    const labelOf = (index: number) =>
      w.__names[`2:${index}`] ?? defaultLabel(index);
    const speakerList = () => {
      const total = w.__segments.reduce(
        (a: number, s: any) => a + s.end_ms - s.start_ms,
        0,
      );
      const ms: Record<number, number> = {};
      const first: Record<number, number> = {};
      for (const s of w.__segments) {
        const i = s.speaker_index ?? 0;
        ms[i] = (ms[i] ?? 0) + s.end_ms - s.start_ms;
        first[i] = Math.min(first[i] ?? Infinity, s.start_ms);
      }
      return Object.keys(ms)
        .map(Number)
        .sort((a, b) => a - b)
        .map((i) => ({
          channel: 2,
          speaker_index: i,
          label: labelOf(i),
          display_name: w.__names[`2:${i}`] ?? null,
          human_id: null,
          share_pct: Math.round((ms[i] * 1000) / total) / 10,
          speech_ms: ms[i],
          first_ms: first[i],
          former_names: [],
          default_label: defaultLabel(i),
        }));
    };
    const changed = () =>
      setTimeout(() => w.__emit("speakers-changed", { meeting_id: "m1" }), 0);
    const minutesBody = () => {
      // Wie das Backend beim Lesen: alte Bezeichnungen -> aktuelle Namen.
      let body =
        "# Protokoll: Komm.ONE\n\n**Datum:** 2026-11-03 · **Dauer:** 1:30 · **Vorlage:** Allgemein\n\n## Zusammenfassung\n\nPerson 2 stellte die laufenden Projekte vor.\n\n## Sprecher & Redeanteile\n\n| Sprecher | Redezeit | Anteil |\n|---|---|---|\n| Person 1 | 00:10 | 11,1 % |\n| Aufnahme | 00:04 | 4,4 % |\n| Person 2 | 01:16 | 84,4 % |\n\n## Offene Fragen\n\n- Wer übernimmt die Kosten?\n";
      for (const [key, name] of Object.entries(w.__names)) {
        const index = Number(key.split(":")[1]);
        const old = defaultLabel(index);
        body =
          index === 0
            ? body.replace(`| ${old} |`, `| ${name} |`)
            : body.replaceAll(old, name as string);
      }
      return body;
    };
    w.__emit = (event: string, payload: unknown) =>
      (listeners[event] ?? []).forEach((h) =>
        callbacks.get(h)?.({ event, id: 0, payload }),
      );
    // Wiedergabe nur mitschreiben: die Position beim play() ist der Probenbeginn.
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
              return 1;
            case "meetings_get_documents":
              return [
                {
                  id: "d1",
                  meeting_id: "m1",
                  kind: "minutes",
                  body_format: "markdown@1",
                  body: minutesBody(),
                  version: 1,
                  created_at: 1793747450,
                  template_id: null,
                  updated_at: 1793747450,
                },
              ];
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
            case "meeting_speaker_suggestions":
            case "meeting_participants":
              return [];
            case "people_list": {
              const q = String(args.query ?? "").toLowerCase();
              return w.__people.filter((p: any) =>
                p.name.toLowerCase().includes(q),
              );
            }
            case "meeting_speaker_merges":
              return w.__merges.map((m: any, i: number) => ({
                id: m.id,
                channel: 2,
                from: m.from,
                into: m.into,
                from_label: m.from_label,
                from_default_label:
                  m.from === 0 ? "Aufnahme" : `Person ${m.from}`,
                segment_count: m.segments.length,
                at: 0,
                undoable: !w.__merges
                  .slice(i + 1)
                  .some((l: any) => l.from === m.into),
              }));
            case "meeting_speaker_merge": {
              const { from, into } = args as { from: number; into: number };
              const segments = w.__segments
                .filter((s: any) => (s.speaker_index ?? 0) === from)
                .map((s: any) => s.segment_index);
              if (segments.length === 0 || into === 0) throw "speaker_invalid";
              for (const s of w.__segments)
                if ((s.speaker_index ?? 0) === from) s.speaker_index = into;
              const name = w.__names[`2:${from}`];
              const carried = !w.__names[`2:${into}`] && name ? name : null;
              if (carried) w.__names[`2:${into}`] = carried;
              const id = `m${++w.__mergeSeq}`;
              w.__merges.push({
                id,
                from,
                into,
                segments,
                from_label: labelOf(from),
                name: name ?? null,
                carried,
              });
              delete w.__names[`2:${from}`];
              changed();
              return id;
            }
            case "meeting_speaker_unmerge": {
              const at = w.__merges.findIndex(
                (m: any) => m.id === args.mergeId,
              );
              if (at < 0) throw "merge_not_found";
              const m = w.__merges[at];
              if (w.__merges.slice(at + 1).some((l: any) => l.from === m.into))
                throw "merge_blocked";
              w.__merges.splice(at, 1);
              for (const s of w.__segments)
                if (m.segments.includes(s.segment_index))
                  s.speaker_index = m.from === 0 ? null : m.from;
              if (m.name) w.__names[`2:${m.from}`] = m.name;
              if (m.carried) delete w.__names[`2:${m.into}`];
              changed();
              return null;
            }
            case "meeting_voice_matches": {
              // Wie das Backend: nur Paare, deren Sprecher es noch gibt.
              const alive = new Set(
                w.__segments.map((x: any) => x.speaker_index ?? 0),
              );
              const dismissed = w.__voiceDismissed ?? [];
              return {
                state: w.__voice ? "ready" : "model_missing",
                matches: (w.__voice ?? []).filter(
                  (m: any) =>
                    alive.has(m.a) &&
                    alive.has(m.b) &&
                    !dismissed.some((d: any) => d.a === m.a && d.b === m.b),
                ),
              };
            }
            case "meeting_voice_match_dismiss":
              (w.__voiceDismissed ??= []).push({ a: args.a, b: args.b });
              return null;
            case "meeting_speaker_rename": {
              const key = `${args.channel}:${args.speakerIndex}`;
              const name = (args.name ?? "").trim();
              if (name === "") delete w.__names[key];
              else w.__names[key] = name;
              setTimeout(
                () => w.__emit("speakers-changed", { meeting_id: "m1" }),
                0,
              );
              return speakerList().find(
                (s: any) => s.speaker_index === args.speakerIndex,
              );
            }
            case "meetings_minutes_state":
              return { running: false, progress: null };
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
            case "meetings_document_types":
            case "integrations_list":
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
  }, theme);

const calls = (page: Page, cmd: string) =>
  page.evaluate(
    (c) => (window as any).__calls.filter((x: Call) => x.cmd === c) as Call[],
    cmd,
  );

const played = (page: Page) =>
  page.evaluate(() => (window as any).__played as number[]);

const rows = (page: Page) => page.getByTestId("speaker-share-row");
const rowOf = (page: Page, key: string) =>
  page.locator(`[data-testid="speaker-share-row"][data-speaker="${key}"]`);

const open = async (
  page: Page,
  width = 1440,
  theme: "light" | "dark" = "light",
) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await install(page, theme);
  await page.emulateMedia({ colorScheme: theme });
  // 390 px: hoch genug, dass die Tabelle im Abnahmebild ganz zu sehen ist.
  await page.setViewportSize({ width, height: width < 600 ? 1700 : 1000 });
  await page.goto("/");
  if (width < 600)
    await page.getByRole("button", { name: "Menü öffnen" }).click();
  await page.getByRole("button", { name: "Aufnahmen", exact: true }).click();
  if (width < 600)
    await page.getByRole("button", { name: "Projekte öffnen" }).click();
  await page.getByText("Komm.ONE", { exact: true }).first().click();
  await page.getByRole("tab", { name: "Protokoll", exact: true }).click();
  await expect(page.getByTestId("minutes-speaker-section")).toBeVisible();
};

const mergeButton = (page: Page, key: string) =>
  rowOf(page, key).getByTestId("speaker-share-merge");

test.describe("Sprecher zusammenführen", () => {
  test("Gleicher Name: ein Klick führt zusammen, Rückgängig trennt wieder", async ({
    page,
  }) => {
    await open(page);
    await expect(rows(page)).toHaveCount(5);
    const hint = page.getByTestId("speaker-duplicate-hint");
    await expect(hint).toHaveCount(1);
    await expect(hint).toContainText("„Patrick Wolff“ steht 3-mal");
    await hint.getByTestId("speaker-duplicate-merge").click();

    // Ziel ist der Sprecher mit der längsten Redezeit (2: 21 s).
    const merge = await calls(page, "meeting_speaker_merge");
    expect(merge.map((c) => [c.args.from, c.args.into])).toEqual([
      [3, 2],
      [4, 2],
    ]);
    await expect(rows(page)).toHaveCount(3);
    await expect(page.getByTestId("speaker-duplicate-hint")).toHaveCount(0);
    await expect(page.getByTestId("speaker-merge-notice")).toContainText(
      "3 Sprecher zu „Patrick Wolff“ zusammengeführt.",
    );
    await expect(
      rowOf(page, "2:2").getByTestId("speaker-share-time"),
    ).toContainText("00:29");
    await expect(
      rowOf(page, "2:2").getByTestId("speaker-absorbed-item"),
    ).toHaveCount(2);

    // Rückgängig: neueste zuerst.
    await page.getByTestId("speaker-merge-undo").click();
    await expect(rows(page)).toHaveCount(5);
    const unmerge = await calls(page, "meeting_speaker_unmerge");
    expect(unmerge.map((c) => c.args.mergeId)).toEqual(["m2", "m1"]);
    await expect(page.getByTestId("speaker-merge-notice")).toContainText(
      "Sprecher wieder getrennt.",
    );
  });

  test("Zusammenführen je Zeile, auch „Aufnahme“; Trennen einzeln", async ({
    page,
  }) => {
    await open(page);
    // In der Zeile "Aufnahme" anfangen: sie geht immer im anderen auf.
    await mergeButton(page, "2:0").click();
    await expect(mergeButton(page, "2:0")).toHaveAttribute(
      "aria-expanded",
      "true",
    );
    const panel = rowOf(page, "2:0").getByTestId("speaker-merge-panel");
    await expect(panel).toContainText(
      "Mit wem ist „Aufnahme“ dieselbe Person?",
    );
    await expect(panel.getByTestId("speaker-merge-target")).toHaveCount(4);
    await panel
      .locator('[data-testid="speaker-merge-target"][data-speaker="2:1"]')
      .click();
    let merge = await calls(page, "meeting_speaker_merge");
    expect(merge.map((c) => [c.args.from, c.args.into])).toEqual([[0, 1]]);
    await expect(rowOf(page, "2:0")).toHaveCount(0);

    // Von der kleinen Zeile aus: Patrick 4 (3 s) in Patrick 3 (5 s).
    await mergeButton(page, "2:3").click();
    await rowOf(page, "2:3")
      .locator('[data-testid="speaker-merge-target"][data-speaker="2:4"]')
      .click();
    merge = await calls(page, "meeting_speaker_merge");
    expect(merge.map((c) => [c.args.from, c.args.into])[1]).toEqual([4, 3]);

    // Peter Wöhrle enthält jetzt "Aufnahme": einzeln trennen.
    const absorbed = rowOf(page, "2:1").getByTestId("speaker-absorbed-item");
    await expect(absorbed).toContainText("Aufnahme");
    await absorbed.getByTestId("speaker-absorbed-split").click();
    await expect(rowOf(page, "2:0")).toHaveCount(1);
    expect(
      (await calls(page, "meeting_speaker_unmerge")).map((c) => c.args.mergeId),
    ).toEqual(["m1"]);
  });

  test("Eine Kette trennt von hinten", async ({ page }) => {
    await open(page);
    await mergeButton(page, "2:4").click();
    await rowOf(page, "2:4")
      .locator('[data-testid="speaker-merge-target"][data-speaker="2:3"]')
      .click();
    await mergeButton(page, "2:3").click();
    await rowOf(page, "2:3")
      .locator('[data-testid="speaker-merge-target"][data-speaker="2:2"]')
      .click();
    // 2 enthält 3; 3 enthielt 4: der ältere Eintrag ist gesperrt.
    const items = rowOf(page, "2:2").getByTestId("speaker-absorbed-split");
    await expect(items).toHaveCount(1);
    await items.click();
    await expect(rowOf(page, "2:3")).toHaveCount(1);
    await rowOf(page, "2:3").getByTestId("speaker-absorbed-split").click();
    await expect(rows(page)).toHaveCount(5);
  });
});

test.describe("Touchziele und Abnahmebilder", () => {
  test.describe.configure({ timeout: 180_000 });
  const shot = (name: string, width: number, theme: string) =>
    `${SHOTS}/${name}-${width}-${theme === "light" ? "hell" : "dunkel"}.png`;
  for (const width of [390, 1440]) {
    for (const theme of ["light", "dark"] as const) {
      test(`${width} px ${theme}`, async ({ page }) => {
        await open(page, width, theme);
        const section = page.getByTestId("minutes-speaker-section");
        await section.scrollIntoViewIfNeeded();
        await page.mouse.move(0, 0);
        fs.mkdirSync(SHOTS, { recursive: true });
        // Der Knopf im Hinweis passt ganz hinein (bricht notfalls um).
        const hint = page.getByTestId("speaker-duplicate-hint");
        const action = hint.getByTestId("speaker-duplicate-merge");
        const [hb, ab] = [await hint.boundingBox(), await action.boundingBox()];
        expect(ab!.x).toBeGreaterThanOrEqual(hb!.x);
        expect(ab!.x + ab!.width).toBeLessThanOrEqual(hb!.x + hb!.width + 0.5);
        const clipped = await action.evaluate(
          (el) => el.scrollWidth > el.clientWidth + 1,
        );
        expect(clipped).toBe(false);
        await hint.scrollIntoViewIfNeeded();
        await page.screenshot({
          animations: "disabled",
          path: shot("1-hinweis", width, theme),
        });

        await mergeButton(page, "2:4").click();
        await page.mouse.move(0, 0);
        // Die Auswahl rollt ins Bild und ist frei bedienbar: in der Mitte
        // jedes Knopfes liegt der Knopf selbst (kein Verlauf, kein Hinweis).
        const panel = rowOf(page, "2:4").getByTestId("speaker-merge-panel");
        await expect(panel).toBeInViewport({ ratio: 1 });
        const buttons = panel.getByRole("button");
        for (let i = 0; i < (await buttons.count()); i++) {
          const free = await buttons.nth(i).evaluate((el) => {
            const r = el.getBoundingClientRect();
            const hit = document.elementFromPoint(
              r.left + r.width / 2,
              r.top + r.height / 2,
            );
            return !!hit && el.contains(hit);
          });
          expect(free).toBe(true);
        }
        await page.screenshot({
          animations: "disabled",
          path: shot("2-auswahl", width, theme),
        });
        await mergeButton(page, "2:4").click();

        await page.getByTestId("speaker-duplicate-merge").click();
        await expect(rows(page)).toHaveCount(3);
        await page.mouse.move(0, 0);
        // Touchziele nach dem Zusammenführen: alle Knöpfe mindestens 44 px.
        const targets = section.locator(
          '[data-testid="speaker-share-merge"], [data-testid="speaker-absorbed-split"], [data-testid="speaker-merge-undo"]',
        );
        const count = await targets.count();
        expect(count).toBe(6);
        const sizes: string[] = [];
        for (let i = 0; i < count; i++) {
          const box = await targets.nth(i).boundingBox();
          expect(box).not.toBeNull();
          expect(box!.height).toBeGreaterThanOrEqual(44);
          expect(box!.width).toBeGreaterThanOrEqual(44);
          sizes.push(`${Math.round(box!.width)}x${Math.round(box!.height)}`);
        }
        fs.appendFileSync(
          `${SHOTS}/touchziele.txt`,
          `${width} ${theme}: ${sizes.join(" ")}\n`,
        );
        const overflow = await page.evaluate(
          () =>
            document.documentElement.scrollWidth >
            document.documentElement.clientWidth,
        );
        expect(overflow).toBe(false);
        await section.screenshot({
          animations: "disabled",
          path: shot("3-zusammengefuehrt", width, theme),
        });
      });
    }
  }
});

// Stimmprofile 2a: Vorschlag "klingen gleich" aus Stimmabdrücken. Patrick ist
// einmal benannt (2), die Sprechertrennung hat ihn zusätzlich als "Person 3"
// geführt; das Backend meldet 87 % Ähnlichkeit.
const SHOTS_VOICE =
  process.env.LV_SHOT_DIR_VOICE ?? "test-results/vorschau/stimmen-vorschlag";

const openVoice = async (
  page: Page,
  width = 1440,
  theme: "light" | "dark" = "light",
) => {
  await page.addInitScript(() => {
    const w = window as any;
    w.__presetNames = { "2:1": "Peter Wöhrle", "2:2": "Patrick Wolff" };
    w.__voice = [{ channel: 2, a: 2, b: 3, score: 87 }];
  });
  await open(page, width, theme);
};

test.describe("Gleiche Stimmen", () => {
  test("Vorschlag führt zusammen, Rückgängig trennt wieder", async ({
    page,
  }) => {
    await openVoice(page);
    const hint = page.getByTestId("speaker-voice-hint");
    await expect(hint).toHaveCount(1);
    await expect(hint).toContainText(
      "„Person 3“ und „Patrick Wolff“ klingen ähnlich (Ähnlichkeit 87 %). Ist das dieselbe Person?",
    );
    await expect(hint).toContainText("keine sichere Erkennung");
    await expect(hint).toContainText(
      "„Verschiedene Personen“ gilt bis zur nächsten Sprechertrennung.",
    );
    await expect(page.getByTestId("speaker-duplicate-hint")).toHaveCount(0);
    await hint.getByTestId("speaker-voice-merge").click();
    const merge = await calls(page, "meeting_speaker_merge");
    // Patrick (21 s) bleibt, Person 3 (5 s) geht in ihm auf.
    expect(merge.map((c) => [c.args.from, c.args.into])).toEqual([[3, 2]]);
    await expect(rowOf(page, "2:3")).toHaveCount(0);
    await expect(page.getByTestId("speaker-voice-hint")).toHaveCount(0);
    await expect(page.getByTestId("speaker-merge-notice")).toContainText(
      "2 Sprecher zu „Patrick Wolff“ zusammengeführt.",
    );
    await page.getByTestId("speaker-merge-undo").click();
    await expect(rowOf(page, "2:3")).toHaveCount(1);
    await expect(page.getByTestId("speaker-voice-hint")).toHaveCount(1);
  });

  test("„Verschiedene Personen“ verwirft den Vorschlag dauerhaft", async ({
    page,
  }) => {
    await openVoice(page);
    await page.getByTestId("speaker-voice-dismiss").click();
    await expect(page.getByTestId("speaker-voice-hint")).toHaveCount(0);
    const sent = await calls(page, "meeting_voice_match_dismiss");
    expect(sent.map((c) => c.args)).toEqual([
      { meetingId: "m1", channel: 2, a: 2, b: 3 },
    ]);
    expect(await calls(page, "meeting_speaker_merge")).toHaveLength(0);
    // Neu laden der Tabelle (Umbenennen): der Vorschlag bleibt weg.
    await page.evaluate(() =>
      (window as any).__emit("speakers-changed", { meeting_id: "m1" }),
    );
    await expect(rows(page)).toHaveCount(5);
    await expect(page.getByTestId("speaker-voice-hint")).toHaveCount(0);
  });

  test("ohne Stimmmodell kein Hinweis", async ({ page }) => {
    await page.addInitScript(() => {
      (window as any).__presetNames = { "2:2": "Patrick Wolff" };
    });
    await open(page);
    await expect(rows(page)).toHaveCount(5);
    await expect(page.getByTestId("speaker-voice-hint")).toHaveCount(0);
  });

  test.describe("Abnahmebilder", () => {
    test.describe.configure({ timeout: 180_000 });
    for (const width of [390, 1440]) {
      for (const theme of ["light", "dark"] as const) {
        test(`${width} px ${theme}`, async ({ page }) => {
          await openVoice(page, width, theme);
          const hint = page.getByTestId("speaker-voice-hint");
          // Überschrift „Sprecher & Redeanteile“ direkt unter die feste Werkzeugleiste
          // rollen: kein Text halb unter der Leiste, der Hinweis ganz im Bild.
          await page.getByTestId("minutes-speaker-section").evaluate((el) => {
            const bar = document.querySelector(
              '[data-testid="minutes-toolbar"]',
            );
            (el as HTMLElement).style.scrollMarginTop =
              `${(bar?.getBoundingClientRect().height ?? 0) + 8}px`;
            el.scrollIntoView({ block: "start" });
          });
          await expect(hint).toBeInViewport({ ratio: 1 });
          await page.mouse.move(0, 0);
          // Beide Knöpfe passen in den Hinweis, sind frei und mindestens 44 px hoch.
          const hb = await hint.boundingBox();
          const sizes: string[] = [];
          for (const id of ["speaker-voice-merge", "speaker-voice-dismiss"]) {
            const button = hint.getByTestId(id);
            const box = await button.boundingBox();
            sizes.push(
              `${id} ${Math.round(box!.width)}x${Math.round(box!.height)}`,
            );
            expect(box!.x).toBeGreaterThanOrEqual(hb!.x);
            expect(box!.x + box!.width).toBeLessThanOrEqual(
              hb!.x + hb!.width + 0.5,
            );
            expect(box!.height).toBeGreaterThanOrEqual(44);
            const free = await button.evaluate((el) => {
              const r = el.getBoundingClientRect();
              const hit = document.elementFromPoint(
                r.left + r.width / 2,
                r.top + r.height / 2,
              );
              return !!hit && el.contains(hit);
            });
            expect(free).toBe(true);
            const clipped = await button.evaluate(
              (el) => el.scrollWidth > el.clientWidth + 1,
            );
            expect(clipped).toBe(false);
          }
          const overflow = await page.evaluate(
            () =>
              document.documentElement.scrollWidth >
              document.documentElement.clientWidth,
          );
          expect(overflow).toBe(false);
          fs.mkdirSync(SHOTS_VOICE, { recursive: true });
          fs.appendFileSync(
            `${SHOTS_VOICE}/masse.txt`,
            `Vorschlag ${width} ${theme}: ${sizes.join(", ")}\n`,
          );
          await page.screenshot({
            animations: "disabled",
            path: `${SHOTS_VOICE}/5-vorschlag-${width}-${theme === "light" ? "hell" : "dunkel"}.png`,
          });
        });
      }
    }
  });
});
