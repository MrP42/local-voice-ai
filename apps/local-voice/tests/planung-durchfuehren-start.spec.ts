import { test, expect, type Page } from "@playwright/test";
import { installMock, setzeTermin } from "./planungMock";
import { ergaenztVon } from "./planungMock";

test.use({ locale: "en-US", timezoneId: "Europe/Berlin" });

const aufrufe = (page: Page): Promise<[string, any][]> =>
  page.evaluate(() => (window as any).__calls);

const person = (page: Page, adresse: string) =>
  page.getByTestId("planung-start-person").filter({ hasText: adresse });

async function oeffnen(
  page: Page,
  titel = "Planung Quartal 1",
  opts: { notizen?: boolean } = {},
) {
  await installMock(page, opts);
  await page.goto("/");
  await page.addStyleTag({
    content:
      "*,*::before,*::after{animation-duration:0s!important;transition-duration:0s!important}",
  });
  await page.getByTestId("planung-zeile").filter({ hasText: titel }).click();
  await expect(page.getByTestId("planung-durchfuehren-start")).toBeEnabled();
  // Der Terminabruf muss vor dem Öffnen abgeschlossen sein.
  if (titel !== "Jour fixe Betrieb") {
    await expect
      .poll(async () =>
        (await aufrufe(page)).some(([cmd]) => cmd === "calendar_upcoming"),
      )
      .toBe(true);
  }
}

async function startOeffnen(page: Page, anzahl = 3) {
  await page.getByTestId("planung-durchfuehren-start").click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(page.getByTestId("planung-start-person")).toHaveCount(anzahl);
  await expect(page.getByTestId("planung-start-notizen")).toBeEnabled();
}

async function zustimmen(page: Page) {
  // A hat bereits zugestimmt; B und C erhalten den mündlichen Nachweis.
  for (const adresse of ["b@example.org", "c@example.org"]) {
    const knopf = person(page, adresse).getByTestId("planung-start-zustimmen");
    await knopf.click();
    await expect(knopf).toHaveCount(0);
  }
  await expect(page.getByTestId("planung-start-los")).toBeEnabled();
}

test("Start mit Transkription schreibt Nachweise vor Prüfung und Aufnahme", async ({
  page,
}) => {
  await oeffnen(page);
  await expect(page.getByTestId("planung-verlauf-eintrag")).toHaveCount(2);
  await startOeffnen(page);
  await zustimmen(page);
  await page.getByTestId("planung-start-los").click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.getByTestId("planung-lauf-modus")).toHaveAttribute(
    "data-modus",
    "transkription",
  );

  const calls = await aufrufe(page);
  const schreiben = calls.flatMap(([cmd, args], index) =>
    cmd === "besprechungsplanung_einwilligung_setzen" ? [{ args, index }] : [],
  );
  expect(schreiben).toHaveLength(2);
  for (const [index, adresse] of ["b@example.org", "c@example.org"].entries()) {
    expect(schreiben[index].args.einwilligung).toMatchObject({
      plan_id: "p_verknuepft",
      person: adresse,
      state: "zugestimmt",
      channel: "muendlich_bei_start",
      recorded_at: expect.any(Number),
    });
  }
  const pruefen = calls.findIndex(
    ([cmd]) => cmd === "besprechungsplanung_start_modus",
  );
  const starten = calls.findIndex(
    ([cmd]) => cmd === "meetings_start_from_event",
  );
  expect(pruefen).toBeGreaterThan(schreiben[1].index);
  expect(starten).toBeGreaterThan(pruefen);
  expect(calls[pruefen][1]).toMatchObject({
    id: "p_verknuepft",
    anwesende: ["a@example.org", "b@example.org", "c@example.org"],
  });
  expect(calls[starten][1]).toMatchObject({
    eventKey: "src1:p_verknuepft",
    consentConfirmed: true,
    targetMeetingId: null,
  });
  const verlauf = page.getByTestId("planung-verlauf-eintrag");
  await expect(verlauf).toHaveCount(4);
  await expect(verlauf.nth(0)).toContainText("c@example.org");
  await expect(verlauf.nth(1)).toContainText("b@example.org");
  await expect(verlauf.nth(2)).toContainText("a@example.org");
});

test("Nicht geplante Transkription lässt ausschließlich den Notizenstart zu", async ({
  page,
}) => {
  await oeffnen(page, "Jour fixe Notizen", { notizen: true });
  await startOeffnen(page, 1);
  await expect(page.getByTestId("planung-start-los")).toBeDisabled();
  // Die Hinweiskarte besitzt keine eigene Test-ID.
  await expect(page.getByRole("dialog")).toContainText(/nicht vorgesehen/i);
  await page.getByTestId("planung-start-notizen").click();
  await expect(page.getByTestId("planung-lauf-modus")).toHaveAttribute(
    "data-modus",
    "notizen",
  );
  const calls = await aufrufe(page);
  const notizen = calls.filter(
    ([cmd]) => cmd === "besprechungsplanung_notizen_starten",
  );
  expect(notizen).toHaveLength(1);
  expect(notizen[0][1]).toMatchObject({
    id: "p_notizen",
    title: expect.stringMatching(/\S/),
  });
  expect(
    calls.filter(([cmd]) => cmd === "meetings_start_from_event"),
  ).toHaveLength(0);
});

