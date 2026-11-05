// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/bookSide.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  createBookSide,
  FILLER_DE,
  FILLER_MS,
  SCOPE_ASK_DE,
  type BookSideContext,
  type BookSideTimers,
} from "../../src/components/voice-call/bookSide.ts";

type Handler = (e: unknown) => void;

interface Sent {
  index: number;
  speech: string;
}

const sentence = (id: string, s: Sent) => ({
  kind: "sentence",
  request_id: id,
  sentence: {
    index: s.index,
    text: s.speech,
    speech: s.speech,
    citations: [],
    general: false,
  },
});

const answerOf = (sentences: Sent[], tail = "") => ({
  intent: "question",
  sentences: sentences.map((s) => ({
    index: s.index,
    text: s.speech,
    speech: s.speech,
    citations: [],
    general: false,
  })),
  text: sentences.map((s) => s.speech).join(" "),
  speech: [...sentences.map((s) => s.speech), tail]
    .filter(Boolean)
    .join(" "),
  citations: [],
  found_only: false,
  dropped: 0,
  scope: "until_position",
  frontier: 100,
});

/** Künstliche Uhr: Zeit läuft nur, wenn der Test sie vorstellt. */
function fakeTimers() {
  let now = 0;
  const pending = new Map<number, { at: number; fn: () => void }>();
  let next = 1;
  const timers: BookSideTimers = {
    set(fn, ms) {
      const id = next++;
      pending.set(id, { at: now + ms, fn });
      return id;
    },
    clear(h) {
      pending.delete(h as number);
    },
  };
  return {
    timers,
    advance(ms: number) {
      now += ms;
      for (const [id, t] of [...pending])
        if (t.at <= now) {
          pending.delete(id);
          t.fn();
        }
    },
  };
}

function setup(command: { kind: string } | null = null) {
  const handlers = new Set<Handler>();
  const asks: { id: string; resolve: (a: unknown) => void; reject: (e: unknown) => void }[] =
    [];
  const cancelled: string[] = [];
  const requests: { request_id: string; scope: string; frontier?: number }[] = [];
  const commands: string[] = [];
  const port = {
    ask: (req: { request_id: string }) =>
      new Promise((resolve, reject) =>
        asks.push({ id: req.request_id, resolve, reject }),
      ),
    cancel: async (id: string) => {
      cancelled.push(id);
      return true;
    },
    history: async () => [],
    clear: async () => {},
    command: async () => command,
    subscribe: async (h: Handler) => {
      handlers.add(h);
      return () => handlers.delete(h);
    },
  };
  const ctx: BookSideContext = {
    bookId: "page_1",
    scope: () => "until_position",
    frontier: () => 1234,
    onRequest: (r) => requests.push(r),
    onCommand: (kind) => {
      commands.push(kind);
      return kind === "scope_book" ? "Ganzes Buch einbezogen." : undefined;
    },
  };
  const clock = fakeTimers();
  const side = createBookSide(port as never, ctx, { timers: clock.timers });
  const emit = (e: unknown) => [...handlers].forEach((h) => h(e));
  const tick = () => new Promise((r) => setImmediate(r));
  return { side, emit, asks, cancelled, requests, commands, clock, tick, handlers };
}

const messages = (q: string) => [{ role: "user" as const, content: q }];

test("jeder geprüfte Satz geht sofort an die Stimme, vor dem Ende der Antwort", async () => {
  const t = setup();
  const spoken: string[] = [];
  const done = t.side.chat("r1", messages("Warum zwei Väter?"), (s) => spoken.push(s));
  await t.tick();
  assert.equal(t.requests[0].request_id, "r1");
  assert.equal(t.requests[0].frontier, 1234);
  t.emit(sentence("r1", { index: 0, speech: "Erster Satz." }));
  assert.deepEqual(spoken, ["Erster Satz."], "gesprochen, bevor die Antwort fertig ist");
  t.emit(sentence("r1", { index: 1, speech: "Zweiter Satz." }));
  t.asks[0].resolve(
    answerOf(
      [
        { index: 0, speech: "Erster Satz." },
        { index: 1, speech: "Zweiter Satz." },
      ],
      "Soll ich dir die Stelle vorlesen?",
    ),
  );
  const text = await done;
  assert.deepEqual(spoken, [
    "Erster Satz.",
    "Zweiter Satz.",
    "Soll ich dir die Stelle vorlesen?",
  ]);
  assert.equal(text, spoken.join(" "));
  assert.equal(t.handlers.size, 0, "Abonnement ist abgemeldet");
});

test("ein Satz, dessen Ereignis nach dem Rückgabewert kommt, geht trotzdem genau einmal raus", async () => {
  const t = setup();
  const spoken: string[] = [];
  const done = t.side.chat("r1", messages("Frage"), (s) => spoken.push(s));
  await t.tick();
  t.asks[0].resolve(answerOf([{ index: 0, speech: "Nur aus der Antwort." }]));
  await done;
  assert.deepEqual(spoken, ["Nur aus der Antwort."]);
});

test("Ereignisse einer fremden oder alten request_id werden verworfen", async () => {
  const t = setup();
  const spoken: string[] = [];
  const done = t.side.chat("neu", messages("Frage"), (s) => spoken.push(s));
  await t.tick();
  t.emit(sentence("alt", { index: 0, speech: "Von früher." }));
  t.emit(sentence("neu", { index: 0, speech: "Aktuell." }));
  t.asks[0].resolve(answerOf([{ index: 0, speech: "Aktuell." }]));
  await done;
  assert.deepEqual(spoken, ["Aktuell."]);
});

