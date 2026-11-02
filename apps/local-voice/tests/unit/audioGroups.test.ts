// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/audioGroups.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  analyzeName,
  groupAudioFiles,
  splitAudiobookGroups,
} from "../../src/lib/library/audioGroups.ts";

const pad = (n: number) => String(n).padStart(2, "0");

/** 3 Discs mit 11, 12 und 13 Tracks = 36 Dateien, absichtlich durcheinander. */
function richDadFiles(): string[] {
  const counts = [11, 12, 13];
  const files: string[] = [];
  counts.forEach((n, d) => {
    for (let t = 1; t <= n; t++)
      files.push(
        `C:\\Hörbücher\\Rich Dad\\Robert Kiyosaki - Rich Dad Poor Dad Disc ${d + 1} - ${pad(t)} - Track ${pad(t)}.mp3`,
      );
  });
  return files.reverse();
}

test("36 Dateien einer CD-Box werden eine Gruppe in Hörreihenfolge", () => {
  const files = richDadFiles();
  assert.equal(files.length, 36);
  const groups = groupAudioFiles(files);
  assert.equal(groups.length, 1);
  const g = groups[0];
  assert.equal(g.paths.length, 36);
  assert.equal(g.title, "Rich Dad Poor Dad");
  assert.deepEqual(g.authors, ["Robert Kiyosaki"]);
  assert.match(g.paths[0], /Disc 1 - 01 - Track 01/);
  assert.match(g.paths[10], /Disc 1 - 11 - Track 11/);
  assert.match(g.paths[11], /Disc 2 - 01 - Track 01/);
  assert.match(g.paths[35], /Disc 3 - 13 - Track 13/);
});

test("natürliche Sortierung: Track 2 vor Track 10", () => {
  const groups = groupAudioFiles([
    "/b/Der Prozess - Track 10.mp3",
    "/b/Der Prozess - Track 2.mp3",
    "/b/Der Prozess - Track 1.mp3",
  ]);
  assert.equal(groups.length, 1);
  assert.deepEqual(
    groups[0].paths.map((p) => p.split("Track ")[1]),
    ["1.mp3", "2.mp3", "10.mp3"],
  );
});

test("verschiedene Bücher im selben Ordner bleiben getrennt", () => {
  const groups = groupAudioFiles([
    "/d/Franz Kafka - Der Prozess Teil 1.mp3",
    "/d/Franz Kafka - Der Prozess Teil 2.mp3",
    "/d/Theodor Storm - Der Schimmelreiter.m4b",
  ]);
  assert.equal(groups.length, 2);
  assert.equal(groups[0].title, "Der Prozess");
  assert.deepEqual(groups[0].authors, ["Franz Kafka"]);
  assert.equal(groups[0].paths.length, 2);
  assert.equal(groups[1].title, "Der Schimmelreiter");
  assert.deepEqual(groups[1].authors, ["Theodor Storm"]);
});

test("gleicher Titel in verschiedenen Ordnern: zwei Gruppen", () => {
  const groups = groupAudioFiles(["/a/Buch Teil 1.mp3", "/b/Buch Teil 1.mp3"]);
  assert.equal(groups.length, 2);
});

test("CD-Unterordner gehören zum Buch darüber", () => {
  const groups = groupAudioFiles([
    "/h/Effi Briest/CD 2/track01.mp3",
    "/h/Effi Briest/CD 1/track02.mp3",
    "/h/Effi Briest/CD 1/track01.mp3",
  ]);
  assert.equal(groups.length, 1);
  assert.equal(groups[0].title, "Effi Briest");
  assert.deepEqual(groups[0].paths, [
    "/h/Effi Briest/CD 1/track01.mp3",
    "/h/Effi Briest/CD 1/track02.mp3",
    "/h/Effi Briest/CD 2/track01.mp3",
  ]);
});

test("Titel mit Zahl und eine einzelne Datei bleiben ganz", () => {
  const groups = groupAudioFiles(["/h/Ray Bradbury - Fahrenheit 451.mp3"]);
  assert.equal(groups.length, 1);
  assert.equal(groups[0].title, "Fahrenheit 451");
  assert.deepEqual(groups[0].authors, ["Ray Bradbury"]);
});

