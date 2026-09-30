import { test, expect, type Locator, type Page } from "@playwright/test";
import * as fs from "node:fs";
import * as path from "node:path";
import {
  TITLE_M2,
  VIEWPORTS,
  installRecMock,
  openRecordings,
  scrollReport,
} from "./recLayoutMock";

// Aufnahmen-Oberflaeche (Goal aufnahmen-ui, M3, AK3): die Projekte-Spalte.
// Projekte = Ordner der obersten Ebene; hier gegen eine zustandsbehaftete
// Attrappe der Ordner-Commands (Zuordnung n:m, Reihenfolge, Zaehler).

const NOW = Date.parse("2026-09-30T09:00:00+02:00");

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

/** Kalenderquelle und ein Termin in einer Stunde (vor dem Laden der Seite). */
const withCalendar = (page: Page) =>
  page.addInitScript(() => {
    (window as any).__calendarOn = true;
  });

/**
 * Legt die Ordner-Commands ueber die Attrappe der Aufnahmenseite. Die Zuordnung
 * (m1, m4 in Privat; m2 in Geschaeftlich und Kunde Stadtwerke; m5 in
 * Geschaeftlich; m6 in Podcast; m3 ohne Projekt) steht in `window.__assigned`.
 */
const installProjectsMock = async (page: Page) => {
  await installRecMock(page);
  await page.addInitScript(
    ({ now }) => {
      const w = window as any;
      const inner = w.__TAURI_INTERNALS__.invoke;
      w.__calls = [];
      w.__assigned = {
        m1: ["f1"],
        m4: ["f1"],
        m2: ["f2", "f3"],
        m5: ["f2"],
        m6: ["f4"],
        m3: [],
      };
      const folders = () =>
        [...w.__folders]
          .sort((a: any, b: any) => a.sort - b.sort)
          .map((f: any) => ({
            ...f,
            meeting_count: w.__meetings.filter((m: any) =>
              (w.__assigned[m.id] ?? []).includes(f.id),
            ).length,
          }));
      const live = (id: string) => w.__folders.some((f: any) => f.id === id);
      const unfiledIds = () =>
        w.__meetings
          .filter(
            (m: any) =>
              !(w.__assigned[m.id] ?? []).some((id: string) => live(id)),
          )
          .map((m: any) => m.id);
      const transcriptOf = (id: string) =>
        id === "m2" ? w.__segments.map((s: any) => s.text).join(" ") : "";
      const event = {
        key: "ics:abstimmung:1",
        source_id: "ics-1",
        uid: "abstimmung",
        title: "Abstimmung Stadtwerke",
        starts_at: now + 3_600_000,
        ends_at: now + 7_200_000,
        all_day: false,
        cancelled: false,
        location: null,
        join_url: null,
        description: null,
        attendees: [
          { email: "a@x.de", name: "Anna", organizer: true },
          { email: "b@x.de", name: "Ben", organizer: false },
        ],
      };
      w.__TAURI_INTERNALS__.invoke = async (
        cmd: string,
        args: Record<string, any> = {},
      ) => {
        w.__calls.push({ cmd, args });
        switch (cmd) {
          case "meeting_folders_list":
            return folders();
          case "meeting_folders_counts":
            return { all: w.__meetings.length, unfiled: unfiledIds().length };
          case "meeting_folders_save": {
            const name = String(args.name).trim();
            if (
              w.__folders.some(
                (f: any) =>
                  f.id !== args.id &&
                  f.name.toLowerCase() === name.toLowerCase(),
              )
            )
              throw "folder_name_taken";
            if (args.id) {
              w.__folders = w.__folders.map((f: any) =>
                f.id === args.id ? { ...f, name } : f,
              );
              return folders().find((f: any) => f.id === args.id);
            }
            const sort =
              Math.max(0, ...w.__folders.map((f: any) => f.sort)) + 1;
            const created = {
              id: `f${sort + 10}`,
              name,
              color: null,
              sort,
              meeting_count: 0,
              created_at: 1,
              updated_at: 1,
            };
            w.__folders = [...w.__folders, created];
            return created;
          }
          case "meeting_folders_delete":
            w.__folders = w.__folders.filter((f: any) => f.id !== args.id);
            return null;
          case "meeting_folders_reorder": {
            const rest = folders()
              .map((f: any) => f.id)
              .filter((id: string) => !args.ids.includes(id));
            [...args.ids, ...rest].forEach((id: string, i: number) => {
              w.__folders = w.__folders.map((f: any) =>
                f.id === id ? { ...f, sort: i + 1 } : f,
              );
            });
            return null;
          }
          case "meetings_get_folders":
            return (w.__assigned[args.meetingId] ?? []).filter(live);
          case "meetings_set_folders":
            w.__assigned[args.meetingId] = args.folderIds;
            return null;
          case "meetings_search": {
            const f = args.filter ?? {};
            const q = String(args.query ?? "").toLowerCase();
            const items = w.__meetings
              .filter((m: any) => {
                const ids = (w.__assigned[m.id] ?? []).filter(live);
                if (f.unfiled && ids.length > 0) return false;
                if (f.folder_id && !ids.includes(f.folder_id)) return false;
                if (f.from && (m.started_at ?? m.created_at) < f.from)
                  return false;
                if (
                  q &&
                  !m.title.toLowerCase().includes(q) &&
                  !transcriptOf(m.id).toLowerCase().includes(q)
                )
                  return false;
                return true;
              })
              .map((m: any) => ({
                meeting: m,
                snippet: null,
                hit_source: null,
              }));
            return { items, total: items.length, truncated: false };
          }
          case "calendar_sources_list":
            return w.__calendarOn
              ? [{ id: "ics-1", kind: "ics", label: "Arbeit" }]
              : [];
          case "calendar_upcoming":
            return w.__calendarOn ? [event] : [];
          case "meetings_start_from_event":
            return { ...w.__meetings[0], id: "m-new", status: "recording" };
        }
        return inner(cmd, args);
      };
    },
    { now: NOW },
  );
};

