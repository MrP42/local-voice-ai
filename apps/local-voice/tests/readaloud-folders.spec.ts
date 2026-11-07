import { test, expect, type Page } from "@playwright/test";

// lva#365 P1: Ordnerleiste im Vorlesen (Alle, Buch-Ordner, Texte) und die
// Ansichten der offenen Seite als senkrechte Reiter in der Liste (E2).
// Attrappe nach readaloud-standards.spec.ts, dazu ein Buch mit zwei Fassungen
// (p1 Deutsch, p3 Englisch) und ein loser Text (p2).

const edition = (id: string, lang: string, page: string) => ({
  id,
  lang,
  title: "Der Sturm",
  subtitle: null,
  isbn: null,
  work_key: null,
  text: { page_id: page, origin: "document", source_name: null },
  audio: null,
  audio_key: "",
});
// Hörbuch der deutschen Fassung: zwei Teile, zusammen 1 h 30 min.
const AUDIO = {
  file: "teil_001.mp3",
  duration_ms: 3_600_000,
  narrator: null,
  transcript_job: null,
  tracks: [
    {
      file: "teil_001.mp3",
      title: "Teil 1",
      duration_ms: 3_600_000,
      transcript_job: null,
    },
    {
      file: "teil_002.mp3",
      title: "Teil 2",
      duration_ms: 1_800_000,
      transcript_job: null,
    },
  ],
};
const BOOKS = [
  {
    id: "b1",
    title: "Der Sturm",
    subtitle: null,
    authors: ["W. S."],
    language: "de",
    text: { page_id: "p1", origin: "document", source_name: null },
    audio: AUDIO,
    editions: [
      { ...edition("e1", "de", "p1"), audio: AUDIO },
      edition("e2", "en", "p3"),
    ],
    cover_path: null,
  },
];

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(
    ([BOOKS, THEME]) => {
      const callbacks = new Map<number, unknown>();
      let callback = 0;
      const settings = {
        onboarding_completed: true,
        app_language: "de",
        theme: THEME,
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
        tts_voice: null,
        tts_engine: "fish",
        tts_piper_voice: null,
      };
      const now = Date.now();
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
          convertFileSrc: (path: string) => path,
          invoke: async (cmd: string, args?: Record<string, unknown>) => {
            if (cmd === "get_app_settings" || cmd === "get_default_settings")
              return settings;
            if (cmd === "plugin:os|locale") return "de-DE";
            if (cmd === "plugin:app|version") return "0.16.0";
            if (cmd.includes("permission")) return true;
            if (cmd === "plugin:event|listen") {
              // Ereignisse für Tests auslösbar machen: __lvEmit("sync-changed").
              const w = window as unknown as {
                __lvEmit?: (event: string) => void;
                __lvHandlers?: Map<string, number[]>;
              };
              w.__lvHandlers ??= new Map();
              const name = String(args?.event);
              w.__lvHandlers.set(name, [
                ...(w.__lvHandlers.get(name) ?? []),
                Number(args?.handler),
              ]);
              w.__lvEmit ??= (event: string) => {
                for (const id of w.__lvHandlers?.get(event) ?? [])
                  (callbacks.get(id) as ((e: unknown) => void) | undefined)?.({
                    event,
                    id: 0,
                    payload: null,
                  });
              };
              return ++callback;
            }
            if (cmd === "get_selected_model") return "";
            if (cmd === "meetings_is_recording") return false;
            if (cmd === "tts_server_status")
              return { phase: "stopped", message: null };
            if (cmd === "library_list") {
              const w = window as unknown as {
                __lvBooks?: unknown;
                __lvLibSlow?: number;
                __lvLibCalls?: number;
                __lvLibFail?: boolean;
              };
              const books = w.__lvBooks ?? BOOKS;
              w.__lvLibCalls = (w.__lvLibCalls ?? 0) + 1;
              if (w.__lvLibFail) throw new Error("Bibliothek nicht erreichbar");
              // Langsame Bibliothek: Antwort erst nach __lvLibSlow ms.
              if (w.__lvLibSlow)
                await new Promise((r) => setTimeout(r, w.__lvLibSlow));
              return books;
            }
            if (cmd === "plugin:dialog|open") {
              // Dateidialog des Dokument-Imports: gezählt, ohne Auswahl beendet.
              const w = window as unknown as { __lvDialog?: number };
              w.__lvDialog = (w.__lvDialog ?? 0) + 1;
              return null;
            }
            if (cmd === "page_state_load") {
              const w = window as unknown as { __lvText?: string };
              return w.__lvText
                ? JSON.stringify({ text: w.__lvText, tab: "original" })
                : null;
            }
            if (cmd === "book_ai_documents") {
              const doc = (id: string, title: string, language: string) => ({
                id,
                title,
                template_id: "kurzbuch",
                template_title: "Kurzbuch",
                source: { kind: "book" },
                source_label: "Ganzes Buch",
                created_ms: now,
                modified_ms: now,
                words: 120,
                model: "test",
                usage: null,
                language,
              });
              const lists: Record<string, unknown[]> = {
                p1: [doc("d1", "Kurzbuch – Sturm", "de")],
                p3: [doc("d2", "Begriffe", "en")],
              };
              return lists[String(args?.pageId)] ?? [];
            }
            if (cmd === "book_ai_document_text") {
              const w = window as unknown as { __lvDocOpened?: string[] };
              w.__lvDocOpened = [
                ...(w.__lvDocOpened ?? []),
                `${args?.pageId}:${args?.id}`,
              ];
              return "# Kurzbuch\n\nEin kurzer Text.";
            }
            if (cmd === "pages_delete") {
              const w = window as unknown as { __lvDeleted?: string[] };
              w.__lvDeleted = [...(w.__lvDeleted ?? []), String(args?.id)];
              return null;
            }
            if (cmd === "pages_create") {
              // Neue Seite p4 (Fassung eines zweiten Buchs, siehe Test).
              (window as unknown as { __lvP4?: boolean }).__lvP4 = true;
              return {
                id: "p4",
                title: "Neues Buch",
                preview: "",
                modified_ms: now,
              };
            }
            if (cmd === "pages_list") {
              const gone =
                (window as unknown as { __lvDeleted?: string[] }).__lvDeleted ??
                [];
              return [
                ...((window as unknown as { __lvP4?: boolean }).__lvP4
                  ? [
                      {
                        id: "p4",
                        title: "Neues Buch",
                        preview: "",
                        modified_ms: now,
                      },
                    ]
                  : []),
                {
                  id: "p1",
                  title: "Der Sturm",
                  preview: "Es war eine dunkle Nacht.",
                  modified_ms: now - 20 * 60_000,
                },
                {
                  id: "p2",
                  title: "Zweite Seite",
                  preview: "Noch ein Text.",
                  modified_ms: now - 3 * 86_400_000,
                },
                {
                  id: "p3",
                  title: "The Tempest",
                  preview: "It was a dark night.",
                  modified_ms: now - 60_000,
                },
              ].filter((p) => !gone.includes(p.id));
            }
            if (cmd === "page_files")
              return [
                {
                  name: "Der-Sturm_2026-09-08_1405.wav",
                  size: 4_500_000,
                  modified_ms: now,
                },
                { name: "notizen.txt", size: 12, modified_ms: now },
              ];
            if (cmd === "page_dir") return "C:/projects/p1";
            if (cmd === "page_audio_note")
              return args?.name === "Der-Sturm_2026-09-08_1405.wav"
                ? {
                    text: "Es war eine dunkle Nacht. Niemand sprach.",
                    voice: "erzaehlerin",
                    seed: 42,
                    // 08.09.2025, mittags UTC: in jeder Zeitzone derselbe Tag.
                    created_ms: 1757333100000,
                    segments: [
                      {
                        text: "Es war eine dunkle Nacht.",
                        voice: "erzaehlerin",
                        start_ms: 0,
                        end_ms: 2000,
                      },
                      {
                        text: "Niemand sprach.",
                        voice: "leo-lausemaus",
                        start_ms: 2000,
                        end_ms: 4000,
                      },
                    ],
                  }
                : null;
            if (
              cmd === "tts_list_voices" ||
              cmd === "tts_list_voice_infos" ||
              cmd === "llm_ps" ||
              cmd === "tts_reading_list" ||
              cmd.includes("history") ||
              cmd.includes("models") ||
              cmd.includes("devices") ||
              cmd === "meetings_list"
            )
              return [];
            if (cmd === "get_custom_sounds")
              return { start: false, stop: false };
            return null;
          },
        },
      });
    },
    [BOOKS, process.env.LV_THEME ?? "light"] as const,
  );
});

