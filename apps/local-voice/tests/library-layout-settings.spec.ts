import { test, expect, type Page } from "@playwright/test";
import { geheZu } from "./profilmenue";

// Einstellungen › Allgemein › „Bibliothek und Sicherungen“ (lva#365 P2b): Neu ordnen,
// Sicherungen zurückspielen und nach 30 Tagen löschen, verständliche Meldung,
// wenn das Laufwerk nicht sicher festschreiben kann (lva#371). Die Tauri-Brücke
// ist die schlanke Attrappe aus readaloud-files.spec.ts.

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
      tts_engine: "fish",
      tts_piper_voice: null,
      tts_export_format: "wav",
    };
    const voices = [
      {
        id: "patrick",
        meta: { display_name: "Patrick", tags: [] },
        origin: "recorded",
        avatar_path: null,
      },
    ];
    const w = window as unknown as {
      __lvVoice?: string | null;
      __savePaths?: string[];
      __lvLayout?: Record<string, unknown>;
    };
    w.__lvLayout ??= {
      layout: 1,
      phase: null,
      probleme: [],
      hinweise: [],
      zu_verschieben: 3,
      platz_benoetigt: 1_288_490_189,
      platz_frei: 50_000_000_000,
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
          if (cmd === "get_app_settings" || cmd === "get_default_settings")
            // Die Stimme je Test: null = Skript-Stimmen, sonst eine Einzelstimme.
            return { ...settings, tts_voice: w.__lvVoice ?? null };
          if (cmd === "plugin:os|locale") return "de-DE";
          if (cmd === "plugin:app|version") return "0.16.0";
          if (cmd.includes("permission")) return true;
          if (cmd === "plugin:event|listen") {
            // Ereignisse für Tests auslösbar: __lvEmit("library-layout-changed").
            const e = window as unknown as {
              __lvEmit?: (event: string) => void;
              __lvHandlers?: Map<string, number[]>;
            };
            e.__lvHandlers ??= new Map();
            const name = String(args?.event);
            e.__lvHandlers.set(name, [
              ...(e.__lvHandlers.get(name) ?? []),
              Number(args?.handler),
            ]);
            e.__lvEmit ??= (event: string) => {
              for (const id of e.__lvHandlers?.get(event) ?? [])
                (callbacks.get(id) as ((ev: unknown) => void) | undefined)?.({
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
          if (cmd === "tts_list_voice_infos") return voices;
          if (cmd === "tts_list_voices") return voices.map((v) => v.id);
          if (cmd === "pages_list") return [{ id: "p1", title: "Der Sturm" }];
          if (cmd === "page_files")
            return [
              {
                name: "Der-Sturm_2026-09-08_1405.wav",
                size: 120,
                modified_ms: 0,
              },
              {
                name: "CASE-GESPRÄCH-IE2S-·-28.09.2026_2026-09-28_1736.wav",
                size: 4096,
                modified_ms: 0,
              },
            ];
          if (cmd === "page_dir")
            return (
              (window as unknown as { __lvPageDir?: string }).__lvPageDir ??
              "C:\\projects\\p1"
            );
          if (cmd === "plugin:dialog|save") {
            const options = args?.options as { defaultPath?: string };
            w.__savePaths = [
              ...(w.__savePaths ?? []),
              options.defaultPath ?? "",
            ];
            // Abbrechen: der Test prueft nur den Vorschlag, nicht den Export.
            return null;
          }
          if (cmd === "tts_list_downloads") return [];
          const lv = window as unknown as {
            __lvLayout?: Record<string, unknown>;
            __lvBackups?: Record<string, unknown>[];
            __lvCalls?: string[];
            __lvRestoreError?: string;
            __lvSync?: string;
            __lvExportDurable?: boolean;
            __lvExportChecked?: boolean;
            __lvExportArgs?: Record<string, unknown>;
            __lvDialogCalls?: number;
            __lvDialogCancel?: boolean;
            __lvOpened?: unknown;
          };
          if (cmd === "plugin:dialog|open") {
            lv.__lvDialogCalls = (lv.__lvDialogCalls ?? 0) + 1;
            // Verzögert wie ein echter Dialog: ein zweiter Klick fällt in die Wartezeit.
            const answer = lv.__lvDialogCancel ? null : "D:\\Sicherungen";
            return new Promise((r) => setTimeout(() => r(answer), 150));
          }
          if (cmd === "plugin:opener|open_url") {
            lv.__lvOpened = args?.url;
            return null;
          }
          if (
            cmd.startsWith("library_layout") ||
            cmd.startsWith("library_backup")
          )
            lv.__lvCalls = [...(lv.__lvCalls ?? []), cmd];
          if (cmd === "library_layout_status") return lv.__lvLayout;
          if (cmd === "library_backup_list") return lv.__lvBackups ?? [];
          if (cmd === "library_layout_migrate") {
            lv.__lvLayout = {
              ...lv.__lvLayout,
              layout: 2,
              zu_verschieben: 0,
              phase: "fertig",
            };
            return {
              phase: "fertig",
              erledigt: 3,
              angehalten: false,
              probleme: [],
              hinweise: [],
            };
          }
          if (cmd === "library_layout_rollback") {
            lv.__lvLayout = {
              ...lv.__lvLayout,
              layout: 1,
              phase: "zurueckgenommen",
              probleme: [],
            };
            return {
              phase: "zurueckgenommen",
              erledigt: 2,
              angehalten: false,
              probleme: [],
              hinweise: [],
            };
          }
          if (cmd === "library_backup_sync_check")
            return {
              ergebnis: lv.__lvSync ?? "ok",
              meldung:
                lv.__lvSync === "fehler"
                  ? "Zugriff verweigert (os error 5)"
                  : null,
            };
          if (cmd === "library_backup_restore_best_effort")
            return { dateien: 7, papierkorb: "sicherungen/.papierkorb/2" };
          if (cmd === "library_backup_export") {
            lv.__lvExportArgs = args;
            return {
              ordner: "D:\\Sicherungen\\buch-ordner-1",
              dateien: 7,
              dauerhaft: lv.__lvExportDurable ?? true,
              ziel_vollstaendig_geprueft: lv.__lvExportChecked ?? true,
            };
          }
          if (cmd === "library_backup_restore") {
            if (lv.__lvRestoreError) throw lv.__lvRestoreError;
            return { dateien: 7, papierkorb: "sicherungen/.papierkorb/1" };
          }
          if (cmd === "library_backup_delete") {
            lv.__lvBackups = [];
            return null;
          }
          if (
            cmd === "llm_ps" ||
            cmd === "tts_reading_list" ||
            cmd.includes("history") ||
            cmd.includes("models") ||
            cmd.includes("devices") ||
            cmd === "meetings_list"
          )
            return [];
          if (cmd === "get_custom_sounds") return { start: false, stop: false };
          return null;
        },
      },
    });
  });
});

