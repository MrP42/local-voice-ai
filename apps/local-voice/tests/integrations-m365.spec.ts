import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { calls, installTauriMock } from "./calendarMock";
import { installIntegrationsMock, NOW } from "./integrationsMock";
import { GUID, installM365Mock, type M365MockOptions } from "./m365Mock";

// A5 (Goal Integrationen, #66, AK8 m365): das Microsoft-365-Konto in der Seite
// „Integrationen“: anlegen, Fähigkeiten (Scopes nur für eingeschaltete), Anmelden
// (Attrappe, kein Browser), Status und die Prüfungen für den Eigentümer. Gegen die
// Tauri-Attrappe; Scopes, Token und Server prüfen die Rust-Tests
// (`cargo test --lib integrations::m365`).

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

const setup = async (page: Page, m365: M365MockOptions = {}) => {
  await page.clock.setFixedTime(NOW);
  await installTauriMock(page, "main");
  await installIntegrationsMock(page, {});
  await installM365Mock(page, m365);
  await page.setViewportSize({ width: 1280, height: 1100 });
  await page.goto("/");
};

const openIntegrations = async (page: Page) => {
  await page
    .getByRole("navigation")
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

const panel = (page: Page) => page.getByTestId("m365-panel");
const state = (page: Page) => page.getByTestId("m365-state");
const scopes = (page: Page) =>
  page.getByTestId("m365-scopes").locator("li").allInnerTexts();

const ACCOUNT = {
  id: "m365-konto",
  label: "Mein Konto",
  caps: ["mail.send", "files.write"],
};

/** Öffnet das Detail eines vorhandenen Kontos. */
const openAccount = async (page: Page, id = ACCOUNT.id) => {
  await openIntegrations(page);
  await page
    .locator(`[data-testid="integration-card"][data-integration-id="${id}"]`)
    .getByTestId("integration-open")
    .click();
  await expect(panel(page)).toBeVisible();
};

/** Legt über den Katalog ein Konto an. */
const create = async (
  page: Page,
  opts: { name?: string; client?: string; caps?: string[] } = {},
) => {
  await openIntegrations(page);
  await openCatalog(page);
  await page.getByTestId("catalog-setup-m365").click();
  await expect(page.getByTestId("m365-dialog")).toBeVisible();
  if (opts.name) await page.getByTestId("m365-name").fill(opts.name);
  await page.getByTestId("m365-client-id").fill(opts.client ?? GUID);
  if (opts.caps) {
    for (const cap of ["mail.send", "files.write", "calendar.write"]) {
      const box = page.getByTestId(`m365-cap-${cap}`);
      if ((await box.isChecked()) !== opts.caps.includes(cap))
        await box.setChecked(opts.caps.includes(cap));
    }
  }
  await page.getByTestId("m365-submit").click();
};

// ---------------------------------------------------------------------------
// Katalog und Anlegen
// ---------------------------------------------------------------------------

test.describe("Anlegen", () => {
  test("Microsoft 365 steht im Katalog als einrichtbar, mit Knopf", async ({
    page,
  }) => {
    await setup(page);
    await openIntegrations(page);
    await openCatalog(page);
    const item = page.locator(
      '[data-testid="catalog-item"][data-catalog-id="m365"]',
    );
    await expect(item).toHaveAttribute("data-status", "available");
    await expect(item.getByTestId("catalog-setup-m365")).toHaveText(
      "Konto einrichten",
    );
    await expect(item.getByTestId("catalog-soon")).toHaveCount(0);
  });

  test("Konto anlegen: Fähigkeiten und Client-ID gehen ans Backend, das Detail führt zur Anmeldung", async ({
    page,
  }) => {
    await setup(page);
    await create(page, { name: "Büro-Konto" });
    // Nach dem Anlegen: das Detail des neuen Kontos, noch nicht angemeldet.
    await expect(page.getByTestId("integration-detail")).toBeVisible();
    await expect(state(page)).toHaveAttribute("data-state", "needs_sign_in");
    await expect(state(page)).toContainText("Anmeldung nötig");
    await expect(page.getByTestId("m365-sign-in")).toHaveText(
      "Mit Microsoft anmelden",
    );
    const sent = (await calls(page, "m365_create"))[0].args;
    expect(sent).toMatchObject({
      label: "Büro-Konto",
      clientId: GUID,
      capabilities: ["mail.send", "files.write"],
      filesMode: "full",
      filesFolder: "Local Voice AI",
    });
    // Es gibt keinen Zugangsschlüssel, solange man nicht angemeldet ist.
    await expect(page.getByText("fehlt, bitte neu einrichten")).toBeVisible();
    await page.getByTestId("integrations-back").click();
    await expect(page.getByTestId("integration-card")).toContainText(
      "Büro-Konto",
    );
  });

  test("ohne Client-ID bleibt das Konto nicht eingerichtet und bietet keine Anmeldung an", async ({
    page,
  }) => {
    await setup(page);
    await create(page, { client: "" });
    await expect(state(page)).toHaveAttribute("data-state", "not_configured");
    await expect(page.getByTestId("m365-sign-in")).toHaveCount(0);
    // Die Client-ID lässt sich im Detail nachtragen; dann ist die Anmeldung möglich.
    await page.getByTestId("m365-client-id").fill(GUID);
    await page.getByTestId("m365-save").click();
    await expect(state(page)).toHaveAttribute("data-state", "needs_sign_in");
    await expect(page.getByTestId("m365-sign-in")).toBeVisible();
  });

  test("eine ungültige Client-ID wird mit Klartext abgewiesen und legt nichts an", async ({
    page,
  }) => {
    await setup(page);
    await create(page, { client: "keine-guid" });
    await expect(page.getByTestId("m365-create-error")).toContainText(
      "keine gültige Client-ID",
    );
    await expect(page.getByTestId("m365-dialog")).toBeVisible();
    await page.getByTestId("m365-cancel-create").click();
    await page.getByTestId("integrations-back").click();
    await expect(page.getByTestId("integration-card")).toHaveCount(0);
  });

  test("OneDrive-Auswahl erscheint nur mit der Fähigkeit „Dateien schreiben“", async ({
    page,
  }) => {
    await setup(page);
    await openIntegrations(page);
    await openCatalog(page);
    await page.getByTestId("catalog-setup-m365").click();
    await expect(page.getByTestId("m365-files-full")).toBeChecked();
    await page.getByTestId("m365-cap-files.write").setChecked(false);
    await expect(page.getByTestId("m365-files-full")).toHaveCount(0);
    await page.getByTestId("m365-cap-files.write").setChecked(true);
    await page.getByTestId("m365-files-app_folder").check();
    await page.getByTestId("m365-client-id").fill(GUID);
    await page.getByTestId("m365-submit").click();
    expect((await calls(page, "m365_create"))[0].args).toMatchObject({
      filesMode: "app_folder",
    });
    expect(await scopes(page)).toContain("Files.ReadWrite.AppFolder");
    expect(await scopes(page)).not.toContain("Files.ReadWrite");
  });
});

// ---------------------------------------------------------------------------
// Fähigkeiten und Berechtigungen
// ---------------------------------------------------------------------------

test.describe("Fähigkeiten", () => {
  test("angefragt werden nur die Berechtigungen der eingeschalteten Fähigkeiten", async ({
    page,
  }) => {
    await setup(page, { accounts: [ACCOUNT] });
    await openAccount(page);
    expect(await scopes(page)).toEqual([
      "offline_access",
      "User.Read",
      "Mail.Send",
      "Files.ReadWrite",
    ]);
    await page.getByTestId("m365-cap-files.write").setChecked(false);
    await expect
      .poll(async () => scopes(page))
      .toEqual(["offline_access", "User.Read", "Mail.Send"]);
    await page.getByTestId("m365-cap-mail.send").setChecked(false);
    await expect(state(page)).toHaveAttribute("data-state", "no_capabilities");
    expect(await scopes(page)).toEqual(["offline_access", "User.Read"]);
    expect(
      (await calls(page, "m365_update_settings")).map(
        (c) => c.args.capabilities,
      ),
    ).toEqual([["mail.send"], []]);
  });

  test("eine zusätzliche Fähigkeit verlangt Zustimmung; erneutes Anmelden löst es", async ({
    page,
  }) => {
    await setup(page, {
      accounts: [
        {
          ...ACCOUNT,
          signed_in: true,
          granted: [
            "offline_access",
            "User.Read",
            "Mail.Send",
            "Files.ReadWrite",
          ],
        },
      ],
    });
    await openAccount(page);
    await expect(state(page)).toHaveAttribute("data-state", "ready");
    await expect(page.getByTestId("m365-account")).toContainText(
      "ich@example.com",
    );

    await page.getByTestId("m365-cap-calendar.write").setChecked(true);
    await expect(state(page)).toHaveAttribute("data-state", "needs_consent");
    await expect(page.getByTestId("m365-missing")).toContainText(
      "Calendars.ReadWrite",
    );
    await expect(
      page.locator('[data-scope="Calendars.ReadWrite"]'),
    ).toBeVisible();
    await expect(page.getByTestId("m365-sign-in")).toHaveText(
      "Zustimmung erweitern",
    );
    // Prüfungen gibt es erst, wenn alles zugestimmt ist.
    await expect(page.getByTestId("m365-checks")).toHaveCount(0);

    await page.getByTestId("m365-sign-in").click();
    await expect(state(page)).toHaveAttribute("data-state", "ready");
    await expect(page.getByTestId("m365-missing")).toHaveCount(0);
    await expect(page.getByTestId("m365-checks")).toBeVisible();
  });

  test("ausschalten verlangt keine neue Zustimmung", async ({ page }) => {
    await setup(page, {
      accounts: [
        {
          ...ACCOUNT,
          signed_in: true,
          granted: [
            "offline_access",
            "User.Read",
            "Mail.Send",
            "Files.ReadWrite",
          ],
        },
      ],
    });
    await openAccount(page);
    await page.getByTestId("m365-cap-files.write").setChecked(false);
    await expect(state(page)).toHaveAttribute("data-state", "ready");
  });

  test("die Rechte-Matrix zeigt nicht eingeschaltete Fähigkeiten als aus und sagt warum", async ({
    page,
  }) => {
    await setup(page, { accounts: [ACCOUNT] });
    await openAccount(page);
    const cal = page.getByTestId("capability-calendar.write");
    await expect(cal).toContainText("nicht eingeschaltet");
    await expect(cal.getByTestId("grant-effective").first()).toContainText(
      "Wirkt jetzt: Aus",
    );
    // Eingeschaltete Fähigkeiten tragen den Hinweis nicht.
    await expect(
      page
        .getByTestId("capability-mail.send")
        .getByTestId("capability-blocked"),
    ).toHaveCount(0);
    // Schreibendes fragt standardmäßig, externe Agenten sind aus.
    await expect(
      page.getByTestId("grant-mail.send-workflow-ask"),
    ).toHaveAttribute("aria-checked", "true");
    await expect(
      page.getByTestId("grant-mail.send-agent_external-off"),
    ).toHaveAttribute("aria-checked", "true");
    // Einschalten hebt den Hinweis auf.
    await page.getByTestId("m365-cap-calendar.write").setChecked(true);
    await expect(
      page
        .getByTestId("capability-calendar.write")
        .getByTestId("capability-blocked"),
    ).toHaveCount(0);
  });
});

// ---------------------------------------------------------------------------
// Anmelden und Status
// ---------------------------------------------------------------------------

test.describe("Anmelden und Status", () => {
  test("Anmelden wartet auf den Browser; Abbrechen beendet es sauber", async ({
    page,
  }) => {
    await setup(page, { accounts: [ACCOUNT], signIn: "pending" });
    await openAccount(page);
    await page.getByTestId("m365-sign-in").click();
    await expect(page.getByTestId("m365-waiting")).toContainText(
      "Warte auf die Anmeldung im Browser",
    );
    await expect(page.getByTestId("m365-sign-in")).toHaveCount(0);
    await page.getByTestId("m365-cancel-sign-in").click();
    await expect(page.getByTestId("m365-waiting")).toHaveCount(0);
    await expect(page.getByTestId("m365-message")).toContainText(
      "Abgebrochen.",
    );
    await expect(state(page)).toHaveAttribute("data-state", "needs_sign_in");
    expect((await calls(page, "m365_cancel_sign_in")).length).toBe(1);
  });

  test("Anmelden gelingt: bereit, Konto sichtbar, Prüfungen erscheinen", async ({
    page,
  }) => {
    await setup(page, { accounts: [ACCOUNT], signIn: "pending" });
    await openAccount(page);
    await page.getByTestId("m365-sign-in").click();
    await expect(page.getByTestId("m365-waiting")).toBeVisible();
    await page.evaluate(() => (window as any).__m365Finish());
    await expect(state(page)).toHaveAttribute("data-state", "ready");
    await expect(state(page)).toContainText("Angemeldet und bereit");
    await expect(page.getByTestId("m365-account")).toContainText(
      "ich@example.com",
    );
    await expect(page.getByTestId("m365-message")).toContainText("Angemeldet.");
    await expect(page.getByTestId("m365-checks")).toBeVisible();
    await expect(page.getByTestId("m365-test-mail")).toBeVisible();
    await expect(page.getByTestId("m365-test-file")).toBeVisible();
    // Der Zugangsschlüssel ist jetzt hinterlegt (Zustand, nie der Inhalt).
    await expect(page.getByText("hinterlegt")).toBeVisible();
    // Nach dem Neuladen steht das Konto weiter angemeldet da.
    await page.reload();
    await openAccount(page);
    await expect(state(page)).toHaveAttribute("data-state", "ready");
  });

  test("scheitert die Anmeldung, steht der Grund da und der Zustand bleibt", async ({
    page,
  }) => {
    await setup(page, {
      accounts: [ACCOUNT],
      signIn: "m365_sign_in_timeout",
    });
    await openAccount(page);
    await page.getByTestId("m365-sign-in").click();
    await expect(page.getByTestId("m365-message")).toContainText(
      "nicht innerhalb von 5 Minuten",
    );
    await expect(state(page)).toHaveAttribute("data-state", "needs_sign_in");
    // Der letzte Fehler steht auch im Zustand der Verbindung.
    await expect(page.getByTestId("integration-status")).toContainText(
      "Anmeldung fehlgeschlagen",
    );
  });

  test("eine abgelaufene Anmeldung zeigt beim Test „neu anmelden“", async ({
    page,
  }) => {
    await setup(page, {
      accounts: [
        {
          ...ACCOUNT,
          signed_in: true,
          granted: [
            "offline_access",
            "User.Read",
            "Mail.Send",
            "Files.ReadWrite",
          ],
        },
      ],
      testCode: "m365_needs_sign_in",
    });
    await openAccount(page);
    await page.getByTestId("m365-test").click();
    await expect(page.getByTestId("m365-message")).toContainText(
      "abgelaufen oder wurde widerrufen",
    );
    await expect(state(page)).toHaveAttribute("data-state", "needs_sign_in");
    await expect(page.getByTestId("m365-sign-in")).toHaveText(
      "Mit Microsoft anmelden",
    );
    await expect(page.getByTestId("m365-checks")).toHaveCount(0);
  });

  test("Verbindung testen, Testmail und Testdatei melden den Erfolg", async ({
    page,
  }) => {
    await setup(page, {
      accounts: [
        {
          ...ACCOUNT,
          signed_in: true,
          granted: [
            "offline_access",
            "User.Read",
            "Mail.Send",
            "Files.ReadWrite",
          ],
        },
      ],
    });
    await openAccount(page);
    await page.getByTestId("m365-test").click();
    await expect(page.getByTestId("m365-message")).toContainText(
      "Verbunden als ich@example.com",
    );
    await page.getByTestId("m365-test-mail").click();
    await expect(page.getByTestId("m365-message")).toContainText(
      "Testmail gesendet",
    );
    await page.getByTestId("m365-test-file").click();
    await expect(page.getByTestId("m365-message")).toContainText(
      "local-voice-test-20261001-101500.txt",
    );
    await expect(page.getByTestId("m365-message")).toContainText(
      "Local Voice AI",
    );
    expect((await calls(page, "m365_send_test_mail")).length).toBe(1);
    expect((await calls(page, "m365_upload_test_file")).length).toBe(1);
  });

  test("ein Fehler der Aktion wird mit Klartext gemeldet (OneDrive voll)", async ({
    page,
  }) => {
    await setup(page, {
      accounts: [
        {
          ...ACCOUNT,
          signed_in: true,
          granted: [
            "offline_access",
            "User.Read",
            "Mail.Send",
            "Files.ReadWrite",
          ],
        },
      ],
      actionCode: "m365_storage_full",
    });
    await openAccount(page);
    await page.getByTestId("m365-test-file").click();
    await expect(page.getByTestId("m365-message")).toContainText(
      "OneDrive-Speicher ist voll",
    );
  });

  test("Abmelden löscht das Token und zeigt wieder „Anmeldung nötig“", async ({
    page,
  }) => {
    await setup(page, {
      accounts: [
        {
          ...ACCOUNT,
          signed_in: true,
          granted: [
            "offline_access",
            "User.Read",
            "Mail.Send",
            "Files.ReadWrite",
          ],
        },
      ],
    });
    await openAccount(page);
    await page.getByTestId("m365-sign-out").click();
    await expect(state(page)).toHaveAttribute("data-state", "needs_sign_in");
    await expect(page.getByTestId("m365-message")).toContainText(
      "Token ist gelöscht",
    );
    await expect(page.getByTestId("m365-sign-out")).toHaveCount(0);
  });

  test("Entfernen des Kontos funktioniert wie bei jeder Integration", async ({
    page,
  }) => {
    await setup(page, { accounts: [ACCOUNT] });
    await openAccount(page);
    await page.getByTestId("integration-remove").click();
    await page.getByTestId("integration-remove-yes").click();
    await expect(page.getByTestId("integration-card")).toHaveCount(0);
  });
});

// ---------------------------------------------------------------------------
// Barrierefreiheit
// ---------------------------------------------------------------------------

test.describe("Barrierefreiheit", () => {
  test("Dialog und Konto-Bereich ohne axe-Verstöße (Kontrast ausgenommen, B4)", async ({
    page,
  }) => {
    await setup(page, {
      accounts: [
        {
          ...ACCOUNT,
          signed_in: true,
          granted: [
            "offline_access",
            "User.Read",
            "Mail.Send",
            "Files.ReadWrite",
          ],
        },
      ],
    });
    await openIntegrations(page);
    await openCatalog(page);
    await page.getByTestId("catalog-setup-m365").click();
    await expect(page.getByTestId("m365-dialog")).toBeVisible();
    const dialog = await new AxeBuilder({ page })
      .include('[data-testid="m365-dialog"]')
      .disableRules(["color-contrast"])
      .analyze();
    expect(dialog.violations).toEqual([]);
    await page.getByTestId("m365-cancel-create").click();
    await page.getByTestId("integrations-back").click();
    await openAccount(page);
    const result = await new AxeBuilder({ page })
      .include('[data-testid="m365-panel"]')
      .disableRules(["color-contrast"])
      .analyze();
    expect(result.violations).toEqual([]);
  });
});
