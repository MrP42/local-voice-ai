import { test, expect, type Page } from "@playwright/test";
import { calEvent, calls, installTauriMock } from "./calendarMock";

// B2 (AK4): Einwilligung zur Aufnahme aus einem Ablauf. Das Hinweisfenster zeigt die Bitte
// des Ablaufs und entscheidet erst nach dem Haekchen und dem Klick die Freigabe; die
// Aufnahme beginnt nie ueber `meetings_start*`, sondern erst, wenn der Ablauf nach der
// Freigabe weiterlaeuft. Die Attrappe bildet das Backend nach (`__workflowRun`): ohne
// Entscheidung bleibt der Lauf `awaiting_approval` und es laeuft keine Aufnahme.
// Gegenstueck mit der echten Engine: `cargo test --lib workflows::recording`.

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

const NOW = Date.parse("2026-09-29T08:59:00+02:00");
const START = Date.parse("2026-09-29T09:00:00+02:00");

const workflowPrompt = (over: Record<string, unknown> = {}) => ({
  prompt_id: "p7",
  kind: "workflow_recording",
  event: calEvent({ starts_at: START, ends_at: START + 3_600_000 }),
  attendee_count: 3,
  app_label: null,
  workflow: {
    name: "Kundentermin protokollieren",
    title: "Jour fixe Vertrieb",
  },
  ...over,
});

const setup = async (
  page: Page,
  prompt: unknown = workflowPrompt(),
  extra?: () => void,
) => {
  await page.clock.setFixedTime(NOW);
  await installTauriMock(page, "meeting_prompt");
  await page.addInitScript((p) => {
    (window as any).__prompt = p;
  }, prompt);
  if (extra) await page.addInitScript(extra);
  await page.setViewportSize({ width: 380, height: 320 });
  await page.goto("/src/meeting-prompt/index.html");
};

const run = (page: Page) =>
  page.evaluate(
    () =>
      (window as any).__workflowRun as { state: string; recordings: number },
  );

const startCalls = async (page: Page) =>
  (await calls(page, "meetings_start")).length +
  (await calls(page, "meetings_start_from_event")).length;

const toConsent = async (page: Page) => {
  await page.getByTestId("prompt-start").click();
  await expect(page.getByTestId("meeting-prompt")).toHaveAttribute(
    "data-step",
    "consent",
  );
};

test("Die Bitte nennt Ablauf und Termin, und es geschieht nichts von selbst", async ({
  page,
}) => {
  await setup(page);
  await expect(page.getByTestId("prompt-workflow-title")).toHaveText(
    "Ein Ablauf möchte aufnehmen",
  );
  await expect(page.getByTestId("prompt-title")).toHaveText(
    "Jour fixe Vertrieb",
  );
  await expect(page.getByTestId("prompt-workflow-intro")).toHaveText(
    "Der Ablauf „Kundentermin protokollieren“ möchte jetzt eine Aufnahme starten. Es beginnt nichts, bevor Sie zustimmen.",
  );
  await expect(page.getByTestId("prompt-start")).toHaveText("Aufnahme starten");
  await expect(page.getByTestId("prompt-decline")).toHaveText(
    "Nicht aufnehmen",
  );
  // „Später“ gibt es hier nicht: die Bitte wird bejaht oder verneint.
  await expect(page.getByTestId("prompt-later")).toHaveCount(0);
  await expect
    .poll(async () => (await calls(page, "meeting_prompt_ready")).length)
    .toBeGreaterThan(0);
  // Ohne Entscheidung: Lauf wartet, keine Aufnahme, kein Aufruf.
  expect(await run(page)).toEqual({
    state: "awaiting_approval",
    recordings: 0,
  });
  expect(await calls(page, "meeting_prompt_workflow_decide")).toHaveLength(0);
  expect(await startCalls(page)).toBe(0);
});

test("Ohne Häkchen keine Entscheidung: auch ein erzwungener Klick ruft nichts", async ({
  page,
}) => {
  await setup(page);
  await toConsent(page);
  const confirm = page.getByTestId("prompt-confirm");
  await expect(confirm).toBeDisabled();
  await confirm.evaluate((el) => (el as HTMLButtonElement).click());
  expect(await calls(page, "meeting_prompt_workflow_decide")).toHaveLength(0);
  expect(await startCalls(page)).toBe(0);
  expect(await run(page)).toEqual({
    state: "awaiting_approval",
    recordings: 0,
  });
  // Dieselbe Einwilligungsabfrage wie bei jeder Aufnahme.
  await expect(page.getByText("Einwilligung erforderlich")).toBeVisible();
  await expect(page.getByText("(§ 201 StGB)")).toBeVisible();
});

