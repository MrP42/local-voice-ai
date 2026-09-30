import { test, expect, type Page } from "@playwright/test";
import * as path from "node:path";
import { calls } from "./calendarMock";
import {
  TITLE_M2,
  goToRecordings,
  openRecordings,
  pickMeeting,
} from "./recLayoutMock";
import { PEOPLE, installLiveMock } from "./recLiveMock";

// U7 (Issue #64): Metadaten einer Besprechung bearbeiten -- Titel, Beschreibung
// (mehrzeilig), Datum/Uhrzeit, Teilnehmende (vorhandene Personen) und Projekte
// (Ordner, n:m) im Details-Dialog; Dateiname und Quelle bleiben unveraendert
// sichtbar. Die Attrappe (recLiveMock) schreibt die Aenderung wie das Backend und
// haelt sie mit `persist` ueber ein Neuladen der Seite; die Regeln (alles oder
// nichts, Migration, Suche, Kontext fuer Chat/KI-Notizen/MCP) prueft Rust.

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

const SHOT_DIR = path.resolve(
  process.cwd(),
  "../../koordination/aufnahmen-ui/screens",
);
/** Nachher-Bilder nur auf Wunsch (LVA_SCREENSHOTS=1), sonst schreibt jeder Lauf sie neu. */
const shoot = async (page: Page, name: string) => {
  if (!process.env.LVA_SCREENSHOTS) return;
  await page.waitForTimeout(250);
  await page.screenshot({
    path: path.join(SHOT_DIR, `${name}.png`),
    animations: "disabled",
  });
};

test.beforeEach(async ({ page }) => {
  await installLiveMock(page, { persist: true });
  await page.addInitScript((people) => {
    (window as any).__people = people;
  }, PEOPLE);
});

const NEW_TITLE = "Kundentermin Stadtwerke – Lastgang (überarbeitet)";
const NEW_DESCRIPTION =
  "Thema: Lastgang Q3 und Speicher\nAnsprechpartnerin: Frau Becker (SWK)";

const dialogOf = (page: Page) => page.getByRole("dialog", { name: "Details" });

const openDetails = async (page: Page) => {
  await page.getByTestId("meeting-details-open").click();
  await expect(dialogOf(page)).toBeVisible();
};

const detailsRow = (page: Page, key: string) =>
  dialogOf(page).getByTestId(`details-${key}`);

const openM2 = async (page: Page, width = 1366, height = 768) => {
  await openRecordings(page, width, height);
  await pickMeeting(page, "m2");
  await expect(page.getByTestId("meeting-title")).toHaveText(TITLE_M2);
};

const editMode = async (page: Page) => {
  await dialogOf(page).getByTestId("details-edit").click();
  await expect(page.getByTestId("meta-form")).toBeVisible();
};

// ---------------------------------------------------------------------------
// Bearbeiten, Speichern, Neuladen
// ---------------------------------------------------------------------------

