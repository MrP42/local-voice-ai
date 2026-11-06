import { test, expect, type Page } from "@playwright/test";
import { join } from "node:path";
import { writeFileSync } from "node:fs";
import AxeBuilder from "@axe-core/playwright";
import {
  addDays,
  goalState,
  localDay,
  pagesOf,
  periodOf,
  summarize,
  toCsv,
  formatDuration,
} from "../src/lib/library/readingStats";

// Lesestatistik (lva#354): reine Logik (Zeiträume, Seiten, Ziel, CSV) und die
// Infoseite über die Bibliothek gegen die Tauri-Attrappe. Nur synthetische
// Daten. Belegbilder gehen nach LV_SHOT_DIR (sonst test-results).
const shot = (name: string) =>
  join(process.env.LV_SHOT_DIR ?? "test-results", name);

type Row = {
  day: string;
  book_id: string;
  format: string;
  lang: string;
  sec: number;
  chars: number;
  audio_ms: number;
};

const book = (id: string, over: Record<string, unknown> = {}) => ({
  id,
  title: id,
  subtitle: null,
  authors: ["Erika Beispiel"],
  language: "Deutsch",
  isbn: null,
  publisher: null,
  publish_date: null,
  pages: null,
  description: null,
  metadata_source: "manual",
  cover_source: "generated",
  text: null,
  audio: null,
  added_ms: 1,
  finished: false,
  cover_path: null,
  audio_path: null,
  ...over,
});

const noLen = () => ({ textChars: 0, audioMs: 0 });

// ---------------------------------------------------------------------------
// Logik
// ---------------------------------------------------------------------------

