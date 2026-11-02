import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import * as path from "node:path";
import { installLiveMock } from "./recLiveMock";
import { installProjekteMock } from "./projekteMock";
import { calls, openRecordings, pickMeeting } from "./recLayoutMock";

// ADR-0026 PR-7: Suite-Projekt eines Besprechungsordners mit Projektordner (nur dieses Projekt),
// Recherche (Modus Schnell, Quelle Web) und Freigabe-Dialog. Die Regeln des Backends (Default-Deny,
// Halt, Freigabe, Audit ohne Inhalte) pruefen die Rust-Tests; hier die Oberflaeche gegen die Attrappe.

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

const SHOT_DIR = path.resolve(
  process.cwd(),
  "../../docs/vorschau/projekte-recherche",
);
/** Bilder nur auf Wunsch (LVA_SCREENSHOTS=1), sonst schreibt jeder Lauf sie neu. */
const shoot = async (page: Page, name: string) => {
  if (!process.env.LVA_SCREENSHOTS) return;
  await page.waitForTimeout(300);
  await page.screenshot({
    path: path.join(SHOT_DIR, `${name}-${page.viewportSize()?.width}.png`),
    animations: "disabled",
  });
};

const setup = async (page: Page, width = 1440, height = 900) => {
  await installLiveMock(page);
  await installProjekteMock(page);
  await openRecordings(page, width, height);
};

const oeffneSuite = async (page: Page, folderId: string) => {
  const row = page.locator(`[data-project-id="${folderId}"]`);
  if (!(await row.isVisible())) {
    // Mittel: Leiste mit Knopf; schmal: Knopf im Seitenkopf.
    const rail = page.getByTestId("sessions-expand");
    await (
      (await rail.count()) > 0 ? rail : page.getByTestId("sessions-open")
    ).click();
  }
  await row.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Suite-Projekt …" }).click();
  await expect(page.getByTestId("suite-projekt")).toBeVisible();
};

const frageStellen = async (page: Page, frage: string) => {
  const box = page.getByTestId("suite-recherche").locator("textarea, input");
  await box.first().fill(frage);
  await page
    .getByTestId("suite-recherche")
    .getByRole("button", { name: "Recherchieren" })
    .click();
};

for (const [width, height] of [
  [1440, 900],
  [390, 844],
] as const) {
  test(`Projektordner und Recherche (${width} px)`, async ({ page }) => {
    await setup(page, width, height);
    await oeffneSuite(page, "f2");

    const seite = page.getByTestId("suite-projekt");
    await expect(seite.getByText("Jour fixe Vertrieb KW 40")).toBeVisible();
    await expect(seite.getByText("Leitfaden Netzentgelte")).toBeVisible();
    // Nur dieses Projekt: keine Projektliste mit Wechsel, keine anderen Projekte.
    await expect(seite.getByText("Personalgespräche")).toHaveCount(0);
    // Modus und Quelle sind fest (Schnell, Web): keine Auswahl, kein "Gründlich"/"Tief".
    await expect(seite.getByText("Gründlich")).toHaveCount(0);
    await expect(seite.getByText("Tief", { exact: true })).toHaveCount(0);
    await expect(seite.getByText("Projektquellen")).toHaveCount(0);
    await shoot(page, "besprechungsprojekt-projektordner");

    await frageStellen(page, "Wer legt die Netzentgelte fest?");
    await expect(
      seite.getByText("1 Quelle", { exact: false }).first(),
    ).toBeVisible();
    await expect(
      seite.getByText("Netzentgelte - Bundesnetzagentur"),
    ).toBeVisible();
    await expect(seite.getByText("1 Quellen")).toHaveCount(0);
    await shoot(page, "recherche-ergebnis");

    // Das Ergebnis liegt jetzt im Projektordner.
    await expect(
      seite.getByText("Wer legt die Netzentgelte fest?").first(),
    ).toBeVisible();
    // Keine zweite Scrollleiste: nur die Seite selbst scrollt.
    const doppelt = await page.evaluate(() => {
      const r = document.documentElement;
      return r.scrollWidth > r.clientWidth;
    });
    expect(doppelt).toBe(false);
  });
}