test("Titel, Beschreibung, Datum, Teilnehmende und Projekte bearbeiten; nach dem Neuladen sind sie erhalten", async ({
  page,
}) => {
  await openM2(page);
  await openDetails(page);
  // Vorher: keine Beschreibung, keine Teilnehmenden, zwei Projekte.
  await expect(detailsRow(page, "description")).toHaveText(
    "Keine Beschreibung",
  );
  await expect(detailsRow(page, "people")).toHaveText(
    "Keine Teilnehmenden erfasst",
  );
  await expect(detailsRow(page, "projects")).toHaveText(
    "Geschäftlich, Kunde Stadtwerke",
  );

  await editMode(page);
  // Quelle und Dateiname stehen weiter da, aber nicht als Eingabefeld.
  await expect(dialogOf(page).getByTestId("details-file")).toHaveText(
    "SWK-Lastgang-2026-09-28.m4a",
  );
  await expect(dialogOf(page).getByTestId("details-source")).toHaveText(
    "Datei-Import",
  );
  // Nichts geaendert: nichts zu speichern.
  await expect(page.getByTestId("meta-save")).toBeDisabled();

  await page.getByTestId("meta-title").fill(NEW_TITLE);
  await page.getByTestId("meta-description").fill(NEW_DESCRIPTION);
  await expect(page.getByTestId("meta-description-count")).toHaveText(
    `${NEW_DESCRIPTION.length} von 4000 Zeichen`,
  );
  await page.getByTestId("meta-datetime").fill("2026-09-20T09:30");

  // Teilnehmende: suchen, waehlen, wieder entfernen, eine andere waehlen.
  await page.getByTestId("meta-people-search").fill("anna");
  await expect(page.getByTestId("meta-person-add-h-ben")).toHaveCount(0);
  await page.getByTestId("meta-person-add-h-anna").click();
  await page.getByTestId("meta-people-search").fill("");
  await page.getByTestId("meta-person-add-h-ben").click();
  await expect(page.getByTestId("meta-person-chip-h-ben")).toBeVisible();
  await page.getByTestId("meta-person-remove-h-ben").click();
  await expect(page.getByTestId("meta-person-chip-h-ben")).toHaveCount(0);
  await page.getByTestId("meta-person-add-h-cem").click();
  await expect(page.getByTestId("meta-people")).toContainText("Anna Berg");
  await expect(page.getByTestId("meta-people")).toContainText("Cem Aydin");

  // Projekte: "Kunde Stadtwerke" ab, "Privat" dazu.
  await expect(page.getByTestId("meta-project-f2")).toBeChecked();
  await page.getByTestId("meta-project-f3").uncheck();
  await page.getByTestId("meta-project-f1").check();

  await shoot(page, "nach-u7-metadaten-bearbeiten");
  await page.getByTestId("meta-save").click();
  // Zurueck in der Ansicht, mit den neuen Werten.
  await expect(page.getByTestId("meta-form")).toHaveCount(0);
  await expect(detailsRow(page, "title")).toHaveText(NEW_TITLE);
  await expect(detailsRow(page, "description")).toHaveText(NEW_DESCRIPTION);
  await expect(detailsRow(page, "people")).toHaveText("Anna Berg, Cem Aydin");
  await expect(detailsRow(page, "projects")).toContainText("Privat");
  await expect(detailsRow(page, "projects")).not.toContainText(
    "Kunde Stadtwerke",
  );
  await expect(detailsRow(page, "started")).toContainText("20.09.2026");
  await expect(detailsRow(page, "file")).toHaveText(
    "SWK-Lastgang-2026-09-28.m4a",
  );
  await shoot(page, "nach-u7-metadaten-details");

  // Ein Aufruf mit genau dem, was sich geaendert hat.
  const update = await calls(page, "meetings_update_metadata");
  expect(update).toHaveLength(1);
  const startedAt = await page.evaluate(() =>
    Math.floor(new Date("2026-09-20T09:30").getTime() / 1000),
  );
  expect(update[0].args).toEqual({
    meetingId: "m2",
    edit: {
      title: NEW_TITLE,
      description: NEW_DESCRIPTION,
      started_at: startedAt,
      participant_ids: ["h-anna", "h-cem"],
      folder_ids: ["f2", "f1"],
    },
  });

  // Kopf und Liste ziehen nach.
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("meeting-title")).toHaveText(NEW_TITLE);
  await expect(page.locator('[data-meeting-id="m2"]').first()).toContainText(
    NEW_TITLE,
  );

  // Nach dem Neuladen liest die Oberflaeche alles vom Backend: nichts davon ging verloren.
  await page.reload();
  await goToRecordings(page);
  await pickMeeting(page, "m2");
  await expect(page.getByTestId("meeting-title")).toHaveText(NEW_TITLE);
  await openDetails(page);
  await expect(detailsRow(page, "title")).toHaveText(NEW_TITLE);
  await expect(detailsRow(page, "description")).toHaveText(NEW_DESCRIPTION);
  await expect(detailsRow(page, "people")).toHaveText("Anna Berg, Cem Aydin");
  await expect(detailsRow(page, "projects")).toContainText("Privat");
  await expect(detailsRow(page, "started")).toContainText("20.09.2026");
  await expect(detailsRow(page, "file")).toHaveText(
    "SWK-Lastgang-2026-09-28.m4a",
  );
  // Die Bearbeitung zeigt die gespeicherten Werte als Vorgabe.
  await editMode(page);
  await expect(page.getByTestId("meta-title")).toHaveValue(NEW_TITLE);
  await expect(page.getByTestId("meta-description")).toHaveValue(
    NEW_DESCRIPTION,
  );
  await expect(page.getByTestId("meta-datetime")).toHaveValue(
    "2026-09-20T09:30",
  );
  await expect(page.getByTestId("meta-project-f1")).toBeChecked();
  await expect(page.getByTestId("meta-project-f3")).not.toBeChecked();
  await expect(page.getByTestId("meta-person-chip-h-anna")).toBeVisible();
});

