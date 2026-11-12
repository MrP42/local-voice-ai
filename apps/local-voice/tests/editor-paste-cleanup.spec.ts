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
          // Seitenzustand: bleibt im sessionStorage und übersteht damit ein Neuladen
          // (Neustart der App); `__failSave` lässt das Schreiben scheitern.
          if (cmd === "page_state_save") {
            if ((window as unknown as { __failSave?: boolean }).__failSave)
              throw "disk full";
            sessionStorage.setItem(`__ps_${args?.id}`, String(args?.state));
            return null;
          }
          if (cmd === "page_state_load")
            return sessionStorage.getItem(`__ps_${args?.id}`);
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

/** Synthetisches Einfügen; `true`, wenn der Editor es übernommen (abgefangen) hat.
 *  Ohne `selection` am Ende des Textes, sonst über der Markierung. */
async function paste(
  editor: ReturnType<Page["locator"]>,
  text: string,
  selection?: [number, number],
) {
  await editor.focus();
  if (selection) {
    await editor.evaluate(
      (el: HTMLTextAreaElement, [a, b]) => el.setSelectionRange(a, b),
      selection,
    );
  } else await editor.press("Control+End");
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

/** Gespeicherter Seitenzustand der Seite „p1“ (state.json). */
const savedState = (page: Page) =>
  page.evaluate(() => {
    const raw = sessionStorage.getItem("__ps_p1");
    return raw ? (JSON.parse(raw) as Record<string, unknown>) : null;
  });

const caret = (editor: ReturnType<Page["locator"]>) =>
  editor.evaluate((el: HTMLTextAreaElement) => [
    el.selectionStart,
    el.selectionEnd,
  ]);
const undoNotice = (page: Page) =>
  page
    .locator("[data-sonner-toast]")
    .getByRole("button", { name: "Rückgängig" });

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

test("a selection is replaced by the cleaned text and the caret ends behind it", async ({
  page,
}) => {
  const editor = await openEditor(page, "Vorher. ALT Nachher.");
  expect(await paste(editor, DIRTY, [8, 11])).toBe(true);
  await expect(editor).toHaveValue(`Vorher. ${CLEAN} Nachher.`);
  const end = 8 + CLEAN.length;
  expect(await caret(editor)).toEqual([end, end]);
  await undoNotice(page).click();
  await expect(editor).toHaveValue(`Vorher. ${DIRTY} Nachher.`);
  const back = 8 + DIRTY.length;
  expect(await caret(editor)).toEqual([back, back]);
});

test("the caret stays at the end of the pasted text when appending", async ({
  page,
}) => {
  const editor = await openEditor(page);
  await paste(editor, DIRTY);
  await expect(editor).toHaveValue(`Vorher. ${CLEAN}`);
  const end = `Vorher. ${CLEAN}`.length;
  expect(await caret(editor)).toEqual([end, end]);
});

test("empty result, ctrl+z, then the notice button inserts nothing and says so", async ({
  page,
}) => {
  const editor = await openEditor(page, "Alt");
  expect(await paste(editor, "Drucken\nTeilen", [0, 3])).toBe(true);
  await expect(editor).toHaveValue("");
  await expect(toast(page)).toContainText("Beim Einfügen bereinigt:");
  await editor.focus();
  await page.keyboard.press("Control+z");
  await expect(editor).toHaveValue("Alt");
  await undoNotice(page).evaluate((el: HTMLElement) => el.click());
  await expect(toast(page).last()).toContainText(
    "Rückgängig nicht mehr möglich",
  );
  await expect(editor).toHaveValue("Alt");
});

test("two pastes in a row leave one notice that only undoes the last", async ({
  page,
}) => {
  const editor = await openEditor(page, "");
  const second = [
    "Skip to content",
    "",
    "Zweiter Absatz mit Text, der lang genug ist, um bleiben zu dürfen.",
    "Seite 2 von 9",
  ].join("\n");
  await paste(editor, DIRTY);
  await expect(editor).toHaveValue(CLEAN);
  await paste(editor, second);
  const cleanSecond =
    "Zweiter Absatz mit Text, der lang genug ist, um bleiben zu dürfen.";
  await expect(editor).toHaveValue(CLEAN + cleanSecond);
  await expect(toast(page)).toHaveCount(1);
  await undoNotice(page).click();
  await expect(editor).toHaveValue(CLEAN + second);
  // Die Rohfassungen beider Einfügungen sind gesichert.
  const state = (await savedState(page)) as {
    einfuegungen: { roh: string }[];
  };
  expect(state.einfuegungen.map((e) => e.roh)).toEqual([DIRTY, second]);
});

test("the notice of an earlier paste is invalid after typing in between", async ({
  page,
}) => {
  const editor = await openEditor(page, "");
  await paste(editor, DIRTY);
  await expect(editor).toHaveValue(CLEAN);
  await editor.focus();
  await page.keyboard.press("Control+End");
  await page.keyboard.type(" Zusatz");
  await undoNotice(page).evaluate((el: HTMLElement) => el.click());
  await expect(toast(page).last()).toContainText("nicht mehr möglich");
  await expect(editor).toHaveValue(`${CLEAN} Zusatz`);
});

test("redo after ctrl+z brings the cleaned text back", async ({ page }) => {
  const editor = await openEditor(page);
  await paste(editor, DIRTY);
  await expect(editor).toHaveValue(`Vorher. ${CLEAN}`);
  await editor.focus();
  await page.keyboard.press("Control+z");
  await expect(editor).toHaveValue("Vorher. ");
  await page.keyboard.press("Control+Shift+z");
  await expect(editor).toHaveValue(`Vorher. ${CLEAN}`);
  await page.keyboard.press("Control+z");
  await page.keyboard.press("Control+y");
  await expect(editor).toHaveValue(`Vorher. ${CLEAN}`);
});

test("the context menu paste is cleaned too (mocked clipboard)", async ({
  page,
}) => {
  const editor = await openEditor(page);
  await page.evaluate((text) => {
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { readText: async () => text, writeText: async () => undefined },
    });
  }, DIRTY);
  await editor.click({ button: "right" });
  await page
    .getByRole("menuitem", { name: "Einfügen", exact: true })
    .evaluate((el: HTMLElement) => el.click());
  await expect(editor).toHaveValue(`Vorher. ${CLEAN}`);
  await expect(toast(page)).toContainText("Beim Einfügen bereinigt:");
  await undoNotice(page).click();
  await expect(editor).toHaveValue(`Vorher. ${DIRTY}`);
});

