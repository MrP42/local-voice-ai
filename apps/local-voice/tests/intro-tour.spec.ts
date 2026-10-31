import { test, expect } from "@playwright/test";
import {
  NEMOTRON,
  commands,
  emit,
  finishDownload,
  setup,
} from "./introTourMock";

// Erstinstallation: Diktatmodell (Nemotron 3.5) lädt ohne Auswahlbildschirm,
// die Einführungstour zeigt Fortschritt und Seiten.
test("Erstinstallation lädt Nemotron 3.5 und startet die Einführungstour", async ({
  page,
}) => {
  await setup(page);
  await page.goto("/");

  const tour = page.getByTestId("intro-tour");
  const dialog = tour.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await expect(
    dialog.getByRole("heading", { name: "Schön, dass du da bist" }),
  ).toBeVisible();
  // Kein Auswahlbildschirm: Download ohne Rückfrage gestartet.
  await expect
    .poll(async () =>
      (await commands(page)).some(
        (c) =>
          c.cmd === "download_model" &&
          (c.args as { modelId?: string }).modelId === NEMOTRON,
      ),
    )
    .toBe(true);

  // Fortschritt als progressbar; die Tour beginnt erst mit bereitem Modell.
  await emit(page, "model-download-progress", {
    model_id: NEMOTRON,
    downloaded: 40,
    total: 100,
    percentage: 40,
  });
  const bar = dialog.getByRole("progressbar");
  await expect(bar).toHaveAttribute("aria-valuenow", "40");
  await expect(bar).toHaveAttribute(
    "aria-valuetext",
    /Nemotron 3.5 ASR Streaming wird eingerichtet … 40 %/,
  );
  const start = dialog.getByRole("button", { name: "Tour starten" });
  await expect(start).toBeDisabled();
  await page.keyboard.press("ArrowRight");
  await expect(
    dialog.getByRole("heading", { name: "Schön, dass du da bist" }),
  ).toBeVisible();

  // Download fertig: Modell wird zum Diktatmodell, Bereitschaft wird angesagt.
  await finishDownload(page);
  await expect
    .poll(async () =>
      (await commands(page)).some(
        (c) =>
          c.cmd === "set_active_model" &&
          (c.args as { modelId?: string }).modelId === NEMOTRON,
      ),
    )
    .toBe(true);
  await expect(tour.getByRole("alert")).toContainText("ist bereit");
  await expect(start).toBeEnabled();
  await expect(start).toBeFocused();

  // Diktieren mit Kürzel aus den Einstellungen; Schrittwechsel wird angesagt.
  await start.click();
  await expect(
    tour.getByRole("heading", { name: "In Textfelder diktieren" }),
  ).toBeVisible();
  await expect(tour).toContainText("Strg + Leertaste gedrückt halten");
  await expect(tour.getByRole("status")).toHaveText(
    "2/6: In Textfelder diktieren",
  );
  // Fokus liegt auf „Weiter“: Pfeiltasten schalten dann nicht weiter.
  const weiter = tour.getByRole("button", { name: "Weiter" });
  await expect(weiter).toBeFocused();
  await page.keyboard.press("ArrowRight");
  await expect(
    tour.getByRole("heading", { name: "In Textfelder diktieren" }),
  ).toBeVisible();

  // Übungsfeld: beschriftet, freigegeben, fokussiert.
  await weiter.click();
  const field = tour.getByRole("textbox", { name: "Übungsfeld zum Diktieren" });
  await expect(field).toBeEnabled();
  await expect(field).toBeFocused();
  // Während einer Aufnahme bricht Esc die Aufnahme ab, die Tour bleibt.
  await page.evaluate(() => {
    (window as unknown as { __recording: boolean }).__recording = true;
  });
  await page.keyboard.press("Escape");
  await expect
    .poll(async () =>
      (await commands(page)).some((c) => c.cmd === "is_recording"),
    )
    .toBe(true);
  await expect(field).toBeVisible();
  await page.evaluate(() => {
    (window as unknown as { __recording: boolean }).__recording = false;
  });

  // Fokusfalle: Tab bleibt im Dialog.
  for (let i = 0; i < 6; i++) await page.keyboard.press("Tab");
  expect(
    await page.evaluate(() =>
      document.querySelector("[role=dialog]")?.contains(document.activeElement),
    ),
  ).toBe(true);

  // Seiten im Spotlight.
  await tour.getByRole("button", { name: "Weiter" }).click();
  await expect(tour).toHaveAttribute("data-mode", "spot");
  await expect(
    tour.getByRole("heading", { name: "Alles auf einen Blick" }),
  ).toBeVisible();

  // Esc schließt die Tour; Fokus landet auf einem sinnvollen Ziel.
  await page.keyboard.press("Escape");
  await expect(tour).toHaveCount(0);
  const again = page.getByTestId("workspace-start-tour");
  await expect(again).toBeFocused();

  // Erneut starten löst keinen weiteren Download aus.
  const downloads = (await commands(page)).filter(
    (c) => c.cmd === "download_model",
  ).length;
  await again.click();
  await expect(
    page.getByRole("heading", { name: "Schön, dass du da bist" }),
  ).toBeVisible();
  await expect(
    page
      .getByTestId("intro-tour")
      .getByRole("button", { name: "Tour starten" }),
  ).toBeEnabled();
  expect(
    (await commands(page)).filter((c) => c.cmd === "download_model").length,
  ).toBe(downloads);
});

