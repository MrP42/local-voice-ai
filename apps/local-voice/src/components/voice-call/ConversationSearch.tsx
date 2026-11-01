import React, { useEffect, useId, useMemo, useRef, useState } from "react";
import { MessageSquare, Search } from "lucide-react";
import { Dialog } from "../ui/Dialog";
import { matchesQuery, type ConversationListItem } from "./ConversationList";

/**
 * Suchoverlay des Verlaufs (wie bei ChatGPT): Schnellaktionen, die letzten Gespräche und bei
 * Eingabe die Treffer der Gesprächssuche. Pfeiltasten und Enter bedienen die Liste, Esc schließt.
 *
 * Rein darstellend (ADR-0021): kein Store, kein Backend, keine Übersetzung.
 */
export interface SearchAction {
  id: string;
  label: string;
  icon: React.ReactNode;
  /** Tastenkürzel zur Anzeige, z. B. „Strg+N“. */
  shortcut?: string;
  onSelect: () => void;
}

export interface ConversationSearchLabels {
  title: string;
  placeholder: string;
  sectionActions: string;
  sectionRecent: string;
  sectionResults: string;
  tabAll: string;
  tabActions: string;
  tabConversations: string;
  tabsLabel: string;
  noResults: string;
  close: string;
}

type Filter = "all" | "actions" | "conversations";

interface Row {
  key: string;
  label: string;
  icon: React.ReactNode;
  shortcut?: string;
  onSelect: () => void;
}

interface Section {
  id: string;
  title: string;
  rows: Row[];
}

const RECENT_COUNT = 5;

export interface ConversationSearchProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  items: ConversationListItem[];
  actions: SearchAction[];
  onSelect: (id: string) => void;
  labels: ConversationSearchLabels;
}

