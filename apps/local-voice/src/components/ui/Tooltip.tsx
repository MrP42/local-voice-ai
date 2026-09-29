import React, { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

type TooltipPosition = "top" | "bottom";

interface TooltipCoords {
  top: number;
  left: number;
  arrowLeft: number;
  actualPosition: TooltipPosition;
  /** Neben oder ausserhalb des `avoidRef`-Elements: kein Pfeil zum Ziel. */
  detached: boolean;
}

interface TooltipProps {
  targetRef: React.RefObject<HTMLElement>;
  position?: TooltipPosition;
  /**
   * Bereich, den die Sprechblase nicht ueberdecken soll (z. B. der Antworttext,
   * in dem ein Beleg-Chip steht). Sie erscheint dann links oder rechts daneben,
   * sonst unter bzw. ueber dem Bereich - nie darauf.
   */
  avoidRef?: React.RefObject<HTMLElement | null>;
  children: React.ReactNode;
}

const TOOLTIP_WIDTH = 200;
const VIEWPORT_PADDING = 12;
const GAP = 8;
const ARROW_MARGIN = 12;
const DEFAULT_HEIGHT = 60;

export const Tooltip: React.FC<TooltipProps> = ({
  targetRef,
  position = "top",
  avoidRef,
  children,
}) => {
  const [coords, setCoords] = useState<TooltipCoords | null>(null);
  const tooltipRef = useRef<HTMLDivElement>(null);

  const updatePosition = useCallback(() => {
    if (!targetRef.current) return;

    const targetRect = targetRef.current.getBoundingClientRect();
    const tooltipHeight = tooltipRef.current?.offsetHeight || DEFAULT_HEIGHT;

    const avoidRect = avoidRef?.current?.getBoundingClientRect();
    if (avoidRect) {
      const clampTop = (value: number) =>
        Math.min(
          Math.max(value, VIEWPORT_PADDING),
          Math.max(
            VIEWPORT_PADDING,
            window.innerHeight - tooltipHeight - VIEWPORT_PADDING,
          ),
        );
      const centeredTop = clampTop(
        targetRect.top + targetRect.height / 2 - tooltipHeight / 2,
      );
      const leftRoom = avoidRect.left - GAP - VIEWPORT_PADDING;
      const rightRoom =
        window.innerWidth - avoidRect.right - GAP - VIEWPORT_PADDING;
      if (leftRoom >= TOOLTIP_WIDTH) {
        setCoords({
          top: centeredTop,
          left: avoidRect.left - GAP - TOOLTIP_WIDTH,
          arrowLeft: 0,
          actualPosition: position,
          detached: true,
        });
        return;
      }
      if (rightRoom >= TOOLTIP_WIDTH) {
        setCoords({
          top: centeredTop,
          left: avoidRect.right + GAP,
          arrowLeft: 0,
          actualPosition: position,
          detached: true,
        });
        return;
      }
      // Kein Platz daneben: unter dem Bereich, sonst darueber, sonst (Notlage)
      // unten am Fenster - immer waagerecht beim Ziel.
      const below = avoidRect.bottom + GAP;
      const above = avoidRect.top - GAP - tooltipHeight;
      const top =
        below + tooltipHeight <= window.innerHeight - VIEWPORT_PADDING
          ? below
          : above >= VIEWPORT_PADDING
            ? above
            : clampTop(below);
      const targetMid = targetRect.left + targetRect.width / 2;
      const left = Math.min(
        Math.max(targetMid - TOOLTIP_WIDTH / 2, VIEWPORT_PADDING),
        window.innerWidth - TOOLTIP_WIDTH - VIEWPORT_PADDING,
      );
      setCoords({
        top,
        left,
        arrowLeft: 0,
        actualPosition: position,
        detached: true,
      });
      return;
    }

    let actualPosition = position;
    let top: number;

    if (position === "top") {
      const spaceAbove = targetRect.top - tooltipHeight - GAP;
      if (spaceAbove < VIEWPORT_PADDING) {
        actualPosition = "bottom";
        top = targetRect.bottom + GAP;
      } else {
        top = targetRect.top - GAP - tooltipHeight;
      }
    } else {
      const spaceBelow =
        window.innerHeight - targetRect.bottom - tooltipHeight - GAP;
      if (spaceBelow < VIEWPORT_PADDING) {
        actualPosition = "top";
        top = targetRect.top - GAP - tooltipHeight;
      } else {
        top = targetRect.bottom + GAP;
      }
    }

    const targetCenter = targetRect.left + targetRect.width / 2;
    let left = targetCenter - TOOLTIP_WIDTH / 2;

    if (left < VIEWPORT_PADDING) {
      left = VIEWPORT_PADDING;
    } else if (left + TOOLTIP_WIDTH > window.innerWidth - VIEWPORT_PADDING) {
      left = window.innerWidth - TOOLTIP_WIDTH - VIEWPORT_PADDING;
    }

    const arrowLeft = Math.min(
      Math.max(targetCenter - left, ARROW_MARGIN),
      TOOLTIP_WIDTH - ARROW_MARGIN,
    );

    setCoords({ top, left, arrowLeft, actualPosition, detached: false });
  }, [targetRef, avoidRef, position]);

  useEffect(() => {
    updatePosition();

    window.addEventListener("scroll", updatePosition, true);
    window.addEventListener("resize", updatePosition);

    return () => {
      window.removeEventListener("scroll", updatePosition, true);
      window.removeEventListener("resize", updatePosition);
    };
  }, [updatePosition]);

  const arrowClasses =
    coords?.actualPosition === "top" ? "top-full" : "bottom-full rotate-180";

  return createPortal(
    <div
      ref={tooltipRef}
      style={{
        position: "fixed",
        top: coords?.top ?? -9999,
        left: coords?.left ?? -9999,
        width: TOOLTIP_WIDTH,
        zIndex: 9999,
        opacity: coords ? 1 : 0,
      }}
      className="px-3 py-2 bg-background border border-mid-gray/80 rounded-lg shadow-lg whitespace-normal transition-opacity duration-150"
    >
      {children}
      {!coords?.detached && (
        <div
          style={{ left: coords?.arrowLeft ?? 0 }}
          className={`absolute ${arrowClasses} transform -translate-x-1/2 w-0 h-0 border-l-[6px] border-r-[6px] border-t-[6px] border-l-transparent border-r-transparent border-t-mid-gray/80`}
        />
      )}
    </div>,
    document.body,
  );
};
