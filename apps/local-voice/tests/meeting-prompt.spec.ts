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
    targetMeetingId: null,
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
  // ToggleSwitch: unsichtbares Eingabefeld unter dem sichtbaren Schalter (force).
  await system.check({ force: true });
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

// P4: Planung im Hinweis und das Startfeld mit personenbezogener Einwilligung.
const promptPlan = (over: Record<string, unknown> = {}) => ({
    id: "pl1",
    title: "Abstimmung Pilotstart",
    goal_type: "entscheiden",
    goal_text: "Wir entscheiden ueber den Pilotstart.",
    expected_result: "Entscheidung mit Termin",
    duration_min: 25,
    classification: "intern",
    agenda: [
        { minutes: 5, topic: "Stand", owner: "A. Beispiel" },
        { minutes: 15, topic: "Optionen", owner: null },
    ],
    roles: { moderation: "A. Beispiel", ergebnissicherung: null, entscheidung: null },
    materials: [],
    hints: {},
    transcription_planned: true,
    minutes_template_id: null,
    status: "verknuepft",
    meeting_id: null,
    created_at: NOW,
    updated_at: NOW,
    ...over,
});

const promptConsent = (person: string, state = "zugestimmt") => ({
    plan_id: "pl1",
    person,
    state,
    channel: "muendlich_bei_start",
    recorded_at: NOW,
});

const plannedPayload = (
    planOver: Record<string, unknown> = {},
    einwilligungen = [promptConsent("anna@example.org")],
    eventOver = {},
) => payload({ plaene: [{ plan: promptPlan(planOver), einwilligungen }] }, eventOver);

test("Planung im Hinweis zeigt Ziel, Agenda, Rollen und jeden Einwilligungsstand", async ({
    page,
}) => {
    const attendees = [
        ...calEvent().attendees,
        { email: null, name: "Gast", organizer: false, is_self: false, partstat: null },
    ];
    await setup(page, plannedPayload({}, [
        promptConsent("anna@example.org"),
        promptConsent("clara@example.org", "abgelehnt"),
    ], { attendees }));
    await expect(page.getByTestId("prompt-plan-ziel"))
        .toContainText("Wir entscheiden ueber den Pilotstart.");
    const punkte = page.getByTestId("prompt-plan-punkt");
    await expect(punkte).toHaveCount(2);
    await expect(punkte.nth(0)).toContainText(/5\s*min/i);
    await expect(punkte.nth(0)).toContainText("Stand");
    await expect(punkte.nth(1)).toContainText(/15\s*min/i);
    await expect(punkte.nth(1)).toContainText("Optionen");
    await expect(page.getByTestId("prompt-plan-rollen")).toContainText("A. Beispiel");
    const personen = page.getByTestId("prompt-plan-person");
    await expect(personen).toHaveCount(4);
    for (const [name, stand] of [
        ["Anna Berg", "zugestimmt"],
        ["Bernd Alt", "offen"],
        ["clara@example.org", "abgelehnt"],
        ["Gast", "nicht_identifiziert"],
    ]) {
        await expect(personen.filter({ hasText: name })).toHaveAttribute("data-stand", stand);
    }
    const stand = page.getByTestId("prompt-plan-stand");
    await expect(stand).toBeVisible();
    // Jede der vier Gruppen enthaelt genau eine Person.
    await expect(stand).toContainText(/1.*1.*1.*1/);
    await expect(stand).not.toContainText("meetings.prompt.plan.stand");
});

test("Ohne Planungsfeld bleibt der bisherige Einwilligungsschritt erhalten", async ({ page }) => {
    await setup(page, payload());
    await expect(page.getByTestId("prompt-start")).toBeVisible();
    await expect(page.getByTestId("prompt-plan")).toHaveCount(0);
    await toConsent(page);
    await expect(page.getByTestId("prompt-consent")).toBeVisible();
});

test("Ohne geplante Transkription fehlt die Personenliste im Planungshinweis", async ({ page }) => {
    await setup(page, plannedPayload({ transcription_planned: false }));
    await expect(page.getByTestId("prompt-plan-einwilligungen")).toBeVisible();
    await expect(page.getByTestId("prompt-plan-person")).toHaveCount(0);
    await expect(page.getByTestId("prompt-plan-stand")).toHaveCount(0);
});

test("Planung startet nach muendlichen Zustimmungen ueber das Startfeld", async ({ page }) => {
    await setup(page, plannedPayload(), () => {
        (window as any).__planConsents = [{
            plan_id: "pl1", person: "anna@example.org", state: "zugestimmt",
            channel: "muendlich_bei_start", recorded_at: Date.parse("2026-09-29T08:59:00+02:00"),
        }];
    });
    await page.getByTestId("prompt-start").click();
    await expect(page.getByTestId("meeting-prompt")).toHaveAttribute("data-step", "planstart");
    await expect(page.getByTestId("prompt-consent")).toHaveCount(0);
    const personen = page.getByTestId("planung-start-person");
    await expect(personen).toHaveCount(3);
    for (const name of ["Bernd Alt", "clara@example.org"]) {
        const zustimmen = personen.filter({ hasText: name })
            .getByTestId("planung-start-zustimmen");
        await zustimmen.click();
        await expect(zustimmen).toHaveCount(0);
    }
    const gesetzt = await calls(page, "besprechungsplanung_einwilligung_setzen");
    expect(gesetzt.map((c) => c.args.einwilligung)).toEqual([
        promptConsent("bernd@example.org"), promptConsent("clara@example.org"),
    ]);
    await page.getByTestId("planung-start-los").click();
    await expect.poll(async () => (await calls(page, "meetings_start_from_event")).length).toBe(1);
    expect((await calls(page, "besprechungsplanung_start_modus"))[0].args).toEqual({
        id: "pl1",
        anwesende: ["anna@example.org", "bernd@example.org", "clara@example.org"],
    });
    expect((await calls(page, "meetings_start_from_event"))[0].args).toEqual({
        eventKey: calEvent().key,
        appKey: null,
        consentConfirmed: true,
        captureSystem: true,
        title: null,
        linkMode: "prompt",
        targetMeetingId: null,
    });
    const reihenfolge = await page.evaluate(() =>
        (window as any).__calls.map((c: { cmd: string }) => c.cmd) as string[],
    );
    expect(reihenfolge.indexOf("besprechungsplanung_start_modus"))
        .toBeLessThan(reihenfolge.indexOf("meetings_start_from_event"));
});