async function openReadAloud(page: Page, width = 1920) {
  await page.setViewportSize({ width, height: 1050 });
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await expect(page.getByTestId("tts-pages")).toBeVisible();
}

const rows = (page: Page) =>
  page.getByTestId("tts-pages").locator("span.min-w-0.truncate.text-sm");

test("Ordnerleiste: Alle, Buch-Ordner mit Fassungen, Texte ohne Buch", async ({
  page,
}) => {
  await openReadAloud(page);
  const folders = page.getByTestId("reader-folders");
  await expect(folders.locator('[data-folder="all"]')).toContainText("3");
  await expect(folders.locator('[data-folder="book:b1"]')).toContainText(
    "Der Sturm",
  );
  await expect(folders.locator('[data-folder="book:b1"]')).toContainText("2");
  await expect(folders.locator('[data-folder="texts"]')).toContainText("1");

  await folders.locator('[data-folder="book:b1"]').click();
  await expect(rows(page)).toHaveText(["Der Sturm", "The Tempest"]);
  // Sprachkennung der Fassung an der Zeile.
  await expect(
    page.getByTestId("tts-pages").getByText("en", { exact: true }),
  ).toBeVisible();

  // Ein Ordner ohne die offene Seite öffnet seine erste Seite.
  await folders.locator('[data-folder="texts"]').click();
  await expect(rows(page)).toHaveText(["Zweite Seite"]);
  await expect(folders.locator('[data-folder="texts"]')).toHaveAttribute(
    "aria-current",
    "true",
  );

  // Bedienziele mindestens 44 px hoch.
  for (const key of ["all", "book:b1", "texts"]) {
    const box = await folders.locator(`[data-folder="${key}"]`).boundingBox();
    expect(box!.height).toBeGreaterThanOrEqual(44);
  }
});

