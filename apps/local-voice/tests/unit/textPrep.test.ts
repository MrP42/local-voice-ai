// node --experimental-strip-types --test tests/unit/textPrep.test.ts
// lva#783: „Text aufbereiten“ in einem Schritt. Synthetische Texte.
import assert from "node:assert/strict";
import { register } from "node:module";
import test from "node:test";

// Die Module importieren einander ohne Endung (Vite); Node braucht „.ts“.
register(
  "data:text/javascript," +
    encodeURIComponent(
      `export async function resolve(s, c, next) {
        try { return await next(s, c); }
        catch (e) {
          if (s.startsWith(".") && !s.endsWith(".ts")) return next(s + ".ts", c);
          throw e;
        }
      }`,
    ),
);
const { prepareText } = await import("../../src/lib/reader/textPrep.ts");

const RAW = "Das ist  ein Satz mit Fehlern.\nEr geht weiter";

test("Reihenfolge Regeln, Aufbereitung, Korrektur; nur gewählte Schritte", async () => {
  const seen: string[] = [];
  const out = await prepareText(RAW, ["spelling", "rules", "layout"], {
    onStep: (s) => seen.push(s),
    tidy: async (text) => {
      // Die Regeln liefen schon: keine doppelten Leerzeichen mehr.
      assert.ok(!text.includes("  "));
      return "Das ist ein Satz mit Feler.";
    },
    grammar: async (prompt) => {
      assert.match(prompt, /Feler/);
      const n = prompt.split("\n")[0].split("|")[0];
      return `${n}|Das ist ein Satz mit Fehler.`;
    },
  });
  assert.deepEqual(seen, ["rules", "layout", "spelling"]);
  assert.equal(out.text, "Das ist ein Satz mit Fehler.");
  assert.deepEqual(out.changed, ["rules", "layout", "spelling"]);
});

test("Ohne KI-Schritte wird kein Modell gefragt", async () => {
  const out = await prepareText(RAW, ["rules"], {
    tidy: async () => assert.fail("tidy"),
    grammar: async () => assert.fail("grammar"),
  });
  assert.ok(!out.text.includes("  "));
  assert.deepEqual(out.changed, ["rules"]);
});

test("Nichts zu tun: Text bleibt, keine Änderung gemeldet", async () => {
  const text = "Alles gut.";
  const out = await prepareText(text, ["layout", "spelling"], {
    tidy: async (t) => t,
    grammar: async () => "",
  });
  assert.equal(out.text, text);
  assert.deepEqual(out.changed, []);
});

test("Fehler der KI bricht ab und reicht ihn weiter", async () => {
  await assert.rejects(
    prepareText(RAW, ["layout"], {
      tidy: async () => {
        throw new Error("Modell weg");
      },
      grammar: async () => "",
    }),
    /Modell weg/,
  );
});
