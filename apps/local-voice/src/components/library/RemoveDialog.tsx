import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { LibraryBook } from "@/bindings";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import type { RemoveScope } from "@/lib/library/remove";

/**
 * Rückfrage vor dem Entfernen (ein oder mehrere Einträge). Wahl: nur aus der
 * Bibliothek (Text bleibt im Vorlesen) oder alles samt Text und Transkript.
 * Die Originaldateien auf dem Rechner bleiben immer unberührt.
 */
export const RemoveDialog: React.FC<{
  books: LibraryBook[];
  onConfirm(scope: RemoveScope): void;
  onCancel(): void;
}> = ({ books, onConfirm, onCancel }) => {
  const { t } = useTranslation();
  const [scope, setScope] = useState<RemoveScope>("library");
  const open = books.length > 0;
  useEffect(() => {
    if (open) setScope("library");
  }, [open]);
  const withText = books.filter((b) => b.text).length;
  const title =
    books.length === 1
      ? t("library.remove.titleOne", { title: books[0].title })
      : t("library.remove.titleMany", { count: books.length });
  return (
    <Dialog
      open={open}
      onOpenChange={(o) => !o && onCancel()}
      title={title}
      closeLabel={t("library.cancel")}
      footer={
        <>
          <Button onClick={onCancel}>{t("library.cancel")}</Button>
          <Button
            variant="danger"
            onClick={() => onConfirm(scope)}
            data-testid="library-remove-confirm"
          >
            {t("library.remove.confirm", { count: books.length })}
          </Button>
        </>
      }
    >
      <div className="library-remove" data-testid="library-remove-dialog">
        {books.length > 1 && (
          <ul className="library-remove__list">
            {books.slice(0, 8).map((b) => (
              <li key={b.id}>{b.title}</li>
            ))}
            {books.length > 8 && (
              <li className="library-meta">
                {t("library.remove.more", { count: books.length - 8 })}
              </li>
            )}
          </ul>
        )}
        <fieldset className="library-remove__scope">
          <legend className="library-meta">{t("library.remove.scope")}</legend>
          <label className="library-remove__option">
            <input
              type="radio"
              name="library-remove-scope"
              checked={scope === "library"}
              onChange={() => setScope("library")}
              data-testid="library-remove-scope-library"
            />
            <span>
              <strong>{t("library.remove.library")}</strong>
              <small>
                {withText
                  ? t("library.remove.libraryHintText")
                  : t("library.remove.libraryHint")}
              </small>
            </span>
          </label>
          <label className="library-remove__option">
            <input
              type="radio"
              name="library-remove-scope"
              checked={scope === "everything"}
              onChange={() => setScope("everything")}
              data-testid="library-remove-scope-everything"
            />
            <span>
              <strong>{t("library.remove.everything")}</strong>
              <small>{t("library.remove.everythingHint")}</small>
            </span>
          </label>
        </fieldset>
        <p className="library-meta">{t("library.remove.originals")}</p>
      </div>
    </Dialog>
  );
};
