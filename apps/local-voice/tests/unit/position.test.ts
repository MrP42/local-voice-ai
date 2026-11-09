import assert from "node:assert/strict";
import { test } from "node:test";
import { mergedPosition } from "../../src/lib/library/position.ts";

const prog = (offset: number, updated: number) => ({ offset, updated });

test("die jüngste Stelle gewinnt und wird auf die Gesamtzeit umgerechnet", () => {
  const got = mergedPosition([
    { durations: [10_000], progress: prog(4000, 100) },
    { durations: [20_000], progress: prog(5000, 300) },
    { durations: [30_000], progress: prog(1000, 200) },
  ]);
  assert.deepEqual(got, { ms: 15_000, updated: 300 });
});

test("bei unbekannter Dauer davor wird die Stelle der Folgebücher nicht geraten", () => {
  const got = mergedPosition([
    { durations: [null], progress: null },
    { durations: [20_000], progress: prog(5000, 300) },
  ]);
  assert.equal(got, null);
  const first = mergedPosition([
    { durations: [null], progress: prog(2000, 5) },
    { durations: [20_000], progress: prog(5000, 3) },
  ]);
  assert.deepEqual(first, { ms: 2000, updated: 5 });
});

test("fertige oder leere Stellen zählen nicht", () => {
  assert.equal(
    mergedPosition([
      {
        durations: [1000],
        progress: { offset: 900, updated: 1, finished: true },
      },
      { durations: [1000], progress: prog(0, 2) },
    ]),
    null,
  );
});
