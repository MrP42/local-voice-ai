import { test, expect, type Page } from "@playwright/test";
import { calls, installTauriMock } from "./calendarMock";
import { geheZu } from "./profilmenue";

// G4 (#499): Karte "Vorbereitet aus deinen letzten Notizen" an der Aufnahme zum
// Termin und die beiden Schalter unter Einstellungen > Eingabe > Besprechungen,
// gegen die Tauri-Attrappe (`window.__prep` ist die Antwort von `meeting_prep_get`).
// Erfundene Daten.

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

/** 29.09.2026 09:00 Ortszeit. */
const NOW = Date.parse("2026-09-29T09:00:00+02:00");
const secs = (iso: string) => Math.floor(Date.parse(iso) / 1000);

const point = (
  text: string,
  kind: string,
  meetingId: string,
  title: string,
  at: number,
) => ({ text, kind, source: { meeting_id: meetingId, title, at } });

const PREP = {
  points: [
    point(
      "Anna: Preisliste an Meyer zusenden",
      "delivery",
      "m-okt",
      "Vertrieb Oktober",
      secs("2026-09-12T10:00:00+02:00"),
    ),
    point(
      "Wer zahlt die Schulung?",
      "question",
      "m-sep",
      "Vertrieb September",
      secs("2026-09-05T10:00:00+02:00"),
    ),
  ],
  mode: "rules",
  meetings: 2,
};

type Init = {
  /** Es gibt einen laufenden Termin (Titelvorschlag). */
  event?: boolean;
  prep?: unknown;
  settings?: Record<string, unknown>;
};

const setup = async (page: Page, init: Init = {}) => {
  await page.clock.setFixedTime(NOW);
  await installTauriMock(page, "main");
  await page.addInitScript(({ event, prep, settings }) => {
    const w = window as any;
    if (event) {
      w.__suggest = {
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
            email: "anna@example.org",
            name: "Anna Berg",
            organizer: true,
            is_self: false,
            partstat: null,
          },
        ],
      };
    }
    w.__prep = prep ?? null;
    Object.assign(w.__settings, settings ?? {});
  }, init);
  await page.setViewportSize({ width: 1280, height: 1000 });
  await page.goto("/");
};

const openRecordings = async (page: Page) => {
  await page.getByRole("button", { name: "Aufnahmen", exact: true }).click();
};

