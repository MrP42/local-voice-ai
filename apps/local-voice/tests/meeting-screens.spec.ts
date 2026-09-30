import { test, expect, type Page } from "@playwright/test";
import * as fs from "node:fs";
import * as path from "node:path";
import {
  LINES,
  PROGRESS,
  SEGMENTS,
  TITLE_M2,
  TITLE_M3,
  VIEWPORTS,
  emitMeeting,
  installRecMock,
  openRecordings,
  pickMeeting,
  scrollReport,
} from "./recLayoutMock";

// Bilder der Seite "Aufnahmen" (Goal aufnahmen-ui). Kein Verhaltenstest: laeuft
// nur mit LVA_SCREENSHOTS (Zielordner) und legt dort PNGs plus je eine
// Messdatei (JSON) ab. Der Namensanfang kommt aus LVA_SCREENSHOT_PREFIX
// (Standard "nach"); die Ist-Bilder vor dem Umbau hiessen "ist-*".
//
// Aufruf (aus apps/local-voice):
//   LVA_SCREENSHOTS=../../koordination/aufnahmen-ui/screens \
//   LVA_SCREENSHOT_PREFIX=nach-m2 \
//   pnpm exec playwright test tests/meeting-screens.spec.ts --reporter=line

const DIR = process.env.LVA_SCREENSHOTS;
const PREFIX = process.env.LVA_SCREENSHOT_PREFIX ?? "nach";
test.skip(!DIR, "nur mit LVA_SCREENSHOTS (Aufnahmen, kein Verhaltenstest)");

/** Messwerte: scrollt die Seite, welche Flaechen scrollen, wo steht was. */
const measure = async (page: Page) => {
  const report = await scrollReport(page);
  const boxes = await page.evaluate(() => {
    const box = (testId: string) => {
      const el = document.querySelector(`[data-testid="${testId}"]`);
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return {
        top: Math.round(r.top),
        left: Math.round(r.left),
        width: Math.round(r.width),
        height: Math.round(r.height),
      };
    };
    return Object.fromEntries(
      [
        "rec-sessions",
        "rec-content",
        "rec-controls",
        "rec-lower",
        "transcript-scroll",
        "live-notes-pad",
        "job-panel",
      ].map((id) => [id, box(id)]),
    );
  });
  return { viewport: { w: VIEWPORTS["1920"].width }, ...report, boxes };
};

const shoot = async (page: Page, name: string) => {
  await page.waitForTimeout(250);
  const file = `${PREFIX}-${name}`;
  await page.screenshot({
    path: path.join(DIR!, `${file}.png`),
    animations: "disabled",
  });
  fs.writeFileSync(
    path.join(DIR!, `${file}.json`),
    JSON.stringify(await measure(page), null, 2),
  );
};

const SIZES = ["1920", "1366", "480"] as const;

test.beforeEach(async ({ page }) => {
  await installRecMock(page);
});

for (const label of SIZES) {
  const vp = VIEWPORTS[label];
  test.describe(`Bilder ${label}`, () => {
    test(`liste ${label}`, async ({ page }) => {
      await openRecordings(page, vp.width, vp.height);
      await emitMeeting(page, PROGRESS);
      await shoot(page, `liste-${label}`);
    });

    test(`live ${label}`, async ({ page }) => {
      await installRecMock(page, { recording: true });
      await openRecordings(page, vp.width, vp.height);
      await expect(page.getByTestId("live-notes-pad")).toBeVisible();
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
        appended: SEGMENTS.slice(0, 14),
      });
      await expect(page.getByText(LINES[3][1]).first()).toBeVisible();
      await page.getByTestId("note-starter").click();
      await page.keyboard.type("Lastspitzen 7:30–9:00, montags am höchsten", {
        delay: 2,
      });
      await page.keyboard.press("Enter");
      await page.keyboard.type("[ ] Kurzfassung bis Freitag", { delay: 2 });
      await shoot(page, `live-${label}`);
    });

    test(`detail ${label}`, async ({ page }) => {
      await openRecordings(page, vp.width, vp.height);
      await pickMeeting(page, "m2");
      await expect(page.locator('[data-segment-index="0"]')).toBeVisible();
      await shoot(page, `detail-${label}`);
    });

    test(`fortschritt ${label}`, async ({ page }) => {
      await openRecordings(page, vp.width, vp.height);
      await pickMeeting(page, "m3");
      await expect(page.locator('[data-segment-index="0"]')).toBeVisible();
      await emitMeeting(page, PROGRESS);
      await expect(page.getByTestId("job-panel")).toBeVisible();
      await shoot(page, `fortschritt-${label}`);
    });
  });
}

test("Bilder 1366: Reiter Protokoll und Notizen", async ({ page }) => {
  await openRecordings(page, 1366, 768);
  await pickMeeting(page, "m2");
  await expect(page.locator('[data-segment-index="0"]')).toBeVisible();
  const tabs = page.getByTestId("rec-content");
  await tabs.getByRole("tab", { name: "Protokoll", exact: true }).click();
  await shoot(page, "protokoll-1366");
  await tabs.getByRole("tab", { name: "Notizen", exact: true }).click();
  await shoot(page, "notizen-1366");
});

test("Bilder 1920: Fragen-Reiter", async ({ page }) => {
  await openRecordings(page, 1920, 1050);
  await pickMeeting(page, "m2");
  await expect(page.locator('[data-segment-index="0"]')).toBeVisible();
  await page
    .getByTestId("rec-controls")
    .getByRole("tab", { name: "Fragen", exact: true })
    .click();
  await shoot(page, "chat-1920");
});

test("Bilder 900: Projekte in der Schublade", async ({ page }) => {
  await openRecordings(page, 900, 700);
  await pickMeeting(page, "m2");
  await expect(
    page.getByTestId("rec-content").getByRole("heading", { name: TITLE_M2 }),
  ).toBeVisible();
  await shoot(page, "detail-900");
  await page.getByTestId("sessions-expand").click();
  await expect(
    page.getByTestId("rec-sessions").getByText(TITLE_M3),
  ).toBeVisible();
  await shoot(page, "schublade-900");
});

test("Bilder 480: Projekte in der Schublade", async ({ page }) => {
  await openRecordings(page, 480, 800);
  await page.getByTestId("sessions-open").click();
  await shoot(page, "schublade-480");
});