test("Offene, abgelehnte und unbekannte Personen verlangen einen bewussten Notizenstart", async ({
  page,
}) => {
  await installMock(page);
  await page.goto("/");
  await page.addStyleTag({
    content:
      "*,*::before,*::after{animation-duration:0s!important;transition-duration:0s!important}",
  });
  // Vor der Auswahl ändern: PlanungDetail liest den Kalender beim Einhängen.
  await setzeTermin(page, "src1:p_verknuepft", [
    ...["a@example.org", "b@example.org", "c@example.org"].map((email) => ({
      email,
      name: null,
      organizer: false,
      is_self: false,
      partstat: null,
    })),
    {
      email: null,
      name: "Gast",
      organizer: false,
      is_self: false,
      partstat: null,
    },
  ]);
  await page
    .getByTestId("planung-zeile")
    .filter({ hasText: "Planung Quartal 1" })
    .click();
  await startOeffnen(page, 4);
  await expect(
    person(page, "Gast").getByTestId("planung-start-zustimmen"),
  ).toHaveCount(0);
  await page.getByTestId("planung-start-los").click();
  const gruende = page.getByTestId("planung-start-gruende");
  await expect(gruende).toContainText("b@example.org");
  await expect(gruende).toContainText("c@example.org");
  await expect(gruende).toContainText(/Zustimmung fehlt/i);
  await expect(gruende).toContainText(/abgelehnt/i);
  await expect(gruende).toContainText("1");
  const vorher = await aufrufe(page);
  expect(
    vorher.filter(([cmd]) => cmd === "meetings_start_from_event"),
  ).toHaveLength(0);
  expect(
    vorher.filter(([cmd]) => cmd === "besprechungsplanung_notizen_starten"),
  ).toHaveLength(0);
  await page.getByTestId("planung-start-notizen").click();
  await expect(page.getByTestId("planung-lauf-modus")).toHaveAttribute(
    "data-modus",
    "notizen",
  );
  const nachher = await aufrufe(page);
  expect(
    nachher.filter(([cmd]) => cmd === "besprechungsplanung_notizen_starten"),
  ).toHaveLength(1);
  expect(
    nachher.filter(([cmd]) => cmd === "meetings_start_from_event"),
  ).toHaveLength(0);
});

test("Das Backend-Tor bleibt bei einem zwischenzeitlichen Widerruf die letzte Instanz", async ({
  page,
}) => {
  await oeffnen(page);
  await startOeffnen(page);
  await zustimmen(page);
  await page.evaluate(() => {
    (window as any).__steuerung.meetings_start_from_event = {
      fehlerText: "nur_notizen: abgelehnt",
    };
  });
  await page.getByTestId("planung-start-los").click();
  const fehler = page.getByTestId("planung-start-fehler");
  await expect(fehler).toBeVisible();
  await expect(fehler).not.toContainText(/nur_notizen|meeting/i);
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(page.getByTestId("planung-start-los")).toBeEnabled();
  await expect(page.getByTestId("planung-lauf-modus")).toHaveCount(0);
  const calls = await aufrufe(page);
  expect(
    calls.filter(([cmd]) => cmd === "meetings_start_from_event"),
  ).toHaveLength(1);
  expect(
    calls.filter(([cmd]) => cmd === "besprechungsplanung_notizen_starten"),
  ).toHaveLength(0);
});

test("Ohne Termin bleibt Transkription gesperrt und Notizen bleiben möglich", async ({
  page,
}) => {
  await oeffnen(page, "Jour fixe Betrieb");
  await startOeffnen(page, 0);
  await expect(page.getByTestId("planung-start-los")).toBeDisabled();
  // StartPanel hat keine Karten-Test-ID; der Hinweis steht im Startdialog.
  await expect(page.getByRole("dialog")).toContainText("Termin");
  await expect(page.getByTestId("planung-start-notizen")).toBeEnabled();
  await page.getByTestId("planung-start-notizen").click();
  await expect(page.getByTestId("planung-lauf-modus")).toHaveAttribute(
    "data-modus",
    "notizen",
  );
  const calls = await aufrufe(page);
  expect(
    calls.filter(([cmd]) => cmd === "meetings_start_from_event"),
  ).toHaveLength(0);
  expect(
    calls.filter(([cmd]) => cmd === "besprechungsplanung_notizen_starten"),
  ).toHaveLength(1);
});

