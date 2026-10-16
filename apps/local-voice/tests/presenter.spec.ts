import { test, expect, type Page } from "@playwright/test";
import { calls, installTauriMock } from "./calendarMock";
import { installIntegrationsMock, NOW } from "./integrationsMock";

// Etappe 4a: WAI Presenter als Integrationsart (Austauschordner waehlt der Nutzer,
// Datenschutzhinweis, Katalog abschaltbar, Rechte der zwei Faehigkeiten).

const setup = async (page: Page) => {
  await page.clock.setFixedTime(NOW);
  await installTauriMock(page, "main");
  await installIntegrationsMock(page, {});
  await page.setViewportSize({ width: 1280, height: 1000 });
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

test("WAI Presenter einrichten: Ordner waehlt der Nutzer, Katalog abschaltbar", async ({
  page,
}) => {
  await setup(page);
  await openIntegrations(page);
  await page.getByTestId("integrations-add").click();
  await expect(page.getByTestId("catalog")).toBeVisible();
  await page.getByTestId("catalog-setup-presenter").click();
  const dialog = page.getByTestId("target-dialog");
  await expect(dialog).toBeVisible();
  // Kein vorgegebener Ordner, ein Hinweis zur Vertraulichkeit.
  await expect(page.getByTestId("target-path")).toHaveValue("");
  await expect(page.getByTestId("target-presenter-privacy")).toContainText(
    "OneDrive",
  );
  await expect(page.getByTestId("target-submit")).toBeDisabled();
  await page.getByTestId("target-name").fill("WAI Presenter");
  await page.getByTestId("target-path").fill("D:\\Austausch\\WAI Presenter");
  await page.getByTestId("target-presenter-catalog").uncheck();
  await page.getByTestId("target-submit").click();

  await expect(page.getByTestId("integration-detail")).toBeVisible();
  const [created] = await calls(page, "integration_create_with_settings");
  expect(created.args).toMatchObject({
    kind: "presenter",
    label: "WAI Presenter",
    settings: { path: "D:\\Austausch\\WAI Presenter", catalog: false },
  });
  const detail = page.getByTestId("integration-detail");
  await expect(detail).toContainText("Pakete an den Presenter schreiben");
  await expect(detail).toContainText("Anfragen des Presenters bearbeiten");

  const dir = process.env.SCREENS_DIR;
  if (dir) {
    await page.screenshot({
      path: `${dir}/integration-presenter.png`,
      fullPage: true,
    });
  }
});
