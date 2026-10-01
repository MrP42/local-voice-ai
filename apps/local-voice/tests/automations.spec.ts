import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { installTauriMock } from "./calendarMock";
import {
  DEMO,
  installIntegrationsMock,
  NOW,
  type IntegrationsMockOptions,
} from "./integrationsMock";
import {
  AUTOMATIONS_DEMO,
  installAutomationsMock,
  wfCalls,
  type AutomationsMockOptions,
} from "./automationsMock";

// B7 (Goal Workflow-Automation, #67, AK9): die Oberflaeche „Automationen“ als Reiter auf der
// Seite „Integrationen“ (kein neuer Seitenleisten-Eintrag): Ablauf aus Vorlage anlegen,
// Ausloeser, Bedingung und Schritt aendern, speichern, Trockenlauf anzeigen, Laufprotokoll und
// Freigabe bedienen, JSON exportieren und wieder importieren (derselbe Ablauf).
// Gegen die Tauri-Attrappe; Pruefung, Plan und Rechte rechnet in Wahrheit das Backend
// (`cargo test --lib workflows::ui`).

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

const setup = async (
  page: Page,
  opts: {
    integrations?: IntegrationsMockOptions;
    automations?: AutomationsMockOptions;
    width?: number;
    height?: number;
    theme?: "light" | "dark";
  } = {},
) => {
  await page.clock.setFixedTime(NOW);
  await installTauriMock(page, "main");
  await installIntegrationsMock(page, opts.integrations ?? DEMO);
  await installAutomationsMock(page, opts.automations ?? AUTOMATIONS_DEMO);
  if (opts.theme) {
    await page.addInitScript((theme) => {
      (window as any).__settings.theme = theme;
    }, opts.theme);
  }
  // Erst breit navigieren (schmal liegt „Integrationen“ hinter „Mehr“), dann verkleinern.
  await page.setViewportSize({ width: 1280, height: opts.height ?? 1000 });
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Integrationen", exact: true })
    .click();
  await page.getByRole("tab", { name: "Automationen" }).click();
  await expect(page.getByTestId("automations-panel")).toBeVisible();
  if (opts.width) {
    await page.setViewportSize({
      width: opts.width,
      height: opts.height ?? 1000,
    });
  }
};

/** Wählt in einem Auswahlfeld (react-select) einen Eintrag. */
const pick = async (page: Page, label: string, option: string) => {
  await page.getByRole("combobox", { name: label, exact: true }).click();
  await page.getByRole("option", { name: option, exact: true }).click();
};

const card = (page: Page, id: string) =>
  page.locator(`[data-testid="workflow-card"][data-workflow-id="${id}"]`);

const OUT_NAME = "Eingangsordner: transkribieren, Protokoll als Word";

const newFromTemplate = async (page: Page, name = "Eingangsordner") => {
  await page.getByTestId("automations-new").click();
  await expect(page.getByTestId("template-list")).toBeVisible();
  await page
    .getByTestId("template-item")
    .filter({ hasText: name })
    .getByTestId("template-use")
    .click();
  await expect(page.getByTestId("workflow-editor")).toBeVisible();
};

const lastSaved = async (page: Page) => {
  const saves = await wfCalls(page, "workflow_save");
  return saves[saves.length - 1];
};

// ---------------------------------------------------------------------------
// Ort und Liste
// ---------------------------------------------------------------------------

