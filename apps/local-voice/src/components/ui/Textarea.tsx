import React from "react";
import {
  Textarea as FluentTextarea,
  type TextareaProps as FluentTextareaProps,
} from "@fluentui/react-components";

interface TextareaProps
  extends React.TextareaHTMLAttributes<HTMLTextAreaElement> {
  variant?: "default" | "compact";
}

/** Mehrzeiliges Feld als Fluent Textarea (Leitlinie §6), vertikal größenverstellbar. */
export const Textarea = React.forwardRef<HTMLTextAreaElement, TextareaProps>(
  ({ className = "", variant = "default", ...props }, ref) => (
    <FluentTextarea
      ref={ref}
      {...(props as FluentTextareaProps)}
      resize="vertical"
      size={variant === "compact" ? "small" : "medium"}
      className={`lv-textarea lv-textarea--${variant} ${className}`.trim()}
    />
  ),
);

Textarea.displayName = "Textarea";
