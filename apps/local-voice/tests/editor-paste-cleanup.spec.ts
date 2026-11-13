import { createHash } from "node:crypto";
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
      tts_engine:
        (window as unknown as { __engine?: string }).__engine ?? "piper",
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
            const w = window as unknown as {
              __failSave?: boolean;
              __saveDelay?: number;
            };
            if (w.__saveDelay)
              await new Promise((r) => setTimeout(r, w.__saveDelay));
            if (w.__failSave) throw "disk full";
            sessionStorage.setItem(`__ps_${args?.id}`, String(args?.state));
            return null;
          }
          if (cmd === "page_state_load") {
            const delay = (
              window as unknown as { __loadDelay?: Record<string, number> }
            ).__loadDelay?.[String(args?.id)];
            // Wie der Kern: erst lesen, dann (später) antworten, also ein
            // verspäteter Ladevorgang bringt den Stand von VOR dem Warten.
            const stored = sessionStorage.getItem(`__ps_${args?.id}`);
            if (delay) await new Promise((r) => setTimeout(r, delay));
            return stored;
          }
          if (cmd === "book_ai_document_import") {
            const delay = (window as unknown as { __importDelay?: number })
              .__importDelay;
            if (delay) await new Promise((r) => setTimeout(r, delay));
            return { id: "d1", title: "Alte Zusammenfassung" };
          }
          if (cmd === "book_ai_document_text") return "Kurz gesagt: alles gut.";
          if (cmd === "book_ai_summarize")
            return { id: "d9", title: "Eine Seite" };
          if (cmd.startsWith("book_ai_term") || cmd === "book_ai_documents")
            return [];
          if (cmd === "book_ai_templates")
            return [
              {
                id: "builtin:eine_seite",
                title: "Eine Seite",
                category: "ueberblick",
                description: "Das Buch auf einer Seite.",
                context: "",
                sections: [],
                target_words: 450,
                form: "Prose",
                audience: "allgemein",
                builtin: true,
              },
            ];
          if (cmd === "book_ai_estimate")
            return {
              figures: {
                input_tokens: 100,
                output_tokens: 50,
                calls: 1,
                cost_micro: 0,
              },
              local: true,
              measured: false,
              cached_steps: 0,
              model: "qwen3-8b",
            };
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
          if (cmd === "pages_list")
            return (
              (window as unknown as { __pagesList?: unknown }).__pagesList ?? [
                { id: "p1", title: "Der Sturm" },
              ]
            );
          if (cmd === "books_list" || cmd === "library_list") return [];
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
          if (cmd === "tts_list_voice_infos")
            return (
              (window as unknown as { __voiceInfos?: unknown[] })
                .__voiceInfos ?? []
            );
          if (
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

test("the notice can be reached and used with the real keyboard only", async ({
  page,
}) => {
  const editor = await openEditor(page);
  await paste(editor, DIRTY);
  await expect(toast(page)).toContainText("Beim Einfügen bereinigt:");
  // Sonner: Alt+T springt in die Meldungen, dann Tab bis zur Aktion, Enter.
  await page.keyboard.press("Alt+KeyT");
  await expect(page.locator("section[aria-live]")).toHaveAttribute(
    "aria-live",
    "polite",
  );
  const focusedText = () =>
    page.evaluate(() => document.activeElement?.textContent ?? "");
  for (let i = 0; i < 8 && (await focusedText()) !== "Rückgängig"; i += 1)
    await page.keyboard.press("Tab");
  expect(await focusedText()).toBe("Rückgängig");
  await page.keyboard.press("Enter");
  await expect(editor).toHaveValue(`Vorher. ${DIRTY}`);
});

const SECOND = [
  "Skip to content",
  "",
  "Zweiter Absatz mit Text, der lang genug ist, um bleiben zu dürfen.",
  "Seite 2 von 9",
].join("\n");

test("a pending paste does not touch another page after a page switch", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as unknown as { __pagesList: unknown }).__pagesList = [
      { id: "p1", title: "Der Sturm" },
      { id: "p2", title: "Das Wintermärchen" },
    ];
  });
  const editor = await openEditor(page);
  await expect
    .poll(async () => ((await savedState(page)) as { text: string })?.text)
    .toBe("Vorher. ");
  await page.evaluate(() => {
    (window as unknown as { __saveDelay: number }).__saveDelay = 1500;
  });
  expect(await paste(editor, DIRTY)).toBe(true);
  await page.getByText("Das Wintermärchen", { exact: true }).click();
  await expect(toast(page).last()).toContainText(
    "Einfügen abgebrochen: Seite gewechselt.",
    { timeout: 15_000 },
  );
  // Nichts wurde eingefügt: p2 hat keinen Text und kein Protokoll.
  const p2 = await page.evaluate(() => sessionStorage.getItem("__ps_p2"));
  expect(p2 === null || !p2.includes("Telefonanlage")).toBe(true);
  // Die Rohfassung blieb bei der Seite des Einfügens gesichert, der Text dort unverändert.
  await expect
    .poll(async () => {
      const state = (await savedState(page)) as {
        text: string;
        einfuegungen?: { roh: string }[];
      };
      return JSON.stringify([
        state.text,
        state.einfuegungen?.map((e) => e.roh),
      ]);
    })
    .toBe(JSON.stringify(["Vorher. ", [DIRTY]]));
});

