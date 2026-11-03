import { test, expect, type Page } from "@playwright/test";
import { locateInMarkdown, saveErrorCode } from "../src/lib/minutesEdit";

// Protokoll von Hand bearbeiten: Stift in der Werkzeugzeile und je Abschnitt,
// Editor mit der Leiste des Vorlesen-Editors (Formatieren, Suchen, Alle
// ersetzen), Speichern mit optimistischer Sperre, Rueckfrage vor dem
// Neu-Erzeugen einer bearbeiteten Version und Versionsauswahl. Gegen die
// Tauri-Attrappe aus `meeting-minutes.spec.ts` (hier um das Speichern erweitert).
//
// Mit `LV_SHOTS=<Ordner>` entstehen zusaetzlich die Vorschaubilder fuer die
// Abnahme (390 und 1440 px, hell und dunkel) samt Touchziel-Messung.
// Den Ordner AUSSERHALB der App waehlen und danach nach docs/vorschau kopieren:
// jede neue Datei im App-Ordner laedt die Seiten im Vite-Server neu.

type Call = { cmd: string; args: Record<string, unknown> };

const BODY = [
  "# Protokoll: Kundentermin Meyer",
  "",
  "## Zusammenfassung",
  "",
  "- Besprochen wurde die Einarbeitung im Indilarco nach Personalwechseln.",
  "- Für das Vernetzungstreffen wird das Programm noch erstellt.",
  "",
  "## Besprochene Punkte",
  "",
  "- Nina Duke wurde über die Zusammenarbeit im Indilarco informiert.",
  "- Im **Indilarco** besteht derzeit wenig Handlungsspielraum.",
  "",
  "## Aufgaben",
  "",
  "- Die Lago und die eingegangenen E-Mails gemeinsam durchsehen. *(Bis: morgen)*",
].join("\n");

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(() => {
    const w = window as any;
    const callbacks = new Map<number, (e: unknown) => void>();
    const listeners: Record<string, number[]> = {};
    const handlerOf = new Map<number, { event: string; handler: number }>();
    let callback = 0;
    const settings = {
      onboarding_completed: true,
      app_language: "de",
      theme: "light",
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
    };
    const spec = {
      version: 1,
      context: "Zweck",
      sections: [
        { id: "a", title: "A", instruction: "", kind: "text" },
        { id: "b", title: "B", instruction: "", kind: "tasks" },
      ],
    };
    w.__settings = settings;
    w.__calls = [];
    w.__template = null; // gemerkte Wahl der Besprechung
    w.__auto = null; // was die Automatik zuletzt gewaehlt hat
    w.__meta = null; // Anzeigedaten des juengsten Protokolls
    w.__runState = {
      running: false,
      progress: null,
      cancelling: false,
      started_at: null,
    };
    // "ok" | "pending" | "busy" | Fehlertext
    w.__generate = "ok";
    w.__release = null; // beendet ein haengendes Erzeugen
    w.__documents = [];
    w.__templates = [
      {
        id: "builtin:allgemein",
        title: "Allgemein",
        builtin: true,
        spec,
        updated_at: 1,
      },
      {
        id: "builtin:vertrieb",
        title: "Kundengespräch / Vertrieb",
        builtin: true,
        spec,
        updated_at: 1,
      },
    ];
    w.__meetings = [
      {
        id: "m1",
        title: "Kundentermin Meyer",
        status: "ready",
        source: "recording",
        started_at: 1790000000,
        ended_at: 1790000600,
        language: "de",
        mic_audio_path: null,
        system_audio_path: null,
        duration_ms: 600000,
        consent_confirmed_at: 1790000000,
        audio_retention_until: null,
        created_at: 1790000000,
        source_path: null,
      },
    ];
    w.__minutesDoc = (version: number, body: string) => ({
      id: `p${version}`,
      meeting_id: "m1",
      kind: "minutes",
      body_format: "markdown@1",
      body,
      version,
      created_at: 1790000900 + version,
      template_id: null,
      updated_at: 300 + version,
    });
    w.__emit = (event: string, payload: unknown) =>
      (listeners[event] ?? []).forEach((h) =>
        callbacks.get(h)?.({ event, id: 0, payload }),
      );

    Object.assign(window, {
      __TAURI_OS_PLUGIN_INTERNALS__: {
        platform: "windows",
        os_type: "windows",
        family: "windows",
        arch: "x86_64",
        version: "11",
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
        convertFileSrc: (p: string) => p,
        invoke: async (cmd: string, args: Record<string, unknown> = {}) => {
          w.__calls.push({ cmd, args });
          switch (cmd) {
            case "get_app_settings":
            case "get_default_settings":
              return settings;
            case "plugin:os|locale":
              return "de-DE";
            case "plugin:app|version":
              return "0.14.0";
            case "plugin:event|listen": {
              (listeners[args.event as string] ??= []).push(
                args.handler as number,
              );
              const eventId = ++callback;
              handlerOf.set(eventId, {
                event: args.event as string,
                handler: args.handler as number,
              });
              return eventId;
            }
            case "plugin:event|unlisten": {
              const entry = handlerOf.get(args.eventId as number);
              if (entry) {
                listeners[entry.event] = (listeners[entry.event] ?? []).filter(
                  (h) => h !== entry.handler,
                );
                handlerOf.delete(args.eventId as number);
              }
              return null;
            }
            case "get_selected_model":
              return "";
            case "meetings_start":
              return { ...w.__meetings[0], id: "m-neu", status: "recording" };
            case "meetings_is_recording":
              return false;
            case "meetings_recording_position":
              return null;
            case "meetings_list":
              return w.__meetings;
            case "meetings_get_segments":
              return [];
            case "meetings_get_documents":
              return w.__documents;
            case "meetings_minutes_file":
              return null;
            case "meetings_get_template":
              return w.__template;
            case "meetings_set_template":
              w.__template = args.templateId;
              return null;
            case "meetings_get_auto_template":
              return w.__auto;
            case "meetings_minutes_meta":
              return w.__meta;
            case "meetings_minutes_state":
              return w.__runState;
            case "meetings_generate_minutes": {
              const mode = w.__generate as string;
              if (mode === "busy") throw "minutes_busy";
              if (mode === "pending") {
                w.__runState = {
                  running: true,
                  progress: { phase: "write", done: 1, total: 4 },
                  cancelling: false,
                  started_at: 1790001000,
                };
                await new Promise<void>((resolve) => {
                  w.__release = resolve;
                });
              } else if (mode !== "ok") {
                throw mode;
              }
              w.__documents = [
                ...w.__documents,
                w.__minutesDoc(1, "# Protokoll: Kundentermin Meyer\n\nFertig."),
              ];
              return w.__documents[w.__documents.length - 1];
            }
            case "meetings_update_document_text": {
              const doc = w.__documents.find(
                (d: { id: string }) => d.id === args.documentId,
              );
              if (!doc) throw "document_not_found";
              if (w.__stale || args.expectedUpdatedAt !== doc.updated_at)
                throw "stale_document";
              doc.body = args.body;
              doc.updated_at += 5000;
              doc.edited = true;
              return { ...doc };
            }
            case "meeting_notes_get":
              // Die Notizen sind der erste Reiter der Arbeitsflaeche.
              return {
                meeting_id: args.meetingId,
                blocks: [],
                revision: 0,
                updated_at: 0,
              };
            case "meeting_templates_list":
              return w.__templates;
            case "plugin:dialog|save":
              return null;
            case "tts_server_status":
              return { phase: "stopped", message: null };
            case "get_custom_sounds":
              return { start: false, stop: false };
            case "pages_list":
            case "page_files":
            case "tts_list_voices":
            case "tts_list_voice_infos":
            case "llm_ps":
            case "tts_reading_list":
            case "tts_list_downloads":
            case "llm_local_list":
            case "action_items_list":
              return [];
          }
          if (
            cmd.includes("permission") ||
            cmd.includes("history") ||
            cmd.includes("models") ||
            cmd.includes("devices")
          )
            return cmd.includes("permission") ? true : [];
          return null;
        },
      },
    });
    localStorage.setItem("lva.ui.settings.tab", "dictation");
  });
});