test("Abbruch ruft book_talk_cancel, verwirft spätere Ereignisse und liefert nichts mehr", async () => {
  const t = setup();
  const spoken: string[] = [];
  const done = t.side.chat("r1", messages("Frage"), (s) => spoken.push(s));
  await t.tick();
  t.emit(sentence("r1", { index: 0, speech: "Vor dem Abbruch." }));
  await t.side.cancel("r1");
  assert.deepEqual(t.cancelled, ["r1"]);
  assert.equal(t.side.activeRequestId(), null);
  t.emit(sentence("r1", { index: 1, speech: "Nach dem Abbruch." }));
  t.emit({ kind: "failed", request_id: "r1", code: "cancelled" });
  t.asks[0].reject(new Error("cancelled"));
  const text = await done;
  assert.deepEqual(spoken, ["Vor dem Abbruch."]);
  assert.equal(text, "Vor dem Abbruch.");
});

test("Abbruch räumt den Füller und die Warteschlange: nach dem Abbruch kommt nichts mehr", async () => {
  const t = setup();
  const spoken: string[] = [];
  const done = t.side.chat("r1", messages("Frage"), (s) => spoken.push(s));
  await t.tick();
  await t.side.cancelActive();
  t.clock.advance(5_000);
  await done;
  assert.deepEqual(spoken, [], "auch der Füller bleibt aus");
});

test("eine neue Frage bricht die laufende ab", async () => {
  const t = setup();
  const first = t.side.chat("a", messages("Erste"), () => {});
  await t.tick();
  const second = t.side.chat("b", messages("Zweite"), () => {});
  await t.tick();
  assert.deepEqual(t.cancelled, ["a"]);
  t.asks[1].resolve(answerOf([{ index: 0, speech: "Zweite Antwort." }]));
  await Promise.all([first, second]);
});

test("Füller nach 600 ms, wenn bis dahin kein inhaltlicher Satz da ist", async () => {
  const t = setup();
  const spoken: string[] = [];
  const done = t.side.chat("r1", messages("Frage"), (s) => spoken.push(s));
  await t.tick();
  t.clock.advance(FILLER_MS - 1);
  assert.deepEqual(spoken, []);
  t.clock.advance(1);
  assert.deepEqual(spoken, [FILLER_DE]);
  t.emit(sentence("r1", { index: 0, speech: "Inhalt." }));
  t.asks[0].resolve(answerOf([{ index: 0, speech: "Inhalt." }]));
  await done;
  assert.deepEqual(spoken, [FILLER_DE, "Inhalt."]);
});

test("kein Füller, wenn der erste Satz vor 600 ms kommt", async () => {
  const t = setup();
  const spoken: string[] = [];
  const done = t.side.chat("r1", messages("Frage"), (s) => spoken.push(s));
  await t.tick();
  t.clock.advance(300);
  t.emit(sentence("r1", { index: 0, speech: "Schnell." }));
  t.clock.advance(1_000);
  t.asks[0].resolve(answerOf([{ index: 0, speech: "Schnell." }]));
  await done;
  assert.deepEqual(spoken, ["Schnell."]);
});

test("ein Sprachbefehl läuft ohne Modell: kein book_talk_ask", async () => {
  const t = setup({ kind: "jump" });
  const spoken: string[] = [];
  const text = await t.side.chat("r1", messages("Spring dahin"), (s) => spoken.push(s));
  assert.deepEqual(t.commands, ["jump"]);
  assert.equal(t.asks.length, 0);
  assert.equal(t.requests.length, 0);
  assert.equal(text, "");
});

test("„Ganzes Buch“ fragt zurück; erst das „Ja“ erweitert den Bereich", async () => {
  // Der Befehlserkenner liefert erst scope_book, dann (bei „Ja“) nichts.
  let reply: { kind: string } | null = { kind: "scope_book" };
  const port = {
    ask: async () => answerOf([]),
    cancel: async () => true,
    history: async () => [],
    clear: async () => {},
    command: async () => reply,
    subscribe: async () => () => {},
  };
  const commands: string[] = [];
  const side = createBookSide(port as never, {
    bookId: "page_1",
    scope: () => "until_position",
    frontier: () => 10,
    onRequest: () => {},
    onCommand: (k) => {
      commands.push(k);
      return k === "scope_book" ? "Ganzes Buch einbezogen." : undefined;
    },
  });
  const spoken: string[] = [];
  await side.chat("r1", messages("Ganzes Buch"), (s) => spoken.push(s));
  assert.deepEqual(spoken, [SCOPE_ASK_DE]);
  assert.deepEqual(commands, [], "noch nicht erweitert");
  reply = null;
  await side.chat("r2", messages("Ja."), (s) => spoken.push(s));
  assert.deepEqual(commands, ["scope_book"]);
  assert.equal(spoken.at(-1), "Ganzes Buch einbezogen.");
});

test("ein anderes Wort nach der Rückfrage lässt den Bereich unverändert", async () => {
  let reply: { kind: string } | null = { kind: "scope_book" };
  const commands: string[] = [];
  const asked: string[] = [];
  const port = {
    ask: async (req: { question: string }) => {
      asked.push(req.question);
      return answerOf([]);
    },
    cancel: async () => true,
    history: async () => [],
    clear: async () => {},
    command: async () => reply,
    subscribe: async () => () => {},
  };
  const side = createBookSide(port as never, {
    bookId: "page_1",
    scope: () => "until_position",
    frontier: () => 10,
    onRequest: () => {},
    onCommand: (k) => {
      commands.push(k);
    },
  });
  await side.chat("r1", messages("Ganzes Buch"), () => {});
  reply = null;
  await side.chat("r2", messages("Nein, wer ist Konrad?"), () => {});
  await side.chat("r3", messages("Ja"), () => {});
  assert.deepEqual(commands, []);
  assert.deepEqual(asked, ["Nein, wer ist Konrad?", "Ja"]);
});