test("two simultaneous pastes keep both raw entries; only the first is inserted", async ({
  page,
}) => {
  const editor = await openEditor(page, "");
  await page.evaluate(() => {
    (window as unknown as { __saveDelay: number }).__saveDelay = 300;
  });
  await editor.focus();
  await editor.evaluate(
    (el: HTMLTextAreaElement, values: string[]) => {
      el.setSelectionRange(el.value.length, el.value.length);
      for (const value of values) {
        const data = new DataTransfer();
        data.setData("text/plain", value);
        el.dispatchEvent(
          new ClipboardEvent("paste", {
            clipboardData: data,
            bubbles: true,
            cancelable: true,
          }),
        );
      }
    },
    [DIRTY, SECOND],
  );
  await expect
    .poll(async () => {
      const state = (await savedState(page)) as {
        einfuegungen?: { roh: string }[];
      };
      return JSON.stringify(state?.einfuegungen?.map((e) => e.roh));
    })
    .toBe(JSON.stringify([DIRTY, SECOND]));
  await expect(editor).toHaveValue(CLEAN);
  await expect(
    toast(page).filter({ hasText: "Einfügen abgebrochen" }),
  ).toHaveCount(1);
});

test("limits count bytes: ä text is too large as an entry although short in characters", async ({
  page,
}) => {
  const editor = await openEditor(page, "");
  // 600 000 „ä“ = 1,2 MB je Fassung: Roh plus bereinigt über 2 MiB als JSON.
  const raw = `Drucken\nTeilen\n${"ä".repeat(600_000)}`;
  expect(await paste(editor, raw)).toBe(true);
  await expect(toast(page).last()).toContainText("zu groß für die Bereinigung");
  await expect(toast(page).last()).toContainText("2 MB");
  await expect
    .poll(async () => (await editor.inputValue()).length)
    .toBe(raw.length);
  const state = (await savedState(page)) as {
    einfuegungen: Record<string, unknown>[];
  };
  const [entry] = state.einfuegungen;
  expect(entry.zuGross).toBe(true);
  expect(entry.roh).toBeUndefined();
  expect(entry.bytes).toBe(2 * 600_000 + 15);
});

// -- Rohfassungen: Dialog, Löschen, Aufbewahrung ---------------------------------

/** Sha-256 wie die App (Rohtext, Zeilenenden vereinheitlicht). */
const sha256Of = (text: string) =>
  createHash("sha256").update(text, "utf8").digest("hex");

const entryFor = (i: number, extra: Record<string, unknown> = {}) => ({
  zeit: new Date(Date.UTC(2026, 10, 12, 8, i, 0)).toISOString(),
  sha256: sha256Of(`Roh ${i}\nDrucken\nTeilen`),
  roh: `Roh ${i}\nDrucken\nTeilen`,
  bereinigt: `Roh ${i}`,
  regeln: [{ rule: "webNoise", count: 2 }],
  ...extra,
});

/** Seite mit gespeichertem Zustand (Text und Rohfassungen) öffnen, Bearbeiten-Ansicht. */
async function openSeeded(
  page: Page,
  state: Record<string, unknown>,
): Promise<ReturnType<Page["locator"]>> {
  await page.addInitScript((json) => {
    sessionStorage.setItem("__ps_p1", json);
  }, JSON.stringify(state));
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await page.getByRole("button", { name: "Bearbeiten", exact: true }).click();
  const editor = page.locator("textarea").first();
  await expect(editor).toBeVisible();
  return editor;
}

async function openLog(page: Page) {
  await page.getByTestId("editor-menu-cleanup").click();
  await page
    .getByTestId("editor-cleanup-pastelog")
    .evaluate((el) => el.click());
  await expect(
    page.getByRole("dialog", { name: "Letzte Einfügungen" }),
  ).toBeVisible();
}

test("the dialog tells how long raw texts are kept; the switch item says it too", async ({
  page,
}) => {
  await openSeeded(page, {
    text: "Roh 1",
    tab: "original",
    einfuegungen: [entryFor(1)],
  });
  const sentence =
    "Rohfassungen der letzten 20 Einfügungen werden mit der Seite auf diesem Gerät gespeichert, bis du sie löschst.";
  await page.getByTestId("editor-menu-cleanup").click();
  await expect(page.getByTestId("editor-cleanup-paste")).toContainText(
    sentence,
  );
  await page
    .getByTestId("editor-cleanup-pastelog")
    .evaluate((el) => el.click());
  await expect(
    page.getByRole("dialog", { name: "Letzte Einfügungen" }),
  ).toContainText(sentence);
});

test("a single raw text can be deleted, the others stay", async ({ page }) => {
  await openSeeded(page, {
    text: "Roh 1 Roh 2",
    tab: "original",
    einfuegungen: [entryFor(1), entryFor(2)],
  });
  await openLog(page);
  const rows = page.getByTestId("paste-log-entry");
  await expect(rows).toHaveCount(2);
  // Neueste zuerst: die erste Zeile ist Eintrag 2.
  await rows.first().getByTestId("paste-log-delete").click();
  await expect(rows).toHaveCount(1);
  await expect
    .poll(async () => {
      const state = (await savedState(page)) as {
        einfuegungen?: { roh: string }[];
      };
      return JSON.stringify(state.einfuegungen?.map((e) => e.roh));
    })
    .toBe(JSON.stringify([entryFor(1).roh]));
});

test("delete all asks first; cancel keeps everything, confirm removes it", async ({
  page,
}) => {
  const editor = await openSeeded(page, {
    text: "Roh 1 Roh 2",
    tab: "original",
    einfuegungen: [entryFor(1), entryFor(2)],
  });
  await openLog(page);
  await page.getByTestId("paste-log-delete-all").click();
  const confirm = page.getByRole("dialog", {
    name: "Alle Rohfassungen löschen?",
  });
  await expect(confirm).toBeVisible();
  await confirm.locator("button.lv-btn", { hasText: "Abbrechen" }).click();
  await expect(page.getByTestId("paste-log-entry")).toHaveCount(2);
  await page.getByTestId("paste-log-delete-all").click();
  await page.getByTestId("paste-log-delete-all-yes").click();
  await expect(page.getByTestId("paste-log-empty")).toBeVisible();
  await expect
    .poll(
      async () =>
        ((await savedState(page)) as { einfuegungen?: unknown }).einfuegungen,
    )
    .toBeUndefined();
  await expect(editor).toHaveValue("Roh 1 Roh 2");
});

