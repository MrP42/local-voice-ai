import { test, expect, type Page } from "@playwright/test";
import { installMock, setzeTermin } from "./planungMock";

// lva#844 U1: Insel und Register der Planung gegen die Tauri-Attrappe. Nur synthetische Daten
// (example.org). Die App-Sprache setzt die Attrappe auf Deutsch.
test.use({ locale: "en-US", timezoneId: "Europe/Berlin" });

const REGISTER = [
  ["planen", /Planen/],
  ["termin", /Termin & Zusagen/],
  ["durchfuehren", /Durchführen/],
  ["protokoll", /Protokoll/],
] as const;

async function oeffne(page: Page, titel: string, breite = 1440, hoehe = 900) {
  await page.setViewportSize({ width: breite, height: hoehe });
  await page.goto("/");
  await expect(page.getByTestId("planung-seite")).toBeVisible();
  await page.addStyleTag({
    content:
      "*,*::before,*::after{animation-duration:0s!important;transition-duration:0s!important}",
  });
  await page.getByTestId("planung-zeile").filter({ hasText: titel }).click();
  await expect(page.getByTestId("planung-insel")).toBeVisible();
}

const tab = (page: Page, name: RegExp) => page.getByRole("tab", { name });
const panel = (page: Page, id: string) =>
  page.getByTestId(`planung-panel-${id}`);

test("Register wechseln per Klick und per Tastatur, Panels bleiben gemountet", async ({
  page,
}) => {
  await installMock(page);
  await oeffne(page, "Planung Quartal 1");
  // Vor dem Start öffnet die Planung in „Planen“.
  await expect(tab(page, /Planen/)).toHaveAttribute("aria-selected", "true");
  await expect(panel(page, "planen")).toBeVisible();

  await tab(page, /Durchführen/).click();
  await expect(tab(page, /Durchführen/)).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(panel(page, "durchfuehren")).toBeVisible();
  await expect(panel(page, "durchfuehren")).toHaveAttribute(
    "aria-labelledby",
    "plg-register-durchfuehren",
  );
  // R1: die anderen Panels sind nur verborgen, nicht ausgehängt.
  for (const [id] of REGISTER) await expect(panel(page, id)).toHaveCount(1);
  await expect(panel(page, "planen")).toBeHidden();
  await expect(page.getByTestId("planung-titel")).toHaveCount(1);
  await expect(page.getByTestId("planung-termin")).toHaveCount(1);

  // Tastatur: Pfeile, Pos1, Ende; der Fokus folgt der Auswahl und bleibt sichtbar.
  await tab(page, /Planen/).click();
  await tab(page, /Planen/).focus();
  await page.keyboard.press("ArrowRight");
  await expect(tab(page, /Termin & Zusagen/)).toBeFocused();
  await expect(tab(page, /Termin & Zusagen/)).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(panel(page, "termin")).toBeVisible();
  // Fluent markiert den per Tastatur fokussierten Reiter (sichtbarer Fokusring).
  await expect(tab(page, /Termin & Zusagen/)).toHaveAttribute(
    "data-fui-focus-visible",
    "",
  );
  await page.keyboard.press("End");
  await expect(tab(page, /Protokoll/)).toBeFocused();
  await expect(panel(page, "protokoll")).toBeVisible();
  // Vor der Besprechung: Leerzustand über der Protokollkarte aus P5.
  await expect(page.getByTestId("planung-protokoll-leer")).toContainText(
    "Noch kein Protokoll.",
  );
  await expect(
    panel(page, "protokoll").getByTestId("planung-protokoll"),
  ).toBeVisible();
  await page.keyboard.press("Home");
  await expect(tab(page, /Planen/)).toBeFocused();
  await page.keyboard.press("ArrowLeft");
  await expect(tab(page, /Protokoll/)).toHaveAttribute("aria-selected", "true");
});

