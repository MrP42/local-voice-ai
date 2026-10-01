import { useCallback, useEffect, useRef, useState } from "react";
import {
  commands,
  type IntegrationView,
  type PendingApproval,
} from "@/bindings";

/** Wie oft die Seite nach neuen Freigaben fragt, solange sie offen ist. */
const APPROVAL_POLL_MS = 5000;

/** Das Register: Liste laden, nach einer Aenderung die Zeile ersetzen. */
export function useIntegrations() {
  const [views, setViews] = useState<IntegrationView[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [loadFailed, setLoadFailed] = useState(false);

  const reload = useCallback(async () => {
    try {
      const result = await commands.integrationsList();
      if (result.status === "ok") {
        setViews(result.data ?? []);
        setLoadFailed(false);
      } else {
        setLoadFailed(true);
      }
    } catch {
      setLoadFailed(true);
    }
    setLoaded(true);
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  /** Ersetzt (oder ergaenzt) eine Zeile mit der Antwort eines Kommandos. */
  const upsert = useCallback((view: IntegrationView) => {
    setViews((list) =>
      list.some((v) => v.integration.id === view.integration.id)
        ? list.map((v) => (v.integration.id === view.integration.id ? view : v))
        : [...list, view],
    );
  }, []);

  const remove = useCallback((id: string) => {
    setViews((list) => list.filter((v) => v.integration.id !== id));
  }, []);

  return { views, loaded, loadFailed, reload, upsert, remove };
}

/** Offene Freigaben; fragt regelmaessig nach, solange `active` gilt. */
export function usePendingApprovals(active: boolean) {
  const [pending, setPending] = useState<PendingApproval[]>([]);
  const alive = useRef(true);

  const reload = useCallback(async () => {
    try {
      const result = await commands.approvalsPending();
      if (alive.current && result.status === "ok") {
        setPending(result.data ?? []);
      }
    } catch {
      /* die naechste Runde versucht es wieder */
    }
  }, []);

  useEffect(() => {
    alive.current = true;
    if (!active) return () => undefined;
    void reload();
    const timer = window.setInterval(() => {
      if (document.visibilityState !== "hidden") void reload();
    }, APPROVAL_POLL_MS);
    return () => {
      alive.current = false;
      window.clearInterval(timer);
    };
  }, [active, reload]);

  return { pending, reload };
}
