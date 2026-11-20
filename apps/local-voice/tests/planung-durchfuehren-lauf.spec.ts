import { test, expect, type Page } from "@playwright/test";
import { installMock } from "./planungMock";
import { ergaenztVon } from "./planungMock";

test.use({ locale: "en-US", timezoneId: "Europe/Berlin" });

const aufrufe = (page: Page): Promise<[string, any][]> =>
  page.evaluate(() => (window as any).__calls);

const laufPerson = (page: Page, adresse: string) =>
  page.getByTestId("planung-lauf-person").filter({ hasText: adresse });

const einwilligung = (page: Page, adresse: string) =>
  page.getByTestId("planung-einw-person").filter({ hasText: adresse });

const modus = (page: Page) => page.getByTestId("planung-lauf-modus");
const sperre = (page: Page) => page.getByTestId("planung-lauf-gesperrt");

async function anzahlPruefungen(page: Page) {
  return (await aufrufe(page)).filter(
    ([cmd]) => cmd === "besprechungsplanung_start_modus",
  ).length;
}

async function oeffnen(page: Page, spaetererNachweis = false) {
  await installMock(page);
  await page.goto("/");
  await page.addStyleTag({
    content:
      "*,*::before,*::after{animation-duration:0s!important;transition-duration:0s!important}",
  });
  if (spaetererNachweis) {
    // Die Karte kann nur bestehende Personen bearbeiten; D ist noch nicht anwesend.
    await page.evaluate(() => {
      (window as any).__daten.consents.p_verknuepft.push({
        plan_id: "p_verknuepft",
        person: "d@example.org",
        state: "offen",
        channel: null,
        recorded_at: null,
      });
    });
  }
  await page
    .getByTestId("planung-zeile")
    .filter({ hasText: "Planung Quartal 1" })
    .click();
  await expect(page.getByTestId("planung-durchfuehren-start")).toBeEnabled();
  await expect
    .poll(async () =>
      (await aufrufe(page)).some(([cmd]) => cmd === "calendar_upcoming"),
    )
    .toBe(true);
}

async function startVorbereiten(page: Page) {
  await page.getByTestId("planung-durchfuehren-start").click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(page.getByTestId("planung-start-person")).toHaveCount(3);
  for (const adresse of ["b@example.org", "c@example.org"]) {
    const knopf = page
      .getByTestId("planung-start-person")
      .filter({ hasText: adresse })
      .getByTestId("planung-start-zustimmen");
    await knopf.click();
    await expect(knopf).toHaveCount(0);
  }
  await expect(page.getByTestId("planung-start-los")).toBeEnabled();
}

async function startMitTranskription(page: Page) {
  await oeffnen(page);
  await startVorbereiten(page);
  await page.getByTestId("planung-start-los").click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(modus(page)).toHaveAttribute("data-modus", "transkription");
  await expect(page.getByTestId("planung-verlauf-eintrag")).toHaveCount(4);
}

async function hinzufuegen(page: Page, adresse = "d@example.org") {
  await page.getByTestId("planung-lauf-hinzu-eingabe").fill(adresse);
  await page.getByTestId("planung-lauf-hinzu").click();
}

test("Zeitboxen bleiben im Lauf einmal sichtbar und bedienbar", async ({
  page,
}) => {
  await startMitTranskription(page);
  const zeitboxen = page.getByTestId("planung-zeitboxen");
  await expect(zeitboxen).toHaveCount(1);
  await expect(zeitboxen).toBeVisible();
  await expect(
    page.getByTestId("planung-durchfuehren").getByTestId("planung-zeitboxen"),
  ).toHaveCount(1);
  await zeitboxen.getByTestId("planung-zeitbox-start").click();
  const aktuell = zeitboxen.locator('[aria-current="step"]');
  await expect(aktuell).toHaveCount(1);
  await expect(aktuell).toContainText("Stand");
  await zeitboxen.getByTestId("planung-zeitbox-weiter").click();
  await expect(aktuell).toHaveCount(1);
  await expect(aktuell).toContainText("Optionen");
  await zeitboxen.getByRole("button", { name: /zurück/i }).click();
  await expect(aktuell).toContainText("Stand");
});

