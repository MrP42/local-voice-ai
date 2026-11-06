/**
 * Sprachfassungen in der Oberfläche (lva#344, PR 2). Reine Hilfen über dem
 * Datenmodell aus PR 1: welche Fassung ist Hauptfassung, welche zuletzt
 * aktiv, unter welchem Schlüssel liegt ihr Fortschritt, wie sieht das Buch
 * aus Sicht einer Fassung aus. Der Fortschritt einer Fassung liegt immer unter
 * `edition.audio_key` bzw. `text:<seite>:original`, nie unter der Buchkennung.
 */
import type {
  Alignment,
  Edition,
  LibraryBook,
  ReadingPosition,
} from "@/bindings";
import { TTS_TARGET_LANGS } from "@/lib/constants/languages";
import { tracksOf } from "./names";
import {
  audioProgressKey,
  loadProgress,
  textProgressKey,
  type ProgressRecord,
} from "./progress";
import type { AlignGroup } from "./languageSwitch";

/** Kennung der Fassung, die ein Schema-1-Buch ohne `editions` darstellt (wie `migrated_id`). */
export const migratedId = (bookId: string) => `ed_${bookId}`;

const ALIASES: Record<string, string> = {
  deu: "de",
  ger: "de",
  deutsch: "de",
  german: "de",
  eng: "en",
  englisch: "en",
  english: "en",
  fra: "fr",
  fre: "fr",
  french: "fr",
  français: "fr",
  francais: "fr",
  französisch: "fr",
  spa: "es",
  spanish: "es",
  español: "es",
  spanisch: "es",
  ita: "it",
  italian: "it",
  italiano: "it",
  italienisch: "it",
};

/** Sprachcode aus freier Angabe („Deutsch“, „de-DE“, „ger“) oder leer. */
export function langCode(raw: string | null | undefined): string {
  const head = (raw ?? "").trim().toLowerCase().split(/[-_]/)[0] ?? "";
  if (ALIASES[head]) return ALIASES[head];
  if (/^[a-z]{2}$/.test(head)) return head;
  return (
    TTS_TARGET_LANGS.find((l) => l.label.toLowerCase() === head)?.code ?? ""
  );
}

/** Eigenname der Sprache („Deutsch“, „English“); unbekannt: das Kürzel. */
export function langName(code: string | null | undefined): string {
  const c = langCode(code);
  if (!c) return "";
  return TTS_TARGET_LANGS.find((l) => l.code === c)?.label ?? c.toUpperCase();
}

/** Kürzel zur Anzeige („DE“); unbekannte Sprache: Gedankenstrich. */
export const langShort = (code: string | null | undefined): string =>
  langCode(code).toUpperCase() || "–";

/**
 * Alle Fassungen des Buchs. Ein Buch ohne `editions` (Schema 1, noch nicht
 * migriert, oder ein Stand ohne Kern) erscheint als eine Fassung, so wie sie
 * der Kern beim Lesen anlegen würde.
 */
export function editionsOf(book: LibraryBook): Edition[] {
  if (book.editions?.length) return book.editions;
  if (!book.text && !book.audio) return [];
  return [
    {
      id: migratedId(book.id),
      lang: langCode(book.language),
      title: book.title,
      subtitle: book.subtitle,
      isbn: book.isbn,
      work_key: book.work_key ?? null,
      text: book.text,
      audio: book.audio,
      audio_key: audioProgressKey(book.id),
      origin: "migration",
      added_ms: book.added_ms,
      audio_paths:
        book.audio_paths ?? (book.audio_path ? [book.audio_path] : []),
    },
  ];
}

/** Hauptfassung: das Register, falls es auf eine lebende Fassung zeigt, sonst die älteste. */
export function primaryEdition(book: LibraryBook): Edition | null {
  const list = editionsOf(book);
  const pinned = book.primary?.edition;
  return (
    list.find((e) => e.id === pinned) ??
    [...list].sort(
      (a, b) =>
        (a.added_ms ?? 0) - (b.added_ms ?? 0) || a.id.localeCompare(b.id),
    )[0] ??
    null
  );
}

