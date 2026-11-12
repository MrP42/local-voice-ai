// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/greeting.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  greetingKey,
  greetingName,
  greetingSlot,
} from "../../src/components/voice-call/greeting.ts";

test("Tageszeit-Grenzen", () => {
  assert.equal(greetingSlot(4), "night");
  assert.equal(greetingSlot(5), "morning");
  assert.equal(greetingSlot(10), "morning");
  assert.equal(greetingSlot(11), "day");
  assert.equal(greetingSlot(17), "day");
  assert.equal(greetingSlot(18), "evening");
  assert.equal(greetingSlot(22), "evening");
  assert.equal(greetingSlot(23), "night");
});

test("Name: Vorname vor Anzeigename, keine Platzhalter oder Mailadressen", () => {
  assert.equal(
    greetingName({ vorname: " Patrick ", anzeigename: "P. W." }),
    "Patrick",
  );
  assert.equal(greetingName({ anzeigename: "Patrick Wolff" }), "Patrick");
  assert.equal(greetingName({ anzeigename: "p@example.org" }), "");
  assert.equal(greetingName({ anzeigename: "Profil 1" }), "");
  // Ersatzname des Kontospeichers: nie „Guten Abend, Lokales.“ (Patrick 12.11.2026).
  assert.equal(greetingName({ anzeigename: "Lokales Profil" }), "");
  assert.equal(greetingName({ anzeigename: " lokales   PROFIL " }), "");
  assert.equal(greetingName({ anzeigename: "Local Profile" }), "");
  assert.equal(
    greetingName({ vorname: "", anzeigename: "Lokales Profil" }),
    "",
  );
  assert.equal(greetingName({ anzeigename: "" }), "");
  assert.equal(greetingName({}), "");
});

test("Schlüssel mit und ohne Namen, Variante wechselt mit dem Startwert", () => {
  assert.deepEqual(greetingKey(9, "Patrick", 0), {
    key: "voiceCall.greeting.morning",
    index: 0,
  });
  assert.deepEqual(greetingKey(20, "", 3), {
    key: "voiceCall.greeting.eveningAnon",
    index: 1,
  });
  assert.equal(greetingKey(14, "P", -7).index, 1);
});