test("the raw text is saved with the page before cleaning and survives a restart", async ({
  page,
}) => {
  const editor = await openEditor(page);
  await paste(editor, DIRTY);
  await expect(editor).toHaveValue(`Vorher. ${CLEAN}`);
  await expect
    .poll(async () => {
      const state = (await savedState(page)) as {
        einfuegungen?: Record<string, unknown>[];
      } | null;
      return state?.einfuegungen?.length ?? 0;
    })
    .toBe(1);
  const state = (await savedState(page)) as {
    einfuegungen: {
      zeit: string;
      sha256: string;
      roh: string;
      bereinigt: string;
      regeln: { rule: string; count: number }[];
    }[];
  };
  const [entry] = state.einfuegungen;
  expect(entry.roh).toBe(DIRTY);
  expect(entry.bereinigt).toBe(CLEAN);
  expect(entry.sha256).toMatch(/^[0-9a-f]{64}$/);
  expect(Number.isNaN(Date.parse(entry.zeit))).toBe(false);
  expect(entry.regeln.map((r) => r.rule)).toEqual(
    expect.arrayContaining(["control", "pageNumbers", "webNoise", "fragments"]),
  );
  // Das gebündelte Sichern (halbe Sekunde) schreibt auch den neuen Text.
  await expect
    .poll(async () => ((await savedState(page)) as { text: string }).text)
    .toBe(`Vorher. ${CLEAN}`);
  // Neustart: Seite neu laden, der Zustand kommt aus dem „Speicher“.
  await page.reload();
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await page.getByRole("button", { name: "Bearbeiten", exact: true }).click();
  const again = page.locator("textarea").first();
  await expect(again).toHaveValue(`Vorher. ${CLEAN}`);
  await page.getByTestId("editor-menu-cleanup").click();
  await page
    .getByTestId("editor-cleanup-pastelog")
    .evaluate((el) => el.click());
  const rows = page.getByTestId("paste-log-entry");
  await expect(rows).toHaveCount(1);
  await expect(rows.first()).toContainText("Änderungen");
  await expect(rows.first()).toContainText("Bedienreste");
  await rows.first().getByTestId("paste-log-restore").click();
  await expect(again).toHaveValue(`Vorher. ${DIRTY}`);
});

