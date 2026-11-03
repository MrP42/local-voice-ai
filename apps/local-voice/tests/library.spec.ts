import { test, expect, type Page } from "@playwright/test";
import { join } from "node:path";
import { writeFileSync } from "node:fs";
import { oeffneProfilmenue } from "./profilmenue";

// Bibliothek (lva#152): ein Buch = ein Eintrag, Raster mit Weiterhören,
// Produktseite, Hinzufügen mit Dublettenerkennung, dauerhafter Fortschritt.
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
          if (cmd === "library_list") return list;
          // Kurzbuch (lva#205): Dokumente und Text aus window.__kidocs / __docText.
          if (cmd === "book_ai_documents") return (w as { __kidocs?: unknown[] }).__kidocs ?? [];
          if (cmd === "book_ai_document_text") return (w as { __docText?: string }).__docText ?? "";
          if (cmd === "library_progress_load") return {};
          // Hörbuch-Sync (Live-Abgleich): Antwort aus window.__liveWindow, sonst
          // „nicht verfügbar“ wie ohne Erkennungsmodell.
          if (cmd === "audiosync_load")
            return (
              (w as { __liveCache?: unknown }).__liveCache ?? {
                text_key: "",
                points: [],
                inserts: [],
                covered: [],
                anchors: [],
              }
            );
          if (cmd === "audiosync_anchor") {
            const a = args as { chars: number; ms: number };
            return {
              text_key: "",
              points: [],
              inserts: [],
              covered: [],
              anchors: [{ chars: a.chars, ms: a.ms }],
            };
          }
          if (cmd === "audiosync_window") {
            const live = (w as { __liveWindow?: (a: unknown) => unknown }).__liveWindow;
            if (!live) throw "Kein Erkennungsmodell";
            return live(args);
          }
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

test("Raster: ein Eintrag je Buch mit Text-/Audio-Kennzeichen und Fortschritt", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  const cards = page.getByTestId("library-card");
  await expect(cards).toHaveCount(3);
  await page.screenshot({ path: shot("bibliothek-raster-mehrere-1440.png") });
  const effi = cards.filter({ hasText: "Effi Briest" });
  await expect(effi.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "40");
  await expect(effi).toContainText("Text");
  // Ohne Audio eindeutig beschriftet, kein Abspielen, sondern Lesen.
  await expect(effi).toContainText("Kein Audio");
  await expect(effi.getByTestId("library-play")).toHaveCount(0);
  await expect(effi.getByTestId("library-read-quick")).toBeVisible();
  const kafka = cards.filter({ hasText: "Die Verwandlung" });
  await expect(kafka).toContainText("Vertont");
  await expect(kafka.getByTestId("library-play")).toBeVisible();
  const storm = cards.filter({ hasText: "Der Schimmelreiter" });
  await expect(storm).toContainText("Hörbuch");
  await expect(storm).toContainText("Wird transkribiert");
  // Weitermachen zeigt das begonnene Buch, „In Arbeit“ die Transkription.
  await expect(page.getByRole("heading", { name: /Weitermachen/ })).toBeVisible();
  await expect(page.getByRole("heading", { name: /In Arbeit/ })).toBeVisible();
  await expect(page.locator(".library-jobs")).toContainText("Wird transkribiert");
  // Filter „Ohne Audio“: nur Effi (Text ohne vertonte Dateien).
  await page.getByTestId("library-filter-noAudio").click();
  await expect(cards).toHaveCount(1);
  await expect(cards.first()).toContainText("Effi Briest");
  await page.screenshot({ path: shot("bibliothek-raster-1440.png") });
});

test("Klick aufs Cover öffnet die Produktseite, Lesen springt ins Vorlesen", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.getByRole("button", { name: "„Effi Briest“: Details öffnen" }).click();
  const detail = page.getByTestId("library-detail");
  await expect(detail.getByRole("heading", { name: "Effi Briest" })).toBeVisible();
  // Ohne Audio kein Abspielen; Lesen ist die Hauptaktion.
  await expect(page.getByTestId("library-detail-play")).toHaveCount(0);
  await detail.getByRole("tab", { name: "Fassungen" }).click();
  const status = page.getByTestId("library-status");
  await expect(status).toContainText("Kein Audio");
  await expect(status).toContainText("Nicht erzeugt");
  await expect(status).toContainText("Nicht nötig");
  await detail.getByRole("tab", { name: "Übersicht" }).click();
  await expect(detail).toContainText("Reclam");
  await expect(detail).toContainText("Die siebzehnjährige Effi");
  await page.screenshot({ path: shot("bibliothek-detail-1440.png") });
  await page.getByTestId("library-detail-read").click();
  await expect
    .poll(() => page.evaluate(() => window.localStorage.getItem("lva.ui.tts.activePage")))
    .toBe("page_1");
  await expect
    .poll(() => page.evaluate(() => window.localStorage.getItem("lva.ui.section")))
    .toBe("tts");
});

test("Play auf der Karte startet sofort: Vorlesen mit Autostart", async ({ page }) => {
  await page.goto("/");
  await page
    .getByTestId("library-card")
    .filter({ hasText: "Die Verwandlung" })
    .getByTestId("library-play")
    .click();
  // Das Vorlesen übernimmt den Auftrag und spricht sofort los.
  await expect
    .poll(() => page.evaluate(() => window.localStorage.getItem("lva.ui.section")))
    .toBe("tts");
  await expect
    .poll(async () =>
      (await calls(page, "tts_speak_text")).length +
      (await calls(page, "tts_speak_resume")).length,
    )
    .toBeGreaterThan(0);
  expect(
    await page.evaluate(() => window.sessionStorage.getItem("lva.library.autoplay")),
  ).toBeNull();
});

test("Hörbuch zu vorhandenem Buch: wird angehängt, kein zweiter Eintrag", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as unknown as { __pick: string }).__pick = "C:/hoerbuecher/Effi_Briest.mp3";
  });
  await page.goto("/");
  await page.getByTestId("library-add").click();
  await page.getByTestId("library-add-pick").click();
  await expect(page.getByTestId("library-add-title")).toHaveValue("Effi Briest");
  await expect(page.getByTestId("library-duplicate")).toContainText(
    "bekommt dieses Hörbuch als Audiofassung",
  );
  await page.screenshot({ path: shot("bibliothek-hinzufuegen.png") });
  await page.getByTestId("library-add-submit").click();
  await expect
    .poll(async () => (await calls(page, "library_audio_import")).map((a) => a.id))
    .toEqual(["book_1"]);
  const saves = await calls(page, "library_save");
  expect(saves.every((a) => (a.book as { id: string }).id === "book_1")).toBe(true);
  // Effi hat schon Text: keine Transkription nötig.
  expect(await calls(page, "meetings_import_audiobook_part")).toHaveLength(0);
});

test("Neues Hörbuch: eigener Eintrag und Transkription in der Warteschlange", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as unknown as { __pick: string }).__pick =
      "C:/hoerbuecher/Franz Kafka - Der Prozess.m4b";
  });
  await page.goto("/");
  await page.getByTestId("library-add").click();
  await page.getByTestId("library-add-pick").click();
  await expect(page.getByTestId("library-add-title")).toHaveValue("Der Prozess");
  await expect(page.getByTestId("library-add-authors")).toHaveValue("Franz Kafka");
  await expect(page.getByTestId("library-duplicate")).toHaveCount(0);
  await page.getByTestId("library-add-submit").click();
  await expect
    .poll(async () => (await calls(page, "meetings_import_audiobook_part")).length)
    .toBe(1);
  const saves = await calls(page, "library_save");
  expect((saves[0].book as { id: string }).id).toBe("");
  expect(
    saves.some((a) => (a.book as { audio?: { transcript_job?: string } }).audio?.transcript_job === "m2"),
  ).toBe(true);
});

test("390 px: zwei Spalten, keine waagerechte Scrollleiste", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  const cards = page.getByTestId("library-card");
  await expect(cards).toHaveCount(3);
  const [a, b] = await Promise.all([cards.nth(0).boundingBox(), cards.nth(1).boundingBox()]);
  expect(Math.abs((a?.y ?? 0) - (b?.y ?? 1))).toBeLessThan(2);
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  expect(overflow).toBeLessThanOrEqual(0);
  await page.screenshot({ path: shot("bibliothek-raster-390.png"), fullPage: true });
});

test("390 px: Produktseite gestapelt, ohne waagerechte Scrollleiste", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  await page.getByRole("button", { name: "„Der Schimmelreiter“: Details öffnen" }).click();
  const detail = page.getByTestId("library-detail");
  await expect(detail.getByRole("heading", { name: "Der Schimmelreiter" })).toBeVisible();
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  expect(overflow).toBeLessThanOrEqual(0);
  await page.screenshot({ path: shot("bibliothek-detail-390.png"), fullPage: true });
  await detail.getByRole("tab", { name: "Fassungen" }).click();
  await expect(page.getByTestId("library-status")).toContainText("Wird transkribiert");
  // Ganze Reiterseite im Bild: Fenster so hoch wie der Inhalt.
  await page.getByTestId("library-status").scrollIntoViewIfNeeded();
  await page.setViewportSize({ width: 390, height: 1500 });
  await expect(page.locator(".library-versions")).toBeInViewport({ ratio: 1 });
  await page.screenshot({ path: shot("bibliothek-fassungen-390.png") });
});

for (const width of [1440, 390]) {
  test(`Mini-Player (${width} px) verdeckt keinen Inhalt`, async ({ page }) => {
    await page.setViewportSize({ width, height: width > 500 ? 900 : 844 });
    await page.goto("/");
    await page
      .getByTestId("library-card")
      .filter({ hasText: "Der Schimmelreiter" })
      .getByTestId("library-play")
      .click();
    const bar = page.locator(".library-audiobar");
    await expect(bar).toBeVisible();
    // Touchflächen: Mehrmenü der Karten und alle Knöpfe der Hörleiste ≥ 44 px.
    const targets = [
      ...(await bar.locator("button").all()),
      ...(await page.locator(".library-touch button").all()),
    ];
    expect(targets.length).toBeGreaterThanOrEqual(7);
    // Keine eingebaute Audioleiste; Schieber mit voller Trefferhöhe.
    await expect(bar.locator("audio")).toBeHidden();
    expect(await bar.locator("audio").getAttribute("controls")).toBeNull();
    const seek = await bar.getByTestId("library-audiobar-seek").boundingBox();
    expect(seek?.height ?? 0).toBeGreaterThanOrEqual(44);
    expect(seek?.width ?? 0).toBeGreaterThanOrEqual(120);
    for (const target of targets) {
      const box = await target.boundingBox();
      expect(box?.width ?? 0).toBeGreaterThanOrEqual(44);
      expect(box?.height ?? 0).toBeGreaterThanOrEqual(44);
    }
    // Bis ans Ende gescrollt, liegt die letzte Karte ganz über der Leiste.
    const last = page.getByTestId("library-card").last();
    await last.scrollIntoViewIfNeeded();
    await page.evaluate(() =>
      document.querySelectorAll("*").forEach((el) => {
        if (el.scrollHeight > el.clientHeight + 1) el.scrollTop = el.scrollHeight;
      }),
    );
    const [cardBox, barBox] = await Promise.all([last.boundingBox(), bar.boundingBox()]);
    expect((cardBox?.y ?? 0) + (cardBox?.height ?? 0)).toBeLessThanOrEqual((barBox?.y ?? 0) + 1);
    await page.screenshot({ path: shot(`bibliothek-miniplayer-${width}.png`) });
  });
}

// ---------------------------------------------------------------------------
// Drag & Drop (Patrick 01.11.)
// ---------------------------------------------------------------------------

const emit = (page: Page, event: string, payload: unknown) =>
  page.evaluate(
    ([e, p]) => (window as unknown as { __emit: (e: string, p: unknown) => void }).__emit(e, p),
    [event, payload] as const,
  );

const centerOf = async (page: Page, locator: ReturnType<Page["locator"]>) => {
  const b = (await locator.boundingBox())!;
  return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
};

const dragTo = async (
  page: Page,
  at: { x: number; y: number },
  paths: string[],
  release = true,
) => {
  await emit(page, "tauri://drag-enter", { paths, position: at });
  await emit(page, "tauri://drag-over", { position: at });
  if (release) await emit(page, "tauri://drag-drop", { paths, position: at });
};

const effiCover = (page: Page) =>
  page.getByTestId("library-card").filter({ hasText: "Effi Briest" }).locator(".library-card__open");

test("Ablegen auf ein Buch ohne Audio: Hörbuch wird ohne Rückfrage angehängt", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  const at = await centerOf(page, effiCover(page));
  await dragTo(page, at, ["C:/Downloads/track01.mp3"], false);
  await expect(page.getByTestId("library-drop-hint")).toContainText("zu „Effi Briest“ hinzufügen");
  await expect(page.locator(".library-card--drop")).toHaveCount(1);
  await page.screenshot({ path: shot("bibliothek-drop-auf-buch-1440.png") });
  await emit(page, "tauri://drag-drop", { paths: ["C:/Downloads/track01.mp3"], position: at });
  await expect
    .poll(async () => (await calls(page, "library_audio_import")).map((a) => a.id))
    .toEqual(["book_1"]);
  await expect(page.getByTestId("library-drop-question")).toHaveCount(0);
  // Effi hat schon Text: keine Transkription.
  expect(await calls(page, "meetings_import_audiobook_part")).toHaveLength(0);
  // Ergebnis sichtbar: Hinweis „angehängt“, Karte zeigt jetzt Audio.
  await expect(page.getByText("An „Effi Briest“ angehängt.")).toBeVisible();
  await expect(
    page.getByTestId("library-card").filter({ hasText: "Effi Briest" }),
  ).toContainText("Hörbuch");
  await page.waitForTimeout(500);
  await page.screenshot({ path: shot("bibliothek-drop-angehaengt-1440.png") });
});

test("Ablegen auf ein Buch mit gleicher Fassung: Rückfrage, „Als neues Buch“ legt neu an", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.evaluate(() => {
    (window as unknown as { __extracted: string }).__extracted =
      "# Effi Briest\n\nvon Theodor Fontane\n\nEs war Mittag.";
  });
  await dragTo(page, await centerOf(page, effiCover(page)), ["C:/Buecher/effi.epub"]);
  const question = page.getByTestId("library-drop-question");
  await expect(question).toContainText("effi.epub");
  await expect(page.getByTestId("library-drop-attach")).toHaveText("Textfassung ersetzen");
  await expect(page.getByTestId("library-drop-consequence")).toContainText(
    "Lesefortschritt beginnt von vorn",
  );
  await expect(page.getByTestId("library-drop-consequence")).toContainText(
    "bleibt als Seite im Vorlesen erhalten",
  );
  await expect(page.getByTestId("library-drop-progress")).toContainText("%");
  await page.screenshot({ path: shot("bibliothek-drop-rueckfrage-1440.png") });
  await page.getByTestId("library-drop-new").click();
  await expect
    .poll(async () => (await calls(page, "library_save")).map((a) => (a.book as { id: string }).id))
    .toContain("");
  expect(await calls(page, "pages_create")).toHaveLength(1);
});

test("Ablegen auf freie Fläche: bekanntes Werk → Rückfrage, Anhängen hängt an", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  const free = await centerOf(page, page.getByRole("heading", { name: /Alle Titel/ }));
  await dragTo(page, { x: free.x + 400, y: free.y }, ["C:/Hoerbuecher/Effi Briest.mp3"]);
  await expect(page.getByTestId("library-drop-question")).toContainText("Effi Briest");
  await page.getByTestId("library-drop-attach").click();
  await expect
    .poll(async () => (await calls(page, "library_audio_import")).map((a) => a.id))
    .toEqual(["book_1"]);
});

test("Ablegen auf freie Fläche: unbekanntes Hörbuch wird sofort neues Buch mit Transkription", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  const free = await centerOf(page, page.getByRole("heading", { name: /Alle Titel/ }));
  await dragTo(page, free, ["C:/Hoerbuecher/Franz Kafka - Der Prozess.m4b"], false);
  await expect(page.getByTestId("library-drop-hint")).toContainText("neues Buch");
  await page.screenshot({ path: shot("bibliothek-drop-frei-390.png") });
  await emit(page, "tauri://drag-drop", {
    paths: ["C:/Hoerbuecher/Franz Kafka - Der Prozess.m4b"],
    position: free,
  });
  await expect.poll(async () => (await calls(page, "meetings_import_audiobook_part")).length).toBe(1);
  expect(await page.getByTestId("library-drop-question").count()).toBe(0);
  const saves = await calls(page, "library_save");
  expect((saves[0].book as { id: string; title: string }).id).toBe("");
  expect((saves[0].book as { title: string }).title).toBe("Der Prozess");
});

test("Vorlesen: Buch ablegen öffnet es, Hörbuch geht in die Bibliothek", async ({ page }) => {
  await page.addInitScript(() => window.localStorage.setItem("lva.ui.section", "tts"));
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  const ws = page.locator(".tts-workspace");
  await expect(ws).toBeVisible();
  const at = await centerOf(page, ws);
  await dragTo(page, at, ["C:/Buecher/kafka.epub"], false);
  await expect(page.getByTestId("reader-drop-hint")).toBeVisible();
  await page.screenshot({ path: shot("vorlesen-drop-1440.png") });
  await emit(page, "tauri://drag-drop", { paths: ["C:/Buecher/kafka.epub"], position: at });
  await expect
    .poll(async () => (await calls(page, "tts_extract_document")).map((a) => a.path))
    .toEqual(["C:/Buecher/kafka.epub"]);
  await dragTo(page, at, ["C:/Hoerbuecher/Franz Kafka - Der Prozess.mp3"]);
  await expect.poll(async () => (await calls(page, "library_audio_import")).length).toBe(1);
  await expect(page.getByText("in die Bibliothek aufgenommen")).toBeVisible();
  const action = page.getByRole("button", { name: "Zur Bibliothek" });
  await expect(action).toBeVisible();
  // Toast samt Aktion vollständig über der Statusleiste.
  const status = (await page.locator(".workspace-status").boundingBox())!;
  // Toast-Unterkante mindestens 44 px über der Statusleiste.
  await expect
    .poll(async () => {
      const toast = (await page.locator("[data-sonner-toast]").first().boundingBox())!;
      const gap = Math.round(status.y - (toast.y + toast.height));
      console.log(`Abstand Toast-Unterkante zur Statusleiste: ${gap} px`);
      return gap;
    })
    .toBeGreaterThanOrEqual(44);
  expect((await action.boundingBox())!.height).toBeGreaterThanOrEqual(28);
  await page.waitForTimeout(500);
  await page.screenshot({ path: shot("vorlesen-drop-hoerbuch-toast-1440.png") });
});

