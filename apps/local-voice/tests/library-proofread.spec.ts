import { test, expect, type Page } from "@playwright/test";
import { join } from "node:path";
import { hashText } from "../src/lib/library/proofread";

// Korrekturlesen des Hörbuch-Transkripts (lva#208 B): Auto-Start nach der
// Transkription, Knopf, Stand, Ergebnis, Änderungen, Original wiederherstellen,
// Verhalten ohne Anbieter. Der Kern ist gemockt: `library_proofread` liefert
// den Text mit festen Korrekturen und hält auf Wunsch an (Stand sichtbar).
const shot = (name: string) =>
  join(process.env.LV_SHOT_DIR ?? "test-results", name);

const PREFIX = "# Der Prozess\n\nvon Franz Kafka\n\n";
const RAW = [
  "das haus ist gross und alt",
  "dann kam der winter mit schnee",
  "es schneite tagelang",
  "am ende war alles weiss",
];
const FIXED = [
  "Das Haus ist groß und alt.",
  "Dann kam der Winter mit Schnee.",
  "es schneite tagelang", // von der Prüfung im Original gelassen
  "Am Ende war alles weiß.",
];
const RAW_TEXT = PREFIX + RAW.join("\n\n");
const FIXED_TEXT = PREFIX + FIXED.join("\n\n");

const startsOf = (text: string) => {
  const out: number[] = [];
  let at = 0;
  for (const p of text.split("\n\n")) {
    out.push(at);
    at += p.length + 2;
  }
  return out;
};
const SYNC = startsOf(RAW_TEXT)
  .slice(2)
  .map((chars, i) => ({ chars, ms: i * 5000 }));

const transcriptBook = (over: Record<string, unknown> = {}) => ({
  id: "book_p",
  title: "Der Prozess",
  subtitle: null,
  authors: ["Franz Kafka"],
  language: "Deutsch",
  isbn: null,
  publisher: null,
  publish_date: null,
  pages: null,
  description: null,
  metadata_source: "manual",
  cover_source: "generated",
  text: {
    page_id: "page_p",
    origin: "transcript",
    source_name: null,
    proofread: null,
  },
  audio: {
    file: "teil_001.wav",
    duration_ms: null,
    narrator: null,
    transcript_job: null,
    tracks: [
      {
        file: "teil_001.wav",
        title: "Teil 1",
        duration_ms: 30_000,
        transcript_job: null,
      },
    ],
    sync: SYNC,
  },
  added_ms: 10,
  finished: false,
  cover_path: null,
  audio_path: "C:/lib/book_p/audio/teil_001.wav",
  audio_paths: ["C:/lib/book_p/audio/teil_001.wav"],
  ...over,
});

type Opts = {
  cloud?: boolean;
  provider?: boolean;
  auto?: boolean;
  hold?: boolean;
  theme?: string;
  proposal?: unknown;
};

/** Verworfener Vorschlag zum dritten Textabsatz (Index 4 mit Titelei). */
const PROPOSAL = {
  index: 4,
  text: "Es schneite tagelang und nächtelang.",
};

