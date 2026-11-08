import { test, expect, type Page } from "@playwright/test";

// lva#344, PR 2: Sprachfassungen in der Oberfläche. Geprüft werden die
// Auflagen des Prüfers: Rückgängig stellt Fassung UND Stelle her, der Hinweis
// hält seine 10 s bei Fokus/Zeiger an, ein laufender Abgleich überlebt den
// Wechsel, die Konfliktentscheidung, der Fokus, die Tastenbedienung des
// Dropups und: nie automatisch zusammenlegen. Beispieldaten sind erfunden.

type Rec = Record<string, unknown>;

const track = (file: string, title: string, ms: number) => ({
  file,
  title,
  duration_ms: ms,
  transcript_job: null,
});
const audio = (tracks: ReturnType<typeof track>[]) => ({
  file: tracks[0].file,
  duration_ms: null,
  narrator: null,
  transcript_job: null,
  tracks,
  sync: [],
});
const paths = (book: string, tracks: ReturnType<typeof track>[]) =>
  tracks.map((t) => `C:/lib/${book}/audio/${t.file}`);
const text = (page: string, name: string) => ({
  page_id: page,
  origin: "document",
  source_name: name,
});

const DE3 = [1, 2, 3].map((n) => track(`de_00${n}.wav`, `Kapitel ${n}`, 20000));
const EN3 = [1, 2, 3].map((n) => track(`en_00${n}.wav`, `Chapter ${n}`, 15000));
const EN2 = [1, 2].map((n) => track(`en_00${n}.wav`, `Chapter ${n}`, 22500));

const base = (id: string, title: string, over: Rec = {}): Rec => ({
  id,
  title,
  subtitle: null,
  authors: ["Franz Beispiel"],
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
  added_ms: 5,
  finished: false,
  schema: 2,
  cover_path: null,
  audio_path: null,
  audio_paths: [],
  editions: [],
  primary: null,
  edition_tombstones: [],
  reading: null,
  alignments: [],
  ...over,
});

/** Zweisprachiges Buch: Deutsch (Hauptfassung) und Englisch, beide mit Text und Hörbuch. */
const twoLanguages = (
  id: string,
  titles: [string, string],
  de: ReturnType<typeof track>[],
  en: ReturnType<typeof track>[],
  over: Rec = {},
  enLocal = true,
): Rec => {
  const dePaths = paths(id, de);
  return base(id, titles[0], {
    text: text(`page_${id}_de`, "de.epub"),
    audio: audio(de),
    audio_path: dePaths[0],
    audio_paths: dePaths,
    editions: [
      {
        id: `ed_${id}_de`,
        lang: "de",
        title: titles[0],
        text: text(`page_${id}_de`, "de.epub"),
        audio: audio(de),
        audio_key: `audio:${id}`,
        origin: "migration",
        added_ms: 1,
        updated: 1,
        audio_paths: dePaths,
      },
      {
        id: `ed_${id}_en`,
        lang: "en",
        title: titles[1],
        text: text(`page_${id}_en`, "en.epub"),
        audio: audio(en),
        audio_key: `audio:${id}:ed_${id}_en`,
        origin: "import",
        added_ms: 2,
        updated: 2,
        audio_paths: enLocal ? paths(id, en) : [],
      },
    ],
    primary: { edition: `ed_${id}_de`, updated: 1 },
    ...over,
  });
};

const confirmed = (n: number) =>
  Array.from({ length: n }, (_, i) => ({ a: [i], b: [i], confirmed: true }));

const headings = (n: number, word: string) =>
  Array.from(
    { length: n },
    (_, i) => `# ${word} ${i + 1}\n\nAbsatz ${i + 1}.`,
  ).join("\n\n");

/** Stilles WAV (8 kHz, 8 Bit, mono). */
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

