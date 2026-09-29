import path from "node:path";
import { test, expect, type Page } from "@playwright/test";
import {
  blockFromMarkdownPrefix,
  formatAt,
  makeBlock,
  mergeConflict,
  mergeWithPrevious,
  newBlockId,
  sectionIdFromTitle,
  splitBlock,
  templateErrorText,
  validateSpecLocally,
} from "../src/lib/meetingNotes";

// M1-P1c: Notizblock (Aufnahmeseite + Detailansicht) und Vorlagenverwaltung
// gegen die Tauri-Attrappe. Die Attrappe merkt sich jeden Aufruf in
// `window.__calls`; Ereignisse (`meeting-event`) loest `window.__emit` aus.

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
    const spec = (n: number) => ({
      version: 1,
      context: "Zweck",
      sections: Array.from({ length: n }, (_, i) => ({
        id: `abschnitt_${i}`,
        title: `Abschnitt ${i + 1}`,
        instruction: "",
        kind: i === n - 1 ? "tasks" : "text",
      })),
    });
    w.__settings = settings;
    w.__calls = [];
    w.__recording = false;
    w.__position = null;
    w.__importError = null;
    w.__notes = { blocks: [], revision: 0 };
    w.__conflictOnce = null;
    w.__segments = [];
    w.__documents = [];
    w.__epoch = 1;
    w.__items = [];
    w.__staleOnce = null;
    w.__enhanceError = null;
    w.__freshBody = null;
    w.__templates = [
      {
        id: "builtin:allgemein",
        title: "Allgemein",
        builtin: true,
        spec: spec(5),
        updated_at: 1,
      },
      {
        id: "builtin:vertrieb",
        title: "Kundengespräch / Vertrieb",
        builtin: true,
        spec: spec(5),
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
              // Ohne das blieben die Hoerer der StrictMode-Doppelmontage stehen.
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
            case "meetings_stop":
              return "m-neu";
            case "meetings_is_recording":
              return w.__recording;
            case "meetings_recording_position":
              return w.__position;
            case "meetings_list":
              return w.__meetings;
            case "meetings_get_segments":
              return w.__segments;
            case "meetings_get_documents":
              return w.__documents;
            // M1-P1d: KI-Notizen
            case "meetings_segment_epoch":
              return w.__epoch;
            case "action_items_list":
              return w.__items;
            case "action_items_set_status":
              w.__items = w.__items.map((i: any) =>
                i.id === args.id
                  ? { ...i, status: args.done ? "done" : "todo" }
                  : i,
              );
              return null;
            case "meeting_notes_update_enhanced": {
              const doc = w.__documents.find(
                (d: any) => d.id === args.documentId,
              );
              if (w.__staleOnce) {
                doc.body = w.__staleOnce;
                doc.updated_at += 5;
                w.__staleOnce = null;
                throw "stale_document";
              }
              doc.body = JSON.stringify(args.notes);
              doc.updated_at += 1;
              return doc.updated_at;
            }
            case "meeting_notes_enhance":
            case "meeting_notes_apply_instruction": {
              if (w.__enhanceError) throw w.__enhanceError;
              const base =
                w.__documents.find((d: any) => d.id === args.documentId) ??
                w.__documents[0];
              const body = base
                ? JSON.parse(base.body)
                : JSON.parse(w.__freshBody);
              if (cmd === "meeting_notes_apply_instruction")
                body.sections[0].entries[1].text = "Kunde prüft Wettbewerber";
              const version = w.__documents.length + 1;
              const doc = {
                id: `d${version}`,
                meeting_id: "m1",
                kind: "enhanced_notes",
                body_format: "enhanced@1",
                body: JSON.stringify(body),
                version,
                created_at: 1790000900 + version,
                template_id: null,
                updated_at: 200 + version,
              };
              w.__documents = [...w.__documents, doc];
              return doc;
            }
            case "meeting_notes_markdown":
              return "# KI-Notizen\n";
            case "meetings_export_document":
              return null;
            case "meetings_minutes_file":
              return null;
            case "meetings_get_template":
              return null;
            case "meetings_set_template":
              return null;
            case "meeting_notes_get":
              return {
                meeting_id: args.meetingId,
                blocks: w.__notes.blocks,
                revision: w.__notes.revision,
                updated_at: 0,
              };
            case "meeting_notes_save": {
              if (w.__conflictOnce) {
                const remote = w.__conflictOnce;
                w.__conflictOnce = null;
                w.__notes = remote;
                throw "revision_conflict";
              }
              w.__notes = {
                blocks: args.blocks,
                revision: (args.baseRevision as number) + 1,
              };
              return w.__notes.revision;
            }
            case "meeting_templates_list":
              return w.__templates;
            case "meeting_templates_duplicate": {
              const src = w.__templates.find((t: any) => t.id === args.id);
              const copy = {
                ...JSON.parse(JSON.stringify(src)),
                id: `user-${w.__templates.length}`,
                title: `${src.title} (Kopie)`,
                builtin: false,
              };
              w.__templates = [...w.__templates, copy];
              return copy;
            }
            case "meeting_templates_save": {
              const info = {
                id: args.id ?? `user-${w.__templates.length}`,
                title: args.title,
                builtin: false,
                spec: args.spec,
                updated_at: 2,
              };
              w.__templates = w.__templates.some((t: any) => t.id === info.id)
                ? w.__templates.map((t: any) => (t.id === info.id ? info : t))
                : [...w.__templates, info];
              return info;
            }
            case "meeting_templates_delete":
              w.__templates = w.__templates.filter(
                (t: any) => t.id !== args.id,
              );
              return null;
            case "meeting_templates_export":
              return null;
            case "meeting_templates_import": {
              if (w.__importError) throw w.__importError;
              const info = {
                id: `user-${w.__templates.length}`,
                title: "Importierte Vorlage",
                builtin: false,
                spec: spec(2),
                updated_at: 3,
              };
              w.__templates = [...w.__templates, info];
              return info;
            }
            case "plugin:dialog|save":
              return "C:\\Temp\\vorlage.lvtemplate.json";
            case "plugin:dialog|open":
              return "C:\\Temp\\import.lvtemplate.json";
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

/** Aufnahme laeuft: die Attrappe meldet Besprechung m1 mit Audioposition. */
const startRecording = async (page: Page, positionMs = 195_000) => {
  await page.addInitScript((ms) => {
    const w = window as any;
    w.__recording = true;
    w.__position = { meeting_id: "m1", position_ms: ms };
  }, positionMs);
};

// ---------------------------------------------------------------------------
// Reine Logik (ohne Browser)
// ---------------------------------------------------------------------------

test.describe("Notizblock Logik", () => {
  test("Zeitstempel als mm:ss", () => {
    expect(formatAt(0)).toBe("00:00");
    expect(formatAt(195_000)).toBe("03:15");
    expect(formatAt(3_599_999)).toBe("59:59");
    expect(formatAt(6_000_000)).toBe("100:00");
    expect(formatAt(-5)).toBe("00:00");
  });

  test("Markdown-Kuerzel", () => {
    expect(blockFromMarkdownPrefix("# Titel")).toEqual({
      kind: "heading",
      text: "Titel",
      checked: false,
    });
    expect(blockFromMarkdownPrefix("- Punkt")?.kind).toBe("bullet");
    expect(blockFromMarkdownPrefix("[ ] Aufgabe")).toEqual({
      kind: "todo",
      text: "Aufgabe",
      checked: false,
    });
    expect(blockFromMarkdownPrefix("[x] Erledigt")?.checked).toBe(true);
    expect(blockFromMarkdownPrefix("#kein Kuerzel")).toBeNull();
    expect(blockFromMarkdownPrefix("Text - mitten")).toBeNull();
  });

  test("Enter teilt den Block, Backspace verschmilzt, leerer Listenpunkt endet die Liste", () => {
    const a = makeBlock({
      id: "A",
      kind: "bullet",
      text: "Budget besprochen",
      at_ms: 5000,
    });
    const fresh = makeBlock({ id: "B", at_ms: null });
    const split = splitBlock([a], 0, 6, fresh);
    expect(split.blocks.map((b) => [b.id, b.kind, b.text])).toEqual([
      ["A", "bullet", "Budget"],
      ["B", "bullet", " besprochen"],
    ]);
    expect(split.focusId).toBe("B");

    const merged = mergeWithPrevious(split.blocks, 1);
    expect(merged?.blocks).toHaveLength(1);
    expect(merged?.blocks[0].text).toBe("Budget besprochen");
    expect(merged?.blocks[0].at_ms).toBe(5000);
    expect(merged?.caret).toBe(6);
    expect(mergeWithPrevious(split.blocks, 0)).toBeNull();

    const empty = makeBlock({ id: "C", kind: "todo", text: "" });
    const out = splitBlock([empty], 0, 0, makeBlock({ id: "D" }));
    expect(out.blocks).toHaveLength(1);
    expect(out.blocks[0].kind).toBe("paragraph");
  });

  test("Konflikt haengt lokale Bloecke als Kopie an, verwirft nichts", () => {
    const remote = [makeBlock({ id: "1", text: "fremd" })];
    const local = [
      makeBlock({ id: "1", text: "lokal geaendert" }),
      makeBlock({ id: "2", text: "nur lokal" }),
      makeBlock({ id: "3", text: "   " }),
    ];
    const merged = mergeConflict(remote, local, () => "NEU");
    expect(merged.map((b) => [b.id, b.text])).toEqual([
      ["1", "fremd"],
      ["NEU", "lokal geaendert"],
      ["2", "nur lokal"],
    ]);
  });

  test("ULID: 26 Zeichen und nach Zeit sortierbar", () => {
    const early = newBlockId(1_000);
    const late = newBlockId(2_000_000_000_000);
    expect(early).toHaveLength(26);
    expect(early < late).toBe(true);
  });

  test("Fehlercodes werden zu Schluesseln, Abschnitts-IDs aus Titeln", () => {
    expect(templateErrorText("template_import:format").key).toBe(
      "meetings.templates.errors.import.format",
    );
    expect(templateErrorText("template_invalid:section_title:x")).toEqual({
      key: "meetings.templates.errors.invalid.section_title",
      params: { section: "x" },
    });
    expect(templateErrorText("template_readonly").key).toBe(
      "meetings.templates.errors.template_readonly",
    );
    expect(sectionIdFromTitle("Nächste Schritte", [])).toBe(
      "naechste_schritte",
    );
    expect(sectionIdFromTitle("Größe!", ["groesse"])).toBe("groesse_2");
    expect(sectionIdFromTitle("???", [])).toBe("abschnitt");
    expect(
      validateSpecLocally("  ", { version: 1, context: "", sections: [] }),
    ).toBe("template_invalid:title");
  });
});

// ---------------------------------------------------------------------------
// Notizblock auf der Aufnahmeseite
// ---------------------------------------------------------------------------

test("Notizblock: Block mit Zeitstempel, Speichern entprellt genau einmal", async ({
  page,
}) => {
  await startRecording(page);
  await openRecordings(page);
  const pad = page.getByTestId("live-notes-pad");
  await expect(pad).toBeVisible();
  await expect(pad.getByTestId("note-starter")).toHaveAttribute(
    "placeholder",
    /Stichpunkte genügen/,
  );

  await pad.getByTestId("note-starter").click();
  await page.keyboard.type("Budget freigegeben bis Freitag", { delay: 15 });

  const block = pad.locator("[data-block-id]").first();
  await expect(block.getByTestId("note-time")).toHaveText("03:15");
  await expect(block.locator("textarea")).toHaveValue(
    "Budget freigegeben bis Freitag",
  );

  // Erst nach der Entprellzeit, und dann nur ein Aufruf fuer die ganze Serie.
  await expect
    .poll(async () => (await calls(page, "meeting_notes_save")).length, {
      timeout: 3000,
    })
    .toBe(1);
  await page.waitForTimeout(1200);
  const saves = await calls(page, "meeting_notes_save");
  expect(saves).toHaveLength(1);
  const args = saves[0].args as any;
  expect(args.meetingId).toBe("m1");
  expect(args.baseRevision).toBe(0);
  expect(args.blocks).toHaveLength(1);
  expect(args.blocks[0]).toMatchObject({
    kind: "paragraph",
    text: "Budget freigegeben bis Freitag",
    at_ms: 195000,
    checked: false,
  });
  await expect(pad.getByTestId("notes-status")).toHaveAttribute(
    "data-status",
    "saved",
  );
});

test("Notizblock: Flush bei Blur schreibt sofort, ohne die Entprellzeit abzuwarten", async ({
  page,
}) => {
  await startRecording(page);
  await openRecordings(page);
  const pad = page.getByTestId("live-notes-pad");
  await pad.getByTestId("note-starter").click();
  await page.keyboard.type("Kurz", { delay: 10 });
  expect(await calls(page, "meeting_notes_save")).toHaveLength(0);
  // Klick auf die Seitenueberschrift nimmt den Fokus aus dem Editor.
  await page.locator(".page-shell__head h1").click();
  await expect
    .poll(async () => (await calls(page, "meeting_notes_save")).length, {
      timeout: 450,
    })
    .toBe(1);
  const saved = (await calls(page, "meeting_notes_save"))[0].args as any;
  expect(saved.blocks[0].text).toBe("Kurz");
});

test("Notizblock: Flush bei verdecktem Fenster (visibilitychange)", async ({
  page,
}) => {
  await startRecording(page);
  await openRecordings(page);
  const pad = page.getByTestId("live-notes-pad");
  await pad.getByTestId("note-starter").click();
  await page.keyboard.type("Vor dem Wechsel", { delay: 10 });
  expect(await calls(page, "meeting_notes_save")).toHaveLength(0);
  await page.evaluate(() => {
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      get: () => "hidden",
    });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await expect
    .poll(async () => (await calls(page, "meeting_notes_save")).length, {
      timeout: 450,
    })
    .toBe(1);
});

test("Notizblock: Tastatur - Enter, Kuerzel, Backspace verschmilzt", async ({
  page,
}) => {
  await startRecording(page, 61_000);
  await openRecordings(page);
  const pad = page.getByTestId("live-notes-pad");
  await pad.getByTestId("note-starter").click();

  await page.keyboard.type("# Kundentermin");
  await page.keyboard.press("Enter");
  await page.keyboard.type("- Preis zu hoch");
  await page.keyboard.press("Enter");
  await page.keyboard.type("Lieferzeit unklar");
  await page.keyboard.press("Enter");
  await page.keyboard.type("[ ] Angebot schicken");

  const blocks = pad.locator("[data-block-id]");
  await expect(blocks).toHaveCount(4);
  await expect(blocks.nth(0)).toHaveAttribute("data-kind", "heading");
  await expect(blocks.nth(1)).toHaveAttribute("data-kind", "bullet");
  await expect(blocks.nth(2)).toHaveAttribute("data-kind", "bullet");
  await expect(blocks.nth(3)).toHaveAttribute("data-kind", "todo");
  await expect(blocks.nth(3).locator("input[type=checkbox]")).toBeVisible();
  await expect(blocks.nth(0).locator("textarea")).toHaveValue("Kundentermin");
  await expect(blocks.nth(1).getByTestId("note-time")).toHaveText("01:01");

  // Backspace am Anfang des dritten Blocks haengt ihn an den zweiten.
  await blocks.nth(2).locator("textarea").click();
  await page.keyboard.press("Home");
  await page.keyboard.press("Backspace");
  await expect(blocks).toHaveCount(3);
  await expect(blocks.nth(1).locator("textarea")).toHaveValue(
    "Preis zu hochLieferzeit unklar",
  );

  // Ein leerer Listenpunkt beendet die Liste.
  await blocks.nth(2).locator("textarea").click();
  await page.keyboard.press("End");
  await page.keyboard.press("Enter");
  await expect(blocks).toHaveCount(4);
  await page.keyboard.press("Enter");
  await expect(blocks.nth(3)).toHaveAttribute("data-kind", "paragraph");
});

test("Notizblock: zu grosse Notizen werden nicht gespeichert, sondern gemeldet", async ({
  page,
}) => {
  await startRecording(page);
  await openRecordings(page);
  const pad = page.getByTestId("live-notes-pad");
  await pad.getByTestId("note-starter").click();
  await page.keyboard.type("x");
  await page.evaluate(() => {
    const el = document.querySelector<HTMLTextAreaElement>(
      "[data-testid=live-notes-pad] [data-block-id] textarea",
    )!;
    const setter = Object.getOwnPropertyDescriptor(
      HTMLTextAreaElement.prototype,
      "value",
    )!.set!;
    setter.call(el, "ä".repeat(150_000)); // 300 KB in UTF-8
    el.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await expect(pad.getByTestId("notes-status")).toHaveAttribute(
    "data-status",
    "tooLarge",
    { timeout: 4000 },
  );
  await expect(pad.getByText(/größer als 200 KB/)).toBeVisible();
  expect(await calls(page, "meeting_notes_save")).toHaveLength(0);
});

test("Notizblock: Konflikt aus einem zweiten Fenster wird zusammengefuehrt", async ({
  page,
}) => {
  await startRecording(page);
  await openRecordings(page);
  const pad = page.getByTestId("live-notes-pad");
  await page.evaluate(() => {
    (window as any).__conflictOnce = {
      revision: 4,
      blocks: [
        {
          id: "FREMD",
          kind: "paragraph",
          text: "Aus dem anderen Fenster",
          at_ms: 1000,
          checked: false,
        },
      ],
    };
  });
  await pad.getByTestId("note-starter").click();
  await page.keyboard.type("Eigener Gedanke");
  await expect(pad.getByTestId("notes-conflict")).toBeVisible({
    timeout: 4000,
  });
  const values = await pad
    .locator("[data-block-id] textarea")
    .evaluateAll((els) => els.map((e) => (e as HTMLTextAreaElement).value));
  expect(values).toEqual(["Aus dem anderen Fenster", "Eigener Gedanke"]);
  const saves = await calls(page, "meeting_notes_save");
  expect(saves).toHaveLength(2);
  expect((saves[1].args as any).baseRevision).toBe(4);
  expect((saves[1].args as any).blocks).toHaveLength(2);
});

test("Notizblock: Aufnahmeseite zweispaltig mit Transkript (Screenshot)", async ({
  page,
}) => {
  await startRecording(page, 12_000);
  await openRecordings(page);
  const pad = page.getByTestId("live-notes-pad");
  await expect(pad).toBeVisible();
  await page.evaluate(() => {
    const w = window as any;
    w.__emit("meeting-event", {
      kind: "state",
      meeting_id: "m1",
      status: "recording",
      paused: false,
    });
  });
  // Das Transkript leert sich mit dem Start einer Aufnahme; erst danach kommen Segmente.
  await page.waitForTimeout(200);
  await page.evaluate(() => {
    const w = window as any;
    const seg = (i: number, start: number, text: string, channel: number) => ({
      segment_index: i,
      text,
      start_ms: start,
      end_ms: start + 8000,
      channel,
      speaker_index: null,
    });
    w.__emit("meeting-event", {
      kind: "segments",
      meeting_id: "m1",
      appended: [
        seg(0, 4_000, "Guten Morgen, schön, dass es heute klappt.", 1),
        seg(
          1,
          12_000,
          "Wir wollten kurz über das Budget für das nächste Quartal sprechen.",
          1,
        ),
        seg(
          2,
          31_000,
          "Ja, das Budget ist freigegeben, aber nur bis Freitag.",
          0,
        ),
      ],
    });
  });
  await expect(
    page.getByText("Wir wollten kurz über das Budget"),
  ).toBeVisible();

  const type = async (text: string, atMs: number) => {
    await page.evaluate((ms) => {
      (window as any).__position = { meeting_id: "m1", position_ms: ms };
    }, atMs);
    await page.keyboard.type(text, { delay: 5 });
    await page.keyboard.press("Enter");
  };
  await pad.getByTestId("note-starter").click();
  await page.evaluate(() => {
    (window as any).__position = { meeting_id: "m1", position_ms: 4_000 };
  });
  await page.keyboard.type("# Kundentermin Meyer", { delay: 5 });
  await page.keyboard.press("Enter");
  await type("- Budget freigegeben, aber nur bis Freitag", 31_000);
  await type("Lieferzeit noch unklar", 95_000);
  await page.keyboard.type("[ ] Angebot bis Donnerstag schicken", { delay: 5 });
  await expect(pad.locator("[data-block-id]")).toHaveCount(4);

  await expect(pad.getByTestId("notes-status")).toHaveAttribute(
    "data-status",
    "saved",
    { timeout: 4000 },
  );
  const layout = await page.evaluate(() => {
    const padBox = document
      .querySelector("[data-testid=live-notes-pad]")!
      .getBoundingClientRect();
    const transcript = [...document.querySelectorAll("p")]
      .find((p) => p.textContent?.includes("Wir wollten kurz"))!
      .getBoundingClientRect();
    return {
      padLeft: padBox.left,
      padRight: padBox.right,
      trLeft: transcript.left,
    };
  });
  // Ab 1024 px nebeneinander: das Transkript beginnt rechts vom Notizblock.
  expect(layout.trLeft).toBeGreaterThan(layout.padRight - 1);

  const shot = path.resolve(
    process.cwd(),
    "../../koordination/granola-besprechungen/abnahme/p1c-notizblock-aufnahme.png",
  );
  await page.screenshot({ path: shot, animations: "disabled" });

  // Schmal: untereinander.
  await page.setViewportSize({ width: 800, height: 900 });
  const stacked = await page.evaluate(() => {
    const padBox = document
      .querySelector("[data-testid=live-notes-pad]")!
      .getBoundingClientRect();
    const transcript = [...document.querySelectorAll("p")]
      .find((p) => p.textContent?.includes("Wir wollten kurz"))!
      .getBoundingClientRect();
    return transcript.top >= padBox.bottom - 1;
  });
  expect(stacked).toBe(true);
});

// ---------------------------------------------------------------------------
// Detailansicht: Tab Notizen
// ---------------------------------------------------------------------------

const openDetailNotes = async (page: Page) => {
  await openRecordings(page);
  await page.getByText("Kundentermin Meyer", { exact: true }).click();
  await page.getByRole("button", { name: "Notizen", exact: true }).click();
};

test("Notizblock: Detailansicht zeigt dieselben Notizen und speichert Aenderungen", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as any).__notes = {
      revision: 3,
      blocks: [
        {
          id: "N1",
          kind: "heading",
          text: "Kundentermin",
          at_ms: 0,
          checked: false,
        },
        {
          id: "N2",
          kind: "bullet",
          text: "Preis zu hoch",
          at_ms: 195000,
          checked: false,
        },
        {
          id: "N3",
          kind: "todo",
          text: "Angebot schicken",
          at_ms: null,
          checked: false,
        },
      ],
    };
  });
  await openDetailNotes(page);
  const view = page.getByTestId("my-notes");
  await expect(view.locator("[data-block-id]")).toHaveCount(3);
  await expect(
    view.locator("[data-block-id]").nth(1).getByTestId("note-time"),
  ).toHaveText("03:15");
  await expect(
    view.locator("[data-block-id]").nth(2).getByTestId("note-time"),
  ).toHaveText("");
  // Umschalter mit Platzhalter fuer die KI-Ansicht.
  await expect(
    page.getByRole("button", { name: "Meine Notizen" }),
  ).toHaveAttribute("aria-pressed", "true");
  await page.getByRole("button", { name: "KI-Notizen" }).click();
  await expect(page.getByTestId("ai-notes-placeholder")).toBeVisible();
  await page.getByRole("button", { name: "Meine Notizen" }).click();

  const ta = view.locator("[data-block-id]").nth(1).locator("textarea");
  await ta.click();
  await page.keyboard.press("End");
  await page.keyboard.type(" (Konkurrenz 12 % billiger)");
  await page.locator("h3").first().click(); // Blur -> sofort speichern
  await expect
    .poll(async () => (await calls(page, "meeting_notes_save")).length, {
      timeout: 500,
    })
    .toBe(1);
  const saved = (await calls(page, "meeting_notes_save"))[0].args as any;
  expect(saved.baseRevision).toBe(3);
  expect(saved.blocks[1].text).toBe("Preis zu hoch (Konkurrenz 12 % billiger)");
  expect(saved.blocks[1].at_ms).toBe(195000);
});

