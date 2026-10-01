import { test, expect, type Locator, type Page } from "@playwright/test";
import * as fs from "node:fs";
import * as path from "node:path";
import { mergeSegments } from "../src/lib/meetingSegments";
import {
  PROGRESS,
  calls,
  emitMeeting,
  openRecordings,
  scrollReport,
} from "./recLayoutMock";
import {
  confirmStartDialog,
  emitLive,
  emitOnly,
  holdImport,
  holdSegments,
  installLiveMock,
  releaseImport,
  releaseSegments,
  seg,
  setPosition,
  startRecording,
} from "./recLiveMock";

// Aufnahmen-Oberflaeche (Goal aufnahmen-ui, M5): die laufende Aufnahme ist die
// gewaehlte Besprechung (Notizblock Mitte, Live-Transkript rechts, Fragen im
// Reiter), ein Importweg (Symbol und Ablage auf der Arbeitsflaeche),
// Startdialog mit Projekt-Vorbelegung, schmales Fenster (AK6, AK7).

test.beforeEach(async ({ page }) => {
  await installLiveMock(page);
});

const pad = (page: Page) => page.getByTestId("live-notes-pad");
const sessions = (page: Page) => page.getByTestId("rec-sessions");
const projectRow = (page: Page, name: string) =>
  sessions(page).getByTestId("project-row").filter({ hasText: name });

/** Liegt die Box ganz im sichtbaren Fenster? */
const inViewport = async (locator: Locator, width: number, height: number) => {
  const b = await locator.boundingBox();
  expect(b).not.toBeNull();
  return (
    b!.x >= 0 &&
    b!.y >= 0 &&
    b!.x + b!.width <= width + 0.5 &&
    b!.y + b!.height <= height + 0.5
  );
};

/** Anzahl und Eindeutigkeit der Saetze im Transkript (nach data-segment-index). */
const renderedIndices = (page: Page) =>
  page
    .locator("[data-segment-index]")
    .evaluateAll((els) =>
      els.map((el) => Number(el.getAttribute("data-segment-index"))),
    );

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

// ---------------------------------------------------------------------------
// Reine Logik: Segmente zusammenfuehren
// ---------------------------------------------------------------------------

test.describe("Segmente zusammenführen", () => {
  test("idempotent: dasselbe Segment aus Datenbank und Ereignis steht einmal da", () => {
    const db = [seg(0, "a"), seg(1, "b"), seg(2, "c")];
    const merged = mergeSegments(db, [seg(2, "c"), seg(3, "d")]);
    expect(merged.map((s) => s.segment_index)).toEqual([0, 1, 2, 3]);
    expect(mergeSegments(merged, [seg(1, "b")])).toHaveLength(4);
  });

  test("bei gleicher Nummer gewinnt das Neue (Sprecher geaendert), nichts geht verloren", () => {
    const merged = mergeSegments(
      [seg(0, "alt"), seg(1, "b")],
      [{ ...seg(0, "neu"), speaker_index: 2 }],
    );
    expect(merged).toHaveLength(2);
    expect(merged[0].text).toBe("neu");
  });

  test("Anzeigereihenfolge: nach Beginn, die zwei Kanaele gemischt", () => {
    const late = { ...seg(5, "spaet"), start_ms: 40_000 };
    const early = { ...seg(9, "frueh"), start_ms: 2_000 };
    expect(mergeSegments([late], [early]).map((s) => s.segment_index)).toEqual([
      9, 5,
    ]);
  });

  test("ohne Neues bleibt die Liste dieselbe (kein Render)", () => {
    const db = [seg(0, "a")];
    expect(mergeSegments(db, [])).toBe(db);
  });
});

// ---------------------------------------------------------------------------
// AK6: Live im kleinen Fenster
// ---------------------------------------------------------------------------

