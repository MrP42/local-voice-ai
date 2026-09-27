import { test, expect } from "@playwright/test";
import { similarTags } from "../src/lib/tags/similarity";

// Vorschlaege fuer freie Beschreibungen muessen zur Stimmung passen -- eine
// Freude-Beschreibung darf nichts Wuetendes vorschlagen (26.09.2026).
const ids = (q: string, lang = "de") => similarTags(q, lang).map((t) => t.id);

test("mood descriptions get suggestions from the same mood", () => {
  const cases: [string, string[], string[]][] = [
    // Beschreibung, mindestens einer davon unter den ersten drei, keiner davon ueberhaupt
    [
      "happy",
      ["joyful", "delight", "delighted"],
      ["angry", "sad", "hysterical"],
    ],
    [
      "müde",
      ["sighing", "exhale", "speaking-slowly", "low-volume"],
      ["excited", "loud"],
    ],
    ["yawning", ["sighing", "exhale", "speaking-slowly"], ["excited", "angry"]],
    [
      "geheimnisvoll",
      ["whisper", "whispering", "low-voice", "low-volume"],
      ["shouting", "loud"],
    ],
    [
      "mysterious",
      ["whisper", "whispering", "low-voice", "low-volume"],
      ["shouting", "angry"],
    ],
    [
      "wütend und laut",
      ["angry", "furious", "shouting", "loud"],
      ["relaxed", "joyful"],
    ],
    ["ängstlich", ["anxious", "scared", "nervous"], ["joyful", "amused"]],
    [
      "traurig leise",
      ["sad", "unhappy", "whisper", "low-volume"],
      ["excited", "amused"],
    ],
    ["erstaunt", ["astonished", "surprised", "shocked"], ["sad"]],
    ["kichernd", ["chuckling", "chuckle", "laughing"], ["sad", "angry"]],
    ["furios", ["furious"], ["relaxed"]],
  ];
  for (const [query, expectedAny, forbidden] of cases) {
    const got = ids(query);
    const top3 = got.slice(0, 3);
    expect(
      top3.some((id) => expectedAny.includes(id)),
      `${query}: ${got.join(", ")}`,
    ).toBe(true);
    for (const f of forbidden) {
      expect(got, `${query} darf ${f} nicht vorschlagen`).not.toContain(f);
    }
  }
});
