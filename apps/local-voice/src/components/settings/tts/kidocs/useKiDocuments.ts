import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  commands,
  type BookTemplate,
  type KiDocument,
  type SummarySource,
} from "@/bindings";

/** Ein laufender Auftrag „KI-Dokument erstellen“ (lva#183). */
export interface KiRun {
  pageId: string;
  templateTitle: string;
  sourceLabel: string;
  done: number;
  total: number;
}

export interface KiDocumentsState {
  docs: KiDocument[];
  templates: BookTemplate[];
  loading: boolean;
  /** Laufender Auftrag – nur, wenn er zu dieser Seite gehört. */
  run: KiRun | null;
  /** Läuft irgendein Auftrag (es darf nur einer laufen)? */
  busy: boolean;
  reload: () => Promise<void>;
  /** Startet die Erstellung; liefert das neue Dokument oder einen Fehlercode. */
  create: (
    source: SummarySource,
    sourceLabel: string,
    template: BookTemplate,
  ) => Promise<{ doc: KiDocument } | { error: string }>;
  cancel: () => void;
}

/**
 * KI-Dokumente einer Seite: Liste, Vorlagen und der eine laufende Auftrag.
 * Liegt in der Reader-Seite statt in der Liste, damit ein Auftrag weiterläuft
 * und sichtbar bleibt, während ein Dokument offen ist.
 */
export function useKiDocuments(pageId: string | null): KiDocumentsState {
  const [docs, setDocs] = useState<KiDocument[]>([]);
  const [templates, setTemplates] = useState<BookTemplate[]>([]);
  const [loading, setLoading] = useState(false);
  const [run, setRun] = useState<KiRun | null>(null);
  const runPage = useRef<string | null>(null);
  /** Läuft ein Auftrag? Ein zweiter Aufruf darf den ersten nicht beenden. */
  const running = useRef(false);
  const pageRef = useRef(pageId);
  pageRef.current = pageId;

  const reload = useCallback(async () => {
    const page = pageRef.current;
    if (!page) {
      setDocs([]);
      return;
    }
    setLoading(true);
    const result = await commands.bookAiDocuments(page);
    setLoading(false);
    // Inzwischen eine andere Seite offen: deren Liste nicht überschreiben.
    if (page !== pageRef.current) return;
    setDocs(
      result.status === "ok" && Array.isArray(result.data) ? result.data : [],
    );
  }, []);

  useEffect(() => {
    void reload();
  }, [pageId, reload]);

  useEffect(() => {
    void commands.bookAiTemplates().then((result) => {
      if (result.status === "ok" && Array.isArray(result.data))
        setTemplates(result.data);
    });
  }, []);

  useEffect(() => {
    const un = listen<{
      page_id: string;
      kind: string;
      done: number;
      total: number;
    }>(
      "book-ai-progress",
      (e) => {
        // Begriffsläufe melden über dasselbe Ereignis (lva#185).
        if (e.payload.kind !== "summary") return;
        if (e.payload.page_id !== runPage.current) return;
        setRun((cur) =>
          cur ? { ...cur, done: e.payload.done, total: e.payload.total } : cur,
        );
      },
    );
    return () => void un.then((off) => off());
  }, []);

  const create = useCallback<KiDocumentsState["create"]>(
    async (source, sourceLabel, template) => {
      const page = pageRef.current;
      if (!page) return { error: "page_missing" };
      if (running.current) return { error: "book_ai:busy" };
      running.current = true;
      runPage.current = page;
      setRun({
        pageId: page,
        templateTitle: template.title,
        sourceLabel,
        done: 0,
        total: 0,
      });
      let result: Awaited<ReturnType<typeof commands.bookAiSummarize>>;
      try {
        result = await commands.bookAiSummarize(
          page,
          source,
          sourceLabel,
          template.id,
        );
      } catch (e) {
        result = { status: "error", error: String(e) };
      } finally {
        running.current = false;
        runPage.current = null;
        setRun(null);
      }
      if (result.status === "error") return { error: result.error };
      if (page === pageRef.current) await reload();
      return { doc: result.data };
    },
    [reload],
  );

  const cancel = useCallback(() => {
    void commands.bookAiCancel();
  }, []);

  return {
    docs,
    templates,
    loading,
    run: run && run.pageId === pageId ? run : null,
    busy: run !== null,
    reload,
    create,
    cancel,
  };
}
