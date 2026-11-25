import { test, expect, type Locator, type Page } from "@playwright/test";
import { installMock, setzeTermin, type MockAttendee } from "./planungMock";

// Auswahlfelder der Planung (lva#839) gegen die Tauri-Attrappe: feste Listen springen per
// Anfangsbuchstabe, Rollen sind freie Comboboxen mit Teilwortsuche über die Teilnehmenden
// des verknüpften Termins. Nur synthetische Daten (example.org).
test.use({ locale: "en-US", timezoneId: "Europe/Berlin" });

async function offne(page: Page) {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await expect(page.getByTestId("planung-seite")).toBeVisible();
  await page.addStyleTag({
    content:
      "*,*::before,*::after{animation-duration:0s!important;transition-duration:0s!important}",
  });
}

const zeile = (page: Page, titel: string) =>
  page.getByTestId("planung-zeile").filter({ hasText: titel });

const letztePruefung = async (page: Page) =>
  page.evaluate(() => {
    const calls = (window as any).__calls as [string, any][];
    return calls
      .filter(([c]) => c === "besprechungsplanung_pruefen")
      .at(-1)?.[1].input;
  });

/** Text der aktiven Option (aria-activedescendant), `null` ohne aktive Option. */
const aktiv = (feld: Locator) =>
  feld.evaluate((el) => {
    const id = el.getAttribute("aria-activedescendant");
    return id ? (document.getElementById(id)?.textContent ?? null) : null;
  });

const optionen = (page: Page) => page.getByRole("listbox").getByRole("option");

function person(name: string | null, email: string): MockAttendee {
  return { email, name, organizer: false, is_self: false, partstat: null };
}

async function setzeKlassifizierung(page: Page, id: string, wert: string) {
  await page.evaluate(
    ({ id, wert }) => {
      const p = (window as any).__daten.plaene.find((x: any) => x.id === id);
      p.classification = wert;
    },
    { id, wert },
  );
  await page.getByTestId("planung-abgleichen").click();
}

test("Klassifizierung: Buchstabe springt, wiederholtes Tippen geht weiter, „o“ findet „ö“", async ({
  page,
}) => {
  await installMock(page);
  await offne(page);
  // Eine eigene Klassifizierung ergänzt die Liste: zwei Einträge mit „v“.
  await setzeKlassifizierung(page, "p_entwurf", "Verschlusssache");
  await zeile(page, "Abstimmung Pilotstart").click();
  const feld = page.getByRole("combobox", { name: "Klassifizierung" });
  await expect(feld).toHaveText("Verschlusssache");
  await feld.focus();

  // Geschlossen: der erste Buchstabe öffnet und springt sofort.
  await page.keyboard.press("v");
  await expect(page.getByRole("listbox")).toBeVisible();
  await expect.poll(() => aktiv(feld)).toBe("vertraulich");
  await page.keyboard.press("v");
  await expect.poll(() => aktiv(feld)).toBe("Verschlusssache");
  // Am Ende wieder vorn; auch nach einer Pause.
  await page.waitForTimeout(600);
  await page.keyboard.press("v");
  await expect.poll(() => aktiv(feld)).toBe("vertraulich");
  // Der Anfangssprung lässt den Grundbuchstaben zu: „o“ und „oe“ finden „öffentlich“.
  await page.waitForTimeout(600);
  await page.keyboard.press("o");
  await expect.poll(() => aktiv(feld)).toBe("öffentlich");
  await page.keyboard.press("End");
  await page.waitForTimeout(600);
  await page.keyboard.type("oe");
  await expect.poll(() => aktiv(feld)).toBe("öffentlich");
  // Mehrere Buchstaben suchen den Anfang.
  await page.waitForTimeout(600);
  await page.keyboard.type("st");
  await expect.poll(() => aktiv(feld)).toBe("streng vertraulich");

  // Übernehmen mit Enter: Feld zeigt den Wert, die Prüfung bekommt ihn.
  await page.keyboard.press("Enter");
  await expect(page.getByRole("listbox")).toBeHidden();
  await expect(feld).toHaveText("streng vertraulich");
  await expect
    .poll(async () => (await letztePruefung(page))?.classification)
    .toBe("streng vertraulich");
});