const calls = (page: Page, cmd: string) =>
  page.evaluate(
    (c) => (window as any).__calls.filter((x: Call) => x.cmd === c) as Call[],
    cmd,
  );

const emit = (page: Page, event: string, payload: unknown) =>
  page.evaluate(([e, p]) => (window as any).__emit(e, p), [event, payload]);

const openRecordings = async (page: Page, width = 1280, height = 900) => {
  await page.setViewportSize({ width, height });
  await page.goto("/");
  // Schmal ist die Navigation eingeklappt: ueber die Startkachel.
  if (width < 640)
    await page
      .getByRole("button", { name: /^Besprechung oder Aufnahme/ })
      .click();
  else
    await page.getByRole("button", { name: "Aufnahmen", exact: true }).click();
};

const openMinutes = async (page: Page, width = 1280, height = 900) => {
  await openRecordings(page, width, height);
  if (width < 640)
    await page.getByRole("button", { name: "Projekte öffnen" }).click();
  await page.getByText("Kundentermin Meyer", { exact: true }).click();
  await page.getByRole("tab", { name: "Protokoll", exact: true }).click();
};

const withMinutes = (page: Page, edited = false) =>
  page.addInitScript(
    ([body, isEdited]) => {
      const w = window as any;
      const doc = w.__minutesDoc(1, body);
      doc.updated_at = 1790000901000;
      doc.edited = isEdited;
      w.__documents = [doc];
    },
    [BODY, edited] as const,
  );

