import React from "react";
import {
  Button as FluentButton,
  type ButtonProps as FluentButtonProps,
} from "@fluentui/react-components";

/**
 * Schaltfläche nach Fluent 2 (Leitlinie windows-fluent.md §4).
 *
 * Hierarchie: `secondary` ist der Standard jedes Buttons; `primary` nur für die
 * EINE nächste Aktion je Ansicht/Dialog. `ghost` = Fluent `subtle` (Werkzeug-
 * und Listenzeilen), `danger` = endgültig zerstörend nach Bestätigung.
 * `primary-soft` bleibt als Name für Bestandsaufrufe und ist optisch Standard.
 *
 * Größen: sm 24 · md 32 (Standard) · lg 40 px. Gedrückt = dunklere Fläche,
 * nie skalieren.
 */
interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?:
    | "primary"
    | "primary-soft"
    | "secondary"
    | "danger"
    | "danger-ghost"
    | "ghost";
  size?: "sm" | "md" | "lg";
}

const APPEARANCE: Record<
  NonNullable<ButtonProps["variant"]>,
  FluentButtonProps["appearance"]
> = {
  primary: "primary",
  "primary-soft": "secondary",
  secondary: "secondary",
  danger: "primary",
  "danger-ghost": "subtle",
  ghost: "subtle",
};

const SIZE = { sm: "small", md: "medium", lg: "large" } as const;

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  (
    {
      children,
      className = "",
      variant = "secondary",
      size = "md",
      // Wie das native <button>: ohne Angabe "submit" (Formulare senden per
      // Enter weiter); Fluent setzte sonst "button".
      type = "submit",
      ...props
    },
    ref,
  ) => (
    <FluentButton
      ref={ref}
      {...({
        ...props,
        type,
        appearance: APPEARANCE[variant],
        size: SIZE[size],
        "data-variant": variant,
        className: `lv-btn lv-btn--${variant} ${className}`.trim(),
      } as unknown as FluentButtonProps)}
    >
      {children}
    </FluentButton>
  ),
);

Button.displayName = "Button";
