import React from "react";
import { useTranslation } from "react-i18next";
import type { LibraryBook } from "@/bindings";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { cleanTitle } from "@/lib/library/names";

/**
 * Titel bereinigen: zeigt alt → neu für jeden Eintrag mit Kennungen oder
 * Formatangaben im Titel (ASIN, ISBN, „LC 128 44100 Stereo“).
 */
export const CleanTitlesDialog: React.FC<{
  books: LibraryBook[];
  busy: boolean;
  onConfirm(): void;
  onCancel(): void;
}> = ({ books, busy, onConfirm, onCancel }) => {
  const { t } = useTranslation();
  return (
    <Dialog
      open={books.length > 0}
      onOpenChange={(o) => !o && !busy && onCancel()}
      title={t("library.clean.title", { count: books.length })}
      closeLabel={t("library.cancel")}
      footer={
        <>
          <Button onClick={onCancel} disabled={busy}>
            {t("library.cancel")}
          </Button>
          <Button
            variant="primary"
            onClick={onConfirm}
            disabled={busy}
            data-testid="library-clean-confirm"
          >
            {t("library.clean.confirm", { count: books.length })}
          </Button>
        </>
      }
    >
      <div className="library-remove" data-testid="library-clean-dialog">
        <p className="library-meta">{t("library.clean.hint")}</p>
        <ul className="library-clean__list">
          {books.map((b) => (
            <li key={b.id}>
              <span className="library-clean__old">{b.title}</span>
              <span data-testid="library-clean-new">{cleanTitle(b.title)}</span>
            </li>
          ))}
        </ul>
      </div>
    </Dialog>
  );
};
