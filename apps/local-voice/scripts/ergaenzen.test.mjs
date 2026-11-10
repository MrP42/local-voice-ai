// Lva#485: Entscheidungslogik für „Ergänzen/Korrigieren“ im Sprachgespräch (reine Funktionen).
// Lauf: node --test scripts/ergaenzen.test.mjs (Node >= 22.6; `.ts` per Type-Stripping).
import assert from "node:assert/strict";
import test from "node:test";

const {
  darfZusammenfuehren,
  entscheide,
  falte,
  fuegeZusammen,
  istRueckmeldung,
  istStoppwort,
  systemMitHinweis,
  unterbrechungsHinweis,
} = await import("../src/components/voice-call/ergaenzen.ts");

const laufend = {
  ausSprache: true,
  vorlage: false,
  ersterSatzFertig: false,
  schonGespeichert: false,
};

test("falte: Groß-/Kleinschreibung, Satzzeichen und Akzente sind egal", () => {
  assert.equal(falte("  Mhm. "), "mhm");
  assert.equal(falte("Ja, genau!"), "ja genau");
  assert.equal(falte("Okay…"), "okay");
  assert.equal(falte("Hm-hm"), "hm hm");
});

test("Rückmeldelaute: die Liste aus dem Auftrag, gefaltet", () => {
  for (const w of [
    "mhm",
    "Mhm.",
    "hm",
    "Hmm",
    "ja",
    "Ja!",
    "okay",
    "OK",
    "ok.",
    "aha",
    "Genau.",
    "stimmt",
    "Verstehe.",
    "Ja, ja.",
    "ja genau",
    "Aha, okay",
  ])
    assert.equal(istRueckmeldung(w), true, w);
});

test("Keine Rückmeldelaute: Inhalt, leere Eingabe, zu lange Ketten", () => {
  for (const w of [
    "",
    "   ",
    "...",
    "ja aber das stimmt so nicht",
    "nein",
    "genau das meinte ich nicht",
    "okay und was ist mit morgen",
    "ja ja ja ja",
    "stopp",
  ])
    assert.equal(istRueckmeldung(w), false, w);
});

test("Stoppwörter: allein, auch mit „mal“/„kurz“/„bitte“", () => {
  for (const w of [
    "stopp",
    "Stopp!",
    "stop",
    "halt",
    "Halt mal",
    "warte",
    "Warte kurz.",
    "moment",
    "Moment!",
    "Einen Moment",
    "Moment mal bitte",
    "stopp stopp",
  ])
    assert.equal(istStoppwort(w), true, w);
});

test("Keine Stoppwörter: in Sätzen, ohne Kernwort, leer", () => {
  for (const w of [
    "",
    "mal",
    "bitte",
    "warte, ich meinte eigentlich morgen",
    "halt den Mund",
    "moment ich habe noch eine frage",
    "ja",
  ])
    assert.equal(istStoppwort(w), false, w);
});

test("Zusammenführen: Äußerungen mit einem Leerzeichen, Ränder bereinigt", () => {
  assert.equal(
    fuegeZusammen("Wie wird das Wetter", "morgen in Hamburg"),
    "Wie wird das Wetter morgen in Hamburg",
  );
  assert.equal(fuegeZusammen(" a ", " b "), "a b");
  assert.equal(fuegeZusammen("", "b"), "b");
  assert.equal(fuegeZusammen("a", "  "), "a");
});

test("Zusammenführen als Kette: dreimal ergänzt ergibt eine Äußerung", () => {
  const eins = fuegeZusammen("Erstens", "zweitens");
  const zwei = fuegeZusammen(eins, "drittens");
  assert.equal(zwei, "Erstens zweitens drittens");
});

test("darfZusammenfuehren: nur eine gesprochene Modellantwort, deren erster Satz noch läuft", () => {
  assert.equal(darfZusammenfuehren(laufend), true);
  assert.equal(
    darfZusammenfuehren({ ...laufend, ersterSatzFertig: true }),
    false,
  );
  assert.equal(darfZusammenfuehren({ ...laufend, ausSprache: false }), false);
  assert.equal(darfZusammenfuehren({ ...laufend, vorlage: true }), false);
  // Schon gespeichert: ohne Rust nicht mehr zu ersetzen.
  assert.equal(
    darfZusammenfuehren({ ...laufend, schonGespeichert: true }),
    false,
  );
});

test("entscheide: Rückmeldelaut während der Antwort setzt sie fort", () => {
  assert.deepEqual(entscheide("Mhm.", laufend), { art: "weiter" });
  assert.deepEqual(entscheide("ja", { ...laufend, ersterSatzFertig: true }), {
    art: "weiter",
  });
});

test("entscheide: Stoppwort hält an, ohne neuen Beitrag", () => {
  assert.deepEqual(entscheide("Moment!", laufend), { art: "stopp" });
  assert.deepEqual(
    entscheide("warte", { ...laufend, ersterSatzFertig: true }),
    {
      art: "stopp",
    },
  );
});

test("entscheide: Rede vor dem ersten Satzende wird zusammengeführt", () => {
  assert.deepEqual(
    entscheide("morgen in Hamburg", laufend, "Wie wird das Wetter"),
    { art: "zusammenfuehren", text: "Wie wird das Wetter morgen in Hamburg" },
  );
});

test("entscheide: späte Rede bleibt ein neuer Beitrag, die Antwort wird abgebrochen", () => {
  assert.deepEqual(
    entscheide(
      "und was ist mit Köln",
      { ...laufend, ersterSatzFertig: true },
      "Wie wird das Wetter",
    ),
    { art: "neu", text: "und was ist mit Köln", abbrechen: true },
  );
});

test("entscheide: ohne zusammenführbaren Schritt (Vorlage, getippt) ein neuer Beitrag", () => {
  assert.deepEqual(entscheide("und morgen", { ...laufend, vorlage: true }), {
    art: "neu",
    text: "und morgen",
    abbrechen: true,
  });
});

test("entscheide: läuft keine Antwort mehr, zählt jede Äußerung als neuer Beitrag", () => {
  assert.deepEqual(entscheide("ja", null), {
    art: "neu",
    text: "ja",
    abbrechen: false,
  });
  assert.deepEqual(entscheide("stopp", null), {
    art: "neu",
    text: "stopp",
    abbrechen: false,
  });
});

test("Unterbrechungshinweis: nur nach unterbrochener Antwort, Sprache folgt der App", () => {
  assert.equal(
    unterbrechungsHinweis(false, "de"),
    null,
    "ohne Unterbrechung kein Hinweis",
  );
  assert.match(unterbrechungsHinweis(true, "de"), /unterbrochen/);
  assert.match(unterbrechungsHinweis(true, "en"), /interrupted/);
  assert.equal(systemMitHinweis("sys", false, "de"), "sys");
  assert.match(systemMitHinweis("sys", true, "de"), /^sys\n\n.*unterbrochen/s);
});