test("Widerruf sperrt Transkription und erhält frühere Nachweise", async ({
  page,
}) => {
  await startMitTranskription(page);
  const verlauf = page.getByTestId("planung-verlauf-eintrag");
  const vorher = await verlauf.allTextContents();
  const pruefungen = await anzahlPruefungen(page);
  await einwilligung(page, "b@example.org")
    .getByTestId("planung-einw-offen")
    .click();
  await expect(modus(page)).toHaveAttribute("data-modus", "notizen");
  await expect(sperre(page)).toBeVisible();
  await expect(sperre(page)).toContainText("b@example.org");
  await expect(sperre(page)).toContainText(/Zustimmung fehlt/i);
  await expect.poll(() => anzahlPruefungen(page)).toBeGreaterThan(pruefungen);
  await expect(verlauf).toHaveCount(vorher.length + 1);
  await expect(verlauf.first()).toContainText("b@example.org");
  await expect(verlauf.first()).toContainText(/offen/i);
  await expect(
    verlauf
      .filter({ hasText: "b@example.org" })
      .filter({ hasText: /zugestimmt/i }),
  ).toHaveCount(1);
  // Auch die übrigen Nachweise müssen unverändert hinter dem Widerruf stehen.
  expect((await verlauf.allTextContents()).slice(1)).toEqual(vorher);
});

test("Späte Person sperrt bis zum mündlichen Nachweis", async ({ page }) => {
  await startMitTranskription(page);
  const verlauf = page.getByTestId("planung-verlauf-eintrag");
  const vorher = await verlauf.count();
  await hinzufuegen(page);
  await expect(page.getByTestId("planung-lauf-anwesende")).toContainText(
    "d@example.org",
  );
  await expect(modus(page)).toHaveAttribute("data-modus", "notizen");
  await expect(sperre(page)).toBeVisible();
  await expect(sperre(page)).toContainText("d@example.org");
  await laufPerson(page, "d@example.org")
    .getByTestId("planung-lauf-zustimmen")
    .click();
  await expect(modus(page)).toHaveAttribute("data-modus", "transkription");
  await expect(sperre(page)).toHaveCount(0);
  const schreiben = (await aufrufe(page)).filter(
    ([cmd, args]) =>
      cmd === "besprechungsplanung_einwilligung_setzen" &&
      args.einwilligung.person === "d@example.org",
  );
  expect(schreiben).toHaveLength(1);
  expect(schreiben[0][1].einwilligung).toMatchObject({
    plan_id: "p_verknuepft",
    person: "d@example.org",
    state: "zugestimmt",
    channel: "muendlich_bei_start",
  });
  await expect(verlauf).toHaveCount(vorher + 1);
  await expect(verlauf.first()).toContainText("d@example.org");
  await expect(verlauf.first()).toContainText(/zugestimmt/i);
});

test("Ungültige und doppelte Personen verändern die Liste nicht", async ({
  page,
}) => {
  await startMitTranskription(page);
  const personen = page.getByTestId("planung-lauf-person");
  const vorher = await personen.allTextContents();
  const meldung = page.getByTestId("planung-lauf").getByRole("alert");
  await hinzufuegen(page, "kein-at");
  await expect(meldung).toBeVisible();
  await expect(page.getByTestId("planung-lauf-hinzu-eingabe")).toHaveAttribute(
    "aria-invalid",
    "true",
  );
  await expect(personen).toHaveCount(vorher.length);
  expect(await personen.allTextContents()).toEqual(vorher);
  await hinzufuegen(page, "a@example.org");
  await expect(meldung).toContainText(/bereits anwesend/i);
  await expect(personen).toHaveCount(vorher.length);
  expect(await personen.allTextContents()).toEqual(vorher);
});

test("Notizlauf bleibt trotz vollständiger Zustimmung ein Notizlauf", async ({
  page,
}) => {
  await oeffnen(page, true);
  // Auch B und C stimmen zu: Das Starttor könnte anschließend Transkription freigeben.
  await startVorbereiten(page);
  await page.getByTestId("planung-start-notizen").click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(modus(page)).toHaveAttribute("data-modus", "notizen");
  await expect(sperre(page)).toHaveCount(0);
  await expect(page.getByTestId("planung-lauf-zustimmen")).toHaveCount(0);
  const zustimmung = einwilligung(page, "d@example.org").getByTestId(
    "planung-einw-zugestimmt",
  );
  await zustimmung.click();
  await expect(zustimmung).toBeDisabled();
  const pruefungen = await anzahlPruefungen(page);
  await hinzufuegen(page);
  await expect(laufPerson(page, "d@example.org")).toContainText(/zugestimmt/i);
  await expect.poll(() => anzahlPruefungen(page)).toBeGreaterThan(pruefungen);
  await expect(modus(page)).toHaveAttribute("data-modus", "notizen");
  await expect(modus(page)).toContainText(/nur mit Notizen/i);
  await expect(sperre(page)).toHaveCount(0);
  await expect(page.getByTestId("planung-lauf-zustimmen")).toHaveCount(0);
});

