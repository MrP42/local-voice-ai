import { mkdir } from "node:fs/promises";
import { resolve } from "node:path";
import { test, expect, type Page } from "@playwright/test";
import { installMock } from "./planungMock";
import { installTerminfindung, type TfZustand } from "./terminfindungMock";

// Terminfindung (lva#845 TF5): Seitenkarte, Dialog in drei Stufen, hell/dunkel, 390/1440.
test.use({ locale: "de-DE", timezoneId: "Europe/Berlin" });

async function aufnehmen(page: Page, name: string) {
  if (process.env.TF_SHOTS !== "1") return;
  const dir = resolve(process.env.LV_SHOT_DIR ?? "docs/vorschau/terminfindung");
  await mkdir(dir, { recursive: true });
  await page.addStyleTag({
    content:
      "*,*::before,*::after{animation-duration:0s!important;transition-duration:0s!important}",
  });
  await page.screenshot({ path: resolve(dir, name), fullPage: true });
}

async function oeffnen(
  page: Page,
  zustand: TfZustand,
  theme: "light" | "dark",
  breite: number,
) {
  await installMock(page, { theme });
  await installTerminfindung(page, zustand);
  await page.setViewportSize({
    width: breite,
    height: breite === 390 ? 844 : 1000,
  });
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  if (breite === 390) {
    await page
      .getByRole("button", { name: /Men(?:ü|ue) (?:ö|oe)ffnen/ })
      .click();
    await page.keyboard.press("Escape");
  }
  await page
    .getByTestId("planung-zeile")
    .filter({ hasText: "Abstimmung Pilotstart" })
    .click();
  await expect(page.getByTestId("tf-karte")).toBeVisible();
}

for (const theme of ["light", "dark"] as const) {
  for (const breite of [390, 1440]) {
    const farbe = theme === "light" ? "hell" : "dunkel";

    test(`entwurf-${breite}-${farbe}`, async ({ page }) => {
      await oeffnen(page, "entwurf", theme, breite);
      await page.getByTestId("tf-karte-oeffnen").click();
      await expect(page.getByTestId("tf-raster")).toBeVisible();
      await expect(page.getByTestId("tf-vorschlag")).toHaveCount(3);
      await expect(page.getByText("Kalender aktuell")).toBeVisible();
      // Vierten Vorschlag setzen und wieder entfernen.
      const zelle = page.getByRole("gridcell", { name: /Fr .* 15:00/ }).first();
      await zelle.click();
      await expect(page.getByTestId("tf-vorschlag")).toHaveCount(4);
      await aufnehmen(page, `1-entwurf-${breite}-${farbe}.png`);
      await zelle.click();
      await expect(page.getByTestId("tf-vorschlag")).toHaveCount(3);
      await expect(page.getByTestId("tf-senden")).toBeEnabled();
      expect(
        await page.evaluate(() => document.documentElement.scrollWidth),
      ).toBeLessThanOrEqual(breite);
    });

    test(`offen-${breite}-${farbe}`, async ({ page }) => {
      await oeffnen(page, "offen", theme, breite);
      await expect(page.getByTestId("tf-karte")).toContainText(
        "Antworten laufen",
      );
      await page.getByTestId("tf-karte-oeffnen").click();
      await expect(page.getByTestId("tf-matrix")).toBeVisible();
      await expect(page.getByTestId("tf-empfehlung")).toContainText("22 P");
      await expect(page.getByTestId("tf-pruefung")).toContainText(
        "Tobias Roth",
      );
      await expect(page.getByTestId("tf-zustellung")).toContainText(
        "Eva Maurer",
      );
      await aufnehmen(page, `2-offen-${breite}-${farbe}.png`);
      await page.getByTestId("tf-weiter-senden").click();
      await expect(page.getByTestId("tf-zustellung")).toHaveCount(0);
      await page
        .getByTestId("tf-pruefung")
        .getByRole("button", { name: "Übernehmen" })
        .click();
      await expect(page.getByTestId("tf-pruefung")).toContainText(
        "Nichts zu prüfen",
      );
      await page.getByTestId("tf-uebernehmen").click();
      await expect(page.getByTestId("tf-matrix")).toHaveCount(0);
      expect(
        await page.evaluate(() => document.documentElement.scrollWidth),
      ).toBeLessThanOrEqual(breite);
    });
  }
}

test("leer: Karte bietet Start an", async ({ page }) => {
  await oeffnen(page, "leer", "light", 1440);
  await expect(page.getByTestId("tf-karte-starten")).toBeEnabled();
  await page.getByTestId("tf-karte-starten").click();
  await expect(page.getByTestId("tf-entwurf")).toBeVisible();
});
