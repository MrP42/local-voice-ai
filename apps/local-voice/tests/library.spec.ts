import { test, expect, type Page } from "@playwright/test";
import { join } from "node:path";

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
    expect(box?.height ?? 0).toBeGreaterThanOrEqual(44);
    expect(box?.width ?? 0).toBeGreaterThanOrEqual(44);
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
