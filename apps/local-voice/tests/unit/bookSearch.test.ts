// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/bookSearch.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  buildSearchIndex,
  foldText,
  searchBook,
} from "../../src/lib/reader/bookSearch.ts";
import type { ReaderBlock } from "../../src/lib/reader/readerDoc.ts";

/** Seitentext und Blöcke wie aus parseReaderDoc: je Zeile ein Block, `#` als Überschrift. */
function doc(lines: string[]): { text: string; blocks: ReaderBlock[] } {
  let text = "";
  const blocks: ReaderBlock[] = [];
  for (const line of lines) {
    const start = text.length;
    text += line + "\n\n";
    const heading = /^(#{1,3}) /.exec(line);
    const body = heading ? line.slice(heading[0].length) : line;
    const at = start + (heading ? heading[0].length : 0);
    // Seitenmarke als unsichtbarer Lauf.
    const mark = /\{\{S\. \d+\}\}/.exec(body);
    const runs = mark
      ? [
          { text: body.slice(0, mark.index), start: at, end: at + mark.index },
          {
            text: mark[0],
            start: at + mark.index,
            end: at + mark.index + mark[0].length,
            tag: true,
          },
          {
            text: body.slice(mark.index + mark[0].length),
            start: at + mark.index + mark[0].length,
            end: at + body.length,
          },
        ]
      : [{ text: body, start: at, end: at + body.length }];
    blocks.push({
      kind: heading ? "heading" : "paragraph",
      level: heading ? heading[1].length : undefined,
      continues: false,
      marked: false,
      runs,
      start,
      end: start + line.length,
    });
  }
  return { text, blocks };
}

const GRIMM = doc([
  "# Kinder- und Hausmärchen",
  "## KHM 1: Der Froschkönig oder der eiserne Heinrich",
  "In den alten Zeiten, wo das Wünschen noch geholfen hat, lebte ein König.",
  "## KHM 26: Rotkäppchen",
  "Es war einmal eine kleine süße Dirne, die hatte jedermann lieb.",
  "Sie ging über die Straße {{S. 12}}zur Großmutter in den Wald.",
]);

test("Faltung wie die Ablage: klein, ohne Diakritika, ß → ss", () => {
  assert.equal(
    foldText("Großmutter  SÜSSE Æsop Øre"),
    "grossmutter susse aesop ore",
  );
  assert.equal(foldText("Café­haus"), "cafehaus");
});

test("Treffer tragen Quell-Offsets und Ausschnitt", () => {
  const index = buildSearchIndex(GRIMM.blocks);
  const r = searchBook(index, "rotkappchen");
  assert.equal(r.total, 1);
  const hit = r.hits[0];
  assert.equal(GRIMM.text.slice(hit.start, hit.end), "Rotkäppchen");
  assert.equal(hit.match, "Rotkäppchen");
  assert.ok(hit.before.endsWith("KHM 26: "), hit.before);
});

test("ß und ss finden einander, Groß- und Kleinschreibung egal", () => {
  const index = buildSearchIndex(GRIMM.blocks);
  for (const q of ["strasse", "STRASSE", "Straße"]) {
    const r = searchBook(index, q);
    assert.equal(r.total, 1, q);
    assert.equal(GRIMM.text.slice(r.hits[0].start, r.hits[0].end), "Straße");
  }
  // Teilwort mitten im ß: das ganze Zeichen gehört zum Treffer.
  const s = searchBook(index, "sus");
  assert.equal(GRIMM.text.slice(s.hits[0].start, s.hits[0].end), "süß");
});

test("Seitenmarken unterbrechen die Suche nicht", () => {
  const index = buildSearchIndex(GRIMM.blocks);
  const r = searchBook(index, "straße zur großmutter");
  assert.equal(r.total, 1);
  const hit = r.hits[0];
  assert.equal(hit.match, "Straße zur Großmutter");
  assert.ok(GRIMM.text.slice(hit.start, hit.end).includes("{{S. 12}}"));
});

test("mehrere Treffer in Reihenfolge, Begrenzung zählt trotzdem alle", () => {
  const index = buildSearchIndex(GRIMM.blocks);
  const r = searchBook(index, "die", 1);
  assert.equal(r.hits.length, 1);
  assert.equal(r.total, 2);
  const all = searchBook(index, "konig");
  assert.deepEqual(
    all.hits.map((h) => h.match),
    ["könig", "König"],
  );
  assert.ok(all.hits[0].start < all.hits[1].start);
});

test("keine Wortfolge: alle Wörter im selben Absatz", () => {
  const index = buildSearchIndex(GRIMM.blocks);
  const r = searchBook(index, "wald großmutter");
  assert.equal(r.allWords, true);
  assert.equal(r.total, 1);
  assert.equal(r.hits[0].match, "Großmutter");
  // Wörter in verschiedenen Absätzen sind kein Treffer.
  assert.equal(searchBook(index, "rotkäppchen wald").total, 0);
});

test("zu kurze Anfrage sucht nicht", () => {
  const index = buildSearchIndex(GRIMM.blocks);
  assert.equal(searchBook(index, " e ").total, 0);
  assert.equal(searchBook(index, "").total, 0);
});

test("Zeichen außerhalb der BMP (Emoji) verschieben nichts", () => {
  const d = doc(["Ein 🐸 Frosch quakt"]);
  const index = buildSearchIndex(d.blocks);
  const frog = searchBook(index, "frosch").hits[0];
  assert.equal(frog.match, "Frosch");
  assert.equal(d.text.slice(frog.start, frog.end), "Frosch");
  assert.equal(frog.before, "Ein 🐸 ");
  const emoji = searchBook(index, "🐸 f").hits[0];
  assert.equal(emoji.match, "🐸 F");
  assert.equal(d.text.slice(emoji.start, emoji.end), "🐸 F");
});
