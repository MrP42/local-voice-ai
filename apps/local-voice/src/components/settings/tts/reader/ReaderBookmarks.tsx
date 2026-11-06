import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Bookmark as BookmarkIcon, BookmarkPlus, X } from "lucide-react";
import type { Bookmark } from "../../../../lib/reader/bookmarks";

interface ReaderBookmarksProps {
  bookmarks: Bookmark[];
  onAdd(): void;
  onGo(bookmark: Bookmark): void;
  onRemove(bookmark: Bookmark): void;
  /** Seite (0-basiert) einer Stelle, für die Anzeige. */
  pageOf?(offset: number): number | null;
}

const ITEM =
  "flex w-full cursor-pointer items-center gap-2 px-3 py-1.5 text-start text-sm hover:bg-mid-gray/15 focus-visible:outline-none focus-visible:bg-mid-gray/15";

/** Lesezeichen der Seite in der Kopfleiste der Buchansicht (lva#148). */
export const ReaderBookmarks: React.FC<ReaderBookmarksProps> = ({
  bookmarks,
  onAdd,
  onGo,
  onRemove,
  pageOf,
}) => {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    menuRef.current
      ?.querySelector<HTMLElement>("[data-bookmark-item]")
      ?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      setOpen(false);
      buttonRef.current?.focus();
    };
    const onDown = (e: PointerEvent) => {
      if (rootRef.current?.contains(e.target as Node)) return;
      setOpen(false);
    };
    document.addEventListener("keydown", onKey, true);
    document.addEventListener("pointerdown", onDown, true);
    return () => {
      document.removeEventListener("keydown", onKey, true);
      document.removeEventListener("pointerdown", onDown, true);
    };
  }, [open]);

  const close = () => {
    setOpen(false);
    buttonRef.current?.focus();
  };

  return (
    <div className="relative" ref={rootRef}>
      <button
        ref={buttonRef}
        type="button"
        className="relative flex h-8 w-8 cursor-pointer items-center justify-center rounded-md text-text/70 hover:bg-mid-gray/15 hover:text-text focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={t("tts.reader.bookmarks", { count: bookmarks.length })}
        title={t("tts.reader.bookmarks", { count: bookmarks.length })}
        onClick={() => setOpen((o) => !o)}
        data-testid="reader-bookmarks"
      >
        <BookmarkIcon
          width={16}
          height={16}
          aria-hidden="true"
          fill={bookmarks.length ? "currentColor" : "none"}
        />
      </button>
      {open && (
        <div
          ref={menuRef}
          role="menu"
          aria-label={t("tts.reader.bookmarksMenu")}
          className="absolute right-0 top-full z-40 mt-1 w-72 max-w-[calc(100vw-32px)] rounded-lg border border-mid-gray/40 bg-background py-1 shadow-lg"
          onKeyDown={(e) => {
            if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key))
              return;
            e.preventDefault();
            const items = Array.from(
              e.currentTarget.querySelectorAll<HTMLElement>(
                "[data-bookmark-item]",
              ),
            );
            const i = items.indexOf(document.activeElement as HTMLElement);
            const next =
              e.key === "Home"
                ? 0
                : e.key === "End"
                  ? items.length - 1
                  : (i + (e.key === "ArrowDown" ? 1 : -1) + items.length) %
                    items.length;
            items[next]?.focus();
          }}
        >
          <button
            type="button"
            role="menuitem"
            tabIndex={-1}
            data-bookmark-item
            className={ITEM}
            onClick={() => {
              onAdd();
              close();
            }}
            data-testid="reader-bookmark-add"
          >
            <BookmarkPlus width={14} height={14} aria-hidden="true" />
            {t("tts.reader.bookmarkAdd")}
          </button>
          {bookmarks.length > 0 && <hr className="my-1 border-mid-gray/20" />}
          {bookmarks.length === 0 && (
            <p className="px-3 py-1.5 text-xs text-text-muted">
              {t("tts.reader.bookmarksEmpty")}
            </p>
          )}
          {bookmarks.map((b) => (
            <div key={b.id} className="flex items-center" role="none">
              <button
                type="button"
                role="menuitem"
                tabIndex={-1}
                data-bookmark-item
                className={`${ITEM} min-w-0 flex-1`}
                onClick={() => {
                  onGo(b);
                  close();
                }}
                data-testid="reader-bookmark"
              >
                <BookmarkIcon
                  width={14}
                  height={14}
                  aria-hidden="true"
                  className="shrink-0"
                />
                <span className="truncate">{b.label}</span>
                {(() => {
                  const p = pageOf?.(b.offset);
                  return p === null || p === undefined ? null : (
                    <span className="ms-auto shrink-0 text-xs text-text-muted tabular-nums">
                      {t("tts.reader.bookmarkPage", { page: p + 1 })}
                    </span>
                  );
                })()}
              </button>
              <button
                type="button"
                role="menuitem"
                tabIndex={-1}
                data-bookmark-item
                className="mr-1 flex h-7 w-7 shrink-0 cursor-pointer items-center justify-center rounded text-text/60 hover:bg-mid-gray/15 hover:text-text focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
                aria-label={t("tts.reader.bookmarkRemove", { label: b.label })}
                title={t("tts.reader.bookmarkRemove", { label: b.label })}
                onClick={() => onRemove(b)}
                data-testid="reader-bookmark-remove"
              >
                <X width={14} height={14} aria-hidden="true" />
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
};
