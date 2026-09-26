import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Redo2, Undo2 } from "lucide-react";

interface HistoryButtonsProps {
  past: string[];
  future: string[];
  present: string;
  onUndo: () => void;
  onRedo: () => void;
  /** n > 0: n Schritte zurueck, n < 0: n Schritte vor. */
  onJump: (n: number) => void;
}

/** Was sich zwischen zwei Staenden geaendert hat, in einer Zeile. */
export function describeChange(from: string, to: string): string {
  let a = 0;
  const max = Math.min(from.length, to.length);
  while (a < max && from[a] === to[a]) a++;
  let b = 0;
  while (b < max - a && from[from.length - 1 - b] === to[to.length - 1 - b])
    b++;
  const removed = from.slice(a, from.length - b);
  const added = to.slice(a, to.length - b);
  const clip = (x: string) => {
    const one = x.replace(/\s+/g, " ").trim();
    return one.length > 40 ? `${one.slice(0, 40)}…` : one;
  };
  if (removed && added) return `„${clip(removed)}“ → „${clip(added)}“`;
  if (added) return `+ „${clip(added)}“`;
  if (removed) return `− „${clip(removed)}“`;
  return "";
}

const BTN =
  "flex h-8 w-8 items-center justify-center rounded-md text-text/70 hover:bg-mid-gray/20 hover:text-text disabled:cursor-not-allowed disabled:opacity-35 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary";

/**
 * Rueckgaengig / Wiederherstellen wie in Word: zwei Pfeile, Tooltip als
 * Beschriftung. Rechtsklick oder kurzes Verweilen oeffnet die Liste der
 * Schritte (scrollbar) -- ein Klick springt mehrere Schritte auf einmal.
 */
export const HistoryButtons: React.FC<HistoryButtonsProps> = ({
  past,
  future,
  present,
  onUndo,
  onRedo,
  onJump,
}) => {
  const { t } = useTranslation();
  const [open, setOpen] = useState<"undo" | "redo" | null>(null);
  const hoverTimer = useRef<number | null>(null);
  const boxRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (boxRef.current && !boxRef.current.contains(e.target as Node)) {
        setOpen(null);
      }
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [open]);

  const startHover = (which: "undo" | "redo") => {
    if (hoverTimer.current) window.clearTimeout(hoverTimer.current);
    hoverTimer.current = window.setTimeout(() => setOpen(which), 900);
  };
  const stopHover = () => {
    if (hoverTimer.current) window.clearTimeout(hoverTimer.current);
    hoverTimer.current = null;
  };

  // Schrittliste: jeder Eintrag beschreibt die Aenderung, die der Sprung
  // rueckgaengig macht (bzw. wiederherstellt).
  const undoSteps = past.map((state, i) => ({
    n: i + 1,
    label: describeChange(state, i === 0 ? present : past[i - 1]),
  }));
  const redoSteps = future.map((state, i) => ({
    n: -(i + 1),
    label: describeChange(i === 0 ? present : future[i - 1], state),
  }));
  const steps = open === "undo" ? undoSteps : redoSteps;

  return (
    <div
      ref={boxRef}
      className="relative flex items-center gap-0.5"
      onMouseLeave={stopHover}
    >
      <button
        type="button"
        className={BTN}
        onClick={() => {
          setOpen(null);
          onUndo();
        }}
        onContextMenu={(e) => {
          e.preventDefault();
          if (past.length) setOpen("undo");
        }}
        onMouseEnter={() => past.length && startHover("undo")}
        onMouseLeave={stopHover}
        disabled={past.length === 0}
        title={t("tts.history.undoHint")}
        aria-label={t("tts.history.undo")}
        data-testid="history-undo"
      >
        <Undo2 width={20} height={20} />
      </button>
      <button
        type="button"
        className={BTN}
        onClick={() => {
          setOpen(null);
          onRedo();
        }}
        onContextMenu={(e) => {
          e.preventDefault();
          if (future.length) setOpen("redo");
        }}
        onMouseEnter={() => future.length && startHover("redo")}
        onMouseLeave={stopHover}
        disabled={future.length === 0}
        title={t("tts.history.redoHint")}
        aria-label={t("tts.history.redo")}
        data-testid="history-redo"
      >
        <Redo2 width={20} height={20} />
      </button>
      {open && steps.length > 0 && (
        <div
          role="menu"
          data-testid="history-list"
          className="absolute right-0 top-full z-40 mt-1 w-80 rounded-lg border border-mid-gray/40 bg-background py-1 text-sm shadow-lg"
        >
          <p className="px-3 py-1 text-xs text-text/50">
            {open === "undo"
              ? t("tts.history.listUndo")
              : t("tts.history.listRedo")}
          </p>
          <div className="max-h-64 overflow-y-auto">
            {steps.map((step) => (
              <button
                key={step.n}
                type="button"
                role="menuitem"
                onClick={() => {
                  setOpen(null);
                  onJump(step.n);
                }}
                className="flex w-full items-baseline gap-2 px-3 py-1.5 text-start hover:bg-mid-gray/15"
              >
                <span className="w-6 shrink-0 text-xs text-text/40 tabular-nums">
                  {Math.abs(step.n)}
                </span>
                <span className="min-w-0 truncate text-text/80">
                  {step.label || t("tts.history.noChange")}
                </span>
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
};
