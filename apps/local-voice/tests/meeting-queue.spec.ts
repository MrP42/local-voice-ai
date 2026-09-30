import { test, expect, type Locator, type Page } from "@playwright/test";
import * as path from "node:path";
import { calls, installTauriMock } from "./calendarMock";
import { PROGRESS, emitMeeting, openRecordings } from "./recLayoutMock";
import {
  confirmStartDialog,
  finishImport,
  holdImport,
  installLiveMock,
  setQueueLimit,
  setQueueMemory,
  setQueueRecording,
} from "./recLiveMock";

// U7 (Issue #64): Import-Warteschlange gegen die Tauri-Attrappe. Waehrend eine
// Transkription laeuft, lassen sich weitere Dateien importieren (Symbol, Ablage,
// mehrere auf einmal): sie bekommen sofort eine Besprechung mit Status "wartet"
// und ihren Platz, werden in der Reihenfolge des Hinzufuegens abgearbeitet und
// lassen sich nach vorn ziehen, herausnehmen und stoppen. Die Attrappe spielt die
// Warteschlange des Backends nach (recLiveMock); die Regeln selbst (Reihenfolge,
// Gleichzeitigkeit, Speicher-Tor, Aufnahme-Vorrang, Neustart) prueft Rust.

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
  await installLiveMock(page);
});

const rowOf = (page: Page, id: string) =>
  page.locator(`[data-meeting-id="${id}"]`);

const chipOf = (page: Page, id: string) =>
  rowOf(page, id).getByTestId("queue-chip");

const snapshot = (page: Page) =>
  page.evaluate(() => (window as any).__queueSnapshot());

const pick = (page: Page, path: string) =>
  page.evaluate((p) => ((window as any).__pick = p), path);

/** Eine Datei ueber das Symbol importieren (Dateiwahl, Einwilligung). */
const importOne = async (page: Page, path: string) => {
  await pick(page, path);
  await page.getByTestId("import-open").click();
  await expect(page.getByTestId("import-dialog")).toContainText(
    path.split("/").pop()!,
  );
  await confirmStartDialog(page);
  await expect(page.getByTestId("import-dialog")).toHaveCount(0);
};

/** Datei-Ablage, wie Tauri sie meldet (physische Pixel; Playwright hat Faktor 1). */
const dropAt = (page: Page, x: number, y: number, paths: string[]) =>
  page.evaluate(
    ({ x, y, paths }) =>
      (window as any).__emit("tauri://drag-drop", {
        paths,
        position: { x, y },
      }),
    { x, y, paths },
  );

const centerOf = async (locator: Locator) => {
  const b = (await locator.boundingBox())!;
  return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
};

/** Drei Importe: A laeuft, B und C warten (A haelt, bis der Test es beendet). */
const startWithThree = async (page: Page) => {
  await openRecordings(page, 1366, 768);
  await holdImport(page);
  await importOne(page, "C:/Audio/Datei A.m4a");
  await expect(rowOf(page, "m-imp1")).toBeVisible();
  await expect
    .poll(async () => (await snapshot(page)).running)
    .toEqual(["m-imp1"]);
  const inside = await centerOf(page.getByTestId("rec-content"));
  await dropAt(page, inside.x, inside.y, [
    "C:/Audio/Datei B.m4a",
    "C:/Audio/Datei C.m4a",
  ]);
  const dialog = page.getByTestId("import-dialog");
  await expect(dialog).toContainText("Datei B.m4a");
  await expect(dialog).toContainText("Datei C.m4a");
  // Der Einwilligungsknopf ist NICHT gesperrt, obwohl A noch transkribiert wird.
  await expect(page.getByTestId("import-confirm")).toBeEnabled();
  await confirmStartDialog(page);
  await expect(dialog).toHaveCount(0);
};

// ---------------------------------------------------------------------------
// Hinzufuegen waehrend eine Transkription laeuft
// ---------------------------------------------------------------------------