test("Notizblock: leere Notizen zeigen den Hinweis Stichpunkte genügen", async ({
  page,
}) => {
  await openDetailNotes(page);
  await expect(page.getByTestId("note-starter")).toHaveAttribute(
    "placeholder",
    "Stichpunkte genügen …",
  );
});

// ---------------------------------------------------------------------------
// Vorlagen
// ---------------------------------------------------------------------------

const openManager = async (page: Page) => {
  await openDetailNotes(page);
  await page.getByRole("button", { name: "Vorlagen verwalten …" }).click();
  const dialog = page.getByRole("dialog", { name: "Vorlagen" });
  await expect(dialog).toBeVisible();
  return dialog;
};

test("Vorlagen: Auswahl merkt sich die Vorlage der Besprechung", async ({
  page,
}) => {
  await openDetailNotes(page);
  await page.locator(".app-select__control").first().click();
  await page
    .getByRole("option", { name: "Kundengespräch / Vertrieb", exact: true })
    .click();
  await expect
    .poll(async () => (await calls(page, "meetings_set_template")).length)
    .toBe(1);
  expect(
    ((await calls(page, "meetings_set_template"))[0].args as any).templateId,
  ).toBe("builtin:vertrieb");
});

test("Vorlagen: Mitgelieferte Vorlage duplizieren", async ({ page }) => {
  const dialog = await openManager(page);
  const row = dialog.locator('[data-template-id="builtin:allgemein"]');
  await expect(row).toContainText("Mitgeliefert");
  await expect(row).toContainText("5 Abschnitte");
  // Mitgelieferte Vorlagen lassen sich nicht loeschen.
  await expect(row.getByRole("button", { name: /^Löschen/ })).toHaveCount(0);

  await row.getByRole("button", { name: "Duplizieren: Allgemein" }).click();
  const copy = dialog.locator("[data-template-id]", {
    hasText: "Allgemein (Kopie)",
  });
  await expect(copy).toBeVisible();
  await expect(copy).not.toContainText("Mitgeliefert");
  const dup = await calls(page, "meeting_templates_duplicate");
  expect(dup).toHaveLength(1);
  expect((dup[0].args as any).id).toBe("builtin:allgemein");
});

