// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/sections.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  jobKey,
  jobsOfTrack,
  mergeSections,
  planSections,
  SECTION_OVERLAP_MS,
  type SectionResult,
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
  assert.ok(
    last.end_ms >= duration,
    "das letzte Stück reicht bis zum Dateiende",
  );
});

const seg = (text: string, start_ms: number, end_ms: number) => ({
  text,
  start_ms,
  end_ms,
});

function sections(): SectionResult[] {
  // Grenze bei 100 s; Abschnitt B beginnt bei 98 s (Versatz), A endet bei 102 s.
  return [
    {
      part: {
        job: "a",
        start_ms: 0,
        end_ms: 102_000,
        from_ms: 0,
        to_ms: 100_000,
      },
      segments: [
        seg("Erster Satz.", 0, 10_000),
        seg("Zweiter Satz vor der Grenze.", 90_000, 99_000),
        seg("Dritter Satz an der Grenze und danach.", 99_000, 102_000),
      ],
    },
    {
      part: {
        job: "b",
        start_ms: 98_000,
        end_ms: 202_000,
        from_ms: 100_000,
        to_ms: null,
      },
      // relativ zu 98 s: der zweite Satz aus A steht hier nicht, der dritte schon.
      segments: [
        seg("Dritter Satz an der Grenze und danach.", 1_000, 4_000),
        seg("Vierter Satz.", 5_000, 15_000),
      ],
    },
  ];
}

test("Segmente werden um den Versatz des Abschnitts verschoben", () => {
  const merged = mergeSections(sections());
  const fourth = merged.find((s) => s.text === "Vierter Satz.");
  assert.deepEqual(fourth, seg("Vierter Satz.", 103_000, 113_000));
});

test("ein an der Grenze doppelt erkannter Satz kommt nur einmal vor", () => {
  const merged = mergeSections(sections());
  assert.deepEqual(
    merged.map((s) => s.text),
    [
      "Erster Satz.",
      "Zweiter Satz vor der Grenze.",
      "Dritter Satz an der Grenze und danach.",
      "Vierter Satz.",
    ],
  );
});

test("ein Segment gehört dem Abschnitt, in dessen Bereich seine Mitte liegt", () => {
  const parts: SectionResult[] = [
    {
      part: {
        job: "a",
        start_ms: 0,
        end_ms: 102_000,
        from_ms: 0,
        to_ms: 100_000,
      },
      segments: [seg("nur im Überlapp von A", 100_500, 101_500)],
    },
    {
      part: {
        job: "b",
        start_ms: 98_000,
        end_ms: 200_000,
        from_ms: 100_000,
        to_ms: null,
      },
      segments: [seg("nur im Überlapp von B", 0, 1_500)],
    },
  ];
  // A: Mitte 101 s ≥ 100 s -> weg. B: Mitte 98,75 s < 100 s -> weg.
  assert.deepEqual(mergeSections(parts), []);
});

test("verschieden geschnittene Segmente derselben Rede: das längere bleibt", () => {
  // A: Mitte 98,45 s (bleibt in A). B: relativ 1–4 s + 98 s Versatz = 99–102 s,
  // Mitte 100,5 s (bleibt in B). Beide überlappen zeitlich und sagen dasselbe.
  const merged = mergeSections([
    {
      part: {
        job: "a",
        start_ms: 0,
        end_ms: 102_000,
        from_ms: 0,
        to_ms: 100_000,
      },
      segments: [seg("und dann ging er nach Hause", 97_000, 99_900)],
    },
    {
      part: {
        job: "b",
        start_ms: 98_000,
        end_ms: 200_000,
        from_ms: 100_000,
        to_ms: null,
      },
      segments: [seg("Und dann ging er nach Hause.", 1_000, 4_000)],
    },
  ]);
  assert.equal(merged.length, 1);
});

test("verschiedene Rede, die sich zeitlich überlappt, bleibt vollständig", () => {
  const merged = mergeSections([
    {
      part: {
        job: "a",
        start_ms: 0,
        end_ms: 102_000,
        from_ms: 0,
        to_ms: 100_000,
      },
      segments: [seg("Guten Morgen miteinander", 97_000, 99_900)],
    },
    {
      part: {
        job: "b",
        start_ms: 98_000,
        end_ms: 200_000,
        from_ms: 100_000,
        to_ms: null,
      },
      segments: [seg("heute geht es um etwas ganz anderes", 2_500, 6_000)],
    },
  ]);
  assert.equal(merged.length, 2);
});

test("ein Abschnitt ohne Überlappung bleibt unverändert", () => {
  const only: SectionResult = {
    part: { job: "x", start_ms: 0, end_ms: null, from_ms: 0, to_ms: null },
    segments: [seg("Eins.", 0, 1_000), seg("Zwei.", 2_000, 3_000)],
  };
  assert.deepEqual(mergeSections([only]), only.segments);
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