test("waehrend A transkribiert: zwei weitere Dateien in einem Zug -> wartend mit Platz, Einwilligung und Symbol bleiben bedienbar", async ({
  page,
}) => {
  await startWithThree(page);

  // In der Reihenfolge der Angabe eingereiht, jede mit Einwilligung.
  const imports = await calls(page, "meetings_import_file");
  expect(imports.map((c) => c.args)).toEqual([
    { path: "C:/Audio/Datei A.m4a", consentConfirmed: true },
    { path: "C:/Audio/Datei B.m4a", consentConfirmed: true },
    { path: "C:/Audio/Datei C.m4a", consentConfirmed: true },
  ]);
  // A laeuft, B und C haben sofort ihre Besprechung mit Status "wartet".
  await expect(rowOf(page, "m-imp2")).toBeVisible();
  await expect(rowOf(page, "m-imp3")).toBeVisible();
  await expect(chipOf(page, "m-imp2")).toHaveText("Wartet · Platz 1 von 2");
  await expect(chipOf(page, "m-imp3")).toHaveText("Wartet · Platz 2 von 2");
  await expect(chipOf(page, "m-imp1")).toHaveCount(0);

  // Die laufende Datei zeigt wie bisher ihren Fortschritt (P8a).
  await emitMeeting(page, { ...PROGRESS, meeting_id: "m-imp1" });
  await expect(rowOf(page, "m-imp1").getByTestId("job-bar")).toBeVisible();

  // Die wartende Datei: im Kopf steht derselbe Platz, in der Bedienspalte der Grund.
  await rowOf(page, "m-imp2").click();
  await expect(page.getByTestId("status-chip")).toHaveAttribute(
    "data-state",
    "queued",
  );
  await expect(page.getByTestId("status-chip")).toHaveText(
    "Wartet · Platz 1 von 2",
  );
  const panel = page.getByTestId("queue-panel");
  await expect(panel).toBeVisible();
  await expect(panel.getByTestId("queue-place")).toHaveText(
    "Wartet · Platz 1 von 2",
  );
  await expect(panel.getByTestId("queue-reason")).toContainText(
    "Eine andere Transkription läuft",
  );
  // Platz 1: nach vorn ziehen ist nichts mehr zu tun.
  await expect(panel.getByTestId("queue-to-front")).toBeDisabled();
  await shoot(page, "nach-u7-warteschlange");

  // Weiter importieren geht jederzeit: Symbol frei, der Dialog laesst sich bestaetigen.
  await expect(page.getByTestId("import-open")).toBeEnabled();
  await importOne(page, "C:/Audio/Datei D.m4a");
  await expect(chipOf(page, "m-imp4")).toHaveText("Wartet · Platz 3 von 3");
});

test("Reihenfolge des Hinzufuegens: jede Datei beginnt erst, wenn die davor fertig ist", async ({
  page,
}) => {
  await startWithThree(page);
  expect((await snapshot(page)).waiting).toEqual(["m-imp2", "m-imp3"]);

  await finishImport(page, "m-imp1");
  await expect
    .poll(async () => (await snapshot(page)).running)
    .toEqual(["m-imp2"]);
  await expect(chipOf(page, "m-imp2")).toHaveCount(0);
  await expect(chipOf(page, "m-imp3")).toHaveText("Wartet · Platz 1 von 1");

  await finishImport(page, "m-imp2");
  await expect
    .poll(async () => (await snapshot(page)).running)
    .toEqual(["m-imp3"]);
  await finishImport(page, "m-imp3");
  await expect
    .poll(async () => await snapshot(page))
    .toMatchObject({ running: [], waiting: [], blocked: null });
  for (const id of ["m-imp1", "m-imp2", "m-imp3"]) {
    await expect(rowOf(page, id)).toContainText("Fertig");
  }
});

// ---------------------------------------------------------------------------
// Nach vorn ziehen, herausnehmen, stoppen
// ---------------------------------------------------------------------------

/** Kontextmenue einer Zeile oeffnen und einen Eintrag waehlen (per JS-Klick: das Menue schliesst bei Scroll). */
const chooseRowMenu = async (page: Page, id: string, label: string) => {
  await rowOf(page, id).click({ button: "right" });
  const menu = page.getByRole("menu");
  await expect(menu).toBeVisible();
  await menu
    .getByRole("menuitem", { name: label, exact: true })
    .evaluate((el) => (el as HTMLElement).click());
};

