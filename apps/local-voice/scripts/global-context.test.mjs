// Lva#509 Negativtests des Inhaltszugriffs im Sprachgespräch (Oberflächenseite): kein Auszug geht an das
// Modell, wenn der Zugriff während eines Abrufs oder einer laufenden Antwort widerrufen wird. Die
// Durchsetzung im Kern steht in `app_context/tests.rs` (`admissible`, `an_version_gebunden`).
// Lauf: node --test scripts/global-context.test.mjs (Node >= 22.6).
import assert from "node:assert/strict";
import test from "node:test";

const { GlobalContext, speakerContext } =
  await import("../src/components/voice-call/globalContext.ts");

const TREFFER = {
  k: 1,
  kind: "meeting",
  title: "Runde",
  chapter: null,
  at_ms: 0,
  excerpt: "Geheimer Auszug",
  jump: { kind: "meeting", id: "m1", offset: 0, page: null, book_id: null },
  score: 1,
  version: "00",
};

/** Kern als Attrappe: `abruf` liefert die Treffer, `nachricht` die Kontextnachricht (oder null). */
function kern({ abruf, nachricht = () => "Kontext [Q1]" }) {
  const aufrufe = [];
  const call = async (cmd, args) => {
    aufrufe.push(cmd);
    if (cmd === "app_context_retrieve") return abruf();
    if (cmd === "app_context_message") return nachricht(args);
    throw new Error(`unerwartet: ${cmd}`);
  };
  return { call, aufrufe };
}

const mitTreffern = () => ({
  hits: [TREFFER],
  searched: ["meeting"],
  retrieve_ms: 1,
  epoch: 1,
  question_hash: "h",
  berechtigung: 3,
});
const widerrufen = () => ({
  hits: [],
  searched: [],
  retrieve_ms: 1,
  epoch: 1,
  question_hash: "h",
  berechtigung: 3,
  berechtigung_geaendert: true,
});

test("Widerruf während des Abrufs: das Ergebnis trägt keine Auszüge und es geht nichts an das Modell", async () => {
  const k = kern({ abruf: widerrufen });
  const g = new GlobalContext(k.call);
  await g.prepare({ turnId: "t1", transcript: "Wer übernimmt?" });
  const ctx = await g.take("t1", "Wer übernimmt?");
  assert.equal(ctx.hits.hits.length, 0);
  assert.equal(ctx.hits.berechtigung_geaendert, true);
  assert.equal(ctx.message, null);
  assert.equal(speakerContext(ctx), null, "kein Kontext an voice_call_chat");
  assert.equal(
    k.aufrufe.includes("app_context_message"),
    false,
    "ohne Auszüge wird gar keine Kontextnachricht gebaut",
  );
});

test("Widerruf zwischen Abruf und Kontextnachricht: der Kern liefert keine Nachricht, die Auszüge gehen nicht weiter", async () => {
  const k = kern({ abruf: mitTreffern, nachricht: () => null });
  const g = new GlobalContext(k.call);
  await g.prepare({ turnId: "t1", transcript: "Wer übernimmt?" });
  const ctx = await g.take("t1", "Wer übernimmt?");
  assert.equal(ctx.hits.hits.length, 1);
  assert.equal(ctx.message, null);
  assert.equal(speakerContext(ctx), null);
});

test("Ohne Widerruf geht der Kontext an das Modell (Gegenprobe)", async () => {
  const k = kern({ abruf: mitTreffern });
  const g = new GlobalContext(k.call);
  await g.prepare({ turnId: "t1", transcript: "Wer übernimmt?" });
  const ctx = await g.take("t1", "Wer übernimmt?");
  const an = speakerContext(ctx);
  assert.equal(an.turn_id, "t1");
  assert.equal(an.hits.hits.length, 1);
});

test("Widerruf während der laufenden Antwort: der vorbereitete Kontext ist verworfen, eine Folgerunde bekommt keine Auszüge", async () => {
  let zugriff = true;
  const k = kern({ abruf: () => (zugriff ? mitTreffern() : widerrufen()) });
  const g = new GlobalContext(k.call);
  await g.prepare({ turnId: "t1", transcript: "Wer übernimmt?" });
  assert.ok(speakerContext(await g.take("t1", "Wer übernimmt?")));
  // Der Zugriff wird widerrufen (Oberfläche: `clear`), die nächste Runde fragt neu.
  zugriff = false;
  g.clear();
  assert.equal(await g.take("t1", "Wer übernimmt?"), null);
  await g.prepare({ turnId: "t2", transcript: "Und bis wann?" });
  const folge = await g.take("t2", "Und bis wann?");
  assert.equal(speakerContext(folge), null, "die Folgerunde bekommt nichts");
});

test("Der Beleg-Abgleich verwirft Marken, wenn der Kern keine gültigen meldet (Widerruf nach dem Abruf)", async () => {
  const call = async (cmd) => {
    if (cmd === "app_context_verify") return [];
    throw new Error("unerwartet");
  };
  const g = new GlobalContext(call);
  const ctx = { hits: mitTreffern(), message: "x", turnId: "t1" };
  assert.deepEqual(await g.verify("Antwort [Q1]", ctx), []);
});
