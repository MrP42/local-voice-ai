import { test, expect, type Page } from "@playwright/test";
import { installMock } from "./planungMock";

test.use({ locale: "en-US", timezoneId: "Europe/Berlin" });

async function oeffnen(page: Page) {
  await installMock(page);
  await page.goto("/");
  await page.addStyleTag({
    content:
      "*,*::before,*::after{animation-duration:0s!important;transition-duration:0s!important}",
  });
  await page
    .getByTestId("planung-zeile")
    .filter({ hasText: "Planung Quartal 1" })
    .click();
  await expect(page.getByTestId("planung-einw-person")).toHaveCount(3);
}

const person = (page: Page, adresse: string) =>
  page.getByTestId("planung-einw-person").filter({ hasText: adresse });

const schreibAufrufe = (page: Page) =>
  page.evaluate(() =>
    ((window as any).__calls as [string, any][]).filter(
      ([cmd]) => cmd === "besprechungsplanung_einwilligung_setzen",
    ),
  );

test("Mündliche Zustimmung und Ablehnung haben Zeitpunkt und Kanal", async ({ page }) => {
  await oeffnen(page);
  await person(page, "b@example.org")
    .getByTestId("planung-einw-zugestimmt")
    .click();
  await expect(
    person(page, "b@example.org").getByTestId("planung-einw-zugestimmt"),
  ).toBeDisabled();
  await person(page, "a@example.org")
    .getByTestId("planung-einw-abgelehnt")
    .click();
  await expect.poll(async () => (await schreibAufrufe(page)).length).toBe(2);
  const aufrufe = await schreibAufrufe(page);
  expect(aufrufe[0][1].einwilligung).toMatchObject({
    plan_id: "p_verknuepft",
    person: "b@example.org",
    state: "zugestimmt",
    channel: "muendlich_bei_start",
    recorded_at: expect.any(Number),
  });
  expect(aufrufe[1][1].einwilligung).toMatchObject({
    person: "a@example.org",
    state: "abgelehnt",
    channel: "muendlich_bei_start",
    recorded_at: expect.any(Number),
  });
});

test("Widerruf schreibt offen und lädt den Backend-Stand neu", async ({ page }) => {
  await oeffnen(page);
  await person(page, "a@example.org").getByTestId("planung-einw-offen").click();
  await expect(
    person(page, "a@example.org").getByTestId("planung-einw-offen"),
  ).toBeDisabled();
  const aufrufe = await schreibAufrufe(page);
  expect(aufrufe).toHaveLength(1);
  expect(aufrufe[0][1].einwilligung).toMatchObject({
    plan_id: "p_verknuepft",
    person: "a@example.org",
    state: "offen",
  });
  // Gegenprobe: Ohne den Schreibaufruf bleibt die Zustimmung bestehen und dies wird rot.
  await expect(person(page, "a@example.org")).toHaveClass(/plg-person--offen/);
});

test("Ein fehlgeschlagener Nachweis gibt keine Zustimmung im Client frei", async ({ page }) => {
  await oeffnen(page);
  await page.evaluate(() => {
    (window as any).__steuerung.besprechungsplanung_einwilligung_setzen = {
      fehler: true,
    };
  });
  await person(page, "b@example.org")
    .getByTestId("planung-einw-zugestimmt")
    .click();
  await expect(page.getByRole("alert")).toContainText(
    "Die Entscheidung konnte nicht gespeichert werden.",
  );
  await expect(person(page, "b@example.org")).toHaveClass(/plg-person--offen/);
});

test("Nachweis bleibt bis zum erneuten Backend-Abruf gegen Doppelklick gesperrt", async ({
  page,
}) => {
  await oeffnen(page);
  await page.evaluate(() => {
    (window as any).__steuerung.besprechungsplanung_einwilligungen = { pause: true };
  });
  await person(page, "b@example.org")
    .getByTestId("planung-einw-zugestimmt")
    .click();
  await expect(page.getByTestId("planung-einw-person")).toHaveCount(0);
  expect(await schreibAufrufe(page)).toHaveLength(1);
  await page.evaluate(() => {
    const w = window as any;
    w.__steuerung.besprechungsplanung_einwilligungen = {};
    const freigaben = w.__freigaben.besprechungsplanung_einwilligungen ?? [];
    w.__freigaben.besprechungsplanung_einwilligungen = [];
    freigaben.forEach((resolve: () => void) => resolve());
  });
  await expect(
    person(page, "b@example.org").getByTestId("planung-einw-zugestimmt"),
  ).toBeDisabled();
});

test("Zeitboxen zeigen Überzug und behalten ihn beim Zurückwechseln", async ({ page }) => {
  const zeit = new Date("2026-11-20T08:00:00Z");
  await page.clock.install({ time: zeit });
  await page.clock.pauseAt(new Date(zeit.getTime() + 1000));
  await oeffnen(page);
  const boxen = page.getByTestId("planung-zeitboxen");
  const punkte = boxen.getByTestId("planung-zeitbox");
  await expect(boxen.getByTestId("planung-zeitbox-zurueck")).toBeDisabled();
  await boxen.getByTestId("planung-zeitbox-start").click();
  await page.clock.runFor(301_000);
  // Gegenprobe: Kappen bei null oder Zurücksetzen beim Wechsel macht diese Prüfung rot.
  await expect(punkte.nth(0)).toHaveAttribute("data-ueberzug", "true");
  await expect(punkte.nth(0)).toContainText("0:01");
  await boxen.getByTestId("planung-zeitbox-weiter").click();
  await expect(boxen.getByTestId("planung-zeitbox-weiter")).toBeDisabled();
  await boxen.getByTestId("planung-zeitbox-zurueck").click();
  await expect(punkte.nth(0)).toHaveAttribute("data-ueberzug", "true");
  await expect(punkte.nth(0)).toContainText("0:01");
});
