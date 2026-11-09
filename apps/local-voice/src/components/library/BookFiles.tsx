import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { FileText, Folder, FolderOpen } from "lucide-react";
import { commands, type LibraryBook, type LibraryBookFile } from "@/bindings";
import { Button } from "../ui/Button";
import { formatBytes } from "@/lib/library/remove";
import {
  editionsOf,
  langName,
  langShort,
  primaryEdition,
} from "@/lib/library/editions";

/**
 * „Dateien im Buch-Ordner“ (lva#365 P2b-2): was zu einem Buch auf der Platte
 * liegt, als Baum. Fassungsordner heißen wie ihre Seite; die Anzeige nennt
 * dazu Sprache und „Hauptfassung“, damit die Zuordnung eindeutig ist.
 */
export const BookFiles: React.FC<{ book: LibraryBook }> = ({ book }) => {
  const { t, i18n } = useTranslation();
  const [entries, setEntries] = useState<LibraryBookFile[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    const r = await commands.libraryBookFiles(book.id);
    if (r.status === "ok") {
      setEntries(r.data);
      setError(null);
    } else {
      setEntries([]);
      setError(r.error);
    }
  }, [book.id]);

  useEffect(() => {
    void load();
  }, [load]);

  const primary = primaryEdition(book)?.id;
  const editionOf = (page: string) =>
    editionsOf(book).find((e) => e.text?.page_id === page);

  const note = (e: LibraryBookFile): string | null => {
    const parts = e.rel.split("/");
    if (parts.length === 2 && parts[0] === "fassungen") {
      const ed = editionOf(parts[1]);
      if (!ed) return t("library.bookFiles.looseEdition");
      const lang = langName(ed.lang) || langShort(ed.lang);
      return ed.id === primary
        ? t("library.bookFiles.primaryEdition", { lang })
        : t("library.bookFiles.edition", { lang });
    }
    if (e.rel === "buch.json") return t("library.bookFiles.metadata");
    if (e.ordner && e.zusammengefasst)
      return t("library.bookFiles.summary", {
        count: e.dateien,
        size: formatBytes(e.bytes, i18n.language),
      });
    return null;
  };

  const reveal = async () => {
    const r = await commands.libraryBookReveal(book.id);
    if (r.status === "error") toast.error(r.error);
  };

  return (
    <section
      className="library-files"
      aria-labelledby="lib-files"
      data-testid="library-files"
    >
      <div className="library-files__head">
        <h3 id="lib-files">{t("library.bookFiles.title")}</h3>
        <Button
          variant="secondary"
          size="sm"
          className="library-files__reveal"
          icon={<FolderOpen size={16} aria-hidden="true" />}
          onClick={() => void reveal()}
          data-testid="library-files-reveal"
        >
          {t("library.bookFiles.reveal")}
        </Button>
      </div>
      {error && (
        <p className="library-meta" role="alert">
          {t("library.bookFiles.failed", { error })}
        </p>
      )}
      {entries && entries.length === 0 && !error && (
        <p className="library-meta">{t("library.bookFiles.empty")}</p>
      )}
      {entries && entries.length > 0 && (
        <ul
          className="library-files__tree"
          aria-label={t("library.bookFiles.title")}
        >
          {entries.map((e) => {
            const name = e.rel.split("/").pop() ?? e.rel;
            const extra = note(e);
            return (
              <li
                key={e.rel}
                className="library-files__row"
                style={{ paddingInlineStart: `${e.tiefe * 18}px` }}
                data-testid="library-file"
                data-rel={e.rel}
              >
                {e.ordner ? (
                  <Folder size={16} aria-hidden="true" />
                ) : (
                  <FileText size={16} aria-hidden="true" />
                )}
                <span className="library-files__name">
                  {e.ordner ? `${name}/` : name}
                </span>
                {extra && <span className="library-files__note">{extra}</span>}
                {!e.ordner && (
                  <span className="library-files__size">
                    {formatBytes(e.bytes, i18n.language)}
                  </span>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
};