test("nummerierte Kapitel mit eigenen Titeln sind ein Buch (Ordnername)", () => {
  const groups = groupAudioFiles([
    "/h/Der Hobbit/01 Eine unerwartete Einladung.mp3",
    "/h/Der Hobbit/02 Hammelkeule.mp3",
    "/h/Der Hobbit/03 Kurze Rast.mp3",
  ]);
  assert.equal(groups.length, 1);
  assert.equal(groups[0].title, "Der Hobbit");
  assert.equal(groups[0].paths.length, 3);
});

test("analyzeName entfernt Marker, behält Titel mit Zahl", () => {
  assert.equal(
    analyzeName("Rich Dad Poor Dad Disc 1 - 01 - Track 01").stem,
    "Rich Dad Poor Dad",
  );
  assert.equal(analyzeName("Fahrenheit 451").stem, "Fahrenheit 451");
  assert.equal(analyzeName("Fahrenheit 451").numbered, false);
  assert.equal(analyzeName("01 - Der Prozess").stem, "Der Prozess");
  assert.equal(analyzeName("Der_Prozess_CD2_Track_03").stem, "Der Prozess");
  assert.equal(analyzeName("Track 01").stem, "");
});

// ---------------------------------------------------------------------------
// Zusammenführen: Bücher, die wie Teile aussehen
// ---------------------------------------------------------------------------

type B = Parameters<typeof splitAudiobookGroups>[0][number];
const book = (id: string, title: string, extra: Partial<B> = {}): B =>
  ({
    id,
    title,
    authors: [],
    text: null,
    audio: {
      file: "hoerbuch.mp3",
      duration_ms: null,
      narrator: null,
      transcript_job: null,
      tracks: [],
      sync: [],
    },
    ...extra,
  }) as unknown as B;

test("36 einzeln importierte Teile sind ein Kandidat, in Hörreihenfolge", () => {
  const books: B[] = [];
  let n = 0;
  for (const [d, count] of [
    [1, 11],
    [2, 12],
    [3, 13],
  ] as const)
    for (let t = 1; t <= count; t++)
      books.push(
        book(
          `book_${++n}`,
          `Robert Kiyosaki - Rich Dad Poor Dad Disc ${d} - ${pad(t)} - Track ${pad(t)}`,
        ),
      );
  books.push(book("book_x", "Effi Briest"));
  const shuffled = [...books].reverse();
  const groups = splitAudiobookGroups(shuffled);
  assert.equal(groups.length, 1);
  assert.equal(groups[0].title, "Rich Dad Poor Dad");
  assert.deepEqual(groups[0].authors, ["Robert Kiyosaki"]);
  assert.equal(groups[0].books.length, 36);
  assert.equal(groups[0].books[0].id, "book_1");
  assert.equal(groups[0].books[35].id, "book_36");
});

test("Reihenbände ohne Marker und Bücher mit Text sind keine Kandidaten", () => {
  const groups = splitAudiobookGroups([
    book("a", "Harry Potter 1"),
    book("b", "Harry Potter 2"),
    book("c", "Buch Teil 1", {
      text: { page_id: "p", origin: "transcript", source_name: null },
    }),
    book("d", "Buch Teil 2"),
    book("e", "Ohne Audio Teil 1", { audio: null }),
    book("f", "Ohne Audio Teil 2", { audio: null }),
  ]);
  assert.equal(groups.length, 0);
});

test("wartende Transkription zählt nicht als Text", () => {
  const waiting = {
    file: "hoerbuch.mp3",
    duration_ms: null,
    narrator: null,
    transcript_job: "m1",
    tracks: [],
    sync: [],
  };
  const groups = splitAudiobookGroups([
    book("a", "Der Prozess Teil 1", { audio: waiting }),
    book("b", "Der Prozess Teil 2", { audio: waiting }),
  ]);
  assert.equal(groups.length, 1);
  assert.deepEqual(
    groups[0].books.map((b) => b.id),
    ["a", "b"],
  );
});
