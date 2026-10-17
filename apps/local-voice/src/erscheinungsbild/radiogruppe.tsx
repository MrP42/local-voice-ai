/**
 * @wai/ui erscheinungsbild v1 — radiogruppe.tsx
 * Quelle: WAI-SDK packages/ui/src/erscheinungsbild.
 * Selbsttragend: importiert nur react.
 *
 * Tastaturmodell einer Radiogruppe nach WAI-ARIA: ein Tab-Stopp (die gewaehlte Option), Pfeiltasten
 * wechseln Wahl und Fokus (mit Umlauf), Pos1/Ende springen an den Rand, Leertaste/Eingabe waehlt.
 */
import { type KeyboardEvent, useCallback, useRef } from "react";

export interface RadioProps {
  role: "radio";
  "aria-checked": boolean;
  tabIndex: 0 | -1;
  ref: (el: HTMLElement | null) => void;
  onKeyDown: (ev: KeyboardEvent<HTMLElement>) => void;
  onClick: () => void;
}

export function useRadiogruppe<T extends string>(
  werte: readonly T[],
  wert: T,
  beiWechsel: (w: T) => void,
): (w: T) => RadioProps {
  const elemente = useRef(new Map<T, HTMLElement>());
  const fokusWert = werte.includes(wert) ? wert : werte[0];

  const taste = useCallback(
    (ev: KeyboardEvent<HTMLElement>, aktuell: T) => {
      const i = werte.indexOf(aktuell);
      const n = werte.length;
      let ziel = -1;
      switch (ev.key) {
        case "ArrowRight":
        case "ArrowDown":
          ziel = (i + 1) % n;
          break;
        case "ArrowLeft":
        case "ArrowUp":
          ziel = (i - 1 + n) % n;
          break;
        case "Home":
          ziel = 0;
          break;
        case "End":
          ziel = n - 1;
          break;
        case " ":
        case "Enter":
          ev.preventDefault();
          beiWechsel(aktuell);
          return;
        default:
          return;
      }
      ev.preventDefault();
      const w = werte[ziel];
      beiWechsel(w);
      elemente.current.get(w)?.focus();
    },
    [werte, beiWechsel],
  );

  return (w: T) => ({
    role: "radio",
    "aria-checked": w === wert,
    tabIndex: w === fokusWert ? 0 : -1,
    ref: (el) => {
      if (el) elemente.current.set(w, el);
      else elemente.current.delete(w);
    },
    onKeyDown: (ev) => taste(ev, w),
    onClick: () => beiWechsel(w),
  });
}

/** Haken-Glyphe (16 px), Farbe ueber currentColor. */
export function Haken({ groesse = 12 }: { groesse?: number }) {
  return (
    <svg
      width={groesse}
      height={groesse}
      viewBox="0 0 16 16"
      aria-hidden="true"
      focusable="false"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M3 8.5l3.2 3.2L13 4.8" />
    </svg>
  );
}
