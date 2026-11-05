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
          if (w.__diarSlow) {
            // Laeuft, bis "Abbrechen" kommt (Teildatei bleibt liegen).
            await new Promise<void>((resolve) => (w.__diarRelease = resolve));
            throw "cancelled";
          }
          w.__diarStatus = { ...w.__diarStatus, model_ready: true };
          return null;
        }
        if (cmd === "meeting_diarization_model_cancel") {
          w.__calls.push({ cmd, args });
          w.__diarRelease?.();
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

test("Abbrechen gibt den Fokus an den Download-Knopf zurueck", async ({
  page,
}) => {
  await open(page, MISSING, 390);
  await page.evaluate(() => ((window as any).__diarSlow = true));
  await page.getByTestId("diarization-model-download").click();
  const cancel = page.getByTestId("diarization-model-cancel");
  await expect(cancel).toBeVisible();
  await expect(page.getByTestId("diarization-model-download")).toBeDisabled();
  fs.mkdirSync(SHOTS, { recursive: true });
  await page.mouse.move(0, 0);
  await page
    .getByTestId("diarization-model")
    .screenshot({ path: `${SHOTS}/laden-390-hell.png` });
  await cancel.focus();
  await cancel.press("Enter");
  await expect(cancel).toHaveCount(0);
  const button = page.getByTestId("diarization-model-download");
  await expect(button).toBeEnabled();
  await expect(button).toBeFocused();
  await expect(
    page.getByTestId("diarization-model").getByRole("status"),
  ).toHaveText(/^Download abgebrochen\./);
});

test("Fokus woanders: kein Zurueckholen nach dem Laden", async ({ page }) => {
  await open(page, MISSING);
  await page.evaluate(() => ((window as any).__diarSlow = true));
  await page.getByTestId("diarization-model-download").click();
  await expect(page.getByTestId("diarization-model-cancel")).toBeVisible();
  // Nutzer geht weiter, waehrend geladen wird.
  await page.evaluate(() => {
    const b = document.createElement("button");
    b.id = "anderswo";
    b.textContent = "anderswo";
    document.body.appendChild(b);
    b.focus();
  });
  await page.evaluate(() => (window as any).__diarRelease());
  await expect(page.getByTestId("diarization-model-download")).toBeEnabled();
  await expect(page.locator("#anderswo")).toBeFocused();
});

test("Statusfehler nach frueherem Erfolg bleibt sichtbar und wiederholbar", async ({
  page,
}) => {
  await open(page, MISSING, 390);
  await expect(page.getByTestId("diarization-model-download")).toBeVisible();
  await page.evaluate(() => {
    const w = window as any;
    w.__diarStatus = "fail";
    w.__diarSlow = true;
  });
  // Laden abbrechen loest einen neuen Statusabruf aus, der jetzt scheitert.
  await page.getByTestId("diarization-model-download").click();
  await page.getByTestId("diarization-model-cancel").focus();
  await page.getByTestId("diarization-model-cancel").press("Enter");
  const retry = page.getByTestId("diarization-model-retry");
  await expect(retry).toBeVisible();
  await expect(page.getByTestId("diarization-model-cancel")).toHaveCount(0);
  await page.mouse.move(0, 0);
  await page
    .getByTestId("diarization-model")
    .screenshot({ path: `${SHOTS}/fehler-390-hell.png` });
  await page.evaluate(() => {
    (window as any).__diarStatus = {
      model_id: "diar-sortformer-4spk-v2.1-q8",
      model_ready: false,
      downloading: false,
      size_mb: 140,
    };
  });
  await retry.click();
  await expect(retry).toHaveCount(0);
  await expect(page.getByTestId("diarization-model-download")).toBeVisible();
});

test("Wiederholen bei laufendem Download fokussiert Abbrechen", async ({
  page,
}) => {
  await open(page, "fail");
  const retry = page.getByTestId("diarization-model-retry");
  await expect(retry).toBeVisible();
  await page.evaluate(() => {
    (window as any).__diarStatus = {
      model_id: "diar-sortformer-4spk-v2.1-q8",
      model_ready: false,
      downloading: true,
      size_mb: 140,
    };
  });
  await retry.focus();
  await retry.press("Enter");
  await expect(retry).toHaveCount(0);
  await expect(page.getByTestId("diarization-model-download")).toBeDisabled();
  await expect(page.getByTestId("diarization-model-cancel")).toBeFocused();
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