test("Kontextmenue: nach vorn ziehen und aus der Warteschlange nehmen", async ({
  page,
}) => {
  await startWithThree(page);
  await importOne(page, "C:/Audio/Datei D.m4a");
  expect((await snapshot(page)).waiting).toEqual([
    "m-imp2",
    "m-imp3",
    "m-imp4",
  ]);

  // D nach vorn: D wird Platz 1, die anderen ruecken nach.
  await chooseRowMenu(page, "m-imp4", "Nach vorn ziehen");
  await expect
    .poll(async () => (await snapshot(page)).waiting)
    .toEqual(["m-imp4", "m-imp2", "m-imp3"]);
  await expect(chipOf(page, "m-imp4")).toHaveText("Wartet · Platz 1 von 3");
  await expect(chipOf(page, "m-imp2")).toHaveText("Wartet · Platz 2 von 3");
  expect((await calls(page, "meetings_queue_to_front"))[0].args).toEqual({
    meetingId: "m-imp4",
  });

  // Platz 1 hat keinen Eintrag "Nach vorn ziehen" mehr, der Rest des Menues bleibt.
  await rowOf(page, "m-imp4").click({ button: "right" });
  await expect(
    page.getByRole("menuitem", { name: "Nach vorn ziehen", exact: true }),
  ).toBeDisabled();
  await page.keyboard.press("Escape");

  // B herausnehmen: die Datei wird abgebrochen, der Rest behaelt die Reihenfolge.
  await chooseRowMenu(page, "m-imp2", "Aus der Warteschlange nehmen");
  await expect
    .poll(async () => (await snapshot(page)).waiting)
    .toEqual(["m-imp4", "m-imp3"]);
  await expect(rowOf(page, "m-imp2")).toContainText("Abgebrochen");
  await expect(chipOf(page, "m-imp2")).toHaveCount(0);
  await expect(chipOf(page, "m-imp3")).toHaveText("Wartet · Platz 2 von 2");

  // A endet: D (vorgezogen) beginnt, B nie.
  await finishImport(page, "m-imp1");
  await expect
    .poll(async () => (await snapshot(page)).running)
    .toEqual(["m-imp4"]);
  expect((await snapshot(page)).waiting).toEqual(["m-imp3"]);
});

test("Bedienspalte einer wartenden Datei: nach vorn ziehen und entfernen mit Symbolen", async ({
  page,
}) => {
  await startWithThree(page);
  await rowOf(page, "m-imp3").click();
  const panel = page.getByTestId("queue-panel");
  await expect(panel.getByTestId("queue-place")).toHaveText(
    "Wartet · Platz 2 von 2",
  );
  await panel.getByTestId("queue-to-front").click();
  await expect(panel.getByTestId("queue-place")).toHaveText(
    "Wartet · Platz 1 von 2",
  );
  await expect(page.getByTestId("status-chip")).toHaveText(
    "Wartet · Platz 1 von 2",
  );
  await expect(panel.getByTestId("queue-to-front")).toBeDisabled();

  await panel.getByTestId("queue-remove").click();
  await expect(page.getByTestId("queue-panel")).toHaveCount(0);
  await expect(page.getByTestId("status-chip")).toHaveAttribute(
    "data-state",
    "cancelled",
  );
  expect((await snapshot(page)).waiting).toEqual(["m-imp2"]);
});

test('herausgenommene Datei: "Wieder einreihen" stellt sie hinten an und sie laeuft wieder mit', async ({
  page,
}) => {
  await startWithThree(page);
  await chooseRowMenu(page, "m-imp2", "Aus der Warteschlange nehmen");
  await expect(rowOf(page, "m-imp2")).toContainText("Abgebrochen");
  await rowOf(page, "m-imp2").click();
  const panel = page.getByTestId("cancelled-panel");
  await expect(panel).toContainText("Aus der Warteschlange genommen");
  await panel.getByTestId("job-continue").click();
  expect((await calls(page, "meetings_continue"))[0].args).toEqual({
    meetingId: "m-imp2",
  });
  // Hinten angestellt: hinter C, nicht an ihrer alten Stelle.
  await expect
    .poll(async () => (await snapshot(page)).waiting)
    .toEqual(["m-imp3", "m-imp2"]);
  await expect(chipOf(page, "m-imp2")).toHaveText("Wartet · Platz 2 von 2");
  await finishImport(page, "m-imp1");
  await expect
    .poll(async () => (await snapshot(page)).running)
    .toEqual(["m-imp3"]);
});

