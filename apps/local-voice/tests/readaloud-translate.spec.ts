import { test, expect, type Page } from "@playwright/test";

// Vorlesen: Übersetzung und Zusammenfassung zeigen Fortschritt und lassen sich
// abbrechen, haben ein Menü (Skript prüfen …), und die Sprecher der Übersetzung
// lassen sich gegen Stimmen der Zielsprache tauschen.
test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(() => {
    const callbacks = new Map<number, unknown>();
    let callback = 0;
    const settings = {
      onboarding_completed: true,
      app_language: "de",
      theme: "light",
      show_whats_new_on_update: false,
      debug_mode: false,
      selected_model: "",
      bindings: {
        transcribe: { id: "transcribe", name: "Diktat", description: "", current_binding: "Ctrl+Space", default_binding: "Ctrl+Space" },
      },
      post_process_providers: [],
      post_process_prompts: [],
      custom_words: [],
      post_process_models: {},
      post_process_api_keys: {},
      push_to_talk: true,
      tts_engine: "fish",
      tts_voice: null,
      tts_piper_voice: null,
      tts_translate_lang: "English",
      tts_script_check: false,
    };
    const voice = (id: string, name: string, language: string) => ({
      id,
      meta: { display_name: name, color: "teal", language, tags: [] },
      origin: "recorded",
      avatar_path: null,
    });
    const voices = [
      voice("erzaehlerin", "Erzählerin", "de-DE"),
      voice("erzaehler", "Erzähler", "de-DE"),
      voice("emma", "Emma", "en-US"),
      voice("brian", "Brian", "en-GB"),
    ];
    const w = window as unknown as Record<string, unknown>;
    const listeners: Record<string, number[]> = {};
    w.__calls = [] as unknown[];
    w.__emit = (name: string, payload: unknown) =>
      (listeners[name] ?? []).forEach((id) =>
        (callbacks.get(id) as (e: unknown) => void)?.({ event: name, id: 0, payload }),
      );
    // Übersetzung/Zusammenfassung warten, bis der Test sie aufloest.
    const pending: Record<string, { resolve: (v: unknown) => void; reject: (e: unknown) => void }> = {};
    w.__finish = (cmd: string, value: unknown, error?: string) => {
      const p = pending[cmd];
      if (!p) return;
      if (error) p.reject(error);
      else p.resolve(value);
      delete pending[cmd];
    };
    Object.assign(window, {
      __TAURI_OS_PLUGIN_INTERNALS__: { platform: "windows", os_type: "windows", family: "windows", arch: "x86_64", version: "10.0.26200", eol: "\r\n" },
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
      __TAURI_INTERNALS__: {
        metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
        transformCallback: (fn: unknown) => {
          callbacks.set(++callback, fn);
          return callback;
        },
        unregisterCallback: (id: number) => callbacks.delete(id),
        convertFileSrc: (path: string) => path,
        invoke: async (cmd: string, args?: Record<string, unknown>) => {
          (w.__calls as unknown[]).push([cmd, args]);
          if (cmd === "get_app_settings" || cmd === "get_default_settings") return JSON.parse(JSON.stringify(settings));
          if (cmd === "plugin:os|locale") return "de-DE";
          if (cmd === "plugin:app|version") return "0.21.14";
          if (cmd.includes("permission")) return true;
          if (cmd === "plugin:event|listen") {
            const name = String(args?.event);
            (listeners[name] ??= []).push(Number(args?.handler));
            return ++callback;
          }
          if (cmd === "get_selected_model") return "";
          if (cmd === "meetings_is_recording") return false;
          if (cmd === "tts_server_status") return { phase: "stopped", message: null };
          if (cmd === "tts_list_voice_infos") return JSON.parse(JSON.stringify(voices));
          if (cmd === "tts_list_voices") return voices.map((v) => v.id);
          if (cmd === "pages_list") return [{ id: "p1", title: "Podcast" }];
          if (cmd === "page_dir") return "C:\\projects\\p1";
          // KI-Dokumente (lva#183): zwei Vorlagen, Liste aus window.__kidocs.
          if (cmd === "book_ai_templates")
            return [
              { id: "builtin:eine_seite", title: "Eine Seite", category: "ueberblick", description: "Das Buch auf einer Seite.", context: "", sections: [], target_words: 450, form: "Prose", audience: "allgemein", builtin: true },
              { id: "builtin:lernzettel", title: "Lernzettel", category: "lernen", description: "Zum Wiederholen.", context: "", sections: [], target_words: 600, form: "List", audience: "lernende", builtin: true },
            ];
          if (cmd === "book_ai_documents") return (w.__kidocs as unknown[] | undefined) ?? [];
          if (cmd === "book_ai_document_text") return "Kurz gesagt: alles gut.";
          if (cmd === "tts_translate" || cmd === "tts_summarize_text" || cmd === "book_ai_summarize")
            return new Promise((resolve, reject) => {
              pending[cmd] = { resolve, reject };
            });
          if (cmd === "tts_cached_translation") return (w.__cached as string | undefined) ?? null;
          if (cmd === "tts_list_downloads") return [];
          if (cmd === "llm_ps" || cmd === "tts_reading_list" || cmd === "page_files" || cmd.includes("history") || cmd.includes("models") || cmd.includes("devices") || cmd === "meetings_list") return [];
          if (cmd === "get_custom_sounds") return { start: false, stop: false };
          return null;
        },
      },
    });
  });
});