test("390 px: Filter-Chips, Sortierung und Menüknöpfe haben Trefferflächen ≥ 44 px", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  const targets = [
    page.getByTestId("library-add"),
    page.locator(".library-bar__search input"),
    ...(await page.locator(".library-chip").all()),
    page.locator(".library-sort .app-select__control"),
    ...(await page.locator(".library-card .library-touch button").all()),
  ];
  expect(targets.length).toBeGreaterThan(5);
  for (const target of targets) {
    const box = (await target.boundingBox())!;
    expect(box.width, await target.evaluate((e) => e.className)).toBeGreaterThanOrEqual(44);
    expect(box.height, await target.evaluate((e) => e.className)).toBeGreaterThanOrEqual(44);
  }
  // Der Sortierpfeil liegt in der 44 px hohen Fläche, die als Ganzes öffnet;
  // er selbst ist mindestens 44 px breit.
  const arrow = (await page
    .locator(".library-sort .app-select__dropdown-indicator")
    .boundingBox())!;
  expect(arrow.width).toBeGreaterThanOrEqual(44);
  await page.locator(".library-sort .app-select__dropdown-indicator").click();
  await expect(page.locator(".app-select__menu")).toBeVisible();
  await page.keyboard.press("Escape");
  await page.screenshot({ path: shot("bibliothek-trefferflaechen-390.png") });
  // Menüknopf einer Karte sichtbar und geöffnet.
  const menu = page.locator(".library-card .library-touch button").first();
  await menu.scrollIntoViewIfNeeded();
  const mb = (await menu.boundingBox())!;
  console.log(`Menüknopf 390 px: ${Math.round(mb.width)}×${Math.round(mb.height)} px`);
  await menu.click();
  await expect(page.getByRole("menu")).toBeVisible();
  await page.waitForTimeout(300);
  await page.screenshot({ path: shot("bibliothek-menue-390.png") });
});

test("Mehrere Dateien auf ein Buch: alle Audiodateien werden Teile, Fehler halten die Warteschlange nicht an", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await dragTo(page, await centerOf(page, effiCover(page)), [
    "C:/x/kaputt.xyz",
    "C:/Downloads/teil2.mp3",
    "C:/Downloads/teil1.mp3",
  ]);
  // Beide Hörteile gehören zu Effi (in Hörreihenfolge), kein neues Buch.
  await expect
    .poll(async () => await calls(page, "library_audio_import_tracks"))
    .toEqual([
      {
        id: "book_1",
        paths: ["C:/Downloads/teil1.mp3", "C:/Downloads/teil2.mp3"],
        append: false,
      },
    ]);
  expect(await calls(page, "library_audio_import")).toHaveLength(0);
  await expect(page.getByText(/Dateityp .xyz|\.xyz/).first()).toBeVisible();
});


// ---------------------------------------------------------------------------
// Mehrteilige Hörbücher, Mitlesen, Zusammenführen, AAX (lva#204)
// ---------------------------------------------------------------------------

/** Suchleiste setzen: nativer Setter plus input-Ereignis, damit React die Änderung sieht. */
const seekTo = (page: Page, seconds: number) =>
  page.getByTestId("library-audiobar-seek").evaluate((el, v) => {
    const input = el as HTMLInputElement;
    const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    set.call(input, String(v));
    input.dispatchEvent(new Event("input", { bubbles: true }));
  }, seconds);

const pad = (n: number, width = 2) => String(n).padStart(width, "0");

/** Stilles WAV (8 kHz, 8 Bit, mono) der gewünschten Länge. */
const wav = (seconds: number): Buffer => {
  const rate = 8000;
  const n = rate * seconds;
  const head = Buffer.alloc(44);
  head.write("RIFF", 0);
  head.writeUInt32LE(36 + n, 4);
  head.write("WAVEfmt ", 8);
  head.writeUInt32LE(16, 16);
  head.writeUInt16LE(1, 20);
  head.writeUInt16LE(1, 22);
  head.writeUInt32LE(rate, 24);
  head.writeUInt32LE(rate, 28);
  head.writeUInt16LE(1, 32);
  head.writeUInt16LE(8, 34);
  head.write("data", 36);
  head.writeUInt32LE(n, 40);
  return Buffer.concat([head, Buffer.alloc(n, 0x80)]);
};

/** Echte Hördateien für die Hörleiste: `/__audio/<datei>` liefert stilles WAV mit Range. */
const serveAudio = async (page: Page, seconds: Record<string, number>) => {
  await page.addInitScript(() => {
    const internals = (
      window as unknown as { __TAURI_INTERNALS__: Record<string, unknown> }
    ).__TAURI_INTERNALS__;
    internals.convertFileSrc = (path: string) => `/__audio/${path.split("/").pop()}`;
  });
  await page.route("**/__audio/**", async (route) => {
    const name = new URL(route.request().url()).pathname.split("/").pop() ?? "";
    const body = wav(seconds[name] ?? 5);
    const range = /bytes=(\d+)-(\d*)/.exec(route.request().headers().range ?? "");
    if (!range) {
      return route.fulfill({
        status: 200,
        contentType: "audio/wav",
        headers: { "accept-ranges": "bytes" },
        body,
      });
    }
    const from = Number(range[1]);
    const to = range[2] ? Number(range[2]) : body.length - 1;
    return route.fulfill({
      status: 206,
      contentType: "audio/wav",
      headers: {
        "accept-ranges": "bytes",
        "content-range": `bytes ${from}-${to}/${body.length}`,
      },
      body: body.subarray(from, to + 1),
    });
  });
};

type Extra = Record<string, unknown>;
const addBooks = (page: Page, extra: Extra[]) =>
  page.addInitScript((books) => {
    const w = window as unknown as { __books: unknown[] };
    w.__books = [...w.__books, ...books];
  }, extra);

const baseBook = (id: string, title: string, over: Extra = {}): Extra => ({
  id,
  title,
  subtitle: null,
  authors: [],
  language: null,
  isbn: null,
  publisher: null,
  publish_date: null,
  pages: null,
  description: null,
  metadata_source: "manual",
  cover_source: "generated",
  text: null,
  audio: null,
  added_ms: 10,
  finished: false,
  cover_path: null,
  audio_path: null,
  audio_paths: [],
  ...over,
});

const track = (n: number, title: string, ms: number | null, job: string | null = null) => ({
  file: `teil_${pad(n, 3)}.wav`,
  title,
  duration_ms: ms,
  transcript_job: job,
});

const multiAudio = (
  id: string,
  tracks: ReturnType<typeof track>[],
  sync: { chars: number; ms: number }[] = [],
): Extra => ({
  audio: {
    file: tracks[0].file,
    duration_ms: null,
    narrator: null,
    transcript_job: null,
    tracks,
    sync,
  },
  audio_path: `C:/lib/${id}/audio/${tracks[0].file}`,
  audio_paths: tracks.map((t) => `C:/lib/${id}/audio/${t.file}`),
});

const legacyAudio = (id: string): Extra => ({
  audio: { file: "hoerbuch.mp3", duration_ms: null, narrator: null, transcript_job: null },
  audio_path: `C:/lib/${id}/audio/hoerbuch.mp3`,
});

const richDadFiles = () => {
  const files: string[] = [];
  [11, 12, 13].forEach((count, d) => {
    for (let t = 1; t <= count; t++)
      files.push(
        `C:/Hoerbuecher/Rich Dad/Robert Kiyosaki - Rich Dad Poor Dad Disc ${d + 1} - ${pad(t)} - Track ${pad(t)}.mp3`,
      );
  });
  return files;
};

test("Ablegen von 36 Dateien einer CD-Box: ein Buch mit 36 Teilen, ein Auftrag je Teil", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  const files = richDadFiles();
  expect(files).toHaveLength(36);
  const free = await centerOf(page, page.getByRole("heading", { name: /Alle Titel/ }));
  // Durcheinander abgelegt: die Reihenfolge stellt das Gruppieren her.
  await dragTo(page, { x: free.x + 400, y: free.y }, [...files].reverse());
  await expect.poll(async () => (await calls(page, "meetings_import_audiobook_part")).length).toBe(36);
  const imports = await calls(page, "library_audio_import_tracks");
  expect(imports).toHaveLength(1);
  expect(imports[0].paths).toEqual(files);
  expect(imports[0].append).toBe(false);
  expect(await calls(page, "library_audio_import")).toHaveLength(0);
  const saves = await calls(page, "library_save");
  const created = saves[0].book as { id: string; title: string; authors: string[] };
  expect(created).toMatchObject({
    id: "",
    title: "Rich Dad Poor Dad",
    authors: ["Robert Kiyosaki"],
  });
  // Die Aufträge landen je Teil am Buch.
  await expect
    .poll(async () => {
      const all = await calls(page, "library_save");
      const last = all[all.length - 1].book as {
        audio: { tracks: { transcript_job: string | null }[] };
      };
      return new Set(last.audio.tracks.map((t) => t.transcript_job)).size;
    })
    .toBe(36);
  // Ein Eintrag, Status „Transkription 0 von 36“.
  const cards = page.getByTestId("library-card").filter({ hasText: "Rich Dad Poor Dad" });
  await expect(cards).toHaveCount(1);
  await expect(cards).toContainText("Transkription 0 von 36");
  await expect(page.getByTestId("library-merge-bar")).toHaveCount(0);
});

test("Ablegen auf ein Buch ohne Audio: alle Dateien werden Teile in Hörreihenfolge", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await dragTo(page, await centerOf(page, effiCover(page)), [
    "C:/Downloads/Effi - Track 10.mp3",
    "C:/Downloads/Effi - Track 02.mp3",
    "C:/Downloads/Effi - Track 01.mp3",
  ]);
  await expect
    .poll(async () => await calls(page, "library_audio_import_tracks"))
    .toEqual([
      {
        id: "book_1",
        paths: [
          "C:/Downloads/Effi - Track 01.mp3",
          "C:/Downloads/Effi - Track 02.mp3",
          "C:/Downloads/Effi - Track 10.mp3",
        ],
        append: false,
      },
    ]);
  await expect(page.getByTestId("library-drop-question")).toHaveCount(0);
  // Effi hat Text: keine Transkription.
  expect(await calls(page, "meetings_import_audiobook_part")).toHaveLength(0);
  await expect(page.getByText("3 Teile an „Effi Briest“ angehängt.")).toBeVisible();
});

test("Ablegen auf ein Buch mit Audio: Rückfrage mit drei Wahlmöglichkeiten", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  const storm = page.getByTestId("library-card").filter({ hasText: "Der Schimmelreiter" });
  await dragTo(page, await centerOf(page, storm.locator(".library-card__open")), [
    "C:/Downloads/Schimmelreiter - Teil 2.mp3",
    "C:/Downloads/Schimmelreiter - Teil 3.mp3",
  ]);
  const question = page.getByTestId("library-drop-question");
  await expect(question).toContainText("2 Dateien");
  await expect(page.getByTestId("library-drop-attach")).toHaveText("Hörbuch ersetzen");
  await expect(page.getByTestId("library-drop-append")).toHaveText("Als weitere Teile anhängen");
  await expect(page.getByTestId("library-drop-new")).toHaveText("Als neues Buch");
  // Ersetzen zerstört das vorhandene Hörbuch: nur Anhängen ist primär.
  await expect(page.getByTestId("library-drop-append")).toHaveAttribute("data-variant", "primary");
  await expect(page.getByTestId("library-drop-attach")).not.toHaveAttribute(
    "data-variant",
    "primary",
  );
  await page.screenshot({ path: shot("drop-rueckfrage-drei-wahlen-1440.png") });
  await page.getByTestId("library-drop-append").click();
  await expect
    .poll(async () => await calls(page, "library_audio_import_tracks"))
    .toEqual([
      {
        id: "book_2",
        paths: [
          "C:/Downloads/Schimmelreiter - Teil 2.mp3",
          "C:/Downloads/Schimmelreiter - Teil 3.mp3",
        ],
        append: true,
      },
    ]);
});

test("Zusammenführen: Hinweisbalken, Rückfrage mit Reihenfolge, ein Buch danach", async ({
  page,
}) => {
  const parts = [3, 1, 2].map((n) =>
    baseBook(
      `book_p${n}`,
      `Robert Kiyosaki - Rich Dad Poor Dad Disc 1 - ${pad(n)} - Track ${pad(n)}`,
      { added_ms: 100 + n, ...legacyAudio(`book_p${n}`) },
    ),
  );
  await addBooks(page, parts);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  const bar = page.getByTestId("library-merge-bar");
  await expect(bar).toHaveCount(1);
  await expect(bar).toContainText("3 Einträge gehören wohl zu einem Hörbuch: Rich Dad Poor Dad");
  await page.screenshot({ path: shot("liste-merge-balken-1440.png") });
  await bar.getByTestId("library-merge-start").click();
  const dialog = page.getByTestId("library-merge-dialog");
  await expect(dialog).toBeVisible();
  await expect(page.getByTestId("library-merge-summary")).toContainText(
    "3 Einträge werden zu einem Hörbuch „Rich Dad Poor Dad“ mit 3 Teilen",
  );
  const order = page.getByTestId("library-merge-order").locator("li");
  await expect(order).toHaveCount(3);
  await expect(order.nth(0)).toContainText("Track 01");
  await expect(order.nth(2)).toContainText("Track 03");
  await expect(page.getByTestId("library-merge-replaces")).toHaveCount(0);
  await page.screenshot({ path: shot("merge-dialog-1440.png") });
  await page.getByTestId("library-merge-confirm").click();
  await expect
    .poll(async () => await calls(page, "library_merge_audio"))
    .toEqual([{ targetId: "book_p1", sourceIds: ["book_p2", "book_p3"], replace: false }]);
  // Das erste Buch bekommt vorher den Stamm-Titel.
  const saves = await calls(page, "library_save");
  expect((saves[0].book as { id: string; title: string }).title).toBe("Rich Dad Poor Dad");
  expect((saves[0].book as { id: string }).id).toBe("book_p1");
  const cards = page.getByTestId("library-card").filter({ hasText: "Rich Dad Poor Dad" });
  await expect(cards).toHaveCount(1);
  await expect(page.getByTestId("library-merge-bar")).toHaveCount(0);
  await expect(page.getByText("„Rich Dad Poor Dad“: 3 Einträge zusammengeführt.")).toBeVisible();
});

test("Zusammenführen in ein vorhandenes Werk: dessen Hörbuch wird ersetzt und im Dialog genannt", async ({
  page,
}) => {
  const parts = [1, 2].map((n) =>
    baseBook(`book_p${n}`, `Rich Dad Poor Dad - Teil ${n}`, legacyAudio(`book_p${n}`)),
  );
  const work = baseBook("book_w", "Rich Dad Poor Dad", {
    authors: ["Robert Kiyosaki"],
    ...legacyAudio("book_w"),
  });
  (work.audio as { transcript_job: string }).transcript_job = "m9";
  await addBooks(page, [...parts, work]);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.getByTestId("library-merge-start").click();
  await expect(page.getByTestId("library-merge-target")).toContainText("vorhandenen Eintrag");
  await expect(page.getByTestId("library-merge-replaces")).toContainText(
    "„Rich Dad Poor Dad“ hat schon ein Hörbuch (1 Dateien). Es wird dabei ersetzt",
  );
  await page.getByTestId("library-merge-confirm").click();
  // Das Ersetzen entscheidet der Kern in einem Schritt (replace), das Ziel wird
  // vorher nicht angefasst.
  await expect
    .poll(async () => await calls(page, "library_merge_audio"))
    .toEqual([{ targetId: "book_w", sourceIds: ["book_p1", "book_p2"], replace: true }]);
  expect(await calls(page, "library_save")).toHaveLength(0);
  // Erst nach Erfolg: der laufende Auftrag des alten Hörbuchs wird gelöscht.
  await expect
    .poll(async () => (await calls(page, "meetings_delete")).map((a) => a.meetingId))
    .toEqual(["m9"]);
});

test("Reihenbände ohne Teil-Angabe lösen keinen Zusammenführen-Hinweis aus", async ({ page }) => {
  await addBooks(
    page,
    [1, 2].map((n) => baseBook(`book_h${n}`, `Harry Potter ${n}`, legacyAudio(`book_h${n}`))),
  );
  await page.goto("/");
  await expect(page.getByTestId("library-card")).toHaveCount(5);
  await expect(page.getByTestId("library-merge-bar")).toHaveCount(0);
});