test("Stopp einer laufenden Datei: nur sie endet als abgebrochen, die naechste beginnt", async ({
  page,
}) => {
  await startWithThree(page);
  await rowOf(page, "m-imp1").click();
  await emitMeeting(page, { ...PROGRESS, meeting_id: "m-imp1" });
  await page.getByTestId("job-stop").click();
  await page.getByTestId("job-stop-confirm").click();
  await expect(rowOf(page, "m-imp1")).toContainText("Abgebrochen");
  await expect
    .poll(async () => (await snapshot(page)).running)
    .toEqual(["m-imp2"]);
  // B und C: B laeuft jetzt, C ist dran.
  await expect(chipOf(page, "m-imp3")).toHaveText("Wartet · Platz 1 von 1");
  expect(await calls(page, "meetings_job_stop")).toHaveLength(1);
});

// ---------------------------------------------------------------------------
// Warum die Schlange steht
// ---------------------------------------------------------------------------

test("Aufnahme hat Vorrang: laufende Importe stehen angehalten, wartende beginnen nicht; danach geht es weiter", async ({
  page,
}) => {
  await startWithThree(page);
  await emitMeeting(page, { ...PROGRESS, meeting_id: "m-imp1" });
  await setQueueRecording(page, true);
  // Der laufende Import steht am naechsten Block still (Pause) und sagt warum.
  await emitMeeting(page, {
    ...PROGRESS,
    meeting_id: "m-imp1",
    state: "paused",
  });
  await expect(rowOf(page, "m-imp1").getByTestId("job-bar-note")).toHaveText(
    "Angehalten – eine Aufnahme läuft",
  );
  await rowOf(page, "m-imp2").click();
  await expect(page.getByTestId("queue-reason")).toContainText(
    "Eine Aufnahme läuft und hat Vorrang",
  );
  await expect(page.getByTestId("queue-panel")).toHaveAttribute(
    "data-reason",
    "recording",
  );
  // Nichts Neues beginnt, auch wenn ein Lauf endet.
  await finishImport(page, "m-imp1");
  expect((await snapshot(page)).running).toEqual([]);

  await setQueueRecording(page, false);
  await expect
    .poll(async () => (await snapshot(page)).running)
    .toEqual(["m-imp2"]);
  await expect(page.getByTestId("queue-panel")).toHaveCount(0);
});

test("ohne Arbeitsspeicher fuer einen weiteren Lauf: die naechste Datei wartet mit dem Hinweis, bis Speicher frei ist", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  await holdImport(page);
  await setQueueLimit(page, 2);
  await setQueueMemory(page, true);
  await importOne(page, "C:/Audio/Datei A.m4a");
  await importOne(page, "C:/Audio/Datei B.m4a");
  await expect
    .poll(async () => await snapshot(page))
    .toMatchObject({
      running: ["m-imp1"],
      waiting: ["m-imp2"],
      blocked: "memory",
    });
  await rowOf(page, "m-imp2").click();
  await expect(page.getByTestId("queue-reason")).toContainText(
    "Wartet auf Arbeitsspeicher",
  );
  await expect(chipOf(page, "m-imp2")).toHaveAttribute("data-reason", "memory");
  await shoot(page, "nach-u7-wartet-auf-speicher");
  // Es ist frei: die zweite Datei beginnt gleichzeitig (Limit 2).
  await setQueueMemory(page, false);
  await expect
    .poll(async () => (await snapshot(page)).running)
    .toEqual(["m-imp1", "m-imp2"]);
  await expect(chipOf(page, "m-imp2")).toHaveCount(0);
});

