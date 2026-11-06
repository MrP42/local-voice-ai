/**
 * KI-Einordnung der Bücher (lva#354). An das Modell gehen nur Titel, Autor
 * und Klappentext (Codex A3), nie Lesedaten. Handkorrekturen (`hand`)
 * werden nie überschrieben.
 */
import { commands, type ClassifyInput, type LibraryBook } from "@/bindings";
import { CATEGORIES } from "./readingStats";

/** Felder, die das Modell sieht (für den Hinweis vor dem Senden). */
export const SENT_FIELDS = ["title", "author", "description"] as const;

export function classifyInput(
  book: LibraryBook,
  knownTopics: string[],
  label: (key: string, de: string) => string = (_k, de) => de,
): ClassifyInput {
  return {
    title: book.title,
    authors: book.authors,
    description: book.description ?? null,
    sample: null,
    categories: CATEGORIES.map((c) => ({
      key: c.key,
      label: label(c.key, c.de),
    })),
    known_topics: knownTopics.slice(0, 80),
  };
}

/** Bücher, die noch eingeordnet werden dürfen (ohne Einordnung, nicht von Hand). */
export const needsClassification = (b: LibraryBook) =>
  b.classified_by !== "hand" && !b.category && !(b.topics ?? []).length;

/** Alle bisher verwendeten Themen, häufigste zuerst. */
export function knownTopics(books: LibraryBook[]): string[] {
  const n = new Map<string, number>();
  for (const b of books)
    for (const t of b.topics ?? []) n.set(t, (n.get(t) ?? 0) + 1);
  return [...n.entries()].sort((a, b) => b[1] - a[1]).map(([t]) => t);
}

/**
 * Ein Buch einordnen lassen und als Vorschlag (`ki`) speichern. Liefert das
 * gespeicherte Buch oder wirft mit der Fehlermeldung des Kerns.
 */
export async function classifyBook(
  book: LibraryBook,
  all: LibraryBook[],
): Promise<LibraryBook> {
  if (book.classified_by === "hand") return book;
  const res = await commands.readingClassify(
    classifyInput(book, knownTopics(all)),
  );
  if (res.status === "error") throw new Error(res.error);
  const saved = await commands.librarySave({
    ...book,
    category: res.data.category,
    topics: res.data.topics,
    classified_by: "ki",
  });
  if (saved.status === "error") throw new Error(saved.error);
  return saved.data;
}

/** Einordnung von Hand setzen (bestätigt oder geändert). */
export async function saveClassification(
  book: LibraryBook,
  category: string | null,
  topics: string[],
): Promise<LibraryBook> {
  const saved = await commands.librarySave({
    ...book,
    category,
    topics,
    classified_by: category || topics.length ? "hand" : null,
  });
  if (saved.status === "error") throw new Error(saved.error);
  return saved.data;
}
