import { test, expect } from "@playwright/test";
import {
  canConfirmAll,
  flipGroups,
  mapPosition,
  proposeGroups,
  validGroups,
  type AlignGroup,
  type Outline,
} from "../src/lib/library/languageSwitch";

// Deutsch 4 Kapitel à 100, Englisch 5 Kapitel (Kapitel 3 in 3+4 geteilt).
const de: Outline = { starts: [0, 100, 200, 300], length: 400 };
const en: Outline = { starts: [0, 120, 240, 300, 360], length: 480 };
const groups: AlignGroup[] = [
  { a: [0], b: [0], confirmed: true },
  { a: [1], b: [1], confirmed: true },
  { a: [2], b: [2, 3], confirmed: true },
  { a: [3], b: [4], confirmed: false },
];

test("kapitelgenau nur in bestätigter Gruppe, Anteil innerhalb der Gruppe", () => {
  const t = mapPosition(de, en, 150, groups);
  expect(t.level).toBe("chapter");
  expect(t.chapters).toEqual([1]);
  expect(t.offset).toBe(180); // Mitte von Kapitel 2 (120…240)
});

test("1:n-Gruppe verteilt über beide Zielkapitel", () => {
  const t = mapPosition(de, en, 250, groups);
  expect(t.level).toBe("chapter");
  expect(t.chapters).toEqual([2, 3]);
  expect(t.offset).toBe(300); // Mitte von 240…360
});

test("unbestätigte Gruppe → ungefähr nach Gesamtanteil", () => {
  const t = mapPosition(de, en, 350, groups);
  expect(t.level).toBe("approx");
  expect(t.fraction).toBeCloseTo(0.875);
  expect(t.offset).toBe(420);
});

test("ohne Zuordnung immer ungefähr, auch bei gleicher Kapitelzahl", () => {
  const same: Outline = { starts: [0, 100, 200, 300], length: 400 };
  expect(mapPosition(de, same, 150, null).level).toBe("approx");
});

test("Rückweg über gedrehte Gruppen landet im selben Kapitel", () => {
  const there = mapPosition(de, en, 150, groups);
  const back = mapPosition(en, de, there.offset, flipGroups(groups));
  expect(back.level).toBe("chapter");
  expect(back.offset).toBe(150);
});

test("Gruppen: Lücke und Doppelzuordnung sind ungültig", () => {
  expect(validGroups(groups)).toBe(true);
  expect(validGroups([{ a: [0], b: [1], confirmed: true }])).toBe(false);
  expect(
    validGroups([
      { a: [0], b: [0], confirmed: true },
      { a: [0], b: [1], confirmed: true },
    ]),
  ).toBe(false);
});

test("„Alle bestätigen“ nur bei gleicher Zahl und reinen 1:1-Gruppen", () => {
  expect(canConfirmAll(proposeGroups(4, 4), 4, 4)).toBe(true);
  expect(canConfirmAll(proposeGroups(18, 21), 18, 21)).toBe(false);
});

test("Vorschlag deckt 18 ↔ 21 und 21 ↔ 18 lückenlos ab, nichts bestätigt", () => {
  for (const [a, b] of [
    [18, 21],
    [21, 18],
    [1, 5],
    [5, 1],
    [24, 24],
  ]) {
    const g = proposeGroups(a, b);
    expect(validGroups(g)).toBe(true);
    expect(g.flatMap((x) => x.a)).toHaveLength(a);
    expect(g.flatMap((x) => x.b)).toHaveLength(b);
    expect(g.every((x) => !x.confirmed)).toBe(true);
  }
});