test("Vorlagen: Bearbeiten eines Builtins legt eine Kopie an und speichert sie", async ({
  page,
}) => {
  const dialog = await openManager(page);
  await dialog
    .locator('[data-template-id="builtin:vertrieb"]')
    .getByRole("button", { name: /^Bearbeiten/ })
    .click();
  const editor = page.getByTestId("template-editor");
  await expect(editor).toBeVisible();
  await expect(editor).toContainText("schreibgeschützt");
  const name = editor.getByLabel("Name", { exact: true });
  await expect(name).toHaveValue("Kundengespräch / Vertrieb (Kopie)");
  await name.fill("Erstgespräch");
  await editor.getByRole("button", { name: "Abschnitt hinzufügen" }).click();
  await editor
    .getByPlaceholder("Titel des Abschnitts")
    .last()
    .fill("Nächste Schritte");
  await editor.getByRole("button", { name: "Speichern", exact: true }).click();

  await expect(
    dialog.locator("[data-template-id]", { hasText: "Erstgespräch" }),
  ).toBeVisible();
  const saves = await calls(page, "meeting_templates_save");
  expect(saves).toHaveLength(1);
  const args = saves[0].args as any;
  expect(args.id).toBe("user-2");
  expect(args.title).toBe("Erstgespräch");
  expect(args.spec.sections).toHaveLength(6);
  expect(args.spec.sections[5]).toMatchObject({
    id: "naechste_schritte",
    title: "Nächste Schritte",
    kind: "text",
  });
});