const openApp = async (page: Page) => {
  await page.goto("/");
  await geheZu(page, "Einstellungen");
  await page.getByRole("tab", { name: "Allgemein" }).click();
  await expect(page.getByTestId("library-layout")).toBeVisible();
};

const calls = (page: Page) =>
  page.evaluate(
    () => (window as unknown as { __lvCalls?: string[] }).__lvCalls ?? [],
  );

test("reorganising asks first, then reports the moved pages", async ({
  page,
}) => {
  await openApp(page);
  await expect(page.getByTestId("library-layout-status")).toHaveText(
    "3 Seiten liegen noch außerhalb ihrer Buch-Ordner.",
  );
  await page.getByTestId("library-layout-open").click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("Bibliothek neu ordnen?");
  await expect(dialog).toContainText("Vorher wird eine Sicherung angelegt");
  expect(await calls(page)).not.toContain("library_layout_migrate");
  await page.getByTestId("library-layout-apply").click();
  await expect(
    page.getByText("3 Seiten wurden in ihre Buch-Ordner verschoben."),
  ).toBeVisible();
  await expect(page.getByTestId("library-layout-status")).toHaveText(
    "Alle Bücher liegen in ihren Buch-Ordnern.",
  );
  await expect(page.getByTestId("library-layout-rollback")).toBeVisible();
});

