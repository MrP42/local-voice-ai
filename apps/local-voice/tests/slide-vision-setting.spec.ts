import { test, expect, type Page } from "@playwright/test";
import { calls, installTauriMock } from "./calendarMock";

// D3 (Goal Issues-Abschluss #70, M7 = #69): die Einstellung "Bildanalyse für Folien" in
// der Gruppe "Besprechungen" (Reiter Diktat). Gegen die Tauri-Attrappe: Standard aus,
// Download des Projektors nur per Knopf, und der Grund, wenn sie nicht angeboten wird
// (keine Grafikkarte, zu wenig Grafikspeicher).

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

type Status = {
  enabled: boolean;
  model_ready: boolean;
  projector_ready: boolean;
  downloading: boolean;
  projector_size_mb: number;
  availability: string;
};

const READY: Status = {
  enabled: false,
  model_ready: true,
  projector_ready: true,
  downloading: false,
  projector_size_mb: 944,
  availability: "ready",
};

/** Die Attrappe kennt die Befehle der Bildanalyse nicht von selbst: hier nachgerüstet. */
const open = async (page: Page, status: Status) => {
  await installTauriMock(page, "main");
  await page.addInitScript((initial) => {
    const w = window as any;
    w.__visionStatus = initial;
    const original = w.__TAURI_INTERNALS__.invoke;
    w.__TAURI_INTERNALS__.invoke = async (
      cmd: string,
      args: Record<string, unknown> = {},
    ) => {
      if (cmd === "meeting_slide_vision_status") {
        w.__calls.push({ cmd, args });
        return w.__visionStatus;
      }
      if (cmd === "meeting_slide_vision_download") {
        w.__calls.push({ cmd, args });
        w.__visionStatus = { ...w.__visionStatus, projector_ready: true };
        return null;
      }
      if (cmd === "change_meeting_slide_vision_setting") {
        w.__calls.push({ cmd, args });
        return null;
      }
      return original(cmd, args);
    };
  }, status);
  await page.setViewportSize({ width: 1280, height: 1000 });
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Einstellungen", exact: true })
    .click();
  await expect(page.getByTestId("slide-vision-toggle")).toBeAttached();
};

test("Bildanalyse ist standardmäßig aus und lässt sich einschalten", async ({
  page,
}) => {
  await open(page, READY);
  const toggle = page.getByTestId("slide-vision-toggle");
  await expect(toggle).not.toBeChecked();
  await expect(page.getByTestId("slide-vision-state")).toHaveText("Bereit");
  await page
    .locator("label", { has: page.getByTestId("slide-vision-toggle") })
    .click();
  await expect(toggle).toBeChecked();
  const sent = await calls(page, "change_meeting_slide_vision_setting");
  expect(sent.map((c) => c.args)).toEqual([{ enabled: true }]);
});

test("ohne Projektor gibt es nur den Download-Knopf, nie einen automatischen Download", async ({
  page,
}) => {
  await open(page, { ...READY, projector_ready: false });
  expect(await calls(page, "meeting_slide_vision_download")).toHaveLength(0);
  const button = page.getByTestId("slide-vision-download");
  await expect(button).toHaveText("Bild-Projektor laden (944 MB)");
  await button.click();
  await expect(page.getByTestId("slide-vision-state")).toHaveText("Bereit");
  expect(await calls(page, "meeting_slide_vision_download")).toHaveLength(1);
});

test("ohne Grafikkarte wird die Bildanalyse nicht angeboten", async ({
  page,
}) => {
  await open(page, { ...READY, availability: "vision_no_gpu" });
  await expect(page.getByTestId("slide-vision-toggle")).toBeDisabled();
  await expect(page.getByTestId("slide-vision-state")).toContainText(
    "Keine geeignete Grafikkarte",
  );
  // Weder Download-Knopf noch Änderung der Einstellung.
  await expect(page.getByTestId("slide-vision-download")).toHaveCount(0);
  expect(await calls(page, "change_meeting_slide_vision_setting")).toHaveLength(
    0,
  );
});

test("zu wenig freier Grafikspeicher nennt den Grund", async ({ page }) => {
  await open(page, { ...READY, availability: "vision_low_vram" });
  await expect(page.getByTestId("slide-vision-toggle")).toBeDisabled();
  await expect(page.getByTestId("slide-vision-state")).toContainText(
    "mindestens 6 GB",
  );
});

test("fehlendes Modell verweist auf die Modellliste", async ({ page }) => {
  await open(page, { ...READY, model_ready: false });
  await expect(page.getByTestId("slide-vision-state")).toContainText(
    "Gemma 4 E4B fehlt",
  );
});
