import React from "react";
import { useTranslation } from "react-i18next";
import { FileText, Library } from "lucide-react";
import type { LibraryBook } from "@/bindings";
import { BookCover } from "../../library/BookCover";
import { Select } from "../../ui/Select";
import {
  FOLDER_ALL,
  bookFolderKey,
  type PageFolders,
} from "@/lib/reader/folders";

/**
 * Ordnerleiste im Vorlesen (lva#365 P1). Seit lva#784 verwaltet die
 * Bibliothek die Bücher (wie am iPhone): hier stehen die eigenen Texte und,
 * nur solange ein Buch aus der Bibliothek offen ist, dieses eine Buch. Der
 * Weg zu allen Büchern führt in die Bibliothek.
 */
export const ReaderFolders: React.FC<{
  folders: PageFolders;
  books: LibraryBook[];
  selection: string;
  onSelect: (key: string) => void;
}> = ({ folders, books, selection, onSelect }) => {
  const { t } = useTranslation();
  const open = folders.books.find((f) => bookFolderKey(f.bookId) === selection);
  const openBook = open ? books.find((b) => b.id === open.bookId) : undefined;
  const toLibrary = () =>
    window.dispatchEvent(
      new CustomEvent("lv-navigate", { detail: { section: "library" } }),
    );

  const row = (
    key: string,
    label: string,
    count: number,
    icon: React.ReactNode,
    extra?: {
      indent?: boolean;
      trailing?: React.ReactNode;
      expanded?: boolean;
    },
  ) => (
    <button
      key={key}
      type="button"
      data-testid="reader-folder"
      data-folder={key}
      aria-current={selection === key ? "true" : undefined}
      aria-expanded={extra?.expanded}
      onClick={() => onSelect(key)}
      className={`flex w-full min-h-[44px] items-center gap-2 rounded-md text-start text-sm transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-logo-primary ${
        extra?.indent ? "ps-6 pe-2" : "px-2"
      } ${
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
      {extra?.trailing}
    </button>
  );

  // Umbruch-Layout (≤ 1100 px): die Seitenliste ist dort ein flaches Regal
  // über dem Text; statt der ganzen Ordnerleiste steht dann eine Auswahl.
  const options = [
    {
      value: FOLDER_ALL,
      label: `${t("tts.folders.texts")} (${folders.loose.length})`,
    },
    ...(open
      ? [
          {
            value: bookFolderKey(open.bookId),
            label: `${open.title} (${open.pageIds.length})`,
          },
        ]
      : []),
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
          t("tts.folders.texts"),
          folders.loose.length,
          <FileText width={16} height={16} />,
        )}
        {open &&
          row(
            bookFolderKey(open.bookId),
            open.title,
            open.pageIds.length,
            openBook ? (
              <span className="block w-5">
                <BookCover book={openBook} size="xs" />
              </span>
            ) : (
              <Library width={16} height={16} />
            ),
          )}
        <button
          type="button"
          onClick={toLibrary}
          className="flex w-full min-h-[44px] items-center gap-2 rounded-md px-2 text-start text-sm text-text/70 transition-colors hover:bg-mid-gray/15 hover:text-text focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-logo-primary"
          data-testid="reader-folders-library"
        >
          <span className="flex w-6 shrink-0 justify-center" aria-hidden="true">
            <Library width={16} height={16} />
          </span>
          <span className="min-w-0 flex-1 truncate">
            {t("tts.folders.booksInLibrary")}
          </span>
        </button>
      </nav>
    </>
  );
};
