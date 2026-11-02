import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import * as path from "node:path";
import { installLiveMock } from "./recLiveMock";
import { installProjekteMock } from "./projekteMock";
import { calls, openRecordings, pickMeeting } from "./recLayoutMock";

// ADR-0026 PR-7: Suite-Projekt eines Besprechungsordners mit Projektordner (nur dieses Projekt),
// Recherche (Modus Schnell, Quelle Web) und Freigabe-Dialog. Die Regeln des Backends (Default-Deny,
// Halt, Freigabe, Audit ohne Inhalte) pruefen die Rust-Tests; hier die Oberflaeche gegen die Attrappe.
// Szene der Bilder: Ordner "Kunde Stadtwerke" = Suite-Projekt "Kunde Stadtwerke" (PERSONAL).

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

const FRAGE = "Wie werden Netzentgelte für die Lastgangmessung berechnet?";

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
  // Erst den Ordner wählen: der markierte Ordner ist der, dessen Suite-Projekt die Seite zeigt.
  await row.click();
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

const bus = (page: Page, art: string, detail?: string) =>
  page.evaluate(
    ([a, d]) =>
      window.dispatchEvent(
        new CustomEvent("lva:projekte", { detail: { art: a, detail: d } }),
      ),
    [art, detail] as const,
  );

const suite = (page: Page, key: string, value: unknown) =>
  page.evaluate(([k, v]) => ((window as any).__suite[k as string] = v), [
    key,
    value,
  ] as const);

for (const [width, height] of [
  [1440, 900],
  [390, 844],
] as const) {
  test(`Projektordner, Halt, Freigabe und Ergebnis (${width} px)`, async ({
    page,
  }) => {
    await setup(page, width, height);
    await oeffneSuite(page, "f3");

    const seite = page.getByTestId("suite-projekt");
    await expect(
      seite.getByText("Kundentermin Stadtwerke Kaiserslautern – Lastgang"),
    ).toBeVisible();
    await expect(seite.getByText("Leitfaden Netzentgelte")).toBeVisible();
    // Der markierte Ordner links ist der gezeigte (nur breit sichtbar).
    await expect(page.locator('[data-project-id="f3"]')).toHaveAttribute(
      "aria-current",
      "true",
    );
    // Nur dieses Projekt, Modus und Quelle fest: keine Auswahl, kein "Gründlich"/"Tief".
    await expect(seite.getByText("Jour fixe Vertrieb KW 40")).toHaveCount(0);
    await expect(seite.getByText("Gründlich")).toHaveCount(0);
    await expect(seite.getByText("Tief", { exact: true })).toHaveCount(0);
    await expect(seite.getByText("Projektquellen")).toHaveCount(0);
    await shoot(page, "besprechungsprojekt-projektordner");

    await frageStellen(page, FRAGE);
    const dialog = page.getByRole("alertdialog");
    await expect(dialog).toBeVisible();
    await expect(dialog).toContainText(FRAGE);
    await expect(dialog).toContainText("personenbezogen");
    await expect(dialog).toContainText("noch 15 Minuten");
    await expect(
      dialog.getByText(
        "Netzentgelte Lastgangmessung Stadtwerke Kaiserslautern",
      ),
    ).toBeVisible();
    // "Nur im Projekt suchen" gibt es nicht (kein Knopf ohne Wirkung).
    await expect(
      dialog.getByRole("button", { name: "Nur im Projekt suchen" }),
    ).toHaveCount(0);
    await shoot(page, "freigabedialog");

    // Nichts ist vorab gewählt; "Ausgewählte senden" ist erst mit Auswahl wirksam.
    const senden = dialog.getByRole("button", { name: "Ausgewählte senden" });
    await expect(senden).toBeDisabled();
    await dialog.getByRole("checkbox").nth(0).check();
    await senden.click();
    const e = await calls(page, "recherche_freigabe_entscheiden");
    expect(e).toHaveLength(1);
    expect(e[0].args.aktion).toBe("senden");
    expect(e[0].args.auswahl).toEqual(["q1"]);
    await expect(dialog).toBeHidden();

    await expect(seite.getByText("1 Quelle").first()).toBeVisible();
    await expect(
      seite.getByText("Netzentgelte Strom - Bundesnetzagentur"),
    ).toBeVisible();
    await expect(seite.getByText("1 Quellen")).toHaveCount(0);
    await shoot(page, "recherche-ergebnis");
    // Das Ergebnis liegt im Projektordner; keine zweite Seitenleiste.
    await expect(seite.getByText(FRAGE).first()).toBeVisible();
    const breiter = await page.evaluate(
      () =>
        document.documentElement.scrollWidth >
        document.documentElement.clientWidth,
    );
    expect(breiter).toBe(false);

    // Löschen: Dokument und Eintrag verschwinden.
    await seite.getByTestId("suite-recherche-loeschen").click();
    const l = await calls(page, "recherche_loeschen");
    expect(l).toHaveLength(1);
    await expect(
      seite.getByText("Netzentgelte Strom - Bundesnetzagentur"),
    ).toHaveCount(0);
  });
}

