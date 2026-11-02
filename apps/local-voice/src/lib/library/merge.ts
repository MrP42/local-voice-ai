import { commands, type LibraryBook } from "@/bindings";
import type { SplitGroup } from "./audioGroups";
import { assetUrl, pendingJobs, totalDuration, tracksOf } from "./books";
import {
  audioProgressKey,
  flushProgress,
  loadProgress,
  recordPosition,
  removeProgress,
  textProgressKey,
} from "./progress";
import { mergedPosition, type MergeEntry } from "./position";
import { probeDuration } from "./probe";
import { findWork, ImportError } from "./importFile";

/**
 * Zusammenführen (lva#204): einzeln importierte Teile eines Hörbuchs werden
 * ein Buch mit Teileliste. Das Ziel ist ein vorhandenes Werk mit passendem
 * Titel und Autor, sonst das erste Buch der Folge.
 */

export interface MergePlan {
  /** Buch, das alle Teile bekommt. */
  target: LibraryBook;
  /** Das Ziel ist ein schon vorhandenes Werk (nicht Teil der Gruppe). */
  existing: boolean;
  /** Ein vorhandenes Hörbuch des Ziels wird dabei ersetzt. */
  replacesAudio: boolean;
  /** Quellbücher in Hörreihenfolge; sie verschwinden. */
  sources: LibraryBook[];
  /** Titel des Hörbuchs nach dem Zusammenführen. */
  title: string;
}

export async function planMerge(group: SplitGroup): Promise<MergePlan> {
  const ids = new Set(group.books.map((b) => b.id));
  const work = await findWork(group.title, group.authors);
  const existing = work && !ids.has(work.id) ? work : null;
  if (existing) {
    return {
      target: existing,
      existing: true,
      replacesAudio: !!existing.audio,
      sources: group.books,
      title: existing.title,
    };
  }
  const [first, ...rest] = group.books;
  return {
    target: first,
    existing: false,
    replacesAudio: false,
    sources: rest,
    title: group.title,
  };
}

/** Teile (Dateien) des Ziels, die ein Zusammenführen ersetzt oder behält. */
export const partCount = (book: LibraryBook) => tracksOf(book.audio).length;

/** Dauern je Teil: gespeichert, sonst aus der Datei ermittelt (`null` = unbekannt). */
async function durationsOf(book: LibraryBook): Promise<(number | null)[]> {
  const tracks = tracksOf(book.audio);
  const paths = book.audio_paths ?? [];
  const out: (number | null)[] = [];
  for (let i = 0; i < tracks.length; i++) {
    const known = tracks[i].duration_ms;
    if (known != null && known > 0) {
      out.push(known);
      continue;
    }
    const url = assetUrl(paths[i]);
    out.push(url ? await probeDuration(url) : null);
  }
  return out;
}

export async function mergeGroup(
  group: SplitGroup,
  plan: MergePlan,
): Promise<LibraryBook> {
  const { target, sources } = plan;
  // Alles, was nach einem Erfolg aufzuräumen ist, vorher festhalten.
  const oldJobs = plan.replacesAudio ? pendingJobs(target) : [];
  const oldText =
    plan.replacesAudio && target.text?.origin === "transcript"
      ? target.text
      : null;
  const entries: MergeEntry[] = [];
  if (!plan.replacesAudio && target.audio)
    entries.push({
      durations: await durationsOf(target),
      progress: loadProgress(audioProgressKey(target.id)),
    });
  for (const source of sources)
    entries.push({
      durations: await durationsOf(source),
      progress: loadProgress(audioProgressKey(source.id)),
    });
  const position = mergedPosition(entries);

  if (!plan.existing) {
    // Das erste Buch bekommt den Stamm-Titel (ohne Disc-/Track-Angaben).
    const retitled = await commands.librarySave({
      ...target,
      title: plan.title,
      authors: target.authors.length ? target.authors : group.authors,
    });
    if (retitled.status !== "ok") throw new ImportError(retitled.error);
  }
  // Ersetzen entscheidet der Kern atomar: Dateien und buch.json zuerst,
  // Aufräumen hier erst nach Erfolg.
  const merged = await commands.libraryMergeAudio(
    target.id,
    sources.map((b) => b.id),
    plan.replacesAudio,
  );
  if (merged.status !== "ok") throw new ImportError(merged.error);
  for (const id of oldJobs) await commands.meetingsDelete(id);
  if (oldText) {
    removeProgress(textProgressKey(oldText.page_id));
    await commands.pagesDelete(oldText.page_id);
  }
  if (plan.replacesAudio) removeProgress(audioProgressKey(target.id));
  for (const source of sources) removeProgress(audioProgressKey(source.id));
  if (position) {
    const total = durationsSum(merged.data);
    recordPosition(audioProgressKey(target.id), {
      to: position.ms,
      length: total ?? 0,
      unit: "ms",
    });
  }
  await flushProgress();
  return merged.data;
}

const durationsSum = (book: LibraryBook): number | null =>
  totalDuration(book.audio);