test("Vorlagen: Import-Fehler werden verstaendlich gemeldet", async ({
  page,
}) => {
  const dialog = await openManager(page);
  await page.evaluate(() => {
    (window as any).__importError = "template_import:format";
  });
  await dialog.getByRole("button", { name: "Importieren …" }).click();
  await expect(dialog.getByText(/unbekanntes Format/)).toBeVisible();
  const imp = await calls(page, "meeting_templates_import");
  expect((imp[0].args as any).path).toBe("C:\\Temp\\import.lvtemplate.json");

  await page.evaluate(() => {
    (window as any).__importError = "template_import:too_large";
  });
  await dialog.getByRole("button", { name: "Importieren …" }).click();
  await expect(dialog.getByText(/zu groß \(höchstens 64 KB\)/)).toBeVisible();

  // Erfolg: Fehlermeldung verschwindet, Vorlage erscheint.
  await page.evaluate(() => {
    (window as any).__importError = null;
  });
  await dialog.getByRole("button", { name: "Importieren …" }).click();
  await expect(
    dialog.locator("[data-template-id]", { hasText: "Importierte Vorlage" }),
  ).toBeVisible();
  await expect(dialog.getByText(/zu groß/)).toHaveCount(0);
});

test("Vorlagen: Export geht mit dem Dialogpfad ans Backend", async ({
  page,
}) => {
  const dialog = await openManager(page);
  await dialog.getByRole("button", { name: "Exportieren: Allgemein" }).click();
  await expect(
    dialog.getByText(/Exportiert nach C:\\Temp\\vorlage\.lvtemplate\.json/),
  ).toBeVisible();
  const exp = await calls(page, "meeting_templates_export");
  expect(exp[0].args).toMatchObject({
    id: "builtin:allgemein",
    path: "C:\\Temp\\vorlage.lvtemplate.json",
  });
});