/** Zuletzt aktive Fassung (`reading`), sonst die Hauptfassung. */
export function readingEdition(book: LibraryBook): Edition | null {
  const list = editionsOf(book);
  return (
    list.find((e) => e.id === book.reading?.edition) ?? primaryEdition(book)
  );
}

export const editionById = (
  book: LibraryBook,
  id: string | null | undefined,
): Edition | null => editionsOf(book).find((e) => e.id === id) ?? null;

/** Mehr als eine Sprachfassung? */
export const hasLanguages = (book: LibraryBook): boolean =>
  editionsOf(book).length > 1;

/** Fortschrittsschlüssel des Hörbuchs einer Fassung. */
export const audioKeyOf = (book: LibraryBook, e: Edition): string =>
  e.audio_key || audioProgressKey(book.id);

/** Fortschrittsschlüssel des Hörbuchs der Hauptfassung (Arbeitskopie des Buchs). */
export const primaryAudioKey = (book: LibraryBook): string => {
  const p = primaryEdition(book);
  return p ? audioKeyOf(book, p) : audioProgressKey(book.id);
};

/** Fortschrittsschlüssel der Hörbücher aller Fassungen (ohne Fassungen: der des Buchs). */
export const audioKeysOf = (book: LibraryBook): string[] => [
  ...new Set(editionsOf(book).map((e) => audioKeyOf(book, e))),
];

/** Fortschrittsschlüssel des Textes einer Fassung (Reiter „Original“). */
export const textKeyOf = (e: Edition): string | null =>
  e.text ? textProgressKey(e.text.page_id) : null;

/** Pfade der Hörbuchteile einer Fassung; die Hauptfassung nutzt die Arbeitskopie. */
export function audioPathsOf(book: LibraryBook, e: Edition): string[] {
  const own = (e.audio_paths ?? []).filter(Boolean);
  if (own.length) return e.audio_paths ?? [];
  if (e.id === primaryEdition(book)?.id) {
    const parts = book.audio_paths?.length
      ? book.audio_paths
      : book.audio_path
        ? [book.audio_path]
        : [];
    return parts;
  }
  return [];
}

/** Hat die Fassung auf diesem Gerät ein abspielbares Hörbuch? */
export const hasAudio = (book: LibraryBook, e: Edition): boolean =>
  !!e.audio && audioPathsOf(book, e).some(Boolean);

/**
 * Das Buch aus Sicht einer Fassung: Titel, Sprache, Text und Hörbuch der
 * Fassung statt der Arbeitskopie. Die Kennung bleibt die des Buchs.
 */
export function editionBook(book: LibraryBook, e: Edition): LibraryBook {
  const paths = audioPathsOf(book, e);
  const primary = e.id === primaryEdition(book)?.id;
  if (primary) return book;
  return {
    ...book,
    title: e.title.trim() ? e.title : book.title,
    subtitle: e.subtitle ?? null,
    language: e.lang || book.language,
    isbn: e.isbn ?? null,
    text: e.text ?? null,
    audio: e.audio ?? null,
    audio_paths: paths,
    audio_path: paths.find(Boolean) ?? null,
  };
}

export interface EditionProgress {
  text: ProgressRecord | null;
  audio: ProgressRecord | null;
  /** Zuletzt gehört oder gelesen (Unix-ms), 0 = nie. */
  lastUsed: number;
}

export function progressOfEdition(
  book: LibraryBook,
  e: Edition,
): EditionProgress {
  const tk = textKeyOf(e);
  const text = tk ? loadProgress(tk) : null;
  const audio = e.audio ? loadProgress(audioKeyOf(book, e)) : null;
  return {
    text,
    audio,
    lastUsed: Math.max(text?.updated ?? 0, audio?.updated ?? 0),
  };
}

/** Sprachkürzel der Fassungen, die zuletzt aktive zuerst. */
export function languageBadges(
  book: LibraryBook,
): { id: string; code: string; label: string; current: boolean }[] {
  const current = readingEdition(book)?.id;
  return editionsOf(book).map((e) => ({
    id: e.id,
    code: langShort(e.lang),
    label: langName(e.lang) || langShort(e.lang),
    current: e.id === current,
  }));
}

