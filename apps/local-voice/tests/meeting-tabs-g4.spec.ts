import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import * as path from "node:path";
import { formatMeetingDate } from "../src/lib/meetingDate";
import { centerFromLegacy, lowerFromLegacy } from "../src/lib/meetingTabs";
import { PEOPLE, installLiveMock } from "./recLiveMock";
import {
  TITLE_M2,
  calls,
  emitMeeting,
  openRecordings,
  pickMeeting,
} from "./recLayoutMock";

// G4 (Goal Issues-Abschluss #70): die Seite Aufnahmen neu verteilt.
//  1. Reiter: Mitte = Transkript + Protokoll, rechts unter der Bedienung =
//     Notizen + KI-Notizen + Fragen; live: Live-Transkript in der Mitte,
//     Notizblock rechts; Quellsprung markiert den Satz ohne Reiterwechsel rechts.
//  2. Kopf: Chip "N Teilnehmende: ..." mit Popover zum Bearbeiten.
//  3. Datum mit Jahr, in Kopf und Liste dieselbe Formatierfunktion.
//  4. Protokoll und KI-Notizen kompakt: Werkzeugzeile, Vorlage/Pfad im Menue
//     und im Info-Dialog, Inhalt direkt unter den Reitern.

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

const participant = (id: string, name: string, over = {}) => ({
  human_id: id,
  name,
  email: null,
  company: null,
  role: "attendee",
  source: "calendar",
  is_self: false,
  meeting_count: 2,
  ...over,
});

const flags = {
  unsupported: false,
  dropped_sources: 0,
  placed_by_fallback: false,
  edited: false,
};
const entry = (id: string, text: string, sources: number[]) => ({
  id,
  origin: "ai",
  text,
  note_id: null,
  source_segment_ids: sources,
  assignee: null,
  due: null,
  flags,
});
const NOTES_BODY = {
  format: "enhanced@1",
  template_id: "builtin:vertrieb",
  template_title: "Kundengespräch / Vertrieb",
  segment_epoch: 1,
  sections: [
    {
      id: "kernpunkte",
      title: "Kernpunkte",
      kind: "text",
      entries: [
        entry("E1", "Die Spitzen liegen montags morgens", [45]),
        entry("E2", "Ein Speicher glättet ein Drittel", [5, 6]),
      ],
    },
  ],
  stats: {
    user_notes_total: 0,
    user_notes_by_model: 0,
    user_notes_by_fallback: 0,
    ai_entries: 2,
    ai_entries_sourced: 2,
    dropped_source_ids: 0,
    chunks_total: 1,
    chunks_failed: [],
    single_pass: true,
  },
};

interface Setup {
  recording?: boolean;
  /** Teilnehmende von m2. */
  participants?: ReturnType<typeof participant>[];
  speakers?: boolean;
  /** Anfangswerte im Speicher der Seite (alte und neue Schluessel). */
  storage?: Record<string, string>;
}

/** Attrappe samt Besprechung m2 mit KI-Notizen, Protokoll, Ablagepfad und Vorlage. */
const setup = async (page: Page, opts: Setup = {}) => {
  await installLiveMock(page, { recording: opts.recording });
  await page.addInitScript(
    ({ people, participants, speakers, notes, storage }) => {
      const w = window as any;
      w.__people = people;
      w.__participants = { m2: participants };
      w.__speakers = speakers
        ? {
            m2: [1, 2].map((i) => ({
              channel: 1,
              speaker_index: i,
              label: `Gegenseite ${i}`,
              display_name: null,
              human_id: null,
              share_pct: 40,
            })),
          }
        : {};
      w.__documents.push({
        id: "d1",
        meeting_id: "m2",
        kind: "enhanced_notes",
        body_format: "enhanced@1",
        body: JSON.stringify(notes),
        version: 1,
        created_at: 1790000700,
        template_id: "builtin:vertrieb",
        updated_at: 100,
      });
      // Ohne Aufgabenliste: deren Kontrollkaesten (MarkdownContent) tragen kein
      // Label (axe `label`, vorbestehend, ausserhalb dieses Pakets).
      const minutes = w.__documents.find((d: any) => d.id === "p1");
      minutes.body = [
        "# Protokoll: Kundentermin Stadtwerke",
        "",
        "## Ergebnisse",
        "- Lastspitzen liegen werktags zwischen 07:30 und 09:00 Uhr.",
        "- Ein Speicher mit 400 kWh kappt rund ein Drittel der Spitzen.",
      ].join("\n");
      w.__minutesFile = "C:/Users/patrick/Protokolle/SWK-Lastgang.docx";
      w.__minutesMeta = {
        document_id: "p1",
        template_id: "builtin:vertrieb",
        template_title: "Kundengespräch / Vertrieb",
        auto: null,
        incomplete: false,
        gaps: [],
        chunks_total: 1,
        chunks_split: 0,
      };
      // Anfangswerte nur beim ersten Laden (ein Neuladen soll sie nicht zuruecksetzen).
      if (!sessionStorage.getItem("__g4_seeded")) {
        sessionStorage.setItem("__g4_seeded", "1");
        for (const [key, value] of Object.entries(storage)) {
          localStorage.setItem(key, value);
        }
      }
    },
    {
      people: PEOPLE,
      participants: opts.participants ?? [
        participant("h-anna", "Anna Berg", { role: "organizer" }),
        participant("h-ben", "Ben Koch"),
      ],
      speakers: opts.speakers ?? false,
      notes: NOTES_BODY,
      storage: opts.storage ?? {},
    },
  );
};