for (const [width, height] of [
  [1440, 900],
  [390, 844],
] as const) {
  test(`Ab PERSONAL haelt die Recherche an; die Frage im Dialog gehoert zum Lauf (${width} px)`, async ({
    page,
  }) => {
    await setup(page, width, height);
    await oeffneSuite(page, "f3");
    await frageStellen(page, "Wie funktioniert Elternzeit?");

    const dialog = page.getByRole("alertdialog");
    await expect(dialog).toBeVisible();
    await expect(dialog).toContainText("Wie funktioniert Elternzeit?");
    await expect(dialog).toContainText("personenbezogen");
    await shoot(page, "freigabedialog");

    // Nichts ist vorab gewaehlt; "Ausgewaehlte senden" ist erst mit Auswahl wirksam.
    const senden = dialog.getByRole("button", { name: "Ausgewählte senden" });
    await expect(senden).toBeDisabled();
    await dialog.getByRole("checkbox").first().check();
    await senden.click();

    const entscheidungen = await calls(page, "recherche_freigabe_entscheiden");
    expect(entscheidungen).toHaveLength(1);
    expect(entscheidungen[0].args.aktion).toBe("senden");
    expect(entscheidungen[0].args.auswahl).toEqual(["q1"]);
    await expect(dialog).toBeHidden();
    await expect(
      page.getByTestId("suite-projekt").getByText("1 Quelle").first(),
    ).toBeVisible();
  });
}

test("Abbrechen sendet nichts", async ({ page }) => {
  await setup(page);
  await oeffneSuite(page, "f3");
  await frageStellen(page, "Wie funktioniert Elternzeit?");
  const dialog = page.getByRole("alertdialog");
  await dialog.getByRole("button", { name: "Abbrechen" }).click();
  const e = await calls(page, "recherche_freigabe_entscheiden");
  expect(e[0].args.aktion).toBe("abbrechen");
  expect(e[0].args.auswahl).toEqual([]);
  await expect(dialog).toBeHidden();
});

test("Ohne Web-Freigabe ist die Recherche gesperrt", async ({ page }) => {
  await setup(page);
  await oeffneSuite(page, "f1");
  await expect(
    page.getByTestId("suite-recherche").getByText("nicht freigegeben"),
  ).toBeVisible();
  await expect(page.getByTestId("suite-web")).not.toBeChecked();
});

test("Chat-Spalte: Recherche-Knopf nutzt ein Suite-Projekt des Ordners", async ({
  page,
}) => {
  await setup(page);
  // m2 liegt in zwei Ordnern mit je einem Suite-Projekt: die Person waehlt das Projekt.
  await pickMeeting(page, "m2");
  await page.getByRole("button", { name: "Fragen", exact: true }).click();
  const panel = page.getByTestId("chat-panel");
  const knopf = panel.getByTestId("chat-recherche");
  await expect(knopf).toBeDisabled();
  await panel.locator("textarea").fill("Wie funktioniert Elternzeit?");
  await knopf.click();
  await panel.getByRole("button", { name: "Personalgespräche" }).click();
  const start = await calls(page, "recherche_starten");
  expect(start).toHaveLength(1);
  expect(start[0].args.suiteProjekt).toBe("sp2");
  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toContainText("Wie funktioniert Elternzeit?");
  await dialog.getByRole("button", { name: "Abbrechen" }).click();
  await expect(dialog).toBeHidden();
});

test("Barrierefreiheit der Projektseite", async ({ page }) => {
  await setup(page);
  await oeffneSuite(page, "f2");
  const r = await new AxeBuilder({ page })
    .include('[data-testid="suite-projekt"]')
    .analyze();
  expect(r.violations.filter((v) => v.impact === "critical")).toEqual([]);
});