test("AK6: bei 480x800 Aufnahme starten, Notiz tippen und Live-Transkript sehen - ohne Scrollen und Seitenwechsel", async ({
  page,
}) => {
  await openRecordings(page, 480, 800);
  // Vor dem Start: Aufnahmezeile oben mit Start, Import, Link.
  const strip = page.getByTestId("rec-strip");
  await expect(strip.getByTestId("rec-start")).toBeVisible();
  await expect(strip.getByTestId("import-open")).toBeVisible();
  await startRecording(page);

  // Die laufende Aufnahme ist die gewaehlte Besprechung: Live-Transkript in der
  // Arbeitsflaeche (oben), Notizblock im Reiter darunter, der Fokus steht im
  // Eingabefeld.
  await expect(pad(page)).toBeVisible();
  await expect(page.getByTestId("note-starter")).toBeFocused();
  await expect(strip.getByTestId("rec-live")).toBeVisible();
  await expect(
    page.getByTestId("rec-content").getByTestId("meeting-title"),
  ).toBeVisible();
  await expect(page.getByTestId("status-chip")).toHaveAttribute(
    "data-state",
    "recording",
  );

  // Notiz tippen: Enter speichert den Punkt mit der Aufnahmezeit, "[ ]" = Aufgabe.
  await setPosition(page, 65_000);
  await page.keyboard.type("Budget bis Freitag freigegeben");
  await expect(pad(page).getByTestId("note-time").first()).toHaveText("01:05");
  await setPosition(page, 95_000);
  await page.keyboard.press("Enter");
  await page.keyboard.type("[ ] Angebot schicken");
  await expect(pad(page).locator("[data-kind=todo]")).toHaveCount(1);

  // Live-Transkript laeuft mit.
  await emitLive(page, "m-neu", [
    seg(0, "Guten Morgen zusammen."),
    seg(1, "Wir starten mit dem Budget."),
  ]);
  await expect(page.getByText("Wir starten mit dem Budget.")).toBeVisible();

  // Kein Scrollen: die Seite nicht, und weder Notizen noch Transkript muessen
  // gescrollt werden, um Eingabefeld und neuesten Satz zu sehen.
  const report = await scrollReport(page);
  expect(report.documentScrollHeight).toBeLessThanOrEqual(
    report.innerHeight + 1,
  );
  expect(report.mainOverflow, JSON.stringify(report.scrollers)).toBe(false);
  expect(report.nested, JSON.stringify(report.scrollers)).toEqual([]);
  const contentScroll = page.getByTestId("rec-content-scroll");
  expect(
    await contentScroll.evaluate((el) => el.scrollHeight - el.clientHeight),
  ).toBeLessThanOrEqual(1);
  const field = pad(page).locator("textarea").last();
  expect(await inViewport(field, 480, 800)).toBe(true);
  const sentence = page.getByText("Wir starten mit dem Budget.");
  expect(await inViewport(sentence, 480, 800)).toBe(true);
  // G4: Transkript oben, Notiz darunter, Aufnahmezeile darueber.
  const b = async (l: Locator) => (await l.boundingBox())!;
  expect((await b(strip)).y + (await b(strip)).height).toBeLessThanOrEqual(
    (await b(sentence)).y + 1,
  );
  expect((await b(sentence)).y).toBeLessThan((await b(field)).y);

  // Die Notiz ist gespeichert, mit Zeitstempel und in der laufenden Besprechung.
  await expect
    .poll(async () => (await calls(page, "meeting_notes_save")).length)
    .toBeGreaterThan(0);
  const saves = await calls(page, "meeting_notes_save");
  const last = saves[saves.length - 1].args as any;
  expect(last.meetingId).toBe("m-neu");
  expect(last.blocks[0]).toMatchObject({
    text: "Budget bis Freitag freigegeben",
    at_ms: 65_000,
  });
  await shoot(page, "nach-m5-live-480");
});

