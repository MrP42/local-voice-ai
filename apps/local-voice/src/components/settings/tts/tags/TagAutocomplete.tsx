import React, {
  forwardRef,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { localizedLabel, searchTags } from "@/lib/tags/registry";
import type { TagDef } from "@/lib/tags/types";

export interface TagAutocompleteHandle {
  /** true = Taste verarbeitet; der Editor unterdrückt dann das
   *  Standardverhalten der textarea (Cursor bewegen, Zeilenumbruch …). */
  handleKey(key: string): boolean;
}

interface TagAutocompleteProps {
  /** Viewport-Rect des Caret-Spans im Mirror. */
  anchor: { left: number; top: number; bottom: number };
  /** Der bereits getippte Teil zwischen `[` und Caret. */
  query: string;
  onPick(def: TagDef): void;
  onDismiss(): void;
}

const MAX_RESULTS = 8;

/**
 * Vorschlagsliste hinter einem ungeschlossenen `[`: Die textarea behält den
 * Fokus, ↑/↓/Enter/Escape werden vom Editor hierher durchgereicht. Enter
 * vervollständigt zu `name]` (der Editor ersetzt den getippten Teil).
 */
export const TagAutocomplete = forwardRef<
  TagAutocompleteHandle,
  TagAutocompleteProps
>(({ anchor, query, onPick, onDismiss }, ref) => {
  const { t, i18n } = useTranslation();
  const uiLang = i18n.language?.split("-")[0] ?? "en";

  const results = useMemo(
    () => searchTags(query.trim(), uiLang).slice(0, MAX_RESULTS),
    [query, uiLang],
  );

  const [selected, setSelected] = useState(0);
  useEffect(() => {
    setSelected(0);
  }, [query]);
  const clampedSelected = Math.min(selected, Math.max(0, results.length - 1));

  useImperativeHandle(
    ref,
    () => ({
      handleKey(key: string): boolean {
        if (key === "Escape") {
          onDismiss();
          return true;
        }
        if (results.length === 0) return false;
        if (key === "ArrowDown") {
          setSelected((clampedSelected + 1) % results.length);
          return true;
        }
        if (key === "ArrowUp") {
          setSelected((clampedSelected - 1 + results.length) % results.length);
          return true;
        }
        if (key === "Enter") {
          onPick(results[clampedSelected]);
          return true;
        }
        return false;
      },
    }),
    [results, clampedSelected, onPick, onDismiss],
  );

  const listRef = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  useLayoutEffect(() => {
    const el = listRef.current;
    if (!el) return;
    const width = el.offsetWidth;
    const height = el.offsetHeight;
    const left = Math.min(
      Math.max(8, anchor.left),
      Math.max(8, window.innerWidth - width - 8),
    );
    let top = anchor.bottom + 2;
    if (top + height > window.innerHeight - 8) {
      top = Math.max(8, anchor.top - height - 2);
    }
    setPos({ left, top });
  }, [anchor, results.length]);

  useEffect(() => {
    listRef.current
      ?.querySelector('[aria-selected="true"]')
      ?.scrollIntoView({ block: "nearest" });
  }, [clampedSelected, results]);

  if (results.length === 0) return null;

  return createPortal(
    <div
      ref={listRef}
      role="listbox"
      aria-label={t("tts.editor.autocompleteAria")}
      style={
        pos
          ? { left: pos.left, top: pos.top }
          : { left: -9999, top: 0, visibility: "hidden" }
      }
      className="fixed z-50 max-h-72 w-64 overflow-y-auto rounded-lg border border-mid-gray/40 bg-background py-1 shadow-lg"
    >
      {results.map((tag, index) => (
        <button
          key={tag.id}
          type="button"
          role="option"
          aria-selected={index === clampedSelected}
          tabIndex={-1}
          // preventDefault: die textarea darf den Fokus nicht verlieren.
          onMouseDown={(event) => event.preventDefault()}
          onClick={() => onPick(tag)}
          onMouseEnter={() => setSelected(index)}
          className={`flex min-h-[36px] w-full cursor-pointer items-center justify-between gap-2 px-3 py-1 text-start text-sm ${
            index === clampedSelected
              ? "bg-logo-primary/15 text-text"
              : "text-text/80"
          }`}
        >
          <span className="truncate">{localizedLabel(tag, uiLang)}</span>
          <span className="shrink-0 text-xs text-text/45">[{tag.insert}]</span>
        </button>
      ))}
    </div>,
    document.body,
  );
});

TagAutocomplete.displayName = "TagAutocomplete";
