import React, { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  useHelpDockSlot,
  usePageHeaderSlot,
  useTitleContextSlot,
} from "../shell/PageHeaderSlot";
import { useSectionActive } from "../shell/KeepAlive";
import { useTranslation } from "react-i18next";
import { Info, X } from "lucide-react";
import { HelpPanel } from "../help/HelpPanel";
import { registerPageHelp } from "../help/pageHelp";
import { IconButton } from "./IconButton";
import {
  SearchBox,
  Tooltip as FluentTooltip,
} from "@fluentui/react-components";

/** Ab dieser Breite stehen ⓘ, Suche und Aktionen in der Titelleiste; darunter
    reicht ihr Platz neben Hamburger und Fensterknöpfen nicht, dann bleiben sie
    als schmale Zeile über dem Inhalt. */
const TITLEBAR_TOOLS_QUERY = "(min-width: 768px)";

function useTitlebarTools(): boolean {
  const [wide, setWide] = useState(
    () =>
      typeof window === "undefined" ||
      window.matchMedia(TITLEBAR_TOOLS_QUERY).matches,
  );
  useEffect(() => {
    const query = window.matchMedia(TITLEBAR_TOOLS_QUERY);
    const onChange = () => setWide(query.matches);
    onChange();
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  }, []);
  return wide;
}

const IS_MAC =
  typeof navigator !== "undefined" &&
  /Mac|iPhone|iPad/.test(navigator.platform);

/** Suche des Moduls (ADR-0027 E5): nur gesetzt, wenn das Modul sie anbietet. */
export interface PageShellSearch {
  value: string;
  onChange: (value: string) => void;
  /** Platzhalter, z. B. „Verlauf durchsuchen“. */
  placeholder: string;
}

interface PageShellProps {
  /** Seitentitel: unsichtbare Überschrift (h1) und Name der Werkzeugleiste
      in der Titelleiste; sichtbar ist er nur in der Navigation. */
  title: string;
  /** Ein Satz, was man hier tut: nur als Tooltip am ⓘ neben dem Titel
      (ADR-0027 E4), keine eigene Zeile. */
  description?: string;
  /** Rechts in der Titelleiste: Schalter, Statussymbole, ein Knopf. Kein Menue. */
  actions?: React.ReactNode;
  /** Suchfeld in der Titelleiste, Strg+K springt hinein (ADR-0027 E5). Ohne
      diese Angabe hat das Modul weder Feld noch Kürzel. */
  search?: PageShellSearch;
  /** Bereich der Kontexthilfe (`src/content/help/<bereich>.<sprache>.md`).
      Gesetzt: Hilfe als Spalte rechts neben dem Inhalt, geöffnet über das
      Profilmenü ▸ Hilfe oder F1 (ADR-0027 E4: genau ein Hilfe-Einstieg).
      Vorlesen bringt seine Hilfe in der eigenen rechten Leiste mit und
      setzt das nicht. */
  help?: string;
  /** Arbeitsflaeche: nimmt die volle Hoehe, der Inhalt scrollt in seinen
      Spalten. Listen und Einstellungen lassen das aus und
      scrollen als Ganzes. */
  fill?: boolean;
  /** Gegenstand des Moduls (z. B. das geöffnete Buch): steht in der Mitte
      der Titelleiste, ohne Platz (Tests ohne Rahmen) über dem Inhalt. */
  context?: { label: string; text: string; detail?: string };
  children: React.ReactNode;
}

/**
 * Der gemeinsame Rahmen aller Inhaltsseiten: unsichtbare Überschrift (h1 für
 * Screenreader), Bedienelemente des Moduls in der Titelleiste, darunter der
 * Inhalt ab der ersten Zeile. Eine sichtbare Titelzeile gibt es nicht mehr
 * (Patrick 02.11.): der Modulname steht in der Navigation links.
 */
