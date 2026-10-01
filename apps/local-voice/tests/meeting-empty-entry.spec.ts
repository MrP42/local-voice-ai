import { test, expect, type Page } from "@playwright/test";
import * as path from "node:path";
import { calls, openRecordings } from "./recLayoutMock";
import { confirmStartDialog, holdImport, installLiveMock } from "./recLiveMock";
import { installYoutubeMock, YT_ID } from "./youtubeMock";
import { installEmptyEntryMock, refuseTarget } from "./emptyEntryMock";

// G1 (#70): "Neue Besprechung" ohne Audio. Ein leerer Eintrag liegt im gewaehlten
// Projekt, ist sofort umbenennbar, hat nutzbare Notizen und eine Startflaeche;
// Aufnahme, Datei und YouTube-Link fuellen DENSELBEN Eintrag. Die Regeln des
// Backends (Ablehnung eines nicht leeren Ziels, Titel, Liste, Suche, MCP, Export)
// pruefen die Rust-Tests; hier die Oberflaeche gegen die Tauri-Attrappe.

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

const SHOT_DIR = path.resolve(
  process.cwd(),
  "../../koordination/issues-abschluss/screens",
);
/** Bilder nur auf Wunsch (LVA_SCREENSHOTS=1), sonst schreibt jeder Lauf sie neu. */
const shoot = async (page: Page, name: string) => {
  if (!process.env.LVA_SCREENSHOTS) return;
  await page.waitForTimeout(300);
  await page.screenshot({
    path: path.join(SHOT_DIR, `${name}.png`),
    animations: "disabled",
  });
};

const DEFAULT_TITLE = "Neue Besprechung";
const FIRST = "m-empty1";

const setup = async (page: Page) => {
  await installLiveMock(page);
  await installYoutubeMock(page, { existing: false });
  await installEmptyEntryMock(page);
  await openRecordings(page, 1366, 768);
};

const rowOf = (page: Page, id: string) =>
  page.locator(`[data-meeting-id="${id}"]`);
const projectRow = (page: Page, id: string) =>
  page.locator(`[data-project-id="${id}"]`);

/** Projekt waehlen und dort "Neue Besprechung" ueber den Spaltenkopf anlegen. */
const createInProject = async (page: Page, projectId: string) => {
  await projectRow(page, projectId).click();
  await page.getByTestId("projects-new-meeting").click();
  await expect(page.getByTestId("empty-start")).toBeVisible();
};

const idsOf = (page: Page) =>
  page.evaluate(() => (window as any).__meetings.map((m: any) => m.id));

// ---------------------------------------------------------------------------
// Anlegen
// ---------------------------------------------------------------------------

test("Plus im Spaltenkopf legt im gewaehlten Projekt einen leeren Eintrag an, oeffnet ihn und macht den Titel umbenennbar", async ({
  page,
}) => {
  await setup(page);
  await createInProject(page, "f3");

  const created = await calls(page, "meetings_create_empty");
  expect(created).toHaveLength(1);
  expect(created[0].args).toEqual({ title: DEFAULT_TITLE, folderId: "f3" });

  // Gewaehlt und in der Mitte geoeffnet; der Titel steht gleich im Eingabefeld.
  await expect(rowOf(page, FIRST)).toHaveAttribute("aria-current", "true");
  const input = page.getByTestId("meeting-title-input");
  await expect(input).toBeFocused();
  await expect(input).toHaveValue(DEFAULT_TITLE);
  await expect(page.getByTestId("status-chip")).toHaveAttribute(
    "data-state",
    "empty",
  );
  await expect(page.getByTestId("status-chip")).toHaveText("Leer");
  await expect(rowOf(page, FIRST).getByTestId("row-status")).toHaveText("Leer");
  // Im Projekt, nicht daneben.
  await expect(projectRow(page, "f3").getByTestId("project-count")).toHaveText(
    "2",
  );
  await expect(page.getByTestId("project-chip")).toContainText(
    "Kunde Stadtwerke",
  );
  // Es gibt keinen Transkript-Platzhalter einer Aufnahme.
  await expect(page.getByTestId("mid-panel-transcript")).toContainText(
    "Das Transkript erscheint hier, sobald du aufnimmst oder eine Datei importierst.",
  );
  await shoot(page, "g1-anlegen");

  // Inline umbenennen: Enter speichert.
  await input.fill("Kick-off Müller");
  await input.press("Enter");
  await expect(page.getByTestId("meeting-title")).toHaveText("Kick-off Müller");
  await expect(rowOf(page, FIRST)).toContainText("Kick-off Müller");
  const renamed = await calls(page, "meetings_rename");
  expect(renamed.at(-1)!.args).toEqual({
    meetingId: FIRST,
    title: "Kick-off Müller",
  });
});

