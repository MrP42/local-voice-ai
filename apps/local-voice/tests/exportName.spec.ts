import { test, expect } from "@playwright/test";
import { exportFileName } from "../src/lib/utils/exportName";

const AT = new Date(2026, 8, 8, 14, 5); // 08.09.2026, 14:05

test("the export carries the worksheet title and the time", () => {
  expect(exportFileName("Der Sturm", "Vorlesen", "wav", AT)).toBe(
    "Der-Sturm_2026-09-08_1405.wav",
  );
});

test("two exports of the same worksheet do not collide", () => {
  const first = exportFileName("Hörspiel", "Vorlesen", "mp3", AT);
  const later = exportFileName(
    "Hörspiel",
    "Vorlesen",
    "mp3",
    new Date(2026, 8, 8, 14, 6),
  );
  expect(first).not.toBe(later);
});

test("characters Windows refuses never reach the file name", () => {
  const name = exportFileName('Kapitel 1: "Nacht" <2/3>', "Vorlesen", "wav", AT);
  expect(name).toBe("Kapitel-1-Nacht-23_2026-09-08_1405.wav");
  expect(name).not.toMatch(/[\/:*?"<>|]/);
});

test("a worksheet without a title falls back instead of starting with the stamp", () => {
  expect(exportFileName("   ", "Vorlesen", "wav", AT)).toBe(
    "Vorlesen_2026-09-08_1405.wav",
  );
  expect(exportFileName(null, "Vorlesen", "wav", AT)).toBe(
    "Vorlesen_2026-09-08_1405.wav",
  );
});

test("a title of nothing but forbidden characters still yields a name", () => {
  expect(exportFileName('***', "Vorlesen", "wav", AT)).toBe(
    "Vorlesen_2026-09-08_1405.wav",
  );
});