const setup = async (
  page: Page,
  books: unknown[],
  pages: Record<string, string>,
  opts: Opts = {},
) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(
    ({ books, pages, opts, rawText, rawParas, fixedParas }) => {
      const w = window as unknown as Record<string, any>;
      const callbacks = new Map<number, (e: unknown) => void>();
      let callback = 0;
      const listeners: Record<string, number[]> = {};
      const provider = opts.cloud
        ? {
            id: "openai",
            label: "OpenAI",
            base_url: "https://api.openai.com/v1",
          }
        : {
            id: "ollama",
            label: "Ollama",
            base_url: "http://localhost:11434/v1",
          };
      const settings = {
        onboarding_completed: true,
        app_language: "de",
        theme: opts.theme ?? "light",
        show_whats_new_on_update: false,
        debug_mode: false,
        selected_model: "",
        bindings: {},
        post_process_providers: opts.provider === false ? [] : [provider],
        post_process_provider_id: opts.provider === false ? "" : provider.id,
        post_process_models:
          opts.provider === false ? {} : { [provider.id]: "qwen3:8b" },
        post_process_prompts: [],
        post_process_api_keys: {},
        custom_words: [],
        push_to_talk: true,
        tts_engine: "piper",
      };
      w.__calls = [] as unknown[];
      w.__books = books;
      w.__pages = pages;
      w.__original = null;
      w.__corrected = null;
      w.__proposals = [] as unknown[];
      w.__origSync = null;
      w.__created = 0;
      w.__running = [] as string[];
      w.__setCloud = () => {
        settings.post_process_providers = [
          {
            id: "openai",
            label: "OpenAI",
            base_url: "https://api.openai.com/v1",
          },
        ];
        settings.post_process_provider_id = "openai";
        settings.post_process_models = { openai: "gpt-x" };
      };
      w.__jobs = [] as string[];
      w.__segments = {} as Record<string, unknown[]>;
      w.__emit = (event: string, payload: unknown) =>
        (listeners[event] ?? []).forEach((h) =>
          callbacks.get(h)?.({ event, id: 0, payload }),
        );
      let release: () => void = () => {};
      const gate = opts.hold
        ? new Promise<void>((resolve) => (release = resolve))
        : Promise.resolve();
      w.__release = () => release();
      if (opts.auto === false)
        window.localStorage.setItem("lva.ui.library.proofreadAuto", "off");
      window.localStorage.setItem("lva.ui.section", "library");
      // Lesestelle im dritten Absatz nach der Titelei („es schneite tagelang“).
      const at = rawText.indexOf("tagelang");
      window.localStorage.setItem(
        "lva.ui.progress.text:page_p:original",
        JSON.stringify({
          offset: at,
          length: rawText.length,
          heard: [[0, at]],
          read: [],
          updated: Date.now(),
        }),
      );
      const fix = (text: string) => {
        const map: Record<string, string> = {};
        rawParas.forEach((r: string, i: number) => (map[r] = fixedParas[i]));
        return text
          .split("\n\n")
          .map((p) => map[p] ?? p)
          .join("\n\n");
      };
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
          transformCallback: (fn: (e: unknown) => void) => {
            callbacks.set(++callback, fn);
            return callback;
          },
          unregisterCallback: (id: number) => callbacks.delete(id),
          convertFileSrc: (path: string) => `asset://${path}`,
          invoke: async (cmd: string, args?: Record<string, any>) => {
            w.__calls.push([cmd, args]);
            const list = w.__books as Record<string, any>[];
            if (cmd === "get_app_settings" || cmd === "get_default_settings")
              return JSON.parse(JSON.stringify(settings));
            if (cmd === "plugin:os|locale") return "de-DE";
            if (cmd === "plugin:app|version") return "0.23.0";
            if (cmd.includes("permission")) return true;
            if (cmd === "plugin:event|listen") {
              (listeners[String(args?.event)] ??= []).push(
                Number(args?.handler),
              );
              return Number(args?.handler);
            }
            if (cmd === "plugin:event|unlisten") return null;
            if (cmd === "library_list") return list;
            if (cmd === "library_save") {
              const b = args?.book as Record<string, any>;
              w.__books = [...list.filter((x) => x.id !== b.id), b];
              return b;
            }
            if (cmd === "library_progress_load") return {};
            if (cmd === "library_progress_put") return null;
            if (cmd === "pages_create")
              return {
                id: `page_new${++w.__created}`,
                title: String(args?.title ?? ""),
                modified_ms: 0,
                preview: "",
              };
            if (cmd === "page_state_load")
              return (w.__pages as Record<string, string>)[String(args?.id)];
            if (cmd === "page_state_save") {
              (w.__pages as Record<string, string>)[String(args?.id)] = String(
                args?.state,
              );
              return null;
            }
            if (cmd === "page_files") return [];
            if (cmd === "meetings_list")
              return (w.__jobs as string[]).map((id) => ({
                id,
                title: "x",
                status: (w.__running as string[]).includes(id)
                  ? "running"
                  : "ready",
                duration_ms: 30_000,
              }));
            if (cmd === "meetings_get_segments")
              return (
                (w.__segments as Record<string, unknown[]>)[
                  String(args?.meetingId)
                ] ?? []
              );
            if (cmd === "library_proofread") {
              const base = args?.fromOriginal
                ? (w.__original as string)
                : String(args?.pageText);
              if (!args?.fromOriginal) w.__original = base;
              w.__emit("library-proofread-progress", {
                book_id: args?.bookId,
                done: 3,
                total: 8,
              });
              await gate;
              w.__corrected = fix(base);
              w.__proposals = [opts.proposal];
              return {
                text: fix(base),
                changed: 3,
                rejected: 1,
                unchanged: 0,
                blocks: 8,
                cached: 0,
                model: "ollama/qwen3:8b",
                at_ms: Date.UTC(2026, 10, 2, 12, 30),
              };
            }
            if (cmd === "library_proofread_original") return w.__original;
            if (cmd === "library_proofread_material")
              return {
                original: w.__original,
                corrected: w.__corrected,
                proposals: w.__proposals,
                original_sync: w.__origSync,
              };
            if (cmd === "library_proofread_save_sync") {
              w.__origSync = String(args?.syncJson);
              return null;
            }
            if (cmd === "library_proofread_discard") {
              w.__original = null;
              w.__corrected = null;
              w.__proposals = [];
              w.__origSync = null;
              return null;
            }
            if (cmd === "library_proofread_cancel") return null;
            if (cmd === "tts_server_status")
              return { phase: "stopped", message: null };
            if (cmd === "pages_list")
              return [{ id: "page_p", title: "Der Prozess" }];
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
      books,
      pages,
      opts: { ...opts, proposal: PROPOSAL },
      rawText: RAW_TEXT,
      rawParas: RAW,
      fixedParas: FIXED,
    },
  );
};

const calls = (page: Page, cmd: string) =>
  page.evaluate(
    (cmd) =>
      (
        (window as unknown as { __calls: [string, Record<string, any>][] })
          .__calls ?? []
      )
        .filter(([c]) => c === cmd)
        .map(([, a]) => a),
    cmd,
  );

