import { commands, type LibraryBook } from "@/bindings";
import type { SplitGroup } from "./audioGroups";
import { pendingJobs, tracksOf } from "./books";
import { audioProgressKey, flushProgress, removeProgress } from "./progress";
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

export async function mergeGroup(
  group: SplitGroup,
  plan: MergePlan,
): Promise<LibraryBook> {
  const { target, sources } = plan;
  if (plan.replacesAudio) {
    // Altes Hörbuch des Ziels lösen: Verweis weg, offene Aufträge löschen,
    // Dateien räumt der Kern beim Zusammenführen weg.
    const cleared = await commands.librarySave({
      ...target,
      audio: null,
      audio_path: null,
      audio_paths: [],
    });
    if (cleared.status !== "ok") throw new ImportError(cleared.error);
    for (const id of pendingJobs(target)) await commands.meetingsDelete(id);
    removeProgress(audioProgressKey(target.id));
  } else if (!plan.existing) {
    // Das erste Buch bekommt den Stamm-Titel (ohne Disc-/Track-Angaben).
    const retitled = await commands.librarySave({
      ...target,
      title: plan.title,
      authors: target.authors.length ? target.authors : group.authors,
    });
    if (retitled.status !== "ok") throw new ImportError(retitled.error);
  }
  const merged = await commands.libraryMergeAudio(
    target.id,
    sources.map((b) => b.id),
  );
  if (merged.status !== "ok") throw new ImportError(merged.error);
  for (const source of sources) removeProgress(audioProgressKey(source.id));
  await flushProgress();
  return merged.data;
}