test("Ein gewählter leerer Ordner bleibt gewählt", async ({ page }) => {
  // Der lose Text p2 gehört hier als dritte Fassung ebenfalls zum Buch:
  // „Ohne Buch“ ist leer.
  await page.addInitScript(
    (books) => {
      (window as unknown as { __lvBooks?: unknown }).__lvBooks = books;
    },
    [
      {
        ...BOOKS[0],
        editions: [...BOOKS[0].editions, edition("e3", "fr", "p2")],
      },
    ],
  );
  await openReadAloud(page);
  const folders = page.getByTestId("reader-folders");
  const texts = folders.locator('[data-folder="texts"]');
  await expect(texts).toContainText("0");
  await texts.click();
  await expect(texts).toHaveAttribute("aria-current", "true");
  await page.waitForTimeout(500);
  await expect(texts).toHaveAttribute("aria-current", "true");
  await expect(rows(page)).toHaveCount(0);
  // Die Ansichten der offenen Seite bleiben im Kopf erreichbar.
  await expect(
    page.getByRole("tab", { name: "Original", exact: true }),
  ).toBeVisible();
});

test("Seitenwechsel bei verzögerter Bibliotheksantwort", async ({ page }) => {
  // Die Bibliothek kennt zunächst keine Bücher: alle drei Seiten sind lose
  // Texte. Gelöscht wird die offene Seite p2; während die nächste, langsame
  // Bücherantwort aussteht, öffnet der Nachbar. Erst die Antwort zeigt,
  // dass der Nachbar eine Fassung von „Der Sturm“ ist: die Auswahl folgt dorthin,
  // und die offene Seite bleibt in der Liste sichtbar.
  await page.addInitScript(() => {
    (window as unknown as { __lvBooks?: unknown }).__lvBooks = [];
    localStorage.setItem("lva.ui.tts.folder", "texts");
    localStorage.setItem("lva.ui.tts.activePage", "p2");
  });
  await openReadAloud(page);
  const folders = page.getByTestId("reader-folders");
  await expect(folders.locator('[data-folder="texts"]')).toHaveAttribute(
    "aria-current",
    "true",
  );
  await expect(rows(page)).toHaveCount(3);
  await page.evaluate((books) => {
    const w = window as unknown as {
      __lvBooks?: unknown;
      __lvLibSlow?: number;
    };
    w.__lvBooks = books;
    w.__lvLibSlow = 1500;
  }, BOOKS);
  const row = page
    .getByTestId("tts-pages")
    .locator("[data-page-id='p2'], :scope *")
    .filter({ hasText: "Zweite Seite" })
    .first();
  await row.hover();
  await page
    .getByTestId("tts-pages")
    .getByRole("button", { name: "Seite löschen" })
    .first()
    .click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Seite löschen" })
    .click();
  await expect(folders.locator('[data-folder="book:b1"]')).toHaveAttribute(
    "aria-current",
    "true",
    { timeout: 5000 },
  );
  await expect(rows(page)).toHaveText(["Der Sturm", "The Tempest"]);
  // Der sichtbare Nachbar (p1 oder p3) ist offen; beide sind Fassungen.
  expect(["p1", "p3"]).toContain(
    await page.evaluate(() => localStorage.getItem("lva.ui.tts.activePage")),
  );
});

const SECOND_BOOK = {
  ...BOOKS[0],
  id: "b2",
  title: "Neues Buch",
  text: { page_id: "p4", origin: "document", source_name: null },
  editions: [{ ...edition("e4", "de", "p4"), title: "Neues Buch" }],
};

test("Neue Seite eines anderen Buchs: Auswahl folgt erst mit den neuen Büchern", async ({
  page,
}) => {
  // Buch A ist gewählt. Eine neue Seite entsteht und öffnet; erst die
  // (langsame) Bücherantwort zeigt, dass sie zu Buch B gehört. Mit den alten
  // Büchern wäre sie „Ohne Buch“ zugeordnet worden.
  await openReadAloud(page);
  const folders = page.getByTestId("reader-folders");
  await folders.locator('[data-folder="book:b1"]').click();
  await expect(rows(page)).toHaveText(["Der Sturm", "The Tempest"]);
  await page.evaluate(
    (books) => {
      const w = window as unknown as {
        __lvBooks?: unknown;
        __lvLibSlow?: number;
      };
      w.__lvBooks = books;
      w.__lvLibSlow = 1500;
    },
    [BOOKS[0], SECOND_BOOK],
  );
  await page
    .getByTestId("tts-pages")
    .getByRole("button", { name: "Neue Seite" })
    .click();
  await expect(folders.locator('[data-folder="book:b2"]')).toHaveAttribute(
    "aria-current",
    "true",
    { timeout: 5000 },
  );
  await expect(rows(page)).toHaveText(["Neues Buch"]);
  await expect(folders.locator('[data-folder="texts"]')).not.toHaveAttribute(
    "aria-current",
    "true",
  );
});

test("Eigene Ordnerwahl während einer laufenden Bücheranfrage bleibt", async ({
  page,
}) => {
  // p2 ist Fassung eines zweiten Buchs: „Ohne Buch“ ist leer. Während eine
  // Sync-Anfrage läuft, wechselt die offene Seite im Buch und danach wählt
  // jemand bewusst den leeren Ordner. Die Antwort darf das nicht überschreiben.
  await page.addInitScript(
    (books) => {
      (window as unknown as { __lvBooks?: unknown }).__lvBooks = books;
    },
    [
      BOOKS[0],
      {
        ...SECOND_BOOK,
        text: { page_id: "p2", origin: "document", source_name: null },
        editions: [{ ...edition("e5", "de", "p2"), title: "Zweite Seite" }],
      },
    ],
  );
  await openReadAloud(page);
  const folders = page.getByTestId("reader-folders");
  await folders.locator('[data-folder="book:b1"]').click();
  await expect(rows(page)).toHaveText(["Der Sturm", "The Tempest"]);
  await rows(page).filter({ hasText: "Der Sturm" }).click();
  await page.evaluate(() => {
    const w = window as unknown as {
      __lvLibSlow?: number;
      __lvEmit?: (event: string) => void;
    };
    w.__lvLibSlow = 1500;
    w.__lvEmit?.("sync-changed");
  });
  await rows(page).filter({ hasText: "The Tempest" }).click();
  const texts = folders.locator('[data-folder="texts"]');
  await expect(texts).toContainText("0");
  await texts.click();
  await expect(texts).toHaveAttribute("aria-current", "true");
  await page.waitForTimeout(2500);
  await expect(texts).toHaveAttribute("aria-current", "true");
});