test("switching paste cleanup off saves nothing more and keeps existing entries", async ({
  page,
}) => {
  const editor = await openSeeded(page, {
    text: "Roh 1",
    tab: "original",
    einfuegungen: [entryFor(1)],
  });
  await page.getByTestId("editor-menu-cleanup").click();
  await page.getByTestId("editor-cleanup-paste").click();
  expect(await paste(editor, DIRTY)).toBe(false);
  await expect(toast(page)).toHaveCount(0);
  const state = (await savedState(page)) as { einfuegungen: { roh: string }[] };
  expect(state.einfuegungen.map((e) => e.roh)).toEqual([entryFor(1).roh]);
  await openLog(page);
  await expect(page.getByTestId("paste-log-entry")).toHaveCount(1);
});

test("a disabled restore button is reachable by Tab and names its reason", async ({
  page,
}) => {
  await openSeeded(page, {
    text: "Ganz anderer Text",
    tab: "original",
    einfuegungen: [entryFor(1)],
  });
  await openLog(page);
  const restore = page.getByTestId("paste-log-restore");
  await expect(restore).toHaveAttribute("aria-disabled", "true");
  await expect(restore).not.toHaveAttribute("disabled", "");
  const why = await restore.getAttribute("aria-describedby");
  expect(why).toContain("paste-log-why-0");
  await expect(page.locator("#paste-log-why-0")).toContainText(
    "nicht mehr unverändert",
  );
  // Per Tab erreichbar.
  await page.keyboard.press("Tab");
  for (let i = 0; i < 6; i += 1) {
    const on = await page.evaluate(() =>
      document.activeElement?.getAttribute("data-testid"),
    );
    if (on === "paste-log-restore") break;
    await page.keyboard.press("Tab");
  }
  await expect(restore).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("paste-log-entry")).toHaveCount(1);
  await expect(
    page.getByRole("dialog", { name: "Letzte Einfügungen" }),
  ).toBeVisible();
});

test("a damaged entry (checksum does not match) cannot be restored", async ({
  page,
}) => {
  const gut = entryFor(1);
  await openSeeded(page, {
    text: "Roh 1",
    tab: "original",
    einfuegungen: [
      gut,
      {
        ...entryFor(2),
        sha256: "f".repeat(64),
        zeit: "2026-11-12T09:00:00.000Z",
      },
    ],
  });
  await openLog(page);
  const rows = page.getByTestId("paste-log-entry");
  // Neueste zuerst: Eintrag 2 (beschädigt), dann Eintrag 1 (intakt, Text steht im Editor).
  await expect(rows.nth(0).getByTestId("paste-log-restore")).toHaveAttribute(
    "aria-disabled",
    "true",
  );
  await expect(rows.nth(0).getByTestId("paste-log-reason")).toContainText(
    "Beschädigt",
  );
  await expect(
    rows.nth(1).getByTestId("paste-log-restore"),
  ).not.toHaveAttribute("aria-disabled", "true");
});

test("a loaded oversized entry loses its raw text and shows only the size", async ({
  page,
}) => {
  const big = { ...entryFor(1), roh: "ä".repeat(1_100_000), bereinigt: "x" };
  await openSeeded(page, {
    text: "x",
    tab: "original",
    einfuegungen: [big, entryFor(2)],
  });
  await openLog(page);
  const rows = page.getByTestId("paste-log-entry");
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(1)).toContainText(
    "Zu groß für die Bereinigung (2,2 MB)",
  );
  await expect(rows.nth(1).getByTestId("paste-log-restore")).toHaveCount(0);
});

test("dialog with 20 entries: scrolls inside, traps focus, returns focus to the menu button", async ({
  page,
}) => {
  const many = Array.from({ length: 20 }, (_, i) => entryFor(i + 1));
  await openSeeded(page, {
    text: many.map((e) => e.bereinigt).join(" "),
    tab: "original",
    einfuegungen: many,
  });
  await openLog(page);
  const dialog = page.getByRole("dialog", { name: "Letzte Einfügungen" });
  await expect(page.getByTestId("paste-log-entry")).toHaveCount(20);
  // Der Dialog bleibt im Fenster, der Inhalt scrollt darin.
  const box = await dialog.boundingBox();
  const viewport = page.viewportSize();
  expect(
    box && viewport && box.y >= 0 && box.y + box.height <= viewport.height,
  ).toBe(true);
  const scroll = await page.getByTestId("paste-log-list").evaluate((el) => {
    const content = el.closest(".lv-dialog__content") as HTMLElement;
    const before = content.scrollTop;
    content.scrollTop = content.scrollHeight;
    return {
      scrollable: content.scrollHeight > content.clientHeight,
      moved: content.scrollTop > before,
    };
  });
  expect(scroll).toEqual({ scrollable: true, moved: true });
  // Fokusfalle: 60 Tabs, der Fokus verlässt den Dialog nie.
  for (let i = 0; i < 60; i += 1) {
    await page.keyboard.press("Tab");
    const inside = await page.evaluate(
      () => !!document.activeElement?.closest('[role="dialog"]'),
    );
    expect(inside).toBe(true);
  }
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(page.getByTestId("editor-menu-cleanup")).toBeFocused();
});

