import { test, expect, type Page } from "@playwright/test";
import { installMock } from "./planungMock";

// Die App-Sprache setzt die Attrappe auf Deutsch; der Browser bleibt bewusst englisch.
test.use({ locale: "en-US", timezoneId: "Europe/Berlin" });

// Seite „Planung“ (lva#534 P2) gegen die Tauri-Attrappe: Liste, Formular, Regelhinweise,
// Vorschau, Statusanzeige und Einwilligungsstand. Nur synthetische Daten (example.org).

async function offne(page: Page, breite = 1440, hoehe = 900) {
  await page.setViewportSize({ width: breite, height: hoehe });
  await page.goto("/");
  await expect(page.getByTestId("planung-seite")).toBeVisible();
  await page.addStyleTag({
    content:
      "*,*::before,*::after{animation-duration:0s!important;transition-duration:0s!important}",
  });
}

const aufrufe = (page: Page) =>
  page.evaluate(() => (window as any).__calls as [string, any][]);

const zeile = (page: Page, titel: string) =>
  page.getByTestId("planung-zeile").filter({ hasText: titel });

async function steuere(
  page: Page,
  cmd: string,
  wert: { pause?: boolean; fehler?: boolean },
) {
  await page.evaluate(
    ({ cmd, wert }) => {
      (window as any).__steuerung[cmd] = wert;
    },
    { cmd, wert },
  );
}

async function freigeben(page: Page, cmd: string) {
  await page.evaluate(
    (cmd) => {
      const w = window as any;
      const freigaben = w.__freigaben[cmd] ?? [];
      w.__freigaben[cmd] = [];
      freigaben.forEach((resolve: () => void) => resolve());
    },
    cmd,
  );
}

test("Englischer Browser zeigt deutsche 24-Stunden-Zeit und verwirft 25:00", async ({ page }) => {
  await installMock(page);
  await offne(page);
  await page.evaluate(() => {
    (window as any).__daten.links.p_verknuepft.starts_at =
      Date.parse("2026-11-20T09:00:00+01:00");
  });
  await page.getByTestId("planung-abgleichen").click();
  await zeile(page, "Planung Quartal 1").click();
  await expect(page.getByTestId("planung-beginn")).toHaveValue("09:00");
  await expect(page.getByTestId("planung-beginn")).toHaveAttribute("type", "text");
  await expect(page.getByTestId("planung-termin")).toContainText("09:00");
  await expect(page.getByTestId("planung-termin")).not.toContainText(/\b(?:AM|PM)\b/);
  await page.getByTestId("planung-neu").click();
  await page.getByTestId("planung-titel").fill("Zeitprüfung");
  await page.getByTestId("planung-beginn").fill("09:00");
  await expect(page.getByTestId("planung-speichern")).toBeEnabled();
  await page.getByTestId("planung-beginn").fill("25:00");
  await expect(page.getByTestId("planung-speichern")).toBeDisabled();
  await expect(page.getByRole("alert")).toContainText("HH:MM");
  await page.getByTestId("planung-beginn").fill("09:00");
  const datum = page.getByPlaceholder("TT.MM.JJJJ");
  await datum.fill("20.11.2026");
  await expect(page.getByTestId("planung-speichern")).toBeEnabled();
  await expect.poll(async () =>
    (await aufrufe(page)).filter(([c]) => c === "besprechungsplanung_pruefen")
      .at(-1)?.[1].input.hints.ergebnisfrist,
  ).toBe("2026-11-20");
  await datum.fill("31.02.2026");
  await expect(page.getByTestId("planung-speichern")).toBeDisabled();
});

test("Neuanlage bleibt während des Listenabrufs gesperrt und wird nur einmal angelegt", async ({
  page,
}) => {
  await installMock(page);
  await offne(page);
  await page.getByTestId("planung-neu").click();
  await page.getByTestId("planung-titel").fill("Einmal anlegen");
  await expect(page.getByTestId("planung-speichern")).toBeEnabled();
  await steuere(page, "besprechungsplanung_liste", { pause: true });
  const vorher = (await aufrufe(page)).filter(([c]) => c === "besprechungsplanung_liste").length;
  await page.getByTestId("planung-speichern").click();
  await expect.poll(async () =>
    (await aufrufe(page)).filter(([c]) => c === "besprechungsplanung_liste").length,
  ).toBe(vorher + 1);
  // Zweiter Speicherversuch während der angehaltenen Listenantwort.
  await page.getByTestId("planung-speichern").evaluate((element) => {
    (element as HTMLButtonElement).click();
  });
  await expect(page.getByTestId("planung-speichern")).toBeDisabled();
  await expect(page.getByTestId("planung-titel")).toBeDisabled();
  await freigeben(page, "besprechungsplanung_liste");
  await expect(zeile(page, "Einmal anlegen")).toHaveAttribute("aria-current", "true");
  expect((await aufrufe(page)).filter(([c]) => c === "besprechungsplanung_anlegen"))
    .toHaveLength(1);
  await page.getByTestId("planung-titel").fill("Spätere Eingabe");
  await expect(page.getByTestId("planung-titel")).toHaveValue("Spätere Eingabe");
});

