import type {
  BookTalkAnswer,
  BookTalkCommand,
  BookTalkEvent,
  BookTalkRequest,
  StoredTurn,
} from "@/bindings";

/**
 * Was die Gesprächsspalte und die Buch-Seite (`bookSide.ts`) vom Kern brauchen
 * (Tauri-Befehle `book_talk_*` und Ereignis `book-talk-event`). Als Schnittstelle,
 * damit die Prüfungen ohne Tauri laufen; die Umsetzung steht in `api.ts`.
 */
export interface BookTalkPort {
  /** Frage stellen; löst mit der endgültigen Antwort auf, wirft den Fehlercode des Kerns. */
  ask(req: BookTalkRequest): Promise<BookTalkAnswer>;
  /** Laufende Frage abbrechen (`failed { cancelled }` folgt als Ereignis). */
  cancel(requestId: string): Promise<boolean>;
  history(bookId: string): Promise<StoredTurn[]>;
  clear(bookId: string): Promise<void>;
  /** Sprachbefehl erkennen, ohne Modell (Plan D7). */
  command(bookId: string, text: string): Promise<BookTalkCommand | null>;
  /** Hört auf `book-talk-event`; gibt die Abmeldung zurück. */
  subscribe(handler: (event: BookTalkEvent) => void): Promise<() => void>;
}