test("a delayed clipboard read does not paste into another page after a page switch", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as unknown as { __pagesList: unknown }).__pagesList = [
      { id: "p1", title: "Der Sturm" },
      { id: "p2", title: "Das Wintermärchen" },
    ];
  });
  const editor = await openEditor(page);
  await expect
    .poll(async () => ((await savedState(page)) as { text: string })?.text)
    .toBe("Vorher. ");
  await page.evaluate((text) => {
    const w = window as unknown as { __releaseClip?: () => void };
    const gate = new Promise<void>((r) => {
      w.__releaseClip = r;
    });
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        readText: async () => {
          await gate;
          return text;
        },
        writeText: async () => undefined,
      },
    });
  }, DIRTY);
  await editor.click({ button: "right" });
  await page
    .getByRole("menuitem", { name: "Einfügen", exact: true })
    .evaluate((el: HTMLElement) => el.click());
  await page.getByText("Das Wintermärchen", { exact: true }).click();
  await page.evaluate(() =>
    (window as unknown as { __releaseClip: () => void }).__releaseClip(),
  );
  await expect(toast(page).last()).toContainText(
    "Einfügen abgebrochen: Seite gewechselt.",
  );
  // Weder Seite 2 noch Seite 1 haben den Text bekommen.
  const p2 = await page.evaluate(() => sessionStorage.getItem("__ps_p2"));
  expect(p2 === null || !p2.includes("Telefonanlage")).toBe(true);
  const p1 = (await savedState(page)) as { text: string };
  expect(p1.text).toBe("Vorher. ");
});

test("the checksum check is pending: restore and copy are locked until it is done", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const w = window as unknown as {
      __digestRelease?: () => void;
    };
    const gate = new Promise<void>((r) => {
      w.__digestRelease = r;
    });
    const subtle = crypto.subtle;
    const original = subtle.digest.bind(subtle);
    subtle.digest = async (...args: Parameters<SubtleCrypto["digest"]>) => {
      await gate;
      return original(...args);
    };
  });
  const editor = await openSeeded(page, {
    text: "Roh 1",
    tab: "original",
    einfuegungen: [entryFor(1)],
  });
  await openLog(page);
  const restore = page.getByTestId("paste-log-restore");
  const copy = page.getByTestId("paste-log-copy");
  await expect(restore).toHaveAttribute("aria-disabled", "true");
  await expect(copy).toHaveAttribute("aria-disabled", "true");
  await expect(page.locator("#paste-log-why-0")).toContainText(
    "Prüfsumme wird geprüft",
  );
  expect(await restore.getAttribute("aria-describedby")).toContain(
    "paste-log-why-0",
  );
  // aria-disabled sperrt die Wirkung, nicht den Klick: er tut nichts.
  await restore.click({ force: true });
  await expect(editor).toHaveValue("Roh 1");
  await expect(
    page.getByRole("dialog", { name: "Letzte Einfügungen" }),
  ).toBeVisible();
  await page.evaluate(() =>
    (window as unknown as { __digestRelease: () => void }).__digestRelease(),
  );
  await expect(restore).not.toHaveAttribute("aria-disabled", "true");
  await expect(copy).not.toHaveAttribute("aria-disabled", "true");
  await restore.click();
  await expect(editor).toHaveValue(entryFor(1).roh);
});

test("a slow load of the page left meanwhile sets neither text nor raw log", async ({
  page,
}) => {
  await page.addInitScript(
    ([first, second]) => {
      const w = window as unknown as Record<string, unknown>;
      w.__pagesList = [
        { id: "p1", title: "Der Sturm" },
        { id: "p2", title: "Das Wintermärchen" },
      ];
      w.__loadDelay = { p1: 2500 };
      sessionStorage.setItem("__ps_p1", first);
      sessionStorage.setItem("__ps_p2", second);
    },
    [
      JSON.stringify({
        text: "Text von Seite eins",
        tab: "original",
        einfuegungen: [entryFor(1)],
      }),
      JSON.stringify({ text: "Text von Seite zwei", tab: "original" }),
    ],
  );
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await page.getByText("Das Wintermärchen", { exact: true }).click();
  await page.waitForTimeout(3500);
  await page.getByRole("button", { name: "Bearbeiten", exact: true }).click();
  const editor = page.locator("textarea").first();
  await expect(editor).toHaveValue("Text von Seite zwei");
  await openLog(page);
  await expect(page.getByTestId("paste-log-empty")).toBeVisible();
});

test("two dialogs: focus stays in the confirm dialog on top, then returns step by step", async ({
  page,
}) => {
  await openSeeded(page, {
    text: "Roh 1 Roh 2",
    tab: "original",
    einfuegungen: [entryFor(1), entryFor(2)],
  });
  await openLog(page);
  const log = page.getByRole("dialog", { name: "Letzte Einfügungen" });
  const confirm = page.getByRole("dialog", {
    name: "Alle Rohfassungen löschen?",
  });
  const inside = (title: string) =>
    page.evaluate(
      (t) =>
        !!document.activeElement
          ?.closest('[role="dialog"]')
          ?.textContent?.includes(t),
      title,
    );
  await page.getByTestId("paste-log-delete-all").click();
  await expect(confirm).toBeVisible();
  expect(await inside("Alle Rohfassungen löschen?")).toBe(true);
  for (let i = 0; i < 12; i += 1) {
    await page.keyboard.press(i % 3 === 2 ? "Shift+Tab" : "Tab");
    expect(await inside("Alle Rohfassungen löschen?")).toBe(true);
  }
  await page.keyboard.press("Escape");
  await expect(confirm).toBeHidden();
  await expect(log).toBeVisible();
  await expect(page.getByTestId("paste-log-delete-all")).toBeFocused();
  for (let i = 0; i < 12; i += 1) {
    await page.keyboard.press("Tab");
    expect(await inside("Letzte Einfügungen")).toBe(true);
  }
  await page.keyboard.press("Escape");
  await expect(log).toBeHidden();
  await expect(page.getByTestId("editor-menu-cleanup")).toBeFocused();
});