test("Abgleich aktualisiert Beginn und Regelargument bei gleichbleibender Plan-ID", async ({
  page,
}) => {
  await installMock(page);
  await offne(page);
  await zeile(page, "Planung Quartal 1").click();
  await page.evaluate(() => {
    (window as any).__daten.links.p_verknuepft.starts_at =
      Date.parse("2026-11-20T09:00:00+01:00");
  });
  await page.getByTestId("planung-abgleichen").click();
  await expect(page.getByTestId("planung-beginn")).toHaveValue("09:00");
  await expect.poll(async () =>
    (await aufrufe(page)).filter(([c]) => c === "besprechungsplanung_pruefen").at(-1)?.[1].beginn,
  ).toEqual({ stunde: 9, minute: 0 });
  await page.evaluate(() => {
    (window as any).__daten.links.p_verknuepft.starts_at =
      Date.parse("2026-11-20T14:30:00+01:00");
  });
  await page.getByTestId("planung-abgleichen").click();
  await expect(page.getByTestId("planung-beginn")).toHaveValue("14:30");
  await expect.poll(async () =>
    (await aufrufe(page)).filter(([c]) => c === "besprechungsplanung_pruefen").at(-1)?.[1].beginn,
  ).toEqual({ stunde: 14, minute: 30 });
});

test("Titeländerung sendet die vorhandene Protokollvorlage zurück", async ({ page }) => {
  await installMock(page);
  await offne(page);
  await zeile(page, "Abstimmung Pilotstart").click();
  await page.getByTestId("planung-titel").fill("Titel geändert");
  await page.getByTestId("planung-speichern").click();
  await expect.poll(async () =>
    (await aufrufe(page)).find(([c]) => c === "besprechungsplanung_aendern")?.[1].input,
  ).toMatchObject({ title: "Titel geändert", minutes_template_id: "vorlage-1" });
});

test("Zeitbox ohne Thema erreicht die Regelprüfung und sperrt Speichern", async ({ page }) => {
  await installMock(page);
  await offne(page);
  await page.getByTestId("planung-neu").click();
  await page.getByTestId("planung-titel").fill("Agendaprüfung");
  await page.getByTestId("planung-agenda-neu").click();
  await page.getByRole("textbox", { name: "Minuten von Punkt 1" }).fill("5");
  await expect(page.getByTestId("planung-regel-agenda_punkt_leer")).toBeVisible();
  await expect(page.getByTestId("planung-speichern")).toBeDisabled();
});

test("Geänderte Eingabe verliert sofort die Freigabe, auch bei fehlgeschlagener Prüfung", async ({
  page,
}) => {
  await installMock(page);
  await offne(page);
  await zeile(page, "Abstimmung Pilotstart").click();
  await page.getByTestId("planung-titel").fill("Gültiger Titel");
  await expect(page.getByTestId("planung-speichern")).toBeEnabled();
  await steuere(page, "besprechungsplanung_pruefen", { pause: true, fehler: true });
  await page.getByTestId("planung-titel").fill("");
  await expect(page.getByTestId("planung-speichern")).toBeDisabled();
  await expect(page.getByTestId("planung-regeln-ok")).toHaveCount(0);
  await expect.poll(async () =>
    (await aufrufe(page)).filter(([c]) => c === "besprechungsplanung_pruefen").at(-1)?.[1].input.title,
  ).toBe("");
  await freigeben(page, "besprechungsplanung_pruefen");
  await expect(page.getByTestId("planung-regeln").getByRole("alert")).toBeVisible();
  await expect(page.getByTestId("planung-speichern")).toBeDisabled();
});