test("Abbrechen sendet nichts und meldet es", async ({ page }) => {
  await setup(page);
  await oeffneSuite(page, "f3");
  await frageStellen(page, FRAGE);
  const dialog = page.getByRole("alertdialog");
  await dialog.getByRole("button", { name: "Abbrechen" }).click();
  const e = await calls(page, "recherche_freigabe_entscheiden");
  expect(e[0].args.aktion).toBe("abbrechen");
  expect(e[0].args.auswahl).toEqual([]);
  await expect(dialog).toBeHidden();
  await expect(page.getByTestId("suite-recherche")).toContainText(
    "Es wurde nichts gesendet.",
  );
  await expect(
    page.getByTestId("suite-projekt").getByText("1 Quelle"),
  ).toHaveCount(0);
});

test("Ohne Web-Freigabe ist die Recherche gesperrt", async ({ page }) => {
  await setup(page);
  await oeffneSuite(page, "f1");
  await expect(
    page.getByTestId("suite-recherche").getByText("nicht freigegeben"),
  ).toBeVisible();
  await expect(page.getByTestId("suite-web")).not.toBeChecked();
});

test("Doppelklick auf Senden entscheidet nur einmal", async ({ page }) => {
  await setup(page);
  await oeffneSuite(page, "f3");
  await suite(page, "entscheidenMs", 600);
  await frageStellen(page, FRAGE);
  const dialog = page.getByRole("alertdialog");
  await dialog.getByRole("checkbox").nth(0).check();
  await dialog
    .getByRole("button", { name: "Ausgewählte senden" })
    .dblclick({ delay: 20 });
  await expect(dialog).toBeHidden({ timeout: 10_000 });
  expect(await calls(page, "recherche_freigabe_entscheiden")).toHaveLength(1);
});

test("Lesefehler beim Prüfen lässt den Dialog stehen und meldet einmal", async ({
  page,
}) => {
  await setup(page);
  await oeffneSuite(page, "f3");
  await frageStellen(page, FRAGE);
  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toBeVisible();
  await suite(page, "failFreigaben", true);
  await bus(page, "freigabe");
  await bus(page, "freigabe");
  await expect(
    page.getByText("Offene Freigaben konnten nicht gelesen werden."),
  ).toHaveCount(1);
  await expect(dialog).toBeVisible();
});

test("Während einer Entscheidung wird nicht gelesen", async ({ page }) => {
  await setup(page);
  await oeffneSuite(page, "f3");
  await suite(page, "entscheidenMs", 1200);
  await frageStellen(page, FRAGE);
  const dialog = page.getByRole("alertdialog");
  await dialog.getByRole("checkbox").nth(0).check();
  await dialog.getByRole("button", { name: "Ausgewählte senden" }).click();
  const vorher = (await calls(page, "recherche_freigaben")).length;
  await bus(page, "freigabe");
  await bus(page, "freigabe");
  await page.waitForTimeout(300);
  expect((await calls(page, "recherche_freigaben")).length).toBe(vorher);
  await expect(dialog).toBeHidden({ timeout: 10_000 });
});

