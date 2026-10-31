import { test, expect, type Page } from "@playwright/test";
import { installTauriMock } from "./calendarMock";
import { installIntegrationsMock, NOW } from "./integrationsMock";

// Ohne sicheren Tresor (macOS/Linux, Codex-Entscheidung 31.10.2026): Hinweis
// vor jeder Eingabe, Arten mit Geheimnis gesperrt, kein Klartext-Ersatz.
// Arten ohne Geheimnis (Ordner, Obsidian, Presenter, MCP) bleiben nutzbar.

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

const setup = async (page: Page, os: "macos" | "windows" | "linux") => {
  await page.clock.setFixedTime(NOW);
  await installTauriMock(page, "main");
  await installIntegrationsMock(page, {});
  await page.addInitScript((osType) => {
    (
      window as unknown as Record<string, unknown>
    ).__TAURI_OS_PLUGIN_INTERNALS__ = {
      platform: osType,
      os_type: osType,
      family: osType === "windows" ? "windows" : "unix",
      arch: "x86_64",
      version: "1",
      eol: osType === "windows" ? "\r\n" : "\n",
    };
  }, os);
  await page.setViewportSize({ width: 1280, height: 1000 });
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Integrationen", exact: true })
    .click();
  await page.getByTestId("integrations-add").click();
  await expect(page.getByTestId("catalog")).toBeVisible();
};

const LOCKED = [
  "calendar_ics",
  "calendar_graph",
  "m365",
  "smtp",
  "wissen",
  "webhook",
];
const OPEN = ["folder", "mcp", "obsidian", "presenter"];

test("macOS: Hinweis vorab, Arten mit Geheimnis und Dienste gesperrt", async ({
  page,
}) => {
  await setup(page, "macos");
  const notice = page.getByTestId("secret-store-notice");
  await expect(notice).toBeVisible();
  await expect(notice).toContainText(
    "Auf macOS derzeit nicht verfügbar: Die sichere Speicherung von Zugangsdaten wird noch nicht unterstützt.",
  );
  for (const id of LOCKED) {
    await expect(page.getByTestId(`catalog-setup-${id}`), id).toBeDisabled();
  }
  for (const id of OPEN) {
    await expect(page.getByTestId(`catalog-setup-${id}`), id).toBeEnabled();
  }
  await expect(page.getByTestId("catalog-setup-service-slack")).toBeDisabled();
  // Der Hinweis steht vor dem Katalog, also vor jeder Eingabe.
  const noticeBox = (await notice.boundingBox())!;
  const firstItem = (await page
    .getByTestId("catalog-item")
    .first()
    .boundingBox())!;
  expect(noticeBox.y).toBeLessThan(firstItem.y);
});

test("Linux: eigener Hinweistext, gleiche Sperre", async ({ page }) => {
  await setup(page, "linux");
  await expect(page.getByTestId("secret-store-notice")).toContainText(
    "Unter Linux",
  );
  await expect(page.getByTestId("catalog-setup-m365")).toBeDisabled();
});

test("Windows: kein Hinweis, alles einrichtbar", async ({ page }) => {
  await setup(page, "windows");
  await expect(page.getByTestId("secret-store-notice")).toHaveCount(0);
  for (const id of [...LOCKED, ...OPEN]) {
    await expect(page.getByTestId(`catalog-setup-${id}`), id).toBeEnabled();
  }
  await expect(page.getByTestId("catalog-setup-service-slack")).toBeEnabled();
});

test("macOS: Kalender verbinden in den Verbindungen ebenfalls gesperrt", async ({
  page,
}) => {
  await setup(page, "macos");
  await page.getByTestId("integrations-back").click();
  const connect = page.getByTestId("calendar-connect");
  await connect.scrollIntoViewIfNeeded();
  await expect(connect).toBeDisabled();
  await expect(connect).toHaveAttribute("title", /noch nicht verfügbar/);
  // Derselbe Hinweis wie im Katalog steht auch hier, nicht nur im Tooltip.
  await expect(page.getByTestId("secret-store-notice")).toContainText(
    "Auf macOS derzeit nicht verfügbar",
  );
});