test("schmal: Kopf mit Titel, Details und Menue, Aufnahmezeile, darunter geteilt Transkript und Notizen", async ({
  page,
}) => {
  await openRecordings(page, 480, 800);
  await startRecording(page);
  await expect(pad(page)).toBeVisible();
  const content = page.getByTestId("rec-content");
  // Titel, Details und Menue in der Kopfzeile der Besprechung; die Symbolzeile
  // mit den Aktionen gibt es nicht (alles steht im Menue).
  await expect(content.getByTestId("meeting-title")).toBeVisible();
  await expect(content.getByTestId("meeting-details-open")).toBeVisible();
  await expect(content.getByTestId("meeting-menu")).toBeVisible();
  await expect(page.getByTestId("rec-actions")).toHaveCount(0);
  await content.getByTestId("meeting-menu").click();
  for (const name of [
    "Exportieren",
    "Personen",
    "Vorlage wählen …",
    "Besprechung löschen …",
  ]) {
    await expect(
      page.getByRole("menuitem", { name: new RegExp(name.replace("…", "")) }),
    ).toBeVisible();
  }
  // Die laufende Aufnahme laesst sich nicht loeschen.
  await expect(
    page.getByRole("menuitem", { name: /Besprechung löschen/ }),
  ).toBeDisabled();
  await page.keyboard.press("Escape");

  const strip = await page.getByTestId("rec-strip").boundingBox();
  const cBox = await content.boundingBox();
  const lower = await page.getByTestId("rec-controls").boundingBox();
  expect(strip!.y + strip!.height).toBeLessThanOrEqual(cBox!.y + 1);
  expect(cBox!.y + cBox!.height).toBeLessThanOrEqual(lower!.y + 1);
  expect(Math.abs(cBox!.width - lower!.width)).toBeLessThanOrEqual(2);
  await expect(
    page
      .getByTestId("rec-controls")
      .getByRole("tab", { name: "Notizen", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  await expect(
    page.getByTestId("rec-controls").getByRole("tab", { name: "Fragen" }),
  ).toBeVisible();
  await expect(
    content.getByRole("tab", { name: "Transkript", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
});

test("schmal: der waagerechte Griff wirkt mit Maus und Tasten, Doppelklick = halbe-halbe, und bleibt nach dem Neuladen", async ({
  page,
}) => {
  await openRecordings(page, 480, 800);
  const handle = page.getByTestId("resize-split");
  await expect(handle).toHaveAttribute("aria-valuenow", "50");
  const height = async () =>
    (await page.getByTestId("rec-content").boundingBox())!.height;
  const before = await height();

  // Ziehen: nach unten macht die Arbeitsflaeche hoeher.
  const hb = (await handle.boundingBox())!;
  await page.mouse.move(hb.x + hb.width / 2, hb.y + hb.height / 2);
  await page.mouse.down();
  await page.mouse.move(hb.x + hb.width / 2, hb.y + hb.height / 2 + 40, {
    steps: 6,
  });
  await page.mouse.up();
  expect(await height()).toBeGreaterThan(before + 20);
  const dragged = await handle.getAttribute("aria-valuenow");
  expect(Number(dragged)).toBeGreaterThan(50);

  await handle.focus();
  await page.keyboard.press("ArrowUp");
  await expect(handle).toHaveAttribute(
    "aria-valuenow",
    String(Math.round(Number(dragged)) - 5),
  );
  await page.keyboard.press("End");
  await expect(handle).toHaveAttribute("aria-valuenow", "75");

  // Bleibt nach dem Neuladen.
  await page.reload();
  await page.getByTestId("rec-content").waitFor();
  await expect(page.getByTestId("resize-split")).toHaveAttribute(
    "aria-valuenow",
    "75",
  );
  await page.getByTestId("resize-split").dblclick();
  await expect(page.getByTestId("resize-split")).toHaveAttribute(
    "aria-valuenow",
    "50",
  );
});

// ---------------------------------------------------------------------------
// Live: Umschalten ohne Segmentverlust
// ---------------------------------------------------------------------------

/** Eine laufende Aufnahme mit `count` Saetzen, im breiten Fenster. */
const liveWith = async (page: Page, count: number, width = 1366) => {
  await openRecordings(page, width, 768);
  await startRecording(page);
  await expect(pad(page)).toBeVisible();
  await emitLive(
    page,
    "m-neu",
    Array.from({ length: count }, (_, i) => seg(i, `Live-Satz ${i}`)),
  );
  await expect(page.getByText(`Live-Satz ${count - 1}`)).toBeVisible();
};

test("Live: Reiterwechsel in beiden Bereichen verliert und doppelt keine Segmente", async ({
  page,
}) => {
  await liveWith(page, 5);
  expect(await renderedIndices(page)).toEqual([0, 1, 2, 3, 4]);

  // Transkript -> Protokoll (Mitte) und Notizen -> Fragen (rechts): waehrenddessen
  // kommen Saetze.
  await page
    .getByTestId("rec-content")
    .getByRole("tab", { name: "Protokoll", exact: true })
    .click();
  await page
    .getByTestId("rec-controls")
    .getByRole("tab", { name: "Fragen", exact: true })
    .click();
  await emitLive(page, "m-neu", [seg(5, "Live-Satz 5"), seg(6, "Live-Satz 6")]);
  await page
    .getByTestId("rec-content")
    .getByRole("tab", { name: "Transkript", exact: true })
    .click();
  await page
    .getByTestId("rec-controls")
    .getByRole("tab", { name: "Notizen", exact: true })
    .click();
  await expect(page.getByText("Live-Satz 6")).toBeVisible();
  expect(await renderedIndices(page)).toEqual([0, 1, 2, 3, 4, 5, 6]);
  // Der Notizblock ist wieder da (und noch die laufende Aufnahme).
  await expect(pad(page)).toBeVisible();
});

test("Live: zu einer anderen Besprechung und zurueck - der Stand kommt aus Datenbank UND Ereignissen, nichts doppelt", async ({
  page,
}) => {
  await liveWith(page, 4);
  await page.locator('[data-meeting-id="m2"]').click();
  await expect(page.locator('[data-segment-index="0"]')).toContainText(
    "Guten Morgen zusammen",
  );
  // Waehrend m2 offen ist, geht die Aufnahme weiter (Ereignisse gehen an niemanden).
  await emitLive(page, "m-neu", [seg(4, "Live-Satz 4"), seg(5, "Live-Satz 5")]);
  await page.locator('[data-meeting-id="m-neu"]').click();
  await expect(page.getByText("Live-Satz 5")).toBeVisible();
  await expect(pad(page)).toBeVisible();
  expect(await renderedIndices(page)).toEqual([0, 1, 2, 3, 4, 5]);
});

test("Live: Saetze, die waehrend des Ladens eintreffen, gehen nicht verloren und doppeln sich nicht", async ({
  page,
}) => {
  await liveWith(page, 3);
  await page.locator('[data-meeting-id="m2"]').click();
  await expect(page.locator('[data-segment-index="0"]')).toContainText(
    "Guten Morgen zusammen",
  );
  // Die Ladeantwort haengt fest: der Stand zum Aufruf (Saetze 0 bis 2).
  await holdSegments(page);
  await page.locator('[data-meeting-id="m-neu"]').click();
  await expect
    .poll(
      async () =>
        (await calls(page, "meetings_get_segments")).filter(
          (c) => (c.args as any).meetingId === "m-neu",
        ).length,
    )
    .toBeGreaterThan(1);
  // Waehrend des Ladens: ein neuer Satz (erst in der Datenbank, dann gemeldet) ...
  await emitLive(page, "m-neu", [seg(3, "Live-Satz 3")]);
  // ... und ein Ereignis zu einem Satz, der schon im geladenen Stand steckt.
  await emitOnly(page, "m-neu", [seg(2, "Live-Satz 2")]);
  await releaseSegments(page);
  await expect(page.getByText("Live-Satz 3")).toBeVisible();
  expect(await renderedIndices(page)).toEqual([0, 1, 2, 3]);
});

test("Live: Zum Live-Ende erscheint, wenn man hochscrollt, und holt das Mitlaufen zurueck", async ({
  page,
}) => {
  await liveWith(page, 40, 1366);
  const list = page.getByTestId("transcript-scroll");
  const atEnd = () =>
    list.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight);
  await expect(page.getByTestId("follow-live")).toHaveCount(0);
  expect(await atEnd()).toBeLessThan(24);

  await list.evaluate((el) => {
    el.scrollTop = 0;
    el.dispatchEvent(new Event("scroll"));
  });
  const follow = page.getByTestId("follow-live");
  await expect(follow).toBeVisible();
  await expect(follow).toHaveText("Zum Live-Ende");
  // Neue Saetze verschieben die Ansicht nicht.
  await emitLive(page, "m-neu", [seg(40, "Live-Satz 40")]);
  await expect(page.getByTestId("autoscroll-paused")).toBeVisible();
  expect(await list.evaluate((el) => el.scrollTop)).toBeLessThan(10);

  await follow.click();
  await expect(follow).toHaveCount(0);
  expect(await atEnd()).toBeLessThan(24);
  await emitLive(page, "m-neu", [seg(41, "Live-Satz 41")]);
  await expect(page.getByText("Live-Satz 41")).toBeInViewport();
  expect(await atEnd()).toBeLessThan(24);
});

test("Live: Fragen stehen im Reiter der rechten Spalte, nicht unter dem Notizblock", async ({
  page,
}) => {
  await liveWith(page, 3, 480);
  await expect(page.getByTestId("live-chat-row")).toHaveCount(0);
  await page
    .getByTestId("rec-controls")
    .getByRole("tab", { name: "Fragen", exact: true })
    .click();
  const panel = page.getByTestId("chat-panel");
  await expect(panel).toBeVisible();
  await expect(panel).toContainText("Laufende Besprechung");
  // Keine geschachtelte Scrollflaeche (das war der Befund aus dem Live-Bild).
  const report = await scrollReport(page);
  expect(report.nested, JSON.stringify(report.scrollers)).toEqual([]);
  expect(report.documentScrollHeight).toBeLessThanOrEqual(
    report.innerHeight + 1,
  );
  // G4: Fragen und Notizen sind Reiter derselben Flaeche; das Live-Transkript
  // bleibt in der Mitte stehen, der Notizblock bleibt eingehaengt (verborgen).
  await expect(pad(page)).toBeHidden();
  expect(await renderedIndices(page)).toEqual([0, 1, 2]);
  // Zurueck zu den Notizen: der Block ist wieder da.
  await page
    .getByTestId("rec-controls")
    .getByRole("tab", { name: "Notizen", exact: true })
    .click();
  await expect(pad(page)).toBeVisible();
  expect(await renderedIndices(page)).toEqual([0, 1, 2]);
});

test("Stopp: nahtlos in die Fortschrittsanzeige derselben Besprechung, Notizen und Transkript bleiben", async ({
  page,
}) => {
  await liveWith(page, 4);
  await setPosition(page, 12_000);
  await pad(page).getByTestId("note-starter").click();
  await page.keyboard.type("Ungespeicherte Notiz vor dem Stopp");
  await page.getByTestId("rec-stop").click();

  // Die Notiz wird vor dem Stopp gespeichert.
  await expect
    .poll(async () => (await calls(page, "meetings_stop")).length)
    .toBe(1);
  const order = await page.evaluate(() =>
    ((window as any).__calls as { cmd: string }[])
      .map((c) => c.cmd)
      .filter((c) => c === "meeting_notes_save" || c === "meetings_stop"),
  );
  expect(order.indexOf("meeting_notes_save")).toBeGreaterThan(-1);
  expect(order.indexOf("meeting_notes_save")).toBeLessThan(
    order.indexOf("meetings_stop"),
  );

  // Dieselbe Besprechung bleibt gewaehlt; der Status wechselt, der Fortschritt
  // (P8a) erscheint dort, wo er immer erscheint.
  await emitMeeting(page, { ...PROGRESS, meeting_id: "m-neu" });
  await expect(page.getByTestId("job-panel")).toBeVisible();
  await expect(page.getByTestId("status-chip")).toHaveAttribute(
    "data-state",
    "processing",
  );
  await expect(page.getByTestId("rec-live")).toHaveCount(0);
  await expect(page.getByTestId("my-notes")).toBeVisible();
  await expect(
    page.getByTestId("my-notes").locator("textarea").first(),
  ).toHaveValue("Ungespeicherte Notiz vor dem Stopp");
  expect(await renderedIndices(page)).toEqual([0, 1, 2, 3]);
  // Nach dem Stopp tragen neue Notizen keinen Zeitstempel mehr (keine Aufnahme).
  await expect(page.getByTestId("live-notes-pad")).toHaveCount(0);
});

// ---------------------------------------------------------------------------
// Startdialog
// ---------------------------------------------------------------------------

test("Startdialog: das links gewählte Projekt belegt vor, sonst das zuletzt genutzte", async ({
  page,
}) => {
  await page.addInitScript(() => {
    localStorage.setItem("lva.ui.meetings.startProject", "f3");
  });
  await openRecordings(page, 1366, 768);
  const projectField = page.getByTestId("start-project");
  const open = async () => {
    await page
      .getByRole("button", { name: /Aufnahme starten/ })
      .first()
      .click();
    await expect(page.getByTestId("start-dialog")).toBeVisible();
  };

  // "Alle Aufnahmen": das zuletzt genutzte Projekt.
  await open();
  await expect(projectField).toContainText("Kunde Stadtwerke");
  await page.keyboard.press("Escape");

  // Ein Projekt links gewaehlt: es belegt vor.
  await projectRow(page, "Podcast").click();
  await open();
  await expect(projectField).toContainText("Podcast");
  await page.keyboard.press("Escape");
  await projectRow(page, "Privat").click();
  await open();
  await expect(projectField).toContainText("Privat");

  // Start: die Besprechung liegt im vorbelegten Projekt und ist gewaehlt.
  await confirmStartDialog(page);
  await expect(pad(page)).toBeVisible();
  await expect
    .poll(async () => (await calls(page, "meetings_set_folders")).length)
    .toBe(1);
  expect((await calls(page, "meetings_set_folders"))[0].args).toEqual({
    meetingId: "m-neu",
    folderIds: ["f1"],
  });
  // In der Liste des Projekts steht die laufende Aufnahme, hervorgehoben.
  await expect(page.locator('[data-meeting-id="m-neu"]')).toHaveAttribute(
    "aria-current",
    "true",
  );
});

test("Start scheitert (Mikrofon fehlt): Fehler im Dialog-Umfeld, nichts wird gewaehlt, kein Notizblock", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as any).__startError = "mic_stream_error: kein Geraet";
  });
  await openRecordings(page, 1366, 768);
  await startRecording(page);
  await expect(page.getByTestId("rec-recorder")).toContainText(
    "Mikrofon-Stream",
  );
  await expect(pad(page)).toHaveCount(0);
  await expect(page.getByTestId("rec-live")).toHaveCount(0);
  await expect(page.getByTestId("rec-start")).toBeVisible();
  await expect(page.getByTestId("rec-content")).toContainText(
    "Wähle eine Aufnahme",
  );
});

// ---------------------------------------------------------------------------
// AK7: Import - ein Weg
// ---------------------------------------------------------------------------

test("AK7: Import per Symbol geht in das links gewaehlte Projekt, Fortschritt an der erwarteten Stelle", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  await projectRow(page, "Geschäftlich").click();
  await holdImport(page);
  await page.evaluate(
    () => ((window as any).__pick = "C:/Audio/Kunde Meyer.m4a"),
  );
  await page.getByTestId("import-open").click();
  const dialog = page.getByTestId("import-dialog");
  await expect(dialog).toContainText("Kunde Meyer.m4a");
  await expect(page.getByTestId("import-target")).toHaveText(
    "Import in das Projekt „Geschäftlich“",
  );
  expect(await calls(page, "meetings_import_file")).toHaveLength(0);
  await confirmStartDialog(page);

  // Noch waehrend die Verarbeitung laeuft: im Projekt, gewaehlt, mit Fortschritt.
  await expect
    .poll(async () => (await calls(page, "meetings_set_folders")).length)
    .toBe(1);
  expect((await calls(page, "meetings_set_folders"))[0].args).toEqual({
    meetingId: "m-imp1",
    folderIds: ["f2"],
  });
  await emitMeeting(page, { ...PROGRESS, meeting_id: "m-imp1" });
  await expect(page.locator('[data-meeting-id="m-imp1"]')).toHaveAttribute(
    "aria-current",
    "true",
  );
  await expect(
    page.locator('[data-meeting-id="m-imp1"]').getByTestId("job-bar"),
  ).toBeVisible();
  await expect(page.getByTestId("job-panel")).toBeVisible();
  await expect(page.getByTestId("status-chip")).toHaveAttribute(
    "data-state",
    "processing",
  );
  await expect(
    page.getByTestId("rec-content").getByTestId("meeting-title"),
  ).toHaveText("Kunde Meyer");
  // Nur ein Importaufruf, mit Einwilligung.
  expect(await calls(page, "meetings_import_file")).toHaveLength(1);
  expect((await calls(page, "meetings_import_file"))[0].args).toEqual({
    path: "C:/Audio/Kunde Meyer.m4a",
    consentConfirmed: true,
    targetMeetingId: null,
  });
  await releaseImport(page);
  await expect(page.getByTestId("status-chip")).toHaveAttribute(
    "data-state",
    "ready",
  );
});