const stateOf = (text: string) =>
  JSON.stringify({ text, summary: "", sourceUrl: "", tab: "original" });

const openVersions = async (page: Page) => {
  await page
    .getByRole("button", { name: "„Der Prozess“: Details öffnen" })
    .click();
  const detail = page.getByTestId("library-detail");
  await detail.getByRole("tab", { name: "Fassungen" }).click();
  return detail;
};

test("Knopf: Stand mit Abschnitten, Ergebnis mit Prüfung, Text und Zeitachse umgerechnet", async ({
  page,
}) => {
  await setup(
    page,
    [transcriptBook()],
    { page_p: stateOf(RAW_TEXT) },
    { hold: true },
  );
  await page.setViewportSize({ width: 1440, height: 1100 });
  await page.goto("/");
  const detail = await openVersions(page);
  const panel = detail.getByTestId("proofread-panel");
  await expect(panel).toContainText(
    "Das Transkript stammt aus der Spracherkennung",
  );
  await panel.getByTestId("proofread-start").click();
  await expect(panel.getByTestId("proofread-status")).toContainText(
    "Korrekturlesen 3 von 8 Abschnitten",
  );
  // „In Arbeit“ in der Bibliothek zeigt den Schritt.
  await detail.getByRole("button", { name: "Bibliothek" }).click();
  await expect(page.locator(".library-jobs")).toContainText(
    "Wird korrekturgelesen · 3 von 8",
  );
  await page
    .getByRole("button", { name: "„Der Prozess“: Details öffnen" })
    .click();
  await detail.getByRole("tab", { name: "Fassungen" }).click();
  await panel.scrollIntoViewIfNeeded();
  await page.screenshot({ path: shot("lektorat-status-1440-hell.png") });
  await page.evaluate(() =>
    (window as unknown as { __release(): void }).__release(),
  );
  await expect(panel.getByTestId("proofread-result")).toContainText(
    "3 Absätze korrigiert",
  );
  await expect(panel.getByTestId("proofread-rejected")).toHaveText(
    "1 Korrekturvorschlag verworfen (Schutzprüfung: Inhalt hätte sich geändert) – zur manuellen Prüfung",
  );
  await expect(panel.getByTestId("proofread-start")).toHaveText(
    "Erneut korrekturlesen",
  );
  // Die Zeitachse des Originals liegt beim Original (für „wiederherstellen“).
  const savedSync = await calls(page, "library_proofread_save_sync");
  expect(savedSync).toHaveLength(1);
  expect(JSON.parse(savedSync[0].syncJson)).toEqual(SYNC);

  const proof = await calls(page, "library_proofread");
  expect(proof).toHaveLength(1);
  expect(proof[0]).toMatchObject({
    bookId: "book_p",
    pageText: RAW_TEXT,
    fromOriginal: false,
    requireLocal: false,
  });
  const saves = await calls(page, "page_state_save");
  expect(JSON.parse(saves[saves.length - 1].state).text).toBe(FIXED_TEXT);
  const books = await calls(page, "library_save");
  const book = books[books.length - 1].book as {
    text: { proofread: { changed: number; rejected: number; model: string } };
    audio: { sync: { chars: number; ms: number }[] };
  };
  expect(book.text.proofread).toMatchObject({
    changed: 3,
    rejected: 1,
    unchanged: 0,
    model: "ollama/qwen3:8b",
    text_hash: hashText(FIXED_TEXT),
  });
  // Mitlesen bleibt auf demselben Absatz: jeder Stützpunkt steht am Absatzanfang des neuen Textes.
  const starts = startsOf(FIXED_TEXT).slice(2);
  expect(book.audio.sync.map((p) => p.chars)).toEqual(starts);
  expect(book.audio.sync.map((p) => p.ms)).toEqual(SYNC.map((p) => p.ms));
  // Die Lesestelle (dritter Absatz) liegt im dritten Absatz des neuen Textes.
  const rec = await page.evaluate(() =>
    JSON.parse(
      window.localStorage.getItem("lva.ui.progress.text:page_p:original") ??
        "{}",
    ),
  );
  expect(rec.offset).toBeGreaterThanOrEqual(starts[2]);
  expect(rec.offset).toBeLessThan(starts[3]);
  expect(rec.length).toBe(FIXED_TEXT.length);
  await panel.scrollIntoViewIfNeeded();
  await page.screenshot({ path: shot("lektorat-ergebnis-1440-hell.png") });
});

const INFO = {
  model: "ollama/qwen3:8b",
  at_ms: Date.UTC(2026, 10, 2, 12, 30),
  changed: 3,
  rejected: 1,
  unchanged: 0,
  text_hash: hashText(FIXED_TEXT),
};
/** Zeitachse des Originals, bewusst nicht an Absatzanfängen (Umrechnen ergäbe sie nicht). */
const ORIG_SYNC = SYNC.map((p) => ({ chars: p.chars + 3, ms: p.ms }));

/** Buch nach dem Korrekturlesen; `text` ist der aktuelle Text der Seite. */
const corrected = (text = FIXED_TEXT, info: Record<string, unknown> = INFO) =>
  setupCorrected(text, info);

