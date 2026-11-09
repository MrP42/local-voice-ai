// ADR-0028 A2: was der Sprachanruf aus der lokalen Einstufung macht (reine Logik, ohne Tauri).
// Lauf: node --test scripts/assistent-plan.test.mjs (Node >= 22.6; Typen werden beim Laden entfernt).
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  baueMessung,
  planAssistent,
  sprachwahl,
  stufeVon,
  vorlage,
} from "../src/components/voice-call/assistentPlan.ts";
import {
  LAUTSTAERKE,
  START_STUFEN,
  TEMPO,
  bedienen,
  bedienung,
} from "../src/components/voice-call/bedienung.ts";
import {
  ANNOUNCE,
  BUCH_BRUECKEN,
  ORCHESTRATOR_BRUECKEN,
  istUeberbrueckung,
} from "../src/components/voice-call/ueberbrueckung.ts";

const ok = (einstufung) => ({ art: "ok", einstufung });
const s0 = (absicht, antwort) =>
  ok({ stufe: "S0", absicht, ansage: null, antwort });

test("Einstufung ohne Ergebnis (Ansagen und Vorlagen aus): weiter, das Tor in Rust gilt trotzdem", () => {
  assert.deepEqual(planAssistent(ok(null), "de"), { kind: "weiter" });
});

test("Einstufung fehlgeschlagen: feste Meldung, nie weiter zum Modell", () => {
  assert.deepEqual(planAssistent({ art: "fehler" }, "de"), {
    kind: "fehler",
    text: "Das kann ich gerade nicht sicher beantworten.",
  });
  assert.deepEqual(planAssistent({ art: "fehler" }, "en"), {
    kind: "fehler",
    text: "I can't answer that reliably right now.",
  });
});

test("S0: Vorlagenantwort ohne Modellaufruf", () => {
  assert.deepEqual(planAssistent(s0("uhrzeit", "Es ist 14:05 Uhr."), "de"), {
    kind: "antwort",
    text: "Es ist 14:05 Uhr.",
    absicht: "uhrzeit",
  });
});

test("S0 wiederholen ist ein eigener Plan (Text kommt aus Rust, nicht aus der Vorlage)", () => {
  assert.deepEqual(planAssistent(s0("wiederholen", "Ich wiederhole."), "de"), {
    kind: "wiederholen",
  });
});

test("S0-Bedienung (lauter, leiser, schneller, langsamer) wird lokal ausgeführt, nicht ans Modell", () => {
  for (const art of ["lauter", "leiser", "schneller", "langsamer"])
    assert.deepEqual(planAssistent(s0(art, art + "."), "de"), {
      kind: "bedienung",
      art,
    });
});

test("Bestätigung: S0 mit Vorlage, nach einer Rückfrage (Rust: S1 fortsetzung) zum Modell", () => {
  assert.deepEqual(planAssistent(s0("bestaetigung", "In Ordnung."), "de"), {
    kind: "antwort",
    text: "In Ordnung.",
    absicht: "bestaetigung",
  });
  assert.deepEqual(
    planAssistent(
      ok({ stufe: "S1", absicht: "fortsetzung", ansage: null, antwort: null }),
      "de",
    ),
    { kind: "weiter" },
  );
});

test("Bedienung: Stufen, Grenzen und wahre Meldungen", () => {
  const start = START_STUFEN;
  const l = bedienen("lauter", start, true, "de");
  assert.equal(l.geaendert, true);
  assert.equal(l.stufen.lautstaerke, start.lautstaerke + 1);
  assert.equal(l.text, "Lauter.");
  // Obere Grenze: nichts ändert sich, die Meldung sagt es.
  let st = start;
  for (let i = 0; i < LAUTSTAERKE.length; i++)
    st = bedienen("lauter", st, true, "de").stufen;
  assert.equal(st.lautstaerke, LAUTSTAERKE.length - 1);
  const grenze = bedienen("lauter", st, true, "de");
  assert.deepEqual(
    [grenze.geaendert, grenze.text],
    [false, "Lauter geht es nicht."],
  );
  assert.equal(
    bedienen("leiser", { ...st, lautstaerke: 0 }, true, "en").text,
    "I can't go any quieter.",
  );
  const t = bedienen("schneller", start, true, "de");
  assert.deepEqual([t.stufen.tempo, t.text], [start.tempo + 1, "Schneller."]);
  assert.equal(
    bedienen("langsamer", { ...start, tempo: 0 }, true, "de").text,
    "Langsamer geht es nicht.",
  );
  // Keine Stellgröße (Textmodus): wahre Meldung, nichts geändert.
  assert.deepEqual(bedienen("lauter", start, false, "de"), {
    stufen: start,
    text: "Die Lautstärke kann ich hier nicht ändern.",
    geaendert: false,
  });
  assert.equal(
    bedienen("schneller", start, false, "en").text,
    "I can't change the speed here.",
  );
  assert.equal(
    bedienen("schneller", start, false, "de").text,
    "Das Tempo kann ich hier nicht ändern.",
  );
  assert.equal(bedienung("uhrzeit"), null);
  assert.ok(LAUTSTAERKE[start.lautstaerke] === 1 && TEMPO[start.tempo] === 1);
});

