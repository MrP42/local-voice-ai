import { test, expect, type Page } from "@playwright/test";
import { calEvent, calls, installTauriMock } from "./calendarMock";

// M5-P5b: Hinweisfenster `meeting_prompt` gegen die Tauri-Attrappe. Das Fenster
// holt seinen Inhalt ueber `meeting_prompt_current`, meldet seine Hoehe mit
// `meeting_prompt_ready` und startet die Aufnahme erst nach dem Einwilligungsschritt.

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

/** 29.09.2026 08:59 Ortszeit: der Termin beginnt in 1 Minute. */
const NOW = Date.parse("2026-09-29T08:59:00+02:00");
const START = Date.parse("2026-09-29T09:00:00+02:00");

const payload = (over: Record<string, unknown> = {}, eventOver = {}) => ({
  prompt_id: "p1",
  kind: "reminder",
  event: calEvent({
    starts_at: START,
    ends_at: START + 3_600_000,
    ...eventOver,
  }),
  attendee_count: 3,
  app_label: null,
  ...over,
});

const setup = async (
  page: Page,
  prompt: unknown = payload(),
  extra?: () => void,
) => {
  await page.clock.setFixedTime(NOW);
  await installTauriMock(page, "meeting_prompt");
  await page.addInitScript((p) => {
    (window as any).__prompt = p;
  }, prompt);
  if (extra) await page.addInitScript(extra);
  await page.setViewportSize({ width: 380, height: 300 });
  await page.goto("/src/meeting-prompt/index.html");
};

const toConsent = async (page: Page) => {
  await page.getByTestId("prompt-start").click();
  await expect(page.getByTestId("meeting-prompt")).toHaveAttribute(
    "data-step",
    "consent",
  );
};

test("Hinweis nennt Termin, Beginn und Teilnehmende und meldet seine Höhe", async ({
  page,
}) => {
  await setup(page);
  await expect(page.getByTestId("prompt-title")).toHaveText(
    "Jour fixe Vertrieb",
  );
  await expect(page.getByTestId("prompt-when")).toHaveText(
    "beginnt in 1 min · 3 Teilnehmende",
  );
  await expect
    .poll(async () => (await calls(page, "meeting_prompt_ready")).length)
    .toBeGreaterThan(0);
  const height = (
    (await calls(page, "meeting_prompt_ready"))[0].args as { height: number }
  ).height;
  expect(height).toBeGreaterThan(60);
  expect(height).toBeLessThan(400);
  // Nichts startet von selbst.
  expect(await calls(page, "meetings_start_from_event")).toHaveLength(0);
});

test("Ein laufender Termin heißt „läuft seit“", async ({ page }) => {
  await setup(page, payload({}, { starts_at: NOW - 3 * 60_000 }));
  await expect(page.getByTestId("prompt-when")).toContainText(
    "läuft seit 3 min",
  );
});

test("Aufnahme starten führt zuerst zur Einwilligung, ohne Häkchen ruft nichts den Start", async ({
  page,
}) => {
  await setup(page);
  await toConsent(page);
  const confirm = page.getByTestId("prompt-confirm");
  await expect(confirm).toBeDisabled();
  // Auch ein erzwungener Klick startet nichts: der Knopf ist gesperrt.
  await confirm.evaluate((el) => (el as HTMLButtonElement).click());
  expect(await calls(page, "meetings_start_from_event")).toHaveLength(0);
  expect(await calls(page, "meetings_start")).toHaveLength(0);
  await expect(page.getByText("Einwilligung erforderlich")).toBeVisible();
  await expect(page.getByText("(§ 201 StGB)")).toBeVisible();
});