test.describe("Lesestatistik – Logik", () => {
  test("Zeiträume: Woche ab Montag, Monat, Jahr, Blättern", () => {
    // 06.11.2026 ist ein Freitag.
    expect(periodOf("week", "2026-11-06")).toEqual({
      kind: "week",
      from: "2026-11-02",
      to: "2026-11-08",
    });
    expect(periodOf("week", "2026-11-06", -1).from).toBe("2026-10-26");
    expect(periodOf("month", "2026-11-06")).toEqual({
      kind: "month",
      from: "2026-11-01",
      to: "2026-11-30",
    });
    expect(periodOf("month", "2026-01-15", -1).from).toBe("2025-12-01");
    expect(periodOf("month", "2028-02-10").to).toBe("2028-02-29");
    expect(periodOf("year", "2026-11-06", 1).from).toBe("2027-01-01");
    expect(addDays("2026-10-31", 1)).toBe("2026-11-01");
  });

  test("Seiten: Buchseiten, Normseiten, Hörbuch nur mit Seitenzahl", () => {
    const lesen = { format: "lesen", chars: 9000, audio_ms: 0 };
    expect(pagesOf(lesen, 300, { textChars: 900_000, audioMs: 0 })).toEqual({
      pages: 3,
      estimated: false,
    });
    expect(pagesOf(lesen, null, noLen())).toEqual({
      pages: 5,
      estimated: true,
    });
    const hoeren = { format: "hoeren", chars: 0, audio_ms: 3_600_000 };
    expect(
      pagesOf(hoeren, null, { textChars: 0, audioMs: 36_000_000 }),
    ).toEqual({ pages: 0, estimated: false });
    expect(
      pagesOf(hoeren, 300, { textChars: 0, audioMs: 36_000_000 }).pages,
    ).toBe(30);
  });

  test("Auswertung: Zeitraum, Kategorien, Sprachen, Belletristik-Anteil", () => {
    const books = [
      book("a", { category: "business", topics: ["Führung"] }),
      book("b", { category: "roman" }),
    ];
    const rows: Row[] = [
      {
        day: "2026-11-02",
        book_id: "a",
        format: "lesen",
        lang: "de",
        sec: 1800,
        chars: 3600,
        audio_ms: 0,
      },
      {
        day: "2026-11-03",
        book_id: "b",
        format: "hoeren",
        lang: "en",
        sec: 600,
        chars: 0,
        audio_ms: 600_000,
      },
      // Außerhalb des Zeitraums:
      {
        day: "2026-10-30",
        book_id: "a",
        format: "lesen",
        lang: "de",
        sec: 9999,
        chars: 0,
        audio_ms: 0,
      },
    ];
    const s = summarize(
      rows,
      books as never,
      periodOf("week", "2026-11-06"),
      noLen,
    );
    expect(s.sec).toBe(2400);
    expect(s.pages).toBe(2);
    expect(s.pagesEstimated).toBe(true);
    expect(s.byCategory.map((x) => x.key)).toEqual(["business", "roman"]);
    expect(s.byTopic).toEqual([{ key: "Führung", sec: 1800 }]);
    expect(s.byLang.map((x) => x.key)).toEqual(["de", "en"]);
    expect(s.byFormat.hoeren).toBe(600);
    expect(s.fictionShare).toBeCloseTo(0.25);
    expect(s.books.map((b) => b.bookId)).toEqual(["a", "b"]);
  });

  test("Ziel: Serie, heute offen bricht nicht, Wochentage ohne Ziel", () => {
    const today = "2026-11-06"; // Freitag
    const rows: Row[] = ["2026-11-03", "2026-11-04", "2026-11-05"].map(
      (day) => ({
        day,
        book_id: "a",
        format: "lesen",
        lang: "de",
        sec: 31 * 60,
        chars: 0,
        audio_ms: 0,
      }),
    );
    // Heute noch nichts gelesen: Serie bleibt 3.
    let g = goalState(rows, 30, 0x7f, today);
    expect(g.streak).toBe(3);
    expect(g.todayMin).toBe(0);
    // Heute erreicht: 4.
    rows.push({ ...rows[0], day: today });
    g = goalState(rows, 30, 0x7f, today);
    expect(g.streak).toBe(4);
    // Nur Mo–Fr, Montag 02.11. fehlt → Serie endet dort.
    g = goalState(rows, 30, 0b0011111, today, periodOf("week", today));
    expect(g.streak).toBe(4);
    expect(g.due).toBe(5);
    expect(g.hit).toBe(4);
    expect(g.avg7).toBeCloseTo((4 * 31) / 7);
    // Ziel aus: keine Serie.
    expect(goalState(rows, 0, 0x7f, today).streak).toBe(0);
  });

  test("CSV: Semikolon, Komma als Dezimaltrenner, Anführungszeichen", () => {
    const books = [book("a", { title: 'Das "kleine" Buch' })];
    const rows: Row[] = [
      {
        day: "2026-11-02",
        book_id: "a",
        format: "lesen",
        lang: "de",
        sec: 90,
        chars: 1800,
        audio_ms: 0,
      },
    ];
    const csv = toCsv(
      rows,
      books as never,
      periodOf("week", "2026-11-06"),
      noLen,
    );
    expect(csv).toBe(
      'Tag;Buch;Format;Sprache;Minuten;Seiten\r\n2026-11-02;"Das ""kleine"" Buch";lesen;de;1,5;1,0\r\n',
    );
    expect(formatDuration(42 * 60)).toBe("42 Min.");
    expect(formatDuration(412 * 60)).toBe("6:52 h");
  });
});

test.describe("Lesestatistik – Taktgeber", () => {
  test("Lesen ruht bei Lücke und Leerlauf, Wiedergabe zählt gedrosselt weiter", async () => {
    const clock = await import("../src/lib/library/readingClock");
    let t = 1_000_000;
    let shown = true;
    clock.__setClockForTests(
      () => t,
      () => shown,
    );
    const lesen = { bookId: "a", format: "lesen" as const, lang: "de" };
    clock.startReading(lesen);
    t += 5_000;
    clock.accrue();
    t += 20_000; // Lücke > 15 s beim Lesen: zählt nicht
    clock.accrue();
    shown = false;
    t += 5_000; // verborgen: Lesen zählt nicht
    clock.accrue();
    expect(clock.__pendingForTests().map((d) => d.sec)).toEqual([5]);
    // Wechsel auf Hören: Lesen endet, gedrosselte 60-s-Takte zählen.
    const hoeren = { bookId: "a", format: "hoeren" as const, lang: "de" };
    clock.startReading(hoeren);
    t += 60_000;
    clock.accrue();
    t += 600_000; // Schlaf: zählt nicht
    clock.accrue();
    const pending = clock.__pendingForTests();
    expect(pending.find((d) => d.format === "hoeren")?.sec).toBe(60);
    expect(pending.find((d) => d.format === "lesen")?.sec).toBe(5);
    // Eine pausierte andere Quelle beendet die laufende nicht.
    clock.stopReading({ bookId: "a", format: "vorlesen" });
    expect(clock.currentReading()?.format).toBe("hoeren");
  });

  test("CSV maskiert Formeln in Buchtiteln", () => {
    const rows: Row[] = [
      {
        day: "2026-11-02",
        book_id: "a",
        format: "lesen",
        lang: "de",
        sec: 60,
        chars: 0,
        audio_ms: 0,
      },
    ];
    const csv = toCsv(
      rows,
      [book("a", { title: "=SUMME(A1)" })] as never,
      periodOf("week", "2026-11-06"),
      noLen,
    );
    expect(csv).toContain(`"'=SUMME(A1)"`);
  });
});

