import { anchorAt, resolveAnchor } from "./resumePoint";
import { getLocal, setLocal } from "../library/progress";

/**
 * Lesezeichen je Seite und Reiter (lva#148). Wie beim Weiterlesen hält ein
 * Textanker die Stelle fest, wenn sich der Text davor ändert; ein Lesezeichen,
 * dessen Text verschwunden ist, wird nicht mehr angezeigt.
 */
/** Unter `lva.ui.` – und dauerhaft im Kern gespiegelt (lva#152). */
const PREFIX = "tts.bookmarks.";
const LABEL = 48;

interface StoredBookmark {
  id: string;
  offset: number;
  anchor: string;
  created: number;
}

export interface Bookmark {
  id: string;
  /** Aktueller Offset im Text. */
  offset: number;
  /** Lesbarer Anfang der Stelle, ohne Tags und Markdown. */
  label: string;
}

const read = (key: string): StoredBookmark[] => {
  try {
    const raw = getLocal(PREFIX + key);
    if (!raw) return [];
    const list = JSON.parse(raw) as unknown;
    return Array.isArray(list)
      ? list.filter(
          (b): b is StoredBookmark =>
            typeof b?.id === "string" &&
            typeof b?.offset === "number" &&
            typeof b?.anchor === "string",
        )
      : [];
  } catch {
    return [];
  }
};

const write = (key: string, list: StoredBookmark[]) => {
  setLocal(PREFIX + key, list.length ? JSON.stringify(list) : null);
};

/** Anfang der Stelle als Klartext: ohne Sprecher, Tags und Markdown. */
export const bookmarkLabel = (text: string, offset: number): string => {
  const line = text
    .slice(offset, offset + 240)
    .replace(/<[^<>\n]{1,60}>|\[[^\]\n]*\]/g, " ")
    .replace(/^[#>\-\s]+/gm, "")
    .replace(/[*_]+/g, "")
    .replace(/\s+/g, " ")
    .trim();
  return line.length > LABEL ? `${line.slice(0, LABEL).trimEnd()}…` : line;
};

export function loadBookmarks(key: string, text: string): Bookmark[] {
  return read(key)
    .map((b) => {
      const offset = resolveAnchor(text, b.offset, b.anchor);
      return offset === null
        ? null
        : { id: b.id, offset, label: bookmarkLabel(text, offset) };
    })
    .filter((b): b is Bookmark => b !== null)
    .sort((a, b) => a.offset - b.offset);
}

/** Lesezeichen setzen; an derselben Stelle gibt es keins doppelt. */
export function addBookmark(key: string, text: string, offset: number): void {
  const at = Math.max(0, Math.min(offset, Math.max(0, text.length - 1)));
  const list = read(key);
  if (list.some((b) => resolveAnchor(text, b.offset, b.anchor) === at)) return;
  list.push({
    id: `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`,
    offset: at,
    anchor: anchorAt(text, at),
    created: Date.now(),
  });
  write(key, list);
}

export function removeBookmark(key: string, id: string): void {
  write(
    key,
    read(key).filter((b) => b.id !== id),
  );
}
