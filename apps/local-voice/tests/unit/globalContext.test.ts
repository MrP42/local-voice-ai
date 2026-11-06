// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/globalContext.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  GlobalContext,
  speakable,
  withContext,
  type ContextHits,
  type ChatMessage,
} from "../../src/components/voice-call/globalContext.ts";

const hit = (k: number) => ({
  k,
  kind: "meeting" as const,
  title: "Runde",
  chapter: null,
  at_ms: 0,
  excerpt: "Frau Berger übernimmt.",
  jump: {
    kind: "meeting" as const,
    id: "m1",
    offset: 0,
    page: null,
    book_id: null,
  },
  score: 1,
  version: "00",
});

function fake(epochs: number[], calls: string[] = []) {
  let i = 0;
  return async <T>(cmd: string, args?: Record<string, unknown>): Promise<T> => {
    calls.push(cmd);
    if (cmd === "app_context_retrieve") {
      const epoch = epochs[Math.min(i++, epochs.length - 1)];
      return {
        hits: [hit(1)],
        searched: ["meeting"],
        retrieve_ms: 1,
        epoch,
      } as T;
    }
    if (cmd === "app_context_message") return "KONTEXT" as T;
    if (cmd === "app_context_verify") {
      assert.ok(args);
      return [1] as T;
    }
    throw new Error(cmd);
  };
}

test("Kontext steht direkt vor der letzten Nutzernachricht", () => {
  const msgs: ChatMessage[] = [
    { role: "system", content: "S" },
    { role: "user", content: "a" },
    { role: "assistant", content: "b" },
    { role: "user", content: "c" },
  ];
  const out = withContext(msgs, "K");
  assert.deepEqual(
    out.map((m) => m.content),
    ["S", "a", "b", "K", "c"],
  );
  assert.equal(withContext(msgs, null), msgs);
});

test("Belegmarken werden nicht gesprochen", () => {
  assert.equal(
    speakable("Frau Berger [Q1]. Bis März [Q2]."),
    "Frau Berger. Bis März.",
  );
  assert.equal(speakable("Kein [Qx] Beleg"), "Kein [Qx] Beleg");
});

test("Ein Abruf je Schritt und Transkript; anderes Transkript holt neu", async () => {
  const calls: string[] = [];
  const g = new GlobalContext(fake([1, 1], calls));
  const a = g.prepare({ turnId: "t", transcript: "x" });
  const b = g.prepare({ turnId: "t", transcript: "x" });
  assert.equal(a, b);
  await a;
  assert.equal(calls.filter((c) => c === "app_context_retrieve").length, 1);
  assert.ok(await g.take("t", "x"));
  assert.equal(await g.take("t", "anders"), null, "Finaltranskript weicht ab");
});

test("Verworfen nach resume; älterer Indexstand ist ungültig", async () => {
  const g = new GlobalContext(fake([1, 1, 2]));
  await g.prepare({ turnId: "t1", transcript: "x" });
  g.discard("t1");
  assert.equal(await g.take("t1", "x"), null);
  await g.prepare({ turnId: "t2", transcript: "x" });
  await g.prepare({ turnId: "t3", transcript: "y" });
  // t2 stammt aus Stand 1, inzwischen gilt Stand 2.
  assert.equal(await g.take("t2", "x"), null);
  assert.ok(await g.take("t3", "y"));
});

test("Fehler im Abruf: Gespräch läuft ohne Kontext weiter", async () => {
  const g = new GlobalContext(async () => {
    throw new Error("weg");
  });
  assert.equal(await g.prepare({ turnId: "t", transcript: "x" }), null);
});

test("Belegprüfung liefert nur bestätigte Treffer", async () => {
  const g = new GlobalContext(fake([1]));
  const ctx = await g.prepare({ turnId: "t", transcript: "x" });
  assert.ok(ctx);
  const ok = await g.verify("Antwort [Q1] [Q5]", ctx!);
  assert.deepEqual(
    ok.map((h) => h.k),
    [1],
  );
  const _unused: ContextHits = ctx!.hits;
  assert.ok(_unused);
});