// ---------------------------------------------------------------------------
// Infoseite gegen die Tauri-Attrappe
// ---------------------------------------------------------------------------

const UI_BOOKS = [
  book("book_1", {
    title: "Führen mit Klarheit",
    authors: ["Erika Beispiel"],
    pages: 240,
    category: "business",
    topics: ["Führung", "Kommunikation"],
    classified_by: "hand",
    text: {
      page_id: "page_1",
      origin: "document",
      source_name: "fuehren.epub",
    },
  }),
  book("book_2", {
    title: "Die freie Rede",
    authors: ["Max Muster"],
    category: "kommunikation",
    topics: ["Rhetorik"],
    classified_by: "ki",
    audio: {
      file: "rede.m4b",
      duration_ms: 28_800_000,
      narrator: null,
      transcript_job: null,
    },
  }),
  book("book_3", {
    title: "The Quiet Garden",
    authors: ["Jane Example"],
    language: "English",
    category: "roman",
    text: { page_id: "page_3", origin: "document", source_name: "garden.epub" },
  }),
  book("book_4", { title: "Ungeordnetes Buch", authors: ["Kim Probe"] }),
];

/** 40 Tage synthetischer Verlauf bis heute; heute 18 von 30 Minuten. */
function uiRows(today: string): Row[] {
  const out: Row[] = [];
  for (let i = 0; i < 40; i++) {
    const day = addDays(today, -i);
    if (i % 6 === 5) continue; // Pausentage
    const base = i === 0 ? 18 : 20 + ((i * 7) % 25);
    out.push({
      day,
      book_id: "book_1",
      format: "lesen",
      lang: "de",
      sec: base * 40,
      chars: base * 900,
      audio_ms: 0,
    });
    out.push({
      day,
      book_id: "book_2",
      format: "hoeren",
      lang: "de",
      sec: base * 20,
      chars: 0,
      audio_ms: base * 20_000,
    });
    if (i % 3 === 0 && i > 0)
      out.push({
        day,
        book_id: "book_3",
        format: "vorlesen",
        lang: "en",
        sec: 600,
        chars: 6000,
        audio_ms: 0,
      });
  }
  return out;
}

async function mock(
  page: Page,
  opts: { rows: Row[]; theme?: string; paused?: boolean; goal?: number },
) {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(
    ({ books, rows, theme, paused, goal }) => {
      const callbacks = new Map<number, unknown>();
      let callback = 0;
      const settings: Record<string, unknown> = {
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
        tts_engine: "piper",
        reading_goal_minutes: goal,
        reading_goal_days: 0x7f,
        reading_stats_sync: false,
        reading_stats_ai: true,
        reading_stats_paused: paused,
      };
      const w = window as unknown as Record<string, unknown>;
      w.__calls = [] as unknown[];
      w.__rows = rows;
      window.localStorage.setItem("lva.ui.section", "library");
      window.localStorage.setItem("lva.ui.readingStats.syncAsked", "1");
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
            if (cmd === "get_app_settings" || cmd === "get_default_settings")
              return JSON.parse(JSON.stringify(settings));
            if (cmd === "change_reading_stats_setting") {
              settings.reading_goal_minutes = args?.goalMinutes;
              settings.reading_goal_days = args?.goalDays;
              settings.reading_stats_paused = args?.paused;
              return null;
            }
            if (cmd === "plugin:os|locale") return "de-DE";
            if (cmd === "plugin:app|version") return "0.23.0";
            if (cmd.includes("permission")) return true;
            if (cmd === "plugin:event|listen") return Number(args?.handler);
            if (cmd === "plugin:event|unlisten") return null;
            if (cmd === "library_list") return books;
            if (cmd === "library_save") return args?.book;
            if (cmd === "library_progress_load") return {};
            if (cmd === "reading_log_rows") return w.__rows;
            if (cmd === "reading_log_add") return 0;
            if (cmd === "reading_log_clear") {
              w.__rows = [];
              return null;
            }
            if (cmd === "sync_status")
              return { connected: false, last_sync_ms: null, error: null };
            if (cmd === "tts_server_status")
              return { phase: "stopped", message: null };
            if (cmd === "get_custom_sounds")
              return { start: false, stop: false };
            if (cmd === "meetings_is_recording") return false;
            if (cmd === "get_selected_model") return "";
            if (
              cmd.endsWith("_list") ||
              cmd.startsWith("tts_list") ||
              cmd === "llm_ps"
            )
              return [];
            if (
              cmd.includes("models") ||
              cmd.includes("history") ||
              cmd.includes("devices")
            )
              return [];
            return null;
          },
        },
      });
    },
    {
      books: UI_BOOKS,
      rows: opts.rows,
      theme: opts.theme ?? "light",
      paused: opts.paused ?? false,
      goal: opts.goal ?? 30,
    },
  );
}

