import React from "react";
import {
  Input as FluentInput,
  type InputProps as FluentInputProps,
} from "@fluentui/react-components";

interface InputProps
  extends Omit<React.InputHTMLAttributes<HTMLInputElement>, "size"> {
  variant?: "default" | "compact";
}

/**
 * Eingabefeld als Fluent Input (Leitlinie §6). `ref` zeigt auf das <input>
 * (Suchfelder werden fokussiert), `className` auf die Feldhülle.
 * Beschriftung über dem Feld: mit Fluent `Field` oder einem <label>.
 */
export const Input = React.forwardRef<HTMLInputElement, InputProps>(
  ({ className = "", variant = "default", ...props }, ref) => (
    <FluentInput
      ref={ref}
      {...(props as Omit<FluentInputProps, "size">)}
      size={variant === "compact" ? "small" : "medium"}
      className={`lv-input ${className}`.trim()}
    />
  ),
);

Input.displayName = "Input";
