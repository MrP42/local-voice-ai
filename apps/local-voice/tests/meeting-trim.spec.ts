import { test, expect, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { join } from "node:path";
import {
  TITLE_M1,
  calls,
  installRecMock,
  openRecordings,
  pickMeeting,
} from "./recLayoutMock";

// lva#522: Bereich einer Aufnahme.
//  A) „Neu transkribieren“ mit „Nur einen Bereich“ (Von/Bis und Zweifach-Regler) ruft
//     `meetings_retranscribe` mit `range`.
//  B) „Aufnahme zuschneiden …“ (endgültig) ruft `meetings_trim_audio` erst nach dem Klick
//     auf den Gefahrenknopf.
// Die Logik (Schneiden, Zeitversatz, Transaktion) prüfen die Rust-Tests in `trim/tests.rs`.
// Vorschau für die Abnahme: ZUSCHNEIDEN_SHOTS=1 schreibt je Dialog Bilder bei 390 und
// 1440 px, hell und dunkel nach docs/vorschau/zuschneiden/ (LV_SHOT_DIR überschreibt).

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

const SHOTS = process.env.ZUSCHNEIDEN_SHOTS === "1";
const SHOT_DIR =
  process.env.LV_SHOT_DIR ?? join("docs", "vorschau", "zuschneiden");

/** m2: importiert, mit Audio, 59:00 (3 540 000 ms). */
const DURATION_M2 = 3_540_000;

type Theme = "light" | "dark";

/**
 * `meetings_trim_audio` antwortet mit der neuen Dauer (1 800 000 ms), bei `__trimError` mit
 * diesem Fehlercode; die Attrappe der Aufnahmen kennt den Befehl nicht.
 */
const installTrimMock = async (page: Page, theme: Theme = "light") => {
  await installRecMock(page);
  await page.addInitScript((themeName) => {
    const w = window as any;
    w.__settings.theme = themeName;
    w.__trimError = null;
    const internals = w.__TAURI_INTERNALS__;
    const original = internals.invoke;
    internals.invoke = async (cmd: string, args: Record<string, unknown>) => {
      if (cmd !== "meetings_trim_audio") return original(cmd, args);
      w.__calls.push({ cmd, args });
      if (w.__trimError) throw w.__trimError;
      return 1_800_000;
    };
  }, theme);
};

const openM2 = async (page: Page, width = 1366, height = 768) => {
  await openRecordings(page, width, height);
  await pickMeeting(page, "m2");
  await expect(page.locator('[data-segment-index="0"]')).toBeVisible();
};

/** Menü „☰“ öffnen und einen Eintrag wählen (per JS-Klick: das Menü schließt bei Scroll). */
const chooseMenu = async (page: Page, testId: string) => {
  await page.getByTestId("meeting-menu").click();
  await expect(page.getByRole("menu")).toBeVisible();
  await page.getByTestId(testId).evaluate((el) => (el as HTMLElement).click());
};

const field = (page: Page, id: string) => page.getByTestId(id);

const setTime = async (page: Page, id: string, text: string) => {
  const input = field(page, id);
  await input.fill(text);
  await input.press("Enter");
};

// ---------------------------------------------------------------------------
// A: Bereich transkribieren
// ---------------------------------------------------------------------------

test.describe("Neu transkribieren: Bereich", () => {
  test.beforeEach(async ({ page }) => {
    await installTrimMock(page);
  });

  test("Standard ist die ganze Aufnahme; erst „Nur einen Bereich“ zeigt Felder und Regler", async ({
    page,
  }) => {
    await openM2(page);
    await chooseMenu(page, "menu-retranscribe");
    const dialog = page.getByTestId("retranscribe-dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog.getByTestId("retranscribe-scope-all")).toBeChecked();
    await expect(dialog.getByTestId("retranscribe-range")).toHaveCount(0);
    await dialog.getByTestId("retranscribe-scope-range").click();
    await expect(dialog.getByTestId("retranscribe-range")).toBeVisible();
    // Vorbelegt: die ganze Aufnahme.
    await expect(field(page, "retranscribe-range-start")).toHaveValue("0:00");
    await expect(field(page, "retranscribe-range-end")).toHaveValue("59:00");
    await expect(dialog.getByTestId("retranscribe-range-hint")).toHaveText(
      "Nur der gewählte Bereich wird transkribiert; die Zeitstempel bleiben an der Aufnahme ausgerichtet.",
    );
  });

  test("ein Bereich geht als range an meetings_retranscribe, die ganze Aufnahme als null", async ({
    page,
  }) => {
    await openM2(page);
    await chooseMenu(page, "menu-retranscribe");
    await page.getByTestId("retranscribe-scope-range").click();
    // Tolerant gelesen: m:ss und h:mm:ss.
    await setTime(page, "retranscribe-range-start", "10:00");
    await setTime(page, "retranscribe-range-end", "0:20:30");
    await expect(field(page, "retranscribe-range-end")).toHaveValue("20:30");
    expect(await calls(page, "meetings_retranscribe")).toHaveLength(0);
    await page.getByTestId("retranscribe-start").click();
    await expect(page.getByTestId("retranscribe-dialog")).toHaveCount(0);
    expect((await calls(page, "meetings_retranscribe"))[0].args).toEqual({
      meetingId: "m2",
      modelId: null,
      language: null,
      range: { start_ms: 600_000, end_ms: 1_230_000 },
    });
  });

  test("ein Bereich, der alles deckt, ist keiner: range ist null", async ({
    page,
  }) => {
    await openM2(page);
    await chooseMenu(page, "menu-retranscribe");
    await page.getByTestId("retranscribe-scope-range").click();
    await page.getByTestId("retranscribe-start").click();
    expect((await calls(page, "meetings_retranscribe"))[0].args.range).toBe(
      null,
    );
  });

  test("Regler und Felder halten denselben Bereich", async ({ page }) => {
    await openM2(page);
    await chooseMenu(page, "menu-retranscribe");
    await page.getByTestId("retranscribe-scope-range").click();
    await field(page, "retranscribe-range-slider-end").fill("1800000");
    await expect(field(page, "retranscribe-range-end")).toHaveValue("30:00");
    await field(page, "retranscribe-range-slider-start").fill("65000");
    await expect(field(page, "retranscribe-range-start")).toHaveValue("1:05");
    await expect(page.getByTestId("retranscribe-range-length")).toContainText(
      "28:55 von 59:00",
    );
    await setTime(page, "retranscribe-range-start", "2:00");
    await expect(
      field(page, "retranscribe-range-slider-start"),
    ).toHaveAttribute("aria-valuetext", "2:00");
    await expect(field(page, "retranscribe-range-slider-start")).toHaveValue(
      "120000",
    );
  });

  test("Unlesbares wird gemeldet und zurückgesetzt; Grenzen stoßen an", async ({
    page,
  }) => {
    await openM2(page);
    await chooseMenu(page, "menu-retranscribe");
    await page.getByTestId("retranscribe-scope-range").click();
    await setTime(page, "retranscribe-range-start", "abc");
    await expect(page.getByTestId("retranscribe-range-start-error")).toHaveText(
      "Zeit nicht lesbar. Beispiele: 1:05 oder 1:02:30.",
    );
    await expect(field(page, "retranscribe-range-start")).toHaveValue("0:00");
    // Anfang hinter dem Ende der Aufnahme: stößt eine Sekunde vor das Ende.
    await setTime(page, "retranscribe-range-start", "2:00:00");
    await expect(field(page, "retranscribe-range-start")).toHaveValue("58:59");
    // Ende vor dem Anfang: stößt eine Sekunde hinter den Anfang.
    await setTime(page, "retranscribe-range-end", "1:00");
    await expect(field(page, "retranscribe-range-end")).toHaveValue("59:00");
  });
});

// ---------------------------------------------------------------------------
// B: Aufnahme zuschneiden
// ---------------------------------------------------------------------------

test.describe("Aufnahme zuschneiden", () => {
  test.beforeEach(async ({ page }) => {
    await installTrimMock(page);
  });

  test("der Befehl läuft erst nach dem Klick auf den Gefahrenknopf", async ({
    page,
  }) => {
    await openM2(page);
    await chooseMenu(page, "menu-trim");
    const dialog = page.getByTestId("trim-dialog");
    await expect(dialog).toBeVisible();
    // Der Verlusthinweis steht da und ist nicht ausgeblendet.
    await expect(dialog.getByTestId("trim-warning")).toContainText(
      "Das Audio außerhalb des Bereichs wird endgültig gelöscht.",
    );
    await expect(dialog.getByTestId("trim-warning")).toContainText(
      "die Zeitstempel beginnen danach bei 0:00",
    );
    await expect(dialog.getByTestId("trim-warning")).toContainText(
      "Ältere Fassungen werden mitgekürzt.",
    );
    // Ganze Aufnahme gewählt: nichts zu schneiden, der Knopf ist gesperrt.
    await expect(page.getByTestId("trim-start")).toBeDisabled();
    await expect(dialog.getByTestId("trim-nothing")).toBeVisible();

    await setTime(page, "trim-range-start", "5:00");
    await setTime(page, "trim-range-end", "35:00");
    await expect(page.getByTestId("trim-range-length")).toHaveText(
      "Neue Länge: 30:00 von 59:00",
    );
    await expect(page.getByTestId("trim-start")).toBeEnabled();
    // Bis hierher nichts geschnitten.
    expect(await calls(page, "meetings_trim_audio")).toHaveLength(0);

    const segmentCalls = (await calls(page, "meetings_get_segments")).length;
    await page.getByTestId("trim-start").click();
    await expect(page.getByTestId("trim-dialog")).toHaveCount(0);
    const trim = await calls(page, "meetings_trim_audio");
    expect(trim).toHaveLength(1);
    expect(trim[0].args).toEqual({
      meetingId: "m2",
      startMs: 300_000,
      endMs: 2_100_000,
    });
    await expect(page.locator("[data-sonner-toast]").first()).toContainText(
      "Aufnahme zugeschnitten. Neue Länge: 30:00.",
    );
    // Segmente und Datensatz werden neu geladen.
    await expect
      .poll(async () => (await calls(page, "meetings_get_segments")).length)
      .toBeGreaterThan(segmentCalls);
  });

  test("Abbrechen und Schließen schneiden nichts", async ({ page }) => {
    await openM2(page);
    await chooseMenu(page, "menu-trim");
    await setTime(page, "trim-range-end", "10:00");
    await page.getByTestId("trim-cancel").click();
    await expect(page.getByTestId("trim-dialog")).toHaveCount(0);
    // Beim nächsten Öffnen wieder die ganze Aufnahme.
    await chooseMenu(page, "menu-trim");
    await expect(field(page, "trim-range-end")).toHaveValue("59:00");
    expect(await calls(page, "meetings_trim_audio")).toHaveLength(0);
  });

  test("ein Fehlercode erscheint übersetzt, der Dialog bleibt offen", async ({
    page,
  }) => {
    await openM2(page);
    await page.evaluate(() => ((window as any).__trimError = "trim_changed"));
    await chooseMenu(page, "menu-trim");
    await setTime(page, "trim-range-start", "1:00");
    await page.getByTestId("trim-start").click();
    await expect(page.locator("[data-sonner-toast]").first()).toContainText(
      "Die Aufnahme hat sich währenddessen verändert. Es wurde nichts geändert; bitte erneut versuchen.",
    );
    await expect(page.getByTestId("trim-dialog")).toBeVisible();
  });

  test("ohne Audio ist der Menüeintrag gesperrt", async ({ page }) => {
    await openRecordings(page, 1366, 768);
    await pickMeeting(page, "m1");
    await expect(
      page.getByRole("heading", { name: TITLE_M1 }).first(),
    ).toBeVisible();
    await page.getByTestId("meeting-menu").click();
    await expect(page.getByRole("menu")).toBeVisible();
    await expect(page.getByTestId("menu-trim")).toBeDisabled();
  });
});

// ---------------------------------------------------------------------------
// Vorschau und Layout: 390 / 1440 px, hell / dunkel
// ---------------------------------------------------------------------------

const BREITEN = [390, 1440] as const;
const HOEHE: Record<number, number> = { 390: 844, 1440: 900 };
const THEMEN: Theme[] = ["light", "dark"];
const NAME_THEME: Record<Theme, string> = { light: "hell", dark: "dunkel" };

const shoot = async (page: Page, name: string) => {
  if (!SHOTS) return;
  mkdirSync(SHOT_DIR, { recursive: true });
  await page.waitForTimeout(300);
  await page.screenshot({
    path: join(SHOT_DIR, name),
    animations: "disabled",
  });
};

/** Keine waagerechte Seitenleiste; alle bedienbaren Teile des Dialogs ≥ 44 px (schmal). */
const checkLayout = async (page: Page, dialogId: string, width: number) => {
  const surface = page.locator('[role="dialog"]').last();
  await expect(surface).toBeVisible();
  const overflow = await page.evaluate(() => ({
    scroll: document.documentElement.scrollWidth,
    inner: window.innerWidth,
  }));
  expect(overflow.scroll).toBeLessThanOrEqual(overflow.inner);
  const box = (await surface.boundingBox())!;
  expect(box.x).toBeGreaterThanOrEqual(-1);
  expect(box.x + box.width).toBeLessThanOrEqual(width + 1);
  if (width >= 640) return;
  const small: string[] = [];
  const targets = surface.locator(
    'button, [role="radio"], input[type="radio"], input[type="range"], input[type="text"], .lv-range2',
  );
  const count = await targets.count();
  for (let i = 0; i < count; i += 1) {
    const target = targets.nth(i);
    if (!(await target.isVisible())) continue;
    const b = await target.boundingBox();
    if (!b) continue;
    // Das Eingabefeld ist 32 px hoch (Fluent compact); Bedienziele sind Knöpfe, Auswahl, Regler.
    const isField = (await target.getAttribute("type")) === "text";
    if (isField) continue;
    if (b.height < 43.5)
      small.push(
        `${await target.evaluate((el) => el.tagName + "." + el.className.toString().slice(0, 40))} ${Math.round(b.height)}`,
      );
  }
  expect(small, `${dialogId}: Ziele unter 44 px`).toEqual([]);
};

for (const theme of THEMEN) {
  for (const width of BREITEN) {
    test.describe(`Vorschau ${width} px ${NAME_THEME[theme]}`, () => {
      test.beforeEach(async ({ page }) => {
        await installTrimMock(page, theme);
      });

      test("Neu transkribieren mit Bereich", async ({ page }) => {
        await openM2(page, width, HOEHE[width]);
        await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
        await chooseMenu(page, "menu-retranscribe");
        await page.getByTestId("retranscribe-scope-range").click();
        await setTime(page, "retranscribe-range-start", "10:00");
        await setTime(page, "retranscribe-range-end", "42:30");
        await checkLayout(page, "retranscribe", width);
        await shoot(
          page,
          `zuschneiden-bereich-transkribieren-${width}-${NAME_THEME[theme]}.png`,
        );
      });

      test("Aufnahme zuschneiden", async ({ page }) => {
        await openM2(page, width, HOEHE[width]);
        await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
        await chooseMenu(page, "menu-trim");
        await setTime(page, "trim-range-start", "5:00");
        await setTime(page, "trim-range-end", "35:00");
        await expect(page.getByTestId("trim-warning")).toBeVisible();
        await checkLayout(page, "trim", width);
        await shoot(
          page,
          `zuschneiden-aufnahme-zuschneiden-${width}-${NAME_THEME[theme]}.png`,
        );
      });
    });
  }
}

// Die Dauer der Aufnahme steht im Test, damit eine Änderung der Attrappe auffällt.
test("die Attrappe liefert für m2 die erwartete Dauer", async ({ page }) => {
  await installTrimMock(page);
  await openM2(page);
  const duration = await page.evaluate(
    () =>
      (window as any).__meetings.find((m: any) => m.id === "m2").duration_ms,
  );
  expect(duration).toBe(DURATION_M2);
});