const setupCorrected = (text: string, info: Record<string, unknown>) => ({
  books: [
    transcriptBook({
      text: {
        page_id: "page_p",
        origin: "transcript",
        source_name: null,
        proofread: info,
      },
      audio: {
        ...transcriptBook().audio,
        sync: startsOf(FIXED_TEXT)
          .slice(2)
          .map((chars, i) => ({ chars, ms: i * 5000 })),
      },
    }),
  ],
  pages: { page_p: stateOf(text) },
});

const seedMaterial = (page: Page) =>
  page.addInitScript(
    ({ raw, fixed, proposal, sync }) => {
      const w = window as unknown as Record<string, unknown>;
      w.__original = raw;
      w.__corrected = fixed;
      w.__proposals = [proposal];
      w.__origSync = JSON.stringify(sync);
    },
    { raw: RAW_TEXT, fixed: FIXED_TEXT, proposal: PROPOSAL, sync: ORIG_SYNC },
  );

const lastText = async (page: Page) => {
  const saves = await calls(page, "page_state_save");
  return JSON.parse(saves[saves.length - 1].state).text as string;
};

/** Texte der Seite mit Absatz `i` auf `value`. */
const withParagraph = (text: string, i: number, value: string) =>
  text
    .split("\n\n")
    .map((x, n) => (n === i ? value : x))
    .join("\n\n");

test("Änderungen anzeigen: Absatz alt/neu mit markierten Wörtern; Original wiederherstellen", async ({
  page,
}) => {
  const { books, pages } = corrected();
  await setup(page, books, pages);
  await seedMaterial(page);
  await page.setViewportSize({ width: 1440, height: 1100 });
  await page.goto("/");
  const detail = await openVersions(page);
  const panel = detail.getByTestId("proofread-panel");
  await expect(panel.getByTestId("proofread-result")).toContainText(
    "3 Absätze korrigiert",
  );
  await expect(panel.getByTestId("proofread-edited-notice")).toHaveCount(0);
  await panel.getByTestId("proofread-show-diff").click();
  const diff = page.getByTestId("proofread-diff");
  await expect(diff.getByTestId("proofread-diff-item")).toHaveCount(3);
  await expect(diff.getByTestId("proofread-diff-note")).toHaveText(
    "Die Korrekturen sind bereits im Text. Abgelehnte Absätze werden auf das Original zurückgesetzt.",
  );
  // Titel- und Autorzeile zählen nicht: das Transkript beginnt bei Absatz 1.
  await expect(diff.getByTestId("proofread-diff-item").first()).toContainText(
    "Absatz 1",
  );
  await expect(diff.locator("del").first()).toBeVisible();
  await expect(diff.locator("ins").first()).toBeVisible();
  await expect(
    diff.locator("[data-side=before] del").filter({ hasText: "gross" }),
  ).toHaveCount(1);
  await expect(
    diff.locator("[data-side=after] ins").filter({ hasText: "groß" }),
  ).toHaveCount(1);
  await page.screenshot({ path: shot("lektorat-aenderungen-1440-hell.png") });
  await page.keyboard.press("Escape");

  await panel.getByTestId("proofread-restore").click();
  await page.getByTestId("proofread-restore-confirm").click();
  await expect(panel.getByTestId("proofread-result")).toHaveCount(0);
  expect(await lastText(page)).toBe(RAW_TEXT);
  const books2 = await calls(page, "library_save");
  const book = books2[books2.length - 1].book as {
    text: { proofread: unknown };
    audio: { sync: { chars: number }[] };
  };
  expect(book.text.proofread).toBeNull();
  // Die Zeitachse von damals kommt genau so zurück, nicht nur umgerechnet.
  expect(book.audio.sync).toEqual(ORIG_SYNC);
  expect(await calls(page, "library_proofread_discard")).toHaveLength(1);
  // Erneut korrekturlesen geht vom Text der Seite aus (keine Korrektur mehr gesichert).
  await panel.getByTestId("proofread-start").click();
  await expect
    .poll(async () => (await calls(page, "library_proofread")).length)
    .toBe(1);
  expect((await calls(page, "library_proofread"))[0].fromOriginal).toBe(false);
});

