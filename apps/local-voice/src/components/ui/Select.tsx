import React from "react";
import SelectComponent, { components as selectParts } from "react-select";
import CreatableSelect from "react-select/creatable";
import { useDialogPortalTarget } from "./Dialog";
import type {
  ActionMeta,
  InputProps,
  MenuListProps,
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
  /** Kompakter Knopf (nur Symbol + Pfeil, Text per `formatOptionLabel`):
   *  die Liste ist dann breiter als das Feld und rechtsbündig daran. */
  compactMenuWidth?: number;
  formatCreateLabel?: (input: string) => string;
  /** Zugänglicher Name des Eingabefelds (die sichtbare Beschriftung steht
   *  meist daneben, ohne mit dem Feld verbunden zu sein). */
  ariaLabel?: string;
  /** Kennung des Elements, das das Feld beschreibt (Hinweistext unter dem Feld). */
  ariaDescribedBy?: string;
  /** Eigene Darstellung eines Eintrags (`menu`: in der offenen Liste,
   *  `value`: im geschlossenen Feld). Standard: nur die Beschriftung. */
  formatOptionLabel?: (
    option: SelectOption,
    context: "menu" | "value",
  ) => React.ReactNode;
  /** Merkt sich die Scrollposition der offenen Liste unter diesem Schlüssel:
   *  beim nächsten Öffnen steht sie wieder dort (Patrick 01.11., lange
   *  Stimmenliste). Ohne gemerkte Stelle steht der gewählte Eintrag im Blick. */
  menuScrollKey?: string;
  /** Kopf über den Einträgen der offenen Liste (z. B. Filter-Chips). Klicks
   *  darin schließen die Liste nicht. */
  menuHeader?: React.ReactNode;
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

/** Gemerkte Scrollpositionen je `menuScrollKey`, solange die App läuft. */
const menuScrollTops = new Map<string, number>();

type MenuExtras = { menuScrollKey?: string; menuHeader?: React.ReactNode };

/** Liste mit gemerkter Scrollposition und optionalem Kopf. */
const MenuListWithMemory = (props: MenuListProps<SelectOption, false>) => {
  const { menuScrollKey, menuHeader } = props.selectProps as unknown as MenuExtras;
  const ref = React.useRef<HTMLDivElement | null>(null);
  // Zielstand beim Öffnen; bis der Nutzer selbst scrollt (Rad, Ziehen,
  // Tasten), setzt jedes fremde Scrollen (react-select scrollt zum
  // Fokus-Eintrag) die Liste dorthin zurück. Gemerkt wird nur, was der
  // Nutzer gescrollt hat.
  const target = React.useRef<number | null>(null);
  const user = React.useRef(false);
  const setRef = (el: HTMLDivElement | null) => {
    ref.current = el;
    const outer = props.innerRef;
    if (typeof outer === "function") outer(el);
    else if (outer) (outer as React.MutableRefObject<HTMLDivElement | null>).current = el;
  };
  React.useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const saved = menuScrollKey ? menuScrollTops.get(menuScrollKey) : undefined;
    if (saved !== undefined) {
      target.current = saved;
    } else {
      const selected = el.querySelector<HTMLElement>(
        '[aria-selected="true"], .app-select__option--is-selected',
      );
      target.current = selected
        ? Math.max(
            0,
            selected.offsetTop - el.clientHeight / 2 + selected.offsetHeight / 2,
          )
        : null;
    }
    if (target.current !== null) el.scrollTop = target.current;
    const mine = () => {
      user.current = true;
    };
    el.addEventListener("wheel", mine, { passive: true });
    el.addEventListener("pointerdown", mine);
    el.addEventListener("touchmove", mine, { passive: true });
    window.addEventListener("keydown", mine, true);
    return () => {
      el.removeEventListener("wheel", mine);
      el.removeEventListener("pointerdown", mine);
      el.removeEventListener("touchmove", mine);
      window.removeEventListener("keydown", mine, true);
    };
    // Nur beim Öffnen (Einhängen der Liste), nicht bei jedem Neuzeichnen.
  }, []);
  return (
    <>
      {menuHeader && (
        <div
          className="app-select__menu-header"
          // Klicks im Kopf nehmen dem Feld nicht den Fokus (Liste bleibt offen).
          onMouseDown={(e) => {
            e.preventDefault();
            e.stopPropagation();
          }}
          onPointerDown={() => {
            // Ein Filterwechsel ändert die Liste: ab hier zählt der Nutzer.
            user.current = true;
          }}
          onTouchEnd={(e) => e.stopPropagation()}
        >
          {menuHeader}
        </div>
      )}
      <selectParts.MenuList
        {...props}
        innerRef={setRef}
        innerProps={{
          ...props.innerProps,
          onScroll: (e: React.UIEvent<HTMLDivElement>) => {
            const el = e.currentTarget;
            if (!user.current) {
              if (target.current !== null && Math.abs(el.scrollTop - target.current) > 1)
                el.scrollTop = target.current;
              return;
            }
            if (menuScrollKey) menuScrollTops.set(menuScrollKey, el.scrollTop);
          },
        }}
      />
    </>
  );
};

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
    compactMenuWidth,
    ariaLabel,
    ariaDescribedBy,
    formatOptionLabel,
    isCreatable,
    formatCreateLabel,
    onCreateOption,
    menuScrollKey,
    menuHeader,
  }) => {
    const dialogTarget = useDialogPortalTarget();
    const selectValue = React.useMemo(() => {
      if (!value) return null;
      const existing = options.find((option) => option.value === value);
      if (existing) return existing;
      return { value, label: value, isDisabled: false };
    }, [value, options]);

    // react-select kennt kein `aria-describedby`: das Eingabefeld bekommt es über eine eigene Teilkomponente.
    const describedInput = React.useMemo(
      () =>
        ariaDescribedBy
          ? (props: InputProps<SelectOption, false>) => (
              <selectParts.Input
                {...props}
                aria-describedby={ariaDescribedBy}
              />
            )
          : null,
      [ariaDescribedBy],
    );

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
      components: {
        ...(describedInput ? { Input: describedInput } : {}),
        ...(menuScrollKey || menuHeader ? { MenuList: MenuListWithMemory } : {}),
      },
      ...({ menuScrollKey, menuHeader } as MenuExtras),
      isDisabled: disabled,
      isLoading,
      onBlur,
      isClearable,
      styles: compactMenuWidth
        ? {
            ...selectStyles,
            menu: (base, state) => ({
              ...selectStyles.menu!(base, state),
              minWidth: compactMenuWidth,
            }),
            // Rechtsbündig am Knopf: der Knopf steht rechts in der Leiste,
            // eine linksbündige Liste liefe aus dem Fenster.
            menuPortal: (base, state) => {
              const b = selectStyles.menuPortal!(base, state) as {
                left?: number;
                width?: number;
              };
              const w = Math.max(b.width ?? 0, compactMenuWidth);
              return {
                ...b,
                width: w,
                left: Math.max(8, (b.left ?? 0) + (b.width ?? 0) - w),
              };
            },
            valueContainer: (base) => ({ ...base, padding: "0 4px 0 8px" }),
            indicatorSeparator: () => ({ display: "none" }),
            dropdownIndicator: (base, state) => ({
              ...selectStyles.dropdownIndicator!(base, state),
              padding: "6px 6px 6px 0",
            }),
          }
        : selectStyles,
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