const serveAudio = async (page: Page) => {
  await page.route("**/__audio/**", async (route) => {
    const name = new URL(route.request().url()).pathname.split("/").pop() ?? "";
    const body = wav(name.startsWith("en_") ? 22 : 20);
    const range = /bytes=(\d+)-(\d*)/.exec(
      route.request().headers().range ?? "",
    );
    if (!range)
      return route.fulfill({
        status: 200,
        contentType: "audio/wav",
        headers: { "accept-ranges": "bytes" },
        body,
      });
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

const setup = async (
  page: Page,
  books: Rec[],
  extra: { pairs?: Rec[]; pageText?: Record<string, string> } = {},
) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await serveAudio(page);
  await page.addInitScript(
    ({ books, pairs, pageText }) => {
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
      const w = window as unknown as Record<string, any>;
      w.__calls = [] as unknown[];
      w.__books = books;
      w.__pairs = pairs;
      w.__pageText = pageText;
      const listeners: Record<string, number[]> = {};
      w.__emit = (event: string, payload: unknown) =>
        (listeners[event] ?? []).forEach((h) =>
          (callbacks.get(h) as ((e: unknown) => void) | undefined)?.({
            event,
            id: 0,
            payload,
          }),
        );
      window.localStorage.setItem("lva.ui.section", "library");
      window.localStorage.setItem("lva.library.device", "pc_test");
      // Stand vor dem Wechsel: Der Prozess steht bei 25 s (Teil 2, 5 s).
      window.localStorage.setItem(
        "lva.ui.progress.audio:book_a",
        JSON.stringify({
          offset: 25000,
          length: 60000,
          heard: [[0, 25000]],
          read: [],
          unit: "ms",
          updated: Date.now() - 60000,
        }),
      );
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
          convertFileSrc: (path: string) => `/__audio/${path.split("/").pop()}`,
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
            if (cmd === "plugin:event|unlisten") {
              const l = listeners[String(args?.event)] ?? [];
              listeners[String(args?.event)] = l.filter(
                (h) => h !== Number(args?.eventId),
              );
              return null;
            }
            if (cmd === "library_list") {
              if (w.__failList) throw "Bibliothek nicht lesbar";
              return list;
            }
            if (cmd === "library_language_pairs") return w.__pairs;
            if (cmd === "sync_now")
              return {
                connected: true,
                user_email: null,
                device_name: null,
                hub_url: null,
                running: false,
                last_success_ms: null,
                last_error: null,
                pending: 0,
              };
            if (cmd === "library_set_reading") {
              (w.__snapshots ??= []).push(
                window.localStorage.getItem("lva.ui.progress.audio:book_a"),
              );
              if (w.__failReading) throw "Speichern fehlgeschlagen";
              const b = list.find((x) => x.id === args?.id)!;
              const next = { ...b, reading: args?.reading };
              w.__books = list.map((x) => (x.id === next.id ? next : x));
              return next;
            }
            if (cmd === "library_set_alignment") {
              const b = list.find((x) => x.id === args?.id)!;
              const next = {
                ...b,
                alignments: [
                  ...(b.alignments ?? []).filter(
                    (a: Record<string, string>) =>
                      !(a.a === args?.alignment.a && a.b === args?.alignment.b),
                  ),
                  args?.alignment,
                ],
              };
              w.__books = list.map((x) => (x.id === next.id ? next : x));
              return next;
            }
            if (cmd === "library_join_editions")
              return list.find((x) => x.id === args?.targetId);
            if (cmd === "page_state_load") {
              const custom = w.__pageText[String(args?.id)];
              return JSON.stringify({
                text: custom ?? "# Kapitel\n\nText.",
                tab: "original",
              });
            }
            if (cmd === "audiosync_load")
              return {
                text_key: "",
                points: [],
                inserts: [],
                covered: [],
                anchors: [],
              };
            if (cmd === "audiosync_window") throw "Kein Erkennungsmodell";
            if (cmd === "library_progress_load") return {};
            if (cmd === "library_progress_put") return null;
            if (cmd === "tts_server_status")
              return { phase: "stopped", message: null };
            if (cmd === "get_custom_sounds")
              return { start: false, stop: false };
            if (cmd === "meetings_is_recording") return false;
            if (cmd === "get_selected_model") return "";
            if (
              cmd.endsWith("_list") ||
              cmd.startsWith("tts_list") ||
              cmd === "llm_ps" ||
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
    { books, pairs: extra.pairs ?? [], pageText: extra.pageText ?? {} },
  );
};

const PAGES = {
  page_book_a_de: headings(3, "Kapitel"),
  page_book_a_en: headings(3, "Chapter"),
  page_book_b_de: headings(2, "Kapitel"),
  page_book_b_en: headings(2, "Chapter"),
  page_book_c_de: headings(3, "Kapitel"),
  page_book_c_en: headings(2, "Chapter"),
};

const calls = (page: Page, cmd: string) =>
  page.evaluate(
    (name) =>
      (window as unknown as { __calls: [string, Rec][] }).__calls
        .filter(([c]) => c === name)
        .map(([, a]) => a),
    cmd,
  );

const play = async (page: Page, title: string) => {
  await page
    .getByTestId("library-card")
    .filter({ hasText: title })
    .getByTestId("library-play")
    .click();
  await expect(page.getByTestId("library-audiobar")).toBeVisible();
};

const seconds = async (page: Page) =>
  Number(await page.getByTestId("library-audiobar-seek").inputValue());

const progress = (page: Page, key: string) =>
  page.evaluate(
    (k) =>
      JSON.parse(window.localStorage.getItem(`lva.ui.progress.${k}`) ?? "null"),
    key,
  );

const pick = async (page: Page, edition: string) => {
  await page.getByTestId("sf-switch-button").click();
  await page.getByTestId(`sf-switch-item-${edition}`).click();
};

const bookA = () =>
  twoLanguages("book_a", ["Der Prozess", "The Trial"], DE3, EN3, {
    alignments: [
      {
        a: "ed_book_a_de",
        b: "ed_book_a_en",
        updated: 1,
        groups: confirmed(3),
      },
    ],
  });

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
});

test("Wechsel mit Hinweis: kapitelgenau, Rückgängig stellt Fassung und Stelle her", async ({
  page,
}) => {
  await setup(page, [bookA()], { pageText: PAGES });
  await page.goto("/");
  await play(page, "Der Prozess");
  await expect.poll(() => seconds(page)).toBeGreaterThanOrEqual(25);
  await expect(page.getByTestId("sf-switch-button")).toHaveAccessibleName(
    /Deutsch/,
  );
  await pick(page, "ed_book_a_en");

  const notice = page.getByTestId("sf-notice");
  await expect(notice).toContainText("English");
  await expect(notice).toContainText("kapitelgenau");
  await expect(page.getByTestId("sf-switch-button")).toHaveAccessibleName(
    /English/,
  );
  // Teil 2 des Deutschen (5 s im Teil) liegt im zweiten Kapitel des Englischen.
  const en = await progress(page, "audio:book_a:ed_book_a_en");
  expect(en.offset).toBeGreaterThanOrEqual(15000);
  expect(en.offset).toBeLessThan(30000);
  const afterSwitch = (await calls(page, "library_set_reading")).at(-1) as {
    reading: { edition: string };
  };
  expect(afterSwitch.reading.edition).toBe("ed_book_a_en");

  await page.getByTestId("sf-notice-undo").click();
  await expect(page.getByTestId("sf-switch-button")).toHaveAccessibleName(
    /Deutsch/,
  );
  const back = (await calls(page, "library_set_reading")).at(-1) as {
    reading: { edition: string; position: { offset: number; key: string } };
  };
  expect(back.reading.edition).toBe("ed_book_a_de");
  expect(back.reading.position.key).toBe("audio:book_a");
  // Dieselbe Stelle wie vor dem Wechsel (25 s, höchstens ein paar Sekunden weiter).
  expect(back.reading.position.offset).toBeGreaterThanOrEqual(25000);
  expect(back.reading.position.offset).toBeLessThan(40000);
  await expect.poll(() => seconds(page)).toBeGreaterThanOrEqual(25);
  expect(await seconds(page)).toBeLessThan(40);
  // Quellstelle (A): beim Setzen der Auswahl steht sie schon exakt im Fortschritt,
  // die Quellleiste startet erst danach.
  const snaps = await page.evaluate(
    () => (window as unknown as { __snapshots: string[] }).__snapshots,
  );
  expect(JSON.parse(snaps.at(-1)!).offset).toBe(back.reading.position.offset);
});

test("Ohne Zuordnung: ungefähr, mit Knopf „Kapitel zuordnen“", async ({
  page,
}) => {
  await setup(
    page,
    [twoLanguages("book_a", ["Der Prozess", "The Trial"], DE3, EN3)],
    {
      pageText: PAGES,
    },
  );
  await page.goto("/");
  await play(page, "Der Prozess");
  await pick(page, "ed_book_a_en");
  const notice = page.getByTestId("sf-notice");
  await expect(notice).toContainText("ungefähr");
  await expect(notice).not.toContainText("kapitelgenau");
  await expect(page.getByTestId("sf-notice-align")).toBeVisible();
  await expect(page.getByTestId("sf-notice-undo")).toBeVisible();
});

test("Der Hinweis hält seine 10 s bei Zeiger und bei Fokus an", async ({
  page,
}) => {
  await page.clock.install({ time: new Date("2026-11-06T10:00:00") });
  await setup(page, [bookA()], { pageText: PAGES });
  await page.goto("/");
  await play(page, "Der Prozess");
  await page.clock.pauseAt(new Date("2026-11-06T11:00:00"));

  // Zeiger: nach 15 s noch da, erst nach dem Wegnehmen läuft der Rest ab.
  await pick(page, "ed_book_a_en");
  const notice = page.getByTestId("sf-notice");
  await expect(notice).toBeVisible();
  await notice.hover();
  await page.clock.runFor(15000);
  await expect(notice).toBeVisible();
  await page.mouse.move(2, 2);
  await page.clock.runFor(9000);
  await expect(notice).toBeVisible();
  await page.clock.runFor(2000);
  await expect(notice).toBeHidden();

  // Fokus: dasselbe mit der Tastatur.
  await pick(page, "ed_book_a_de");
  await expect(notice).toBeVisible();
  await page.getByTestId("sf-notice-undo").focus();
  await page.clock.runFor(15000);
  await expect(notice).toBeVisible();
  await page.getByTestId("sf-switch-button").focus();
  await page.clock.runFor(9000);
  await expect(notice).toBeVisible();
  await page.clock.runFor(2000);
  await expect(notice).toBeHidden();
});

test("Ein laufender Abgleich überlebt den Wechsel: Wiedergabe läuft weiter", async ({
  page,
}) => {
  const book = twoLanguages(
    "book_a",
    ["Der Prozess", "The Trial"],
    DE3,
    EN3,
    {},
    false,
  );
  await setup(page, [book], { pageText: PAGES });
  await page.goto("/");
  await play(page, "Der Prozess");
  const toggle = page.getByTestId("library-audiobar-toggle");
  await expect
    .poll(async () => {
      const paused = await page.evaluate(
        () => document.querySelector("audio")?.paused ?? true,
      );
      if (paused) await toggle.click();
      return !paused;
    })
    .toBe(true);
  await page.evaluate(() => {
    document.querySelector("audio")!.setAttribute("data-marker", "same");
  });

  await page.getByTestId("sf-switch-button").click();
  const item = page.getByTestId("sf-switch-item-ed_book_a_en");
  await expect(item).toContainText("Nicht auf diesem Gerät");
  await item.click();

  // Abgleich angestoßen, Hinweis „wird geladen“, nichts umgeschaltet.
  await expect(page.getByTestId("sf-notice")).toContainText("wird geladen");
  expect((await calls(page, "sync_now")).length).toBeGreaterThan(0);
  expect(await calls(page, "library_set_reading")).toHaveLength(0);
  await expect(page.getByTestId("sf-switch-button")).toHaveAccessibleName(
    /Deutsch/,
  );
  await expect(page.locator("audio[data-marker='same']")).toHaveCount(1);
  expect(
    await page.evaluate(() => document.querySelector("audio")!.paused),
  ).toBe(false);

  // Der Abgleich bringt das Hörbuch: Eintrag zeigt „Wird geladen“, dann ist er wählbar.
  await page.getByTestId("sf-switch-button").click();
  await expect(item).toContainText("Wird geladen");
  await page.keyboard.press("Escape");
  await page.evaluate(
    (paths) => {
      const w = window as unknown as {
        __books: Rec[];
        __emit(event: string, payload: unknown): void;
      };
      w.__books = w.__books.map((b) => ({
        ...b,
        editions: (b.editions as Rec[]).map((e) =>
          e.id === "ed_book_a_en" ? { ...e, audio_paths: paths } : e,
        ),
      }));
      w.__emit("sync-changed", null);
    },
    paths("book_a", EN3),
  );
  await page.getByTestId("sf-switch-button").click();
  await expect(item).not.toContainText("Nicht auf diesem Gerät");
  await expect(item).not.toContainText("Wird geladen");
  await item.click();
  await expect(page.getByTestId("sf-switch-button")).toHaveAccessibleName(
    /English/,
  );
});

test("Konflikt nach dem Abgleich: „Hier bleiben“ und „Dorthin wechseln“", async ({
  page,
}) => {
  await setup(page, [bookA()], { pageText: PAGES });
  await page.goto("/");
  await play(page, "Der Prozess");
  const foreign = (updated: number, offset: number) =>
    page.evaluate(
      ({ updated, offset }) => {
        const w = window as unknown as {
          __books: Rec[];
          __emit(event: string, payload: unknown): void;
        };
        w.__books = w.__books.map((b) => ({
          ...b,
          reading: {
            edition: "ed_book_a_en",
            updated: Date.now() + updated,
            device: "ios_beispiel",
            position: {
              key: "audio:book_a:ed_book_a_en",
              unit: "ms",
              offset,
            },
          },
        }));
        w.__emit("sync-changed", null);
      },
      { updated, offset },
    );

  await foreign(5000, 30000);
  const notice = page.getByTestId("sf-notice");
  await expect(notice).toHaveAttribute("role", "alert");
  await expect(notice).toContainText("iPhone");
  await expect(notice).toContainText("English");
  // Der Konflikt ist hartnäckig: er läuft nicht von selbst ab.
  await page.waitForTimeout(500);
  await expect(notice).toBeVisible();
  await page.getByTestId("sf-notice-stay").click();
  await expect(notice).toBeHidden();
  const stay = (await calls(page, "library_set_reading")).at(-1) as {
    reading: { edition: string; device: string };
  };
  expect(stay.reading.edition).toBe("ed_book_a_de");
  expect(stay.reading.device).toBe("pc_test");
  await expect(page.getByTestId("sf-switch-button")).toHaveAccessibleName(
    /Deutsch/,
  );

  // Ein späterer, neuerer Stand fragt erneut; diesmal dorthin wechseln.
  await foreign(9000, 30000);
  await expect(notice).toContainText("iPhone");
  await page.getByTestId("sf-notice-go").click();
  await expect(page.getByTestId("sf-switch-button")).toHaveAccessibleName(
    /English/,
  );
  await expect.poll(() => seconds(page)).toBeGreaterThanOrEqual(30);
});

test("Fokus und Tastatur: Dropup mit Pfeilen, Esc, Eingabe; Fokus kehrt zum Sprachknopf zurück", async ({
  page,
}) => {
  await setup(page, [bookA()], { pageText: PAGES });
  await page.goto("/");
  await play(page, "Der Prozess");
  const button = page.getByTestId("sf-switch-button");

  // Alle Bedienelemente sind benannte Knöpfe mit mindestens 44 px.
  const size = await button.boundingBox();
  expect(size!.height).toBeGreaterThanOrEqual(44);
  expect(size!.width).toBeGreaterThanOrEqual(44);
  expect(await button.evaluate((el) => el.tagName)).toBe("BUTTON");
  await expect(button).toHaveAttribute("aria-haspopup", "menu");
  await expect(button).toHaveAttribute("aria-expanded", "false");

  // Esc schließt und lässt den Fokus am Knopf.
  await button.focus();
  await page.keyboard.press("Enter");
  const menu = page.getByTestId("sf-switch-menu");
  await expect(menu).toBeVisible();
  await expect(button).toHaveAttribute("aria-expanded", "true");
  await expect(menu).toHaveAttribute("role", "menu");
  await expect(page.getByTestId("sf-switch-item-ed_book_a_de")).toHaveAttribute(
    "aria-checked",
    "true",
  );
  await expect(page.getByTestId("sf-switch-item-ed_book_a_de")).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(menu).toBeHidden();
  await expect(button).toBeFocused();

  // Pfeil, Pfeil, Eingabe: auf Englisch wechseln; danach wieder der Sprachknopf.
  await page.keyboard.press("Enter");
  await page.keyboard.press("ArrowDown");
  await expect(page.getByTestId("sf-switch-item-ed_book_a_en")).toBeFocused();
  await page.keyboard.press("Home");
  await expect(page.getByTestId("sf-switch-item-ed_book_a_de")).toBeFocused();
  await page.keyboard.press("End");
  await expect(page.getByTestId("sf-switch-align")).toBeFocused();
  await page.keyboard.press("ArrowUp");
  await expect(page.getByTestId("sf-switch-item-ed_book_a_en")).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(button).toHaveAccessibleName(/English/);
  await expect(button).toBeFocused();

  // Der Hinweis erreicht seine Knöpfe per Tab, und „Rückgängig“ ist ein 44-px-Knopf.
  const undo = page.getByTestId("sf-notice-undo");
  expect((await undo.boundingBox())!.height).toBeGreaterThanOrEqual(44);
  expect(await undo.evaluate((el) => el.tagName)).toBe("BUTTON");
  await undo.focus();
  await page.keyboard.press("Enter");
  await expect(button).toHaveAccessibleName(/Deutsch/);
  await expect(button).toBeFocused();
});

test("Nie automatisch zusammenlegen: erst nach Bestätigung, Unsicheres nicht vorgewählt", async ({
  page,
}) => {
  const single = (id: string, title: string, lang: string, page_id: string) =>
    base(id, title, {
      language: lang,
      schema: 1,
      text: text(page_id, "x.epub"),
      editions: [],
      alignments: [],
    });
  await setup(
    page,
    [
      single("book_p1", "Das Schloss", "Deutsch", "page_p1"),
      single("book_p2", "The Castle", "English", "page_p2"),
      single("book_p3", "Die Burg", "Deutsch", "page_p3"),
      single("book_p4", "Fortress Tales", "English", "page_p4"),
    ],
    {
      pairs: [
        { a: "book_p1", b: "book_p2", sure: true, reason: "work_key" },
        { a: "book_p3", b: "book_p4", sure: false, reason: "author_only" },
      ],
    },
  );
  await page.goto("/");
  await expect(page.getByTestId("sf-pairs-bar")).toBeVisible();
  // Der Hinweis allein legt nichts zusammen.
  await page.waitForTimeout(500);
  expect(await calls(page, "library_join_editions")).toHaveLength(0);

  await page.getByTestId("sf-pairs-review").click();
  const checks = page.getByTestId("sf-pair-check");
  await expect(checks).toHaveCount(2);
  await expect(checks.nth(0)).toBeChecked();
  await expect(checks.nth(1)).not.toBeChecked();
  expect(await calls(page, "library_join_editions")).toHaveLength(0);

  await page.getByTestId("sf-pairs-confirm").click();
  await expect
    .poll(async () => (await calls(page, "library_join_editions")).length)
    .toBe(1);
  const [join] = (await calls(page, "library_join_editions")) as {
    targetId: string;
    sourceIds: string[];
  }[];
  expect([join.targetId, ...join.sourceIds].sort()).toEqual([
    "book_p1",
    "book_p2",
  ]);
});

test("„Alle bestätigen“ bleibt gesperrt, solange Gruppen offen sind", async ({
  page,
}) => {
  await setup(
    page,
    [
      twoLanguages("book_c", ["Das Urteil", "The Verdict"], DE3, EN2),
      twoLanguages("book_a", ["Der Prozess", "The Trial"], DE3, EN3),
    ],
    { pageText: PAGES },
  );
  await page.goto("/");

  // Ungleiche Kapitelzahlen (3 gegen 2): nur einzeln bestätigen.
  await play(page, "Das Urteil");
  await page.getByTestId("sf-switch-button").click();
  await page.getByTestId("sf-switch-align").click();
  const all = page.getByTestId("sf-align-confirm-all");
  await expect(page.getByTestId("sf-align-dialog")).toBeVisible();
  await expect(all).toBeDisabled();
  await page.keyboard.press("Escape");
  await page.getByTestId("library-audiobar-close").click();

  // Gleiche Zahlen, alles 1:1: erlaubt; danach ist alles bestätigt und speicherbar.
  await play(page, "Der Prozess");
  await page.getByTestId("sf-switch-button").click();
  await page.getByTestId("sf-switch-align").click();
  await expect(all).toBeEnabled();
  await expect(page.getByTestId("sf-align-count")).toContainText("0 von 3");
  await page.getByTestId("sf-align-confirm-0").click();
  await expect(page.getByTestId("sf-align-count")).toContainText("1 von 3");
  await all.click();
  await expect(page.getByTestId("sf-align-count")).toContainText("3 von 3");
  await expect(all).toBeDisabled();
  await page.getByTestId("sf-align-save").click();
  await expect
    .poll(async () => (await calls(page, "library_set_alignment")).length)
    .toBe(1);
  const [saved] = (await calls(page, "library_set_alignment")) as {
    alignment: { groups: { confirmed: boolean }[] };
  }[];
  expect(saved.alignment.groups.every((g) => g.confirmed)).toBe(true);
});

const seekTo = (page: Page, value: number) =>
  page.getByTestId("library-audiobar-seek").evaluate((el, v) => {
    const input = el as HTMLInputElement;
    const set = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!;
    set.call(input, String(v));
    input.dispatchEvent(new Event("input", { bubbles: true }));
  }, value);

const foreignReading = (page: Page, updated: number, offset: number) =>
  page.evaluate(
    ({ updated, offset }) => {
      const w = window as unknown as {
        __books: Rec[];
        __emit(event: string, payload: unknown): void;
      };
      w.__books = w.__books.map((b) =>
        b.id !== "book_a"
          ? b
          : {
              ...b,
              reading: {
                edition: "ed_book_a_en",
                updated: Date.now() + updated,
                device: "ios_beispiel",
                position: {
                  key: "audio:book_a:ed_book_a_en",
                  unit: "ms",
                  offset,
                },
              },
            },
      );
      w.__emit("sync-changed", null);
    },
    { updated, offset },
  );

const covers0 = (r: { heard: number[][] } | null) =>
  !!r?.heard.some(([a, b]) => a <= 0 && b >= 10000);

test("Zielfassung „fertig“: der Wechsel startet an der Zielstelle, Rückgängig stellt den Stand her", async ({
  page,
}) => {
  await setup(page, [bookA()], { pageText: PAGES });
  const finished = {
    offset: 0,
    length: 45000,
    heard: [[0, 10000]],
    read: [],
    unit: "ms",
    updated: 1,
    finished: true,
  };
  await page.addInitScript((rec) => {
    window.localStorage.setItem(
      "lva.ui.progress.audio:book_a:ed_book_a_en",
      JSON.stringify(rec),
    );
  }, finished);
  await page.goto("/");
  await play(page, "Der Prozess");
  await pick(page, "ed_book_a_en");
  await expect(page.getByTestId("sf-notice")).toContainText("kapitelgenau");
  const en = await progress(page, "audio:book_a:ed_book_a_en");
  expect(en.finished).toBeFalsy();
  expect(en.offset).toBeGreaterThanOrEqual(15000);
  await expect.poll(() => seconds(page)).toBeGreaterThanOrEqual(15);
  await page.getByTestId("sf-notice-undo").click();
  await expect(page.getByTestId("sf-switch-button")).toHaveAccessibleName(
    /Deutsch/,
  );
  await expect
    .poll(
      async () => (await progress(page, "audio:book_a:ed_book_a_en"))?.finished,
    )
    .toBe(true);
  expect(covers0(await progress(page, "audio:book_a:ed_book_a_en"))).toBe(true);
});

test("Konflikt: „Dorthin wechseln“ ohne lokales Hörbuch lässt die Wiedergabe unverändert", async ({
  page,
}) => {
  await setup(
    page,
    [twoLanguages("book_a", ["Der Prozess", "The Trial"], DE3, EN3, {}, false)],
    { pageText: PAGES },
  );
  await page.goto("/");
  await play(page, "Der Prozess");
  await page.evaluate(() => {
    document.querySelector("audio")!.setAttribute("data-marker", "same");
  });
  await foreignReading(page, 5000, 30000);
  await page.getByTestId("sf-notice-go").click();
  expect((await calls(page, "sync_now")).length).toBeGreaterThan(0);
  await expect(page.getByTestId("sf-switch-button")).toHaveAccessibleName(
    /Deutsch/,
  );
  await expect(page.locator("audio[data-marker='same']")).toHaveCount(1);
  await expect(page.getByText(/wird geladen/)).toBeVisible();
});

test("Konflikt: „Hier bleiben“ nimmt die Stelle beim Klick; ein anderes Buch räumt den Hinweis ab", async ({
  page,
}) => {
  await setup(
    page,
    [bookA(), twoLanguages("book_c", ["Das Urteil", "The Verdict"], DE3, EN3)],
    { pageText: PAGES },
  );
  await page.goto("/");
  await play(page, "Der Prozess");
  await foreignReading(page, 5000, 30000);
  await expect(page.getByTestId("sf-notice")).toContainText("iPhone");
  await seekTo(page, 50);
  await page.getByTestId("sf-notice-stay").click();
  await expect
    .poll(async () => {
      const last = (await calls(page, "library_set_reading")).at(-1) as
        { reading: { position: { offset: number } } } | undefined;
      return last?.reading.position.offset ?? 0;
    })
    .toBeGreaterThanOrEqual(50000);

  await foreignReading(page, 20000, 31000);
  await expect(page.getByTestId("sf-notice")).toContainText("iPhone");
  await page.getByTestId("library-audiobar-close").click();
  await play(page, "Das Urteil");
  await expect(page.getByTestId("sf-notice")).toBeHidden();
});

test("Aufräumen: Ersetzen und Endgültig löschen räumen den Fortschritt der Fassungen", async ({
  page,
}) => {
  const book = bookA();
  await setup(page, [book], { pageText: PAGES });
  await page.addInitScript(() => {
    for (const k of ["audio:book_a", "audio:book_a:ed_book_a_en"])
      window.localStorage.setItem(
        `lva.ui.progress.${k}`,
        JSON.stringify({
          offset: 5,
          length: 9,
          heard: [],
          read: [],
          unit: "ms",
          updated: 1,
        }),
      );
  });
  await page.goto("/");
  await page.evaluate(async (b) => {
    const mod = await import("/src/lib/library/importFile.ts");
    await mod.cleanupReplaced(b as never);
  }, book);
  expect(await progress(page, "audio:book_a")).toBeNull();
  expect(await progress(page, "audio:book_a:ed_book_a_en")).not.toBeNull();
  await page.evaluate(async (b) => {
    const mod = await import("/src/lib/library/remove.ts");
    await mod.purgeBooks([b as never]);
  }, book);
  expect(await progress(page, "audio:book_a:ed_book_a_en")).toBeNull();
});

/** Zielfassung ist gewechselt und hat eine eigene Hörstelle samt Bereich. */
const switchAndListen = async (
  page: Page,
  beforePick?: () => Promise<void>,
) => {
  await page.goto("/");
  await play(page, "Der Prozess");
  await beforePick?.();
  await pick(page, "ed_book_a_en");
  await expect(page.getByTestId("sf-notice")).toContainText("kapitelgenau");
  // Hören seit dem Wechsel: Bereich 15–16 s in der Zielfassung.
  await page.evaluate(() => {
    const k = "lva.ui.progress.audio:book_a:ed_book_a_en";
    const r = JSON.parse(window.localStorage.getItem(k)!);
    window.localStorage.setItem(
      k,
      JSON.stringify({ ...r, heard: [[15000, 16000]], offset: 16000 }),
    );
  });
};

const heardEn = async (page: Page) =>
  (await progress(page, "audio:book_a:ed_book_a_en"))?.heard as
    number[][] | undefined;

const covers = (heard: number[][] | undefined) =>
  !!heard?.some(([a, b]) => a <= 15000 && b >= 16000);

test("Rückgängig behält, was in der Zielfassung gehört wurde", async ({
  page,
}) => {
  await setup(page, [bookA()], { pageText: PAGES });
  await switchAndListen(page);
  await page.getByTestId("sf-notice-undo").click();
  await expect(page.getByTestId("sf-switch-button")).toHaveAccessibleName(
    /Deutsch/,
  );
  expect(covers(await heardEn(page))).toBe(true);
});

test("Rückgängig scheitert: Zielstand unverändert, Leiste auf der Zielfassung, Fehlerhinweis", async ({
  page,
}) => {
  await setup(page, [bookA()], { pageText: PAGES });
  await switchAndListen(page);
  await page.evaluate(() => {
    (window as unknown as { __failReading: boolean }).__failReading = true;
  });
  await page.getByTestId("sf-notice-undo").click();
  await expect(page.getByText("Speichern fehlgeschlagen")).toBeVisible();
  await expect(page.getByTestId("sf-switch-button")).toHaveAccessibleName(
    /English/,
  );
  await expect(page.getByTestId("library-audiobar")).toBeVisible();
  expect(covers(await heardEn(page))).toBe(true);
  expect(
    (await progress(page, "audio:book_a:ed_book_a_en")).offset,
  ).toBeGreaterThanOrEqual(15000);
});

test("Rückgängig ohne bestätigtes Sichern der Leiste: abgebrochen, nichts geändert", async ({
  page,
}) => {
  await page.clock.install({ time: new Date("2026-11-06T10:00:00") });
  await setup(page, [bookA()], { pageText: PAGES });
  await switchAndListen(page, () =>
    page.clock.pauseAt(new Date("2026-11-06T11:00:00")),
  );
  await page.evaluate(() => {
    (window as unknown as { __lvaHoldDispose: boolean }).__lvaHoldDispose =
      true;
  });
  const before = (await calls(page, "library_set_reading")).length;
  await page.getByTestId("sf-notice-undo").click();
  await page.clock.runFor(2500);
  await expect(page.getByText(/nicht gesichert/)).toBeVisible();
  await expect(page.getByTestId("sf-switch-button")).toHaveAccessibleName(
    /English/,
  );
  await expect(page.getByTestId("library-audiobar")).toBeVisible();
  expect((await calls(page, "library_set_reading")).length).toBe(before);
  expect(covers(await heardEn(page))).toBe(true);
});

test("Neuladen scheitert nach erfolgreichem Rückgängig: Leiste bleibt auf der Quellfassung", async ({
  page,
}) => {
  await setup(page, [bookA()], { pageText: PAGES });
  await page.goto("/");
  await play(page, "Der Prozess");
  await pick(page, "ed_book_a_en");
  await expect(page.getByTestId("sf-notice")).toContainText("kapitelgenau");
  await page.evaluate(() => {
    (window as unknown as { __failList: boolean }).__failList = true;
  });
  await page.getByTestId("sf-notice-undo").click();
  await expect(page.getByTestId("sf-switch-button")).toHaveAccessibleName(
    /Deutsch/,
  );
  await expect(page.getByTestId("library-audiobar")).toBeVisible();
  const last = (await calls(page, "library_set_reading")).at(-1) as {
    reading: { edition: string; position: { offset: number } };
  };
  expect(last.reading.edition).toBe("ed_book_a_de");
  await expect
    .poll(() => seconds(page))
    .toBeGreaterThanOrEqual(last.reading.position.offset / 1000 - 1);
});

test("Rückgängig: wirft ein Schreibschritt, stellt der Rollback den Stand davor wieder her", async ({
  page,
}) => {
  await setup(page, [bookA()], { pageText: PAGES });
  await page.goto("/");
  const keys = ["audio:book_a", "audio:book_a:ed_book_a_en"];
  const result = await page.evaluate(async (keys) => {
    const put = (k: string, offset: number) =>
      window.localStorage.setItem(
        `lva.ui.progress.${k}`,
        JSON.stringify({
          offset,
          length: 9000,
          heard: [],
          read: [],
          unit: "ms",
          updated: 5,
        }),
      );
    put(keys[0], 3000);
    put(keys[1], 7000);
    const before = keys.map((k) =>
      window.localStorage.getItem(`lva.ui.progress.${k}`),
    );
    const mod = await import("/src/lib/library/switching.ts");
    let failed = false;
    try {
      await mod.undoSwitch({
        bookId: "book_a",
        fromEdition: "ed_book_a_de",
        fromPosition: { key: keys[0], unit: "ms", offset: 1000 },
        targetKey: keys[1],
        // Kaputter Stand: das Schreiben des Zielstands wirft nach dem Entfernen.
        targetRecord: { offset: 1, length: 2, heard: 5 as never, read: [] },
        resumeKey: null,
        resumeBefore: null,
        wasPlaying: false,
      } as never);
    } catch {
      failed = true;
    }
    const after = keys.map((k) =>
      window.localStorage.getItem(`lva.ui.progress.${k}`),
    );
    return { failed, before, after };
  }, keys);
  expect(result.failed).toBe(true);
  const pick = (raw: string | null) => {
    const r = JSON.parse(raw!);
    return [r.offset, r.length, r.heard, r.read];
  };
  expect(result.after.map(pick)).toEqual(result.before.map(pick));
  expect(await calls(page, "library_set_reading")).toHaveLength(0);
});

// Hauptfassung ohne Hörbuch, englische Fassung mit Hörbuch: Die Buchseite
// zeigt es im Kopf an und macht es abspielbar (Kopf, Fassungszeile, Menü).
const textOnlyDe = (enLocal = true) => {
  const b = twoLanguages(
    "book_a",
    ["Der Prozess", "The Trial"],
    DE3,
    EN3,
    { audio: null, audio_path: null, audio_paths: [] },
    enLocal,
  );
  const eds = b.editions as Rec[];
  return {
    ...b,
    editions: [{ ...eds[0], audio: null, audio_paths: [] }, eds[1]],
  };
};

const openBook = (page: Page) =>
  page.getByRole("button", { name: "„Der Prozess“: Details öffnen" }).click();

test("Hörbuch nur in der englischen Fassung: Kopf zeigt es und spielt es ab", async ({
  page,
}) => {
  await setup(page, [textOnlyDe()], { pageText: PAGES });
  await page.goto("/");
  await openBook(page);
  await expect(page.getByTestId("library-fmt-audio")).toHaveText(
    /Hörbuch \(English\)/,
  );
  await expect(page.getByTestId("library-detail-play")).toHaveCount(0);
  const listen = page.getByTestId("library-detail-listen-edition");
  await expect(listen).toHaveText(/Hörbuch hören · English/);
  await listen.click();
  await expect(page.getByTestId("library-audiobar")).toBeVisible();
  await expect(page.getByTestId("sf-switch-button")).toHaveAccessibleName(
    /English/,
  );
});

test("Fassungszeile und Menü: Hörbuch der Fassung abspielen", async ({
  page,
}) => {
  await setup(page, [textOnlyDe()], { pageText: PAGES });
  await page.goto("/");
  await openBook(page);
  await expect(page.getByTestId("sf-edition-play-ed_book_a_de")).toHaveCount(0);
  const cell = page.getByTestId("sf-edition-play-ed_book_a_en");
  await expect(cell).toHaveAccessibleName(/Hörbuch English abspielen/);
  await cell.click();
  await expect(page.getByTestId("library-audiobar")).toBeVisible();
  await page.reload();
  await openBook(page);
  await expect(page.getByTestId("library-audiobar")).toHaveCount(0);

  await page.getByTestId("sf-edition-menu-ed_book_a_de").click();
  await expect(
    page.getByTestId("sf-edition-listen-ed_book_a_de"),
  ).toBeDisabled();
  await page.keyboard.press("Escape");
  await page.getByTestId("sf-edition-menu-ed_book_a_en").click();
  await page.getByTestId("sf-edition-listen-ed_book_a_en").click();
  await expect(page.getByTestId("library-audiobar")).toBeVisible();
  await expect(page.getByTestId("sf-switch-button")).toHaveAccessibleName(
    /English/,
  );
});

test("Englisches Hörbuch noch nicht auf dem Gerät: Abgleich statt leerer Hörleiste", async ({
  page,
}) => {
  await setup(page, [textOnlyDe(false)], { pageText: PAGES });
  await page.goto("/");
  await openBook(page);
  await page.getByTestId("library-detail-listen-edition").click();
  await expect(
    page.getByTestId("library-detail-listen-edition-hint"),
  ).toHaveText("Noch nicht auf diesem Gerät");
  await expect(
    page.getByTestId("library-detail-listen-edition"),
  ).toHaveAccessibleDescription("Noch nicht auf diesem Gerät");
  await expect(page.getByTestId("library-fmt-audio")).toHaveAttribute(
    "data-state",
    "none",
  );
  await expect(page.getByText(/noch nicht geladen/)).toBeVisible();
  await expect(page.getByTestId("library-audiobar")).toHaveCount(0);
  expect((await calls(page, "sync_now")).length).toBeGreaterThan(0);
});

test("Abgleich scheitert: Fehler statt „angestoßen“", async ({ page }) => {
  await setup(page, [textOnlyDe(false)], { pageText: PAGES });
  await page.addInitScript(() => {
    const w = window as unknown as {
      __TAURI_INTERNALS__: {
        invoke: (c: string, a?: unknown) => Promise<unknown>;
      };
    };
    const inner = w.__TAURI_INTERNALS__.invoke;
    w.__TAURI_INTERNALS__.invoke = async (c, a) => {
      if (c === "sync_now") throw "Hub nicht erreichbar";
      return inner(c, a);
    };
  });
  await page.goto("/");
  await openBook(page);
  await page.getByTestId("library-detail-listen-edition").click();
  await expect(page.getByText(/Abgleich ist fehlgeschlagen/)).toBeVisible();
  await expect(page.getByText(/Hub nicht erreichbar/)).toBeVisible();
  await expect(page.getByText(/Der Abgleich ist angestoßen/)).toHaveCount(0);
  await expect(page.getByTestId("library-audiobar")).toHaveCount(0);
});

test("Begonnenes Hörbuch der Fassung: Kopf sagt „Weiterhören“", async ({
  page,
}) => {
  await setup(page, [textOnlyDe()], { pageText: PAGES });
  await page.addInitScript(() =>
    window.localStorage.setItem(
      "lva.ui.progress.audio:book_a:ed_book_a_en",
      JSON.stringify({
        offset: 9000,
        length: 45000,
        heard: [[0, 9000]],
        read: [],
        unit: "ms",
        updated: Date.now() - 1000,
      }),
    ),
  );
  await page.goto("/");
  await openBook(page);
  await expect(page.getByTestId("library-detail-listen-edition")).toHaveText(
    /Weiterhören · English/,
  );
  await expect(
    page.getByTestId("library-detail-listen-edition-hint"),
  ).toHaveCount(0);
});

for (const width of [1440, 390])
  for (const theme of ["light", "dark"] as const)
    test(`Vorschau Hörbuch einer anderen Fassung ${width} ${theme}`, async ({
      page,
    }) => {
      test.skip(!process.env.LV_SHOT_DIR, "nur mit LV_SHOT_DIR");
      await setup(page, [textOnlyDe()], { pageText: PAGES });
      if (theme === "dark")
        await page.addInitScript(() => {
          const w = window as unknown as {
            __TAURI_INTERNALS__: {
              invoke: (c: string, a?: unknown) => Promise<unknown>;
            };
          };
          const inner = w.__TAURI_INTERNALS__.invoke;
          w.__TAURI_INTERNALS__.invoke = async (c, a) => {
            const r = await inner(c, a);
            return c === "get_app_settings" || c === "get_default_settings"
              ? { ...(r as object), theme: "dark" }
              : r;
          };
        });
      await page.setViewportSize({ width, height: width < 500 ? 844 : 900 });
      await page.goto("/");
      await openBook(page);
      const tag = `${width}-${theme === "dark" ? "dunkel" : "hell"}`;
      const dir = process.env.LV_SHOT_DIR!;
      await page.waitForTimeout(300);
      await page.screenshot({
        path: `${dir}/fassung-hoerbuch-01-kopf-${tag}.png`,
      });
      await page.getByTestId("sf-editions").scrollIntoViewIfNeeded();
      await page.screenshot({
        path: `${dir}/fassung-hoerbuch-02-zeile-${tag}.png`,
      });
      await page.getByTestId("sf-edition-menu-ed_book_a_en").click();
      await page.waitForTimeout(200);
      await page.screenshot({
        path: `${dir}/fassung-hoerbuch-03-menue-${tag}.png`,
      });
      await page.keyboard.press("Escape");
      await page.getByTestId("library-detail-listen-edition").click();
      await expect(page.getByTestId("library-audiobar")).toBeVisible();
      await page.waitForTimeout(500);
      await page.screenshot({
        path: `${dir}/fassung-hoerbuch-04-hoerleiste-${tag}.png`,
      });
    });

for (const width of [1440, 390])
  test(`Vorschau Hörbuch nicht auf dem Gerät ${width}`, async ({ page }) => {
    test.skip(!process.env.LV_SHOT_DIR, "nur mit LV_SHOT_DIR");
    await setup(page, [textOnlyDe(false)], { pageText: PAGES });
    await page.setViewportSize({ width, height: width < 500 ? 844 : 900 });
    await page.goto("/");
    await openBook(page);
    const dir = process.env.LV_SHOT_DIR!;
    await page
      .getByTestId("library-detail-listen-edition")
      .scrollIntoViewIfNeeded();
    await page.getByTestId("library-detail-listen-edition").click();
    await expect(page.getByText(/noch nicht geladen/)).toBeVisible();
    await page.screenshot({
      path: `${dir}/fassung-hoerbuch-05-nicht-lokal-${width}-hell.png`,
    });
  });
