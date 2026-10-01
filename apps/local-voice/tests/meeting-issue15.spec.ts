import { test, expect } from "@playwright/test";
import * as fs from "node:fs";
import * as path from "node:path";
import { translateMeetingError } from "../src/components/settings/meetings/meetingErrors";
import {
  calls,
  emitMeeting,
  openRecordings,
  pickMeeting,
} from "./recLayoutMock";
import { installLiveMock, startRecording } from "./recLiveMock";

// #15 (M8-Follow-up-Backlog), Frontend-Punkte: veraltetes `t()` im Ereignis-
// Listener der Aufnahmekarte (Sprachwechsel), Standardtitel statt Platzhaltertext,
// System-Audio als ToggleSwitch (CI), Fehlerkonvention (Code vor dem Doppelpunkt)
// und ein nicht mehr stummes Speichern einer Segmentkorrektur.

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

test.beforeEach(async ({ page }) => {
  await installLiveMock(page);
});

const locale = (lang: string) =>
  JSON.parse(
    fs.readFileSync(
      path.resolve(process.cwd(), `src/i18n/locales/${lang}/translation.json`),
      "utf-8",
    ),
  );

/** `t` auf einer Locale-Datei: der Schluessel selbst, wenn es ihn nicht gibt. */
const tOf = (lang: string) => {
  const root = locale(lang);
  return ((key: string) =>
    key.split(".").reduce<any>((node, part) => node?.[part], root) ??
    `FEHLT:${key}`) as any;
};

// ---------------------------------------------------------------------------
// Fehlerkonvention
// ---------------------------------------------------------------------------

test("Fehlercodes werden ueber den Code vor dem Doppelpunkt uebersetzt, in beiden Sprachen", () => {
  for (const lang of ["de", "en"]) {
    const t = tOf(lang);
    for (const code of [
      "meetings_unavailable",
      "store_failed",
      "subtitle_unreadable",
      "subtitle_invalid",
    ]) {
      const plain = translateMeetingError(code, t);
      expect(plain, `${lang}: ${code}`).not.toContain("FEHLT:");
      expect(plain, `${lang}: ${code}`).not.toBe(code);
      // Mit Detail (englische Ursache der Datenbank): derselbe Satz, die Ursache bleibt weg.
      const detailed = translateMeetingError(
        `${code}: no such table: transcripts`,
        t,
      );
      expect(detailed).toBe(plain);
      expect(detailed).not.toContain("transcripts");
    }
  }
  // Ein unbekannter Code bleibt sichtbar (Fehlerbericht), statt verschluckt zu werden.
  expect(translateMeetingError("irgendwas_neues: x", tOf("de"))).toBe(
    "irgendwas_neues: x",
  );
});

test("Segmentkorrektur: ein Fehler beim Speichern wird gemeldet und die Eingabe bleibt offen", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const w = window as any;
    const inner = w.__TAURI_INTERNALS__.invoke;
    w.__TAURI_INTERNALS__.invoke = async (
      cmd: string,
      args: Record<string, unknown>,
    ) => {
      if (cmd === "meetings_update_segment") {
        (w.__calls ??= []).push({ cmd, args });
        throw "store_failed: no such table: transcripts";
      }
      return inner(cmd, args);
    };
  });
  await openRecordings(page, 1366, 768);
  await pickMeeting(page, "m1");
  const first = page.locator("[data-segment-index]").first();
  await first.hover();
  await first.getByTitle("Segment bearbeiten").click();
  await first.getByRole("textbox").fill("Korrigierter Text");
  await first.getByRole("button", { name: "Speichern" }).click();
  await expect
    .poll(async () => (await calls(page, "meetings_update_segment")).length)
    .toBe(1);
  const toast = page.locator("[data-sonner-toast]");
  await expect(toast).toContainText(
    "Die Änderung konnte nicht gespeichert werden.",
  );
  await expect(toast).not.toContainText("transcripts");
  // Nichts ging verloren: das Eingabefeld ist noch da, mit dem getippten Text.
  await expect(first.getByRole("textbox")).toHaveValue("Korrigierter Text");
});

// ---------------------------------------------------------------------------
// Aufnahmekarte
// ---------------------------------------------------------------------------

/** Die Sprache der laufenden App wie die Sprachauswahl der Einstellungen wechseln. */
const switchLanguage = (page: import("@playwright/test").Page, lang: string) =>
  page.evaluate(async (l) => {
    const i18n = (await import("/src/i18n/index.ts" as string)).default;
    await i18n.changeLanguage(l);
  }, lang);

test("ein Fehlerereignis nach einem Sprachwechsel erscheint in der neuen Sprache (kein veraltetes t)", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  await expect(page.getByTestId("rec-start")).toBeVisible();
  await switchLanguage(page, "en");
  await expect(
    page.getByRole("button", { name: /Start recording/ }).first(),
  ).toBeVisible();

  // Der Listener der Karte wurde VOR dem Wechsel registriert.
  await emitMeeting(page, {
    kind: "error",
    meeting_id: "m1",
    message: "dictation_active",
  });
  const text = tOf("en")("meetings.errors.dictationActive");
  await expect(page.getByText(text)).toBeVisible();
  await expect(
    page.getByText(tOf("de")("meetings.errors.dictationActive")),
  ).toHaveCount(0);
});

test("eine Aufnahme ohne Titel heisst wie ein neuer Eintrag, nicht wie der Platzhaltertext des Feldes", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  await startRecording(page);
  await expect
    .poll(async () => (await calls(page, "meetings_start")).length)
    .toBe(1);
  const args = (await calls(page, "meetings_start"))[0].args as any;
  expect(args.title).toBe("Neue Besprechung");
  expect(args.title).not.toBe("Titel der Besprechung");
});

// ---------------------------------------------------------------------------
// System-Audio als ToggleSwitch (CI)
// ---------------------------------------------------------------------------

test("System-Audio im Startdialog ist ein ToggleSwitch mit Beschreibung, kein rohes Kontrollkaestchen", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  await page
    .getByRole("button", { name: /Aufnahme starten/ })
    .first()
    .click();
  const dialog = page.getByRole("dialog", { name: "Aufnahme starten" });
  const box = dialog.getByTestId("capture-system");
  const toggle = dialog
    .locator("label")
    .filter({ has: page.getByTestId("capture-system") });
  await expect(box).toBeChecked();
  // Der Schalter der Oberflaeche (Design-System), nicht das Browser-Kontrollkaestchen.
  await expect(box).toHaveClass(/sr-only/);
  await expect(box).not.toHaveClass(/accent-logo-primary/);
  await expect(toggle).toBeVisible();
  await expect(dialog).toContainText("System-Audio (Gegenseite) mitschneiden");

  // Klick auf den sichtbaren Schalter schaltet die Einstellung um.
  await toggle.click();
  await expect
    .poll(
      async () =>
        (await calls(page, "change_meeting_capture_system_setting")).length,
    )
    .toBe(1);
  expect(
    (await calls(page, "change_meeting_capture_system_setting"))[0].args,
  ).toEqual({ enabled: false });
});
