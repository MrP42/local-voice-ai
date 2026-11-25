import { test, expect, type Page } from "@playwright/test";
import { installMock } from "./planungMock";

// P5 (lva#736): Protokoll aus der Agenda, "Ziel erreicht", Freigabe, Entwurf an Teilnehmende.
// Ohne Browser-Backend: die Tauri-Attrappe bildet die Befehle nach, die Seite zeigt nur, was die
// Attrappe zurückgibt. Nur synthetische Daten (example.org).

test.use({ locale: "en-US", timezoneId: "Europe/Berlin" });

const aufrufe = (page: Page): Promise<[string, any][]> =>
  page.evaluate(() => (window as any).__calls);

const befehle = async (page: Page, name: string) =>
  (await aufrufe(page)).filter(([cmd]) => cmd === name);

async function oeffnen(page: Page, titel: string) {
  await installMock(page, { protokoll: true });
  await page.goto("/");
  await page.addStyleTag({
    content:
      "*,*::before,*::after{animation-duration:0s!important;transition-duration:0s!important}",
  });
  // Durchgeführte und protokollierte Planungen stehen im Archiv; „Alle“ zeigt jede.
  await page.getByRole("tab", { name: /^(Alle|All)( \d+)?$/ }).click();
  await page.getByTestId("planung-zeile").filter({ hasText: titel }).click();
  // lva#844: Die Protokollkarte steht im Register „Protokoll“.
  await page.getByRole("tab", { name: /^Protokoll/ }).click();
  await expect(page.getByTestId("planung-protokoll")).toBeVisible();
  await expect
    .poll(
      async () =>
        (await befehle(page, "besprechungsplanung_protokoll_stand")).length,
    )
    .toBeGreaterThan(0);
}

const erzeugen = (page: Page) => page.getByTestId("protokoll-erzeugen");
const freigeben = (page: Page) => page.getByTestId("protokoll-freigeben");
const entwurf = (page: Page) => page.getByTestId("protokoll-entwurf-oeffnen");

test("ohne Besprechung gibt es kein Protokoll und einen Hinweis", async ({
  page,
}) => {
  await oeffnen(page, "Workshop Ablage");
  await expect(page.getByTestId("protokoll-ohne-besprechung")).toBeVisible();
  await expect(erzeugen(page)).toBeDisabled();
  await expect(freigeben(page)).toBeDisabled();
  await expect(entwurf(page)).toBeDisabled();
});

test("Protokoll aus Agenda: erst die Vorlage, dann der vorhandene Erzeugungsweg", async ({
  page,
}) => {
  await oeffnen(page, "Abstimmung Posteingang");
  await expect(erzeugen(page)).toBeEnabled();
  await expect(freigeben(page)).toBeDisabled();
  await erzeugen(page).click();
  await expect(page.getByTestId("protokoll-version")).toBeVisible();
  const reihenfolge = (await aufrufe(page))
    .map(([cmd]) => cmd)
    .filter((cmd) =>
      [
        "besprechungsplanung_protokollvorlage",
        "meetings_generate_minutes",
      ].includes(cmd),
    );
  expect(reihenfolge).toEqual([
    "besprechungsplanung_protokollvorlage",
    "meetings_generate_minutes",
  ]);
  const [[, erzeugt]] = await befehle(page, "meetings_generate_minutes");
  expect(erzeugt.meetingId).toBe("m_prot");
  expect(erzeugt.templateId).toBe("vorlage_p_prot");
  await expect(freigeben(page)).toBeEnabled();
  await expect(entwurf(page)).toBeDisabled();
});

test("Einwilligungs-Tor: ohne Zustimmung aller kein Protokoll aus dem Transkript", async ({
  page,
}) => {
  await oeffnen(page, "Jour fixe Archiv");
  await erzeugen(page).click();
  const fehler = page.getByTestId("protokoll-fehler");
  await expect(fehler).toBeVisible();
  await expect(fehler).toContainText(/consent gate|Einwilligungs-Tor/);
  await expect(page.getByTestId("protokoll-version")).toHaveCount(0);
  await expect(freigeben(page)).toBeDisabled();
});

test("Ziel erreicht wird von Hand gesetzt und zurückgenommen", async ({
  page,
}) => {
  await oeffnen(page, "Abstimmung Posteingang");
  await expect(page.getByTestId("protokoll-erreicht-offen")).toBeVisible();
  for (const wert of ["ja", "teilweise", "nein"]) {
    await expect(
      page.getByTestId(`protokoll-erreicht-${wert}`),
    ).not.toBeChecked();
  }
  await page.getByTestId("protokoll-erreicht-teilweise").check();
  await expect(page.getByTestId("protokoll-erreicht-teilweise")).toBeChecked();
  await expect(page.getByTestId("protokoll-erreicht-offen")).toHaveCount(0);
  const gesetzt = await befehle(page, "besprechungsplanung_erreicht_setzen");
  expect(gesetzt.map(([, a]) => a)).toEqual([
    { id: "p_prot", erreicht: "teilweise" },
  ]);
  await page.getByTestId("protokoll-erreicht-zurueck").click();
  await expect(page.getByTestId("protokoll-erreicht-offen")).toBeVisible();
  const alle = await befehle(page, "besprechungsplanung_erreicht_setzen");
  expect(alle[1][1]).toEqual({ id: "p_prot", erreicht: null });
});

