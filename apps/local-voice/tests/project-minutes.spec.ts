import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import * as path from "node:path";
import { calls, goToRecordings, openRecordings } from "./recLayoutMock";
import { installLiveMock } from "./recLiveMock";
import {
  PM_FOLDER,
  PM_KEY,
  failProjectMinutes,
  holdProjectMinutes,
  installProjectMinutesMock,
  releaseProjectMinutes,
} from "./projectMinutesMock";
import {
  clock,
  eligibleIds,
  projectMinutesErrorCode,
  projectMinutesErrorDetail,
  projectMinutesJobKey,
  pruneSelection,
  shortTitle,
  sourceToCitation,
} from "../src/lib/projectMinutes";

// G3 (#70, U9): mehrere Aufnahmen eines Projekts per Haekchen waehlen und daraus EIN
// gemeinsames Protokoll bzw. eine Zusammenfassung erzeugen. Die Regeln des Backends
// (Reihenfolge, Blockgrenzen, Ueberlaenge, Ablehnung einer Aufnahme ohne Transkript,
// Belege, Speicherung) pruefen die Rust-Tests; hier die Oberflaeche gegen die
// Tauri-Attrappe: Auswahl, Ausgrauen, Start, Fortschritt und Stopp, Ergebnis im
// Projekt, Quellen-Klick mit Sprung zur Aufnahme und Zeit.

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

const setup = async (page: Page, width = 1366, height = 768) => {
  await installLiveMock(page);
  await installProjectMinutesMock(page);
  await openRecordings(page, width, height);
};

const rowOf = (page: Page, id: string) =>
  page.locator(`[data-testid="meeting-row"][data-meeting-id="${id}"]`);
const projectRow = (page: Page, id: string) =>
  page.locator(`[data-project-id="${id}"]`);

/** Das Projekt waehlen und ueber das Menue den Auswahlmodus einschalten. */
const startSelecting = async (page: Page) => {
  // Ein Klick auf das gewaehlte Projekt klappt die Liste zu: nur waehlen, wenn noetig.
  const project = projectRow(page, PM_FOLDER);
  if ((await project.getAttribute("aria-current")) !== "true") {
    await project.click();
  } else if ((await project.getAttribute("aria-expanded")) !== "true") {
    await project.click();
  }
  await expect(project).toHaveAttribute("aria-current", "true");
  await expect(rowOf(page, "m4")).toBeVisible();
  await page.getByTestId("projects-more").click();
  await page.getByTestId("projects-pm").click();
  await expect(page.getByTestId("pm-bar")).toBeVisible();
  // Die Auskunft, wer waehlbar ist, ist da.
  await expect(page.getByTestId("pm-hint")).not.toHaveText(
    "Aufnahmen werden geprüft …",
  );
};

const count = (page: Page) => page.getByTestId("pm-count");

/** Die drei Aufnahmen mit Transkript waehlen (m4, m2, m1 in Listenreihenfolge). */
const pickThree = async (page: Page) => {
  await page.getByTestId("pm-all").click();
  await expect(count(page)).toHaveText("3 Aufnahmen gewählt");
};

/** Dialog bestaetigen; `kind` und Vorlage wie vom Nutzer gewaehlt. */
const startRun = async (
  page: Page,
  options: { kind?: "minutes" | "summary"; template?: string } = {},
) => {
  await page.getByTestId("pm-start").click();
  await expect(page.getByTestId("pm-dialog")).toBeVisible();
  if (options.kind === "summary") {
    await page.getByTestId("pm-kind-summary").check();
  }
  if (options.template) {
    await page
      .getByTestId("pm-dialog-template")
      .locator(".app-select__control")
      .click();
    await page
      .getByRole("option", { name: options.template, exact: true })
      .click();
  }
  await page.getByTestId("pm-dialog-start").click();
};

const finishWithDoc = async (page: Page) => {
  await startSelecting(page);
  await pickThree(page);
  await startRun(page);
  await expect(page.getByTestId("pm-view")).toBeVisible();
};

// ---------------------------------------------------------------------------
// Reine Logik
// ---------------------------------------------------------------------------

