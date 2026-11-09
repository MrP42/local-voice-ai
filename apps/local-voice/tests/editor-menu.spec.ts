import { test, expect, type Page } from "@playwright/test";
import * as path from "node:path";

// Editor-Menü über dem Textfeld (Vorlesen, Bearbeiten): Suchen/Ersetzen,
// Rechtschreibung, Formatierung, Bereinigen ohne KI und KI-Textarbeit mit
// Zeilenprotokoll. Bruecke wie in readaloud-toolbar.spec.ts; die KI-Antwort
// gibt der Test ueber window.__assist vor.
test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(() => {
    const callbacks = new Map<number, unknown>();
    let callback = 0;
    const calls: string[] = [];
    (window as unknown as { __calls: string[] }).__calls = calls;
    const settings = {
      onboarding_completed: true,
      app_language: "de",
      theme: window.matchMedia("(prefers-color-scheme: dark)").matches
        ? "dark"
        : "light",
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
      tts_engine: "piper",
      tts_piper_voice: null,
      tts_script_check: true,
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
        transformCallback: (fn: unknown) => {
          callbacks.set(++callback, fn);
          return callback;
        },
        unregisterCallback: (id: number) => callbacks.delete(id),
        convertFileSrc: (path: string) => path,
        invoke: async (cmd: string, args?: Record<string, unknown>) => {
          calls.push(cmd);
          if (cmd === "tts_text_assist") {
            const w = window as unknown as {
              __assistArgs?: unknown;
              __assist?: string;
            };
            w.__assistArgs = args;
            return w.__assist ?? "OK";
          }
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
          if (cmd === "pages_list") return [{ id: "p1", title: "Der Sturm" }];
          if (cmd === "books_list") return [];
          if (cmd === "books_templates") return [];
          if (cmd === "books_memory_read") return [];
          if (cmd === "tts_tidy_text") return "Sauberer Text.";
          if (cmd === "page_files") return [];
          if (cmd === "page_dir") return "C:/projects/p1";
          if (cmd === "tts_list_downloads") return [];
          if (cmd === "get_custom_sounds") return { start: false, stop: false };
          // Testweise vorgegebene Stimmenliste (window.__voices), sonst leer.
          if (cmd === "tts_list_voices")
            return (
              (window as unknown as { __voices?: string[] }).__voices ?? []
            );
          if (
            cmd === "tts_list_voice_infos" ||
            cmd === "llm_ps" ||
            cmd === "tts_reading_list" ||
            cmd.includes("history") ||
            cmd.includes("models") ||
            cmd.includes("devices") ||
            cmd === "meetings_list"
          )
            return [];
          return null;
        },
      },
    });
  });
});

const TRANSCRIPT = [
  "[2:37]: Meiner Meinung nach schaffen es die meisten Autoren nicht, ein gutes Buch zu schreiben.",
  "[2:46]: Sie schreiben entweder langatmig oder so trocken, dass die Ge-",
  "danken abschweifen.",
  "[3:00]: J.",
  "[3:01]: L.",
  "[3:01]: Collins hat mit diesem althergebrachten Stil überhaupt nichts am Hut.",
  "[3:08]: Er schreibt so, dass man bei ihm seine Zuflucht nehmen möchte.",
  "[3:47]: Genau das passierte, als Jim eine Reihe von Blogbeiträgen auf",
  "www.jlcollins.com",
  "[4:00]: zu schreiben begann.",
  "[4:02]: Ich habe sie alle gelesen , sobald sie  erschienen sind.",
].join("\n");

async function openEditor(page: Page, text = TRANSCRIPT) {
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  const editor = page.locator("textarea").first();
  await editor.fill(text);
  await expect(page.getByTestId("editor-menu")).toBeVisible();
  return editor;
}

test("the editor menu sits directly above the text field", async ({ page }) => {
  const editor = await openEditor(page);
  const menu = await page
    .getByRole("toolbar", { name: "Editor-Menü" })
    .boundingBox();
  const field = await editor.boundingBox();
  expect(menu && field).toBeTruthy();
  expect(menu!.y + menu!.height).toBeLessThanOrEqual(field!.y + 1);
  expect(field!.y - (menu!.y + menu!.height)).toBeLessThan(40);
});

test("cleanup with all rules removes timestamps and joins broken lines", async ({
  page,
}) => {
  const editor = await openEditor(page);
  await page.getByTestId("editor-menu-cleanup").click();
  await page.getByTestId("editor-cleanup-all").click();
  const value = await editor.inputValue();
  expect(value).not.toMatch(/\[\d+:\d+\]/);
  expect(value).toContain("J. L. Collins hat");
  expect(value).toContain("die Gedanken abschweifen.");
  expect(value).toContain("www.jlcollins.com zu schreiben begann.");
  expect(value).toContain("gelesen, sobald sie erschienen");
  // Ein Schritt in der Historie: Strg+Z holt das Original zurück.
  await editor.focus();
  await page.keyboard.press("Control+z");
  await expect(editor).toHaveValue(TRANSCRIPT);
});

