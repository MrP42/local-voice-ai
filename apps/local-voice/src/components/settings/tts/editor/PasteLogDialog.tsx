import React from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";
import {
  pasteEntryChanges,
  restoreRaw,
  type PasteLogEntry,
} from "@/lib/reader/pasteLog";

interface PasteLogDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Einfügungen der Seite, älteste zuerst. */
  entries: readonly PasteLogEntry[];
  /** Aktueller Text des Editors (Grundlage für „wiederherstellen“). */
  text: string;
  /** Ganzen Text setzen (ein Schritt in der Seiten-Historie). */
  onRestore: (next: string) => void;
}

/**
 * „Letzte Einfügungen“: je Einfügung Zeit, Änderungen je Regel und die Knöpfe
 * „Rohfassung wiederherstellen“ (nur, wenn der bereinigte Text genau einmal
 * unverändert im Text steht; sonst gesperrt mit Grund) und „Rohfassung
 * kopieren“. Die Einträge stammen aus `state.json` der Seite und überstehen
 * deshalb einen Neustart.
 */
export const PasteLogDialog: React.FC<PasteLogDialogProps> = ({
  open,
  onOpenChange,
  entries,
  text,
  onRestore,
}) => {
  const { t, i18n } = useTranslation();
  // Geschlossen nichts berechnen: Wiederherstellbarkeit durchsucht den ganzen Text.
  const newestFirst = open ? [...entries].reverse() : [];

  const copy = (entry: PasteLogEntry) => {
    if (entry.roh === undefined) return;
    void navigator.clipboard
      ?.writeText(entry.roh)
      .then(() => toast(t("tts.editorMenu.pasteLog.copied")))
      .catch(() => undefined);
  };

  const restore = (entry: PasteLogEntry) => {
    const result = restoreRaw(text, entry);
    if (!result.ok) return;
    onRestore(result.text);
    toast(t("tts.editorMenu.pasteLog.restored"));
    onOpenChange(false);
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

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("tts.editorMenu.pasteLog.title")}
      description={t("tts.editorMenu.pasteLog.description")}
      closeLabel={t("tts.editorMenu.pasteLog.close")}
      className="max-w-2xl"
    >
      {newestFirst.length === 0 ? (
        <p className="text-sm" data-testid="paste-log-empty">
          {t("tts.editorMenu.pasteLog.empty")}
        </p>
      ) : (
        <ul
          className="flex flex-col gap-3"
          aria-label={t("tts.editorMenu.pasteLog.list")}
          data-testid="paste-log-list"
        >
          {newestFirst.map((entry, index) => {
            const result = restoreRaw(text, entry);
            const canCopy = entry.roh !== undefined;
            return (
              <li
                key={`${entry.zeit}-${entry.sha256}-${index}`}
                className="flex flex-col gap-1 rounded-md border border-mid-gray/20 p-2"
                data-testid="paste-log-entry"
              >
                <div className="flex flex-wrap items-baseline justify-between gap-x-3">
                  <time className="text-sm font-medium" dateTime={entry.zeit}>
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
                      count: entry.zeichen ?? 0,
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
                {canCopy && (
                  <div className="flex flex-wrap items-center gap-2 pt-1">
                    <Button
                      type="button"
                      size="sm"
                      variant="secondary"
                      disabled={!result.ok}
                      aria-describedby={
                        result.ok ? undefined : `paste-log-why-${index}`
                      }
                      data-testid="paste-log-restore"
                      onClick={() => restore(entry)}
                    >
                      {t("tts.editorMenu.pasteLog.restore")}
                    </Button>
                    <Button
                      type="button"
                      size="sm"
                      variant="secondary"
                      data-testid="paste-log-copy"
                      onClick={() => copy(entry)}
                    >
                      {t("tts.editorMenu.pasteLog.copy")}
                    </Button>
                  </div>
                )}
                {canCopy && !result.ok && result.reason !== "noRaw" && (
                  <p
                    id={`paste-log-why-${index}`}
                    className="text-xs text-text-muted"
                    data-testid="paste-log-reason"
                  >
                    {t(`tts.editorMenu.pasteLog.reason.${result.reason}`)}
                  </p>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </Dialog>
  );
};