const editor = (page: Page) =>
  page.getByTestId("minutes-editor").locator("textarea");

test.describe("Protokoll bearbeiten: Logik", () => {
  test("angeklickter Block wird im Markdown gefunden", () => {
    const at = locateInMarkdown(
      BODY,
      "Im Indilarco besteht derzeit wenig Handlungsspielraum.",
    );
    expect(at).not.toBeNull();
    expect(BODY.slice(at!.start, at!.end)).toBe(
      "Im **Indilarco** besteht derzeit wenig Handlungsspielraum",
    );
    const heading = locateInMarkdown(BODY, "Aufgaben");
    expect(BODY.slice(heading!.start, heading!.end)).toBe("Aufgaben");
    expect(locateInMarkdown(BODY, "gibt es nicht")).toBeNull();
    expect(locateInMarkdown(BODY, "   ")).toBeNull();
    // Wortgrenzen: "Lago" trifft nicht "Lagos".
    expect(locateInMarkdown("Lagos und Lago", "Lago")!.start).toBe(10);
  });

  test("Fehlercodes des Speicherns", () => {
    expect(saveErrorCode("stale_document")).toBe("stale_document");
    expect(saveErrorCode("document_empty: x")).toBe("document_empty");
    expect(saveErrorCode("store_failed: disk")).toBe("other");
  });
});