const content = (page: Page) => page.getByTestId("rec-content");
const lower = (page: Page) => page.getByTestId("rec-controls");
const centerTab = (page: Page, name: string) =>
  content(page).getByRole("tab", { name, exact: true });
const lowerTab = (page: Page, name: string) =>
  lower(page).getByRole("tab", { name, exact: true });
const head = (page: Page) => page.getByTestId("rec-detail-head");

const openM2 = async (page: Page, width = 1366, height = 768) => {
  await openRecordings(page, width, height);
  await pickMeeting(page, "m2");
  await expect(page.locator('[data-segment-index="0"]')).toBeVisible();
};

const chooseMenu = async (page: Page, testId: string) => {
  await page.getByTestId("meeting-menu").click();
  await expect(page.getByRole("menu")).toBeVisible();
  await page.getByTestId(testId).evaluate((el) => (el as HTMLElement).click());
};

// ---------------------------------------------------------------------------
// Reine Logik: Datum und Migration der Reiter
// ---------------------------------------------------------------------------

test.describe("G4 Logik", () => {
  test("Datum: mit Jahr, Kopf mit Wochentag, Liste kompakt, de und en", () => {
    const date = new Date(2026, 8, 30, 22, 54);
    expect(formatMeetingDate(date, "de", "full")).toBe("Mi 30.09.2026, 22:54");
    expect(formatMeetingDate(date, "de", "compact")).toBe(
      "30.09.2026 \u00b7 22:54",
    );
    expect(formatMeetingDate(date, "en", "full")).toMatch(
      /^Wed 09\/30\/2026, 10:54\sPM$/,
    );
    expect(formatMeetingDate(date, "en", "compact")).toMatch(
      /^09\/30\/2026 \u00b7 10:54\sPM$/,
    );
  });

  test("Migration: alte Reiter der Mitte und rechts ergeben die neuen", () => {
    expect(centerFromLegacy(null)).toBe("transcript");
    expect(centerFromLegacy("notes")).toBe("transcript");
    expect(centerFromLegacy("ai")).toBe("transcript");
    expect(centerFromLegacy("minutes")).toBe("minutes");
    expect(centerFromLegacy("compare")).toBe("compare");
    expect(lowerFromLegacy(null, null)).toBe("notes");
    expect(lowerFromLegacy("ai", "transcript")).toBe("ai");
    expect(lowerFromLegacy("minutes", "transcript")).toBe("notes");
    expect(lowerFromLegacy("ai", "chat")).toBe("chat");
  });
});

// ---------------------------------------------------------------------------
// 1. Reiterverteilung
// ---------------------------------------------------------------------------