test("Gescheiterter Download: Wiederholen in der Tour", async ({ page }) => {
  await setup(page, { downloadFails: 1 });
  await page.goto("/");
  const tour = page.getByTestId("intro-tour");
  await expect(tour.getByRole("alert")).toContainText(
    "konnte nicht geladen werden",
  );
  await expect(
    tour.getByRole("button", { name: "Tour starten" }),
  ).toBeDisabled();
  await tour.getByRole("button", { name: "Download wiederholen" }).click();
  await expect(tour.getByRole("progressbar")).toBeVisible();
  expect(
    (await commands(page)).filter((c) => c.cmd === "download_model").length,
  ).toBe(2);
  await finishDownload(page);
  await expect(
    tour.getByRole("button", { name: "Tour starten" }),
  ).toBeEnabled();
});

test("Bewegung reduzieren zeigt das Diktat als Endzustand", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await setup(page);
  await page.goto("/");
  const tour = page.getByTestId("intro-tour");
  await finishDownload(page);
  await tour.getByRole("button", { name: "Tour starten" }).click();
  await expect(tour).toHaveAttribute("data-reduced", "true");
  await expect(tour.locator(".lv-tour-anim__field")).toHaveText(
    "Sehr geehrte Frau Berger, vielen Dank für Ihre Nachricht.",
  );
});

test("Installer ohne Diktatmodell zeigt die Modellauswahl", async ({
  page,
}) => {
  await setup(page, { erststart: "[diktat]\r\nmodell=aus\r\n" });
  await page.goto("/");
  await expect(page.getByText("Nemotron 3.5 ASR Streaming")).toBeVisible();
  await expect(page.getByTestId("intro-tour")).toHaveCount(0);
  expect((await commands(page)).some((c) => c.cmd === "download_model")).toBe(
    false,
  );
});

test("Bestehende Installation startet ohne Tour", async ({ page }) => {
  await setup(page, { onboardingCompleted: true });
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "Was möchtest du tun?" }),
  ).toBeVisible();
  await expect(page.getByTestId("intro-tour")).toHaveCount(0);
});

test("Anderes Modell auswählen führt zur Modellauswahl", async ({ page }) => {
  await setup(page, { downloadFails: 5 });
  await page.goto("/");
  const tour = page.getByTestId("intro-tour");
  await tour.getByRole("button", { name: "Anderes Modell auswählen" }).click();
  await expect(page.getByTestId("intro-tour")).toHaveCount(0);
  await expect(page.getByText("Nemotron 3.5 ASR Streaming")).toBeVisible();
});