/** Anzahl der Kapitel einer Fassung für die Zuordnung. */
export interface ChapterList {
  /** Kapitelüberschriften bzw. Teiltitel. */
  titles: string[];
  /** Kapitelanfänge in der Einheit der Fassung; leer, wenn unbekannt. */
  starts: number[];
  length: number;
  unit: "ms" | "chars";
}

const HEADING = /^(#{1,2})[ \t]+(.+?)[ \t]*#*[ \t]*$/;

/** Kapitel aus den Überschriften (# und ##) des Seitentexts. */
export function textChapters(text: string): ChapterList {
  const titles: string[] = [];
  const starts: number[] = [];
  let at = 0;
  let first = true;
  const lines = text.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const m = HEADING.exec(line);
    if (m) {
      const next = lines.slice(i + 1).find((l) => l.trim()) ?? "";
      // Titelseite des EPUB-Imports: „# Titel“ und „von Autor“ ist kein Kapitel.
      const titlePage =
        first && m[1] === "#" && /^(von|by)\s+/i.test(next.trim());
      first = false;
      if (!titlePage) {
        titles.push(m[2].replace(/[*_]/g, "").trim());
        starts.push(at + line.indexOf(m[2]));
      }
    }
    at += line.length + 1;
  }
  return { titles, starts, length: text.length, unit: "chars" };
}

/** Kapitel aus den Teilen des Hörbuchs; fehlt eine Dauer, bleiben `starts` leer. */
export function audioChapters(
  e: Edition,
  durations?: (number | null)[],
): ChapterList {
  const tracks = tracksOf(e.audio);
  const ms = tracks.map((t, i) => durations?.[i] ?? t.duration_ms ?? null);
  const titles = tracks.map((t, i) => t.title?.trim() || `Teil ${i + 1}`);
  if (ms.some((d) => d == null || !(d > 0)))
    return { titles, starts: [], length: 0, unit: "ms" };
  const starts: number[] = [];
  let sum = 0;
  for (const d of ms as number[]) {
    starts.push(sum);
    sum += d;
  }
  return { titles, starts, length: sum, unit: "ms" };
}

/** Zuordnung eines Fassungspaars, in Richtung `from` → `to` gedreht. */
export function alignmentFor(
  book: LibraryBook,
  from: string,
  to: string,
): { alignment: Alignment | null; groups: AlignGroup[] | null } {
  const found = (book.alignments ?? [])
    .filter((a) => (a.a === from && a.b === to) || (a.a === to && a.b === from))
    .sort((x, y) => y.updated - x.updated)[0];
  if (!found) return { alignment: null, groups: null };
  const straight = found.a === from;
  return {
    alignment: found,
    groups: found.groups.map((g) =>
      straight ? g : { a: g.b, b: g.a, confirmed: g.confirmed },
    ),
  };
}

/** Gehört die Gruppenliste zu genau diesen Kapitelzahlen? */
export function groupsFit(
  groups: AlignGroup[] | null,
  countA: number,
  countB: number,
): groups is AlignGroup[] {
  if (!groups?.length) return false;
  const a = groups.reduce((n, g) => n + g.a.length, 0);
  const b = groups.reduce((n, g) => n + g.b.length, 0);
  return a === countA && b === countB;
}

/** Aus einer Gruppenliste in Richtung `a → b` die gespeicherte Form für das Paar (`a`, `b`). */
export function newAlignment(
  a: string,
  b: string,
  groups: AlignGroup[],
  device: string,
): Alignment {
  return { a, b, updated: Date.now(), device, groups };
}

const DEVICE_KEY = "lva.library.device";

/** Kennung dieses Geräts für die Register (stabil, nur lokal gespeichert). */
export function deviceId(): string {
  try {
    const stored = window.localStorage.getItem(DEVICE_KEY);
    if (stored) return stored;
    const fresh = `pc_${Math.random().toString(36).slice(2, 10)}`;
    window.localStorage.setItem(DEVICE_KEY, fresh);
    return fresh;
  } catch {
    return "pc";
  }
}

/** Welche Zielstelle gehört zu welchem Schlüssel? Für Anzeige und Rückgängig. */
export interface PositionSnapshot {
  key: string;
  record: ProgressRecord | null;
}

export const positionKeyUnit = (p: ReadingPosition): "ms" | "chars" =>
  p.unit === "ms" ? "ms" : "chars";
