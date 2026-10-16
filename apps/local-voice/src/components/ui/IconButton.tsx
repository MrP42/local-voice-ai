import React, { forwardRef, useId } from "react";
import { Button } from "./Button";
import { TooltipTrigger } from "./TooltipTrigger";

interface IconButtonProps extends Omit<
  React.ButtonHTMLAttributes<HTMLButtonElement>,
  "children" | "title" | "aria-label" | "aria-describedby"
> {
  /** Das Symbol (lucide, aria-hidden). */
  icon: React.ReactNode;
  /** Name der Aktion: aria-label UND Tooltip-Text. */
  label: string;
  /** Standard: ghost (Fluent subtle) – Werkzeugzeilen, Panelköpfe. */
  variant?: "ghost" | "secondary" | "primary";
  size?: "sm" | "md" | "lg";
}

/**
 * Symbol-Button nach Fluent 2 (windows-fluent.md §4): der Button-Baustein als
 * quadratischer Icon-only-Button, immer mit Tooltip und aria-label.
 */
export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(
  (
    { icon, label, variant = "ghost", size = "sm", type = "button", ...props },
    ref,
  ) => {
    const tipId = `${useId()}-tip`;
    return (
      <TooltipTrigger
        tooltipId={tipId}
        content={<div className="text-sm">{label}</div>}
      >
        <Button
          ref={ref}
          type={type}
          variant={variant}
          size={size}
          aria-label={label}
          aria-describedby={tipId}
          {...({ icon } as object)}
          {...props}
        />
      </TooltipTrigger>
    );
  },
);

IconButton.displayName = "IconButton";