/** Datei-Ablage, wie Tauri sie meldet (physische Pixel; Playwright hat Faktor 1). */
const dropAt = async (
  page: Page,
  x: number,
  y: number,
  paths: string[],
  type: "drag-over" | "drag-drop" = "drag-drop",
) =>
  page.evaluate(
    ({ t, x, y, paths }) =>
      (window as any).__emit(`tauri://${t}`, { paths, position: { x, y } }),
    { t: type, x, y, paths },
  );

const centerOf = async (locator: Locator) => {
  const b = (await locator.boundingBox())!;
  return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
};

test("AK7: Datei auf die Arbeitsflaeche ziehen fuehrt in denselben Importweg und das gewaehlte Projekt", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  await projectRow(page, "Podcast").click();
  const content = page.getByTestId("rec-content");
  const inside = await centerOf(content);

  // Schweben: das Ablagefeld nennt das Ziel.
  await dropAt(page, inside.x, inside.y, [], "drag-over");
  await expect(page.getByTestId("drop-overlay")).toHaveText(
    "Datei hier ablegen – Import in „Podcast“",
  );
  // Ueber der Projekte-Spalte: kein Ablagefeld.
  const outside = await centerOf(sessions(page));
  await dropAt(page, outside.x, outside.y, [], "drag-over");
  await expect(page.getByTestId("drop-overlay")).toHaveCount(0);

  // Ablegen neben der Arbeitsflaeche tut nichts.
  await dropAt(page, outside.x, outside.y, ["C:/Audio/Folge 18.m4a"]);
  await expect(page.getByTestId("import-dialog")).toHaveCount(0);

  // Falsches Format: Hinweis, kein Dialog.
  await dropAt(page, inside.x, inside.y, ["C:/Audio/Notizen.docx"]);
  await expect(
    page
      .locator("[data-sonner-toast]")
      .filter({ hasText: "nicht unterstützt" }),
  ).toBeVisible();
  await expect(page.getByTestId("import-dialog")).toHaveCount(0);

  // Ablegen auf der Arbeitsflaeche: Einwilligung, dann Import ins Projekt.
  await dropAt(page, inside.x, inside.y, ["C:/Audio/Folge 18.m4a"]);
  await expect(page.getByTestId("import-dialog")).toContainText("Folge 18.m4a");
  await expect(page.getByTestId("import-target")).toHaveText(
    "Import in das Projekt „Podcast“",
  );
  expect(await calls(page, "meetings_import_file")).toHaveLength(0);
  await confirmStartDialog(page);
  await expect
    .poll(async () => (await calls(page, "meetings_import_file")).length)
    .toBe(1);
  await expect
    .poll(async () => (await calls(page, "meetings_set_folders")).length)
    .toBe(1);
  expect((await calls(page, "meetings_set_folders"))[0].args).toEqual({
    meetingId: "m-imp1",
    folderIds: ["f4"],
  });
  // Die Liste des Projekts zeigt die neue Besprechung.
  await expect(page.locator('[data-meeting-id="m-imp1"]')).toBeVisible();
});

