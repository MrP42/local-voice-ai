import { expect, type Page } from "@playwright/test";

// lva#826: Projekte stehen nicht mehr in der Navigation, sondern als Räume in der Seitenleiste des
// Chats (Start). Diese Helfer ersetzen den früheren Weg über den Menüpunkt „Projekte“.

/** Öffnet die Seitenleiste des Chats (breit: Spalte, schmal: Schublade), falls sie zu ist. */
export const chatSeitenleiste = async (page: Page) => {
  await expect(
    page.getByTestId("chat-input").or(page.getByTestId("projekt-raum")).first(),
  ).toBeVisible();
  const auf = page.locator(
    'button[aria-label="Gesprächsverlauf"][aria-expanded="false"]',
  );
  if ((await auf.count()) > 0) await auf.first().click();
  await expect(page.getByTestId("projekt-zeile").first()).toBeVisible();
};

/** Öffnet den Raum eines Projekts aus der Seitenleiste des Chats. */
export const raumOeffnen = async (page: Page, titel: string) => {
  await chatSeitenleiste(page);
  await page
    .getByTestId("projekt-zeile")
    .filter({ hasText: titel })
    .first()
    .click();
  await expect(page.getByTestId("projekt-raum")).toBeVisible();
};

/**
 * Früher „Projekte“ in der Navigation: zum Chat, und breit wie früher das erste Projekt öffnen.
 * Schmal (≤ 620 px) bleibt es bei der Liste, wie früher die Projektliste allein stand.
 */
export const zuDenProjekten = async (page: Page) => {
  const start = page.getByRole("button", { name: "Start", exact: true });
  if (!(await start.isVisible())) {
    const toggle = page.getByRole("button", { name: /Menü/ }).first();
    if (await toggle.isVisible()) await toggle.click();
  }
  await start.click();
  await chatSeitenleiste(page);
  const breite = page.viewportSize()?.width ?? 1440;
  if (breite > 620) await page.getByTestId("projekt-zeile").first().click();
};
