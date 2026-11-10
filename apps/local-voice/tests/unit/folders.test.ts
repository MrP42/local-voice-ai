// Läuft ohne Zusatzwerkzeuge: node --experimental-strip-types --test tests/unit/folders.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  bookPageIds,
  folderOf,
  followFolder,
  groupPages,
} from "../../src/lib/reader/folders.ts";

const pages = ["p1", "p2", "p3", "p4", "p5"].map((id) => ({ id }));

test("Fassungen eines Buchs landen in einem Ordner, Rest bleibt lose", () => {
  const books = [
    {
      id: "b1",
      title: "Rich Dad Poor Dad",
      text: { page_id: "p3" },
      editions: [
        { lang: "de", text: { page_id: "p3" } },
        { lang: "en", text: { page_id: "p1" } },
      ],
    },
  ];
  const f = groupPages(pages, books);
  assert.deepEqual(f.books, [
    { bookId: "b1", title: "Rich Dad Poor Dad", pageIds: ["p1", "p3"] },
  ]);
  assert.deepEqual(f.loose, ["p2", "p4", "p5"]);
  assert.equal(folderOf(f, "p1"), "b1");
  assert.equal(folderOf(f, "p2"), null);
});

test("Buch ohne vorhandene Seite erscheint nicht", () => {
  const f = groupPages(pages, [
    { id: "h", title: "Nur Hörbuch", text: null, editions: [] },
    { id: "x", title: "Gelöscht", text: { page_id: "weg" } },
  ]);
  assert.deepEqual(f.books, []);
  assert.equal(f.loose.length, 5);
});

test("Seite in zwei Büchern gehört dem ersten", () => {
  const f = groupPages(pages, [
    { id: "a", title: "A", text: { page_id: "p2" } },
    {
      id: "b",
      title: "B",
      text: { page_id: "p2" },
      editions: [{ lang: "en", text: { page_id: "p4" } }],
    },
  ]);
  assert.deepEqual(
    f.books.map((b) => [b.bookId, b.pageIds]),
    [
      ["a", ["p2"]],
      ["b", ["p4"]],
    ],
  );
});

test("bookPageIds ohne Doppel und ohne leere Verweise", () => {
  assert.deepEqual(
    bookPageIds({
      id: "b",
      title: "B",
      text: { page_id: "p1" },
      editions: [
        { lang: "de", text: { page_id: "p1" } },
        { lang: "en", text: null },
      ],
    }),
    ["p1"],
  );
});

import {
  FOLDER_ALL,
  FOLDER_TEXTS,
  bookFolderKey,
  pageLanguages,
  pagesInFolder,
  resolveFolder,
} from "../../src/lib/reader/folders.ts";

test("Auswahl: Buch, Texte, Alle und Rückfall bei gelöschtem Buch", () => {
  const books = [
    {
      id: "b1",
      title: "B",
      text: { page_id: "p2" },
      editions: [{ lang: "en", text: { page_id: "p2" } }],
    },
  ];
  const f = groupPages(pages, books);
  const all = pages.map((p) => p.id);
  assert.deepEqual(pagesInFolder(f, bookFolderKey("b1"), all), ["p2"]);
  assert.deepEqual(pagesInFolder(f, FOLDER_TEXTS, all), [
    "p1",
    "p3",
    "p4",
    "p5",
  ]);
  assert.deepEqual(pagesInFolder(f, FOLDER_ALL, all), all);
  assert.equal(resolveFolder(f, bookFolderKey("weg")), FOLDER_ALL);
  assert.equal(resolveFolder(f, bookFolderKey("b1")), "book:b1");
  assert.equal(pageLanguages(books).get("p2"), "en");
});

test("followFolder: offene Seite außerhalb der Auswahl folgt ihrem Ordner", () => {
  const books = [
    {
      id: "b1",
      title: "Buch",
      text: { page_id: "p1" },
      editions: [{ lang: "de", text: { page_id: "p1" } }],
    },
  ];
  const all = pages.map((p) => p.id);
  const folders = groupPages(pages, books);
  assert.equal(followFolder(folders, ["p2"], all, "p1"), "book:b1");
  assert.equal(followFolder(folders, ["p1"], all, "p2"), "texts");
  assert.equal(followFolder(folders, ["p1"], all, "p1"), null);
  assert.equal(followFolder(folders, [], all, ""), null);
  assert.equal(followFolder(folders, [], all, "gibt-es-nicht"), null);
  // Mit veralteten Büchern (noch keine) gilt p1 als lose: genau deshalb
  // darf der Hook erst nach der jüngsten Antwort folgen.
  assert.equal(followFolder(groupPages(pages, []), [], all, "p1"), "texts");
});

import { FOLDER_BOOKS, showsBooks } from "../../src/lib/reader/folders.ts";

test("Ordner „Bücher“ (lva#435): Seiten aller Bücher, Bücher nur bei Bedarf", () => {
  const books = [
    { id: "a", title: "A", text: { page_id: "p4" } },
    {
      id: "b",
      title: "B",
      text: { page_id: "p2" },
      editions: [{ lang: "en", text: { page_id: "p5" } }],
    },
  ];
  const f = groupPages(pages, books);
  const all = pages.map((p) => p.id);
  // Reihenfolge der Seitenliste, nicht der Bibliothek.
  assert.deepEqual(pagesInFolder(f, FOLDER_BOOKS, all), ["p2", "p4", "p5"]);
  assert.equal(resolveFolder(f, FOLDER_BOOKS), FOLDER_BOOKS);
  // Auch ohne Bücher bleibt die Wahl gültig (Leerzustand mit Hinweis).
  const leer = groupPages(pages, []);
  assert.equal(resolveFolder(leer, FOLDER_BOOKS), FOLDER_BOOKS);
  assert.deepEqual(pagesInFolder(leer, FOLDER_BOOKS, all), []);
  assert.equal(showsBooks(FOLDER_BOOKS), true);
  assert.equal(showsBooks(bookFolderKey("a")), true);
  assert.equal(showsBooks(FOLDER_ALL), false);
  assert.equal(showsBooks(FOLDER_TEXTS), false);
  // Eine Buchseite bleibt unter „Bücher“ sichtbar: keine Seitenfolge.
  assert.equal(followFolder(f, ["p2", "p4", "p5"], all, "p4"), null);
});
