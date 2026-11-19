// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/menueOffen.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  istMenueOffen,
  MENUE_SELECTOR,
} from "../../src/components/ui/menueOffen.ts";

const menue = (enthaelt: unknown[] = []) => ({
  contains: (x: unknown) => enthaelt.includes(x),
});
const wurzel = (menues: ReturnType<typeof menue>[]) => ({
  querySelectorAll: (selector: string) => {
    assert.equal(selector, MENUE_SELECTOR);
    return menues;
  },
});

test("ohne Menü ist keins offen", () => {
  assert.equal(istMenueOffen(wurzel([]), "ziel"), false);
});

test("ein fremdes offenes Menü verdeckt das Ziel", () => {
  assert.equal(istMenueOffen(wurzel([menue()]), "ziel"), true);
});

test("ein Menü, das das Ziel enthält, zählt nicht", () => {
  assert.equal(istMenueOffen(wurzel([menue(["ziel"])]), "ziel"), false);
});

test("ohne Ziel zählt jedes offene Menü", () => {
  assert.equal(istMenueOffen(wurzel([menue(["x"])])), true);
});

test("ein fremdes Menü neben einem eigenen genügt", () => {
  assert.equal(istMenueOffen(wurzel([menue(["ziel"]), menue()]), "ziel"), true);
});
