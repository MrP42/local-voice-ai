import React from "react";
import { useTranslation } from "react-i18next";
import { FileText, List, Plus } from "lucide-react";
import type { LibraryBook } from "@/bindings";
import { BookCover } from "../../library/BookCover";
import { Button } from "../../ui/Button";
import { Select } from "../../ui/Select";
import {
  FOLDER_ALL,
  FOLDER_TEXTS,
  bookFolderKey,
  type PageFolders,
} from "@/lib/reader/folders";

/**
 * Ordnerleiste im Vorlesen (lva#365 P1), nach dem Muster der Projekte in
 * „Aufnahmen“: Alle, je Buch ein Ordner, lose Texte. Die Auswahl filtert die
 * Seitenliste darunter; ein Buch-Ordner zeigt alle Seiten seiner Fassungen.
 */
export const ReaderFolders: React.FC<{
  folders: PageFolders;
  books: LibraryBook[];
  total: number;
  selection: string;
  onSelect: (key: string) => void;
  /** „Buch hinzufügen“ im Leerzustand (Dokument-Import des Vorlesens). */
  onAddBook?: () => void;
}> = ({ folders, books, total, selection, onSelect, onAddBook }) => {
  const { t } = useTranslation();
  const byId = new Map(books.map((b) => [b.id, b]));

  const row = (
    key: string,
    label: string,
    count: number,
    icon: React.ReactNode,
  ) => (
    <button
      key={key}
      type="button"
      data-testid="reader-folder"
      data-folder={key}
      aria-current={selection === key ? "true" : undefined}
      onClick={() => onSelect(key)}
      className={`flex w-full min-h-[44px] items-center gap-2 rounded-md px-2 text-start text-sm transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-logo-primary ${
        selection === key
          ? "bg-logo-primary/15 font-semibold text-text"
          : "text-text/80 hover:bg-mid-gray/15 hover:text-text"
      }`}
    >
      <span className="flex w-6 shrink-0 justify-center" aria-hidden="true">
        {icon}
      </span>
      <span className="min-w-0 flex-1 truncate">{label}</span>
      <span className="text-xs text-text/60">{count}</span>
    </button>
  );

  // Umbruch-Layout (≤ 1100 px): die Seitenliste ist dort ein flaches Regal
  // über dem Text; statt der ganzen Ordnerleiste steht dann eine Auswahl.
  const options = [
    { value: FOLDER_ALL, label: `${t("tts.folders.all")} (${total})` },
    ...folders.books.map((f) => ({
      value: bookFolderKey(f.bookId),
      label: `${f.title} (${f.pageIds.length})`,
    })),
    {
      value: FOLDER_TEXTS,
      label: `${t("tts.folders.withoutBook")} (${folders.loose.length})`,
    },
  ];
  const compact = (
    <div
      className="reader-folders__compact items-center gap-2 pb-1 text-sm"
      data-testid="reader-folders-compact"
    >
      <span className="shrink-0 text-xs font-semibold text-text/60">
        {t("tts.folders.label")}
      </span>
      <Select
        value={selection}
        options={options}
        onChange={(value) => value && onSelect(value)}
        ariaLabel={t("tts.folders.label")}
        className="min-w-0 flex-1"
        menuPortal
        isClearable={false}
      />
    </div>
  );

  return (
    <>
      {compact}
      <nav
        aria-label={t("tts.folders.label")}
        data-testid="reader-folders"
        className="reader-folders__full space-y-0.5 border-b border-mid-gray/20 pb-2 mb-1"
      >
        {row(
          FOLDER_ALL,
          t("tts.folders.all"),
          total,
          <List width={16} height={16} />,
        )}
        <p className="px-2 pt-2 pb-0.5 text-xs font-semibold text-text/60">
          {t("tts.folders.books")}
        </p>
        {folders.books.length === 0 ? (
          <div className="space-y-2 px-2 py-1" data-testid="reader-folders-empty">
            <p className="text-sm font-semibold text-text">
              {t("tts.folders.noBooks")}
            </p>
            <p className="text-xs text-text/70">
              {t("tts.folders.noBooksHint")}
            </p>
            {onAddBook && (
              <Button
                variant="primary"
                className="reader-folders__btn"
                icon={<Plus width={16} height={16} aria-hidden="true" />}
                onClick={onAddBook}
                data-testid="reader-add-book"
              >
                {t("tts.folders.addBook")}
              </Button>
            )}
          </div>
        ) : (
          folders.books.map((f) => {
            const book = byId.get(f.bookId);
            return row(
              bookFolderKey(f.bookId),
              f.title,
              f.pageIds.length,
              book ? (
                <span className="block w-5">
                  <BookCover book={book} size="xs" />
                </span>
              ) : (
                <FileText width={16} height={16} />
              ),
            );
          })
        )}
        <p className="px-2 pt-2 pb-0.5 text-xs font-semibold text-text/60">
          {t("tts.folders.texts")}
        </p>
        {row(
          FOLDER_TEXTS,
          t("tts.folders.withoutBook"),
          folders.loose.length,
          <FileText width={16} height={16} />,
        )}
      </nav>
    </>
  );
};