test.describe("projectMinutes.ts (rein)", () => {
  test("Fehlercodes: ganzer Code, mit Detail, unbekannt", () => {
    expect(projectMinutesErrorCode("no_transcript: 01ABC")).toBe(
      "no_transcript",
    );
    expect(projectMinutesErrorDetail("no_transcript: 01ABC")).toBe("01ABC");
    expect(projectMinutesErrorCode("minutes_busy")).toBe("minutes_busy");
    expect(projectMinutesErrorDetail("minutes_busy")).toBe("");
    expect(projectMinutesErrorCode("irgendwas Neues")).toBe("llm_failed");
    expect(projectMinutesErrorCode("no_selection_not")).toBe("llm_failed");
  });

  test("Auswahl: nur waehlbare bleiben, Schluessel passt zum Backend", () => {
    const c = (id: string, ok: boolean) => ({
      meeting_id: id,
      eligible: ok,
      reason: ok ? null : "no_transcript",
      segments: ok ? 3 : 0,
    });
    const list = [c("a", true), c("b", false), c("c", true)];
    expect(eligibleIds(list)).toEqual(["a", "c"]);
    expect(pruneSelection(["a", "b", "x", "c"], list)).toEqual(["a", "c"]);
    expect(projectMinutesJobKey("f2")).toBe("project-minutes:f2");
  });

  test("Zeit, Titel und Beleg als Chat-Beleg fuer die vorhandene Sprungmechanik", () => {
    expect(clock(0)).toBe("00:00");
    expect(clock(195_000)).toBe("03:15");
    expect(clock(3_723_000)).toBe("1:02:03");
    expect(shortTitle("Kurz")).toBe("Kurz");
    expect(shortTitle("Ein sehr langer Titel einer Aufnahme", 10)).toBe(
      "Ein sehr …",
    );
    const citation = sourceToCitation(
      { recording: 2, meeting_id: "m1", segment_index: 7, start_ms: 12_000 },
      "Review",
    );
    expect(citation).toMatchObject({
      meeting_id: "m1",
      meeting_title: "Review",
      source: "transcript",
      segment_index: 7,
      start_ms: 12_000,
    });
  });
});

// ---------------------------------------------------------------------------
// Auswahl
// ---------------------------------------------------------------------------

test("Auswahlmodus: Haekchen, Alle/Keine, Zaehler; nur Aufnahmen mit Transkript sind waehlbar", async ({
  page,
}) => {
  await setup(page);
  await startSelecting(page);

  await expect(count(page)).toHaveText("0 Aufnahmen gewählt");
  await expect(page.getByTestId("pm-start")).toBeDisabled();
  await expect(page.getByTestId("pm-hint")).toHaveText(
    "Wähle mindestens zwei Aufnahmen.",
  );

  // Ausgegraut, mit Grund: kein Transkript, laeuft noch, leerer Eintrag.
  for (const [id, reason, label] of [
    ["m5", "no_transcript", "Kein Transkript"],
    ["m3", "meeting_not_finished", "Läuft noch"],
    ["m7", "empty_entry", "Leer"],
  ] as const) {
    const row = rowOf(page, id);
    await expect(row).toHaveAttribute("data-pickable", "false");
    await expect(row).toHaveAttribute("aria-disabled", "true");
    await expect(row.getByTestId("row-reason")).toHaveText(label);
    await expect(row.getByTestId("row-reason")).toHaveAttribute(
      "data-reason",
      reason,
    );
  }
  await expect(rowOf(page, "m5")).toHaveAttribute(
    "title",
    /kein Transkript, daraus kann nichts protokolliert werden/,
  );
  // Ein Klick auf eine ausgegraute Aufnahme waehlt nichts.
  await rowOf(page, "m5").click({ force: true });
  await expect(count(page)).toHaveText("0 Aufnahmen gewählt");
  await expect(rowOf(page, "m5")).toHaveAttribute("aria-checked", "false");

  // Haekchen je Aufnahme.
  for (const id of ["m4", "m2", "m1"]) {
    await expect(rowOf(page, id)).toHaveAttribute("data-pickable", "true");
  }
  await rowOf(page, "m1").click();
  await expect(count(page)).toHaveText("1 Aufnahme gewählt");
  await expect(rowOf(page, "m1")).toHaveAttribute("aria-checked", "true");
  await expect(page.getByTestId("pm-start")).toBeDisabled();
  await rowOf(page, "m2").click();
  await expect(count(page)).toHaveText("2 Aufnahmen gewählt");
  await expect(page.getByTestId("pm-start")).toBeEnabled();
  await expect(page.getByTestId("pm-hint")).toHaveText("");

  // Alle = nur die waehlbaren; Keine hebt auf.
  await page.getByTestId("pm-all").click();
  await expect(count(page)).toHaveText("3 Aufnahmen gewählt");
  for (const id of ["m4", "m2", "m1"]) {
    await expect(rowOf(page, id)).toHaveAttribute("aria-checked", "true");
  }
  await expect(rowOf(page, "m5")).toHaveAttribute("aria-checked", "false");
  await shoot(page, "g3-auswahl");
  await page.getByTestId("pm-none").click();
  await expect(count(page)).toHaveText("0 Aufnahmen gewählt");
  await expect(page.getByTestId("pm-none")).toBeDisabled();

  // Abbrechen beendet den Modus: normale Zeilen, kein Haekchenfeld.
  await page.getByTestId("pm-cancel").click();
  await expect(page.getByTestId("pm-bar")).toHaveCount(0);
  await expect(rowOf(page, "m5")).not.toHaveAttribute("data-pickable", /.*/);
  await expect(rowOf(page, "m5").getByTestId("row-reason")).toHaveCount(0);
});

