import React from "react";
import SelectComponent from "react-select";
import CreatableSelect from "react-select/creatable";
import { useDialogPortalTarget } from "./Dialog";
import type {
  ActionMeta,
  Props as ReactSelectProps,
  SingleValue,
  StylesConfig,
} from "react-select";

export type SelectOption = {
  value: string;
  label: string;
  isDisabled?: boolean;
};

type BaseProps = {
  value: string | null;
  options: SelectOption[];
  placeholder?: string;
  disabled?: boolean;
  isLoading?: boolean;
  isClearable?: boolean;
  onChange: (value: string | null, action: ActionMeta<SelectOption>) => void;
  onBlur?: () => void;
  className?: string;
  /** Liste als schwebende Ebene an `document.body` statt im Elternelement:
   *  nötig in scrollenden/abschneidenden Containern (Bedienspalte der
   *  Vorlesen-Seite), sonst verdrängt oder verdeckt die Liste dort andere
   *  Inhalte. Standard: aus. */
  menuPortal?: boolean;
  formatCreateLabel?: (input: string) => string;
  /** Zugänglicher Name des Eingabefelds (die sichtbare Beschriftung steht
   *  meist daneben, ohne mit dem Feld verbunden zu sein). */
  ariaLabel?: string;
  /** Eigene Darstellung eines Eintrags (`menu`: in der offenen Liste,
   *  `value`: im geschlossenen Feld). Standard: nur die Beschriftung. */
  formatOptionLabel?: (
    option: SelectOption,
    context: "menu" | "value",
  ) => React.ReactNode;
};

type CreatableProps = {
  isCreatable: true;
  onCreateOption: (value: string) => void;
};

type NonCreatableProps = {
  isCreatable?: false;
  onCreateOption?: never;
};

export type SelectProps = BaseProps & (CreatableProps | NonCreatableProps);

// Optik wie Fluent Dropdown/Combobox (Leitlinie §6): Radius 4, neutraler
// Rand, Unterkante als Akzentlinie, Fokus = 2px Markenlinie unten, Liste als
// Flyout (Radius 8, shadow16). Werte aus den Fluent-Tokens (FluentRoot).
const selectStyles: StylesConfig<SelectOption, false> = {
  control: (base, state) => ({
    ...base,
    minHeight: 32,
    borderRadius: 4,
    borderColor: "var(--colorNeutralStroke1)",
    borderBottomColor: state.isFocused
      ? "var(--colorCompoundBrandStroke)"
      : "var(--colorNeutralStrokeAccessible)",
    boxShadow: state.isFocused
      ? "inset 0 -1px 0 var(--colorCompoundBrandStroke)"
      : "none",
    backgroundColor: "var(--colorNeutralBackground1)",
    fontSize: "var(--fontSizeBase300)",
    color: "var(--colorNeutralForeground1)",
    ":hover": {
      borderColor: "var(--colorNeutralStroke1Hover)",
      borderBottomColor: "var(--colorNeutralStrokeAccessibleHover)",
    },
  }),
  valueContainer: (base) => ({
    ...base,
    paddingInline: 10,
    paddingBlock: 2,
  }),
  input: (base) => ({
    ...base,
    color: "var(--colorNeutralForeground1)",
  }),
  singleValue: (base) => ({
    ...base,
    color: "var(--colorNeutralForeground1)",
  }),
  indicatorSeparator: () => ({ display: "none" }),
  dropdownIndicator: (base) => ({
    ...base,
    padding: 6,
    color: "var(--colorNeutralForeground3)",
    ":hover": { color: "var(--colorNeutralForeground2)" },
  }),
  clearIndicator: (base) => ({
    ...base,
    padding: 6,
    color: "var(--colorNeutralForeground3)",
    ":hover": { color: "var(--colorNeutralForeground2)" },
  }),
  // Nur mit `menuPortal` wirksam. Über allem, was die App als Ebene nutzt
  // (Menü/Popover z-40 in ActionMenu), damit die Liste dort nie verdeckt wird.
  menuPortal: (base) => ({ ...base, zIndex: 60 }),
  menu: (provided) => ({
    ...provided,
    zIndex: 30,
    borderRadius: 8,
    padding: 4,
    backgroundColor: "var(--colorNeutralBackground1)",
    color: "var(--colorNeutralForeground1)",
    border: "1px solid var(--colorTransparentStroke)",
    boxShadow: "var(--shadow16)",
  }),
  option: (base, state) => ({
    ...base,
    borderRadius: 4,
    backgroundColor: state.isSelected
      ? "var(--colorNeutralBackground1Selected)"
      : state.isFocused
        ? "var(--colorNeutralBackground1Hover)"
        : "transparent",
    color: state.isDisabled
      ? "var(--colorNeutralForegroundDisabled)"
      : "var(--colorNeutralForeground1)",
    fontWeight: state.isSelected ? 600 : 400,
    cursor: state.isDisabled ? "not-allowed" : base.cursor,
  }),
  placeholder: (base) => ({
    ...base,
    color: "var(--colorNeutralForeground4)",
  }),
};

export const Select: React.FC<SelectProps> = React.memo(
  ({
    value,
    options,
    placeholder,
    disabled,
    isLoading,
    isClearable = true,
    onChange,
    onBlur,
    className = "",
    menuPortal = false,
    ariaLabel,
    formatOptionLabel,
    isCreatable,
    formatCreateLabel,
    onCreateOption,
  }) => {
    const dialogTarget = useDialogPortalTarget();
    const selectValue = React.useMemo(() => {
      if (!value) return null;
      const existing = options.find((option) => option.value === value);
      if (existing) return existing;
      return { value, label: value, isDisabled: false };
    }, [value, options]);

    const handleChange = (
      option: SingleValue<SelectOption>,
      action: ActionMeta<SelectOption>,
    ) => {
      onChange(option?.value ?? null, action);
    };

    const sharedProps: Partial<ReactSelectProps<SelectOption, false>> = {
      className,
      classNamePrefix: "app-select",
      value: selectValue,
      options,
      onChange: handleChange,
      placeholder,
      "aria-label": ariaLabel,
      isDisabled: disabled,
      isLoading,
      onBlur,
      isClearable,
      styles: selectStyles,
      ...(formatOptionLabel
        ? {
            formatOptionLabel: (
              option: SelectOption,
              meta: { context: "menu" | "value" },
            ) => formatOptionLabel(option, meta.context),
          }
        : {}),
      // Portal + fixed: die Liste richtet sich am Fenster aus und flippt bei
      // Platzmangel; Scrollen der Seite soll sie nicht verschieben.
      ...(menuPortal && typeof document !== "undefined"
        ? {
            // Im Fluent-Dialog in die Dialogfläche, sonst an body.
            menuPortalTarget: dialogTarget ?? document.body,
            menuPosition: "fixed" as const,
            menuPlacement: "auto" as const,
            menuShouldScrollIntoView: false,
          }
        : {}),
    };

    if (isCreatable) {
      return (
        <CreatableSelect<SelectOption, false>
          {...sharedProps}
          onCreateOption={onCreateOption}
          formatCreateLabel={formatCreateLabel}
        />
      );
    }

    return <SelectComponent<SelectOption, false> {...sharedProps} />;
  },
);

Select.displayName = "Select";