test("Gescheiterte Bücheranfrage: Auswahl bleibt, folgt nach Erfolg", async ({
  page,
}) => {
  await openReadAloud(page);
  const folders = page.getByTestId("reader-folders");
  await folders.locator('[data-folder="book:b1"]').click();
  await expect(rows(page)).toHaveText(["Der Sturm", "The Tempest"]);
  await page.evaluate(
    (books) => {
      const w = window as unknown as {
        __lvBooks?: unknown;
        __lvLibFail?: boolean;
      };
      w.__lvBooks = books;
      w.__lvLibFail = true;
    },
    [BOOKS[0], SECOND_BOOK],
  );
  await page
    .getByTestId("tts-pages")
    .getByRole("button", { name: "Neue Seite" })
    .click();
  await expect
    .poll(() =>
      page.evaluate(() => localStorage.getItem("lva.ui.tts.activePage")),
    )
    .toBe("p4");
  await page.waitForTimeout(800);
  // Mit veralteten Büchern wäre p4 „Ohne Buch“: die Auswahl bleibt stehen.
  await expect(folders.locator('[data-folder="book:b1"]')).toHaveAttribute(
    "aria-current",
    "true",
  );
  // Die nächste Anfrage gelingt: jetzt folgt die Auswahl in Buch B.
  await page.evaluate(() => {
    const w = window as unknown as {
      __lvLibFail?: boolean;
      __lvEmit?: (event: string) => void;
    };
    w.__lvLibFail = false;
    w.__lvEmit?.("sync-changed");
  });
  await expect(folders.locator('[data-folder="book:b2"]')).toHaveAttribute(
    "aria-current",
    "true",
  );
  await expect(rows(page)).toHaveText(["Neues Buch"]);
});

test("Keine Anfrageschleife: libraryList-Aufrufe bleiben über Renders konstant", async ({
  page,
}) => {
  await openReadAloud(page);
  const folders = page.getByTestId("reader-folders");
  await folders.locator('[data-folder="book:b1"]').click();
  await page.waitForTimeout(500);
  const count = () =>
    page.evaluate(
      () => (window as unknown as { __lvLibCalls?: number }).__lvLibCalls ?? 0,
    );
  const before = await count();
  // Viele Renders ohne neue Seitenmenge: Reiter und Ordner wechseln. (Ein
  // Seitenwechsel lädt anderswo im Vorlesen die Bibliothek ohnehin neu und
  // würde hier mitgezählt.)
  const sections = page.getByTestId("page-sections");
  for (let i = 0; i < 3; i++) {
    await sections
      .getByRole("tab", { name: "Übersetzung", exact: true })
      .click();
    await sections.getByRole("tab", { name: "Original", exact: true }).click();
    await folders.locator('[data-folder="all"]').click();
    await folders.locator('[data-folder="book:b1"]').click();
  }
  await folders.locator('[data-folder="all"]').click();
  await page.waitForTimeout(1000);
  expect(await count()).toBe(before);
});

test("Ordnerauswahl bleibt über ein Neuladen erhalten", async ({ page }) => {
  await openReadAloud(page);
  const folders = page.getByTestId("reader-folders");
  await folders.locator('[data-folder="book:b1"]').click();
  await expect(rows(page)).toHaveText(["Der Sturm", "The Tempest"]);
  await page.reload();
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await expect(
    page.getByTestId("reader-folders").locator('[data-folder="book:b1"]'),
  ).toHaveAttribute("aria-current", "true");
  await expect(rows(page)).toHaveText(["Der Sturm", "The Tempest"]);
});

test("Bedienziele und gedämpfter Text bei 390 und 768 px", async ({ page }) => {
  // WCAG-Kontrast aus berechneten Farben (Alpha gegen die Fläche gemischt).
  const measure = () =>
    page.evaluate(() => {
      const rgba = (c: string) => {
        const m = c.match(/[\d.]+/g)!.map(Number);
        return [m[0], m[1], m[2], m[3] ?? 1];
      };
      const bgOf = (el: Element | null): number[] => {
        while (el) {
          const c = rgba(getComputedStyle(el).backgroundColor);
          if (c[3] > 0.99) return c;
          el = el.parentElement;
        }
        return [255, 255, 255, 1];
      };
      const lum = (c: number[]) => {
        const f = (v: number) => {
          v /= 255;
          return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
        };
        return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]);
      };
      const ratio = (el: Element) => {
        const fg = rgba(getComputedStyle(el).color);
        const bg = bgOf(el);
        const mix = fg
          .slice(0, 3)
          .map((v, i) => v * fg[3] + bg[i] * (1 - fg[3]));
        const [a, b] = [lum(mix), lum(bg)].sort((x, y) => y - x);
        return (a + 0.05) / (b + 0.05);
      };
      const scope = document.querySelector('[data-testid="tts-pages"]')!;
      const small = [
        ...scope.querySelectorAll("button, [role=tab], .app-select__control"),
      ]
        .filter((el) => {
          const r = el.getBoundingClientRect();
          return r.width > 0 && r.height > 0 && (r.width < 24 || r.height < 24);
        })
        .map((el) => el.getAttribute("aria-label") ?? el.textContent);
      const muted = [...scope.querySelectorAll(".text-text\\/60")]
        .filter((el) => el.getBoundingClientRect().height > 0)
        .map(ratio);
      return { small, minMuted: Math.min(...muted, 99) };
    });
  for (const width of [768, 390]) {
    await openReadAloud(page, 768);
    if (width < 768) await page.setViewportSize({ width, height: 844 });
    const result = await measure();
    expect(result.small).toEqual([]);
    expect(result.minMuted).toBeGreaterThanOrEqual(4.5);
  }
});

