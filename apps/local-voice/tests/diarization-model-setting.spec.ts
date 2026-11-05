import fs from "node:fs";
import { test, expect, type Page } from "@playwright/test";
import { calls, installTauriMock } from "./calendarMock";
import { geheZu } from "./profilmenue";

// #294: Modell der Sprechertrennung (Sortformer) per Knopf laden. Vorher gab
// es dafuer keinen Weg in der App. Codex-Auflagen: Fehlerzustand mit
// "Wiederholen", aria-live-Region, Fokus bleibt, Abbrechen, Ziel >= 44 px.
// Abnahmebilder: LV_SHOT_DIR_DIAR=… (Standard test-results/vorschau/sprechertrennung).
const SHOTS =
  process.env.LV_SHOT_DIR_DIAR ?? "test-results/vorschau/sprechertrennung";

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

type Status = {
  model_id: string;
  model_ready: boolean;
  downloading: boolean;
  size_mb: number;
};

const MISSING: Status = {
  model_id: "diar-sortformer-4spk-v2.1-q8",
  model_ready: false,
  downloading: false,
  size_mb: 140,
};

const open = async (
  page: Page,
  status: Status | "fail",
  width = 1280,
  theme: "light" | "dark" = "light",
) => {
  await installTauriMock(page, "main");
  await page.addInitScript(
    ({ initial, theme }) => {
      const w = window as any;
      w.__diarStatus = initial;
      const original = w.__TAURI_INTERNALS__.invoke;
      w.__TAURI_INTERNALS__.invoke = async (
        cmd: string,
        args: Record<string, unknown> = {},
      ) => {
        if (cmd === "get_app_settings" || cmd === "get_default_settings")
          return { ...(await original(cmd, args)), theme };
        if (cmd === "meeting_diarization_model_status") {
          w.__calls.push({ cmd, args });
          if (w.__diarStatus === "fail") throw "io";
          return w.__diarStatus;
        }
        if (cmd === "meeting_diarization_model_download") {
          w.__calls.push({ cmd, args });
          w.__diarStatus = { ...w.__diarStatus, model_ready: true };
          return null;
        }
        return original(cmd, args);
      };
    },
    { initial: status, theme },
  );
  await page.emulateMedia({ colorScheme: theme });
  await page.setViewportSize({ width, height: 1000 });
  await page.goto("/");
  await geheZu(page, "Einstellungen");
  await expect(page.getByTestId("diarization-model")).toBeAttached();
};

test("Knopf laedt das Modell, danach Installiert mit Fokus und Ansage", async ({
  page,
}) => {
  await open(page, MISSING);
  const button = page.getByTestId("diarization-model-download");
  await expect(button).toHaveText("Herunterladen (140 MB)");
  expect(await calls(page, "meeting_diarization_model_download")).toHaveLength(
    0,
  );
  await button.focus();
  await button.press("Enter");
  const state = page.getByTestId("diarization-model-state");
  await expect(state).toHaveText("Installiert");
  await expect(state).toBeFocused();
  expect(await calls(page, "meeting_diarization_model_download")).toHaveLength(
    1,
  );
  await expect(
    page.getByTestId("diarization-model").getByRole("status"),
  ).toHaveCount(1);
});

test("Fehler beim Statusabruf zeigt Wiederholen statt nichts", async ({
  page,
}) => {
  await open(page, "fail");
  await expect(page.getByTestId("diarization-model-retry")).toBeVisible();
  await page.evaluate(() => {
    (window as any).__diarStatus = {
      model_id: "diar-sortformer-4spk-v2.1-q8",
      model_ready: false,
      downloading: false,
      size_mb: 140,
    };
  });
  await page.getByTestId("diarization-model-retry").click();
  await expect(page.getByTestId("diarization-model-download")).toBeVisible();
});

test.describe("Abnahmebilder Sprechertrennung", () => {
  for (const width of [390, 1440]) {
    for (const theme of ["light", "dark"] as const) {
      test(`${width} px ${theme}`, async ({ page }) => {
        await open(page, MISSING, width, theme);
        const button = page.getByTestId("diarization-model-download");
        await button.scrollIntoViewIfNeeded();
        await expect(button).toBeVisible();
        const box = await button.boundingBox();
        expect(box!.height).toBeGreaterThanOrEqual(44);
        expect(box!.x + box!.width).toBeLessThanOrEqual(width);
        const overflow = await page.evaluate(
          () =>
            document.documentElement.scrollWidth >
            document.documentElement.clientWidth,
        );
        expect(overflow).toBe(false);
        await page.mouse.move(0, 0);
        fs.mkdirSync(SHOTS, { recursive: true });
        fs.appendFileSync(
          `${SHOTS}/masse.txt`,
          `Sprechertrennung ${width} ${theme}: Knopf ${Math.round(box!.width)}x${Math.round(box!.height)}\n`,
        );
        await page.screenshot({
          animations: "disabled",
          path: `${SHOTS}/einstellung-${width}-${theme === "light" ? "hell" : "dunkel"}.png`,
        });
      });
    }
  }
});
