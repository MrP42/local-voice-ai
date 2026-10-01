import { useCallback, useEffect, useRef, useState } from "react";
import {
  commands,
  type WorkflowCatalog,
  type WorkflowItem,
  type WorkflowRunSummary,
  type WorkflowStatus,
  type WorkflowTemplate,
} from "@/bindings";

/** Wie oft Laeufe und Stand nachgeladen werden, solange die Seite offen ist. */
const POLL_MS = 4000;

/** Fehlertext eines Kommandos (Klartext des Backends). */
export const errorOf = (error: unknown): string => {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "";
};

/** Katalog und Vorlagen: aendern sich nur mit der App, einmal laden. */
export function useCatalog() {
  const [catalog, setCatalog] = useState<WorkflowCatalog | null>(null);
  const [templates, setTemplates] = useState<WorkflowTemplate[]>([]);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let alive = true;
    void (async () => {
      try {
        const [c, t] = await Promise.all([
          commands.workflowCatalog(),
          commands.workflowTemplates(),
        ]);
        if (!alive) return;
        if (c.status === "ok") setCatalog(c.data);
        else setFailed(true);
        if (t.status === "ok") setTemplates(t.data);
      } catch {
        if (alive) setFailed(true);
      }
    })();
    return () => {
      alive = false;
    };
  }, []);

  return { catalog, templates, failed };
}

/** Liste der Ablaeufe samt Stand der Ausloeser; fragt regelmaessig nach. */
export function useWorkflows(version: number) {
  const [items, setItems] = useState<WorkflowItem[]>([]);
  const [status, setStatus] = useState<WorkflowStatus | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [failed, setFailed] = useState(false);
  const alive = useRef(true);

  const reload = useCallback(async () => {
    try {
      const [list, st] = await Promise.all([
        commands.workflowList(),
        commands.workflowStatus(),
      ]);
      if (!alive.current) return;
      if (list.status === "ok") {
        setItems(list.data);
        setFailed(false);
      } else {
        setFailed(true);
      }
      if (st.status === "ok") setStatus(st.data);
    } catch {
      if (alive.current) setFailed(true);
    }
    if (alive.current) setLoaded(true);
  }, []);

  useEffect(() => {
    alive.current = true;
    void reload();
    const timer = window.setInterval(() => {
      if (document.visibilityState !== "hidden") void reload();
    }, POLL_MS);
    return () => {
      alive.current = false;
      window.clearInterval(timer);
    };
  }, [reload, version]);

  /** Ersetzt oder ergaenzt eine Zeile mit der Antwort eines Kommandos. */
  const upsert = useCallback((item: WorkflowItem) => {
    setItems((list) =>
      list.some((i) => i.id === item.id)
        ? list.map((i) => (i.id === item.id ? item : i))
        : [item, ...list],
    );
  }, []);

  return { items, status, loaded, failed, reload, upsert };
}

/** Laeufe (eines Ablaufs oder alle); fragt regelmaessig nach. */
export function useRuns(
  workflowId: string | null,
  openOnly: boolean,
  version: number,
) {
  const [runs, setRuns] = useState<WorkflowRunSummary[]>([]);
  const [loaded, setLoaded] = useState(false);
  const alive = useRef(true);

  const reload = useCallback(async () => {
    try {
      const result = await commands.workflowRuns(workflowId, openOnly, 100);
      if (alive.current && result.status === "ok") setRuns(result.data);
    } catch {
      /* die naechste Runde versucht es wieder */
    }
    if (alive.current) setLoaded(true);
  }, [workflowId, openOnly]);

  useEffect(() => {
    alive.current = true;
    void reload();
    const timer = window.setInterval(() => {
      if (document.visibilityState !== "hidden") void reload();
    }, POLL_MS);
    return () => {
      alive.current = false;
      window.clearInterval(timer);
    };
  }, [reload, version]);

  return { runs, loaded, reload };
}
