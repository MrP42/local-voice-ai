/**
 * Paritaetspruefung: TS-Sprechererkennung (`src/lib/voices/speakerMarkers.ts`)
 * gegen die Rust-Wahrheit (`src-tauri/src/managers/tts/protocol.rs`).
 *
 * Warum eigens: Der Editor faerbt einen Marker als Chip ein und behauptet
 * damit, die Pipeline werde dort die Stimme wechseln. Laufen beide Seiten
 * auseinander, zeigt der Editor Sprecherwechsel, die beim Vorlesen nicht
 * stattfinden (oder umgekehrt Text, der stumm verschluckt wird) — ein Fehler,
 * den kein Compiler und kein Lint findet. Die Faelle unten sind dieselben,
 * die drueben `#[test]` sind; aendert sich der Rust-Parser, muessen beide
 * Seiten zusammen angefasst werden.
 *
 * Lauf: `node scripts/check-speaker-markers.mjs` (Node >= 22.6 fuehrt die
 * TS-Datei ueber sein Type-Stripping direkt aus, es braucht kein Bundling).
 */
import {
  scanSpeakerMarkers,
  speakerMarkerText,
} from "../src/lib/voices/speakerMarkers.ts";

const anna = { id: "anna-id", displayName: "Anna", color: "#f00" };
const mueller = { id: "mueller-id", displayName: "Frau Müller", color: "#0f0" };
const olga = { id: "olga", displayName: "Olga", color: "#00f" };
const all = [anna, mueller, olga];

let fails = 0;
const check = (name, got, want) => {
  if (JSON.stringify(got) === JSON.stringify(want)) {
    console.log("  ok   " + name);
    return;
  }
  fails++;
  console.log(
    "  FAIL " +
      name +
      "\n    ist:  " +
      JSON.stringify(got) +
      "\n    soll: " +
      JSON.stringify(want),
  );
};

const found = (text, speakers = all) =>
  scanSpeakerMarkers(text, speakers).map((m) => [
    m.raw,
    m.speaker.id,
    m.style ?? null,
    m.legacy,
  ]);

console.log("Sprecher-Marker: TS gegen protocol.rs");

check("Spitzklammer am Zeilenanfang", found("<Anna>\nGuten Morgen."), [
  ["<Anna>", "anna-id", null, false],
]);
check("Spitzklammer inline", found("Er sagte: <Anna> Hallo."), [
  ["<Anna>", "anna-id", null, false],
]);
check(
  "Stil am ersten Doppelpunkt abgetrennt",
  found("<Anna:fluesternd> Ganz leise."),
  [["<Anna:fluesternd>", "anna-id", "fluesternd", false]],
);
check("leerer Stil zaehlt als kein Stil", found("<Anna: > Text."), [
  ["<Anna: >", "anna-id", null, false],
]);
check(
  "Umlaute und Grossschreibung matchen",
  found("<FRAU MÜLLER> Guten Tag."),
  [["<FRAU MÜLLER>", "mueller-id", null, false]],
);
check(
  "Alt-Format greift auf den Anzeigenamen",
  found("Frau Müller: Guten Tag."),
  [["Frau Müller:", "mueller-id", null, true]],
);
check(
  "gewoehnlicher Doppelpunkt ist keine Sprecherzeile",
  found("Achtung: nicht vergessen."),
  [],
);
check("unbekannte Spitzklammer schaltet nichts", found("Hallo <div> Welt"), []);
check(
  "alt und neu gemischt",
  found("Frau Müller: Guten Tag.\n<Anna> Hallo zurück."),
  [
    ["Frau Müller:", "mueller-id", null, true],
    ["<Anna>", "anna-id", null, false],
  ],
);
check(
  "Tag hinter der Sprecherzeile bleibt Text",
  found("Anna: [whisper] Ganz leise."),
  [["Anna:", "anna-id", null, true]],
);
check("ohne bekannte Stimmen kein Marker", found("olga: Hallo.", []), []);
check(
  "eingefuegter Marker ist die Spitzklammer-Form",
  [speakerMarkerText(olga), speakerMarkerText(olga, "fluesternd")],
  ["<Olga>", "<Olga:fluesternd>"],
);

// Offsets: davon haengen Chip-Grenze, Ersetzen und Streckenfarbe ab.
const text = "Vorspann.\nAnna: Hallo <Olga> du.";
const markers = scanSpeakerMarkers(text, all);
check(
  "Offsets treffen genau den Marker",
  markers.map((m) => text.slice(m.start, m.end)),
  ["Anna:", "<Olga>"],
);
check(
  "Strecke laeuft bis zum naechsten Marker",
  text.slice(markers[0].end, markers[1].start),
  " Hallo ",
);
check(
  "letzte Strecke laeuft bis zum Textende",
  text.slice(markers[1].end),
  " du.",
);

if (fails > 0) {
  console.error(`\n${fails} Abweichung(en) zur Rust-Seite.`);
  process.exit(1);
}
console.log("\nAlle Faelle stimmen mit protocol.rs ueberein.");