test.describe("Protokoll bearbeiten", () => {
  test("Stift oeffnet den Editor mit der Leiste des Vorlesen-Editors; Speichern landet im Dokument", async ({
    page,
  }) => {
    await withMinutes(page);
    await openMinutes(page);
    await expect(page.getByTestId("minutes-doc")).toContainText("Indilarco");
    await page.getByTestId("minutes-edit").click();

    await expect(page.getByTestId("minutes-editor")).toBeVisible();
    await expect(page.getByTestId("editor-menu")).toBeVisible();
    await expect(page.getByTestId("editor-menu-format-bold")).toBeVisible();
    await expect(editor(page)).toHaveValue(BODY);
    // Neu erzeugen ist waehrend des Bearbeitens gesperrt.
    await expect(page.getByTestId("minutes-generate")).toBeDisabled();

    // Suchen und Ersetzen: alle "Indilarco" -> "Enelaco".
    await page.getByTestId("editor-menu-find").click();
    // Beschriftet: "Suchen und Ersetzen" oeffnet beide Felder.
    await expect(page.getByTestId("editor-menu-find")).toHaveText(
      "Suchen und Ersetzen",
    );
    await expect(page.getByLabel("Suchen nach")).toBeVisible();
    await expect(page.getByLabel("Ersetzen durch")).toBeVisible();
    await expect(page.getByTestId("replace-all")).toHaveText("Alle ersetzen");
    await expect(page.getByTestId("replace-one")).toHaveText("Ersetzen");
    await page.getByTestId("find-input").fill("Indilarco");
    await expect(page.getByTestId("find-count")).toHaveText("1 von 3");
    await page.getByTestId("replace-input").fill("Enelaco");
    await page.getByTestId("replace-all").click();
    await expect(editor(page)).not.toHaveValue(/Indilarco/);

    await page.getByTestId("minutes-edit-save").click();
    await expect(page.getByTestId("minutes-editor")).toHaveCount(0);
    const saves = await calls(page, "meetings_update_document_text");
    expect(saves).toHaveLength(1);
    expect(saves[0].args.documentId).toBe("p1");
    expect(saves[0].args.expectedUpdatedAt).toBe(1790000901000);
    expect(saves[0].args.body).toBe(BODY.replaceAll("Indilarco", "Enelaco"));
    await expect(page.getByTestId("minutes-doc")).toContainText("Enelaco");
    await expect(page.getByTestId("minutes-doc")).not.toContainText(
      "Indilarco",
    );
    await expect(page.getByTestId("minutes-edited")).toBeVisible();

    // Herunterladen nutzt dasselbe (gespeicherte) Dokument.
    await page.getByTestId("minutes-export").click();
    // Der Speichern-Dialog der Attrappe liefert null: kein Export, kein Fehler.
    await expect(page.getByRole("alert")).toHaveCount(0);
  });

  test("Stift am Abschnitt oeffnet den Editor an dieser Stelle", async ({
    page,
  }) => {
    await withMinutes(page);
    await openMinutes(page);
    const item = page
      .getByTestId("minutes-doc")
      .getByText("besteht derzeit wenig Handlungsspielraum", { exact: false });
    await item.hover();
    await page.getByTestId("minutes-block-edit").click();
    await expect(page.getByTestId("minutes-editor")).toBeVisible();
    const selected = await editor(page).evaluate((el) => {
      const ta = el as HTMLTextAreaElement;
      return ta.value.slice(ta.selectionStart, ta.selectionEnd);
    });
    expect(selected).toBe(
      "Im **Indilarco** besteht derzeit wenig Handlungsspielraum",
    );
  });

  test("Abschnitt bearbeiten steht beschriftet an jeder Abschnittsueberschrift", async ({
    page,
  }) => {
    await withMinutes(page);
    await openMinutes(page);
    const buttons = page.getByTestId("minutes-section-edit");
    // Drei Abschnitte, der Titel nur, wenn sein Knopf nicht den des ersten
    // Abschnitts ueberdeckt.
    await expect(buttons.first()).toBeVisible();
    const count = await buttons.count();
    expect(count).toBeGreaterThanOrEqual(3);
    const boxes = [];
    for (let i = 0; i < count; i++)
      boxes.push((await buttons.nth(i).boundingBox())!);
    for (let i = 1; i < boxes.length; i++)
      expect(boxes[i].y).toBeGreaterThanOrEqual(
        boxes[i - 1].y + boxes[i - 1].height,
      );
    const target = page.getByRole("button", {
      name: "Abschnitt „Besprochene Punkte“ bearbeiten",
    });
    await expect(target).toHaveText("Abschnitt bearbeiten");
    // Der Knopf steht auf der Hoehe seiner Ueberschrift.
    const heading = await page
      .getByTestId("minutes-doc")
      .getByRole("heading", { name: "Besprochene Punkte" })
      .boundingBox();
    const button = await target.boundingBox();
    expect(
      Math.abs(
        heading!.y + heading!.height / 2 - (button!.y + button!.height / 2),
      ),
    ).toBeLessThan(6);
    await target.click();
    const selected = await editor(page).evaluate((el) => {
      const ta = el as HTMLTextAreaElement;
      return ta.value.slice(ta.selectionStart, ta.selectionEnd);
    });
    expect(selected).toBe("Besprochene Punkte");
    // Kopieren nimmt weder Knoepfe noch Markierung mit.
  });

  test("Doppelklick auf eine Ueberschrift oeffnet den Editor dort", async ({
    page,
  }) => {
    await withMinutes(page);
    await openMinutes(page);
    await page
      .getByTestId("minutes-doc")
      .getByRole("heading", { name: "Aufgaben" })
      .dblclick();
    const selected = await editor(page).evaluate((el) => {
      const ta = el as HTMLTextAreaElement;
      return ta.value.slice(ta.selectionStart, ta.selectionEnd);
    });
    expect(selected).toBe("Aufgaben");
  });

  test("Abbrechen mit Aenderungen fragt nach; Strg+S speichert", async ({
    page,
  }) => {
    await withMinutes(page);
    await openMinutes(page);
    await page.getByTestId("minutes-edit").click();
    await editor(page).fill(BODY + "\n- Nachtrag");
    await page.getByTestId("minutes-edit-cancel").click();
    await expect(page.getByTestId("minutes-edit-discard")).toBeVisible();
    await page.getByTestId("minutes-edit-keep").click();
    await editor(page).press("Control+s");
    await expect(page.getByTestId("minutes-editor")).toHaveCount(0);
    await expect(page.getByTestId("minutes-doc")).toContainText("Nachtrag");

    // Ohne Aenderung schliesst Abbrechen sofort.
    await page.getByTestId("minutes-edit").click();
    await page.getByTestId("minutes-edit-cancel").click();
    await expect(page.getByTestId("minutes-editor")).toHaveCount(0);
  });

  test("Entwurf ueberlebt Reiterwechsel und einen fertigen Lauf im Hintergrund", async ({
    page,
  }) => {
    await withMinutes(page);
    await openMinutes(page);
    await page.getByTestId("minutes-edit").click();
    await editor(page).fill(BODY + "\n- Entwurf");
    // Kopieren, Herunterladen und Vorlesen sind waehrend des Bearbeitens gesperrt.
    await expect(page.getByTestId("minutes-copy")).toBeDisabled();
    await expect(page.getByTestId("minutes-export")).toBeDisabled();

    // Ein Lauf (z. B. aus dem Menue) legt Version 2 an und meldet "done".
    await page.evaluate(() => {
      const w = window as any;
      w.__documents.push(w.__minutesDoc(2, "# Protokoll: Neu"));
      w.__emit("minutes-event", { kind: "done", meeting_id: "m1" });
    });
    await expect(editor(page)).toHaveValue(/Entwurf/);
    await expect(page.getByTestId("minutes-editor")).toContainText("Version 1");

    // Reiterwechsel haengt den Editor aus; zurueck ist der Entwurf wieder da.
    await page.getByRole("tab", { name: "Transkript", exact: true }).click();
    await page.getByRole("tab", { name: "Protokoll", exact: true }).click();
    await expect(editor(page)).toHaveValue(/Entwurf/);
    await page.getByTestId("minutes-edit-save").click();
    const saves = await calls(page, "meetings_update_document_text");
    expect(saves[0].args.documentId).toBe("p1");
    // Nach dem Speichern bleibt die bearbeitete (aeltere) Version gewaehlt.
    await expect(page.getByTestId("minutes-old-version")).toBeVisible();
  });

  test("veralteter Stand: nichts gespeichert, Text bleibt im Editor", async ({
    page,
  }) => {
    await withMinutes(page);
    await page.addInitScript(() => {
      (window as any).__stale = true;
    });
    await openMinutes(page);
    await page.getByTestId("minutes-edit").click();
    await editor(page).fill(BODY + "\n- Meine Korrektur");
    await page.getByTestId("minutes-edit-save").click();
    await expect(page.getByTestId("minutes-edit-error")).toContainText(
      "inzwischen geändert",
    );
    await expect(editor(page)).toHaveValue(/Meine Korrektur/);
  });

  test("Neu erzeugen einer bearbeiteten Version fragt nach; die Version bleibt waehlbar", async ({
    page,
  }) => {
    await withMinutes(page, true);
    await openMinutes(page);
    await expect(page.getByTestId("minutes-edited")).toBeVisible();

    await page.getByTestId("minutes-generate").click();
    await expect(page.getByTestId("regenerate-edited-dialog")).toBeVisible();
    await page.getByTestId("regenerate-edited-cancel").click();
    await expect(page.getByTestId("regenerate-edited-dialog")).toHaveCount(0);
    expect(await calls(page, "meetings_generate_minutes")).toHaveLength(0);

    await page.getByTestId("minutes-generate").click();
    await page.getByTestId("regenerate-edited-go").click();
    await expect
      .poll(async () => (await calls(page, "meetings_generate_minutes")).length)
      .toBe(1);
  });

  test("Versionen: aeltere bearbeitete Version anzeigen und zurueck", async ({
    page,
  }) => {
    await withMinutes(page, true);
    await page.addInitScript(() => {
      const w = window as any;
      w.__documents.push(w.__minutesDoc(2, "# Protokoll: Neu erzeugt"));
    });
    await openMinutes(page);
    await expect(page.getByTestId("minutes-doc")).toContainText("Neu erzeugt");
    await page.getByTestId("minutes-versions").click();
    await page.getByTestId("minutes-version-1").click();
    await expect(page.getByTestId("minutes-old-version")).toBeVisible();
    await expect(page.getByTestId("minutes-doc")).toContainText("Indilarco");
    await expect(page.getByTestId("minutes-edited")).toBeVisible();
    await page.getByTestId("minutes-version-latest").click();
    await expect(page.getByTestId("minutes-doc")).toContainText("Neu erzeugt");
  });
});