test("Ein Fehler der Entscheidung gilt nur für diesen Halt", async ({
  page,
}) => {
  await setup(page);
  await oeffneSuite(page, "f3");
  await suite(
    page,
    "entscheidenFehler",
    "recherche.invalid: Auswahl passt nicht",
  );
  await frageStellen(page, FRAGE);
  const dialog = page.getByRole("alertdialog");
  await dialog.getByRole("checkbox").nth(0).check();
  await dialog.getByRole("button", { name: "Ausgewählte senden" }).click();
  await expect(dialog.getByRole("alert")).toContainText("Auswahl");
  // Zweiter Halt: kein alter Fehler im Dialog.
  await suite(page, "entscheidenFehler", null);
  await dialog.getByRole("button", { name: "Abbrechen" }).click();
  await expect(dialog).toBeHidden();
  await frageStellen(page, "Wie hoch war der Leistungspreis 2025?");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("alert")).toHaveCount(0);
});

test("Fremde Recherchen erscheinen nicht im falschen Kontext", async ({
  page,
}) => {
  await setup(page);
  await oeffneSuite(page, "f3");
  // Eine Recherche aus einem anderen Kontext meldet sich fertig: diese Seite wartet nicht darauf.
  await bus(page, "recherche", "rc-fremd");
  await page.waitForTimeout(300);
  expect(await calls(page, "recherche_lesen")).toHaveLength(0);
  await expect(page.getByTestId("suite-recherche")).toContainText(
    "Noch keine Recherche",
  );
});

test("Chat-Spalte: Projekt wählen, Eingabe bleibt bis zum angenommenen Start", async ({
  page,
}) => {
  await setup(page);
  // m2 liegt in zwei Ordnern mit je einem Suite-Projekt: die Person wählt das Projekt.
  await pickMeeting(page, "m2");
  await page.getByRole("button", { name: "Fragen", exact: true }).click();
  const panel = page.getByTestId("chat-panel");
  const knopf = panel.getByTestId("chat-recherche");
  await expect(knopf).toBeDisabled();
  const eingabe = panel.locator("textarea");
  await eingabe.fill(FRAGE);
  await knopf.click();
  // Vor der Wahl ist noch nichts gestartet, die Frage bleibt stehen.
  await expect(eingabe).toHaveValue(FRAGE);
  await panel.getByRole("button", { name: "Kunde Stadtwerke" }).click();
  const start = await calls(page, "recherche_starten");
  expect(start).toHaveLength(1);
  expect(start[0].args.suiteProjekt).toBe("sp2");
  expect(start[0].args.besprechung).toBe("m2");
  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toContainText(FRAGE);
  await expect(eingabe).toHaveValue("");
  // Nach der Freigabe gehört genau dieses Ergebnis in die Chat-Spalte.
  await dialog.getByRole("checkbox").nth(0).check();
  await dialog.getByRole("button", { name: "Ausgewählte senden" }).click();
  await expect(panel.getByTestId("recherche-karte")).toContainText(FRAGE);
  await expect(panel.getByTestId("recherche-abgelegt")).toContainText(
    "Kunde Stadtwerke",
  );
});

test("Chat-Spalte: abgebrochener Halt zeigt kein Ergebnis", async ({
  page,
}) => {
  await setup(page);
  await pickMeeting(page, "m2");
  await page.getByRole("button", { name: "Fragen", exact: true }).click();
  const panel = page.getByTestId("chat-panel");
  await panel.locator("textarea").fill(FRAGE);
  await panel.getByTestId("chat-recherche").click();
  await panel.getByRole("button", { name: "Kunde Stadtwerke" }).click();
  const dialog = page.getByRole("alertdialog");
  await dialog.getByRole("button", { name: "Abbrechen" }).click();
  await expect(panel.getByTestId("chat-notice")).toContainText(
    "Es wurde nichts gesendet.",
  );
  await expect(panel.getByTestId("recherche-karte")).toHaveCount(0);
});

test("Barrierefreiheit der Projektseite", async ({ page }) => {
  await setup(page);
  await oeffneSuite(page, "f2");
  const r = await new AxeBuilder({ page })
    .include('[data-testid="suite-projekt"]')
    .analyze();
  expect(r.violations.filter((v) => v.impact === "critical")).toEqual([]);
});
