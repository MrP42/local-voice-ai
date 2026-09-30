import { test, expect, type Page } from "@playwright/test";
import {
  PROGRESS,
  TITLE_M2,
  VIEWPORTS,
  emitMeeting,
  goToRecordings,
  installRecMock,
  openRecordings,
  pickMeeting,
  scrollReport,
} from "./recLayoutMock";

// Aufnahmen-Oberflaeche (Goal aufnahmen-ui, M2): Projekte | Arbeitsflaeche |
// Bedienung und Transkript. AK2 (drei Bereiche, Griffe, Klappen, Neuladen),
// AK4 (eine Scrollbar je Bereich, vier Fenstergroessen), AK8 (Breiten,
// Klappzustand, Auswahl, Reiter ueberstehen Neuladen und Seitenwechsel).

test.beforeEach(async ({ page }) => {
  await installRecMock(page);
});

const box = async (page: Page, testId: string) => {
  const b = await page.getByTestId(testId).boundingBox();
  expect(b, testId).not.toBeNull();
  return b!;
};

/** Senkrechten Ziehgriff mit der Maus um `dx` Pixel verschieben. */
async function drag(page: Page, testId: string, dx: number) {
  const b = await box(page, testId);
  const x = b.x + b.width / 2;
  const y = b.y + b.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + dx / 2, y, { steps: 4 });
  await page.mouse.move(x + dx, y, { steps: 4 });
  await page.mouse.up();
}

const navTo = (page: Page, name: string) =>
  page
    .getByRole("navigation")
    .getByRole("button", { name, exact: true })
    .click();

test("AK2: three areas side by side, projects left, controls right", async ({
  page,
}) => {
  await openRecordings(page, 1920, 1050);
  const sessions = await box(page, "rec-sessions");
  const content = await box(page, "rec-content");
  const controls = await box(page, "rec-controls");
  expect(sessions.x + sessions.width).toBeLessThanOrEqual(content.x + 1);
  expect(content.x + content.width).toBeLessThanOrEqual(controls.x + 1);
  expect(sessions.width).toBeCloseTo(248, 0);
  expect(controls.width).toBeCloseTo(420, 0);
  // Die Arbeitsflaeche nimmt den Rest.
  expect(content.width).toBeGreaterThan(900);
  await expect(page.getByTestId("rec-sessions")).toContainText("Besprechungen");
  await expect(
    page.getByTestId("rec-sessions").getByRole("heading", { name: "Projekte" }),
  ).toBeVisible();
  // Die Aufnahmekarte sitzt in der Bedienung, darunter die Reiter.
  await expect(
    page
      .getByTestId("rec-controls")
      .getByRole("button", { name: /Aufnahme starten/ }),
  ).toBeVisible();
  await expect(
    page.getByTestId("rec-controls").getByRole("tab", { name: "Transkript" }),
  ).toHaveAttribute("aria-selected", "true");
  await expect(
    page.getByTestId("rec-controls").getByRole("tab", { name: "Fragen" }),
  ).toBeVisible();
});

test("AK2: selecting a recording fills the workspace and the right column", async ({
  page,
}) => {
  await openRecordings(page, 1920, 1050);
  await pickMeeting(page, "m2");
  const content = page.getByTestId("rec-content");
  await expect(content.getByText(TITLE_M2, { exact: true })).toBeVisible();
  for (const name of ["Notizen", "KI-Notizen", "Protokoll"]) {
    await expect(content.getByRole("tab", { name, exact: true })).toBeVisible();
  }
  // Transkript und Wiedergabe rechts, die Liste bleibt stehen.
  await expect(page.locator('[data-segment-index="0"]')).toBeVisible();
  await expect(
    page
      .getByTestId("rec-controls")
      .getByRole("button", { name: "Exportieren" }),
  ).toBeVisible();
  await expect(page.locator('[data-meeting-id="m2"]')).toHaveAttribute(
    "aria-current",
    "true",
  );
  await expect(page.locator('[data-meeting-id="m1"]')).toBeVisible();
  const transcript = await box(page, "transcript-scroll");
  const controls = await box(page, "rec-controls");
  expect(transcript.x).toBeGreaterThanOrEqual(controls.x);
  // Ein Wechsel der Besprechung laesst die Liste stehen.
  await pickMeeting(page, "m4");
  await expect(
    content.getByRole("heading", { name: "Elternabend Klasse 3b" }),
  ).toBeVisible();
});