test("AAX: Toast mit DRM-Hinweis beim Ablegen und im Dialog „Hinzufügen“", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  const free = await centerOf(page, page.getByRole("heading", { name: /Alle Titel/ }));
  await dragTo(page, { x: free.x + 400, y: free.y }, ["C:/Audible/Das Buch.aax"]);
  await expect(
    page.getByText(
      "„Das Buch.aax“ ist mit Audible-DRM geschützt und kann nicht importiert werden. Verwende eine DRM-freie Fassung (MP3, M4B).",
    ),
  ).toBeVisible();
  expect(await calls(page, "library_audio_import")).toHaveLength(0);
  expect(await calls(page, "library_audio_import_tracks")).toHaveLength(0);
  // Der Toast liegt nach dem Einblenden vollständig im Fenster und ist lesbar.
  const toast = page.locator("[data-sonner-toast]").first();
  await expect(toast).toHaveCSS("opacity", "1");
  await page.waitForTimeout(600);
  const box = (await toast.boundingBox())!;
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.y).toBeGreaterThanOrEqual(0);
  expect(box.x + box.width).toBeLessThanOrEqual(1440);
  expect(box.y + box.height).toBeLessThanOrEqual(900);
  const status = (await page.locator(".workspace-status").boundingBox())!;
  expect(status.y - (box.y + box.height)).toBeGreaterThanOrEqual(44);
  const contrast = await toast.evaluate((el) => {
    const rgb = (c: string) => (c.match(/[\d.]+/g) ?? []).slice(0, 3).map(Number);
    const lum = ([r, g, b]: number[]) => {
      const f = (v: number) => {
        const x = v / 255;
        return x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4;
      };
      return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
    };
    const cs = getComputedStyle(el);
    const a = lum(rgb(cs.color));
    const b = lum(rgb(cs.backgroundColor));
    return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
  });
  expect(contrast).toBeGreaterThanOrEqual(4.5);
  await page.screenshot({ path: shot("aax-hinweis-1440.png") });
  // Dialog: nur eine AAX gewählt → Meldung im Dialog.
  await page.evaluate(() => {
    (window as unknown as { __pick: string[] }).__pick = ["C:/Audible/Anderes.aaxc"];
  });
  await page.getByTestId("library-add").click();
  await page.getByTestId("library-add-pick").click();
  await expect(page.getByRole("alert")).toContainText(
    "„Anderes.aaxc“ ist mit Audible-DRM geschützt",
  );
  await expect(page.getByTestId("library-add-title")).toHaveCount(0);
});

test("Dialog „Hinzufügen“ mit Mehrfachauswahl: eine Gruppe wird ein Buch mit Teilen", async ({
  page,
}) => {
  const files = richDadFiles().slice(0, 4);
  await page.addInitScript((picked) => {
    (window as unknown as { __pick: string[] }).__pick = picked;
  }, [...files].reverse());
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.getByTestId("library-add").click();
  await page.getByTestId("library-add-pick").click();
  await expect(page.getByTestId("library-add-title")).toHaveValue("Rich Dad Poor Dad");
  await expect(page.getByTestId("library-add-authors")).toHaveValue("Robert Kiyosaki");
  await expect(page.locator(".library-add__drop")).toContainText("4 Dateien");
  await page.getByTestId("library-add-submit").click();
  await expect
    .poll(async () => await calls(page, "library_audio_import_tracks"))
    .toEqual([{ id: "book_new", paths: files, append: false }]);
  await expect.poll(async () => (await calls(page, "meetings_import_audiobook_part")).length).toBe(4);
});

test("Hörleiste mehrteilig: durchgehende Zeit, Teilwahl, ±30 s über Teilgrenzen", async ({
  page,
}) => {
  const tracks = [
    track(1, "Disc 1 - Track 01", 10_000),
    track(2, "Disc 1 - Track 02", 20_000),
    track(3, "Disc 1 - Track 03", 30_000),
  ];
  await serveAudio(page, { "teil_001.wav": 10, "teil_002.wav": 20, "teil_003.wav": 30 });
  await addBooks(page, [
    baseBook("book_a", "Rich Dad Poor Dad", {
      authors: ["Robert Kiyosaki"],
      ...multiAudio("book_a", tracks),
    }),
  ]);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page
    .getByTestId("library-card")
    .filter({ hasText: "Rich Dad Poor Dad" })
    .getByTestId("library-play")
    .click();
  const bar = page.getByTestId("library-audiobar");
  await expect(bar).toBeVisible();
  await expect(page.getByTestId("library-audiobar-sub")).toHaveText("Teil 1 von 3");
  const seek = page.getByTestId("library-audiobar-seek");
  // Gesamtzeit über alle Teile: 60 s.
  await expect(seek).toHaveAttribute("max", "60");
  // Teilwahl: Liste der Teile, Klick springt an den Anfang des Teils.
  await page.getByTestId("library-audiobar-parts").click();
  const items = page.getByTestId("library-parts-item");
  await expect(items).toHaveCount(3);
  await page.screenshot({ path: shot("hoerleiste-teilwahl-1440.png") });
  await items.nth(1).click();
  await expect(page.getByTestId("library-audiobar-sub")).toHaveText("Teil 2 von 3");
  await expect(seek).toHaveValue(/^(10|11)$/);
  // +30 s von 10 s: 40 s liegen schon im dritten Teil.
  await page.getByTestId("library-audiobar-forward").click();
  await expect(page.getByTestId("library-audiobar-sub")).toHaveText("Teil 3 von 3");
  await expect(seek).toHaveValue(/^(40|41)$/);
  // −30 s zurück über die Grenze: 10 s = Anfang des zweiten Teils.
  await page.getByTestId("library-audiobar-back").click();
  await expect(page.getByTestId("library-audiobar-sub")).toHaveText("Teil 2 von 3");
  await expect(seek).toHaveValue(/^(10|11)$/);
  // Suchleiste über alle Teile.
  await seekTo(page, 55);
  await expect(page.getByTestId("library-audiobar-sub")).toHaveText("Teil 3 von 3");
  await expect(seek).toHaveValue(/^(55|56)$/);
  // Gesicherte Stelle: Gesamt-ms im Fortschritt `audio:<buch>`.
  await expect
    .poll(() =>
      page.evaluate(() => {
        const raw = window.localStorage.getItem("lva.ui.progress.audio:book_a");
        return raw ? Math.round((JSON.parse(raw).offset as number) / 1000) : null;
      }),
    )
    .toBeGreaterThanOrEqual(54);
  const saved = await page.evaluate(() =>
    JSON.parse(window.localStorage.getItem("lva.ui.progress.audio:book_a") ?? "{}"),
  );
  expect(saved.length).toBe(60_000);
  expect(saved.unit).toBe("ms");
});

test("Hörleiste: fehlende Dauern werden ermittelt und am Buch gesichert", async ({ page }) => {
  await serveAudio(page, { "teil_001.wav": 4, "teil_002.wav": 6 });
  await addBooks(page, [
    baseBook("book_d", "Der Prozess", {
      ...multiAudio("book_d", [track(1, "Eins", null), track(2, "Zwei", null)]),
    }),
  ]);
  await page.goto("/");
  await page
    .getByTestId("library-card")
    .filter({ hasText: "Der Prozess" })
    .getByTestId("library-play")
    .click();
  await expect(page.getByTestId("library-audiobar-seek")).toHaveAttribute("max", "10");
  await expect
    .poll(async () => {
      const saves = await calls(page, "library_save");
      const withDurations = saves
        .map((a) => a.book as { audio?: { tracks?: { duration_ms: number | null }[] } })
        .find((b) => b.audio?.tracks?.every((t) => t.duration_ms));
      return withDurations?.audio?.tracks?.map((t) => t.duration_ms);
    })
    .toEqual([4000, 6000]);
});

/** Text mit zwölf langen Absätzen und Zeitachse (Absatz k beginnt bei k × 2,5 s). */
const followText = () => {
  const paragraphs = Array.from(
    { length: 12 },
    (_, k) =>
      `Absatz ${k + 1}: ${"Der Text läuft hier ein gutes Stück weiter, damit die Lesefläche scrollen muss. ".repeat(3).trim()}`,
  );
  const prefix = "# Der Prozess\n\nvon Franz Kafka\n\n";
  let text = prefix;
  const sync: { chars: number; ms: number }[] = [];
  paragraphs.forEach((p, k) => {
    if (k > 0) text += "\n\n";
    sync.push({ chars: text.length, ms: k * 2500 });
    text += p;
  });
  return { text, sync, prefix };
};

const mitlesenBook = (withSync: boolean): Extra => {
  const { sync } = followText();
  return baseBook("book_m", "Der Prozess", {
    authors: ["Franz Kafka"],
    text: { page_id: "page_m", origin: withSync ? "transcript" : "document", source_name: null },
    ...multiAudio(
      "book_m",
      [track(1, "Teil 1", 10_000), track(2, "Teil 2", 20_000)],
      withSync ? sync : [],
    ),
  });
};

const mitlesenSetup = async (page: Page, withSync = true) => {
  const { text } = followText();
  await serveAudio(page, { "teil_001.wav": 10, "teil_002.wav": 20 });
  await addBooks(page, [mitlesenBook(withSync)]);
  await page.addInitScript((t) => {
    (window as unknown as { __pageText: Record<string, string> }).__pageText["page_m"] = t;
  }, text);
};

const openDetail = (page: Page) =>
  page.getByRole("button", { name: "„Der Prozess“: Details öffnen" }).click();

const openFollow = async (page: Page) => {
  await openDetail(page);
  await page.getByTestId("library-detail-listen-follow").click();
  await expect(page.getByTestId("library-follow")).toBeVisible();
};

const activeText = (page: Page) => page.locator(".library-follow__p--now").first().innerText();

test("Mitlesen: der gehörte Absatz ist hervorgehoben, Klick springt, gemeinsame Position", async ({
  page,
}) => {
  await mitlesenSetup(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await openFollow(page);
  expect(
    await page.evaluate(() => window.localStorage.getItem("lva.ui.library.followText")),
  ).toBe("on");
  // Ab 1024 px als Bereich über der Leiste (kein Vollbild).
  const player = page.getByTestId("library-player");
  expect((await player.boundingBox())!.height).toBeLessThan(700);
  await expect(page.locator(".library-follow__p--now")).toHaveCount(1);
  await expect.poll(() => activeText(page)).toMatch(/^Absatz 1:/);
  // 17 s liegen im Absatz 7 (Beginn 15 s).
  const seek = page.getByTestId("library-audiobar-seek");
  await seekTo(page, 17);
  await expect.poll(() => activeText(page)).toMatch(/^Absatz 7:/);
  await expect(page.locator(".library-follow__p--now")).toBeInViewport();
  // Klick auf Absatz 3 (Beginn 5 s) springt im Audio dorthin.
  await page.locator(".library-follow__p", { hasText: /^Absatz 3:/ }).dispatchEvent("click");
  await expect(seek).toHaveValue(/^(5|6)$/);
  await expect.poll(() => activeText(page)).toMatch(/^Absatz 3:/);
  // Gemeinsame Position: der Textfortschritt steht am Beginn von Absatz 3.
  const { text, sync } = followText();
  await expect
    .poll(() =>
      page.evaluate(() => {
        const raw = window.localStorage.getItem("lva.ui.progress.text:page_m:original");
        return raw ? (JSON.parse(raw).offset as number) : null;
      }),
    )
    .toBeGreaterThanOrEqual(sync[2].chars - 2);
  const offset = await page.evaluate(
    () =>
      JSON.parse(window.localStorage.getItem("lva.ui.progress.text:page_m:original") ?? "{}")
        .offset,
  );
  expect(offset).toBeLessThan(sync[3].chars);
  expect(
    await page.evaluate(() => window.localStorage.getItem("lva.ui.tts.resume.page_m:original")),
  ).not.toBeNull();
  expect(text.length).toBeGreaterThan(sync[11].chars);
  // Genaue Zeitachse: keine „ungefähr“-Kennzeichnung.
  await expect(page.getByTestId("library-follow-approx")).toHaveCount(0);
  await page.screenshot({ path: shot("mitlesen-1440-hell.png") });
});

test("Mitlesen ohne Transkript: der Live-Abgleich setzt die Stelle genau", async ({ page }) => {
  await mitlesenSetup(page, false);
  const { sync } = followText();
  // Gesprochen wird langsamer als geschätzt: Absatz k beginnt bei k × 1 s
  // (anteilig wären es k × 2,5 s). Das Fenster liefert die Stützpunkte.
  await page.addInitScript((points) => {
    (window as unknown as { __liveWindow: (a: { fromMs: number; lenMs: number }) => unknown }).__liveWindow = (
      a,
    ) => ({
      points: points.filter((p) => p.ms >= a.fromMs && p.ms < a.fromMs + a.lenMs),
      inserts: [],
      coverage: 0.95,
      next_hint: null,
      from_ms: a.fromMs,
      to_ms: Math.min(a.fromMs + a.lenMs, 30_000),
    });
  }, sync.map((p, k) => ({ chars: p.chars, ms: k * 1000 })));
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await openFollow(page);
  // Ohne Abgleich erst „ungefähr“, mit Abgleich nicht mehr.
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as { __calls: [string, Record<string, unknown>][] }).__calls.filter(
            ([c]) => c === "audiosync_window",
          ).length,
      ),
    )
    .toBeGreaterThan(0);
  await expect(page.getByTestId("library-follow-approx")).toHaveCount(0);
  const call = await page.evaluate(
    () =>
      (window as unknown as { __calls: [string, Record<string, unknown>][] }).__calls.find(
        ([c]) => c === "audiosync_window",
      )![1],
  );
  expect(call.bookId).toBe("book_m");
  expect(call.fromMs).toBeLessThan(2000);
  // 5,5 s: nach Live-Abgleich Absatz 6 (Beginn 5 s), geschätzt wäre es Absatz 3.
  // Anhalten, sonst läuft die Zeit (Absatz je Sekunde) weiter.
  await page.getByTestId("library-audiobar-toggle").click();
  await expect(page.getByTestId("library-audiobar-toggle")).toHaveAccessibleName("Abspielen");
  await seekTo(page, 5);
  await expect.poll(() => activeText(page)).toMatch(/^Absatz 6:/);
});

for (const durations of ["bekannt", "unbekannt"] as const)
  test(`„Hören und mitlesen“ aus Weitermachen startet an der gelesenen Stelle (Dauern ${durations})`, async ({
    page,
  }) => {
    const { text, sync } = followText();
    await serveAudio(page, { "teil_001.wav": 10, "teil_002.wav": 20 });
    const known = durations === "bekannt";
    await addBooks(page, [
      baseBook("book_m", "Der Prozess", {
        authors: ["Franz Kafka"],
        text: { page_id: "page_m", origin: "transcript", source_name: null },
        ...multiAudio(
          "book_m",
          [track(1, "Teil 1", known ? 10_000 : null), track(2, "Teil 2", known ? 20_000 : null)],
          sync,
        ),
      }),
    ]);
    // Zuletzt gelesen: Absatz 7 (Beginn 15 s); gehört wurde vorher nur bis 2 s.
    await page.addInitScript(
      ([t, at]) => {
        const w = window as unknown as { __pageText: Record<string, string> };
        w.__pageText["page_m"] = t as string;
        if (sessionStorage.getItem("seeded")) return;
        sessionStorage.setItem("seeded", "1");
        const now = Date.now();
        localStorage.setItem(
          "lva.ui.progress.audio:book_m",
          JSON.stringify({ offset: 2000, length: 30_000, heard: [], read: [], updated: now - 60_000, unit: "ms" }),
        );
        localStorage.setItem(
          "lva.ui.progress.text:page_m:original",
          JSON.stringify({ offset: (at as number) + 5, length: (t as string).length, heard: [], read: [], updated: now - 1000 }),
        );
      },
      [text, sync[6].chars] as const,
    );
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto("/");
    await page.getByTestId("library-continue-follow").first().click();
    await expect(page.getByTestId("library-follow")).toBeVisible();
    await expect.poll(() => activeText(page)).toMatch(/^Absatz 7:/);
    await expect
      .poll(async () => Number(await page.getByTestId("library-audiobar-seek").inputValue()))
      .toBeGreaterThanOrEqual(15);
  });

const liveCalls = (page: Page, cmd: string) =>
  page.evaluate(
    (c) =>
      (window as unknown as { __calls: [string, Record<string, unknown>][] }).__calls
        .filter(([n]) => n === c)
        .map(([, a]) => a),
    cmd,
  );

const pause = async (page: Page) => {
  const toggle = page.getByTestId("library-audiobar-toggle");
  // Erst laufen lassen (Autoplay), dann anhalten: sonst läuft die Zeit weiter.
  await expect(toggle).toHaveAccessibleName("Pause");
  await toggle.click();
  await expect(toggle).toHaveAccessibleName("Abspielen");
};

test("Satzzeile über der Hörleiste: der gehörte Satz, Klick öffnet Mitlesen", async ({ page }) => {
  await mitlesenSetup(page);
  await page.addInitScript(() => localStorage.setItem("lva.ui.library.followText", JSON.stringify("off")));
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page
    .getByTestId("library-card")
    .filter({ hasText: "Der Prozess" })
    .getByTestId("library-play")
    .click();
  const line = page.getByTestId("library-audiobar-line");
  await expect(line).toBeVisible();
  await pause(page);
  // 15 s: Beginn von Absatz 7 – erster Satz.
  await seekTo(page, 15);
  await expect(line).toContainText("Absatz 7: Der Text läuft hier");
  // 17 s: 80 % in Absatz 7 – dritter Satz, ohne „Absatz 7“.
  await seekTo(page, 17);
  await expect(line).toContainText("Der Text läuft hier");
  await expect(line).not.toContainText("Absatz");
  // Genaue Zeitachse: kein „≈“.
  await expect(page.getByTestId("library-audiobar-line-approx")).toHaveCount(0);
  const box = (await line.boundingBox())!;
  expect(box.height).toBeLessThan(48);
  await line.click();
  await expect(page.getByTestId("library-follow")).toBeVisible();
  await expect(line).toHaveCount(0);
});