const ORIGINAL = "<Erzählerin> Guten Tag.\n<Erzähler> Hallo zusammen.";

const open = async (page: Page) => {
  await page.goto("/");
  await page.getByRole("navigation").getByRole("button", { name: "Vorlesen", exact: true }).click();
  await page.locator("textarea").first().fill(ORIGINAL);
};

const emit = (page: Page, name: string, payload: unknown) =>
  page.evaluate(
    ([name, payload]) => (window as unknown as { __emit: (n: string, p: unknown) => void }).__emit(name as string, payload),
    [name, payload] as const,
  );
const finish = (page: Page, cmd: string, value: unknown, error?: string) =>
  page.evaluate(
    ([cmd, value, error]) =>
      (window as unknown as { __finish: (c: string, v: unknown, e?: string) => void }).__finish(cmd as string, value, error as string | undefined),
    [cmd, value, error] as const,
  );
const calls = (page: Page) => page.evaluate(() => (window as unknown as { __calls: [string, unknown][] }).__calls);

test("Übersetzung zeigt Block x von y und lässt sich abbrechen", async ({ page }) => {
  await open(page);
  await page.getByRole("tab", { name: "Übersetzung" }).click();
  await page.getByTestId("tts-action-translate").click();
  await expect(page.getByText("Übersetze …", { exact: true })).toBeVisible();
  await emit(page, "tts-transform-progress", { kind: "translation", done: 1, total: 4 });
  await expect(page.getByText("Übersetze … Block 2 von 4")).toBeVisible();
  await page.getByRole("button", { name: "Abbrechen" }).first().click();
  expect((await calls(page)).filter(([c]) => c === "tts_transform_cancel")).toEqual([
    ["tts_transform_cancel", { kind: "translation" }],
  ]);
  await finish(page, "tts_translate", null, "transform_cancelled");
  await expect(page.getByText("Abgebrochen.")).toBeVisible();
  await expect(page.getByText("Übersetze … Block 2 von 4")).toHaveCount(0);
});

test("nach der Übersetzung werden die Sprecher gegen Stimmen der Zielsprache getauscht", async ({ page }) => {
  await open(page);
  await page.getByRole("tab", { name: "Übersetzung" }).click();
  await page.getByTestId("tts-action-translate").click();
  // Das Backend hat die Marken geschützt: sie kommen unverändert zurück.
  await finish(page, "tts_translate", "<Erzählerin> Good day.\n<Erzähler> Hello everyone.");
  const list = page.getByTestId("speaker-swap-list");
  await expect(list).toBeVisible();
  await expect(page.getByTestId("speaker-swap-erzaehlerin")).toHaveValue("emma");
  await expect(page.getByTestId("speaker-swap-erzaehler")).toHaveValue("brian");
  await page.getByTestId("speaker-swap-apply").click();
  await expect(page.locator("textarea").first()).toHaveValue("<Emma> Good day.\n<Brian> Hello everyone.");
});

test("im Reiter Übersetzung gibt es das Menü mit Skript prüfen und Sprecher tauschen", async ({ page }) => {
  await open(page);
  await page.getByRole("tab", { name: "Übersetzung" }).click();
  await page.getByTestId("tts-action-menu-translation").click();
  await expect(page.getByTestId("script-check-run")).toBeVisible();
  await expect(page.getByTestId("speaker-swap-open")).toBeVisible();
  await expect(page.getByTestId("autotag-open")).toHaveCount(0);
});