test("AK7: ohne gewaehltes Projekt wird ohne Projekt importiert, ein geloeschtes Projekt wird nicht benutzt", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  const inside = await centerOf(page.getByTestId("rec-content"));
  await dropAt(page, inside.x, inside.y, ["C:/Audio/a.wav"]);
  await expect(page.getByTestId("import-target")).toHaveText(
    "Import ohne Projekt",
  );
  await confirmStartDialog(page);
  await expect
    .poll(async () => (await calls(page, "meetings_import_file")).length)
    .toBe(1);
  expect(await calls(page, "meetings_set_folders")).toHaveLength(0);

  // Projekt gewaehlt, dann (waehrend der Dialog offen ist) geloescht.
  await projectRow(page, "Podcast").click();
  await dropAt(page, inside.x, inside.y, ["C:/Audio/b.wav"]);
  await expect(page.getByTestId("import-target")).toContainText("Podcast");
  await page.evaluate(() => {
    const w = window as any;
    w.__folders = w.__folders.filter((f: any) => f.id !== "f4");
  });
  await page.keyboard.press("Escape");
  await dropAt(page, inside.x, inside.y, ["C:/Audio/b.wav"]);
  // Das Projekt gibt es nicht mehr: die Auswahl faellt zurueck, der Import geht ohne Projekt.
  await expect(page.getByTestId("import-target")).toHaveText(
    "Import ohne Projekt",
  );
});