test("Mitlesen: der gehörte Satz ist hervorgehoben (ohne Umbruchwechsel)", async ({ page }) => {
  await mitlesenSetup(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await openFollow(page);
  await pause(page);
  await seekTo(page, 15);
  const painted = () =>
    page.evaluate(() => {
      const h = CSS.highlights.get("follow-cur");
      if (!h) return null;
      return [...h].map((r) => (r as Range).toString()).join("");
    });
  await expect.poll(painted).toMatch(/^Absatz 7: Der Text läuft hier.*muss\.$/);
  const html = await page.locator(".library-follow__p--now").innerHTML();
  await seekTo(page, 17);
  await expect.poll(painted).toMatch(/^Der Text läuft hier.*muss\.$/);
  // Der Absatz selbst bleibt unverändert (kein wandernder Span).
  expect(await page.locator(".library-follow__p--now").innerHTML()).toBe(html);
});

test("Mitlesen: Text nur im Hörbuch steht farbig an seiner Stelle", async ({ page }) => {
  await mitlesenSetup(page, false);
  const { sync } = followText();
  await page.addInitScript((at) => {
    (window as unknown as { __liveCache: unknown }).__liveCache = {
      text_key: "",
      points: [],
      inserts: [{ at, start_ms: 4000, end_ms: 6000, text: "Kapitel drei, gelesen von Anna" }],
      covered: [],
      anchors: [],
    };
  }, sync[2].chars + 9);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await openFollow(page);
  const ins = page.getByTestId("library-follow-insert");
  await expect(ins).toHaveCount(1);
  await expect(ins).toContainText("Kapitel drei, gelesen von Anna");
  await expect(ins).toHaveAttribute("title", "Nur im Hörbuch");
  // Im dritten Absatz, nach „Absatz 3:“.
  const para = page.locator(".library-follow__p", { has: ins });
  expect(await para.textContent()).toMatch(/^Absatz 3:Nur im Hörbuch: Kapitel drei, gelesen von Anna Der Text/);
  const color = await ins.evaluate((el) => getComputedStyle(el).color);
  const plain = await para.evaluate((el) => getComputedStyle(el).color);
  expect(color).not.toBe(plain);
});

test("Mitlesen: Stelle von Hand korrigieren setzt den gehörten Satz", async ({ page }) => {
  await mitlesenSetup(page, false);
  const { sync } = followText();
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await openFollow(page);
  await pause(page);
  await seekTo(page, 6);
  await expect.poll(() => activeText(page)).toMatch(/^Absatz 3:/);
  await expect(page.getByTestId("library-follow-approx")).toBeVisible();
  await page.getByTestId("library-follow-correct").click();
  await expect(page.getByTestId("library-follow-correct-hint")).toBeVisible();
  await page.locator(".library-follow__p", { hasText: /^Absatz 9:/ }).dispatchEvent("click");
  await expect(page.getByTestId("library-follow-correct-hint")).toHaveCount(0);
  await expect.poll(() => activeText(page)).toMatch(/^Absatz 9:/);
  const [call] = await liveCalls(page, "audiosync_anchor");
  expect(call.bookId).toBe("book_m");
  expect(call.chars).toBe(sync[8].chars);
  expect(Math.abs((call.ms as number) - 6000)).toBeLessThan(1500);
  // Die Hörstelle bleibt, nur der Text folgt ihr jetzt genau.
  await expect(page.getByTestId("library-audiobar-seek")).toHaveValue("6");
  await expect(page.getByTestId("library-follow-approx")).toHaveCount(0);
});

test("„Hören und mitlesen“ ohne Transkript sucht die gelesene Stelle im Hörbuch", async ({ page }) => {
  await mitlesenSetup(page, false);
  const { text, sync } = followText();
  // Im Hörbuch: 4 s Vorspann, dann Absatz k bei 4 s + k × 2 s (anteilig
  // geschätzt läge Absatz 7 woanders).
  await page.addInitScript(
    ([points, t, at]) => {
      const w = window as unknown as {
        __liveWindow: (a: { fromMs: number; lenMs: number }) => unknown;
      };
      w.__liveWindow = (a) => ({
        points: (points as { chars: number; ms: number }[]).filter(
          (p) => p.ms >= a.fromMs && p.ms < a.fromMs + a.lenMs,
        ),
        inserts: [],
        coverage: 0.9,
        next_hint: null,
        from_ms: a.fromMs,
        to_ms: Math.min(a.fromMs + a.lenMs, 30_000),
      });
      if (sessionStorage.getItem("seeded")) return;
      sessionStorage.setItem("seeded", "1");
      localStorage.setItem(
        "lva.ui.progress.text:page_m:original",
        JSON.stringify({
          offset: at,
          length: (t as string).length,
          heard: [],
          read: [],
          updated: Date.now() - 1000,
        }),
      );
    },
    [sync.map((p, k) => ({ chars: p.chars, ms: 4000 + k * 2000 })), text, sync[6].chars] as const,
  );
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.getByTestId("library-continue-follow").first().click();
  await expect(page.getByTestId("library-follow")).toBeVisible();
  // Absatz 7 beginnt im Hörbuch bei 16 s.
  await expect
    .poll(async () => Number(await page.getByTestId("library-audiobar-seek").inputValue()))
    .toBeGreaterThanOrEqual(16);
  expect(Number(await page.getByTestId("library-audiobar-seek").inputValue())).toBeLessThanOrEqual(17);
  await expect.poll(() => activeText(page)).toMatch(/^Absatz 7:/);
  const calls = await liveCalls(page, "audiosync_window");
  expect(calls[0].lenMs).toBe(15_000);
  expect(calls[0].hintChars).toBe(sync[6].chars);
});

test("Mitlesen ohne Erkennungsmodell bleibt bei der Schätzung", async ({ page }) => {
  await mitlesenSetup(page, false);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await openFollow(page);
  await expect(page.getByTestId("library-follow-approx")).toBeVisible();
  await seekTo(page, 6);
  // anteilig: 6 s von 30 s = 20 % des Texts.
  await expect.poll(() => activeText(page)).toMatch(/^Absatz 3:/);
});

test("Mitlesen im dunklen Erscheinungsbild (1440 px)", async ({ page }) => {
  await mitlesenSetup(page);
  await page.addInitScript(() => {
    (window as unknown as { __settings: { theme: string } }).__settings.theme = "dark";
  });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await openFollow(page);
  await seekTo(page, 17);
  await expect.poll(() => activeText(page)).toMatch(/^Absatz 7:/);
  await page.waitForTimeout(400);
  await page.screenshot({ path: shot("mitlesen-1440-dunkel.png") });
});

test("Mitlesen bei 390 px: Vollbild-Blatt mit bedienbarer Leiste, Trefferflächen ≥ 44 px", async ({
  page,
}) => {
  await mitlesenSetup(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  await openFollow(page);
  await seekTo(page, 17);
  await expect.poll(() => activeText(page)).toMatch(/^Absatz 7:/);
  // Das Blatt füllt das Fenster, die Hörleiste bleibt unten sichtbar.
  const sheet = (await page.getByTestId("library-player").boundingBox())!;
  expect(sheet.height).toBeGreaterThan(800);
  const bar = (await page.getByTestId("library-audiobar").boundingBox())!;
  expect(bar.y + bar.height).toBeLessThanOrEqual(844 + 1);
  for (const id of [
    "library-audiobar-toggle",
    "library-audiobar-back",
    "library-audiobar-forward",
    "library-audiobar-parts",
    "library-audiobar-follow",
    "library-audiobar-close",
    "library-follow-close",
  ]) {
    const box = (await page.getByTestId(id).boundingBox())!;
    expect(box.width, id).toBeGreaterThanOrEqual(44);
    expect(box.height, id).toBeGreaterThanOrEqual(44);
  }
  // Titel und Teil-Angabe: eigene Zeile, kaum gekürzt, „Teil 1 von 2“ in einer Zeile.
  const title = (await page.locator(".library-audiobar .library-title").boundingBox())!;
  expect(title.width).toBeGreaterThan(200);
  const sub = (await page.getByTestId("library-audiobar-sub").boundingBox())!;
  expect(sub.height).toBeLessThanOrEqual(20);
  expect(sub.width).toBeGreaterThan(60);
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  expect(overflow).toBeLessThanOrEqual(0);
  // Keine doppelten Scrollleisten: nur die Lesefläche scrollt im Blatt.
  const scrollers = await page.evaluate(() =>
    [...document.querySelectorAll(".library-player *")]
      .filter((el) => {
        const o = getComputedStyle(el).overflowY;
        return (o === "auto" || o === "scroll") && el.scrollHeight > el.clientHeight + 1;
      })
      .map((el) => el.className),
  );
  expect(scrollers).toEqual(["library-follow__text"]);
  await page.waitForTimeout(400);
  await page.screenshot({ path: shot("mitlesen-390.png") });
  // Schließen der Lesefläche: zurück zur Leiste.
  await page.getByTestId("library-follow-close").click();
  await expect(page.getByTestId("library-follow")).toHaveCount(0);
  expect(
    await page.evaluate(() => window.localStorage.getItem("lva.ui.library.followText")),
  ).toBe("off");
});

test("Mitlesen: Scrollen des Nutzers pausiert das Mitscrollen, danach „Zur aktuellen Stelle“", async ({
  page,
}) => {
  await mitlesenSetup(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await openFollow(page);
  const seek = page.getByTestId("library-audiobar-seek");
  // Ohne Eingriff scrollt die Lesefläche zum gehörten Absatz.
  await seekTo(page, 27);
  await expect.poll(() => activeText(page)).toMatch(/^Absatz 11:/);
  await expect(page.locator(".library-follow__p--now")).toBeInViewport({ ratio: 0.9 });
  // Der Nutzer scrollt nach oben: das Hören läuft an eine andere Stelle,
  // die Fläche folgt nicht.
  const text = page.getByTestId("library-follow-text");
  await text.evaluate((el) => (el.scrollTop = 0));
  await text.dispatchEvent("wheel", { deltaY: -200 });
  await seekTo(page, 5);
  await expect.poll(() => activeText(page)).toMatch(/^Absatz 3:/);
  await page.waitForTimeout(600);
  expect(await text.evaluate((el) => el.scrollTop)).toBe(0);
  await expect(page.getByTestId("library-follow-now")).toHaveCount(0);
  // Nach 5 s erscheint der Weg zurück.
  await expect(page.getByTestId("library-follow-now")).toBeVisible({ timeout: 8000 });
  await seekTo(page, 27);
  await expect.poll(() => activeText(page)).toMatch(/^Absatz 11:/);
  expect(await text.evaluate((el) => el.scrollTop)).toBe(0);
  await page.getByTestId("library-follow-now").dispatchEvent("click");
  await expect(page.locator(".library-follow__p--now")).toBeInViewport({ ratio: 0.9 });
  await expect(page.getByTestId("library-follow-now")).toHaveCount(0);
});

test("Mitlesen ohne Zeitachse (EPUB-Text + Hörbuch): anteilig und als „ungefähr“ gekennzeichnet", async ({
  page,
}) => {
  await mitlesenSetup(page, false);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await openFollow(page);
  await expect(page.getByTestId("library-follow-approx")).toBeVisible();
  const { text } = followText();
  // Absatz 7 beginnt bei etwa 55 % des Textes → etwa 16–17 s von 30 s.
  const start = text.indexOf("Absatz 7:");
  const expected = (start / text.length) * 30;
  await page.locator(".library-follow__p", { hasText: /^Absatz 7:/ }).dispatchEvent("click");
  await expect
    .poll(async () => Math.abs(Number(await page.getByTestId("library-audiobar-seek").inputValue()) - expected))
    .toBeLessThanOrEqual(1.5);
});

const progressAtStart = async (page: Page, audioAge: number, textAge: number) => {
  const { text, sync } = followText();
  await page.addInitScript(
    ([length, chars, audioAgeMs, textAgeMs]) => {
      const now = Date.now();
      window.localStorage.setItem(
        "lva.ui.progress.audio:book_m",
        JSON.stringify({
          offset: 4000,
          length: 30000,
          heard: [],
          read: [],
          updated: now - audioAgeMs,
          unit: "ms",
        }),
      );
      window.localStorage.setItem(
        "lva.ui.progress.text:page_m:original",
        JSON.stringify({
          offset: chars,
          length,
          heard: [],
          read: [],
          updated: now - textAgeMs,
          unit: "chars",
        }),
      );
    },
    [text.length, sync[8].chars, audioAge, textAge] as const,
  );
};

test("Start der Hörleiste: ist der Textfortschritt neuer, startet das Audio an der abgebildeten Stelle", async ({
  page,
}) => {
  await mitlesenSetup(page);
  await progressAtStart(page, 60_000, 0);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await openDetail(page);
  // Ohne Mitlesen starten: die gemeinsame Position gilt trotzdem.
  await page.getByTestId("library-detail-play").click();
  await expect(page.getByTestId("library-audiobar")).toBeVisible();
  // Absatz 9 beginnt bei 20 s.
  const seek = page.getByTestId("library-audiobar-seek");
  await expect
    .poll(async () => Number(await seek.inputValue()))
    .toBeGreaterThanOrEqual(19);
  expect(Number(await seek.inputValue())).toBeLessThanOrEqual(21);
});

test("Start der Hörleiste: ist das Hören neuer, bleibt es bei der Hörstelle", async ({ page }) => {
  await mitlesenSetup(page);
  await progressAtStart(page, 0, 60_000);
  await page.goto("/");
  await openDetail(page);
  await page.getByTestId("library-detail-play").click();
  const seek = page.getByTestId("library-audiobar-seek");
  await expect
    .poll(async () => Number(await seek.inputValue()))
    .toBeGreaterThanOrEqual(3);
  expect(Number(await seek.inputValue())).toBeLessThanOrEqual(6);
});

test("Transkription je Teil: sind alle fertig, entstehen Text und Zeitachse mit Versatz, Hilfsaufnahmen verschwinden", async ({
  page,
}) => {
  await addBooks(page, [
    baseBook("book_t", "Das Schloss", {
      authors: ["Franz Kafka"],
      ...multiAudio("book_t", [
        track(1, "Eins", 10_000, "m21"),
        track(2, "Zwei", 20_000, "m22"),
        track(3, "Drei", null, "m23"),
      ]),
    }),
  ]);
  await page.addInitScript(() => {
    const w = window as unknown as Record<string, any>;
    w.__jobs.push("m21", "m22", "m23");
    for (const id of ["m21", "m22", "m23"]) w.__jobStatus[id] = "ready";
    w.__jobDuration["m23"] = 15_000;
    const seg = (i: number, text: string, from: number, to: number) => ({
      segment_index: i,
      text,
      start_ms: from,
      end_ms: to,
      channel: 0,
      speaker_index: null,
    });
    w.__segments["m21"] = [seg(0, "Eins.", 0, 1000), seg(1, "Zwei.", 5000, 6000)];
    w.__segments["m22"] = [seg(0, "Drei.", 0, 1000)];
    w.__segments["m23"] = [seg(0, "Vier.", 2000, 3000)];
  });
  await page.goto("/");
  await expect.poll(async () => (await calls(page, "page_state_save")).length).toBe(1);
  const stored = JSON.parse((await calls(page, "page_state_save"))[0].state as string) as {
    text: string;
  };
  const prefix = "# Das Schloss\n\nvon Franz Kafka\n\n";
  expect(stored.text).toBe(`${prefix}Eins.\n\nZwei.\n\nDrei.\n\nVier.`);
  const saves = await calls(page, "library_save");
  const book = saves[saves.length - 1].book as {
    text: { origin: string };
    audio: {
      sync: { chars: number; ms: number }[];
      tracks: { transcript_job: string | null; duration_ms: number | null }[];
    };
  };
  expect(book.text.origin).toBe("transcript");
  // Versatz = Summe der vorigen Teildauern: 0 / 10 s / 30 s (+ 2 s im dritten Teil).
  expect(book.audio.sync.map((p) => p.ms)).toEqual([0, 5000, 10_000, 32_000]);
  expect(book.audio.sync[0].chars).toBe(prefix.length);
  expect(book.audio.tracks.map((t) => t.transcript_job)).toEqual([null, null, null]);
  // Fehlende Teildauer aus der Hilfsaufnahme übernommen.
  expect(book.audio.tracks[2].duration_ms).toBe(15_000);
  await expect
    .poll(async () => (await calls(page, "meetings_delete")).map((a) => a.meetingId).sort())
    .toEqual(["m21", "m22", "m23"]);
});


const transcriptBook = (jobOnFirst: string | null = null): Extra =>
  baseBook("book_m", "Der Prozess", {
    authors: ["Franz Kafka"],
    text: { page_id: "page_m", origin: "transcript", source_name: null },
    ...multiAudio(
      "book_m",
      [track(1, "Teil 1", 10_000, jobOnFirst), track(2, "Teil 2", 20_000)],
      followText().sync,
    ),
  });

test("Anhängen an ein Buch mit Transkript: nur die neuen Teile werden transkribiert", async ({
  page,
}) => {
  await addBooks(page, [transcriptBook()]);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  const card = page.getByTestId("library-card").filter({ hasText: "Der Prozess" });
  await dragTo(page, await centerOf(page, card.locator(".library-card__open")), [
    "C:/Downloads/Prozess - Teil 3.mp3",
  ]);
  await page.getByTestId("library-drop-append").click();
  await expect
    .poll(async () => (await calls(page, "meetings_import_audiobook_part")).map((a) => a.path))
    .toEqual(["C:/lib/book_m/audio/teil_003.mp3"]);
});

test("Ersetzen des Hörbuchs löscht Aufträge und die Transkript-Seite des alten Hörbuchs", async ({
  page,
}) => {
  await addBooks(page, [transcriptBook("m7")]);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  const card = page.getByTestId("library-card").filter({ hasText: "Der Prozess" });
  await dragTo(page, await centerOf(page, card.locator(".library-card__open")), [
    "C:/Downloads/Prozess - Teil 1.mp3",
    "C:/Downloads/Prozess - Teil 2.mp3",
  ]);
  await page.getByTestId("library-drop-attach").click();
  await expect
    .poll(async () => (await calls(page, "meetings_delete")).map((a) => a.meetingId))
    .toEqual(["m7"]);
  await expect
    .poll(async () => (await calls(page, "pages_delete")).map((a) => a.id))
    .toEqual(["page_m"]);
});

test("Fertiges Transkript angehängter Teile ergänzt Text und Zeitachse am Ende", async ({
  page,
}) => {
  const { text, sync } = followText();
  await addBooks(page, [
    baseBook("book_m", "Der Prozess", {
      authors: ["Franz Kafka"],
      text: { page_id: "page_m", origin: "transcript", source_name: null },
      ...multiAudio(
        "book_m",
        [
          track(1, "Teil 1", 10_000),
          track(2, "Teil 2", 20_000),
          track(3, "Teil 3", 15_000, "m31"),
        ],
        sync,
      ),
    }),
  ]);
  await page.addInitScript(
    ([t]) => {
      const w = window as unknown as Record<string, any>;
      w.__pageText["page_m"] = t;
      w.__jobs.push("m31");
      w.__jobStatus["m31"] = "ready";
      w.__segments["m31"] = [
        { segment_index: 0, text: "Neu.", start_ms: 2000, end_ms: 3000, channel: 0, speaker_index: null },
      ];
    },
    [text],
  );
  await page.goto("/");
  await expect.poll(async () => (await calls(page, "page_state_save")).length).toBe(1);
  const stored = JSON.parse((await calls(page, "page_state_save"))[0].state as string) as {
    text: string;
  };
  expect(stored.text).toBe(`${text}\n\nNeu.`);
  const saves = await calls(page, "library_save");
  const book = saves[saves.length - 1].book as {
    text: { page_id: string; origin: string };
    audio: {
      sync: { chars: number; ms: number }[];
      tracks: { transcript_job: string | null }[];
    };
  };
  expect(book.text.page_id).toBe("page_m");
  // Versatz = bisherige Gesamtdauer 30 s, dazu 2 s im neuen Teil.
  expect(book.audio.sync).toHaveLength(sync.length + 1);
  expect(book.audio.sync[sync.length]).toEqual({ chars: text.length + 2, ms: 32_000 });
  expect(book.audio.tracks.map((t) => t.transcript_job)).toEqual([null, null, null]);
  await expect
    .poll(async () => (await calls(page, "meetings_delete")).map((a) => a.meetingId))
    .toEqual(["m31"]);
});

// Kurzbuch (lva#205): Karte in der Übersicht des Buchs.
test("die Produktseite zeigt das Kurzbuch mit Dauer und Anhören",async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript(() => {
    const w = window as unknown as Record<string, unknown>;
    w.__kidocs = [
      {
        id: "01KURZBUCH000000000000000001",
        title: "Kurzbuch – Ganzes Buch",
        template_id: "builtin:kurzbuch",
        template_title: "Kurzbuch",
        source: { kind: "book" },
        source_label: "Ganzes Buch",
        created_ms: 1,
        modified_ms: 1,
        words: 400,
        model: "ollama/x",
      },
    ];
    w.__docText = [
      "## Worum es geht", "", "Eine Ehe und ihre Folgen.", "",
      "## Für wen", "", "- Leser von Klassikern", "- Schulen", "- Alle", "",
      "## Pflicht: Die Ehe folgt der Konvention.", "", "Text. ".repeat(150), "",
      "## Folgen: Ein Brieffund verändert alles.", "", "Text. ".repeat(150), "",
      "## Fazit", "", "Ende.",
    ].join("\n");
  });
  await page.goto("/");
  await page.getByRole("button", { name: "„Effi Briest“: Details öffnen" }).click();
  const card = page.getByRole("region", { name: "Kurzbuch" });
  await expect(card).toContainText("2 Kernaussagen");
  await expect(card.getByRole("button", { name: "Anhören" })).toBeVisible();
  await page.screenshot({ path: "docs/vorschau/buch-ki-b7/05-app-bibliothek.png" });
});

test("ohne Kurzbuch bietet die Produktseite das Erstellen an", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "„Effi Briest“: Details öffnen" }).click();
  const card = page.getByRole("region", { name: "Kurzbuch" });
  await expect(card.getByRole("button", { name: "Kurzbuch erstellen" })).toBeVisible();
  // lva#217: Sprache vorher wählbar, vorbelegt mit der App-Sprache.
  await expect(card.getByRole("combobox", { name: "Sprache des Kurzbuchs" })).toBeVisible();
  await expect(card).toContainText("Deutsch");
});

