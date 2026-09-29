import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Search, X } from "lucide-react";
import { Input } from "../../../ui/Input";

/** Entprellung der Suche: erst nach dieser Tipp-Pause geht EIN Aufruf raus. */
export const SEARCH_DEBOUNCE_MS = 250;

interface SearchBarProps {
  /** Bekommt den getrimmten Suchtext, entprellt. */
  onSearch: (query: string) => void;
}

export const SearchBar: React.FC<SearchBarProps> = ({ onSearch }) => {
  const { t } = useTranslation();
  const [text, setText] = useState("");
  // Der Rueckruf darf die Entprellung nicht neu starten, nur weil der
  // Elternteil ihn neu erzeugt.
  const onSearchRef = useRef(onSearch);
  onSearchRef.current = onSearch;

  useEffect(() => {
    const timer = window.setTimeout(
      () => onSearchRef.current(text.trim()),
      SEARCH_DEBOUNCE_MS,
    );
    return () => window.clearTimeout(timer);
  }, [text]);

  return (
    <div className="relative flex-1 min-w-0">
      <Search
        width={14}
        height={14}
        aria-hidden="true"
        className="absolute start-2.5 top-1/2 -translate-y-1/2 text-text/50 pointer-events-none"
      />
      <Input
        type="text"
        role="searchbox"
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape" && text !== "") {
            e.stopPropagation();
            setText("");
          }
        }}
        placeholder={t("meetings.search.placeholder")}
        aria-label={t("meetings.search.label")}
        variant="compact"
        className="w-full ps-8 pe-8 font-normal"
        data-testid="meeting-search-input"
      />
      {text !== "" && (
        <button
          type="button"
          className="absolute end-1.5 top-1/2 -translate-y-1/2 p-1 rounded text-text/50 hover:text-text cursor-pointer"
          title={t("meetings.search.clear")}
          aria-label={t("meetings.search.clear")}
          onClick={() => setText("")}
        >
          <X width={14} height={14} />
        </button>
      )}
    </div>
  );
};

type SnippetPart = { text: string; marked: boolean };

const ENTITIES: Record<string, string> = {
  "&amp;": "&",
  "&lt;": "<",
  "&gt;": ">",
  "&quot;": '"',
};

const unescape = (s: string) =>
  s.replace(/&(amp|lt|gt|quot);/g, (m) => ENTITIES[m] ?? m);

/** Zerlegt das Snippet des Stores (maskiertes HTML, nur `<mark>` echt) in
 *  Textteile. Gerendert wird als React-Text, nie als HTML. */
const snippetParts = (snippet: string): SnippetPart[] => {
  const parts: SnippetPart[] = [];
  let marked = false;
  for (const piece of snippet.split(/(<mark>|<\/mark>)/)) {
    if (piece === "<mark>") marked = true;
    else if (piece === "</mark>") marked = false;
    else if (piece !== "") parts.push({ text: unescape(piece), marked });
  }
  return parts;
};

export const SearchSnippet: React.FC<{ snippet: string }> = ({ snippet }) => (
  <p
    className="text-xs text-text/70 line-clamp-2"
    data-testid="meeting-search-snippet"
  >
    {snippetParts(snippet).map((part, i) =>
      part.marked ? (
        <mark
          key={i}
          className="bg-logo-primary/25 text-text rounded-sm px-0.5"
        >
          {part.text}
        </mark>
      ) : (
        <React.Fragment key={i}>{part.text}</React.Fragment>
      ),
    )}
  </p>
);