test("an autosave queued behind a raw-text write never overwrites it, even after a page switch", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as unknown as { __pagesList: unknown }).__pagesList = [
      { id: "p1", title: "Der Sturm" },
      { id: "p2", title: "Das Wintermärchen" },
    ];
  });
  const editor = await openEditor(page);
  await expect
    .poll(async () => ((await savedState(page)) as { text: string })?.text)
    .toBe("Vorher. ");
  await page.evaluate(() => {
    (window as unknown as { __saveDelay: number }).__saveDelay = 1500;
  });
  // 1. Schreiben der Rohfassung läuft (1,5 s) ...
  expect(await paste(editor, DIRTY)).toBe(true);
  // 2. ... dahinter reiht sich der Autosave eines neuen Textes ein ...
  await editor.press("X");
  await page.waitForTimeout(800);
  // 3. ... und die Seite wird gewechselt, bevor beides durch ist.
  await page.getByText("Das Wintermärchen", { exact: true }).click();
  await expect
    .poll(
      async () => {
        const state = (await savedState(page)) as {
          text: string;
          einfuegungen?: { roh: string }[];
        };
        return JSON.stringify([
          state.text,
          state.einfuegungen?.map((e) => e.roh),
        ]);
      },
      { timeout: 15_000 },
    )
    .toBe(JSON.stringify(["Vorher. X", [DIRTY]]));
});

test("an entry without checksum but with raw text is damaged, restore stays locked", async ({
  page,
}) => {
  await openSeeded(page, {
    text: "Roh 1",
    tab: "original",
    einfuegungen: [entryFor(1, { sha256: "" })],
  });
  await openLog(page);
  await expect(page.getByTestId("paste-log-restore")).toHaveAttribute(
    "aria-disabled",
    "true",
  );
  await expect(page.getByTestId("paste-log-reason")).toContainText(
    "Beschädigt",
  );
});

test("without WebCrypto a stored hash is not verifiable: restore locked with reason, copy stays", async ({
  page,
}) => {
  await page.addInitScript(() => {
    Object.defineProperty(window.crypto, "subtle", {
      configurable: true,
      get: () => undefined,
    });
  });
  await openSeeded(page, {
    text: "Roh 1",
    tab: "original",
    einfuegungen: [entryFor(1)],
  });
  await openLog(page);
  const restore = page.getByTestId("paste-log-restore");
  await expect(restore).toHaveAttribute("aria-disabled", "true");
  await expect(page.getByTestId("paste-log-reason")).toContainText(
    "Nicht prüfbar",
  );
  expect(await restore.getAttribute("aria-describedby")).toContain(
    "paste-log-why-0",
  );
  await expect(page.getByTestId("paste-log-copy")).not.toHaveAttribute(
    "aria-disabled",
    "true",
  );
  // Der Hinweis hängt auch am Kopierknopf (der Inhalt ist ungeprüft).
  expect(
    await page.getByTestId("paste-log-copy").getAttribute("aria-describedby"),
  ).toContain("paste-log-why-0");
});

test("a rejected digest() ends the check as not verifiable instead of hanging", async ({
  page,
}) => {
  await page.addInitScript(() => {
    crypto.subtle.digest = () => Promise.reject(new Error("digest kaputt"));
  });
  await openSeeded(page, {
    text: "Roh 1",
    tab: "original",
    einfuegungen: [entryFor(1)],
  });
  await openLog(page);
  await expect(page.getByTestId("paste-log-reason")).toContainText(
    "Nicht prüfbar",
  );
  await expect(page.getByTestId("paste-log-restore")).toHaveAttribute(
    "aria-disabled",
    "true",
  );
  await expect(page.getByTestId("paste-log-reason")).not.toContainText(
    "wird geprüft",
  );
});

test("without WebCrypto no raw version is written: the text is pasted unchanged with a notice", async ({
  page,
}) => {
  await page.addInitScript(() => {
    Object.defineProperty(window.crypto, "subtle", {
      configurable: true,
      get: () => undefined,
    });
  });
  const editor = await openEditor(page);
  expect(await paste(editor, DIRTY)).toBe(true);
  await expect(editor).toHaveValue(`Vorher. ${DIRTY}`);
  await expect(toast(page).last()).toContainText("Rohfassung ließ sich nicht");
  await expect
    .poll(async () => ((await savedState(page)) as { text: string })?.text)
    .toBe(`Vorher. ${DIRTY}`);
  expect(
    ((await savedState(page)) as { einfuegungen?: unknown }).einfuegungen,
  ).toBeUndefined();
});

test("a failed state write stops a dependent action with a notice; the chain keeps working", async ({
  page,
}) => {
  const editor = await openEditor(page);
  await expect
    .poll(async () => ((await savedState(page)) as { text: string })?.text)
    .toBe("Vorher. ");
  await page.evaluate(() => {
    (window as unknown as { __failSave: boolean }).__failSave = true;
  });
  const openExport = async () => {
    await page.getByRole("button", { name: "Weitere Aktionen" }).click();
    await page
      .getByTestId("tts-export-open")
      .evaluate((el: HTMLElement) => el.click());
  };
  await openExport();
  await expect(
    toast(page).filter({ hasText: "nicht gespeichert werden" }),
  ).toHaveCount(1);
  await expect(page.getByRole("dialog")).toHaveCount(0);
  // Der Fehler hat die Kette nicht angehalten: danach gelingt der nächste Auftrag.
  await page.evaluate(() => {
    (window as unknown as { __failSave: boolean }).__failSave = false;
  });
  expect(await paste(editor, DIRTY)).toBe(true);
  await expect
    .poll(async () => {
      const state = (await savedState(page)) as {
        einfuegungen?: { roh: string }[];
      };
      return JSON.stringify(state?.einfuegungen?.map((e) => e.roh));
    })
    .toBe(JSON.stringify([DIRTY]));
  await openExport();
  await expect(page.getByRole("dialog")).toBeVisible();
});