// lva#217: Kurzbücher in mehreren Sprachen je Buch.
test("die Produktseite zeigt Kurzbücher je Sprache und bietet weitere an", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript(() => {
    const w = window as unknown as Record<string, unknown>;
    const base = {
      template_id: "builtin:kurzbuch",
      template_title: "Kurzbuch",
      source: { kind: "book" },
      source_label: "Ganzes Buch",
      created_ms: 1,
      words: 400,
      model: "ollama/x",
    };
    w.__kidocs = [
      { ...base, id: "01KURZBUCHEN", title: "Kurzbuch – Ganzes Buch", modified_ms: 2, language: "en" },
      { ...base, id: "01KURZBUCHDE", title: "Kurzbuch – Ganzes Buch", modified_ms: 1, language: "de" },
    ];
    w.__docText = [
      "## Worum es geht", "", "Eine Ehe und ihre Folgen.", "",
      "## Für wen", "", "- Leser von Klassikern", "- Schulen", "- Alle", "",
      "## Pflicht: Die Ehe folgt der Konvention.", "", "Text. ".repeat(150), "",
      "## Folgen: Ein Brieffund verändert alles.", "", "Text. ".repeat(150), "",
      "## Fazit", "", "Ende.",
    ].join("\n");
  });
  await page.goto("/");
  await page.getByRole("button", { name: "„Effi Briest“: Details öffnen" }).click();
  const card = page.getByRole("region", { name: "Kurzbuch" });
  const langs = card.getByRole("group", { name: "Vorhandene Sprachen" });
  // Bevorzugt: die App-Sprache, auch wenn das englische neuer ist.
  await expect(langs.getByRole("button", { name: "Kurzbuch auf Deutsch zeigen" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(card.getByRole("heading", { name: /Kurzbuch/ })).toContainText("Deutsch");
  await langs.getByRole("button", { name: "Kurzbuch auf English zeigen" }).click();
  await expect(card.getByRole("heading", { name: /Kurzbuch/ })).toContainText("English");
  await card.getByRole("button", { name: "Weitere Sprache" }).click();
  await expect(card.getByRole("combobox", { name: "Sprache des Kurzbuchs" })).toBeVisible();
  await expect(card.getByRole("button", { name: "Erstellen", exact: true })).toBeVisible();
  // Vorbelegt mit einer Sprache, die es noch nicht gibt.
  await expect(card.locator(".library-kurzbuch__lang-select")).toContainText("Français");
  await page.screenshot({ path: "docs/vorschau/kurzbuch-sprache/04-bibliothek-1440.png" });
  await page.setViewportSize({ width: 390, height: 844 });
  await card.scrollIntoViewIfNeeded();
  // Mobil alle Ziele der Karte mindestens 44 × 44 px (Codex).
  const targets = [
    ...(await card.getByRole("button").all()),
    card.locator(".app-select__control"),
  ];
  expect(targets.length).toBeGreaterThan(5);
  for (const target of targets) {
    const box = await target.boundingBox();
    const name = await target.evaluate((e) => (e.textContent ?? "").trim().slice(0, 30));
    expect(box?.height ?? 0, `Höhe von „${name}“`).toBeGreaterThanOrEqual(44);
    expect(box?.width ?? 0, `Breite von „${name}“`).toBeGreaterThanOrEqual(44);
  }
  await page.screenshot({ path: "docs/vorschau/kurzbuch-sprache/05-bibliothek-390.png" });
});
// ---------------------------------------------------------------------------
// lva#208: lange Einzeldatei in Abschnitten, Zusammenführen, Fortsetzen
// ---------------------------------------------------------------------------

const MIN = 60_000;

const sectionPart = (
  job: string,
  start_ms: number,
  end_ms: number | null,
  from_ms: number,
  to_ms: number | null,
) => ({ job, start_ms, end_ms, from_ms, to_ms });

const sectioned = (
  id: string,
  title: string,
  parts: ReturnType<typeof sectionPart>[],
  durationMs = 200_000,
): Extra =>
  baseBook(id, title, {
    authors: ["Franz Kafka"],
    ...multiAudio(id, [
      { ...track(1, "Das Schloss", durationMs, null), transcript_jobs: parts } as ReturnType<
        typeof track
      >,
    ]),
  });

const segmentOf = (i: number, text: string, from: number, to: number) => ({
  segment_index: i,
  text,
  start_ms: from,
  end_ms: to,
  channel: 0,
  speaker_index: null,
});

test("Lange Einzeldatei: bis zu vier Abschnitte, ein Hörbuch-Auftrag je Bereich", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const w = window as unknown as Record<string, unknown>;
    w.__pick = "C:/hoerbuecher/Franz Kafka - Das Schloss.m4b";
    w.__audioDuration = 100 * 60_000;
  });
  await page.goto("/");
  await page.getByTestId("library-add").click();
  await page.getByTestId("library-add-pick").click();
  await page.getByTestId("library-add-submit").click();
  await expect
    .poll(async () => (await calls(page, "meetings_import_audiobook_part")).length)
    .toBe(4);
  const parts = await calls(page, "meetings_import_audiobook_part");
  const path = "C:/lib/x/audio/hoerbuch.mp3";
  expect(parts.every((p) => p.path === path)).toBe(true);
  // 100 min / 4 = 25 min je Abschnitt, 2 s Überlappung an inneren Grenzen.
  expect(parts.map((p) => p.startMs)).toEqual([0, 25 * MIN - 12_000, 50 * MIN - 12_000, 75 * MIN - 12_000]);
  expect(parts.map((p) => p.endMs)).toEqual([
    25 * MIN + 12_000,
    50 * MIN + 12_000,
    75 * MIN + 12_000,
    null,
  ]);
  // Der Kern kennt nur diese Art von Auftrag für Bibliotheks-Teile.
  expect(await calls(page, "meetings_import_file")).toHaveLength(0);
  await expect
    .poll(async () => {
      const saves = await calls(page, "library_save");
      const last = saves[saves.length - 1].book as {
        audio: { transcript_job: string | null; tracks?: { transcript_jobs?: unknown[] }[] };
      };
      return [last.audio.transcript_job, last.audio.tracks?.[0]?.transcript_jobs?.length];
    })
    .toEqual([null, 4]);
  // Das neue Buch öffnet sich; vier Abschnitte laufen gleichzeitig.
  await expect(page.getByText("Transkription 0 von 4 · 4 parallel")).toBeVisible();
});

test("Kurze Einzeldatei (30 min): ein Auftrag für die ganze Datei, kein Bereich", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const w = window as unknown as Record<string, unknown>;
    w.__pick = "C:/hoerbuecher/Franz Kafka - Das Urteil.m4b";
    w.__audioDuration = 30 * 60_000;
  });
  await page.goto("/");
  await page.getByTestId("library-add").click();
  await page.getByTestId("library-add-pick").click();
  await page.getByTestId("library-add-submit").click();
  await expect
    .poll(async () => (await calls(page, "meetings_import_audiobook_part")).length)
    .toBe(1);
  const [only] = await calls(page, "meetings_import_audiobook_part");
  expect(only.startMs).toBeNull();
  expect(only.endMs).toBeNull();
});

test("Abschnitte fertig: Text und Zeitachse ohne doppelten Satz an der Naht", async ({ page }) => {
  await addBooks(page, [
    sectioned("book_s", "Das Schloss", [
      sectionPart("m41", 0, 102_000, 0, 100_000),
      sectionPart("m42", 98_000, 260_000, 100_000, null),
    ]),
  ]);
  await page.addInitScript(() => {
    const w = window as unknown as Record<string, any>;
    w.__jobs.push("m41", "m42");
    for (const id of ["m41", "m42"]) w.__jobStatus[id] = "ready";
    const seg = (i: number, text: string, from: number, to: number) => ({
      segment_index: i,
      text,
      start_ms: from,
      end_ms: to,
      channel: 0,
      speaker_index: null,
    });
    w.__segments["m41"] = [
      seg(0, "Erster Satz.", 0, 10_000),
      seg(1, "Zweiter Satz vor der Grenze.", 90_000, 99_000),
      seg(2, "Dritter Satz an der Grenze und danach.", 99_000, 102_000),
    ];
    // Relativ zu 98 s: der dritte Satz steht in beiden Abschnitten.
    w.__segments["m42"] = [
      seg(0, "Dritter Satz an der Grenze und danach.", 1_000, 4_000),
      seg(1, "Vierter Satz.", 5_000, 15_000),
    ];
  });
  await page.goto("/");
  await expect.poll(async () => (await calls(page, "page_state_save")).length).toBe(1);
  const stored = JSON.parse((await calls(page, "page_state_save"))[0].state as string) as {
    text: string;
  };
  const prefix = "# Das Schloss\n\nvon Franz Kafka\n\n";
  expect(stored.text).toBe(
    `${prefix}Erster Satz.\n\nZweiter Satz vor der Grenze. Dritter Satz an der Grenze und danach. Vierter Satz.`,
  );
  const saves = await calls(page, "library_save");
  const book = saves[saves.length - 1].book as {
    text: { origin: string };
    audio: {
      sync: { ms: number }[];
      tracks: { transcript_job: string | null; transcript_jobs: unknown[] }[];
    };
  };
  expect(book.text.origin).toBe("transcript");
  expect(book.audio.sync.map((p) => p.ms)).toEqual([0, 90_000]);
  expect(book.audio.tracks[0].transcript_job).toBeNull();
  expect(book.audio.tracks[0].transcript_jobs).toEqual([]);
  await expect
    .poll(async () => (await calls(page, "meetings_delete")).map((a) => a.meetingId).sort())
    .toEqual(["m41", "m42"]);
});

test("In Arbeit: Stand „12 von 36“ nennt, wie viele parallel laufen", async ({ page }) => {
  const ids = Array.from({ length: 36 }, (_, i) => `m${100 + i}`);
  await addBooks(page, [
    baseBook("book_big", "Rich Dad Poor Dad", {
      authors: ["Robert Kiyosaki"],
      ...multiAudio(
        "book_big",
        ids.map((id, i) => track(i + 1, `Track ${i + 1}`, 60_000, id)),
      ),
    }),
  ]);
  await page.addInitScript((ids) => {
    const w = window as unknown as Record<string, any>;
    w.__jobs.push(...ids);
    ids.forEach((id, i) => {
      w.__jobStatus[id] = i < 12 ? "ready" : i < 16 ? "processing" : "queued";
    });
  }, ids);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  const card = page.getByTestId("library-card").filter({ hasText: "Rich Dad Poor Dad" });
  await expect(card).toContainText("Transkription 12 von 36 · 4 parallel");
});

test("Fortsetzen: nur der fehlgeschlagene Abschnitt wird neu gestartet", async ({ page }) => {
  const parts = [
    sectionPart("m51", 0, 62_000, 0, 60_000),
    sectionPart("m52", 58_000, 122_000, 60_000, 120_000),
    sectionPart("m53", 118_000, 182_000, 120_000, 180_000),
    sectionPart("m54", 178_000, 300_000, 180_000, null),
  ];
  await addBooks(page, [sectioned("book_r", "Das Schloss", parts, 240_000)]);
  await page.addInitScript(() => {
    const w = window as unknown as Record<string, any>;
    w.__jobs.push("m51", "m52", "m53", "m54");
    w.__jobStatus["m51"] = "ready";
    w.__jobStatus["m52"] = "failed";
    w.__jobStatus["m53"] = "queued";
    w.__jobStatus["m54"] = "ready";
  });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.getByRole("button", { name: "„Das Schloss“: Details öffnen" }).click();
  const detail = page.getByTestId("library-detail");
  await detail.getByRole("tab", { name: "Fassungen" }).click();
  await expect(detail).toContainText("Transkription 2 von 4 · 1 fehlgeschlagen");
  await detail.getByRole("button", { name: "Fehlende Teile transkribieren" }).click();
  await expect
    .poll(async () => (await calls(page, "meetings_import_audiobook_part")).length)
    .toBe(1);
  const [restarted] = await calls(page, "meetings_import_audiobook_part");
  expect(restarted).toEqual({
    path: "C:/lib/book_r/audio/teil_001.wav",
    startMs: 58_000,
    endMs: 122_000,
  });
  await expect
    .poll(async () => {
      const saves = await calls(page, "library_save");
      const last = saves[saves.length - 1]?.book as {
        audio: { tracks: { transcript_jobs: { job: string }[] }[] };
      };
      return last?.audio.tracks[0].transcript_jobs.map((p) => p.job);
    })
    .toEqual(["m51", "m2", "m53", "m54"]);
  await expect
    .poll(async () => (await calls(page, "meetings_delete")).map((a) => a.meetingId))
    .toEqual(["m52"]);
});

// ---------------------------------------------------------------------------
// Aufräumen (Patrick 02.11.): Entfernen, Zusammenführen von Hand, Titel
// ---------------------------------------------------------------------------

const partWithTranscript = (id: string, title: string, ms: number): Extra => ({
  ...baseBook(id, title, {
    authors: ["Robert Kiyosaki"],
    text: { page_id: `page_${id}`, origin: "transcript", source_name: null },
    audio: {
      file: "hoerbuch.mp3",
      duration_ms: ms,
      narrator: null,
      transcript_job: null,
      tracks: [],
      sync: [{ chars: title.length + 4, ms: 0 }],
    },
    audio_path: `C:/lib/${id}/audio/hoerbuch.mp3`,
  }),
});

test("Entfernen über ⋮: Papierkorb mit Umfang, Rückgängig im Hinweis holt den Eintrag zurück", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await expect(page.getByTestId("library-card")).toHaveCount(3);
  await page.getByTestId("library-more-book_2").click();
  await page.getByTestId("library-remove-book_2").click();
  const dialog = page.getByTestId("library-remove-dialog");
  await expect(dialog).toBeVisible();
  await expect(page.getByRole("dialog")).toContainText("„Der Schimmelreiter“ entfernen?");
  await expect(page.getByTestId("library-remove-summary")).toContainText("10 MB");
  await expect(page.getByTestId("library-remove-scope-trash")).toBeChecked();
  await expect(dialog).toContainText("30 Tage lang wiederherstellen");
  await expect(dialog).toContainText("Originaldateien");
  await expect(page.getByTestId("library-remove-confirm")).toHaveText("In den Papierkorb legen");
  await page.screenshot({ path: shot("entfernen-dialog-1440.png") });
  await page.getByTestId("library-remove-confirm").click();
  await expect(page.getByTestId("library-card")).toHaveCount(2);
  expect(await calls(page, "library_trash")).toEqual([{ ids: ["book_2"] }]);
  const toast = page.locator(".library-undo-toast");
  await expect(toast).toContainText("„Der Schimmelreiter“ liegt im Papierkorb.");
  await expect(toast).toContainText("Papierkorb");
  await expect(page.getByTestId("library-trash-open")).toContainText("Papierkorb (1)");
  await page.mouse.move(0, 0);
  await page.waitForTimeout(800);
  await page.screenshot({ path: shot("entfernen-rueckgaengig-1440.png") });
  await toast.getByRole("button", { name: "Rückgängig" }).click();
  await expect(page.getByTestId("library-card")).toHaveCount(3);
  expect(await calls(page, "library_restore")).toEqual([{ ids: ["book_2"] }]);
  await page.waitForTimeout(300);
  // Nichts wird endgültig gelöscht (nur das Aufräumen alter Reste ohne Kennungen).
  expect((await calls(page, "library_purge")).filter((a) => a.ids)).toEqual([]);
});