const callsOf = (page: Page, cmd: string): Promise<{ args: any }[]> =>
  page.evaluate(
    (c) => (window as any).__calls.filter((x: any) => x.cmd === c),
    cmd,
  );

const sessions = (page: Page) => page.getByTestId("rec-sessions");
const row = (page: Page, name: string | RegExp) =>
  sessions(page).getByTestId("project-row").filter({ hasText: name });
const meetingRow = (page: Page, id: string) =>
  page.locator(`[data-meeting-id="${id}"]`);
const projectNames = (page: Page) =>
  sessions(page)
    .getByTestId("project-row")
    .evaluateAll((els) =>
      els.map((el) =>
        (el.querySelector("span.truncate")?.textContent ?? "").trim(),
      ),
    );

/** Eintrag im Kontextmenue (schliesst beim Scrollen: per JS klicken). */
const menuItem = (page: Page, name: string) =>
  page
    .getByRole("menuitem", { name, exact: true })
    .evaluate((el) => (el as HTMLElement).click());

/** Besprechung mit dem Zeiger auf ein Ziel ziehen. */
async function dragTo(
  page: Page,
  source: Locator,
  target: Locator,
  options: { ctrl?: boolean; drop?: boolean } = {},
) {
  const a = await source.boundingBox();
  const b = await target.boundingBox();
  expect(a).not.toBeNull();
  expect(b).not.toBeNull();
  await page.mouse.move(a!.x + 24, a!.y + 10);
  await page.mouse.down();
  await page.mouse.move(a!.x + 40, a!.y + 24, { steps: 3 });
  if (options.ctrl) await page.keyboard.down("Control");
  await page.mouse.move(b!.x + b!.width / 2, b!.y + b!.height / 2, {
    steps: 8,
  });
  if (options.drop === false) return;
  await page.mouse.up();
  if (options.ctrl) await page.keyboard.up("Control");
}

test.beforeEach(async ({ page }) => {
  await installProjectsMock(page);
});

test("Spalte: Alle Aufnahmen, Projekte mit Anzahl, Ohne Projekt", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  await expect(
    sessions(page).getByRole("heading", { name: "Projekte" }),
  ).toBeVisible();
  expect(await projectNames(page)).toEqual([
    "Alle Aufnahmen",
    "Privat",
    "Geschäftlich",
    "Kunde Stadtwerke",
    "Podcast",
    "Ohne Projekt",
  ]);
  const counts = await sessions(page)
    .getByTestId("project-count")
    .allTextContents();
  expect(counts).toEqual(["6", "2", "2", "1", "1", "1"]);
  // Die Filter-Chips stehen nicht mehr in der Spalte.
  await expect(
    sessions(page).getByRole("button", { name: "7 Tage" }),
  ).toHaveCount(0);
  await expect(
    sessions(page).getByRole("button", { name: "Mit Notizen" }),
  ).toHaveCount(0);
  // Unter der gewaehlten Zeile stehen ihre Besprechungen.
  await expect(row(page, "Alle Aufnahmen")).toHaveAttribute(
    "aria-current",
    "true",
  );
  await expect(sessions(page).getByTestId("meeting-row")).toHaveCount(6);
});