test("im Reiter KI-Dokumente gibt es das Menü, den Erstellen-Dialog und den Fortschritt", async ({ page }) => {
  await open(page);
  await page.getByRole("tab", { name: "KI-Dokumente" }).click();
  await page.getByTestId("tts-action-menu-summary").click();
  await expect(page.getByTestId("script-check-run")).toBeVisible();
  await expect(page.getByTestId("speaker-swap-open")).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("kidocs-empty")).toBeVisible();

  // Neu erstellen: Quelle und Vorlage wählen, Laufzeit nur als Schätzung.
  await page.getByTestId("tts-action-summarize").click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("Neues KI-Dokument");
  await expect(page.getByTestId("kidoc-src-book")).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByTestId("kidoc-src-selection")).toBeDisabled();
  await page.getByTestId("kidoc-tpl-search").fill("lern");
  await expect(page.getByTestId("kidoc-tpl-builtin:eine_seite")).toHaveCount(0);
  await page.getByTestId("kidoc-tpl-search").fill("");
  await page.getByTestId("kidoc-tpl-builtin:eine_seite").click();
  await expect(page.getByTestId("kidoc-estimate")).toContainText("Schätzung");
  await page.getByTestId("kidoc-create").click();
  await expect(dialog).toHaveCount(0);

  // Erstellung läuft: als eigene Karte mit Schritten, getrennt vom Hörfortschritt.
  await emit(page, "book-ai-progress", { page_id: "p1", done: 1, total: 3 });
  await expect(page.getByTestId("kidocs-running")).toContainText("1 von 3 Schritten");
  const call = (await calls(page)).filter(([c]) => c === "book_ai_summarize").at(-1)!;
  expect(call[1]).toMatchObject({
    pageId: "p1",
    source: { kind: "book" },
    templateId: "builtin:eine_seite",
  });

  const doc = {
    id: "01JAAAAAAAAAAAAAAAAAAAAAAA",
    title: "Eine Seite – Ganzes Buch",
    template_id: "builtin:eine_seite",
    template_title: "Eine Seite",
    source: { kind: "book" },
    source_label: "Ganzes Buch",
    created_ms: 1,
    modified_ms: 1,
    words: 4,
    model: "ollama/x",
  };
  await page.evaluate((d) => {
    (window as unknown as { __kidocs: unknown[] }).__kidocs = [d];
  }, doc);
  await finish(page, "book_ai_summarize", doc);

  // Fertig: das Dokument öffnet sich im selben Reader, der Weg zurück ist sichtbar.
  await expect(page.getByTestId("kidoc-head")).toContainText("Eine Seite – Ganzes Buch");
  await expect(page.getByText("Kurz gesagt: alles gut.").first()).toBeVisible();
  await page.getByTestId("kidoc-back").click();
  const card = page.getByTestId(`kidocs-card-${doc.id}`);
  await expect(card).toContainText("noch nicht gehört");
  await expect(card).toContainText("Überblick · Ganzes Buch");
});

test("eine korrigierte Übersetzung wird abgelegt und beim Neuladen nicht überschrieben", async ({ page }) => {
  await open(page);
  await page.getByRole("tab", { name: "Übersetzung" }).click();
  await page.getByTestId("tts-action-translate").click();
  await finish(page, "tts_translate", "<Erzählerin> Good day.");
  // Das Tausch-Fenster schliessen, dann korrigieren.
  await page.getByRole("button", { name: "Abbrechen" }).last().click();
  const editor = page.locator("textarea").first();
  await editor.fill("<Erzählerin> Good morning, everyone.");
  await expect
    .poll(async () => (await calls(page)).filter(([c]) => c === "tts_store_translation").length)
    .toBeGreaterThan(0);
  const stored = (await calls(page)).filter(([c]) => c === "tts_store_translation").at(-1)!;
  expect(stored[1]).toMatchObject({
    text: ORIGINAL,
    targetLang: "English",
    translation: "<Erzählerin> Good morning, everyone.",
  });
});

test("eine unveränderte maschinelle Übersetzung wird nicht noch einmal abgelegt", async ({ page }) => {
  await open(page);
  await page.getByRole("tab", { name: "Übersetzung" }).click();
  await page.getByTestId("tts-action-translate").click();
  await finish(page, "tts_translate", "Hello there.");
  await page.waitForTimeout(900);
  const before = (await calls(page)).filter(([c]) => c === "tts_store_translation").length;
  // Unveraenderte maschinelle Uebersetzung: nichts abzulegen.
  expect(before).toBe(0);
});

test("nach dem Übersetzen steht die Sprache als Reiter über dem Feld", async ({ page }) => {
  await open(page);
  await page.getByRole("tab", { name: "Übersetzung" }).click();
  await expect(page.getByTestId("translation-lang-tabs")).toHaveCount(0);
  await page.getByTestId("tts-action-translate").click();
  await finish(page, "tts_translate", "<Erzählerin> Good day.");
  const tab = page.getByTestId("lang-tab-English");
  await expect(tab).toBeVisible();
  await expect(tab).toHaveAttribute("aria-selected", "true");
});

