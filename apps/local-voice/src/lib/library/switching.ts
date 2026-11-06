/**
 * Sprachwechsel mit Positionsübernahme (lva#344, Spezifikation §6 und §8).
 *
 * `planSwitch` rechnet die Stelle der gerade benutzten Fassung auf die Zielfassung
 * um (`languageSwitch.ts`), `applySwitch` schreibt Stelle und Auswahl,
 * `undoSwitch` stellt Fassung UND Stelle wieder her. Hören und Lesen wechseln
 * jeweils im selben Medium: Hörbuch → Hörbuch (Teile als Kapitel), Text → Text
 * (Überschriften als Kapitel). Eine Zuordnung gilt nur, wenn ihre Kapitelzahlen
 * zu diesem Medium passen; sonst bleibt es ehrlich bei „ungefähr“.
 */
import { commands, type LibraryBook, type ReadingPosition } from "@/bindings";
import {
  chapterOf,
  mapPosition,
  type Outline,
  type SwitchLevel,
} from "./languageSwitch";
import {
  alignmentFor,
  audioChapters,
  audioKeyOf,
  audioPathsOf,
  deviceId,
  editionById,
  groupsFit,
  textChapters,
  textKeyOf,
  type ChapterList,
} from "./editions";
import {
  getLocal,
  loadProgress,
  removeProgress,
  saveProgress,
  setLocal,
  type ProgressRecord,
} from "./progress";
import { probeDuration } from "./probe";
import { assetUrl } from "./books";
import { anchorAt } from "../reader/resumePoint";

export type SwitchMedium = "audio" | "text";

export interface SwitchPlan {
  from: string;
  to: string;
  medium: SwitchMedium;
  level: SwitchLevel;
  /** Stelle in der Zielfassung. */
  position: ReadingPosition;
  /** Kapitel (0-basiert) der Gruppe in der Zielfassung. */
  chapters: number[];
  chapterTotal: number;
  /** Kapitel (0-basiert) der Quellstelle, sofern die Fassung Kapitel kennt. */
  fromChapter: number | null;
  /** Anteil am ganzen Buch (0…1). */
  fraction: number;
  /** Ein Paar ist zugeordnet, aber diese Gruppe (oder das Paar) ist noch offen. */
  hasAlignment: boolean;
  /** Stelle im Zielkapitel, nur beim Hörbuch und kapitelgenau (ms). */
  within: { at: number; length: number } | null;
}

/** Text einer Vorlesen-Seite (Quelltext, Reiter „Original“); `null`, wenn er fehlt. */
export async function loadPageText(pageId: string): Promise<string | null> {
  const r = await commands.pageStateLoad(pageId);
  if (r.status !== "ok" || !r.data) return null;
  try {
    const state = JSON.parse(r.data) as { text?: unknown };
    return typeof state.text === "string" ? state.text : null;
  } catch {
    return r.data;
  }
}

async function chaptersOf(
  book: LibraryBook,
  id: string,
  medium: SwitchMedium,
): Promise<ChapterList | null> {
  const e = editionById(book, id);
  if (!e) return null;
  if (medium === "text") {
    if (!e.text) return null;
    const text = await loadPageText(e.text.page_id);
    return text === null ? null : textChapters(text);
  }
  if (!e.audio) return null;
  const known = audioChapters(e);
  if (known.starts.length || !known.titles.length) return known;
  // Fehlende Dauern aus den Dateien lesen.
  const paths = audioPathsOf(book, e);
  const found = await Promise.all(
    known.titles.map(async (_, i) => {
      const url = assetUrl(paths[i]);
      return url ? probeDuration(url) : null;
    }),
  );
  return audioChapters(e, found);
}

const outlineOf = (c: ChapterList, minLength = 0): Outline => ({
  starts: c.starts,
  length: Math.max(c.length, minLength),
});

/**
 * Zielstelle für den Wechsel von `fromId` nach `toId`. `offset`/`length` sind
 * Stelle und Gesamtlänge der Quellfassung (ms beim Hörbuch, Zeichen beim Text).
 */
