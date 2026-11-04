import { test, expect, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { join } from "node:path";
import {
  calls,
  installRecMock,
  openRecordings,
  pickMeeting,
} from "./recLayoutMock";

// Mehrere Audiodateien je Besprechung: beim Import zusammenfassen, im Menü
// „Audiodatei anhängen …“, in der Mehrfachauswahl „Zusammenführen“, Teile-Chip
// mit Teilliste und Trennzeilen im Transkript. Belege für die Codex-Abnahme
// (390/1440 px, hell/dunkel) nach docs/vorschau/mehrere-audiodateien/, wenn
// LV_SHOT_DIR gesetzt ist; sonst nach test-results/.

const SHOT_DIR = process.env.LV_SHOT_DIR ?? "test-results/mehrere-audiodateien";
mkdirSync(SHOT_DIR, { recursive: true });
const shot = (name: string) => join(SHOT_DIR, name);

const PARTS_M2 = {
  parts: [
    {
      part_index: 0,
      title: "SWK-Lastgang-2026-09-28.m4a",
      source_path: "C:/Users/patrick/Aufnahmen/SWK-Lastgang-2026-09-28.m4a",
      offset_ms: 0,
      duration_ms: 870_000,
    },
    {
      part_index: 1,
      title: "SWK-Lastgang-2026-09-28 nach der Pause.m4a",
      source_path:
        "C:/Users/patrick/Aufnahmen/SWK-Lastgang-2026-09-28 nach der Pause.m4a",
      offset_ms: 870_000,
      duration_ms: 870_000,
    },
  ],
  pending: [],
  append_to: null,
};

/** Teile-Befehle zusätzlich zur Aufnahmen-Attrappe; `theme` hell oder dunkel. */
const installParts = async (
  page: Page,
  opts: { theme?: "light" | "dark"; parts?: unknown } = {},
) => {
  await installRecMock(page);
  await page.addInitScript(
    ({ theme, parts }) => {
      const w = window as any;
      w.__settings.theme = theme;
      w.__partsInfo = parts;
      const inner = w.__TAURI_INTERNALS__.invoke;
      w.__TAURI_INTERNALS__.invoke = async (
        cmd: string,
        args: Record<string, any> = {},
      ) => {
        switch (cmd) {
          case "meetings_parts_info":
            w.__calls.push({ cmd, args });
            return (
              w.__partsInfo[args.meetingId] ?? {
                parts: [],
                pending: [],
                append_to: null,
              }
            );
          case "meetings_merge":
            w.__calls.push({ cmd, args });
            return args.meetingIds[0];
          case "meetings_append_files":
            w.__calls.push({ cmd, args });
            return args.paths.map((_: string, i: number) => `neu-${i}`);
          case "meetings_append_detach":
            w.__calls.push({ cmd, args });
            return null;
          case "meetings_import_file":
            w.__calls.push({ cmd, args });
            return { ...w.__meetings[1], id: "m-import" };
        }
        return inner(cmd, args);
      };
    },
    { theme: opts.theme ?? "light", parts: opts.parts ?? { m2: PARTS_M2 } },
  );
};

const VIEWS = [
  { label: "1440", width: 1440, height: 900 },
  { label: "390", width: 390, height: 844 },
] as const;

/** Alle sichtbaren Bedienelemente in `scope` müssen bei 390 px ≥ 44 px hoch sein. */
const expectTouchTargets = async (page: Page, testIds: string[]) => {
  for (const id of testIds) {
    const items = page.getByTestId(id);
    const n = await items.count();
    expect(n, id).toBeGreaterThan(0);
    for (let i = 0; i < n; i++) {
      const el = items.nth(i);
      if (!(await el.isVisible())) continue;
      const box = (await el.boundingBox())!;
      expect(box.height, `${id}[${i}] Höhe`).toBeGreaterThanOrEqual(44);
      expect(box.width, `${id}[${i}] Breite`).toBeGreaterThanOrEqual(44);
    }
  }
};

/**
 * Offener Dialog: Schließen-Knopf >= 44 x 44 px und kein Ausblenden des
 * Inhalts zum Rand hin (Hinweise zu Verlust und Einwilligung voll lesbar).
 */
const expectDialogReadable = async (page: Page, touch: boolean) => {
  const content = page.locator(".lv-dialog .lv-dialog__content").last();
  expect(
    await content.evaluate((el) => {
      const cs = getComputedStyle(el);
      return cs.maskImage || cs.webkitMaskImage || "none";
    }),
  ).toBe("none");
  if (!touch) return;
  const close = page
    .locator(".lv-dialog .fui-DialogTitle__action button")
    .last();
  const box = (await close.boundingBox())!;
  expect(box.height, "Schließen Höhe").toBeGreaterThanOrEqual(44);
  expect(box.width, "Schließen Breite").toBeGreaterThanOrEqual(44);
};

/** Auswahlmodus der Liste einschalten; bei 390 px erst die Liste aufklappen. */
const startSelection = async (page: Page) => {
  const open = page.getByTestId("sessions-open");
  if (await open.isVisible()) await open.click();
  await page.getByTestId("projects-more").click();
  await page.getByTestId("projects-select").click();
};

const openM2 = async (page: Page, width: number, height: number) => {
  await openRecordings(page, width, height);
  await pickMeeting(page, "m2");
  await expect(page.locator('[data-segment-index="0"]')).toBeVisible();
};

test.describe("Teile einer Besprechung", () => {
  test("Teile-Chip öffnet die Teilliste; ein Klick springt an den Beginn des Teils", async ({
    page,
  }) => {
    await installParts(page);
    await openM2(page, 1440, 900);
    const chip = page.getByTestId("parts-chip");
    await expect(chip).toHaveText(/2 Teile/);
    await chip.click();
    const dialog = page.getByTestId("parts-dialog");
    await expect(dialog.getByTestId("parts-row")).toHaveCount(2);
    await expect(dialog.getByTestId("parts-row").nth(1)).toContainText("14:30");
  });

  test("Trennzeile im Transkript am Beginn von Teil 2", async ({ page }) => {
    await installParts(page);
    await openM2(page, 1440, 900);
    const divider = page.getByTestId("part-divider");
    await expect(divider).toHaveCount(1);
    await expect(divider).toContainText("Teil 2");
    await expect(divider).toContainText("ab 14:30");
  });

  test("Menü „Audiodatei anhängen …“: Dateiwahl, Reihenfolge, Einwilligung, dann erst anhängen", async ({
    page,
  }) => {
    await installParts(page);
    await openM2(page, 1440, 900);
    await page.evaluate(
      () =>
        ((window as any).__pick = [
          "C:/Audio/Teil-3.m4a",
          "C:/Audio/Teil-2.m4a",
        ]),
    );
    await page.getByTestId("meeting-menu").click();
    await page
      .getByTestId("menu-append-audio")
      .evaluate((el) => (el as HTMLElement).click());
    const dialog = page.getByTestId("append-dialog");
    await expect(dialog).toBeVisible();
    expect(await calls(page, "meetings_append_files")).toHaveLength(0);
    // Reihenfolge tauschen: Teil 2 nach vorn
    await dialog.getByTestId("append-order-up").nth(1).click();
    await page.getByTestId("append-confirm").click();
    await expect
      .poll(async () => (await calls(page, "meetings_append_files")).length)
      .toBe(1);
    expect((await calls(page, "meetings_append_files"))[0].args).toEqual({
      targetMeetingId: "m2",
      paths: ["C:/Audio/Teil-2.m4a", "C:/Audio/Teil-3.m4a"],
      consentConfirmed: true,
    });
  });

  test("Mehrfachauswahl: „Zusammenführen“ ab zwei Aufnahmen, Dialog mit Reihenfolge", async ({
    page,
  }) => {
    await installParts(page);
    await openRecordings(page, 1440, 900);
    await startSelection(page);
    await expect(page.getByTestId("selection-merge")).toBeDisabled();
    await page.locator('[data-meeting-id="m2"]').click();
    await page.locator('[data-meeting-id="m6"]').click();
    await expect(page.getByTestId("selection-merge")).toBeEnabled();
    await page.getByTestId("selection-merge").click();
    const dialog = page.getByTestId("merge-dialog");
    await expect(dialog).toBeVisible();
    await expect(page.getByTestId("merge-target")).toBeVisible();
    await page.getByTestId("merge-confirm").click();
    await expect
      .poll(async () => (await calls(page, "meetings_merge")).length)
      .toBe(1);
    const args = (await calls(page, "meetings_merge"))[0].args;
    expect(args.keepOrder).toBe(true);
    expect([...args.meetingIds].sort()).toEqual(["m2", "m6"]);
  });

  test("Import mehrerer Dateien: „Als eine Besprechung zusammenfassen“ importiert die erste und hängt den Rest an", async ({
    page,
  }) => {
    await installParts(page);
    await openRecordings(page, 1440, 900);
    await page.evaluate(
      () =>
        ((window as any).__pick = [
          "C:/Audio/Vor-der-Pause.m4a",
          "C:/Audio/Nach-der-Pause.m4a",
        ]),
    );
    await page.getByTestId("import-open").click();
    const combine = page.getByTestId("import-combine");
    await expect(combine).toBeVisible();
    await combine.check();
    await expect(page.getByTestId("import-order-item")).toHaveCount(2);
    await page.getByTestId("import-confirm").click();
    await expect
      .poll(async () => (await calls(page, "meetings_append_files")).length)
      .toBe(1);
    expect((await calls(page, "meetings_import_file"))[0].args.path).toBe(
      "C:/Audio/Vor-der-Pause.m4a",
    );
    expect((await calls(page, "meetings_append_files"))[0].args.paths).toEqual([
      "C:/Audio/Nach-der-Pause.m4a",
    ]);
  });

  test("390 px: Teile-Zeile, Trennzeile und Dialogknöpfe sind ≥ 44 px hoch", async ({
    page,
  }) => {
    await installParts(page, {
      parts: {
        m2: {
          ...PARTS_M2,
          pending: [
            {
              meeting_id: "m9",
              title: "Teil-3.m4a",
              status: "failed",
              position: 0,
            },
          ],
        },
      },
    });
    await openM2(page, 390, 844);
    await expectTouchTargets(page, [
      "parts-row-trigger",
      "part-divider-button",
    ]);
    await page.getByTestId("parts-row-trigger").click();
    await expect(page.getByTestId("parts-dialog")).toBeVisible();
    await expectTouchTargets(page, ["parts-row"]);
    const detach = page
      .getByTestId("parts-pending-row")
      .getByRole("button", { name: "Nicht anhängen" });
    expect((await detach.boundingBox())!.height).toBeGreaterThanOrEqual(44);
  });
});

test.describe("Belege für die Abnahme", () => {
  for (const theme of ["light", "dark"] as const) {
    for (const view of VIEWS) {
      const suffix = `${view.label}-${theme === "light" ? "hell" : "dunkel"}`;

      test(`Detail mit Teilen und Trennzeile (${suffix})`, async ({ page }) => {
        await installParts(page, { theme });
        await openM2(page, view.width, view.height);
        await page.getByTestId("part-divider").scrollIntoViewIfNeeded();
        await page.screenshot({
          path: shot(`detail-trennzeile-${suffix}.png`),
        });
        await page
          .getByTestId(
            view.label === "390" ? "parts-row-trigger" : "parts-chip",
          )
          .click();
        await expect(page.getByTestId("parts-dialog")).toBeVisible();
        await expectDialogReadable(page, view.label === "390");
        await page.screenshot({ path: shot(`teilliste-${suffix}.png`) });
      });

      test(`Audiodatei anhängen (${suffix})`, async ({ page }) => {
        await installParts(page, { theme });
        await openM2(page, view.width, view.height);
        await page.evaluate(
          () =>
            ((window as any).__pick = [
              "C:/Audio/SWK nach der Pause.m4a",
              "C:/Audio/SWK Nachgespräch.m4a",
            ]),
        );
        await page.getByTestId("meeting-menu").click();
        await page
          .getByTestId("menu-append-audio")
          .evaluate((el) => (el as HTMLElement).click());
        await expect(page.getByTestId("append-dialog")).toBeVisible();
        await expectDialogReadable(page, view.label === "390");
        if (view.label === "390")
          await expectTouchTargets(page, [
            "append-order-up",
            "append-order-down",
          ]);
        await page.screenshot({ path: shot(`anhaengen-${suffix}.png`) });
      });

      test(`Zusammenführen in der Mehrfachauswahl (${suffix})`, async ({
        page,
      }) => {
        await installParts(page, { theme });
        await openRecordings(page, view.width, view.height);
        await startSelection(page);
        await page.locator('[data-meeting-id="m2"]').click();
        await page.locator('[data-meeting-id="m6"]').click();
        if (view.label === "390")
          await expectTouchTargets(page, ["selection-merge"]);
        await page.screenshot({ path: shot(`auswahl-${suffix}.png`) });
        await page.getByTestId("selection-merge").click();
        await expect(page.getByTestId("merge-dialog")).toBeVisible();
        await expectDialogReadable(page, view.label === "390");
        if (view.label === "390")
          await expectTouchTargets(page, [
            "merge-order-up",
            "merge-order-down",
          ]);
        await page.screenshot({ path: shot(`zusammenfuehren-${suffix}.png`) });
      });

      test(`Import zusammenfassen (${suffix})`, async ({ page }) => {
        await installParts(page, { theme });
        await openRecordings(page, view.width, view.height);
        await page.evaluate(
          () =>
            ((window as any).__pick = [
              "C:/Audio/Klausur Teil 1.m4a",
              "C:/Audio/Klausur Teil 2.m4a",
              "C:/Audio/Klausur Teil 3.m4a",
            ]),
        );
        await page.getByTestId("import-open").click();
        await page.getByTestId("import-combine").check();
        await expect(page.getByTestId("import-order-item")).toHaveCount(3);
        await page.screenshot({
          path: shot(`import-zusammenfassen-${suffix}.png`),
        });
      });
    }
  }
});
