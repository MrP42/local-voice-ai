import { expect, type Page } from "@playwright/test";

// Suite-Shell (ADR-0027): Einstellungen, Modelle und Integrationen sind keine
// Einträge der Navigation mehr, sondern stehen im Profilmenü unten links.
// Diese Helfer ersetzen den früheren Klick auf den Navigationseintrag.

export type ShellZiel = "Einstellungen" | "Integrationen" | "Modelle";

/** Öffnet das Profilmenü (Profilkreis unten in der Navigation). */
export async function oeffneProfilmenue(page: Page): Promise<void> {
  const knopf = page
    .getByRole("navigation", { name: "Hauptnavigation" })
    .getByRole("button", { name: /^Profil:/ });
  // Schmal liegt die Navigation in der Schublade hinter dem Hamburger.
  if (!(await knopf.isVisible()))
    await page.locator(".lv-titlebar__hamburger").click();
  await knopf.click();
  await expect(page.locator(".wai-konto-menue")).toBeVisible();
}

/** Navigiert über das Profilmenü zu Einstellungen, Integrationen oder Modelle. */
export async function geheZu(page: Page, ziel: ShellZiel): Promise<void> {
  await oeffneProfilmenue(page);
  const menue = page.locator(".wai-konto-menue");
  if (ziel === "Modelle") {
    await menue.getByRole("button", { name: "Modelle", exact: true }).click();
    await menue
      .getByRole("button", { name: "Modelle verwalten", exact: true })
      .click();
  } else {
    await menue.getByRole("button", { name: ziel, exact: true }).click();
  }
  await expect(page.locator(".wai-konto-menue")).toHaveCount(0);
}

/** Öffnet Profilmenü ▸ Modelle (Diktat- und Sprachmodell wählen). */
export async function oeffneModellwahl(page: Page): Promise<void> {
  await oeffneProfilmenue(page);
  await page
    .locator(".wai-konto-menue")
    .getByRole("button", { name: "Modelle", exact: true })
    .click();
  await expect(page.getByTestId("profilmenue-modelle")).toBeVisible();
}

export const DIAGNOSE_AN = "lva.ui.statusbar.diagnose";

/** Schaltet die Diagnose der Statusleiste (Speicher, Fish, Schild) ein. */
export async function diagnoseAn(page: Page): Promise<void> {
  await page.addInitScript(
    (key) => window.localStorage.setItem(key, "auf"),
    DIAGNOSE_AN,
  );
}
