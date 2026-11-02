// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/proofread.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  acceptEdits,
  buildReview,
  hashText,
  isLocalEndpoint,
  isLoopbackUrl,
  leadingParagraphs,
  paragraphChanges,
  paragraphSpans,
  rejectEdits,
  remapOffset,
  remapSync,
  replaceParagraphs,
  wordDiff,
} from "../../src/lib/library/proofread.ts";
import { lockBook, withBookLock } from "../../src/lib/library/bookLock.ts";
import { textWithSync } from "../../src/lib/library/transcript.ts";

const PARAS = [
  { text: "das haus ist gross und alt", start_ms: 1000, end_ms: 4000 },
  { text: "dann kam der winter", start_ms: 9000, end_ms: 12000 },
  { text: "es schneite tagelang", start_ms: 20000, end_ms: 23000 },
];
const PREFIX = "# Mein Buch\n\nvon A. Autor\n\n";

test("Absatzgrenzen: Start und Ende je Absatz, Rückweg über join", () => {
  const text = "ab\n\ncde\n\nf";
  assert.deepEqual(paragraphSpans(text), [
    { start: 0, end: 2 },
    { start: 4, end: 7 },
    { start: 9, end: 10 },
  ]);
  assert.equal(paragraphSpans("").length, 1);
});

test("die Zeitachse bleibt auf demselben Absatz, auch wenn Text länger und kürzer wird", () => {
  const { text, sync } = textWithSync(PARAS, PREFIX);
  const fixed = text
    .replace("das haus ist gross und alt", "Das Haus ist groß und alt.")
    .replace("dann kam der winter", "Dann kam der Winter.")
    .replace("es schneite tagelang", "Es schneite tagelang.");
  assert.notEqual(fixed, text);
  const next = remapSync(text, fixed, sync);
  assert.equal(next.length, sync.length);
  const starts = paragraphSpans(fixed).map((s) => s.start);
  // Stützpunkte = Absatzanfänge (die letzten drei Absätze, davor Titelei).
  const firstPara = starts.length - 3;
  next.forEach((p, i) => {
    assert.equal(p.chars, starts[firstPara + i], `Stützpunkt ${i}`);
    assert.equal(p.ms, sync[i].ms, "Zeit bleibt");
    assert.ok(
      fixed
        .slice(p.chars)
        .startsWith(["Das Haus", "Dann kam", "Es schneite"][i]),
    );
  });
});

test("eine Stelle mitten im Absatz bleibt im selben Absatz im gleichen Verhältnis", () => {
  const before = "eins zwei\n\ndrei vier fünf sechs\n\nsieben";
  const after = "Eins, zwei.\n\nDrei, vier, fünf, sechs.\n\nSieben.";
  const at = before.indexOf("fünf");
  const mapped = remapOffset(before, after, at);
  const spans = paragraphSpans(after);
  assert.ok(mapped >= spans[1].start && mapped <= spans[1].end);
  // Absatzanfang bleibt Absatzanfang, Ende bleibt Ende.
  assert.equal(remapOffset(before, after, 0), 0);
  assert.equal(remapOffset(before, after, before.length), after.length);
  assert.equal(
    remapOffset(before, after, before.indexOf("sieben")),
    spans[2].start,
  );
});

test("bei anderer Absatzzahl wird im Verhältnis gerechnet statt falsch zugeordnet", () => {
  assert.equal(remapOffset("aaaa\n\nbbbb", "cc", 5), 1);
});

test("Zeitachse ohne Änderung bleibt gleich, Reihenfolge bleibt aufsteigend", () => {
  const { text, sync } = textWithSync(PARAS, PREFIX);
  assert.deepEqual(remapSync(text, text, sync), sync);
  const shorter = text.replace("das haus ist gross und alt", "Haus.");
  const next = remapSync(text, shorter, sync);
  for (let i = 1; i < next.length; i++)
    assert.ok(next[i].chars >= next[i - 1].chars);
});

test("Wortvergleich markiert nur geänderte Wörter", () => {
  const ops = wordDiff("das haus ist gross", "Das Haus ist groß.");
  const words = (kind: string) =>
    ops
      .filter((o) => o.kind === kind)
      .flatMap((o) => o.text.split(/\s+/))
      .filter(Boolean);
  assert.deepEqual(words("del"), ["das", "haus", "gross"]);
  assert.deepEqual(words("add"), ["Das", "Haus", "groß."]);
  assert.ok(ops.some((o) => o.kind === "same" && o.text.includes("ist")));
  // Aus den Stücken lassen sich beide Fassungen wieder zusammensetzen.
  assert.equal(
    ops
      .filter((o) => o.kind !== "add")
      .map((o) => o.text)
      .join(""),
    "das haus ist gross",
  );
  assert.equal(
    ops
      .filter((o) => o.kind !== "del")
      .map((o) => o.text)
      .join(""),
    "Das Haus ist groß.",
  );
});

test("der Absatzvergleich nennt nur geänderte Absätze mit ihrem Index", () => {
  const a = "eins\n\nzwei\n\ndrei";
  const b = "eins\n\nZwei.\n\ndrei";
  const changes = paragraphChanges(a, b);
  assert.equal(changes.length, 1);
  assert.equal(changes[0].index, 1);
  assert.equal(changes[0].before, "zwei");
  assert.equal(changes[0].after, "Zwei.");
  assert.deepEqual(paragraphChanges(a, a), []);
});

