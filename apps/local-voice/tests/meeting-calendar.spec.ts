import { test, expect, type Page } from "@playwright/test";
import { calEvent, calls, installTauriMock } from "./calendarMock";
import {
  distinctAttendees,
  endOfLocalDay,
  hoursUntilEndOfDay,
  todaysEvents,
} from "../src/lib/meetingCalendar";

// M5-P5b: Kalender in den Einstellungen (Gruppe Besprechungen) und auf der
// Aufnahmeseite (Titelvorschlag, "Naechste Termine (heute)") gegen die
// Tauri-Attrappe.

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

/** 29.09.2026 09:00 Ortszeit. */
const NOW = Date.parse("2026-09-29T09:00:00+02:00");
const at = (hhmm: string) => Date.parse(`2026-09-29T${hhmm}:00+02:00`);
const URL_SECRET =
  "https://outlook.office365.com/owa/calendar/abc/reid/calendar.ics";

const source = (over: Record<string, unknown> = {}) => ({
  id: "ics-1",
  kind: "ics",
  label: "Outlook Arbeit",
  account_hint: "outlook.office365.com",
  enabled: true,
  has_attendee_data: true,
  last_sync_at: NOW,
  last_ok_at: NOW,
  last_error: null,
  event_count: 23,
  ...over,
});

const setup = async (page: Page, init?: () => void) => {
  await page.clock.setFixedTime(NOW);
  await installTauriMock(page, "main");
  if (init) await page.addInitScript(init);
  await page.setViewportSize({ width: 1280, height: 1000 });
  await page.goto("/");
};

const openSettings = async (page: Page) => {
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Einstellungen", exact: true })
    .click();
  await expect(page.getByTestId("calendar-settings")).toBeVisible();
};

const openRecordings = async (page: Page) => {
  await page.getByRole("button", { name: "Aufnahmen", exact: true }).click();
};

// Titel, Projekt und Vorlage stehen seit M4 im Startdialog.
const openStart = async (page: Page) => {
  await page.getByRole("button", { name: "Aufnahme starten" }).first().click();
  await expect(page.getByTestId("start-dialog")).toBeVisible();
};

const confirmStart = async (page: Page) => {
  await page
    .getByRole("button", { name: "Alle Beteiligten haben zugestimmt" })
    .click();
};

const consentAndStart = async (page: Page) => {
  await openStart(page);
  await confirmStart(page);
};

// ---------------------------------------------------------------------------
// Reine Logik
// ---------------------------------------------------------------------------

test.describe("Kalender Logik", () => {
  const ev = (over = {}) => calEvent(over) as any;

  test("Teilnehmende zählen je Adresse einmal, Organisator eingerechnet", () => {
    expect(distinctAttendees(ev())).toBe(3);
    expect(
      distinctAttendees({
        attendees: [
          { email: "A@x.de", name: null },
          { email: " a@x.de ", name: "Anna" },
          { email: null, name: "Bernd" },
          { email: null, name: "bernd" },
          { email: null, name: null },
        ],
      } as any),
    ).toBe(2);
    expect(distinctAttendees({ attendees: [] })).toBe(0);
  });

  test("Ende des Tages und Stunden bis dahin", () => {
    // 29.09.2026 09:00 Ortszeit -> 15 h bis Mitternacht.
    expect(endOfLocalDay(NOW)).toBe(Date.parse("2026-09-30T00:00:00+02:00"));
    expect(hoursUntilEndOfDay(NOW)).toBe(15);
    expect(hoursUntilEndOfDay(at("23:59"))).toBe(1);
  });

  test("Heutige Termine: ohne Ganztag, Abgesagte, Beendete und Morgen; nach Beginn sortiert", () => {
    const events = [
      ev({ key: "spaet", starts_at: at("14:00"), ends_at: at("15:00") }),
      ev({ key: "frueh", starts_at: at("09:10"), ends_at: at("10:00") }),
      ev({
        key: "tag",
        starts_at: at("00:00"),
        ends_at: at("23:59"),
        all_day: true,
      }),
      ev({
        key: "weg",
        starts_at: at("11:00"),
        ends_at: at("12:00"),
        cancelled: true,
      }),
      ev({ key: "vorbei", starts_at: at("07:00"), ends_at: at("08:00") }),
      ev({ key: "laeuft", starts_at: at("08:30"), ends_at: at("09:30") }),
      ev({
        key: "morgen",
        starts_at: Date.parse("2026-09-30T10:00:00+02:00"),
        ends_at: Date.parse("2026-09-30T11:00:00+02:00"),
      }),
    ];
    expect(todaysEvents(events, NOW).map((e) => e.key)).toEqual([
      "laeuft",
      "frueh",
      "spaet",
    ]);
  });
});