test("Annehmen/Ablehnen je Absatz und für alle; abgelehnt = Original genau dieses Absatzes", async ({
  page,
}) => {
  const { books, pages } = corrected();
  await setup(page, books, pages);
  await seedMaterial(page);
  await page.setViewportSize({ width: 1440, height: 1100 });
  await page.goto("/");
  const detail = await openVersions(page);
  await detail.getByTestId("proofread-show-diff").click();
  const diff = page.getByTestId("proofread-diff");
  const items = diff.getByTestId("proofread-diff-item");
  await expect(items).toHaveCount(3);
  await expect(items.first().getByTestId("proofread-item-state")).toHaveText(
    "Korrigiert",
  );
  // Einen Absatz annehmen: nur die Markierung.
  await items.first().getByTestId("proofread-accept").click();
  await expect(items.first().getByTestId("proofread-item-state")).toHaveText(
    "Angenommen",
  );
  // Einen Absatz ablehnen: genau dieser Absatz geht auf das Original zurück.
  await items.nth(1).getByTestId("proofread-reject").click();
  await expect(items.nth(1).getByTestId("proofread-item-state")).toHaveText(
    "Abgelehnt (Original)",
  );
  expect(await lastText(page)).toBe(withParagraph(FIXED_TEXT, 3, RAW[1]));
  await page.screenshot({ path: shot("lektorat-annehmen-1440-hell.png") });
  // Alle ablehnen: der ganze Text ist wieder das Original.
  await diff.getByTestId("proofread-reject-all").click();
  await expect(diff.getByTestId("proofread-reject-all")).toBeDisabled();
  expect(await lastText(page)).toBe(RAW_TEXT);
  // Alle annehmen: die Korrekturen sind wieder da.
  await diff.getByTestId("proofread-accept-all").click();
  await expect(diff.getByTestId("proofread-accept-all")).toBeDisabled();
  expect(await lastText(page)).toBe(FIXED_TEXT);
  // Der Vermerk am Buch trägt immer die Prüfsumme des geschriebenen Textes.
  const saved = await calls(page, "library_save");
  const last = saved[saved.length - 1].book as {
    text: { proofread: { text_hash: string } };
  };
  expect(last.text.proofread.text_hash).toBe(hashText(FIXED_TEXT));
});

test("von Hand bearbeiteter Absatz: Warnung, gesperrt, bleibt bei Ablehnen und Wiederherstellen", async ({
  page,
}) => {
  const mine = "Dann kam der Winter mit sehr viel Schnee.";
  const text = withParagraph(FIXED_TEXT, 3, mine);
  const { books, pages } = corrected(text);
  await setup(page, books, pages);
  await seedMaterial(page);
  await page.setViewportSize({ width: 1440, height: 1100 });
  await page.goto("/");
  const detail = await openVersions(page);
  const panel = detail.getByTestId("proofread-panel");
  await expect(panel.getByTestId("proofread-edited-notice")).toContainText(
    "Der Text wurde nach dem Korrekturlesen bearbeitet",
  );
  await panel.getByTestId("proofread-show-diff").click();
  const diff = page.getByTestId("proofread-diff");
  await expect(diff.getByTestId("proofread-edited-warning")).toContainText(
    "1 Absatz wurde von Hand bearbeitet",
  );
  const locked = diff
    .getByTestId("proofread-diff-item")
    .filter({ has: page.getByTestId("proofread-item-locked") });
  await expect(locked).toHaveCount(1);
  await expect(locked.getByRole("button")).toHaveCount(0);
  await expect(locked.getByTestId("proofread-item-state")).toHaveText(
    "Von Hand bearbeitet · gesperrt",
  );
  await page.screenshot({ path: shot("lektorat-bearbeitet-1440-hell.png") });
  // Alle ablehnen setzt nur die unveränderten Absätze zurück.
  await diff.getByTestId("proofread-reject-all").click();
  await expect(diff.getByTestId("proofread-reject-all")).toBeDisabled();
  expect(await lastText(page)).toBe(withParagraph(RAW_TEXT, 3, mine));
  await page.keyboard.press("Escape");
  // Alle annehmen setzt die Korrekturen wieder ein, nicht über die eigene Änderung.
  await panel.getByTestId("proofread-show-diff").click();
  await diff.getByTestId("proofread-accept-all").click();
  await expect(diff.getByTestId("proofread-accept-all")).toBeDisabled();
  expect(await lastText(page)).toBe(text);
  await page.keyboard.press("Escape");
  // Wiederherstellen: der eigene Absatz bleibt, Vermerk und Sicherung auch.
  await panel.getByTestId("proofread-restore").click();
  await page.getByTestId("proofread-restore-confirm").click();
  await expect
    .poll(
      async () => (await lastText(page)) === withParagraph(RAW_TEXT, 3, mine),
    )
    .toBe(true);
  await expect(panel.getByTestId("proofread-result")).toBeVisible();
  expect(await calls(page, "library_proofread_discard")).toHaveLength(0);
  // Erneut korrekturlesen überschreibt den eigenen Absatz ebenfalls nicht.
  await panel.getByTestId("proofread-start").click();
  await expect
    .poll(async () => (await calls(page, "library_proofread")).length)
    .toBe(1);
  await expect
    .poll(async () => (await lastText(page)).includes(mine))
    .toBe(true);
  expect(await lastText(page)).toBe(text);
});

