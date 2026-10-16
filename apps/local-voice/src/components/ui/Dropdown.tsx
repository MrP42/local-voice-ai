import React from "react";
import { useTranslation } from "react-i18next";
import {
  Dropdown as FluentDropdown,
  Option,
} from "@fluentui/react-components";
import { useSettingLabel } from "./SettingContainer";

export interface DropdownOption {
  value: string;
  label: string;
  disabled?: boolean;
}

interface DropdownProps {
  options: DropdownOption[];
  className?: string;
  selectedValue: string | null;
  onSelect: (value: string) => void;
  placeholder?: string;
  disabled?: boolean;
  onRefresh?: () => void;
  /** Zugänglicher Name, wenn keine sichtbare Beschriftung verbunden ist. */
  ariaLabel?: string;
}

/**
 * Auswahlliste als Fluent Dropdown (Leitlinie §6: Liste → Dropdown).
 * Auslöser `role="combobox"`, Einträge `role="option"`.
 */
export const Dropdown: React.FC<DropdownProps> = ({
  options,
  selectedValue,
  onSelect,
  className = "",
  placeholder,
  disabled = false,
  onRefresh,
  ariaLabel,
}) => {
  const { t } = useTranslation();
  // In einer Einstellungszeile nennt deren Titel die Liste.
  const settingLabel = useSettingLabel();
  const selectedOption = options.find(
    (option) => option.value === selectedValue,
  );

  return (
    <FluentDropdown
      className={`lv-dropdown ${className}`}
      value={selectedOption?.label ?? ""}
      selectedOptions={selectedValue ? [selectedValue] : []}
      placeholder={placeholder}
      disabled={disabled}
      aria-label={ariaLabel ?? settingLabel ?? undefined}
      onOpenChange={(_e, data) => {
        if (data.open && onRefresh) onRefresh();
      }}
      onOptionSelect={(_e, data) => {
        if (data.optionValue !== undefined) onSelect(data.optionValue);
      }}
    >
      {options.length === 0 ? (
        <Option value="" disabled text={t("common.noOptionsFound")}>
          {t("common.noOptionsFound")}
        </Option>
      ) : (
        options.map((option) => (
          <Option
            key={option.value}
            value={option.value}
            disabled={option.disabled}
            text={option.label}
          >
            {option.label}
          </Option>
        ))
      )}
    </FluentDropdown>
  );
};
