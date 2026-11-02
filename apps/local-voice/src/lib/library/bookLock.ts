/**
 * Sperre je Buch: Korrektur übernehmen, Text anhängen, Hörbuch ersetzen,
 * Original wiederherstellen und Sicherung löschen laufen nacheinander, nie
 * gleichzeitig. Gilt im ganzen Modul (ein Fenster, eine Kette je Buch).
 */
const tails = new Map<string, Promise<void>>();

/** Wartet, bis das Buch frei ist; die Rückgabe gibt es wieder frei. */
export async function lockBook(bookId: string): Promise<() => void> {
  const previous = tails.get(bookId) ?? Promise.resolve();
  let release: () => void = () => undefined;
  const mine = new Promise<void>((resolve) => {
    release = resolve;
  });
  const tail = previous.then(() => mine);
  tails.set(bookId, tail);
  await previous;
  return () => {
    release();
    if (tails.get(bookId) === tail) tails.delete(bookId);
  };
}

export async function withBookLock<T>(
  bookId: string,
  work: () => Promise<T>,
): Promise<T> {
  const release = await lockBook(bookId);
  try {
    return await work();
  } finally {
    release();
  }
}

type Settler = (bookId: string) => Promise<void>;
const settlers: Settler[] = [];

/** Das Korrekturlesen meldet sich hier an (vermeidet eine Importschleife). */
export function registerSettler(settler: Settler): void {
  settlers.push(settler);
}

/**
 * Laufendes oder wartendes Korrekturlesen dieses Buchs abbrechen und warten,
 * bis es zu Ende ist. Vor Verwerfen, Wiederherstellen, Anhängen und Ersetzen.
 */
export async function settleBook(bookId: string): Promise<void> {
  for (const settle of settlers) await settle(bookId);
}