test("Ansichten der Seite stehen senkrecht unter ihrer Zeile, nicht im Kopf", async ({
  page,
}) => {
  await openReadAloud(page);
  const sections = page.getByTestId("page-sections");
  const original = sections.getByRole("tab", { name: "Original", exact: true });
  const translation = sections.getByRole("tab", {
    name: "Übersetzung",
    exact: true,
  });
  await expect(original).toHaveAttribute("aria-selected", "true");
  // Genau eine Reiterleiste ist sichtbar.
  await expect(
    page.getByRole("tab", { name: "Original", exact: true }),
  ).toHaveCount(1);
  await original.focus();
  await page.keyboard.press("ArrowDown");
  await expect(translation).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("ArrowUp");
  await expect(original).toHaveAttribute("aria-selected", "true");
  expect((await original.boundingBox())!.height).toBeGreaterThanOrEqual(44);
});

test("Umbruch-Layout: Ordner als Auswahl, Ansichten wieder im Kopf", async ({
  page,
}) => {
  await openReadAloud(page, 900);
  await expect(page.getByTestId("reader-folders")).toBeHidden();
  const select = page
    .getByTestId("reader-folders-compact")
    .locator(".app-select__control");
  await expect(select).toBeVisible();
  expect((await select.boundingBox())!.height).toBeGreaterThanOrEqual(44);
  await select.click();
  await page.getByRole("option", { name: "Der Sturm (2)" }).click();
  await expect(rows(page)).toHaveText(["Der Sturm", "The Tempest"]);
  await expect(page.getByTestId("page-sections")).toBeHidden();
  await expect(
    page.getByRole("tab", { name: "Original", exact: true }),
  ).toHaveCount(1);
  await expect(
    page.getByRole("tab", { name: "Original", exact: true }),
  ).toBeVisible();
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - window.innerWidth,
  );
  expect(overflow).toBeLessThanOrEqual(0);
});

const bookHead = (page: Page) => page.getByTestId("book-folder-head");
const pagesCol = (page: Page) => page.getByTestId("tts-pages");
const openBook = async (page: Page) => {
  await page
    .getByTestId("reader-folders")
    .locator('[data-folder="book:b1"]')
    .click();
  await expect(bookHead(page)).toBeVisible();
};
const activePage = (page: Page) =>
  page.evaluate(() => localStorage.getItem("lva.ui.tts.activePage"));

test("Buch-Ordner: Kopf, Gruppen und Beschriftungen", async ({ page }) => {
  await openReadAloud(page);
  await openBook(page);
  const head = bookHead(page);
  await expect(head).toContainText("Der Sturm");
  await expect(head).toContainText("W. S.");
  await expect(head).toContainText("2 Fassungen");
  await expect(head.getByRole("button", { name: "Weiterlesen" })).toBeVisible();

  const col = pagesCol(page);
  // Kleine Überschriften in der Folge Fassungen, Hörbuch, KI-Dokumente.
  await expect(col.locator("p.tts-pages__list, section > p")).toHaveText([
    "Fassungen",
    "Hörbuch",
    "KI-Dokumente",
  ]);

  // Fassungen: die bestehenden Seitenzeilen, mit Sprache und Hauptfassung.
  await expect(rows(page)).toHaveText(["Der Sturm", "The Tempest"]);
  await expect(
    col.getByText("Deutsch · Hauptfassung", { exact: true }),
  ).toBeVisible();
  await expect(col.getByText("English", { exact: true })).toBeVisible();

  // Hörbuch: zwei Teile, 3600 s + 1800 s.
  const audio = page.getByTestId("book-audio");
  await expect(audio).toHaveCount(1);
  await expect(audio).toContainText("Hörbuch Deutsch");
  await expect(audio).toContainText("2 Teile · 1 h 30 min");

  // KI-Dokumente je Fassungsseite, mit Fassung.
  const docs = page.getByTestId("book-ai-doc");
  await expect(docs).toHaveCount(2);
  await expect(docs.nth(0)).toContainText("Kurzbuch – Sturm");
  await expect(docs.nth(0)).toContainText("Fassung Deutsch (Haupt)");
  await expect(docs.nth(1)).toContainText("Begriffe");
  await expect(docs.nth(1)).toContainText("Fassung English");

  // Bedienziele in den neuen Zeilen mindestens 44 px hoch.
  for (const el of [
    head.getByRole("button", { name: "Weiterlesen" }),
    audio,
    docs.nth(0),
    docs.nth(1),
  ])
    expect((await el.boundingBox())!.height).toBeGreaterThanOrEqual(44);

  // „Alle“ und „Ohne Buch“ bleiben flache Listen.
  const folders = page.getByTestId("reader-folders");
  await folders.locator('[data-folder="all"]').click();
  await expect(bookHead(page)).toHaveCount(0);
  await expect(page.getByTestId("book-ai-doc")).toHaveCount(0);
  await expect(rows(page)).toHaveCount(3);
  await folders.locator('[data-folder="texts"]').click();
  await expect(bookHead(page)).toHaveCount(0);
  await expect(rows(page)).toHaveText(["Zweite Seite"]);
});

