import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, Highlighter, ScanSearch, X } from "lucide-react";
import type { TermsRunState } from "./useBookTerms";
import "./terms.css";

const ICON_BTN =
  "flex h-8 w-8 cursor-pointer items-center justify-center rounded-md text-text/70 hover:bg-mid-gray/15 hover:text-text focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary";
const MENU_ITEM =
  "flex w-full cursor-pointer items-center gap-2 px-3 py-1.5 text-start text-sm hover:bg-mid-gray/15 disabled:cursor-not-allowed disabled:opacity-40";

/**
 * Begriffe in der Werkzeugleiste (lva#185): Hervorhebung ein/aus und der
 * Erkennungslauf auf Knopfdruck (Codex 01.11.: nie automatisch).
 */
export function TermsMenu({
  highlight,
  onHighlight,
  count,
  run,
  busy,
  onExtract,
  onCancel,
}: {
  highlight: boolean;
  onHighlight: (on: boolean) => void;
  /** Bekannte Begriffe des Buchs. */
  count: number;
  run: TermsRunState | null;
  /** Ein anderer KI-Lauf (Zusammenfassung) belegt das Modell. */
  busy: boolean;
  onExtract: () => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      setOpen(false);
    };
    document.addEventListener("keydown", onKey, true);
    return () => document.removeEventListener("keydown", onKey, true);
  }, [open]);
  const label = t("tts.terms.menu");
  return (
    <div className="relative">
      <button
        type="button"
        className={`${ICON_BTN} ${highlight && count > 0 ? "bg-logo-primary/15 text-text" : ""}`}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={label}
        title={label}
        onClick={() => setOpen((o) => !o)}
        data-testid="reader-terms"
      >
        <Highlighter width={16} height={16} aria-hidden="true" />
      </button>
      {run && (
        <span
          className="terms-menu__dot"
          aria-hidden="true"
          data-testid="reader-terms-running"
        />
      )}
      {open && (
        <>
          <div className="fixed inset-0 z-30" onClick={() => setOpen(false)} />
          <div
            role="menu"
            aria-label={label}
            className="absolute right-0 top-full z-40 mt-1 w-64 rounded-lg border border-mid-gray/40 bg-background py-1 shadow-lg"
            data-testid="reader-terms-menu"
          >
            <button
              type="button"
              role="menuitemcheckbox"
              aria-checked={highlight}
              onClick={() => onHighlight(!highlight)}
              className={MENU_ITEM}
              data-testid="reader-terms-highlight"
            >
              <Check
                width={14}
                height={14}
                aria-hidden="true"
                className={highlight ? "" : "invisible"}
              />
              {t("tts.terms.highlight")}
            </button>
            <hr className="my-1 border-mid-gray/20" />
            {run ? (
              <button
                type="button"
                role="menuitem"
                onClick={onCancel}
                className={MENU_ITEM}
                data-testid="reader-terms-cancel"
              >
                <X width={14} height={14} aria-hidden="true" />
                {t("tts.terms.cancelRun")}
              </button>
            ) : (
              <button
                type="button"
                role="menuitem"
                disabled={busy}
                onClick={() => {
                  setOpen(false);
                  onExtract();
                }}
                className={MENU_ITEM}
                data-testid="reader-terms-extract"
              >
                <ScanSearch width={14} height={14} aria-hidden="true" />
                {count > 0
                  ? t("tts.terms.extractAgain")
                  : t("tts.terms.extract")}
              </button>
            )}
            <p
              className="terms-menu__status"
              aria-live="polite"
              data-testid="reader-terms-status"
            >
              {run
                ? run.total > 0
                  ? t("tts.terms.running", {
                      done: Math.min(run.done, run.total),
                      total: run.total,
                    })
                  : t("tts.terms.runningStart")
                : count > 0
                  ? t("tts.terms.count", { count })
                  : t("tts.terms.none")}
            </p>
          </div>
        </>
      )}
    </div>
  );
}