test("Entwurf nur nach Freigabe: Empfänger ohne Dubletten, Betreff, Klassifizierung, nie versendet", async ({
  page,
}) => {
  await oeffnen(page, "Abstimmung Posteingang");
  await erzeugen(page).click();
  await expect(freigeben(page)).toBeEnabled();
  await expect(entwurf(page)).toBeDisabled();
  await freigeben(page).click();
  await expect(page.getByTestId("protokoll-freigegeben")).toBeVisible();
  await expect(entwurf(page)).toBeEnabled();
  await entwurf(page).click();

  const dialog = page.getByTestId("protokoll-entwurf");
  await expect(dialog).toBeVisible();
  await expect(page.getByTestId("protokoll-entwurf-an")).toHaveValue(
    "a@example.org, b@example.org",
  );
  await expect(page.getByTestId("protokoll-entwurf-an-hilfe")).toContainText(
    /without an email address|ohne E-Mail-Adresse/,
  );
  await expect(page.getByTestId("protokoll-entwurf-betreff")).toHaveValue(
    "Protokoll: Abstimmung Posteingang (2026-11-12)",
  );
  const text = await page.getByTestId("protokoll-entwurf-text").inputValue();
  expect(text.startsWith("Klassifizierung: intern\n")).toBe(true);
  expect(text.match(/Klassifizierung: intern/g)).toHaveLength(1);

  // Nur Ausgabe ueber den vorhandenen Weg, und nur auf Klick.
  expect(await befehle(page, "meeting_followup_open")).toHaveLength(0);
  await page.getByTestId("protokoll-entwurf-kopieren").click();
  await expect(page.getByTestId("protokoll-entwurf-meldung")).toBeVisible();
  const [[, ausgabe]] = await befehle(page, "meeting_followup_open");
  expect(ausgabe.mode).toBe("copy");
  expect(ausgabe.draft.to).toEqual(["a@example.org", "b@example.org"]);
  expect(ausgabe.draft.subject).toBe(
    "Protokoll: Abstimmung Posteingang (2026-11-12)",
  );
  const namen = (await aufrufe(page)).map(([cmd]) => cmd);
  expect(namen.filter((n) => /send|versend/i.test(n))).toEqual([]);
});

test("Entwurf: bei zu langer Mail im Mailprogramm erscheint der Einfügehinweis", async ({
  page,
}) => {
  await oeffnen(page, "Abstimmung Posteingang");
  await erzeugen(page).click();
  await freigeben(page).click();
  await entwurf(page).click();
  await expect(page.getByTestId("protokoll-entwurf")).toBeVisible();
  const meldung = page.getByTestId("protokoll-entwurf-meldung");

  // Kurze Mail: der Link wurde geöffnet, kein Hinweis auf die Zwischenablage.
  await page.getByTestId("protokoll-entwurf-mailto").click();
  await expect(meldung).toBeVisible();
  await expect(meldung).toHaveAttribute("data-art", "ok");
  await expect(meldung).not.toContainText(/clipboard|Zwischenablage/);

  // Lange Mail: das Backend meldet die Kürzung, der Text liegt in der Zwischenablage.
  await page.evaluate(() => {
    (window as any).__steuerung.meeting_followup_open = { gekuerzt: true };
  });
  await page.getByTestId("protokoll-entwurf-mailto").click();
  await expect(meldung).toHaveAttribute("data-art", "hinweis");
  await expect(meldung).toContainText(
    /too long for the mail link|zu lang für den Mail-Link/,
  );
  await expect(meldung).toContainText(/Ctrl\+V|Strg\+V/);
  const mailto = await befehle(page, "meeting_followup_open");
  expect(mailto.map(([, a]) => a.mode)).toEqual(["mailto", "mailto"]);

  // Kopieren und Datei sind davon nicht betroffen: nie der Mailto-Hinweis.
  await page.getByTestId("protokoll-entwurf-kopieren").click();
  await expect(meldung).toHaveAttribute("data-art", "ok");
  await expect(meldung).not.toContainText(/too long|zu lang/);
});

test("nach der Freigabe geändertes Protokoll ist nicht mehr freigegeben", async ({
  page,
}) => {
  await oeffnen(page, "Abstimmung Posteingang");
  await erzeugen(page).click();
  await freigeben(page).click();
  await expect(entwurf(page)).toBeEnabled();
  await page.getByTestId("protokoll-erreicht-ja").check();
  await expect(page.getByTestId("protokoll-geaendert")).toBeVisible();
  await expect(page.getByTestId("protokoll-freigegeben")).toHaveCount(0);
  await expect(entwurf(page)).toBeDisabled();
  await expect(freigeben(page)).toBeEnabled();
  await freigeben(page).click();
  await expect(entwurf(page)).toBeEnabled();
});

test("Entwurf ohne Freigabe wird vom Backend abgelehnt und zeigt den Grund", async ({
  page,
}) => {
  await oeffnen(page, "Abstimmung Posteingang");
  await page.evaluate(() => {
    (window as any).__steuerung.besprechungsplanung_protokoll_entwurf = {
      fehlerText: "protokoll_nicht_freigegeben",
    };
  });
  await erzeugen(page).click();
  await freigeben(page).click();
  await entwurf(page).click();
  await expect(page.getByTestId("protokoll-entwurf-fehler")).toContainText(
    /approved|freigegeben/,
  );
  await expect(page.getByTestId("protokoll-entwurf-kopieren")).toHaveCount(0);
});