test("Veraltete Prüfantwort gibt die neue Eingabe nicht frei", async ({ page }) => {
  await installMock(page);
  await offne(page);
  await page.getByTestId("planung-neu").click();
  await steuere(page, "besprechungsplanung_pruefen", { pause: true });
  await page.getByTestId("planung-titel").fill("Alte gültige Fassung");
  await expect.poll(async () =>
    (await aufrufe(page)).filter(([c]) => c === "besprechungsplanung_pruefen").at(-1)?.[1].input.title,
  ).toBe("Alte gültige Fassung");
  await steuere(page, "besprechungsplanung_pruefen", {});
  await page.getByTestId("planung-titel").fill("");
  await expect(page.getByTestId("planung-regel-titel_leer")).toBeVisible();
  await freigeben(page, "besprechungsplanung_pruefen");
  await expect(page.getByTestId("planung-speichern")).toBeDisabled();
  await expect(page.getByTestId("planung-regel-titel_leer")).toBeVisible();
});

test("Einwilligungsabruf zeigt Fehler und erlaubt erneuten Versuch", async ({ page }) => {
  await installMock(page);
  await offne(page);
  await steuere(page, "besprechungsplanung_einwilligungen", { fehler: true });
  await zeile(page, "Planung Quartal 1").click();
  const karte = page.getByTestId("planung-einwilligungen");
  await expect(karte.getByRole("alert")).toBeVisible();
  await expect(karte.getByTestId("planung-einw-leer")).toHaveCount(0);
  await steuere(page, "besprechungsplanung_einwilligungen", {});
  await karte.getByRole("button", { name: "Erneut versuchen" }).click();
  await expect(karte.getByTestId("planung-einw-person")).toHaveCount(3);
  await expect(karte.getByRole("alert")).toHaveCount(0);
});

test("Liste: aktive Planungen mit Status, Archiv getrennt, genau eine Überschrift", async ({
  page,
}) => {
  await installMock(page);
  await offne(page);
  await expect(
    page.getByRole("heading", { level: 1, name: "Planung" }),
  ).toHaveCount(1);
  // Aktiv: Entwurf, übergeben, verknüpft, verschoben, Konflikt; nicht abgesagt und durchgeführt.
  await expect(page.getByTestId("planung-zeile")).toHaveCount(5);
  await expect(zeile(page, "Abstimmung Pilotstart")).toContainText("Entwurf");
  await expect(zeile(page, "Abstimmung Pilotstart")).toContainText(
    "Kein Termin",
  );
  await expect(zeile(page, "Jour fixe Betrieb")).toContainText("Übergeben");
  await expect(zeile(page, "Planung Quartal 1")).toContainText("Verknüpft");
  await expect(zeile(page, "Planung Quartal 1")).toContainText("50 Min.");
  await expect(zeile(page, "Review Datenschutz")).toContainText("Verschoben");
  const konflikt = zeile(page, "Workshop Rollen");
  await expect(konflikt.getByTestId("planung-status-verknuepft")).toBeVisible();
  await expect(konflikt.getByTestId("planung-status-konflikt")).toBeVisible();
  await page.getByRole("tab", { name: "Archiv" }).click();
  await expect(page.getByTestId("planung-zeile")).toHaveCount(2);
  await expect(zeile(page, "Kickoff Archiv")).toContainText("Abgesagt");
  await expect(zeile(page, "Retrospektive Oktober")).toContainText(
    "Durchgeführt",
  );
  await expect(page.getByTestId("planung-seite")).not.toContainText(
    /\bplanung\.[a-z]+\./,
  );
});

test("Leere Liste zeigt den Hinweis", async ({ page }) => {
  await installMock(page, { leer: true });
  await offne(page);
  await expect(page.getByTestId("planung-leer")).toContainText(
    "Noch keine Planung",
  );
});