test.describe("Terminvorbereitung: Karte", () => {
  test("zeigt die Punkte mit Quelle, fragt den Termin ab und springt zur Besprechung", async ({
    page,
  }) => {
    await setup(page, { event: true, prep: PREP });
    await openRecordings(page);

    const card = page.getByTestId("prep-card");
    await expect(card).toBeVisible();
    await expect(card).toContainText("Vorbereitet aus deinen letzten Notizen");
    await expect(page.getByTestId("prep-point")).toHaveCount(2);
    await expect(page.getByTestId("prep-point").first()).toContainText(
      "Anna: Preisliste an Meyer zusenden",
    );
    await expect(page.getByTestId("prep-source").first()).toHaveText(
      "Vertrieb Oktober · 12.09.2026",
    );
    await expect(page.getByTestId("prep-mode")).toHaveText(
      "Aus deinen Notizen übernommen, wie sie dort stehen.",
    );
    expect((await calls(page, "meeting_prep_get"))[0].args).toEqual({
      eventKey: "ics-1:uid-1:1790589600000",
    });

    // Der Sprung wechselt zu den Aufnahmen und meldet die Besprechung.
    await page.evaluate(() => {
      (window as any).__navigated = [];
      window.addEventListener("lv-navigate", (e) =>
        (window as any).__navigated.push((e as CustomEvent).detail),
      );
    });
    await page
      .getByRole("button", {
        name: "Besprechung „Vertrieb Oktober“ vom 12.09.2026 öffnen",
      })
      .click();
    expect(await page.evaluate(() => (window as any).__navigated)).toEqual([
      { section: "meetings" },
    ]);
  });

  test("mit Modell steht der Hinweis auf die Auswahl durch das Modell", async ({
    page,
  }) => {
    await setup(page, { event: true, prep: { ...PREP, mode: "model" } });
    await openRecordings(page);
    await expect(page.getByTestId("prep-mode")).toHaveText(
      "Vom gewählten Modell aus deinen Notizen ausgewählt und gekürzt.",
    );
  });

  test("klappt ein und wieder auf; Schreiben in ein Notizfeld klappt sie ein", async ({
    page,
  }) => {
    await setup(page, { event: true, prep: PREP });
    await openRecordings(page);
    const card = page.getByTestId("prep-card");
    await expect(card).toHaveAttribute("data-expanded", "true");

    await page.getByTestId("prep-toggle").click();
    await expect(card).toHaveAttribute("data-expanded", "false");
    await expect(page.getByTestId("prep-point")).toHaveCount(0);
    await page.getByTestId("prep-toggle").click();
    await expect(card).toHaveAttribute("data-expanded", "true");

    // Ein Notizfeld ausserhalb der Karte: Tippen klappt die Karte ein.
    await page.evaluate(() => {
      const area = document.createElement("textarea");
      area.setAttribute("data-testid", "fake-notes");
      document.body.appendChild(area);
    });
    await page.getByTestId("fake-notes").click();
    await page.keyboard.type("Meine erste Notiz");
    await expect(card).toHaveAttribute("data-expanded", "false");
    // Die Karte bleibt da und laesst sich wieder aufklappen.
    await page.getByTestId("prep-toggle").click();
    await expect(card).toHaveAttribute("data-expanded", "true");
  });

  test("ohne frühere Notiz (leere Liste oder keine Antwort) erscheint nichts", async ({
    page,
  }) => {
    await setup(page, {
      event: true,
      prep: { points: [], mode: "rules", meetings: 0 },
    });
    await openRecordings(page);
    await expect
      .poll(async () => (await calls(page, "meeting_prep_get")).length)
      .toBeGreaterThan(0);
    await expect(page.getByTestId("prep-card")).toHaveCount(0);
  });

  test("ohne Termin gibt es keine Anfrage und keine Karte", async ({
    page,
  }) => {
    await setup(page, {
      prep: { points: PREP.points, mode: "rules", meetings: 1 },
    });
    await openRecordings(page);
    await expect(
      page.getByRole("button", { name: "Aufnahme starten" }).first(),
    ).toBeVisible();
    expect(await calls(page, "meeting_prep_get")).toHaveLength(0);
    await expect(page.getByTestId("prep-card")).toHaveCount(0);
  });

  test("ausgeschaltet in den Einstellungen: keine Anfrage, keine Karte", async ({
    page,
  }) => {
    await setup(page, {
      event: true,
      prep: PREP,
      settings: { meeting_prep_enabled: false },
    });
    await openRecordings(page);
    await expect(
      page.getByRole("button", { name: "Aufnahme starten" }).first(),
    ).toBeVisible();
    expect(await calls(page, "meeting_prep_get")).toHaveLength(0);
    await expect(page.getByTestId("prep-card")).toHaveCount(0);
  });
});

test.describe("Terminvorbereitung: Einstellungen", () => {
  const toggleOf = (page: Page, label: string) =>
    page
      .getByText(label, { exact: true })
      .locator("xpath=ancestor::div[.//input[@type='checkbox']][1]")
      .locator("input[type=checkbox]");

  test("Funktion steht auf an, Vorab auf aus; beide Schalter gehen ans Backend", async ({
    page,
  }) => {
    await setup(page);
    await geheZu(page, "Einstellungen");
    await expect(page.getByTestId("meeting-prep-settings")).toBeVisible();

    const enabled = toggleOf(page, "Vorbereitung vor dem Termin");
    const prefetch = toggleOf(page, "Termine des Tages vorab vorbereiten");
    await expect(enabled).toBeChecked();
    await expect(prefetch).not.toBeChecked();

    await prefetch.evaluate((el) => (el as HTMLInputElement).click());
    await expect
      .poll(
        async () =>
          (await calls(page, "change_meeting_prep_prefetch_setting")).length,
      )
      .toBe(1);
    expect(
      (await calls(page, "change_meeting_prep_prefetch_setting"))[0].args,
    ).toEqual({ enabled: true });

    await enabled.evaluate((el) => (el as HTMLInputElement).click());
    await expect
      .poll(
        async () =>
          (await calls(page, "change_meeting_prep_enabled_setting")).length,
      )
      .toBe(1);
    expect(
      (await calls(page, "change_meeting_prep_enabled_setting"))[0].args,
    ).toEqual({ enabled: false });
    // Ohne Funktion gibt es nichts vorab zu erzeugen.
    await expect(prefetch).toBeDisabled();
  });
});
