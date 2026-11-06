// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/sections.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  aggregateStatus,
  jobKey,
  jobsOfTrack,
  mergeSections,
  planSections,
  SECTION_OVERLAP_MS,
} from "../../src/lib/library/sections.ts";

const MIN = 60_000;

test("bis 40 Minuten oder ohne Dauer wird nicht geteilt", () => {
  assert.deepEqual(planSections(null), []);
  assert.deepEqual(planSections(0), []);
  assert.deepEqual(planSections(40 * MIN), []);
  assert.equal(
    planSections(40 * MIN + 1).length,
    3,
    "ceil(40 min + 1 ms / 20 min)",
  );
});

test("Zahl der Abschnitte: aufgerundet auf 20 Minuten, höchstens vier", () => {
  const n = (min: number) => planSections(min * MIN).length;
  assert.equal(n(41), 3);
  assert.equal(n(60), 3);
  assert.equal(n(61), 4);
  assert.equal(n(80), 4);
  assert.equal(n(600), 4);
});

test("die Abschnitte schließen lückenlos an und überlappen an inneren Grenzen", () => {
  const duration = 95 * MIN + 1234;
  const plan = planSections(duration);
  assert.equal(plan.length, 4);
  assert.equal(plan[0].start_ms, 0);
  assert.equal(plan[0].from_ms, 0);
  for (let k = 0; k < plan.length - 1; k++) {
    assert.equal(
      plan[k].to_ms,
      plan[k + 1].from_ms,
      "Zuständigkeit ohne Lücke",
    );
    assert.equal(
      plan[k + 1].start_ms,
      plan[k + 1].from_ms - SECTION_OVERLAP_MS,
    );
    assert.equal(
      plan[k].end_ms,
      (plan[k].to_ms as number) + SECTION_OVERLAP_MS,
    );
  }
  const last = plan[plan.length - 1];
  assert.equal(last.to_ms, null);
  assert.equal(last.end_ms, null, "letzter Abschnitt: keine Obergrenze");
});

const seg = (text: string, start_ms: number, end_ms: number) => ({
  text,
  start_ms,
  end_ms,
});
const S = 1000;
const G = 1000 * S; // Naht bei 1000 s

/** A zuständig bis G, B ab G; beide dekodieren 12 s über die Naht. */
const partA = {
  job: "a",
  start_ms: 0,
  end_ms: G + 12 * S,
  from_ms: 0,
  to_ms: G,
};
const partB = {
  job: "b",
  start_ms: G - 12 * S,
  end_ms: null,
  from_ms: G,
  to_ms: null,
};

/** Alle Wörter genau einmal und in Reihenfolge? */
const textOf = (segments: { text: string }[]) =>
  segments.map((s) => s.text).join(" ");

test("letzter Abschnitt ohne Obergrenze, die anderen mit 12 s Überlappung", () => {
  const plan = planSections(95 * MIN);
  assert.equal(SECTION_OVERLAP_MS, 12_000);
  assert.equal(plan[plan.length - 1].end_ms, null);
  assert.equal(plan[0].end_ms, (plan[0].to_ms as number) + SECTION_OVERLAP_MS);
});

test("Naht, Fall 1: A=[G−6, G+1,5], B=[G−2, G+5] - jedes Wort genau einmal", () => {
  const merged = mergeSections([
    {
      part: partA,
      segments: [
        seg("Es war einmal vor langer Zeit", 900 * S, 950 * S),
        seg(
          "wir gingen gestern zusammen in den Park und dann sahen wir einen Hund",
          G - 6 * S,
          G + 1500,
        ),
      ],
    },
    {
      part: partB,
      // relativ zu G−12 s
      segments: [
        seg(
          "sahen wir einen Hund der laut bellte und rannte weg",
          10 * S,
          17 * S,
        ),
        seg("Danach war es still.", 18 * S, 20 * S),
      ],
    },
  ]);
  assert.equal(
    textOf(merged),
    "Es war einmal vor langer Zeit wir gingen gestern zusammen in den Park und dann sahen wir einen Hund der laut bellte und rannte weg Danach war es still.",
  );
  for (let i = 1; i < merged.length; i++)
    assert.ok(merged[i].start_ms >= merged[i - 1].start_ms, "Zeiten steigen");
});

test("Naht, Fall 2: A=[G−1, G+2], B=[G−2, G+0,5] - kein Loch, kein Doppel", () => {
  const merged = mergeSections([
    {
      part: partA,
      segments: [seg("das ist ein wichtiger Satz heute", G - 1 * S, G + 2 * S)],
    },
    {
      part: partB,
      segments: [
        seg("wir haben das ist ein wichtiger", 10 * S, 12500),
        seg("Satz heute und es geht weiter", 12500, 16 * S),
      ],
    },
  ]);
  assert.equal(
    textOf(merged),
    "das ist ein wichtiger Satz heute und es geht weiter",
  );
});