test("Neue Planung: Termin, Durchführen und Protokoll erst nach dem Speichern", async ({
  page,
}) => {
  await installMock(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.getByTestId("planung-neu").click();
  await expect(tab(page, /Termin & Zusagen/)).toBeDisabled();
  await expect(tab(page, /Durchführen/)).toBeDisabled();
  await expect(tab(page, /Protokoll/)).toBeDisabled();
  await expect(page.getByTestId("planung-durchfuehren-start")).toHaveCount(0);
  await expect(page.getByTestId("planung-mehr")).toHaveCount(0);
  // Kopfbefehle der Seite sind sekundär, die Primäre steht in der Insel.
  await expect(page.getByTestId("planung-neu")).toHaveAttribute(
    "data-variant",
    "secondary",
  );
});

test("Registerinfo folgt dem Datenstand, nicht der Ansicht", async ({
  page,
}) => {
  await installMock(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await setzeTermin(
    page,
    "src1:p_verknuepft",
    (
      [
        ["a@example.org", "ACCEPTED"],
        ["b@example.org", "TENTATIVE"],
        ["c@example.org", null],
      ] as const
    ).map(([email, partstat]) => ({
      email,
      name: null,
      organizer: false,
      is_self: false,
      partstat,
    })),
  );
  await page
    .getByTestId("planung-zeile")
    .filter({ hasText: "Planung Quartal 1" })
    .click();
  const info = (id: string) => page.getByTestId(`planung-register-info-${id}`);
  await expect(info("planen")).toHaveText("Vollständig, 2 Punkte");
  await expect(info("termin")).toHaveText("1 von 3 zugesagt");
  await expect(info("durchfuehren")).toHaveText(
    /^\S+ \d{2}\.\d{2}\., \d{2}:\d{2}$/,
  );
  await expect(info("protokoll")).toHaveText("nach der Besprechung");
  await expect(page.getByTestId("planung-insel")).toContainText(
    "3 Teilnehmende",
  );

  // Ein Registerwechsel ändert nichts am Stand.
  await tab(page, /Durchführen/).click();
  await expect(info("planen")).toHaveText("Vollständig, 2 Punkte");
  await expect(info("termin")).toHaveText("1 von 3 zugesagt");

  // Eine ungespeicherte Änderung sagt die Insel sofort.
  await tab(page, /Planen/).click();
  await page.getByTestId("planung-titel").fill("Planung Quartal 1 (neu)");
  await expect(info("planen")).toHaveText("Ungespeichert, 2 Punkte");
  await expect(page.getByTestId("planung-register-planen")).toHaveAttribute(
    "data-stand",
    "offen",
  );

  // Eine Kalendermeldung öffnet die Planung in „Termin & Zusagen“ mit Warnung.
  await page
    .getByTestId("planung-zeile")
    .filter({ hasText: "Workshop Rollen" })
    .click();
  await expect(tab(page, /Termin & Zusagen/)).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(page.getByTestId("planung-register-termin")).toHaveAttribute(
    "data-stand",
    "achtung",
  );
  await expect(info("termin")).toHaveText("Konflikt");
});

test("Chip „Verschoben“ springt zur Meldung unter „Termin & Zusagen“", async ({
  page,
}) => {
  await installMock(page);
  await oeffne(page, "Review Datenschutz");
  await tab(page, /Planen/).click();
  await expect(panel(page, "termin")).toBeHidden();
  const chip = page.getByTestId("planung-insel-verschoben");
  await expect(chip).toHaveRole("button");
  await chip.click();
  await expect(tab(page, /Termin & Zusagen/)).toHaveAttribute(
    "aria-selected",
    "true",
  );
  const meldung = page.getByTestId("planung-verschiebung-meldung");
  await expect(meldung).toBeVisible();
  await expect(meldung).toBeFocused();
  await expect(meldung).toContainText("Beitrittslink im Outlook-Termin");
  await expect(meldung).not.toContainText(/Webex|Teams/);
  await expect(
    meldung.getByTestId("planung-verschiebung-bestaetigen"),
  ).toHaveAttribute("data-variant", "primary");
  // Genau eine Bestätigung (nicht zusätzlich in der Termin-Karte).
  await expect(
    page.getByTestId("planung-verschiebung-bestaetigen"),
  ).toHaveCount(1);
});

test("„Besprechung starten …“ steht in jedem Register, primär nur in „Durchführen“", async ({
  page,
}) => {
  await installMock(page);
  await oeffne(page, "Planung Quartal 1");
  const start = page.getByTestId("planung-durchfuehren-start");
  for (const [id, name] of REGISTER) {
    await tab(page, name).click();
    await expect(start).toBeVisible();
    await expect(start).toBeEnabled();
    await expect(start).toHaveAttribute(
      "data-variant",
      id === "durchfuehren" ? "primary" : "secondary",
    );
    // Höchstens eine Primäraktion je Ansicht.
    expect(
      await page
        .getByTestId("planung-insel")
        .locator('[data-variant="primary"]')
        .count(),
    ).toBeLessThanOrEqual(1);
  }
  await tab(page, /Protokoll/).click();
  await start.click();
  await expect(page.getByRole("dialog")).toBeVisible();
  for (const adresse of ["b@example.org", "c@example.org"]) {
    const knopf = page
      .getByTestId("planung-start-person")
      .filter({ hasText: adresse })
      .getByTestId("planung-start-zustimmen");
    await knopf.click();
    await expect(knopf).toHaveCount(0);
  }
  await page.getByTestId("planung-start-los").click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  // Nach dem Start: einmal nach „Durchführen“, der Startknopf entfällt, die Insel meldet den Lauf.
  await expect(tab(page, /Durchführen/)).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(start).toHaveCount(0);
  await expect(page.getByTestId("planung-insel-lauf")).toContainText(
    "Läuft mit Transkription",
  );
  await expect(
    page.getByTestId("planung-register-info-durchfuehren"),
  ).toHaveText("Läuft mit Transkription");

  // R1: der Lauf überlebt den Registerwechsel samt aktuellem Punkt.
  const zeitboxen = page.getByTestId("planung-zeitboxen");
  await zeitboxen.getByTestId("planung-zeitbox-start").click();
  await zeitboxen.getByTestId("planung-zeitbox-weiter").click();
  const aktuell = zeitboxen.locator('[aria-current="step"]');
  await expect(aktuell).toContainText("Optionen");
  await tab(page, /Planen/).click();
  await expect(page.getByTestId("planung-lauf-modus")).toHaveCount(1);
  await expect(page.getByTestId("planung-insel-lauf")).toBeVisible();
  await tab(page, /Durchführen/).click();
  await expect(aktuell).toContainText("Optionen");
  await expect(page.getByTestId("planung-lauf-modus")).toHaveAttribute(
    "data-modus",
    "transkription",
  );
});

test("Einwilligungen: zugeklappt bleibt der Verlauf im DOM (R2)", async ({
  page,
}) => {
  await installMock(page);
  await oeffne(page, "Planung Quartal 1");
  const knopf = page.getByRole("button", { name: /Einwilligungen/ });
  await expect(knopf).toHaveAttribute("aria-expanded", "false");
  await expect(page.getByTestId("planung-einwilligungen")).toContainText(
    "1 zugestimmt, 1 offen, 1 abgelehnt",
  );
  await expect(page.getByTestId("planung-verlauf-eintrag")).toHaveCount(2);
  await expect(page.getByTestId("planung-einw-verlauf")).toBeHidden();
  await knopf.click();
  await expect(knopf).toHaveAttribute("aria-expanded", "true");
  await expect(page.getByTestId("planung-einw-verlauf")).toBeVisible();
  await expect(page.getByTestId("planung-einw-person")).toHaveCount(3);
});

test("Menü „Weitere Aktionen“ per Tastatur: Pfeile und Esc", async ({
  page,
}) => {
  await installMock(page);
  await oeffne(page, "Abstimmung Pilotstart");
  const mehr = page.getByRole("button", {
    name: "Weitere Aktionen zur Planung",
  });
  await mehr.focus();
  await page.keyboard.press("Enter");
  const eintraege = page.getByRole("menuitem");
  await expect(eintraege.first()).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(page.getByRole("menuitem", { name: /Löschen/ })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("menu")).toHaveCount(0);
  await expect(mehr).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(page.getByRole("menu")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(mehr).toBeFocused();
});

test("Insel lässt sich per Klasse abschalten", async ({ page }) => {
  await installMock(page);
  await oeffne(page, "Planung Quartal 1");
  const insel = page.getByTestId("planung-insel");
  const flaeche = () =>
    insel.evaluate((el) => {
      const cs = getComputedStyle(el);
      return { bild: cs.backgroundImage, farbe: cs.backgroundColor };
    });
  expect((await flaeche()).bild).toContain("linear-gradient");
  await insel.evaluate((el) => el.classList.add("plg-insel--aus"));
  const aus = await flaeche();
  expect(aus.bild).toBe("none");
  // Normaler Fluent-Kopf: Background1 (hell weiß), Titel in Foreground1.
  expect(aus.farbe).toBe("rgb(255, 255, 255)");
  const titel = await insel
    .locator(".plg-insel__name")
    .evaluate((el) => getComputedStyle(el).color);
  expect(titel).not.toBe("rgb(255, 255, 255)");
});

test("Unter 900 px wird die Registerleiste kompakt", async ({ page }) => {
  await installMock(page);
  await oeffne(page, "Planung Quartal 1", 1440, 900);
  for (const [id] of REGISTER)
    await expect(
      page.getByTestId(`planung-register-${id}`).locator(".plg-phase__name"),
    ).toBeVisible();
  await page.setViewportSize({ width: 900, height: 800 });
  await expect(
    page.getByTestId("planung-register-planen").locator(".plg-phase__name"),
  ).toBeVisible();
  // Name und Stand der übrigen Register sind nur optisch ausgeblendet (1 px, abgeschnitten).
  const textBreite = (id: string) =>
    page
      .getByTestId(`planung-register-${id}`)
      .locator(".plg-phase__text")
      .evaluate((el) => el.getBoundingClientRect().width);
  for (const id of ["termin", "durchfuehren", "protokoll"]) {
    await expect.poll(() => textBreite(id)).toBeLessThanOrEqual(1);
    await expect(
      page.getByTestId(`planung-register-${id}`).locator(".plg-phase__punkt"),
    ).toBeVisible();
  }
  // Der Name bleibt der zugängliche Name des Registers.
  await expect(tab(page, /^Termin & Zusagen/)).toHaveCount(1);
  await expect(tab(page, /^Durchführen/)).toHaveCount(1);
  const breit = await page.evaluate(() => ({
    scroll: document.documentElement.scrollWidth,
    fenster: window.innerWidth,
  }));
  expect(breit.scroll).toBeLessThanOrEqual(breit.fenster);
});

test("Insel verdichtet sich beim Scrollen auf eine Zeile", async ({ page }) => {
  await installMock(page);
  await oeffne(page, "Planung Quartal 1", 1440, 700);
  const insel = page.getByTestId("planung-insel");
  await expect(insel).toHaveAttribute("data-kompakt", "false");
  const hoeheVorher = (await insel.boundingBox())!.height;
  await page.locator("main.workspace-main").evaluate((el) => {
    el.scrollTop = 400;
  });
  await expect(insel).toHaveAttribute("data-kompakt", "true");
  await expect(insel).toBeInViewport();
  await expect(tab(page, /^Durchführen/)).toHaveCount(1);
  expect((await insel.boundingBox())!.height).toBeLessThan(hoeheVorher);
  // Eine Zeile: Titel, Register und Befehle (nur Symbol, Name bleibt zugänglich).
  expect((await insel.boundingBox())!.height).toBeLessThanOrEqual(48);
  await expect(
    page.getByRole("button", { name: /Besprechung starten/ }),
  ).toHaveCount(1);
  await page.locator("main.workspace-main").evaluate((el) => {
    el.scrollTop = 0;
  });
  await expect(insel).toHaveAttribute("data-kompakt", "false");
});

test("Protokoll-Registerinfo folgt dem Protokollstand aus P5", async ({
  page,
}) => {
  await installMock(page, { protokoll: true });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.addStyleTag({
    content:
      "*,*::before,*::after{animation-duration:0s!important;transition-duration:0s!important}",
  });
  const info = page.getByTestId("planung-register-info-protokoll");

  // Ohne Besprechung: Leerzustand, die Karte bleibt mit ihrem Hinweis darunter.
  await page
    .getByTestId("planung-zeile")
    .filter({ hasText: "Workshop Ablage" })
    .click();
  await expect(info).toHaveText("nach der Besprechung");
  await tab(page, /Protokoll/).click();
  await expect(page.getByTestId("planung-protokoll-leer")).toBeVisible();
  await expect(page.getByTestId("protokoll-ohne-besprechung")).toBeVisible();

  // Besprechung gehalten: noch offen, dann Version 1 mit offener Freigabe, dann freigegeben.
  await page.getByRole("tab", { name: /^(Alle|All)$/ }).click();
  await page
    .getByTestId("planung-zeile")
    .filter({ hasText: "Abstimmung Posteingang" })
    .click();
  await expect(info).toHaveText("Noch offen");
  await tab(page, /Protokoll/).click();
  await expect(page.getByTestId("planung-protokoll-leer")).toHaveCount(0);
  await page.getByTestId("protokoll-erzeugen").click();
  await expect(info).toHaveText("Version 1, Freigabe offen");
  await page.getByTestId("protokoll-freigeben").click();
  await expect(info).toHaveText("Freigegeben");
  await expect(page.getByTestId("planung-register-protokoll")).toHaveAttribute(
    "data-stand",
    "fertig",
  );
  // Nach der Freigabe geändert: Achtung.
  await page.getByTestId("protokoll-erreicht-ja").click();
  await expect(info).toHaveText("Seit der Freigabe geändert");
  await expect(page.getByTestId("planung-register-protokoll")).toHaveAttribute(
    "data-stand",
    "achtung",
  );
});