test("in 'Alle Aufnahmen' und 'Ohne Projekt' entsteht ein Eintrag ohne Projekt", async ({
  page,
}) => {
  await setup(page);
  await page.getByTestId("projects-new-meeting").click();
  await expect(page.getByTestId("empty-start")).toBeVisible();
  expect((await calls(page, "meetings_create_empty"))[0].args).toEqual({
    title: DEFAULT_TITLE,
    folderId: null,
  });
  await expect(page.getByTestId("project-chip")).toHaveCount(0);

  await projectRow(page, "none").click();
  await page.getByTestId("projects-new-meeting").click();
  await expect
    .poll(async () => (await calls(page, "meetings_create_empty")).length)
    .toBe(2);
  expect((await calls(page, "meetings_create_empty"))[1].args).toEqual({
    title: DEFAULT_TITLE,
    folderId: null,
  });
  await expect(rowOf(page, "m-empty2")).toBeVisible();
});

test("Kontextmenue 'Neue Besprechung hier' legt im angeklickten Projekt an und wechselt dorthin", async ({
  page,
}) => {
  await setup(page);
  // Gewaehlt ist "Alle Aufnahmen"; das Menue gehoert dem Projekt "Podcast".
  await projectRow(page, "f4").click({ button: "right" });
  const menu = page.getByRole("menu");
  await expect(menu).toBeVisible();
  await menu
    .getByRole("menuitem", { name: "Neue Besprechung hier", exact: true })
    .evaluate((el) => (el as HTMLElement).click());
  await expect(page.getByTestId("empty-start")).toBeVisible();
  expect((await calls(page, "meetings_create_empty"))[0].args).toEqual({
    title: DEFAULT_TITLE,
    folderId: "f4",
  });
  await expect(projectRow(page, "f4")).toHaveAttribute("aria-current", "true");
  await expect(rowOf(page, FIRST)).toBeVisible();

  // Auch die Zeile "Ohne Projekt" bietet es an (ohne Projekt).
  await projectRow(page, "none").click({ button: "right" });
  await page
    .getByRole("menu")
    .getByRole("menuitem", { name: "Neue Besprechung hier", exact: true })
    .evaluate((el) => (el as HTMLElement).click());
  await expect
    .poll(async () => (await calls(page, "meetings_create_empty")).length)
    .toBe(2);
  expect((await calls(page, "meetings_create_empty"))[1].args).toEqual({
    title: DEFAULT_TITLE,
    folderId: null,
  });
});

// ---------------------------------------------------------------------------
// Notizen und Loeschen
// ---------------------------------------------------------------------------

test("Notizen lassen sich sofort tippen und werden gespeichert", async ({
  page,
}) => {
  await setup(page);
  await createInProject(page, "f2");
  await page.getByTestId("meeting-title-input").press("Enter");
  await expect(page.getByTestId("my-notes")).toBeVisible();
  const starter = page.getByTestId("note-starter");
  await starter.click();
  await page.keyboard.type("Agenda: Preis, Termin, Rollen", { delay: 5 });
  await expect
    .poll(async () => (await calls(page, "meeting_notes_save")).length, {
      timeout: 8_000,
    })
    .toBeGreaterThan(0);
  const saves = await calls(page, "meeting_notes_save");
  const last = JSON.stringify(saves.at(-1)!.args);
  expect(last).toContain(FIRST);
  expect(last).toContain("Agenda: Preis, Termin, Rollen");
});

test("Loeschen: der leere Eintrag verschwindet aus Liste und Mitte", async ({
  page,
}) => {
  await setup(page);
  await createInProject(page, "f3");
  await page.getByTestId("meeting-title-input").press("Enter");
  await rowOf(page, FIRST).click({ button: "right" });
  await page
    .getByRole("menu")
    .getByRole("menuitem", { name: "Löschen", exact: true })
    .evaluate((el) => (el as HTMLElement).click());
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Löschen", exact: true })
    .click();
  await expect(rowOf(page, FIRST)).toHaveCount(0);
  await expect(page.getByTestId("empty-start")).toHaveCount(0);
  expect((await calls(page, "meetings_delete"))[0].args).toEqual({
    meetingId: FIRST,
  });
  expect(await idsOf(page)).not.toContain(FIRST);
});

// ---------------------------------------------------------------------------
// Fuellen: Aufnahme, Datei, Link
// ---------------------------------------------------------------------------