test.describe("Reiterverteilung", () => {
  for (const [width, height] of [
    [1366, 768],
    [1920, 1050],
  ] as const) {
    test(`breit (${width}): Mitte Transkript + Protokoll, rechts Notizen + KI-Notizen + Fragen`, async ({
      page,
    }) => {
      await setup(page);
      await openM2(page, width, height);
      const centerTabs = content(page).getByRole("tab");
      await expect(centerTabs).toHaveText(["Transkript", "Protokoll"]);
      await expect(centerTab(page, "Transkript")).toHaveAttribute(
        "aria-selected",
        "true",
      );
      const lowerTabs = lower(page).getByRole("tab");
      await expect(lowerTabs).toHaveText(["Notizen", "KI-Notizen", "Fragen"]);
      await expect(lowerTab(page, "Notizen")).toHaveAttribute(
        "aria-selected",
        "true",
      );
      // Das Transkript steht in der Mitte, der Notizblock rechts unter der Bedienung.
      await expect(
        content(page).locator('[data-segment-index="0"]'),
      ).toBeVisible();
      await expect(lower(page).getByTestId("my-notes")).toBeVisible();
      const c = (await content(page).boundingBox())!;
      const r = (await lower(page).boundingBox())!;
      expect(c.x + c.width).toBeLessThanOrEqual(r.x + 1);
      // Die Bedienung (Aufnahmekarte, Symbolzeile) steht ueber den Reitern.
      const actions = (await page.getByTestId("rec-actions").boundingBox())!;
      const tabs = (await lower(page).getByRole("tablist").boundingBox())!;
      expect(actions.y + actions.height).toBeLessThanOrEqual(tabs.y + 1);

      // Reiterwechsel: jede Seite wechselt fuer sich.
      await centerTab(page, "Protokoll").click();
      await expect(content(page).getByTestId("minutes-doc")).toBeVisible();
      await expect(lower(page).getByTestId("my-notes")).toBeVisible();
      await lowerTab(page, "KI-Notizen").click();
      await expect(lower(page).getByTestId("enhanced-notes")).toBeVisible();
      await expect(content(page).getByTestId("minutes-doc")).toBeVisible();
      await lowerTab(page, "Fragen").click();
      await expect(page.getByTestId("chat-panel")).toBeVisible();
      await expect(lower(page).getByTestId("enhanced-notes")).toBeHidden();
    });
  }

  test("Fenster 480: oben Transkript/Protokoll, unten Notizen/KI-Notizen/Fragen mit Griff", async ({
    page,
  }) => {
    await setup(page);
    await openM2(page, 480, 800);
    await expect(content(page).getByRole("tab")).toHaveText([
      "Transkript",
      "Protokoll",
    ]);
    await expect(lower(page).getByRole("tab")).toHaveText([
      "Notizen",
      "KI-Notizen",
      "Fragen",
    ]);
    const c = (await content(page).boundingBox())!;
    const r = (await lower(page).boundingBox())!;
    expect(r.y).toBeGreaterThanOrEqual(c.y + c.height - 1);
    await expect(page.getByTestId("resize-split")).toBeVisible();
    await expect(
      content(page).locator('[data-segment-index="0"]'),
    ).toBeVisible();
    await expect(lower(page).getByTestId("my-notes")).toBeVisible();
    await lowerTab(page, "KI-Notizen").click();
    await expect(lower(page).getByTestId("enhanced-notes")).toBeVisible();
  });

  test("live: Live-Transkript in der Mitte, Notizblock rechts; Enter speichert mit Zeitstempel", async ({
    page,
  }) => {
    await setup(page, { recording: true });
    await openRecordings(page, 1366, 768);
    const pad = lower(page).getByTestId("live-notes-pad");
    await expect(pad).toBeVisible();
    // Der Fokus steht im Notizfeld.
    await expect(page.getByTestId("note-starter")).toBeFocused();
    await emitMeeting(page, {
      kind: "state",
      meeting_id: "m1",
      status: "recording",
      paused: false,
    });
    await page.waitForTimeout(200);
    await emitMeeting(page, {
      kind: "segments",
      meeting_id: "m1",
      appended: [0, 1, 2].map((i) => ({
        segment_index: i,
        text: `Live-Satz ${i}: Die Lastspitzen liegen montags.`,
        start_ms: i * 5000,
        end_ms: i * 5000 + 4000,
        channel: i % 2,
        speaker_index: null,
      })),
    });
    await expect(
      content(page).getByText("Live-Satz 2", { exact: false }),
    ).toBeVisible();
    await expect(centerTab(page, "Transkript")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(lowerTab(page, "Notizen")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await page.keyboard.type("Budget bis Freitag freigegeben");
    await page.keyboard.press("Enter");
    await expect
      .poll(async () => (await calls(page, "meeting_notes_save")).length)
      .toBeGreaterThan(0);
    const saves = await calls(page, "meeting_notes_save");
    const last = saves[saves.length - 1].args as any;
    expect(last.blocks[0]).toMatchObject({
      text: "Budget bis Freitag freigegeben",
      at_ms: 754_000,
    });
    await shoot(page, "g4-live-1366");
  });

  test("live im Fenster 480: Transkript oben, Notizblock darunter", async ({
    page,
  }) => {
    await setup(page, { recording: true });
    await openRecordings(page, 480, 800);
    await expect(lower(page).getByTestId("live-notes-pad")).toBeVisible();
    await emitMeeting(page, {
      kind: "state",
      meeting_id: "m1",
      status: "recording",
      paused: false,
    });
    await page.waitForTimeout(200);
    await emitMeeting(page, {
      kind: "segments",
      meeting_id: "m1",
      appended: [0, 1, 2, 3].map((i) => ({
        segment_index: i,
        text: `Live-Satz ${i}: Die Lastspitzen liegen montags.`,
        start_ms: i * 5000,
        end_ms: i * 5000 + 4000,
        channel: i % 2,
        speaker_index: null,
      })),
    });
    await expect(content(page).getByText("Live-Satz 3")).toBeVisible();
    const sentence = (await content(page)
      .getByText("Live-Satz 3")
      .boundingBox())!;
    const field = (await page.getByTestId("note-starter").boundingBox())!;
    expect(sentence.y).toBeLessThan(field.y);
    await shoot(page, "g4-live-480");
  });

  test("Quellsprung aus den KI-Notizen markiert den Satz im Transkript, ohne Reiterwechsel rechts", async ({
    page,
  }) => {
    await setup(page);
    await openM2(page);
    await lowerTab(page, "KI-Notizen").click();
    const view = lower(page).getByTestId("enhanced-notes");
    await expect(view).toBeVisible();
    const chip = view.locator(
      '[data-testid=enhanced-entry][data-entry-id="E1"] [data-source-id="45"]',
    );
    await chip.click();
    const row = content(page).locator('[data-segment-index="45"]');
    await expect(row).toHaveAttribute("data-highlighted", "true");
    await expect(row).toBeInViewport();
    // Rechts bleibt es bei den KI-Notizen, links beim Transkript.
    await expect(lowerTab(page, "KI-Notizen")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(view).toBeVisible();
    await expect(centerTab(page, "Transkript")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    // Die Markierung verschwindet nach kurzer Zeit wieder.
    await expect(row).not.toHaveAttribute("data-highlighted", "true", {
      timeout: 4000,
    });
  });

  test("Quellsprung bei geoeffnetem Protokoll zeigt das Transkript, die Notizen rechts bleiben", async ({
    page,
  }) => {
    await setup(page);
    await openM2(page);
    await lowerTab(page, "KI-Notizen").click();
    await centerTab(page, "Protokoll").click();
    await expect(content(page).getByTestId("minutes-doc")).toBeVisible();
    await lower(page)
      .locator('[data-entry-id="E1"] [data-source-id="45"]')
      .click();
    await expect(centerTab(page, "Transkript")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(
      content(page).locator('[data-segment-index="45"]'),
    ).toHaveAttribute("data-highlighted", "true");
    await expect(lowerTab(page, "KI-Notizen")).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  test("Zeitmarken im Transkript sprechen weiter den Player an", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      const w = window as any;
      w.__played = [];
      const times = new WeakMap<object, number>();
      Object.defineProperty(HTMLMediaElement.prototype, "currentTime", {
        configurable: true,
        get() {
          return times.get(this) ?? 0;
        },
        set(v: number) {
          times.set(this, v);
        },
      });
      HTMLMediaElement.prototype.play = function () {
        w.__played.push(times.get(this) ?? 0);
        return Promise.resolve();
      };
      HTMLMediaElement.prototype.pause = function () {};
    });
    await setup(page);
    await openM2(page);
    await content(page)
      .locator('[data-segment-index="1"] [data-act="seek"]')
      .click();
    await expect
      .poll(() => page.evaluate(() => (window as any).__played))
      .toEqual([33]);
  });

  test("Persistenz: neue Schluessel, ueberstehen Neuladen", async ({
    page,
  }) => {
    await setup(page);
    await openM2(page);
    await centerTab(page, "Protokoll").click();
    await lowerTab(page, "KI-Notizen").click();
    const stored = () =>
      page.evaluate(() => ({
        center: localStorage.getItem("lva.ui.meetings.centerTab"),
        lower: localStorage.getItem("lva.ui.meetings.lowerTab"),
      }));
    await expect.poll(stored).toEqual({ center: "minutes", lower: "ai" });
    await page.reload();
    await page.getByTestId("rec-content").waitFor();
    await expect(centerTab(page, "Protokoll")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(lowerTab(page, "KI-Notizen")).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  for (const [name, storage, center, lowerName] of [
    [
      "Protokoll",
      { "lva.ui.meetings.midTab": "minutes" },
      "Protokoll",
      "Notizen",
    ],
    [
      "KI-Notizen",
      { "lva.ui.meetings.midTab": "ai" },
      "Transkript",
      "KI-Notizen",
    ],
    ["Fragen", { "lva.ui.meetings.rightTab": "chat" }, "Transkript", "Fragen"],
  ] as const) {
    test(`Migration: alter Reiter "${name}" wird sinngemaess uebernommen`, async ({
      page,
    }) => {
      await setup(page, { storage });
      await openRecordings(page, 1366, 768);
      await pickMeeting(page, "m2");
      await expect(centerTab(page, center)).toHaveAttribute(
        "aria-selected",
        "true",
      );
      await expect(lowerTab(page, lowerName)).toHaveAttribute(
        "aria-selected",
        "true",
      );
    });
  }
});

// ---------------------------------------------------------------------------
// 2. Teilnehmende im Kopf
// ---------------------------------------------------------------------------

test.describe("Teilnehmende", () => {
  test("Chip nennt Anzahl und Namen, gekuerzt; der Tooltip nennt alle", async ({
    page,
  }) => {
    await setup(page, {
      participants: [
        participant("h-anna", "Anna Berg", { role: "organizer" }),
        participant("h-ben", "Ben Koch"),
        participant("h-cem", "Cem Aydin"),
        participant("h-d", "Dora Lang"),
        participant("h-e", "Emil Roth"),
      ],
    });
    await openM2(page);
    const chip = page.getByTestId("participants-chip");
    await expect(chip).toHaveText(
      /^5 Teilnehmende:\s*Anna Berg, Ben Koch, Cem Aydin, \+2$/,
    );
    await chip.hover();
    const tip = page.getByRole("tooltip");
    await expect(tip).toContainText(
      "Anna Berg, Ben Koch, Cem Aydin, Dora Lang, Emil Roth",
    );
    // Der Kopf bleibt hoechstens 120 px hoch.
    expect((await head(page).boundingBox())!.height).toBeLessThanOrEqual(120);
  });

  test("Popover: Person entfernen speichert ueber meetings_update_metadata", async ({
    page,
  }) => {
    await setup(page);
    await openM2(page);
    await page.getByTestId("participants-chip").click();
    const rows = page.getByTestId("participant-row");
    await expect(rows).toHaveCount(2);
    await rows
      .filter({ hasText: "Ben Koch" })
      .getByTestId("participant-remove")
      .click();
    await expect(rows).toHaveCount(1);
    const saved = await calls(page, "meetings_update_metadata");
    expect(saved).toHaveLength(1);
    expect(saved[0].args).toEqual({
      meetingId: "m2",
      edit: {
        title: null,
        description: null,
        started_at: null,
        participant_ids: ["h-anna"],
        folder_ids: null,
      },
    });
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("participants-chip")).toHaveText(
      /^1 Teilnehmende:\s*Anna Berg$/,
    );
  });

  test("Popover: Person hinzufuegen mit Autovervollstaendigung aus den vorhandenen Personen", async ({
    page,
  }) => {
    await setup(page);
    await openM2(page);
    await page.getByTestId("participants-chip").click();
    const input = page.getByTestId("participant-add-input");
    await input.fill("cem");
    await page
      .getByTestId("person-match")
      .filter({ hasText: "Cem Aydin" })
      .click();
    await expect(input).toHaveValue("Cem Aydin");
    await page.getByTestId("participant-add").click();
    await expect(page.getByTestId("participant-row")).toHaveCount(3);
    const saved = await calls(page, "meetings_update_metadata");
    expect((saved[0].args as any).edit.participant_ids).toEqual([
      "h-anna",
      "h-ben",
      "h-cem",
    ]);
    // Eine schon eingetragene Person wird nicht noch einmal angeboten.
    await input.fill("Anna");
    await expect(page.getByTestId("person-match")).toHaveCount(0);
  });

  test("Popover: ein unbekannter Name braucht einen erkannten Sprecher; dann wird der Sprecher benannt", async ({
    page,
  }) => {
    await setup(page, { speakers: true });
    await openM2(page);
    await page.getByTestId("participants-chip").click();
    const input = page.getByTestId("participant-add-input");
    await input.fill("Dirk Voss");
    await expect(page.getByTestId("participant-unknown")).toBeVisible();
    await expect(page.getByTestId("participant-add")).toBeDisabled();
    // Mit einem Sprecher verknuepft: der Name benennt ihn, die Person entsteht.
    await page
      .getByTestId("participant-speaker")
      .filter({ hasText: "Gegenseite 2" })
      .click();
    await expect(page.getByTestId("participant-add")).toBeEnabled();
    await page.getByTestId("participant-add").click();
    const renamed = await calls(page, "meeting_speaker_rename");
    expect(renamed).toHaveLength(1);
    expect(renamed[0].args).toEqual({
      meetingId: "m2",
      channel: 1,
      speakerIndex: 2,
      name: "Dirk Voss",
    });
    await expect(page.getByTestId("participant-row")).toHaveCount(3);
    await expect(
      page.getByTestId("participant-row").filter({ hasText: "Dirk Voss" }),
    ).toBeVisible();
  });

  test("Popover: Schliessen mit Escape bringt den Fokus zurueck, Klick daneben schliesst", async ({
    page,
  }) => {
    await setup(page);
    await openM2(page);
    const chip = page.getByTestId("participants-chip");
    await chip.click();
    await expect(page.getByTestId("participants-popover")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("participants-popover")).toHaveCount(0);
    await expect(chip).toBeFocused();
    await chip.click();
    await page.getByTestId("meeting-title").click({ position: { x: 4, y: 4 } });
    await expect(page.getByTestId("participants-popover")).toHaveCount(0);
  });
});

// ---------------------------------------------------------------------------
// 3. Datum mit Jahr
// ---------------------------------------------------------------------------

test("Datum: mit Jahr, im Kopf mit Wochentag, in der Liste kompakt", async ({
  page,
}) => {
  await setup(page);
  await openM2(page);
  // m2 beginnt am 25.09.2026 um 16:13 (Europe/Berlin).
  await expect(page.getByTestId("date-chip")).toHaveText(
    "Fr 25.09.2026, 16:13",
  );
  await expect(page.locator('[data-meeting-id="m2"]')).toContainText(
    "25.09.2026 \u00b7 16:13",
  );
  // Jede Zeile der Liste traegt das Jahr.
  for (const id of ["m1", "m4", "m5"]) {
    await expect(page.locator(`[data-meeting-id="${id}"]`)).toContainText(
      /\d{2}\.\d{2}\.2026 \u00b7 \d{2}:\d{2}/,
    );
  }
  // Der Info-Dialog nutzt dieselbe Form.
  await page.getByTestId("meeting-details-open").click();
  await expect(page.getByTestId("details-started")).toHaveText(
    "Fr 25.09.2026, 16:13",
  );
});

// ---------------------------------------------------------------------------
// 4. Protokoll und KI-Notizen kompakt
// ---------------------------------------------------------------------------

test.describe("Protokoll und KI-Notizen kompakt", () => {
  test("Protokoll: Werkzeugzeile mit Symbolen, keine Vorlagenwahl und keine Pfadzeilen im Reiter", async ({
    page,
  }) => {
    await setup(page);
    await openM2(page);
    await centerTab(page, "Protokoll").click();
    const panel = page.getByTestId("mid-panel-minutes");
    const toolbar = panel.getByTestId("minutes-toolbar");
    await expect(toolbar).toBeVisible();
    for (const [id, name] of [
      ["minutes-generate", "Neu erzeugen"],
      ["minutes-copy", "Protokoll kopieren"],
      ["minutes-export", "Herunterladen"],
    ] as const) {
      const button = toolbar.getByTestId(id);
      await expect(button).toHaveAttribute("aria-label", name);
      const b = (await button.boundingBox())!;
      expect([Math.round(b.width), Math.round(b.height)]).toEqual([28, 28]);
    }
    // Tooltip: Name und Kurzerklaerung.
    await toolbar.getByTestId("minutes-copy").hover();
    const tip = page.getByRole("tooltip");
    await expect(tip.locator("strong")).toHaveText("Protokoll kopieren");
    await expect(tip.locator("div.text-xs")).not.toHaveText("");
    // Was die Hoehe kostete, ist weg.
    await expect(panel.locator(".app-select__control")).toHaveCount(0);
    await expect(panel.getByText("Vorlagen verwalten")).toHaveCount(0);
    await expect(panel.getByText("Automatisch abgelegt unter")).toHaveCount(0);
    await expect(panel.getByText("Erzeugt mit der Vorlage")).toHaveCount(0);
    await shoot(page, "g4-protokoll-1366");
  });

  test("KI-Notizen: Werkzeugzeile mit Symbolen, die Vorlage steht nicht mehr im Reiter", async ({
    page,
  }) => {
    await setup(page);
    await openM2(page);
    await lowerTab(page, "KI-Notizen").click();
    const view = lower(page).getByTestId("enhanced-notes");
    await expect(view).toBeVisible();
    const toolbar = view.getByTestId("enhanced-toolbar");
    await expect(toolbar.getByTestId("enhanced-generate")).toHaveAttribute(
      "aria-label",
      "KI-Notizen neu erzeugen",
    );
    await expect(toolbar.getByTestId("enhanced-export")).toHaveAttribute(
      "aria-label",
      "Herunterladen",
    );
    await expect(view).not.toContainText("Kundengespräch / Vertrieb");
  });

  test("Menue: Vorlage waehlen, Vorlagen verwalten, Neu erzeugen mit Vorlage", async ({
    page,
  }) => {
    await setup(page);
    await openM2(page);
    // Das Menue steht im Kopf neben dem Info-Symbol.
    const menu = head(page).getByTestId("meeting-menu");
    const info = head(page).getByTestId("meeting-details-open");
    expect((await menu.boundingBox())!.x).toBeGreaterThan(
      (await info.boundingBox())!.x,
    );
    await page.getByTestId("meeting-menu").click();
    for (const name of [
      "Neu erzeugen mit Vorlage …",
      "Vorlage wählen …",
      "Vorlagen verwalten …",
    ]) {
      await expect(page.getByRole("menuitem", { name })).toBeVisible();
    }
    await page.keyboard.press("Escape");

    // Vorlagen verwalten: die Verwaltung der Vorlagen.
    await chooseMenu(page, "menu-template-manage");
    await expect(page.getByRole("dialog", { name: "Vorlagen" })).toBeVisible();
    await page.keyboard.press("Escape");

    // Neu erzeugen mit Vorlage: Auswahl, dann Protokoll oder KI-Notizen.
    await chooseMenu(page, "menu-regen-template");
    const dialog = page.getByRole("dialog", {
      name: "Neu erzeugen mit Vorlage",
    });
    await expect(dialog).toBeVisible();
    // G5: unter der Vorlage stehen jetzt auch Grundlage und Sprache; die Vorlage ist die erste Liste.
    await dialog.locator(".app-select__control").first().click();
    await page
      .getByRole("option", { name: "Kundengespräch / Vertrieb", exact: true })
      .click();
    await expect
      .poll(async () => (await calls(page, "meetings_set_template")).length)
      .toBe(1);
    await dialog.getByTestId("regen-minutes-go").click();
    await expect
      .poll(async () => (await calls(page, "meetings_generate_minutes")).length)
      .toBe(1);
    expect((await calls(page, "meetings_generate_minutes"))[0].args).toEqual({
      meetingId: "m2",
      templateId: null,
      // G5: Grundlage und Sprache aus dem Dialog (Standard: aktive Fassung, Sprache der App)
      basis: { variant_id: null, output_language: "de" },
    });
    await expect(centerTab(page, "Protokoll")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(page.getByTestId("template-chip")).toHaveText(
      "Vorlage: Kundengespräch / Vertrieb",
    );
  });

  test("Info-Dialog: Ablagepfad, Vorlage und Herkunft des Protokolls", async ({
    page,
  }) => {
    await setup(page);
    await openM2(page);
    await page.getByTestId("meeting-details-open").click();
    const dialog = page.getByRole("dialog", { name: "Details" });
    await expect(dialog.getByTestId("details-minutes-file")).toContainText(
      "C:/Users/patrick/Protokolle/SWK-Lastgang.docx",
    );
    await expect(dialog.getByTestId("details-minutes-origin")).toHaveText(
      "Erzeugt mit der Vorlage: Kundengespräch / Vertrieb",
    );
    await expect(dialog.getByTestId("details-template")).toContainText(
      "Kundengespräch / Vertrieb",
    );
  });

  test("Kopf: der Chip Vorlage zeigt die Wahl, ein Klick oeffnet die Vorlagenwahl", async ({
    page,
  }) => {
    await setup(page);
    await openM2(page);
    const chip = page.getByTestId("template-chip");
    await expect(chip).toHaveText("Vorlage: Kundengespräch / Vertrieb");
    await chip.click();
    await expect(page.getByTestId("template-dialog")).toBeVisible();
  });

  test("Fortschritt der Erzeugung bleibt sichtbar", async ({ page }) => {
    await setup(page);
    await openM2(page);
    await centerTab(page, "Protokoll").click();
    await emitMeeting(page, {
      kind: "progress",
      meeting_id: "m2",
      phase: "notes",
      done: 1,
      total: 4,
      elapsed_ms: 1000,
      eta_ms: 3000,
      state: "running",
      pausable: false,
    });
    await expect(page.getByTestId("status-chip")).toHaveAttribute(
      "data-state",
      "processing",
    );
    await expect(page.getByTestId("job-panel")).toBeVisible();
  });

  for (const [width, height] of [[1366, 768]] as const) {
    test(`Abstand: Reiter-Unterkante bis erster Inhalt <= 48 px (${width}x${height})`, async ({
      page,
    }) => {
      await setup(page);
      await openM2(page, width, height);
      // Protokoll: Mitte.
      await centerTab(page, "Protokoll").click();
      const tabs = (await head(page).getByRole("tablist").boundingBox())!;
      const doc = (await page.getByTestId("minutes-doc").boundingBox())!;
      const gapMinutes = doc.y - (tabs.y + tabs.height);
      expect(gapMinutes, "Protokoll").toBeGreaterThan(0);
      expect(gapMinutes, "Protokoll").toBeLessThanOrEqual(48);
      // KI-Notizen: rechts.
      await lowerTab(page, "KI-Notizen").click();
      const lowerTabs = (await lower(page).getByRole("tablist").boundingBox())!;
      const card = (await lower(page)
        .getByTestId("enhanced-notes")
        .locator("> div.rounded-lg")
        .first()
        .boundingBox())!;
      const gapAi = card.y - (lowerTabs.y + lowerTabs.height);
      expect(gapAi, "KI-Notizen").toBeGreaterThan(0);
      expect(gapAi, "KI-Notizen").toBeLessThanOrEqual(48);
    });
  }

  test("axe: Detail mit Protokoll, KI-Notizen und geoeffnetem Teilnehmenden-Popover", async ({
    page,
  }) => {
    await setup(page, { speakers: true });
    await openM2(page);
    await centerTab(page, "Protokoll").click();
    await lowerTab(page, "KI-Notizen").click();
    await page.getByTestId("participants-chip").click();
    await expect(page.getByTestId("participants-popover")).toBeVisible();
    const result = await new AxeBuilder({ page })
      .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "best-practice"])
      .analyze();
    const severe = result.violations
      .filter((v) => v.impact === "critical" || v.impact === "serious")
      .map(
        (v) =>
          `${v.id} (${v.impact}): ${v.nodes
            .slice(0, 3)
            .map((n) => n.target.join(" "))
            .join(" | ")}`,
      );
    expect(severe).toEqual([]);
  });
});

// ---------------------------------------------------------------------------
// Bilder (nur mit LVA_SCREENSHOTS=1)
// ---------------------------------------------------------------------------

test("Bild: Detail bei 1366 x 768", async ({ page }) => {
  test.skip(!process.env.LVA_SCREENSHOTS, "Bilder nur mit LVA_SCREENSHOTS=1");
  await setup(page, { speakers: true });
  await openM2(page, 1366, 768);
  await expect(content(page).getByText(TITLE_M2)).toBeVisible();
  await shoot(page, "g4-detail-1366");
});