test("Mit Häkchen startet meetings_start_from_event mit consent_confirmed=true und dem Termin", async ({
  page,
}) => {
  await setup(page);
  await toConsent(page);
  await page.getByTestId("prompt-consent").check();
  const confirm = page.getByTestId("prompt-confirm");
  await expect(confirm).toBeEnabled();
  await confirm.click();
  await expect
    .poll(async () => (await calls(page, "meetings_start_from_event")).length)
    .toBe(1);
  expect((await calls(page, "meetings_start_from_event"))[0].args).toEqual({
    eventKey: "ics-1:uid-1:1790589600000",
    appKey: null,
    consentConfirmed: true,
    captureSystem: true,
    title: null,
    linkMode: "prompt",
  });
  // Der Start geht nie an den alten Weg ohne Terminbezug.
  expect(await calls(page, "meetings_start")).toHaveLength(0);
});

test("Systemton: die Vorgabe kommt aus den Einstellungen und die Wahl wird gemerkt", async ({
  page,
}) => {
  await setup(page, payload(), () => {
    (window as any).__settings.meeting_capture_system = false;
  });
  await toConsent(page);
  const system = page.getByTestId("prompt-capture-system");
  await expect(system).not.toBeChecked();
  await system.check();
  await expect
    .poll(
      async () =>
        (await calls(page, "change_meeting_capture_system_setting")).length,
    )
    .toBe(1);
  expect(
    (await calls(page, "change_meeting_capture_system_setting"))[0].args,
  ).toEqual({ enabled: true });
  await page.getByTestId("prompt-consent").check();
  await page.getByTestId("prompt-confirm").click();
  await expect
    .poll(async () => (await calls(page, "meetings_start_from_event")).length)
    .toBe(1);
  expect(
    ((await calls(page, "meetings_start_from_event"))[0].args as any)
      .captureSystem,
  ).toBe(true);
});

test("Der Hinweistext zum Kopieren steht im Einwilligungsschritt", async ({
  page,
}) => {
  await setup(page);
  await expect(page.getByTestId("prompt-chat-notice")).toHaveCount(0);
  await toConsent(page);
  await expect(page.getByTestId("prompt-chat-notice-text")).toContainText(
    "Ich transkribiere diese Besprechung lokal auf meinem Rechner",
  );
  await expect(page.getByTestId("prompt-chat-notice-copy")).toBeVisible();
});

test("Zurück verwirft das Häkchen und meldet die Höhe neu", async ({
  page,
}) => {
  await setup(page);
  await toConsent(page);
  await page.getByTestId("prompt-consent").check();
  await page.getByTestId("prompt-back").click();
  await expect(page.getByTestId("meeting-prompt")).toHaveAttribute(
    "data-step",
    "offer",
  );
  await page.getByTestId("prompt-start").click();
  await expect(page.getByTestId("prompt-consent")).not.toBeChecked();
  expect(
    (await calls(page, "meeting_prompt_ready")).length,
  ).toBeGreaterThanOrEqual(3);
});

test("Beitreten gibt es nur mit Beitritts-Adresse", async ({ page }) => {
  await setup(page);
  await expect(page.getByTestId("prompt-join")).toHaveCount(0);
});

test("Mit Beitritts-Adresse öffnet [Beitreten] sie über das Backend", async ({
  page,
}) => {
  await setup(
    page,
    payload({}, { join_url: "https://teams.microsoft.com/l/meetup-join/x" }),
  );
  await page.getByTestId("prompt-join").click();
  await expect
    .poll(async () => (await calls(page, "calendar_open_join_url")).length)
    .toBe(1);
  expect((await calls(page, "calendar_open_join_url"))[0].args).toEqual({
    eventKey: "ics-1:uid-1:1790589600000",
  });
});

test("Später meldet das Verwerfen des Hinweises", async ({ page }) => {
  await setup(page);
  await page.getByTestId("prompt-later").click();
  await expect
    .poll(async () => (await calls(page, "meeting_prompt_dismiss")).length)
    .toBe(1);
  expect((await calls(page, "meeting_prompt_dismiss"))[0].args).toEqual({
    promptId: "p1",
    action: "later",
  });
  expect(await calls(page, "meetings_start_from_event")).toHaveLength(0);
});

