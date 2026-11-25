// lva#882: Anzeige der Firmenrichtlinie. Gesperrt ist nur, was gesetzt UND
// durchgesetzt ist; Backend-Meldungen `richtlinie:<schalter>: …` werden auch
// mitten in einer Fehlerkette erkannt; jede Sperre hat einen Text in de und en.
// Lauf: node --test scripts/firmenrichtlinie.test.mjs (Node >= 22.6).
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const r = await import("../src/lib/firmenrichtlinie.ts");

const ausgabe = (gesetzt, durchgesetzt) => ({
  keine_programm_downloads: false,
  kein_selbstupdate: false,
  kein_autostart: false,
  keine_tastatur_hooks: false,
  keine_besprechungserkennung: false,
  kein_systemton: false,
  ...Object.fromEntries(gesetzt.map((s) => [s, true])),
  herkunft: "datei",
  durchgesetzt,
});

test("gesperrt nur, wenn gesetzt und durchgesetzt", () => {
  assert.equal(r.istGesperrt(null, "kein_autostart"), false);
  assert.equal(
    r.istGesperrt(ausgabe(["kein_autostart"], []), "kein_autostart"),
    false,
  );
  assert.equal(
    r.istGesperrt(ausgabe([], ["kein_autostart"]), "kein_autostart"),
    false,
  );
  assert.equal(
    r.istGesperrt(
      ausgabe(["kein_autostart"], ["kein_autostart"]),
      "kein_autostart",
    ),
    true,
  );
  assert.deepEqual(
    r.gesperrteSchalter(ausgabe([...r.SCHALTER], [...r.SCHALTER])),
    [...r.SCHALTER],
  );
});

test("Backend-Meldung wird erkannt, auch in einer Fehlerkette", () => {
  assert.equal(
    r.richtlinienSchalter("richtlinie:kein_autostart: Autostart ist gesperrt."),
    "kein_autostart",
  );
  assert.equal(
    r.richtlinienSchalter(
      "Download fehlgeschlagen: richtlinie:keine_programm_downloads: Programm-Downloads sind gesperrt.",
    ),
    "keine_programm_downloads",
  );
  assert.equal(
    r.richtlinienSchalter(new Error("richtlinie:kein_systemton: x")),
    "kein_systemton",
  );
  assert.equal(r.richtlinienSchalter("richtlinie:unbekannt: x"), null);
  assert.equal(r.richtlinienSchalter("Netzwerkfehler"), null);
  assert.equal(r.richtlinienSchalter({ code: "x" }), null);
  assert.equal(r.richtlinienSchalter(null), null);
});

test("richtlinienText uebersetzt nur Richtlinienfehler", () => {
  const t = (k) => `T:${k}`;
  assert.equal(
    r.richtlinienText(t, "richtlinie:kein_selbstupdate: y"),
    "T:firmenrichtlinie.gesperrt.kein_selbstupdate",
  );
  assert.equal(r.richtlinienText(t, "anderer Fehler"), null);
});

test("jede Sperre hat einen Text in Deutsch und Englisch", () => {
  for (const sprache of ["de", "en"]) {
    const baum = JSON.parse(
      readFileSync(
        new URL(
          `../src/i18n/locales/${sprache}/translation.json`,
          import.meta.url,
        ),
        "utf8",
      ),
    );
    for (const schalter of r.SCHALTER) {
      const text = baum.firmenrichtlinie?.gesperrt?.[schalter];
      assert.equal(typeof text, "string", `${sprache}: ${schalter}`);
      assert.ok(text.length > 20, `${sprache}: ${schalter} zu kurz`);
      assert.ok(!/WAI/.test(text), `${sprache}: ${schalter} nennt WAI`);
    }
  }
});
