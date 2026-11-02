// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/libraryCleanup.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  cleanTitle,
  guessFromFileName,
  hasTitleNoise,
} from "../../src/lib/library/names.ts";
import { joinTranscripts } from "../../src/lib/library/transcript.ts";
import { analyzeName } from "../../src/lib/library/audioGroups.ts";

test("ASIN, ISBN und Format-Rauschen fallen weg (Patricks Bibliothek)", () => {
  const cases: [string, string][] = [
    ["Genesis B0DD7W1VFS LC 128 44100 Stereo", "Genesis"],
    [
      "The Technological Re B0DB6CNCKK LC 128 44100 Stereo",
      "The Technological Re",
    ],
    [
      "Der einfache Weg zum B0D37Y6QKC LC 128 44100 Stereo",
      "Der einfache Weg zum",
    ],
    ["Alles Idioten! 3732455823 LC 128 44100 Stereo", "Alles Idioten!"],
    [
      "Wie man Freunde gewi B017XEVWR0 LC 128 44100 Stereo",
      "Wie man Freunde gewi",
    ],
    ["Effi Briest (128 kbps)", "Effi Briest"],
    ["Effi Briest [B0DD7W1VFS]", "Effi Briest"],
    ["Effi Briest 978-3-16-148410-0", "Effi Briest"],
    ["Effi Briest 44100 Hz Stereo", "Effi Briest"],
  ];
  for (const [raw, clean] of cases) assert.equal(cleanTitle(raw), clean, raw);
});

test("echte Titel bleiben unverändert", () => {
  for (const title of [
    "Fahrenheit 451",
    "1984",
    "Harry Potter 2",
    "The 5 AM Club",
    "Mono no aware",
    "2001: Odyssee im Weltraum",
    "Robert Kiyosaki - Rich Dad Poor Dad Disc 3 - 11 - Track 11",
  ]) {
    assert.equal(cleanTitle(title), title, title);
    assert.equal(hasTitleNoise(title), false, title);
  }
  // Nur Rauschen: lieber den alten Titel behalten als einen leeren.
  assert.equal(cleanTitle("B0DD7W1VFS"), "B0DD7W1VFS");
});

test("Import: Dateiname ohne Rauschen, Stamm für den Vorschlag", () => {
  assert.equal(
    guessFromFileName("C:/x/Genesis B0DD7W1VFS LC 128 44100 Stereo.m4b").title,
    "Genesis",
  );
  assert.equal(
    analyzeName("Rich Dad B017XEVWR0 LC 128 44100 Stereo - Track 03").stem,
    "Rich Dad",
  );
});

test("Transkripte zu einem Text: Titelei weg, Zeitachse versetzt", () => {
  const a = "# Teil A\n\nvon X\n\nErster Absatz.\n\nZweiter Absatz.";
  const aStart = a.indexOf("Erster");
  const b = "# Teil B\n\nDritter Absatz.";
  const bStart = b.indexOf("Dritter");
  const joined = joinTranscripts("# Buch\n\n", [
    {
      heading: "Teil 1",
      text: a,
      sync: [
        { chars: aStart, ms: 0 },
        { chars: a.indexOf("Zweiter"), ms: 4000 },
      ],
      offsetMs: 0,
    },
    { heading: "Teil 2", text: "", sync: [], offsetMs: 9000 },
    {
      heading: "Teil 3",
      text: b,
      sync: [{ chars: bStart, ms: 500 }],
      offsetMs: 10_000,
    },
  ]);
  assert.equal(
    joined.text,
    "# Buch\n\n## Teil 1\n\nErster Absatz.\n\nZweiter Absatz.\n\n## Teil 3\n\nDritter Absatz.",
  );
  assert.deepEqual(
    joined.sync.map((p) => p.ms),
    [0, 4000, 10_500],
  );
  for (const [i, word] of ["Erster", "Zweiter", "Dritter"].entries())
    assert.equal(joined.sync[i].chars, joined.text.indexOf(word), word);
});
