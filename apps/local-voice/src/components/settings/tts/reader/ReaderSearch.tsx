import React from "react";
import { useTranslation } from "react-i18next";
import { Search, X } from "lucide-react";
import type { ReaderBlock } from "../../../../lib/reader/readerDoc";
import { chapterAt, type ContentsEntry } from "../../../../lib/reader/contents";
import {
  buildSearchIndex,
  MIN_QUERY,
  searchBook,
  type SearchHit,
  type SearchIndex,
  type SearchResult,
} from "../../../../lib/reader/bookSearch";

const EMPTY: SearchResult = { hits: [], total: 0, allWords: false };

interface ReaderSearchProps {
  blocks: ReaderBlock[];
  contents: ContentsEntry[];
  /** Zum Treffer blättern und ihn markieren; `null` hebt die Markierung auf. */
  onGo(hit: SearchHit | null): void;
}

/**
 * „Im Buch suchen“ über dem Inhaltsverzeichnis (lva#476): Trefferliste mit
 * Kapitel und Ausschnitt, Klick oder Enter blättert zur Fundstelle.
 * Strg+F setzt den Fokus ins Feld (in der Leseansicht; im Bearbeiten-Modus
 * gehört Strg+F der Suchleiste des Editors).
 */
export const ReaderSearch: React.FC<ReaderSearchProps> = ({
  blocks,
  contents,
  onGo,
}) => {
  const { t } = useTranslation();
  const [query, setQuery] = React.useState("");
  const [active, setActive] = React.useState(-1);
  const deferred = React.useDeferredValue(query);
  const inputRef = React.useRef<HTMLInputElement>(null);
  const listRef = React.useRef<HTMLOListElement>(null);
  const listId = React.useId();
  const searching = deferred.trim().length >= MIN_QUERY;
  // Der Index hängt nur am Text und entsteht erst bei der ersten Suche
  // (ein langes Buch braucht dafür einige Zehntelsekunden); danach bleibt er.
  const built = React.useRef<{
    blocks: ReaderBlock[];
    index: SearchIndex;
  } | null>(null);
  const index = React.useMemo(() => {
    if (!searching) return null;
    if (built.current?.blocks !== blocks)
      built.current = { blocks, index: buildSearchIndex(blocks) };
    return built.current.index;
  }, [blocks, searching]);
  const result = React.useMemo(
    () => (index ? searchBook(index, deferred) : EMPTY),
    [index, deferred],
  );

  // Neue Anfrage oder neuer Text: keine Auswahl, keine Markierung.
  const onGoRef = React.useRef(onGo);
  onGoRef.current = onGo;
  React.useEffect(() => {
    setActive(-1);
    onGoRef.current(null);
  }, [result]);

  React.useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey) || event.altKey || event.shiftKey)
        return;
      if (event.defaultPrevented || event.key.toLowerCase() !== "f") return;
      const target = event.target as Element | null;
      if (target?.closest?.('[role="dialog"]')) return;
      event.preventDefault();
      inputRef.current?.focus();
      inputRef.current?.select();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const go = (i: number) => {
    const hit = result.hits[i];
    if (!hit) return;
    setActive(i);
    onGo(hit);
    listRef.current
      ?.querySelector(`[data-hit="${i}"]`)
      ?.scrollIntoView({ block: "nearest" });
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Escape" && query) {
      setQuery("");
      e.preventDefault();
      e.stopPropagation();
    } else if (e.key === "Enter" && result.hits.length) {
      const n = result.hits.length;
      go(e.shiftKey ? (active <= 0 ? n - 1 : active - 1) : (active + 1) % n);
      e.preventDefault();
    }
  };

  const summary = !searching
    ? null
    : result.total === 0
      ? t("tts.reader.search.none")
      : result.allWords
        ? t("tts.reader.search.allWords", { count: result.total })
        : result.total > result.hits.length
          ? t("tts.reader.search.countMore", {
              count: result.total,
              shown: result.hits.length,
            })
          : t("tts.reader.search.count", { count: result.total });

  return (
    <div
      className="mt-3 border-t border-mid-gray/20 pt-2"
      data-testid="reader-search"
    >
      <div className="relative">
        <Search
          width={14}
          height={14}
          aria-hidden="true"
          className="pointer-events-none absolute start-2 top-1/2 -translate-y-1/2 text-text/50"
        />
        <input
          ref={inputRef}
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder={t("tts.reader.search.placeholder")}
          aria-label={t("tts.reader.search.label")}
          title={t("tts.reader.search.hint")}
          aria-controls={searching ? listId : undefined}
          className="w-full rounded-md border border-mid-gray/30 bg-background py-1 ps-7 pe-7 text-sm text-text placeholder:text-text/50 focus:border-logo-primary focus:outline-none [&::-webkit-search-cancel-button]:hidden"
          data-testid="reader-search-input"
        />
        {query && (
          <button
            type="button"
            onClick={() => {
              setQuery("");
              inputRef.current?.focus();
            }}
            aria-label={t("tts.reader.search.clear")}
            title={t("tts.reader.search.clear")}
            className="absolute end-1 top-1/2 -translate-y-1/2 cursor-pointer rounded p-0.5 text-text/60 hover:bg-mid-gray/15 hover:text-text focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
            data-testid="reader-search-clear"
          >
            <X width={14} height={14} aria-hidden="true" />
          </button>
        )}
      </div>
      {summary && (
        <p
          className="px-2 pt-1 text-xs text-text/60"
          role="status"
          data-testid="reader-search-summary"
        >
          {summary}
        </p>
      )}
      {searching && result.hits.length > 0 && (
        <ol
          id={listId}
          ref={listRef}
          className="mt-1 max-h-72 space-y-0.5 overflow-y-auto"
          aria-label={t("tts.reader.search.results")}
        >
          {result.hits.map((hit, i) => {
            const chapter = chapterAt(contents, hit.start);
            return (
              <li key={`${hit.start}-${i}`}>
                <button
                  type="button"
                  onClick={() => go(i)}
                  aria-current={i === active ? "location" : undefined}
                  data-hit={i}
                  className={`w-full cursor-pointer rounded-md px-2 py-1 text-start text-xs transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary ${
                    i === active
                      ? "bg-logo-primary/15 text-text"
                      : "text-text/75 hover:bg-mid-gray/15 hover:text-text"
                  }`}
                  data-testid="reader-search-hit"
                >
                  {chapter >= 0 && (
                    <span className="block truncate font-semibold text-text/80">
                      {contents[chapter].title}
                    </span>
                  )}
                  <span className="line-clamp-3 [overflow-wrap:anywhere]">
                    {hit.before}
                    <mark className="rounded-sm bg-logo-primary/35 px-px text-text">
                      {hit.match}
                    </mark>
                    {hit.after}
                  </span>
                </button>
              </li>
            );
          })}
        </ol>
      )}
    </div>
  );
};