/**
 * Seite 1 verlassen, während das Sichern einer Rohfassung dort angehalten ist,
 * und wieder öffnen. Zeitplan (ms): Autosave „X“ belegt die Kette von 500 bis
 * 2000; das Einfügen reiht sich dahinter ein, die Seite wird gewechselt, das
 * Sichern läuft ab 2000 im Zweig der verlassenen Seite (landet bei 3500). Bei
 * 2300 wird Seite 1 wieder geöffnet. `loadDelay`: wie spät der Ladevorgang
 * antwortet (er liest den Stand von 2300, vor dem Sichern).
 */
async function leaveAndReopen(page: Page, loadDelay: number) {
  await page.addInitScript(() => {
    (window as unknown as { __pagesList: unknown }).__pagesList = [
      { id: "p1", title: "Der Sturm" },
      { id: "p2", title: "Das Wintermärchen" },
    ];
  });
  const editor = await openEditor(page);
  await expect
    .poll(async () => ((await savedState(page)) as { text: string })?.text)
    .toBe("Vorher. ");
  await page.evaluate(() => {
    (window as unknown as { __saveDelay: number }).__saveDelay = 1500;
  });
  await editor.press("End");
  await editor.press("X");
  await page.waitForTimeout(700);
  expect(await paste(editor, DIRTY)).toBe(true);
  await page.waitForTimeout(100);
  await page.getByText("Das Wintermärchen", { exact: true }).click();
  await page.waitForTimeout(1400);
  await page.evaluate((delay) => {
    (window as unknown as { __loadDelay: unknown }).__loadDelay = { p1: delay };
  }, loadDelay);
  await page.getByText("Der Sturm", { exact: true }).click();
}

/** Nach dem Wiedereröffnen im Editor tippen: das löst den Autosave der offenen Seite aus. */
async function typeAfterReopen(page: Page, mark: string) {
  const edit = page.getByRole("button", { name: "Bearbeiten", exact: true });
  if (await edit.count()) await edit.click();
  const editor = page.locator("textarea").first();
  await expect(editor).toBeVisible();
  await editor.press("End");
  await editor.press(mark);
}

test("a raw-text write stalled on the left page survives reopening it (fast load, then autosave)", async ({
  page,
}) => {
  await leaveAndReopen(page, 0);
  await page.waitForTimeout(2500);
  await typeAfterReopen(page, "Y");
  await expect
    .poll(
      async () => {
        const state = (await savedState(page)) as {
          text: string;
          einfuegungen?: { roh: string }[];
        };
        return JSON.stringify([
          state.text.endsWith("Y"),
          state.einfuegungen?.map((e) => e.roh),
        ]);
      },
      { timeout: 15_000 },
    )
    .toBe(JSON.stringify([true, [DIRTY]]));
});

test("a late load answer after the raw-text write is done does not roll the log back", async ({
  page,
}) => {
  await leaveAndReopen(page, 2500);
  // Die Ladeantwort kommt bei ~4800 und bringt den Stand von vor dem Sichern.
  await page.waitForTimeout(3200);
  await typeAfterReopen(page, "Y");
  await expect
    .poll(
      async () => {
        const state = (await savedState(page)) as {
          text: string;
          einfuegungen?: { roh: string }[];
        };
        return JSON.stringify([
          state.text.endsWith("Y"),
          state.einfuegungen?.map((e) => e.roh),
        ]);
      },
      { timeout: 20_000 },
    )
    .toBe(JSON.stringify([true, [DIRTY]]));
});

test("the legacy summary migration of a left page goes through the same chain as the raw-text write", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const w = window as unknown as Record<string, unknown>;
    w.__pagesList = [
      { id: "p1", title: "Der Sturm" },
      { id: "p2", title: "Das Wintermärchen" },
    ];
    w.__importDelay = 1500;
    sessionStorage.setItem(
      "__ps_p1",
      JSON.stringify({
        text: "Vorher. ",
        summary: "Alter Text",
        tab: "original",
      }),
    );
  });
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await page.getByRole("button", { name: "Bearbeiten", exact: true }).click();
  const editor = page.locator("textarea").first();
  await expect(editor).toHaveValue("Vorher. ");
  await page.evaluate(() => {
    (window as unknown as { __saveDelay: number }).__saveDelay = 1200;
  });
  // Die Kette ist von 600 bis 1800 belegt, das Sichern der Rohfassung wartet
  // dahinter; die Migration (Import fertig bei ~1500) trifft auf die Seite 2.
  await editor.press("End");
  await editor.press("X");
  await page.waitForTimeout(700);
  expect(await paste(editor, DIRTY)).toBe(true);
  await page.waitForTimeout(100);
  await page.getByText("Das Wintermärchen", { exact: true }).click();
  await expect
    .poll(
      async () => {
        const state = (await savedState(page)) as {
          summary?: string;
          einfuegungen?: { roh: string }[];
        };
        return JSON.stringify([
          state.summary ?? "",
          state.einfuegungen?.map((e) => e.roh),
        ]);
      },
      { timeout: 20_000 },
    )
    .toBe(JSON.stringify(["", [DIRTY]]));
});