test("Projekt wählen zeigt seine Besprechungen, n:m: eine Besprechung steht in jedem ihrer Projekte", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  await row(page, "Geschäftlich").click();
  await expect(row(page, "Geschäftlich")).toHaveAttribute(
    "aria-current",
    "true",
  );
  await expect(sessions(page).getByTestId("meeting-row")).toHaveCount(2);
  await expect(meetingRow(page, "m2")).toBeVisible();
  await expect(meetingRow(page, "m5")).toBeVisible();
  const search = (await callsOf(page, "meetings_search")).at(-1)!;
  expect(search.args.filter.folder_id).toBe("f2");

  await row(page, "Kunde Stadtwerke").click();
  await expect(sessions(page).getByTestId("meeting-row")).toHaveCount(1);
  await expect(meetingRow(page, "m2")).toBeVisible();

  // Ohne Projekt: nur Besprechungen ohne lebendes Projekt.
  await row(page, "Ohne Projekt").click();
  await expect(sessions(page).getByTestId("meeting-row")).toHaveCount(1);
  await expect(meetingRow(page, "m3")).toBeVisible();
  expect(
    (await callsOf(page, "meetings_search")).at(-1)!.args.filter,
  ).toMatchObject({ unfiled: true, folder_id: null });

  // Ein zweiter Klick auf die gewaehlte Zeile klappt sie zu.
  await row(page, "Ohne Projekt").click();
  await expect(sessions(page).getByTestId("meeting-row")).toHaveCount(0);
  await expect(row(page, "Ohne Projekt")).toHaveAttribute(
    "aria-expanded",
    "false",
  );

  // Eine Besprechung oeffnet die Arbeitsflaeche.
  await row(page, "Geschäftlich").click();
  await meetingRow(page, "m2").click();
  await expect(page.getByText(TITLE_M2).first()).toBeVisible();
});

test("Anlegen: Plus, Name, Enter; doppelter Name bleibt in der Eingabe", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  await page.getByTestId("projects-add").click();
  const input = sessions(page).getByRole("textbox", {
    name: "Name des Projekts",
  });
  await expect(input).toBeFocused();
  await input.fill("privat");
  await input.press("Enter");
  await expect(sessions(page).getByRole("alert")).toContainText(
    "Ein Projekt mit diesem Namen gibt es schon.",
  );
  await expect(input).toBeVisible();
  await input.fill("Weiterbildung");
  await input.press("Enter");
  await expect(input).toHaveCount(0);
  const save = (await callsOf(page, "meeting_folders_save")).at(-1)!;
  expect(save.args).toMatchObject({ id: null, name: "Weiterbildung" });
  // Das neue Projekt steht vor "Ohne Projekt" und ist gewaehlt.
  expect((await projectNames(page)).slice(-2)).toEqual([
    "Weiterbildung",
    "Ohne Projekt",
  ]);
  await expect(row(page, "Weiterbildung")).toHaveAttribute(
    "aria-current",
    "true",
  );
  await expect(
    row(page, "Weiterbildung").getByTestId("project-count"),
  ).toHaveText("0");
  await expect(
    sessions(page).getByText("Noch leer. Ziehe eine Besprechung hierher."),
  ).toBeVisible();

  // Escape und leerer Name verwerfen.
  await page.getByTestId("projects-add").click();
  await sessions(page)
    .getByRole("textbox", { name: "Name des Projekts" })
    .press("Escape");
  await expect(
    sessions(page).getByRole("textbox", { name: "Name des Projekts" }),
  ).toHaveCount(0);
  expect(await callsOf(page, "meeting_folders_save")).toHaveLength(2);
});

test("Umbenennen: Doppelklick, F2 und Kontextmenü", async ({ page }) => {
  await openRecordings(page, 1366, 768);
  const edit = sessions(page).getByRole("textbox", {
    name: "Name des Projekts",
  });

  // Die Liste einer anderen Zeile klappt beim Waehlen zu und schiebt die Zeilen
  // darunter nach oben: erst waehlen, dann doppelklicken.
  await row(page, "Privat").click();
  await row(page, "Privat").dblclick();
  await expect(edit).toBeFocused();
  await edit.fill("Familie");
  await edit.press("Enter");
  await expect(row(page, "Familie")).toBeVisible();
  expect(
    (await callsOf(page, "meeting_folders_save")).at(-1)!.args,
  ).toMatchObject({ id: "f1", name: "Familie" });

  await row(page, "Podcast").focus();
  await page.keyboard.press("F2");
  await edit.fill("Podcasts");
  // Ein Klick daneben speichert.
  await page.getByTestId("rec-content").click({ position: { x: 20, y: 20 } });
  await expect(row(page, "Podcasts")).toBeVisible();

  await row(page, "Geschäftlich").click({ button: "right" });
  await menuItem(page, "Umbenennen");
  await expect(edit).toBeFocused();
  await edit.fill("Beruf");
  await edit.press("Enter");
  await expect(row(page, "Beruf")).toBeVisible();

  // Escape lässt den Namen, wie er war.
  await row(page, "Beruf").click();
  await row(page, "Beruf").dblclick();
  await edit.fill("Nichts");
  await edit.press("Escape");
  await expect(row(page, "Beruf")).toBeVisible();
  await expect(row(page, "Nichts")).toHaveCount(0);
});