test("Vorlagen: eigene Vorlage loeschen fragt nach", async ({ page }) => {
  const dialog = await openManager(page);
  await dialog.getByRole("button", { name: "Duplizieren: Allgemein" }).click();
  const copy = dialog.locator("[data-template-id]", {
    hasText: "Allgemein (Kopie)",
  });
  await copy.getByRole("button", { name: /^Löschen/ }).click();
  await expect(copy).toContainText("wirklich löschen");
  expect(await calls(page, "meeting_templates_delete")).toHaveLength(0);
  await copy.getByRole("button", { name: "Löschen", exact: true }).click();
  await expect(copy).toHaveCount(0);
  expect(await calls(page, "meeting_templates_delete")).toHaveLength(1);
});

test("Vorlagen: ungueltige Eingabe wird vor dem Speichern abgefangen", async ({
  page,
}) => {
  const dialog = await openManager(page);
  await dialog.getByRole("button", { name: "Neue Vorlage" }).click();
  const editor = page.getByTestId("template-editor");
  await editor.getByRole("button", { name: "Speichern", exact: true }).click();
  await expect(
    editor.getByText("Der Name muss 1 bis 60 Zeichen lang sein."),
  ).toBeVisible();
  expect(await calls(page, "meeting_templates_save")).toHaveLength(0);
});

// ---------------------------------------------------------------------------
// M1-P1f: Hinweistext, Systemton-Vorgabe, Vorlage vor dem Start, Auto-Lauf
// ---------------------------------------------------------------------------

const LOCAL_NOTICE =
  "Hinweis: Ich transkribiere diese Besprechung lokal auf meinem Rechner mit Local Voice AI, um Notizen zu erstellen. Es werden keine Daten an Dritte übertragen.";
const REMOTE_NOTICE =
  "Hinweis: Ich transkribiere diese Besprechung lokal auf meinem Rechner mit Local Voice AI, um Notizen zu erstellen. Für die Notizen wird der Text an OpenAI übermittelt.";

/** Aktiver Anbieter im Einstellungsstand der Attrappe (`local` oder ein entfernter). */
const useProvider = async (page: Page, kind: "local" | "remote") => {
  await page.addInitScript((k) => {
    const w = window as any;
    w.__settings.post_process_providers =
      k === "local"
        ? [
            {
              id: "local",
              label: "Lokales Modell",
              base_url: "http://127.0.0.1:0/v1",
            },
          ]
        : [
            {
              id: "openai",
              label: "OpenAI",
              base_url: "https://api.openai.com/v1",
            },
          ];
    w.__settings.post_process_provider_id = k === "local" ? "local" : "openai";
  }, kind);
};

const withSettings = async (page: Page, patch: Record<string, unknown>) => {
  await page.addInitScript((p) => {
    Object.assign((window as any).__settings, p);
  }, patch);
};

const grantClipboard = async (page: Page) => {
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"], {
    origin: "http://localhost:1420",
  });
};

const readClipboard = (page: Page) =>
  page.evaluate(() => navigator.clipboard.readText());

test.describe("Hinweis Meeting-Chat", () => {
  test("Hinweis kopieren: lokaler Anbieter, waehrend der Aufnahme", async ({
    page,
  }) => {
    await grantClipboard(page);
    await useProvider(page, "local");
    await startRecording(page);
    await openRecordings(page);
    const notice = page.getByTestId("recording-chat-notice");
    await expect(notice).toBeVisible();
    await expect(page.getByTestId("recording-chat-notice-text")).toHaveText(
      LOCAL_NOTICE,
    );
    await page.getByTestId("recording-chat-notice-copy").click();
    await expect(page.getByTestId("recording-chat-notice-copy")).toHaveText(
      "Kopiert",
    );
    expect(await readClipboard(page)).toBe(LOCAL_NOTICE);
  });

  test("Hinweis kopieren: externer Anbieter nennt den Empfaenger", async ({
    page,
  }) => {
    await grantClipboard(page);
    await useProvider(page, "remote");
    await startRecording(page);
    await openRecordings(page);
    await expect(page.getByTestId("recording-chat-notice-text")).toHaveText(
      REMOTE_NOTICE,
    );
    await expect(
      page.getByTestId("recording-chat-notice-text"),
    ).not.toContainText("keine Daten an Dritte");
    await page.getByTestId("recording-chat-notice-copy").click();
    expect(await readClipboard(page)).toBe(REMOTE_NOTICE);
  });

  test("Hinweis kopieren: auch im Einwilligungsdialog", async ({ page }) => {
    await grantClipboard(page);
    await useProvider(page, "remote");
    await openRecordings(page);
    // Vor dem Start gibt es die Zeile nicht, nur im Dialog.
    await expect(page.getByTestId("recording-chat-notice")).toHaveCount(0);
    await page.getByRole("button", { name: "Aufnahme starten" }).click();
    const dialog = page.getByRole("dialog");
    await expect(dialog.getByTestId("consent-chat-notice-text")).toHaveText(
      REMOTE_NOTICE,
    );
    await dialog.getByTestId("consent-chat-notice-copy").click();
    await expect(dialog.getByTestId("consent-chat-notice-copy")).toHaveText(
      "Kopiert",
    );
    expect(await readClipboard(page)).toBe(REMOTE_NOTICE);
  });

  test("Hinweis: ohne eingerichteten Anbieter gilt die lokale Fassung", async ({
    page,
  }) => {
    await startRecording(page);
    await openRecordings(page);
    await expect(page.getByTestId("recording-chat-notice-text")).toHaveText(
      LOCAL_NOTICE,
    );
  });
});

test.describe("Systemton Vorgabe", () => {
  test("Systemton: ohne gespeicherte Wahl ist das Haekchen an", async ({
    page,
  }) => {
    await openRecordings(page);
    await expect(page.getByTestId("capture-system")).toBeChecked();
    await page.getByRole("button", { name: "Aufnahme starten" }).click();
    await page
      .getByRole("button", { name: "Alle Beteiligten haben zugestimmt" })
      .click();
    await expect
      .poll(async () => (await calls(page, "meetings_start")).length)
      .toBe(1);
    expect(
      ((await calls(page, "meetings_start"))[0].args as any).captureSystem,
    ).toBe(true);
  });

  test("Systemton: die Einstellung ist die Vorgabe, die Wahl wird gemerkt", async ({
    page,
  }) => {
    await withSettings(page, { meeting_capture_system: false });
    await openRecordings(page);
    const box = page.getByTestId("capture-system");
    await expect(box).not.toBeChecked();
    await box.check();
    await expect
      .poll(
        async () =>
          (await calls(page, "change_meeting_capture_system_setting")).length,
      )
      .toBe(1);
    expect(
      (await calls(page, "change_meeting_capture_system_setting"))[0].args,
    ).toEqual({ enabled: true });
    await page.getByRole("button", { name: "Aufnahme starten" }).click();
    await page
      .getByRole("button", { name: "Alle Beteiligten haben zugestimmt" })
      .click();
    await expect
      .poll(async () => (await calls(page, "meetings_start")).length)
      .toBe(1);
    expect(
      ((await calls(page, "meetings_start"))[0].args as any).captureSystem,
    ).toBe(true);
  });

  test("Systemton: die Einstellungen zeigen die drei Vorgaben in der Gruppe Besprechungen", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1280, height: 1000 });
    await page.goto("/");
    await page
      .getByRole("navigation")
      .getByRole("button", { name: "Einstellungen", exact: true })
      .click();
    await expect(
      page.getByText("Systemton standardmäßig aufnehmen"),
    ).toBeVisible();
    await expect(
      page.getByText("KI-Notizen nach der Besprechung automatisch erstellen"),
    ).toBeVisible();
    await expect(
      page.getByText("Standardvorlage", { exact: true }),
    ).toBeVisible();
    // Standard: beide Schalter an.
    const toggles = page.locator("label:has(input.peer)");
    const capture = page
      .getByText("Systemton standardmäßig aufnehmen")
      .locator("xpath=ancestor::div[.//input[@type='checkbox']][1]")
      .locator("input[type=checkbox]");
    await expect(capture).toBeChecked();
    expect(await toggles.count()).toBeGreaterThan(1);
    await capture.evaluate((el) => (el as HTMLInputElement).click());
    await expect
      .poll(
        async () =>
          (await calls(page, "change_meeting_capture_system_setting")).length,
      )
      .toBe(1);
    expect(
      (await calls(page, "change_meeting_capture_system_setting"))[0].args,
    ).toEqual({ enabled: false });
  });
});

