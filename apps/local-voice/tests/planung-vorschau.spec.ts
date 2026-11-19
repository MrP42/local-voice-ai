import { mkdir } from "node:fs/promises";
import { resolve } from "node:path";
import { test, expect, type Page } from "@playwright/test";
import { installMock } from "./planungMock";

// Englischer Browser als Gegenprobe für die deutsche 24-Stunden-Zeit.
test.use({ locale: "en-US", timezoneId: "Europe/Berlin" });

const zustaende = [
  "1-liste",
  "2-neue-planung",
  "3-regel-verletzt",
  "4-verschoben",
  "5-einwilligungen",
  "6-einwilligungen-fehler",
] as const;

const zeile = (page: Page, titel: string) =>
  page.getByTestId("planung-zeile").filter({ hasText: titel });

async function einwilligungsfehler(page: Page, fehler: boolean) {
  await page.evaluate((fehler) => {
    (window as any).__steuerung.besprechungsplanung_einwilligungen = {
      fehler,
    };
  }, fehler);
}

async function aufnehmen(page: Page, dateiname: string) {
  if (process.env.PLANUNG_SHOTS !== "1") return;
  const verzeichnis = resolve(
    process.env.LV_SHOT_DIR ?? "docs/vorschau/p2-besprechungsplanung",
  );
  await mkdir(verzeichnis, { recursive: true });
  await page.addStyleTag({
    content:
      "*,*::before,*::after{animation-duration:0s!important;animation-delay:0s!important;transition-duration:0s!important;transition-delay:0s!important}",
  });
  await page.screenshot({
    path: resolve(verzeichnis, dateiname),
    fullPage: true,
  });
}