test("too little free space blocks reorganising", async ({ page }) => {
  await page.addInitScript(() => {
    (window as unknown as { __lvLayout: unknown }).__lvLayout = {
      layout: 1,
      phase: null,
      probleme: [],
      hinweise: [],
      zu_verschieben: 3,
      platz_benoetigt: 2_000_000_000,
      platz_frei: 100_000_000,
    };
  });
  await openApp(page);
  await page.getByTestId("library-layout-open").click();
  await expect(page.getByRole("dialog")).toContainText(
    "Zu wenig freier Speicher",
  );
  await expect(page.getByTestId("library-layout-apply")).toBeDisabled();
});

test("a stopped run lists the duplicated folders", async ({ page }) => {
  await page.addInitScript(() => {
    (window as unknown as { __lvLayout: unknown }).__lvLayout = {
      layout: 1,
      phase: "gesichert",
      probleme: [
        {
          seite: "page_7",
          art: "doppelt",
          meldung: "Seite page_7 ist doppelt vorhanden.",
          pfade: ["projects/page_7", "library/book_1/fassungen/page_7"],
        },
      ],
      hinweise: [],
      zu_verschieben: 1,
      platz_benoetigt: 1000,
      platz_frei: null,
    };
  });
  await openApp(page);
  await expect(page.getByText("Neu ordnen angehalten")).toBeVisible();
  await expect(page.getByTestId("library-layout-problems")).toContainText(
    "library/book_1/fassungen/page_7",
  );
  await expect(page.getByTestId("library-layout-open")).toHaveText(
    "Neu ordnen fortsetzen",
  );
});

test("a stopped rollback continues as rollback, not as reorganising", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as unknown as { __lvLayout: unknown }).__lvLayout = {
      layout: 1,
      phase: "rueckfall",
      probleme: [],
      hinweise: [],
      zu_verschieben: 0,
      platz_benoetigt: 0,
      platz_frei: null,
    };
  });
  await openApp(page);
  // Nur eine Aktion, die den Rückfall weiterführt; kein zweites „Zurücknehmen“.
  await expect(page.getByTestId("library-layout-rollback")).toHaveCount(0);
  await expect(page.getByTestId("library-layout-open")).toHaveText(
    "Zurücknehmen fortsetzen",
  );
  await page.getByTestId("library-layout-open").click();
  // Doppelklick auf „Ausführen“ startet den Lauf nur einmal.
  await page.getByTestId("library-layout-apply").dblclick();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  const c = await calls(page);
  expect(c.filter((x) => x === "library_layout_rollback")).toHaveLength(1);
  expect(c).not.toContain("library_layout_migrate");
});

test("an open run can be rolled back before the layout switched", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as unknown as { __lvLayout: unknown }).__lvLayout = {
      layout: 1,
      phase: "gesichert",
      probleme: [],
      hinweise: [],
      zu_verschieben: 1,
      platz_benoetigt: 1000,
      platz_frei: null,
    };
  });
  await openApp(page);
  await page.getByTestId("library-layout-rollback").click();
  await page.getByTestId("library-layout-apply").click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(await calls(page)).toContain("library_layout_rollback");
  expect(await calls(page)).not.toContain("library_layout_migrate");
});

const withBackup = (page: Page, abgelaufen: boolean) =>
  page.addInitScript((alt) => {
    (window as unknown as { __lvBackups: unknown }).__lvBackups = [
      {
        name: "buch-ordner-1",
        erstellt_ms: Date.UTC(2026, 9, 1, 8, 0),
        dateien: 7,
        bytes: 3_500_000,
        abgelaufen: alt,
      },
    ];
  }, abgelaufen);

test("a backup is restored only after confirming", async ({ page }) => {
  await withBackup(page, false);
  await openApp(page);
  const row = page.getByTestId("library-backup");
  await expect(row).toContainText("7 Dateien");
  // Erst nach 30 Tagen bietet die Zeile das Löschen an.
  await expect(row.getByRole("button", { name: "Löschen …" })).toHaveCount(0);
  await row.getByRole("button", { name: "Zurückspielen …" }).click();
  await expect(page.getByRole("dialog")).toContainText(
    "nichts wird überschrieben",
  );
  await page.getByTestId("library-layout-apply").click();
  await expect(
    page.getByText("7 Dateien wurden zurückgespielt."),
  ).toBeVisible();
  await expect(page.getByText("sicherungen/.papierkorb/1")).toBeVisible();
});

