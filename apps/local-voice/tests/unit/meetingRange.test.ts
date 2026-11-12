// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/meetingRange.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  coversAll,
  formatRangeTime,
  fullRange,
  normalizeRange,
  parseRangeTime,
  rangeLength,
  recordingDurationMs,
} from "../../src/lib/meetingRange.ts";

test("Zeit wird als m:ss unter einer Stunde und als h:mm:ss darüber gezeigt", () => {
  assert.equal(formatRangeTime(0), "0:00");
  assert.equal(formatRangeTime(65_000), "1:05");
  assert.equal(formatRangeTime(3_599_999), "59:59");
  assert.equal(formatRangeTime(3_600_000), "1:00:00");
  // 918 Minuten, die Aufnahme aus dem Anlass.
  assert.equal(formatRangeTime(918 * 60_000), "15:18:00");
});

test("Eingaben werden tolerant gelesen", () => {
  assert.equal(parseRangeTime("1:05"), 65_000);
  assert.equal(parseRangeTime("  1:05  "), 65_000);
  assert.equal(parseRangeTime("1:00:00"), 3_600_000);
  assert.equal(parseRangeTime("0:01:05"), 65_000);
  assert.equal(parseRangeTime("90"), 90_000);
  assert.equal(parseRangeTime("90:00"), 5_400_000);
  assert.equal(parseRangeTime("1:02,5"), 63_000);
  assert.equal(parseRangeTime("1:02.4"), 62_000);
  assert.equal(parseRangeTime("0"), 0);
});

test("Unlesbares ergibt null", () => {
  for (const bad of [
    "",
    "  ",
    "abc",
    "-5",
    "1:-5",
    "1:2:3:4",
    "1.5:30",
    "1:",
    ":30",
    "1h",
  ]) {
    assert.equal(parseRangeTime(bad), null, bad);
  }
});

test("Hin und zurück: was formatiert wurde, wird wieder gelesen", () => {
  for (const ms of [0, 1_000, 59_000, 61_000, 3_661_000, 55_080_000]) {
    assert.equal(parseRangeTime(formatRangeTime(ms)), ms);
  }
});

test("Der Bereich bleibt in der Aufnahme und hat mindestens eine Sekunde", () => {
  const d = 60_000;
  assert.deepEqual(
    normalizeRange({ start_ms: -5_000, end_ms: 200_000 }, d, "start"),
    { start_ms: 0, end_ms: d },
  );
  // Der Anfang stößt an das Ende, nicht umgekehrt.
  assert.deepEqual(
    normalizeRange({ start_ms: 50_000, end_ms: 30_000 }, d, "start"),
    { start_ms: 29_000, end_ms: 30_000 },
  );
  // Das Ende stößt an den Anfang.
  assert.deepEqual(
    normalizeRange({ start_ms: 30_000, end_ms: 10_000 }, d, "end"),
    { start_ms: 30_000, end_ms: 31_000 },
  );
  // Am Rand rückt die andere Grenze mit.
  assert.deepEqual(
    normalizeRange({ start_ms: 60_000, end_ms: 60_000 }, d, "start"),
    { start_ms: 59_000, end_ms: 60_000 },
  );
  assert.deepEqual(normalizeRange({ start_ms: 0, end_ms: 0 }, d, "end"), {
    start_ms: 0,
    end_ms: 1_000,
  });
});

test("Ganze Aufnahme, Länge und Dauer", () => {
  assert.deepEqual(fullRange(5_000), { start_ms: 0, end_ms: 5_000 });
  assert.equal(coversAll({ start_ms: 0, end_ms: 5_000 }, 5_000), true);
  assert.equal(coversAll({ start_ms: 1_000, end_ms: 5_000 }, 5_000), false);
  assert.equal(coversAll({ start_ms: 0, end_ms: 4_000 }, 5_000), false);
  assert.equal(rangeLength({ start_ms: 1_000, end_ms: 4_500 }), 3_500);
  assert.equal(recordingDurationMs(null), null);
  assert.equal(recordingDurationMs(0), null);
  assert.equal(recordingDurationMs(1_000), 1_000);
});