test("verworfene Vorschläge: eigene Gruppe, Übernahme erst nach Bestätigung", async ({
  page,
}) => {
  const { books, pages } = corrected();
  await setup(page, books, pages);
  await seedMaterial(page);
  await page.setViewportSize({ width: 1440, height: 1100 });
  await page.goto("/");
  const detail = await openVersions(page);
  await detail.getByTestId("proofread-show-diff").click();
  const group = page.getByTestId("proofread-proposals");
  await expect(group).toContainText("Verworfene Vorschläge");
  await expect(group.getByTestId("proofread-proposal-item")).toHaveCount(1);
  await expect(group).toContainText("Es schneite tagelang und nächtelang.");
  await page.screenshot({ path: shot("lektorat-vorschlaege-1440-hell.png") });
  await group.getByTestId("proofread-adopt").click();
  // Erst die Bestätigung schreibt etwas.
  await expect(page.getByTestId("proofread-adopt-confirm")).toBeVisible();
  expect(await calls(page, "page_state_save")).toHaveLength(0);
  await page.getByTestId("proofread-adopt-confirm").click();
  await expect(group.getByTestId("proofread-adopt-revert")).toBeVisible();
  expect(await lastText(page)).toBe(
    withParagraph(FIXED_TEXT, 4, PROPOSAL.text),
  );
  await group.getByTestId("proofread-adopt-revert").click();
  await expect(group.getByTestId("proofread-adopt")).toBeVisible();
  expect(await lastText(page)).toBe(FIXED_TEXT);
});

test("erneutes Korrekturlesen geht vom gesicherten Original aus", async ({
  page,
}) => {
  const { books, pages } = corrected();
  await setup(page, books, pages);
  await seedMaterial(page);
  await page.goto("/");
  const detail = await openVersions(page);
  await detail.getByTestId("proofread-start").click();
  await expect
    .poll(async () => (await calls(page, "library_proofread")).length)
    .toBe(1);
  expect((await calls(page, "library_proofread"))[0].fromOriginal).toBe(true);
});

const segments = () => {
  const seg = (i: number, text: string, from: number, to: number) => ({
    segment_index: i,
    text,
    start_ms: from,
    end_ms: to,
    channel: 0,
    speaker_index: null,
  });
  return RAW.map((t, i) => seg(i, t, i * 5000, i * 5000 + 1000));
};

const untranscribed = () =>
  transcriptBook({
    text: null,
    audio: {
      file: "teil_001.wav",
      duration_ms: null,
      narrator: null,
      transcript_job: null,
      tracks: [
        {
          file: "teil_001.wav",
          title: "Teil 1",
          duration_ms: 30_000,
          transcript_job: "m21",
        },
      ],
      sync: [],
    },
  });

const second = () => ({
  ...untranscribed(),
  id: "book_q",
  title: "Zweites Buch",
  audio: {
    ...untranscribed().audio,
    tracks: [
      {
        file: "teil_001.wav",
        title: "Teil 1",
        duration_ms: 30_000,
        transcript_job: "m22",
      },
    ],
  },
  audio_path: "C:/lib/book_q/audio/teil_001.wav",
  audio_paths: ["C:/lib/book_q/audio/teil_001.wav"],
});

test("Auto-Start: nach der Transkription liest der Kern den neuen Text Korrektur", async ({
  page,
}) => {
  await setup(page, [untranscribed()], {}, { auto: true });
  await page.addInitScript((segs) => {
    const w = window as unknown as Record<string, any>;
    w.__jobs.push("m21");
    w.__segments["m21"] = segs;
  }, segments());
  await page.goto("/");
  await expect
    .poll(async () => (await calls(page, "library_proofread")).length)
    .toBe(1);
  const proof = (await calls(page, "library_proofread"))[0];
  expect(proof.fromOriginal).toBe(false);
  // Automatischer Start: der Kern prüft selbst, dass der Anbieter lokal ist.
  expect(proof.requireLocal).toBe(true);
  // Ausgangstext: Titelei und die vier Absätze der Transkription.
  expect(proof.pageText).toBe(RAW_TEXT);
  await expect
    .poll(async () => {
      const saves = await calls(page, "page_state_save");
      return saves.length ? JSON.parse(saves[saves.length - 1].state).text : "";
    })
    .toBe(FIXED_TEXT);
  await expect
    .poll(async () => {
      const books = await calls(page, "library_save");
      const last = books[books.length - 1]?.book as
        { text?: { proofread?: unknown } } | undefined;
      return !!last?.text?.proofread;
    })
    .toBe(true);
});

test("Auto-Start aus: nach der Transkription wird nicht korrekturgelesen", async ({
  page,
}) => {
  await setup(page, [untranscribed()], {}, { auto: false });
  await page.addInitScript((segs) => {
    const w = window as unknown as Record<string, any>;
    w.__jobs.push("m21");
    w.__segments["m21"] = segs;
  }, segments());
  await page.goto("/");
  await expect
    .poll(async () => (await calls(page, "page_state_save")).length)
    .toBeGreaterThan(0);
  await page.waitForTimeout(800);
  expect(await calls(page, "library_proofread")).toHaveLength(0);
});

