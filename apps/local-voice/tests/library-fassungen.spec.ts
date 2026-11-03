import { test, expect, type Page } from "@playwright/test";
import { join } from "node:path";
import { writeFileSync } from "node:fs";

// lva#236: Übersetzung, Kurzbuch und KI-Dokument aus der Bibliothek starten
// (ohne Wechsel ins Vorlesen) und jede Fassung einzeln exportieren. Grundlage
// ist der Aufbau von library.spec.ts (dieselben Bücher und Befehle).
const shot = (name: string) =>
  join(process.env.LV_SHOT_DIR ?? "test-results", name);

const BOOKS = [
  {
    id: "book_1",
    title: "Effi Briest",
    subtitle: null,
    authors: ["Theodor Fontane"],
    language: "Deutsch",
    isbn: "9783150069616",
    publisher: "Reclam",
    publish_date: "1895",
    pages: 336,
    description: "Die siebzehnjährige Effi heiratet den Baron von Innstetten.",
    metadata_source: "openlibrary",
    cover_source: "generated",
    text: { page_id: "page_1", origin: "document", source_name: "effi.epub" },
    audio: null,
    added_ms: 2,
    finished: false,
    cover_path: null,
    audio_path: null,
  },
  {
    id: "book_2",
    title: "Der Schimmelreiter",
    subtitle: null,
    authors: ["Theodor Storm"],
    language: null,
    isbn: null,
    publisher: null,
    publish_date: null,
    pages: null,
    description: null,
    metadata_source: "manual",
    cover_source: "generated",
    text: null,
    audio: { file: "hoerbuch.m4b", duration_ms: null, narrator: null, transcript_job: "m1" },
    added_ms: 1,
    finished: false,
    cover_path: null,
    audio_path: "C:/lib/book_2/audio/hoerbuch.m4b",
  },
  {
    id: "book_3",
    title: "Die Verwandlung",
    subtitle: null,
    authors: ["Franz Kafka"],
    language: "Deutsch",
    isbn: null,
    publisher: null,
    publish_date: "1915",
    pages: 96,
    description: null,
    metadata_source: "manual",
    cover_source: "generated",
    // Textfassung, im Vorlesen vertont (Audiodateien im Seitenordner).
    text: { page_id: "page_3", origin: "document", source_name: "kafka.epub" },
    audio: null,
    added_ms: 3,
    finished: false,
    cover_path: null,
    audio_path: null,
  },
];

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript((books) => {
    const callbacks = new Map<number, unknown>();
    let callback = 0;
    const settings = {
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
      tts_engine: "piper",
    };
    const w = window as unknown as Record<string, unknown>;
    w.__calls = [] as unknown[];
    const listeners: Record<string, number[]> = {};
    w.__emit = (event: string, payload: unknown) =>
      (listeners[event] ?? []).forEach((h) =>
        (callbacks.get(h) as ((e: unknown) => void) | undefined)?.({
          event,
          id: 0,
          payload,
        }),
      );
    w.__books = books;
    w.__settings = settings;
    // Hilfs-Besprechungen der Transkription (lva#204): Kennung → Stand.
    w.__jobs = [] as string[];
    w.__jobStatus = {} as Record<string, string>;
    w.__jobDuration = {} as Record<string, number>;
    w.__segments = {} as Record<string, unknown[]>;
    w.__pageText = {} as Record<string, string>;
    let jobCounter = 1;
    // Fortschritt des Readers: Effi zu 40 % gehört.
    window.localStorage.setItem(
      "lva.ui.progress.text:page_1:original",
      JSON.stringify({
        offset: 400,
        length: 1000,
        heard: [[0, 400]],
        read: [],
        updated: Date.now(),
      }),
    );
    window.localStorage.setItem("lva.ui.section", "library");
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
        invoke: async (cmd: string, args?: Record<string, unknown>) => {
          (w.__calls as unknown[]).push([cmd, args]);
          const list = w.__books as Record<string, unknown>[];
          if (cmd === "get_app_settings" || cmd === "get_default_settings")
            return JSON.parse(JSON.stringify(settings));
          if (cmd === "plugin:os|locale") return "de-DE";
          if (cmd === "plugin:app|version") return "0.23.0";
          if (cmd.includes("permission")) return true;
          if (cmd === "plugin:event|listen") {
            (listeners[String(args?.event)] ??= []).push(Number(args?.handler));
            return Number(args?.handler);
          }
          if (cmd === "plugin:event|unlisten") {
            const list = listeners[String(args?.event)] ?? [];
            listeners[String(args?.event)] = list.filter((h) => h !== Number(args?.eventId));
            return null;
          }
          if (cmd === "pages_create")
            return { id: "page_new", title: String(args?.title ?? ""), modified_ms: 0, preview: "" };
          if (cmd === "tts_extract_document")
            return (w as { __extracted?: string }).__extracted ?? "# Ein Buch\n\nText.";
          // lva#236: Fassungen, KI-Aufträge, Export.
          if (cmd === "page_state_load" && args?.id === "page_1")
            return JSON.stringify({
              text: "# Effi Briest\n\nIn Front des Herrenhauses.",
              tab: "original",
              translatedLangs: ["English"],
            });
          if (cmd === "page_state_save") {
            ((w as { __saved?: unknown[] }).__saved ??= []).push(args);
            return null;
          }
          if (cmd === "tts_cached_translation")
            return args?.targetLang === "English" ? "# Effi Briest\n\nIn front of the manor." : null;
          if (cmd === "book_ai_templates")
            return [
              { id: "builtin:kurzbuch", title: "Kurzbuch", category: "ueberblick", description: "Kernaussagen", context: "", sections: [], target_words: 0, form: "prose" },
              { id: "builtin:zusammenfassung", title: "Zusammenfassung", category: "ueberblick", description: "Das Buch auf zwei Seiten.", context: "", sections: [], target_words: 0, form: "prose" },
              { id: "builtin:lernkarten", title: "Lernkarten", category: "lernen", description: "Fragen und Antworten zum Wiederholen.", context: "", sections: [], target_words: 0, form: "prose" },
              { id: "builtin:handlungen", title: "Handlungsempfehlungen", category: "anwenden", description: "Was sich aus dem Buch umsetzen lässt.", context: "", sections: [], target_words: 0, form: "prose" },
            ];
          if (cmd === "book_ai_summarize")
            return new Promise((resolve) => {
              (w as { __finishSummary?: () => void }).__finishSummary = () =>
                resolve({ id: "doc_new", title: "Kurzbuch – Buch", template_id: String(args?.templateId), template_title: "Kurzbuch", source: { kind: "book" }, source_label: "Buch", created_ms: 1, modified_ms: 1, words: 900, model: "", usage: null, language: args?.targetLang ?? null });
            });
          if (cmd === "tts_translate")
            return new Promise((resolve) => {
              (w as { __finishTranslate?: () => void }).__finishTranslate = () => resolve("übersetzt");
            });
          if (cmd === "library_ffmpeg_available") return true;
          if (cmd === "library_export_text") return String(args?.path);
          if (cmd === "library_export_audio") return [`${String(args?.outPath)}.${String(args?.format)}`];
          if (cmd === "library_list") return list;
          // Kurzbuch (lva#205): Dokumente und Text aus window.__kidocs / __docText.
          if (cmd === "book_ai_documents") return (w as { __kidocs?: unknown[] }).__kidocs ?? [];
          if (cmd === "book_ai_document_text") return (w as { __docText?: string }).__docText ?? "";
          if (cmd === "library_progress_load") return {};
          if (cmd === "library_progress_put") return null;
          if (cmd === "page_files")
            return args?.id === "page_3"
              ? [{ name: "teil-01.mp3", size: 1, modified_ms: 0 }]
              : [];
          if (cmd === "meetings_list")
            return [
              { id: "m1", title: "x", status: "processing" },
              ...(w.__jobs as string[]).map((id) => ({
                id,
                title: "x",
                status: (w.__jobStatus as Record<string, string>)[id] ?? "processing",
                duration_ms: (w.__jobDuration as Record<string, number>)[id] ?? null,
              })),
            ];
          if (cmd === "meetings_get_status") {
            const id = String(args?.meetingId);
            if (id === "m1") return "processing";
            return (w.__jobs as string[]).includes(id)
              ? ((w.__jobStatus as Record<string, string>)[id] ?? "processing")
              : null;
          }
          if (cmd === "meetings_get_segments")
            return (w.__segments as Record<string, unknown[]>)[String(args?.meetingId)] ?? [];
          if (cmd === "plugin:dialog|open")
            return (w as { __pick?: string }).__pick ?? null;
          if (cmd === "library_find_duplicate") {
            const t = String(args?.title ?? "").toLowerCase();
            return list.find((b) => String(b.title).toLowerCase() === t) ?? null;
          }
          if (cmd === "library_save") {
            const b = args?.book as Record<string, unknown>;
            const saved = { ...b, id: b.id || "book_new" };
            w.__books = [...list.filter((x) => x.id !== saved.id), saved];
            return saved;
          }
          if (cmd === "library_audio_import") {
            const b = list.find((x) => x.id === args?.id)!;
            // Wie der Kern: die Audiofassung wird am Buch gespeichert.
            const withAudio = {
              ...b,
              audio: {
                file: "hoerbuch.mp3",
                duration_ms: (w as { __audioDuration?: number }).__audioDuration ?? null,
                narrator: null,
                transcript_job: null,
              },
              audio_path: "C:/lib/x/audio/hoerbuch.mp3",
            };
            w.__books = [...list.filter((x) => x.id !== b.id), withAudio];
            return withAudio;
          }
          if (cmd === "meetings_import_audiobook_part") {
            const id = `m${++jobCounter}`;
            (w.__jobs as string[]).push(id);
            return id;
          }
          if (cmd === "library_audio_import_tracks") {
            const b = list.find((x) => x.id === args?.id) as Record<string, any>;
            const old = b.audio?.tracks?.length
              ? b.audio.tracks
              : b.audio
                ? [{ file: b.audio.file, title: "hoerbuch", duration_ms: null, transcript_job: null }]
                : [];
            const base = args?.append ? old : [];
            const fresh = (args?.paths as string[]).map((p, i) => ({
              file: `teil_${String(base.length + i + 1).padStart(3, "0")}.mp3`,
              title: (p.split("/").pop() ?? p).replace(/\.[^.]+$/, ""),
              duration_ms: null,
              transcript_job: null,
            }));
            const tracks = [...base, ...fresh];
            const withTracks = {
              ...b,
              audio: {
                file: tracks[0].file,
                duration_ms: null,
                narrator: null,
                transcript_job: null,
                tracks,
                sync: [],
              },
              audio_paths: tracks.map((t: { file: string }) => `C:/lib/${b.id}/audio/${t.file}`),
              audio_path: `C:/lib/${b.id}/audio/${tracks[0].file}`,
            };
            w.__books = [...list.filter((x) => x.id !== b.id), withTracks];
            return withTracks;
          }
          // Papierkorb (Entfernen mit Rückgängig).
          if (cmd === "library_trash") {
            const ids = new Set(args?.ids as string[]);
            const trash = ((w as { __trash?: unknown[] }).__trash ??= []) as Record<string, unknown>[];
            trash.push(...list.filter((x) => ids.has(String(x.id))));
            w.__books = list.filter((x) => !ids.has(String(x.id)));
            return null;
          }
          if (cmd === "library_restore") {
            const ids = new Set(args?.ids as string[]);
            const trash = ((w as { __trash?: unknown[] }).__trash ?? []) as Record<string, unknown>[];
            const back = trash.filter((x) => ids.has(String(x.id)));
            (w as { __trash?: unknown[] }).__trash = trash.filter((x) => !ids.has(String(x.id)));
            w.__books = [...list, ...back];
            return back;
          }
          if (cmd === "library_trash_list") {
            const trash = ((w as { __trash?: unknown[] }).__trash ?? []) as Record<string, unknown>[];
            return trash.map((book) => ({
              book,
              removed_ms: 1_793_000_000_000,
              bytes: book.audio ? 10_485_760 : 49_152,
            }));
          }
          if (cmd === "library_sizes") {
            // Mit Hörbuch-Kopie 10 MB, sonst nur Angaben und Cover (48 KB).
            return (args?.ids as string[]).map((id) =>
              list.find((x) => x.id === id)?.audio ? 10_485_760 : 49_152,
            );
          }
          if (cmd === "library_purge") {
            const ids = args?.ids as string[] | null | undefined;
            if (ids) {
              const gone = new Set(ids);
              const trash = ((w as { __trash?: unknown[] }).__trash ?? []) as Record<string, unknown>[];
              (w as { __trash?: unknown[] }).__trash = trash.filter((x) => !gone.has(String(x.id)));
            }
            return null;
          }
          if (cmd === "library_merge_audio") {
            const target = list.find((x) => x.id === args?.targetId) as Record<string, any>;
            const parts = (b: Record<string, any>) =>
              b.audio?.tracks?.length
                ? b.audio.tracks
                : b.audio
                  ? [{ file: b.audio.file, title: b.title, duration_ms: b.audio.duration_ms, transcript_job: b.audio.transcript_job }]
                  : [];
            const sources = (args?.sourceIds as string[]).map(
              (id) => list.find((x) => x.id === id) as Record<string, any>,
            );
            const tracks = [...parts(target), ...sources.flatMap(parts)].map((t, i) => ({
              ...t,
              file: `teil_${String(i + 1).padStart(3, "0")}.mp3`,
            }));
            const merged = {
              ...target,
              audio: { file: tracks[0].file, duration_ms: null, narrator: null, transcript_job: null, tracks, sync: [] },
              audio_paths: tracks.map((t: { file: string }) => `C:/lib/${target.id}/audio/${t.file}`),
              audio_path: `C:/lib/${target.id}/audio/${tracks[0].file}`,
            };
            const gone = new Set(args?.sourceIds as string[]);
            w.__books = [...list.filter((x) => x.id !== target.id && !gone.has(String(x.id))), merged];
            return merged;
          }
          // Das Vorlesen nach „Lesen“/Play braucht seine Grunddaten.
          if (cmd === "tts_server_status") return { phase: "stopped", message: null };
          if (cmd === "pages_list")
            return [
              { id: "page_1", title: "Effi Briest" },
              { id: "page_3", title: "Die Verwandlung" },
            ];
          if (cmd === "page_state_load") {
            const custom = (w.__pageText as Record<string, string>)[String(args?.id)];
            return JSON.stringify({
              text: custom ?? "# Effi Briest\n\nIn Front des Herrenhauses.",
              tab: "original",
            });
          }
          if (cmd === "page_dir") return "C:/projects/page_1";
          if (cmd === "get_custom_sounds") return { start: false, stop: false };
          if (cmd === "meetings_is_recording") return false;
          if (cmd === "get_selected_model") return "";
          if (
            cmd.endsWith("_list") ||
            cmd.startsWith("tts_list") ||
            cmd === "tts_reading_list" ||
            cmd === "llm_ps"
          )
            return [];
          if (cmd === "library_metadata_search") return [];
          // Cover ändern: Vorschläge aus window.__coverCandidates.
          if (cmd === "library_cover_candidates") {
            const all = ((w as { __coverCandidates?: Record<string, unknown>[] })
              .__coverCandidates ?? []);
            return args?.online ? all : all.filter((c) => c.source === "audio");
          }
          if (
            cmd === "library_cover_fetch" ||
            cmd === "library_cover_from_audio" ||
            cmd === "library_cover_from_file" ||
            cmd === "library_cover_clear"
          ) {
            const b = list.find((x) => x.id === args?.id)!;
            const source =
              cmd === "library_cover_fetch"
                ? "openlibrary"
                : cmd === "library_cover_from_audio"
                  ? "audio"
                  : cmd === "library_cover_from_file"
                    ? "file"
                    : "generated";
            const changed = {
              ...b,
              cover_source: source,
              cover_path:
                source === "generated"
                  ? null
                  : `C:/lib/${b.id}/cover.jpg?v=${args?.coverId ?? source}`,
            };
            w.__books = [...list.filter((x) => x.id !== b.id), changed];
            return changed;
          }
          if (cmd.includes("models") || cmd.includes("history") || cmd.includes("devices"))
            return [];
          return null;
        },
      },
    });
  }, BOOKS);
});

