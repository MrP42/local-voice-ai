// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/bookStart.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { bookStart, normalizeHeading } from "../../src/lib/reader/bookStart.ts";

interface Case {
  readPreface: boolean;
  startsWith: string;
  hasPreface: boolean;
}
const fixture = JSON.parse(
  readFileSync(new URL("../fixtures/book-start.json", import.meta.url), "utf8"),
) as { books: { name: string; text: string; cases: Case[] }[] };

for (const book of fixture.books) {
  for (const c of book.cases) {
    test(`${book.name} (Vorwort ${c.readPreface ? "mit" : "ohne"})`, () => {
      const r = bookStart(book.text, { readPreface: c.readPreface });
      if (c.startsWith === "") assert.equal(r.offset, 0);
      else
        assert.ok(
          book.text.startsWith(c.startsWith, r.offset),
          `Start bei ${JSON.stringify(book.text.slice(r.offset, r.offset + 40))}`,
        );
      assert.equal(r.hasPreface, c.hasPreface);
    });
  }
}

test("Überschriften werden ohne Markdown und Satzzeichen verglichen", () => {
  assert.equal(normalizeHeading("## *Inhalt:* {{S. 3}}"), "inhalt");
  assert.equal(normalizeHeading("# Über das Buch"), "über das buch");
});

test("leerer Text beginnt vorn", () => {
  assert.deepEqual(bookStart(""), { offset: 0, hasPreface: false });
});