test("Feste Liste: Pfeile, Pos1/Ende, Bild ab, Enter und Escape", async ({
  page,
}) => {
  await installMock(page);
  await offne(page);
  await zeile(page, "Abstimmung Pilotstart").click();
  const feld = page.getByRole("combobox", { name: "Art des Ziels" });
  await expect(feld).toHaveText("Entscheiden");
  await feld.focus();

  await page.keyboard.press("ArrowDown");
  await expect(page.getByRole("listbox")).toBeVisible();
  await expect.poll(() => aktiv(feld)).toBe("Entscheiden");
  await page.keyboard.press("Home");
  await expect.poll(() => aktiv(feld)).toBe("Klären");
  await page.keyboard.press("End");
  await expect.poll(() => aktiv(feld)).toBe("Informieren");
  await page.keyboard.press("ArrowUp");
  await expect.poll(() => aktiv(feld)).toBe("Entscheiden");
  await page.keyboard.press("PageUp");
  await expect.poll(() => aktiv(feld)).toBe("Klären");
  await page.keyboard.press("PageDown");
  await expect.poll(() => aktiv(feld)).toBe("Informieren");

  // Escape schließt, ohne etwas zu übernehmen.
  await page.keyboard.press("f");
  await expect.poll(() => aktiv(feld)).toBe("Feedback einholen");
  await page.keyboard.press("Escape");
  await expect(page.getByRole("listbox")).toBeHidden();
  await expect(feld).toHaveText("Entscheiden");

  // Buchstabe + Enter übernimmt.
  await page.keyboard.press("f");
  await expect.poll(() => aktiv(feld)).toBe("Feedback einholen");
  await page.keyboard.press("Enter");
  await expect(feld).toHaveText("Feedback einholen");
  await expect
    .poll(async () => (await letztePruefung(page))?.goal_type)
    .toBe("feedback");
});

test("Feste Liste: Esc, Enter und Fokuswechsel leeren den Tipp-Puffer", async ({
  page,
}) => {
  await installMock(page);
  await offne(page);
  await zeile(page, "Abstimmung Pilotstart").click();
  const feld = page.getByRole("combobox", { name: "Art des Ziels" });
  await expect(feld).toHaveText("Entscheiden");
  await feld.focus();
  // Ohne Leeren hieße der Puffer „fi“ (kein Treffer) und die Liste bliebe auf der Auswahl.
  // Alle Tasten folgen innerhalb der Tipppause (500 ms) aufeinander.

  // Escape
  await page.keyboard.press("f");
  await expect.poll(() => aktiv(feld)).toBe("Feedback einholen");
  await page.keyboard.press("Escape");
  await page.keyboard.press("i");
  await expect.poll(() => aktiv(feld)).toBe("Informieren");
  await page.keyboard.press("Escape");
  await page.waitForTimeout(600);

  // Enter (übernimmt „Feedback einholen“)
  await page.keyboard.press("f");
  await expect.poll(() => aktiv(feld)).toBe("Feedback einholen");
  await page.keyboard.press("Enter");
  await expect(feld).toHaveText("Feedback einholen");
  await page.keyboard.press("k");
  await expect.poll(() => aktiv(feld)).toBe("Klären");
  await page.keyboard.press("Escape");
  await page.waitForTimeout(600);

  // Fokuswechsel
  await page.keyboard.press("e");
  await expect.poll(() => aktiv(feld)).toBe("Entscheiden");
  await feld.evaluate((el) => (el as HTMLElement).blur());
  await feld.focus();
  await page.keyboard.press("i");
  await expect.poll(() => aktiv(feld)).toBe("Informieren");
  await page.keyboard.press("Escape");
  await expect(feld).toHaveText("Feedback einholen");
});