test("Aufnahme starten fuellt denselben Eintrag (Einwilligung bleibt Pflicht, Projekt bleibt)", async ({
  page,
}) => {
  await setup(page);
  await createInProject(page, "f3");
  await page.getByTestId("meeting-title-input").press("Enter");
  const before = (await idsOf(page)).length;

  await page.getByTestId("empty-record").click();
  const dialog = page.getByTestId("start-dialog");
  await expect(dialog).toBeVisible();
  // Der Dialog nennt den Eintrag, zeigt dessen Titel und fragt kein Projekt.
  await expect(dialog.getByTestId("start-target")).toContainText(
    `„${DEFAULT_TITLE}“`,
  );
  await expect(dialog.getByTestId("start-title")).toHaveValue(DEFAULT_TITLE);
  await expect(dialog.getByTestId("start-project")).toHaveCount(0);
  await shoot(page, "g1-aufnahme-dialog");
  // Ohne die Einwilligung startet nichts.
  expect(await calls(page, "meetings_start")).toHaveLength(0);

  await dialog.getByTestId("start-title").fill("Montags-Runde");
  await confirmStartDialog(page);

  const started = await calls(page, "meetings_start");
  expect(started).toHaveLength(1);
  expect(started[0].args).toEqual({
    title: "Montags-Runde",
    consentConfirmed: true,
    captureSystem: true,
    targetMeetingId: FIRST,
  });
  // Derselbe Eintrag, jetzt eine laufende Aufnahme; keine zweite Besprechung.
  await expect(page.getByTestId("status-chip")).toHaveAttribute(
    "data-state",
    "recording",
  );
  await expect(rowOf(page, FIRST)).toContainText("Montags-Runde");
  await expect(page.getByTestId("empty-start")).toHaveCount(0);
  await expect(page.getByTestId("rec-live")).toBeVisible();
  await expect(page.getByTestId("project-chip")).toContainText(
    "Kunde Stadtwerke",
  );
  expect((await idsOf(page)).length).toBe(before);
  // Das Projekt wurde nicht neu zugeordnet.
  expect(await calls(page, "meetings_set_folders")).toHaveLength(0);
  await shoot(page, "g1-aufnahme-laeuft");
});

test("Die Aufnahmekarte rechts startet ebenfalls in den gewaehlten leeren Eintrag", async ({
  page,
}) => {
  await setup(page);
  await createInProject(page, "f2");
  await page.getByTestId("meeting-title-input").press("Enter");
  await page
    .getByTestId("rec-start")
    .getByRole("button", { name: /Aufnahme starten/ })
    .click();
  await expect(page.getByTestId("start-target")).toBeVisible();
  await confirmStartDialog(page);
  const started = await calls(page, "meetings_start");
  expect(started[0].args).toMatchObject({
    title: DEFAULT_TITLE,
    targetMeetingId: FIRST,
  });
});

test("Ohne leeren Eintrag legt die Aufnahme weiter eine neue Besprechung an", async ({
  page,
}) => {
  await setup(page);
  await page.locator('[data-meeting-id="m1"]').click();
  await expect(page.getByTestId("empty-start")).toHaveCount(0);
  await page
    .getByTestId("rec-start")
    .getByRole("button", { name: /Aufnahme starten/ })
    .click();
  await expect(page.getByTestId("start-project")).toBeVisible();
  await expect(page.getByTestId("start-target")).toHaveCount(0);
  await confirmStartDialog(page);
  const started = await calls(page, "meetings_start");
  expect((started[0].args as any).targetMeetingId).toBeNull();
});

test("Datei importieren fuellt denselben Eintrag (Titel ersetzt, solange er der vorgeschlagene ist)", async ({
  page,
}) => {
  await setup(page);
  await holdImport(page);
  await createInProject(page, "f3");
  await page.getByTestId("meeting-title-input").press("Enter");
  const before = (await idsOf(page)).length;

  await page.evaluate(
    (p) => ((window as any).__pick = p),
    "C:/Audio/Montag Runde.m4a",
  );
  await page.getByTestId("empty-import").click();
  const dialog = page.getByTestId("import-dialog");
  await expect(dialog).toContainText("Montag Runde.m4a");
  await expect(dialog.getByTestId("import-target")).toContainText(
    `„${DEFAULT_TITLE}“`,
  );
  await shoot(page, "g1-import-dialog");
  await confirmStartDialog(page);
  await expect(dialog).toHaveCount(0);

  const imports = await calls(page, "meetings_import_file");
  expect(imports).toHaveLength(1);
  expect(imports[0].args).toEqual({
    path: "C:/Audio/Montag Runde.m4a",
    consentConfirmed: true,
    targetMeetingId: FIRST,
  });
  // Derselbe Eintrag, jetzt ein Import mit dem Dateinamen als Titel.
  await expect(page.getByTestId("meeting-title")).toHaveText("Montag Runde");
  await expect(page.getByTestId("empty-start")).toHaveCount(0);
  await expect(rowOf(page, FIRST)).toContainText("Montag Runde");
  await expect(page.getByTestId("source-chip")).toContainText("Import");
  await expect(page.getByTestId("project-chip")).toContainText(
    "Kunde Stadtwerke",
  );
  expect((await idsOf(page)).length).toBe(before);
  expect(await calls(page, "meetings_set_folders")).toHaveLength(0);
  await shoot(page, "g1-import-laeuft");
});