test("Sortieren: Kontextmenü hoch/runter und Alt+Pfeil", async ({ page }) => {
  await openRecordings(page, 1366, 768);
  await row(page, "Privat").click({ button: "right" });
  // Ganz oben gibt es kein "Nach oben".
  await expect(page.getByRole("menuitem", { name: "Nach oben" })).toHaveCount(
    0,
  );
  await menuItem(page, "Nach unten");
  await expect
    .poll(async () => (await projectNames(page)).slice(1, 5))
    .toEqual(["Geschäftlich", "Privat", "Kunde Stadtwerke", "Podcast"]);
  const reorder = (await callsOf(page, "meeting_folders_reorder")).at(-1)!;
  expect(reorder.args.ids).toEqual(["f2", "f1", "f3", "f4"]);

  await row(page, "Podcast").focus();
  await page.keyboard.press("Alt+ArrowUp");
  await expect
    .poll(async () => (await projectNames(page)).slice(1, 5))
    .toEqual(["Geschäftlich", "Privat", "Podcast", "Kunde Stadtwerke"]);
  await expect(row(page, "Podcast")).toBeFocused();

  await row(page, "Kunde Stadtwerke").click({ button: "right" });
  await expect(page.getByRole("menuitem", { name: "Nach unten" })).toHaveCount(
    0,
  );
  await page.keyboard.press("Escape");
});

test("Löschen: Rückfrage im Dialog, Besprechungen bleiben", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  await row(page, "Privat").click({ button: "right" });
  await menuItem(page, "Löschen");
  const dialog = page.getByRole("dialog", { name: "Projekt löschen?" });
  await expect(dialog).toContainText("„Privat“");
  await expect(dialog).toContainText(
    "Die Besprechungen darin bleiben erhalten",
  );

  // Abbrechen: nichts passiert.
  await dialog.locator("button", { hasText: "Abbrechen" }).click();
  await expect(dialog).toHaveCount(0);
  expect(await callsOf(page, "meeting_folders_delete")).toHaveLength(0);
  await expect(row(page, "Privat")).toBeVisible();

  await row(page, "Privat").click({ button: "right" });
  await menuItem(page, "Löschen");
  await dialog.getByRole("button", { name: "Löschen" }).click();
  await expect(row(page, "Privat")).toHaveCount(0);
  expect((await callsOf(page, "meeting_folders_delete")).at(-1)!.args).toEqual({
    id: "f1",
  });
  // m1 und m4 lagen nur in Privat: sie stehen jetzt unter Ohne Projekt.
  await expect(
    row(page, "Ohne Projekt").getByTestId("project-count"),
  ).toHaveText("3");
  await expect(
    row(page, "Alle Aufnahmen").getByTestId("project-count"),
  ).toHaveText("6");
  await row(page, "Ohne Projekt").click();
  await expect(meetingRow(page, "m1")).toBeVisible();
  await expect(meetingRow(page, "m4")).toBeVisible();
});

test("Löschen des gewählten Projekts fällt auf Alle Aufnahmen zurück", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  await row(page, "Podcast").click();
  await row(page, "Podcast").click({ button: "right" });
  await menuItem(page, "Löschen");
  await page
    .getByRole("dialog", { name: "Projekt löschen?" })
    .getByRole("button", { name: "Löschen" })
    .click();
  await expect(row(page, "Alle Aufnahmen")).toHaveAttribute(
    "aria-current",
    "true",
  );
  await expect(sessions(page).getByTestId("meeting-row")).toHaveCount(6);
});

test("Ziehen verschiebt: aus dem gewählten Projekt heraus, in das Ziel hinein", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  await row(page, "Privat").click();
  await expect(sessions(page).getByTestId("meeting-row")).toHaveCount(2);

  await dragTo(page, meetingRow(page, "m1"), row(page, "Geschäftlich"));
  await expect
    .poll(async () => (await callsOf(page, "meetings_set_folders")).length)
    .toBe(1);
  const set = (await callsOf(page, "meetings_set_folders")).at(-1)!;
  expect(set.args).toEqual({ meetingId: "m1", folderIds: ["f2"] });
  // Die Besprechung ist aus Privat verschwunden, die Zaehler stimmen.
  await expect(sessions(page).getByTestId("meeting-row")).toHaveCount(1);
  await expect(meetingRow(page, "m1")).toHaveCount(0);
  await expect(row(page, "Privat").getByTestId("project-count")).toHaveText(
    "1",
  );
  await expect(
    row(page, "Geschäftlich").getByTestId("project-count"),
  ).toHaveText("3");
  // Der Klick nach dem Loslassen oeffnet die Besprechung nicht.
  await expect(page.getByTestId("rec-content")).not.toContainText(
    "Jour fixe Vertrieb",
  );

  const toast = page.locator("[data-sonner-toast]");
  await expect(toast).toContainText("liegt jetzt in „Geschäftlich“");

  // Rückgängig stellt die vorherigen Projekte wieder her.
  await toast.getByRole("button", { name: "Rückgängig" }).click();
  await expect
    .poll(async () => (await callsOf(page, "meetings_set_folders")).length)
    .toBe(2);
  expect((await callsOf(page, "meetings_set_folders")).at(-1)!.args).toEqual({
    meetingId: "m1",
    folderIds: ["f1"],
  });
  await expect(meetingRow(page, "m1")).toBeVisible();
  await expect(row(page, "Privat").getByTestId("project-count")).toHaveText(
    "2",
  );
});

