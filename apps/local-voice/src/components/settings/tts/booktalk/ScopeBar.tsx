import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, Lock, MoreHorizontal, Trash2, TriangleAlert } from "lucide-react";
import type { BookTalkScope } from "@/bindings";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";

/**
 * Kopf der Gesprächsspalte (lva#309): Bereichsknopf „Bis S. n geschützt ▾“ mit
 * Menü, Bestätigung für „Ganzes Buch“ (gilt, bis Spalte oder Gespräch
 * schließen) und das Menü „Verlauf leeren“.
 */
export function ScopeBar({
  scope,
  page,
  onScope,
  onClear,
  canClear,
}: {
  scope: BookTalkScope;
  /** Seite an der Lesegrenze; `null`, wenn keine Seitenmarken vorliegen. */
  page: number | null;
  onScope(scope: BookTalkScope): void;
  onClear(): void;
  canClear: boolean;
}) {
  const { t } = useTranslation();
  const [menu, setMenu] = useState<"scope" | "more" | null>(null);
  const [confirm, setConfirm] = useState(false);
  const [announce, setAnnounce] = useState("");
  const root = useRef<HTMLDivElement>(null);
  const book = scope === "book";

  useEffect(() => {
    if (!menu) return;
    const close = (e: MouseEvent) => {
      if (!root.current?.contains(e.target as Node)) setMenu(null);
    };
    const esc = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      setMenu(null);
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", esc, true);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", esc, true);
    };
  }, [menu]);

  const label = book
    ? t("tts.bookTalk.scope.book")
    : page !== null
      ? t("tts.bookTalk.scope.until", { page })
      : t("tts.bookTalk.scope.untilHere");
  const aria = book
    ? t("tts.bookTalk.scope.ariaBook")
    : page !== null
      ? t("tts.bookTalk.scope.ariaUntil", { page })
      : t("tts.bookTalk.scope.ariaUntilHere");

  const choose = (next: BookTalkScope) => {
    setMenu(null);
    if (next === "book" && !book) {
      setConfirm(true);
      return;
    }
    if (next === scope) return;
    onScope(next);
    setAnnounce(t("tts.bookTalk.scope.changedUntil"));
  };

  return (
    <div className="bt__head" ref={root} data-testid="booktalk-head">
      <button
        type="button"
        className={`bt__scope${book ? " bt__scope--book" : ""}`}
        aria-haspopup="menu"
        aria-expanded={menu === "scope"}
        aria-label={aria}
        onClick={() => setMenu(menu === "scope" ? null : "scope")}
        data-testid="booktalk-scope"
      >
        {book ? (
          <TriangleAlert width={14} height={14} aria-hidden="true" />
        ) : (
          <Lock width={14} height={14} aria-hidden="true" />
        )}
        {label}
        <ChevronDown width={14} height={14} aria-hidden="true" />
      </button>
      <span className="bt__spacer" />
      <button
        type="button"
        className="bt__iconbtn"
        aria-haspopup="menu"
        aria-expanded={menu === "more"}
        aria-label={t("tts.bookTalk.menu.more")}
        title={t("tts.bookTalk.menu.more")}
        onClick={() => setMenu(menu === "more" ? null : "more")}
        data-testid="booktalk-more"
      >
        <MoreHorizontal width={16} height={16} aria-hidden="true" />
      </button>

      {menu === "scope" && (
        <div
          className="bt__menu"
          role="menu"
          aria-label={t("tts.bookTalk.scope.menuLabel")}
          data-testid="booktalk-scope-menu"
        >
          <button
            type="button"
            role="menuitemradio"
            aria-checked={!book}
            className="bt__menuitem"
            onClick={() => choose("until_position")}
            data-testid="booktalk-scope-until"
          >
            <span>{t("tts.bookTalk.scope.untilOption")}</span>
            <small>{t("tts.bookTalk.scope.untilHint")}</small>
          </button>
          <button
            type="button"
            role="menuitemradio"
            aria-checked={book}
            className="bt__menuitem"
            onClick={() => choose("book")}
            data-testid="booktalk-scope-book"
          >
            <span>{t("tts.bookTalk.scope.bookOption")}</span>
            <small>{t("tts.bookTalk.scope.bookHint")}</small>
          </button>
          <p className="bt__note" data-testid="booktalk-scope-note">
            {t("tts.bookTalk.scope.note")}
          </p>
        </div>
      )}
      {menu === "more" && (
        <div
          className="bt__menu bt__menu--end"
          role="menu"
          aria-label={t("tts.bookTalk.menu.more")}
        >
          <button
            type="button"
            role="menuitem"
            className="bt__menuitem"
            disabled={!canClear}
            onClick={() => {
              setMenu(null);
              onClear();
            }}
            data-testid="booktalk-clear"
          >
            <span className="inline-flex items-center gap-2">
              <Trash2 width={14} height={14} aria-hidden="true" />
              {t("tts.bookTalk.menu.clear")}
            </span>
          </button>
        </div>
      )}

      <span className="sr-only" role="status" aria-live="polite">
        {announce}
      </span>

      <Dialog
        open={confirm}
        onOpenChange={setConfirm}
        title={t("tts.bookTalk.scope.confirmTitle")}
        closeLabel={t("tts.bookTalk.scope.confirmClose")}
        footer={
          <>
            <Button
              variant="primary"
              onClick={() => {
                setConfirm(false);
                onScope("book");
                setAnnounce(t("tts.bookTalk.scope.changedBook"));
              }}
              data-testid="booktalk-scope-confirm"
            >
              {t("tts.bookTalk.scope.confirmOk")}
            </Button>
            <Button variant="secondary" onClick={() => setConfirm(false)}>
              {t("tts.bookTalk.scope.confirmCancel")}
            </Button>
          </>
        }
      >
        <p className="text-sm" data-testid="booktalk-scope-confirm-text">
          {t("tts.bookTalk.scope.confirmBody")}
        </p>
      </Dialog>
    </div>
  );
}
