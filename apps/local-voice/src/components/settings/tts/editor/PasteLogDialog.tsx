import React, { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";
import {
  pasteEntryChanges,
  pasteEntryKey,
  removePasteEntry,
  restoreRaw,
  type PasteLogEntry,
} from "@/lib/reader/pasteLog";

interface PasteLogDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Einfügungen der Seite, älteste zuerst. */
  entries: readonly PasteLogEntry[];
  /** Prüfsummen werden noch gegen die Rohfassungen gehalten: Wiederherstellen
   *  und Kopieren sind bis dahin gesperrt. */
  checking?: boolean;
  /** Aktueller Text des Editors (Grundlage für „wiederherstellen“). */
  text: string;
  /** Ganzen Text setzen (ein Schritt in der Seiten-Historie). */
  onRestore: (next: string) => void;
  /** Protokoll der Seite ändern (gesichert, nacheinander mit anderen Schreibvorgängen). */
  onChange: (
    change: (log: PasteLogEntry[]) => PasteLogEntry[],
  ) => Promise<boolean>;
}

const MEGA = 1_000_000;

/**
 * „Letzte Einfügungen“: je Einfügung Zeit, Änderungen je Regel und die Knöpfe
 * „Rohfassung wiederherstellen“ (nur, wenn der bereinigte Text genau einmal
 * unverändert im Text steht und der Eintrag unbeschädigt ist; sonst gesperrt,
 * aber per Tab erreichbar, mit Grund), „Rohfassung kopieren“ und „Löschen“.
 * „Alle Rohfassungen löschen“ fragt nach. Die Einträge stammen aus `state.json`
 * der Seite und überstehen deshalb einen Neustart.
 */