test("Buch-Ordner: Weiterlesen öffnet Lesefassung, sonst Hauptfassung", async ({
  page,
}) => {
  await openReadAloud(page);
  await openBook(page);
  await bookHead(page).getByRole("button", { name: "Weiterlesen" }).click();
  await expect.poll(() => activePage(page)).toBe("p1");

  await page.evaluate(
    (books) => {
      (window as unknown as { __lvBooks?: unknown }).__lvBooks = books;
      (window as unknown as { __lvEmit?: (e: string) => void }).__lvEmit?.(
        "sync-changed",
      );
    },
    [
      {
        ...BOOKS[0],
        reading: { edition: "e2", updated: 1, position: null },
      },
    ],
  );
  await expect
    .poll(async () => {
      await bookHead(page).getByRole("button", { name: "Weiterlesen" }).click();
      return activePage(page);
    })
    .toBe("p3");
});

test("Buch-Ordner: Hörbuch öffnet die Textseite seiner Fassung", async ({
  page,
}) => {
  await openReadAloud(page);
  await openBook(page);
  await rows(page).filter({ hasText: "The Tempest" }).click();
  await expect.poll(() => activePage(page)).toBe("p3");
  await page.getByTestId("book-audio").click();
  await expect.poll(() => activePage(page)).toBe("p1");

  // Ohne Textseite der Fassung ist die Zeile nicht klickbar.
  await page.evaluate(
    (books) => {
      (window as unknown as { __lvBooks?: unknown }).__lvBooks = books;
      (window as unknown as { __lvEmit?: (e: string) => void }).__lvEmit?.(
        "sync-changed",
      );
    },
    [
      {
        ...BOOKS[0],
        editions: [
          { ...BOOKS[0].editions[0], text: null },
          BOOKS[0].editions[1],
          { ...edition("e9", "fr", "p1"), audio: AUDIO },
        ],
      },
    ],
  );
  await expect(page.getByTestId("book-audio").first()).toBeVisible();
  await expect(page.getByTestId("book-audio").first()).toContainText(
    "Hörbuch Deutsch",
  );
  await expect(page.getByTestId("book-audio").first()).not.toHaveJSProperty(
    "tagName",
    "BUTTON",
  );
});

test("Klick auf ein KI-Dokument öffnet Seite, Reiter und Dokument", async ({
  page,
}) => {
  await openReadAloud(page);
  await openBook(page);
  const docs = page.getByTestId("book-ai-doc");
  const sections = page.getByTestId("page-sections");
  // Dokument einer anderen Fassung: Seite p3 öffnen, Reiter „KI-Dokumente“.
  await docs.nth(1).click();
  await expect.poll(() => activePage(page)).toBe("p3");
  await expect(
    sections.getByRole("tab", { name: "KI-Dokumente", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { __lvDocOpened?: string[] }).__lvDocOpened,
      ),
    )
    .toContain("p3:d2");
  await expect(docs.nth(1)).toHaveAttribute("aria-current", "true");
  await expect(docs.nth(0)).not.toHaveAttribute("aria-current", "true");

  // Dokument der Fassung, die schon offen ist? Hier wieder die andere Seite.
  await docs.nth(0).click();
  await expect.poll(() => activePage(page)).toBe("p1");
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { __lvDocOpened?: string[] }).__lvDocOpened,
      ),
    )
    .toContain("p1:d1");
  await expect(docs.nth(0)).toHaveAttribute("aria-current", "true");
  await expect(
    sections.getByRole("tab", { name: "KI-Dokumente", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
});

test("Quelle fehlt: Fassung bleibt sichtbar als „Datei fehlt“, nichts wird gelöscht", async ({
  page,
}) => {
  await page.addInitScript(
    (books) => {
      (window as unknown as { __lvBooks?: unknown }).__lvBooks = books;
    },
    [
      {
        ...BOOKS[0],
        editions: [...BOOKS[0].editions, edition("e3", "fr", "p9")],
      },
    ],
  );
  await openReadAloud(page);
  await openBook(page);
  const missing = page.getByTestId("edition-missing");
  await expect(missing).toHaveCount(1);
  await expect(missing).toContainText("Datei fehlt");
  await expect(missing).toContainText("Nichts wurde gelöscht.");
  // Keine Löschaktion; die Reparatur folgt mit P2.
  await expect(missing.getByRole("button")).toHaveCount(0);
  await expect(rows(page)).toHaveText(["Der Sturm", "The Tempest"]);
  await expect(page.getByTestId("book-folder-head")).toContainText(
    "3 Fassungen",
  );
  expect(
    await page.evaluate(
      () => (window as unknown as { __lvDeleted?: string[] }).__lvDeleted,
    ),
  ).toBeUndefined();
});

test("Leerzustand ohne Bücher: Hinweis und „Buch hinzufügen“", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as unknown as { __lvBooks?: unknown }).__lvBooks = [];
  });
  await openReadAloud(page);
  const empty = page.getByTestId("reader-folders-empty");
  await expect(empty).toContainText("Noch keine Bücher");
  await expect(empty).toContainText("Ziehe ein EPUB, PDF, DOCX oder Hörbuch");
  const add = empty.getByRole("button", { name: "Buch hinzufügen" });
  expect((await add.boundingBox())!.height).toBeGreaterThanOrEqual(44);
  await add.click();
  // Der vorhandene Dokument-Import (Dateidialog) öffnet.
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { __lvDialog?: number }).__lvDialog ?? 0,
      ),
    )
    .toBe(1);
});