test("Strg+Ziehen fügt hinzu (n:m): die Besprechung bleibt im Quellprojekt", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  await row(page, "Privat").click();
  await dragTo(page, meetingRow(page, "m4"), row(page, "Podcast"), {
    ctrl: true,
  });
  await expect
    .poll(async () => (await callsOf(page, "meetings_set_folders")).length)
    .toBe(1);
  expect((await callsOf(page, "meetings_set_folders")).at(-1)!.args).toEqual({
    meetingId: "m4",
    folderIds: ["f1", "f4"],
  });
  await expect(meetingRow(page, "m4")).toBeVisible();
  await expect(row(page, "Privat").getByTestId("project-count")).toHaveText(
    "2",
  );
  await expect(row(page, "Podcast").getByTestId("project-count")).toHaveText(
    "2",
  );
  await expect(page.locator("[data-sonner-toast]")).toContainText(
    "liegt jetzt auch in „Podcast“",
  );
  // Die Besprechung erscheint in jedem ihrer Projekte.
  await row(page, "Podcast").click();
  await expect(meetingRow(page, "m4")).toBeVisible();
  await expect(meetingRow(page, "m6")).toBeVisible();
});

test("Ziehen aus Ohne Projekt und auf Ohne Projekt", async ({ page }) => {
  await openRecordings(page, 1366, 768);
  await row(page, "Ohne Projekt").click();
  await dragTo(page, meetingRow(page, "m3"), row(page, "Podcast"));
  await expect
    .poll(async () => (await callsOf(page, "meetings_set_folders")).length)
    .toBe(1);
  expect((await callsOf(page, "meetings_set_folders")).at(-1)!.args).toEqual({
    meetingId: "m3",
    folderIds: ["f4"],
  });
  await expect(sessions(page).getByTestId("meeting-row")).toHaveCount(0);

  // Auf "Ohne Projekt": danach in keinem Projekt.
  await row(page, "Geschäftlich").click();
  await dragTo(page, meetingRow(page, "m2"), row(page, "Ohne Projekt"));
  await expect
    .poll(async () => (await callsOf(page, "meetings_set_folders")).length)
    .toBe(2);
  expect((await callsOf(page, "meetings_set_folders")).at(-1)!.args).toEqual({
    meetingId: "m2",
    folderIds: [],
  });
  await expect(
    page.locator("[data-sonner-toast]", {
      hasText: "liegt in keinem Projekt mehr",
    }),
  ).toBeVisible();
});

test("Ziehen: ein Klick ohne Bewegung öffnet, Escape bricht ab, ohne Ziel passiert nichts", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  await row(page, "Privat").click();
  // Abbrechen mit Escape.
  await dragTo(page, meetingRow(page, "m1"), row(page, "Podcast"), {
    drop: false,
  });
  await expect(page.getByTestId("drag-ghost")).toBeVisible();
  await expect(row(page, "Podcast")).toHaveClass(/border-logo-primary/);
  await page.keyboard.press("Escape");
  await page.mouse.up();
  await expect(page.getByTestId("drag-ghost")).toHaveCount(0);
  // Ohne Ziel (losgelassen in der Arbeitsflaeche).
  const a = (await meetingRow(page, "m1").boundingBox())!;
  await page.mouse.move(a.x + 20, a.y + 10);
  await page.mouse.down();
  await page.mouse.move(a.x + 120, a.y + 40, { steps: 4 });
  await page.mouse.move(700, 300, { steps: 4 });
  await page.mouse.up();
  expect(await callsOf(page, "meetings_set_folders")).toHaveLength(0);

  // Ein Klick ohne Bewegung oeffnet die Besprechung weiterhin.
  await meetingRow(page, "m1").click();
  await expect(meetingRow(page, "m1")).toHaveAttribute("aria-current", "true");
});

test("Kontextmenü „In Projekt verschieben …“ ordnet per Dialog zu", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  await meetingRow(page, "m3").click({ button: "right" });
  await menuItem(page, "In Projekt verschieben …");
  const dialog = page.getByRole("dialog", { name: "In Projekt verschieben …" });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("checkbox", { name: "Podcast" }).click();
  await dialog.getByRole("checkbox", { name: "Privat" }).click();
  await dialog.getByRole("button", { name: "Speichern" }).click();
  expect((await callsOf(page, "meetings_set_folders")).at(-1)!.args).toEqual({
    meetingId: "m3",
    folderIds: ["f4", "f1"],
  });
  await expect(
    row(page, "Ohne Projekt").getByTestId("project-count"),
  ).toHaveText("0");
  await expect(row(page, "Podcast").getByTestId("project-count")).toHaveText(
    "2",
  );
});

