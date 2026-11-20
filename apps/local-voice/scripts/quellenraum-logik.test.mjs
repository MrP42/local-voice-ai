import test from "node:test";
import assert from "node:assert/strict";

const {
  artSchluessel,
  zustandSchluessel,
  auswahlText,
  versionLabel,
  sortiert,
  nichtWaehlbar,
} = await import("../src/lib/quellenraumLogik.ts");

const zeile = (werte = {}) => ({
  id: "quelle",
  kind: "text",
  art: "text",
  title: "Beispiel",
  link: null,
  state: "bereit",
  selected: false,
  form: null,
  version: null,
  versionCount: 0,
  versionHash: null,
  tokens: null,
  ...werte,
});

test("Art folgt art und nicht kind", () => {
  for (const art of [
    "buch", "aufnahme", "ablage", "text", "datei", "recherche", "sonstige",
  ]) {
    assert.equal(artSchluessel(zeile({ art })), art);
  }
});

test("Zustände und Wählbarkeit sind unabhängig von der Auswahl", () => {
  for (const state of ["bereit", "kein_text", "nicht_verfuegbar"]) {
    for (const selected of [false, true]) {
      const z = zeile({ state, selected });
      assert.equal(zustandSchluessel(z), state);
      assert.equal(nichtWaehlbar(z), state === "nicht_verfuegbar");
    }
  }
});

test("Auswahl übernimmt die Backend-Zähler auch bei leerer Liste", () => {
  const sicht = {
    sources: [], selectedCount: 3, total: 4, stand: "s", reset: false,
  };
  assert.deepEqual(auswahlText(sicht), { count: 3, total: 4 });
  assert.deepEqual(
    auswahlText({ ...sicht, selectedCount: 0, total: 0 }),
    { count: 0, total: 0 },
  );
});

test("Version unterscheidet null von nullwertiger Versionsnummer", () => {
  assert.equal(versionLabel(zeile({ versionCount: 7 })), null);
  assert.deepEqual(versionLabel(zeile({ version: 0, versionCount: 1 })), {
    nr: 0, anzahl: 1,
  });
  assert.deepEqual(versionLabel(zeile({ version: 2, versionCount: 5 })), {
    nr: 2, anzahl: 5,
  });
});

test("Sortierung priorisiert Auswahl, deutsche Titel und bleibt stabil", () => {
  const eingabe = [
    zeile({ id: "weg", title: null, state: "nicht_verfuegbar" }),
    zeile({ id: "z", title: "Zebra" }),
    zeile({ id: "a1", title: "Ähre" }),
    zeile({ id: "a2", title: "Ähre" }),
    zeile({ id: "s", title: "Zug", selected: true }),
    zeile({ id: "benannt", title: "Bach", state: "nicht_verfuegbar" }),
  ];
  const vorher = [...eingabe];
  assert.deepEqual(sortiert(eingabe).map((z) => z.id), [
    "s", "a1", "a2", "benannt", "z", "weg",
  ]);
  assert.deepEqual(eingabe, vorher);
  assert.notEqual(sortiert(eingabe), eingabe);
});

test("Leere und ausschließlich nicht verfügbare Quellen", () => {
  assert.deepEqual(sortiert([]), []);
  const eingabe = [
    zeile({ id: "1", title: null, state: "nicht_verfuegbar" }),
    zeile({ id: "2", title: null, state: "nicht_verfuegbar" }),
  ];
  assert.deepEqual(sortiert(eingabe), eingabe);
  assert.deepEqual(
    sortiert([
      ...eingabe,
      zeile({ id: "3", title: null, selected: true }),
    ]).map((z) => z.id),
    ["3", "1", "2"],
  );
});