export async function planSwitch(
  book: LibraryBook,
  fromId: string,
  toId: string,
  medium: SwitchMedium,
  offset: number,
  length = 0,
): Promise<SwitchPlan | null> {
  const to = editionById(book, toId);
  if (!to) return null;
  const [src, dst] = await Promise.all([
    chaptersOf(book, fromId, medium),
    chaptersOf(book, toId, medium),
  ]);
  if (!dst) return null;
  const from = outlineOf(
    src ?? { titles: [], starts: [], length: 0, unit: dst.unit },
    length,
  );
  const target = outlineOf(dst);
  const { alignment, groups } = alignmentFor(book, fromId, toId);
  // Nur eine Zuordnung, die zu den Kapitelzahlen dieses Mediums passt.
  const usable =
    src && groupsFit(groups, src.titles.length, dst.titles.length)
      ? groups
      : null;
  const mapped = mapPosition(from, target, offset, usable);
  const key = medium === "audio" ? audioKeyOf(book, to) : (textKeyOf(to) ?? "");
  const first = mapped.chapters[0];
  const last = mapped.chapters[mapped.chapters.length - 1];
  let within: SwitchPlan["within"] = null;
  if (medium === "audio" && mapped.level === "chapter" && first != null) {
    const start = target.starts[first] ?? 0;
    const end = target.starts[last + 1] ?? target.length;
    within = { at: Math.max(0, mapped.offset - start), length: end - start };
  }
  return {
    from: fromId,
    to: toId,
    medium,
    level: mapped.level,
    position: {
      key,
      unit: medium === "audio" ? "ms" : "chars",
      offset: mapped.offset,
    },
    chapters: mapped.chapters,
    chapterTotal: dst.titles.length,
    fromChapter: from.starts.length ? chapterOf(from, offset) : null,
    fraction: mapped.fraction,
    hasAlignment: !!alignment,
    within,
  };
}

/** Stand, den ein Wechsel überschrieben hat: für „Rückgängig“. */
export interface SwitchUndo {
  bookId: string;
  fromEdition: string;
  /** Stelle der Quellfassung zum Zeitpunkt des Wechsels. */
  fromPosition: ReadingPosition | null;
  /** Fortschrittsdatensatz der Zielfassung vor dem Wechsel. */
  targetKey: string;
  targetRecord: ProgressRecord | null;
  /** Weiterlese-Stelle des Readers (Text) vor dem Wechsel. */
  resumeKey: string | null;
  resumeBefore: string | null;
  /** Spielte das Hörbuch beim Wechsel? Rückgängig startet genauso. */
  wasPlaying: boolean;
}

/** Zielstelle in den Fortschritt der Zielfassung schreiben (ohne „gehört“ zu zählen). */
function writeTarget(
  book: LibraryBook,
  plan: SwitchPlan,
  textLength: number,
  total: number,
): {
  key: string;
  before: ProgressRecord | null;
  resumeKey: string | null;
  resumeBefore: string | null;
} {
  const key = plan.position.key;
  const before = loadProgress(key);
  const base: ProgressRecord = before ?? {
    offset: 0,
    length: 0,
    heard: [],
    read: [],
    updated: 0,
  };
  const unit = plan.position.unit === "ms" ? "ms" : "chars";
  // Eine als fertig markierte Zielfassung startet an der Zielstelle, nicht bei 0.
  const { finished: _finished, ...open } = base;
  void _finished;
  // Erst entfernen: Schreiben führt sonst mit dem Stand („fertig“, Stelle) zusammen.
  removeProgress(key);
  saveProgress(key, {
    ...open,
    offset: plan.position.offset,
    length: unit === "ms" ? total || base.length : textLength || base.length,
    unit,
    updated: Date.now(),
  });
  if (plan.medium !== "text")
    return { key, before, resumeKey: null, resumeBefore: null };
  const page = editionById(book, plan.to)?.text?.page_id;
  if (!page) return { key, before, resumeKey: null, resumeBefore: null };
  const resumeKey = `tts.resume.${page}:original`;
  return { key, before, resumeKey, resumeBefore: getLocal(resumeKey) };
}