const calls = (page: Page, cmd: string) =>
  page.evaluate(
    (cmd) =>
      ((window as unknown as { __calls: [string, Record<string, unknown>][] })
        .__calls ?? [])
        .filter(([c]) => c === cmd)
        .map(([, a]) => a),
    cmd,
  );

const KIDOCS = [
  {
    id: "doc_kb",
    title: "Kurzbuch – Buch",
    template_id: "builtin:kurzbuch",
    template_title: "Kurzbuch",
    source: { kind: "book" },
    source_label: "Buch",
    created_ms: 1,
    modified_ms: 2,
    words: 900,
    model: "",
    usage: null,
    language: "de",
  },
];

const setTheme = (page: Page, theme: "light" | "dark") =>
  page.addInitScript((th) => {
    (window as unknown as { __settings: { theme: string } }).__settings.theme = th;
  }, theme);

for (const theme of ["light", "dark"] as const) {
  for (const width of [1440, 390]) {
    test(`Vorschau ${theme} ${width}: KI-Menü, Dialog „Mit KI erstellen“, Exportieren`, async ({
      page,
    }) => {
      await setTheme(page, theme);
      await page.addInitScript((docs) => {
        (window as unknown as { __kidocs: unknown[] }).__kidocs = docs;
      }, KIDOCS);
      await page.setViewportSize({ width, height: width < 500 ? 844 : 900 });
      await page.goto("/");
      await expect(page.getByTestId("library-card")).toHaveCount(3);
      const suffix = `${width}-${theme === "light" ? "hell" : "dunkel"}`;
      await page.getByTestId("library-ki-book_1").click();
      await expect(page.getByTestId("library-ki-translate-book_1")).toBeVisible();
      await page.waitForTimeout(350);
      await page.screenshot({ path: shot(`fassungen-01-kimenue-${suffix}.png`) });
      await page.getByTestId("library-ki-translate-book_1").click();
      await expect(page.getByTestId("ki-create")).toBeVisible();
      await expect(page.getByTestId("ki-create-have-translations")).toContainText("English");
      await page.waitForTimeout(350);
      await page.screenshot({ path: shot(`fassungen-02-uebersetzen-${suffix}.png`) });
      await page.getByRole("tab", { name: "KI-Dokument" }).click();
      await expect(page.getByTestId("ki-create-template-builtin:lernkarten")).toBeVisible();
      await page.waitForTimeout(350);
      await page.screenshot({ path: shot(`fassungen-03-kidokument-${suffix}.png`) });
      await page.getByRole("tab", { name: "Kurzbuch" }).click();
      await page.getByTestId("ki-create-start").click();
      await expect(page.getByTestId("ki-create")).toHaveCount(0);
      await expect(page.getByTestId("library-kijob-book_1")).toBeVisible();
      await page.evaluate(() =>
        (window as unknown as { __emit: (e: string, p: unknown) => void }).__emit(
          "book-ai-progress",
          { page_id: "page_1", kind: "summary", done: 3, total: 8 },
        ),
      );
      await expect(page.getByTestId("library-kijob-book_1")).toContainText("38 %");
      await page.waitForTimeout(350);
      await page.screenshot({ path: shot(`fassungen-04-in-arbeit-${suffix}.png`), fullPage: width < 500 });
      await page.getByTestId("library-more-book_1").click();
      await page.getByTestId("library-export-book_1").click();
      await expect(page.getByTestId("export-dialog")).toBeVisible();
      await expect(page.getByTestId("export-row-translation")).toBeVisible();
      await expect(page.getByTestId("export-row-kurzbuch")).toBeVisible();
      await page.waitForTimeout(350);
      await page.screenshot({ path: shot(`fassungen-05-export-${suffix}.png`) });
    });
  }
}