test("AK2: sessions handle - drag, arrow keys, double click, limits", async ({
  page,
}) => {
  await openRecordings(page, 1920, 1050);
  const width = async () => (await box(page, "rec-sessions")).width;
  expect(await width()).toBeCloseTo(248, 0);

  await drag(page, "resize-sessions", 40);
  expect(await width()).toBeCloseTo(288, 0);

  const handle = page.getByTestId("resize-sessions");
  await expect(handle).toHaveAttribute("role", "separator");
  await expect(handle).toHaveAttribute("aria-orientation", "vertical");
  await expect(handle).toHaveAttribute("aria-valuenow", "288");
  await expect(handle).toHaveAttribute("aria-valuemin", "180");
  await expect(handle).toHaveAttribute("aria-valuemax", "400");
  await handle.focus();
  await page.keyboard.press("ArrowRight");
  expect(await width()).toBeCloseTo(304, 0);
  await page.keyboard.press("ArrowLeft");
  await page.keyboard.press("ArrowLeft");
  expect(await width()).toBeCloseTo(272, 0);

  await handle.dblclick();
  expect(await width()).toBeCloseTo(248, 0);

  await drag(page, "resize-sessions", 900);
  expect(await width()).toBeCloseTo(400, 0);
  await drag(page, "resize-sessions", -900);
  expect(await width()).toBeCloseTo(180, 0);
});

test("AK2: right handle - drag, arrow keys, double click, limits", async ({
  page,
}) => {
  await openRecordings(page, 1920, 1050);
  const width = async () => (await box(page, "rec-controls")).width;
  expect(await width()).toBeCloseTo(420, 0);

  // Nach links ziehen macht die rechte Spalte breiter.
  await drag(page, "resize-right", -60);
  expect(await width()).toBeCloseTo(480, 0);

  const handle = page.getByTestId("resize-right");
  await expect(handle).toHaveAttribute("role", "separator");
  await expect(handle).toHaveAttribute("aria-valuenow", "480");
  await expect(handle).toHaveAttribute("aria-valuemin", "300");
  await expect(handle).toHaveAttribute("aria-valuemax", "640");
  await handle.focus();
  await page.keyboard.press("ArrowLeft");
  expect(await width()).toBeCloseTo(496, 0);
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("ArrowRight");
  expect(await width()).toBeCloseTo(464, 0);

  await handle.dblclick();
  expect(await width()).toBeCloseTo(420, 0);

  await drag(page, "resize-right", -900);
  expect(await width()).toBeCloseTo(640, 0);
  await drag(page, "resize-right", 900);
  expect(await width()).toBeCloseTo(300, 0);
});

test("AK2: the workspace keeps at least 380 px in a narrow window", async ({
  page,
}) => {
  await openRecordings(page, 1280, 800);
  await drag(page, "resize-right", -900);
  await drag(page, "resize-sessions", 900);
  const content = await box(page, "rec-content");
  expect(content.width).toBeGreaterThanOrEqual(378);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBeTruthy();
});

test("AK2: both side columns collapse and expand", async ({ page }) => {
  await openRecordings(page, 1920, 1050);
  const before = await box(page, "rec-content");

  await page.getByTestId("sessions-collapse").click();
  await expect(page.getByTestId("resize-sessions")).toHaveCount(0);
  const rail = await box(page, "rec-sessions");
  expect(rail.width).toBeLessThan(60);
  await expect(page.getByTestId("sessions-expand")).toBeVisible();
  expect((await box(page, "rec-content")).width).toBeGreaterThan(before.width);

  await page.getByTestId("right-collapse").click();
  await expect(page.getByTestId("resize-right")).toHaveCount(0);
  expect((await box(page, "rec-controls")).width).toBeLessThan(60);
  await expect(page.getByTestId("right-expand")).toBeVisible();

  await page.getByTestId("sessions-expand").click();
  await page.getByTestId("right-expand").click();
  await expect(page.getByTestId("resize-sessions")).toBeVisible();
  await expect(page.getByTestId("resize-right")).toBeVisible();
  expect((await box(page, "rec-sessions")).width).toBeCloseTo(248, 0);
  expect((await box(page, "rec-controls")).width).toBeCloseTo(420, 0);
});