test("ohne Projekt ist die Aktion gesperrt (mit Grund), im Projekt ueber Menue und Kontextmenue erreichbar", async ({
  page,
}) => {
  await setup(page);
  // "Alle Aufnahmen" gewaehlt: kein Projekt, kein gemeinsames Protokoll.
  await page.getByTestId("projects-more").click();
  const item = page.getByTestId("projects-pm");
  await expect(item).toBeDisabled();
  await expect(item).toHaveAttribute(
    "title",
    "Wähle zuerst ein Projekt in der Liste.",
  );
  await page.keyboard.press("Escape");

  // Kontextmenue des Projekts: waehlt es und beginnt die Auswahl.
  await projectRow(page, PM_FOLDER).click({ button: "right" });
  await page
    .getByRole("menu")
    .getByRole("menuitem", { name: "Gemeinsam protokollieren", exact: true })
    .evaluate((el) => (el as HTMLElement).click());
  await expect(page.getByTestId("pm-bar")).toBeVisible();
  await expect(rowOf(page, "m4")).toBeVisible();

  // Wechsel des Projekts beendet die Auswahl (sie galt dem alten).
  await projectRow(page, "f4").click();
  await expect(page.getByTestId("pm-bar")).toHaveCount(0);
});

// ---------------------------------------------------------------------------
// Start, Fortschritt, Stopp
// ---------------------------------------------------------------------------