test("Suche wirkt innerhalb des gewählten Projekts (Titel und Transkript)", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  await row(page, "Geschäftlich").click();
  const box = page.getByRole("searchbox", {
    name: "Besprechungen durchsuchen",
  });
  await expect(box).toHaveAttribute(
    "placeholder",
    "In „Geschäftlich“ suchen …",
  );

  // Transkript: "Speicher" steht nur im Transkript von m2.
  await box.fill("Speicher");
  await expect(sessions(page).getByTestId("meeting-row")).toHaveCount(1);
  await expect(meetingRow(page, "m2")).toBeVisible();
  await expect(sessions(page).getByTestId("search-scope")).toHaveText(
    "1 Treffer in „Geschäftlich“",
  );
  const search = (await callsOf(page, "meetings_search")).at(-1)!;
  expect(search.args.query).toBe("Speicher");
  expect(search.args.filter.folder_id).toBe("f2");

  // Titel: m1 passt zum Suchwort, liegt aber nicht in Geschäftlich.
  await box.fill("Jour fixe");
  await expect(
    sessions(page).getByText("Keine Treffer.", { exact: false }),
  ).toBeVisible();
  await expect(meetingRow(page, "m1")).toHaveCount(0);

  // Anderes Projekt, gleiche Suche: m1 liegt in Privat.
  await box.fill("");
  await row(page, "Privat").click();
  await box.fill("Jour fixe");
  await expect(meetingRow(page, "m1")).toBeVisible();
  await expect(sessions(page).getByTestId("search-scope")).toHaveText(
    "1 Treffer in „Privat“",
  );

  // Während der Suche stehen die Projektzeilen nicht im Weg.
  await expect(sessions(page).getByTestId("project-row")).toHaveCount(0);
  await box.fill("");
  await expect(sessions(page).getByTestId("project-row")).toHaveCount(6);
});

test("Filter-Popover: Zeitraum, Quelle und Notizen hinter einem Symbol", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  const filter = sessions(page).getByTestId("projects-filter");
  await expect(filter).toBeVisible();
  await expect(page.getByTestId("projects-filter-popover")).toHaveCount(0);
  await expect(page.getByTestId("projects-filter-count")).toHaveCount(0);

  await filter.click();
  const popover = page.getByTestId("projects-filter-popover");
  await expect(popover).toBeVisible();
  for (const name of [
    "7 Tage",
    "30 Tage",
    "12 Monate",
    "Aufnahme",
    "Import",
    "Untertitel",
    "Mit Notizen",
  ]) {
    await expect(
      popover.getByRole("button", { name, exact: true }),
    ).toBeVisible();
  }
  await popover.getByRole("button", { name: "7 Tage", exact: true }).click();
  await expect
    .poll(
      async () =>
        (await callsOf(page, "meetings_search")).at(-1)?.args.filter.from,
    )
    .not.toBeNull();
  await expect(page.getByTestId("projects-filter-count")).toHaveText("1");
  await popover
    .getByRole("button", { name: "Mit Notizen", exact: true })
    .click();
  await expect(page.getByTestId("projects-filter-count")).toHaveText("2");
  expect(
    (await callsOf(page, "meetings_search")).at(-1)!.args.filter.has_notes,
  ).toBe(true);

  // Zuruecksetzen leert alle und kehrt zur bisherigen Liste zurueck.
  await popover.getByRole("button", { name: "Filter zurücksetzen" }).click();
  await expect(page.getByTestId("projects-filter-count")).toHaveCount(0);
  await expect(
    popover.getByRole("button", { name: "7 Tage", exact: true }),
  ).toBeFocused();
  // Escape schliesst das Popover, der Fokus kehrt zum Symbol zurueck.
  await page.keyboard.press("Escape");
  await expect(popover).toHaveCount(0);
  await expect(filter).toBeFocused();
});

test("Persistenz: das gewählte Projekt übersteht Neuladen und Seitenwechsel", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  await row(page, "Kunde Stadtwerke").click();
  await expect(meetingRow(page, "m2")).toBeVisible();

  await page.reload();
  await page.getByRole("button", { name: "Aufnahmen", exact: true }).click();
  await expect(row(page, "Kunde Stadtwerke")).toHaveAttribute(
    "aria-current",
    "true",
  );
  await expect(sessions(page).getByTestId("meeting-row")).toHaveCount(1);

  // Seitenwechsel: Vorlesen und zurueck.
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Vorlesen", exact: true })
    .click();
  await page.getByRole("button", { name: "Aufnahmen", exact: true }).click();
  await expect(row(page, "Kunde Stadtwerke")).toHaveAttribute(
    "aria-current",
    "true",
  );

  // Ein Projekt, das es nicht mehr gibt, faellt auf "Alle Aufnahmen" zurueck.
  await page.evaluate(() =>
    localStorage.setItem("lva.ui.meetings.project", "gibt-es-nicht"),
  );
  await page.reload();
  await page.getByRole("button", { name: "Aufnahmen", exact: true }).click();
  await expect(row(page, "Alle Aufnahmen")).toHaveAttribute(
    "aria-current",
    "true",
  );
});