test("Endgültig löschen: eigene Bestätigung mit Zahl und Größe, sofort samt Text im Vorlesen", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.getByTestId("library-more-book_1").click();
  await page.getByTestId("library-remove-book_1").click();
  await page.getByTestId("library-remove-scope-delete").check();
  const confirm = page.getByTestId("library-remove-confirm");
  await expect(confirm).toHaveText("Endgültig löschen");
  await expect(confirm).toBeDisabled();
  await expect(page.getByTestId("library-remove-sure")).toBeVisible();
  await expect(page.getByTestId("library-remove-dialog")).toContainText("endgültig gelöscht");
  await page.getByTestId("library-remove-sure").check();
  await expect(confirm).toBeEnabled();
  await page.screenshot({ path: shot("endgueltig-loeschen-1440.png") });
  await confirm.click();
  await expect(page.getByTestId("library-card")).toHaveCount(2);
  await expect
    .poll(async () => await calls(page, "library_purge"))
    .toContainEqual({ ids: ["book_1"] });
  await expect
    .poll(async () => (await calls(page, "pages_delete")).map((a) => a.id))
    .toEqual(["page_1"]);
  await expect(page.getByText("„Effi Briest“ endgültig gelöscht.")).toBeVisible();
  // Kein Rückgängig bei endgültigem Löschen.
  await expect(page.locator(".library-undo-toast")).toHaveCount(0);
});

test("Papierkorb: Liste, Wiederherstellen und einzeln endgültig löschen", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.getByTestId("library-select-start").click();
  await page.locator('[data-drop-book="book_1"] [data-testid="library-select"]').check();
  await page.locator('[data-drop-book="book_2"] [data-testid="library-select"]').check();
  await page.getByTestId("library-select-remove").click();
  await expect(page.getByTestId("library-remove-confirm")).toHaveText("2 in den Papierkorb legen");
  await page.getByTestId("library-remove-confirm").click();
  await expect(page.getByTestId("library-card")).toHaveCount(1);
  await page.getByTestId("library-trash-open").click();
  const dialog = page.getByTestId("library-trash-dialog");
  await expect(page.getByRole("dialog")).toContainText("Papierkorb: 2 Einträge");
  await expect(page.getByTestId("library-trash-item")).toHaveCount(2);
  // Offener Papierkorb: kein Hinweis unten rechts darüber.
  await expect(page.locator("[data-sonner-toast]")).toHaveCount(0);
  await page.screenshot({ path: shot("papierkorb-1440.png") });
  await page.getByTestId("library-trash-item").filter({ hasText: "Effi Briest" })
    .getByTestId("library-trash-restore").click();
  await expect(page.getByTestId("library-card")).toHaveCount(2);
  expect(await calls(page, "library_restore")).toEqual([{ ids: ["book_1"] }]);
  await expect(page.getByTestId("library-trash-item")).toHaveCount(1);
  // Ergebnis steht im Dialog, nicht als Hinweis darüber.
  await expect(page.getByTestId("library-trash-notice")).toHaveText("„Effi Briest“ wiederhergestellt.");
  await page.getByTestId("library-trash-delete").click();
  await expect(dialog).toContainText("lässt sich nicht rückgängig machen");
  await page.mouse.move(0, 0);
  await page.waitForTimeout(400);
  await expect(page.locator("[data-sonner-toast]")).toHaveCount(0);
  await page.screenshot({ path: shot("papierkorb-loeschen-1440.png") });
  await page.getByTestId("library-trash-delete-confirm").click();
  await expect
    .poll(async () => await calls(page, "library_purge"))
    .toContainEqual({ ids: ["book_2"] });
  await expect(page.getByTestId("library-trash-none")).toBeVisible();
  await expect(page.getByTestId("library-trash-notice")).toHaveText("„Der Schimmelreiter“ endgültig gelöscht.");
  await expect(page.locator("[data-sonner-toast]")).toHaveCount(0);
});

test("Mehrfachauswahl: drei Einträge entfernen, Leiste zeigt die Zahl", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.getByTestId("library-select-start").click();
  const boxes = page.getByTestId("library-select");
  await expect(boxes).toHaveCount(3);
  await page.getByRole("button", { name: "Alle auswählen" }).click();
  await expect(page.getByTestId("library-selectbar")).toContainText("3 ausgewählt");
  await expect(page.getByTestId("library-select-merge")).toBeEnabled();
  await page.screenshot({ path: shot("auswahl-leiste-1440.png") });
  await page.getByTestId("library-select-remove").click();
  await expect(page.getByRole("dialog")).toContainText("3 Einträge entfernen?");
  await page.getByTestId("library-remove-scope-delete").check();
  await expect(page.getByTestId("library-remove-sure")).toContainText("3 Einträge (");
  await page.screenshot({ path: shot("endgueltig-mehrere-1440.png") });
  await page.getByTestId("library-remove-scope-trash").check();
  await page.getByTestId("library-remove-confirm").click();
  await expect(page.getByTestId("library-card")).toHaveCount(0);
  expect((await calls(page, "library_trash"))[0].ids).toHaveLength(3);
});

test("Zusammenführen von Hand: Teile mit Transkript werden ein Text mit Zeitachse", async ({
  page,
}) => {
  const titles = [2, 1].map(
    (n) => `Robert Kiyosaki - Rich Dad Poor Dad Disc 3 - ${pad(n)} - Track ${pad(n)}`,
  );
  await addBooks(page, [
    partWithTranscript("book_t2", titles[0], 60_000),
    partWithTranscript("book_t1", titles[1], 30_000),
  ]);
  await page.addInitScript((titles) => {
    const w = window as unknown as { __pageText: Record<string, string> };
    w.__pageText.page_book_t1 = `# ${titles[1]}\n\nAnfang des Buchs.`;
    w.__pageText.page_book_t2 = `# ${titles[0]}\n\nZweiter Teil.`;
  }, titles);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  // Der Vorschlag erscheint auch für Teile mit Transkript.
  await expect(page.getByTestId("library-merge-bar")).toContainText("Rich Dad Poor Dad");
  await page.getByTestId("library-select-start").click();
  for (const id of ["book_t1", "book_t2"])
    await page.locator(`[data-drop-book="${id}"] [data-testid="library-select"]`).check();
  await page.getByTestId("library-select-merge").click();
  const order = page.getByTestId("library-merge-item");
  await expect(order).toHaveCount(2);
  await expect(order.nth(0)).toContainText("Track 01");
  await expect(page.getByRole("dialog")).toContainText(
    "Zu einem Hörbuch zusammenführen: Rich Dad Poor Dad",
  );
  await expect(page.getByTestId("library-merge-transcripts")).toContainText(
    "zu einem Text in dieser Reihenfolge verbunden",
  );
  await page.screenshot({ path: shot("zusammenfuehren-hand-1440.png") });
  await page.getByTestId("library-merge-confirm").click();
  await expect
    .poll(async () => await calls(page, "library_merge_audio"))
    .toEqual([{ targetId: "book_t1", sourceIds: ["book_t2"], replace: false }]);
  // Gemeinsamer Text in der Seite des ersten Teils, Teil 2 mit Versatz 30 s.
  await expect
    .poll(async () =>
      (await calls(page, "page_state_save")).filter((a) => a.id === "page_book_t1").length,
    )
    .toBe(1);
  const saved = (await calls(page, "page_state_save")).find((a) => a.id === "page_book_t1")!;
  const text = JSON.parse(String(saved.state)).text as string;
  expect(text).toContain("## Teil 1\n\nAnfang des Buchs.\n\n## Teil 2\n\nZweiter Teil.");
  const last = (await calls(page, "library_save")).at(-1)!.book as {
    text: { page_id: string };
    audio: { sync: { chars: number; ms: number }[] };
  };
  expect(last.text.page_id).toBe("page_book_t1");
  expect(last.audio.sync.map((p) => p.ms)).toEqual([0, 30_000]);
  expect(last.audio.sync[1].chars).toBe(text.indexOf("Zweiter Teil."));
  // Die Seite des zweiten Teils ist im gemeinsamen Text aufgegangen.
  await expect
    .poll(async () => (await calls(page, "pages_delete")).map((a) => a.id))
    .toEqual(["page_book_t2"]);
});

test("Titel bereinigen: Hinweis, alt → neu, Speichern ohne ASIN und Format", async ({ page }) => {
  await addBooks(page, [
    baseBook("book_g", "Genesis B0DD7W1VFS LC 128 44100 Stereo", legacyAudio("book_g")),
    baseBook("book_i", "Alles Idioten! 3732455823 LC 128 44100 Stereo", legacyAudio("book_i")),
  ]);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  const bar = page.getByTestId("library-clean-bar");
  await expect(bar).toContainText("2 Titel enthalten Kennungen");
  await page.screenshot({ path: shot("titel-hinweis-1440.png") });
  await bar.getByTestId("library-clean-start").click();
  await expect(page.getByTestId("library-clean-new")).toHaveText(["Genesis", "Alles Idioten!"]);
  await page.screenshot({ path: shot("titel-dialog-1440.png") });
  await page.getByTestId("library-clean-confirm").click();
  await expect
    .poll(async () =>
      (await calls(page, "library_save")).map((a) => (a.book as { title: string }).title),
    )
    .toEqual(["Genesis", "Alles Idioten!"]);
  await expect(page.getByTestId("library-clean-bar")).toHaveCount(0);
});

test("Text und Hörbuch eines Titels: Zusammenführen hängt das Hörbuch ans Buch", async ({
  page,
}) => {
  await addBooks(page, [
    baseBook("book_a", "Effi Briest (Hörbuch)", { authors: ["Theodor Fontane"], ...legacyAudio("book_a") }),
  ]);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.getByTestId("library-select-start").click();
  await page.locator('[data-drop-book="book_1"] [data-testid="library-select"]').check();
  await page.locator('[data-drop-book="book_a"] [data-testid="library-select"]').check();
  await page.getByTestId("library-select-merge").click();
  await expect(page.getByTestId("library-merge-target")).toContainText("„Effi Briest“");
  await page.getByTestId("library-merge-confirm").click();
  await expect
    .poll(async () => await calls(page, "library_merge_audio"))
    .toEqual([{ targetId: "book_1", sourceIds: ["book_a"], replace: false }]);
});

test("390 px: Auswahl-Leiste und Rückfrage ohne waagerechte Scrollleiste, Ziele ≥ 44 px", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  await page.getByTestId("library-select-start").click();
  const box = await page.locator(".library-select").first().boundingBox();
  expect(box!.width).toBeGreaterThanOrEqual(44);
  expect(box!.height).toBeGreaterThanOrEqual(44);
  await page.locator(".library-select input").first().check();
  const selectbar: { ziel: string; text: string | null; breite: number; hoehe: number }[] = [];
  for (const el of await page.locator(".library-selectbar button").all()) {
    const r = (await el.boundingBox())!;
    expect(r.height).toBeGreaterThanOrEqual(44);
    selectbar.push({ ziel: "Auswahlleiste", text: (await el.innerText()).trim(), breite: Math.round(r.width), hoehe: Math.round(r.height) });
  }
  await page.screenshot({ path: shot("auswahl-390.png") });
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth > document.documentElement.clientWidth,
  );
  expect(overflow).toBe(false);
  await page.getByTestId("library-select-remove").click();
  await expect(page.getByTestId("library-remove-dialog")).toBeVisible();
  await page.getByTestId("library-remove-scope-delete").check();
  // Belege für die Abnahme: jede Trefferfläche mit Maßen, Schließen auch in der Breite.
  const measure = async (name: string, selector: string, wide = false) => {
    const all = await page.locator(selector).all();
    expect(all.length, name).toBeGreaterThan(0);
    const out = [];
    for (const el of all) {
      const r = (await el.boundingBox())!;
      const label = (await el.innerText()).split("\n")[0].trim() || (await el.getAttribute("aria-label"));
      expect(r.height, `${name}: ${label}`).toBeGreaterThanOrEqual(44);
      if (wide) expect(r.width, `${name}: ${label}`).toBeGreaterThanOrEqual(44);
      out.push({ ziel: name, text: label, breite: Math.round(r.width), hoehe: Math.round(r.height) });
    }
    return out;
  };
  const belege = [
    ...selectbar,
    ...(await measure("Schließen", ".lv-dialog.library-dialog-touch .fui-DialogTitle__action button", true)),
    ...(await measure("Option", ".library-remove__option")),
    ...(await measure("Dialogknopf", ".lv-dialog.library-dialog-touch .lv-dialog__actions button")),
  ];
  await page.screenshot({ path: shot("entfernen-dialog-390.png") });
  await page.getByTestId("library-remove-scope-trash").check();
  await page.getByTestId("library-remove-confirm").click();
  await page.getByTestId("library-trash-open").click();
  await expect(page.getByTestId("library-trash-item")).toHaveCount(1);
  await expect(page.locator("[data-sonner-toast]")).toHaveCount(0);
  belege.push(
    ...(await measure("Papierkorb-Schließen", ".lv-dialog.library-dialog-touch .fui-DialogTitle__action button", true)),
    ...(await measure("Papierkorb-Aktion", ".library-trash__actions button")),
    ...(await measure("Papierkorb-Dialogknopf", ".lv-dialog.library-dialog-touch .lv-dialog__actions button")),
  );
  await page.screenshot({ path: shot("papierkorb-390.png") });
  writeFileSync(shot("trefferflaechen-390.json"), JSON.stringify(belege, null, 2) + "\n");
});

test("Dunkel 1440: Patricks Fall mit Auswahl, Titelhinweis und Rückfrage", async ({ page }) => {
  await addBooks(page, [
    baseBook("book_g", "Genesis B0DD7W1VFS LC 128 44100 Stereo", legacyAudio("book_g")),
    ...[3, 2, 1].map((n) =>
      partWithTranscript(
        `book_r${n}`,
        `Robert Kiyosaki - Rich Dad Poor Dad Disc 3 - ${pad(n)} - Track ${pad(n)}`,
        60_000,
      ),
    ),
  ]);
  await page.addInitScript(() => {
    (window as unknown as { __settings: { theme: string } }).__settings.theme = "dark";
  });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await expect(page.getByTestId("library-clean-bar")).toBeVisible();
  await expect(page.getByTestId("library-merge-bar")).toBeVisible();
  await page.screenshot({ path: shot("hinweise-1440-dunkel.png") });
  await page.getByTestId("library-select-start").click();
  for (const n of [1, 2, 3])
    await page.locator(`[data-drop-book="book_r${n}"] [data-testid="library-select"]`).check();
  await page.screenshot({ path: shot("auswahl-1440-dunkel.png") });
  await page.getByTestId("library-select-merge").click();
  await expect(page.getByTestId("library-merge-item")).toHaveCount(3);
  await page.screenshot({ path: shot("zusammenfuehren-1440-dunkel.png") });
  await page.getByRole("dialog").getByRole("button", { name: "Abbrechen" }).last().click();
  await page.getByTestId("library-select-remove").click();
  await page.getByTestId("library-remove-scope-delete").check();
  await page.getByTestId("library-remove-sure").check();
  await page.screenshot({ path: shot("entfernen-dialog-1440-dunkel.png") });
  await page.getByTestId("library-remove-scope-trash").check();
  await page.getByTestId("library-remove-confirm").click();
  await page.getByTestId("library-trash-open").click();
  await expect(page.getByTestId("library-trash-item")).toHaveCount(3);
  await expect(page.locator("[data-sonner-toast]")).toHaveCount(0);
  await page.screenshot({ path: shot("papierkorb-1440-dunkel.png") });
});

// ---------------------------------------------------------------------------
// Eigene Cover und Titel bearbeiten
// ---------------------------------------------------------------------------

const VORSCHAU = "docs/vorschau/cover-titel";

/** Farbiges SVG als Ersatz für Cover aus dem Netz (Tests laufen offline). */
const svgCover = (label: string, hue: number) =>
  `<svg xmlns="http://www.w3.org/2000/svg" width="180" height="270" viewBox="0 0 180 270">` +
  `<defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="hsl(${hue} 55% 46%)"/>` +
  `<stop offset="1" stop-color="hsl(${(hue + 30) % 360} 60% 22%)"/></linearGradient></defs>` +
  `<rect width="180" height="270" fill="url(#g)"/><rect x="12" y="12" width="156" height="246" fill="none" stroke="#fff" stroke-opacity=".5"/>` +
  `<text x="90" y="120" fill="#fff" font-family="Georgia" font-size="19" font-weight="700" text-anchor="middle">${label}</text>` +
  `<text x="90" y="150" fill="#fff" font-family="Georgia" font-size="12" text-anchor="middle" opacity=".85">Robert T. Kiyosaki</text></svg>`;

/** Cover-Bilder: Open Library und lokale Dateien (asset://) über eine Testadresse. */
const serveCovers = async (page: Page) => {
  await page.addInitScript(() => {
    const internals = (window as unknown as { __TAURI_INTERNALS__: Record<string, unknown> })
      .__TAURI_INTERNALS__;
    internals.convertFileSrc = (path: string) =>
      `https://lva.test/asset/${encodeURIComponent(path)}`;
  });
  const hues: Record<string, number> = { "1": 205, "2": 28, "3": 350, "4": 140, "5": 265, "6": 48 };
  await page.route("https://covers.openlibrary.org/**", (route) => {
    const id = route.request().url().match(/\/id\/(\d+)/)?.[1] ?? "1";
    if (id === "999") return route.fulfill({ status: 404, body: "" });
    return route.fulfill({
      contentType: "image/svg+xml",
      body: svgCover(id === "5" ? "Reicher Vater" : "Rich Dad", hues[id.slice(-1)] ?? 200),
    });
  });
  await page.route("https://lva.test/asset/**", (route) =>
    route.fulfill({
      contentType: "image/svg+xml",
      body: route.request().url().includes("cover.jpg")
        ? svgCover("Rich Dad", 205)
        : svgCover("Hörbuch", 140),
    }),
  );
};

