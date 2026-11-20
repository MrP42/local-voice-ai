import { test, expect } from "@playwright/test";
import {
  UNBEKANNT,
  anwesendeAusTermin,
  anwesendeFuerBackend,
  anwesenderHinzufuegen,
  personSchluessel,
  personStand,
  startGruende,
  zustandsVerlauf,
} from "../src/components/planung/durchfuehren";

test("Personenschlüssel normalisieren Adressen und Personen-Ids", () => {
  for (const eingabe of [" A@X.ORG ", "MAILTO:A@X.org", "Name <a@x.org>"]) {
    expect(personSchluessel(eingabe)).toBe("a@x.org");
  }
  expect(personSchluessel("id:ok_1")).toBe("id:ok_1");
  expect(personSchluessel(`id:${"a".repeat(64)}`)).toBe(`id:${"a".repeat(64)}`);
  for (const eingabe of [
    "",
    "a@localhost",
    "a@@x.org",
    "@x.org",
    "a@.x.org",
    "a@x.org.",
    "a b@x.org",
    "a@x .org",
    "id:",
    "id:kein!",
    `id:${"a".repeat(65)}`,
  ]) {
    expect(personSchluessel(eingabe)).toBeNull();
  }
});

test("Terminteilnehmende werden dedupliziert, unbekannte bleiben einzeln", () => {
  expect(
    anwesendeAusTermin([
      { email: " A@X.ORG ", name: "A" },
      { email: "mailto:a@x.org", name: "Duplikat" },
      { email: null, name: "Ohne Adresse" },
      { email: "ungültig", name: null },
      { email: null, name: "Weitere Person" },
    ]),
  ).toEqual([
    { id: "a1", email: "a@x.org", name: "A" },
    { id: "a2", email: null, name: "Ohne Adresse" },
    { id: "a3", email: null, name: null },
    { id: "a4", email: null, name: "Weitere Person" },
  ]);
});

test("Hinzufügen meldet ungültige und doppelte Identitäten ohne Änderung", () => {
  const liste = [{ id: "a1", email: "a@x.org", name: "A" }];
  const vorher = liste.map((a) => ({ ...a }));
  expect(anwesenderHinzufuegen(liste, "ungültig")).toEqual({
    liste: vorher,
    fehler: "ungueltig",
  });
  expect(anwesenderHinzufuegen(liste, "Name <A@X.ORG>")).toEqual({
    liste: vorher,
    fehler: "doppelt",
  });
  expect(liste).toEqual(vorher);
});

test("Neue Id folgt der höchsten vorhandenen Zahl auch nach Entfernen", () => {
  const liste = [
    { id: "a1", email: "a@x.org", name: null },
    { id: "n9", email: "b@x.org", name: null },
  ];
  const ergebnis = anwesenderHinzufuegen(liste, "MAILTO:C@X.ORG");
  expect(ergebnis.fehler).toBeNull();
  expect(ergebnis.liste).toEqual([
    ...liste,
    { id: "n10", email: "c@x.org", name: null },
  ]);
  expect(liste).toHaveLength(2);
  expect(anwesenderHinzufuegen([], "id:ok_1").liste[0].id).toBe("n1");
});

test("Backend erhält Identitäten und jede unbekannte Person in Reihenfolge", () => {
  expect(
    anwesendeFuerBackend([
      { id: "a1", email: null, name: "A" },
      { id: "a2", email: "id:ok_1", name: null },
      { id: "a3", email: null, name: null },
    ]),
  ).toEqual([UNBEKANNT, "id:ok_1", UNBEKANNT]);
});

test("Ohne Identität ist der Stand nicht identifiziert, ohne Einwilligung offen", () => {
  expect(personStand({ id: "a1", email: null, name: null }, [])).toBe(
    "nicht_identifiziert",
  );
  expect(personStand({ id: "a2", email: "a@x.org", name: null }, [])).toBe(
    "offen",
  );
});

test("Der Stand folgt der Einwilligung derselben Person", () => {
  const c = (person: string, state: "zugestimmt" | "abgelehnt") => ({
    plan_id: "p",
    person,
    state,
    channel: null,
    recorded_at: null,
  });
  const liste = [c("A@X.org", "zugestimmt"), c("b@x.org", "abgelehnt")];
  expect(personStand({ id: "a1", email: "a@x.org", name: null }, liste)).toBe(
    "zugestimmt",
  );
  expect(personStand({ id: "a2", email: "b@x.org", name: null }, liste)).toBe(
    "abgelehnt",
  );
});

test("Startgründe trennen Namen und nicht identifizierte Personen", () => {
  expect(startGruende({ modus: "mit_transkription" })).toEqual({
    nichtGeplant: false,
    offen: [],
    abgelehnt: [],
    nichtIdentifiziert: 0,
  });
  expect(
    startGruende({
      modus: "nur_notizen",
      nicht_geplant: true,
      offen: [],
      abgelehnt: [],
    }).nichtGeplant,
  ).toBe(true);
  expect(
    startGruende({
      modus: "nur_notizen",
      nicht_geplant: false,
      offen: ["a@x.org", UNBEKANNT, UNBEKANNT],
      abgelehnt: ["b@x.org"],
    }),
  ).toEqual({
    nichtGeplant: false,
    offen: ["a@x.org"],
    abgelehnt: ["b@x.org"],
    nichtIdentifiziert: 2,
  });
});

test("Der Verlauf steht neueste zuerst und lässt die Eingabe unverändert", () => {
  const e = (person: string, am: number) => ({
    einwilligung: {
      plan_id: "p",
      person,
      state: "offen" as const,
      channel: null,
      recorded_at: null,
    },
    festgehalten_am: am,
  });
  const eingabe = [e("a", 1), e("b", 3), e("c", 3), e("d", 2)];
  expect(zustandsVerlauf(eingabe).map((x) => x.einwilligung.person)).toEqual([
    "c",
    "b",
    "d",
    "a",
  ]);
  expect(eingabe.map((x) => x.einwilligung.person)).toEqual([
    "a",
    "b",
    "c",
    "d",
  ]);
});