test("Import bei 480 Breite: Symbol in der Aufnahmezeile, Fortschritt in derselben Besprechung", async ({
  page,
}) => {
  await openRecordings(page, 480, 800);
  await holdImport(page);
  await page.evaluate(
    () => ((window as any).__pick = "C:/Audio/Interview.mp3"),
  );
  await page.getByTestId("rec-strip").getByTestId("import-open").click();
  await expect(page.getByTestId("import-dialog")).toContainText(
    "Interview.mp3",
  );
  await confirmStartDialog(page);
  await expect
    .poll(async () => (await calls(page, "meetings_import_file")).length)
    .toBe(1);
  await expect(
    page.getByTestId("rec-content").getByTestId("meeting-title"),
  ).toHaveText("Interview");
  await emitMeeting(page, { ...PROGRESS, meeting_id: "m-imp1" });
  await expect(page.getByTestId("job-panel")).toBeVisible();
  const report = await scrollReport(page);
  expect(report.documentScrollHeight).toBeLessThanOrEqual(
    report.innerHeight + 1,
  );
  expect(report.nested, JSON.stringify(report.scrollers)).toEqual([]);
  await shoot(page, "nach-m5-import-480");
  await releaseImport(page);
});

// ---------------------------------------------------------------------------
// Fehlerfaelle
// ---------------------------------------------------------------------------

