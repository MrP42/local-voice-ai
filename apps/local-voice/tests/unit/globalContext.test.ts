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