// ---------------------------------------------------------------------------
// Einstellungen
// ---------------------------------------------------------------------------

test.describe("Kalender in den Einstellungen", () => {
  test("Verbinden: der Dialog ruft calendar_source_add_ics, die Adresse bleibt maskiert und verschwindet danach", async ({
    page,
  }) => {
    await setup(page);
    await openSettings(page);
    await expect(page.getByTestId("calendar-empty")).toBeVisible();

    await page.getByTestId("calendar-connect").click();
    const dialog = page.getByRole("dialog", { name: "Kalender verbinden" });
    await expect(dialog).toBeVisible();
    // Anleitung fuer Outlook und Google steht im Dialog.
    await dialog.getByText("Wo finde ich die Adresse?").click();
    await expect(dialog.getByText("Kalender veröffentlichen")).toBeVisible();
    await expect(
      dialog.getByText("Privatadresse im iCal-Format"),
    ).toBeVisible();

    const url = page.getByTestId("calendar-url");
    await expect(url).toHaveAttribute("type", "password");
    await page.getByTestId("calendar-name").fill("Outlook Arbeit");
    await url.fill(URL_SECRET);
    await page.getByTestId("calendar-url-toggle").click();
    await expect(url).toHaveAttribute("type", "text");
    await page.getByTestId("calendar-url-toggle").click();
    await expect(url).toHaveAttribute("type", "password");

    await page.getByTestId("calendar-connect-submit").click();
    await expect
      .poll(async () => (await calls(page, "calendar_source_add_ics")).length)
      .toBe(1);
    expect((await calls(page, "calendar_source_add_ics"))[0].args).toEqual({
      label: "Outlook Arbeit",
      url: URL_SECRET,
    });

    await expect(dialog).toBeHidden();
    const row = page.getByTestId("calendar-source");
    await expect(row).toContainText("Outlook Arbeit");
    await expect(page.getByTestId("calendar-source-hint")).toHaveText(
      "· outlook.office365.com/…",
    );
    await expect(page.getByTestId("calendar-source-status")).toContainText(
      "23 Termine",
    );
    // Die Adresse ist nirgends mehr zu sehen.
    await expect(page.locator("body")).not.toContainText("reid/calendar.ics");
  });

  test("Ohne Adresse ist Verbinden gesperrt", async ({ page }) => {
    await setup(page);
    await openSettings(page);
    await page.getByTestId("calendar-connect").click();
    await expect(page.getByTestId("calendar-connect-submit")).toBeDisabled();
    await page.getByTestId("calendar-url").fill("https://x.example/a.ics");
    await expect(page.getByTestId("calendar-connect-submit")).toBeEnabled();
    expect(await calls(page, "calendar_source_add_ics")).toHaveLength(0);
  });

  test("Probeabruf fehlgeschlagen: der Grund steht im Dialog, es entsteht keine Quelle", async ({
    page,
  }) => {
    await setup(page, () => {
      (window as any).__addError =
        "Die Adresse liefert keinen Kalender (ICS), vermutlich eine Anmelde- oder Fehlerseite.";
    });
    await openSettings(page);
    await page.getByTestId("calendar-connect").click();
    await page.getByTestId("calendar-url").fill(URL_SECRET);
    await page.getByTestId("calendar-connect-submit").click();
    await expect(page.getByTestId("calendar-connect-error")).toContainText(
      "liefert keinen Kalender",
    );
    await expect(
      page.getByRole("dialog", { name: "Kalender verbinden" }),
    ).toBeVisible();
    await page
      .getByRole("dialog", { name: "Kalender verbinden" })
      .getByText("Abbrechen", { exact: true })
      .click();
    await expect(page.getByTestId("calendar-empty")).toBeVisible();
    await expect(page.getByTestId("calendar-source")).toHaveCount(0);
  });

  test("Während des Probeabrufs zeigt der Knopf den Prüfzustand", async ({
    page,
  }) => {
    await setup(page, () => {
      (window as any).__addDelay = 600;
    });
    await openSettings(page);
    await page.getByTestId("calendar-connect").click();
    await page.getByTestId("calendar-url").fill(URL_SECRET);
    const submit = page.getByTestId("calendar-connect-submit");
    await submit.click();
    await expect(submit).toHaveText("Kalender wird geprüft …");
    await expect(submit).toBeDisabled();
    await expect(page.getByTestId("calendar-source")).toHaveCount(1);
  });

  test("Eine Quelle mit Fehler zeigt den Klartext, Jetzt aktualisieren ruft calendar_sync_now", async ({
    page,
  }) => {
    await setup(page, () => {
      (window as any).__sources = [
        {
          id: "ics-1",
          kind: "ics",
          label: "Outlook Arbeit",
          account_hint: "outlook.office365.com",
          enabled: true,
          has_attendee_data: true,
          last_sync_at: 1,
          last_ok_at: null,
          last_error:
            "Zugriff verweigert (HTTP 403): Die Adresse wurde widerrufen oder das Veröffentlichen ist gesperrt.",
          event_count: 0,
        },
      ];
    });
    await openSettings(page);
    await expect(page.getByTestId("calendar-source-error")).toContainText(
      "Zugriff verweigert (HTTP 403)",
    );
    await page.getByTestId("calendar-refresh").click();
    await expect
      .poll(async () => (await calls(page, "calendar_sync_now")).length)
      .toBe(1);
    expect((await calls(page, "calendar_sync_now"))[0].args).toEqual({
      id: null,
    });
    await expect(page.getByTestId("calendar-source-error")).toHaveCount(0);
    await expect(page.getByTestId("calendar-source-status")).toBeVisible();
  });

  test("Entfernen fragt nach und ruft dann calendar_source_remove", async ({
    page,
  }) => {
    await setup(page, () => {
      (window as any).__sources = [
        {
          id: "ics-1",
          kind: "ics",
          label: "Outlook Arbeit",
          account_hint: "outlook.office365.com",
          enabled: true,
          has_attendee_data: true,
          last_sync_at: 1,
          last_ok_at: 1,
          last_error: null,
          event_count: 5,
        },
      ];
    });
    await openSettings(page);
    await page.getByTestId("calendar-remove").click();
    await expect(page.getByTestId("calendar-remove-confirm")).toContainText(
      "Kalender „Outlook Arbeit“ entfernen?",
    );
    expect(await calls(page, "calendar_source_remove")).toHaveLength(0);
    await page.getByTestId("calendar-remove-yes").click();
    await expect
      .poll(async () => (await calls(page, "calendar_source_remove")).length)
      .toBe(1);
    expect((await calls(page, "calendar_source_remove"))[0].args).toEqual({
      id: "ics-1",
    });
    await expect(page.getByTestId("calendar-empty")).toBeVisible();
  });

  test("Erinnerung: die Wahl geht ans Backend, bei „Aus“ ist der Schalter gesperrt", async ({
    page,
  }) => {
    await setup(page);
    await openSettings(page);
    await expect(page.getByText("Erinnerung vor Terminen")).toBeVisible();
    const row = page
      .getByText("Erinnerung vor Terminen", { exact: true })
      .locator("xpath=ancestor::div[.//button[contains(@class,'min-w-')]][1]");
    await row.getByRole("button", { name: "1 Minute vorher" }).click();
    await row
      .getByRole("button", { name: "5 Minuten vorher", exact: true })
      .click();
    await expect
      .poll(
        async () =>
          (await calls(page, "change_meeting_reminder_lead_setting")).length,
      )
      .toBe(1);
    expect(
      (await calls(page, "change_meeting_reminder_lead_setting"))[0].args,
    ).toEqual({ seconds: 300 });

    const all = page
      .getByText("Auch Termine ohne Teilnehmende")
      .locator("xpath=ancestor::div[.//input[@type='checkbox']][1]")
      .locator("input[type=checkbox]");
    await expect(all).not.toBeChecked();
    await all.evaluate((el) => (el as HTMLInputElement).click());
    await expect
      .poll(
        async () =>
          (await calls(page, "change_meeting_reminder_all_events_setting"))
            .length,
      )
      .toBe(1);
    expect(
      (await calls(page, "change_meeting_reminder_all_events_setting"))[0].args,
    ).toEqual({ enabled: true });

    // Bei "Aus" gibt es nichts zu erinnern: der Schalter ist gesperrt.
    await row.getByRole("button", { name: "5 Minuten vorher" }).click();
    await row.getByRole("button", { name: "Aus", exact: true }).click();
    await expect
      .poll(
        async () =>
          (await calls(page, "change_meeting_reminder_lead_setting")).length,
      )
      .toBe(2);
    expect(
      (await calls(page, "change_meeting_reminder_lead_setting"))[1].args,
    ).toEqual({ seconds: 0 });
    await expect(all).toBeDisabled();
  });
});

