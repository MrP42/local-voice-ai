import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ChevronDown,
  Cloud,
  Lock,
  MoreHorizontal,
  Sparkles,
  Trash2,
  TriangleAlert,
} from "lucide-react";
import type { BookTalkScope } from "@/bindings";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";

/**
 * Kopf der Gesprächsspalte (lva#309): Bereichsknopf „Bis S. n geschützt ▾“ mit
 * Menü, Bestätigung für „Ganzes Buch“ (gilt, bis Spalte oder Gespräch
 * schließen) und das Menü „Weitere Aktionen“: Verlauf leeren, Schalter „KI-
 * Antworten (Vorschau)“ (Plan §11.5) und Widerruf der Cloud-Freigabe (§11.4).
 *
 * Tastatur der Menüs: Fokus geht beim Öffnen hinein und beim Schließen zum
 * Knopf zurück, Pfeiltasten wechseln den Eintrag, Pos1/Ende springen, Esc schließt.
 */
export function ScopeBar({
  scope,
  page,
  onScope,
  onClear,
  canClear,
  preview,
  onPreview,
  cloud,
  onRevoke,
}: {
  scope: BookTalkScope;
  /** Seite an der Lesegrenze; `null`, wenn keine Seitenmarken vorliegen. */
  page: number | null;
  onScope(scope: BookTalkScope): void;
  onClear(): void;
  canClear: boolean;
  /** Schalter „KI-Antworten (Vorschau)“ (`book_talk_enabled`). */
  preview: boolean;
  onPreview(on: boolean): void;
  /** Cloud-Modell mit Freigabe (Name), sonst `null`: dann entfällt der Menüpunkt. */
  cloud: string | null;
  onRevoke(): void;
}) {
  const { t } = useTranslation();
  const [menu, setMenu] = useState<"scope" | "more" | null>(null);
  const [confirm, setConfirm] = useState(false);
  const [announce, setAnnounce] = useState("");
  const root = useRef<HTMLDivElement>(null);
  const scopeBtn = useRef<HTMLButtonElement>(null);
  const moreBtn = useRef<HTMLButtonElement>(null);
  const menuEl = useRef<HTMLDivElement>(null);
  const giveFocusBack = useRef(false);
  const book = scope === "book";

  const items = () =>
    Array.from(
      menuEl.current?.querySelectorAll<HTMLElement>(
        '[role^="menuitem"]:not(:disabled)',
      ) ?? [],
    );

  // Menü auf: der Fokus geht auf den gewählten (sonst ersten) Eintrag; Menü zu:
  // zurück zum Knopf, wenn die Tastatur es geschlossen hat.
  const openedFor = useRef<"scope" | "more" | null>(null);
  useEffect(() => {
    if (menu) {
      openedFor.current = menu;
      const list = items();
      (
        list.find((el) => el.getAttribute("aria-checked") === "true") ?? list[0]
      )?.focus();
      return;
    }
    const was = openedFor.current;
    openedFor.current = null;
    if (!was || !giveFocusBack.current) return;
    giveFocusBack.current = false;
    (was === "scope" ? scopeBtn : moreBtn).current?.focus();
  }, [menu]);

  const closeMenu = (backToButton: boolean) => {
    giveFocusBack.current = backToButton;
    setMenu(null);
  };

  const onMenuKey = (e: React.KeyboardEvent) => {
    const list = items();
    const at = list.indexOf(document.activeElement as HTMLElement);
    const go = (i: number) => {
      e.preventDefault();
      list[(i + list.length) % list.length]?.focus();
    };
    if (e.key === "ArrowDown") go(at + 1);
    else if (e.key === "ArrowUp") go(at < 0 ? list.length - 1 : at - 1);
    else if (e.key === "Home") go(0);
    else if (e.key === "End") go(list.length - 1);
    else if (e.key === "Tab") closeMenu(false);
  };
  const onTriggerKey = (kind: "scope" | "more") => (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown" && menu !== kind) {
      e.preventDefault();
      setMenu(kind);
    }
  };

  useEffect(() => {
    if (!menu) return;
    const close = (e: MouseEvent) => {
      if (!root.current?.contains(e.target as Node)) setMenu(null);
    };
    const esc = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      closeMenu(true);
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
    closeMenu(true);
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
        ref={scopeBtn}
        className={`bt__scope${book ? " bt__scope--book" : ""}`}
        aria-haspopup="menu"
        aria-expanded={menu === "scope"}
        aria-label={aria}
        onKeyDown={onTriggerKey("scope")}
        onClick={() => (menu === "scope" ? closeMenu(true) : setMenu("scope"))}
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
        ref={moreBtn}
        className="bt__iconbtn"
        aria-haspopup="menu"
        aria-expanded={menu === "more"}
        aria-label={t("tts.bookTalk.menu.more")}
        title={t("tts.bookTalk.menu.more")}
        onKeyDown={onTriggerKey("more")}
        onClick={() => (menu === "more" ? closeMenu(true) : setMenu("more"))}
        data-testid="booktalk-more"
      >
        <MoreHorizontal width={16} height={16} aria-hidden="true" />
      </button>

      {menu === "scope" && (
        <div
          className="bt__menu"
          role="menu"
          ref={menuEl}
          aria-label={t("tts.bookTalk.scope.menuLabel")}
          onKeyDown={onMenuKey}
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
          ref={menuEl}
          aria-label={t("tts.bookTalk.menu.more")}
          onKeyDown={onMenuKey}
          data-testid="booktalk-more-menu"
        >
          <button
            type="button"
            role="menuitem"
            className="bt__menuitem"
            disabled={!canClear}
            onClick={() => {
              closeMenu(true);
              onClear();
            }}
            data-testid="booktalk-clear"
          >
            <span className="inline-flex items-center gap-2">
              <Trash2 width={14} height={14} aria-hidden="true" />
              {t("tts.bookTalk.menu.clear")}
            </span>
          </button>
          <button
            type="button"
            role="menuitemcheckbox"
            aria-checked={preview}
            className="bt__menuitem"
            onClick={() => {
              closeMenu(true);
              onPreview(!preview);
            }}
            data-testid="booktalk-preview"
          >
            <span className="inline-flex items-center gap-2">
              <Sparkles width={14} height={14} aria-hidden="true" />
              {t("tts.bookTalk.menu.preview")}
              <b className="bt__tag" data-testid="booktalk-preview-state">
                {preview ? t("tts.bookTalk.menu.on") : t("tts.bookTalk.menu.off")}
              </b>
            </span>
            <small data-testid="booktalk-preview-hint">
              {t("tts.bookTalk.menu.previewHint")}
            </small>
          </button>
          {cloud && (
            <button
              type="button"
              role="menuitem"
              className="bt__menuitem"
              onClick={() => {
                closeMenu(true);
                onRevoke();
              }}
              data-testid="booktalk-revoke"
            >
              <span className="inline-flex items-center gap-2">
                <Cloud width={14} height={14} aria-hidden="true" />
                {t("tts.bookTalk.menu.revoke", { model: cloud })}
              </span>
              <small>{t("tts.bookTalk.menu.revokeHint")}</small>
            </button>
          )}
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
