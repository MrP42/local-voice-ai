import React, { useEffect, useRef, useState } from "react";
import { Tooltip } from "./Tooltip";

/** Verzögerung beim Überfahren mit der Maus. Tastaturfokus zeigt sofort:
 *  wer per Tab wandert, wartet nicht auf einen Zeiger. */
const HOVER_DELAY_MS = 400;

/** Kurze Aktionstexte brauchen mehr Platz als die 200 px des Standards. */
const ACTION_TOOLTIP_WIDTH = 300;

interface TooltipTriggerProps {
  /** Was das Tooltip sagt. */
  content: React.ReactNode;
  /** id des Tooltip-Elements — das Ziel nennt sie in `aria-describedby`. */
  tooltipId: string;
  /** Solange true, bleibt das Tooltip zu (z. B. bei geöffnetem Menü). */
  suppressed?: boolean;
  width?: number;
  className?: string;
  /** Zusatz-Kennung am Wrapper, für Tests, die den Wrapper ansprechen. */
  testId?: string;
  children: React.ReactNode;
}

/**
 * Hängt ein Tooltip an ein beliebiges Kind: bei Hover nach kurzer Verzögerung
 * und bei Tastaturfokus sofort. Esc schließt, ein Klick ebenfalls (sonst
 * verdeckt es, was man gerade bedient). Die Ereignisse sitzen am Wrapper und
 * nicht am Knopf, weil ein deaktivierter Knopf keine Mausereignisse liefert —
 * das Tooltip soll aber gerade dort erklären, warum nichts geht.
 */
export const TooltipTrigger: React.FC<TooltipTriggerProps> = ({
  content,
  tooltipId,
  suppressed = false,
  width = ACTION_TOOLTIP_WIDTH,
  className = "inline-flex",
  testId,
  children,
}) => {
  const wrapperRef = useRef<HTMLSpanElement>(null);
  const timer = useRef<number | null>(null);
  // Zeigerklick fokussiert Eingabefelder (react-select) ohne Tastatur; das
  // soll kein Tooltip öffnen. Das Flag gilt bis zur nächsten Taste.
  const pointerActive = useRef(false);
  const [open, setOpen] = useState(false);

  const clearTimer = () => {
    if (timer.current !== null) {
      window.clearTimeout(timer.current);
      timer.current = null;
    }
  };
  const close = () => {
    clearTimer();
    setOpen(false);
  };

  useEffect(() => clearTimer, []);

  useEffect(() => {
    if (suppressed) close();
  }, [suppressed]);

  // Esc schließt auch, wenn der Fokus nicht im Wrapper liegt (reines Hover).
  useEffect(() => {
    if (!open) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") close();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open]);

  return (
    <span
      ref={wrapperRef}
      className={className}
      data-testid={testId}
      onMouseEnter={() => {
        if (suppressed || open || timer.current !== null) return;
        timer.current = window.setTimeout(() => {
          timer.current = null;
          setOpen(true);
        }, HOVER_DELAY_MS);
      }}
      onMouseLeave={close}
      onPointerDown={() => {
        pointerActive.current = true;
        close();
      }}
      onKeyDown={() => {
        pointerActive.current = false;
      }}
      onFocus={() => {
        if (suppressed || pointerActive.current) return;
        clearTimer();
        setOpen(true);
      }}
      onBlur={close}
    >
      {children}
      {open && !suppressed && (
        <Tooltip
          targetRef={wrapperRef}
          id={tooltipId}
          role="tooltip"
          width={width}
          passThrough
        >
          {content}
        </Tooltip>
      )}
    </span>
  );
};
