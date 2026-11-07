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
const BOOKS = [
  {
    id: "b1",
    title: "Der Sturm",
    subtitle: null,
    authors: ["W. S."],
    language: "de",
    text: { page_id: "p1", origin: "document", source_name: null },
    audio: null,
    editions: [edition("e1", "de", "p1"), edition("e2", "en", "p3")],
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
            if (cmd === "plugin:event|listen") return ++callback;
            if (cmd === "get_selected_model") return "";
            if (cmd === "meetings_is_recording") return false;
            if (cmd === "tts_server_status")
              return { phase: "stopped", message: null };
            if (cmd === "library_list") return BOOKS;
            if (cmd === "pages_list")
              return [
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
              ];
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