test("ohne gemeinsame Wörter: Schnitt an der Segmentgrenze von B, ohne Doppel und Loch", () => {
  const merged = mergeSections([
    {
      part: partA,
      segments: [
        seg("erster Teil vor der Naht", G - 8 * S, G - 3 * S),
        seg("völlig anderer Text bei A", G - 3 * S, G + 4 * S),
      ],
    },
    {
      part: partB,
      segments: [
        seg("noch ein Satz davor", 6 * S, 9 * S),
        seg("dies hört B ganz anders", 12 * S + 100, 18 * S),
        seg("und so weiter", 18 * S, 20 * S),
      ],
    },
  ]);
  // Naht G = 12 s in B: die nächste B-Grenze liegt bei 12,1 s; A wird dort gekürzt.
  const text = textOf(merged);
  assert.ok(text.endsWith("dies hört B ganz anders und so weiter"));
  assert.ok(!text.includes("noch ein Satz davor"), "vor dem Schnitt kommt A");
  assert.ok(text.startsWith("erster Teil vor der Naht"));
  for (let i = 1; i < merged.length; i++)
    assert.ok(
      merged[i].start_ms >= merged[i - 1].end_ms - 1,
      "keine Überlappung",
    );
});

test("ein Abschnitt allein bleibt unverändert", () => {
  const only = [seg("Eins.", 0, 1_000), seg("Zwei.", 2_000, 3_000)];
  assert.deepEqual(
    mergeSections([
      {
        part: { job: "x", start_ms: 0, end_ms: null, from_ms: 0, to_ms: null },
        segments: only,
      },
    ]),
    only,
  );
});

test("drei Abschnitte: beide Nähte werden einzeln zusammengesetzt", () => {
  const G2 = 2 * G;
  const mid = {
    job: "m",
    start_ms: G - 12 * S,
    end_ms: G2 + 12 * S,
    from_ms: G,
    to_ms: G2,
  };
  const last = {
    job: "c",
    start_ms: G2 - 12 * S,
    end_ms: null,
    from_ms: G2,
    to_ms: null,
  };
  const merged = mergeSections([
    {
      part: partA,
      segments: [seg("a1 a2 a3 a4 a5 n1 n2 n3 n4 n5", G - 10 * S, G + 5 * S)],
    },
    {
      part: mid,
      segments: [
        seg("n1 n2 n3 n4 n5 b1 b2 b3 b4 b5", 2 * S, 17 * S),
        // abs G2−6 s … G2+1 s
        seg("b6 m1 m2 m3 m4 m5", G + 6 * S, G + 13 * S),
      ],
    },
    { part: last, segments: [seg("m1 m2 m3 m4 m5 z1 z2", 8 * S, 16 * S)] },
  ]);
  assert.equal(
    textOf(merged),
    "a1 a2 a3 a4 a5 n1 n2 n3 n4 n5 b1 b2 b3 b4 b5 b6 m1 m2 m3 m4 m5 z1 z2",
  );
});

test("jobsOfTrack: Abschnitte, sonst der Einzel-Auftrag, sonst keiner", () => {
  assert.deepEqual(jobsOfTrack({ transcript_job: null }), []);
  assert.deepEqual(jobsOfTrack({ transcript_job: "m1" }), [
    { job: "m1", start_ms: 0, end_ms: null, from_ms: 0, to_ms: null },
  ]);
  const parts = [
    { job: "b", start_ms: 0, end_ms: 1, from_ms: 0, to_ms: 1 },
    { job: "a", start_ms: 0, end_ms: null, from_ms: 1, to_ms: null },
  ];
  assert.deepEqual(
    jobsOfTrack({ transcript_job: "m1", transcript_jobs: parts }),
    parts,
  );
  assert.equal(jobKey({ transcript_job: null, transcript_jobs: parts }), "a,b");
});

test("aggregateStatus: fehlgeschlagen erst, wenn nichts mehr in Arbeit ist", () => {
  assert.equal(
    aggregateStatus(["ready", "failed", "processing"]),
    "processing",
  );
  assert.equal(aggregateStatus(["failed", "queued"]), "queued");
  assert.equal(aggregateStatus(["ready", "failed", "queued"]), "processing");
  assert.equal(aggregateStatus(["ready", "failed", "ready"]), "failed");
  assert.equal(
    aggregateStatus(["ready", "cancelled", "ready"]),
    "failed",
    "cancelled wie failed",
  );
  assert.equal(aggregateStatus(["ready", "missing"]), "missing");
  assert.equal(aggregateStatus(["ready", "done"]), "ready");
  assert.equal(aggregateStatus(["queued", "queued"]), "queued");
  assert.equal(aggregateStatus([]), "queued");
});