test("Leerzustand eines Ordners: „Keine losen Texte“ und „Neue Seite“", async ({
  page,
}) => {
  await page.addInitScript(
    (books) => {
      (window as unknown as { __lvBooks?: unknown }).__lvBooks = books;
    },
    [
      {
        ...BOOKS[0],
        editions: [...BOOKS[0].editions, edition("e3", "fr", "p2")],
      },
    ],
  );
  await openReadAloud(page);
  const folders = page.getByTestId("reader-folders");
  await folders.locator('[data-folder="texts"]').click();
  const empty = page.getByTestId("pages-empty");
  await expect(empty).toContainText("Keine losen Texte");
  const create = empty.getByRole("button", { name: "Neue Seite" });
  expect((await create.boundingBox())!.height).toBeGreaterThanOrEqual(44);
  await create.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as unknown as { __lvP4?: boolean }).__lvP4),
    )
    .toBe(true);
  // Ordner mit Einträgen zeigen den Leerzustand nicht.
  await folders.locator('[data-folder="all"]').click();
  await expect(empty).toHaveCount(0);
});

test("Mobil 390 px: drei Ebenen Ordner, Liste, Reader mit Zurück-Knöpfen", async ({
  page,
}) => {
  await openReadAloud(page, 768);
  await page.setViewportSize({ width: 390, height: 844 });
  const noOverflow = async () => {
    const over = await page.evaluate(() =>
      Math.max(
        document.documentElement.scrollWidth - window.innerWidth,
        ...[
          ...document.querySelectorAll(".workspace-main, .tts-workspace"),
        ].map((el) => el.scrollWidth - el.clientWidth),
      ),
    );
    expect(over).toBeLessThanOrEqual(0);
  };
  const big = async (id: string) => {
    const box = await page.getByTestId(id).boundingBox();
    expect(box!.width).toBeGreaterThanOrEqual(44);
    expect(box!.height).toBeGreaterThanOrEqual(44);
  };
  const folderNav = page.getByTestId("reader-folders");

  // Ebene 3: Reader (Einstieg wie bisher), Liste und Ordner verborgen.
  await expect(page.locator(".tts-editor")).toBeVisible();
  await expect(pagesCol(page)).toBeHidden();
  await big("mback-reader");
  await noOverflow();

  // Zurück: Ebene 2, die Liste.
  await page.getByTestId("mback-reader").click();
  await expect(pagesCol(page)).toBeVisible();
  await expect(page.locator(".tts-editor")).toBeHidden();
  await expect(folderNav).toBeHidden();
  await expect(rows(page)).toHaveCount(3);
  await big("mback-list");
  await noOverflow();
  // Je Ebene nur ein Bereich: Dateien und Bedienung der rechten Spalte nicht.
  await expect(page.getByTestId("tts-files")).toBeHidden();
  // Fokus wandert auf den Anfang der neuen Ebene, Name enthält den Text.
  await expect(page.getByTestId("mback-list")).toBeFocused();
  await expect(page.getByTestId("mback-list")).toHaveAccessibleName(
    /Zurück.*Ordner/,
  );

  // Zurück: Ebene 1, die Ordner.
  await page.getByTestId("mback-list").click();
  await expect(folderNav).toBeVisible();
  await expect(rows(page).first()).toBeHidden();
  await expect(page.locator(".tts-editor")).toBeHidden();
  await noOverflow();
  for (const key of ["all", "book:b1", "texts"]) {
    const box = await folderNav.locator(`[data-folder="${key}"]`).boundingBox();
    expect(box!.height).toBeGreaterThanOrEqual(44);
  }

  // Ordner wählen führt zur Liste (gruppiert), Seite wählen zum Reader.
  await folderNav.locator('[data-folder="book:b1"]').click();
  await expect(bookHead(page)).toBeVisible();
  await expect(folderNav).toBeHidden();
  await expect(page.getByTestId("book-ai-doc")).toHaveCount(2);
  for (const el of [
    page.getByTestId("book-continue"),
    page.getByTestId("book-audio"),
    page.getByTestId("book-ai-doc").first(),
  ])
    expect((await el.boundingBox())!.height).toBeGreaterThanOrEqual(44);
  await noOverflow();
  await rows(page).filter({ hasText: "The Tempest" }).click();
  await expect(page.locator(".tts-editor")).toBeVisible();
  await expect(pagesCol(page)).toBeHidden();
  await expect.poll(() => activePage(page)).toBe("p3");
  await expect(page.getByTestId("mback-reader")).toContainText("The Tempest");
  await noOverflow();

  // Ein KI-Dokument öffnet ebenfalls den Reader.
  await page.getByTestId("mback-reader").click();
  await page.getByTestId("book-ai-doc").first().click();
  await expect(page.locator(".tts-editor")).toBeVisible();
  await expect.poll(() => activePage(page)).toBe("p1");
});

