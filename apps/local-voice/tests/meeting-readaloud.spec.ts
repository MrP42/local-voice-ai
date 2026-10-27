import { test, expect, type Page } from "@playwright/test";
import { markdownToSpeech } from "../src/lib/speechText";

// Aufnahmen: "Vorlesen" fuer erzeugte Inhalte (Protokoll, Dokumente) ueber den vorhandenen
// Befehl `tts_speak_text`, und der Status "Fertig" erscheint nirgends mehr (nur Laufendes
// und Fehler). Gegen die Tauri-Attrappe: Aufrufe in `window.__calls`, `tts_speak_text`
// haengt bis `window.__speakEnd()` oder `tts_cancel` (wie die echte Wiedergabe).

type Call = { cmd: string; args: Record<string, unknown> };

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
    const settings = {
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
    };
    const spec = {
      version: 1,
      context: "Zweck",
      sections: [
        { id: "a", title: "A", instruction: "", kind: "text" },
        { id: "b", title: "B", instruction: "", kind: "tasks" },
      ],
    };
    w.__settings = settings;
    w.__calls = [];
    w.__template = null; // gemerkte Wahl der Besprechung
    w.__auto = null; // was die Automatik zuletzt gewaehlt hat
    w.__meta = null; // Anzeigedaten des juengsten Protokolls
    w.__runState = {
      running: false,
      progress: null,
      cancelling: false,
      started_at: null,
    };
    // "ok" | "pending" | "busy" | Fehlertext
    w.__generate = "ok";
    w.__release = null; // beendet ein haengendes Erzeugen
    w.__documents = [];
    w.__templates = [
      {
        id: "builtin:allgemein",
        title: "Allgemein",
        builtin: true,
        spec,
        updated_at: 1,
      },
      {
        id: "builtin:vertrieb",
        title: "Kundengespräch / Vertrieb",
        builtin: true,
        spec,
        updated_at: 1,
      },
    ];
    w.__meetings = [
      {
        id: "m1",
        title: "Kundentermin Meyer",
        status: "ready",
        source: "recording",
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
      },
    ];
    w.__minutesDoc = (version: number, body: string) => ({
      id: `p${version}`,
      meeting_id: "m1",
      kind: "minutes",
      body_format: "markdown@1",
      body,
      version,
      created_at: 1790000900 + version,
      template_id: null,
      updated_at: 300 + version,
    });
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
        invoke: async (cmd: string, args: Record<string, unknown> = {}) => {
          w.__calls.push({ cmd, args });
          switch (cmd) {
            case "get_app_settings":
            case "get_default_settings":
              return settings;
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
            case "meetings_start":
              return { ...w.__meetings[0], id: "m-neu", status: "recording" };
            case "meetings_is_recording":
              return false;
            case "meetings_recording_position":
              return null;
            case "meetings_list":
              return [...(w.__extraMeetings ?? []), ...w.__meetings];
            case "meetings_get_segments":
              return [];
            case "meetings_get_documents":
              return w.__documents;
            case "meetings_minutes_file":
              return null;
            case "meetings_get_template":
              return w.__template;
            case "meetings_set_template":
              w.__template = args.templateId;
              return null;
            case "meetings_get_auto_template":
              return w.__auto;
            case "meetings_minutes_meta":
              return w.__meta;
            case "meetings_minutes_state":
              return w.__runState;
            case "meetings_generate_minutes": {
              const mode = w.__generate as string;
              if (mode === "busy") throw "minutes_busy";
              if (mode === "pending") {
                w.__runState = {
                  running: true,
                  progress: { phase: "write", done: 1, total: 4 },
                  cancelling: false,
                  started_at: 1790001000,
                };
                await new Promise<void>((resolve) => {
                  w.__release = resolve;
                });
              } else if (mode !== "ok") {
                throw mode;
              }
              w.__documents = [
                ...w.__documents,
                w.__minutesDoc(1, "# Protokoll: Kundentermin Meyer\n\nFertig."),
              ];
              return w.__documents[w.__documents.length - 1];
            }
            case "meeting_notes_get":
              // Die Notizen sind der erste Reiter der Arbeitsflaeche.
              return {
                meeting_id: args.meetingId,
                blocks: [],
                revision: 0,
                updated_at: 0,
              };
            case "meeting_templates_list":
              return w.__templates;
            case "plugin:dialog|save":
              return w.__saveTarget ?? null;
            case "tts_speak_text":
            case "tts_speak_text_at":
            case "tts_speak_resume":
            case "tts_speak_seek": {
              w.__speaking = true;
              // Der Befehl kehrt erst nach der Wiedergabe zurueck. Ein neuer Lauf
              // (Weiter, Satzsprung) loest den laufenden ab, wie im Backend.
              const previous = w.__speakEnd;
              await new Promise<void>((resolve) => {
                w.__speakEnd = resolve;
                previous?.();
              });
              w.__speaking = false;
              if (w.__speakError) throw w.__speakError;
              return null;
            }
            case "tts_cancel":
              w.__speakEnd?.();
              return null;
            case "tts_speak_to_file":
              return args.outPath;
            case "tts_list_voices":
              return w.__voices ?? [];
            case "tts_server_status":
              return { phase: "stopped", message: null };
            case "get_custom_sounds":
              return { start: false, stop: false };
            case "pages_list":
            case "page_files":
            case "tts_list_voice_infos":
            case "llm_ps":
            case "tts_reading_list":
            case "tts_list_downloads":
            case "llm_local_list":
            case "action_items_list":
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

const openRecordings = async (page: Page) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/");
  await page.getByRole("button", { name: "Aufnahmen", exact: true }).click();
};

const MINUTES = [
  "# Protokoll: Kundentermin Meyer",
  "",
  "## Beschlüsse",
  "",
  "- **Angebot** bis Freitag senden [F3]",
  "- Termin im [Kalender](https://example.org) eintragen",
  "",
  "| Wer | Was |",
  "| --- | --- |",
  "| Anna | Angebot |",
].join("\n");

const SPOKEN =
  "Protokoll: Kundentermin Meyer.\n\nBeschlüsse.\n\nAngebot bis Freitag senden.\nTermin im Kalender eintragen.\n\nWer, Was.\nAnna, Angebot.";

const withMinutes = (page: Page) =>
  page.addInitScript((body) => {
    const w = window as any;
    w.__documents = [w.__minutesDoc(1, body)];
  }, MINUTES);

const center = async (page: Page, testId: string) => {
  const box = await page.getByTestId(testId).boundingBox();
  if (!box) throw new Error(`${testId} nicht sichtbar`);
  return { x: box.x + box.width / 2, y: box.y + box.height / 2, ...box };
};

/** Eine zweite fertige Aufnahme (gleiche Dokumente wie die erste, anderer Schluessel). */
const withSecondMeeting = (page: Page) =>
  page.addInitScript(() => {
    const w = window as any;
    w.__extraMeetings = [
      {
        ...w.__meetings[0],
        id: "m2",
        title: "Zweiter Termin Schulz",
        started_at: 1790100000,
        created_at: 1790100000,
      },
    ];
  });

const openMinutes = async (page: Page) => {
  await openRecordings(page);
  await page.getByText("Kundentermin Meyer", { exact: true }).click();
  await page.getByRole("tab", { name: "Protokoll", exact: true }).click();
};

test.describe("Vorlesen in den Aufnahmen", () => {
  test("Markdown wird zu Klartext: ohne Zeichen, Belege und Tabellenstriche", () => {
    expect(markdownToSpeech(MINUTES)).toBe(SPOKEN);
    expect(markdownToSpeech("")).toBe("");
    expect(markdownToSpeech("**Nur** _fett_ und `Code`")).toBe(
      "Nur fett und Code",
    );
    expect(markdownToSpeech("1. Erstens\n2. Zweitens.\n- [x] Erledigt")).toBe(
      "Erstens.\nZweitens.\nErledigt.",
    );
    expect(markdownToSpeech("snake_case_name bleibt, 2 * 3 auch")).toBe(
      "snake_case_name bleibt, 2 * 3 auch",
    );
  });

  test("Vorlesen-Knopf blendet die Steuerung in der Werkzeugzeile ein, Stopp blendet sie aus", async ({
    page,
  }) => {
    await withMinutes(page);
    await openMinutes(page);
    const speak = page.getByTestId("minutes-readaloud");
    const bar = page.getByTestId("minutes-readaloud-bar");
    await expect(speak).toHaveAttribute("aria-label", "Vorlesen");
    await expect(bar).toHaveCount(0);

    await speak.click();
    // Der vorhandene Vorlese-Befehl bekommt den angezeigten Text als Klartext.
    await expect
      .poll(async () => (await calls(page, "tts_speak_text")).length)
      .toBe(1);
    expect((await calls(page, "tts_speak_text"))[0].args).toEqual({
      text: SPOKEN,
    });

    // Die Steuerung steht IN der Werkzeugzeile (keine zweite Zeile darunter):
    // gleiche Mittellinie wie "Neu erzeugen", "Kopieren", Stimmenmenue und "Audio speichern".
    await expect(bar).toBeVisible();
    await expect(speak).toHaveCount(0);
    await expect(
      page.getByTestId("minutes-toolbar").getByTestId("minutes-readaloud-bar"),
    ).toBeVisible();
    const row = (await center(page, "minutes-generate")).y;
    for (const id of [
      "minutes-copy",
      "minutes-readaloud-bar",
      "minutes-readaloud-bar-voice-menu",
      "minutes-readaloud-save",
    ]) {
      expect(Math.abs((await center(page, id)).y - row)).toBeLessThan(4);
    }
    const toolbar = await page.getByTestId("minutes-toolbar").boundingBox();
    const document = await page.getByTestId("minutes-doc").boundingBox();
    expect(document!.y).toBeGreaterThan(toolbar!.y + toolbar!.height - 1);
    expect(toolbar!.height).toBeLessThan(44);

    // Stopp: Steuerung weg, Knopf zurueck auf Vorlesen.
    await page.getByTestId("minutes-readaloud-bar-stop").click();
    await expect
      .poll(async () => (await calls(page, "tts_cancel")).length)
      .toBe(1);
    await expect(bar).toHaveCount(0);
    await expect(speak).toHaveAttribute("aria-label", "Vorlesen");
    // Kein Fehler-Toast nach eigenem Stopp.
    await expect(page.locator("[data-sonner-toast]")).toHaveCount(0);

    // Zweiter Lauf, diesmal bis zum Ende: die Steuerung verschwindet von selbst.
    await speak.click();
    await expect(bar).toBeVisible();
    await page.evaluate(() => (window as any).__speakEnd());
    await expect(bar).toHaveCount(0);
    await expect(speak).toHaveAttribute("aria-label", "Vorlesen");
    expect((await calls(page, "tts_speak_text")).length).toBe(2);
  });

  test("Reiterwechsel pausiert die Wiedergabe, die Steuerung bleibt und Weiter setzt an derselben Stelle fort", async ({
    page,
  }) => {
    await withMinutes(page);
    await openMinutes(page);
    await page.getByTestId("minutes-readaloud").click();
    const bar = page.getByTestId("minutes-readaloud-bar");
    await expect(bar).toBeVisible();
    await page.evaluate(() => {
      const w = window as any;
      w.__emit("tts-current-sentence", {
        context: "speak",
        index: 2,
        text: "Angebot bis Freitag senden.",
      });
      w.__emit("tts-speak-progress", { position: 2, total: 6 });
    });
    await expect(
      page.getByTestId("minutes-readaloud-bar-position"),
    ).toContainText("3/6");

    await page.getByRole("tab", { name: "Transkript", exact: true }).click();
    // Es laeuft kein Ton weiter: angehalten, nicht neu gestartet.
    await expect
      .poll(async () => (await calls(page, "tts_cancel")).length)
      .toBe(1);
    expect((await calls(page, "tts_speak_text")).length).toBe(1);

    // Zurueck am selben Dokument: Steuerung ist da, pausiert, selber Satz.
    await page.getByRole("tab", { name: "Protokoll", exact: true }).click();
    await expect(bar).toBeVisible();
    const toggle = page.getByTestId("minutes-readaloud-bar-toggle");
    await expect(toggle).toHaveAttribute("aria-label", "Weiter");
    await expect(
      page.getByTestId("minutes-readaloud-bar-position"),
    ).toContainText("3/6");
    // Weiter: die vorhandene Sitzung wird fortgesetzt (kein Neustart mit dem Text).
    await toggle.click();
    await expect
      .poll(async () => (await calls(page, "tts_speak_resume")).length)
      .toBe(1);
    await expect(toggle).toHaveAttribute("aria-label", "Pause");
    expect((await calls(page, "tts_speak_text")).length).toBe(1);
    expect((await calls(page, "tts_speak_text_at")).length).toBe(0);
    await expect(page.locator("[data-sonner-toast]")).toHaveCount(0);

    // Ein Fehler des Vorlesens steht als Hinweis da.
    await page.getByTestId("minutes-readaloud-bar-stop").click();
    await expect(bar).toHaveCount(0);
    await page.evaluate(() => {
      (window as any).__speakError = "Der Sprachserver läuft nicht";
    });
    await page.getByTestId("minutes-readaloud").click();
    await page.evaluate(() => (window as any).__speakEnd());
    await expect(page.locator("[data-sonner-toast]")).toContainText(
      "Vorlesen fehlgeschlagen: Der Sprachserver läuft nicht",
    );
  });

  test("Aufnahmewechsel pausiert; Rueckkehr: Weiter an derselben Stelle, nach einer anderen Wiedergabe Fortsetzen ab dem gemerkten Satz", async ({
    page,
  }) => {
    await withSecondMeeting(page);
    await withMinutes(page);
    await openMinutes(page);
    const bar = page.getByTestId("minutes-readaloud-bar");
    const toggle = page.getByTestId("minutes-readaloud-bar-toggle");
    const position = page.getByTestId("minutes-readaloud-bar-position");
    const openRecording = async (title: string) => {
      await page.getByText(title, { exact: true }).click();
      await page.getByRole("tab", { name: "Protokoll", exact: true }).click();
    };

    await page.getByTestId("minutes-readaloud").click();
    await expect(bar).toBeVisible();
    await page.evaluate(() => {
      const w = window as any;
      w.__emit("tts-current-sentence", {
        context: "speak",
        index: 2,
        text: "Angebot bis Freitag senden.",
      });
      w.__emit("tts-speak-progress", { position: 2, total: 6 });
    });
    await expect(position).toContainText("3/6");

    // Andere Aufnahme: die Wiedergabe pausiert (kein Ton), dort gibt es keine Steuerung.
    await openRecording("Zweiter Termin Schulz");
    await expect
      .poll(async () => (await calls(page, "tts_cancel")).length)
      .toBe(1);
    await expect(page.getByTestId("minutes-readaloud")).toBeVisible();
    await expect(bar).toHaveCount(0);

    // Zurueck: dieselbe Sitzung, "Weiter" setzt fort -- ohne Neustart.
    await openRecording("Kundentermin Meyer");
    await expect(bar).toBeVisible();
    await expect(toggle).toHaveAttribute("aria-label", "Weiter");
    await expect(position).toContainText("3/6");
    expect((await calls(page, "tts_speak_text")).length).toBe(1);

    // Wieder weg, und dort eine NEUE Wiedergabe: sie ersetzt die globale Sitzung.
    await openRecording("Zweiter Termin Schulz");
    await page.getByTestId("minutes-readaloud").click();
    await expect
      .poll(async () => (await calls(page, "tts_speak_text")).length)
      .toBe(2);

    // Zurueck bei der ersten: ehrlich "ab Satz 3 fortsetzen"; die Sprungknoepfe ruhen.
    await openRecording("Kundentermin Meyer");
    await expect(bar).toBeVisible();
    await expect(bar).toHaveAttribute("data-view", "saved");
    await expect(toggle).toHaveAttribute("aria-label", "Ab Satz 3 fortsetzen");
    await expect(position).toContainText("3/6");
    await expect(page.getByTestId("minutes-readaloud-bar-next")).toBeDisabled();
    // Es hat nichts geklungen, als man zurueckkam: die fremde Wiedergabe ist pausiert.
    const resumes = (await calls(page, "tts_speak_resume")).length;

    await toggle.click();
    await expect
      .poll(async () => (await calls(page, "tts_speak_text_at")).length)
      .toBe(1);
    expect((await calls(page, "tts_speak_text_at"))[0].args).toEqual({
      text: SPOKEN,
      sentenceIndex: 2,
    });
    expect((await calls(page, "tts_speak_resume")).length).toBe(resumes);
    // Jetzt ist es wieder die laufende Sitzung dieser Aufnahme.
    await expect(bar).toHaveAttribute("data-view", "live");
    await expect(toggle).toHaveAttribute("aria-label", "Pause");
    await expect(position).toContainText("3/6");

    // Eine gemerkte Stelle laesst sich verwerfen: Stopp blendet die Steuerung aus.
    await openRecording("Zweiter Termin Schulz");
    await page.getByTestId("minutes-readaloud").click();
    await openRecording("Kundentermin Meyer");
    await expect(bar).toHaveAttribute("data-view", "saved");
    await page.getByTestId("minutes-readaloud-bar-stop").click();
    await expect(bar).toHaveCount(0);
    await expect(page.getByTestId("minutes-readaloud")).toBeVisible();
  });

  test("ohne Protokoll gibt es keinen Vorlesen-Knopf", async ({ page }) => {
    await openMinutes(page);
    await expect(page.getByTestId("minutes-empty")).toBeVisible();
    await expect(page.getByTestId("minutes-readaloud")).toHaveCount(0);
  });

  test("Wiedergabeleiste: Pause/Weiter, Satzsprung, Stopp, Satzanzeige", async ({
    page,
  }) => {
    await withMinutes(page);
    await openMinutes(page);
    const bar = page.getByTestId("minutes-readaloud-bar");
    // Ohne Vorlesen keine Leiste.
    await expect(bar).toHaveCount(0);

    await page.getByTestId("minutes-readaloud").click();
    await expect(bar).toBeVisible();
    await page.evaluate(() => {
      const w = window as any;
      w.__emit("tts-current-sentence", {
        context: "speak",
        index: 1,
        text: "Beschlüsse.",
      });
      w.__emit("tts-speak-progress", { position: 1, total: 6 });
    });
    // Kompakter Zaehler; voller Wortlaut fuer Screenreader, der Satz selbst im Tooltip.
    const position = page.getByTestId("minutes-readaloud-bar-position");
    await expect(position).toContainText("2/6");
    await expect(position).toContainText("Satz 2 von 6");
    await expect(position).toHaveAttribute("title", "Beschlüsse.");
    await expect(
      page.getByTestId("minutes-readaloud-bar-sentence"),
    ).toHaveCount(0);

    // Pause: dasselbe Anhalten wie auf der Vorlesen-Seite; die Leiste bleibt, Weiter erscheint.
    const toggle = page.getByTestId("minutes-readaloud-bar-toggle");
    await expect(toggle).toHaveAttribute("aria-label", "Pause");
    await toggle.click();
    await expect
      .poll(async () => (await calls(page, "tts_cancel")).length)
      .toBe(1);
    await expect(toggle).toHaveAttribute("aria-label", "Weiter");
    await expect(bar).toBeVisible();
    // Kein Fehler-Toast nach eigener Pause.
    await expect(page.locator("[data-sonner-toast]")).toHaveCount(0);

    // Weiter: setzt die Sitzung fort (kein neuer Start mit demselben Text).
    await toggle.click();
    await expect
      .poll(async () => (await calls(page, "tts_speak_resume")).length)
      .toBe(1);
    await expect(toggle).toHaveAttribute("aria-label", "Pause");
    expect((await calls(page, "tts_speak_text")).length).toBe(1);

    // Satzsprung vor und zurueck: der Befehl der Vorlesen-Seite mit +1 / -1.
    await page.getByTestId("minutes-readaloud-bar-next").click();
    await page.getByTestId("minutes-readaloud-bar-prev").click();
    await expect
      .poll(async () => (await calls(page, "tts_speak_seek")).length)
      .toBe(2);
    expect(
      (await calls(page, "tts_speak_seek")).map((c) => c.args.delta),
    ).toEqual([1, -1]);
    await expect(toggle).toHaveAttribute("aria-label", "Pause");

    // Stopp: Leiste weg, Knopf zurueck auf Vorlesen.
    await page.getByTestId("minutes-readaloud-bar-stop").click();
    await expect(bar).toHaveCount(0);
    await expect(page.getByTestId("minutes-readaloud")).toHaveAttribute(
      "aria-label",
      "Vorlesen",
    );
    await expect(page.locator("[data-sonner-toast]")).toHaveCount(0);
  });

  test("Wiedergabeleiste: Ende der Sitzung blendet sie aus; angehalten von aussen bleibt Weiter", async ({
    page,
  }) => {
    await withMinutes(page);
    await openMinutes(page);
    const bar = page.getByTestId("minutes-readaloud-bar");
    await page.getByTestId("minutes-readaloud").click();
    await expect(bar).toBeVisible();
    // Von aussen angehalten (Vorlesen-Seite): nicht am Ende, also Weiter moeglich.
    await page.evaluate(() => {
      const w = window as any;
      w.__emit("tts-speak-progress", { position: 2, total: 6 });
      w.__speakEnd();
    });
    await expect(
      page.getByTestId("minutes-readaloud-bar-toggle"),
    ).toHaveAttribute("aria-label", "Weiter");
    // Weiter, dann bis zum letzten Satz gelesen: die Leiste verschwindet.
    await page.getByTestId("minutes-readaloud-bar-toggle").click();
    await expect
      .poll(async () => (await calls(page, "tts_speak_resume")).length)
      .toBe(1);
    await page.evaluate(() => {
      const w = window as any;
      w.__emit("tts-speak-progress", { position: 6, total: 6 });
      w.__speakEnd();
    });
    await expect(bar).toHaveCount(0);
  });

  test("Wiedergabeleiste: Stimme wechseln wirkt live (Einstellung schreiben, dann tts_voice_changed)", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      (window as any).__voices = ["anna"];
    });
    await withMinutes(page);
    await openMinutes(page);
    await page.getByTestId("minutes-readaloud").click();
    const voice = page.getByTestId("minutes-readaloud-bar-voice");
    await expect(voice).toBeVisible();
    // Dezent: kein Dropdown-Feld, nur ein kleiner Pfeil-Knopf und daneben klein der Name.
    await expect(voice.getByRole("combobox")).toHaveCount(0);
    const menuButton = page.getByTestId("minutes-readaloud-bar-voice-menu");
    const size = await menuButton.boundingBox();
    expect(size!.width).toBeLessThanOrEqual(32);
    expect(size!.height).toBeLessThanOrEqual(32);
    const name = page.getByTestId("minutes-readaloud-bar-voice-name");
    await expect(name).toHaveText("Skript mit Sprechern");
    expect(
      await name.evaluate((el) => parseFloat(getComputedStyle(el).fontSize)),
    ).toBeLessThanOrEqual(12.5);
    expect((await voice.boundingBox())!.width).toBeLessThan(220);

    await menuButton.click();
    await page.getByRole("menuitem", { name: "anna", exact: true }).click();
    await expect
      .poll(async () => (await calls(page, "tts_voice_changed")).length)
      .toBe(1);
    const all = await page.evaluate(() =>
      ((window as any).__calls as Call[]).map((c) => c.cmd),
    );
    // Erst die Einstellung, dann das Backend: sonst hoert man die alte Stimme weiter.
    expect(all.indexOf("change_tts_voice_setting")).toBeGreaterThan(-1);
    expect(all.indexOf("change_tts_voice_setting")).toBeLessThan(
      all.indexOf("tts_voice_changed"),
    );
    expect((await calls(page, "change_tts_voice_setting"))[0].args).toEqual({
      value: "anna",
    });
    // Die laufende Sitzung wurde dabei nicht neu gestartet.
    expect((await calls(page, "tts_speak_text")).length).toBe(1);
  });

  test("Stimmwechsel beim Vorlesen: dieselbe Wiedergabe läuft weiter, Pause bleibt, kein Neustart", async ({
    page,
  }) => {
    // Patrick 27.10.2026: der Stimmwechsel startet im Backend KEINE neue Sitzung mehr -- der
    // laufende Satz wird zu Ende gelesen, ab dem naechsten liest die neue Stimme, und der
    // urspruengliche Sprechbefehl laeuft einfach weiter (er kehrt nicht zurueck).
    await page.addInitScript(() => {
      (window as any).__voices = ["anna"];
    });
    await withMinutes(page);
    await openMinutes(page);
    await page.getByTestId("minutes-readaloud").click();
    const toggle = page.getByTestId("minutes-readaloud-bar-toggle");
    await expect(toggle).toHaveAttribute("aria-label", "Pause");
    await page.evaluate(() =>
      (window as any).__emit("tts-state-changed", {
        phase: "speaking",
        owns_server: false,
        message: null,
        server_up: true,
      }),
    );
    await page.getByTestId("minutes-readaloud-bar-voice-menu").click();
    await page.getByRole("menuitem", { name: "anna", exact: true }).click();
    await expect
      .poll(async () => (await calls(page, "tts_voice_changed")).length)
      .toBe(1);
    // Das Backend liest weiter, mitten im Text; der Befehl ist nicht zurueckgekehrt.
    await page.evaluate(() =>
      (window as any).__emit("tts-speak-progress", { position: 2, total: 6 }),
    );
    await expect(toggle).toHaveAttribute("aria-label", "Pause");
    // Auch ein kurzer Phasenwechsel (z. B. Serverpruefung) aendert nichts daran.
    await page.evaluate(() => {
      const w = window as any;
      const status = { owns_server: false, message: null, server_up: true };
      w.__emit("tts-state-changed", { ...status, phase: "ready" });
      w.__emit("tts-state-changed", { ...status, phase: "speaking" });
    });
    await page.waitForTimeout(800);
    await expect(toggle).toHaveAttribute("aria-label", "Pause");
    // Keine zweite Sitzung, kein Fortsetzen: nur der eine Start.
    expect((await calls(page, "tts_speak_text")).length).toBe(1);
    expect((await calls(page, "tts_speak_resume")).length).toBe(0);
    // Das Backend liest bis zum Ende: der Befehl kehrt zurueck, die Leiste verschwindet.
    await page.evaluate(() => {
      const w = window as any;
      w.__emit("tts-speak-progress", { position: 6, total: 6 });
      w.__speakEnd();
    });
    await expect(page.getByTestId("minutes-readaloud-bar")).toHaveCount(0);
  });

  test("Stimmwechsel, dann Pause: Weiter erscheint sofort", async ({ page }) => {
    await page.addInitScript(() => {
      (window as any).__voices = ["anna"];
    });
    await withMinutes(page);
    await openMinutes(page);
    await page.getByTestId("minutes-readaloud").click();
    await page.getByTestId("minutes-readaloud-bar-voice-menu").click();
    await page.getByRole("menuitem", { name: "anna", exact: true }).click();
    await page.evaluate(() =>
      (window as any).__emit("tts-speak-progress", { position: 2, total: 6 }),
    );
    const toggle = page.getByTestId("minutes-readaloud-bar-toggle");
    await expect(toggle).toHaveAttribute("aria-label", "Pause");
    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-label", "Weiter");
    expect((await calls(page, "tts_cancel")).length).toBeGreaterThan(0);
  });

  test("Satzzähler: steht sofort beim Start, folgt Sprung und Satzwechsel, die Breite bleibt gleich", async ({
    page,
  }) => {
    // Patrick 27.10.2026: "1/m" soll beim Klick auf Vorlesen sofort stehen, nicht erst ab Satz 2,
    // und die Schaltflaeche daneben darf nicht springen. Das Backend meldet zu Beginn jedes Laufs
    // (Start, Weiter, Satzsprung) die Startposition.
    await withMinutes(page);
    await openMinutes(page);
    await page.getByTestId("minutes-readaloud").click();
    const position = page.getByTestId("minutes-readaloud-bar-position");
    // Noch keine Meldung: die Gesamtzahl ist unbekannt, also kein Zaehler.
    await expect(position).toHaveCount(0);
    const emit = (name: string, payload: unknown) =>
      page.evaluate(([n, p]) => (window as any).__emit(n, p), [
        name,
        payload,
      ] as const);

    // Beginn des Laufs: Startposition 0 von 12 -- der Zaehler steht sofort auf 1/12.
    await emit("tts-speak-progress", { position: 0, total: 12 });
    await expect(position).toContainText("1/12");
    const widths: number[] = [];
    const measure = async () =>
      widths.push((await position.boundingBox())!.width);
    await measure();

    // Der erste Satz beginnt zu spielen: weiter 1/12.
    await emit("tts-current-sentence", {
      context: "speak",
      index: 0,
      text: "Erster Satz.",
    });
    await expect(position).toContainText("1/12");
    await expect(position).toHaveAttribute("title", "Erster Satz.");

    // Satz 1 ist durch: sofort 2/12, auch bevor der naechste Satz meldet; der alte Satz
    // steht nicht mehr im Tooltip.
    await emit("tts-speak-progress", { position: 1, total: 12 });
    await expect(position).toContainText("2/12");
    await expect(position).not.toHaveAttribute("title", "Erster Satz.");
    await measure();
    await emit("tts-current-sentence", {
      context: "speak",
      index: 1,
      text: "Zweiter Satz.",
    });
    await expect(position).toContainText("2/12");

    // Satzsprung vorwaerts: das Backend meldet die neue Startposition.
    await emit("tts-speak-progress", { position: 8, total: 12 });
    await expect(position).toContainText("9/12");
    await measure();
    // Weiter ueber die Stellenzahl hinaus: 10/12, 12/12.
    await emit("tts-speak-progress", { position: 9, total: 12 });
    await expect(position).toContainText("10/12");
    await measure();
    await emit("tts-speak-progress", { position: 11, total: 12 });
    await expect(position).toContainText("12/12");
    await measure();
    // Satzsprung zurueck.
    await emit("tts-speak-progress", { position: 2, total: 12 });
    await expect(position).toContainText("3/12");
    await measure();

    // Feste Breite: kein Pixel Unterschied zwischen "1/12" und "12/12".
    expect(Math.max(...widths) - Math.min(...widths)).toBeLessThan(0.5);
    // Tabellarische Ziffern.
    expect(
      await position.evaluate((el) => getComputedStyle(el).fontVariantNumeric),
    ).toContain("tabular-nums");
  });

  test("schmales Fenster: die Zeile bricht sauber um, nichts wird abgeschnitten", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      (window as any).__voices = ["anna"];
    });
    await withMinutes(page);
    await openMinutes(page);
    await page.getByTestId("minutes-readaloud").click();
    await expect(page.getByTestId("minutes-readaloud-bar")).toBeVisible();
    await page.evaluate(() =>
      (window as any).__emit("tts-speak-progress", { position: 2, total: 6 }),
    );
    for (const width of [900, 640, 480, 380]) {
      await page.setViewportSize({ width, height: 800 });
      // Das Layout stellt die Spalten nach dem Größenwechsel erst um.
      await expect
        .poll(() =>
          page.getByTestId("minutes-toolbar").evaluate((el) => el.clientWidth),
        )
        .toBeGreaterThan(100);
      const fits = await page.evaluate(() => {
        const toolbar = document.querySelector(
          '[data-testid="minutes-toolbar"]',
        ) as HTMLElement;
        const limit = toolbar.getBoundingClientRect().right + 1;
        return {
          page: document.documentElement.scrollWidth <= innerWidth,
          toolbar: toolbar.scrollWidth <= toolbar.clientWidth + 1,
          inside: Array.from(toolbar.querySelectorAll("button")).every(
            (el) => el.getBoundingClientRect().right <= limit,
          ),
        };
      });
      expect(fits, `Breite ${width}`).toEqual({
        page: true,
        toolbar: true,
        inside: true,
      });
    }
  });

  test("Audio speichern: Dateidialog mit Aufnahmetitel, tts_speak_to_file, Fortschritt, Abbrechen", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      (window as any).__saveTarget = "C:\\Audio\\protokoll.wav";
    });
    await withMinutes(page);
    await openMinutes(page);
    const save = page.getByTestId("minutes-readaloud-save");
    await expect(save).toHaveAttribute("aria-label", "Audio speichern");
    await save.click();

    await expect
      .poll(async () => (await calls(page, "tts_speak_to_file")).length)
      .toBe(1);
    // Dasselbe Format wie auf der Vorlesen-Seite: WAV vorgeschlagen, Titel + Zeitstempel.
    const dialog = (await calls(page, "plugin:dialog|save"))[0].args as any;
    expect(dialog.options.defaultPath).toMatch(
      /^Kundentermin-Meyer_\d{4}-\d{2}-\d{2}_\d{4}\.wav$/,
    );
    expect(dialog.options.filters[0].extensions).toEqual(["wav"]);
    expect((await calls(page, "tts_speak_to_file"))[0].args).toEqual({
      text: SPOKEN,
      outPath: "C:\\Audio\\protokoll.wav",
    });
    // Lesen ist dafuer nicht noetig: keine Wiedergabe gestartet.
    expect((await calls(page, "tts_speak_text")).length).toBe(0);

    // Fortschritt im Knopf, Klick bricht ab.
    await page.evaluate(() =>
      (window as any).__emit("tts-export-progress", {
        position: 2,
        total: 6,
        cancelled: false,
      }),
    );
    await expect(save).toHaveAttribute(
      "aria-label",
      "Speichern abbrechen (2 von 6 Sätzen)",
    );
    await save.click();
    await expect
      .poll(async () => (await calls(page, "tts_export_cancel")).length)
      .toBe(1);
    await page.evaluate(() =>
      (window as any).__emit("tts-export-progress", {
        position: 2,
        total: 6,
        cancelled: true,
      }),
    );
    await expect(save).toHaveAttribute("aria-label", "Audio speichern");
    await expect(page.locator("[data-sonner-toast]")).toHaveCount(0);

    // Zweiter Lauf bis zum Ende: Meldung mit dem Pfad.
    await save.click();
    await expect
      .poll(async () => (await calls(page, "tts_speak_to_file")).length)
      .toBe(2);
    await page.evaluate(() =>
      (window as any).__emit("tts-export-progress", {
        position: 6,
        total: 6,
        cancelled: false,
      }),
    );
    await expect(page.locator("[data-sonner-toast]")).toContainText(
      "C:\\Audio\\protokoll.wav",
    );
    await expect(save).toHaveAttribute("aria-label", "Audio speichern");
  });

  test("Audio speichern: ohne Auswahl im Dialog passiert nichts", async ({
    page,
  }) => {
    await withMinutes(page);
    await openMinutes(page);
    await page.getByTestId("minutes-readaloud-save").click();
    await expect
      .poll(async () => (await calls(page, "plugin:dialog|save")).length)
      .toBe(1);
    expect((await calls(page, "tts_speak_to_file")).length).toBe(0);
  });

  test("Screenshot: Einzeiler aus, laufend, pausiert, schmal", async ({
    page,
  }) => {
    const dir = process.env.SCREENS_DIR;
    test.skip(!dir, "SCREENS_DIR nicht gesetzt");
    await page.addInitScript(() => {
      (window as any).__voices = ["anna"];
    });
    await withMinutes(page);
    await openMinutes(page);
    await expect(page.getByTestId("minutes-readaloud")).toBeVisible();
    await page.screenshot({ path: `${dir}/einzeiler-aus.png` });
    await page.getByTestId("minutes-readaloud").click();
    await expect(page.getByTestId("minutes-readaloud-bar")).toBeVisible();
    await page.evaluate(() => {
      const w = window as any;
      w.__emit("tts-current-sentence", {
        context: "speak",
        index: 2,
        text: "Angebot bis Freitag senden.",
      });
      w.__emit("tts-speak-progress", { position: 2, total: 6 });
    });
    await expect(
      page.getByTestId("minutes-readaloud-bar-position"),
    ).toContainText("3/6");
    await page.screenshot({ path: `${dir}/einzeiler-laufend.png` });
    await page.getByTestId("minutes-readaloud-bar-toggle").click();
    await expect(
      page.getByTestId("minutes-readaloud-bar-toggle"),
    ).toHaveAttribute("aria-label", "Weiter");
    await page.screenshot({ path: `${dir}/einzeiler-pausiert.png` });
    await page.getByTestId("minutes-readaloud-bar-voice-menu").click();
    await expect(page.getByRole("menu")).toBeVisible();
    await page.screenshot({ path: `${dir}/einzeiler-stimmenmenue.png` });
    await page.keyboard.press("Escape");
    await page.setViewportSize({ width: 480, height: 800 });
    await page.screenshot({ path: `${dir}/einzeiler-schmal.png` });
  });
});