// ---------------------------------------------------------------------------
// Microsoft-Anmeldung (P5f)
// ---------------------------------------------------------------------------

test.describe("Microsoft-Anmeldung in den Einstellungen", () => {
  const CLIENT_ID = "11111111-2222-3333-4444-555555555555";
  const withClientId = () => {
    (window as any).__settings.calendar_graph_client_id =
      "11111111-2222-3333-4444-555555555555";
  };
  const graphSource = (over: Record<string, unknown> = {}) => ({
    id: "graph-1",
    kind: "graph",
    label: "Outlook / Microsoft 365",
    account_hint: "anna.berg@firma.de",
    enabled: true,
    has_attendee_data: true,
    last_sync_at: 1,
    last_ok_at: 1,
    last_error: null,
    event_count: 12,
    ...over,
  });

  const openDialog = async (page: Page) => {
    await openSettings(page);
    await page.getByTestId("calendar-connect").click();
    const dialog = page.getByRole("dialog", { name: "Kalender verbinden" });
    await expect(dialog).toBeVisible();
    return dialog;
  };

  test("ohne Client-ID ist die Anmeldung gesperrt, der Hinweis erklärt die Registrierung", async ({
    page,
  }) => {
    await setup(page);
    await openDialog(page);
    const signIn = page.getByTestId("calendar-graph-signin");
    await expect(signIn).toBeDisabled();
    const hint = page.getByTestId("calendar-graph-hint");
    await expect(hint).toContainText("Ohne eigene Client-ID");
    await expect(hint).toContainText("http://localhost");
    await expect(hint).toContainText("Calendars.Read");
    await expect(page.getByTestId("calendar-graph-tenant")).toHaveAttribute(
      "placeholder",
      "common",
    );

    // Der Link führt auf die Microsoft-Anleitung (über den Systembrowser).
    await page.getByTestId("calendar-graph-help-link").click();
    await expect
      .poll(async () => (await calls(page, "plugin:opener|open_url")).length)
      .toBe(1);
    expect(
      String((await calls(page, "plugin:opener|open_url"))[0].args.url),
    ).toContain("learn.microsoft.com/entra");

    // Eine Eingabe, die keine Client-ID ist, ändert nichts.
    await page.getByTestId("calendar-graph-client-id").fill("nicht-gueltig");
    await expect(signIn).toBeDisabled();
    await expect(hint).toContainText("keine gültige Client-ID");
    expect(await calls(page, "calendar_graph_sign_in")).toHaveLength(0);
    expect(
      await calls(page, "change_calendar_graph_client_id_setting"),
    ).toHaveLength(0);
  });

  test("mit Client-ID aus den Einstellungen ist die Anmeldung aktiv und ruft calendar_graph_sign_in", async ({
    page,
  }) => {
    await setup(page, withClientId);
    await openDialog(page);
    await expect(page.getByTestId("calendar-graph-client-id")).toHaveValue(
      CLIENT_ID,
    );
    await expect(page.getByTestId("calendar-graph-hint")).toHaveCount(0);
    const signIn = page.getByTestId("calendar-graph-signin");
    await expect(signIn).toBeEnabled();
    await signIn.click();
    await expect
      .poll(async () => (await calls(page, "calendar_graph_sign_in")).length)
      .toBe(1);
    // Unveränderte Einstellungen werden nicht neu geschrieben.
    expect(
      await calls(page, "change_calendar_graph_client_id_setting"),
    ).toHaveLength(0);
    expect(
      await calls(page, "change_calendar_graph_tenant_setting"),
    ).toHaveLength(0);

    await expect(
      page.getByRole("dialog", { name: "Kalender verbinden" }),
    ).toBeHidden();
    await expect(page.getByTestId("calendar-source")).toContainText(
      "Outlook / Microsoft 365",
    );
    // Bei Microsoft steht das Konto, nicht ein Host mit "/…".
    await expect(page.getByTestId("calendar-source-hint")).toHaveText(
      "· anna.berg@firma.de",
    );
    await expect(page.getByTestId("calendar-source-status")).toContainText(
      "12 Termine",
    );
  });

  test("eine eingetragene Client-ID und ein Verzeichnis werden gespeichert, bevor die Anmeldung startet", async ({
    page,
  }) => {
    await setup(page);
    await openDialog(page);
    await page
      .getByTestId("calendar-graph-client-id")
      .fill(CLIENT_ID.toUpperCase());
    await page.getByTestId("calendar-graph-tenant").fill("contoso.com");
    await expect(page.getByTestId("calendar-graph-hint")).toHaveCount(0);
    await page.getByTestId("calendar-graph-signin").click();
    await expect
      .poll(async () => (await calls(page, "calendar_graph_sign_in")).length)
      .toBe(1);
    expect(
      (await calls(page, "change_calendar_graph_client_id_setting"))[0].args,
    ).toEqual({ clientId: CLIENT_ID });
    expect(
      (await calls(page, "change_calendar_graph_tenant_setting"))[0].args,
    ).toEqual({ tenant: "contoso.com" });
    // Reihenfolge: erst speichern, dann anmelden (das Backend liest die Einstellungen).
    const order = await page.evaluate(() =>
      (window as any).__calls
        .map((c: { cmd: string }) => c.cmd)
        .filter(
          (c: string) =>
            c.startsWith("change_calendar_graph") ||
            c === "calendar_graph_sign_in",
        ),
    );
    expect(order).toEqual([
      "change_calendar_graph_client_id_setting",
      "change_calendar_graph_tenant_setting",
      "calendar_graph_sign_in",
    ]);
  });

  test("während der Anmeldung zeigt der Dialog den Wartezustand, Abbrechen ruft calendar_graph_cancel_sign_in ohne Fehlermeldung", async ({
    page,
  }) => {
    await setup(page, () => {
      (window as any).__settings.calendar_graph_client_id =
        "11111111-2222-3333-4444-555555555555";
      (window as any).__graphWait = true;
    });
    const dialog = await openDialog(page);
    const signIn = page.getByTestId("calendar-graph-signin");
    await signIn.click();
    await expect(signIn).toHaveText("Warte auf die Anmeldung im Browser …");
    await expect(signIn).toBeDisabled();
    // Nichts anderes im Dialog ist jetzt bedienbar.
    await expect(page.getByTestId("calendar-connect-submit")).toBeDisabled();
    await expect(page.getByTestId("calendar-graph-client-id")).toBeDisabled();

    await page.getByTestId("calendar-graph-cancel").click();
    await expect
      .poll(
        async () => (await calls(page, "calendar_graph_cancel_sign_in")).length,
      )
      .toBe(1);
    await expect(signIn).toHaveText("Mit Microsoft anmelden");
    await expect(signIn).toBeEnabled();
    await expect(page.getByTestId("calendar-graph-error")).toHaveCount(0);
    await expect(page.getByTestId("calendar-graph-cancel")).toHaveCount(0);
    await expect(dialog).toBeVisible();
    await expect(page.getByTestId("calendar-source")).toHaveCount(0);
  });

  test("scheitert die Anmeldung, steht der Grund im Dialog und es entsteht keine Quelle", async ({
    page,
  }) => {
    await setup(page, () => {
      (window as any).__settings.calendar_graph_client_id =
        "11111111-2222-3333-4444-555555555555";
      (window as any).__graphError =
        "Microsoft hat den Zugriff abgelehnt: AADSTS65001: Die Einwilligung fehlt.";
    });
    await openDialog(page);
    await page.getByTestId("calendar-graph-signin").click();
    await expect(page.getByTestId("calendar-graph-error")).toContainText(
      "Die Einwilligung fehlt.",
    );
    await expect(
      page.getByRole("dialog", { name: "Kalender verbinden" }),
    ).toBeVisible();
    await expect(page.getByTestId("calendar-graph-signin")).toBeEnabled();
    await page
      .getByRole("dialog", { name: "Kalender verbinden" })
      .getByText("Abbrechen", { exact: true })
      .last()
      .click();
    await expect(page.getByTestId("calendar-empty")).toBeVisible();
  });

  test("ein Konto mit Fehler bietet „Erneut anmelden“, Abmelden ruft calendar_graph_sign_out", async ({
    page,
  }) => {
    await setup(page, () => {
      (window as any).__settings.calendar_graph_client_id =
        "11111111-2222-3333-4444-555555555555";
      (window as any).__sources = [
        {
          id: "graph-1",
          kind: "graph",
          label: "Outlook / Microsoft 365",
          account_hint: "anna.berg@firma.de",
          enabled: true,
          has_attendee_data: true,
          last_sync_at: 1,
          last_ok_at: 1,
          last_error: "Anmeldung nötig: Bitte erneut mit Microsoft anmelden.",
          event_count: 12,
        },
      ];
    });
    await openSettings(page);
    await expect(page.getByTestId("calendar-source-error")).toContainText(
      "Anmeldung nötig",
    );
    await page.getByTestId("calendar-graph-reauth").click();
    const dialog = page.getByRole("dialog", { name: "Kalender verbinden" });
    await expect(dialog).toBeVisible();
    await expect(page.getByTestId("calendar-graph-signin")).toBeEnabled();
    await dialog.getByText("Abbrechen", { exact: true }).last().click();
    await expect(dialog).toBeHidden();

    await page.getByTestId("calendar-remove").click();
    await expect(page.getByTestId("calendar-remove-confirm")).toContainText(
      "Konto „anna.berg@firma.de“ abmelden?",
    );
    await page.getByTestId("calendar-remove-yes").click();
    await expect
      .poll(async () => (await calls(page, "calendar_graph_sign_out")).length)
      .toBe(1);
    expect((await calls(page, "calendar_graph_sign_out"))[0].args).toEqual({
      id: "graph-1",
    });
    expect(await calls(page, "calendar_source_remove")).toHaveLength(0);
    await expect(page.getByTestId("calendar-empty")).toBeVisible();
  });

  test("ein Konto ohne Fehler zeigt keinen Knopf „Erneut anmelden“", async ({
    page,
  }) => {
    await setup(page, () => {
      (window as any).__sources = [
        {
          id: "graph-1",
          kind: "graph",
          label: "Outlook / Microsoft 365",
          account_hint: "anna.berg@firma.de",
          enabled: true,
          has_attendee_data: true,
          last_sync_at: 1,
          last_ok_at: 1,
          last_error: null,
          event_count: 3,
        },
      ];
    });
    await openSettings(page);
    await expect(page.getByTestId("calendar-source")).toHaveCount(1);
    await expect(page.getByTestId("calendar-graph-reauth")).toHaveCount(0);
  });
});

