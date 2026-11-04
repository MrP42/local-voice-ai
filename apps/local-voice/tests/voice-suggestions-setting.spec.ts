import fs from "node:fs";
import { test, expect, type Page } from "@playwright/test";
import { calls, installTauriMock } from "./calendarMock";
import { geheZu } from "./profilmenue";

// Stimmprofile 2a: Einstellung "Gleiche Stimmen erkennen" in der Gruppe
// "Besprechungen". Standard an, das Stimmmodell (CAM++) nur per Knopf.
// Abnahmebilder: LV_SHOT_DIR_VOICE=docs/vorschau/stimmen-vorschlag.
const SHOTS =
  process.env.LV_SHOT_DIR_VOICE ?? "test-results/vorschau/stimmen-vorschlag";

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

type Status = {
  enabled: boolean;
  model_ready: boolean;
  downloading: boolean;
  size_mb: number;
};

const MISSING: Status = {
  enabled: true,
  model_ready: false,
  downloading: false,
  size_mb: 30,
};

const open = async (
  page: Page,
  status: Status,
  width = 1280,
  theme: "light" | "dark" = "light",
) => {
  await installTauriMock(page, "main");
  await page.addInitScript((initial) => {
    const w = window as any;
    w.__voiceStatus = initial;
    const original = w.__TAURI_INTERNALS__.invoke;
    w.__TAURI_INTERNALS__.invoke = async (
      cmd: string,
      args: Record<string, unknown> = {},
    ) => {
      if (cmd === "meeting_voiceprint_status") {
        w.__calls.push({ cmd, args });
        return w.__voiceStatus;
      }
      if (cmd === "meeting_voiceprint_download") {
        w.__calls.push({ cmd, args });
        w.__voiceStatus = { ...w.__voiceStatus, model_ready: true };
        return null;
      }
      if (cmd === "change_meeting_voice_suggestions_setting") {
        w.__calls.push({ cmd, args });
        w.__voiceStatus = { ...w.__voiceStatus, enabled: args.enabled };
        return null;
      }
      return original(cmd, args);
    };
  }, status);
  await page.emulateMedia({ colorScheme: theme });
  await page.setViewportSize({ width, height: 1000 });
  await page.goto("/");
  await geheZu(page, "Einstellungen");
  await expect(page.getByTestId("voiceprint-toggle")).toBeAttached();
};

test("Standard an, Modell nur per Knopf, Ausschalten blendet die Modellzeile aus", async ({
  page,
}) => {
  await open(page, MISSING);
  const toggle = page.getByTestId("voiceprint-toggle");
  await expect(toggle).toBeChecked();
  expect(await calls(page, "meeting_voiceprint_download")).toHaveLength(0);
  const button = page.getByTestId("voiceprint-download");
  await expect(button).toHaveText("Stimmmodell laden (30 MB)");
  await button.click();
  await expect(page.getByTestId("voiceprint-state")).toHaveText(
    "Stimmmodell CAM++ ist installiert (Apache-2.0).",
  );
  expect(await calls(page, "meeting_voiceprint_download")).toHaveLength(1);
  await toggle.click();
  await expect(toggle).not.toBeChecked();
  const sent = await calls(page, "change_meeting_voice_suggestions_setting");
  expect(sent.map((c) => c.args)).toEqual([{ enabled: false }]);
  await expect(page.getByTestId("voiceprint-state")).toHaveCount(0);
});

test.describe("Abnahmebilder Einstellung", () => {
  for (const width of [390, 1440]) {
    for (const theme of ["light", "dark"] as const) {
      test(`${width} px ${theme}`, async ({ page }) => {
        await open(page, MISSING, width, theme);
        const toggle = page.getByTestId("voiceprint-toggle");
        await toggle.scrollIntoViewIfNeeded();
        const button = page.getByTestId("voiceprint-download");
        await expect(button).toBeVisible();
        const box = await button.boundingBox();
        expect(box!.x + box!.width).toBeLessThanOrEqual(width);
        const overflow = await page.evaluate(
          () =>
            document.documentElement.scrollWidth >
            document.documentElement.clientWidth,
        );
        expect(overflow).toBe(false);
        await page.mouse.move(0, 0);
        fs.mkdirSync(SHOTS, { recursive: true });
        await page.screenshot({
          animations: "disabled",
          path: `${SHOTS}/4-einstellung-${width}-${theme === "light" ? "hell" : "dunkel"}.png`,
        });
      });
    }
  }
});