test("Ein Startfehler bleibt im Fenster sichtbar", async ({ page }) => {
  await setup(page, payload(), () => {
    (window as any).__startError = "already_recording";
  });
  await toConsent(page);
  await page.getByTestId("prompt-consent").check();
  await page.getByTestId("prompt-confirm").click();
  await expect(page.getByTestId("prompt-error")).toHaveText(
    "Es läuft bereits eine Aufnahme.",
  );
  await expect(page.getByTestId("prompt-confirm")).toBeEnabled();
});

test("Ein neuer Hinweis per Ereignis ersetzt den alten und beginnt wieder beim Angebot", async ({
  page,
}) => {
  await setup(page);
  await toConsent(page);
  await page.evaluate(
    (next) => {
      const w = window as any;
      w.__prompt = next;
      w.__emit("meeting-prompt-event", { kind: "show", prompt_id: "p2" });
    },
    payload(
      { prompt_id: "p2", attendee_count: 0 },
      { title: "Zweiter Termin" },
    ),
  );
  await expect(page.getByTestId("prompt-title")).toHaveText("Zweiter Termin");
  await expect(page.getByTestId("meeting-prompt")).toHaveAttribute(
    "data-step",
    "offer",
  );
  // Ohne Teilnehmende steht nur der Beginn da.
  await expect(page.getByTestId("prompt-when")).toHaveText("beginnt in 1 min");
  await page.evaluate(() =>
    (window as any).__emit("meeting-prompt-event", { kind: "close" }),
  );
  await expect(page.getByTestId("meeting-prompt")).toHaveCount(0);
});

// M5-P5e: "Vorbereiten" im Hinweisfenster.
test("Vorbereiten im Hinweisfenster: ruft people_brief_open mit dem Termin", async ({
  page,
}) => {
  await setup(page, payload(), () => {
    (window as any).__brief = {
      event_key: "ics-1:uid-1:1790589600000",
      event_title: "Jour fixe Vertrieb",
      names: ["Anna Berg"],
      shared_meetings: 2,
      filter: { meeting_ids: ["m2", "m1"], event_uid: "uid-1" },
      recipe_id: "builtin:vorbereitung-termin",
      recipe_var: "teilnehmende",
      recipe_value: "Anna Berg",
      thread_id: null,
    };
  });
  const button = page.getByTestId("prompt-brief");
  await expect(button).toBeEnabled();
  await expect(button).toHaveText("Vorbereiten");
  await button.click();
  await expect
    .poll(async () => (await calls(page, "people_brief_open")).length)
    .toBe(1);
  expect((await calls(page, "people_brief_open"))[0].args).toEqual({
    eventKey: "ics-1:uid-1:1790589600000",
  });
  // Vorbereiten startet keine Aufnahme.
  expect(await calls(page, "meetings_start_from_event")).toHaveLength(0);
});

test("Vorbereiten im Hinweisfenster: ohne gemeinsame Besprechungen deaktiviert", async ({
  page,
}) => {
  await setup(page, payload(), () => {
    (window as any).__brief = {
      event_key: "ics-1:uid-1:1790589600000",
      event_title: "Jour fixe Vertrieb",
      names: ["Neu Person"],
      shared_meetings: 0,
      filter: { meeting_ids: [], event_uid: "uid-1" },
      recipe_id: "builtin:vorbereitung-termin",
      recipe_var: "teilnehmende",
      recipe_value: "Neu Person",
      thread_id: null,
    };
  });
  const button = page.getByTestId("prompt-brief");
  await expect(button).toBeDisabled();
  await expect(button.locator("xpath=..")).toHaveAttribute(
    "title",
    "Keine früheren Besprechungen mit diesen Teilnehmenden.",
  );
  expect(await calls(page, "people_brief_open")).toHaveLength(0);
});

test("Vorbereiten fehlt beim Hinweis auf eine erkannte Anwendung ohne Termin", async ({
  page,
}) => {
  await setup(
    page,
    payload({
      kind: "detected",
      event: null,
      attendee_count: 0,
      app_label: "Microsoft Teams",
    }),
  );
  await expect(page.getByTestId("prompt-start")).toBeVisible();
  await expect(page.getByTestId("prompt-brief")).toHaveCount(0);
});