test("ohne Anbieter: Hinweis mit Link zu den Einstellungen, kein Auto-Start, Knopf gesperrt", async ({
  page,
}) => {
  await setup(page, [untranscribed()], {}, { provider: false, auto: true });
  await page.addInitScript((segs) => {
    const w = window as unknown as Record<string, any>;
    w.__jobs.push("m21");
    w.__segments["m21"] = segs;
  }, segments());
  await page.setViewportSize({ width: 1440, height: 1100 });
  await page.goto("/");
  await expect
    .poll(async () => (await calls(page, "page_state_save")).length)
    .toBeGreaterThan(0);
  await page.waitForTimeout(800);
  expect(await calls(page, "library_proofread")).toHaveLength(0);
  const detail = await openVersions(page);
  const notice = detail.getByTestId("proofread-no-provider");
  await expect(notice).toContainText(
    "Zum Korrekturlesen fehlt ein Sprachmodell",
  );
  await expect(detail.getByTestId("proofread-start")).toBeDisabled();
  await notice.scrollIntoViewIfNeeded();
  await page.screenshot({ path: shot("lektorat-ohne-anbieter-1440-hell.png") });
  await notice.getByRole("button", { name: "Einstellungen öffnen" }).click();
  await expect
    .poll(() =>
      page.evaluate(() => window.localStorage.getItem("lva.ui.section")),
    )
    .toBe("settings");
  await expect
    .poll(() =>
      page.evaluate(() => window.localStorage.getItem("lva.ui.settings.tab")),
    )
    .toBe("models");
});

test("Fehler des Kerns erscheint am Buch", async ({ page }) => {
  await setup(page, [transcriptBook()], { page_p: stateOf(RAW_TEXT) });
  await page.addInitScript(() => {
    const internals = (window as unknown as Record<string, any>)
      .__TAURI_INTERNALS__;
    const invoke = internals.invoke;
    internals.invoke = async (cmd: string, args?: unknown) => {
      if (cmd === "library_proofread") {
        // Der Kern meldet einen Code; die Oberfläche übersetzt ihn.
        throw "proofread:busy";
      }
      return invoke(cmd, args);
    };
  });
  await page.goto("/");
  const detail = await openVersions(page);
  await detail.getByTestId("proofread-start").click();
  await expect(detail.getByTestId("proofread-error")).toContainText(
    "Es läuft bereits ein Korrekturlesen",
  );
});

for (const [theme, width, label] of [
  ["dark", 1440, "1440-dunkel"],
  ["light", 390, "390-hell"],
  ["dark", 390, "390-dunkel"],
] as const) {
  test(`Abnahmebilder ${label}`, async ({ page }) => {
    const { books, pages } = corrected();
    await setup(page, books, pages, { theme, hold: true });
    await seedMaterial(page);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 1100 });
    await page.goto("/");
    const detail = await openVersions(page);
    const panel = detail.getByTestId("proofread-panel");
    await expect(panel.getByTestId("proofread-result")).toBeVisible();
    await panel.scrollIntoViewIfNeeded();
    await page.screenshot({ path: shot(`lektorat-ergebnis-${label}.png`) });
    // Stand während des Laufs.
    await panel.getByTestId("proofread-start").click();
    await expect(panel.getByTestId("proofread-status")).toContainText(
      "3 von 8",
    );
    await panel.scrollIntoViewIfNeeded();
    await page.screenshot({ path: shot(`lektorat-status-${label}.png`) });
    await page.evaluate(() =>
      (window as unknown as { __release(): void }).__release(),
    );
    await expect(panel.getByTestId("proofread-result")).toBeVisible();
    await panel.getByTestId("proofread-show-diff").click();
    await expect(
      page.getByTestId("proofread-diff").getByTestId("proofread-diff-item"),
    ).toHaveCount(3);
    await page.screenshot({ path: shot(`lektorat-aenderungen-${label}.png`) });
    const overflow = await page.evaluate(
      () =>
        document.documentElement.scrollWidth -
        document.documentElement.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(0);
  });
}

test("Touch-Ziele bei 390 px: Knöpfe in Feld und Dialogen mindestens 44 × 44", async ({
  page,
}) => {
  const { books, pages } = corrected(
    withParagraph(FIXED_TEXT, 5, "Am Ende war alles sehr weiß."),
  );
  await setup(page, books, pages, { hold: true });
  await seedMaterial(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  const detail = await openVersions(page);
  const panel = detail.getByTestId("proofread-panel");
  const big = async (
    locator: import("@playwright/test").Locator,
    what: string,
  ) => {
    const count = await locator.count();
    expect(count, `${what}: keine Knöpfe gefunden`).toBeGreaterThan(0);
    for (let i = 0; i < count; i++) {
      const box = await locator.nth(i).boundingBox();
      const name = (await locator.nth(i).textContent())?.trim() || "(Symbol)";
      expect(box, `${what} „${name}“ ohne Fläche`).not.toBeNull();
      expect(box!.width, `${what} „${name}“ Breite`).toBeGreaterThanOrEqual(44);
      expect(box!.height, `${what} „${name}“ Höhe`).toBeGreaterThanOrEqual(44);
    }
  };
  await panel.scrollIntoViewIfNeeded();
  await big(panel.getByRole("button"), "Feld");
  // Während des Laufs: Abbrechen.
  await panel.getByTestId("proofread-start").click();
  await expect(panel.getByTestId("proofread-cancel")).toBeVisible();
  await big(panel.getByTestId("proofread-cancel"), "Abbrechen");
  await page.evaluate(() =>
    (window as unknown as { __release(): void }).__release(),
  );
  await expect(panel.getByTestId("proofread-start")).toHaveText(
    "Erneut korrekturlesen",
  );
  await big(panel.getByTestId("proofread-start"), "Erneut korrekturlesen");
  // Änderungsansicht: alle Knöpfe, auch das Schließen-X.
  await panel.getByTestId("proofread-show-diff").click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByTestId("proofread-diff-item").first()).toBeVisible();
  await big(dialog.getByRole("button"), "Änderungsansicht");
  await expect(dialog.getByRole("button", { name: "Schließen" })).toBeVisible();
  await page.screenshot({ path: shot("lektorat-annehmen-390-hell.png") });
  // Vorschlag übernehmen: Bestätigungsdialog.
  await dialog.getByTestId("proofread-adopt").click();
  await big(
    page.getByRole("dialog").last().getByRole("button"),
    "Vorschlag übernehmen",
  );
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
  // Wiederherstellen-Dialog.
  await panel.getByTestId("proofread-restore").click();
  await big(
    page.getByRole("dialog").last().getByRole("button"),
    "Wiederherstellen",
  );
});

