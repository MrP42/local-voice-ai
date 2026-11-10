// lva#395 D7-Rest: reine Logik der Ablage (Unterordnerbaum verknüpfter Ordner, Zählung für die
// Cloud-Ausnahme, Einstellungsvergleich). Lauf: node --test scripts/ablage-logik.test.mjs
// (Node >= 22.6; Typen werden beim Laden entfernt).
import assert from "node:assert/strict";
import test from "node:test";

import {
  ansichtZerlegen,
  anbieterNamen,
  cloudAenderung,
  cloudAlle,
  cloudQuellen,
  dateienImPfad,
  elternPfad,
  knotenZu,
  letztesGlied,
  unterAnsicht,
  unterbaumBauen,
  vertraulicheImUnterbaum,
  vorfahrenPfade,
  wirksameKlasseNeu,
} from "../src/components/ablage/logik.ts";

const u = (pfad, nur_online = false) => ({ pfad, nur_online });
const namen = (knoten) => knoten.map((k) => k.pfad);

test("Ansicht eines Unterordners: zusammensetzen und zerlegen", () => {
  assert.equal(unterAnsicht("ord_1", ""), "ord_1");
  assert.equal(unterAnsicht("ord_1", "Akte/Angebote"), "ord_1|Akte/Angebote");
  assert.deepEqual(ansichtZerlegen("ord_1|Akte/Angebote"), {
    ordner: "ord_1",
    pfad: "Akte/Angebote",
  });
  assert.deepEqual(ansichtZerlegen("ord_1"), { ordner: "ord_1", pfad: "" });
  assert.deepEqual(ansichtZerlegen(null), { ordner: null, pfad: "" });
  // Ein Pfad darf selbst das Trennzeichen enthalten: nur das erste zählt.
  assert.deepEqual(ansichtZerlegen("ord_1|a|b"), {
    ordner: "ord_1",
    pfad: "a|b",
  });
});

test("Baum aus Unterordnern und Dateipfaden: auch leere Ordner, Vorfahren ergänzt, sortiert", () => {
  const baum = unterbaumBauen(
    [u("Leer"), u("Akte"), u("Akte/Protokolle")],
    ["a.txt", "Akte/Angebote/x.txt", "Akte/Angebote/y.txt", "Zeta/z.txt"],
  );
  assert.deepEqual(namen(baum), ["Akte", "Leer", "Zeta"]);
  const akte = baum[0];
  assert.equal(akte.name, "Akte");
  assert.equal(akte.tiefe, 0);
  assert.deepEqual(namen(akte.kinder), ["Akte/Angebote", "Akte/Protokolle"]);
  assert.equal(akte.kinder[0].tiefe, 1);
  assert.equal(akte.kinder[0].name, "Angebote");
  // Dateien je Ordner, rekursiv.
  assert.equal(akte.dateien, 2);
  assert.equal(akte.kinder[0].dateien, 2);
  assert.equal(akte.kinder[1].dateien, 0);
  assert.equal(baum[2].dateien, 1);
  // Ordnerloser Altbestand: nur aus Dateipfaden.
  assert.deepEqual(namen(unterbaumBauen([], ["a.txt", "B/c.txt"])), ["B"]);
  assert.deepEqual(unterbaumBauen([], []), []);
});

test("Nur online: der Ordner und alles darunter, nicht aufklappbar markiert", () => {
  const baum = unterbaumBauen(
    [u("Akte", true), u("Akte/Angebote"), u("Frei")],
    ["Akte/Angebote/x.txt", "Frei/y.txt"],
  );
  assert.equal(baum[0].nurOnline, true);
  assert.equal(baum[0].kinder[0].nurOnline, true, "erbt die Marke von oben");
  assert.equal(baum[1].nurOnline, false);
  assert.equal(knotenZu(baum, "Akte/Angebote")?.nurOnline, true);
  assert.equal(knotenZu(baum, "Frei")?.nurOnline, false);
  assert.equal(knotenZu(baum, "gibt/es/nicht"), null);
  assert.equal(knotenZu(baum, ""), null);
});