test.describe("Vorlage vor dem Start", () => {
  const startNow = async (page: Page) => {
    await page.getByRole("button", { name: "Aufnahme starten" }).click();
    await page
      .getByRole("button", { name: "Alle Beteiligten haben zugestimmt" })
      .click();
  };

  test("Vorlage vor Start: Wahl geht nach dem Start an die Besprechung", async ({
    page,
  }) => {
    await openRecordings(page);
    const picker = page.getByTestId("record-template");
    await picker.locator(".app-select__control").click();
    await page
      .getByRole("option", { name: "Kundengespräch / Vertrieb", exact: true })
      .click();
    // Vor dem Start wird nichts gesetzt: es gibt noch keine Besprechung.
    expect(await calls(page, "meetings_set_template")).toHaveLength(0);
    await startNow(page);
    await expect
      .poll(async () => (await calls(page, "meetings_set_template")).length)
      .toBe(1);
    expect((await calls(page, "meetings_set_template"))[0].args).toEqual({
      meetingId: "m-neu",
      templateId: "builtin:vertrieb",
    });
  });

  test("Vorlage vor Start: ohne Wahl gilt die Standardvorlage aus den Einstellungen", async ({
    page,
  }) => {
    await withSettings(page, {
      meeting_default_template_id: "builtin:vertrieb",
    });
    await openRecordings(page);
    await expect(
      page.getByTestId("record-template").locator(".app-select__single-value"),
    ).toHaveText("Kundengespräch / Vertrieb");
    await startNow(page);
    await expect
      .poll(async () => (await calls(page, "meetings_set_template")).length)
      .toBe(1);
    expect(
      ((await calls(page, "meetings_set_template"))[0].args as any).templateId,
    ).toBe("builtin:vertrieb");
  });

  test("Vorlage vor Start: waehrend der Aufnahme gibt es nur eine Vorlagenwahl", async ({
    page,
  }) => {
    await startRecording(page);
    await openRecordings(page);
    await expect(page.getByTestId("live-notes-pad")).toBeVisible();
    await expect(page.getByTestId("record-template")).toHaveCount(0);
    await expect(
      page.getByTestId("live-notes-pad").locator(".app-select__control"),
    ).toHaveCount(1);
  });
});

test.describe("Auto-Lauf nach dem Stopp", () => {
  test("flushAllNotes laeuft vor meetings_stop", async ({ page }) => {
    await startRecording(page);
    await openRecordings(page);
    const pad = page.getByTestId("live-notes-pad");
    await pad.getByTestId("note-starter").click();
    await page.keyboard.type("Nicht vergessen: Angebot bis Freitag", {
      delay: 10,
    });
    // Sofort beenden, weit vor der Entprellzeit von 700 ms. Per JS-Klick:
    // ein echter Mausklick nimmt dem Feld den Fokus, und der Blur-Flush
    // wuerde das Speichern schon vor `flushAllNotes()` erledigen.
    await page
      .getByRole("button", { name: "Beenden" })
      .evaluate((el) => (el as HTMLButtonElement).click());
    await expect
      .poll(async () => (await calls(page, "meetings_stop")).length)
      .toBe(1);
    const order = await page.evaluate(() =>
      ((window as any).__calls as Call[])
        .map((c) => c.cmd)
        .filter((c) => c === "meeting_notes_save" || c === "meetings_stop"),
    );
    expect(order[0]).toBe("meeting_notes_save");
    expect(order).toContain("meetings_stop");
    expect(order.indexOf("meeting_notes_save")).toBeLessThan(
      order.indexOf("meetings_stop"),
    );
  });

  test("Auto-Lauf: Fortschritt, Ende und der Hinweis ohne Anbieter erscheinen an der Aufnahmekarte", async ({
    page,
  }) => {
    await openRecordings(page);
    const status = page.getByTestId("auto-notes-status");
    await expect(status).toHaveCount(0);
    // Ereignisse anderer Besprechungen bleiben ohne Wirkung.
    await page.evaluate(() => {
      (window as any).__emit("meeting-event", {
        kind: "state",
        meeting_id: "m1",
        status: "processing",
        paused: false,
      });
    });
    await page.evaluate(() => {
      (window as any).__emit("meeting-notes-event", {
        kind: "progress",
        meeting_id: "andere",
        step: 1,
        total: 3,
      });
    });
    await expect(status).toHaveCount(0);
    await page.evaluate(() => {
      (window as any).__emit("meeting-notes-event", {
        kind: "progress",
        meeting_id: "m1",
        step: 2,
        total: 5,
      });
    });
    await expect(status).toContainText("KI-Notizen werden erstellt … 2/5");
    await page.evaluate(() => {
      (window as any).__emit("meeting-notes-event", {
        kind: "failed",
        meeting_id: "m1",
        code: "no_provider",
      });
    });
    await expect(status).toHaveAttribute("data-state", "failed");
    await expect(status).toContainText(
      "kein Sprachmodell-Anbieter eingerichtet",
    );
    await page.evaluate(() => {
      (window as any).__emit("meeting-notes-event", {
        kind: "done",
        meeting_id: "m1",
        document_id: "d1",
      });
    });
    await expect(status).toHaveAttribute("data-state", "done");
  });
});

// ---------------------------------------------------------------------------
// KI-Notizen (M1-P1d): Ansicht, Quellen, Bearbeiten, Anweisung, Checkliste
// ---------------------------------------------------------------------------

