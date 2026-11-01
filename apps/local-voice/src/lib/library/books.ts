import { convertFileSrc } from "@tauri-apps/api/core";
import type { LibraryBook } from "@/bindings";
import {
  audioProgressKey,
  loadProgress,
  progressFraction,
  textProgressKey,
  type ProgressRecord,
} from "./progress";

/** Status einer Fassung: vorhanden, entsteht gerade, fehlt. */
export type VersionState = "ok" | "busy" | "none";

export interface BookView {
  book: LibraryBook;
  /** Fortschritt des Buchs: die zuletzt benutzte Fassung. */
  progress: ProgressRecord | null;
  fraction: number;
  /** Zuletzt gehört oder gelesen (Unix-ms), 0 = nie. */
  lastUsed: number;
  text: VersionState;
  audio: VersionState;
  /** Fassung, mit der „Weiterhören“ weitermacht. */
  resumeWith: "text" | "audio" | null;
}

export type Filter =
  | "all"
  | "notStarted"
  | "inProgress"
  | "finished"
  | "noAudio"
  | "working";
export type Sort = "recent" | "added" | "title" | "author";

/**
 * Sicht auf ein Buch für die Bibliothek. `pageAudio` sagt, ob zur
 * Textfassung schon vertonte Dateien vorliegen (Seitenordner).
 */
export function viewOf(book: LibraryBook, pageAudio: boolean): BookView {
  const textRec = book.text
    ? loadProgress(textProgressKey(book.text.page_id))
    : null;
  const audioRec = book.audio_path
    ? loadProgress(audioProgressKey(book.id))
    : null;
  const progress =
    (textRec?.updated ?? 0) >= (audioRec?.updated ?? 0) ? textRec : audioRec;
  const resumeWith: BookView["resumeWith"] =
    progress === audioRec && audioRec
      ? "audio"
      : book.text
        ? "text"
        : book.audio_path
          ? "audio"
          : null;
  const fraction = book.finished ? 1 : progressFraction(progress);
  return {
    book,
    progress,
    fraction,
    lastUsed: Math.max(textRec?.updated ?? 0, audioRec?.updated ?? 0),
    text: book.text ? "ok" : book.audio?.transcript_job ? "busy" : "none",
    audio: book.audio_path || pageAudio ? "ok" : "none",
    resumeWith,
  };
}

export const isFinished = (v: BookView) => v.book.finished || v.fraction >= 1;
export const isStarted = (v: BookView) => !isFinished(v) && v.fraction > 0;

export function matchesFilter(v: BookView, filter: Filter): boolean {
  switch (filter) {
    case "notStarted":
      return !isFinished(v) && v.fraction === 0;
    case "inProgress":
      return isStarted(v);
    case "finished":
      return isFinished(v);
    case "noAudio":
      return v.audio === "none";
    case "working":
      return v.text === "busy" || v.audio === "busy";
    default:
      return true;
  }
}

const fold = (s: string) =>
  s
    .toLocaleLowerCase("de")
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "");

export function matchesSearch(v: BookView, query: string): boolean {
  const q = fold(query.trim());
  if (!q) return true;
  const b = v.book;
  return [b.title, b.subtitle ?? "", ...b.authors, b.audio?.narrator ?? ""]
    .map(fold)
    .some((s) => s.includes(q));
}

/** Nachname zum Sortieren: „Theodor Fontane“ → „fontane“. */
const surname = (authors: string[]) => {
  const a = authors[0] ?? "";
  const name = a.includes(",") ? a.split(",")[0] : a.split(/\s+/).pop();
  return fold(name ?? "");
};

export function sortViews(views: BookView[], sort: Sort): BookView[] {
  const byTitle = (a: BookView, b: BookView) =>
    fold(a.book.title).localeCompare(fold(b.book.title), "de");
  const sorted = [...views];
  switch (sort) {
    case "title":
      return sorted.sort(byTitle);
    case "author":
      return sorted.sort(
        (a, b) =>
          surname(a.book.authors).localeCompare(surname(b.book.authors), "de") ||
          byTitle(a, b),
      );
    case "added":
      return sorted.sort((a, b) => b.book.added_ms - a.book.added_ms);
    default:
      return sorted.sort(
        (a, b) =>
          b.lastUsed - a.lastUsed || b.book.added_ms - a.book.added_ms,
      );
  }
}

/** Dateipfad mit `?v=` aus dem Kern in eine Bild-URL für die WebView. */
export function assetUrl(path: string | null | undefined): string | null {
  if (!path) return null;
  const at = path.lastIndexOf("?v=");
  const file = at >= 0 ? path.slice(0, at) : path;
  const version = at >= 0 ? path.slice(at + 3) : "";
  const url = convertFileSrc(file);
  return version ? `${url}?v=${version}` : url;
}

const TEXT_EXT = ["epub", "pdf", "docx", "txt", "md"];
const AUDIO_EXT = ["mp3", "m4b", "m4a", "aac", "wav", "flac", "ogg", "opus", "wma"];

export const extensionOf = (path: string) =>
  path.split(/[\\/]/).pop()?.split(".").pop()?.toLowerCase() ?? "";