test("Titel- und Autorzeile zählen nicht zu den Absätzen der Anzeige", () => {
  assert.equal(leadingParagraphs("# Buch\n\nvon A. Autor\n\nText"), 2);
  assert.equal(leadingParagraphs("# Buch\n\nText"), 1);
  assert.equal(leadingParagraphs("Text\n\nmehr"), 0);
});

const O = "# T\n\nalt eins\n\nalt zwei\n\nalt drei";
const C = "# T\n\nNeu eins.\n\nNeu zwei.\n\nalt drei";

test("Prüfsumme: gleicher Text gleich, jede Änderung anders", () => {
  assert.equal(hashText(C), hashText(C));
  assert.notEqual(hashText(C), hashText(O));
  assert.notEqual(hashText("ab"), hashText("ba"));
  assert.notEqual(hashText(""), hashText(" "));
});

test("Ersetzen je Absatz: nur wo der Absatz noch der Erwartung entspricht", () => {
  const r = replaceParagraphs(C, [
    { index: 1, expect: "Neu eins.", next: "alt eins" },
    { index: 2, expect: "Eigene Fassung.", next: "alt zwei" },
    { index: 9, expect: "x", next: "y" },
  ]);
  assert.equal(r.text, "# T\n\nalt eins\n\nNeu zwei.\n\nalt drei");
  assert.deepEqual(r.applied, [1]);
  assert.deepEqual(r.skipped, [2, 9]);
});

test("Änderungsansicht: Zustand je Absatz, Alle ablehnen/annehmen rühren Eigenes nicht an", () => {
  const edited = "# T\n\nNeu eins.\n\nMeine zwei.\n\nalt drei";
  const review = buildReview(O, C, [], edited);
  assert.ok(review);
  assert.deepEqual(
    review.items.map((i) => [i.index, i.state]),
    [
      [1, "corrected"],
      [2, "edited"],
    ],
  );
  assert.equal(review.edited, 1);
  const rejected = replaceParagraphs(edited, rejectEdits(review));
  assert.equal(rejected.text, "# T\n\nalt eins\n\nMeine zwei.\n\nalt drei");
  const again = buildReview(O, C, [], rejected.text);
  assert.ok(again);
  assert.equal(again.items[0].state, "rejected");
  assert.equal(
    replaceParagraphs(rejected.text, acceptEdits(again)).text,
    edited,
  );
  // Absätze zusammengelegt: keine Ansicht.
  assert.equal(buildReview(O, C, [], "# T\n\nein Absatz"), null);
});

test("Verworfene Vorschläge: offen, übernommen, bearbeitet", () => {
  const proposal = { index: 3, text: "Alt drei, anders." };
  const open = buildReview(O, C, [proposal], C);
  assert.equal(open?.proposals[0].state, "open");
  const adopted = buildReview(
    O,
    C,
    [proposal],
    C.replace("alt drei", proposal.text),
  );
  assert.equal(adopted?.proposals[0].state, "adopted");
  assert.equal(adopted?.edited, 0);
  const mine = buildReview(O, C, [proposal], C.replace("alt drei", "Eigenes."));
  assert.equal(mine?.proposals[0].state, "edited");
  assert.equal(mine?.edited, 1);
});

test("Lokal nur nach der Adresse: Schleife ja, LAN und Kennung nein", () => {
  for (const url of [
    "http://localhost:11434/v1",
    "http://127.0.0.1:8000",
    "https://127.5.6.7/x",
    "http://[::1]:1234/v1",
  ])
    assert.equal(isLoopbackUrl(url), true, url);
  for (const url of [
    "http://192.168.1.20:11434/v1",
    "http://10.0.0.5/v1",
    "https://api.openai.com/v1",
    "http://localhost.evil.example/v1",
    "http://127.0.0.1.evil.example/v1",
    "http://127.0.0.999/v1",
    "http://0.0.0.0:80",
    "",
  ])
    assert.equal(isLoopbackUrl(url), false, url);
  // „ollama“ im LAN ist nicht lokal; die Kennung allein entscheidet nichts.
  assert.equal(
    isLocalEndpoint({ id: "ollama", base_url: "http://192.168.1.20:11434/v1" }),
    false,
  );
  assert.equal(
    isLocalEndpoint({ id: "custom", base_url: "http://localhost:1234/v1" }),
    true,
  );
  assert.equal(isLocalEndpoint({ id: "local", base_url: "" }), true);
});

test("Sperre je Buch: Aufträge laufen nacheinander, andere Bücher gleichzeitig", async () => {
  const order: string[] = [];
  const slow = withBookLock("a", async () => {
    order.push("a1 start");
    await new Promise((r) => setTimeout(r, 30));
    order.push("a1 ende");
  });
  const next = withBookLock("a", async () => {
    order.push("a2");
  });
  const other = withBookLock("b", async () => {
    order.push("b");
  });
  await Promise.all([slow, next, other]);
  assert.deepEqual(order, ["a1 start", "b", "a1 ende", "a2"]);
  // Fehler geben die Sperre frei.
  await assert.rejects(
    withBookLock("a", async () => {
      throw new Error("x");
    }),
  );
  const release = await lockBook("a");
  release();
});
