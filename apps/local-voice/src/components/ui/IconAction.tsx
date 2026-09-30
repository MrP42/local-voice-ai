import React, { forwardRef, useId } from "react";
import type { LucideIcon } from "lucide-react";
import { TooltipTrigger } from "./TooltipTrigger";

export interface IconActionProps extends Omit<
  React.ButtonHTMLAttributes<HTMLButtonElement>,
  "children" | "title" | "aria-label" | "aria-describedby"
> {
  icon: LucideIcon;
  /** Zusatzklassen am Symbol, z. B. Pulsieren oder Signalfarbe. */
  iconClassName?: string;
  /** Der Name der Aktion samt laufendem Zustand ("Aufnahme beenden"): steht
   *  als aria-label am Knopf und fett im Tooltip. */
  label: string;
  /** Eine Zeile Erklärung unter dem Namen. */
  description: string;
  testId: string;
  /** Zweite Kennung am Wrapper, damit ältere Tests ihren Knopf weiter
   *  finden (Klick auf den Wrapper trifft den Knopf in der Mitte). */
  wrapperTestId?: string;
  /** Kleine Zahl an der Ecke (z. B. Fehlerzähler am Menü). */
  badge?: React.ReactNode;
  badgeTestId?: string;
  /** Tooltip zurückhalten, solange ein Menü am Knopf offen ist. */
  suppressTooltip?: boolean;
  /** Ohne Rahmen und Fläche (Symbol neben einem Titel); Größe bleibt gleich. */
  ghost?: boolean;
}

/**
 * Ein Symbol-Knopf der Bedienspalte: immer 36 x 36 px, Symbol 18 px, kein
 * Text. Alles, was sonst auf dem Knopf stünde, steht im Tooltip und im
 * aria-label — so bleiben alle Knöpfe gleich groß und die Zeile ruhig.
 * Die Optik entspricht `Button variant="secondary"`; ein eigener Baustein,
 * weil Button seine Größe über Innenabstände und Text bestimmt.
 */
export const IconAction = forwardRef<HTMLButtonElement, IconActionProps>(
  (
    {
      icon: Icon,
      iconClassName,
      label,
      description,
      testId,
      wrapperTestId,
      badge,
      badgeTestId,
      suppressTooltip = false,
      ghost = false,
      className = "",
      type = "button",
      ...props
    },
    ref,
  ) => {
    const tooltipId = `${useId()}-tip`;
    return (
      <TooltipTrigger
        tooltipId={tooltipId}
        suppressed={suppressTooltip}
        testId={wrapperTestId}
        content={
          <>
            <div className="text-sm font-semibold">
              <strong>{label}</strong>
            </div>
            <div className="text-xs text-text/70">{description}</div>
          </>
        }
      >
        <button
          ref={ref}
          type={type}
          aria-label={label}
          aria-describedby={tooltipId}
          data-testid={testId}
          className={`relative inline-flex h-[36px] w-[36px] shrink-0 items-center justify-center rounded-lg border text-text transition-colors focus:outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-logo-primary disabled:cursor-not-allowed disabled:opacity-50 cursor-pointer ${
            ghost
              ? "border-transparent bg-transparent text-text/70 hover:bg-mid-gray/15 hover:text-text"
              : "border-mid-gray/20 bg-mid-gray/10 hover:border-logo-primary hover:bg-background-ui/30"
          } ${className}`}
          {...props}
        >
          <Icon
            width={18}
            height={18}
            aria-hidden="true"
            className={iconClassName}
          />
          {badge !== undefined && badge !== null && (
            <span
              data-testid={badgeTestId}
              className="absolute -end-1.5 -top-1.5 min-w-4 rounded-full bg-red-500 px-1 text-center text-[10px] font-semibold leading-4 text-white"
            >
              {badge}
            </span>
          )}
        </button>
      </TooltipTrigger>
    );
  },
);

IconAction.displayName = "IconAction";