test("Sprungziel je Quellenart", async () => {
  const { sourceTarget } =
    await import("../../src/components/voice-call/globalContext.ts");
  const j = (kind: string, id: string, book_id: string | null = null) =>
    ({ kind, id, offset: 0, page: null, book_id }) as Parameters<
      typeof sourceTarget
    >[0];
  assert.deepEqual(sourceTarget(j("book", "p1")), { page: "p1" });
  assert.deepEqual(sourceTarget(j("ki_doc", "p2/d1")), { page: "p2" });
  assert.deepEqual(sourceTarget(j("book_note", "n1", "p3")), { page: "p3" });
  assert.equal(sourceTarget(j("book_note", "n1")), null);
  assert.deepEqual(sourceTarget(j("meeting", "m1")), { section: "meetings" });
  assert.deepEqual(sourceTarget(j("dictation", "7")), { section: "history" });
});

test("Wechsel allgemein / im Buch nur als ganze Äußerung", async () => {
  const { modeSwitch, modeSwitchReply } =
    await import("../../src/components/voice-call/globalContext.ts");
  assert.deepEqual(modeSwitch("Allgemein, bitte."), {
    mode: "general",
    german: true,
  });
  assert.deepEqual(modeSwitch("Zurück zum Buch!"), {
    mode: "book",
    german: true,
  });
  assert.deepEqual(modeSwitch("back to the book"), {
    mode: "book",
    german: false,
  });
  assert.equal(modeSwitch("Was steht im Buch über den Leuchtturm?"), null);
  assert.equal(modeSwitch("Ganz allgemein: wer ist Jansen?"), null);
  assert.match(modeSwitchReply("general", true), /allgemein/);
});

test("An den Sprecher gehen nur Treffer, keine fertige Nachricht", async () => {
  const { speakerContext } =
    await import("../../src/components/voice-call/globalContext.ts");
  const g = new GlobalContext(fake([1]));
  const ctx = await g.prepare({ turnId: "t", transcript: "x", german: false });
  assert.deepEqual(speakerContext(ctx), {
    hits: ctx!.hits,
    german: false,
    turn_id: "t",
  });
  assert.equal(speakerContext(null), null);
});

test("Höchstens eine Spekulation gleichzeitig", async () => {
  const g = new GlobalContext(fake([1, 1]));
  await g.prepare({ turnId: "a", transcript: "x" });
  await g.prepare({ turnId: "b", transcript: "y" });
  assert.equal(await g.take("a", "x"), null, "ältere Spekulation verworfen");
  assert.ok(await g.take("b", "y"));
});

test("Parallele Spekulationen laufen nacheinander, Verworfenes liefert nichts", async () => {
  let running = 0;
  let maxRunning = 0;
  let release: (() => void) | null = null;
  const call = async <T>(cmd: string): Promise<T> => {
    if (cmd === "app_context_retrieve") {
      running++;
      maxRunning = Math.max(maxRunning, running);
      await new Promise<void>((r) => (release = r));
      running--;
      return {
        hits: [hit(1)],
        searched: ["meeting"],
        retrieve_ms: 1,
        epoch: 1,
      } as T;
    }
    if (cmd === "app_context_message") return "K" as T;
    throw new Error(cmd);
  };
  const g = new GlobalContext(call);
  const a = g.prepare({ turnId: "a", transcript: "x" });
  const b = g.prepare({ turnId: "b", transcript: "y" });
  await new Promise((r) => setTimeout(r, 0));
  release!();
  assert.equal(await a, null, "a wurde während des Abrufs verworfen");
  await new Promise((r) => setTimeout(r, 0));
  release!();
  const ctx = await b;
  assert.ok(ctx);
  assert.equal(maxRunning, 1, "nie zwei Abrufe gleichzeitig");
  assert.equal(ctx!.turnId, "b");
});
