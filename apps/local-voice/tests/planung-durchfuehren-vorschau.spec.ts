import { mkdir } from "node:fs/promises";
import { resolve } from "node:path";
import { test, expect, type Page } from "@playwright/test";
import { installMock } from "./planungMock";
import { calEvent, installTauriMock } from "./calendarMock";

// Vorschau P4 (Durchführen): Aufnahmen nur mit PLANUNG_P4_SHOTS=1, sonst laufen dieselben Abläufe
// als Rauchtest. Nur synthetische Daten (example.org).
test.use({ locale: "en-US", timezoneId: "Europe/Berlin" });

const AUFNAHMEN = process.env.PLANUNG_P4_SHOTS === "1";

async function aufnehmen(page: Page, dateiname: string) {
  if (!AUFNAHMEN) return;
  const verzeichnis = resolve(
    process.env.LV_SHOT_DIR ?? "docs/vorschau/p4-besprechungsplanung",
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

const hoehe = (breite: number) =>
  AUFNAHMEN ? (breite === 390 ? 2600 : 1500) : breite === 390 ? 844 : 900;

for (const theme of ["light", "dark"] as const) {
  for (const breite of [390, 1440]) {
    const farbe = theme === "light" ? "hell" : "dunkel";

    test(`planung-start-lauf-${breite}-${farbe}`, async ({ page }) => {
      await installMock(page, { theme });
      await page.setViewportSize({ width: breite, height: hoehe(breite) });
      await page.goto("/");
      await expect(page.getByTestId("planung-seite")).toBeVisible();
      if (breite === 390) {
        await page
          .getByRole("button", { name: /Men(?:ü|ue) (?:ö|oe)ffnen/ })
          .click();
        await page.keyboard.press("Escape");
      }
      await page
        .getByTestId("planung-zeile")
        .filter({ hasText: "Planung Quartal 1" })
        .click();
      // Einwilligungskarte mit Verlauf.
      await expect(page.getByTestId("planung-verlauf-eintrag")).toHaveCount(2);
      // lva#844: Einwilligungen und Verlauf sind eine Aufklappkarte.
      await page.getByRole("button", { name: /Einwilligungen/ }).click();
      await page.getByTestId("planung-einw-verlauf").scrollIntoViewIfNeeded();
      await aufnehmen(page, `1-einwilligungen-verlauf-${breite}-${farbe}.png`);

      // Startdialog.
      await page.getByTestId("planung-durchfuehren-start").click();
      await expect(page.getByTestId("planung-start-person")).toHaveCount(3);
      await aufnehmen(page, `2-startdialog-${breite}-${farbe}.png`);
      for (const adresse of ["b@example.org", "c@example.org"]) {
        const knopf = page
          .getByTestId("planung-start-person")
          .filter({ hasText: adresse })
          .getByTestId("planung-start-zustimmen");
        await knopf.click();
        await expect(knopf).toHaveCount(0);
      }
      await page.getByTestId("planung-start-los").click();

      // Lauf mit Zeitboxen.
      await expect(page.getByTestId("planung-lauf-modus")).toHaveAttribute(
        "data-modus",
        "transkription",
      );
      await page.getByTestId("planung-zeitbox-start").click();
      await page.getByTestId("planung-durchfuehren").scrollIntoViewIfNeeded();
      await aufnehmen(page, `3-lauf-zeitboxen-${breite}-${farbe}.png`);
    });

    test(`hinweisfenster-mit-planung-${breite}-${farbe}`, async ({ page }) => {
      const start = Date.parse("2026-09-29T09:00:00+02:00");
      const plan = {
        id: "pl1",
        title: "Abstimmung Pilotstart",
        goal_type: "entscheiden",
        goal_text: "Wir entscheiden über den Pilotstart.",
        expected_result: "Entscheidung mit Termin",
        duration_min: 25,
        agenda: [
          { minutes: 5, topic: "Stand", owner: "A. Beispiel" },
          { minutes: 15, topic: "Optionen", owner: null },
        ],
        roles: {
          moderation: "A. Beispiel",
          ergebnissicherung: null,
          entscheidung: null,
        },
        transcription_planned: true,
      };
      const person = (email: string | null, name: string | null) => ({
        email,
        name,
        organizer: false,
        is_self: false,
        partstat: null,
      });
      const consent = (p: string, state: string) => ({
        plan_id: "pl1",
        person: p,
        state,
        channel: null,
        recorded_at: null,
      });
      await page.clock.setFixedTime(start - 60_000);
      await installTauriMock(page, "meeting_prompt");
      await page.addInitScript(
        (p) => {
          (window as any).__prompt = p;
        },
        {
          prompt_id: "p1",
          kind: "reminder",
          event: calEvent({
            starts_at: start,
            ends_at: start + 3_600_000,
            attendees: [
              person("a@example.org", null),
              person("b@example.org", null),
              person(null, "Gast"),
            ],
          }),
          attendee_count: 3,
          app_label: null,
          plaene: [
            {
              plan,
              einwilligungen: [
                consent("a@example.org", "zugestimmt"),
                consent("b@example.org", "offen"),
              ],
            },
          ],
        },
      );
      await page.emulateMedia({ colorScheme: theme });
      await page.setViewportSize({
        width: 380,
        height: breite === 390 ? 700 : 760,
      });
      await page.goto("/src/meeting-prompt/index.html");
      await expect(page.getByTestId("prompt-plan")).toBeVisible();
      await expect(page.getByTestId("prompt-plan-person")).toHaveCount(3);
      await aufnehmen(page, `4-hinweisfenster-${breite}-${farbe}.png`);
    });
  }
}