test("Aufnahme startet waehrend ein Import laeuft: die Aufnahme ist gewaehlt, der Import bleibt im Projekt und springt nicht dazwischen", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  await projectRow(page, "Geschäftlich").click();
  await holdImport(page);
  await page.evaluate(() => ((window as any).__pick = "C:/Audio/lang.m4a"));
  await page.getByTestId("import-open").click();
  await confirmStartDialog(page);
  await expect
    .poll(async () => (await calls(page, "meetings_set_folders")).length)
    .toBe(1);
  await expect(
    page.getByTestId("rec-content").getByTestId("meeting-title"),
  ).toHaveText("lang");

  // Aufnahme starten, waehrend der Import noch laeuft.
  await startRecording(page);
  await expect(pad(page)).toBeVisible();
  await emitMeeting(page, { ...PROGRESS, meeting_id: "m-imp1" });
  await emitLive(page, "m-neu", [seg(0, "Live-Satz 0")]);
  await expect(page.getByText("Live-Satz 0")).toBeVisible();
  // Nur der Live-Satz steht im Transkript, nichts vom Import.
  expect(await renderedIndices(page)).toEqual([0]);

  // Der Import endet: die Auswahl bleibt bei der Aufnahme.
  await releaseImport(page);
  await expect(
    page.locator('[data-meeting-id="m-imp1"]').getByTestId("job-bar"),
  ).toHaveCount(0);
  await expect(pad(page)).toBeVisible();
  await expect(page.getByTestId("rec-live")).toBeVisible();
  await expect(page.locator('[data-meeting-id="m-neu"]')).toHaveAttribute(
    "aria-current",
    "true",
  );
});