test("Übersetzen aus der Bibliothek: läuft im Hintergrund und trägt die Sprache in die Seite ein", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.getByTestId("library-ki-book_1").click();
  await page.getByTestId("library-ki-translate-book_1").click();
  await expect(page.getByTestId("ki-create")).toBeVisible();
  // Vorbelegt ist eine Sprache, die das Buch noch nicht hat (nicht English).
  await expect(page.getByTestId("ki-create-start")).toHaveText("Übersetzung starten");
  await page.getByTestId("ki-create-start").click();
  await expect(page.getByTestId("library-kijob-row")).toBeVisible();
  const [call] = await calls(page, "tts_translate");
  expect(call.text).toContain("In Front des Herrenhauses");
  expect(call.targetLang).not.toBe("English");
  // Seite verlassen und zurück: der Auftrag bleibt sichtbar.
  await page.getByTestId("library-ki-book_1").click();
  await page.keyboard.press("Escape");
  await page.evaluate(() =>
    (window as unknown as { __finishTranslate: () => void }).__finishTranslate(),
  );
  await expect(page.getByTestId("library-kijob-row")).toHaveCount(0);
  const saved = (await page.evaluate(
    () => (window as unknown as { __saved?: { id: string; state: string }[] }).__saved ?? [],
  )) as { id: string; state: string }[];
  const last = saved.filter((s) => s.id === "page_1").pop()!;
  expect(JSON.parse(last.state).translatedLangs).toEqual(["English", call.targetLang]);
  await expect(page.getByText(/ist fertig/)).toBeVisible();
});