test("AK2: the right column cannot be collapsed while recording", async ({
  page,
}) => {
  await page.addInitScript(() => {
    localStorage.setItem("lva.ui.meetings.ui.rightCollapsed", "1");
  });
  await installRecMock(page, { recording: true });
  await openRecordings(page, 1920, 1050);
  await expect(page.getByTestId("live-notes-pad")).toBeVisible();
  // Dort sitzt der Stopp-Knopf: die Spalte bleibt offen, der Knopf ist aus.
  await expect(page.getByTestId("right-collapse")).toBeDisabled();
  expect((await box(page, "rec-controls")).width).toBeGreaterThan(300);
});

test("AK2: timestamps address the player through one seam", async ({
  page,
}) => {
  // Audio-Wiedergabe nur mitschreiben: Position beim play() = Sprungziel.
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
  await openRecordings(page, 1920, 1050);
  await pickMeeting(page, "m2");
  const stamp = page.locator('[data-segment-index="1"] [data-act="seek"]');
  await expect(stamp).toHaveAttribute("data-seek-ms", "33000");
  await stamp.click();
  await expect
    .poll(() => page.evaluate(() => (window as any).__played))
    .toEqual([33]);
});

test("AK4: the open help panel keeps the page free of scrollbars", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  await pickMeeting(page, "m2");
  await expect(page.locator('[data-segment-index="0"]')).toBeVisible();
  await page.getByRole("button", { name: "Hilfe zu dieser Seite" }).click();
  await expect(
    page.getByRole("complementary", { name: "Hilfe" }),
  ).toBeVisible();
  const r = await scrollReport(page);
  expect(r.mainOverflow, JSON.stringify(r.scrollers)).toBe(false);
  expect(r.nested, JSON.stringify(r.scrollers)).toEqual([]);
  const content = await box(page, "rec-content");
  expect(content.height).toBeGreaterThan(300);
});

