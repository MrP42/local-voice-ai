import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { LibraryBook } from "@/bindings";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { formatBytes, sizesOf, type RemoveScope } from "@/lib/library/remove";

/**
 * Rückfrage vor dem Entfernen (ein oder mehrere Einträge), Codex-Abnahme
 * lva#221: Papierkorb (wiederherstellbar) oder endgültig löschen, mit Anzahl,
 * Hörbuch-Umfang und Texten. Endgültiges Löschen braucht eine eigene
 * Bestätigung. Die Originaldateien auf dem Rechner bleiben immer unberührt.
 */
export const RemoveDialog: React.FC<{
  books: LibraryBook[];
  onConfirm(scope: RemoveScope): void;
  onCancel(): void;
}> = ({ books, onConfirm, onCancel }) => {
  const { t } = useTranslation();
  const [scope, setScope] = useState<RemoveScope>("trash");
  const [sure, setSure] = useState(false);
  const [bytes, setBytes] = useState<number | null>(null);
  const open = books.length > 0;
  const key = books.map((b) => b.id).join(",");
  useEffect(() => {
    if (!open) return;
    setScope("trash");
    setSure(false);
    setBytes(null);
    let live = true;
    void sizesOf(books).then((n) => live && setBytes(n));
    return () => {
      live = false;
    };
    // books ändert sich nur mit key
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, key]);
  const count = books.length;
  const audio = books.filter((b) => b.audio).length;
  const texts = books.filter((b) => b.text).length;
  const size = bytes === null ? "…" : formatBytes(bytes);
  const title =
    count === 1
      ? t("library.remove.titleOne", { title: books[0].title })
      : t("library.remove.titleMany", { count });
  const deleting = scope === "delete";
  return (
    <Dialog
      open={open}
      onOpenChange={(o) => !o && onCancel()}
      title={title}
      closeLabel={t("library.cancel")}
      className="library-dialog-touch"
      footer={
        <>
          <Button onClick={onCancel}>{t("library.cancel")}</Button>
          <Button
            variant={deleting ? "danger" : "primary"}
            disabled={deleting && !sure}
            onClick={() => onConfirm(scope)}
            data-testid="library-remove-confirm"
          >
            {deleting
              ? t("library.remove.confirmDelete", { count })
              : t("library.remove.confirmTrash", { count })}
          </Button>
        </>
      }
    >
      <div className="library-remove" data-testid="library-remove-dialog">
        <p data-testid="library-remove-summary">
          {t("library.remove.summary", { count, audio, texts, size })}
        </p>
        {count > 1 && (
          <ul className="library-remove__list">
            {books.slice(0, 8).map((b) => (
              <li key={b.id}>{b.title}</li>
            ))}
            {count > 8 && (
              <li className="library-meta">
                {t("library.remove.more", { count: count - 8 })}
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
              checked={scope === "trash"}
              onChange={() => setScope("trash")}
              data-testid="library-remove-scope-trash"
            />
            <span>
              <strong>{t("library.remove.trash")}</strong>
              <small>{t("library.remove.trashHint", { size })}</small>
            </span>
          </label>
          <label className="library-remove__option library-remove__option--danger">
            <input
              type="radio"
              name="library-remove-scope"
              checked={deleting}
              onChange={() => setScope("delete")}
              data-testid="library-remove-scope-delete"
            />
            <span>
              <strong>{t("library.remove.delete")}</strong>
              <small>{t("library.remove.deleteHint", { size, texts })}</small>
            </span>
          </label>
        </fieldset>
        {deleting && (
          <label
            className="library-remove__option library-remove__sure"
            data-testid="library-remove-sure"
          >
            <input
              type="checkbox"
              checked={sure}
              onChange={(e) => setSure(e.target.checked)}
            />
            <span>
              <strong>
                {t("library.remove.sure", { count, size, texts })}
              </strong>
            </span>
          </label>
        )}
        <p className="library-meta">{t("library.remove.originals")}</p>
      </div>
    </Dialog>
  );
};
