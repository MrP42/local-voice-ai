// lva#943: Prüf- und Listenlogik des Aussprache-Dialogs (src/lib/reader/lexikon.ts).
// Die Regeln müssen dem Backend entsprechen (managers/tts/lexikon/mod.rs: ist_wort,
// ist_aussprache, validieren). Lauf: node --test scripts/lexikon-ui.test.mjs (Node >= 22.6).
import assert from "node:assert/strict";
import * as modul from "node:module";
import test from "node:test";

modul.register("./test-stubs/hooks.mjs", import.meta.url);

const l = await import("../src/lib/reader/lexikon.ts");
const e = (wort, aussprache, gross_klein = false) => ({
  wort,
  aussprache,
  gross_klein,
});

test("Wort: gültige Formen", () => {
  for (const w of ["Microsoft", "GE", "A4", "Rock’n’Roll", "O'Neill", "E-Mail", "Ärzte"])
    assert.equal(l.wortBefund(w), null, w);
});

test("Wort: ungültige Formen", () => {
  assert.equal(l.wortBefund(""), "wort_leer");
  assert.equal(l.wortBefund("zwei Wörter"), "wort_zeichen");
  assert.equal(l.wortBefund("123"), "wort_zeichen");
  assert.equal(l.wortBefund("-ab"), "wort_zeichen");
  assert.equal(l.wortBefund("x".repeat(81)), "wort_zu_lang");
  assert.equal(l.wortBefund("ä".repeat(80)), null);
});

test("Aussprache: Zeichen und Ränder", () => {
  assert.equal(l.ausspracheBefund("Maikrosoft"), null);
  assert.equal(l.ausspracheBefund("Dschi-Ie, Punkt."), null);
  assert.equal(l.ausspracheBefund(" Mai"), "aussprache_zeichen");
  assert.equal(l.ausspracheBefund("Mai!"), "aussprache_zeichen");
  assert.equal(l.ausspracheBefund("[lachen]"), "aussprache_zeichen");
  assert.equal(l.ausspracheBefund("42"), "aussprache_zeichen");
  assert.equal(l.ausspracheBefund(""), "aussprache_leer");
  assert.equal(l.ausspracheBefund("a".repeat(161)), "aussprache_zu_lang");
});

test("Doppelt: Groß/Klein-Regel wie validieren()", () => {
  const b = (liste) => l.befunde(liste).map((z) => z.wort);
  assert.deepEqual(b([e("GE", "a"), e("ge", "b")]), [null, "doppelt"]);
  assert.deepEqual(b([e("GE", "a", true), e("ge", "b", true)]), [null, null]);
  assert.deepEqual(b([e("GE", "a", true), e("GE", "b", true)]), [null, "doppelt"]);
  assert.deepEqual(b([e("GE", "a", true), e("ge", "b")]), [null, "doppelt"]);
});

test("Leerzeilen, Vergleich, Vorschläge übernehmen", () => {
  assert.deepEqual(l.ohneLeerzeilen([e("", ""), e("A", "b")]), [e("A", "b")]);
  assert.equal(l.gleich([e("A", "b")], [e("A", "b")]), true);
  assert.equal(l.gleich([e("A", "b")], [e("A", "b", true)]), false);
  const neu = l.vorschlaegeUebernehmen(
    [e("Microsoft", "Maikrosoft")],
    [e("microsoft", "x"), e("Meeting", "Mieting"), e("meeting", "y")],
  );
  assert.deepEqual(
    neu.map((x) => x.wort),
    ["Microsoft", "Meeting"],
  );
});

test("Markierung als Startwort", () => {
  assert.equal(l.wortAusMarkierung("  „Microsoft“, "), "Microsoft");
  assert.equal(l.wortAusMarkierung("zwei Wörter"), "");
  assert.equal(l.wortAusMarkierung(null), "");
});

test("Dokument wird auf die Byte-Grenze gekürzt, ohne Zeichen zu zerschneiden", () => {
  const kurz = "ä".repeat(10);
  assert.equal(l.dokumentFuerVorschlaege(kurz), kurz);
  const lang = "ä".repeat(l.MAX_DOKUMENT_BYTES / 2 + 1);
  const out = l.dokumentFuerVorschlaege(lang);
  assert.ok(new TextEncoder().encode(out).length <= l.MAX_DOKUMENT_BYTES);
  assert.ok(!out.includes("�"));
});