test("Hydrierung: nach dem Wechsel der Seite und zurueck zeigen die wartenden Dateien weiter ihren Platz", async ({
  page,
}) => {
  await startWithThree(page);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Einstellungen", exact: true })
    .click();
  await expect(page.getByTestId("rec-content")).toHaveCount(0);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Aufnahmen", exact: true })
    .click();
  await expect(chipOf(page, "m-imp2")).toHaveText("Wartet · Platz 1 von 2");
  await expect(chipOf(page, "m-imp3")).toHaveText("Wartet · Platz 2 von 2");
});

test("schmales Fenster (480): Platz und Bedienspalte der wartenden Datei ohne Seiten-Scroll", async ({
  page,
}) => {
  await openRecordings(page, 480, 800);
  await holdImport(page);
  // Im schmalen Fenster sitzt das Symbol in der Aufnahmezeile.
  for (const name of ["A", "B"]) {
    await pick(page, `C:/Audio/Datei ${name}.m4a`);
    await page.getByTestId("rec-strip").getByTestId("import-open").click();
    await confirmStartDialog(page);
    await expect(page.getByTestId("import-dialog")).toHaveCount(0);
  }
  await expect
    .poll(async () => await snapshot(page))
    .toMatchObject({ running: ["m-imp1"], waiting: ["m-imp2"] });
  // Die zuletzt importierte (wartende) Datei ist gewaehlt.
  await expect(page.getByTestId("queue-panel")).toBeVisible();
  await expect(page.getByTestId("queue-place")).toHaveText(
    "Wartet · Platz 1 von 1",
  );
  await expect(page.getByTestId("status-chip")).toHaveAttribute(
    "data-state",
    "queued",
  );
  const overflow = await page.evaluate(() => ({
    v: document.scrollingElement!.scrollHeight - window.innerHeight,
    h: document.scrollingElement!.scrollWidth - window.innerWidth,
  }));
  expect(overflow.v).toBeLessThanOrEqual(1);
  expect(overflow.h).toBeLessThanOrEqual(1);
  await shoot(page, "nach-u7-warteschlange-480");
});

// ---------------------------------------------------------------------------
// Einstellung
// ---------------------------------------------------------------------------

test('Einstellung "Gleichzeitige Transkriptionen": 1 ist Standard, 2 und 3 lassen sich waehlen', async ({
  page,
}) => {
  // Eigene Seite ohne Aufnahmen-Attrappe: die Einstellungen der Gruppe Besprechungen.
  const fresh = await page.context().newPage();
  await installTauriMock(fresh, "main");
  await fresh.setViewportSize({ width: 1280, height: 1000 });
  await fresh.goto("/");
  await fresh
    .getByRole("navigation")
    .getByRole("button", { name: "Einstellungen", exact: true })
    .click();
  const title = fresh.getByText("Gleichzeitige Transkriptionen", {
    exact: true,
  });
  await expect(title).toBeVisible();
  const dropdown = fresh.getByRole("button", { name: /1 \(Standard\)/ });
  await expect(dropdown).toBeVisible();
  await dropdown.click();
  await fresh
    .getByRole("button", { name: "2", exact: true })
    .evaluate((el) => (el as HTMLElement).click());
  await expect
    .poll(
      async () =>
        (await calls(fresh, "change_meeting_import_parallel_setting")).length,
    )
    .toBe(1);
  expect(
    (await calls(fresh, "change_meeting_import_parallel_setting"))[0].args,
  ).toEqual({ value: 2 });
  // Die Auswahl zeigt jetzt "2".
  await expect(
    fresh.getByRole("button", { name: "2", exact: true }),
  ).toBeVisible();
  // Die Beschreibung (Tooltip am Info-Symbol) nennt die Bedingung: Speicher, sonst wartet die Datei.
  await title
    .locator("xpath=ancestor::div[.//*[@aria-label='More information']][1]")
    .getByLabel("More information")
    .hover();
  await expect(
    fresh.getByText(/wartet auf Arbeitsspeicher/).first(),
  ).toBeVisible();
  await fresh.close();
});
