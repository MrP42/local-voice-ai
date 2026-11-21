// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/meldungenBuendeln.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  buendeln,
  istSpeicherschutzMeldung,
  type MeldungsDaten,
} from "../../src/lib/meldungenBuendeln.ts";

function aufzeichner() {
  const aufrufe: { text: unknown; data?: MeldungsDaten }[] = [];
  const original = (text: unknown, data?: MeldungsDaten) => {
    aufrufe.push({ text, data });
    return data?.id ?? "frei";
  };
  return { aufrufe, original };
}

test("gleiche Meldungen teilen eine Kennung und zählen hoch", () => {
  const { aufrufe, original } = aufzeichner();
  const fehler = buendeln(original, { praefix: "e" });
  fehler("Modell nicht geladen");
  fehler("Modell nicht geladen");
  fehler("Modell nicht geladen");
  assert.equal(aufrufe.length, 3);
  const ids = new Set(aufrufe.map((a) => a.data?.id));
  assert.equal(ids.size, 1, "eine Meldung, kein Stapel gleicher Kästen");
  assert.equal(aufrufe[2].text, "Modell nicht geladen (3×)");
  assert.equal(aufrufe[0].text, "Modell nicht geladen");
});

test("verschiedene Meldungen bekommen verschiedene Kennungen", () => {
  const { aufrufe, original } = aufzeichner();
  const fehler = buendeln(original, { praefix: "e" });
  fehler("A");
  fehler("B");
  fehler("A", { description: "anders" });
  const ids = aufrufe.map((a) => a.data?.id);
  assert.equal(new Set(ids).size, 3);
});

test("nach dem Schließen beginnt dieselbe Meldung neu", () => {
  const { aufrufe, original } = aufzeichner();
  const fehler = buendeln(original, { praefix: "e" });
  fehler("A");
  fehler("A");
  aufrufe[1].data?.onAutoClose?.({});
  fehler("A");
  assert.equal(aufrufe[2].text, "A");
  assert.notEqual(aufrufe[2].data?.id, aufrufe[0].data?.id);
});

test("altes Schließen löscht den neuen Eintrag nicht", () => {
  const { aufrufe, original } = aufzeichner();
  const fehler = buendeln(original, { praefix: "e" });
  fehler("A");
  aufrufe[0].data?.onDismiss?.({});
  fehler("A");
  // Das erste (alte) Schließen kommt verspätet noch einmal an.
  aufrufe[0].data?.onDismiss?.({});
  fehler("A");
  assert.equal(aufrufe[2].text, "A (2×)");
  assert.equal(aufrufe[2].data?.id, aufrufe[1].data?.id);
});

test("eigene Kennung und React-Inhalte gehen unverändert durch", () => {
  const { aufrufe, original } = aufzeichner();
  const fehler = buendeln(original, { praefix: "e" });
  fehler("A", { id: "meins" });
  fehler("A", { id: "meins" });
  const knoten = { type: "div" };
  fehler(knoten);
  assert.equal(aufrufe[1].text, "A");
  assert.equal(aufrufe[1].data?.id, "meins");
  assert.equal(aufrufe[2].text, knoten);
  assert.equal(aufrufe[2].data, undefined);
});

test("nach toast.dismiss(id) beginnt dieselbe Meldung neu", () => {
  const { aufrufe, original } = aufzeichner();
  const fehler = buendeln(original, { praefix: "e" });
  fehler("A");
  fehler.vergessen(aufrufe[0].data?.id);
  fehler("A");
  assert.equal(aufrufe[1].text, "A");
  fehler("B");
  fehler.vergessen();
  fehler("B");
  assert.equal(aufrufe[3].text, "B");
});

test("Rückrufe des Aufrufers laufen weiter", () => {
  const { aufrufe, original } = aufzeichner();
  const fehler = buendeln(original, { praefix: "e" });
  let geschlossen = 0;
  fehler("A", { onDismiss: () => geschlossen++ });
  aufrufe[0].data?.onDismiss?.({});
  assert.equal(geschlossen, 1);
});

test("Speicherschutz-Meldungen bekommen den Knopf, andere nicht", () => {
  const { aufrufe, original } = aufzeichner();
  const knopf = { label: "Grenze ändern", onClick: () => {} };
  const fehler = buendeln(original, {
    praefix: "e",
    aktionFuer: (text) => (istSpeicherschutzMeldung(text) ? knopf : undefined),
  });
  fehler("Speicherschutz: „Nemotron“ wird nicht geladen.");
  fehler("Datei fehlt");
  fehler("Speicherschutz: X", { action: { label: "eigener" } });
  fehler("Speicherschutz: Y", { id: "guard" });
  assert.equal(aufrufe[0].data?.action, knopf);
  assert.equal(aufrufe[1].data?.action, undefined);
  assert.deepEqual(aufrufe[2].data?.action, { label: "eigener" });
  assert.equal(aufrufe[3].data?.action, knopf);
  assert.equal(aufrufe[3].data?.id, "guard");
});

test("Meldungen mit eigener Aktion werden nicht zusammengelegt", () => {
  const { aufrufe, original } = aufzeichner();
  const fehler = buendeln(original, { praefix: "e" });
  const erste = { label: "Wiederholen 1", onClick: () => {} };
  const zweite = { label: "Wiederholen 2", onClick: () => {} };
  fehler("Fehler", { action: erste });
  fehler("Fehler", { action: zweite });
  fehler("Fehler", { description: { type: "span" } });
  fehler("Fehler", { cancel: { label: "Abbrechen" } });
  fehler("Fehler", { onDismiss: () => {} });
  assert.equal(aufrufe[0].data?.action, erste);
  assert.equal(aufrufe[1].data?.action, zweite);
  assert.equal(aufrufe[1].text, "Fehler");
  assert.equal(aufrufe[0].data?.id, undefined);
  assert.equal(aufrufe[2].data?.id, undefined);
  assert.equal(aufrufe[3].data?.id, undefined);
  assert.equal(aufrufe[4].data?.id, undefined);
});

test("Speicherschutz-Meldungen erkennen, auch eingebettet", () => {
  assert.ok(
    istSpeicherschutzMeldung("Speicher knapp: „X“ wurde trotzdem geladen."),
  );
  assert.ok(
    istSpeicherschutzMeldung(
      "Anbieter nicht verfügbar: Zu wenig freier Arbeitsspeicher: 0.9 GB frei",
    ),
  );
  assert.ok(!istSpeicherschutzMeldung("Download fehlgeschlagen"));
});