test("Projekt wird geloescht, waehrend die Aufnahme darin laeuft: die Aufnahme bleibt, die Auswahl faellt auf Alle zurueck", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  await projectRow(page, "Podcast").click();
  await startRecording(page);
  await expect(pad(page)).toBeVisible();
  await setPosition(page, 30_000);
  await page.keyboard.type("Wichtig waehrend der Aufnahme");
  await emitLive(page, "m-neu", [seg(0, "Live-Satz 0")]);

  await projectRow(page, "Podcast").click({ button: "right" });
  await page
    .getByRole("menuitem", { name: "Löschen", exact: true })
    .evaluate((el) => (el as HTMLElement).click());
  await page
    .getByRole("dialog", { name: "Projekt löschen?" })
    .getByRole("button", { name: "Löschen", exact: true })
    .click();

  await expect(projectRow(page, "Podcast")).toHaveCount(0);
  // Die Besprechung wurde nicht geloescht, die Aufnahme laeuft weiter.
  expect(await calls(page, "meetings_delete")).toHaveLength(0);
  await expect(pad(page)).toBeVisible();
  await expect(page.getByTestId("rec-live")).toBeVisible();
  await expect(pad(page).locator("textarea").first()).toHaveValue(
    "Wichtig waehrend der Aufnahme",
  );
  await emitLive(page, "m-neu", [seg(1, "Live-Satz 1")]);
  await expect(page.getByText("Live-Satz 1")).toBeVisible();
  expect(await renderedIndices(page)).toEqual([0, 1]);
  // Die Liste steht auf "Alle Aufnahmen" und zeigt die laufende Aufnahme.
  await expect(page.locator('[data-meeting-id="m-neu"]')).toBeVisible();
  await expect(projectRow(page, "Alle Aufnahmen")).toBeVisible();
});

test("Fenster wird waehrend der Aufnahme schmal und wieder breit: Notizblock, Text, Transkript und Aufnahmezeile bleiben, nichts wird neu geladen", async ({
  page,
}) => {
  await liveWith(page, 6, 1366);
  await setPosition(page, 20_000);
  await pad(page).getByTestId("note-starter").click();
  await page.keyboard.type("Noch nicht gespeichert");
  const loads = async () =>
    (await calls(page, "meeting_notes_get")).filter(
      (c) => (c.args as any).meetingId === "m-neu",
    ).length;
  const segLoads = async () =>
    (await calls(page, "meetings_get_segments")).filter(
      (c) => (c.args as any).meetingId === "m-neu",
    ).length;
  const notesLoadsBefore = await loads();
  const segLoadsBefore = await segLoads();

  await page.setViewportSize({ width: 480, height: 800 });
  await expect(page.getByTestId("rec-strip")).toBeVisible();
  await expect(pad(page).locator("textarea").first()).toHaveValue(
    "Noch nicht gespeichert",
  );
  // Der Fokus im Feld ueberlebt den Umzug.
  await expect(pad(page).locator("textarea").first()).toBeFocused();
  await emitLive(page, "m-neu", [seg(6, "Live-Satz 6")]);
  await expect(page.getByText("Live-Satz 6")).toBeVisible();
  expect(await renderedIndices(page)).toEqual([0, 1, 2, 3, 4, 5, 6]);
  await expect(page.getByTestId("rec-live")).toBeVisible();

  await page.setViewportSize({ width: 1366, height: 768 });
  await expect(page.getByTestId("rec-strip")).toHaveCount(0);
  await emitLive(page, "m-neu", [seg(7, "Live-Satz 7")]);
  await expect(page.getByText("Live-Satz 7")).toBeVisible();
  expect(await renderedIndices(page)).toEqual([0, 1, 2, 3, 4, 5, 6, 7]);
  await expect(pad(page).locator("textarea").first()).toHaveValue(
    "Noch nicht gespeichert",
  );
  await expect(page.getByTestId("rec-live")).toBeVisible();
  // Weder Notizen noch Segmente wurden beim Umbau neu geladen (kein Neuaufbau).
  expect(await loads()).toBe(notesLoadsBefore);
  expect(await segLoads()).toBe(segLoadsBefore);
  // Und die Notiz wird gespeichert, sobald der Fokus geht.
  await page.locator(".page-shell__head h1").click();
  await expect
    .poll(async () => (await calls(page, "meeting_notes_save")).length)
    .toBeGreaterThan(0);
});

test("die Bedienung sitzt im breiten Fenster rechts oben, die laufende Aufnahme zeigt Notizblock und Transkript nebeneinander (Bild)", async ({
  page,
}) => {
  await liveWith(page, 12, 1366);
  await setPosition(page, 40_000);
  await pad(page).getByTestId("note-starter").click();
  await page.keyboard.type("Kunde moechte Angebot bis Freitag");
  await page.keyboard.press("Enter");
  await page.keyboard.type("[ ] Angebot schicken");
  const content = (await page.getByTestId("rec-content").boundingBox())!;
  const lower = (await page.getByTestId("rec-lower").boundingBox())!;
  expect(lower.x).toBeGreaterThanOrEqual(content.x + content.width - 1);
  await expect(
    page.getByTestId("rec-controls").getByTestId("rec-live"),
  ).toBeVisible();
  const report = await scrollReport(page);
  expect(report.nested, JSON.stringify(report.scrollers)).toEqual([]);
  expect(report.documentScrollHeight).toBeLessThanOrEqual(
    report.innerHeight + 1,
  );
  await shoot(page, "nach-m5-live-1366");
  // Verzeichnis fuer die Bilder sicherstellen (nur bei Bedarf).
  if (process.env.LVA_SCREENSHOTS) fs.mkdirSync(SHOT_DIR, { recursive: true });
});
