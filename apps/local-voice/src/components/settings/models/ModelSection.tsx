import React from "react";
import { ChevronDown, ChevronRight } from "lucide-react";
import { usePersistentState } from "@/hooks/usePersistentState";

/**
 * Eine Gruppe der Modellseite (Installiert, Zum Laden, Modellordner, …).
 * Zuklappbar; der Zustand wird je Gruppe gemerkt. Mit `collapsible={false}`
 * eine feste Gruppe (das Installierte soll man nicht suchen müssen).
 */
export const ModelSection: React.FC<{
  id: string;
  title: string;
  count?: number;
  /** Kurzinfo neben dem Titel, z. B. "9 laden nicht". */
  summary?: React.ReactNode;
  /** Rechts im Kopf, z. B. Filter oder "Neu durchsuchen". */
  actions?: React.ReactNode;
  defaultOpen?: boolean;
  collapsible?: boolean;
  /** Bei einer Suche alles aufklappen, sonst sind Treffer versteckt. */
  forceOpen?: boolean;
  children: React.ReactNode;
}> = ({
  id,
  title,
  count,
  summary,
  actions,
  defaultOpen = true,
  collapsible = true,
  forceOpen = false,
  children,
}) => {
  const [stored, setStored] = usePersistentState<"1" | "0">(
    `models.section.${id}`,
    defaultOpen ? "1" : "0",
    (v) => v === "1" || v === "0",
  );
  const open = !collapsible || forceOpen || stored === "1";
  const header = (
    <>
      {collapsible &&
        (open ? (
          <ChevronDown className="h-4 w-4 text-text/40" />
        ) : (
          <ChevronRight className="h-4 w-4 text-text/40" />
        ))}
      <span className="text-xs font-semibold uppercase tracking-wide text-text/60">
        {title}
      </span>
      {count !== undefined && (
        <span className="text-xs tabular-nums text-text/40">{count}</span>
      )}
      {summary && <span className="text-xs text-text/40">· {summary}</span>}
    </>
  );
  return (
    <section className="space-y-1.5" data-model-section={id}>
      <div className="flex min-h-8 items-center justify-between gap-2">
        {collapsible ? (
          <button
            type="button"
            onClick={() => setStored(open ? "0" : "1")}
            aria-expanded={open}
            className="flex items-center gap-1.5 rounded focus:outline-none focus-visible:ring-1 focus-visible:ring-logo-primary"
          >
            {header}
          </button>
        ) : (
          <div className="flex items-center gap-1.5">{header}</div>
        )}
        {actions && open && (
          <div className="flex items-center gap-2">{actions}</div>
        )}
      </div>
      {open && (
        <div className="overflow-hidden rounded-lg border border-mid-gray/20">
          {children}
        </div>
      )}
    </section>
  );
};

export default ModelSection;