test("Offene Zustimmungen erlauben nur Notizen und schliessen danach den Hinweis", async ({
    page,
}) => {
    await setup(page, plannedPayload());
    await page.getByTestId("prompt-start").click();
    await page.getByTestId("planung-start-los").click();
    const gruende = page.getByTestId("planung-start-gruende");
    await expect(gruende).toBeVisible();
    for (const email of ["anna@example.org", "bernd@example.org", "clara@example.org"]) {
        await expect(gruende).toContainText(email);
    }
    expect(await calls(page, "meetings_start_from_event")).toHaveLength(0);
    await page.getByTestId("planung-start-notizen").click();
    await expect.poll(async () => (await calls(page, "meeting_prompt_dismiss")).length).toBe(1);
    expect((await calls(page, "besprechungsplanung_notizen_starten"))[0].args).toEqual({
        id: "pl1", title: "Abstimmung Pilotstart",
    });
    expect((await calls(page, "meeting_prompt_dismiss"))[0].args).toEqual({
        promptId: "p1", action: "close",
    });
    const reihenfolge = await page.evaluate(() =>
        (window as any).__calls.map((c: { cmd: string }) => c.cmd) as string[],
    );
    expect(reihenfolge.indexOf("besprechungsplanung_notizen_starten"))
        .toBeLessThan(reihenfolge.indexOf("meeting_prompt_dismiss"));
    await expect(page.getByTestId("meeting-prompt")).toHaveCount(0);
    expect(await calls(page, "meetings_start_from_event")).toHaveLength(0);
});

test("Das Backendtor kann die Aufnahme trotz bestandener Vorpruefung sperren", async ({ page }) => {
    await setup(page, plannedPayload(), () => {
        const w = window as any;
        w.__planStartModus = { modus: "mit_transkription" };
        w.__startError = "nur_notizen: offen";
    });
    await page.getByTestId("prompt-start").click();
    await page.getByTestId("planung-start-los").click();
    const fehler = page.getByTestId("planung-start-fehler");
    await expect(fehler).toBeVisible();
    await expect(fehler).toHaveText(/\S/);
    await expect(fehler).not.toContainText("nur_notizen");
    await expect(fehler).not.toContainText("planung.");
    await expect(fehler).not.toContainText("meetings.");
    await expect(page.getByTestId("meeting-prompt")).toHaveAttribute("data-step", "planstart");
    await expect(page.getByTestId("planung-start-los")).toBeEnabled();
    expect(await calls(page, "meetings_start_from_event")).toHaveLength(1);
    expect(await calls(page, "meeting_prompt_dismiss")).toHaveLength(0);
});

test("Ein langer Plan scrollt innerhalb der gemeldeten Fensterhoehe", async ({ page }) => {
    await setup(page, plannedPayload({
        agenda: Array.from({ length: 14 }, (_, i) => ({
            minutes: 5, topic: `Ausfuehrlicher Agendapunkt ${i + 1}`, owner: "A. Beispiel",
        })),
    }));
    await page.setViewportSize({ width: 380, height: 560 });
    const root = page.getByTestId("meeting-prompt");
    await expect(page.getByTestId("prompt-plan-punkt")).toHaveCount(14);
    const masse = await root.evaluate((el) => ({
        height: el.getBoundingClientRect().height,
        clientHeight: el.clientHeight,
        scrollHeight: el.scrollHeight,
        overflowY: getComputedStyle(el).overflowY,
    }));
    expect(masse.height).toBeLessThanOrEqual(560);
    expect(masse.scrollHeight).toBeGreaterThan(masse.clientHeight);
    expect(masse.overflowY).toBe("auto");
    await expect.poll(async () => (await calls(page, "meeting_prompt_ready")).length)
        .toBeGreaterThan(0);
    for (const call of await calls(page, "meeting_prompt_ready")) {
        const height = (call.args as { height: number }).height;
        expect(height).toBeGreaterThan(0);
        expect(height).toBeLessThanOrEqual(560);
    }
});

test("Eine Workflowbitte zeigt auch bei mitgelieferter Planung keinen Planungshinweis", async ({
    page,
}) => {
    await setup(page, {
        ...plannedPayload(),
        workflow: { name: "Pilotablauf", title: "Aufnahme freigeben" },
    });
    await expect(page.getByTestId("prompt-workflow-title")).toBeVisible();
    await expect(page.getByTestId("prompt-plan")).toHaveCount(0);
    await toConsent(page);
    await expect(page.getByTestId("planung-start-person")).toHaveCount(0);
});