test("Mehrfachklick startet während einer ausstehenden Prüfung nur eine Aufnahme", async ({
  page,
}) => {
  await oeffnen(page);
  await startOeffnen(page);
  await zustimmen(page);
  await page.evaluate(() => {
    (window as any).__steuerung.besprechungsplanung_start_modus = {
      pause: true,
    };
  });
  const los = page.getByTestId("planung-start-los");
  await los.click();
  await expect(los).toBeDisabled();
  await los.click({ force: true });
  const wartend = await aufrufe(page);
  expect(
    wartend.filter(([cmd]) => cmd === "besprechungsplanung_start_modus"),
  ).toHaveLength(1);
  expect(
    wartend.filter(([cmd]) => cmd === "meetings_start_from_event"),
  ).toHaveLength(0);
  await page.evaluate(() => {
    const w = window as any;
    w.__steuerung.besprechungsplanung_start_modus = {};
    const freigaben = w.__freigaben.besprechungsplanung_start_modus ?? [];
    w.__freigaben.besprechungsplanung_start_modus = [];
    freigaben.forEach((resolve: () => void) => resolve());
  });
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.getByTestId("planung-lauf-modus")).toHaveAttribute(
    "data-modus",
    "transkription",
  );
  expect(
    (await aufrufe(page)).filter(
      ([cmd]) => cmd === "meetings_start_from_event",
    ),
  ).toHaveLength(1);
});

async function startMitZusatzVorbereiten(page: Page) {
  await oeffnen(page);
  await startOeffnen(page);
  await page.getByTestId("planung-start-hinzu-eingabe").fill("d@example.org");
  await page.getByTestId("planung-start-hinzu").click();
  for (const adresse of ["b@example.org", "c@example.org", "d@example.org"]) {
    const knopf = person(page, adresse).getByTestId("planung-start-zustimmen");
    await knopf.click();
    await expect(knopf).toHaveCount(0);
  }
}

test("Startfeld meldet ergänzte Person vor dem Lauf", async ({ page }) => {
  await startMitZusatzVorbereiten(page);
  await page.evaluate(() => {
    (window as any).__steuerung.besprechungsplanung_anwesende_ergaenzen = {
      pause: true,
    };
  });
  await page.getByTestId("planung-start-los").click();
  await expect
    .poll(
      async () =>
        (await aufrufe(page)).filter(
          ([cmd]) => cmd === "besprechungsplanung_anwesende_ergaenzen",
        ).length,
    )
    .toBe(1);
  const calls = await aufrufe(page);
  const start = calls.findIndex(([cmd]) => cmd === "meetings_start_from_event");
  const meldung = calls.findIndex(
    ([cmd]) => cmd === "besprechungsplanung_anwesende_ergaenzen",
  );
  expect(start).toBeGreaterThanOrEqual(0);
  expect(meldung).toBeGreaterThan(start);
  expect(calls[meldung][1]).toEqual({
    meetingId: "m_aufnahme",
    id: "p_verknuepft",
    personen: ["d@example.org"],
  });
  await expect(page.getByTestId("planung-lauf-modus")).toHaveCount(0);
  expect(await ergaenztVon(page, "m_aufnahme")).toEqual([]);
  await page.evaluate(() => {
    const w = window as any;
    w.__freigaben.besprechungsplanung_anwesende_ergaenzen.shift()();
  });
  await expect(page.getByTestId("planung-lauf-modus")).toBeVisible();
  expect(await ergaenztVon(page, "m_aufnahme")).toEqual(["d@example.org"]);
});

test("Gescheiterte Startmeldung wiederholt nur die Meldung", async ({
  page,
}) => {
  await startMitZusatzVorbereiten(page);
  await page.evaluate(() => {
    (window as any).__steuerung.besprechungsplanung_anwesende_ergaenzen = {
      fehler: true,
    };
  });
  await page.getByTestId("planung-start-los").click();
  await expect(
    page.getByTestId("planung-start-ergaenzen-fehler"),
  ).toBeVisible();
  await expect(page.getByTestId("planung-lauf-modus")).toHaveCount(0);
  await expect(page.getByTestId("planung-start-los")).toBeDisabled();
  await expect(page.getByTestId("planung-start-notizen")).toBeDisabled();
  expect(
    (await aufrufe(page)).filter(
      ([cmd]) => cmd === "meetings_start_from_event",
    ),
  ).toHaveLength(1);
  expect(await ergaenztVon(page, "m_aufnahme")).toEqual([]);
  await page.evaluate(() => {
    (window as any).__steuerung.besprechungsplanung_anwesende_ergaenzen = {};
  });
  await page.getByTestId("planung-start-erneut-melden").click();
  await expect(page.getByTestId("planung-lauf-modus")).toBeVisible();
  expect(
    (await aufrufe(page)).filter(
      ([cmd]) => cmd === "meetings_start_from_event",
    ),
  ).toHaveLength(1);
  expect(
    (await aufrufe(page)).filter(
      ([cmd]) => cmd === "besprechungsplanung_anwesende_ergaenzen",
    ),
  ).toHaveLength(2);
  expect(await ergaenztVon(page, "m_aufnahme")).toEqual(["d@example.org"]);
});

test("Ohne Zusatzperson erfolgt keine Meldung", async ({ page }) => {
  await oeffnen(page);
  await startOeffnen(page);
  await zustimmen(page);
  await page.getByTestId("planung-start-los").click();
  await expect(page.getByTestId("planung-lauf-modus")).toBeVisible();
  expect(
    (await aufrufe(page)).filter(
      ([cmd]) => cmd === "besprechungsplanung_anwesende_ergaenzen",
    ),
  ).toHaveLength(0);
});
