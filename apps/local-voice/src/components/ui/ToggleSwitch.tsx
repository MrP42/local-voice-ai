import React from "react";
import { Spinner, Switch } from "@fluentui/react-components";
import { SettingContainer } from "./SettingContainer";

interface ToggleSwitchProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  isUpdating?: boolean;
  label: string;
  description: string;
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
  tooltipPosition?: "top" | "bottom";
  /** `data-testid` des Eingabefelds (das Kontrollkästchen unter dem Schalter). */
  testId?: string;
}

export const ToggleSwitch: React.FC<ToggleSwitchProps> = ({
  checked,
  onChange,
  disabled = false,
  isUpdating = false,
  label,
  description,
  descriptionMode = "tooltip",
  grouped = false,
  tooltipPosition = "top",
  testId,
}) => {
  return (
    <SettingContainer
      title={label}
      description={description}
      descriptionMode={descriptionMode}
      grouped={grouped}
      disabled={disabled}
      tooltipPosition={tooltipPosition}
    >
      {/* Fluent Switch: wirkt sofort (Leitlinie §6). Das Eingabefeld behält
          type="checkbox" mit role="switch" und den data-testid. */}
      <Switch
        checked={checked}
        disabled={disabled || isUpdating}
        onChange={(_e, data) => onChange(data.checked)}
        aria-label={label}
        data-testid={testId}
      />
      {isUpdating && (
        <div className="absolute inset-0 flex items-center justify-center">
          <Spinner size="tiny" />
        </div>
      )}
    </SettingContainer>
  );
};