test("Ein umbenannter Eintrag behaelt seinen Titel, wenn eine Datei kommt; weitere Dateien werden neue Besprechungen", async ({
  page,
}) => {
  await setup(page);
  await holdImport(page);
  await createInProject(page, "f3");
  const input = page.getByTestId("meeting-title-input");
  await input.fill("Mein Kick-off");
  await input.press("Enter");
  await expect(page.getByTestId("meeting-title")).toHaveText("Mein Kick-off");

  // Zwei Dateien auf einmal, abgelegt auf der Arbeitsflaeche.
  const box = (await page.getByTestId("rec-content").boundingBox())!;
  await page.evaluate(
    ({ x, y }) =>
      (window as any).__emit("tauri://drag-drop", {
        paths: ["C:/Audio/Erste.m4a", "C:/Audio/Zweite.m4a"],
        position: { x, y },
      }),
    { x: box.x + box.width / 2, y: box.y + box.height / 2 },
  );
  const dialog = page.getByTestId("import-dialog");
  await expect(dialog.getByTestId("import-target")).toContainText(
    "Die erste Datei füllt den Eintrag „Mein Kick-off“",
  );
  await confirmStartDialog(page);
  await expect(dialog).toHaveCount(0);

  const imports = await calls(page, "meetings_import_file");
  expect(imports.map((c) => (c.args as any).targetMeetingId)).toEqual([
    FIRST,
    null,
  ]);
  await expect(page.getByTestId("meeting-title")).toHaveText("Mein Kick-off");
  await expect(rowOf(page, FIRST)).toBeVisible();
  await expect(rowOf(page, "m-imp1")).toBeVisible();
});

test("Link einfuegen fuellt denselben Eintrag (Dialog nennt ihn, fragt kein Projekt)", async ({
  page,
}) => {
  await setup(page);
  await createInProject(page, "f3");
  await page.getByTestId("meeting-title-input").press("Enter");
  const before = (await idsOf(page)).length;

  await page.getByTestId("empty-link").click();
  const dialog = page.getByRole("dialog", { name: "YouTube-Link einfügen" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByTestId("yt-link-target")).toContainText(
    `„${DEFAULT_TITLE}“`,
  );
  await expect(dialog.getByTestId("yt-link-project")).toHaveCount(0);
  const link = `https://youtu.be/${YT_ID}`;
  await dialog.getByTestId("yt-link-input").fill(link);
  await expect(dialog.getByTestId("yt-link-check")).toContainText(
    "Video erkannt",
  );
  await shoot(page, "g1-link-dialog");
  await dialog.getByTestId("yt-add").click();
  await expect(dialog).toHaveCount(0);

  const added = await calls(page, "youtube_add_source");
  expect(added).toHaveLength(1);
  expect(added[0].args).toEqual({
    url: link,
    projectId: null,
    targetMeetingId: FIRST,
  });
  // Derselbe Eintrag, jetzt mit YouTube-Quelle und dem Titel des Videos.
  await expect(page.getByTestId("empty-start")).toHaveCount(0);
  await expect(page.getByTestId("meeting-title")).toHaveText(
    "Lastgang verstehen: Spitzen glätten",
  );
  await expect(page.getByTestId("source-chip")).toContainText("YouTube");
  await expect(page.getByTestId("project-chip")).toContainText(
    "Kunde Stadtwerke",
  );
  expect((await idsOf(page)).length).toBe(before);
});

test("Ein nicht mehr leeres Ziel wird abgelehnt und gemeldet", async ({
  page,
}) => {
  await setup(page);
  await createInProject(page, "f3");
  await page.getByTestId("meeting-title-input").press("Enter");
  await refuseTarget(page);
  await page.getByTestId("empty-record").click();
  await confirmStartDialog(page);
  await expect(
    page.getByText("Dieser Eintrag ist nicht mehr leer"),
  ).toBeVisible();
  // Nichts hat sich geaendert: der Eintrag ist noch ein Eintrag ohne Aufnahme.
  await expect(page.getByTestId("status-chip")).toHaveAttribute(
    "data-state",
    "empty",
  );
});

test("Im schmalen Fenster steht die Startflaeche ebenfalls da und der Titel ist umbenennbar", async ({
  page,
}) => {
  await installLiveMock(page);
  await installYoutubeMock(page, { existing: false });
  await installEmptyEntryMock(page);
  await openRecordings(page, 480, 800);
  await page.getByTestId("sessions-open").click();
  await page.getByTestId("projects-new-meeting").click();
  await expect(page.getByTestId("empty-start")).toBeVisible();
  await expect(page.getByTestId("meeting-title-input")).toBeFocused();
});
