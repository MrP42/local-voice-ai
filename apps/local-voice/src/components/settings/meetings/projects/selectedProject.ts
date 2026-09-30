import { useSyncExternalStore } from "react";
import { ALL_PROJECTS, NO_PROJECT } from "./projectModel";

/**
 * Das links gewaehlte Projekt, geteilt zwischen Projekte-Spalte, Leiste und
 * allem, was es braucht (etwa der Startdialog, der das Projekt vorbelegt).
 * Gespeichert im localStorage (Schluessel wie `usePersistentState`), damit die
 * Wahl Neuladen und Neustart uebersteht; der Speicher ist die Wahrheit, der
 * Zwischenspeicher hier nur ein Lesecache. Fehlt der Speicher oder ist er
 * gesperrt, gilt "Alle Aufnahmen".
 */
const STORAGE_KEY = "lva.ui.meetings.project";

const listeners = new Set<() => void>();
let cache: string | null = null;

const read = (): string => {
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    return stored !== null && stored.trim() !== "" ? stored : ALL_PROJECTS;
  } catch {
    return ALL_PROJECTS;
  }
};

/** Die Auswahl als Text: "all", "none" oder eine Projekt-ID (Ordner-ID). */
export const getSelectedProject = (): string => (cache ??= read());

export const setSelectedProject = (selection: string) => {
  if (selection === getSelectedProject()) return;
  cache = selection;
  try {
    window.localStorage.setItem(STORAGE_KEY, selection);
  } catch {
    /* nicht merken ist verkraftbar, abstuerzen nicht */
  }
  listeners.forEach((listener) => listener());
};

const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};

/**
 * Das gewaehlte Projekt als Hook. `projectId` ist die Ordner-ID, wenn ein echtes
 * Projekt gewaehlt ist, sonst `null` ("Alle Aufnahmen", "Ohne Projekt"). Ob es
 * das Projekt noch gibt, prueft der Aufrufer gegen seine Ordnerliste.
 */
export function useSelectedProject(): {
  selection: string;
  projectId: string | null;
} {
  const selection = useSyncExternalStore(
    subscribe,
    getSelectedProject,
    () => ALL_PROJECTS,
  );
  return {
    selection,
    projectId:
      selection === ALL_PROJECTS || selection === NO_PROJECT ? null : selection,
  };
}
