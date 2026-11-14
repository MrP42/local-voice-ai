// G2 Teil 2 (lva#556): Übersetzungen der Seite „Assistenten“. Jeder Schlüssel, den die Komponenten benutzen, steht in Deutsch
// und Englisch (Mehrzahlformen mit `_one` und `_other`), kein Schlüssel ist zugleich Text und Gruppe (i18next könnte ihn nicht
// auflösen), und kein sichtbarer Text nennt „WAI“. Die dynamischen Schlüssel (`code.${…}`, `fehler.${…}`) werden gegen die
// Listen des Kerns und der Verwaltung geprüft.
// Lauf: node --test scripts/assistent-i18n.test.mjs (Node >= 22.6).
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import * as modul from "node:module";
import test from "node:test";

modul.register("./test-stubs/hooks.mjs", import.meta.url);
globalThis.window = { addEventListener() {}, removeEventListener() {} };

const kern = await import("../src/components/voice-call/assistenten/profil.ts");
const editor = await import("../src/components/voice-call/profil/editor.ts");

const lade = (sprache) =>
  JSON.parse(
    readFileSync(
      new URL(
        `../src/i18n/locales/${sprache}/translation.json`,
        import.meta.url,
      ),
      "utf8",
    ),
  );
const sprachen = { de: lade("de"), en: lade("en") };

const hole = (baum, schluessel) =>
  schluessel
    .split(".")
    .reduce((b, k) => (b && typeof b === "object" ? b[k] : undefined), baum);
const hat = (baum, schluessel) => {
  const wert = hole(baum, schluessel);
  if (typeof wert === "string") return true;
  // Mehrzahl: `schluessel_one` und `schluessel_other`.
  return (
    typeof hole(baum, `${schluessel}_one`) === "string" &&
    typeof hole(baum, `${schluessel}_other`) === "string"
  );
};

const ordner = new URL("../src/components/assistenten/", import.meta.url);
const quellen = readdirSync(ordner)
  .filter((n) => /\.(ts|tsx)$/.test(n))
  .map((n) => readFileSync(new URL(n, ordner), "utf8"));
quellen.push(
  readFileSync(
    new URL(
      "../src/components/voice-call/AssistentPicker.tsx",
      import.meta.url,
    ),
    "utf8",
  ),
);