test("Dateien direkt in einem Pfad (nicht darunter)", () => {
  const d = (name) => ({ name });
  const alle = [
    d("a.txt"),
    d("Akte/b.txt"),
    d("Akte/Angebote/c.txt"),
    d("Akte2/d.txt"),
  ];
  assert.deepEqual(
    dateienImPfad(alle, "").map((x) => x.name),
    ["a.txt"],
  );
  assert.deepEqual(
    dateienImPfad(alle, "Akte").map((x) => x.name),
    ["Akte/b.txt"],
  );
  assert.deepEqual(
    dateienImPfad(alle, "Akte/Angebote").map((x) => x.name),
    ["Akte/Angebote/c.txt"],
  );
});

test("Vorfahren eines Pfads (zum Aufklappen)", () => {
  assert.deepEqual(vorfahrenPfade("A/B/C"), ["A", "A/B"]);
  assert.deepEqual(vorfahrenPfade("A"), []);
  assert.deepEqual(vorfahrenPfade(""), []);
});

const o = (id, name, eltern, klasse, datenklasse = "erbt") => ({
  id,
  name,
  eltern,
  klasse,
  datenklasse,
});
const f = (id, ordner, klasse) => ({ id, ordner, klasse });

test("Zählung für die Bestätigung: Dateien mit wirksamer Klasse Vertraulich im Unterbaum", () => {
  const ordner = [
    o("a", "Verträge", null, "confidential", "confidential"),
    o("b", "Wartung", "a", "confidential"),
    o("c", "Gehälter", "a", "personal", "personal"),
    o("d", "Unter Gehälter", "c", "personal"),
    o("e", "Daneben", null, "confidential", "confidential"),
  ];
  const dateien = [
    f("1", "a", "confidential"),
    f("2", "b", "confidential"),
    f("3", "b", "personal"), // Mindestklasse der Datei: personenbezogen
    f("4", "c", "personal"),
    f("5", "d", "personal"),
    f("6", "e", "confidential"), // anderer Baum
    f("7", "a", "internal"), // kann nicht vorkommen, zählt nie
  ];
  const z = vertraulicheImUnterbaum(ordner, dateien, "a");
  assert.equal(z.dateien, 2, "nur wirksam Vertraulich");
  assert.equal(z.unterordner, 3);
  assert.deepEqual(
    z.personenbezogen,
    ["Gehälter"],
    "nur der oberste, nicht die darunter",
  );
  const leer = vertraulicheImUnterbaum(ordner, dateien, "e");
  assert.deepEqual(leer, { dateien: 1, unterordner: 0, personenbezogen: [] });
  // Zyklus oder fehlender Ordner: kein Absturz, nichts gezählt.
  assert.deepEqual(vertraulicheImUnterbaum(ordner, dateien, "gibtesnicht"), {
    dateien: 0,
    unterordner: 0,
    personenbezogen: [],
  });
  const zyklus = [
    o("x", "X", "y", "confidential"),
    o("y", "Y", "x", "confidential"),
  ];
  assert.equal(
    vertraulicheImUnterbaum(zyklus, [f("9", "y", "confidential")], "x").dateien,
    1,
  );
});

test("Wirksame Klasse nach der Auswahl im Dialog", () => {
  const ordner = [
    o("p", "Eltern", null, "confidential", "confidential"),
    o("k", "Kind", "p", "confidential"),
    o("w", "Wurzel", null, "internal"),
  ];
  // „Erben“: die Klasse des Elternordners; ohne Elternordner die Vorgabe.
  assert.equal(wirksameKlasseNeu(ordner, "k", "erbt"), "confidential");
  assert.equal(wirksameKlasseNeu(ordner, "w", "erbt"), "internal");
  // Eine niedrigere Wahl wird auf die geerbte angehoben, nie umgekehrt.
  assert.equal(wirksameKlasseNeu(ordner, "k", "public"), "confidential");
  assert.equal(wirksameKlasseNeu(ordner, "k", "personal"), "personal");
  assert.equal(wirksameKlasseNeu(ordner, "w", "confidential"), "confidential");
});

