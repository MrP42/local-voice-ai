import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { RotateCcw, Trash2 } from "lucide-react";
import type { TrashEntry } from "@/bindings";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { BookCover } from "./BookCover";
import { formatBytes } from "@/lib/library/remove";

/**
 * Papierkorb der Bibliothek (Codex-Abnahme lva#221): dauerhaft erreichbar,
 * je Eintrag Wiederherstellen oder endgültig löschen; endgültiges Löschen
 * braucht einen zweiten Klick mit klarer Benennung.
 */
export const TrashDialog: React.FC<{
  open: boolean;
  entries: TrashEntry[];
  busy: boolean;
  onRestore(entries: TrashEntry[]): void;
  onPurge(entries: TrashEntry[]): void;
  onClose(): void;
}> = ({ open, entries, busy, onRestore, onPurge, onClose }) => {
  const { t, i18n } = useTranslation();
  /** Kennung des Eintrags, dessen Löschen bestätigt wird; `*` = alle. */
  const [confirm, setConfirm] = useState<string | null>(null);
  const total = entries.reduce((a, e) => a + e.bytes, 0);
  const date = (ms: number) =>
    new Date(ms).toLocaleDateString(i18n.language, {
      day: "numeric",
      month: "short",
      hour: "2-digit",
      minute: "2-digit",
    });
  const close = () => {
    setConfirm(null);
    onClose();
  };
  return (
    <Dialog
      open={open}
      onOpenChange={(o) => !o && !busy && close()}
      title={t("library.trash.title", { count: entries.length })}
      closeLabel={t("library.trash.close")}
      className="library-dialog-touch"
      contentClassName="library-detail"
      footer={
        confirm === "*" ? (
          <>
            <Button onClick={() => setConfirm(null)} disabled={busy}>
              {t("library.cancel")}
            </Button>
            <Button
              variant="danger"
              disabled={busy}
              onClick={() => {
                setConfirm(null);
                onPurge(entries);
              }}
              data-testid="library-trash-empty-confirm"
            >
              {t("library.trash.emptyConfirm", {
                count: entries.length,
                size: formatBytes(total, i18n.language),
              })}
            </Button>
          </>
        ) : (
          <>
            {entries.length > 0 && (
              <Button
                variant="danger-ghost"
                icon={<Trash2 size={16} aria-hidden="true" />}
                disabled={busy}
                onClick={() => setConfirm("*")}
                data-testid="library-trash-empty"
              >
                {t("library.trash.empty")}
              </Button>
            )}
            <Button onClick={close} disabled={busy}>
              {t("library.trash.close")}
            </Button>
          </>
        )
      }
    >
      <div className="library-remove" data-testid="library-trash-dialog">
        <p className="library-meta">
          {t("library.trash.hint", {
            size: formatBytes(total, i18n.language),
          })}
        </p>
        {confirm === "*" && (
          <p className="library-trash__warn" role="alert">
            {t("library.trash.emptyWarn", {
              count: entries.length,
              size: formatBytes(total, i18n.language),
            })}
          </p>
        )}
        {entries.length === 0 ? (
          <p data-testid="library-trash-none">{t("library.trash.none")}</p>
        ) : (
          <ul className="library-trash__list">
            {entries.map((e) => (
              <li key={e.book.id} data-testid="library-trash-item">
                <BookCover book={e.book} size="xs" />
                <div className="library-trash__text">
                  <strong>{e.book.title}</strong>
                  <span className="library-meta">
                    {t("library.trash.meta", {
                      date: date(e.removed_ms),
                      size: formatBytes(e.bytes, i18n.language),
                    })}
                  </span>
                  {confirm === e.book.id && (
                    <span className="library-trash__warn" role="alert">
                      {t("library.trash.deleteWarn", {
                        size: formatBytes(e.bytes, i18n.language),
                      })}
                    </span>
                  )}
                </div>
                <div className="library-trash__actions">
                  {confirm === e.book.id ? (
                    <>
                      <Button
                        key="delete-cancel"
                        onClick={() => setConfirm(null)}
                        disabled={busy}
                      >
                        {t("library.cancel")}
                      </Button>
                      <Button
                        key="delete-confirm"
                        variant="danger"
                        disabled={busy}
                        onClick={() => {
                          setConfirm(null);
                          onPurge([e]);
                        }}
                        data-testid="library-trash-delete-confirm"
                      >
                        {t("library.trash.deleteConfirm")}
                      </Button>
                    </>
                  ) : (
                    <>
                      <Button
                        icon={<RotateCcw size={16} aria-hidden="true" />}
                        disabled={busy}
                        onClick={() => onRestore([e])}
                        data-testid="library-trash-restore"
                      >
                        {t("library.trash.restore")}
                      </Button>
                      <Button
                        variant="danger-ghost"
                        disabled={busy}
                        onClick={() => setConfirm(e.book.id)}
                        aria-label={t("library.trash.deleteOne", {
                          title: e.book.title,
                        })}
                        data-testid="library-trash-delete"
                      >
                        {t("library.trash.delete")}
                      </Button>
                    </>
                  )}
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>
    </Dialog>
  );
};