test("S0 ohne Antworttext: feste Meldung (das Tor lässt S0 nicht ans Modell)", () => {
  assert.equal(planAssistent(s0("gruss", null), "de").kind, "fehler");
});

test("S1 bis S3: Ansage, danach das Modell", () => {
  for (const stufe of ["S1", "S2", "S3"]) {
    const plan = planAssistent(
      ok({
        stufe,
        absicht: "mail-lesen",
        ansage: "Ich schaue in deine Mails.",
        antwort: null,
      }),
      "de",
    );
    assert.deepEqual(plan, {
      kind: "ansage",
      text: "Ich schaue in deine Mails.",
      stufe,
      absicht: "mail-lesen",
    });
  }
});

test("S1 ohne Ansagetext fällt auf das Modell zurück (das Tor hat schon eingestuft)", () => {
  assert.deepEqual(
    planAssistent(
      ok({ stufe: "S1", absicht: "x", ansage: null, antwort: null }),
      "de",
    ),
    { kind: "weiter" },
  );
});

test("Sprachwahl: Englisch nur bei en, sonst Deutsch", () => {
  for (const c of ["en", "en-US", "EN_gb"]) assert.equal(sprachwahl(c), "en");
  for (const c of ["de", "de-AT", "fr", "ent", "", null, undefined])
    assert.equal(sprachwahl(c), "de");
});

test("Vorlagen gibt es auf Deutsch und Englisch", () => {
  assert.equal(
    vorlage("nichts-zu-wiederholen", "de"),
    "Ich habe noch nichts gesagt, das ich wiederholen kann.",
  );
  for (const n of ["fehler", "wiederhole", "nichts-zu-wiederholen"])
    assert.notEqual(vorlage(n, "de"), vorlage(n, "en"));
});

test("stufeVon liefert nur bei erfolgreicher Einstufung eine Stufe", () => {
  assert.equal(stufeVon({ art: "fehler" }), null);
  assert.equal(stufeVon(ok(null)), null);
  assert.equal(stufeVon(s0("gruss", "Hallo.")), "S0");
});

test("Messung: relative Zeiten, fehlende Marken bleiben leer, nie negativ", () => {
  const art = { warm: true, weg: "lokal" };
  const m = baueMessung(
    "t1",
    { start: 1000, ansage: 1180.4, inhalt: 1900, fertig: 900 },
    "S1",
    art,
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
  const s0m = baueMessung("t2", { start: 5, inhalt: 25 }, "S0", art);
  assert.equal(s0m.erste_ansage_ms, null);
  assert.equal(s0m.fertig_ms, null);
  assert.equal(s0m.erster_inhalt_ms, 20);
});

test("Messung trägt warm und weg der Engine (kalter erster Schritt, Relay)", () => {
  const m = baueMessung("t3", { start: 0 }, "S2", {
    warm: false,
    weg: "relay",
  });
  assert.equal(m.warm, false);
  assert.equal(m.weg, "relay");
});

test("Überbrückungstexte: bekannt, normalisiert, Inhalt nicht", () => {
  assert.ok(istUeberbrueckung("Moment, ich schaue kurz nach."));
  assert.ok(istUeberbrueckung("  Moment,  ich schaue kurz nach. "));
  assert.ok(istUeberbrueckung(ANNOUNCE.meetings_list));
  assert.ok(istUeberbrueckung("One moment, I am checking the book."));
  assert.ok(!istUeberbrueckung("Die Besprechung beginnt um 10 Uhr."));
});

test("Überbrückungstexte stimmen mit den vendorten und eigenen Quellen überein", () => {
  const rel = (p) => new URL(p, import.meta.url);
  const orchestrator = readFileSync(
    rel("../src/vendor/wai-voice/orchestrator.ts"),
    "utf8",
  );
  for (const t of ORCHESTRATOR_BRUECKEN)
    assert.ok(orchestrator.includes(`"${t}"`), `fehlt im Orchestrator: ${t}`);
  const buch = readFileSync(
    rel("../src/components/voice-call/bookSide.ts"),
    "utf8",
  );
  for (const t of BUCH_BRUECKEN)
    assert.ok(buch.includes(`"${t}"`), `fehlt in bookSide: ${t}`);
  // Eigene Ankündigungen: ANNOUNCE ist die einzige Quelle der Orchestrierung.
  const orch = readFileSync(
    rel("../src/components/voice-call/orchestration.ts"),
    "utf8",
  );
  assert.ok(orch.includes('from "./ueberbrueckung"'));
});
