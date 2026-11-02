import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { commands, type BookTerm, type TermHit } from "@/bindings";

/** Eine geprüfte Fundstelle im angezeigten Text (UTF-16, Ende exklusiv). */
export interface TermMark {
  start: number;
  end: number;
  termId: string;
  page: number | null;
}

export interface TermsRunState {
  done: number;
  total: number;
}

export interface BookTermsState {
  terms: BookTerm[];
  marks: TermMark[];
  run: TermsRunState | null;
  /** Lauf starten; liefert den Fehlerschlüssel oder null. */
  extract: () => Promise<{ error: string } | { done: number; failed: number }>;
  cancel: () => void;
  byId: Map<string, BookTerm>;
}

const norm = (s: string) =>
  s.split(/\s+/).filter(Boolean).join(" ").toLocaleLowerCase();

/**
 * Fundstellen nur übernehmen, wenn der Text an der Stelle noch zum Begriff
 * passt. Die Suche läuft auf dem gesicherten Text; während des Tippens kann
 * der angezeigte Text schon anders sein, dann fällt die Stelle weg, statt
 * an der falschen Stelle zu stehen.
 */
export function verifiedMarks(
  text: string,
  hits: TermHit[],
  terms: Map<string, BookTerm>,
): TermMark[] {
  const out: TermMark[] = [];
  for (const h of hits) {
    const term = terms.get(h.term_id);
    if (!term || term.hidden || h.end > text.length || h.start >= h.end)
      continue;
    const found = norm(text.slice(h.start, h.end));
    const names = [term.term, ...term.variants].map(norm);
    if (!names.includes(found)) continue;
    out.push({ start: h.start, end: h.end, termId: h.term_id, page: h.page });
  }
  return out;
}

/**
 * Begriffe eines Buchs (lva#185): Liste, Fundstellen im Text und der
 * Erkennungslauf. `text` ist der angezeigte Originaltext; `enabled` schaltet
 * Laden und Suche ab (andere Reiter, Hervorhebung aus).
 */
export function useBookTerms(
  pageId: string | null,
  text: string,
  enabled: boolean,
): BookTermsState {
  const [terms, setTerms] = useState<BookTerm[]>([]);
  const [hits, setHits] = useState<{ page: string; hits: TermHit[] } | null>(
    null,
  );
  const [run, setRun] = useState<TermsRunState | null>(null);
  const runPage = useRef<string | null>(null);
  const pageRef = useRef(pageId);
  pageRef.current = pageId;

  const loadTerms = useCallback(async (page: string) => {
    const r = await commands.bookAiTerms(page);
    if (pageRef.current !== page) return;
    setTerms(r.status === "ok" && Array.isArray(r.data) ? r.data : []);
  }, []);

  useEffect(() => {
    setTerms([]);
    setHits(null);
    if (!pageId) return;
    void loadTerms(pageId);
  }, [pageId, loadTerms]);

  // Fundstellen nach dem Sichern des Texts neu suchen (gesichert wird eine
  // halbe Sekunde nach der letzten Änderung).
  useEffect(() => {
    if (!pageId || !enabled || terms.length === 0) return;
    const handle = window.setTimeout(() => {
      void commands.bookAiTermHits(pageId).then((r) => {
        if (pageRef.current !== pageId) return;
        setHits({
          page: pageId,
          hits: r.status === "ok" && Array.isArray(r.data) ? r.data : [],
        });
      });
    }, 1200);
    return () => window.clearTimeout(handle);
  }, [pageId, enabled, terms, text]);

  useEffect(() => {
    const un = listen<{
      page_id: string;
      kind: string;
      done: number;
      total: number;
    }>("book-ai-progress", (e) => {
      if (e.payload.kind !== "terms" || e.payload.page_id !== runPage.current)
        return;
      setRun({ done: e.payload.done, total: e.payload.total });
    });
    return () => void un.then((off) => off());
  }, []);

  const byId = useMemo(() => new Map(terms.map((t) => [t.id, t])), [terms]);
  const marks = useMemo(
    () =>
      enabled && hits && hits.page === pageId
        ? verifiedMarks(text, hits.hits, byId)
        : [],
    [enabled, hits, pageId, text, byId],
  );

  const extract = useCallback<BookTermsState["extract"]>(async () => {
    const page = pageRef.current;
    if (!page) return { error: "page_missing" };
    if (runPage.current) return { error: "book_ai:busy" };
    runPage.current = page;
    setRun({ done: 0, total: 0 });
    try {
      const r = await commands.bookAiTermsExtract(page);
      if (r.status === "error") return { error: r.error };
      if (pageRef.current === page) setTerms(r.data.terms);
      return { done: r.data.chapters_done, failed: r.data.chapters_failed };
    } finally {
      runPage.current = null;
      setRun(null);
    }
  }, []);

  const cancel = useCallback(() => void commands.bookAiCancel(), []);

  return { terms, marks, run, extract, cancel, byId };
}
