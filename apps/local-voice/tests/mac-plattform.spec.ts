import { test, expect, type Page } from "@playwright/test";
import { installTauriMock } from "./calendarMock";

// Mac-Befunde aus dem Sichttest vor 0.23.0: Tastenkuerzel in Mac-Symbolen,
// kein Windows-Explorer-Schalter auf dem Mac. Windows bleibt unveraendert.

test.use({ locale: "de-DE" });

const openOutput = async (page: Page, os: "macos" | "windows") => {
  await installTauriMock(page, "main");
  await page.addInitScript((osType) => {
    (
      window as unknown as Record<string, unknown>
    ).__TAURI_OS_PLUGIN_INTERNALS__ = {
      platform: osType,
      os_type: osType,
      family: osType === "windows" ? "windows" : "unix",
      arch: "x86_64",
      version: "1",
      eol: osType === "windows" ? "\r\n" : "\n",
    };
    // Die Titelleiste erkennt den Mac an navigator.platform.
    Object.defineProperty(navigator, "platform", {
      get: () => (osType === "macos" ? "MacIntel" : "Win32"),
    });
  }, os);
  // Das Vorlese-Kuerzel wie in den echten Standardeinstellungen.
  await page.addInitScript(() => {
    const w = window as unknown as { __settings: Record<string, unknown> };
    w.__settings.bindings = {
      speak_clipboard: {
        id: "speak_clipboard",
        name: "Speak Clipboard",
        description: "Reads the current clipboard text aloud.",
        default_binding: "ctrl+alt+space",
        current_binding: "ctrl+alt+space",
      },
    };
  });
  await page.setViewportSize({ width: 1280, height: 1000 });
  await page.goto("/");
  await page
    .getByRole("button", { name: "Einstellungen", exact: true })
    .first()
    .click();
  await page.getByRole("tab", { name: "Ausgabe", exact: true }).click();
};

test("macOS: Kuerzel als Mac-Symbole, kein Explorer-Kontextmenue", async ({
  page,
}) => {
  await openOutput(page, "macos");
  await expect(page.getByText("⌃ ⌥ Space", { exact: true })).toBeVisible();
  await expect(page.getByText("Ctrl + Alt + Space")).toHaveCount(0);
  await expect(page.getByText("Explorer-Kontextmenü")).toHaveCount(0);
});

test("Windows: Kuerzel mit Namen, Explorer-Kontextmenue sichtbar", async ({
  page,
}) => {
  await openOutput(page, "windows");
  await expect(
    page.getByText("Ctrl + Alt + Space", { exact: true }),
  ).toBeVisible();
  await expect(page.getByText("Explorer-Kontextmenü")).toBeVisible();
});

const openShortcutDialog = async (page: Page) => {
  await page.getByRole("button", { name: "Hilfe", exact: true }).click();
  await page.getByRole("menuitem", { name: "Tastenkürzel" }).click();
  return page.getByRole("dialog", { name: "Tastenkürzel" });
};

test("macOS: der Kuerzel-Dialog der Titelleiste zeigt Mac-Symbole", async ({
  page,
}) => {
  await openOutput(page, "macos");
  const dialog = await openShortcutDialog(page);
  await expect(dialog.getByText("⌃ ⌥ Space", { exact: true })).toBeVisible();
  await expect(dialog.getByText("ctrl+alt+space")).toHaveCount(0);
});

test("Windows: der Kuerzel-Dialog der Titelleiste zeigt Namen", async ({
  page,
}) => {
  await openOutput(page, "windows");
  const dialog = await openShortcutDialog(page);
  await expect(
    dialog.getByText("Ctrl + Alt + Space", { exact: true }),
  ).toBeVisible();
});
