import { test, expect } from "@playwright/test";
import { installTauriMock } from "./calendarMock";

// Der Toast im Hauptfenster zum Ereignis `paste-fallback` (zweiter Kanal neben
// dem Overlay-Hinweis). Bei Segment-Modus und Live-Injektion (D14) ist nur der
// REST nicht eingefuegt worden: der Text darf nicht behaupten, der vollstaendige
// Text liege in der Zwischenablage.

const emitFallback = async (
  page: import("@playwright/test").Page,
  payload: Record<string, unknown>,
) => {
  await page.goto("/");
  await page.waitForFunction(() =>
    (
      window as unknown as {
        __calls: { cmd: string; args: { event?: string } }[];
      }
    ).__calls.some(
      (call) =>
        call.cmd === "plugin:event|listen" &&
        call.args.event === "paste-fallback",
    ),
  );
  await page.evaluate(
    (data) =>
      (window as unknown as { __emit: (e: string, p: unknown) => void }).__emit(
        "paste-fallback",
        data,
      ),
    payload,
  );
};

test("a partial fallback toast says the REST is in the clipboard", async ({
  page,
}) => {
  await installTauriMock(page);
  await emitFallback(page, {
    reason: "live_focus_changed",
    transcript_in_clipboard: true,
    partial: true,
  });

  const toast = page.locator("[data-sonner-toast]");
  await expect(toast).toContainText("Rest des Diktats wurde nicht eingefügt");
  await expect(toast).toContainText(
    "Der Fokus wechselte während des Diktierens",
  );
  await expect(toast).toContainText("Der Rest liegt in der Zwischenablage");
  await expect(toast).not.toContainText("vollständige Text liegt");
});

test("a partial fallback without clipboard points at the history", async ({
  page,
}) => {
  await installTauriMock(page);
  await emitFallback(page, {
    reason: "clipboard_unverified",
    transcript_in_clipboard: false,
    partial: true,
  });

  const toast = page.locator("[data-sonner-toast]");
  await expect(toast).toContainText("Der vollständige Text steht im Verlauf");
});

test("a whole-dictation fallback keeps the existing toast", async ({
  page,
}) => {
  await installTauriMock(page);
  await emitFallback(page, {
    reason: "focus_changed",
    transcript_in_clipboard: true,
  });

  const toast = page.locator("[data-sonner-toast]");
  await expect(toast).toContainText("Text wurde nicht eingefügt");
  await expect(toast).toContainText("Der vollständige Text liegt in der");
});
