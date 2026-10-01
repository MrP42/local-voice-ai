import { useCallback, useEffect, useRef, useState } from "react";
import {
  commands,
  events,
  type ProjectCandidate,
  type ProjectMinutesSummary,
} from "@/bindings";
import { PROJECT_MINUTES_CHANGED_EVENT } from "@/lib/projectMinutes";

/**
 * Die Projekt-Protokolle eines Projekts (juengste zuerst). Laedt beim Wechsel des
 * Projekts und neu, wenn ein Lauf endet (Ereignis des Backends) oder ein
 * Dokument geloescht wurde (lokale Meldung). `null`: kein echtes Projekt gewaehlt.
 */
export function useProjectMinutesList(folderId: string | null) {
  const [items, setItems] = useState<ProjectMinutesSummary[]>([]);
  const requestRef = useRef(0);

  const reload = useCallback(async () => {
    if (!folderId) {
      setItems([]);
      return;
    }
    const request = ++requestRef.current;
    const result = await commands.projectMinutesList(folderId);
    // Eine ueberholte Antwort (Projekt inzwischen gewechselt) zaehlt nicht.
    if (request !== requestRef.current) return;
    setItems(result.status === "ok" ? (result.data ?? []) : []);
  }, [folderId]);

  useEffect(() => {
    void reload();
  }, [reload]);

  useEffect(() => {
    const un = events.projectMinutesEvent.listen((event) => {
      if (event.payload.folder_id === folderId) void reload();
    });
    const local = () => void reload();
    window.addEventListener(PROJECT_MINUTES_CHANGED_EVENT, local);
    return () => {
      void un.then((f) => f());
      window.removeEventListener(PROJECT_MINUTES_CHANGED_EVENT, local);
    };
  }, [folderId, reload]);

  return { items, reload };
}

/**
 * Welche Aufnahmen eines Projekts in ein Projekt-Protokoll eingehen koennen (und
 * warum nicht), solange die Auswahl offen ist (`active`). Wird neu geladen, wenn
 * sich die Liste der Besprechungen aendert (`refreshKey`).
 */
export function useProjectCandidates(
  folderId: string | null,
  active: boolean,
  refreshKey: unknown,
) {
  const [candidates, setCandidates] = useState<ProjectCandidate[] | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    if (!active || !folderId) {
      setCandidates(null);
      setFailed(false);
      return;
    }
    let alive = true;
    void commands.projectMinutesCandidates(folderId).then((result) => {
      if (!alive) return;
      if (result.status === "ok") {
        setCandidates(result.data ?? []);
        setFailed(false);
      } else {
        setCandidates([]);
        setFailed(true);
      }
    });
    return () => {
      alive = false;
    };
  }, [active, folderId, refreshKey]);

  return { candidates, failed };
}
