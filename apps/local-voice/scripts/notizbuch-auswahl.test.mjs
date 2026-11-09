// lva#396 N2: Umrechnung der Notizbuch-Freigabe zwischen Einstellung und Anzeige (reine Logik).
// Lauf: node --test scripts/notizbuch-auswahl.test.mjs (Node >= 22.6; Typen werden beim Laden entfernt).
import assert from "node:assert/strict";
import test from "node:test";

import {
  auswahlAus,
  eintraegeAus,
  frei,
  setzeAbschnitte,
  setzeGanz,
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
