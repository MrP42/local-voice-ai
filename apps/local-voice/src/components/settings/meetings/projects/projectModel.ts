import type { Folder } from "@/bindings";

/**
 * Die Auswahl in der Projekte-Spalte: alle Aufnahmen, die ohne Projekt oder
 * ein Projekt (= Ordner der obersten Ebene, `Folder.id`). Gespeichert wird der
 * Text; Ordner-IDs sind ULIDs und kollidieren nie mit diesen beiden Werten.
 */
export const ALL_PROJECTS = "all";
export const NO_PROJECT = "none";

export const isSelectionText = (value: string) => value.trim() !== "";

/** Gibt es die Auswahl noch? Ein geloeschtes Projekt faellt auf "Alle" zurueck. */
export const resolveSelection = (
  selection: string,
  folders: Pick<Folder, "id">[],
): string =>
  selection === ALL_PROJECTS ||
  selection === NO_PROJECT ||
  folders.some((f) => f.id === selection)
    ? selection
    : ALL_PROJECTS;

/** Kuerzel fuer die eingeklappte Leiste: Anfangsbuchstaben zweier Woerter, sonst
 *  die ersten beiden Zeichen ("Kunde Stadtwerke" -> "KS", "Podcast" -> "Po"). */
export const abbreviation = (name: string): string => {
  const words = name.trim().split(/\s+/).filter(Boolean);
  const letters = (w: string) => Array.from(w)[0] ?? "";
  if (words.length >= 2) {
    return (letters(words[0]) + letters(words[1])).toLocaleUpperCase();
  }
  const chars = Array.from(words[0] ?? "");
  return (
    (chars[0] ?? "").toLocaleUpperCase() + (chars[1] ?? "").toLocaleLowerCase()
  );
};

/**
 * Neue Reihenfolge der Projekt-IDs, wenn `id` um einen Platz nach oben
 * (`-1`) oder unten (`+1`) rutscht. `null`, wenn es dort nichts zu tauschen
 * gibt (ganz oben / ganz unten / unbekannt).
 */
export const movedOrder = (
  ids: string[],
  id: string,
  direction: -1 | 1,
): string[] | null => {
  const from = ids.indexOf(id);
  const to = from + direction;
  if (from < 0 || to < 0 || to >= ids.length) return null;
  const next = [...ids];
  [next[from], next[to]] = [next[to], next[from]];
  return next;
};

/** Wohin eine Besprechung nach dem Ablegen gehoert. */
export type DropMode = "move" | "add";

/**
 * Projekte einer Besprechung nach dem Ablegen auf `target`:
 *
 * - `target` = Projekt, `add` (Strg): dazu, die bisherigen bleiben.
 * - `target` = Projekt, `move`: aus dem Projekt, aus dem gezogen wurde
 *   (`from`), heraus und hinein; kommt die Besprechung aus "Alle" oder
 *   "Ohne Projekt", liegt sie danach genau in diesem einen Projekt.
 * - `target` = "Ohne Projekt": danach in keinem Projekt.
 *
 * `null`, wenn sich nichts aendert.
 */
export const dropResult = (
  current: string[],
  from: string,
  target: string,
  mode: DropMode,
): string[] | null => {
  let next: string[];
  if (target === NO_PROJECT) {
    next = [];
  } else if (mode === "add") {
    next = current.includes(target) ? current : [...current, target];
  } else if (from === ALL_PROJECTS || from === NO_PROJECT) {
    next = [target];
  } else {
    next = current.filter((id) => id !== from);
    if (!next.includes(target)) next.push(target);
  }
  const same =
    next.length === current.length && next.every((id) => current.includes(id));
  return same ? null : next;
};