// AK4: die Seite scrollt nie, jede Flaeche hoechstens einmal.
for (const [label, vp] of Object.entries(VIEWPORTS)) {
  test(`AK4: one scrollbar per area, list only (${label})`, async ({
    page,
  }) => {
    await openRecordings(page, vp.width, vp.height);
    await emitMeeting(page, PROGRESS);
    const r = await scrollReport(page);
    expect(r.documentScrollHeight).toBeLessThanOrEqual(r.innerHeight + 1);
    expect(r.mainOverflow, JSON.stringify(r.scrollers)).toBe(false);
    expect(r.nested).toEqual([]);
  });

  test(`AK4: one scrollbar per area, detail with transcript (${label})`, async ({
    page,
  }) => {
    await openRecordings(page, vp.width, vp.height);
    await pickMeeting(page, "m2");
    await expect(page.locator('[data-segment-index="0"]')).toBeVisible();
    await emitMeeting(page, PROGRESS);
    const r = await scrollReport(page);
    expect(r.documentScrollHeight).toBeLessThanOrEqual(r.innerHeight + 1);
    expect(r.mainOverflow, JSON.stringify(r.scrollers)).toBe(false);
    expect(r.nested, JSON.stringify(r.scrollers)).toEqual([]);
    // Das Transkript fuellt die Resthoehe seiner Spalte und hat keinen
    // festen Deckel (vorher max-h-96 = 384 px).
    const transcript = page.getByTestId("transcript-scroll");
    expect(
      await transcript.evaluate((el) => getComputedStyle(el).maxHeight),
    ).toBe("none");
    const lower = await box(page, "rec-lower");
    const t = await box(page, "transcript-scroll");
    expect(t.y + t.height).toBeLessThanOrEqual(lower.y + lower.height + 1);
    expect(t.height).toBeGreaterThan(40);
    expect(
      await transcript.evaluate((el) => el.scrollHeight > el.clientHeight),
    ).toBe(true);
  });

  test(`AK4: one scrollbar per area, notes, AI notes and minutes (${label})`, async ({
    page,
  }) => {
    await openRecordings(page, vp.width, vp.height);
    await pickMeeting(page, "m2");
    await expect(page.locator('[data-segment-index="0"]')).toBeVisible();
    for (const name of ["Protokoll", "KI-Notizen", "Notizen"]) {
      await page
        .getByTestId("rec-content")
        .getByRole("tab", { name, exact: true })
        .click();
      await page.waitForTimeout(100);
      const r = await scrollReport(page);
      expect(r.mainOverflow, `${name}: ${JSON.stringify(r.scrollers)}`).toBe(
        false,
      );
      expect(r.nested, name).toEqual([]);
    }
  });

  test(`AK4: one scrollbar per area, recording live (${label})`, async ({
    page,
  }) => {
    await installRecMock(page, { recording: true });
    await openRecordings(page, vp.width, vp.height);
    await expect(page.getByTestId("live-notes-pad")).toBeVisible();
    await emitMeeting(page, {
      kind: "state",
      meeting_id: "m1",
      status: "recording",
      paused: false,
    });
    // Das Live-Transkript leert sich beim Start; erst danach kommen Saetze.
    await page.waitForTimeout(200);
    await emitMeeting(page, {
      kind: "segments",
      meeting_id: "m1",
      appended: Array.from({ length: 30 }, (_, i) => ({
        segment_index: i,
        text: `Live-Satz ${i}: Die Lastspitzen liegen montags morgens am höchsten.`,
        start_ms: i * 5000,
        end_ms: i * 5000 + 4000,
        channel: i % 2,
        speaker_index: null,
      })),
    });
    await expect(page.getByText("Live-Satz 29")).toBeVisible();
    const r = await scrollReport(page);
    expect(r.documentScrollHeight).toBeLessThanOrEqual(r.innerHeight + 1);
    expect(r.mainOverflow, JSON.stringify(r.scrollers)).toBe(false);
    expect(r.nested, JSON.stringify(r.scrollers)).toEqual([]);
  });
}

test("responsive: medium keeps projects in a rail with a drawer", async ({
  page,
}) => {
  await openRecordings(page, 900, 700);
  await expect(page.getByTestId("resize-sessions")).toHaveCount(0);
  expect((await box(page, "rec-sessions")).width).toBeLessThan(60);
  await page.getByTestId("sessions-expand").click();
  const drawer = page.getByTestId("rec-sessions");
  await expect(drawer).toContainText("Jour fixe Vertrieb KW 40");
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("sessions-expand")).toBeVisible();
  // Auswahl in der Schublade schliesst sie.
  await page.getByTestId("sessions-expand").click();
  await page.locator('[data-meeting-id="m2"]').click();
  await expect(page.getByTestId("sessions-expand")).toBeVisible();
  await expect(
    page.getByTestId("rec-content").getByRole("heading", { name: TITLE_M2 }),
  ).toBeVisible();
});

test("responsive: narrow stacks workspace above transcript with a split handle", async ({
  page,
}) => {
  await openRecordings(page, 480, 800);
  await pickMeeting(page, "m2");
  await expect(page.locator('[data-segment-index="0"]')).toBeVisible();
  const content = await box(page, "rec-content");
  const controls = await box(page, "rec-controls");
  expect(controls.y).toBeGreaterThanOrEqual(content.y + content.height - 1);
  expect(Math.abs(controls.width - content.width)).toBeLessThanOrEqual(2);

  const handle = page.getByTestId("resize-split");
  await expect(handle).toHaveAttribute("role", "separator");
  await expect(handle).toHaveAttribute("aria-orientation", "horizontal");
  await expect(handle).toHaveAttribute("aria-valuenow", "50");
  await handle.focus();
  await page.keyboard.press("ArrowDown");
  await expect(handle).toHaveAttribute("aria-valuenow", "55");
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("ArrowUp");
  await expect(handle).toHaveAttribute("aria-valuenow", "45");
  await handle.dblclick();
  await expect(handle).toHaveAttribute("aria-valuenow", "50");
  await page.keyboard.press("End");
  await expect(handle).toHaveAttribute("aria-valuenow", "75");
  await page.keyboard.press("Home");
  await expect(handle).toHaveAttribute("aria-valuenow", "25");
});

