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
          if (cmd === "library_progress_load") return {};
          if (cmd === "library_progress_put") return null;
          if (cmd === "page_files")
            return args?.id === "page_3"
              ? [{ name: "teil-01.mp3", size: 1, modified_ms: 0 }]
              : [];
          if (cmd === "meetings_list")
            return [{ id: "m1", title: "x", status: "processing" }];
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
            return {
              ...b,
              audio: { file: "hoerbuch.mp3", duration_ms: null, narrator: null, transcript_job: null },
              audio_path: "C:/lib/x/audio/hoerbuch.mp3",
            };
          }
          if (cmd === "meetings_import_file") return "m2";
          // Das Vorlesen nach „Lesen“/Play braucht seine Grunddaten.
          if (cmd === "tts_server_status") return { phase: "stopped", message: null };
          if (cmd === "pages_list")
            return [
              { id: "page_1", title: "Effi Briest" },
              { id: "page_3", title: "Die Verwandlung" },
            ];
          if (cmd === "page_state_load")
            return JSON.stringify({ text: "# Effi Briest\n\nIn Front des Herrenhauses.", tab: "original" });
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
  expect(await calls(page, "meetings_import_file")).toHaveLength(0);
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
    .poll(async () => (await calls(page, "meetings_import_file")).length)
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
  expect(await calls(page, "meetings_import_file")).toHaveLength(0);
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
  await expect(page.getByTestId("library-drop-attach")).toHaveText("Fassung ersetzen");
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
  await expect.poll(async () => (await calls(page, "meetings_import_file")).length).toBe(1);
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
});

test("Mehrere Dateien auf ein Buch: je Art nur die erste, Fehler halten die Warteschlange nicht an", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await dragTo(page, await centerOf(page, effiCover(page)), [
    "C:/x/kaputt.xyz",
    "C:/Downloads/teil1.mp3",
    "C:/Downloads/teil2.mp3",
  ]);
  // teil1 hängt an Effi; teil2 geht wie auf freie Fläche (unbekannt → neu).
  await expect
    .poll(async () => (await calls(page, "library_audio_import")).map((a) => a.id))
    .toEqual(["book_1", "book_new"]);
});
