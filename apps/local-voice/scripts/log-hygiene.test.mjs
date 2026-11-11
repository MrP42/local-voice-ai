// Lva#509: Auszüge und Äußerungen stehen nie im Log. Grep-artige Prüfung der Log-Aufrufe im Pfad des
// Inhaltszugriffs (Rust: Abrufindex, Tor, Einstellungsbefehle; Oberfläche: Kontext, Engine, Freigabe).
// Das ersetzt keine Durchsicht, hält aber ein neues `info!("{text}")` im Pfad sofort auf.
// Lauf: node --test scripts/log-hygiene.test.mjs (Node >= 22.6).
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const wurzel = join(dirname(fileURLToPath(import.meta.url)), "..");
const RUST = [
  "src-tauri/src/app_context/service.rs",
  "src-tauri/src/app_context/retrieve.rs",
  "src-tauri/src/app_context/index.rs",
  "src-tauri/src/app_context/prompt.rs",
  "src-tauri/src/app_context/berechtigung.rs",
  "src-tauri/src/app_context/model.rs",
  "src-tauri/src/commands/voice_tools.rs",
  "src-tauri/src/voice_call/mod.rs",
];
const TS = [
  "src/components/voice-call/globalContext.ts",
  "src/components/voice-call/engine.ts",
  "src/components/voice-call/alleBereiche.ts",
  "src/components/voice-call/SettingsPanel.tsx",
];
/** Namen, die Inhalt tragen: Auszug, Frage, Äußerung, Antwort, Treffer. */
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

for (const rel of RUST) {
  test(`Log-Hygiene Rust: ${rel} loggt keinen Inhalt`, () => {
    const q = ohneTests(lies(rel));
    const logs = aufrufe(
      q,
      /\b(?:log::)?(?:trace|debug|info|warn|error)!\(|\bprintln!\(|\beprintln!\(|\bdbg!\(/g,
    );
    for (const a of logs)
      assert.equal(
        INHALT.test(a),
        false,
        `${rel}: Log-Aufruf mit Inhaltsbezug: ${a.trim().slice(0, 120)}`,
      );
  });
}

for (const rel of TS) {
  test(`Log-Hygiene TypeScript: ${rel} loggt keinen Inhalt`, () => {
    const logs = aufrufe(
      lies(rel),
      /\bconsole\.(?:log|info|warn|error|debug)\(/g,
    );
    for (const a of logs)
      assert.equal(
        INHALT.test(a),
        false,
        `${rel}: console-Aufruf mit Inhaltsbezug: ${a.trim().slice(0, 120)}`,
      );
  });
}

test("Log-Hygiene: die Prüfung erkennt einen Verstoß (Selbsttest)", () => {
  const schlecht = ohneTests('fn f() { log::info!("Auszug: {}", excerpt); }');
  const logs = aufrufe(schlecht, /\b(?:log::)?(?:info)!\(/g);
  assert.equal(logs.length, 1);
  assert.equal(INHALT.test(logs[0]), true);
  const gut = aufrufe('log::info!("Abruf verworfen");', /\b(?:log::)?info!\(/g);
  assert.equal(INHALT.test(gut[0]), false);
});