test("nur Geaendertes geht an das Backend; die Beschreibung laesst sich wieder loeschen", async ({
  page,
}) => {
  await openM2(page);
  await openDetails(page);
  await editMode(page);
  await page.getByTestId("meta-description").fill("Nur eine Notiz");
  await page.getByTestId("meta-save").click();
  await expect(detailsRow(page, "description")).toHaveText("Nur eine Notiz");
  const first = (await calls(page, "meetings_update_metadata"))[0].args;
  expect(first.edit).toEqual({
    title: null,
    description: "Nur eine Notiz",
    started_at: null,
    participant_ids: null,
    folder_ids: null,
  });

  await editMode(page);
  await page.getByTestId("meta-description").fill("   ");
  await page.getByTestId("meta-save").click();
  await expect(detailsRow(page, "description")).toHaveText(
    "Keine Beschreibung",
  );
  const second = (await calls(page, "meetings_update_metadata"))[1].args;
  expect(second.edit.description).toBe("");
  expect(second.edit.title).toBeNull();
});

// ---------------------------------------------------------------------------
// Eingabepruefung und Fehler
// ---------------------------------------------------------------------------

test("leerer Titel, zu lange Beschreibung und ungueltiges Datum sperren das Speichern mit Hinweis", async ({
  page,
}) => {
  await openM2(page);
  await openDetails(page);
  await editMode(page);
  const save = page.getByTestId("meta-save");

  await page.getByTestId("meta-title").fill("   ");
  await expect(save).toBeDisabled();
  await expect(page.getByTestId("meta-form")).toContainText(
    "Der Titel darf nicht leer sein.",
  );
  await page.getByTestId("meta-title").fill("Neuer Titel");
  await expect(save).toBeEnabled();

  await page.getByTestId("meta-description").fill("x".repeat(4001));
  await expect(save).toBeDisabled();
  await expect(page.getByTestId("meta-description-count")).toHaveText(
    "4001 von 4000 Zeichen",
  );
  await page.getByTestId("meta-description").fill("x".repeat(4000));
  await expect(save).toBeEnabled();

  await page.getByTestId("meta-datetime").fill("");
  await expect(save).toBeDisabled();
  await expect(page.getByTestId("meta-form")).toContainText(
    "Das Datum ist ungültig.",
  );
  expect(await calls(page, "meetings_update_metadata")).toHaveLength(0);
});