test("ctrl+h opens find and replace, replace all works", async ({ page }) => {
  const editor = await openEditor(page);
  await editor.focus();
  await page.keyboard.press("Control+h");
  await expect(page.getByTestId("find-bar")).toBeVisible();
  await page.getByTestId("find-input").fill("Collins");
  await expect(page.getByTestId("find-count")).toHaveText("1 von 2");
  await page.getByTestId("replace-input").fill("COLLINS");
  await page.getByTestId("replace-all").click();
  await expect(editor).toHaveValue(/COLLINS hat/);
  await page.getByTestId("find-input").press("Escape");
  await expect(page.getByTestId("find-bar")).toBeHidden();
});

test("format buttons set a heading on the current line", async ({ page }) => {
  const editor = await openEditor(page, "Der Anfang\nErster Satz.");
  await editor.focus();
  await editor.evaluate((el: HTMLTextAreaElement) =>
    el.setSelectionRange(2, 2),
  );
  await page.getByTestId("editor-menu-format-h2").click();
  await expect(editor).toHaveValue("## Der Anfang\nErster Satz.");
  await page.getByTestId("editor-menu-format-h2").click();
  await expect(editor).toHaveValue("Der Anfang\nErster Satz.");
});

test("spell check can be switched off and stays off", async ({ page }) => {
  const editor = await openEditor(page);
  await expect(editor).toHaveAttribute("spellcheck", "true");
  await page.getByTestId("editor-menu-spellcheck").click();
  await expect(editor).toHaveAttribute("spellcheck", "false");
  await page.reload();
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await expect(page.locator("textarea").first()).toHaveAttribute(
    "spellcheck",
    "false",
  );
});

test("AI grammar sends numbered lines without timestamps and applies only changes", async ({
  page,
}) => {
  const editor = await openEditor(
    page,
    "[0:01]: Das sind ein Fehler.\n[0:02]: Alles gut.",
  );
  await page.evaluate(() => {
    (window as unknown as { __assist: string }).__assist =
      "1|Das ist ein Fehler.";
  });
  await page.getByTestId("editor-menu-ai").click();
  await page.getByTestId("editor-ai-grammar").click();
  await expect(editor).toHaveValue(
    "[0:01]: Das ist ein Fehler.\n[0:02]: Alles gut.",
  );
  const args = await page.evaluate(
    () => (window as unknown as { __assistArgs: unknown }).__assistArgs,
  );
  expect(args).toEqual({
    task: "grammar",
    lines: "1|Das sind ein Fehler.\n2|Alles gut.",
  });
});

test("AI structure turns marks into headings and paragraphs", async ({
  page,
}) => {
  const editor = await openEditor(
    page,
    "Der Anfang\nWenn man greift, steht man da.\nAber sauber.\nNeuer Gedanke.",
  );
  await page.evaluate(() => {
    (window as unknown as { __assist: string }).__assist = "1|H2\n4|P";
  });
  await page.getByTestId("editor-menu-ai").click();
  await page.getByTestId("editor-ai-structure").click();
  await expect(editor).toHaveValue(
    "## Der Anfang\n\nWenn man greift, steht man da. Aber sauber.\n\nNeuer Gedanke.",
  );
});

const SHOTS = "docs/vorschau/editor-menue";

for (const theme of ["light", "dark"] as const) {
  test(`screenshots of the editor menu (${theme})`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: theme });
    await page.setViewportSize({ width: 1440, height: 900 });
    const editor = await openEditor(page);
    for (const w of [1440, 390]) {
      await page.setViewportSize({ width: w, height: w > 600 ? 900 : 844 });
      await editor.fill(TRANSCRIPT);
      await editor.focus();
      await page.keyboard.press("Control+h");
      await page.getByTestId("find-input").fill("Collins");
      await page.getByTestId("editor-menu-cleanup").click();
      await expect(page.getByTestId("editor-cleanup-all")).toBeVisible();
      await page.screenshot({ path: `${SHOTS}/${w}-${theme}-bereinigen.png` });
      await page.keyboard.press("Escape");
      await page.getByTestId("editor-menu-ai").click();
      await expect(page.getByTestId("editor-ai-grammar")).toBeVisible();
      await page.screenshot({ path: `${SHOTS}/${w}-${theme}-ki.png` });
      await page.keyboard.press("Escape");
      await page.getByTestId("editor-menu-cleanup").click();
      await page.getByTestId("editor-cleanup-all").click();
      await page.getByTestId("find-close").click();
      await page.screenshot({ path: `${SHOTS}/${w}-${theme}-bereinigt.png` });
    }
  });
}