test.describe("KI-Notizen", () => {
  const flags = (unsupported = false, edited = false) => ({
    unsupported,
    dropped_sources: 0,
    placed_by_fallback: false,
    edited,
  });
  const entry = (
    id: string,
    origin: "ai" | "user",
    text: string,
    sources: number[],
    extra: Record<string, unknown> = {},
  ) => ({
    id,
    origin,
    text,
    note_id: origin === "user" ? "N2" : null,
    source_segment_ids: sources,
    assignee: null,
    due: null,
    flags: flags(origin === "ai" && sources.length === 0),
    ...extra,
  });
  const notesBody = (secondText = "Kunde vergleicht mit dem Wettbewerber") => ({
    format: "enhanced@1",
    template_id: "builtin:vertrieb",
    template_title: "Kundengespräch / Vertrieb",
    segment_epoch: 1,
    sections: [
      {
        id: "kernpunkte",
        title: "Kernpunkte",
        kind: "text",
        entries: [
          entry("E1", "user", "Preis zu hoch", [12]),
          entry("E2", "ai", secondText, [12, 3, 5, 7]),
          entry("E3", "ai", "Liefertermin ist noch offen", []),
        ],
      },
      {
        id: "aufgaben",
        title: "Aufgaben",
        kind: "tasks",
        entries: [
          entry("E4", "ai", "Angebot bis Freitag schicken", [12], {
            assignee: "Patrick",
            due: "Freitag",
          }),
        ],
      },
    ],
    stats: {
      user_notes_total: 1,
      user_notes_by_model: 1,
      user_notes_by_fallback: 0,
      ai_entries: 3,
      ai_entries_sourced: 2,
      dropped_source_ids: 0,
      chunks_total: 1,
      chunks_failed: [],
      single_pass: true,
    },
  });

  type Setup = {
    audio?: boolean;
    epoch?: number;
    withDocument?: boolean;
  };

  /** Besprechung m1 mit 15 Segmenten (Nr. 12 bei 03:15) und einer KI-Notizen-Version. */
  const setup = async (page: Page, opts: Setup = {}) => {
    const { audio = false, epoch = 1, withDocument = true } = opts;
    await page.addInitScript(
      ({ audio, epoch, withDocument, body }) => {
        const w = window as any;
        w.__played = [];
        // Wiedergabe nur mitschreiben: Position beim play() = Sprungziel.
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
        if (audio) w.__meetings[0].mic_audio_path = "C:/audio/m1-mic.wav";
        w.__epoch = epoch;
        w.__segments = Array.from({ length: 15 }, (_, i) => ({
          segment_index: i,
          text: i === 12 ? "Der Preis ist uns zu hoch." : `Satz Nummer ${i}.`,
          start_ms: i === 12 ? 195_000 : i * 15_000,
          end_ms: i === 12 ? 200_000 : i * 15_000 + 5_000,
          channel: 0,
          speaker_index: null,
        }));
        w.__freshBody = JSON.stringify(body);
        w.__documents = withDocument
          ? [
              {
                id: "d1",
                meeting_id: "m1",
                kind: "enhanced_notes",
                body_format: "enhanced@1",
                body: JSON.stringify(body),
                version: 1,
                created_at: 1790000700,
                template_id: "builtin:vertrieb",
                updated_at: 100,
              },
            ]
          : [];
        w.__items = [
          {
            id: "a1",
            meeting_id: "m1",
            text: "Angebot bis Freitag schicken",
            status: "todo",
            assignee_label: "Patrick",
            document_id: "d1",
            entry_id: "E4",
            source_segment_ids: [12],
            source: "ai",
          },
        ];
      },
      { audio, epoch, withDocument, body: notesBody() },
    );
  };

  const openAiNotes = async (page: Page) => {
    await openDetailNotes(page);
    await page.getByRole("button", { name: "KI-Notizen", exact: true }).click();
    const view = page.getByTestId("enhanced-notes");
    await expect(view).toBeVisible();
    return view;
  };

  const entryOf = (page: Page, id: string) =>
    page.locator(`[data-testid=enhanced-entry][data-entry-id="${id}"]`);

  test("KI-Notizen: Nutzertext normal, KI-Text grau, ohne Beleg, Quellen als mm:ss", async ({
    page,
  }) => {
    await setup(page);
    const view = await openAiNotes(page);
    await expect(
      view.getByRole("heading", { name: "Kernpunkte" }),
    ).toBeVisible();
    await expect(view.getByRole("heading", { name: "Aufgaben" })).toBeVisible();

    const user = entryOf(page, "E1");
    await expect(user).toHaveAttribute("data-origin", "user");
    const userText = user.getByTestId("entry-text");
    await expect(userText).toHaveClass(/(^|\s)text-text(\s|$)/);
    await expect(userText).not.toHaveClass(/text-text\/60/);

    const ai = entryOf(page, "E2");
    await expect(ai).toHaveAttribute("data-origin", "ai");
    await expect(ai.getByTestId("entry-text")).toHaveClass(/text-text\/60/);
    // Hoechstens drei Chips, der Rest hinter "+1".
    await expect(ai.locator("[data-source-id]")).toHaveCount(3);
    await expect(ai.locator('[data-source-id="12"]')).toHaveText("03:15");
    await expect(ai.getByTestId("source-more")).toHaveText("+1");
    await ai.getByTestId("source-more").click();
    await expect(ai.locator("[data-source-id]")).toHaveCount(4);

    const noSource = entryOf(page, "E3");
    await expect(noSource).toHaveAttribute("data-origin", "ai");
    await expect(noSource.getByTestId("no-evidence")).toHaveText("ohne Beleg");
    await expect(noSource.locator("[data-source-id]")).toHaveCount(0);
    await expect(page.getByTestId("stale-hint")).toHaveCount(0);
  });

  test("KI-Notizen: Quell-Klick wechselt ins Transkript, markiert Segment 12 und spielt ab 195 s", async ({
    page,
  }) => {
    await setup(page, { audio: true });
    await openAiNotes(page);
    await expect(page.getByTestId("audio-gone-hint")).toHaveCount(0);
    await entryOf(page, "E1").locator('[data-source-id="12"]').click();

    const row = page.locator('[data-segment-index="12"]');
    await expect(row).toBeVisible();
    await expect(row).toContainText("Der Preis ist uns zu hoch.");
    await expect(row).toHaveAttribute("data-highlighted", "true");
    // Tab Transkript ist aktiv: die KI-Notizen sind ausgeblendet.
    await expect(page.getByTestId("enhanced-notes")).toHaveCount(0);
    await expect
      .poll(() => page.evaluate(() => (window as any).__played))
      .toEqual([195]);
    // Die Markierung verschwindet nach rund 2 s wieder.
    await expect(row).not.toHaveAttribute("data-highlighted", "true", {
      timeout: 4000,
    });
  });

  test("KI-Notizen: ohne Audio springt die Quelle nur ins Transkript", async ({
    page,
  }) => {
    await setup(page, { audio: false });
    await openAiNotes(page);
    await expect(page.getByTestId("audio-gone-hint")).toContainText(
      "Audio ist nicht mehr vorhanden",
    );
    await entryOf(page, "E2").locator('[data-source-id="12"]').click();
    await expect(page.locator('[data-segment-index="12"]')).toHaveAttribute(
      "data-highlighted",
      "true",
    );
    expect(await page.evaluate(() => (window as any).__played)).toEqual([]);
  });

  test("KI-Notizen: Bearbeiten ruft meeting_notes_update_enhanced, Eintrag wird Nutzertext", async ({
    page,
  }) => {
    await setup(page);
    await openAiNotes(page);
    const ai = entryOf(page, "E2");
    await ai.getByTestId("entry-text").click();
    const editor = ai.getByTestId("entry-editor");
    await expect(editor).toBeFocused();
    await page.keyboard.type(" (geprüft)");
    await page.keyboard.press("Enter"); // beendet das Bearbeiten -> sofort speichern
    await expect
      .poll(
        async () => (await calls(page, "meeting_notes_update_enhanced")).length,
      )
      .toBe(1);
    const args = (await calls(page, "meeting_notes_update_enhanced"))[0]
      .args as any;
    expect(args.documentId).toBe("d1");
    expect(args.expectedUpdatedAt).toBe(100);
    const saved = args.notes.sections[0].entries[1];
    expect(saved.text).toBe("Kunde vergleicht mit dem Wettbewerber (geprüft)");
    expect(saved.flags.edited).toBe(true);
    // Nutzertext: nicht mehr grau.
    await expect(ai).toHaveAttribute("data-origin", "user");
    await expect(ai.getByTestId("entry-text")).not.toHaveClass(/text-text\/60/);
    await expect(page.getByTestId("enhanced-status")).toHaveAttribute(
      "data-status",
      "saved",
    );
  });

  test("KI-Notizen: stale_document laedt den Serverstand neu und meldet es", async ({
    page,
  }) => {
    await setup(page);
    await page.addInitScript((body) => {
      (window as any).__staleOnce = JSON.stringify(body);
    }, notesBody("Fremde Änderung aus dem zweiten Fenster"));
    await openAiNotes(page);
    const ai = entryOf(page, "E2");
    await ai.getByTestId("entry-text").click();
    await page.keyboard.type(" lokal");
    await page.keyboard.press("Enter");
    await expect(
      page.getByText("zwischenzeitlich geändert", { exact: false }),
    ).toBeVisible();
    await expect(ai.getByTestId("entry-text")).toHaveText(
      "Fremde Änderung aus dem zweiten Fenster",
    );
    expect(await calls(page, "meeting_notes_update_enhanced")).toHaveLength(1);
    // Neu geladen: Dokumentliste zweimal gelesen (Start + nach dem Konflikt).
    expect(
      (await calls(page, "meetings_get_documents")).length,
    ).toBeGreaterThanOrEqual(2);
  });

  test("KI-Notizen: Checkbox der Aufgabe ruft action_items_set_status", async ({
    page,
  }) => {
    await setup(page);
    await openAiNotes(page);
    const task = page.locator('[data-task-entry="E4"]');
    await expect(task).toContainText("@Patrick · bis Freitag");
    const box = task.getByRole("checkbox", { name: "Aufgabe erledigt" });
    await expect(box).not.toBeChecked();
    await box.check();
    await expect
      .poll(async () => (await calls(page, "action_items_set_status")).length)
      .toBe(1);
    expect((await calls(page, "action_items_set_status"))[0].args).toEqual({
      id: "a1",
      done: true,
    });
    await expect(box).toBeChecked();
    await expect(entryOf(page, "E4").getByTestId("entry-text")).toHaveClass(
      /line-through/,
    );
  });

  test("KI-Notizen: Anweisung erzeugt eine neue Version", async ({ page }) => {
    await setup(page);
    await openAiNotes(page);
    const input = page.getByRole("textbox", { name: "Anweisung an die KI" });
    await input.fill("kürzer");
    await page.getByRole("button", { name: "Anwenden" }).click();
    await expect
      .poll(
        async () =>
          (await calls(page, "meeting_notes_apply_instruction")).length,
      )
      .toBe(1);
    expect(
      (await calls(page, "meeting_notes_apply_instruction"))[0].args,
    ).toEqual({ documentId: "d1", instruction: "kürzer" });
    await expect(entryOf(page, "E2").getByTestId("entry-text")).toHaveText(
      "Kunde prüft Wettbewerber",
    );
    await expect(input).toHaveValue("");
    // Zwei Versionen: die Versionswahl erscheint und steht auf der neuesten.
    await expect(page.getByTestId("version-picker")).toContainText("Version 2");
  });

  test("KI-Notizen: Fortschritt und failed-Code aus MeetingNotesEvent werden uebersetzt", async ({
    page,
  }) => {
    await setup(page, { withDocument: false });
    const view = await openAiNotes(page);
    await expect(page.getByTestId("ai-notes-placeholder")).toBeVisible();
    const emit = (payload: unknown) =>
      page.evaluate(
        (p) => (window as any).__emit("meeting-notes-event", p),
        payload,
      );
    // Ereignis einer anderen Besprechung: ignoriert.
    await emit({ kind: "progress", meeting_id: "m2", step: 1, total: 5 });
    await expect(page.getByTestId("enhance-progress")).toHaveCount(0);
    await emit({ kind: "progress", meeting_id: "m1", step: 2, total: 5 });
    await expect(page.getByTestId("enhance-progress")).toHaveText(
      "KI-Notizen werden erstellt … 2/5",
    );
    await emit({ kind: "failed", meeting_id: "m1", code: "memory_low" });
    await expect(page.getByTestId("enhance-progress")).toHaveCount(0);
    const error = view.getByTestId("enhance-error");
    await expect(error).toHaveAttribute("data-code", "memory_low");
    await expect(error).toContainText("Zu wenig freier Arbeitsspeicher");
    await emit({ kind: "failed", meeting_id: "m1", code: "no_provider" });
    await expect(error).toContainText(
      "Es ist kein Sprachmodell-Anbieter eingerichtet.",
    );
    await expect(error).toContainText("Einstellungen → Sprachmodelle");
  });

  test("KI-Notizen: Erzeugen ruft meeting_notes_enhance, Fehler mit Code wird uebersetzt", async ({
    page,
  }) => {
    await setup(page, { withDocument: false });
    await page.addInitScript(() => {
      (window as any).__enhanceError = "enhance_busy: laeuft schon";
    });
    await openAiNotes(page);
    const generate = page.getByRole("button", { name: "KI-Notizen erzeugen" });
    await generate.click();
    await expect(page.getByTestId("enhance-error")).toContainText(
      "Es läuft bereits ein KI-Notizen-Lauf.",
    );
    await page.evaluate(() => {
      (window as any).__enhanceError = null;
    });
    await generate.click();
    await expect(entryOf(page, "E1")).toBeVisible();
    await expect(page.getByTestId("enhance-error")).toHaveCount(0);
    const enhance = await calls(page, "meeting_notes_enhance");
    expect(enhance).toHaveLength(2);
    expect(enhance[1].args).toEqual({ meetingId: "m1", templateId: null });
    await expect(
      page.getByRole("button", { name: "KI-Notizen neu erzeugen" }),
    ).toBeVisible();
  });

  test("KI-Notizen: veraltete Epoche sperrt die Spruenge und zeigt den Hinweis", async ({
    page,
  }) => {
    await setup(page, { audio: true, epoch: 2 });
    const view = await openAiNotes(page);
    await expect(view).toHaveAttribute("data-stale", "true");
    await expect(page.getByTestId("stale-hint")).toContainText(
      "Quellen veraltet – neu erzeugen",
    );
    const chip = entryOf(page, "E1").locator('[data-source-id="12"]');
    await expect(chip).toBeDisabled();
    await expect(chip).toHaveText("--:--");
    await chip.click({ force: true });
    await expect(page.getByTestId("enhanced-notes")).toBeVisible();
    expect(await page.evaluate(() => (window as any).__played)).toEqual([]);
  });

  test("KI-Notizen: Export schreibt das Markdown ueber meetings_export_document", async ({
    page,
  }) => {
    await setup(page);
    await openAiNotes(page);
    await page.getByRole("button", { name: /^Exportieren – als Word/ }).click();
    await expect(page.getByTestId("exported")).toBeVisible();
    expect((await calls(page, "meeting_notes_markdown"))[0].args).toEqual({
      documentId: "d1",
    });
    expect((await calls(page, "meetings_export_document"))[0].args).toEqual({
      path: "C:\\Temp\\vorlage.lvtemplate.json",
      body: "# KI-Notizen\n",
    });
  });

  test("KI-Notizen: Ansicht (Screenshot)", async ({ page }) => {
    await setup(page, { audio: true });
    await openAiNotes(page);
    await expect(entryOf(page, "E4")).toBeVisible();
    const shot = path.resolve(
      process.cwd(),
      "../../koordination/granola-besprechungen/abnahme/p1d-ki-notizen.png",
    );
    await page.screenshot({
      path: shot,
      animations: "disabled",
      fullPage: true,
    });
  });
});