test("Meldung „Liste geändert“ (meetingsBus) lädt Liste und Zähler neu", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  await row(page, "Geschäftlich").click();
  await expect(sessions(page).getByTestId("meeting-row")).toHaveCount(2);
  // Anderswo (Detailkopf, Bedienspalte) wird eine Besprechung verschoben.
  await page.evaluate(() => {
    const w = window as any;
    w.__assigned.m3 = ["f2"];
    window.dispatchEvent(new CustomEvent("lva:meetings-changed"));
  });
  await expect(sessions(page).getByTestId("meeting-row")).toHaveCount(3);
  await expect(meetingRow(page, "m3")).toBeVisible();
  await expect(
    row(page, "Geschäftlich").getByTestId("project-count"),
  ).toHaveText("3");
  await expect(
    row(page, "Ohne Projekt").getByTestId("project-count"),
  ).toHaveText("0");
});

test("Das gewählte Projekt steht für andere Bausteine bereit (Schlüssel im Speicher)", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  await row(page, "Kunde Stadtwerke").click();
  await expect
    .poll(() =>
      page.evaluate(() => localStorage.getItem("lva.ui.meetings.project")),
    )
    .toBe("f3");
  await row(page, "Ohne Projekt").click();
  await expect
    .poll(() =>
      page.evaluate(() => localStorage.getItem("lva.ui.meetings.project")),
    )
    .toBe("none");
});

test("„Alle Besprechungen fragen“ öffnet den Chat mit dem Projekt als Umfang", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  await row(page, "Geschäftlich").click();
  await sessions(page).getByTestId("projects-ask").click();
  await expect(
    page.getByTestId("rec-controls").getByRole("tab", { name: "Fragen" }),
  ).toHaveAttribute("aria-selected", "true");
  await expect(page.getByTestId("rec-chat")).toBeVisible();
});

test("Eingeklappte Leiste zeigt Projekte als Kürzel und nimmt Ablagen an", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  await page.getByTestId("sessions-collapse").click();
  const rail = page.getByTestId("projects-rail");
  await expect(rail).toBeVisible();
  const labels = await rail
    .getByTestId("rail-project")
    .evaluateAll((els) => els.map((el) => el.getAttribute("aria-label")));
  expect(labels).toEqual([
    "Alle Aufnahmen",
    "Privat",
    "Geschäftlich",
    "Kunde Stadtwerke",
    "Podcast",
    "Ohne Projekt",
  ]);
  await expect(
    rail.getByRole("button", { name: "Kunde Stadtwerke" }),
  ).toHaveText("KS");
  await expect(rail.getByRole("button", { name: "Geschäftlich" })).toHaveText(
    "Ge",
  );
  // Tooltip mit Name und Anzahl.
  await rail.getByRole("button", { name: "Podcast" }).hover();
  await expect(page.getByRole("tooltip")).toContainText("Podcast");
  await expect(page.getByRole("tooltip")).toContainText("1 Besprechung");

  // Ein Klick waehlt das Projekt und klappt die Spalte wieder auf.
  await rail.getByRole("button", { name: "Kunde Stadtwerke" }).click();
  await expect(page.getByTestId("resize-sessions")).toBeVisible();
  await expect(row(page, "Kunde Stadtwerke")).toHaveAttribute(
    "aria-current",
    "true",
  );
  await expect(meetingRow(page, "m2")).toBeVisible();
});

