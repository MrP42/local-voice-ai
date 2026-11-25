import { test, expect } from "@playwright/test";
import { installTauriMock, calls } from "./calendarMock";
import { geheZu } from "./profilmenue";

test("alte deutsche Besprechungseinstellung vererbt sich nicht; beide Hörbuch-Felder speichern separat", async ({ page }) => {
  await installTauriMock(page);
  await page.addInitScript(() => {
    const w = window as unknown as {
      __settings: Record<string, unknown>;
      __calls: { cmd: string; args: Record<string, unknown> }[];
      __TAURI_INTERNALS__: { invoke(cmd: string, args?: Record<string, unknown>): Promise<unknown> };
    };
    w.__settings.meeting_language = "de";
    w.__settings.meeting_model = "synthetic_meeting";
    const original = w.__TAURI_INTERNALS__.invoke;
    w.__TAURI_INTERNALS__.invoke = async (cmd, args = {}) => {
      if (cmd === "get_available_models") return [{
        id: "synthetic_turbo", name: "Synthetic turbo Q8", is_downloaded: true,
        supported_languages: ["en", "de"], engine_type: "TranscribeCpp",
      }];
      if (cmd === "change_audiobook_model_setting" || cmd === "change_audiobook_language_setting") {
        w.__calls.push({ cmd, args });
        if (cmd === "change_audiobook_model_setting") w.__settings.audiobook_model = args.model;
        else w.__settings.audiobook_language = args.language;
        return null;
      }
      return original(cmd, args);
    };
  });
  await page.goto("/");
  await geheZu(page, "Einstellungen");
  const language = page.getByRole("combobox", { name: "Hörbuch-Sprache", exact: true });
  await expect(language).toHaveText("Automatisch (Buchmetadaten, dann Erkennung)");
  await language.click();
  await page.getByRole("option", { name: "English", exact: true }).click();
  await expect.poll(() => calls(page, "change_audiobook_language_setting"))
    .toEqual([{ cmd: "change_audiobook_language_setting", args: { language: "en" } }]);
  const model = page.getByRole("combobox", { name: "Hörbuch-Modell", exact: true });
  await model.click();
  await page.getByRole("option", { name: "Synthetic turbo Q8", exact: true }).click();
  await expect.poll(() => calls(page, "change_audiobook_model_setting"))
    .toEqual([{ cmd: "change_audiobook_model_setting", args: { model: "synthetic_turbo" } }]);
  await model.click();
  await page.getByRole("option", { name: "Automatisch (bevorzugt Whisper turbo Q8)", exact: true }).click();
  await expect.poll(async () => (await calls(page, "change_audiobook_model_setting")).at(-1)?.args)
    .toEqual({ model: null });
  expect(await calls(page, "change_meeting_language_setting")).toEqual([]);
  expect(await calls(page, "change_meeting_model_setting")).toEqual([]);
  expect(await page.evaluate(() => (window as unknown as { __settings: Record<string, unknown> }).__settings.meeting_language)).toBe("de");
});
