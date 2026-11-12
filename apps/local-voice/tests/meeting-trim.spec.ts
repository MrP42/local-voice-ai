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
 * `meetings_trim_audio` antwortet mit der neuen Dauer (1 800 000 ms) und `__leftover`
 * gesperrten alten Dateien (`__uncertain`: Aufräumen unsicher), bei `__trimError` mit diesem Fehlercode; `meetings_get` liefert
 * den einen Datensatz (`__getFail` = "error" | "null" lässt ihn scheitern). Beim Aufruf von `meetings_trim_audio` wird gemerkt, wie viele
 * `<audio src>` noch an der Seite hängen (`__audioAtTrim`): die Player müssen die Dateien
 * vorher losgelassen haben. Die Attrappe der Aufnahmen kennt die Befehle nicht.
 */
const installTrimMock = async (page: Page, theme: Theme = "light") => {
  await installRecMock(page);
  await page.addInitScript((themeName) => {
    const w = window as any;
    w.__settings.theme = themeName;
    w.__trimError = null;
    w.__leftover = 0;
    w.__uncertain = false;
    w.__audioAtTrim = null;
    w.__getFail = null;
    const internals = w.__TAURI_INTERNALS__;
    const original = internals.invoke;
    internals.invoke = async (cmd: string, args: Record<string, unknown>) => {
      if (cmd === "meetings_get") {
        w.__calls.push({ cmd, args });
        if (w.__getFail === "error") throw "store_failed";
        if (w.__getFail === "null") return null;
        return w.__meetings.find((m: any) => m.id === args.meetingId) ?? null;
      }
      if (cmd !== "meetings_trim_audio") return original(cmd, args);
      w.__calls.push({ cmd, args });
      w.__audioAtTrim = document.querySelectorAll("audio[src]").length;
      if (w.__trimError) throw w.__trimError;
      return {
        duration_ms: 1_800_000,
        leftover_files: w.__leftover,
        cleanup_uncertain: w.__uncertain,
      };
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
      "Nur der gewählte Bereich wird transkribiert; die Zeitstempel bleiben an der Aufnahme ausgerichtet. Der Rest bleibt in der vorherigen Fassung erhalten.",
    );
  });

  test("ein unlesbares Feld sperrt Neu transkribieren, es gibt keinen Rückgriff auf älteren Wert", async ({
    page,
  }) => {
    await openM2(page);
    await chooseMenu(page, "menu-retranscribe");
    await page.getByTestId("retranscribe-scope-range").click();
    await setTime(page, "retranscribe-range-start", "5:00");
    await field(page, "retranscribe-range-end").fill("35:");
    await expect(page.getByTestId("retranscribe-start")).toBeDisabled();
    await page.getByTestId("retranscribe-start").click({ force: true });
    await expect(
      page.getByTestId("retranscribe-range-end-error"),
    ).toBeVisible();
    // Der Text bleibt stehen (kein stilles Zurücksetzen auf 59:00).
    await expect(field(page, "retranscribe-range-end")).toHaveValue("35:");
    expect(await calls(page, "meetings_retranscribe")).toHaveLength(0);
    // Wieder lesbar: es läuft mit dem Feldtext.
    await setTime(page, "retranscribe-range-end", "35:00");
    await expect(page.getByTestId("retranscribe-start")).toBeEnabled();
    await page.getByTestId("retranscribe-start").click();
    expect((await calls(page, "meetings_retranscribe"))[0].args.range).toEqual({
      start_ms: 300_000,
      end_ms: 2_100_000,
    });
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

  test("Unlesbares wird gemeldet und sperrt; Grenzen stoßen an", async ({
    page,
  }) => {
    await openM2(page);
    await chooseMenu(page, "menu-retranscribe");
    await page.getByTestId("retranscribe-scope-range").click();
    await setTime(page, "retranscribe-range-start", "abc");
    await expect(page.getByTestId("retranscribe-range-start-error")).toHaveText(
      "Zeit nicht lesbar. Beispiele: 1:05 oder 1:02:30.",
    );
    // Der unlesbare Text bleibt stehen und sperrt den Knopf (kein stiller Rückgriff).
    await expect(field(page, "retranscribe-range-start")).toHaveValue("abc");
    await expect(page.getByTestId("retranscribe-start")).toBeDisabled();
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
    const listCalls = (await calls(page, "meetings_list")).length;
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
    // Segmente und genau dieser Datensatz werden neu geladen (nicht die ganze Liste).
    await expect
      .poll(async () => (await calls(page, "meetings_get_segments")).length)
      .toBeGreaterThan(segmentCalls);
    await expect
      .poll(async () => (await calls(page, "meetings_get")).length)
      .toBe(1);
    expect((await calls(page, "meetings_get"))[0].args).toEqual({
      meetingId: "m2",
    });
    expect(await calls(page, "meetings_list")).toHaveLength(listCalls);
    // Die Player sind danach wieder da.
    await expect(page.getByTestId("rec-player")).toBeVisible();
  });

  test("ein unlesbares Feld sperrt Zuschneiden: kein Aufruf, kein Rückgriff auf älteren Wert", async ({
    page,
  }) => {
    await openM2(page);
    await chooseMenu(page, "menu-trim");
    await setTime(page, "trim-range-start", "5:00");
    await field(page, "trim-range-end").fill("35:");
    await expect(page.getByTestId("trim-start")).toBeDisabled();
    await page.getByTestId("trim-start").click({ force: true });
    await expect(page.getByTestId("trim-range-end-error")).toBeVisible();
    await expect(field(page, "trim-range-end")).toHaveValue("35:");
    await expect(page.getByTestId("trim-dialog")).toBeVisible();
    expect(await calls(page, "meetings_trim_audio")).toHaveLength(0);
  });

  test("ein getipptes, noch nicht bestätigtes Feld gilt beim Klick mit seinem aktuellen Text", async ({
    page,
  }) => {
    await openM2(page);
    await chooseMenu(page, "menu-trim");
    await field(page, "trim-range-start").fill("5:00");
    await page.getByTestId("trim-start").click();
    await expect(page.getByTestId("trim-dialog")).toHaveCount(0);
    const trim = await calls(page, "meetings_trim_audio");
    expect(trim).toHaveLength(1);
    expect(trim[0].args.startMs).toBe(300_000);
  });

  test("scheitert das Laden nach dem Zuschnitt: Hinweis mit Erneut laden, Player bleiben abgebaut", async ({
    page,
  }) => {
    await openM2(page);
    await page.evaluate(() => ((window as any).__getFail = "null"));
    await chooseMenu(page, "menu-trim");
    await setTime(page, "trim-range-start", "5:00");
    await page.getByTestId("trim-start").click();
    await expect(page.getByTestId("trim-dialog")).toHaveCount(0);
    const failed = page.locator("[data-sonner-toast]", {
      hasText: "der neue Stand ließ sich nicht laden",
    });
    await expect(failed).toHaveCount(1);
    // Keine alten Pfade laden: kein Player, kein <audio>.
    await expect(page.getByTestId("rec-player")).toHaveCount(0);
    await expect(page.locator("audio")).toHaveCount(0);

    await page.evaluate(() => ((window as any).__getFail = null));
    await failed.getByRole("button", { name: "Erneut laden" }).click();
    await expect(page.getByTestId("rec-player")).toBeVisible();
    expect((await calls(page, "meetings_get")).length).toBe(2);
  });

  test("ein Fehler beim Laden (statt null) wird ebenso gemeldet", async ({
    page,
  }) => {
    await openM2(page);
    await page.evaluate(() => ((window as any).__getFail = "error"));
    await chooseMenu(page, "menu-trim");
    await setTime(page, "trim-range-start", "5:00");
    await page.getByTestId("trim-start").click();
    await expect(
      page.locator("[data-sonner-toast]", {
        hasText: "der neue Stand ließ sich nicht laden",
      }),
    ).toHaveCount(1);
    await expect(page.getByTestId("rec-player")).toHaveCount(0);
  });

  test("die Griffe des Doppelreglers haben bei 390 px eine echte Trefferfläche von 44 px", async ({
    page,
  }) => {
    await openM2(page, 390, 844);
    await chooseMenu(page, "menu-trim");
    await setTime(page, "trim-range-start", "10:00");
    await setTime(page, "trim-range-end", "50:00");
    const max = 3_540_000;
    for (const [which, ms] of [
      ["start", 600_000],
      ["end", 3_000_000],
    ] as const) {
      const id = `trim-range-slider-${which}`;
      const box = await field(page, id).boundingBox();
      expect(box).not.toBeNull();
      const thumb = 44;
      const cx = box!.x + thumb / 2 + (box!.width - thumb) * (ms / max);
      const cy = box!.y + box!.height / 2;
      const hits = await page.evaluate(
        ({ cx, cy, id }) => {
          const probe = (dx: number, dy: number) =>
            document
              .elementFromPoint(cx + dx, cy + dy)
              ?.closest("[data-testid]")
              ?.getAttribute("data-testid") ?? null;
          return {
            mitte: probe(0, 0),
            links: probe(-21, 0),
            rechts: probe(21, 0),
            oben: probe(0, -21),
            unten: probe(0, 21),
            weit: probe(40, 0),
            id,
          };
        },
        { cx, cy, id },
      );
      expect(hits.mitte, `${which} Mitte`).toBe(id);
      expect(hits.links, `${which} links`).toBe(id);
      expect(hits.rechts, `${which} rechts`).toBe(id);
      expect(hits.oben, `${which} oben`).toBe(id);
      expect(hits.unten, `${which} unten`).toBe(id);
      // Außerhalb der 44 px trifft der Griff nichts mehr.
      expect(hits.weit, `${which} weit`).not.toBe(id);
    }
  });

  test("Nachkommastellen: Von rundet ab, Bis rundet auf", async ({ page }) => {
    await openM2(page);
    await chooseMenu(page, "menu-trim");
    await setTime(page, "trim-range-start", "5:00,5");
    await setTime(page, "trim-range-end", "35:00,5");
    await expect(field(page, "trim-range-start")).toHaveValue("5:00");
    await expect(field(page, "trim-range-end")).toHaveValue("35:01");
    await page.getByTestId("trim-start").click();
    await expect(page.getByTestId("trim-dialog")).toHaveCount(0);
    const trim = await calls(page, "meetings_trim_audio");
    expect(trim[0].args).toEqual({
      meetingId: "m2",
      startMs: 300_000,
      endMs: 2_101_000,
    });
  });

  test("die Player lassen die Dateien los, bevor der Befehl läuft", async ({
    page,
  }) => {
    await openM2(page);
    await expect(page.locator("audio[src]")).not.toHaveCount(0);
    await chooseMenu(page, "menu-trim");
    await setTime(page, "trim-range-start", "5:00");
    await page.getByTestId("trim-start").click();
    await expect(page.getByTestId("trim-dialog")).toHaveCount(0);
    expect(await page.evaluate(() => (window as any).__audioAtTrim)).toBe(0);
  });

  test("gesperrte alte Dateien: Hinweis, dass sie beim nächsten Start gelöscht werden", async ({
    page,
  }) => {
    await openM2(page);
    await page.evaluate(() => ((window as any).__leftover = 2));
    await chooseMenu(page, "menu-trim");
    await setTime(page, "trim-range-start", "5:00");
    await page.getByTestId("trim-start").click();
    await expect(
      page.locator("[data-sonner-toast]", {
        hasText:
          "2 alte Audiodateien sind gerade gesperrt und werden beim nächsten Start der App gelöscht.",
      }),
    ).toHaveCount(1);
  });

  test("unsicheres Aufräumen: vorsichtiger Hinweis, nie stillschweigend", async ({
    page,
  }) => {
    await openM2(page);
    await page.evaluate(() => ((window as any).__uncertain = true));
    await chooseMenu(page, "menu-trim");
    await setTime(page, "trim-range-start", "5:00");
    await page.getByTestId("trim-start").click();
    await expect(
      page.locator("[data-sonner-toast]", {
        hasText:
          "Alte Audiodateien konnten nicht sicher entfernt werden; Nachholen beim nächsten Start.",
      }),
    ).toHaveCount(1);
  });

  for (const [code, text] of [
    [
      "metadata_unreadable",
      "Die Metadaten dieser Besprechung sind nicht lesbar. Es wurde nichts geändert und nichts gelöscht.",
    ],
    [
      "rerun_pending",
      "Erst die angehaltene Neu-Transkription fortsetzen oder verwerfen.",
    ],
  ] as const) {
    test(`Fehlercode ${code} erscheint übersetzt`, async ({ page }) => {
      await openM2(page);
      await page.evaluate((c) => ((window as any).__trimError = c), code);
      await chooseMenu(page, "menu-trim");
      await setTime(page, "trim-range-start", "1:00");
      await page.getByTestId("trim-start").click();
      await expect(page.locator("[data-sonner-toast]").first()).toContainText(
        text,
      );
      await expect(page.getByTestId("trim-dialog")).toBeVisible();
    });
  }

  test("bei einem Fehler kommen die Player zurück", async ({ page }) => {
    await openM2(page);
    await page.evaluate(() => ((window as any).__trimError = "trim_changed"));
    await chooseMenu(page, "menu-trim");
    await setTime(page, "trim-range-start", "1:00");
    await page.getByTestId("trim-start").click();
    await expect(page.getByTestId("trim-dialog")).toBeVisible();
    await expect(page.getByTestId("rec-player")).toBeVisible();
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