export const PageShell: React.FC<PageShellProps> = ({
  title,
  description,
  actions,
  search,
  help,
  fill = false,
  context,
  children,
}) => {
  const { t } = useTranslation();
  const [helpOpen, setHelpOpen] = useState(false);
  const dock = useHelpDockSlot();
  const headerSlot = usePageHeaderSlot();
  const slot = useTitlebarTools() ? headerSlot : null;
  const kontextSlot = useTitleContextSlot();
  const searchRef = useRef<HTMLInputElement>(null);
  const hasSearch = !!search;
  // Erhaltene, aber verdeckte Module (KeepAlive) hören auf keine Taste.
  const active = useSectionActive();
  // Strg+K (⌘K) nur in Modulen mit Suche (ADR-0027 E5).
  useEffect(() => {
    if (!hasSearch || !active) return;
    const onKey = (event: KeyboardEvent) => {
      if (
        (IS_MAC ? event.metaKey : event.ctrlKey) &&
        !event.altKey &&
        event.key.toLowerCase() === "k"
      ) {
        event.preventDefault();
        searchRef.current?.focus();
        searchRef.current?.select();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [hasSearch, active]);
  // Profilmenü ▸ Hilfe und F1 schalten die Seitenhilfe über diese Anmeldung.
  useEffect(
    () =>
      help && active
        ? registerPageHelp(() => setHelpOpen((open) => !open))
        : undefined,
    [help, active],
  );
  // Keine Titelzeile im Modul (Patrick 02.11.): Der Name steht in der
  // Navigation links. Für Screenreader und Sprunglisten bleibt er als
  // unsichtbare Überschrift; ⓘ, Suche und Aktionen wandern in die Titelleiste.
  const heading = (
    <h1
      id={active ? "page-title" : undefined}
      className="page-shell__title sr-only"
    >
      {title}
    </h1>
  );
  const hasTools = !!(description || actions || search);
  const tools = hasTools ? (
    <div
      className="page-shell__head page-shell__actions"
      role="group"
      aria-label={title}
      data-testid="page-tools"
    >
      {description && (
        <FluentTooltip
          content={description}
          relationship="description"
          withArrow
        >
          {/* Ein Knopf, damit Tastatur und Screenreader ihn als Bedien-
              element finden; der Tooltip hängt die Kurzbeschreibung per
              aria-describedby an (relationship="description"). */}
          <button
            type="button"
            className="page-shell__info"
            aria-label={t("shell.info")}
            data-testid="page-info"
          >
            <Info width={16} height={16} aria-hidden="true" />
          </button>
        </FluentTooltip>
      )}
      {search && (
        <div className="lv-search">
          <SearchBox
            ref={searchRef}
            size="medium"
            placeholder={search.placeholder}
            aria-label={search.placeholder}
            value={search.value}
            onChange={(_event, data) => search.onChange(data.value)}
            data-module-search
          />
        </div>
      )}
      {actions}
    </div>
  ) : null;
  const helpAside = (className: string) =>
    help ? (
      <aside className={className} aria-label={t("help.title")}>
        <div className="flex items-center justify-between">
          <span className="text-xs font-semibold text-text/50">
            {t("help.title")}
          </span>
          <IconButton
            onClick={() => setHelpOpen(false)}
            label={t("help.close")}
            icon={<X width={14} height={14} />}
          />
        </div>
        <HelpPanel section={help} />
      </aside>
    ) : null;
  return (
    <section
      className={`page-shell w-full ${fill ? "page-shell--fill flex flex-col gap-4" : "space-y-4"}`}
      aria-labelledby="page-title"
    >
      {heading}
      {context &&
        (kontextSlot ? (
          createPortal(<PageContext {...context} />, kontextSlot)
        ) : (
          <PageContext {...context} />
        ))}
      {tools && (slot ? createPortal(tools, slot) : tools)}
      {help && helpOpen && !dock ? (
        <div
          className={`flex gap-4 ${fill ? "flex-1 min-h-0 items-stretch" : "items-start"}`}
        >
          <div className={`flex-1 min-w-0 ${fill ? "min-h-0" : "space-y-4"}`}>
            {children}
          </div>
          {helpAside(
            `w-72 shrink-0 space-y-2 border-s border-mid-gray/20 ps-4 ${
              fill ? "min-h-0 overflow-y-auto" : ""
            }`,
          )}
        </div>
      ) : fill ? (
        <div className="flex-1 min-h-0">{children}</div>
      ) : (
        children
      )}
      {help && helpOpen && dock
        ? createPortal(helpAside("lv-helpdock__panel space-y-2"), dock)
        : null}
    </section>
  );
};

/** Gegenstand in der Titelleiste: Name kräftig, Zusatz (Ansicht) leiser. */
const PageContext: React.FC<{
  label: string;
  text: string;
  detail?: string;
}> = ({ label, text, detail }) => (
  <span
    className="page-shell__context"
    role="status"
    aria-label={label}
    title={detail ? `${text} · ${detail}` : text}
    data-testid="page-context"
  >
    <span className="page-shell__context-text">{text}</span>
    {detail && <span className="page-shell__context-detail">{detail}</span>}
  </span>
);