const RICH_DAD_CANDIDATES = [
  { source: "audio", cover_id: null, isbn: null, preview: "C:/lib/book_rd/vorschau-hoerbuch-1.jpg", label: "" },
  ...[1, 2, 3, 5, 6].map((id) => ({
    source: "openlibrary",
    cover_id: id,
    isbn: id === 1 ? "9781612680194" : null,
    preview: `https://covers.openlibrary.org/b/id/${id}-M.jpg`,
    label: ["Rich Dad Poor Dad · 1997 · Warner", "Rich Dad Poor Dad · 2017 · Plata", "Rich Dad Poor Dad · 2011", "Rich Dad Poor Dad (dt.) · 2014 · FinanzBuch", "Rich Dad Poor Dad · 2000"][[1, 2, 3, 5, 6].indexOf(id)],
  })),
  // Ein Cover, das nicht lädt, erscheint gar nicht erst.
  { source: "openlibrary", cover_id: 999, isbn: null, preview: "https://covers.openlibrary.org/b/id/999-M.jpg", label: "kaputt" },
];

const richDad = (title = "Rich Dad Poor Dad") =>
  baseBook("book_rd", title, {
    authors: ["Robert T. Kiyosaki"],
    ...legacyAudio("book_rd"),
  });

const withCandidates = (page: Page) =>
  page.addInitScript((c) => {
    (window as unknown as { __coverCandidates: unknown }).__coverCandidates = c;
  }, RICH_DAD_CANDIDATES);

test("Cover ändern: Hörbuch-Cover und mehrere Ausgaben zur Wahl, Übernehmen lädt das gewählte", async ({
  page,
}) => {
  await addBooks(page, [richDad()]);
  await withCandidates(page);
  await serveCovers(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.getByTestId("library-more-book_rd").click();
  await page.getByTestId("library-cover-book_rd").click();
  const dialog = page.getByTestId("library-cover-dialog");
  await expect(dialog).toBeVisible();
  // Aktuell + Hörbuch + 5 Ausgaben; das kaputte Bild fällt weg.
  await expect(dialog.getByTestId("library-cover-tile")).toHaveCount(7);
  await expect(dialog.locator('[data-cover="ol-999"]')).toHaveCount(0);
  await expect(dialog.locator('[data-cover="audio"]')).toContainText("Aus dem Hörbuch");
  await expect(dialog.getByRole("status")).toHaveText("6 Cover gefunden.");
  // Online gesucht wird beim Öffnen genau einmal (dazu einmal lokal).
  const asked = await calls(page, "library_cover_candidates");
  expect(asked.map((a) => a.online)).toEqual([false, true]);
  await expect(page.getByTestId("library-cover-apply")).toBeDisabled();
  await dialog.locator('[data-cover="ol-5"]').click();
  await expect(dialog.locator('[data-cover="ol-5"]')).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByTestId("library-cover-apply")).toBeEnabled();
  await page.waitForTimeout(400); // Übergang der Knopffarbe abwarten
  await page.screenshot({ path: `${VORSCHAU}/cover-dialog-1440.png` });
  await page.getByTestId("library-cover-apply").click();
  await expect(dialog).toHaveCount(0);
  await expect.poll(() => calls(page, "library_cover_fetch")).toEqual([
    { id: "book_rd", coverId: 5, isbn: null },
  ]);
  await expect(page.getByText("Cover geändert.")).toBeVisible();
});

test("Cover ändern: das Bild aus dem Hörbuch übernehmen, eigenes Bild von der Platte", async ({
  page,
}) => {
  await addBooks(page, [richDad()]);
  await withCandidates(page);
  await serveCovers(page);
  await page.goto("/");
  await page.getByTestId("library-more-book_rd").click();
  await page.getByTestId("library-cover-book_rd").click();
  const dialog = page.getByTestId("library-cover-dialog");
  await dialog.locator('[data-cover="audio"]').click();
  await page.getByTestId("library-cover-apply").click();
  await expect.poll(() => calls(page, "library_cover_from_audio")).toEqual([{ id: "book_rd" }]);
  // Eigenes Bild: Dateiauswahl, dann sofort übernommen.
  await page.evaluate(() => {
    (window as unknown as { __pick: string }).__pick = "C:/Bilder/rich-dad.jpg";
  });
  await page.getByTestId("library-more-book_rd").click();
  await page.getByTestId("library-cover-book_rd").click();
  // Jetzt hat das Buch ein Cover: „Generiert“ steht zur Wahl.
  await expect(page.locator('[data-cover="generated"]')).toBeVisible();
  await page.getByTestId("library-cover-file").click();
  await expect.poll(() => calls(page, "library_cover_from_file")).toEqual([
    { id: "book_rd", path: "C:/Bilder/rich-dad.jpg" },
  ]);
  await expect(page.getByTestId("library-cover-dialog")).toHaveCount(0);
});

test("Produktseite: Klick aufs Cover öffnet die Auswahl, im dunklen Erscheinungsbild", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as unknown as { __settings: { theme: string } }).__settings.theme = "dark";
  });
  await addBooks(page, [richDad()]);
  await withCandidates(page);
  await serveCovers(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.getByRole("button", { name: "„Rich Dad Poor Dad“: Details öffnen" }).click();
  await page.getByTestId("library-detail-cover").click();
  const dialog = page.getByTestId("library-cover-dialog");
  await expect(dialog.getByTestId("library-cover-tile")).toHaveCount(7);
  await dialog.locator('[data-cover="ol-1"]').click();
  await expect(page.getByTestId("library-cover-apply")).toBeEnabled();
  await page.waitForTimeout(400); // Übergang der Knopffarbe abwarten
  await page.screenshot({ path: `${VORSCHAU}/cover-dialog-1440-dunkel.png` });
  await page.getByTestId("library-cover-apply").click();
  await expect.poll(() => calls(page, "library_cover_fetch")).toEqual([
    { id: "book_rd", coverId: 1, isbn: "9781612680194" },
  ]);
  await page.screenshot({ path: `${VORSCHAU}/produktseite-mit-cover-1440-dunkel.png` });
});

test("Titel bearbeiten: Stift auf der Produktseite, Vorschlag ohne ASIN, Enter speichert", async ({
  page,
}) => {
  await addBooks(page, [richDad("Rich Dad Poor Dad B0DD7W1VFS LC 128 44100 Stereo")]);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page
    .getByRole("button", {
      name: "„Rich Dad Poor Dad B0DD7W1VFS LC 128 44100 Stereo“: Details öffnen",
    })
    .click();
  await page.getByTestId("library-title-edit").click();
  const input = page.getByTestId("library-title-input");
  await expect(input).toBeFocused();
  await expect(input).toHaveValue("Rich Dad Poor Dad B0DD7W1VFS LC 128 44100 Stereo");
  await expect(page.getByTestId("library-title-suggestion")).toContainText("Rich Dad Poor Dad");
  await page.screenshot({ path: `${VORSCHAU}/titel-bearbeiten-1440.png` });
  await page.getByTestId("library-title-suggestion").click();
  await expect(input).toHaveValue("Rich Dad Poor Dad");
  await expect(page.getByTestId("library-title-suggestion")).toHaveCount(0);
  await input.press("End");
  await input.type(" (Hörbuch)");
  await input.press("Enter");
  await expect
    .poll(async () => (await calls(page, "library_save")).map((a) => (a.book as { title: string }).title))
    .toEqual(["Rich Dad Poor Dad (Hörbuch)"]);
  await expect(page.getByTestId("library-title-editor")).toHaveCount(0);
  await expect(page.locator(".library-hero__title")).toHaveText("Rich Dad Poor Dad (Hörbuch)");
});

test("Titel bearbeiten über ⋮: Produktseite öffnet mit dem Titelfeld, Esc bricht ab", async ({
  page,
}) => {
  await addBooks(page, [richDad()]);
  await page.goto("/");
  await page.getByTestId("library-more-book_rd").click();
  await page.getByTestId("library-title-book_rd").click();
  const input = page.getByTestId("library-title-input");
  await expect(input).toBeFocused();
  // Unverändert: Speichern bleibt aus, kein Vorschlag.
  await expect(page.getByTestId("library-title-save")).toBeDisabled();
  await expect(page.getByTestId("library-title-suggestion")).toHaveCount(0);
  await input.press("Escape");
  await expect(page.getByTestId("library-title-editor")).toHaveCount(0);
  await expect(page.getByTestId("library-detail")).toBeVisible();
  expect(await calls(page, "library_save")).toEqual([]);
});

test("390 px: Cover-Auswahl und Titelfeld ohne waagerechte Scrollleiste, Ziele ≥ 44 px", async ({
  page,
}) => {
  await addBooks(page, [
    {
      ...richDad("Rich Dad Poor Dad B0DD7W1VFS LC 128 44100 Stereo"),
      cover_source: "openlibrary",
      cover_path: "C:/lib/book_rd/cover.jpg?v=1",
    },
  ]);
  await withCandidates(page);
  await serveCovers(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  await page.getByTestId("library-more-book_rd").click();
  await page.getByTestId("library-title-book_rd").click();
  const field = page.getByTestId("library-title-input");
  await expect(field).toBeVisible();
  // Mehrzeilig: der lange Rohtitel ist ganz sichtbar, nichts abgeschnitten.
  const fit = await field.evaluate((el: HTMLTextAreaElement) => ({
    full: el.scrollHeight <= el.clientHeight + 1 && el.scrollWidth <= el.clientWidth + 1,
    lines: el.clientHeight / parseFloat(getComputedStyle(el).lineHeight),
  }));
  expect(fit.full).toBe(true);
  expect(fit.lines).toBeGreaterThanOrEqual(1.9);
  await expect(page.getByTestId("library-title-suggestion")).toContainText(
    "Vorschlag übernehmen: Rich Dad Poor Dad",
  );
  for (const id of ["library-title-suggestion", "library-title-save"]) {
    const box = (await page.getByTestId(id).boundingBox())!;
    expect(box.height, id).toBeGreaterThanOrEqual(44);
  }
  const noScroll = () =>
    page.evaluate(() => {
      const els = [document.documentElement, ...document.querySelectorAll<HTMLElement>("main, [role=dialog]")];
      return els.every((e) => e.scrollWidth <= e.clientWidth + 1);
    });
  expect(await noScroll()).toBe(true);
  await page.screenshot({ path: `${VORSCHAU}/titel-bearbeiten-390.png` });
  await page.getByRole("button", { name: "Abbrechen" }).click();
  // Fokus kehrt auf den Stift zurück; auch er ist groß genug.
  await expect(page.getByTestId("library-title-edit")).toBeFocused();
  // Alle Knöpfe der Produktseite (Stift, Kurzbuch „Transkribieren“ …) ≥ 44 px hoch.
  for (const b of await page
    .locator(".library-hero__title-edit, .library-kurzbuch__actions button, .library-hero .library-actions button")
    .all()) {
    const box = (await b.boundingBox())!;
    expect(box.height, await b.innerText()).toBeGreaterThanOrEqual(44);
  }
  const pen = (await page.getByTestId("library-title-edit").boundingBox())!;
  expect(pen.width).toBeGreaterThanOrEqual(44);
  await page.getByTestId("library-detail-cover").click();
  const dialog = page.getByTestId("library-cover-dialog");
  // Aktuell + Generiert + Hörbuch + 5 Ausgaben.
  await expect(dialog.getByTestId("library-cover-tile")).toHaveCount(8);
  await expect(page.getByTestId("library-cover-restore")).toBeVisible();
  expect(await noScroll()).toBe(true);
  // Schließen-X ausdrücklich (Codex-Auflage): ≥ 44 × 44 px bei 390 px.
  const closeX = page.locator(".lv-dialog .fui-DialogTitle__action button");
  await expect(closeX).toHaveCount(1);
  const xBox = (await closeX.boundingBox())!;
  console.log(`Schließen-X bei 390 px: ${xBox.width} × ${xBox.height} px`);
  expect(xBox.width).toBeGreaterThanOrEqual(44);
  expect(xBox.height).toBeGreaterThanOrEqual(44);
  // Jeder Knopf im Dialog, auch Schließen-X, Übernehmen, Abbrechen, Eigenes Bild.
  const buttons = page.getByRole("dialog").getByRole("button");
  expect(await buttons.count()).toBeGreaterThanOrEqual(13);
  for (const b of await buttons.all()) {
    await b.scrollIntoViewIfNeeded();
    const box = (await b.boundingBox())!;
    const name = (await b.getAttribute("aria-label")) ?? (await b.innerText());
    expect(box.height, name).toBeGreaterThanOrEqual(44);
    expect(box.width, name).toBeGreaterThanOrEqual(44);
  }
  await expect(page.getByTestId("library-cover-consequence")).toContainText("Abbrechen");
  await dialog.locator('[data-cover="ol-2"]').click();
  await expect(page.getByTestId("library-cover-consequence")).toContainText(
    "ersetzt das bisherige Cover",
  );
  // Mobil stehen Titel, Jahr und Verlag vollständig da.
  await expect(dialog.locator('[data-cover="ol-5"] .library-meta')).toHaveText(
    "Rich Dad Poor Dad (dt.) · 2014 · FinanzBuch",
  );
  const clipped = await dialog
    .locator('[data-cover="ol-5"] .library-meta')
    .evaluate((el) => el.scrollHeight > el.clientHeight + 1);
  expect(clipped).toBe(false);
  await expect(page.getByTestId("library-cover-apply")).toBeEnabled();
  await page.locator(".library-cover-pick__grid").evaluate((el) => el.scrollTo(0, 0));
  await page.locator(".lv-dialog__content").evaluate((el) => el.scrollTo(0, 0));
  await page.waitForTimeout(400); // Übergang der Knopffarbe abwarten
  await page.screenshot({ path: `${VORSCHAU}/cover-dialog-390.png` });
});

test("Cover ändern: generiertes Cover wiederherstellen mit einem Klick", async ({ page }) => {
  await addBooks(page, [
    { ...richDad(), cover_source: "audio", cover_path: "C:/lib/book_rd/cover.jpg?v=1" },
  ]);
  await withCandidates(page);
  await serveCovers(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.getByTestId("library-more-book_rd").click();
  await page.getByTestId("library-cover-book_rd").click();
  const dialog = page.getByTestId("library-cover-dialog");
  await expect(dialog.locator('[data-cover="current"]')).toContainText("Aus dem Hörbuch");
  await expect(dialog.locator('[data-cover="generated"]')).toBeVisible();
  await expect(page.getByTestId("library-cover-consequence")).toBeVisible();
  await page.waitForTimeout(400);
  await page.screenshot({ path: `${VORSCHAU}/cover-dialog-wiederherstellen-1440.png` });
  await page.getByTestId("library-cover-restore").click();
  await expect.poll(() => calls(page, "library_cover_clear")).toEqual([{ id: "book_rd" }]);
  await expect(dialog).toHaveCount(0);
});

test("Neues Hörbuch ablegen: ein eingebettetes Cover wird gleich übernommen (lokal)", async ({
  page,
}) => {
  let net = 0;
  await page.route(/openlibrary\.org/, (route) => {
    net++;
    return route.abort();
  });
  await page.goto("/");
  const free = await centerOf(page, page.getByRole("heading", { name: /Alle Titel/ }));
  await emit(page, "tauri://drag-drop", {
    paths: ["C:/Hoerbuecher/Franz Kafka - Der Prozess.m4b"],
    position: free,
  });
  await expect.poll(async () => (await calls(page, "library_cover_from_audio")).length).toBe(1);
  // Ins Netz geht dabei nichts.
  expect(await calls(page, "library_cover_candidates")).toEqual([]);
  expect(await calls(page, "library_cover_fetch")).toEqual([]);
  expect(await calls(page, "library_metadata_search")).toEqual([]);
  expect(net).toBe(0);
});

test("Titel per Stift bei offenem Metadatenformular: Formular-Speichern setzt ihn nicht zurück", async ({
  page,
}) => {
  await addBooks(page, [richDad("Rich Dad Poor Dad B0DD7W1VFS")]);
  await page.goto("/");
  await page.getByRole("button", { name: "„Rich Dad Poor Dad B0DD7W1VFS“: Details öffnen" }).click();
  await page.getByRole("tab", { name: "Metadaten" }).click();
  await page.getByLabel("Verlag").fill("Plata");
  await page.getByTestId("library-title-edit").click();
  await page.getByTestId("library-title-input").fill("Rich Dad Poor Dad");
  await page.getByTestId("library-title-input").press("Enter");
  await expect(page.getByTestId("library-title-editor")).toHaveCount(0);
  await page.getByRole("button", { name: "Speichern", exact: true }).click();
  await expect
    .poll(async () => (await calls(page, "library_save")).map((a) => (a.book as { title: string; publisher: string | null }).title + "|" + (a.book as { publisher: string | null }).publisher))
    .toEqual(["Rich Dad Poor Dad|null", "Rich Dad Poor Dad|Plata"]);
});

test("Anhängen weiterer Teile holt ein bewusst entferntes Cover nicht zurück", async ({ page }) => {
  await addBooks(page, [richDad()]);
  await page.addInitScript(() => {
    (window as unknown as { __pick: string[] }).__pick = ["C:/Hoerbuecher/Teil 2.mp3"];
  });
  await page.goto("/");
  await page.getByRole("button", { name: "„Rich Dad Poor Dad“: Details öffnen" }).click();
  await page.getByRole("tab", { name: "Fassungen" }).click();
  await page.getByRole("button", { name: /Teile anhängen/ }).click();
  await expect.poll(async () => (await calls(page, "library_audio_import_tracks")).length).toBe(1);
  expect(await calls(page, "library_cover_from_audio")).toEqual([]);
});

test("Favoriten: Stern oben rechts (≥ 44 px) holt das Buch nach vorn und filtert", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  const cards = page.getByTestId("library-card");
  await expect(cards).toHaveCount(3);
  const titles = () =>
    cards.locator(".library-title").allInnerTexts();
  // Zuletzt gehört: Effi (40 %), dann nach Hinzufügen.
  expect(await titles()).toEqual(["Effi Briest", "Die Verwandlung", "Der Schimmelreiter"]);

  const star = page.getByTestId("library-star-book_2");
  await expect(star).toHaveAttribute("aria-pressed", "false");
  await expect(star).toHaveAccessibleName("Favorit: „Der Schimmelreiter“");
  const box = (await star.boundingBox())!;
  expect(box.width).toBeGreaterThanOrEqual(44);
  expect(box.height).toBeGreaterThanOrEqual(44);
  // Oben rechts auf dem Cover.
  const cover = (await cards
    .filter({ hasText: "Der Schimmelreiter" })
    .locator(".library-card__open")
    .boundingBox())!;
  expect(box.y - cover.y).toBeLessThan(8);
  expect(cover.x + cover.width - (box.x + box.width)).toBeLessThan(8);

  await star.click();
  await expect(page.getByTestId("library-star-book_2")).toHaveAttribute("aria-pressed", "true");
  const saved = (await calls(page, "library_save")).at(-1) as { book: { id: string; favorite: boolean } };
  expect(saved.book).toMatchObject({ id: "book_2", favorite: true });
  await expect.poll(titles).toEqual(["Der Schimmelreiter", "Effi Briest", "Die Verwandlung"]);
  await page.screenshot({ path: shot("bibliothek-favorit-390.png") });

  // Filter „Favoriten“ zeigt nur ihn.
  await page.getByTestId("library-filter-favorites").click();
  await expect(cards).toHaveCount(1);
  await expect(cards.first()).toContainText("Der Schimmelreiter");
  await expect(page.getByTestId("library-filter-favorites")).toContainText("1");

  // Wieder entfernen.
  await page.getByTestId("library-filter-all").click();
  await page.getByTestId("library-star-book_2").click();
  await expect(page.getByTestId("library-star-book_2")).toHaveAttribute("aria-pressed", "false");
  await expect.poll(titles).toEqual(["Effi Briest", "Die Verwandlung", "Der Schimmelreiter"]);
});

for (const [width, height, theme] of [
  [390, 844, "hell"],
  [390, 844, "dunkel"],
  [1440, 900, "hell"],
  [1440, 900, "dunkel"],
] as const) {
  test(`Stern und Auswahl ${width} ${theme}: ≥ 44 × 44 px, ohne Überlappung; Papierkorb mit Stern`, async ({
    page,
  }) => {
    if (theme === "dunkel")
      await page.addInitScript(() => {
        (window as unknown as { __settings: { theme: string } }).__settings.theme = "dark";
      });
    await page.setViewportSize({ width, height });
    await page.goto("/");
    await page.getByTestId("library-star-book_2").click();
    await expect(page.getByTestId("library-star-book_2")).toHaveAttribute("aria-pressed", "true");
    const card = page.locator('[data-drop-book="book_2"]');
    const r = (b: { x: number; y: number; width: number; height: number }) => ({
      x: Math.round(b.x),
      y: Math.round(b.y),
      breite: Math.round(b.width),
      hoehe: Math.round(b.height),
    });
    const overlap = (
      a: { x: number; y: number; width: number; height: number },
      b: { x: number; y: number; width: number; height: number },
    ) =>
      a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;
    const star = (await page.getByTestId("library-star-book_2").boundingBox())!;
    const play = await card.locator(".library-card__play").boundingBox();
    const cover = (await card.locator(".library-card__open").boundingBox())!;
    expect(star.width).toBeGreaterThanOrEqual(44);
    expect(star.height).toBeGreaterThanOrEqual(44);
    // Stern liegt auf dem Cover der Karte, nicht über dem Abspielknopf.
    expect(star.x).toBeGreaterThanOrEqual(cover.x - 1);
    expect(star.x + star.width).toBeLessThanOrEqual(cover.x + cover.width + 1);
    if (play) expect(overlap(star, play)).toBe(false);

    // Auswahlmodus: Checkbox ≥ 44 × 44, Stern und Abspielknopf ausgeblendet.
    await page.getByTestId("library-select-start").click();
    const check = card.locator(".library-select");
    const box = (await check.boundingBox())!;
    // Die Auswahlleiste verschiebt das Raster; verglichen wird relativ zum Cover.
    const coverSel = (await card.locator(".library-card__open").boundingBox())!;
    const rel = (
      b: { x: number; y: number; width: number; height: number },
      c: { x: number; y: number },
    ) => ({ x: b.x - c.x, y: b.y - c.y, width: b.width, height: b.height });
    const starRel = rel(star, cover);
    const boxRel = rel(box, coverSel);
    expect(box.width).toBeGreaterThanOrEqual(44);
    expect(box.height).toBeGreaterThanOrEqual(44);
    expect(boxRel.x).toBeGreaterThanOrEqual(-1);
    expect(boxRel.y).toBeGreaterThanOrEqual(-1);
    expect(boxRel.x + boxRel.width).toBeLessThanOrEqual(coverSel.width + 1);
    await expect(page.getByTestId("library-star-book_2")).toHaveCount(0);
    await expect(card.locator(".library-card__play")).toHaveCount(0);
    // Checkbox (links oben) und Stern (rechts oben) teilen sich keine Fläche.
    expect(overlap(boxRel, starRel)).toBe(false);
    if (play) expect(overlap(boxRel, rel(play, cover))).toBe(false);
    for (const other of await page.locator(".library-select").all()) {
      if ((await other.boundingBox())!.x === box.x && (await other.boundingBox())!.y === box.y) continue;
      expect(overlap(box, (await other.boundingBox())!)).toBe(false);
    }
    await check.locator("input").check();
    await page.screenshot({ path: shot(`auswahl-stern-${width}-${theme}.png`) });
    writeFileSync(
      shot(`stern-auswahl-${width}-${theme}.json`),
      JSON.stringify(
        {
          breite: width,
          theme,
          hinweis: "x/y relativ zur linken oberen Ecke des Covers",
          cover: { breite: Math.round(cover.width), hoehe: Math.round(cover.height) },
          stern: r(starRel),
          auswahl: r(boxRel),
          abspielen: play && r(rel(play, cover)),
        },
        null,
        2,
      ) + "\n",
    );

    // Papierkorb mit Stern: anderen Eintrag entfernen, Favorit bleibt mit Stern sichtbar.
    await page.getByTestId("library-select-end").click();
    await page.getByTestId("library-more-book_1").click();
    await page.getByTestId("library-remove-book_1").click();
    await page.getByTestId("library-remove-confirm").click();
    await expect(page.getByTestId("library-star-book_2")).toHaveAttribute("aria-pressed", "true");
    await page.getByTestId("library-trash-open").click();
    await expect(page.getByTestId("library-trash-item")).toHaveCount(1);
    await expect(page.locator("[data-sonner-toast]")).toHaveCount(0);
    await page.screenshot({ path: shot(`papierkorb-stern-${width}-${theme}.png`) });
  });
}

// Patrick 03.11.: Weitermachen kompakt, ganze Karte öffnet die Produktseite.
test("Weitermachen: kompakte Karte, Klick irgendwo auf die Karte öffnet die Produktseite", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  const card = page.getByTestId("library-continue-card").filter({ hasText: "Effi Briest" });
  await expect(card).toBeVisible();
  const box = (await card.boundingBox())!;
  expect(box.width).toBeLessThanOrEqual(420);
  await expect(card.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "40");
  await page.screenshot({ path: shot("bibliothek-weitermachen-1440.png") });
  // Unten links auf den Fortschritt, nicht auf den Titel: öffnet trotzdem.
  await card.click({ position: { x: box.width / 2, y: box.height - 14 } });
  await expect(
    page.getByTestId("library-detail").getByRole("heading", { name: "Effi Briest" }),
  ).toBeVisible();
});

