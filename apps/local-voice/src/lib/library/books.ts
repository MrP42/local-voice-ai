import { convertFileSrc } from "@tauri-apps/api/core";
import type { AudioVersion, LibraryBook } from "@/bindings";
import { guessFromFileName, tracksOf } from "./names.ts";
import { jobsOfTrack } from "./sections.ts";
import {
  audioProgressKey,
  loadProgress,
  progressFraction,
  textProgressKey,
  type ProgressRecord,
} from "./progress";

/** Aufträge der Teile, die noch auf ihr Transkript warten, in Teilreihenfolge. */
export function pendingJobs(book: LibraryBook): string[] {
  return tracksOf(book.audio).flatMap((t) => jobsOfTrack(t).map((p) => p.job));
}

/** Gesamtdauer in ms, wenn jeder Teil eine Dauer hat, sonst `null`. */
export function totalDuration(audio: AudioVersion | null | undefined): number | null {
  const tracks = tracksOf(audio);
  if (!tracks.length) return null;
  let sum = 0;
  for (const t of tracks) {
    if (t.duration_ms == null || !(t.duration_ms > 0)) return null;
    sum += t.duration_ms;
  }
  return sum;
}

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
  /** Fassung, mit der „Weitermachen“ weitermacht. */
  resumeWith: "text" | "audio" | null;
  /**
   * Woher das Audio kommt: importiertes Hörbuch, per Sprachausgabe erzeugte
   * Vertonung der Textfassung oder gar keins (dann gibt es kein Abspielen).
   */
  audioSource: "audiobook" | "narrated" | null;
  /** Stand der Transkription (Hörbuch → Text). */
  transcription: "done" | "running" | "possible" | "none";
}

export type Filter =
  | "all"
  | "notStarted"
  | "inProgress"
  | "finished"
  | "noAudio"
  | "working"
  | "favorites";
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
    text: book.text ? "ok" : pendingJobs(book).length ? "busy" : "none",
    audio: book.audio_path || pageAudio ? "ok" : "none",
    resumeWith,
    audioSource: book.audio_path ? "audiobook" : pageAudio ? "narrated" : null,
    transcription: pendingJobs(book).length
      ? "running"
      : book.text?.origin === "transcript"
        ? "done"
        : book.audio_path && !book.text
          ? "possible"
          : "none",
  };
}

export const isFinished = (v: BookView) => v.book.finished || v.fraction >= 1;
export const isStarted = (v: BookView) => !isFinished(v) && v.fraction > 0;

export const isFavorite = (v: BookView) => v.book.favorite === true;

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
    case "favorites":
      return isFavorite(v);
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

/**
 * Sortiert nach `sort`. Favoriten stehen davor, untereinander in derselben
 * Ordnung (`favoritesFirst: false` z. B. für „Weitermachen“).
 */
export function sortViews(
  views: BookView[],
  sort: Sort,
  { favoritesFirst = true }: { favoritesFirst?: boolean } = {},
): BookView[] {
  const byTitle = (a: BookView, b: BookView) =>
    fold(a.book.title).localeCompare(fold(b.book.title), "de");
  const order = (a: BookView, b: BookView): number => {
    switch (sort) {
      case "title":
        return byTitle(a, b);
      case "author":
        return (
          surname(a.book.authors).localeCompare(surname(b.book.authors), "de") ||
          byTitle(a, b)
        );
      case "added":
        return b.book.added_ms - a.book.added_ms;
      default:
        return b.lastUsed - a.lastUsed || b.book.added_ms - a.book.added_ms;
    }
  };
  const favorite = (v: BookView) => (favoritesFirst && isFavorite(v) ? 0 : 1);
  return [...views].sort((a, b) => favorite(a) - favorite(b) || order(a, b));
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
/** Audible: durch DRM geschützt, wird nicht importiert und nicht umgangen. */
const DRM_EXT = ["aax", "aa", "aaxc"];

export { guessFromFileName, tracksOf };
export {
  textWithSync,
  transcriptParagraphs,
  transcriptToText,
  type TranscriptParagraph,
  type TranscriptSegment,
} from "./transcript.ts";

export const extensionOf = (path: string) =>
  path.split(/[\\/]/).pop()?.split(".").pop()?.toLowerCase() ?? "";

export function kindOfFile(path: string): "text" | "audio" | "drm" | null {
  const ext = extensionOf(path);
  if (TEXT_EXT.includes(ext)) return "text";
  if (AUDIO_EXT.includes(ext)) return "audio";
  if (DRM_EXT.includes(ext)) return "drm";
  return null;
}

/** Dateiname ohne Ordner, für Meldungen. */
export const fileNameOf = (path: string) => path.split(/[\\/]/).pop() ?? path;

export const LIBRARY_FILE_EXTENSIONS = [...TEXT_EXT, ...AUDIO_EXT];

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

const KIDOC_KEY = "lva.library.kidoc";

/** Auftrag aus der Bibliothek an Vorlesen: KI-Dokument öffnen oder erstellen. */
export type LibraryKiDocRequest =
  | { page: string; doc: string; play: boolean }
  | { page: string; create: "kurzbuch" };

/**
 * Kurzbuch aus der Bibliothek (lva#205): Vorlesen öffnen und dort das
 * Dokument zeigen, vorlesen oder erstellen.
 */
export function openKiDocInReader(request: LibraryKiDocRequest): void {
  try {
    window.sessionStorage.setItem(KIDOC_KEY, JSON.stringify(request));
  } catch {
    /* ohne Speicher öffnet Vorlesen nur die Seite */
  }
  openInReader(request.page, false);
}

/** Für Vorlesen: wartender Auftrag aus der Bibliothek (einmalig). */
export function takeLibraryKiDoc(): LibraryKiDocRequest | null {
  try {
    const raw = window.sessionStorage.getItem(KIDOC_KEY);
    window.sessionStorage.removeItem(KIDOC_KEY);
    const v = raw ? (JSON.parse(raw) as LibraryKiDocRequest) : null;
    return v && typeof v.page === "string" ? v : null;
  } catch {
    return null;
  }
}
