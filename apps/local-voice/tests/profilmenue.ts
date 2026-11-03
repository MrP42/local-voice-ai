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
  // „Modelle“ führt seit Patrick 03.11. ohne Zwischenebene zur Verwaltung.
  await menue.getByRole("button", { name: ziel, exact: true }).click();
  await expect(page.locator(".wai-konto-menue")).toHaveCount(0);
}

/**
 * Modellwahl (Diktat- und Sprachmodell): steht seit Patrick 02.11. wieder
 * links unten in der Fußleiste, je Modul. Der Helfer prüft nur, dass sie da ist.
 */
export async function oeffneModellwahl(page: Page): Promise<void> {
  await expect(page.getByTestId("fuss-modelle")).toBeVisible();
}

export const DIAGNOSE_AN = "lva.ui.statusbar.diagnose";

/** Schaltet die Diagnose der Statusleiste (Speicher, Fish, Schild) ein. */
export async function diagnoseAn(page: Page): Promise<void> {
  await page.addInitScript(
    (key) => window.localStorage.setItem(key, "auf"),
    DIAGNOSE_AN,
  );
}
