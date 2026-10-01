import { test, expect, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { installTauriMock } from "./calendarMock";
import { DEMO, installIntegrationsMock, NOW } from "./integrationsMock";
import { AUTOMATIONS_DEMO, installAutomationsMock } from "./automationsMock";

// B7 (#67, AK12): Bilder der Oberflaeche "Automationen" fuer die Abnahme: Liste, Editor,
// Trockenlauf, Lauf mit Herkunft, Freigabe (und schmal). Nur mit LVA_SCREENS=1; die Bilder
// landen in koordination/workflow-automation/screens-b7/.

test.skip(!process.env.LVA_SCREENS, "Bilder nur mit LVA_SCREENS=1");
test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

const OUT = resolve(
  dirname(fileURLToPath(import.meta.url)),
  "../../../koordination/workflow-automation/screens-b7",
);

const shot = async (page: Page, name: string, fullPage = false) => {
  mkdirSync(OUT, { recursive: true });
  await page.addStyleTag({
    content:
      "*,*::before,*::after{transition:none!important;animation:none!important}",
  });
  await page.waitForTimeout(300);
  await page.screenshot({
    path: resolve(OUT, `${name}.png`),
    animations: "disabled",
    fullPage,
  });
};

const setup = async (page: Page, width = 1280, height = 900) => {
  await page.clock.setFixedTime(NOW);
  await installTauriMock(page, "main");
  await installIntegrationsMock(page, DEMO);
  await installAutomationsMock(page, AUTOMATIONS_DEMO);
  await page.setViewportSize({ width: 1280, height });
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Integrationen", exact: true })
    .click();
  await page.getByRole("tab", { name: "Automationen" }).click();
  await expect(page.getByTestId("workflow-card")).toHaveCount(2);
  if (width !== 1280) await page.setViewportSize({ width, height });
};

test("Bild: Liste", async ({ page }) => {
  await setup(page);
  await shot(page, "1-liste");
});

test("Bild: Editor", async ({ page }) => {
  await setup(page, 1280, 1500);
  await page
    .locator('[data-workflow-id="wf-eingang"]')
    .getByTestId("workflow-edit")
    .click();
  await expect(page.getByTestId("workflow-editor")).toBeVisible();
  // Ein Befund mit Pfad, damit die Prüfung sichtbar ist
  await page.getByTestId("step-2-name").fill("");
  await page.getByTestId("step-1-when").fill("{{trigger.size > 1000");
  await expect(page.getByTestId("editor-issues")).toBeVisible();
  await page
    .getByRole("heading", { name: "Ablauf bearbeiten" })
    .scrollIntoViewIfNeeded();
  await shot(page, "2-editor");
});

test("Bild: Trockenlauf", async ({ page }) => {
  await setup(page, 1280, 1100);
  await page
    .locator('[data-workflow-id="wf-eingang"]')
    .getByTestId("workflow-plan")
    .click();
  await expect(page.getByTestId("plan-step")).toHaveCount(3);
  await shot(page, "3-trockenlauf");
});

test("Bild: Lauf mit Herkunft", async ({ page }) => {
  await setup(page, 1280, 1100);
  await page.getByRole("tab", { name: "Läufe", exact: true }).click();
  await page.locator('[data-run-id="run-ok"]').getByTestId("run-open").click();
  await expect(page.getByTestId("run-provenance")).toBeVisible();
  await shot(page, "4-lauf");
});

test("Bild: Freigabe", async ({ page }) => {
  await setup(page, 1280, 1000);
  await page.getByRole("tab", { name: "Läufe", exact: true }).click();
  await page
    .locator('[data-run-id="run-wait"]')
    .getByTestId("run-open")
    .click();
  await expect(page.getByTestId("run-approval")).toBeVisible();
  await shot(page, "5-lauf-wartet-auf-freigabe");
  await page.getByTestId("run-approval-open").click();
  await expect(page.getByTestId("approval-dialog")).toBeVisible();
  await shot(page, "6-freigabe-dialog");
});

test("Bild: schmal", async ({ page }) => {
  await setup(page, 390, 1100);
  await shot(page, "7-schmal-liste");
  await page
    .locator('[data-workflow-id="wf-eingang"]')
    .getByTestId("workflow-edit")
    .click();
  await expect(page.getByTestId("workflow-editor")).toBeVisible();
  await shot(page, "8-schmal-editor");
});
