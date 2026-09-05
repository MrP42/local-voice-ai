import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { commands, type BuilderDraft } from "@/bindings";

/** Wie lange nach der letzten Eingabe gewartet wird, bevor der Entwurf auf
 *  die Platte geht. Kurz genug, dass ein Absturz hoechstens einen Satz
 *  kostet, lang genug, dass Tippen keine Schreiblast erzeugt. */
const SAVE_DEBOUNCE_MS = 600;

export interface BuilderProgress {
  done: number;
  total: number;
}

/**
 * Der Entwurf lebt im Backend, nicht hier: dieser Hook ist nur die Anzeige
 * davon plus das entprellte Zurueckschreiben. Ein Absturz kostet damit
 * hoechstens die letzte halbe Sekunde Tippen — nie die gewuerfelten
 * Kandidaten, die nicht reproduzierbar waeren.
 */
export function useBuilderDraft() {
  const [draft, setDraft] = useState<BuilderDraft | null>(null);
  const [drafts, setDrafts] = useState<BuilderDraft[]>([]);
  const [progress, setProgress] = useState<BuilderProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const saveTimer = useRef<number | null>(null);

  const reloadDrafts = useCallback(() => {
    void commands
      .ttsBuilderListDrafts()
      .then(setDrafts)
      .catch(() => undefined);
  }, []);

  useEffect(() => {
    reloadDrafts();
  }, [reloadDrafts]);

  // Der Fortschritt kommt als Ereignis, weil das Erzeugen Minuten dauert und
  // die Kandidaten einzeln erscheinen sollen.
  useEffect(() => {
    const un = listen<{ done: number; total: number }>(
      "tts-builder-progress",
      (event) => {
        setProgress({ done: event.payload.done, total: event.payload.total });
        void commands
          .ttsBuilderListDrafts()
          .then((all) => {
            setDrafts(all);
            setDraft((current) =>
              current
                ? (all.find((d) => d.id === current.id) ?? current)
                : current,
            );
          })
          .catch(() => undefined);
      },
    );
    return () => {
      void un.then((f) => f());
    };
  }, []);

  // Ein noch offener Schreibauftrag darf beim Schliessen nicht ins Leere
  // laufen: der Entwurf soll geschrieben sein, bevor die Karte verschwindet.
  useEffect(
    () => () => {
      if (saveTimer.current !== null) window.clearTimeout(saveTimer.current);
    },
    [],
  );

  /** Aendert den Entwurf sofort in der Anzeige und entprellt das Schreiben. */
  const patch = useCallback((changes: Partial<BuilderDraft>) => {
    setDraft((current) => {
      if (!current) return current;
      const next = { ...current, ...changes };
      if (saveTimer.current !== null) window.clearTimeout(saveTimer.current);
      saveTimer.current = window.setTimeout(() => {
        void commands.ttsBuilderUpdateDraft(next).catch(() => undefined);
      }, SAVE_DEBOUNCE_MS);
      return next;
    });
  }, []);

  return {
    draft,
    setDraft,
    drafts,
    reloadDrafts,
    progress,
    setProgress,
    error,
    setError,
    patch,
  };
}