test("Neue Planung: Regelhinweise und Vorschau folgen der Eingabe, Speichern schickt die Eingabe", async ({
  page,
}) => {
  await installMock(page);
  await offne(page);
  await page.getByTestId("planung-neu").click();
  const formular = page.getByTestId("planung-formular");
  await expect(formular).toBeVisible();
  // Ohne Titel: Fehler vom Backend, Speichern gesperrt.
  await expect(page.getByTestId("planung-regel-titel_leer")).toContainText(
    "Die Besprechung braucht einen Titel.",
  );
  await expect(page.getByTestId("planung-speichern")).toBeDisabled();
  await expect(page.getByTestId("planung-uebergeben")).toBeDisabled();

  await page.getByTestId("planung-titel").fill("Planung Pilotbetrieb");
  await page
    .getByTestId("planung-ziel")
    .fill("Wir entscheiden über den Pilotstart.");
  await expect(page.getByTestId("planung-regel-titel_leer")).toHaveCount(0);
  await expect(page.getByTestId("planung-regel-ziel_fehlt")).toHaveCount(0);
  await expect(page.getByTestId("planung-regel-agenda_leer")).toContainText(
    "Ohne Agenda",
  );
  await expect(page.getByTestId("planung-speichern")).toBeEnabled();

  // Agenda: Summe gegen Dauer, Regel „Agenda zu lang“ mit den Zahlen aus dem Formular.
  await page.getByTestId("planung-agenda-neu").click();
  await page.getByRole("textbox", { name: "Thema von Punkt 1" }).fill("Stand");
  await page.getByRole("textbox", { name: "Minuten von Punkt 1" }).fill("40");
  await expect(page.getByTestId("planung-summe")).toHaveText(
    "Zeitboxen: 40 von 25 Minuten",
  );
  await expect(page.getByTestId("planung-regel-agenda_zu_lang")).toContainText(
    "Die Agenda braucht 40 Minuten, geplant sind 25.",
  );
  await page.getByRole("textbox", { name: "Minuten von Punkt 1" }).fill("20");
  await expect(page.getByTestId("planung-regel-agenda_zu_lang")).toHaveCount(0);

  // Dauer 30: nicht empfohlen.
  await page.getByTestId("planung-dauer").fill("30");
  await expect(page.getByTestId("planung-regel-dauer_unueblich")).toContainText(
    "Empfohlen sind 25 oder 50 Minuten",
  );
  await formular.getByRole("button", { name: "50 Min." }).click();
  await expect(page.getByTestId("planung-regel-dauer_unueblich")).toHaveCount(
    0,
  );

  // Transkription: Hinweis und Einwilligungskarte.
  await page.getByTestId("planung-transkription").check();
  await expect(
    page.getByTestId("planung-regel-transkription_einwilligung"),
  ).toBeVisible();

  // Vorschau: formatiert im Rahmen ohne Rechte, als Text mit der Vorschau-Marke.
  const rahmen = page.getByTestId("planung-vorschau-html");
  await expect(rahmen).toHaveAttribute("sandbox", "");
  await expect(
    page
      .frameLocator('[data-testid="planung-vorschau-html"]')
      .getByText("Planung Pilotbetrieb"),
  ).toBeVisible();
  await page.getByRole("tab", { name: "Text" }).click();
  await expect(page.getByTestId("planung-vorschau-text")).toContainText(
    "lva-plan:vorschau",
  );

  await page.getByTestId("planung-speichern").click();
  const anlegen = (await aufrufe(page)).find(
    ([c]) => c === "besprechungsplanung_anlegen",
  );
  expect(anlegen).toBeTruthy();
  const input = anlegen![1].input;
  expect(input).toMatchObject({
    title: "Planung Pilotbetrieb",
    goal_type: "klaeren",
    duration_min: 50,
    classification: "intern",
    transcription_planned: true,
    agenda: [{ topic: "Stand", minutes: 20, owner: null }],
  });
  // Vorschau und Prüfung haben nie gespeichert oder übergeben.
  const befehle = (await aufrufe(page)).map(([c]) => c);
  expect(befehle).not.toContain("besprechungsplanung_einladung");
  // Nach dem Speichern steht die Planung in der Liste und ist gewählt.
  await expect(zeile(page, "Planung Pilotbetrieb")).toHaveAttribute(
    "aria-current",
    "true",
  );
});

test("Verschiebung: Status, Termin aus Outlook und Bestätigung", async ({
  page,
}) => {
  await installMock(page);
  await offne(page);
  await zeile(page, "Review Datenschutz").click();
  const termin = page.getByTestId("planung-termin");
  await expect(termin.getByTestId("planung-status-verschoben")).toBeVisible();
  await expect(termin).toContainText("Raum 1.02");
  // Der Beginn kommt aus dem Termin: Feld gesperrt, Hinweis darunter.
  await expect(page.getByTestId("planung-beginn")).toHaveAttribute(
    "readonly",
    "",
  );
  await page.getByTestId("planung-verschiebung-bestaetigen").click();
  const bestaetigt = (await aufrufe(page)).filter(
    ([c]) => c === "besprechungsplanung_verschiebung_bestaetigen",
  );
  expect(bestaetigt).toHaveLength(1);
  await expect(termin.getByTestId("planung-status-verknuepft")).toBeVisible();
  await expect(
    page.getByTestId("planung-verschiebung-bestaetigen"),
  ).toHaveCount(0);
});