test("Kurzbuch aus der Bibliothek: Kern-Aufruf mit Vorlage und Sprache, ohne Vorlesen zu öffnen", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.getByTestId("library-ki-book_1").click();
  await page.getByTestId("library-ki-kurzbuch-book_1").click();
  await page.getByTestId("ki-create-start").click();
  const [call] = await calls(page, "book_ai_summarize");
  expect(call.pageId).toBe("page_1");
  expect(call.templateId).toBe("builtin:kurzbuch");
  // Die Bibliothek bleibt offen.
  await expect(page.getByTestId("library-card")).toHaveCount(3);
  // Ein zweites Kurzbuch derselben Sprache startet nicht doppelt.
  await page.getByTestId("library-ki-book_1").click();
  await page.getByTestId("library-ki-kurzbuch-book_1").click();
  await expect(page.getByTestId("ki-create-start")).toBeDisabled();
  await page.keyboard.press("Escape");
  await page.evaluate(() =>
    (window as unknown as { __finishSummary: () => void }).__finishSummary(),
  );
  await expect(page.getByTestId("library-kijob-row")).toHaveCount(0);
  expect(await calls(page, "book_ai_summarize")).toHaveLength(1);
});

test("Hörbuch ohne Text: KI-Dialog verweist auf die Transkription", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.getByTestId("library-ki-book_2").click();
  await page.getByTestId("library-ki-translate-book_2").click();
  await expect(page.getByTestId("ki-create-notext")).toContainText("transkribieren", { ignoreCase: true });
  await expect(page.getByTestId("ki-create-start")).toHaveCount(0);
});

