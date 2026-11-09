// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/chatGuide.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  GUIDE_STEPS,
  hasSpeakerMarkers,
  splitStory,
  storyPrompt,
} from "../../src/components/voice-call/guide/steps.ts";

test("Schritte: eindeutig, Anfang ohne Ziel, Ende ist das Mitmach-Beispiel", () => {
  const ids = GUIDE_STEPS.map((s) => s.id);
  assert.equal(new Set(ids).size, ids.length);
  assert.equal(GUIDE_STEPS[0].target, undefined);
  assert.equal(ids.at(-1), "story");
});

test("Anweisung nennt nur erlaubte Marker, ohne Sprecher keine Marker", () => {
  const p = storyPrompt({
    topic: "Drache",
    speakers: ["Anton", "Nele", "Bruno", "Mia", "Zoe"],
    language: "de",
  });
  assert.match(p, /<Anton>, <Nele>, <Bruno>, <Mia>\./);
  assert.doesNotMatch(p, /Zoe/);
  assert.match(p, /Thema: Drache/);
  const solo = storyPrompt({ topic: "X", speakers: ["Anton"], language: "en" });
  assert.doesNotMatch(solo, /</);
});

test("Marker erkennen und zerlegen, unbekannte Namen bleiben Text", () => {
  const text = "Titel\n<Anton> Es war einmal.\n<nele> „Hallo!“ <Fremd> bleibt.";
  assert.equal(hasSpeakerMarkers(text, ["Anton", "Nele"]), true);
  assert.equal(hasSpeakerMarkers("<Fremd> x", ["Anton"]), false);
  assert.deepEqual(splitStory(text, ["Anton", "Nele"]), [
    { speaker: null, text: "Titel" },
    { speaker: "Anton", text: "Es war einmal." },
    { speaker: "Nele", text: "„Hallo!“ <Fremd> bleibt." },
  ]);
});