test("Tastatur: Zeilen sind fokussierbar, Enter wählt, die Menütaste öffnet das Menü", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  await row(page, "Podcast").focus();
  await page.keyboard.press("Enter");
  await expect(row(page, "Podcast")).toHaveAttribute("aria-current", "true");
  await meetingRow(page, "m6").focus();
  await page.keyboard.press("Enter");
  await expect(meetingRow(page, "m6")).toHaveAttribute("aria-current", "true");
  await row(page, "Podcast").focus();
  await page.keyboard.press("ContextMenu");
  await expect(
    page.getByRole("menuitem", { name: "Umbenennen" }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  // Symbole mit Tooltip und Namen.
  for (const name of [
    "Neues Projekt",
    "Alle Besprechungen fragen",
    "Mehr",
    "Filter",
  ]) {
    await expect(
      sessions(page).getByRole("button", { name, exact: true }),
    ).toBeVisible();
  }
});

test("Mehr-Menü: Importieren und Auswählen", async ({ page }) => {
  await openRecordings(page, 1366, 768);
  await sessions(page).getByTestId("projects-more").click();
  await expect(
    page.getByRole("menuitem", { name: "Importieren…" }),
  ).toBeVisible();
  await page.getByRole("menuitem", { name: "Auswählen" }).click();
  await expect(sessions(page).getByText("0 ausgewählt")).toBeVisible();
  await meetingRow(page, "m1").click();
  await expect(sessions(page).getByText("1 ausgewählt")).toBeVisible();
  await expect(meetingRow(page, "m1")).toHaveAttribute("aria-checked", "true");
});

test.describe("Als Nächstes", () => {
  test("ohne Kalender erscheint nichts", async ({ page }) => {
    await openRecordings(page, 1366, 768);
    await expect(page.getByTestId("next-up")).toHaveCount(0);
  });

  test("mit Kalender: nächster Termin, Knopf startet die Aufnahme nach der Einwilligung", async ({
    page,
  }) => {
    await withCalendar(page);
    await page.clock.setFixedTime(NOW);
    await openRecordings(page, 1366, 768);
    const next = page.getByTestId("next-up");
    await expect(next).toBeVisible();
    await expect(
      next.getByRole("heading", { name: "Als Nächstes" }),
    ).toBeVisible();
    await expect(next.getByTestId("next-up-title")).toHaveText(
      "Abstimmung Stadtwerke",
    );
    await expect(next).toContainText("Heute 10:00");
    await expect(next).toContainText("2 Teilnehmende");
    // Fest am Fuss der Spalte, ausserhalb der scrollenden Liste.
    const n = (await next.boundingBox())!;
    const s = (await sessions(page).boundingBox())!;
    expect(n.y + n.height).toBeLessThanOrEqual(s.y + s.height + 1);
    await expect(
      page.getByTestId("projects-scroll").getByTestId("next-up"),
    ).toHaveCount(0);

    await next.getByRole("button", { name: "Termin aufnehmen" }).click();
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    expect(await callsOf(page, "meetings_start_from_event")).toHaveLength(0);
    await dialog
      .getByRole("button", { name: "Alle Beteiligten haben zugestimmt" })
      .click();
    await expect
      .poll(
        async () => (await callsOf(page, "meetings_start_from_event")).length,
      )
      .toBe(1);
    expect(
      (await callsOf(page, "meetings_start_from_event")).at(-1)!.args,
    ).toMatchObject({
      eventKey: "ics:abstimmung:1",
      consentConfirmed: true,
      linkMode: "prompt",
    });
  });
});

for (const [label, size] of Object.entries(VIEWPORTS)) {
  test(`eine Scrollbar: die Projekte-Spalte scrollt für sich (${label})`, async ({
    page,
  }) => {
    await openRecordings(page, size.width, size.height);
    if (size.width < 1000) {
      const open =
        (await page.getByTestId("sessions-expand").count()) > 0
          ? page.getByTestId("sessions-expand")
          : page.getByTestId("sessions-open");
      await open.click();
    }
    await expect(row(page, "Alle Aufnahmen")).toBeVisible();
    const r = await scrollReport(page);
    expect(r.documentScrollHeight).toBeLessThanOrEqual(r.innerHeight + 1);
    expect(r.nested, JSON.stringify(r.scrollers)).toEqual([]);
    // Die Kopfzeile mit Suche bleibt stehen, nur die Liste scrollt.
    const search = page.getByRole("searchbox", {
      name: "Besprechungen durchsuchen",
    });
    await expect(search).toBeInViewport();
    const list = page.getByTestId("projects-scroll");
    const before = (await search.boundingBox())!.y;
    await list.evaluate((el) => el.scrollTo(0, el.scrollHeight));
    expect((await search.boundingBox())!.y).toBe(before);
  });
}

// Bilder der Projekte-Spalte (nur mit LVA_SCREENSHOTS, kein Verhaltenstest).
test.describe("Bilder", () => {
  const DIR = process.env.LVA_SCREENSHOTS;
  test.skip(!DIR, "nur mit LVA_SCREENSHOTS");
  const shoot = async (page: Page, name: string) => {
    await page.waitForTimeout(250);
    await page.screenshot({
      path: path.join(DIR!, `nach-m3-${name}.png`),
      animations: "disabled",
    });
  };

  test("Liste, Kontextmenü, Ziehen, Filter, Leiste, Schublade", async ({
    page,
  }) => {
    await withCalendar(page);
    await page.clock.setFixedTime(NOW);
    await openRecordings(page, 1366, 768);
    await row(page, "Privat").click();
    await shoot(page, "liste-1366");

    await row(page, "Geschäftlich").click({ button: "right" });
    await shoot(page, "menue-1366");
    await page.keyboard.press("Escape");

    await dragTo(page, meetingRow(page, "m1"), row(page, "Podcast"), {
      drop: false,
    });
    await shoot(page, "ziehen-1366");
    await page.keyboard.press("Escape");
    await page.mouse.up();

    await sessions(page).getByTestId("projects-filter").click();
    await page
      .getByTestId("projects-filter-popover")
      .getByRole("button", { name: "30 Tage", exact: true })
      .click();
    await shoot(page, "filter-1366");
    await page.keyboard.press("Escape");

    await page.getByTestId("sessions-collapse").click();
    await shoot(page, "leiste-1366");

    await page.setViewportSize({ width: 480, height: 800 });
    await page.getByTestId("sessions-open").click();
    await shoot(page, "schublade-480");
    fs.writeFileSync(
      path.join(DIR!, "nach-m3-messwerte.json"),
      JSON.stringify(await scrollReport(page), null, 2),
    );
  });
});