test("Automationen ist ein Reiter auf der Seite Integrationen, kein neuer Menüpunkt", async ({
  page,
}) => {
  await setup(page);
  await expect(
    page
      .getByRole("navigation")
      .getByRole("button", { name: "Automationen", exact: true }),
  ).toHaveCount(0);
  await expect(page.getByRole("tab", { name: "Automationen" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(
    page.getByRole("heading", { name: "Integrationen", level: 1 }),
  ).toBeVisible();
});

test("Die Liste zeigt an/aus, scharf oder Trockenlauf, letzten Lauf und den Stand der Auslöser", async ({
  page,
}) => {
  await setup(page);
  await expect(page.getByTestId("workflow-card")).toHaveCount(2);

  const eingang = card(page, "wf-eingang");
  await expect(eingang.getByTestId("workflow-name")).toHaveText(
    "Eingangsordner: Protokoll als Word",
  );
  await expect(eingang.getByTestId("workflow-mode")).toHaveText("Scharf");
  await expect(eingang.getByTestId("workflow-enabled")).toBeChecked();
  await expect(eingang.getByTestId("workflow-lastrun")).toContainText(
    "wartet auf Freigabe",
  );
  await expect(eingang).toContainText("Datei im Ordner · 3 Schritte");

  const kanal = card(page, "wf-kanal");
  await expect(kanal.getByTestId("workflow-mode")).toHaveText("Trockenlauf");
  await expect(kanal.getByTestId("workflow-enabled")).not.toBeChecked();
  await expect(kanal.getByTestId("workflow-start")).toBeDisabled();

  // Ordner-Platzhalter und Kanal-Ausfall
  await expect(page.getByTestId("status-cloud")).toContainText(
    "Besprechung-Vertrieb.mp4",
  );
  await expect(page.getByTestId("status-channels")).toContainText(
    "UC0123456789012345678901",
  );
  await expect(page.getByTestId("status-channels")).toContainText(
    "4 Fehlversuche in Folge",
  );
});

test("Ein/Aus, scharf schalten (mit Rückfrage), zurück in den Trockenlauf und Löschen (mit Rückfrage)", async ({
  page,
}) => {
  await setup(page);
  const kanal = card(page, "wf-kanal");
  await kanal.getByTestId("workflow-enabled").check();
  await expect(kanal.getByTestId("workflow-enabled")).toBeChecked();
  expect(await wfCalls(page, "workflow_set_enabled")).toHaveLength(1);

  // Scharf schalten braucht eine Bestätigung.
  await kanal.getByTestId("workflow-arm").click();
  await expect(kanal.getByTestId("arm-confirm")).toBeVisible();
  expect(await wfCalls(page, "workflow_set_armed")).toHaveLength(0);
  await kanal.getByTestId("arm-confirm-yes").click();
  await expect(kanal.getByTestId("workflow-mode")).toHaveText("Scharf");
  await expect(page.getByTestId("automations-notice")).toContainText(
    "„Kanal beobachten“ ist scharf geschaltet.",
  );
  await expect(kanal.getByTestId("workflow-start")).toBeEnabled();
  await kanal.getByTestId("workflow-disarm").click();
  await expect(kanal.getByTestId("workflow-mode")).toHaveText("Trockenlauf");

  // Löschen: erst nach der Rückfrage
  await kanal.getByTestId("workflow-delete").click();
  await expect(kanal.getByTestId("delete-confirm")).toBeVisible();
  expect(await wfCalls(page, "workflow_delete")).toHaveLength(0);
  await kanal.getByTestId("delete-confirm-yes").click();
  await expect(page.getByTestId("workflow-card")).toHaveCount(1);
  await expect(page.getByTestId("automations-notice")).toContainText(
    "„Kanal beobachten“ wurde gelöscht.",
  );
});

test("Ohne Abläufe bietet die Seite den Einstieg über eine Vorlage an", async ({
  page,
}) => {
  await setup(page, { automations: {} });
  await expect(page.getByTestId("workflows-empty")).toBeVisible();
  await page.getByTestId("workflows-empty").getByRole("button").click();
  await expect(page.getByTestId("template-list")).toBeVisible();
  await expect(page.getByTestId("template-item")).toHaveCount(2);
});

// ---------------------------------------------------------------------------
// AK9: aus Vorlage anlegen, ändern, speichern, Trockenlauf
// ---------------------------------------------------------------------------

test("AK9: Ablauf aus Vorlage anlegen, Auslöser, Bedingung und Schritt ändern, speichern und den Trockenlauf anzeigen", async ({
  page,
}) => {
  await setup(page);
  await newFromTemplate(page);
  await expect(page.getByTestId("editor-name")).toHaveValue(OUT_NAME);
  await expect(page.getByTestId("step-editor")).toHaveCount(3);

  // Auslöser ändern: Quelle wählen und Endungen setzen
  await pick(page, "Quelle (Integration)", "Ablage Berichte (ordner-berichte)");
  const ext = page.getByTestId("trigger-extensions");
  await ext.fill("wav, mp3, m4a");
  await ext.blur();
  await expect(ext).toHaveValue("wav, mp3, m4a");

  // Bedingung ändern
  await page.getByTestId("step-1-when").fill("{{trigger.size}} > 1000");

  // Schritt ändern: Dateiname und Zielordner des Exports
  await page.getByTestId("step-2-name").fill("Mein Protokoll");
  await pick(page, "Zielordner", "Ablage Berichte (ordner-berichte)");

  // Schritt hinzufügen: ein Pflichtfeld fehlt, der Befund steht mit Pfad da
  await pick(page, "Schritt hinzufügen", "Windows-Mitteilung");
  await expect(page.getByTestId("step-editor")).toHaveCount(4);
  await expect(page.getByTestId("editor-issues")).toContainText(
    "/steps/3/params/title",
  );
  await expect(page.getByTestId("step-3-title-error")).toBeVisible();
  await page.getByTestId("step-3-title").fill("Protokoll fertig");
  await expect(page.getByTestId("editor-issues")).toHaveCount(0);

  // Speichern
  await page.getByTestId("editor-save").click();
  await expect(page.getByTestId("editor-saved")).toBeVisible();
  const saved = JSON.parse((await lastSaved(page)).args.definitionJson);
  expect((await lastSaved(page)).args.id).toBeNull();
  expect(saved.trigger.integration).toBe("ordner-berichte");
  expect(saved.trigger.extensions).toEqual(["wav", "mp3", "m4a"]);
  expect(saved.steps).toHaveLength(4);
  expect(saved.steps[1].when).toBe("{{trigger.size}} > 1000");
  expect(saved.steps[2].params.name).toBe("Mein Protokoll");
  expect(saved.steps[2].params.target).toBe("ordner-berichte");
  expect(saved.steps[3]).toMatchObject({
    action: "notify.local",
    params: { title: "Protokoll fertig" },
  });
  // Die Variablen und Felder, die der Editor nicht anfasst, bleiben erhalten.
  expect(saved.variables.protokoll_vorlage.type).toBe("string");
  expect(saved.steps[0].params.via).toBe("{{trigger.integration}}");

  // Trockenlauf aus dem Editor
  await page.getByTestId("editor-plan").click();
  await expect(page.getByTestId("plan-view")).toBeVisible();
  await expect(page.getByTestId("plan-step")).toHaveCount(4);
  await expect(page.getByTestId("plan-summary")).toHaveAttribute(
    "data-ok",
    "true",
  );
  await expect(
    page.locator('[data-testid="plan-step"][data-step-id="protokoll"]'),
  ).toContainText("hängt von einem Vorschritt ab");
  await page.getByRole("button", { name: "Schließen" }).last().click();

  // Zurück zur Liste: der neue Ablauf ist ausgeschaltet und im Trockenlauf
  await page.getByTestId("editor-back").click();
  await expect(page.getByTestId("workflow-card")).toHaveCount(3);
  const created = page
    .getByTestId("workflow-card")
    .filter({ hasText: OUT_NAME });
  await expect(created.getByTestId("workflow-mode")).toHaveText("Trockenlauf");
  await expect(created.getByTestId("workflow-enabled")).not.toBeChecked();
});

test("Ein zweites Speichern ersetzt denselben Ablauf (keine Kopie)", async ({
  page,
}) => {
  await setup(page);
  await card(page, "wf-kanal").getByTestId("workflow-edit").click();
  await page.getByTestId("editor-name").fill("Kanal beobachten (neu)");
  await page.getByTestId("editor-save").click();
  await expect(page.getByTestId("editor-saved")).toBeVisible();
  await page.getByTestId("editor-name").fill("Kanal beobachten (neu 2)");
  await page.getByTestId("editor-save").click();
  const saves = await wfCalls(page, "workflow_save");
  expect(saves).toHaveLength(2);
  expect(saves[0].args.id).toBe("wf-kanal");
  expect(saves[1].args.id).toBe("wf-kanal");
  await page.getByTestId("editor-back").click();
  await expect(page.getByTestId("workflow-card")).toHaveCount(2);
});

test("Befunde der Prüfung stehen mit JSON-Zeiger am Feld und gesammelt; feste Felder haben keinen Variablen-Umschalter", async ({
  page,
}) => {
  await setup(page);
  await page.getByTestId("automations-new").click();
  await page.getByTestId("template-blank").click();
  await expect(page.getByTestId("steps-empty")).toBeVisible();
  // Leer: noch keine Befunde, aber Speichern geht nicht ohne Namen.
  await expect(page.getByTestId("editor-save")).toBeDisabled();
  await pick(page, "Schritt hinzufügen", "Mail senden");
  const issues = page.getByTestId("editor-issues");
  await expect(issues).toContainText("/name");
  await expect(issues).toContainText("/steps/0/params/via");
  await expect(issues).toContainText("/steps/0/params/to");
  await expect(issues).toContainText("/steps/0/params/subject");
  await expect(issues).toContainText("Schritt 1 (send)");
  await expect(page.getByTestId("step-0-subject-error")).toContainText(
    "Pflichtfeld",
  );
  // Der Empfänger ist ein fester Wert: kein Umschalter auf {{…}}.
  await expect(page.getByTestId("step-0-to-dynamic")).toHaveCount(0);
  await expect(page.getByTestId("step-0-via-dynamic")).toHaveCount(1);

  await page.getByTestId("editor-name").fill("Mail an mich");
  await pick(page, "Empfänger", "Nur ich");
  await page.getByTestId("step-0-subject").fill("Hallo");
  await page.getByTestId("step-0-via-dynamic").click();
  await page.getByTestId("step-0-via").fill("{{trigger.integration}}");
  await expect(issues).toHaveCount(0);
  await page.getByTestId("editor-save").click();
  await expect(page.getByTestId("editor-saved")).toBeVisible();
  const saved = JSON.parse((await lastSaved(page)).args.definitionJson);
  expect(saved.steps[0].params).toEqual({
    via: "{{trigger.integration}}",
    to: "me",
    subject: "Hallo",
  });
});

test("Speichern scheitert mit allen Befunden, ohne etwas zu schreiben; Verlassen fragt bei Änderungen nach", async ({
  page,
}) => {
  await setup(page);
  await newFromTemplate(page);
  await page.getByTestId("step-0-path").fill("");
  await page.getByTestId("editor-name").fill("Mit Fehler");
  await expect(page.getByTestId("editor-issues")).toContainText(
    "/steps/0/params/path",
  );
  await page.getByTestId("editor-save").click();
  await expect(page.getByTestId("editor-save-error")).toContainText(
    "Nicht gespeichert: 1 Befund.",
  );
  await expect(page.getByTestId("editor-saved")).toHaveCount(0);

  await page.getByTestId("editor-back").click();
  await expect(page.getByTestId("editor-leave-confirm")).toBeVisible();
  await page.getByRole("button", { name: "Weiter bearbeiten" }).click();
  await expect(page.getByTestId("editor-leave-confirm")).toHaveCount(0);
  await page.getByTestId("editor-back").click();
  await page.getByTestId("editor-discard").click();
  await expect(page.getByTestId("workflow-card")).toHaveCount(2);
});

test("Schritte lassen sich verschieben und entfernen; Variablen bearbeiten", async ({
  page,
}) => {
  await setup(page);
  await card(page, "wf-eingang").getByTestId("workflow-edit").click();
  const ids = () =>
    page
      .getByTestId("step-editor")
      .evaluateAll((els) => els.map((e) => e.getAttribute("data-step-id")));
  expect(await ids()).toEqual(["import", "protokoll", "word"]);
  await page.getByTestId("step-2-up").click();
  expect(await ids()).toEqual(["import", "word", "protokoll"]);
  await page.getByTestId("step-0-remove").click();
  expect(await ids()).toEqual(["word", "protokoll"]);

  // Variable umbenennen und eine neue anlegen
  await expect(page.getByTestId("variable-row")).toHaveCount(1);
  await page.getByTestId("variable-0-default").fill("einkauf");
  await page.getByTestId("variable-add").click();
  await expect(page.getByTestId("variable-row")).toHaveCount(2);
  await page.getByTestId("variable-1-name").fill("anzahl");
  await page.getByRole("combobox", { name: "Art", exact: true }).nth(1).click();
  await page.getByRole("option", { name: "Zahl", exact: true }).click();
  await page.getByTestId("editor-save").click();
  const saved = JSON.parse((await lastSaved(page)).args.definitionJson);
  expect(saved.steps.map((s: any) => s.id)).toEqual(["word", "protokoll"]);
  expect(saved.variables.protokoll_vorlage.default).toBe("einkauf");
  expect(saved.variables.anzahl.type).toBe("number");
});

// ---------------------------------------------------------------------------
// Trockenlauf aus der Liste
// ---------------------------------------------------------------------------

test("Trockenlauf anzeigen: jeder Schritt mit Wirkung, Bedingung und Rechte-Ergebnis", async ({
  page,
}) => {
  await setup(page);
  await card(page, "wf-eingang").getByTestId("workflow-plan").click();
  await expect(page.getByTestId("plan-view")).toBeVisible();
  await expect(page.getByTestId("plan-step")).toHaveCount(3);
  await expect(page.getByTestId("plan-summary")).toContainText("3 Schritte");
  await expect(page.getByTestId("plan-summary")).toHaveAttribute(
    "data-ok",
    "true",
  );
  await expect(
    page.getByText("Die Daten des Auslösers sind Beispieldaten."),
  ).toBeVisible();
  const word = page.locator('[data-testid="plan-step"][data-step-id="word"]');
  await expect(word).toContainText(
    "Dokument als docx in ordner-berichte ablegen",
  );
  await expect(word.getByTestId("plan-permission")).toHaveText("erlaubt");
  const calls = await wfCalls(page, "workflow_plan");
  expect(calls[0].args.workflowId).toBe("wf-eingang");
});

test("Eine Vorlage mit Platzhalter-Ordnern zeigt im Trockenlauf, was noch fehlt", async ({
  page,
}) => {
  await setup(page);
  await newFromTemplate(page);
  await page.getByTestId("editor-plan").click();
  await expect(page.getByTestId("plan-summary")).toHaveAttribute(
    "data-ok",
    "false",
  );
  await expect(page.getByTestId("plan-summary")).toContainText(
    "Es braucht noch Rechte oder Korrekturen.",
  );
  await expect(
    page
      .getByTestId("plan-permission")
      .filter({ hasText: "abgelehnt" })
      .first(),
  ).toBeVisible();
});

test("Probelauf starten legt einen Lauf an und öffnet ihn", async ({
  page,
}) => {
  await setup(page);
  await card(page, "wf-kanal").getByTestId("workflow-probe").click();
  await expect(page.getByTestId("run-detail")).toBeVisible();
  await expect(page.getByTestId("run-origin")).toContainText("Von Hand");
  await expect(page.getByTestId("run-origin")).toContainText("Trockenlauf");
  await expect(page.getByTestId("step-state")).toHaveText("geplant");
  await expect(page.getByTestId("run-provenance-empty")).toContainText(
    "Ein Trockenlauf erzeugt nichts",
  );
  const start = (await wfCalls(page, "workflow_run_start"))[0];
  expect(start.args).toMatchObject({ id: "wf-kanal", dryRun: true });
  await page.getByTestId("run-back").click();
  await expect(page.getByTestId("workflow-card")).toHaveCount(2);
});

// ---------------------------------------------------------------------------
// Laufprotokoll, Herkunft, Abbrechen, Wiederholen, Freigabe
// ---------------------------------------------------------------------------

const openRuns = async (page: Page) => {
  await page.getByRole("tab", { name: "Läufe", exact: true }).click();
  await expect(page.getByTestId("run-list")).toBeVisible();
};

const openRun = async (page: Page, id: string) => {
  await page
    .locator(`[data-testid="run-row"][data-run-id="${id}"]`)
    .getByTestId("run-open")
    .click();
  await expect(page.getByTestId("run-detail")).toHaveAttribute(
    "data-run-id",
    id,
  );
};

test("Das Laufprotokoll listet die Läufe, filtert nach Ablauf und nach offenen Läufen", async ({
  page,
}) => {
  await setup(page);
  await openRuns(page);
  await expect(page.getByTestId("run-row")).toHaveCount(4);
  await expect(
    page.locator('[data-run-id="run-fail"]').getByTestId("run-state"),
  ).toHaveText("gescheitert");
  await page.getByTestId("runs-open-only").check();
  await expect(page.getByTestId("run-row")).toHaveCount(1);
  await page.getByTestId("runs-open-only").uncheck();

  // Filter über die Karte
  await page.getByRole("tab", { name: "Abläufe", exact: true }).click();
  await card(page, "wf-kanal").getByTestId("workflow-runs").click();
  await expect(page.getByTestId("run-list")).toContainText(
    "Läufe von „Kanal beobachten“",
  );
  await expect(page.getByTestId("run-row")).toHaveCount(1);
  await page.getByTestId("runs-clear-filter").click();
  await expect(page.getByTestId("run-row")).toHaveCount(4);
});

test("Ein Lauf zeigt Schritte, Herkunft des Starts und Herkunft der Ergebnisse", async ({
  page,
}) => {
  await setup(page);
  await openRuns(page);
  await openRun(page, "run-ok");
  await expect(page.getByTestId("run-state").first()).toHaveText("fertig");
  await expect(page.getByTestId("run-origin")).toHaveText("Auslöser");
  await expect(page.getByTestId("run-step")).toHaveCount(3);
  await expect(
    page.locator('[data-testid="run-step"][data-step-id="import"]'),
  ).toContainText("Datei importieren und transkribieren");
  const prov = page.getByTestId("run-provenance");
  await expect(prov).toHaveCount(1);
  await expect(prov).toContainText("minutes");
  await expect(prov).toContainText("Gemma 4 E4B");
  await expect(prov).toContainText("llama.cpp");
  await expect(prov).toContainText("42 s");
  await expect(prov).toContainText("1 Quelle");
  // Beendet: weder abbrechen noch wiederholen
  await expect(page.getByTestId("run-cancel")).toHaveCount(0);
  await expect(page.getByTestId("run-retry")).toHaveCount(0);
  // Die Daten des Auslösers stehen auf Wunsch da
  await page.getByText("Daten des Auslösers").click();
  await expect(page.getByText("Eingang-Bericht.wav").first()).toBeVisible();
});

test("Ein gescheiterter Lauf lässt sich ab dem gescheiterten Schritt wiederholen", async ({
  page,
}) => {
  await setup(page);
  await openRuns(page);
  await openRun(page, "run-fail");
  await expect(page.getByTestId("run-error")).toHaveText("Zugriff verweigert");
  await expect(page.getByTestId("run-retry")).toBeEnabled();
  await page.getByTestId("run-retry").click();
  await expect(page.getByTestId("run-state").first()).toHaveText("fertig");
  const retry = (await wfCalls(page, "workflow_run_retry"))[0];
  expect(retry.args).toMatchObject({
    runId: "run-fail",
    acceptUncertain: false,
  });
  // Beide Versuche stehen im Protokoll
  await expect(
    page.locator('[data-testid="run-step"][data-step-id="word"]'),
  ).toHaveCount(2);
  await expect(page.getByText("Versuch 2")).toBeVisible();
});

test("Bei unklarer Wirkung braucht das Wiederholen die ausdrückliche Bestätigung", async ({
  page,
}) => {
  await setup(page);
  await openRuns(page);
  await openRun(page, "run-unsure");
  await expect(page.getByTestId("step-state")).toHaveText("Wirkung unklar");
  await expect(page.getByTestId("run-retry")).toBeDisabled();
  await page.getByTestId("run-retry-accept").check();
  await expect(page.getByTestId("run-retry")).toBeEnabled();
  await page.getByTestId("run-retry").click();
  await expect(page.getByTestId("run-state").first()).toHaveText("fertig");
  expect((await wfCalls(page, "workflow_run_retry"))[0].args).toMatchObject({
    acceptUncertain: true,
  });
});

test("Ein wartender Lauf zeigt die Freigabe; sie wird im vorhandenen Dialog entschieden und der Lauf geht weiter", async ({
  page,
}) => {
  await setup(page);
  await openRuns(page);
  await openRun(page, "run-wait");
  await expect(page.getByTestId("run-state").first()).toHaveText(
    "wartet auf Freigabe",
  );
  await expect(page.getByTestId("run-approval")).toContainText(
    "wartet auf Ihre Freigabe",
  );
  await expect(page.getByTestId("run-approval-preview")).toContainText(
    "Protokoll-2026-09-30.md",
  );
  await expect(page.getByTestId("run-cancel")).toBeVisible();
  await page.getByTestId("run-approval-open").click();
  await expect(page.getByTestId("approval-dialog")).toBeVisible();
  await page
    .locator('[data-testid="approval-item"][data-approval-id="ap-2"]')
    .getByTestId("approval-allow")
    .click();
  await expect(page.getByTestId("run-state").first()).toHaveText("fertig");
  await expect(page.getByTestId("run-approval")).toHaveCount(0);
  expect(await wfCalls(page, "workflow_approvals_changed")).toHaveLength(1);
});

test("Eine verweigerte Freigabe beendet den Lauf als abgelehnt", async ({
  page,
}) => {
  await setup(page);
  await openRuns(page);
  await openRun(page, "run-wait");
  await page.getByTestId("run-approval-open").click();
  await page
    .locator('[data-testid="approval-item"][data-approval-id="ap-2"]')
    .getByTestId("approval-deny")
    .click();
  await expect(page.getByTestId("run-state").first()).toHaveText("gescheitert");
  await expect(
    page.locator('[data-testid="run-step"][data-step-id="word"]'),
  ).toHaveAttribute("data-state", "denied");
});

test("Ein wartender Lauf lässt sich abbrechen", async ({ page }) => {
  await setup(page);
  await openRuns(page);
  await openRun(page, "run-wait");
  await page.getByTestId("run-cancel").click();
  await expect(page.getByTestId("run-state").first()).toHaveText("abgebrochen");
  await expect(page.getByTestId("run-cancel")).toHaveCount(0);
});

// ---------------------------------------------------------------------------
// AK9: JSON exportieren und wieder importieren
// ---------------------------------------------------------------------------

test("AK9: Export, dann Import ergibt denselben Ablauf (als neuer, ausgeschalteter Ablauf)", async ({
  page,
}) => {
  await setup(page);
  await card(page, "wf-eingang").getByTestId("workflow-export").click();
  await expect(page.getByTestId("export-dialog")).toBeVisible();
  const exported = await page.getByTestId("export-text").inputValue();
  const original = await page.evaluate(() =>
    JSON.stringify((window as any).__wf.workflows[0].def),
  );
  expect(JSON.parse(exported)).toEqual(JSON.parse(original));
  await page.getByRole("button", { name: "Schließen" }).last().click();

  await page.getByTestId("automations-import").click();
  await expect(page.getByTestId("import-dialog")).toBeVisible();
  await page.getByTestId("import-text").fill(exported);
  await page.getByTestId("import-submit").click();
  await expect(page.getByTestId("automations-notice")).toContainText(
    "wurde importiert",
  );
  await expect(page.getByTestId("workflow-card")).toHaveCount(3);

  const defs = await page.evaluate(() =>
    (window as any).__wf.workflows.map((w: any) => ({
      id: w.id,
      enabled: w.enabled,
      dry_run: w.dry_run,
      def: w.def,
    })),
  );
  const imported = defs.find((d: any) => d.id.startsWith("wf-neu-"));
  expect(imported.def).toEqual(JSON.parse(original));
  expect(imported.enabled).toBe(false);
  expect(imported.dry_run).toBe(true);

  // Und der zweite Export ist wortgleich zum ersten.
  await page
    .locator(`[data-testid="workflow-card"][data-workflow-id="${imported.id}"]`)
    .getByTestId("workflow-export")
    .click();
  expect(await page.getByTestId("export-text").inputValue()).toBe(exported);
});

test("Ein ungültiger Import nennt alle Befunde mit Pfad und legt nichts an", async ({
  page,
}) => {
  await setup(page);
  await page.getByTestId("automations-import").click();
  await page
    .getByTestId("import-text")
    .fill(
      '{"schema":"lva-workflow@1","name":"","trigger":{"type":"manual"},"steps":[{"id":"a","action":"gibt.es.nicht"}]}',
    );
  await page.getByTestId("import-submit").click();
  const issues = page.getByTestId("import-issues");
  await expect(issues).toContainText("/name");
  await expect(issues).toContainText("/steps/0/action");
  await expect(page.getByTestId("import-dialog")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("workflow-card")).toHaveCount(2);
});

test("Der Import liest auch eine Datei", async ({ page }) => {
  const text = JSON.stringify({
    schema: "lva-workflow@1",
    name: "Aus Datei",
    trigger: { type: "manual" },
    steps: [{ id: "a", action: "wait", params: { minutes: 5 } }],
  });
  await setup(page, {
    integrations: { ...DEMO, pickedPath: "C:\\Ablage\\ablauf.json" },
    automations: {
      ...AUTOMATIONS_DEMO,
      files: { "C:\\Ablage\\ablauf.json": text },
    },
  });
  await page.getByTestId("automations-import").click();
  await page.getByTestId("import-file").click();
  await expect(page.getByTestId("import-text")).toHaveValue(text);
  await page.getByTestId("import-submit").click();
  await expect(
    page.getByTestId("workflow-card").filter({ hasText: "Aus Datei" }),
  ).toBeVisible();
});

// ---------------------------------------------------------------------------
// Katalog generisch: ein unbekannter Baustein stört das Formular nicht
// ---------------------------------------------------------------------------

test("Ein Baustein, den die Oberfläche nicht kennt, erscheint mit dem Titel des Katalogs", async ({
  page,
}) => {
  await setup(page);
  // Der Katalog der Attrappe kennt `webhook.post`; die Oberfläche hat dafür nur den
  // Standardtitel: das Formular entsteht aus den Feldern des Katalogs.
  await page.getByTestId("automations-new").click();
  await page.getByTestId("template-blank").click();
  await pick(page, "Schritt hinzufügen", "Webhook senden");
  await expect(page.getByTestId("step-0-url")).toBeVisible();
  await expect(page.getByTestId("step-0-effect")).toContainText(
    "wirkt nach außen",
  );
});

// ---------------------------------------------------------------------------
// Schmal und Zugänglichkeit
// ---------------------------------------------------------------------------

test.describe("schmal", () => {
  test("Liste, Editor und Lauf laufen bei 380 px nicht über den Rand", async ({
    page,
  }) => {
    await setup(page, { width: 380, height: 900 });
    const overflow = () =>
      page.evaluate(
        () => document.documentElement.scrollWidth - window.innerWidth,
      );
    await expect(page.getByTestId("workflow-card")).toHaveCount(2);
    expect(await overflow()).toBeLessThanOrEqual(0);
    await card(page, "wf-eingang").getByTestId("workflow-edit").click();
    await expect(page.getByTestId("workflow-editor")).toBeVisible();
    expect(await overflow()).toBeLessThanOrEqual(0);
    await page.getByTestId("editor-back").click();
    await openRuns(page);
    await openRun(page, "run-ok");
    expect(await overflow()).toBeLessThanOrEqual(0);
  });
});

for (const theme of ["light", "dark"] as const) {
  test(`axe (WCAG 2 A/AA) ohne serious/critical auf Liste, Editor, Lauf, Trockenlauf und Dialogen - ${theme}`, async ({
    page,
  }) => {
    await setup(page, { theme });
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const findings = async (where: string) => {
      await page.addStyleTag({
        content:
          "*,*::before,*::after{transition:none!important;animation:none!important}",
      });
      await page.waitForTimeout(250);
      const result = await new AxeBuilder({ page })
        .withTags(["wcag2a", "wcag2aa"])
        .exclude("input.peer")
        .analyze();
      const bad = result.violations
        .filter((v) => v.impact === "critical" || v.impact === "serious")
        .map(
          (v) =>
            `${where}: ${v.id} ${v.nodes.map((n) => n.target.join(" ")).join(" | ")}`,
        );
      expect(bad).toEqual([]);
    };
    await expect(page.getByTestId("workflow-card")).toHaveCount(2);
    await findings("Liste");
    await card(page, "wf-eingang").getByTestId("workflow-plan").click();
    await expect(page.getByTestId("plan-view")).toBeVisible();
    await findings("Trockenlauf");
    await page.keyboard.press("Escape");
    await card(page, "wf-eingang").getByTestId("workflow-export").click();
    await expect(page.getByTestId("export-dialog")).toBeVisible();
    await findings("Export");
    await page.keyboard.press("Escape");
    await page.getByTestId("automations-import").click();
    await findings("Import");
    await page.keyboard.press("Escape");
    await card(page, "wf-eingang").getByTestId("workflow-edit").click();
    await expect(page.getByTestId("workflow-editor")).toBeVisible();
    await findings("Editor");
    await page.getByTestId("step-0-path").fill("");
    await expect(page.getByTestId("editor-issues")).toBeVisible();
    await findings("Editor mit Befund");
    await page.getByTestId("editor-back").click();
    await page.getByTestId("editor-discard").click();
    await openRuns(page);
    await findings("Läufe");
    await openRun(page, "run-wait");
    await findings("Lauf mit Freigabe");
    await page.getByTestId("run-approval-open").click();
    await expect(page.getByTestId("approval-dialog")).toBeVisible();
    await findings("Freigabe");
  });
}