test("restore is disabled with a reason once the cleaned text was edited; copy still works", async ({
  page,
}) => {
  const editor = await openEditor(page);
  await page.evaluate(() => {
    const w = window as unknown as { __copied?: string };
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        readText: async () => "",
        writeText: async (t: string) => {
          w.__copied = t;
        },
      },
    });
  });
  await paste(editor, DIRTY);
  await expect(editor).toHaveValue(`Vorher. ${CLEAN}`);
  await expect.poll(async () => (await savedState(page)) !== null).toBe(true);
  await editor.focus();
  await page.keyboard.press("Control+Home");
  await page.keyboard.type("Neu ");
  await page.getByTestId("editor-menu-cleanup").click();
  await page
    .getByTestId("editor-cleanup-pastelog")
    .evaluate((el) => el.click());
  const row = page.getByTestId("paste-log-entry").first();
  // Der Text wurde nicht im bereinigten Bereich geändert: Wiederherstellen geht noch.
  await expect(row.getByTestId("paste-log-restore")).toBeEnabled();
  await page.keyboard.press("Escape");
  await editor.focus();
  await page.keyboard.press("Control+End");
  await page.keyboard.type("!");
  await page.getByTestId("editor-menu-cleanup").click();
  await page
    .getByTestId("editor-cleanup-pastelog")
    .evaluate((el) => el.click());
  await expect(row.getByTestId("paste-log-restore")).toBeEnabled();
  await page.keyboard.press("Escape");
  // Jetzt den bereinigten Text selbst ändern.
  await editor.evaluate((el: HTMLTextAreaElement) => {
    el.focus();
    el.setSelectionRange(
      el.value.indexOf("Telefonanlage"),
      el.value.indexOf("Telefonanlage") + 3,
    );
    document.execCommand("insertText", false, "XYZ");
  });
  await page.getByTestId("editor-menu-cleanup").click();
  await page
    .getByTestId("editor-cleanup-pastelog")
    .evaluate((el) => el.click());
  await expect(row.getByTestId("paste-log-restore")).toBeDisabled();
  await expect(row.getByTestId("paste-log-reason")).toContainText(
    "nicht mehr unverändert",
  );
  await row.getByTestId("paste-log-copy").click();
  expect(
    await page.evaluate(
      () => (window as unknown as { __copied?: string }).__copied,
    ),
  ).toBe(DIRTY);
});

test("a text over 2 MiB is pasted unchanged with a notice and only a checksum is kept", async ({
  page,
}) => {
  const editor = await openEditor(page);
  const huge = `Drucken\nTeilen\n${"x".repeat(2 * 1024 * 1024)}`;
  expect(await paste(editor, huge)).toBe(false);
  await expect(toast(page)).toContainText("zu groß für die Bereinigung");
  await expect
    .poll(async () => {
      const state = (await savedState(page)) as {
        einfuegungen?: Record<string, unknown>[];
      } | null;
      return state?.einfuegungen?.length ?? 0;
    })
    .toBe(1);
  const state = (await savedState(page)) as {
    einfuegungen: Record<string, unknown>[];
  };
  const [entry] = state.einfuegungen;
  expect(entry.zuGross).toBe(true);
  expect(entry.roh).toBeUndefined();
  expect(entry.bereinigt).toBeUndefined();
  expect(String(entry.sha256)).toMatch(/^[0-9a-f]{64}$/);
});

test("if the raw text cannot be saved it is pasted unchanged", async ({
  page,
}) => {
  const editor = await openEditor(page);
  await page.evaluate(() => {
    (window as unknown as { __failSave?: boolean }).__failSave = true;
  });
  expect(await paste(editor, DIRTY)).toBe(true);
  await expect(editor).toHaveValue(`Vorher. ${DIRTY}`);
  await expect(toast(page)).toContainText("ließ sich nicht sichern");
});

test("the notice can be reached and used with the keyboard", async ({
  page,
}) => {
  const editor = await openEditor(page);
  await paste(editor, DIRTY);
  await expect(toast(page)).toContainText("Beim Einfügen bereinigt:");
  // Sonner: Alt+T springt in die Meldungen, die Aktion ist ein echter Knopf.
  await page.keyboard.press("Alt+KeyT");
  const live = page.locator("section[aria-live]");
  await expect(live).toHaveAttribute("aria-live", "polite");
  const button = undoNotice(page);
  await button.focus();
  await expect(button).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(editor).toHaveValue(`Vorher. ${DIRTY}`);
});