test("Rückgängig und Wiederherstellen gelten auch in der Übersetzung, mehrere Schritte", async ({ page }) => {
  await open(page);
  await page.getByRole("tab", { name: "Übersetzung" }).click();
  await page.getByTestId("tts-action-translate").click();
  await finish(page, "tts_translate", "Good day.");
  const editor = page.locator("textarea").first();
  await editor.fill("Good day, everyone.");
  await editor.fill("Good day, everyone. Welcome.");
  await editor.press("Control+z");
  await expect(editor).toHaveValue("Good day, everyone.");
  await editor.press("Control+z");
  await expect(editor).toHaveValue("Good day.");
  await editor.press("Control+y");
  await expect(editor).toHaveValue("Good day, everyone.");
});

test("eine aus dem Speicher geladene Übersetzung lässt sich nicht wegundo-n", async ({ page }) => {
  await page.addInitScript(() => {
    (window as unknown as { __cached: string }).__cached = "Hello there.";
  });
  await open(page);
  await page.getByRole("tab", { name: "Übersetzung" }).click();
  const editor = page.locator("textarea").first();
  await expect(editor).toHaveValue("Hello there.");
  await editor.press("Control+z");
  await expect(editor).toHaveValue("Hello there.");
});

const kiDoc = (n: number, tpl: string) => ({
  id: `01JKIDOC${String(n).padStart(18, "0")}`,
  title: `Dokument ${n}`,
  template_id: tpl,
  template_title: tpl.endsWith("lernzettel") ? "Lernzettel" : "Eine Seite",
  source: { kind: "book" },
  source_label: "Ganzes Buch",
  created_ms: 100 - n,
  modified_ms: 1,
  words: 10,
  model: "ollama/x",
});
const manyKiDocs = Array.from({ length: 14 }, (_, i) =>
  kiDoc(i + 1, i % 2 ? "builtin:eine_seite" : "builtin:lernzettel"),
);

test("zurück zu allen KI-Dokumenten behält Filter und Scrollstand", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 640 });
  await page.addInitScript((docs) => {
    (window as unknown as { __kidocs: unknown[] }).__kidocs = docs;
  }, manyKiDocs);
  await open(page);
  await page.getByRole("tab", { name: "KI-Dokumente" }).click();
  await page.getByTestId("kidocs-filter-lernen").click();
  await expect(page.getByTestId("kidocs-filter-lernen")).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator('[data-testid^="kidocs-card-"]')).toHaveCount(7);
  const list = page.getByTestId("kidocs-list");
  await list.evaluate((el) => {
    el.scrollTop = 180;
    el.dispatchEvent(new Event("scroll"));
  });
  const before = await list.evaluate((el) => el.scrollTop);
  expect(before).toBeGreaterThan(0);

  await page.getByTestId(`kidocs-card-${manyKiDocs[6].id}`).locator("button").first().click();
  await expect(page.getByTestId("kidoc-head")).toContainText("Dokument 7");
  await page.getByTestId("kidoc-back").click();

  await expect(page.getByTestId("kidocs-filter-lernen")).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator('[data-testid^="kidocs-card-"]')).toHaveCount(7);
  await expect.poll(() => page.getByTestId("kidocs-list").evaluate((el) => el.scrollTop)).toBe(before);
});

test("das vorgelesene KI-Dokument ist in Kopfzeile und Liste gekennzeichnet", async ({ page }) => {
  await page.addInitScript((docs) => {
    (window as unknown as { __kidocs: unknown[] }).__kidocs = docs;
  }, manyKiDocs.slice(0, 2));
  await open(page);
  await page.getByRole("tab", { name: "KI-Dokumente" }).click();
  const id = manyKiDocs[0].id;
  await page.getByTestId(`kidocs-play-${id}`).click();
  await expect(page.getByTestId("kidoc-head")).toContainText("Dokument 1");
  await emit(page, "tts-state-changed", { phase: "speaking", owns_server: false, message: null, server_up: true });
  await expect(page.getByTestId("kidoc-playing")).toHaveText("Wird vorgelesen");
  await page.getByTestId("kidoc-back").click();
  await expect(page.getByTestId(`kidocs-card-${id}`).getByTestId("kidocs-playing")).toBeVisible();
  await expect(page.getByTestId(`kidocs-card-${manyKiDocs[1].id}`).getByTestId("kidocs-playing")).toHaveCount(0);
});