test("if the checksum cannot be computed, the pasted text goes in unchanged with a notice (keyboard)", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  await page.addInitScript(() => {
    crypto.subtle.digest = () => Promise.reject(new Error("digest kaputt"));
  });
  const editor = await openEditor(page);
  expect(await paste(editor, DIRTY)).toBe(true);
  await expect(editor).toHaveValue(`Vorher. ${DIRTY}`);
  await expect(toast(page).last()).toContainText("Rohfassung ließ sich nicht");
  expect(errors).toEqual([]);
});

test("if the checksum cannot be computed, the context menu paste goes in unchanged with a notice", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  await page.addInitScript(() => {
    crypto.subtle.digest = () => Promise.reject(new Error("digest kaputt"));
  });
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
  await expect(editor).toHaveValue(`Vorher. ${DIRTY}`);
  await expect(toast(page).last()).toContainText("Rohfassung ließ sich nicht");
  expect(errors).toEqual([]);
});

test("copying a damaged original really copies it (clipboard stub) and carries the reason", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const copied: string[] = [];
    (window as unknown as { __copied: string[] }).__copied = copied;
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: async (text: string) => {
          copied.push(text);
        },
        readText: async () => "",
      },
    });
  });
  await openSeeded(page, {
    text: "Roh 1",
    tab: "original",
    einfuegungen: [entryFor(1, { sha256: "f".repeat(64) })],
  });
  await openLog(page);
  await expect(page.getByTestId("paste-log-restore")).toHaveAttribute(
    "aria-disabled",
    "true",
  );
  const copy = page.getByTestId("paste-log-copy");
  expect(await copy.getAttribute("aria-describedby")).toContain(
    "paste-log-why-0",
  );
  await expect(page.locator("#paste-log-why-0")).toContainText("Beschädigt");
  await copy.click();
  await expect(toast(page).last()).toContainText("Rohfassung kopiert");
  expect(
    await page.evaluate(
      () => (window as unknown as { __copied: string[] }).__copied,
    ),
  ).toEqual([entryFor(1).roh]);
});

test("copying an unverifiable original really copies it (clipboard stub) and carries the reason", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const copied: string[] = [];
    (window as unknown as { __copied: string[] }).__copied = copied;
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: async (text: string) => {
          copied.push(text);
        },
        readText: async () => "",
      },
    });
    // Die Prüfsumme lässt sich nicht berechnen: der Eintrag gilt als nicht prüfbar.
    crypto.subtle.digest = () => Promise.reject(new Error("digest kaputt"));
  });
  await openSeeded(page, {
    text: "Roh 1",
    tab: "original",
    einfuegungen: [entryFor(1)],
  });
  await openLog(page);
  await expect(page.locator("#paste-log-why-0")).toContainText("Nicht prüfbar");
  await expect(page.getByTestId("paste-log-restore")).toHaveAttribute(
    "aria-disabled",
    "true",
  );
  const copy = page.getByTestId("paste-log-copy");
  expect(await copy.getAttribute("aria-describedby")).toContain(
    "paste-log-why-0",
  );
  await copy.click();
  await expect(toast(page).last()).toContainText("Rohfassung kopiert");
  expect(
    await page.evaluate(
      () => (window as unknown as { __copied: string[] }).__copied,
    ),
  ).toEqual([entryFor(1).roh]);
});

test("a failed state write aborts creating a KI document: nothing is started, a notice appears", async ({
  page,
}) => {
  await openEditor(page);
  await page.getByRole("tab", { name: "KI-Dokumente" }).click();
  // Der Autosave nach dem Reiterwechsel ist durch, dann fällt das Schreiben aus.
  await expect
    .poll(async () => ((await savedState(page)) as { tab?: string })?.tab)
    .toBe("summary");
  await page.evaluate(() => {
    (window as unknown as { __failSave: boolean }).__failSave = true;
  });
  await page.getByTestId("tts-action-summarize").click();
  await page.getByTestId("kidoc-tpl-builtin:eine_seite").click();
  await page.getByTestId("kidoc-create").click();
  await expect(
    toast(page).filter({ hasText: "nicht gespeichert werden" }).first(),
  ).toBeVisible();
  await page.waitForTimeout(500);
  const calls = await page.evaluate(
    () => (window as unknown as { __calls: string[] }).__calls,
  );
  expect(calls).not.toContain("book_ai_summarize");
  // Gegenprobe: ohne Schreibfehler startet dieselbe Aktion.
  await page.evaluate(() => {
    (window as unknown as { __failSave: boolean }).__failSave = false;
  });
  await page.getByTestId("tts-action-summarize").click();
  await page.getByTestId("kidoc-tpl-builtin:eine_seite").click();
  await page.getByTestId("kidoc-create").click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as unknown as { __calls: string[] }).__calls.includes(
          "book_ai_summarize",
        ),
      ),
    )
    .toBe(true);
});

/**
 * Fremder Schreiber auf einer verlassenen Seite (lva#530 I1, Codex Runde 5):
 * „Sprecher ersetzen“ schreibt die Seite, während das Sichern einer Rohfassung
 * dort noch aussteht. Beides muss bestehen bleiben: der ersetzte Sprecher UND
 * die Rohfassung. Ohne gemeinsame Kette überschriebe der später landende
 * Schreiber den anderen.
 */