test("Exportieren: Übersetzung als EPUB und Buchtext als PDF in einen Ordner", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript(() => {
    (window as unknown as { __pick: string }).__pick = "C:/Export";
  });
  await page.goto("/");
  await page.getByTestId("library-more-book_1").click();
  await page.getByTestId("library-export-book_1").click();
  await expect(page.getByTestId("export-dialog")).toBeVisible();
  const translation = page.getByTestId("export-row-translation");
  await translation.getByRole("checkbox").check();
  await page.getByTestId("export-run").click();
  await expect(page.getByTestId("export-state-original")).toContainText("gespeichert");
  await expect(page.getByTestId("export-state-translation")).toContainText("gespeichert");
  const exported = await calls(page, "library_export_text");
  expect(exported.map((c) => c.path)).toEqual([
    "C:/Export/Effi Briest – Buchtext · Deutsch.epub",
    "C:/Export/Effi Briest – Übersetzung · English.epub",
  ]);
  expect(exported[1].markdown).toContain("In front of the manor.");
  expect((exported[0].meta as { authors: string[] }).authors).toEqual(["Theodor Fontane"]);
  await expect(page.getByTestId("export-reveal")).toBeVisible();
});

test("Exportieren: Hörbuch als M4B mit ffmpeg", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript(() => {
    (window as unknown as { __pick: string }).__pick = "C:/Export";
  });
  await page.goto("/");
  await page.getByTestId("library-more-book_2").click();
  await page.getByTestId("library-export-book_2").click();
  const row = page.getByTestId("export-row-audiobook");
  await expect(row.getByRole("checkbox")).toBeChecked();
  await row.getByLabel(/Format für/).click();
  await page.getByRole("option", { name: "Hörbuch (M4B, Kapitel)" }).click();
  await page.getByTestId("export-run").click();
  await expect(page.getByTestId("export-state-audiobook")).toContainText("gespeichert");
  const [call] = await calls(page, "library_export_audio");
  expect(call.format).toBe("m4b");
  expect(call.paths).toEqual(["C:/lib/book_2/audio/hoerbuch.m4b"]);
  expect(call.outPath).toBe("C:/Export/Der Schimmelreiter – Hörbuch");
});
