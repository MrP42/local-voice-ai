// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/frontier.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  frontierOf,
  liftedFrontier,
  requestFrontier,
  steadyFrontier,
} from "../../src/lib/library/frontier.ts";

test("die Grenze ist das Größte aus gespeichert, gehört und gelesen", () => {
  assert.equal(frontierOf(null), 0);
  assert.equal(frontierOf({ heard: [[0, 300]], read: [[300, 900]] }), 900);
  assert.equal(frontierOf({ frontier: 1200, heard: [[0, 300]] }), 1200);
  assert.equal(frontierOf({ frontier: 100, heard: [[0, 300]] }), 300);
});

test("ein Kapitelsprung (nur die Stelle) hebt die Grenze nicht", () => {
  assert.equal(
    frontierOf({ heard: [[0, 400]], ...{ offset: 9000 } } as never),
    400,
  );
});

test("Zurückspulen senkt die Grenze nicht", () => {
  const before = { frontier: 800, heard: [[0, 800]] as [number, number][] };
  const after = { heard: [[0, 200]] as [number, number][] };
  assert.equal(steadyFrontier(after, before), 800);
  assert.equal(steadyFrontier({ read: [[0, 1000]] }, before), 1000);
});

test("ohne Grenze fehlt sie in der Anfrage (Kern: no_frontier)", () => {
  assert.equal(requestFrontier(null), undefined);
  assert.equal(requestFrontier({ heard: [] }), undefined);
  assert.equal(requestFrontier({ frontier: 55 }), 55);
});

test("ungültige Werte zählen nicht", () => {
  assert.equal(frontierOf({ frontier: -5, heard: [[0, Number.NaN]] }), 0);
});

test("nur sequentielles Weiterblättern hebt die Grenze, höchstens um eine Ansicht", () => {
  // Weiterblättern an die Grenze anschließend: hebt auf das Ende der Ansicht.
  assert.equal(
    liftedFrontier(900, { start: 900, end: 1800, sequential: true }),
    1800,
  );
  // Ansicht innerhalb des Gelesenen: nichts zu heben.
  assert.equal(
    liftedFrontier(900, { start: 300, end: 800, sequential: true }),
    900,
  );
  // Die neue Grenze liegt nie über alter Grenze + einer Ansicht.
  const view = { start: 500, end: 1400, sequential: true };
  assert.ok(liftedFrontier(900, view) <= 900 + (view.end - view.start));
});

test("Sprünge heben die Grenze nie, auch nicht ans Ende des Buchs", () => {
  // Inhaltsverzeichnis, Lesezeichen, Suche, Seitenzahl, Belegsprung, revealOffset.
  assert.equal(
    liftedFrontier(900, { start: 8000, end: 8900, sequential: false }),
    900,
  );
  // Ein Sprung knapp hinter die Grenze (weniger als eine Ansicht) hebt ebenfalls nicht.
  assert.equal(
    liftedFrontier(900, { start: 900, end: 1800, sequential: false }),
    900,
  );
  // Weiterblättern nach einem Sprung schließt nicht an: keine Hebung.
  assert.equal(
    liftedFrontier(900, { start: 8900, end: 9800, sequential: true }),
    900,
  );
});

test("die erste Ansicht ab Textanfang zählt, ungültige Werte nicht", () => {
  assert.equal(
    liftedFrontier(0, { start: 0, end: 700, sequential: false }),
    700,
  );
  assert.equal(
    liftedFrontier(0, { start: 5000, end: 5700, sequential: true }),
    0,
  );
  assert.equal(
    liftedFrontier(400, { start: Number.NaN, end: 900, sequential: true }),
    400,
  );
  assert.equal(
    liftedFrontier(400, { start: 100, end: 300, sequential: true }),
    400,
  );
});