/**
 * Wechsel ausführen: Stelle der Zielfassung schreiben, dann die Auswahl
 * (`reading`) setzen. Scheitert das Setzen, bleibt die Stelle der Zielfassung
 * zurückgesetzt und der Fehler geht an den Aufrufer.
 */
export async function applySwitch(
  book: LibraryBook,
  plan: SwitchPlan,
  current: { offset: number; unit: "ms" | "chars" },
  options: { wasPlaying: boolean; text?: string | null; total?: number },
): Promise<{ book: LibraryBook; undo: SwitchUndo }> {
  const from = editionById(book, plan.from);
  const fromKey =
    plan.medium === "audio"
      ? from
        ? audioKeyOf(book, from)
        : ""
      : from
        ? (textKeyOf(from) ?? "")
        : "";
  const written = writeTarget(
    book,
    plan,
    options.text?.length ?? 0,
    options.total ?? 0,
  );
  if (written.resumeKey && options.text) {
    const at = plan.position.offset;
    setLocal(
      written.resumeKey,
      JSON.stringify({ offset: at, anchor: anchorAt(options.text, at) }),
    );
  }
  const undo: SwitchUndo = {
    bookId: book.id,
    fromEdition: plan.from,
    fromPosition: fromKey
      ? { key: fromKey, unit: current.unit, offset: current.offset }
      : null,
    targetKey: written.key,
    targetRecord: written.before,
    resumeKey: written.resumeKey,
    resumeBefore: written.resumeBefore,
    wasPlaying: options.wasPlaying,
  };
  const result = await commands.librarySetReading(book.id, {
    edition: plan.to,
    updated: Date.now(),
    device: deviceId(),
    position: plan.position,
  });
  if (result.status !== "ok") {
    restoreTarget(undo);
    throw new Error(result.error);
  }
  return { book: result.data, undo };
}

function restoreTarget(undo: SwitchUndo): void {
  removeProgress(undo.targetKey);
  if (undo.targetRecord) saveProgress(undo.targetKey, undo.targetRecord);
  if (undo.resumeKey) setLocal(undo.resumeKey, undo.resumeBefore);
}

const replaceProgress = (key: string, rec: ProgressRecord | null): void => {
  removeProgress(key);
  if (rec) saveProgress(key, rec);
};

const hasRanges = (rec: ProgressRecord | null): rec is ProgressRecord =>
  !!rec && (rec.heard.length > 0 || rec.read.length > 0);

/**
 * Rückgängig: Auswahl zurück auf die Quellfassung samt Stelle. Was in der
 * Zielfassung seit dem Wechsel gehört wurde, bleibt (Stand direkt vor
 * Rückgängig); ohne Hörbereiche gilt der Stand vor dem Wechsel. Die Quellstelle
 * steht vor dem Setzen der Auswahl im Fortschritt. Scheitert das Setzen,
 * ist alles wie unmittelbar vor dem Aufruf.
 */
export async function undoSwitch(undo: SwitchUndo): Promise<LibraryBook> {
  const target = loadProgress(undo.targetKey);
  const resume = undo.resumeKey ? getLocal(undo.resumeKey) : null;
  const src = undo.fromPosition;
  const source = src ? loadProgress(src.key) : null;
  const rollback = () => {
    replaceProgress(undo.targetKey, target);
    if (undo.resumeKey) setLocal(undo.resumeKey, resume);
    if (src) replaceProgress(src.key, source);
  };
  try {
    if (hasRanges(target)) {
      const kept: ProgressRecord = { ...target };
      if (undo.targetRecord?.finished) kept.finished = true;
      else delete kept.finished;
      replaceProgress(undo.targetKey, kept);
    } else restoreTarget(undo);
    if (src && source)
      replaceProgress(src.key, {
        ...source,
        offset: src.offset,
        updated: Date.now(),
      });
    const result = await commands.librarySetReading(undo.bookId, {
      edition: undo.fromEdition,
      updated: Date.now(),
      device: deviceId(),
      position: undo.fromPosition,
    });
    if (result.status !== "ok") throw new Error(result.error);
    return result.data;
  } catch (e) {
    rollback();
    throw e;
  }
}