test("Über 620 px bleibt es bei Leiste und Auswahl, ohne Zurück-Knöpfe", async ({
  page,
}) => {
  await openReadAloud(page, 1920);
  await expect(page.getByTestId("mback-reader")).toBeHidden();
  await expect(page.getByTestId("mback-list")).toBeHidden();
  await expect(page.getByTestId("reader-folders")).toBeVisible();
  await page.setViewportSize({ width: 900, height: 900 });
  await expect(page.getByTestId("reader-folders-compact")).toBeVisible();
  await expect(page.getByTestId("mback-reader")).toBeHidden();
  await expect(page.locator(".tts-editor")).toBeVisible();
  await expect(pagesCol(page)).toBeVisible();
  await page.setViewportSize({ width: 640, height: 900 });
  await expect(page.getByTestId("mback-reader")).toBeHidden();
  await expect(page.locator(".tts-editor")).toBeVisible();
  await expect(pagesCol(page)).toBeVisible();
});

test("Lesespalte: Einzelseite bei 2560 px höchstens etwa 80 Zeichen, mittig", async ({
  page,
}) => {
  await page.addInitScript(() => {
    localStorage.setItem("lva.ui.tts.reader.two", "0");
    (window as unknown as { __lvText?: string }).__lvText =
      "Es war eine dunkle Nacht, und der Sturm trieb die Wolken vor sich her.\n\n".repeat(
        120,
      );
  });
  await openReadAloud(page, 2560);
  const book = page.getByTestId("reader-book");
  await expect(book).toBeVisible();
  await expect(page.getByTestId("reader-flow")).toBeVisible();
  const m = await page.evaluate(() => {
    const book = document.querySelector('[data-testid="reader-book"]')!;
    const flow = document.querySelector('[data-testid="reader-flow"]')!;
    const stage = document.querySelector('[data-testid="reader-stage"]')!;
    const editor = document.querySelector(".tts-editor")!;
    // 80ch in der Schrift des Lesetextes, gemessen an einer Sonde.
    const probe = document.createElement("div");
    probe.style.cssText = `position:absolute;visibility:hidden;width:80ch;font:${getComputedStyle(flow).font}`;
    book.appendChild(probe);
    const ch80 = probe.getBoundingClientRect().width;
    probe.remove();
    const b = book.getBoundingClientRect();
    const s = stage.getBoundingClientRect();
    return {
      text: flow.getBoundingClientRect().width,
      ch80,
      book: b.width,
      stage: s.width,
      editor: editor.getBoundingClientRect().width,
      left: b.left - s.left,
      right: s.right - b.right,
    };
  });
  expect(m.text / m.ch80).toBeGreaterThan(0.9);
  expect(m.text / m.ch80).toBeLessThan(1.1);
  // Nur der Lesetext ist begrenzt, nicht die Spalte.
  expect(m.stage).toBeGreaterThan(m.book + 200);
  expect(m.editor).toBeGreaterThan(m.book + 200);
  expect(Math.abs(m.left - m.right)).toBeLessThanOrEqual(2);
});

// Belege für die Codex-Abnahme (kein Verhaltenstest), nur mit LV_P1_SHOTS:
//   LV_P1_SHOTS=docs/vorschau/projektordner-p1 LV_THEME=dark pnpm exec playwright test readaloud-folders
test("Belege P1", async ({ page }) => {
  const dir = process.env.LV_P1_SHOTS;
  test.skip(!dir, "nur für Vorschaubilder");
  const theme = process.env.LV_THEME === "dark" ? "dunkel" : "hell";
  for (const width of [2560, 1440, 768, 390]) {
    // Bei 390 px liegt die Navigation hinter dem Menüknopf: breit öffnen, dann schmal.
    await openReadAloud(page, Math.max(width, 768));
    if (width < 768) await page.setViewportSize({ width, height: 844 });
    if (width > 1100) {
      await page
        .getByTestId("reader-folders")
        .locator('[data-folder="book:b1"]')
        .click();
    }
    await page.waitForTimeout(300);
    await page.screenshot({
      path: `${dir}/p1-buchordner-${width}-${theme}.png`,
    });
  }
});

// Belege P1b (lva#365): Buch-Ordner gruppiert, 390 px in drei Ebenen.
//   LV_P1B_SHOTS=docs/vorschau/projektordner-p1b LV_THEME=dark pnpm exec playwright test readaloud-folders -g "Belege P1b"
test("Belege P1b", async ({ page }) => {
  const dir = process.env.LV_P1B_SHOTS;
  test.skip(!dir, "nur für Vorschaubilder");
  const theme = process.env.LV_THEME === "dark" ? "dunkel" : "hell";
  for (const width of [2560, 1440]) {
    await openReadAloud(page, width);
    await page
      .getByTestId("reader-folders")
      .locator('[data-folder="book:b1"]')
      .click();
    await page.waitForTimeout(400);
    await page.screenshot({
      path: `${dir}/p1b-buchordner-${width}-${theme}.png`,
    });
  }
  await openReadAloud(page, 768);
  await page.waitForTimeout(300);
  await page.screenshot({ path: `${dir}/p1b-buchordner-768-${theme}.png` });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.waitForTimeout(300);
  await page.screenshot({
    path: `${dir}/p1b-buchordner-390-reader-${theme}.png`,
  });
  await page.getByTestId("mback-reader").click();
  await page.waitForTimeout(300);
  await page.screenshot({
    path: `${dir}/p1b-buchordner-390-liste-${theme}.png`,
  });
  await page.getByTestId("mback-list").click();
  await page.waitForTimeout(300);
  await page.screenshot({
    path: `${dir}/p1b-buchordner-390-ordner-${theme}.png`,
  });
});