// ---------------------------------------------------------------------------
// Status: "Fertig" bleibt weg
// ---------------------------------------------------------------------------

const meeting = (id: string, title: string, status: string) => ({
  id,
  title,
  status,
  source: "recording",
  started_at: 1790100000,
  ended_at: 1790100600,
  language: "de",
  mic_audio_path: null,
  system_audio_path: null,
  duration_ms: 600000,
  consent_confirmed_at: 1790100000,
  audio_retention_until: null,
  created_at: 1790100000,
  source_path: null,
});

test.describe("Status nur bei Laufendem und Fehlern", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(
      (extra) => {
        (window as any).__extraMeetings = extra;
      },
      [
        meeting("m-fail", "Fehlerhafte Aufnahme", "failed"),
        meeting("m-proc", "Laufende Aufnahme", "processing"),
        meeting("m-wait", "Wartende Aufnahme", "queued"),
      ],
    );
    await openRecordings(page);
  });

  const row = (page: Page, id: string) =>
    page.locator(`[data-meeting-id="${id}"]`);

  test("Liste: fertig ohne Hinweis, Fehler deutlich rot, laufend und wartend sichtbar", async ({
    page,
  }) => {
    await expect(row(page, "m1")).toBeVisible();
    await expect(row(page, "m1")).not.toContainText("Fertig");
    await expect(row(page, "m1").getByTestId("row-status")).toHaveCount(0);
    await expect(page.getByText("Fertig", { exact: true })).toHaveCount(0);

    const failed = row(page, "m-fail").getByTestId("row-status");
    await expect(failed).toHaveText("Fehlgeschlagen");
    await expect(failed).toHaveClass(/text-red-500/);
    await expect(row(page, "m-proc").getByTestId("row-status")).toHaveText(
      "Wird verarbeitet",
    );
    await expect(row(page, "m-wait").getByTestId("row-status")).toHaveText(
      "Wartet",
    );
  });

  test("Kopf der Detailansicht: fertig ohne Chip, Fehler und Verarbeitung mit Chip", async ({
    page,
  }) => {
    await page.getByText("Kundentermin Meyer", { exact: true }).click();
    await expect(page.getByTestId("rec-detail-head")).toBeVisible();
    await expect(page.getByTestId("status-chip")).toHaveCount(0);
    await expect(page.getByTestId("rec-detail-head")).not.toContainText(
      "Fertig",
    );
    // Die anderen Chips bleiben.
    await expect(page.getByTestId("source-chip")).toBeVisible();
    await expect(page.getByTestId("duration-chip")).toBeVisible();

    await page.getByText("Fehlerhafte Aufnahme", { exact: true }).click();
    const chip = page.getByTestId("status-chip");
    await expect(chip).toHaveText("Fehlgeschlagen");
    await expect(chip).toHaveClass(/text-red-500/);

    await page.getByText("Laufende Aufnahme", { exact: true }).click();
    await expect(page.getByTestId("status-chip")).toHaveText(
      "Wird verarbeitet",
    );
  });
});
