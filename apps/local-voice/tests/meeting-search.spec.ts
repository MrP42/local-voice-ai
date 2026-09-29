import { test, expect, type Page } from "@playwright/test";

// M4-P4d: Suche, Filter und Ordner in der Besprechungsliste gegen die
// Tauri-Attrappe. Die Attrappe merkt sich jeden Aufruf in `window.__calls`.

type Call = { cmd: string; args: Record<string, any> };

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(() => {
    const w = window as any;
    const callbacks = new Map<number, (e: unknown) => void>();
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
    };
    const meeting = (id: string, title: string, source = "live") => ({
      id,
      title,
      status: "ready",
      source,
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
      deleted_at: null,
    });
    const folder = (id: string, name: string, count = 0) => ({
      id,
      name,
      color: null,
      sort: 1,
      meeting_count: count,
      created_at: 1,
      updated_at: 1,
    });
    w.__calls = [];
    w.__meetings = [
      meeting("m1", "Kundentermin Meyer"),
      meeting("m2", "Teamrunde", "import"),
    ];
    w.__folders = [folder("f1", "Vertrieb", 1), folder("f2", "Projekte")];
    w.__assigned = { m1: ["f1"], m2: [] } as Record<string, string[]>;
    w.__nullFolders = false;

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
        invoke: async (cmd: string, args: Record<string, any> = {}) => {
          w.__calls.push({ cmd, args: JSON.parse(JSON.stringify(args)) });
          switch (cmd) {
            case "get_app_settings":
            case "get_default_settings":
              return settings;
            case "plugin:os|locale":
              return "de-DE";
            case "plugin:app|version":
              return "0.14.0";
            case "plugin:event|listen":
              return ++callback;
            case "plugin:event|unlisten":
              return null;
            case "meetings_is_recording":
              return false;
            case "meetings_recording_position":
              return null;
            case "meetings_list":
              return w.__meetings;
            case "meetings_search": {
              const q = String(args.query).toLowerCase();
              const f = args.filter;
              let list = w.__meetings.filter(
                (m: any) =>
                  (!f.folder_id ||
                    (w.__assigned[m.id] ?? []).includes(f.folder_id)) &&
                  (!f.source || m.source === f.source),
              );
              if (q.includes("budget")) {
                list = list.filter((m: any) => m.id === "m1");
                return {
                  items: list.map((m: any) => ({
                    meeting: m,
                    snippet: "Das &lt;b&gt; <mark>Budget</mark> für 2027 steht",
                    hit_source: "transcript",
                  })),
                  total: list.length,
                  truncated: false,
                };
              }
              if (q !== "") return { items: [], total: 0, truncated: false };
              return {
                items: list.map((m: any) => ({
                  meeting: m,
                  snippet: null,
                  hit_source: null,
                })),
                total: list.length,
                truncated: false,
              };
            }
            case "meeting_folders_list":
              return w.__nullFolders ? null : w.__folders;
            case "meeting_folders_save": {
              if (args.id) {
                w.__folders = w.__folders.map((x: any) =>
                  x.id === args.id ? { ...x, name: args.name } : x,
                );
                return w.__folders.find((x: any) => x.id === args.id);
              }
              const created = folder(`f${w.__folders.length + 1}`, args.name);
              w.__folders = [...w.__folders, created];
              return created;
            }
            case "meeting_folders_delete":
              w.__folders = w.__folders.filter((x: any) => x.id !== args.id);
              return null;
            case "meetings_get_folders":
              return w.__assigned[args.meetingId] ?? [];
            case "meetings_set_folders":
              w.__assigned[args.meetingId] = args.folderIds;
              return null;
            case "get_selected_model":
              return "";
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
            case "meeting_templates_list":
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

const openRecordings = async (page: Page) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/");
  await page.getByRole("button", { name: "Aufnahmen", exact: true }).click();
  await expect(page.getByText("Kundentermin Meyer")).toBeVisible();
};

const searchbox = (page: Page) =>
  page.getByRole("searchbox", { name: "Besprechungen durchsuchen" });

const folderGroup = (page: Page) => page.getByRole("group", { name: "Ordner" });

test.describe("Besprechungsliste: Suche, Filter, Ordner", () => {
  test("ohne Suchtext bleibt die bisherige Liste", async ({ page }) => {
    await openRecordings(page);
    await expect(page.getByText("Teamrunde")).toBeVisible();
    const list = await calls(page, "meetings_list");
    expect(list.length).toBeGreaterThan(0);
    expect(list[0].args).toEqual({ offset: 0, limit: 25 });
    expect(await calls(page, "meetings_search")).toHaveLength(0);
  });

  test("Suche ist entprellt: genau ein Aufruf je Tippserie", async ({
    page,
  }) => {
    await openRecordings(page);
    await searchbox(page).pressSequentially("Budget", { delay: 60 });
    await expect(page.getByTestId("meeting-search-snippet")).toBeVisible();
    await page.waitForTimeout(600);
    let search = await calls(page, "meetings_search");
    expect(search).toHaveLength(1);
    expect(search[0].args.query).toBe("Budget");
    expect(search[0].args.offset).toBe(0);
    expect(search[0].args.limit).toBe(25);
    expect(search[0].args.filter).toEqual({
      folder_id: null,
      from: null,
      to: null,
      source: null,
      has_notes: null,
    });

    // Zweite Tippserie: wieder genau ein Aufruf, mit dem ganzen Text.
    await searchbox(page).pressSequentially(" 2027", { delay: 60 });
    await page.waitForTimeout(600);
    search = await calls(page, "meetings_search");
    expect(search).toHaveLength(2);
    expect(search[1].args.query).toBe("Budget 2027");

    // Treffer ohne Snippet in der Attrappe: kein Treffer -> Hinweis.
    await searchbox(page).fill("Xylophon");
    await expect(
      page.getByText("Keine Treffer.", { exact: false }),
    ).toBeVisible();

    // Leeren kehrt zur bisherigen Liste zurueck (meetings_list).
    const before = (await calls(page, "meetings_list")).length;
    await page.getByRole("button", { name: "Suche leeren" }).click();
    await expect(page.getByText("Teamrunde")).toBeVisible();
    expect((await calls(page, "meetings_list")).length).toBeGreaterThan(before);
  });

  test("Treffer zeigen das Snippet mit <mark>, Rest als Text", async ({
    page,
  }) => {
    await openRecordings(page);
    await searchbox(page).fill("budget");
    const snippet = page.getByTestId("meeting-search-snippet");
    await expect(snippet).toBeVisible();
    await expect(snippet.locator("mark")).toHaveText("Budget");
    // Maskiertes HTML bleibt Text, es entsteht kein <b>-Element.
    await expect(snippet).toContainText("Das <b> Budget für 2027 steht");
    await expect(snippet.locator("b")).toHaveCount(0);
    await expect(page.getByText("Treffer in: Transkript")).toBeVisible();
    await expect(page.getByText("Teamrunde")).toHaveCount(0);
  });

  test("Ordner-Chip setzt folder_id, „Alle“ hebt ihn auf", async ({ page }) => {
    await openRecordings(page);
    await folderGroup(page)
      .getByRole("button", { name: /Vertrieb/ })
      .click();
    await expect(page.getByText("Teamrunde")).toHaveCount(0);
    const search = await calls(page, "meetings_search");
    expect(search.at(-1)!.args.filter.folder_id).toBe("f1");
    expect(search.at(-1)!.args.query).toBe("");
    await expect(
      folderGroup(page).getByRole("button", { name: /Vertrieb/ }),
    ).toHaveAttribute("aria-pressed", "true");

    await folderGroup(page)
      .getByRole("button", { name: "Alle", exact: true })
      .click();
    await expect(page.getByText("Teamrunde")).toBeVisible();

    // Ordner und Suche zusammen.
    await folderGroup(page)
      .getByRole("button", { name: /Vertrieb/ })
      .click();
    await searchbox(page).fill("Budget");
    await expect(page.getByTestId("meeting-search-snippet")).toBeVisible();
    const last = (await calls(page, "meetings_search")).at(-1)!;
    expect(last.args.query).toBe("Budget");
    expect(last.args.filter.folder_id).toBe("f1");
  });

  test("Filter-Chips setzen Zeitraum, Quelle und „mit Notizen“", async ({
    page,
  }) => {
    await openRecordings(page);
    const now = Math.floor(Date.now() / 1000);
    await page.getByRole("button", { name: "7 Tage" }).click();
    await page.getByRole("button", { name: "Import", exact: true }).click();
    await page.getByRole("button", { name: "Mit Notizen" }).click();
    await expect(page.getByText("Kundentermin Meyer")).toHaveCount(0);
    const last = (await calls(page, "meetings_search")).at(-1)!;
    expect(last.args.filter.source).toBe("import");
    expect(last.args.filter.has_notes).toBe(true);
    expect(Math.abs(last.args.filter.from - (now - 7 * 86400))).toBeLessThan(
      120,
    );
    // Zweiter Klick hebt den Chip auf.
    await page.getByRole("button", { name: "Import", exact: true }).click();
    const after = (await calls(page, "meetings_search")).at(-1)!;
    expect(after.args.filter.source).toBeNull();
  });

  test("Kontextmenü „In Ordner …“: Mehrfachwahl ruft meetings_set_folders", async ({
    page,
  }) => {
    await openRecordings(page);
    await page
      .locator("[data-meeting-id=m1]")
      .click({ button: "right", position: { x: 20, y: 10 } });
    const item = page.getByRole("menuitem", { name: "In Ordner …" });
    await expect(item).toBeVisible();
    await item.evaluate((el) => (el as HTMLElement).click());

    const dialog = page.getByRole("dialog");
    await expect(dialog.getByText("Kundentermin Meyer")).toBeVisible();
    const vertrieb = dialog.getByRole("checkbox", { name: "Vertrieb" });
    const projekte = dialog.getByRole("checkbox", { name: "Projekte" });
    await expect(vertrieb).toHaveAttribute("aria-checked", "true");
    await expect(projekte).toHaveAttribute("aria-checked", "false");
    await projekte.click();
    await dialog.getByRole("button", { name: "Speichern" }).click();
    await expect(dialog).toHaveCount(0);

    const set = await calls(page, "meetings_set_folders");
    expect(set).toHaveLength(1);
    expect(set[0].args).toEqual({
      meetingId: "m1",
      folderIds: ["f1", "f2"],
    });
    expect((await calls(page, "meetings_get_folders"))[0].args.meetingId).toBe(
      "m1",
    );
  });

  test("Ordner-Knopf der Zeile öffnet dieselbe Auswahl; neuer Ordner im Dialog", async ({
    page,
  }) => {
    await openRecordings(page);
    await page
      .locator("[data-meeting-id=m2]")
      .getByRole("button", { name: "In Ordner …" })
      .click();
    const dialog = page.getByRole("dialog");
    await dialog.getByRole("textbox", { name: "Neuer Ordner" }).fill("Kunden");
    await dialog.getByRole("button", { name: "Anlegen" }).click();
    await expect(
      dialog.getByRole("checkbox", { name: "Kunden" }),
    ).toHaveAttribute("aria-checked", "true");
    await dialog.getByRole("button", { name: "Speichern" }).click();
    const set = (await calls(page, "meetings_set_folders")).at(-1)!;
    expect(set.args).toEqual({ meetingId: "m2", folderIds: ["f3"] });
    await expect(
      folderGroup(page).getByRole("button", { name: /Kunden/ }),
    ).toBeVisible();
  });

  test("Ordner anlegen, umbenennen, löschen – Besprechungen bleiben", async ({
    page,
  }) => {
    await openRecordings(page);
    await folderGroup(page)
      .getByRole("button", { name: "Ordner anlegen" })
      .click();
    let dialog = page.getByRole("dialog");
    await dialog.getByRole("textbox").fill("Strategie");
    await dialog.getByRole("button", { name: "Anlegen" }).click();
    await expect(dialog).toHaveCount(0);
    const created = (await calls(page, "meeting_folders_save")).at(-1)!;
    expect(created.args).toEqual({ id: null, name: "Strategie", color: null });
    const chip = folderGroup(page).getByRole("button", { name: /Strategie/ });
    await expect(chip).toBeVisible();

    // Umbenennen per Kontextmenü des Chips.
    await chip.click({ button: "right" });
    await page
      .getByRole("menuitem", { name: "Umbenennen" })
      .evaluate((el) => (el as HTMLElement).click());
    dialog = page.getByRole("dialog");
    await expect(dialog.getByRole("textbox")).toHaveValue("Strategie");
    await dialog.getByRole("textbox").fill("Strategie 2027");
    await dialog.getByRole("button", { name: "Speichern" }).click();
    const renamed = (await calls(page, "meeting_folders_save")).at(-1)!;
    expect(renamed.args).toEqual({
      id: "f3",
      name: "Strategie 2027",
      color: null,
    });
    const renamedChip = folderGroup(page).getByRole("button", {
      name: /Strategie 2027/,
    });
    await expect(renamedChip).toBeVisible();

    // Aktiver Ordner wird gelöscht: Filter fällt zurück auf „Alle“.
    await renamedChip.click();
    await renamedChip.click({ button: "right" });
    await page
      .getByRole("menuitem", { name: "Löschen" })
      .evaluate((el) => (el as HTMLElement).click());
    dialog = page.getByRole("dialog");
    await expect(dialog).toContainText("Die Besprechungen darin bleiben");
    await dialog.getByRole("button", { name: "Löschen" }).click();
    expect((await calls(page, "meeting_folders_delete")).at(-1)!.args).toEqual({
      id: "f3",
    });
    await expect(
      folderGroup(page).getByRole("button", { name: /Strategie/ }),
    ).toHaveCount(0);
    await expect(
      folderGroup(page).getByRole("button", { name: "Alle", exact: true }),
    ).toHaveAttribute("aria-pressed", "true");
    await expect(page.getByText("Kundentermin Meyer")).toBeVisible();
    await expect(page.getByText("Teamrunde")).toBeVisible();
  });

  test("Backend ohne Ordner-Antwort (null): Liste bleibt bedienbar", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      (window as any).__nullFolders = true;
    });
    await openRecordings(page);
    await expect(
      folderGroup(page).getByRole("button", { name: "Alle", exact: true }),
    ).toBeVisible();
    await expect(
      folderGroup(page).getByRole("button", { name: /Vertrieb/ }),
    ).toHaveCount(0);
  });
});
