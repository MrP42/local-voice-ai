// lva#396 N2: Umrechnung der Notizbuch-Freigabe zwischen Einstellung und Anzeige (reine Logik).
// Lauf: node --test scripts/notizbuch-auswahl.test.mjs (Node >= 22.6; Typen werden beim Laden entfernt).
import assert from "node:assert/strict";
import test from "node:test";

import {
  auswahlAus,
  eintraegeAus,
  frei,
  schreibkette,
  setzeAbschnitte,
  setzeGanz,
  startOffen,
} from "../src/components/voice-call/notebookSelection.ts";

const NB = {
  id: "nb1",
  name: "Arbeit",
  sections: [
    { id: "a1", name: "Besprechungen", group: null, pages: 24 },
    { id: "a2", name: "Haushalt 2027", group: "Projekte", pages: 9 },
    { id: "a3", name: "Radwegekonzept", group: "Projekte", pages: 5 },
  ],
};

test("Standard leer: nichts frei", () => {
  const a = auswahlAus([]);
  assert.equal(frei(a, NB), 0);
  assert.deepEqual(eintraegeAus(a), []);
});

test("Abschnitte und ganzes Notizbuch hin und zurück", () => {
  let a = auswahlAus(["abschnitt:a1", "notizbuch:nb9", "fremd:x"]);
  assert.equal(frei(a, NB), 1);
  a = setzeAbschnitte(a, ["a2", "a3"], true);
  assert.equal(frei(a, NB), 3);
  a = setzeAbschnitte(a, ["a1"], false);
  assert.deepEqual(eintraegeAus(a, ["abschnitt:a1", "notizbuch:nb9"]), [
    "notizbuch:nb9",
    "abschnitt:a2",
    "abschnitt:a3",
  ]);
  // Ganz: alle Abschnitte zählen, die Einzelauswahl bleibt für das Ausschalten erhalten.
  a = setzeGanz(setzeAbschnitte(a, ["a2", "a3"], false), "nb1", true);
  assert.equal(frei(a, NB), 3);
  a = setzeGanz(a, "nb1", false);
  assert.equal(frei(a, NB), 0);
});

test("unveränderte Auswahl ergibt dieselben Einträge in derselben Reihenfolge", () => {
  const bisher = ["notizbuch:nb2", "abschnitt:a3", "abschnitt:a1"];
  assert.deepEqual(eintraegeAus(auswahlAus(bisher), bisher), bisher);
});

const NB2 = {
  id: "nb2",
  name: "Privat",
  sections: [{ id: "b1", name: "Reisen", group: null, pages: 3 }],
};

test("Anfangs offen: Notizbücher mit Auswahl, ohne Auswahl das erste", () => {
  assert.deepEqual(startOffen([NB, NB2], auswahlAus([])), { nb1: true });
  assert.deepEqual(startOffen([NB, NB2], auswahlAus(["abschnitt:b1"])), {
    nb2: true,
  });
  assert.deepEqual(startOffen([NB, NB2], auswahlAus(["notizbuch:nb1"])), {
    nb1: true,
  });
  assert.deepEqual(startOffen([], auswahlAus([])), {});
});

/** Schreibvorgänge, die der Test einzeln beendet (auch in vertauschter Reihenfolge). */
function attrappe() {
  const offen = [];
  const ende = [];
  const kette = schreibkette(
    (v) => new Promise((ok, nein) => offen.push({ v, ok, nein })),
    (ok) => ende.push(ok),
  );
  return { kette, offen, ende };
}
const tick = () => new Promise((r) => setTimeout(r, 0));

test("Schreibkette: nie zwei zugleich, nur die letzte Änderung geht hinterher", async () => {
  const { kette, offen, ende } = attrappe();
  kette.setze(["abschnitt:a1"]);
  kette.setze(["abschnitt:a1", "abschnitt:a2"]);
  kette.setze(["abschnitt:a2"]);
  assert.equal(offen.length, 1);
  assert.equal(kette.laeuft(), true);
  offen[0].ok(true);
  await tick();
  assert.deepEqual(
    offen.map((o) => o.v),
    [["abschnitt:a1"], ["abschnitt:a2"]],
  );
  assert.deepEqual(ende, []);
  offen[1].ok(true);
  await tick();
  assert.deepEqual(ende, [true]);
  assert.equal(kette.laeuft(), false);
});

test("Schreibkette: eine späte Antwort der ersten Änderung überholt die zweite nicht", async () => {
  const { kette, offen, ende } = attrappe();
  kette.setze(["abschnitt:a1"]);
  kette.setze(["abschnitt:a2"]);
  // Die zweite Änderung wird erst geschrieben, wenn die erste beantwortet ist.
  assert.equal(offen.length, 1);
  offen[0].ok(true);
  await tick();
  offen[1].ok(true);
  await tick();
  assert.deepEqual(offen.at(-1).v, ["abschnitt:a2"]);
  assert.deepEqual(ende, [true]);
});

test("Schreibkette: Fehler wird gemeldet, danach geht es normal weiter", async () => {
  const { kette, offen, ende } = attrappe();
  kette.setze(["abschnitt:a1"]);
  offen[0].ok(false);
  await tick();
  assert.deepEqual(ende, [false]);
  kette.setze(["abschnitt:a2"]);
  offen[1].nein(new Error("weg"));
  await tick();
  assert.deepEqual(ende, [false, false]);
  kette.setze(["abschnitt:a3"]);
  offen[2].ok(true);
  await tick();
  assert.deepEqual(ende, [false, false, true]);
});

test("Schreibkette: scheitert eine Zwischenstufe, zählt das Ergebnis der letzten", async () => {
  const { kette, offen, ende } = attrappe();
  kette.setze(["abschnitt:a1"]);
  kette.setze(["abschnitt:a2"]);
  offen[0].nein(new Error("weg"));
  await tick();
  offen[1].ok(true);
  await tick();
  assert.deepEqual(ende, [true]);
});
