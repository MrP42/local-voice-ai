// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/bookTalkTurns.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  citationsOf,
  lastCitation,
  reduceTurns,
  runningTurn,
  type Turn,
  type TurnAction,
} from "../../src/components/settings/tts/booktalk/turns.ts";

const cite = (start: number, page = 2) => ({
  q: 1,
  page,
  chapter_index: 0,
  chapter_title: "Einleitung",
  start,
  end: start + 40,
  quote: "Ich hatte zwei Lehrmeister.",
});

const sent = (id: string, index: number, citations = [cite(10)]) =>
  ({
    kind: "sentence" as const,
    request_id: id,
    sentence: { index, text: `S${index}`, speech: `S${index}`, citations, general: false },
  });

const answer = (sentences: number[], extra: Record<string, unknown> = {}) => ({
  intent: "question" as const,
  sentences: sentences.map((i) => ({
    index: i,
    text: `S${i}`,
    speech: `S${i}`,
    citations: [cite(10)],
    general: false,
  })),
  text: "",
  speech: "",
  citations: [cite(10)],
  found_only: false,
  dropped: 0,
  scope: "until_position" as const,
  frontier: 100,
  ...extra,
});

const begin = (id: string): TurnAction => ({
  type: "begin",
  turn: { id, question: "F", input: "text", scope: "until_position", frontier: 100 },
});

const run = (...actions: TurnAction[]): Turn[] =>
  actions.reduce(reduceTurns, [] as Turn[]);

test("Sätze kommen in Indexreihenfolge, ein Wiederholungsindex ersetzt", () => {
  const turns = run(
    begin("a"),
    { type: "event", event: sent("a", 1) },
    { type: "event", event: sent("a", 0) },
    { type: "event", event: sent("a", 1) },
  );
  assert.deepEqual(
    turns[0].sentences.map((s) => s.index),
    [0, 1],
  );
  assert.equal(turns[0].status, "pending");
});

test("done schließt den Wechsel mit der endgültigen Antwort", () => {
  const turns = run(begin("a"), {
    type: "event",
    event: { kind: "done", request_id: "a", answer: answer([0, 1]) },
  });
  assert.equal(turns[0].status, "done");
  assert.equal(turns[0].sentences.length, 2);
  assert.equal(runningTurn(turns), undefined);
});

test("Ereignisse einer fremden request_id ändern nichts", () => {
  const turns = run(begin("a"), { type: "event", event: sent("fremd", 0) });
  assert.equal(turns[0].sentences.length, 0);
});

test("nach dem Abbruch werden späte Ereignisse der alten Anfrage verworfen", () => {
  const turns = run(
    begin("a"),
    { type: "event", event: sent("a", 0) },
    { type: "cancel", id: "a", reason: "scope" },
    { type: "event", event: sent("a", 1) },
    { type: "event", event: { kind: "done", request_id: "a", answer: answer([0, 1]) } },
    { type: "result", id: "a", answer: answer([0, 1]) },
  );
  assert.equal(turns[0].status, "cancelled");
  assert.equal(turns[0].cancelReason, "scope");
  assert.equal(turns[0].sentences.length, 1, "angezeigte Sätze bleiben, neue kommen nicht dazu");
  assert.equal(turns[0].answer, undefined, "nichts landet als Antwort");
});

test("failed cancelled markiert abgebrochen, andere Codes als Fehler", () => {
  const a = run(begin("a"), {
    type: "event",
    event: { kind: "failed", request_id: "a", code: "cancelled" },
  });
  assert.equal(a[0].status, "cancelled");
  const b = run(begin("b"), { type: "error", id: "b", code: "chat_busy" });
  assert.equal(b[0].status, "failed");
  assert.equal(b[0].error, "chat_busy");
});

test("der Rückgabewert gilt nur, wenn das done-Ereignis fehlt", () => {
  const late = run(begin("a"), { type: "result", id: "a", answer: answer([0]) });
  assert.equal(late[0].status, "done");
  const twice = run(
    begin("a"),
    { type: "event", event: { kind: "done", request_id: "a", answer: answer([0], { dropped: 3 }) } },
    { type: "result", id: "a", answer: answer([0], { dropped: 9 }) },
  );
  assert.equal(twice[0].answer?.dropped, 3);
});

test("Belege ohne Doppelte, letzter Beleg des letzten fertigen Wechsels", () => {
  const turns = run(
    begin("a"),
    { type: "event", event: { kind: "done", request_id: "a", answer: answer([0]) } },
    begin("b"),
  );
  assert.equal(citationsOf(turns[0]).length, 1);
  assert.equal(lastCitation(turns)?.start, 10, "der laufende Wechsel zählt nicht");
});

test("der nachgeladene Verlauf verdrängt eine inzwischen gestellte Frage nicht", () => {
  const stored: Turn = {
    id: "stored-0",
    question: "Alt",
    input: "text",
    scope: "until_position",
    frontier: 10,
    sentences: [],
    status: "done",
  };
  let turns = reduceTurns([], {
    type: "begin",
    turn: { id: "bt-1", question: "Neu", input: "voice", scope: "until_position", frontier: 20 },
  });
  turns = reduceTurns(turns, { type: "load", turns: [stored] });
  assert.deepEqual(
    turns.map((t) => t.id),
    ["stored-0", "bt-1"],
    "Verlauf zuerst, die laufende Frage dahinter",
  );
  assert.equal(runningTurn(turns)?.id, "bt-1");
  // Ein zweites Laden (anderes Buch) ersetzt nur gespeicherte Wechsel.
  turns = reduceTurns(turns, { type: "load", turns: [] });
  assert.deepEqual(turns.map((t) => t.id), ["bt-1"]);
});