test("Mit Häkchen und Klick geht der Lauf weiter: eine Entscheidung, eine Aufnahme, nie über den Direktstart", async ({
  page,
}) => {
  await setup(page);
  await toConsent(page);
  await page.getByTestId("prompt-consent").check();
  const confirm = page.getByTestId("prompt-confirm");
  await expect(confirm).toBeEnabled();
  await expect(confirm).toHaveText("Aufnahme starten");
  await confirm.click();
  await expect
    .poll(
      async () => (await calls(page, "meeting_prompt_workflow_decide")).length,
    )
    .toBe(1);
  expect((await calls(page, "meeting_prompt_workflow_decide"))[0].args).toEqual(
    {
      promptId: "p7",
      approve: true,
    },
  );
  // Der Ablauf ist weitergelaufen und hat genau eine Aufnahme gestartet ...
  expect(await run(page)).toEqual({ state: "done", recordings: 1 });
  // ... aber nicht das Fenster: es ruft den Start nie selbst.
  expect(await startCalls(page)).toBe(0);
  await expect(page.getByTestId("meeting-prompt")).toHaveCount(0);
});

test("Nicht aufnehmen verneint sofort, ohne Häkchen, und der Lauf endet ohne Aufnahme", async ({
  page,
}) => {
  await setup(page);
  await page.getByTestId("prompt-decline").click();
  await expect
    .poll(
      async () => (await calls(page, "meeting_prompt_workflow_decide")).length,
    )
    .toBe(1);
  expect((await calls(page, "meeting_prompt_workflow_decide"))[0].args).toEqual(
    {
      promptId: "p7",
      approve: false,
    },
  );
  expect(await run(page)).toEqual({ state: "failed", recordings: 0 });
  expect(await startCalls(page)).toBe(0);
});

test("Zurück verwirft das Häkchen: danach braucht es die Einwilligung erneut", async ({
  page,
}) => {
  await setup(page);
  await toConsent(page);
  await page.getByTestId("prompt-consent").check();
  await page.getByTestId("prompt-back").click();
  await toConsent(page);
  await expect(page.getByTestId("prompt-consent")).not.toBeChecked();
  await expect(page.getByTestId("prompt-confirm")).toBeDisabled();
  expect(await calls(page, "meeting_prompt_workflow_decide")).toHaveLength(0);
});

test("Eine nicht mehr offene Bitte zeigt die Meldung und schließt das Fenster", async ({
  page,
}) => {
  await setup(page, workflowPrompt(), () => {
    (window as any).__decideError = "consent_not_pending";
  });
  await toConsent(page);
  await page.getByTestId("prompt-consent").check();
  await page.getByTestId("prompt-confirm").click();
  await expect(page.getByTestId("prompt-error")).toContainText(
    "nicht mehr offen",
  );
  expect(await run(page)).toEqual({
    state: "awaiting_approval",
    recordings: 0,
  });
  await expect(page.getByTestId("meeting-prompt")).toHaveCount(0, {
    timeout: 6_000,
  });
});

test("Ein Fehler bei „Nicht aufnehmen“ bleibt im Fenster sichtbar", async ({
  page,
}) => {
  await setup(page, workflowPrompt(), () => {
    (window as any).__decideError = "store_failed: gesperrt";
  });
  await page.getByTestId("prompt-decline").click();
  await expect(page.getByTestId("prompt-error")).toHaveText(
    "Die Änderung konnte nicht gespeichert werden.",
  );
  await expect(page.getByTestId("prompt-decline")).toBeEnabled();
});

test("Ein Ablauf ohne Termin zeigt den Titel aus dem Ablauf und keinen Beginn", async ({
  page,
}) => {
  await setup(
    page,
    workflowPrompt({
      event: null,
      attendee_count: 0,
      workflow: { name: "Tagesbericht", title: null },
    }),
  );
  await expect(page.getByTestId("prompt-title")).toHaveText("Besprechung");
  await expect(page.getByTestId("prompt-workflow-intro")).toContainText(
    "Tagesbericht",
  );
  await expect(page.getByTestId("prompt-brief")).toHaveCount(0);
});

