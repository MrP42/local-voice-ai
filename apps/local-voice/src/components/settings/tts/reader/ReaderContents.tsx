import React from "react";
import { useTranslation } from "react-i18next";
import { ScanBarcode, ChevronDown, ChevronRight } from "lucide-react";
import { usePersistentState } from "../../../../hooks/usePersistentState";
import type { BookInfo, ContentsEntry } from "../../../../lib/reader/contents";

interface ReaderContentsProps {
  entries: ContentsEntry[];
  /** Index des aktuellen Kapitels (-1: vor der ersten Überschrift). */
  current: number;
  onGo(entry: ContentsEntry): void;
  /** Seite (0-basiert) je Eintrag, nach dem Umbruch gemessen. */
  pages?: (number | null)[];
  /** Titel und Autor eines importierten Buchs, über dem Verzeichnis. */
  book?: BookInfo | null;
  /** Buchdaten per ISBN suchen (nur im Originaltext, lva#149). */
  onIsbn?: () => void;
}

/**
 * Inhaltsverzeichnis der offenen Seite in der linken Leiste (lva#148/#159):
 * die Überschriften des Texts, das aktuelle Kapitel hervorgehoben.
 */
export const ReaderContents: React.FC<ReaderContentsProps> = ({
  entries,
  current,
  onGo,
  pages,
  book,
  onIsbn,
}) => {
  const { t } = useTranslation();
  // Einklappbar: im schmalen Fenster steht die Leiste über dem Buch.
  const [openStored, setOpen] = usePersistentState<"1" | "0">(
    "tts.reader.contentsOpen",
    "1",
    (v) => v === "1" || v === "0",
  );
  const open = openStored === "1";
  const listId = React.useId();
  if (entries.length === 0 && !book && !onIsbn) return null;
  return (
    <nav
      className="mt-3 border-t border-mid-gray/20 pt-2"
      aria-label={t("tts.reader.contents")}
      data-testid="reader-contents"
    >
      {book && (
        <div className="mb-2 px-2" data-testid="reader-bookinfo">
          <p
            className="line-clamp-2 text-sm font-semibold text-text"
            title={book.title}
          >
            {book.title}
          </p>
          <p className="truncate text-xs text-text/70" title={book.authors}>
            {book.authors}
          </p>
        </div>
      )}
      {onIsbn && (
        <button
          type="button"
          onClick={onIsbn}
          title={t("tts.isbn.buttonHint")}
          className="mb-2 flex w-full cursor-pointer items-center gap-1.5 rounded-md px-2 py-1 text-start text-xs text-text/70 hover:bg-mid-gray/15 hover:text-text focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
          data-testid="reader-isbn"
        >
          <ScanBarcode width={14} height={14} aria-hidden="true" />
          <span className="min-w-0 truncate">{t("tts.isbn.button")}</span>
        </button>
      )}
      {entries.length > 0 && (
        <>
          <button
            type="button"
            className="flex w-full cursor-pointer items-center gap-1 rounded pb-1 text-start text-xs font-semibold text-text/60 hover:text-text focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
            aria-expanded={open}
            aria-controls={listId}
            onClick={() => setOpen(open ? "0" : "1")}
            data-testid="reader-contents-toggle"
          >
            {open ? (
              <ChevronDown width={14} height={14} aria-hidden="true" />
            ) : (
              <ChevronRight width={14} height={14} aria-hidden="true" />
            )}
            <span className="flex-1">{t("tts.reader.contents")}</span>
            {!open && current >= 0 && (
              <span className="min-w-0 truncate font-normal">
                {entries[current].title}
              </span>
            )}
          </button>
          <ol id={listId} className="space-y-0.5" hidden={!open}>
            {entries.map((e, i) => {
              const page = pages?.[i];
              const active = i === current;
              return (
                <li key={`${e.start}-${i}`}>
                  <button
                    type="button"
                    onClick={() => onGo(e)}
                    aria-current={active ? "location" : undefined}
                    className={`flex w-full cursor-pointer items-baseline gap-2 rounded-md py-1 pe-2 text-start text-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary ${
                      active
                        ? "bg-logo-primary/15 font-medium text-text"
                        : "text-text/75 hover:bg-mid-gray/15 hover:text-text"
                    }`}
                    style={{
                      paddingInlineStart: `${8 + (e.level - 1) * 12}px`,
                    }}
                    data-testid="reader-contents-entry"
                  >
                    <span className="min-w-0 flex-1 truncate" title={e.title}>
                      {e.title}
                    </span>
                    {page !== null && page !== undefined && (
                      <span className="shrink-0 text-xs tabular-nums text-text/60">
                        {page + 1}
                      </span>
                    )}
                  </button>
                </li>
              );
            })}
          </ol>
        </>
      )}
    </nav>
  );
};