// Alle Zeichenketten, die mit `assistenten.` beginnen (auch die als Argument weitergereichten Schlüssel).
const gefunden = new Set();
const dynamisch = new Set();
for (const q of quellen) {
  for (const m of q.matchAll(
    /["'`](assistenten\.[A-Za-z0-9_.${}()[\]\->]+)["'`]/g,
  )) {
    (m[1].includes("${") ? dynamisch : gefunden).add(m[1]);
  }
}
const aussprache = [
  "voiceCall.assistant.manage",
  "voiceCall.assistant.loadFailed",
  "voiceCall.assistant.loadUnsafe",
];

test("i18n: jeder benutzte Schlüssel der Seite steht in Deutsch und Englisch", () => {
  assert.ok(gefunden.size > 100, `nur ${gefunden.size} Schlüssel gefunden`);
  for (const [sprache, baum] of Object.entries(sprachen)) {
    const fehlt = [...gefunden, ...aussprache].filter((k) => !hat(baum, k));
    assert.deepEqual(fehlt, [], `${sprache}: fehlende Schlüssel`);
  }
});

test("i18n: dynamische Schlüssel sind für alle ihre Werte übersetzt", () => {
  const erwartet = {
    "assistenten.code.${f.code}": kern.FEHLER_CODES.map(
      (c) => `assistenten.code.${c}`,
    ),
    // Zeilen der Ausspracheregeln: eigene Sätze für Länge, Zeichen und Dublette, sonst der allgemeine Text des Codes.
    "assistenten.aussprache.fehler.${code}": [
      "laenge",
      "muster",
      "doppelt",
    ].map((c) => `assistenten.aussprache.fehler.${c}`),
    "assistenten.aussprache.fehler.liste.${c}": ["zu_viele"].map(
      (c) => `assistenten.aussprache.fehler.liste.${c}`,
    ),
    "assistenten.code.${code}": kern.FEHLER_CODES.map(
      (c) => `assistenten.code.${c}`,
    ),
    "assistenten.code.${c}": kern.FEHLER_CODES.map(
      (c) => `assistenten.code.${c}`,
    ),
    "assistenten.fehler.${grund}": [
      "invalid_id",
      "reserved_id",
      "not_found",
      "too_large",
      "too_many",
      "occupied",
      "write_failed",
      "read_failed",
      "unsafe_path",
      "ungueltig",
      "abgewiesen",
      "zu_gross",
      "leer",
    ].map((c) => `assistenten.fehler.${c}`),
    "assistenten.hoerprobe.fehler.${grund}": [
      "leer",
      "gespraech",
      "synthese",
      "wiedergabe",
    ].map((c) => `assistenten.hoerprobe.fehler.${c}`),
    'assistenten.pfad.${pfad.replace(/\\./g, "_")}': [
      "schema",
      "id",
      "name",
      "beschreibung",
      "anweisungen",
      "begruessung",
      "sprache",
      "stimme",
      "verhalten_stil",
      "verhalten_sprechpause",
      "verhalten_unterbrechen",
      "verhalten_nachfrage_stille_s",
    ].map((c) => `assistenten.pfad.${c}`),
    "assistenten.pfad.aussprache_${feld}": ["wort", "aussprache"].map(
      (c) => `assistenten.pfad.aussprache_${c}`,
    ),
    "assistenten.pfad.${feld}": ["verhalten", "wissen"].map(
      (c) => `assistenten.pfad.${c}`,
    ),
    "assistenten.platzhalter.namen.${n}": editor.PLATZHALTER.map(
      (n) => `assistenten.platzhalter.namen.${n}`,
    ),
    "assistenten.verhalten.stil.${p.verhalten.stil}": kern.STILE.map(
      (s) => `assistenten.verhalten.stil.${s}`,
    ),
    "assistenten.verhalten.pause.${p.verhalten.sprechpause}":
      kern.SPRECHPAUSEN.map((s) => `assistenten.verhalten.pause.${s}`),
    "assistenten.wissen.bereich.${z.schluessel}": kern.BEKANNTE_BEREICHE.map(
      (b) => `assistenten.wissen.bereich.${b}`,
    ),
  };
  const unbekannt = [...dynamisch].filter(
    (k) => !(k in erwartet) && !k.startsWith("assistenten.fehlerFeld.${"),
  );
  assert.deepEqual(
    unbekannt,
    [],
    "ein dynamischer Schlüssel ist hier nicht geprüft",
  );
  for (const [sprache, baum] of Object.entries(sprachen)) {
    for (const keys of Object.values(erwartet)) {
      const fehlt = keys.filter((k) => !hat(baum, k));
      assert.deepEqual(fehlt, [], `${sprache}: fehlende Schlüssel`);
    }
  }
});

test("i18n: kein Schlüssel ist zugleich Text und Gruppe, Deutsch und Englisch haben dieselbe Form", () => {
  const blatt = (baum, vorsatz = "") =>
    Object.entries(baum).flatMap(([k, v]) =>
      v && typeof v === "object"
        ? blatt(v, `${vorsatz}${k}.`)
        : [`${vorsatz}${k}`],
    );
  const de = blatt(sprachen.de.assistenten).sort();
  const en = blatt(sprachen.en.assistenten).sort();
  assert.deepEqual(de, en, "Schlüssel von Deutsch und Englisch weichen ab");
  // Ein Text, dessen Name ein Teil eines anderen Schlüssels ist, wäre in JSON schon ein Fehler beim Laden; hier bleibt die Probe,
  // dass kein Blatt als Vorsatz eines anderen vorkommt.
  for (const k of de)
    assert.ok(
      !de.some((a) => a.startsWith(`${k}.`)),
      `${k} ist Text und Gruppe`,
    );
});

test("i18n: kein sichtbarer Text der Seite nennt WAI", () => {
  for (const [sprache, baum] of Object.entries(sprachen)) {
    const texte = JSON.stringify(baum.assistenten);
    assert.ok(!/\bWAI\b/.test(texte), `${sprache}: „WAI“ im Text`);
    assert.ok(!/\bWAI\b/.test(baum.voiceCall.assistant.manage));
  }
});

test("i18n: deutsche Texte nutzen echte Umlaute (keine Umschrift)", () => {
  const texte = [];
  const sammle = (b) =>
    Object.values(b).forEach((v) =>
      v && typeof v === "object" ? sammle(v) : texte.push(v),
    );
  sammle(sprachen.de.assistenten);
  const verdacht = texte.filter((t) =>
    /\b(fuer|ueber|koennen|waehle|zurueck|Groesse|loesch|Aenderung|Begruessung|ungueltig)/i.test(
      t,
    ),
  );
  assert.deepEqual(verdacht, []);
});