test("Cloud-Änderung: erteilen und entziehen aus Auswahl und Bestand", () => {
  assert.deepEqual(cloudAenderung(["a"], ["a"]), {
    erteilen: [],
    entziehen: [],
  });
  assert.deepEqual(cloudAenderung([], ["b", "a"]), {
    erteilen: ["a", "b"],
    entziehen: [],
  });
  assert.deepEqual(cloudAenderung(["a", "b"], ["b"]), {
    erteilen: [],
    entziehen: ["a"],
  });
  assert.deepEqual(cloudAenderung(["a"], ["c"]), {
    erteilen: ["c"],
    entziehen: ["a"],
  });
  assert.deepEqual(cloudAenderung([], []), { erteilen: [], entziehen: [] });
});

test("Pfadteile eines relativen Pfads", () => {
  assert.equal(elternPfad("A/B/c.txt"), "A/B");
  assert.equal(elternPfad("c.txt"), "");
  assert.equal(letztesGlied("A/B/c.txt"), "c.txt");
  assert.equal(letztesGlied("c.txt"), "c.txt");
});

test("Cloud-Ausnahmen eines Eintrags: eigene und geerbte vereint, ohne Doppelte, sortiert", () => {
  assert.deepEqual(cloudAlle(undefined), []);
  assert.deepEqual(cloudAlle(null), []);
  assert.deepEqual(cloudAlle({ anbieter: [], geerbt: [] }), []);
  assert.deepEqual(cloudAlle({ anbieter: ["b"], geerbt: ["a", "b"] }), [
    "a",
    "b",
  ]);
});

test("Anbieternamen: aus der Liste, sonst die Kennung; sortiert, ohne Doppelte", () => {
  const liste = [
    { id: "openai_eu", name: "OpenAI (EU)", waehlbar: true, grund: null },
    { id: "mistral", name: "Mistral", waehlbar: true, grund: null },
  ];
  assert.deepEqual(anbieterNamen(["openai_eu", "mistral"], liste), [
    "Mistral",
    "OpenAI (EU)",
  ]);
  assert.deepEqual(anbieterNamen(["weg", "mistral", "mistral"], liste), [
    "Mistral",
    "weg",
  ]);
  assert.deepEqual(anbieterNamen(["mistral"], []), ["mistral"]);
  assert.deepEqual(anbieterNamen([], liste), []);
});

test("Cloud-Quelle: nächster Ordner (selbst oder darüber), an dem die Ausnahme hängt", () => {
  const mit = (id, name, eltern, anbieter = []) => ({
    ...o(id, name, eltern, "confidential"),
    cloud: { anbieter, geerbt: [] },
  });
  const ordner = [
    mit("a", "Verträge", null, ["x", "y"]),
    mit("b", "Wartung", "a", ["y"]),
    mit("c", "Tief", "b"),
  ];
  const q = cloudQuellen(ordner, "c", ["x", "y"]);
  assert.equal(q.get("y"), "Wartung", "die nächste gewinnt");
  assert.equal(q.get("x"), "Verträge");
  assert.equal(
    cloudQuellen(ordner, "a", ["x"]).get("x"),
    "Verträge",
    "auch der Ordner selbst",
  );
  assert.equal(
    cloudQuellen(ordner, "c", ["z"]).size,
    0,
    "unbekannter Anbieter: keine Quelle",
  );
  assert.equal(cloudQuellen(ordner, "gibtesnicht", ["x"]).size, 0);
  // Zyklus: kein Absturz.
  const zyklus = [mit("p", "P", "q", []), mit("q", "Q", "p", [])];
  assert.equal(cloudQuellen(zyklus, "p", ["x"]).size, 0);
});