export const PasteLogDialog: React.FC<PasteLogDialogProps> = ({
  open,
  onOpenChange,
  entries,
  checking = false,
  text,
  onRestore,
  onChange,
}) => {
  const { t, i18n } = useTranslation();
  const [confirmAll, setConfirmAll] = useState(false);
  const listRef = useRef<HTMLUListElement>(null);
  const deleteAllRef = useRef<HTMLButtonElement>(null);
  // Geschlossen nichts berechnen: Wiederherstellbarkeit durchsucht den ganzen Text.
  const newestFirst = open ? [...entries].reverse() : [];

  const copy = (entry: PasteLogEntry) => {
    if (entry.roh === undefined || checking) return;
    void navigator.clipboard
      ?.writeText(entry.roh)
      .then(() => toast(t("tts.editorMenu.pasteLog.copied")))
      .catch(() => undefined);
  };

  const restore = (entry: PasteLogEntry) => {
    const result = restoreRaw(text, entry);
    if (checking || !result.ok) return;
    onRestore(result.text);
    toast(t("tts.editorMenu.pasteLog.restored"));
    onOpenChange(false);
  };

  /** Nach dem Löschen bleibt der Fokus im Dialog (der Knopf ist weg). */
  const refocus = () =>
    window.requestAnimationFrame(() => {
      const next =
        listRef.current?.querySelector<HTMLElement>(
          '[data-testid="paste-log-delete"]',
        ) ?? deleteAllRef.current;
      next?.focus();
    });

  const remove = (entry: PasteLogEntry) => {
    const key = pasteEntryKey(entry);
    void onChange((log) => removePasteEntry(log, key)).then((ok) => {
      if (ok) toast(t("tts.editorMenu.pasteLog.deleted"));
      refocus();
    });
  };

  const removeAll = () => {
    setConfirmAll(false);
    void onChange(() => []).then((ok) => {
      if (ok) toast(t("tts.editorMenu.pasteLog.allDeleted"));
    });
  };

  const when = (iso: string) => {
    const date = new Date(iso);
    return Number.isNaN(date.getTime())
      ? iso
      : date.toLocaleString(i18n.language, {
          dateStyle: "short",
          timeStyle: "medium",
        });
  };

  const megabytes = (entry: PasteLogEntry) =>
    new Intl.NumberFormat(i18n.language, {
      maximumFractionDigits: 1,
    }).format((entry.bytes ?? entry.zeichen ?? 0) / MEGA);

  return (
    <>
      <Dialog
        open={open}
        onOpenChange={onOpenChange}
        title={t("tts.editorMenu.pasteLog.title")}
        description={t("tts.editorMenu.pasteKeep")}
        closeLabel={t("tts.editorMenu.pasteLog.close")}
        className="max-w-2xl"
        contentClassName="max-h-[60vh] overflow-y-auto"
        footer={
          <div className="flex w-full flex-wrap justify-between gap-2">
            <Button
              ref={deleteAllRef}
              type="button"
              variant="secondary"
              disabled={newestFirst.length === 0}
              data-testid="paste-log-delete-all"
              onClick={() => setConfirmAll(true)}
            >
              {t("tts.editorMenu.pasteLog.deleteAll")}
            </Button>
            <Button
              type="button"
              variant="secondary"
              onClick={() => onOpenChange(false)}
            >
              {t("tts.editorMenu.pasteLog.close")}
            </Button>
          </div>
        }
      >
        {newestFirst.length === 0 ? (
          <p className="text-sm" data-testid="paste-log-empty">
            {t("tts.editorMenu.pasteLog.empty")}
          </p>
        ) : (
          <ul
            ref={listRef}
            className="flex flex-col gap-3"
            aria-label={t("tts.editorMenu.pasteLog.list")}
            data-testid="paste-log-list"
          >
            {newestFirst.map((entry, index) => {
              const result = restoreRaw(text, entry);
              const canCopy = entry.roh !== undefined;
              const timeId = `paste-log-time-${index}`;
              const whyId = `paste-log-why-${index}`;
              const why = !canCopy
                ? null
                : checking
                  ? "checking"
                  : !result.ok && result.reason !== "noRaw"
                    ? result.reason
                    : null;
              return (
                <li
                  key={pasteEntryKey(entry) + index}
                  className="flex flex-col gap-1 rounded-md border border-mid-gray/20 p-2"
                  data-testid="paste-log-entry"
                >
                  <div className="flex flex-wrap items-baseline justify-between gap-x-3">
                    <time
                      id={timeId}
                      className="text-sm font-medium"
                      dateTime={entry.zeit}
                    >
                      {when(entry.zeit)}
                    </time>
                    {!entry.zuGross && (
                      <span className="text-xs text-text-muted">
                        {t("tts.editorMenu.pasteLog.changes", {
                          count: pasteEntryChanges(entry),
                        })}
                      </span>
                    )}
                  </div>
                  {entry.zuGross ? (
                    <p className="text-sm text-text-muted">
                      {t("tts.editorMenu.pasteLog.tooLarge", {
                        size: megabytes(entry),
                      })}
                    </p>
                  ) : (
                    <ul className="flex flex-wrap gap-x-3 text-xs text-text-muted">
                      {entry.regeln.map((r) => (
                        <li key={r.rule}>
                          {t(`tts.editorMenu.pasteLog.rules.${r.rule}`)}:{" "}
                          {r.count}
                        </li>
                      ))}
                    </ul>
                  )}
                  <div className="flex flex-wrap items-center gap-2 pt-1">
                    {canCopy && (
                      <>
                        <Button
                          type="button"
                          size="sm"
                          variant="secondary"
                          disabledFocusable={checking || !result.ok}
                          aria-describedby={why ? `${whyId} ${timeId}` : timeId}
                          data-testid="paste-log-restore"
                          onClick={() => restore(entry)}
                        >
                          {t("tts.editorMenu.pasteLog.restore")}
                        </Button>
                        <Button
                          type="button"
                          size="sm"
                          variant="secondary"
                          disabledFocusable={checking}
                          aria-describedby={
                            checking ? `${whyId} ${timeId}` : timeId
                          }
                          data-testid="paste-log-copy"
                          onClick={() => copy(entry)}
                        >
                          {t("tts.editorMenu.pasteLog.copy")}
                        </Button>
                      </>
                    )}
                    <Button
                      type="button"
                      size="sm"
                      variant="secondary"
                      aria-describedby={timeId}
                      data-testid="paste-log-delete"
                      onClick={() => remove(entry)}
                    >
                      {t("tts.editorMenu.pasteLog.delete")}
                    </Button>
                  </div>
                  {why && (
                    <p
                      id={whyId}
                      className="text-xs text-text-muted"
                      data-testid="paste-log-reason"
                    >
                      {t(`tts.editorMenu.pasteLog.reason.${why}`)}
                    </p>
                  )}
                </li>
              );
            })}
          </ul>
        )}
      </Dialog>
      <Dialog
        open={open && confirmAll}
        onOpenChange={setConfirmAll}
        title={t("tts.editorMenu.pasteLog.deleteAllTitle")}
        description={t("tts.editorMenu.pasteLog.deleteAllBody")}
        closeLabel={t("tts.editorMenu.pasteLog.deleteAllNo")}
        className="max-w-md"
        footer={
          <>
            <Button
              type="button"
              variant="danger"
              data-testid="paste-log-delete-all-yes"
              onClick={removeAll}
            >
              {t("tts.editorMenu.pasteLog.deleteAllYes")}
            </Button>
            <Button
              type="button"
              variant="secondary"
              onClick={() => setConfirmAll(false)}
            >
              {t("tts.editorMenu.pasteLog.deleteAllNo")}
            </Button>
          </>
        }
      >
        {null}
      </Dialog>
    </>
  );
};
