import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ListTree, MessageSquare, NotebookPen } from "lucide-react";
import { Dialog } from "../../../ui/Dialog";
import "./kidocs.css";

export type BookSheet = "contents" | "notes" | "chat";
type Sheet = BookSheet;

/** Bis zu dieser Breite zeigt der Leser die Blätter statt der Seitenspalten (siehe `kidocs.css`). */
export const BOOK_SHEETS_QUERY = "(max-width: 640px)";

/**
 * Mobil (lva#183, Codex Must-fix 4): Inhalt, Notizen und Chat als Blätter von
 * unten. Die Leiste erscheint nur im schmalen Fenster; auf dem Desktop liegen
 * diese Bereiche in den Seitenspalten. Das Blatt „Gespräch“ zeigt die
 * Gesprächsspalte mit dem Buch (lva#309); Notizen (B5, #186) zeigen bis dahin,
 * was dort kommt.
 */
export function BookSheets({
  renderContents,
  renderChat,
  open: controlledOpen,
  onOpenChange,
}: {
  /** Inhaltsverzeichnis; `close` schließt das Blatt nach dem Sprung. */
  renderContents: (close: () => void) => React.ReactNode;
  /**
   * Gesprächsspalte; ohne sie (kein lesbares Buch) bleibt das Blatt leer.
   * `close` schließt das Blatt (nach „Hinspringen“ soll der Text sichtbar sein).
   */
  renderChat?: (close: () => void) => React.ReactNode;
  /** Gesteuert von außen (z. B. Taste G öffnet „Gespräch“); ohne bleibt der Zustand hier. */
  open?: Sheet | null;
  onOpenChange?: (open: Sheet | null) => void;
}) {
  const { t } = useTranslation();
  const [own, setOwn] = useState<Sheet | null>(null);
  const open = controlledOpen === undefined ? own : controlledOpen;
  const setOpen = (next: Sheet | null) => {
    setOwn(next);
    onOpenChange?.(next);
  };
  const items: { id: Sheet; label: string; icon: typeof ListTree }[] = [
    { id: "contents", label: t("tts.kiDocs.sheets.contents"), icon: ListTree },
    { id: "notes", label: t("tts.kiDocs.sheets.notes"), icon: NotebookPen },
    { id: "chat", label: t("tts.kiDocs.sheets.chat"), icon: MessageSquare },
  ];
  const current = items.find((i) => i.id === open);
  return (
    <>
      <nav
        className="book-sheets"
        aria-label={t("tts.kiDocs.sheets.label")}
        data-testid="book-sheets"
      >
        {items.map(({ id, label, icon: Icon }) => (
          <button
            key={id}
            type="button"
            className="book-sheets__btn"
            aria-haspopup="dialog"
            aria-expanded={open === id}
            onClick={() => setOpen(id)}
            data-testid={`book-sheet-${id}`}
          >
            <Icon className="h-4 w-4" aria-hidden="true" />
            {label}
          </button>
        ))}
      </nav>
      <Dialog
        open={open !== null}
        onOpenChange={(o) => !o && setOpen(null)}
        title={current?.label ?? ""}
        closeLabel={t("tts.kiDocs.sheets.close")}
        className="kidoc-sheet"
      >
        <div data-testid={open ? `book-sheet-panel-${open}` : undefined}>
          {open === "contents" &&
            (renderContents(() => setOpen(null)) ?? (
              <p className="kidoc-new__desc">
                {t("tts.kiDocs.sheets.noContents")}
              </p>
            ))}
          {open === "notes" && (
            <p className="text-sm">{t("tts.kiDocs.sheets.notesSoon")}</p>
          )}
          {open === "chat" && (renderChat?.(() => setOpen(null)) ?? null)}
        </div>
      </Dialog>
    </>
  );
}