test("Der Hinweis zu einem gewöhnlichen Termin bleibt unverändert", async ({
  page,
}) => {
  await setup(
    page,
    workflowPrompt({ kind: "reminder", workflow: null, prompt_id: "p1" }),
  );
  await expect(page.getByTestId("prompt-workflow-title")).toHaveCount(0);
  await expect(page.getByTestId("prompt-decline")).toHaveCount(0);
  await expect(page.getByTestId("prompt-later")).toBeVisible();
  await toConsent(page);
  await page.getByTestId("prompt-consent").check();
  await page.getByTestId("prompt-confirm").click();
  await expect
    .poll(async () => (await calls(page, "meetings_start_from_event")).length)
    .toBe(1);
  expect(await calls(page, "meeting_prompt_workflow_decide")).toHaveLength(0);
});

// A8: Dieselbe Bitte, wenn ein externer Agent (Claude Code, Codex ...) ueber die
// Agentenbruecke um eine Aufnahme bittet. Gleiche Schritte, gleiches Haekchen; nur der
// Wortlaut nennt den Agenten. Gegenstueck mit der echten Bruecke:
// `cargo test --lib agent_bridge::tools` und `workflows::consent`.
const agentPrompt = () =>
  workflowPrompt({
    prompt_id: "p9",
    event: null,
    attendee_count: 0,
    workflow: { name: "Claude Code", title: "Jour fixe Vertrieb", agent: true },
  });

test("A8: Die Bitte eines Agenten nennt den Agenten, und es geschieht nichts von selbst", async ({
  page,
}) => {
  await setup(page, agentPrompt());
  await expect(page.getByTestId("prompt-workflow-title")).toHaveText(
    "Ein Agent möchte aufnehmen",
  );
  await expect(page.getByTestId("prompt-workflow-intro")).toHaveText(
    "Der Agent „Claude Code“ möchte jetzt eine Aufnahme starten. Es beginnt nichts, bevor Sie zustimmen.",
  );
  await expect(page.getByTestId("prompt-title")).toHaveText(
    "Jour fixe Vertrieb",
  );
  await expect(page.getByTestId("prompt-start")).toHaveText("Aufnahme starten");
  await expect(page.getByTestId("prompt-decline")).toHaveText(
    "Nicht aufnehmen",
  );
  await expect(page.getByTestId("prompt-later")).toHaveCount(0);
  expect(await calls(page, "meeting_prompt_workflow_decide")).toHaveLength(0);
  expect(await startCalls(page)).toBe(0);
});

test("A8: Auch für einen Agenten gilt: ohne Häkchen keine Entscheidung, mit Häkchen genau eine, nie der Direktstart", async ({
  page,
}) => {
  await setup(page, agentPrompt());
  await toConsent(page);
  const confirm = page.getByTestId("prompt-confirm");
  await expect(confirm).toBeDisabled();
  await confirm.evaluate((el) => (el as HTMLButtonElement).click());
  expect(await calls(page, "meeting_prompt_workflow_decide")).toHaveLength(0);
  await expect(page.getByText("Einwilligung erforderlich")).toBeVisible();
  await expect(page.getByText("(§ 201 StGB)")).toBeVisible();
  await page.getByTestId("prompt-consent").check();
  await confirm.click();
  await expect
    .poll(
      async () => (await calls(page, "meeting_prompt_workflow_decide")).length,
    )
    .toBe(1);
  expect((await calls(page, "meeting_prompt_workflow_decide"))[0].args).toEqual(
    { promptId: "p9", approve: true },
  );
  expect(await startCalls(page)).toBe(0);
});

test("A8: „Nicht aufnehmen“ verneint die Bitte eines Agenten sofort", async ({
  page,
}) => {
  await setup(page, agentPrompt());
  await page.getByTestId("prompt-decline").click();
  await expect
    .poll(
      async () => (await calls(page, "meeting_prompt_workflow_decide")).length,
    )
    .toBe(1);
  expect((await calls(page, "meeting_prompt_workflow_decide"))[0].args).toEqual(
    { promptId: "p9", approve: false },
  );
  expect(await run(page)).toEqual({ state: "failed", recordings: 0 });
  expect(await startCalls(page)).toBe(0);
});
