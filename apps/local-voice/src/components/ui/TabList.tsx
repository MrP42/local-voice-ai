import { useRef } from "react";
import { Tab, TabList as FluentTabList } from "@fluentui/react-components";

export interface TabListItem<T extends string> {
  id: T;
  label: string;
}

/**
 * Reiterleiste als Fluent TabList (Leitlinie §3): Unterstrich statt Pille, `role="tablist"`/`"tab"`, ein Tabstopp (roving tabIndex), Pfeiltasten
 * sowie Pos1/Ende, in RTL-Oberflächen gespiegelt. Ein Reiter-Stil für die ganze
 * App: die Vorlesen-Seite hatte drei (Unterstrich, graue Pillen, gelbe Pillen).
 *
 * `compact` ist für schmale Leisten gedacht: gleiche Bildsprache, aber nur
 * ~34 statt ~41 px hoch, damit die Leiste neben den Symbolknöpfen nicht
 * aufgebläht wirkt.
 */
export function TabList<T extends string>({
  tabs,
  value,
  onChange,
  compact = false,
  ariaLabel,
  className = "",
  idPrefix,
  controls,
}: {
  tabs: TabListItem<T>[];
  value: T;
  onChange: (id: T) => void;
  compact?: boolean;
  ariaLabel?: string;
  /** Rahmen und Abstände der Leiste, z. B. eine Unterlinie unter allen Reitern. */
  className?: string;
  /** Gesetzt: Reiter bekommen die id `<idPrefix>-<id>` (für aria-labelledby). */
  idPrefix?: string;
  /** id des Reiterinhalts (aria-controls). */
  controls?: string;
}) {
  const listRef = useRef<HTMLDivElement>(null);

  const onKeyDown = (
    event: React.KeyboardEvent<HTMLButtonElement>,
    index: number,
  ) => {
    const keys = ["ArrowRight", "ArrowLeft", "Home", "End"];
    if (!keys.includes(event.key)) return;
    event.preventDefault();
    const rtl =
      event.currentTarget.closest("[dir]")?.getAttribute("dir") === "rtl";
    const step = (event.key === "ArrowRight" ? 1 : -1) * (rtl ? -1 : 1);
    const next =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? tabs.length - 1
          : (index + step + tabs.length) % tabs.length;
    onChange(tabs[next].id);
    // Der Fokus folgt der Auswahl; das Element existiert schon, auch wenn sein
    // tabIndex erst nach dem nächsten Render auf 0 springt.
    listRef.current
      ?.querySelectorAll<HTMLButtonElement>('[role="tab"]')
      [next]?.focus();
  };

  return (
    <FluentTabList
      ref={listRef}
      aria-label={ariaLabel}
      selectedValue={value}
      onTabSelect={(_e, data) => onChange(data.value as T)}
      size={compact ? "small" : "medium"}
      className={`lv-tablist ${className}`}
    >
      {tabs.map((tab, index) => (
        <Tab
          key={tab.id}
          value={tab.id}
          id={idPrefix ? `${idPrefix}-${tab.id}` : undefined}
          aria-controls={controls}
          onKeyDown={(event) => onKeyDown(event, index)}
        >
          {tab.label}
        </Tab>
      ))}
    </FluentTabList>
  );
}