test("Rolle: Teilwortsuche über die Teilnehmenden, Umlaute, Hervorhebung und Tastatur", async ({
  page,
}) => {
  await installMock(page);
  await offne(page);
  await setzeTermin(page, "src1:p_verknuepft", [
    person("Jörg Müller", "a@example.org"),
    person("Anna Schäfer", "b@example.org"),
    person(null, "c@example.org"),
    person("Mara Klein", "d@example.org"),
    person("Petra Strauß", "e@example.org"),
    // Doppelt im Termin: nur ein Vorschlag.
    person("Jörg Müller", "a@example.org"),
  ]);
  await zeile(page, "Planung Quartal 1").click();
  const feld = page.getByRole("combobox", { name: "Moderation" });
  // Der gespeicherte Freitext steht im Feld, auch ohne passenden Vorschlag.
  await expect(feld).toHaveValue("A. Beispiel");

  await feld.fill("");
  await feld.pressSequentially("m");
  // Ein einzelner Buchstabe filtert: Feldanfang vor Wortanfang vor Wortmitte.
  await expect(optionen(page)).toHaveText([
    "Mara Klein",
    "Jörg Müller",
    "c@example.org",
  ]);
  await expect(optionen(page).first().locator("mark")).toHaveText("M");
  // Nichts ist vorausgewählt: Enter übernähme sonst ungewollt einen Vorschlag.
  expect(await aktiv(feld)).toBeNull();

  // Teilwort mitten im Wort, hervorgehoben.
  await feld.fill("ller");
  await expect(optionen(page)).toHaveText(["Jörg Müller"]);
  await expect(optionen(page).locator("mark")).toHaveText("ller");
  // Die Statuszeile nennt die Zahl der Vorschläge (Bildschirmleser).
  await expect(
    page.getByTestId("auswahl-status").filter({ hasText: /\S/ }),
  ).toHaveText("1 Vorschlag");

  // Umlaut-Toleranz in beide Richtungen.
  await feld.fill("mue");
  await expect(optionen(page)).toHaveText(["Jörg Müller"]);
  await expect(optionen(page).locator("mark")).toHaveText("Mü");
  await feld.fill("SCHAEF");
  await expect(optionen(page)).toHaveText(["Anna Schäfer"]);
  await feld.fill("strauss");
  await expect(optionen(page)).toHaveText(["Petra Strauß"]);
  await expect(optionen(page).locator("mark")).toHaveText("Strauß");

  // Tastatur: Pfeil ab aktiviert den ersten Vorschlag, Ende/Pos1, Enter übernimmt.
  // „a“ am Feldanfang zuerst, die übrigen (mitten im Wort) in der Reihenfolge des Termins.
  await feld.fill("a");
  await expect(optionen(page)).toHaveText([
    "Anna Schäfer",
    "c@example.org",
    "Mara Klein",
    "Petra Strauß",
  ]);
  await page.keyboard.press("ArrowDown");
  await expect.poll(() => aktiv(feld)).toBe("Anna Schäfer");
  await page.keyboard.press("ArrowDown");
  await expect.poll(() => aktiv(feld)).toBe("c@example.org");
  await page.keyboard.press("End");
  await expect.poll(() => aktiv(feld)).toBe("Petra Strauß");
  await page.keyboard.press("Home");
  await expect.poll(() => aktiv(feld)).toBe("Anna Schäfer");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  await expect.poll(() => aktiv(feld)).toBe("Mara Klein");
  await page.keyboard.press("Enter");
  await expect(page.getByRole("listbox")).toBeHidden();
  await expect(feld).toHaveValue("Mara Klein");
  await expect
    .poll(async () => (await letztePruefung(page))?.roles.moderation)
    .toBe("Mara Klein");

  // Escape schließt die Liste und lässt den getippten Text stehen.
  await feld.fill("Pet");
  await expect(optionen(page)).toHaveText(["Petra Strauß"]);
  await page.keyboard.press("Escape");
  await expect(page.getByRole("listbox")).toBeHidden();
  await expect(feld).toHaveValue("Pet");
});

test("Rolle: Freitext ohne Vorschlag bleibt erhalten, auch mit Enter und Tab", async ({
  page,
}) => {
  await installMock(page);
  await offne(page);
  await setzeTermin(page, "src1:p_verknuepft", [
    person("Anna Schäfer", "b@example.org"),
  ]);
  await zeile(page, "Planung Quartal 1").click();

  const ergebnis = page.getByRole("combobox", { name: "Ergebnissicherung" });
  await ergebnis.fill("Externe Agentur");
  await page.keyboard.press("Enter");
  await expect(ergebnis).toHaveValue("Externe Agentur");
  // Ein Teilwort eines Vorschlags wird mit Tab nicht zum Vorschlag.
  const entscheidung = page.getByRole("combobox", { name: "Entscheidung" });
  await entscheidung.fill("Ann");
  await expect(optionen(page)).toHaveText(["Anna Schäfer"]);
  await page.keyboard.press("Tab");
  await expect(entscheidung).toHaveValue("Ann");
  await expect
    .poll(async () => (await letztePruefung(page))?.roles)
    .toMatchObject({
      ergebnissicherung: "Externe Agentur",
      entscheidung: "Ann",
    });
});

test("Rolle ohne verknüpften Termin: freies Feld ohne Liste", async ({
  page,
}) => {
  await installMock(page);
  await offne(page);
  await zeile(page, "Abstimmung Pilotstart").click();
  const feld = page.getByRole("combobox", { name: "Moderation" });
  await expect(feld).toHaveValue("A. Beispiel");
  await feld.fill("Bernd");
  await page.keyboard.press("ArrowDown");
  await expect(page.getByRole("listbox")).toHaveCount(0);
  await expect(feld).toHaveValue("Bernd");
});
