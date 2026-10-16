import React from "react";
import ResetIcon from "../icons/ResetIcon";
import { Button } from "./Button";

interface ResetButtonProps {
  onClick: () => void;
  disabled?: boolean;
  className?: string;
  ariaLabel?: string;
  children?: React.ReactNode;
}

/** Zurücksetzen-Symbol neben einer Einstellung: Fluent-Button (subtle, klein). */
export const ResetButton: React.FC<ResetButtonProps> = React.memo(
  ({ onClick, disabled = false, className = "", ariaLabel, children }) => (
    <Button
      type="button"
      variant="ghost"
      size="sm"
      aria-label={ariaLabel}
      title={ariaLabel}
      className={className}
      onClick={onClick}
      disabled={disabled}
    >
      {children ?? <ResetIcon />}
    </Button>
  ),
);