test("Konflikt und abgesagter Termin werden gezeigt, ohne Aktion zum Überschreiben", async ({
  page,
}) => {
  await installMock(page);
  await offne(page);
  await zeile(page, "Workshop Rollen").click();
  const termin = page.getByTestId("planung-termin");
  await expect(termin.getByTestId("planung-status-konflikt")).toBeVisible();
  await expect(termin).toContainText("überschreibt ihn nicht");
  await expect(termin.getByRole("button")).toHaveCount(0);
  await page.getByRole("tab", { name: "Archiv" }).click();
  await zeile(page, "Kickoff Archiv").click();
  await expect(
    page.getByTestId("planung-termin").getByTestId("planung-status-abgesagt"),
  ).toBeVisible();
});

test("Einwilligungsstand je Person mit Aktionen zur Dokumentation", async ({ page }) => {
  await installMock(page);
  await offne(page);
  await zeile(page, "Planung Quartal 1").click();
  const karte = page.getByTestId("planung-einwilligungen");
  await expect(karte.getByTestId("planung-einw-person")).toHaveCount(3);
  // Ablehnung zuerst, dann offen, dann Zustimmung.
  const namen = await karte.locator(".plg-person__name").allTextContents();
  expect(namen).toEqual(["c@example.org", "b@example.org", "a@example.org"]);
  await expect(karte).toContainText("1 zugestimmt, 1 offen, 1 abgelehnt");
  await expect(karte).toContainText("Einladung");
  await expect(karte.getByTestId("planung-einw-zugestimmt")).toHaveCount(3);
  await expect(karte.getByTestId("planung-einw-abgelehnt")).toHaveCount(3);
  await expect(karte.getByTestId("planung-einw-offen")).toHaveCount(3);
  await expect(karte.getByRole("switch")).toHaveCount(0);
});

test("Durchgeführte Planung ist gesperrt", async ({ page }) => {
  await installMock(page);
  await offne(page);
  await page.getByRole("tab", { name: "Archiv" }).click();
  await zeile(page, "Retrospektive Oktober").click();
  await expect(page.getByTestId("planung-speichern")).toBeDisabled();
  await expect(page.getByTestId("planung-uebergeben")).toBeDisabled();
  await expect(page.getByTestId("planung-titel")).toBeDisabled();
  await expect(page.getByTestId("planung-detail")).toContainText(
    "lässt sich nicht mehr ändern",
  );
});

test("Entwurf übergeben: Status wechselt, Löschen fragt nach", async ({
  page,
}) => {
  await installMock(page);
  await offne(page);
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  await zeile(page, "Abstimmung Pilotstart").click();
  await page.getByTestId("planung-uebergeben").click();
  await expect(
    page.getByTestId("planung-termin").getByTestId("planung-status-uebergeben"),
  ).toBeVisible();
  const einladung = (await aufrufe(page)).filter(
    ([c]) => c === "besprechungsplanung_einladung",
  );
  expect(einladung).toHaveLength(1);
  expect(einladung[0][1].id).toBe("p_entwurf");

  await page.getByTestId("planung-loeschen").click();
  await expect(page.getByRole("dialog")).toContainText(
    "Planung „Abstimmung Pilotstart“ löschen?",
  );
  await page.getByTestId("planung-loeschen-ja").click();
  await expect(zeile(page, "Abstimmung Pilotstart")).toHaveCount(0);
});

test("Schmal: erst Liste, dann Blatt mit Zurück, kein Seitwärtsrollen", async ({
  page,
}) => {
  await installMock(page);
  await offne(page, 390, 844);
  await expect(page.getByTestId("planung-liste")).toBeVisible();
  await expect(page.getByTestId("planung-detail")).toHaveCount(0);
  await zeile(page, "Planung Quartal 1").click();
  await expect(page.getByTestId("planung-detail")).toBeVisible();
  await expect(page.getByTestId("planung-liste")).toHaveCount(0);
  const breit = await page.evaluate(() => ({
    scroll: document.documentElement.scrollWidth,
    fenster: window.innerWidth,
  }));
  expect(breit.scroll).toBeLessThanOrEqual(breit.fenster);
  await page.getByTestId("planung-zurueck").click();
  await expect(page.getByTestId("planung-liste")).toBeVisible();
});
