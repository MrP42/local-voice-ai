// Lva#509/#511: Auszüge und Äußerungen stehen nie im Log. Grep-artige Prüfung ALLER Log-Aufrufe in den
// Dateien des Inhaltszugriffs (Rust: `app_context/`, `voice_call/`, `commands/voice_tools.rs`;
// Oberfläche: `components/voice-call/`). Ein Log-Aufruf besteht nur, wenn er ein reines Textliteral
// ohne Platzhalter und ohne weitere Argumente ist; alles andere (ein `console.log(payload)`, ein
// `info!("{text}")`) fällt auf, auch ohne Inhaltsnamen, und muss einzeln in `ERLAUBT` stehen (geprüfter
// Wortlaut, mit Grund). Das ersetzt keine Durchsicht, hält aber ein neues Log mit Inhalt sofort auf.
// Lauf: node --test scripts/log-hygiene.test.mjs (Node >= 22.6).
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const wurzel = join(dirname(fileURLToPath(import.meta.url)), "..");

/** Dateien eines Ordners (nicht rekursiv) ohne reine Testdateien (`tests.rs`, `*.test.*`). */
function ordner(rel, endung) {
  return readdirSync(join(wurzel, rel))
    .filter((f) => endung.test(f) && !/^tests\.rs$|\.test\./.test(f))
    .map((f) => `${rel}/${f}`);
}
const RUST = [
  ...ordner("src-tauri/src/app_context", /\.rs$/),
  ...ordner("src-tauri/src/voice_call", /\.rs$/),
  "src-tauri/src/commands/voice_tools.rs",
];
const TS = ordner("src/components/voice-call", /\.tsx?$/);

/**
 * Geprüfte Ausnahmen: genauer Wortlaut der Argumente (Leerraum zusammengefasst) und Grund. Neue
 * Einträge brauchen eine Durchsicht, dass nie Auszug, Frage oder Antwort in die Platzhalter gelangt.
 */
const ERLAUBT = new Map([
  [
    '"voice_tools: {what} nicht lesbar: {e}"',
    "`what` ist ein fester Bereichsname, `e` ein Dateifehler des Systems",
  ],
  [
    '"story send failed", e',
    "nur das Fehlerobjekt des Sendens, kein Gesprächstext",
  ],
]);

/** Namen, die Inhalt tragen (nur noch für den Plausibilitätstest der Erlaubnisliste). */
const INHALT =
  /\b(excerpt|excerpts|question|transcript|utterance|text|content|answer|hits?|prompt|message|messages|req|sentence|piece|userText|heard)\b/i;

function lies(rel) {
  return readFileSync(join(wurzel, rel), "utf8").replace(/\r\n/g, "\n");
}

/** Der Text eines Aufrufs ab der öffnenden Klammer bis zur passenden schließenden. */
function aufrufe(quelle, kopf) {
  const treffer = [];
  for (const m of quelle.matchAll(kopf)) {
    let tiefe = 1;
    let i = m.index + m[0].length;
    const start = i;
    for (; i < quelle.length && tiefe > 0; i++) {
      if (quelle[i] === "(") tiefe++;
      else if (quelle[i] === ")") tiefe--;
    }
    treffer.push(quelle.slice(start, i - 1));
  }
  return treffer;
}

/** Ohne `#[cfg(test)]`-Teil: Tests dürfen Inhalte ausgeben. */
const ohneTests = (rs) => rs.split(/\n#\[cfg\(test\)\]/)[0];
const knapp = (a) => a.replace(/\s+/g, " ").trim().replace(/,$/, "");

/** Rust: genau ein Textliteral ohne `{}`-Platzhalter, keine weiteren Argumente. */
const RUST_LITERAL = /^"(?:[^"\\{}]|\\.)*"$/;
/** TypeScript: nur Textliterale (auch `...`) ohne `${}`, durch Kommas getrennt. */
const TS_LITERAL =
  /^(?:"[^"$]*"|'[^'$]*'|`[^`$]*`)(?:\s*,\s*(?:"[^"$]*"|'[^'$]*'|`[^`$]*`))*$/;

const ruestLog =
  /\b(?:log::)?(?:trace|debug|info|warn|error)!\(|\bprintln!\(|\beprintln!\(|\bdbg!\(/g;
const tsLog = /\bconsole\.(?:log|info|warn|error|debug|trace)\(/g;

const unbedenklich = (a, literal) =>
  literal.test(knapp(a)) || ERLAUBT.has(knapp(a));

test("Log-Hygiene: die Dateilisten sind nicht leer", () => {
  assert.ok(RUST.length >= 8, `Rust-Dateien: ${RUST.length}`);
  assert.ok(TS.length >= 10, `TypeScript-Dateien: ${TS.length}`);
});

for (const rel of RUST) {
  test(`Log-Hygiene Rust: ${rel} loggt nur feste Texte`, () => {
    for (const a of aufrufe(ohneTests(lies(rel)), ruestLog))
      assert.equal(
        unbedenklich(a, RUST_LITERAL),
        true,
        `${rel}: Log-Aufruf mit Platzhalter oder Argument: ${knapp(a).slice(0, 120)}`,
      );
  });
}

for (const rel of TS) {
  test(`Log-Hygiene TypeScript: ${rel} loggt nur feste Texte`, () => {
    for (const a of aufrufe(lies(rel), tsLog))
      assert.equal(
        unbedenklich(a, TS_LITERAL),
        true,
        `${rel}: console-Aufruf mit Argument: ${knapp(a).slice(0, 120)}`,
      );
  });
}

test("Log-Hygiene: die Erlaubnisliste enthält nichts mit Inhaltsbezug", () => {
  for (const wortlaut of ERLAUBT.keys())
    assert.equal(INHALT.test(wortlaut), false, wortlaut);
});

test("Log-Hygiene: die Prüfung erkennt Verstöße auch ohne Inhaltsnamen (Selbsttest)", () => {
  const rs = (q) =>
    aufrufe(ohneTests(q), ruestLog).map((a) => unbedenklich(a, RUST_LITERAL));
  const ts = (q) => aufrufe(q, tsLog).map((a) => unbedenklich(a, TS_LITERAL));
  // Verstöße: Platzhalter, Argument, nicht-literales Argument, Objekt.
  assert.deepEqual(rs('fn f() { log::info!("Auszug: {}", excerpt); }'), [
    false,
  ]);
  assert.deepEqual(rs('fn f() { log::warn!("{text}"); }'), [false]);
  assert.deepEqual(rs('fn f() { log::debug!("Abruf", payload); }'), [false]);
  assert.deepEqual(ts("console.log(payload);"), [false]);
  assert.deepEqual(ts('console.log("Treffer", hits);'), [false]);
  assert.deepEqual(ts("console.warn(`Frage ${frage}`);"), [false]);
  // Erlaubt: feste Texte; ein Log im Testteil zählt nicht.
  assert.deepEqual(rs('log::info!("Abruf verworfen");'), [true]);
  assert.deepEqual(
    rs('fn f() {}\n#[cfg(test)]\nmod t { fn g() { println!("{x}"); } }'),
    [],
  );
  assert.deepEqual(ts('console.error("Start fehlgeschlagen");'), [true]);
  assert.deepEqual(ts('console.error("Start", "fehlgeschlagen");'), [true]);
});