test("Start: Dialog mit Vorlage (auch Automatisch) und Art; der Aufruf traegt die Auswahl in Listenreihenfolge", async ({
  page,
}) => {
  await setup(page);
  await startSelecting(page);
  await pickThree(page);
  await page.getByTestId("pm-start").click();

  const dialog = page.getByTestId("pm-dialog");
  await expect(dialog).toBeVisible();
  await expect(page.getByRole("dialog")).toContainText(
    "Aus 3 Aufnahmen des Projekts „Geschäftlich“ entsteht ein gemeinsames Dokument",
  );
  await expect(page.getByTestId("pm-kind-minutes")).toBeChecked();
  // Dieselbe Vorlagenwahl wie beim Einzelprotokoll, inklusive "Automatisch".
  await dialog.locator(".app-select__control").click();
  await expect(
    page.getByRole("option", {
      name: "Automatisch (nach Inhalt)",
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    page.getByRole("option", {
      name: "Kundengespräch / Vertrieb",
      exact: true,
    }),
  ).toBeVisible();
  await page
    .getByRole("option", { name: "Automatisch (nach Inhalt)", exact: true })
    .click();
  await page.getByTestId("pm-kind-summary").check();
  await page.getByTestId("pm-dialog-start").click();

  await expect(page.getByTestId("pm-view")).toBeVisible();
  const started = await calls(page, "project_minutes_generate");
  expect(started).toHaveLength(1);
  expect(started[0].args).toEqual({
    folderId: PM_FOLDER,
    meetingIds: ["m4", "m2", "m1"],
    templateId: "auto",
    kind: "summary",
  });
  await expect(page.getByTestId("pm-title")).toHaveText(
    "Projekt-Zusammenfassung: Geschäftlich",
  );
  await expect(page.getByTestId("pm-origin-template")).toContainText(
    "automatisch gewählt",
  );
  // Der Auswahlmodus ist beendet.
  await expect(page.getByTestId("pm-bar")).toHaveCount(0);
});

test("Fortschritt und Stopp wie bei der Einzelverarbeitung; der Zustand ueberlebt den Seitenwechsel", async ({
  page,
}) => {
  await setup(page);
  await holdProjectMinutes(page);
  await startSelecting(page);
  await pickThree(page);
  await startRun(page);

  // Lauf in der Arbeitsflaeche: dieselbe Statusleiste wie bei der Einzelverarbeitung.
  const run = page.getByTestId("pm-run");
  await expect(run).toBeVisible();
  const panel = run.getByTestId("job-panel");
  await expect(panel).toHaveAttribute("data-phase", "minutes");
  await expect(panel.getByTestId("job-phase")).toHaveText("Protokoll");
  await expect(panel.getByTestId("job-amount")).toHaveText("Schritt 1 von 4");
  // Wie beim Einzelprotokoll: nicht pausierbar, nur stoppen.
  await expect(panel.getByTestId("job-pause")).toBeDisabled();
  await expect(panel.getByTestId("job-stop")).toBeEnabled();
  // Und im Projekt steht der laufende Eintrag mit Balken.
  const running = page.getByTestId("pm-row-running");
  await expect(running).toBeVisible();
  await expect(running.getByTestId("job-bar")).toHaveAttribute(
    "data-phase",
    "minutes",
  );

  // Seitenwechsel: weg und zurueck. Der Lauf gehoert dem Backend, nicht der Seite.
  await page
    .getByRole("button", { name: "Einstellungen", exact: true })
    .click();
  await expect(page.getByTestId("rec-content")).toHaveCount(0);
  await goToRecordings(page);
  await expect(projectRow(page, PM_FOLDER)).toBeVisible();
  const again = page.getByTestId("pm-row-running");
  await expect(again).toBeVisible();
  await expect(again.getByTestId("job-bar")).toHaveAttribute(
    "data-state",
    "running",
  );
  // Ein Klick auf den laufenden Eintrag zeigt wieder die Statusleiste mit Stopp.
  await again.click();
  await expect(
    page.getByTestId("pm-run").getByTestId("job-panel"),
  ).toBeVisible();

  // Stopp (mit Rueckfrage): der Lauf endet ohne Ergebnis, nichts steht im Projekt.
  await page.getByTestId("job-stop").click();
  await page.getByTestId("job-stop-confirm").click();
  await expect(page.getByTestId("pm-run")).toHaveCount(0);
  await expect(page.getByTestId("pm-row-running")).toHaveCount(0);
  await expect(page.getByTestId("pm-row")).toHaveCount(0);
  const stops = await calls(page, "meetings_job_stop");
  expect(stops.at(-1)!.args).toEqual({ meetingId: PM_KEY });
  expect(await calls(page, "project_minutes_generate")).toHaveLength(1);
});

test("ein Fehler des Backends bleibt in der Ansicht stehen und nennt den Grund", async ({
  page,
}) => {
  await setup(page);
  await failProjectMinutes(page, "no_transcript");
  await startSelecting(page);
  await pickThree(page);
  await startRun(page);

  const error = page.getByTestId("pm-run-error");
  await expect(error).toBeVisible();
  await expect(error).toContainText("hat kein Transkript");
  await expect(page.getByTestId("pm-row")).toHaveCount(0);
  await page.getByTestId("pm-run-close").click();
  await expect(page.getByTestId("pm-run")).toHaveCount(0);

  // Ein neuer Versuch ohne Fehler geht durch.
  await failProjectMinutes(page, null);
  await startSelecting(page);
  await pickThree(page);
  await startRun(page);
  await expect(page.getByTestId("pm-view")).toBeVisible();
});

// ---------------------------------------------------------------------------
// Ergebnis im Projekt
// ---------------------------------------------------------------------------

test("das Ergebnis steht im Projekt als Eintrag, ist erneut oeffnbar und traegt seine Herkunft", async ({
  page,
}) => {
  await setup(page);
  await finishWithDoc(page);

  // Titel, Art, Zahl der Aufnahmen, Vorlage.
  await expect(page.getByTestId("pm-title")).toHaveText(
    "Projekt-Protokoll: Geschäftlich",
  );
  await expect(page.getByTestId("pm-kind-label")).toHaveText(
    "Projekt-Protokoll",
  );
  await expect(page.getByTestId("pm-recording-count")).toContainText(
    "3 Aufnahmen",
  );
  await expect(page.getByTestId("pm-template")).toHaveText("Allgemein");

  // Abschnitte der Vorlage mit Aussagen; Aufgabe mit Verantwortlichem und Termin.
  const sections = page.getByTestId("pm-section");
  await expect(sections).toHaveCount(3);
  await expect(sections.nth(0).getByRole("heading")).toHaveText(
    "Zusammenfassung",
  );
  await expect(sections.nth(2).getByTestId("pm-entry")).toContainText(
    "Kurzfassung schicken",
  );
  await expect(sections.nth(2).getByTestId("pm-entry")).toContainText(
    "Wer: Patrick",
  );
  await expect(sections.nth(2).getByTestId("pm-entry")).toContainText(
    "Bis: Freitag",
  );
  // Eine Aussage ohne Beleg ist markiert, nicht verschwiegen.
  const unsupported = page.locator(
    '[data-testid="pm-entry"][data-unsupported="true"]',
  );
  await expect(unsupported).toHaveCount(1);
  await expect(unsupported.getByTestId("pm-no-proof")).toHaveText("ohne Beleg");
  await expect(page.getByTestId("pm-proof-note")).toContainText(
    "Eine Aussage hat keinen Beleg",
  );

  // Quellaufnahmen chronologisch: A1 ist die aelteste.
  const recordings = page.getByTestId("pm-recording");
  await expect(recordings).toHaveCount(3);
  await expect(recordings.nth(0)).toContainText("Elternabend Klasse 3b");
  await expect(recordings.nth(2)).toContainText("Jour fixe Vertrieb KW 40");

  // Herkunft: Modell, Vorlage, Zeitpunkt, Verfahren.
  const origin = page.getByTestId("pm-origin");
  await origin.locator("summary").click();
  await expect(origin.getByTestId("pm-origin-model")).toHaveText(
    "test-model (custom)",
  );
  await expect(origin.getByTestId("pm-origin-template")).toHaveText(
    "Allgemein",
  );
  await expect(origin.getByTestId("pm-origin-created")).not.toHaveText("");
  await expect(origin.getByTestId("pm-origin-mode")).toHaveText(
    "ein Durchlauf",
  );
  await shoot(page, "g3-ergebnis");

  // Im Projekt als Eintrag "Projekt-Protokoll" sichtbar ...
  const row = page.getByTestId("pm-row");
  await expect(row).toHaveCount(1);
  await expect(row).toContainText("Projekt-Protokoll");
  await expect(row).toContainText("3 Aufnahmen");
  await expect(row).toHaveAttribute("aria-current", "true");

  // ... und erneut oeffnbar: erst eine Aufnahme ansehen, dann zurueck.
  await rowOf(page, "m4").click();
  await expect(page.getByTestId("pm-view")).toHaveCount(0);
  await expect(rowOf(page, "m4")).toHaveAttribute("aria-current", "true");
  await page.getByTestId("pm-row").click();
  await expect(page.getByTestId("pm-view")).toBeVisible();
  await expect(page.getByTestId("pm-title")).toHaveText(
    "Projekt-Protokoll: Geschäftlich",
  );
  // Nach dem Neuladen steht es weiter im Projekt (Ablage im Backend).
  await page.reload();
  await goToRecordings(page);
  await expect(page.getByTestId("pm-row")).toHaveCount(1);
});

test("Export wie beim Protokoll (Dialog, Word/Text/Markdown) und Loeschen mit Rueckfrage", async ({
  page,
}) => {
  await setup(page);
  await finishWithDoc(page);

  // Export: derselbe Weg wie beim Einzelprotokoll (Speichern-Dialog, `meetings_export_document`).
  await page.getByTestId("pm-export").click();
  await expect
    .poll(async () => (await calls(page, "meetings_export_document")).length)
    .toBe(1);
  const saves = await calls(page, "plugin:dialog|save");
  expect(saves).toHaveLength(1);
  const exported = (await calls(page, "meetings_export_document"))[0].args as {
    body: string;
  };
  expect(exported.body).toContain("# Projekt-Protokoll: Geschäftlich");
  await expect(page.getByTestId("pm-saved")).toBeVisible();

  // Loeschen: Rueckfrage, danach weg aus der Ansicht und aus dem Projekt.
  await page.getByTestId("pm-delete").click();
  await expect(page.getByRole("dialog")).toContainText(
    "Projekt-Protokoll löschen?",
  );
  await page.getByTestId("pm-delete-cancel").click();
  await expect(page.getByTestId("pm-row")).toHaveCount(1);
  await page.getByTestId("pm-delete").click();
  await page.getByTestId("pm-delete-confirm").click();
  await expect(page.getByTestId("pm-view")).toHaveCount(0);
  await expect(page.getByTestId("pm-row")).toHaveCount(0);
  expect(await calls(page, "project_minutes_delete")).toHaveLength(1);
});

// ---------------------------------------------------------------------------
// Quellen
// ---------------------------------------------------------------------------

test("Quellen-Klick oeffnet die Aufnahme und springt zur Stelle im Transkript und im Ton", async ({
  page,
}) => {
  await setup(page);
  await finishWithDoc(page);

  // Jede Aussage traegt ihre Herkunft: Aufnahme und Zeit.
  const first = page.getByTestId("pm-entry").first();
  const chips = first.getByTestId("pm-source");
  await expect(chips).toHaveCount(2);
  await expect(chips.nth(0)).toContainText("A1");
  await expect(chips.nth(0)).toContainText("Elternabend");
  await expect(chips.nth(0)).toContainText("00:33");
  await expect(chips.nth(1)).toContainText("A3");
  await expect(chips.nth(1)).toContainText("02:29");
  await expect(chips.nth(1)).toHaveAttribute("data-source-meeting", "m1");
  await expect(chips.nth(1)).toHaveAttribute(
    "title",
    "Jour fixe Vertrieb KW 40, 02:29: öffnet die Aufnahme an dieser Stelle",
  );

  // Klick auf A3 (Jour fixe, Segment 5 = 149 s): die Aufnahme wird gewaehlt, das
  // Segment markiert, der Ton springt dorthin.
  await chips.nth(1).click();
  await expect(rowOf(page, "m1")).toHaveAttribute("aria-current", "true");
  await expect(page.getByTestId("pm-view")).toHaveCount(0);
  const segment = page.locator('[data-segment-index="5"]');
  await expect(segment).toBeVisible();
  await expect(segment).toHaveAttribute("data-highlighted", "true");
  await expect
    .poll(() => page.evaluate(() => (window as any).__played))
    .toEqual([149]);

  // Zurueck zum Dokument und die erste Quelle: eine andere Aufnahme, andere Zeit.
  await page.getByTestId("pm-row").click();
  await page
    .getByTestId("pm-entry")
    .first()
    .getByTestId("pm-source")
    .first()
    .click();
  await expect(rowOf(page, "m4")).toHaveAttribute("aria-current", "true");
  const seg1 = page.locator('[data-segment-index="1"]');
  await expect(seg1).toHaveAttribute("data-highlighted", "true");
  await expect
    .poll(() => page.evaluate(() => (window as any).__played))
    .toEqual([149, 33]);
});

test("eine Quellaufnahme, die es nicht mehr gibt, meldet das und laesst das Dokument stehen", async ({
  page,
}) => {
  await setup(page);
  await finishWithDoc(page);
  await page.evaluate(() => {
    const w = window as any;
    w.__meetings = w.__meetings.filter((m: any) => m.id !== "m1");
  });
  await page
    .getByTestId("pm-entry")
    .first()
    .getByTestId("pm-source")
    .nth(1)
    .click();
  await expect(page.locator("[data-sonner-toast]")).toContainText(
    /gibt es nicht mehr|gelöscht/,
  );
  await expect(page.getByTestId("pm-view")).toBeVisible();
  // Die Quellenliste im Dokument nennt sie weiter (Stand zur Zeit der Erzeugung).
  await expect(page.getByTestId("pm-recording")).toHaveCount(3);
});

test("schmales Fenster: Auswahl und Ergebnis bleiben ohne waagerechtes Scrollen bedienbar", async ({
  page,
}) => {
  await setup(page, 480, 800);
  await page.getByTestId("sessions-open").click();
  await projectRow(page, PM_FOLDER).click();
  await page.getByTestId("projects-more").click();
  await page.getByTestId("projects-pm").click();
  await expect(page.getByTestId("pm-bar")).toBeVisible();
  const overflow = await page.evaluate(
    () => document.scrollingElement!.scrollWidth - innerWidth,
  );
  expect(overflow).toBeLessThanOrEqual(1);
  await pickThree(page);
  await startRun(page);
  await expect(page.getByTestId("pm-view")).toBeVisible();
  const after = await page.evaluate(
    () => document.scrollingElement!.scrollWidth - innerWidth,
  );
  expect(after).toBeLessThanOrEqual(1);
});

test("Zeilen und Kontextmenue anderer Seiten bleiben unveraendert, wenn nichts ausgewaehlt wird", async ({
  page,
}) => {
  await setup(page);
  await projectRow(page, PM_FOLDER).click();
  await expect(rowOf(page, "m1")).toHaveAttribute("role", "button");
  await expect(page.getByTestId("pm-list")).toHaveCount(0);
  await expect(page.getByTestId("pm-bar")).toHaveCount(0);
  // Die bestehende "Auswahl fragen" arbeitet weiter und kennt kein Ausgrauen.
  await page.getByTestId("projects-more").click();
  await page.getByTestId("projects-select").click();
  await expect(rowOf(page, "m5")).toHaveAttribute("role", "checkbox");
  await expect(rowOf(page, "m5")).not.toHaveAttribute("aria-disabled", "true");
});

// ---------------------------------------------------------------------------
// Barrierefreiheit (axe, inkl. color-contrast)
// ---------------------------------------------------------------------------

const axeSevere = async (page: Page) =>
  (
    await new AxeBuilder({ page })
      .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "best-practice"])
      .analyze()
  ).violations
    .filter((v) => v.impact === "critical" || v.impact === "serious")
    .map(
      (v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(" | ")}`,
    );

test("axe: Auswahl mit ausgegrauten Zeilen, Dialog, laufender Lauf und Ergebnis ohne schwere Befunde", async ({
  page,
}) => {
  await setup(page);
  await holdProjectMinutes(page);
  await startSelecting(page);
  await pickThree(page);
  expect(await axeSevere(page), "Auswahl").toEqual([]);

  await page.getByTestId("pm-start").click();
  await expect(page.getByTestId("pm-dialog")).toBeVisible();
  expect(await axeSevere(page), "Dialog").toEqual([]);
  await page.getByTestId("pm-dialog-start").click();
  await expect(
    page.getByTestId("pm-run").getByTestId("job-panel"),
  ).toBeVisible();
  expect(await axeSevere(page), "laufender Lauf").toEqual([]);

  await releaseProjectMinutes(page);
  await expect(page.getByTestId("pm-view")).toBeVisible();
  expect(await axeSevere(page), "Ergebnis").toEqual([]);
});