// ---------------------------------------------------------------------------
// Vorschaubilder fuer die Abnahme (nur mit LV_SHOTS)
// ---------------------------------------------------------------------------

const SHOTS = process.env.LV_SHOTS;

test.describe("Protokoll bearbeiten: Vorschau", () => {
  test.skip(!SHOTS, "nur mit LV_SHOTS");
  for (const theme of ["hell", "dunkel"] as const) {
    for (const width of [1440, 390]) {
      test(`Vorschau ${width} ${theme}`, async ({ page }) => {
        const height = width === 390 ? 844 : 960;
        await page.addInitScript(
          (t) => {
            (window as any).__settings.theme = t;
          },
          theme === "dunkel" ? "dark" : "light",
        );
        await withMinutes(page);
        await openMinutes(page, width, height);
        const name = (step: string) =>
          `${SHOTS}/protokoll-${step}-${width}-${theme}.png`;
        const item = page
          .getByTestId("minutes-doc")
          .getByText("besteht derzeit wenig Handlungsspielraum", {
            exact: false,
          });
        if (width === 390) {
          // Touch: kein Hover; oben beginnen, Abschnittsknoepfe und Hinweis
          // "Weiter unten" sichtbar.
          await expect(page.getByTestId("minutes-more-below")).toBeVisible();
        } else {
          await item.hover();
          await expect(page.getByTestId("minutes-block-edit")).toBeVisible();
        }
        await expect(
          page.getByTestId("minutes-section-edit").first(),
        ).toBeVisible();
        await page.screenshot({ path: name("1-stift") });

        await page
          .getByRole("button", {
            name: "Abschnitt „Zusammenfassung“ bearbeiten",
          })
          .click();
        await page.getByTestId("editor-menu-find").click();
        await page.getByTestId("find-input").fill("Indilarco");
        await page.getByTestId("replace-input").fill("Enelaco");
        await page.screenshot({ path: name("2-bearbeiten") });

        let touch: string[] = [];
        if (width === 390) {
          // Touchziele >= 44 px (boundingBox) in Werkzeugzeile und Editor.
          const boxes = await page
            .locator(
              ".minutes-toolbar button:visible, .minutes-edit button:visible, .minutes-edit input:visible",
            )
            .evaluateAll((els) =>
              els.map((el) => {
                const r = el.getBoundingClientRect();
                return {
                  name:
                    el.getAttribute("aria-label") ||
                    el.getAttribute("data-testid") ||
                    el.textContent?.trim() ||
                    el.tagName,
                  w: Math.round(r.width),
                  h: Math.round(r.height),
                };
              }),
            );
          touch = boxes.map((b) => `${b.name}: ${b.w} × ${b.h} px`);
          expect(boxes.filter((b) => b.h < 44 || b.w < 44)).toEqual([]);
        }

        await page.getByTestId("replace-all").click();
        await page.getByTestId("minutes-edit-save").click();
        await page.getByTestId("minutes-generate").click();
        await expect(
          page.getByTestId("regenerate-edited-dialog"),
        ).toBeVisible();
        await page.screenshot({ path: name("3-neu-erzeugen") });
        if (touch.length > 0) {
          const fs = await import("node:fs");
          fs.writeFileSync(
            `${SHOTS}/touchziele-390-${theme}.txt`,
            touch.join("\n") + "\n",
          );
        }
      });
    }
  }
});