test("replace speaker on a left page while its raw-text write is still pending keeps both changes", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  await page.addInitScript(() => {
    const w = window as unknown as Record<string, unknown>;
    w.__pagesList = [
      { id: "p1", title: "Der Sturm" },
      { id: "p2", title: "Das Wintermärchen" },
    ];
    const voice = (id: string, name: string) => ({
      id,
      meta: { display_name: name, color: "teal", language: "de-DE", tags: [] },
      origin: "recorded",
      avatar_path: null,
    });
    w.__voiceInfos = [voice("erzaehlerin", "Erzählerin"), voice("leo", "Leo")];
    w.__voices = ["erzaehlerin", "leo"];
  });
  const editor = await openEditor(page, "<Erzählerin> Hallo. ");
  await expect
    .poll(async () => ((await savedState(page)) as { text: string })?.text)
    .toBe("<Erzählerin> Hallo. ");
  await page.evaluate(() => {
    (window as unknown as { __saveDelay: number }).__saveDelay = 4000;
  });
  expect(await paste(editor, DIRTY)).toBe(true);
  // Seite verlassen: das Sichern der Rohfassung hängt noch (4 s).
  await page.getByText("Das Wintermärchen", { exact: true }).click();
  await page.getByTestId("voice-select").click({ button: "right" });
  await page.getByTestId("voice-menu-replace").click();
  await page
    .getByTestId("replace-speaker-from")
    .selectOption({ label: "Erzählerin" });
  await page.getByTestId("replace-speaker-to").selectOption({ label: "Leo" });
  await expect(page.getByTestId("replace-speaker-preview")).toContainText(
    "1 Marker",
  );
  await page.getByTestId("replace-speaker-apply").click();
  await expect(page.getByTestId("replace-speaker-preview")).toContainText(
    "Fertig",
    { timeout: 20_000 },
  );
  await expect
    .poll(
      async () => {
        const state = (await savedState(page)) as {
          text: string;
          einfuegungen?: { roh: string }[];
        };
        return JSON.stringify([
          state.text.startsWith("<Leo> Hallo."),
          state.einfuegungen?.map((e) => e.roh),
        ]);
      },
      { timeout: 20_000 },
    )
    .toBe(JSON.stringify([true, [DIRTY]]));
  // Und es bleibt so (kein später landender Schreiber dreht es zurück).
  await page.waitForTimeout(5000);
  const state = (await savedState(page)) as {
    text: string;
    einfuegungen?: { roh: string }[];
  };
  expect(state.text.startsWith("<Leo> Hallo.")).toBe(true);
  expect(state.einfuegungen?.map((e) => e.roh)).toEqual([DIRTY]);
  expect(errors).toEqual([]);
});

/**
 * Die Rohfassungen haben nur eine Quelle, die Datei (Codex Runde 7): Ein
 * Fremdschreiber (Bibliothek hängt ein Transkript an) speichert einen neuen Text
 * über die Kette, danach löscht der Nutzer im Dialog eine Rohfassung. Eine reine
 * Protokolländerung schreibt nur ihr Feld: der Text in der Datei bleibt der
 * neue, der Eintrag ist weg.
 */
test("deleting a raw text after a foreign text write keeps the new text in the file", async ({
  page,
}) => {
  await openSeeded(page, {
    text: "T0 ",
    tab: "original",
    einfuegungen: [entryFor(1), entryFor(2)],
  });
  // Der erste Autosave nach dem Laden ist durch (er schriebe sonst T0 zurück,
  // das ist der vorbestehende Text-Konflikt, nicht Gegenstand dieses Tests).
  await page.waitForTimeout(1500);
  const written = await page.evaluate(async () => {
    const mod = await import("/src/lib/reader/pageStore.ts");
    return mod.getPageStateStore().mutate("p1", (state) => {
      state.text = "T1 angehängt.";
      return true;
    });
  });
  expect(written).toBe(true);
  await openLog(page);
  const rows = page.getByTestId("paste-log-entry");
  await expect(rows).toHaveCount(2);
  await rows.first().getByTestId("paste-log-delete").click();
  await expect(rows).toHaveCount(1);
  await expect
    .poll(async () => {
      const state = (await savedState(page)) as {
        text: string;
        einfuegungen?: { roh: string }[];
      };
      return JSON.stringify([
        state.text,
        state.einfuegungen?.map((e) => e.roh),
      ]);
    })
    .toBe(JSON.stringify(["T1 angehängt.", [entryFor(1).roh]]));
  // Und es bleibt so: kein später landender Schreiber dreht den Text zurück.
  await page.waitForTimeout(1500);
  expect(((await savedState(page)) as { text: string }).text).toBe(
    "T1 angehängt.",
  );
});

/**
 * Vorbestehend, nicht Teil von lva#530 (Codex Runde 6, Fall 1): Eine Seite lädt
 * noch (die Antwort kommt verspätet und bringt den Stand von VOR dem Warten), ein
 * Fremdschreiber ändert den Text in der Datei (hier über die Kette des
 * Programms, `mutate`), danach schreibt der Autosave den geladenen alten Text
 * zurück. Der Test beschreibt das gewünschte Verhalten (die
 * Änderung bleibt) und ist deshalb als erwarteter Fehlschlag markiert; auf
 * origin/main schlägt er genauso fehl. Siehe lva#546.
 */
test("a foreign text change during a page load survives the autosave of the loaded page", async ({
  page,
}) => {
  test.fail(
    true,
    "vorbestehend auf main (Text bei fremden Schreibern), lva#546",
  );
  await page.addInitScript(() => {
    const w = window as unknown as Record<string, unknown>;
    w.__loadDelay = { p1: 2500 };
    sessionStorage.setItem(
      "__ps_p1",
      JSON.stringify({ text: "Vorher. ", tab: "original" }),
    );
  });
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await page.waitForTimeout(500);
  // Fremdschreiber (wie Bibliothek, Zusammenführen, Sprecher ersetzen) über
  // die Kette des Programms (derselbe Store wie im Vorlesen, per Vite-Import).
  void page.evaluate(async () => {
    const mod = await import("/src/lib/reader/pageStore.ts");
    return mod.getPageStateStore().mutate("p1", (state) => {
      state.text = "Fremd geändert. ";
      return true;
    });
  });
  await page.waitForTimeout(9000);
  expect(((await savedState(page)) as { text: string }).text).toBe(
    "Fremd geändert. ",
  );
});