// AK8: Zustand der Oberflaeche bleibt.
const tabOf = (page: Page, area: string, name: string) =>
  page.getByTestId(area).getByRole("tab", { name, exact: true });

test("AK8: widths and collapsed state survive reload and page switch", async ({
  page,
}) => {
  await openRecordings(page, 1920, 1050);
  await drag(page, "resize-sessions", 50);
  await drag(page, "resize-right", -40);
  await page.getByTestId("right-collapse").click();

  const check = async () => {
    expect((await box(page, "rec-sessions")).width).toBeCloseTo(298, 0);
    await expect(page.getByTestId("right-expand")).toBeVisible();
  };
  await check();

  await page.reload();
  await goToRecordings(page);
  await check();

  // Seitenwechsel und zurueck.
  await navTo(page, "Verlauf");
  await expect(page.getByTestId("rec-content")).toHaveCount(0);
  await navTo(page, "Aufnahmen");
  await page.getByTestId("rec-content").waitFor();
  await check();

  await page.getByTestId("right-expand").click();
  expect((await box(page, "rec-controls")).width).toBeCloseTo(460, 0);
  await page.getByTestId("sessions-collapse").click();
  await page.reload();
  await goToRecordings(page);
  await expect(page.getByTestId("sessions-expand")).toBeVisible();
  expect((await box(page, "rec-controls")).width).toBeCloseTo(460, 0);
});

test("AK8: selection and both tab choices survive reload and page switch", async ({
  page,
}) => {
  await openRecordings(page, 1920, 1050);
  await pickMeeting(page, "m2");
  await expect(page.locator('[data-segment-index="0"]')).toBeVisible();
  await tabOf(page, "rec-content", "Protokoll").click();
  await tabOf(page, "rec-controls", "Fragen").click();
  await expect(tabOf(page, "rec-controls", "Fragen")).toHaveAttribute(
    "aria-selected",
    "true",
  );

  const check = async () => {
    await expect(
      page.getByTestId("rec-content").getByRole("heading", { name: TITLE_M2 }),
    ).toBeVisible();
    await expect(tabOf(page, "rec-content", "Protokoll")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(tabOf(page, "rec-controls", "Fragen")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(page.locator('[data-meeting-id="m2"]')).toHaveAttribute(
      "aria-current",
      "true",
    );
  };
  await check();

  await page.reload();
  await goToRecordings(page);
  await check();

  await navTo(page, "Verlauf");
  await navTo(page, "Aufnahmen");
  await check();

  // Zurueck auf Transkript und Notizen: bleibt ebenfalls.
  await tabOf(page, "rec-controls", "Transkript").click();
  await tabOf(page, "rec-content", "KI-Notizen").click();
  await page.reload();
  await goToRecordings(page);
  await expect(tabOf(page, "rec-controls", "Transkript")).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(tabOf(page, "rec-content", "KI-Notizen")).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(page.locator('[data-segment-index="0"]')).toBeVisible();
});

test("AK8: a remembered recording that no longer exists is forgotten", async ({
  page,
}) => {
  await page.addInitScript(() => {
    localStorage.setItem("lva.ui.meetings.selected", "gone");
  });
  await openRecordings(page, 1920, 1050);
  await expect(page.getByTestId("rec-content")).toContainText(
    "Wähle eine Aufnahme",
  );
  await expect
    .poll(() =>
      page.evaluate(() => localStorage.getItem("lva.ui.meetings.selected")),
    )
    .toBe("");
});
