import { test, expect, type Page } from "@playwright/test";
// Einfügen im Editor einer Vorlesen-Seite (lva#530 I1): verschmutzter Text wird
// bereinigt, ein Hinweis bietet „Rückgängig“ an, der Schalter im Bereinigen-Menü
// schaltet es ab. Bruecke wie in editor-menu.spec.ts.
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

// Synthetischer Inhalt (Intranetbeitrag mit Navigation, Seitenzahl, NBSP).
const DIRTY = [
  "Zum Inhalt springen",
  "",
  "Neue Telefonanlage\u00A0ab Montag",
  "",
  "Ab Montag stehen alle Nebenstellen im Haus zur Verfügung, die Umstellung dauert etwa eine Stun-",
  "de.",
  "Seite 3 von 30",
  "Drucken · Teilen",
].join("\n");
const CLEAN = [
  "Neue Telefonanlage ab Montag",
  "",
  "Ab Montag stehen alle Nebenstellen im Haus zur Verfügung, die Umstellung dauert etwa eine Stunde.",
].join("\n");

async function openEditor(page: Page, text = "Vorher. ") {
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

/** Synthetisches Einfügen; `true`, wenn der Editor es übernommen (abgefangen) hat. */
async function paste(editor: ReturnType<Page["locator"]>, text: string) {
  await editor.focus();
  await editor.press("Control+End");
  return editor.evaluate((el, value) => {
    const data = new DataTransfer();
    data.setData("text/plain", value);
    const event = new ClipboardEvent("paste", {
      clipboardData: data,
      bubbles: true,
      cancelable: true,
    });
    el.dispatchEvent(event);
    return event.defaultPrevented;
  }, text);
}

const toast = (page: Page) => page.locator("[data-sonner-toast]");

test("pasted text is cleaned, a notice appears and undo restores the original", async ({
  page,
}) => {
  const editor = await openEditor(page);
  expect(await paste(editor, DIRTY)).toBe(true);
  await expect(editor).toHaveValue(`Vorher. ${CLEAN}`);
  await expect(toast(page)).toContainText("Beim Einfügen bereinigt:");
  await expect(toast(page)).toContainText("Änderungen");
  await toast(page).getByRole("button", { name: "Rückgängig" }).click();
  await expect(editor).toHaveValue(`Vorher. ${DIRTY}`);
});

test("ctrl+z after a cleaned paste still works", async ({ page }) => {
  const editor = await openEditor(page);
  await paste(editor, DIRTY);
  await expect(editor).toHaveValue(`Vorher. ${CLEAN}`);
  await editor.focus();
  await page.keyboard.press("Control+z");
  await expect(editor).toHaveValue("Vorher. ");
});

test("undo from the notice warns when the pasted range was edited meanwhile", async ({
  page,
}) => {
  const editor = await openEditor(page);
  await paste(editor, DIRTY);
  await editor.focus();
  await page.keyboard.press("Control+Home");
  for (let i = 0; i < 12; i++) await page.keyboard.press("ArrowRight");
  await page.keyboard.type("X");
  await toast(page)
    .getByRole("button", { name: "Rückgängig" })
    .evaluate((el: HTMLElement) => el.click());
  await expect(toast(page).last()).toContainText("inzwischen geändert");
  const value = await editor.inputValue();
  expect(value).toContain("X");
  expect(value).not.toContain("Zum Inhalt springen");
});

test("clean text and short lines are pasted natively without a notice", async ({
  page,
}) => {
  const editor = await openEditor(page);
  expect(await paste(editor, "Ein kurzer, sauberer Satz.")).toBe(false);
  expect(await paste(editor, "Zeile eins ist sauber.\nZeile zwei auch.")).toBe(
    false,
  );
  await expect(toast(page)).toHaveCount(0);
});

test("the switch in the cleanup menu turns paste cleanup off and persists", async ({
  page,
}) => {
  const editor = await openEditor(page);
  await page.getByTestId("editor-menu-cleanup").click();
  const item = page.getByTestId("editor-cleanup-paste");
  await expect(item).toHaveAttribute("aria-checked", "true");
  await item.click();
  expect(
    await page.evaluate(() => localStorage.getItem("lva.ui.paste.cleanup")),
  ).toBe("0");
  await page.getByTestId("editor-menu-cleanup").click();
  await expect(page.getByTestId("editor-cleanup-paste")).toHaveAttribute(
    "aria-checked",
    "false",
  );
  await page.keyboard.press("Escape");
  expect(await paste(editor, DIRTY)).toBe(false);
  await expect(toast(page)).toHaveCount(0);
  await expect(editor).toHaveValue("Vorher. ");
});

test("the switch is also reachable while the editor is empty", async ({
  page,
}) => {
  await openEditor(page, "");
  await page.getByTestId("editor-menu-cleanup").click();
  await expect(page.getByTestId("editor-cleanup-paste")).toBeEnabled();
  await expect(page.getByTestId("editor-cleanup-all")).toBeDisabled();
});