test("a backup older than 30 days can be deleted after asking", async ({
  page,
}) => {
  await withBackup(page, true);
  await openApp(page);
  const row = page.getByTestId("library-backup");
  await expect(row).toContainText("Älter als 30 Tage");
  await row.getByRole("button", { name: "Löschen …" }).click();
  await expect(page.getByRole("dialog")).toContainText("älter als 30 Tage");
  await page.getByTestId("library-layout-apply").click();
  await expect(page.getByText("Die Sicherung wurde gelöscht.")).toBeVisible();
  await expect(page.getByTestId("library-backups-empty")).toBeVisible();
});

test("a drive without safe commit explains the way out (lva#371)", async ({
  page,
}) => {
  await withBackup(page, false);
  await page.addInitScript(() => {
    (window as unknown as { __lvRestoreError: string }).__lvRestoreError =
      "Zurückspielen nicht begonnen: C:\\x lässt sich auf diesem Dateisystem nicht dauerhaft sichern; nichts überschrieben oder gelöscht.";
  });
  await openApp(page);
  await page
    .getByTestId("library-backup")
    .getByRole("button", { name: "Zurückspielen …" })
    .click();
  await page.getByTestId("library-layout-apply").click();
  await expect(page.getByTestId("library-layout-nosync")).toHaveText(
    "Dieses Laufwerk kann Änderungen nicht sicher festschreiben",
  );
  await expect(
    page.getByRole("button", { name: "Datenordner öffnen" }),
  ).toBeVisible();
});

const drive = (page: Page, sync: string, durable = true) =>
  page.addInitScript(
    ([s, d]) => {
      Object.assign(window, { __lvSync: s, __lvExportDurable: d });
    },
    [sync, durable] as const,
  );

test("the drive is checked first; without sync only an explicit best effort restores (lva#371)", async ({
  page,
}) => {
  await withBackup(page, false);
  await drive(page, "nicht_unterstuetzt");
  await openApp(page);
  await page
    .getByTestId("library-backup")
    .getByRole("button", { name: "Zurückspielen …" })
    .click();
  await expect(page.getByTestId("library-restore-nosync")).toHaveText(
    "Dieses Laufwerk kann Änderungen nicht sicher festschreiben",
  );
  const apply = page.getByTestId("library-layout-apply");
  await expect(apply).toBeDisabled();
  await page
    .getByLabel(/Trotzdem zurückspielen, ohne Schutz bei Absturz/)
    .check();
  await expect(apply).toHaveText("Trotzdem zurückspielen");
  await apply.click();
  await expect(
    page.getByText("7 Dateien wurden zurückgespielt."),
  ).toBeVisible();
  const c = await calls(page);
  expect(c).toContain("library_backup_sync_check");
  expect(c).toContain("library_backup_restore_best_effort");
  expect(c).not.toContain("library_backup_restore");
});

test("a working drive restores normally after the check (lva#371)", async ({
  page,
}) => {
  await withBackup(page, false);
  await openApp(page);
  await page
    .getByTestId("library-backup")
    .getByRole("button", { name: "Zurückspielen …" })
    .click();
  await expect(page.getByTestId("library-restore-nosync")).toHaveCount(0);
  await page.getByTestId("library-layout-apply").click();
  await expect(
    page.getByText("7 Dateien wurden zurückgespielt."),
  ).toBeVisible();
  const c = await calls(page);
  expect(c).toContain("library_backup_restore");
  expect(c).not.toContain("library_backup_restore_best_effort");
});

