// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/menueOffen.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  istMenueOffen,
  MENUE_SELECTOR,
  menueVerdecktZiel,
  sichtbareMenues,
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

test("ein unsichtbares Menü zählt nicht als offen", () => {
  const verborgen = { ...menue(), hidden: true };
  assert.equal(istMenueOffen(wurzel([verborgen]), "ziel"), false);
  const ariaVerborgen = {
    ...menue(),
    getAttribute: (n: string) => (n === "aria-hidden" ? "true" : null),
  };
  assert.equal(istMenueOffen(wurzel([ariaVerborgen]), "ziel"), false);
  assert.equal(istMenueOffen(wurzel([verborgen, menue()]), "ziel"), true);
});

test("ein sichtbares Menü neben dem Ziel zählt, die Sichtbarkeit ist austauschbar", () => {
  assert.equal(
    istMenueOffen(wurzel([menue()]), "ziel", () => false),
    false,
  );
  assert.equal(
    istMenueOffen(wurzel([menue()]), "ziel", () => true),
    true,
  );
});

test("die Menüliste wird einmal berechnet und gegen mehrere Ziele geprüft", () => {
  const eigenes = menue(["a"]);
  const fremdes = menue();
  const liste = sichtbareMenues(wurzel([eigenes, fremdes]));
  assert.equal(liste.length, 2);
  assert.equal(menueVerdecktZiel(liste, "a"), true); // das fremde Menü verdeckt
  assert.equal(menueVerdecktZiel([eigenes], "a"), false);
  assert.equal(menueVerdecktZiel([], "a"), false);
  assert.equal(menueVerdecktZiel([eigenes], null), true);
});