async function openStats(page: Page) {
  await page.goto("/");
  const knob = page.getByTestId("reading-stats-open");
  await expect(knob).toBeVisible();
  await knob.click();
  await expect(page.getByTestId("reading-stats")).toBeVisible();
}

const today = localDay();

test.describe("Lesestatistik – Infoseite", () => {
  test("Ring-Knopf nennt den Zielstand und öffnet die Seite", async ({
    page,
  }) => {
    await mock(page, { rows: uiRows(today) });
    await page.goto("/");
    const knob = page.getByTestId("reading-stats-open");
    await expect(knob).toHaveAttribute("aria-label", /18 von 30 Minuten/);
    const box = await knob.boundingBox();
    expect(box!.height).toBeGreaterThanOrEqual(44);
    await knob.click();
    await expect(page.getByTestId("reading-stats")).toBeVisible();
    await page.getByTestId("reading-stats-back").click();
    await expect(page.getByTestId("reading-stats")).toHaveCount(0);
  });

  test("Zeiträume, Tabelle zum Diagramm, Bücher", async ({ page }) => {
    await mock(page, { rows: uiRows(today) });
    await openStats(page);
    await expect(page.getByText("Führen mit Klarheit").first()).toBeVisible();
    await expect(page.getByText("The Quiet Garden").first()).toBeVisible();
    const label = page.getByTestId("reading-period-label");
    await page.getByTestId("reading-period-month").click();
    const month = await label.textContent();
    await page
      .getByRole("button", { name: "Vorheriger Zeitraum" })
      .first()
      .click();
    await expect(label).not.toHaveText(month ?? "");
    await page.getByTestId("reading-period-year").click();
    await page.getByRole("button", { name: /Als Tabelle/i }).click();
    await expect(page.getByTestId("reading-history-table")).toBeVisible();
  });

  test("Leseziel ändern: Dialog mit Tastatur, schließt mit Escape", async ({
    page,
  }) => {
    await mock(page, { rows: uiRows(today) });
    await openStats(page);
    await page.getByTestId("reading-goal-open").click();
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
    await page.getByTestId("reading-goal-open").click();
    await page.getByTestId("reading-goal-45").click();
    await page.getByTestId("reading-goal-save").click();
    await expect
      .poll(async () =>
        page.evaluate(() =>
          (
            window as unknown as {
              __calls: [string, Record<string, unknown>][];
            }
          ).__calls
            .filter(([c]) => c === "change_reading_stats_setting")
            .map(([, a]) => a.goalMinutes),
        ),
      )
      .toContain(45);
  });

  test("Leerzustand: Export und Löschen deaktiviert", async ({ page }) => {
    await mock(page, { rows: [] });
    await openStats(page);
    await expect(page.getByTestId("reading-stats-empty")).toBeVisible();
    await expect(page.getByTestId("reading-stats-clear")).toBeDisabled();
  });

  test("Pause wird erklärt", async ({ page }) => {
    await mock(page, { rows: uiRows(today), paused: true });
    await openStats(page);
    await expect(page.getByTestId("reading-stats-paused")).toBeVisible();
  });

  test("Löschen fragt nach dem Geltungsbereich", async ({ page }) => {
    await mock(page, { rows: uiRows(today) });
    await openStats(page);
    await page.getByTestId("reading-stats-clear").click();
    const dialog = page.getByRole("dialog");
    await expect(dialog).toContainText(/auf allen deinen Geräten/);
    await page.getByTestId("reading-stats-clear-confirm").click();
    await expect(page.getByTestId("reading-stats-empty")).toBeVisible();
  });

  test("Barrierefreiheit: keine axe-Verstöße (Kontrast, Namen)", async ({
    page,
  }) => {
    await mock(page, { rows: uiRows(today) });
    await openStats(page);
    const res = await new AxeBuilder({ page })
      .include('[data-testid="reading-stats"]')
      .analyze();
    expect(res.violations.map((v) => `${v.id}: ${v.nodes.length}`)).toEqual([]);
  });

  test("Buchansicht: KI-Vorschlag sichtbar und übernehmbar", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 1000 });
    await mock(page, { rows: uiRows(today) });
    await page.goto("/");
    await page.getByText("Die freie Rede").first().click();
    const box = page.getByTestId("book-classification");
    await box.scrollIntoViewIfNeeded();
    await expect(box).toContainText("Kommunikation & Rhetorik");
    await expect(page.getByTestId("book-classification-ai")).toBeVisible();
    await box.screenshot({ path: shot("rs-app-buch-1440-hell.png") });
    await page.getByTestId("book-classification-accept").click();
    await expect
      .poll(async () =>
        page.evaluate(() =>
          (
            window as unknown as {
              __calls: [string, Record<string, unknown>][];
            }
          ).__calls
            .filter(([c]) => c === "library_save")
            .map(
              ([, a]) => (a.book as { classified_by: string }).classified_by,
            ),
        ),
      )
      .toContain("hand");
  });

  test("Diagramm: Beschriftungen 12 px groß, Tabelle per Tastatur hin und zurück", async ({
    page,
  }) => {
    for (const width of [1440, 390]) {
      await page.setViewportSize({ width, height: width > 600 ? 1000 : 844 });
      await mock(page, { rows: uiRows(today) });
      await openStats(page);
      // Gerenderte Höhe der Achsentexte (nicht nur CSS-Wert): ≥ 12 px.
      const heights = await page
        .locator(".rs-chart text")
        .evaluateAll((els) =>
          els.map(
            (e) => (e as SVGGraphicsElement).getBoundingClientRect().height,
          ),
        );
      expect(heights.length).toBeGreaterThan(5);
      expect(Math.min(...heights)).toBeGreaterThanOrEqual(12);
      const size = await page
        .locator(".rs-axis")
        .first()
        .evaluate((e) => getComputedStyle(e).fontSize);
      expect(size).toBe("12px");
      // Nur Tastatur: Tab bis zum Umschalter, Enter, Tabelle da, Enter zurück.
      const toggle = page.getByRole("button", { name: "Als Tabelle zeigen" });
      let reached = false;
      for (let i = 0; i < 40 && !reached; i++) {
        await page.keyboard.press("Tab");
        reached = await toggle.evaluate((e) => e === document.activeElement);
      }
      expect(reached).toBe(true);
      await page.keyboard.press("Enter");
      await expect(toggle).toHaveAttribute("aria-pressed", "true");
      await expect(page.getByTestId("reading-history-table")).toBeVisible();
      await page.keyboard.press("Enter");
      await expect(page.getByTestId("reading-history-table")).toHaveCount(0);
      await expect(page.locator(".rs-chart")).toBeVisible();
    }
  });

  for (const width of [1440, 390]) {
    test(`Trefferflächen ≥ 44 px (${width})`, async ({ page }) => {
      await page.setViewportSize({ width, height: width > 600 ? 1000 : 844 });
      await mock(page, { rows: uiRows(today) });
      await page.goto("/");
      const ring = page.getByTestId("reading-stats-open");
      await page
        .locator(".library-bar")
        .first()
        .screenshot({ path: shot(`rs-app-ring-${width}.png`) });
      const boxes: Record<string, { w: number; h: number }> = {};
      const measure = async (
        name: string,
        loc: import("@playwright/test").Locator,
      ) => {
        await loc.scrollIntoViewIfNeeded();
        const b = (await loc.boundingBox())!;
        boxes[name] = { w: Math.round(b.width), h: Math.round(b.height) };
      };
      await measure("Ring", ring);
      await ring.click();
      await measure("Zurück", page.getByTestId("reading-stats-back"));
      await measure("Ziel ändern", page.getByTestId("reading-goal-open"));
      await measure("Woche", page.getByTestId("reading-period-week"));
      await measure(
        "Vorheriger",
        page.getByRole("button", { name: "Vorheriger Zeitraum" }),
      );
      await measure(
        "Nächster",
        page.getByRole("button", { name: "Nächster Zeitraum" }),
      );
      await measure(
        "Tabelle",
        page.getByRole("button", { name: "Als Tabelle zeigen" }),
      );
      await measure("Themenchip", page.locator(".rs-topic").first());
      await measure(
        "Export",
        page.getByRole("button", { name: /Exportieren/ }),
      );
      await measure("Pause", page.getByTestId("reading-stats-pause"));
      await measure("Löschen", page.getByTestId("reading-stats-clear"));
      await measure("Schalter Sync", page.locator(".rs-switch").first());
      console.log(`Trefferflächen ${width}: ${JSON.stringify(boxes)}`);
      writeFileSync(
        shot(`rs-trefferflaechen-${width}.json`),
        JSON.stringify(boxes, null, 2),
      );
      // Touch (390 px): 44 px (Codex A4). Maus (1440 px): 24 px (WCAG 2.5.8).
      const min = width <= 640 ? 44 : 24;
      for (const [name, b] of Object.entries(boxes)) {
        expect.soft(b.h, `${name} Höhe`).toBeGreaterThanOrEqual(min);
        expect.soft(b.w, `${name} Breite`).toBeGreaterThanOrEqual(min);
      }
    });
  }

  test("Wertetabelle mobil erreichbar, Leerzeitraum, Grabstein-Löschen", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await mock(page, { rows: uiRows(today) });
    await openStats(page);
    const toggle = page.getByRole("button", { name: "Als Tabelle zeigen" });
    await toggle.focus();
    await page.keyboard.press("Enter");
    await expect(page.getByTestId("reading-history-table")).toBeVisible();
    await page
      .getByTestId("reading-history-table")
      .screenshot({ path: shot("rs-app-tabelle-390.png") });
    // Leerer Zeitraum: zwei Jahre zurück.
    await page.getByTestId("reading-period-year").click();
    const prev = page.getByRole("button", { name: "Vorheriger Zeitraum" });
    await prev.click();
    await prev.click();
    await expect(page.getByTestId("reading-stats-period-empty")).toBeVisible();
    await page.screenshot({ path: shot("rs-app-leerzeitraum-390.png") });
    // Löschen geht an den Kern (der setzt Grabsteine, siehe
    // grabstein_loescht_auf_allen_geraeten_und_neue_epoche_zaehlt), mit Zeitraum oder alles.
    await page.getByTestId("reading-stats-clear").click();
    await page.screenshot({ path: shot("rs-app-loeschen-390.png") });
    await page.getByTestId("reading-stats-clear-confirm").click();
    await expect
      .poll(async () =>
        page.evaluate(() =>
          (window as unknown as { __calls: [string, unknown][] }).__calls.some(
            ([c]) => c === "reading_log_clear",
          ),
        ),
      )
      .toBe(true);
  });

  for (const theme of ["light", "dark"]) {
    for (const width of [1440, 390]) {
      test(`Beleg ${width} ${theme}`, async ({ page }) => {
        await page.setViewportSize({ width, height: width > 600 ? 1000 : 844 });
        await mock(page, { rows: uiRows(today), theme });
        await openStats(page);
        await page.waitForTimeout(300);
        const tag = theme === "light" ? "hell" : "dunkel";
        await page.screenshot({ path: shot(`rs-app-${width}-${tag}.png`) });
        await page.getByTestId("reading-stats").evaluate((el) => {
          let p: HTMLElement | null = el;
          while (p && p.scrollHeight <= p.clientHeight + 1) p = p.parentElement;
          if (p) p.scrollTop = p.scrollHeight;
        });
        await page.waitForTimeout(200);
        await page.screenshot({
          path: shot(`rs-app-${width}-${tag}-unten.png`),
        });
        await expect(page.locator("html")).toHaveJSProperty(
          "scrollWidth",
          await page.evaluate(() => document.documentElement.clientWidth),
        );
      });
    }
  }
});