test("without file sync restoring is blocked, no best effort, guide offered (lva#371)", async ({
  page,
}) => {
  await withBackup(page, false);
  await drive(page, "datei_nicht_unterstuetzt");
  await openApp(page);
  await page
    .getByTestId("library-backup")
    .getByRole("button", { name: "Zurückspielen …" })
    .click();
  await expect(page.getByTestId("library-restore-nosync")).toContainText(
    "nicht einmal Dateien sicher festschreiben",
  );
  await expect(page.getByTestId("library-restore-besteffort")).toHaveCount(0);
  await expect(page.getByTestId("library-layout-apply")).toBeDisabled();
  await expect(page.getByRole("dialog")).toContainText(
    "App vollständig beenden",
  );
  await page.getByTestId("library-restore-guide").click();
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { __lvOpened?: string }).__lvOpened,
      ),
    )
    .toContain("docs/DATENORDNER.md");
});

test("export opens one folder dialog at a time; cancelling keeps the dialog (lva#371)", async ({
  page,
}) => {
  await withBackup(page, false);
  await drive(page, "nicht_unterstuetzt");
  await page.addInitScript(() => {
    Object.assign(window, { __lvDialogCancel: true });
  });
  await openApp(page);
  await page
    .getByTestId("library-backup")
    .getByRole("button", { name: "Zurückspielen …" })
    .click();
  const exportButton = page.getByTestId("library-restore-export");
  await exportButton.dispatchEvent("click");
  await exportButton.dispatchEvent("click");
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(exportButton).toBeEnabled();
  const n = await page.evaluate(
    () => (window as unknown as { __lvDialogCalls?: number }).__lvDialogCalls,
  );
  expect(n).toBe(1);
  expect(await calls(page)).not.toContain("library_backup_export");
});

test("a failing drive check blocks restoring (lva#371)", async ({ page }) => {
  await withBackup(page, false);
  await drive(page, "fehler");
  await openApp(page);
  await page
    .getByTestId("library-backup")
    .getByRole("button", { name: "Zurückspielen …" })
    .click();
  await expect(page.getByRole("dialog")).toContainText(
    "Das Laufwerk ließ sich nicht prüfen: Zugriff verweigert",
  );
  await expect(page.getByTestId("library-layout-apply")).toBeDisabled();
});

for (const width of [1440, 390]) {
  test(`the backup is exported to a new folder (lva#371) @${width}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await withBackup(page, false);
    await drive(page, "nicht_unterstuetzt", false);
    // Bei 390 px: nicht jeder Vorfahr des Ziels war prüfbar (lva#375).
    await page.addInitScript((checked) => {
      Object.assign(window, { __lvExportChecked: checked });
    }, width !== 390);
    await openApp(page);
    await page
      .getByTestId("library-backup")
      .getByRole("button", { name: "Zurückspielen …" })
      .click();
    await expect(page.getByTestId("library-restore-nosync")).toBeVisible();
    if (width === 390) {
      // Touch-Ziele ≥ 44 px: Fußknöpfe, Schließen, Auswege.
      const dialog = page.getByRole("dialog");
      for (const target of [
        page.getByTestId("library-layout-apply"),
        dialog.getByRole("button", { name: "Später" }),
        dialog.getByRole("button", { name: "Schließen" }),
        page.getByTestId("library-restore-export"),
        page.getByTestId("library-restore-guide"),
      ]) {
        const box = await target.boundingBox();
        expect(box?.height ?? 0).toBeGreaterThanOrEqual(44);
        expect(box?.width ?? 0).toBeGreaterThanOrEqual(44);
      }
    }
    await page.screenshot({
      path: `test-results/sicherung-ohne-sync-${width}.png`,
    });
    await page.getByTestId("library-restore-export").click();
    await expect(page.getByTestId("library-layout-success")).toHaveText(
      "Die Sicherung wurde mit 7 Dateien nach D:\\Sicherungen\\buch-ordner-1 kopiert und geprüft.",
    );
    await expect(
      page.getByText("Auch dieser Ordner kann Änderungen nicht sicher"),
    ).toBeVisible();
    await expect(
      page.getByText("Nicht jeder übergeordnete Ordner des Ziels"),
    ).toHaveCount(width === 390 ? 1 : 0);
    const args = await page.evaluate(
      () =>
        (window as unknown as { __lvExportArgs?: Record<string, unknown> })
          .__lvExportArgs,
    );
    expect(args?.ziel).toBe("D:\\Sicherungen");
    await page.screenshot({
      path: `test-results/sicherung-exportiert-${width}.png`,
    });
  });
}
