import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { calls, installTauriMock } from "./calendarMock";
import {
  DEMO,
  installIntegrationsMock,
  NOW,
  type IntegrationsMockOptions,
} from "./integrationsMock";

// A4 (Goal Integrationen, #66, AK7): die Seite „Integrationen“ zwischen
// „Modelle“ und „Einstellungen“: Liste, Katalog, Detail mit Richtung und
// Rechte-Matrix, Protokoll, Freigabedialog, Kalenderquellen als Karten und
// der MCP-Schalter, der aus den Einstellungen hierher gezogen ist.
// Gegen die Tauri-Attrappe; die Rechenregel der Rechte pruefen die Rust-Tests.

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

const setup = async (
  page: Page,
  options: IntegrationsMockOptions = {},
  init?: (arg: any) => void,
  arg?: unknown,
) => {
  await page.clock.setFixedTime(NOW);
  await installTauriMock(page, "main");
  await installIntegrationsMock(page, options);
  if (init) await page.addInitScript(init, arg);
  await page.setViewportSize({ width: 1280, height: 1000 });
  await page.goto("/");
};

const nav = (page: Page) => page.getByRole("navigation");

const openIntegrations = async (page: Page) => {
  await nav(page)
    .getByRole("button", { name: "Integrationen", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Integrationen", level: 1 }),
  ).toBeVisible();
};

const openCatalog = async (page: Page) => {
  await page.getByTestId("integrations-add").click();
  await expect(page.getByTestId("catalog")).toBeVisible();
};

const card = (page: Page, id: string) =>
  page.locator(`[data-testid="integration-card"][data-integration-id="${id}"]`);

const openDetail = async (page: Page, id: string) => {
  await card(page, id).getByTestId("integration-open").click();
  await expect(page.getByTestId("integration-detail")).toBeVisible();
};

const grant = (page: Page, cap: string, caller: string, mode: string) =>
  page.getByTestId(`grant-${cap}-${caller}-${mode}`);

/** Wählt in einem Auswahlfeld (react-select) einen Eintrag. */
const pick = async (page: Page, label: string, option: string) => {
  await page.getByRole("combobox", { name: label, exact: true }).click();
  await page.getByRole("option", { name: option, exact: true }).click();
};

const FOLDER = "C:\\Ablage\\Berichte";

/** Legt ueber den Katalog einen Ordner an (Name, Pfad getippt). */
const createFolder = async (page: Page, name = "Ablage Berichte") => {
  await openCatalog(page);
  await page.getByTestId("catalog-setup-folder").click();
  await expect(page.getByTestId("folder-dialog")).toBeVisible();
  await page.getByTestId("folder-name").fill(name);
  await page.getByTestId("folder-path").fill(FOLDER);
  await page.getByTestId("folder-submit").click();
};

// ---------------------------------------------------------------------------
// Seitenleiste
// ---------------------------------------------------------------------------

test.describe("Seitenleiste", () => {
  test("Integrationen steht zwischen Modelle und Einstellungen", async ({
    page,
  }) => {
    await setup(page);
    const names = await page
      .locator(".workspace-nav__utilities button")
      .allInnerTexts();
    expect(names.map((n) => n.trim())).toEqual([
      "Modelle",
      "Integrationen",
      "Einstellungen",
    ]);
  });

  test("die Seite ist leer, wenn nichts eingerichtet ist, und sagt, wie es weitergeht", async ({
    page,
  }) => {
    await setup(page);
    await openIntegrations(page);
    await expect(page.getByTestId("integrations-empty")).toContainText(
      "Noch keine Integration eingerichtet",
    );
    await expect(page.getByTestId("integration-card")).toHaveCount(0);
  });
});

// ---------------------------------------------------------------------------
// Katalog
// ---------------------------------------------------------------------------

test.describe("Katalog", () => {
  test("mindestens sieben Arten, nicht verfügbare als „bald“ markiert", async ({
    page,
  }) => {
    await setup(page);
    await openIntegrations(page);
    await openCatalog(page);
    const items = page.getByTestId("catalog-item");
    expect(await items.count()).toBeGreaterThanOrEqual(7);
    const statuses = await items.evaluateAll((els) =>
      els.map((el) => (el as HTMLElement).dataset.status),
    );
    expect(statuses).toContain("available");
    expect(statuses).toContain("soon");
    // Jede „bald“-Art trägt die Marke und hat keinen Einrichten-Knopf.
    const soon = page.locator(
      '[data-testid="catalog-item"][data-status="soon"]',
    );
    for (let i = 0; i < (await soon.count()); i++) {
      await expect(soon.nth(i).getByTestId("catalog-soon")).toHaveText("bald");
      await expect(soon.nth(i).getByRole("button")).toHaveCount(0);
    }
    const names = await page.getByTestId("catalog-name").allInnerTexts();
    for (const n of [
      "Kalender (ICS-Adresse)",
      "Microsoft-365-Kalender",
      "Ordner",
      "Obsidian-Vault",
      "WAI-Wissensbasis",
      "MCP und Agenten",
      "YouTube",
    ]) {
      expect(names.join("|")).toContain(n);
    }
  });

  test("zurück zur Liste", async ({ page }) => {
    await setup(page);
    await openIntegrations(page);
    await openCatalog(page);
    await page.getByTestId("integrations-back").click();
    await expect(page.getByTestId("catalog")).toHaveCount(0);
    await expect(page.getByTestId("integrations-empty")).toBeVisible();
  });

  test("Kalender (ICS-Adresse) öffnet den Verbinden-Dialog", async ({
    page,
  }) => {
    await setup(page);
    await openIntegrations(page);
    await openCatalog(page);
    await page.getByTestId("catalog-setup-calendar_ics").click();
    await expect(page.getByTestId("calendar-url")).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// Ordner anlegen
// ---------------------------------------------------------------------------

test.describe("Ordner", () => {
  test("anlegen, sehen, Neuladen überstehen", async ({ page }) => {
    await setup(page);
    await openIntegrations(page);
    await createFolder(page);
    // Nach dem Anlegen: Liste mit der neuen Karte.
    await expect(page.getByTestId("folder-dialog")).toHaveCount(0);
    const created = page.getByTestId("integration-card");
    await expect(created).toHaveCount(1);
    await expect(created).toContainText("Ablage Berichte");
    await expect(created).toContainText(FOLDER);
    await expect(created.getByTestId("integration-direction")).toHaveText(
      "Lesen und schreiben",
    );
    const sent = (await calls(page, "integration_create"))[0].args;
    expect(sent).toMatchObject({
      kind: "folder",
      label: "Ablage Berichte",
      path: FOLDER,
      direction: "both",
    });

    await page.reload();
    await openIntegrations(page);
    await expect(page.getByTestId("integration-card")).toHaveCount(1);
    await expect(page.getByTestId("integration-card")).toContainText(
      "Ablage Berichte",
    );
  });

  test("Richtung beim Anlegen wählbar", async ({ page }) => {
    await setup(page);
    await openIntegrations(page);
    await openCatalog(page);
    await page.getByTestId("catalog-setup-folder").click();
    await page.getByTestId("folder-name").fill("Nur lesen");
    await page.getByTestId("folder-path").fill(FOLDER);
    await page.getByTestId("folder-direction-read").click();
    await page.getByTestId("folder-submit").click();
    await expect(page.getByTestId("integration-direction")).toHaveText(
      "Nur lesen",
    );
  });

  test("Ordner wählen übernimmt den Pfad des Dialogs", async ({ page }) => {
    await setup(page, { pickedPath: "D:\\Projekte\\Akten" });
    await openIntegrations(page);
    await openCatalog(page);
    await page.getByTestId("catalog-setup-folder").click();
    await page.getByTestId("folder-pick").click();
    await expect(page.getByTestId("folder-path")).toHaveValue(
      "D:\\Projekte\\Akten",
    );
  });

  test("ohne Pfad ist Anlegen gesperrt; ein falscher Pfad zeigt eine verständliche Meldung und legt nichts an", async ({
    page,
  }) => {
    await setup(page);
    await openIntegrations(page);
    await openCatalog(page);
    await page.getByTestId("catalog-setup-folder").click();
    await page.getByTestId("folder-name").fill("Ablage");
    await expect(page.getByTestId("folder-submit")).toBeDisabled();
    await page.getByTestId("folder-path").fill("Ablage\\unten");
    await page.getByTestId("folder-submit").click();
    await expect(page.getByTestId("folder-error")).toContainText(
      "vollständigen Pfad",
    );
    await page.getByTestId("folder-path").fill("C:\\gibt-es-nicht");
    await page.getByTestId("folder-submit").click();
    await expect(page.getByTestId("folder-error")).toContainText(
      "nicht gefunden",
    );
    await page.getByTestId("folder-cancel").click();
    await page.getByTestId("integrations-back").click();
    await expect(page.getByTestId("integration-card")).toHaveCount(0);
  });
});

// ---------------------------------------------------------------------------
// Detail: Richtung und Rechte-Matrix
// ---------------------------------------------------------------------------

test.describe("Detail und Rechte-Matrix", () => {
  test("Standardrechte stehen da: externe Agenten aus, Lesen erlaubt, Schreiben fragen", async ({
    page,
  }) => {
    await setup(page);
    await openIntegrations(page);
    await createFolder(page);
    await openDetail(page, "int-1");
    const pressed = (cap: string, caller: string, mode: string) =>
      grant(page, cap, caller, mode).getAttribute("aria-checked");
    expect(await pressed("files.read", "agent_external", "off")).toBe("true");
    expect(await pressed("files.read", "workflow", "allow")).toBe("true");
    expect(await pressed("files.write", "workflow", "ask")).toBe("true");
    expect(await pressed("files.write", "agent_local", "ask")).toBe("true");
    expect(await pressed("files.write", "agent_external", "off")).toBe("true");
  });

  test("Richtung und Fähigkeitsmodus ändern übersteht Neuladen", async ({
    page,
  }) => {
    await setup(page);
    await openIntegrations(page);
    await createFolder(page);
    await openDetail(page, "int-1");

    await grant(page, "files.write", "agent_external", "ask").click();
    await expect(
      grant(page, "files.write", "agent_external", "ask"),
    ).toHaveAttribute("aria-checked", "true");
    expect((await calls(page, "integration_set_grant"))[0].args).toEqual({
      id: "int-1",
      capability: "files.write",
      caller: "agent_external",
      mode: "ask",
    });
    await page.getByTestId("direction-read").click();
    await expect(page.getByTestId("direction-read")).toHaveAttribute(
      "aria-checked",
      "true",
    );
    expect(
      (await calls(page, "integration_update")).at(-1)!.args,
    ).toMatchObject({
      id: "int-1",
      direction: "read",
    });

    await page.reload();
    await openIntegrations(page);
    await expect(page.getByTestId("integration-card")).toContainText(
      "Nur lesen",
    );
    await openDetail(page, "int-1");
    await expect(page.getByTestId("direction-read")).toHaveAttribute(
      "aria-checked",
      "true",
    );
    await expect(
      grant(page, "files.write", "agent_external", "ask"),
    ).toHaveAttribute("aria-checked", "true");
  });

  test("Richtung „Nur lesen“ macht Schreiben wirkungslos und sagt es", async ({
    page,
  }) => {
    await setup(page, DEMO);
    await openIntegrations(page);
    await openDetail(page, "ordner-archiv");
    // Archiv: nur lesen. files.write: Hinweis, dass die Richtung es sperrt.
    await expect(page.getByTestId("capability-files.write")).toContainText(
      "Die Richtung der Integration erlaubt dies nicht",
    );
    await expect(page.getByTestId("capability-files.read")).not.toContainText(
      "Die Richtung der Integration erlaubt dies nicht",
    );
  });

  test("Aufnahme starten lässt sich nie auf „erlaubt“ stellen", async ({
    page,
  }) => {
    await setup(page, {
      integrations: [{ id: "agent-1", kind: "agent", label: "Claude Code" }],
    });
    await openIntegrations(page);
    await openDetail(page, "agent-1");
    const allow = grant(page, "recording.start", "agent_external", "allow");
    await expect(allow).toBeDisabled();
    await expect(page.getByTestId("capability-recording.start")).toContainText(
      "Einwilligungsdialog",
    );
    // Zugänge (Token je Programm) liefert die Agentenbrücke: bis dahin ein Platzhalter.
    await expect(page.getByTestId("agent-clients-placeholder")).toContainText(
      "Agentenbrücke",
    );
    // Die anderen dürfen.
    await expect(
      grant(page, "meeting.create", "agent_external", "allow"),
    ).toBeEnabled();
  });

  test("ausschalten und wieder einschalten", async ({ page }) => {
    await setup(page, DEMO);
    await openIntegrations(page);
    await openDetail(page, "ordner-berichte");
    await page.getByTestId("enabled-toggle").click();
    expect(
      (await calls(page, "integration_update")).at(-1)!.args,
    ).toMatchObject({
      id: "ordner-berichte",
      enabled: false,
    });
    await expect(page.getByTestId("integration-detail")).toContainText(
      "Die Integration ist ausgeschaltet",
    );
    await page.getByTestId("enabled-toggle").click();
    await expect(page.getByTestId("integration-detail")).not.toContainText(
      "Die Integration ist ausgeschaltet",
    );
  });

  test("Verbindung testen zeigt das Ergebnis in Worten", async ({ page }) => {
    await setup(page, { ...DEMO, testCode: "folder_path_not_found" });
    await openIntegrations(page);
    await openDetail(page, "ordner-berichte");
    await page.getByTestId("integration-test").click();
    await expect(page.getByTestId("integration-test-result")).toContainText(
      "Der Ordner wurde nicht gefunden",
    );
  });

  test("Rechte zurücksetzen löscht die gespeicherten Zeilen", async ({
    page,
  }) => {
    await setup(page, DEMO);
    await openIntegrations(page);
    await openDetail(page, "ordner-berichte");
    await expect(
      grant(page, "files.read", "agent_external", "allow"),
    ).toHaveAttribute("aria-checked", "true");
    await page.getByTestId("grants-reset").click();
    await expect(
      grant(page, "files.read", "agent_external", "off"),
    ).toHaveAttribute("aria-checked", "true");
    const sent = (await calls(page, "integration_set_grant")).map(
      (c) => c.args,
    );
    expect(sent.every((a) => a.mode === null)).toBe(true);
    expect(sent.length).toBe(2);
  });

  test("Entfernen fragt nach und ruft integration_delete", async ({ page }) => {
    await setup(page, DEMO);
    await openIntegrations(page);
    await openDetail(page, "ordner-archiv");
    await page.getByTestId("integration-remove").click();
    await expect(page.getByTestId("integration-remove-confirm")).toBeVisible();
    expect(await calls(page, "integration_delete")).toHaveLength(0);
    await page.getByTestId("integration-remove-yes").click();
    await expect(page.getByTestId("integration-detail")).toHaveCount(0);
    await expect(card(page, "ordner-archiv")).toHaveCount(0);
    expect((await calls(page, "integration_delete"))[0].args).toEqual({
      id: "ordner-archiv",
    });
  });
});

// ---------------------------------------------------------------------------
// Kalender als Karten
// ---------------------------------------------------------------------------

test.describe("Kalenderquellen", () => {
  const source = {
    id: "ics-1",
    kind: "ics",
    label: "Outlook Arbeit",
    account_hint: "outlook.office365.com",
    enabled: true,
    has_attendee_data: true,
    last_sync_at: NOW,
    last_ok_at: NOW,
    last_error: null,
    event_count: 23,
  };

  test("stehen als Karten auf der Seite, nicht mehr in den Einstellungen", async ({
    page,
  }) => {
    await setup(
      page,
      { integrations: [{ id: "ics-1", kind: "ics", label: "Outlook Arbeit" }] },
      () => {
        (window as any).__sources = [
          {
            id: "ics-1",
            kind: "ics",
            label: "Outlook Arbeit",
            account_hint: "outlook.office365.com",
            enabled: true,
            has_attendee_data: true,
            last_sync_at: 1_790_000_000_000,
            last_ok_at: 1_790_000_000_000,
            last_error: null,
            event_count: 23,
          },
        ];
      },
    );
    await openIntegrations(page);
    const cal = page.getByTestId("calendar-source");
    await expect(cal).toHaveCount(1);
    await expect(cal).toContainText("Outlook Arbeit");
    await expect(page.getByTestId("calendar-source-status")).toContainText(
      "23",
    );
    // Rechte & Richtung der Quelle: ein Klick bis zur Matrix.
    await cal.getByTestId("calendar-rights").click();
    await expect(page.getByTestId("integration-detail")).toContainText(
      "Outlook Arbeit",
    );
    await expect(
      grant(page, "calendar.read", "workflow", "allow"),
    ).toHaveAttribute("aria-checked", "true");
    // Die Einstellungen zeigen nur noch den Verweis.
    await nav(page)
      .getByRole("button", { name: "Einstellungen", exact: true })
      .click();
    await expect(page.getByTestId("calendar-source")).toHaveCount(0);
    await expect(page.getByTestId("calendar-settings-link")).toBeVisible();
    // Die Erinnerung bleibt dort.
    await expect(page.getByTestId("calendar-reminder-settings")).toBeVisible();
    void source;
  });

  test("der Verweis in den Einstellungen führt auf die Seite", async ({
    page,
  }) => {
    await setup(page);
    await nav(page)
      .getByRole("button", { name: "Einstellungen", exact: true })
      .click();
    await page.getByTestId("calendar-settings-link").click();
    await expect(
      page.getByRole("heading", { name: "Integrationen", level: 1 }),
    ).toBeVisible();
    await expect(page.getByTestId("calendar-settings")).toBeVisible();
    await expect(page.getByTestId("calendar-empty")).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// MCP-Schalter
// ---------------------------------------------------------------------------

test.describe("MCP", () => {
  const mcpSwitch = (page: Page) =>
    page
      .getByText("Lokaler MCP-Server (nur lesend)", { exact: true })
      .locator("xpath=ancestor::div[.//input[@type='checkbox']][1]")
      .locator("input[type=checkbox]");

  test("unter Einstellungen > Besprechungen steht nur ein Verweis, kein Schalter", async ({
    page,
  }) => {
    await setup(page);
    await nav(page)
      .getByRole("button", { name: "Einstellungen", exact: true })
      .click();
    await expect(page.getByTestId("mcp-settings-link")).toBeVisible();
    await expect(
      page.getByText("Lokaler MCP-Server (nur lesend)", { exact: true }),
    ).toHaveCount(0);
    await expect(page.locator("input[type=checkbox]")).not.toHaveCount(0); // andere Schalter bleiben
  });

  test("der Schalter auf der Seite wirkt wie vorher: meeting_mcp_enabled", async ({
    page,
  }) => {
    await setup(page);
    await nav(page)
      .getByRole("button", { name: "Einstellungen", exact: true })
      .click();
    await page.getByTestId("mcp-settings-link").click();
    const toggle = mcpSwitch(page);
    await expect(toggle).not.toBeChecked();
    expect(
      await calls(page, "change_meeting_mcp_enabled_setting"),
    ).toHaveLength(0);
    await toggle.evaluate((el) => (el as HTMLInputElement).click());
    await expect
      .poll(
        async () =>
          (await calls(page, "change_meeting_mcp_enabled_setting")).length,
      )
      .toBe(1);
    expect(
      (await calls(page, "change_meeting_mcp_enabled_setting"))[0].args,
    ).toEqual({
      enabled: true,
    });
    await expect(page.getByTestId("mcp-snippet")).toContainText(
      "claude mcp add local-voice",
    );
    expect(
      await page.evaluate(() => (window as any).__settings.meeting_mcp_enabled),
    ).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// Freigaben
// ---------------------------------------------------------------------------

test.describe("Freigabedialog", () => {
  test("ein Hinweis nennt die Zahl, der Dialog zeigt jede Anfrage vollständig", async ({
    page,
  }) => {
    await setup(page, DEMO);
    await openIntegrations(page);
    const banner = page.getByTestId("approvals-banner");
    await expect(banner).toContainText("2 Freigaben warten");
    await page.getByTestId("approvals-open").click();
    const dialog = page.getByTestId("approval-dialog");
    await expect(dialog).toBeVisible();
    const items = dialog.getByTestId("approval-item");
    await expect(items).toHaveCount(2);
    await expect(items.first()).toContainText("Externer Agent");
    await expect(items.first()).toContainText("Ablage Berichte");
    await expect(items.first()).toContainText("Dateien schreiben");
    await expect(items.first().getByTestId("approval-preview")).toContainText(
      "Wochenbericht-KW40.md",
    );
    await expect(dialog).toContainText("gilt nur für diese eine Aktion");
  });

  test("Erlauben und Ablehnen gehen ans Backend; der Hinweis zählt mit und verschwindet", async ({
    page,
  }) => {
    await setup(page, DEMO);
    await openIntegrations(page);
    await page.getByTestId("approvals-open").click();
    const items = page.getByTestId("approval-item");
    await items.first().getByTestId("approval-allow").click();
    await expect(items).toHaveCount(1);
    expect((await calls(page, "approval_decide"))[0].args).toEqual({
      id: "ap-1",
      approve: true,
    });
    await items.first().getByTestId("approval-deny").click();
    expect((await calls(page, "approval_decide"))[1].args).toEqual({
      id: "ap-2",
      approve: false,
    });
    // Nichts mehr offen: Dialog zu, Hinweis weg.
    await expect(page.getByTestId("approval-dialog")).toHaveCount(0);
    await expect(page.getByTestId("approvals-banner")).toHaveCount(0);
  });

  test("ohne offene Freigaben gibt es keinen Hinweis", async ({ page }) => {
    await setup(page, { integrations: DEMO.integrations });
    await openIntegrations(page);
    await expect(page.getByTestId("approvals-banner")).toHaveCount(0);
  });

  test("eine inzwischen entschiedene Freigabe meldet es in Worten", async ({
    page,
  }) => {
    await setup(page, DEMO);
    await openIntegrations(page);
    await page.getByTestId("approvals-open").click();
    // Zwischen Anzeige und Klick entscheidet anderswo jemand.
    await page.evaluate(() => {
      (window as any).__reg.approvals[0].state = "approved";
    });
    await page
      .getByTestId("approval-item")
      .first()
      .getByTestId("approval-allow")
      .click();
    await expect(page.getByTestId("approval-error")).toContainText(
      "bereits entschieden",
    );
  });
});

// ---------------------------------------------------------------------------
// Protokoll
// ---------------------------------------------------------------------------

test.describe("Protokoll", () => {
  const openAudit = async (page: Page) => {
    await openIntegrations(page);
    await page.getByRole("tab", { name: "Protokoll" }).click();
    await expect(page.getByTestId("audit")).toBeVisible();
  };

  const byOutcome = (page: Page, outcome: string) =>
    page.getByTestId("audit-row").filter({
      has: page
        .getByTestId("audit-outcome")
        .filter({ hasText: new RegExp(`^${outcome}$`) }),
    });

  test("zeigt Aufrufer, Integration, Fähigkeit, Ziel und Ergebnis in Worten", async ({
    page,
  }) => {
    await setup(page, DEMO);
    await openAudit(page);
    const rows = page.getByTestId("audit-row");
    await expect(rows).toHaveCount(5);
    const first = rows.first();
    await expect(first).toContainText("Externer Agent");
    await expect(first).toContainText("Ablage Berichte");
    await expect(first).toContainText("Dateien schreiben");
    await expect(first).toContainText("Wochenbericht-KW40.md");
    await expect(first.getByTestId("audit-outcome")).toHaveText("wartet");
    // Verweigerung mit Grund und Zähler.
    const denied = byOutcome(page, "verweigert");
    await expect(denied).toHaveCount(1);
    await expect(denied).toContainText("Das Recht steht auf „aus“");
    await expect(denied).toContainText("3×");
    // Fehler mit Text.
    await expect(byOutcome(page, "Fehler")).toContainText("Zugriff verweigert");
    // Eine Nutzeraktion.
    const mine = rows.filter({
      has: page.getByTestId("audit-caller").filter({ hasText: /^Du$/ }),
    });
    await expect(mine).toHaveCount(1);
    await expect(mine).toContainText("angelegt");
  });

  test("Filter nach Ergebnis und Integration gehen ans Backend", async ({
    page,
  }) => {
    await setup(page, DEMO);
    await openAudit(page);
    await pick(page, "Ergebnis", "verweigert");
    await expect(page.getByTestId("audit-row")).toHaveCount(1);
    expect(
      (await calls(page, "integrations_audit_list")).at(-1)!.args,
    ).toMatchObject({
      outcome: "denied",
    });
    await pick(page, "Ergebnis", "Fehler");
    await expect(page.getByTestId("audit-row")).toHaveCount(1);
    await expect(page.getByTestId("audit-row")).toContainText(
      "Zugriff verweigert",
    );
    await pick(page, "Integration", "Ablage Berichte");
    await expect(page.getByTestId("audit-row")).toHaveCount(1);
    expect(
      (await calls(page, "integrations_audit_list")).at(-1)!.args,
    ).toMatchObject({
      outcome: "error",
      integrationId: "ordner-berichte",
    });
    await pick(page, "Wer", "Du");
    await expect(page.getByTestId("audit-empty")).toBeVisible();
  });

  test("leeres Protokoll sagt es", async ({ page }) => {
    await setup(page);
    await openAudit(page);
    await expect(page.getByTestId("audit-empty")).toBeVisible();
  });

  test("aus dem Detail führt ein Link auf das gefilterte Protokoll", async ({
    page,
  }) => {
    await setup(page, DEMO);
    await openIntegrations(page);
    await openDetail(page, "ordner-archiv");
    await page.getByTestId("integration-audit-link").click();
    await expect(page.getByTestId("audit")).toBeVisible();
    await expect(page.getByTestId("audit-filter-integration")).toContainText(
      "Archiv (nur lesen)",
    );
    await expect(page.getByTestId("audit-row")).toHaveCount(1);
  });
});

// ---------------------------------------------------------------------------
// Darstellung
// ---------------------------------------------------------------------------

test.describe("Darstellung", () => {
  for (const width of [1280, 900, 390]) {
    test(`kein waagerechtes Scrollen bei ${width} px (Liste, Katalog, Detail)`, async ({
      page,
    }) => {
      await setup(page, DEMO);
      await page.setViewportSize({ width, height: 900 });
      const overflow = () =>
        page.evaluate(
          () =>
            document.documentElement.scrollWidth -
            document.documentElement.clientWidth,
        );
      // Schmal liegt „Integrationen“ hinter „Mehr“.
      if (width < 768) {
        await nav(page).getByRole("button", { name: "Mehr" }).click();
      }
      await openIntegrations(page);
      expect(await overflow()).toBeLessThanOrEqual(0);
      await openCatalog(page);
      expect(await overflow()).toBeLessThanOrEqual(0);
      await page.getByTestId("integrations-back").click();
      await openDetail(page, "ordner-berichte");
      expect(await overflow()).toBeLessThanOrEqual(0);
    });
  }
});

// ---------------------------------------------------------------------------
// Zugaenglichkeit und Kontrast (theme.css-Tokens), hell und dunkel
// ---------------------------------------------------------------------------

for (const theme of ["light", "dark"] as const) {
  test(`axe (WCAG 2 A/AA) ohne serious/critical auf jeder Ansicht - ${theme}`, async ({
    page,
  }) => {
    await setup(
      page,
      DEMO,
      (value: string) => {
        (window as any).__settings.theme = value;
        (window as any).__sources = [
          {
            id: "ics-1",
            kind: "ics",
            label: "Outlook Arbeit",
            account_hint: "outlook.office365.com",
            enabled: true,
            has_attendee_data: true,
            last_sync_at: 1_790_000_000_000,
            last_ok_at: 1_790_000_000_000,
            last_error: null,
            event_count: 23,
          },
        ];
      },
      theme,
    );
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const findings = async (where: string) => {
      await page.addStyleTag({
        content:
          "*,*::before,*::after{transition:none!important;animation:none!important}",
      });
      await page.waitForTimeout(250);
      // Ausnahme: das Kontrollkästchen des gemeinsamen ToggleSwitch hat app-weit
      // keinen eigenen Namen (die Beschriftung steht daneben, nicht daran) - ein
      // vorbestehender Befund der Komponente, nicht dieser Seite.
      const result = await new AxeBuilder({ page })
        .withTags(["wcag2a", "wcag2aa"])
        .exclude("input.peer")
        .analyze();
      const bad = result.violations
        .filter((v) => v.impact === "critical" || v.impact === "serious")
        .map(
          (v) =>
            `${where}: ${v.id} ${v.nodes.map((n) => n.target.join(" ")).join(" | ")}`,
        );
      expect(bad).toEqual([]);
    };
    await openIntegrations(page);
    await expect(page.getByTestId("approvals-banner")).toBeVisible();
    await findings("Liste");
    await page.getByTestId("approvals-open").click();
    await expect(page.getByTestId("approval-dialog")).toBeVisible();
    await findings("Freigabe");
    await page.keyboard.press("Escape");
    await openCatalog(page);
    await findings("Katalog");
    await page.getByTestId("integrations-back").click();
    await openDetail(page, "ordner-berichte");
    await findings("Detail");
    await page.getByTestId("integrations-back").click();
    await page.getByRole("tab", { name: "Protokoll" }).click();
    await expect(page.getByTestId("audit")).toBeVisible();
    await findings("Protokoll");
  });
}
