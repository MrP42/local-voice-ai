// ADR-0028 A2: was der Sprachanruf aus der lokalen Einstufung macht (reine Logik, ohne Tauri).
// Lauf: node --test scripts/assistent-plan.test.mjs (Node >= 22.6; Typen werden beim Laden entfernt).
import assert from "node:assert/strict";
import test from "node:test";

import {
  OHNE_WIRKUNG,
  baueMessung,
  planAssistent,
} from "../src/components/voice-call/assistentPlan.ts";

const s0 = (absicht, antwort) => ({
  stufe: "S0",
  absicht,
  ansage: null,
  antwort,
});

test("ohne Einstufung (Schalter aus, kein Ergebnis): bisheriger Ablauf", () => {
  assert.deepEqual(planAssistent(null), { kind: "weiter" });
  assert.deepEqual(planAssistent(undefined), { kind: "weiter" });
});

test("S0: Vorlagenantwort ohne Modellaufruf", () => {
  const plan = planAssistent(s0("uhrzeit", "Es ist 14:05 Uhr."));
  assert.deepEqual(plan, {
    kind: "antwort",
    text: "Es ist 14:05 Uhr.",
    absicht: "uhrzeit",
    wiederholen: false,
  });
});

test("S0 wiederholen ist als solches gekennzeichnet", () => {
  const plan = planAssistent(s0("wiederholen", "Ich wiederhole."));
  assert.equal(plan.kind, "antwort");
  assert.equal(plan.wiederholen, true);
});

test("S0-Bedienung ohne Wirkung läuft wie bisher, statt etwas Falsches zu behaupten", () => {
  for (const absicht of OHNE_WIRKUNG)
    assert.deepEqual(planAssistent(s0(absicht, "Lauter.")), { kind: "weiter" });
});

test("S0 ohne Antworttext fällt zurück", () => {
  assert.deepEqual(planAssistent(s0("gruss", null)), { kind: "weiter" });
});

test("S1 bis S3: Ansage, danach der bisherige Ablauf", () => {
  for (const stufe of ["S1", "S2", "S3"]) {
    const plan = planAssistent({
      stufe,
      absicht: "mail-lesen",
      ansage: "Ich schaue in deine Mails.",
      antwort: null,
    });
    assert.deepEqual(plan, {
      kind: "ansage",
      text: "Ich schaue in deine Mails.",
      stufe,
      absicht: "mail-lesen",
    });
  }
});

test("S1 ohne Ansagetext fällt zurück", () => {
  assert.deepEqual(
    planAssistent({ stufe: "S1", absicht: "x", ansage: null, antwort: null }),
    { kind: "weiter" },
  );
});

test("Messung: relative Zeiten, fehlende Marken bleiben leer, nie negativ", () => {
  const m = baueMessung(
    "t1",
    { start: 1000, ansage: 1180.4, inhalt: 1900, fertig: 900 },
    "S1",
  );
  assert.deepEqual(m, {
    beitrag_id: "t1",
    erste_ansage_ms: 180,
    erster_inhalt_ms: 900,
    fertig_ms: 0,
    warm: true,
    weg: "lokal",
    stufe: "S1",
  });
  const s0m = baueMessung("t2", { start: 5, inhalt: 25 }, "S0");
  assert.equal(s0m.erste_ansage_ms, null);
  assert.equal(s0m.fertig_ms, null);
  assert.equal(s0m.erster_inhalt_ms, 20);
});

test("S0-Bestätigung geht ans Modell, weil ein Ja meist dessen Frage beantwortet", () => {
  assert.ok(OHNE_WIRKUNG.has("bestaetigung"));
  assert.deepEqual(planAssistent(s0("bestaetigung", "In Ordnung.")), {
    kind: "weiter",
  });
});
