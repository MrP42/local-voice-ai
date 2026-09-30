import { useRef } from "react";

export interface TabListItem<T extends string> {
  id: T;
  label: string;
}

/**
 * Reiterleiste im Stil der Einstellungen (AppSettings.tsx): Unterstrich statt
 * Pille, `role="tablist"`/`"tab"`, ein Tabstopp (roving tabIndex), Pfeiltasten
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
}: {
  tabs: TabListItem<T>[];
  value: T;
  onChange: (id: T) => void;
  compact?: boolean;
  ariaLabel?: string;
  /** Rahmen und Abstände der Leiste, z. B. eine Unterlinie unter allen Reitern. */
  className?: string;
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
    <div
      ref={listRef}
      role="tablist"
      aria-label={ariaLabel}
      className={`flex gap-1 ${className}`}
    >
      {tabs.map((tab, index) => {
        const active = tab.id === value;
        return (
          <button
            key={tab.id}
            type="button"
            role="tab"
            aria-selected={active}
            tabIndex={active ? 0 : -1}
            onClick={() => onChange(tab.id)}
            onKeyDown={(event) => onKeyDown(event, index)}
            className={`${
              compact ? "min-h-9 px-2 py-1" : "min-h-11 px-3 py-2"
            } first:pl-0 text-sm font-medium border-b-2 cursor-pointer whitespace-nowrap transition-colors ${
              active
                ? "border-logo-primary text-text"
                : "border-transparent text-text/60 hover:text-text"
            }`}
          >
            {tab.label}
          </button>
        );
      })}
    </div>
  );
}