test("Entfernen der späten Person hebt die Sperre auf", async ({ page }) => {
  await startMitTranskription(page);
  await hinzufuegen(page);
  await expect(sperre(page)).toContainText("d@example.org");
  await expect(modus(page)).toHaveAttribute("data-modus", "notizen");
  await laufPerson(page, "d@example.org")
    .getByTestId("planung-lauf-entfernen")
    .click();
  await expect(laufPerson(page, "d@example.org")).toHaveCount(0);
  await expect(page.getByTestId("planung-lauf-person")).toHaveCount(3);
  await expect(modus(page)).toHaveAttribute("data-modus", "transkription");
  await expect(sperre(page)).toHaveCount(0);
});

test("Anzeige schließen zeigt wieder den Start", async ({ page }) => {
  await startMitTranskription(page);
  await page.getByTestId("planung-durchfuehren-schliessen").click();
  await expect(page.getByTestId("planung-durchfuehren-start")).toBeVisible();
  await expect(page.getByTestId("planung-durchfuehren-start")).toBeEnabled();
  await expect(modus(page)).toHaveCount(0);
});

test("Späte Person wird vor der Anzeige gemeldet", async ({ page }) => {
  await startMitTranskription(page);
  await page.evaluate(() => {
    (window as any).__steuerung.besprechungsplanung_anwesende_ergaenzen = {
      pause: true,
    };
  });
  await hinzufuegen(page);
  await expect
    .poll(
      async () =>
        (await aufrufe(page)).filter(
          ([cmd]) => cmd === "besprechungsplanung_anwesende_ergaenzen",
        ).length,
    )
    .toBe(1);
  expect(
    (await aufrufe(page)).filter(
      ([cmd]) => cmd === "besprechungsplanung_anwesende_ergaenzen",
    )[0][1],
  ).toEqual({
    meetingId: "m_aufnahme",
    id: "p_verknuepft",
    personen: ["d@example.org"],
  });
  await expect(page.getByTestId("planung-lauf-anwesende")).not.toContainText(
    "d@example.org",
  );
  expect(await ergaenztVon(page, "m_aufnahme")).toEqual([]);
  await page.evaluate(() => {
    (
      window as any
    ).__freigaben.besprechungsplanung_anwesende_ergaenzen.shift()();
  });
  await expect(page.getByTestId("planung-lauf-anwesende")).toContainText(
    "d@example.org",
  );
  expect(await ergaenztVon(page, "m_aufnahme")).toEqual(["d@example.org"]);
});

test("Scheiternde Meldung übernimmt die späte Person nicht", async ({
  page,
}) => {
  await startMitTranskription(page);
  await page.evaluate(() => {
    (window as any).__steuerung.besprechungsplanung_anwesende_ergaenzen = {
      fehler: true,
    };
  });
  await hinzufuegen(page);
  await expect(page.getByTestId("planung-lauf-ergaenzen-fehler")).toBeVisible();
  await expect(page.getByTestId("planung-lauf-anwesende")).not.toContainText(
    "d@example.org",
  );
  await expect(page.getByTestId("planung-lauf-person")).toHaveCount(3);
  await expect(modus(page)).toHaveAttribute("data-modus", "transkription");
  expect(await ergaenztVon(page, "m_aufnahme")).toEqual([]);
  await page.evaluate(() => {
    (window as any).__steuerung.besprechungsplanung_anwesende_ergaenzen = {};
  });
  await hinzufuegen(page);
  await expect(page.getByTestId("planung-lauf-anwesende")).toContainText(
    "d@example.org",
  );
  await expect(page.getByTestId("planung-lauf-ergaenzen-fehler")).toHaveCount(
    0,
  );
  expect(await ergaenztVon(page, "m_aufnahme")).toEqual(["d@example.org"]);
});

test("Personen-Ids behalten ihre Groß- und Kleinschreibung", async ({
  page,
}) => {
  await startMitTranskription(page);
  await hinzufuegen(page, "id:ABC");
  const genau = (wert: RegExp) =>
    page.getByTestId("planung-lauf-person").filter({ hasText: wert });
  await expect(genau(/id:ABC/)).toHaveCount(1);
  await hinzufuegen(page, "id:abc");
  await expect(genau(/id:abc/)).toHaveCount(1);
  await expect(genau(/id:ABC/)).toHaveCount(1);
  await expect(page.getByTestId("planung-lauf-person")).toHaveCount(5);
  expect(
    (await aufrufe(page))
      .filter(([cmd]) => cmd === "besprechungsplanung_anwesende_ergaenzen")
      .map(([, args]) => args),
  ).toEqual([
    { meetingId: "m_aufnahme", id: "p_verknuepft", personen: ["id:ABC"] },
    { meetingId: "m_aufnahme", id: "p_verknuepft", personen: ["id:abc"] },
  ]);
  expect(await ergaenztVon(page, "m_aufnahme")).toEqual(["id:ABC", "id:abc"]);
});
