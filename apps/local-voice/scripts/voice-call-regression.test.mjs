// Fixrunde #831 B: Verträge der Hinweise und des synthetischen Anrufstarts.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const quelle = (pfad) => readFileSync(new URL(pfad, import.meta.url), "utf8");
const panel = quelle("../src/components/voice-call/SettingsPanel.tsx");

test("Gesperrter Sprecher behält seinen erklärenden Alarm in den Einstellungen", () => {
  const alarm = panel.slice(panel.indexOf("{speakerBlocked && ("), panel.indexOf("label={t(\"voiceCall.settings.denker\")}"));
  assert.match(alarm, /role="alert"/);
  assert.match(alarm, /t\("voiceCall.settings.speakerBlocked"\)/);
  const transcript = quelle("../src/components/voice-call/Transcript.tsx");
  assert.match(transcript, /role=\{notice === "speaker_blocked" \? "alert" : "note"\}/);
  assert.match(transcript, /onClick=\{onOpenSettings\}/);
});

test("Gesperrter Denker behält den Hinweis, warum er nicht genutzt wird", () => {
  assert.match(panel, /denkerBlocked\s*\? t\("voiceCall.settings.denkerBlocked"\)/);
});

test("Turnfehler-Tests starten am Chatfeld und prüfen einen laufenden Anruf", () => {
  const spec = quelle("../tests/voice-call.spec.ts");
  const start = spec.slice(spec.indexOf('test.describe("Turnfehler im Anruf"'), spec.indexOf('test.describe("Sprachgespraech Stufe 3'));
  assert.match(start, /__startHang = true/);
  assert.match(start, /getByTestId\("call-button"\)\.click\(\)/);
  assert.doesNotMatch(start, /voice-call-hotkey/);
  // „starting“ allein wäre nur Textmodus: die Engine muss denselben Zustand wie der Store haben.
  assert.match(start, /engine\.setState\("listening"\)/);
  assert.match(start, /engine\.inCall/);
  assert.match(start, /toBe\(true\)/);
  assert.match(start, /toHaveAttribute\("data-state", "active"\)/);
});