test("Datenschutz: lokaler Auto-Start gilt nie als Einwilligung für einen Anbieter im Netz", async ({
  page,
}) => {
  await setup(page, [untranscribed(), second()], {}, { auto: true });
  await page.addInitScript((segs) => {
    const w = window as unknown as Record<string, any>;
    w.__jobs.push("m21", "m22");
    w.__running.push("m22");
    w.__segments["m21"] = segs;
    w.__segments["m22"] = segs;
  }, segments());
  await page.setViewportSize({ width: 1440, height: 1100 });
  await page.goto("/");
  // Lokal: das erste Buch wird automatisch korrekturgelesen.
  await expect
    .poll(async () => (await calls(page, "library_proofread")).length)
    .toBe(1);
  expect((await calls(page, "library_proofread"))[0].requireLocal).toBe(true);
  // Anbieter auf Cloud umstellen, dann wird das zweite Transkript fertig.
  await page.evaluate(() => {
    const w = window as unknown as Record<string, any>;
    w.__setCloud();
    w.__running = [];
  });
  await expect
    .poll(async () => (await calls(page, "page_state_save")).length, {
      timeout: 20_000,
    })
    .toBeGreaterThan(1);
  await page.waitForTimeout(1000);
  expect(await calls(page, "library_proofread")).toHaveLength(1);
  await page
    .getByRole("button", { name: "„Zweites Buch“: Details öffnen" })
    .click();
  const detail = page.getByTestId("library-detail");
  await detail.getByRole("tab", { name: "Fassungen" }).click();
  await expect(detail.getByTestId("proofread-cloud-notice")).toHaveText(
    "Der Buchtext verlässt dieses Gerät und wird an OpenAI gesendet.",
  );
  await expect(detail.getByTestId("proofread-auto-skipped")).toBeVisible();
  await page.screenshot({ path: shot("lektorat-cloud-hinweis-1440-hell.png") });
});

test("Cloud-Anbieter: kein Auto-Start, Hinweis „Der Buchtext verlässt dieses Gerät …“, Start nur per Klick", async ({
  page,
}) => {
  await setup(page, [untranscribed()], {}, { cloud: true, auto: true });
  await page.addInitScript((segs) => {
    const w = window as unknown as Record<string, any>;
    w.__jobs.push("m21");
    w.__segments["m21"] = segs;
  }, segments());
  await page.setViewportSize({ width: 1440, height: 1100 });
  await page.goto("/");
  await expect
    .poll(async () => (await calls(page, "page_state_save")).length)
    .toBeGreaterThan(0);
  await page.waitForTimeout(800);
  expect(await calls(page, "library_proofread")).toHaveLength(0);
  const detail = await openVersions(page);
  const notice = detail.getByTestId("proofread-cloud-notice");
  await expect(notice).toHaveText(
    "Der Buchtext verlässt dieses Gerät und wird an OpenAI gesendet.",
  );
  await notice.scrollIntoViewIfNeeded();
  await page.screenshot({ path: shot("lektorat-cloud-hinweis-1440-hell.png") });
  await detail.getByTestId("proofread-start").click();
  await expect
    .poll(async () => (await calls(page, "library_proofread")).length)
    .toBe(1);
});

test("Übersicht: kompakter Einstieg mit Knopf und Stand", async ({ page }) => {
  await setup(
    page,
    [transcriptBook()],
    { page_p: stateOf(RAW_TEXT) },
    { hold: true },
  );
  await page.goto("/");
  await page
    .getByRole("button", { name: "„Der Prozess“: Details öffnen" })
    .click();
  const panel = page
    .getByTestId("library-detail")
    .getByTestId("proofread-panel");
  await expect(panel).toBeVisible();
  await expect(panel.getByTestId("proofread-auto")).toHaveCount(0);
  await panel.getByTestId("proofread-start").click();
  await expect(panel.getByTestId("proofread-status")).toContainText("3 von 8");
  await page.evaluate(() =>
    (window as unknown as { __release(): void }).__release(),
  );
  await expect(panel.getByTestId("proofread-result")).toContainText(
    "3 Absätze korrigiert",
  );
});