test("Weitermachen 390: eine Spalte, keine waagerechte Scrollleiste, Lesen ≥ 44 px", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  const card = page.getByTestId("library-continue-card").first();
  await expect(card).toBeVisible();
  const quick = card.getByTestId("library-read-quick");
  const q = (await quick.boundingBox())!;
  expect(q.width).toBeGreaterThanOrEqual(44);
  expect(q.height).toBeGreaterThanOrEqual(44);
  expect(
    await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth),
  ).toBe(true);
});

const nav = (page: Page, name: string) =>
  page.getByRole("navigation").getByRole("button", { name, exact: true });

test("Hin und her zwischen Bibliothek und Vorlesen: kein Neuladen, Zustand bleibt, Buchtitel oben", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  const search = page.getByRole("searchbox").or(page.getByPlaceholder(/Titel, Autor/)).first();
  await search.fill("Effi");
  await expect(page.getByTestId("library-card")).toHaveCount(1);

  await nav(page, "Vorlesen").click();
  await expect(page.getByTestId("reader-book").or(page.locator(".tts-workspace")).first()).toBeVisible();
  // Geöffnetes Buch steht in der Titelleiste.
  const context = page.locator(".lv-titlebar").getByTestId("page-context");
  await expect(context).toContainText("Effi Briest");
  await expect(context).toHaveAttribute("aria-label", "Geöffnetes Buch: Effi Briest");
  await page.screenshot({ path: shot("vorlesen-buchtitel-1440.png") });
  // Merkmal am Lesetext: überlebt nur, wenn Vorlesen nicht neu eingehängt wird.
  await page.getByTestId("reader-book").evaluate((el) => el.setAttribute("data-probe", "1"));

  await nav(page, "Bibliothek").click();
  // Die Suche ist noch da, der Buchtitel des Vorlesens nicht mehr.
  await expect(search).toHaveValue("Effi");
  await expect(page.getByTestId("library-card")).toHaveCount(1);
  await expect(page.locator(".lv-titlebar").getByTestId("page-context")).toHaveCount(0);
  // Hinzufügen ist dezent: nur das Plus, der Name steht im Tooltip.
  const add = page.locator(".lv-titlebar").getByRole("button", { name: /Hinzufügen/ });
  await expect(add).toHaveCount(1);
  await expect(add).toHaveText("");

  await nav(page, "Vorlesen").click();
  await expect(context).toContainText("Effi Briest");
  // Nicht neu eingehängt: derselbe Lesetext, sofort sichtbar, nichts geleert.
  await expect(page.locator("[data-testid=reader-book][data-probe='1']")).toBeVisible({
    timeout: 1000,
  });
  // Die Werkzeuge der verdeckten Bibliothek bleiben aus der Titelleiste.
  await expect(
    page.locator(".lv-titlebar").getByRole("button", { name: /Hinzufügen/ }),
  ).toHaveCount(0);
});

test("Vorlesen bleibt erhalten und übernimmt trotzdem ein anderes Buch aus der Bibliothek", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await nav(page, "Vorlesen").click();
  const context = page.locator(".lv-titlebar").getByTestId("page-context");
  await expect(context).toContainText("Effi Briest");
  await nav(page, "Bibliothek").click();
  await page
    .getByTestId("library-card")
    .filter({ hasText: "Die Verwandlung" })
    .getByTestId("library-play")
    .click();
  await expect(context).toContainText("Die Verwandlung");
  await expect
    .poll(async () =>
      (await calls(page, "tts_speak_text")).length +
      (await calls(page, "tts_speak_resume")).length,
    )
    .toBeGreaterThan(0);
});

test("Profilmenü: kein Kontingent, Modelle führt direkt zur Verwaltung", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await oeffneProfilmenue(page);
  const menue = page.locator(".wai-konto-menue");
  await expect(menue.locator("[data-profilmenue=kontingent]")).toHaveCount(0);
  await expect(menue.getByText(/Kontingent/)).toHaveCount(0);
  await menue.getByRole("button", { name: "Modelle", exact: true }).click();
  await expect(menue).toHaveCount(0);
  await expect
    .poll(() => page.evaluate(() => window.localStorage.getItem("lva.ui.section")))
    .toBe("models");
});

// Belege für die Abnahme (Patrick 03.11.): Weitermachen mit drei Büchern,
// Buchtitel im Vorlesen, Profilmenü – 390/1440, hell/dunkel.
for (const [width, height] of [
  [1440, 900],
  [390, 844],
] as const) {
  for (const theme of ["hell", "dunkel"] as const) {
    test(`Belege Weitermachen und Buchtitel ${width} ${theme}`, async ({ page }) => {
      await page.addInitScript((dark) => {
        const w = window as unknown as { __settings: { theme: string } };
        if (dark) w.__settings.theme = "dark";
        const now = Date.now();
        window.localStorage.setItem(
          "lva.ui.progress.text:page_3:original",
          JSON.stringify({ offset: 120, length: 1000, heard: [[0, 120]], read: [], updated: now - 5000 }),
        );
        window.localStorage.setItem(
          "lva.ui.progress.audio:book_2",
          JSON.stringify({ offset: 1800000, length: 31260000, unit: "ms", heard: [[0, 1800000]], read: [], updated: now - 9000 }),
        );
      }, theme === "dunkel");
      await page.setViewportSize({ width, height });
      await page.goto("/");
      await expect(page.getByTestId("library-continue-card")).toHaveCount(3);
      await expect(page.getByTestId("library-continue-card").first()).toBeVisible();
      await page.screenshot({ path: shot(`vorschau-weitermachen-${width}-${theme}.png`) });
      if (width < 768) await page.locator(".lv-titlebar__hamburger").click();
      await nav(page, "Vorlesen").click();
      await expect(page.locator(".lv-titlebar").getByTestId("page-context")).toBeVisible();
      await page.screenshot({ path: shot(`vorschau-vorlesen-titel-${width}-${theme}.png`) });
      expect(
        await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth),
      ).toBe(true);
      await oeffneProfilmenue(page);
      await page.screenshot({ path: shot(`vorschau-profilmenue-${width}-${theme}.png`) });
    });
  }
}

// Codex-Auflagen (03.11.): Spielerleiste bei 390 ganz sichtbar, Tooltip des
// Plus, Touchziele ≥ 44 px mit Messwerten.
for (const theme of ["hell", "dunkel"] as const) {
  test(`390 ${theme}: Spielerleiste im Vorlesen vollständig über der Statusleiste`, async ({
    page,
  }) => {
    if (theme === "dunkel")
      await page.addInitScript(() => {
        (window as unknown as { __settings: { theme: string } }).__settings.theme = "dark";
      });
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto("/");
    await page.locator(".lv-titlebar__hamburger").click();
    await nav(page, "Vorlesen").click();
    const player = page.getByTestId("reader-player");
    await expect(player).toBeVisible();
    const status = (await page.locator(".lv-statusbar").boundingBox())!;
    const buttons = await player.locator("button:visible").all();
    expect(buttons.length).toBeGreaterThan(3);
    for (const b of buttons) {
      const box = (await b.boundingBox())!;
      expect(box.y + box.height, await b.getAttribute("aria-label")).toBeLessThanOrEqual(status.y + 0.5);
      expect(box.y).toBeGreaterThanOrEqual(0);
    }
    await page.screenshot({ path: shot(`vorschau-vorlesen-titel-390-${theme}.png`) });
    // Ganz nach unten gerollt: der Inhalt (Seitenfußzeile) steht vollständig
    // über der festen Spielerleiste (Codex 03.11.).
    await page.locator("#workspace-content").evaluate((el) => (el.scrollTop = el.scrollHeight));
    const footer = page.getByTestId("reader-book").getByText("Seite 1 von 1");
    await expect(footer).toBeVisible();
    const f = (await footer.boundingBox())!;
    const p = (await player.boundingBox())!;
    expect(f.y + f.height).toBeLessThanOrEqual(p.y);
    await page.screenshot({ path: shot(`vorschau-vorlesen-titel-390-${theme}-unten.png`) });
  });
}

test("Plus in der Titelleiste: Tooltip „Hinzufügen“ beim Überfahren", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  const add = page.getByTestId("library-add");
  await add.hover();
  await expect(page.getByRole("tooltip").filter({ hasText: "Hinzufügen" })).toBeVisible();
  await page.screenshot({ path: shot("vorschau-plus-tooltip-1440-hell.png") });
});

test("Touchziele ≥ 44 px: Plus, Weitermachen-Karte, Profilmenü (390), Messwerte", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  const messung: string[] = [];
  const pruefe = async (name: string, loc: import("@playwright/test").Locator) => {
    const b = (await loc.boundingBox())!;
    messung.push(`${name}: ${Math.round(b.width)} × ${Math.round(b.height)} px`);
    expect(b.width, name).toBeGreaterThanOrEqual(44);
    expect(b.height, name).toBeGreaterThanOrEqual(44);
  };
  await pruefe("Plus (Hinzufügen)", page.getByTestId("library-add"));
  const card = page.getByTestId("library-continue-card").first();
  await pruefe("Weitermachen-Karte", card);
  await pruefe("Weitermachen-Karte: Lesen/Abspielen", card.locator(".library-ccard__play"));
  await oeffneProfilmenue(page);
  for (const id of ["benachrichtigungen", "modelle", "integrationen", "einstellungen", "hilfe"])
    await pruefe(`Profilmenü ${id}`, page.locator(`.wai-konto-menue [data-profilmenue=${id}]`));
  writeFileSync(shot("touchziele-390.txt"), messung.join("\n") + "\n");
  console.log(messung.join("\n"));
});


test("Erhaltenes Vorlesen übernimmt still einen in der Bibliothek geänderten Text", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await nav(page, "Vorlesen").click();
  await expect(page.getByTestId("reader-book")).toContainText("In Front des Herrenhauses");
  await nav(page, "Bibliothek").click();
  await page.waitForTimeout(700);
  await page.evaluate(() => {
    (window as unknown as { __pageText: Record<string, string> }).__pageText.page_1 =
      "# Effi Briest\n\nZusammengeführt aus der Bibliothek.";
  });
  await nav(page, "Vorlesen").click();
  await expect(page.getByTestId("reader-book")).toContainText("Zusammengeführt aus der Bibliothek");
});