// ---------------------------------------------------------------------------
// Aufnahmeseite
// ---------------------------------------------------------------------------

test.describe("Aufnahmeseite mit Kalender", () => {
  const withSuggestion = () => {
    (window as any).__suggest = {
      key: "ics-1:uid-1:1790589600000",
      source_id: "ics-1",
      uid: "uid-1",
      title: "Jour fixe Vertrieb",
      starts_at: 1790589600000,
      ends_at: 1790593200000,
      all_day: false,
      cancelled: false,
      location: null,
      join_url: null,
      description: null,
      attendees: [
        {
          email: "a@x.de",
          name: null,
          organizer: true,
          is_self: false,
          partstat: null,
        },
        {
          email: "b@x.de",
          name: null,
          organizer: false,
          is_self: false,
          partstat: null,
        },
        {
          email: "c@x.de",
          name: null,
          organizer: false,
          is_self: false,
          partstat: null,
        },
      ],
    };
  };

  test("Titelvorschlag: laufender Termin füllt Titel und Chip, der Start geht über den Termin", async ({
    page,
  }) => {
    await setup(page, withSuggestion);
    await openRecordings(page);
    await openStart(page);
    await expect(page.getByPlaceholder("Titel der Besprechung")).toHaveValue(
      "Jour fixe Vertrieb",
    );
    await expect(page.getByTestId("calendar-chip")).toContainText(
      "aus Kalender · 3 Teilnehmende",
    );
    await confirmStart(page);
    await expect
      .poll(async () => (await calls(page, "meetings_start_from_event")).length)
      .toBe(1);
    expect((await calls(page, "meetings_start_from_event"))[0].args).toEqual({
      eventKey: "ics-1:uid-1:1790589600000",
      appKey: null,
      consentConfirmed: true,
      captureSystem: true,
      title: null,
      linkMode: "auto",
    });
    expect(await calls(page, "meetings_start")).toHaveLength(0);
    // Die Standardvorlage darf die Vorlage der Serie nicht ueberschreiben.
    expect(await calls(page, "meetings_set_template")).toHaveLength(0);
  });

  test("Titelvorschlag: ✕ löst den Bezug, der Start läuft wie bisher über meetings_start", async ({
    page,
  }) => {
    await setup(page, withSuggestion);
    await openRecordings(page);
    await openStart(page);
    await page.getByTestId("calendar-chip-clear").click();
    await expect(page.getByTestId("calendar-chip")).toHaveCount(0);
    // Der Titel bleibt stehen, wird aber nicht wieder vorgeschlagen.
    await expect(page.getByPlaceholder("Titel der Besprechung")).toHaveValue(
      "Jour fixe Vertrieb",
    );
    await confirmStart(page);
    await expect
      .poll(async () => (await calls(page, "meetings_start")).length)
      .toBe(1);
    expect((await calls(page, "meetings_start"))[0].args).toEqual({
      title: "Jour fixe Vertrieb",
      consentConfirmed: true,
      captureSystem: true,
    });
    expect(await calls(page, "meetings_start_from_event")).toHaveLength(0);
  });

  test("Titelvorschlag: ein selbst geschriebener Titel gewinnt", async ({
    page,
  }) => {
    await setup(page, withSuggestion);
    await openRecordings(page);
    await openStart(page);
    const input = page.getByPlaceholder("Titel der Besprechung");
    await input.fill("Meyer Nachbesprechung");
    await confirmStart(page);
    await expect
      .poll(async () => (await calls(page, "meetings_start_from_event")).length)
      .toBe(1);
    expect(
      (await calls(page, "meetings_start_from_event"))[0].args as any,
    ).toMatchObject({
      eventKey: "ics-1:uid-1:1790589600000",
      title: "Meyer Nachbesprechung",
    });
  });

  test("Ohne Termin im Fenster gibt es weder Vorschlag noch Chip noch Terminkarte", async ({
    page,
  }) => {
    await setup(page);
    await openRecordings(page);
    await expect(page.getByTestId("upcoming-card")).toHaveCount(0);
    await openStart(page);
    await expect(page.getByPlaceholder("Titel der Besprechung")).toHaveValue(
      "",
    );
    await expect(page.getByTestId("calendar-chip")).toHaveCount(0);
    await confirmStart(page);
    await expect
      .poll(async () => (await calls(page, "meetings_start")).length)
      .toBe(1);
    expect(await calls(page, "meetings_start_from_event")).toHaveLength(0);
  });

  const withUpcoming = () => {
    const base = {
      source_id: "ics-1",
      all_day: false,
      cancelled: false,
      location: null,
      join_url: null,
      description: null,
      attendees: [
        {
          email: "a@x.de",
          name: null,
          organizer: true,
          is_self: false,
          partstat: null,
        },
        {
          email: "b@x.de",
          name: null,
          organizer: false,
          is_self: false,
          partstat: null,
        },
      ],
    };
    (window as any).__upcoming = [
      {
        ...base,
        key: "ics-1:a:1",
        uid: "a",
        title: "Design Review",
        starts_at: Date.parse("2026-09-29T09:10:00+02:00"),
        ends_at: Date.parse("2026-09-29T10:00:00+02:00"),
      },
      {
        ...base,
        key: "ics-1:b:2",
        uid: "b",
        title: "Kundentermin Meyer",
        starts_at: Date.parse("2026-09-29T14:00:00+02:00"),
        ends_at: Date.parse("2026-09-29T15:00:00+02:00"),
      },
      {
        ...base,
        key: "ics-1:c:3",
        uid: "c",
        title: "Urlaub",
        all_day: true,
        starts_at: Date.parse("2026-09-29T00:00:00+02:00"),
        ends_at: Date.parse("2026-09-30T00:00:00+02:00"),
      },
    ];
  };

  test("Als Nächstes: der nächste Termin steht oben, die weiteren von heute hinter „Weitere Termine“", async ({
    page,
  }) => {
    await setup(page, withUpcoming);
    await openRecordings(page);
    // Die Karte „Nächste Termine“ der Aufnahmezeile gibt es nicht mehr.
    await expect(page.getByTestId("rec-controls")).not.toContainText(
      "Nächste Termine (heute)",
    );
    await expect(page.getByTestId("next-up-title")).toHaveText("Design Review");
    const card = page.getByTestId("upcoming-card");
    await expect(card).toContainText("Weitere Termine heute (1)");
    const rows = page.getByTestId("upcoming-event");
    // Standardmäßig zu: der Abschnitt bleibt klein.
    await expect(rows).toHaveCount(0);
    await expect(page.getByTestId("upcoming-toggle")).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    await page.getByTestId("upcoming-toggle").click();
    await expect(rows).toHaveCount(1); // der Ganztagstermin fehlt
    await expect(rows.nth(0)).toContainText("14:00–15:00");
    await expect(rows.nth(0)).toContainText("Kundentermin Meyer");
    await expect(rows.nth(0)).toContainText("2 Teilnehmende");
    await page.getByTestId("upcoming-toggle").click();
    await expect(rows).toHaveCount(0);
  });

  test("Als Nächstes: [Aufnehmen] je Termin öffnet den Startdialog mit dem Termin und geht über die Einwilligung", async ({
    page,
  }) => {
    await setup(page, withUpcoming);
    await openRecordings(page);
    await page.getByTestId("upcoming-toggle").click();
    await page.getByTestId("upcoming-start").click();
    const dialog = page.getByTestId("start-dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog.getByPlaceholder("Titel der Besprechung")).toHaveValue(
      "Kundentermin Meyer",
    );
    await expect(page.getByText("Einwilligung erforderlich")).toBeVisible();
    expect(await calls(page, "meetings_start_from_event")).toHaveLength(0);
    await confirmStart(page);
    await expect
      .poll(async () => (await calls(page, "meetings_start_from_event")).length)
      .toBe(1);
    expect((await calls(page, "meetings_start_from_event"))[0].args).toEqual({
      eventKey: "ics-1:b:2",
      appKey: null,
      consentConfirmed: true,
      captureSystem: true,
      title: null,
      linkMode: "prompt",
    });
  });

  test("Als Nächstes: der Knopf am nächsten Termin öffnet denselben Startdialog", async ({
    page,
  }) => {
    await setup(page, withUpcoming);
    await openRecordings(page);
    await page.getByTestId("next-up-start").click();
    const dialog = page.getByTestId("start-dialog");
    await expect(dialog.getByPlaceholder("Titel der Besprechung")).toHaveValue(
      "Design Review",
    );
    await confirmStart(page);
    await expect
      .poll(async () => (await calls(page, "meetings_start_from_event")).length)
      .toBe(1);
    expect(
      (await calls(page, "meetings_start_from_event"))[0].args,
    ).toMatchObject({ eventKey: "ics-1:a:1", linkMode: "prompt" });
  });

  test("Mit verbundenem Kalender, aber ohne Termine sagt „Als Nächstes“ es", async ({
    page,
  }) => {
    await setup(page, () => {
      (window as any).__sources = [
        {
          id: "ics-1",
          kind: "ics",
          label: "Outlook Arbeit",
          account_hint: "outlook.office365.com",
          enabled: true,
          has_attendee_data: true,
          last_sync_at: 1,
          last_ok_at: 1,
          last_error: null,
          event_count: 0,
        },
      ];
    });
    await openRecordings(page);
    await expect(page.getByTestId("next-up")).toContainText(
      "Kein Termin in den nächsten 7 Tagen.",
    );
  });
});
