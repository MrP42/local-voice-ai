import { test, expect, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { installTauriMock } from "./calendarMock";
import { DEMO, installIntegrationsMock, NOW } from "./integrationsMock";

// A4 (#66, AK12 Buendel 2): Bilder der Seite "Integrationen" fuer die Abnahme:
// Liste, Katalog, Rechte-Matrix, Freigabe, Protokoll (und schmal). Nur mit
// LVA_SCREENS=1, sonst uebersprungen; die Bilder landen in
// koordination/integrationen/screens-a4/.

test.skip(!process.env.LVA_SCREENS, "Bilder nur mit LVA_SCREENS=1");
test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

const OUT = resolve(
  dirname(fileURLToPath(import.meta.url)),
  "../../../koordination/integrationen/screens-a4",
);

const shot = async (page: Page, name: string) => {
  mkdirSync(OUT, { recursive: true });
  await page.addStyleTag({
    content:
      "*,*::before,*::after{transition:none!important;animation:none!important}",
  });
  await page.waitForTimeout(250);
  await page.screenshot({
    path: resolve(OUT, `${name}.png`),
    animations: "disabled",
  });
};

const setup = async (page: Page, width = 1280, height = 900) => {
  const narrow = width < 768;
  await page.clock.setFixedTime(NOW);
  await installTauriMock(page, "main");
  await installIntegrationsMock(page, DEMO);
  await page.addInitScript(() => {
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
      {
        id: "graph-1",
        kind: "graph",
        label: "Outlook / Microsoft 365",
        account_hint: "anna.berg@firma.de",
        enabled: true,
        has_attendee_data: true,
        last_sync_at: 1_790_000_000_000,
        last_ok_at: 1_790_000_000_000,
        last_error: null,
        event_count: 12,
      },
    ];
    (window as any).__settings.meeting_mcp_enabled = true;
  });
  await page.setViewportSize({ width, height });
  await page.goto("/");
  const nav = page.getByRole("navigation");
  if (narrow) await nav.getByRole("button", { name: "Mehr" }).click();
  await nav.getByRole("button", { name: "Integrationen", exact: true }).click();
  await expect(page.getByTestId("integration-card").first()).toBeVisible();
};

test("Bilder Integrationen", async ({ page }) => {
  await setup(page);
  await shot(page, "1-liste");

  await page.getByTestId("integrations-add").click();
  await expect(page.getByTestId("catalog")).toBeVisible();
  await shot(page, "2-katalog");
  await page.getByTestId("integrations-back").click();

  await page
    .locator('[data-integration-id="ordner-berichte"]')
    .getByTestId("integration-open")
    .click();
  await expect(page.getByTestId("rights-matrix")).toBeVisible();
  await page.setViewportSize({ width: 1280, height: 1500 });
  await shot(page, "3-rechte-matrix");
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.getByTestId("integrations-back").click();

  await page.getByTestId("approvals-open").click();
  await expect(page.getByTestId("approval-dialog")).toBeVisible();
  await shot(page, "4-freigabe");
  await page.keyboard.press("Escape");

  await page.getByRole("tab", { name: "Protokoll" }).click();
  await expect(page.getByTestId("audit-row").first()).toBeVisible();
  await shot(page, "5-protokoll");
});

test("Bilder Integrationen schmal", async ({ page }) => {
  await setup(page, 390, 844);
  await shot(page, "6-schmal-liste");
  await page
    .locator('[data-integration-id="ordner-berichte"]')
    .getByTestId("integration-open")
    .click();
  await expect(page.getByTestId("rights-matrix")).toBeVisible();
  await shot(page, "7-schmal-rechte-matrix");
});