for (const theme of ["light", "dark"] as const) {
  for (const breite of [390, 1440]) {
    for (const zustand of zustaende) {
      const farbe = theme === "light" ? "hell" : "dunkel";
      test(`${zustand}-${breite}-${farbe}`, async ({ page }) => {
        await installMock(page, { theme });
        await page.setViewportSize({
          width: breite,
          // Aufnahmen: hoeher, damit Regelhinweise, Vorschau und Einwilligungen mit ins Bild kommen.
          height: SHOTS ? (breite === 390 ? 2600 : 1700) : breite === 390 ? 844 : 900,
        });
        await page.goto("/");
        await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
        // Die Attrappe wählt Planung bereits vor; den schmalen Drawer kurz öffnen.
        if (breite === 390) {
          await page
            .getByRole("button", { name: /Men(?:ü|ue) (?:ö|oe)ffnen/ })
            .click();
          await page.keyboard.press("Escape");
        }
        await expect(page.getByTestId("planung-seite")).toBeVisible();

        if (zustand === "1-liste") {
          await expect(page.getByTestId("planung-liste")).toBeVisible();
          await expect(page.getByTestId("planung-zeile")).toHaveCount(5);
          await expect(zeile(page, "Abstimmung Pilotstart")).toContainText(
            "Entwurf",
          );
          await expect(zeile(page, "Review Datenschutz")).toContainText(
            "Verschoben",
          );
          const archiv = page.getByRole("tab", { name: "Archiv" });
          await expect(archiv).toBeVisible();
          await aufnehmen(page, `${zustand}-${breite}-${farbe}.png`);
          await archiv.click();
          await expect(page.getByTestId("planung-zeile")).toHaveCount(2);
          await expect(zeile(page, "Kickoff Archiv")).toContainText("Abgesagt");
          await expect(zeile(page, "Retrospektive Oktober")).toContainText(
            "Durchgeführt",
          );
          return;
        }

        if (zustand === "2-neue-planung" || zustand === "3-regel-verletzt") {
          await page.getByTestId("planung-neu").click();
          await expect(page.getByTestId("planung-formular")).toBeVisible();
          await expect(page.getByTestId("planung-speichern")).toBeDisabled();
          await page.getByTestId("planung-titel").fill("Planung Pilotbetrieb");
          await page
            .getByTestId("planung-ziel")
            .fill("Wir entscheiden über den Pilotstart.");
          const beginn = page.getByTestId("planung-beginn");
          await beginn.fill("25:00");
          await expect(page.getByTestId("planung-speichern")).toBeDisabled();
          await expect(page.getByRole("alert")).toContainText("HH:MM");
          await beginn.fill("09:00");
          await expect(beginn).toHaveAttribute("type", "text");
          await expect(beginn).toHaveValue("09:00");
          await expect(page.getByTestId("planung-speichern")).toBeEnabled();
          await expect(page.getByTestId("planung-regel-agenda_leer")).toBeVisible();
          await expect(page.getByTestId("planung-vorschau-html")).toHaveAttribute(
            "sandbox",
            "",
          );
          await expect(
            page
              .frameLocator('[data-testid="planung-vorschau-html"]')
              .getByText("Planung Pilotbetrieb"),
          ).toBeVisible();
          if (zustand === "3-regel-verletzt") {
            await page.getByTestId("planung-agenda-neu").click();
            await page
              .getByRole("textbox", { name: "Minuten von Punkt 1" })
              .fill("5");
            await expect(
              page.getByTestId("planung-regel-agenda_punkt_leer"),
            ).toBeVisible();
            await expect(page.getByTestId("planung-speichern")).toBeDisabled();
          }
        } else if (zustand === "4-verschoben") {
          await zeile(page, "Review Datenschutz").click();
          const termin = page.getByTestId("planung-termin");
          await expect(
            termin.getByTestId("planung-status-verschoben"),
          ).toBeVisible();
          await expect(termin).toContainText("Raum 1.02");
          await expect(page.getByTestId("planung-beginn")).toHaveAttribute(
            "readonly",
            "",
          );
          await expect(
            page.getByTestId("planung-verschiebung-bestaetigen"),
          ).toBeVisible();
        } else {
          const fehler = zustand === "6-einwilligungen-fehler";
          await einwilligungsfehler(page, fehler);
          await zeile(page, "Planung Quartal 1").click();
          const karte = page.getByTestId("planung-einwilligungen");
          await expect(karte).toBeVisible();
          if (fehler) {
            await expect(karte.getByRole("alert")).toBeVisible();
            await expect(
              karte.getByRole("button", { name: "Erneut versuchen" }),
            ).toBeVisible();
            await expect(karte.getByTestId("planung-einw-leer")).toHaveCount(0);
            await expect(karte.getByTestId("planung-einw-person")).toHaveCount(0);
          } else {
            await expect(karte.getByTestId("planung-einw-person")).toHaveCount(3);
            await expect(karte).toContainText(
              "1 zugestimmt, 1 offen, 1 abgelehnt",
            );
            expect(
              await karte.locator(".plg-person__name").allTextContents(),
            ).toEqual(["c@example.org", "b@example.org", "a@example.org"]);
            await expect(karte.getByRole("alert")).toHaveCount(0);
          }
        }

        await aufnehmen(page, `${zustand}-${breite}-${farbe}.png`);

        // Gegenproben nach der Aufnahme verändern den dokumentierten Zustand nicht.
        if (zustand === "3-regel-verletzt") {
          await page
            .getByRole("textbox", { name: "Thema von Punkt 1" })
            .fill("Stand");
          await expect(
            page.getByTestId("planung-regel-agenda_punkt_leer"),
          ).toHaveCount(0);
          await expect(page.getByTestId("planung-speichern")).toBeEnabled();
        } else if (zustand === "4-verschoben") {
          await page.getByTestId("planung-verschiebung-bestaetigen").click();
          await expect(
            page
              .getByTestId("planung-termin")
              .getByTestId("planung-status-verknuepft"),
          ).toBeVisible();
          await expect(
            page.getByTestId("planung-verschiebung-bestaetigen"),
          ).toHaveCount(0);
        } else if (zustand === "6-einwilligungen-fehler") {
          await einwilligungsfehler(page, false);
          const karte = page.getByTestId("planung-einwilligungen");
          await karte.getByRole("button", { name: "Erneut versuchen" }).click();
          await expect(karte.getByTestId("planung-einw-person")).toHaveCount(3);
          await expect(karte.getByRole("alert")).toHaveCount(0);
        }
      });
    }
  }
}
