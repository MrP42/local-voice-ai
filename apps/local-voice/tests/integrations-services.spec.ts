import { test, expect, type Page } from "@playwright/test";
import { calls, installTauriMock } from "./calendarMock";
import { installIntegrationsMock, NOW } from "./integrationsMock";

// Welle 1 (Spec 2026-10-06-verbindungen-aktionen): Dienste im Katalog, Dialog je Dienst,
// Schlüssel/Webhook als Geheimnis, Test ohne Testnachricht, Rechte je Dienst.

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

const setup = async (page: Page) => {
  await page.clock.setFixedTime(NOW);
  await installTauriMock(page, "main");
  await installIntegrationsMock(page, {});
  await page.setViewportSize({ width: 1280, height: 1000 });
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Integrationen", exact: true })
    .click();
  await page.getByTestId("integrations-add").click();
  await expect(page.getByTestId("catalog-services")).toBeVisible();
};

const connect = async (page: Page, id: string) => {
  await page.getByTestId(`catalog-setup-service-${id}`).click();
  await expect(page.getByTestId("target-dialog")).toHaveAttribute(
    "data-kind",
    "service",
  );
};

const TOKEN = "2/1209/GEHEIM-asana-token";
const SLACK = "https://hooks.slack.com/services/T0/B0/GEHEIM";

test("der Katalog zeigt 17 Dienste in sechs Gruppen", async ({ page }) => {
  await setup(page);
  const services = page.getByTestId("catalog-service");
  await expect(services).toHaveCount(17);
  const ids = await services.evaluateAll((els) =>
    els.map((el) => (el as HTMLElement).dataset.service),
  );
  expect(ids).toEqual([
    "slack",
    "teams",
    "discord",
    "asana",
    "clickup",
    "jira",
    "trello",
    "todoist",
    "monday",
    "linear",
    "github",
    "notion",
    "confluence",
    "hubspot",
    "pipedrive",
    "airtable",
    "icloud",
  ]);
  for (const group of [
    "Kanäle",
    "Aufgaben und Tickets",
    "Seiten und Wiki",
    "CRM",
    "Tabellen",
    "Kalender",
  ]) {
    await expect(page.getByTestId("catalog-services")).toContainText(group);
  }
});

test("Asana: Felder aus dem Backend, Schlüssel verdeckt, Anlegen erst wenn alles da ist", async ({
  page,
}) => {
  await setup(page);
  await connect(page, "asana");
  const dialog = page.getByTestId("target-dialog");
  await expect(page.getByRole("dialog")).toContainText("Asana verbinden");
  await expect(dialog).toContainText("Projekt-ID");
  await expect(page.getByTestId("target-secret")).toHaveAttribute(
    "type",
    "password",
  );
  await expect(page.getByTestId("target-svc-help")).toContainText("Asana");
  await page.getByTestId("target-name").fill("Kundenprojekt");
  await expect(page.getByTestId("target-submit")).toBeDisabled();
  await page.getByTestId("target-svc-project_id").fill("1209");
  await expect(page.getByTestId("target-submit")).toBeDisabled();
  await page.getByTestId("target-secret").fill(TOKEN);
  await page.getByTestId("target-submit").click();

  const detail = page.getByTestId("integration-detail");
  await expect(detail).toBeVisible();
  await expect(detail).toContainText("Asana");
  await expect(detail).toContainText("Aufgaben anlegen");
  await expect(page.getByTestId("direction-write")).toBeDisabled();
  await expect(page.locator("body")).not.toContainText("GEHEIM");

  const [create] = await calls(page, "integration_create_with_settings");
  expect(create.args.kind).toBe("service");
  expect(create.args.settings.service).toBe("asana");
  expect(create.args.settings.fields).toEqual({ project_id: "1209" });
  expect(create.args.settings.secret).toBe(TOKEN);

  // Testen: ein lesender Aufruf, das Ergebnis grün mit dem Namen des Ziels.
  await page.getByTestId("integration-test").click();
  await expect(page.getByTestId("integration-test-result")).toContainText(
    "Verbindung in Ordnung",
  );
  await expect(page.getByTestId("integration-test-detail")).toContainText(
    "Kundenprojekt",
  );
});

test("Slack: nur Adressen von Slack, danach steht nur der Server im Detail", async ({
  page,
}) => {
  await setup(page);
  await connect(page, "slack");
  await expect(page.getByTestId("target-dialog")).toContainText(
    "Webhook-Adresse",
  );
  await expect(page.getByTestId("target-svc-no-test-post")).toBeVisible();
  await page.getByTestId("target-name").fill("Team-Kanal");
  await page
    .getByTestId("target-secret")
    .fill("https://evil.example/services/T0/B0/X");
  await page.getByTestId("target-submit").click();
  await expect(page.getByTestId("target-error")).toContainText(
    "gehört nicht zu diesem Dienst",
  );
  await page.getByTestId("target-secret").fill(SLACK);
  await page.getByTestId("target-submit").click();
  await expect(page.getByTestId("integration-detail")).toBeVisible();
  await expect(page.getByTestId("integration-detail")).toContainText(
    "In Kanal posten",
  );
  await expect(page.locator("body")).not.toContainText("GEHEIM");
  await expect(page.locator("body")).not.toContainText("/services/");
});

test("Jira: optionale Felder sind gekennzeichnet, Pflichtfelder sperren das Anlegen", async ({
  page,
}) => {
  await setup(page);
  await connect(page, "jira");
  const dialog = page.getByTestId("target-dialog");
  await expect(dialog).toContainText("Vorgangstyp (optional)");
  await page.getByTestId("target-name").fill("Jira LV");
  await page.getByTestId("target-svc-site").fill("firma.atlassian.net");
  await page.getByTestId("target-svc-email").fill("p@example.de");
  await page.getByTestId("target-secret").fill("atl-token");
  await expect(page.getByTestId("target-submit")).toBeDisabled();
  await page.getByTestId("target-svc-project_key").fill("LV");
  await expect(page.getByTestId("target-submit")).toBeEnabled();
});

test("iCloud: Apple-ID und app-spezifisches Passwort, Kalendername optional", async ({
  page,
}) => {
  await setup(page);
  await connect(page, "icloud");
  const dialog = page.getByTestId("target-dialog");
  await expect(dialog).toContainText("App-spezifisches Passwort");
  await expect(dialog).toContainText("Kalender (Name) (optional)");
  await page.getByTestId("target-name").fill("iCloud privat");
  await page.getByTestId("target-svc-email").fill("ich@icloud.com");
  await page.getByTestId("target-secret").fill("abcd-efgh-ijkl-mnop");
  await page.getByTestId("target-submit").click();
  await expect(page.getByTestId("integration-detail")).toContainText(
    "iCloud-Kalender",
  );
  await expect(page.getByTestId("integration-detail")).toContainText(
    "Termine anlegen",
  );
  await expect(page.locator("body")).not.toContainText("abcd-efgh");
});

test("Bilder für die Abnahme (nur mit SCREENS_DIR)", async ({ page }) => {
  const dir = process.env.SCREENS_DIR;
  test.skip(!dir, "nur für Abnahmebilder");
  await setup(page);
  await page.getByTestId("catalog-services").scrollIntoViewIfNeeded();
  await page.screenshot({
    path: `${dir}/dienste-katalog.png`,
    animations: "disabled",
    fullPage: true,
  });
  await connect(page, "jira");
  await page.getByTestId("target-name").fill("Jira LV");
  await page.getByTestId("target-svc-site").fill("firma.atlassian.net");
  await page.screenshot({
    path: `${dir}/dienste-dialog-jira.png`,
    animations: "disabled",
  });
});