test("Abbrechen und Esc verwerfen die Eingabe; ein Fehler des Backends laesst den Dialog mit der Eingabe offen", async ({
  page,
}) => {
  await openM2(page);
  await openDetails(page);
  await editMode(page);
  await page.getByTestId("meta-title").fill("Verworfen");
  await page.getByTestId("meta-cancel").click();
  await expect(page.getByTestId("meta-form")).toHaveCount(0);
  await expect(detailsRow(page, "title")).toHaveText(TITLE_M2);
  // Ein neuer Anlauf beginnt wieder mit den gespeicherten Werten.
  await editMode(page);
  await expect(page.getByTestId("meta-title")).toHaveValue(TITLE_M2);
  await page.keyboard.press("Escape");
  await expect(dialogOf(page)).toHaveCount(0);
  await openDetails(page);
  await expect(detailsRow(page, "title")).toHaveText(TITLE_M2);
  expect(await calls(page, "meetings_update_metadata")).toHaveLength(0);

  // Fehler des Backends (ein Projekt ist inzwischen weg): Meldung, Eingabe bleibt.
  await page.evaluate(() => ((window as any).__metaError = "folder_not_found"));
  await editMode(page);
  await page.getByTestId("meta-title").fill("Bleibt stehen");
  await page.getByTestId("meta-save").click();
  await expect(page.getByTestId("meta-error")).toContainText(
    "Eines der Projekte gibt es nicht mehr.",
  );
  await expect(page.getByTestId("meta-form")).toBeVisible();
  await expect(page.getByTestId("meta-title")).toHaveValue("Bleibt stehen");
  // Ohne den Fehler klappt es beim zweiten Versuch.
  await page.evaluate(() => ((window as any).__metaError = null));
  await page.getByTestId("meta-save").click();
  await expect(page.getByTestId("meta-form")).toHaveCount(0);
  await expect(detailsRow(page, "title")).toHaveText("Bleibt stehen");
});

test("Tastatur: Enter im Titel speichert, Enter in der Personensuche waehlt den ersten Vorschlag", async ({
  page,
}) => {
  await openM2(page);
  await openDetails(page);
  await editMode(page);
  await page.getByTestId("meta-people-search").fill("ben");
  await page.getByTestId("meta-people-search").press("Enter");
  await expect(page.getByTestId("meta-person-chip-h-ben")).toBeVisible();
  await expect(page.getByTestId("meta-people-search")).toHaveValue("");
  // Die Suche zeigt nur Personen, die noch nicht gewaehlt sind.
  await expect(page.getByTestId("meta-person-add-h-ben")).toHaveCount(0);
  await page.getByTestId("meta-title").fill("Per Enter gespeichert");
  await page.getByTestId("meta-title").press("Enter");
  await expect(page.getByTestId("meta-form")).toHaveCount(0);
  await expect(detailsRow(page, "title")).toHaveText("Per Enter gespeichert");
  await expect(detailsRow(page, "people")).toHaveText("Ben Koch");
});

test("Suche ohne Treffer zeigt einen Hinweis, eine unbekannte Person wird nicht angelegt", async ({
  page,
}) => {
  await openM2(page);
  await openDetails(page);
  await editMode(page);
  await page.getByTestId("meta-people-search").fill("zzz");
  await expect(page.getByTestId("meta-suggestions")).toContainText(
    "Keine Person gefunden.",
  );
  await page.getByTestId("meta-people-search").press("Enter");
  await expect(page.getByTestId("meta-people")).toContainText(
    "Keine Person ausgewählt",
  );
});

// ---------------------------------------------------------------------------
// Schmales Fenster
// ---------------------------------------------------------------------------

test("schmales Fenster (480): der Bearbeiten-Dialog passt in die Breite und Speichern bleibt erreichbar", async ({
  page,
}) => {
  await openM2(page, 480, 800);
  await openDetails(page);
  await editMode(page);
  const dialog = dialogOf(page);
  const box = (await dialog.boundingBox())!;
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.x + box.width).toBeLessThanOrEqual(480 + 1);
  await page.getByTestId("meta-title").fill("Schmal");
  await shoot(page, "nach-u7-metadaten-480");
  await page.getByTestId("meta-save").scrollIntoViewIfNeeded();
  await expect(page.getByTestId("meta-save")).toBeInViewport();
  const overflow = await page.evaluate(
    () => document.scrollingElement!.scrollWidth - window.innerWidth,
  );
  expect(overflow).toBeLessThanOrEqual(1);
  await page.getByTestId("meta-save").click();
  await expect(detailsRow(page, "title")).toHaveText("Schmal");
});