export const ConversationSearch: React.FC<ConversationSearchProps> = ({
  open,
  onOpenChange,
  items,
  actions,
  onSelect,
  labels,
}) => {
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listId = useId();

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setFilter("all");
    setActive(0);
  }, [open]);

  const sections = useMemo<Section[]>(() => {
    const q = query.trim().toLocaleLowerCase();
    const actionRows: Row[] = actions
      .filter((a) => !q || a.label.toLocaleLowerCase().includes(q))
      .map((a) => ({
        key: `action-${a.id}`,
        label: a.label,
        icon: a.icon,
        shortcut: a.shortcut,
        onSelect: a.onSelect,
      }));
    const conversationRows: Row[] = (
      q
        ? items.filter((i) => matchesQuery(i, q))
        : [...items]
            .filter((i) => !i.archived)
            .sort((a, b) => b.updatedAt - a.updatedAt)
            .slice(0, RECENT_COUNT)
    ).map((i) => ({
      key: `conv-${i.id}`,
      label: i.title,
      icon: <MessageSquare className="w-4 h-4" aria-hidden="true" />,
      onSelect: () => onSelect(i.id),
    }));
    const out: Section[] = [];
    if (filter !== "conversations" && actionRows.length > 0)
      out.push({
        id: "actions",
        title: labels.sectionActions,
        rows: actionRows,
      });
    if (filter !== "actions" && conversationRows.length > 0)
      out.push({
        id: "conversations",
        title: q ? labels.sectionResults : labels.sectionRecent,
        rows: conversationRows,
      });
    return out;
  }, [actions, items, query, filter, labels, onSelect]);

  const flat = sections.flatMap((s) => s.rows);
  const index = Math.min(active, Math.max(0, flat.length - 1));
  const optionId = (key: string) => `${listId}-${key}`;

  useEffect(() => {
    if (!open || flat.length === 0) return;
    document
      .getElementById(optionId(flat[index].key))
      ?.scrollIntoView?.({ block: "nearest" });
  }, [open, index, flat]);

  const run = (row: Row | undefined) => {
    if (!row) return;
    onOpenChange(false);
    row.onSelect();
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.nativeEvent.isComposing) return;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (flat.length === 0) return;
      const step = e.key === "ArrowDown" ? 1 : -1;
      setActive((index + step + flat.length) % flat.length);
    } else if (e.key === "Enter") {
      e.preventDefault();
      run(flat[index]);
    }
  };

  const tabs: { id: Filter; label: string }[] = [
    { id: "all", label: labels.tabAll },
    { id: "actions", label: labels.tabActions },
    { id: "conversations", label: labels.tabConversations },
  ];

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={<span className="sr-only">{labels.title}</span>}
      closeLabel={labels.close}
      showCloseButton={false}
      initialFocusRef={inputRef}
      className="max-w-xl"
      contentClassName="!p-0"
      contentFades={false}
    >
      <div
        data-testid="history-search"
        className="flex flex-col -mt-2"
        onKeyDown={onKeyDown}
      >
        <label className="relative block">
          <span className="sr-only">{labels.title}</span>
          <Search
            className="absolute start-3 top-1/2 -translate-y-1/2 w-4 h-4 text-text/50 pointer-events-none"
            aria-hidden="true"
          />
          <input
            ref={inputRef}
            role="combobox"
            aria-expanded="true"
            aria-controls={listId}
            aria-autocomplete="list"
            aria-activedescendant={
              flat.length > 0 ? optionId(flat[index].key) : undefined
            }
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setActive(0);
            }}
            placeholder={labels.placeholder}
            className="w-full min-h-11 ps-9 pe-3 rounded-md border border-mid-gray/30 bg-transparent text-sm text-text placeholder:text-text/50 focus:outline-none focus:border-logo-primary"
          />
        </label>
        <div
          id={listId}
          role="listbox"
          aria-label={labels.title}
          className="mt-2 max-h-[min(50vh,360px)] overflow-y-auto"
        >
          {flat.length === 0 && (
            <p className="m-0 px-2 py-4 text-sm text-text/60">
              {labels.noResults}
            </p>
          )}
          {sections.map((section) => (
            <div
              key={section.id}
              role="group"
              aria-label={section.title}
              className="mb-2"
            >
              <div className="px-2 pb-1 text-xs font-medium text-text/55">
                {section.title}
              </div>
              {section.rows.map((row) => {
                const on = flat[index]?.key === row.key;
                return (
                  <div
                    key={row.key}
                    id={optionId(row.key)}
                    role="option"
                    aria-selected={on}
                    onMouseMove={() =>
                      setActive(flat.findIndex((r) => r.key === row.key))
                    }
                    onClick={() => run(row)}
                    className={`flex items-center gap-2.5 min-h-11 md:min-h-9 px-2 rounded-md cursor-pointer text-sm text-text ${
                      on ? "bg-mid-gray/15" : ""
                    }`}
                  >
                    <span className="shrink-0 text-text/70">{row.icon}</span>
                    <span className="flex-1 min-w-0 truncate">
                      {row.label}
                    </span>
                    {row.shortcut && (
                      <kbd className="shrink-0 text-[11px] text-text/55 font-sans">
                        {row.shortcut}
                      </kbd>
                    )}
                  </div>
                );
              })}
            </div>
          ))}
        </div>
        <div
          role="tablist"
          aria-label={labels.tabsLabel}
          className="flex gap-1 pt-2 mt-1 border-t border-mid-gray/20"
        >
          {tabs.map((tab) => (
            <button
              key={tab.id}
              type="button"
              role="tab"
              aria-selected={filter === tab.id}
              onClick={() => {
                setFilter(tab.id);
                setActive(0);
                inputRef.current?.focus();
              }}
              className={`min-h-9 px-3 rounded-full text-xs cursor-pointer ${
                filter === tab.id
                  ? "bg-mid-gray/20 text-text font-semibold"
                  : "text-text/70 hover:bg-mid-gray/10"
              }`}
            >
              {tab.label}
            </button>
          ))}
        </div>
      </div>
    </Dialog>
  );
};