export function kindOfFile(path: string): "text" | "audio" | null {
  const ext = extensionOf(path);
  if (TEXT_EXT.includes(ext)) return "text";
  if (AUDIO_EXT.includes(ext)) return "audio";
  return null;
}

export const LIBRARY_FILE_EXTENSIONS = [...TEXT_EXT, ...AUDIO_EXT];

/**
 * Titel und Autor aus einem Dateinamen raten: „Theodor Fontane - Effi
 * Briest.m4b“, „Effi_Briest.epub“. Nur ein Vorschlag, der Dialog zeigt ihn an.
 */
export function guessFromFileName(path: string): {
  title: string;
  authors: string[];
} {
  const base = (path.split(/[\\/]/).pop() ?? "")
    .replace(/\.[^.]+$/, "")
    .replace(/[_]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  const parts = base.split(/\s+[-–]\s+/);
  if (parts.length === 2) {
    const [a, b] = parts;
    // Der kürzere Teil mit zwei bis drei Wörtern ist eher der Autor.
    const looksLikeName = (s: string) => /^\S+(\s+\S+){1,2}$/.test(s);
    if (looksLikeName(a) && !looksLikeName(b))
      return { title: b, authors: [a] };
    if (looksLikeName(b) && !looksLikeName(a))
      return { title: a, authors: [b] };
    return { title: b, authors: [a] };
  }
  return { title: base, authors: [] };
}

/**
 * Titel und Autor aus der Titelei, die der EPUB-Import voranstellt
 * (`# Titel` und darunter `von Autor`).
 */
export function guessFromText(text: string): {
  title: string;
  authors: string[];
} | null {
  const lines = text
    .split(/\n+/)
    .map((l) => l.trim())
    .filter(Boolean)
    .slice(0, 4);
  const head = lines[0]?.match(/^#\s+(.+)$/)?.[1];
  if (!head) return null;
  const by = lines[1]?.match(/^(?:von|by)\s+(.{2,120})$/i)?.[1];
  return {
    title: head.replace(/[*_]/g, "").trim(),
    authors: by
      ? by
          .split(/\s*(?:,|;|&|\bund\b|\band\b)\s*/)
          .map((a) => a.trim())
          .filter(Boolean)
      : [],
  };
}

export interface TranscriptSegment {
  text: string;
  start_ms: number;
  end_ms: number;
}

/**
 * Transkript zu Lesetext: Segmente zu Absätzen zusammenfassen, neue Absätze
 * bei einer Pause über 1,5 s oder nach etwa 700 Zeichen.
 */
export function transcriptToText(segments: TranscriptSegment[]): string {
  const paragraphs: string[] = [];
  let current = "";
  let lastEnd = -1;
  for (const s of [...segments].sort((a, b) => a.start_ms - b.start_ms)) {
    const text = s.text.replace(/\s+/g, " ").trim();
    if (!text) continue;
    const pause = lastEnd >= 0 && s.start_ms - lastEnd > 1500;
    if (current && (pause || current.length > 700)) {
      paragraphs.push(current);
      current = "";
    }
    current = current ? `${current} ${text}` : text;
    lastEnd = s.end_ms;
  }
  if (current) paragraphs.push(current);
  return paragraphs.join("\n\n");
}

/** „3 h 44 min“ / „23 min“ aus Millisekunden. */
export function formatDuration(ms: number): string {
  const minutes = Math.max(0, Math.round(ms / 60000));
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  if (h === 0) return `${m} min`;
  return m ? `${h} h ${m} min` : `${h} h`;
}

/** Vorlesen öffnen: Seite aktiv setzen, optional sofort lesen. */
export function openInReader(pageId: string, autoplay: boolean): void {
  try {
    window.localStorage.setItem("lva.ui.tts.activePage", pageId);
    if (autoplay) window.sessionStorage.setItem(AUTOPLAY_KEY, pageId);
    else window.sessionStorage.removeItem(AUTOPLAY_KEY);
  } catch {
    /* ohne Speicher öffnet Vorlesen die zuletzt aktive Seite */
  }
  window.dispatchEvent(
    new CustomEvent("lv-navigate", { detail: { section: "tts" } }),
  );
}

const AUTOPLAY_KEY = "lva.library.autoplay";

/** Für Vorlesen: soll diese Seite gleich nach dem Laden vorgelesen werden? */
export function takeLibraryAutoplay(): string | null {
  try {
    const page = window.sessionStorage.getItem(AUTOPLAY_KEY);
    window.sessionStorage.removeItem(AUTOPLAY_KEY);
    return page;
  } catch {
    return null;
  }
}

/**
 * Seitentitel für ein hinzugefügtes Dokument: die Titelzeile (`# Titel`,
 * EPUB- und DOCX-Import), sonst der Dateiname ohne Endung.
 */
export function documentTitle(text: string, path: string): string {
  const firstLine = text.trimStart().split("\n", 1)[0] ?? "";
  const heading = /^#[ \t]+(.+)$/.exec(firstLine)?.[1]?.replace(/[*_]/g, "").trim();
  const title = heading || guessFromFileName(path).title;
  return title.replace(/\s+/g, " ").trim().slice(0, 120);
}
