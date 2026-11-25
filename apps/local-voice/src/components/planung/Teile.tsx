import React, { useId } from "react";
import { useTranslation } from "react-i18next";
import { Badge } from "@fluentui/react-components";
import {
  CalendarCheck,
  CalendarClock,
  CalendarX,
  CircleAlert,
  CircleCheck,
  FileEdit,
  Send,
  TriangleAlert,
  Unlink,
  type LucideIcon,
} from "lucide-react";
import type { PlanLink } from "@/bindings";
import type { StatusMarke, Ton } from "./logik";

const FARBE = {
  neutral: "informative",
  info: "brand",
  success: "success",
  warning: "warning",
  danger: "danger",
} as const satisfies Record<Ton, string>;

const SYMBOL: Record<StatusMarke["schluessel"], LucideIcon> = {
  entwurf: FileEdit,
  uebergeben: Send,
  verknuepft: CalendarCheck,
  verschoben: CalendarClock,
  abgesagt: CalendarX,
  durchgefuehrt: CircleCheck,
  protokolliert: CircleCheck,
  versendet: CircleCheck,
  konflikt: TriangleAlert,
  verwaist: Unlink,
  fremd: CircleAlert,
};

/** Eine Statusmarke: Symbol und Text, nie nur Farbe. */
export const Statusmarke: React.FC<{
  marke: StatusMarke;
  /** Abweichende Test-ID (z. B. im Kopf der Planung), sonst `planung-status-<schluessel>`. */
  testId?: string;
  className?: string;
}> = ({ marke, testId, className }) => {
  const { t } = useTranslation();
  const Symbol = SYMBOL[marke.schluessel];
  return (
    <Badge
      appearance="tint"
      shape="rounded"
      color={FARBE[marke.ton]}
      icon={<Symbol size={14} aria-hidden="true" />}
      className={className}
      data-testid={testId ?? `planung-status-${marke.schluessel}`}
    >
      {t(`planung.status.${marke.schluessel}`)}
    </Badge>
  );
};

/** Beschriftung über einem Feld, mit Hilfetext, der mit dem Feld verbunden ist. */
export const Feld: React.FC<{
  label: string;
  hilfe?: string;
  children: (ids: {
    id: string;
    hilfeId: string | undefined;
  }) => React.ReactNode;
  className?: string;
}> = ({ label, hilfe, children, className = "" }) => {
  const id = useId();
  const hilfeId = hilfe ? `${id}-hilfe` : undefined;
  return (
    <div className={`plg-feld ${className}`.trim()}>
      <label htmlFor={id} className="plg-feld__label">
        {label}
      </label>
      {children({ id, hilfeId })}
      {hilfe && (
        <p id={hilfeId} className="plg-feld__hilfe">
          {hilfe}
        </p>
      )}
    </div>
  );
};

export const Abschnitt: React.FC<{
  titel: string;
  hilfe?: string;
  children: React.ReactNode;
  testId?: string;
}> = ({ titel, hilfe, children, testId }) => {
  const id = useId();
  return (
    <section
      className="plg-abschnitt"
      aria-labelledby={id}
      data-testid={testId}
    >
      <h2 id={id} className="plg-abschnitt__titel">
        {titel}
      </h2>
      {hilfe && <p className="plg-abschnitt__hilfe">{hilfe}</p>}
      {children}
    </section>
  );
};

/** Datum und Uhrzeit eines UTC-Zeitpunkts (ms) in der Sprache der App. */
export function zeitText(
  ms: number | null | undefined,
  sprache: string,
): string {
  if (ms == null || !Number.isFinite(ms)) return "";
  try {
    return new Intl.DateTimeFormat(sprache, {
      weekday: "short",
      day: "2-digit",
      month: "2-digit",
      year: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    }).format(new Date(ms));
  } catch {
    return new Date(ms).toLocaleString();
  }
}

/** Beginn und Ende des Termins als eine Zeile; ohne Termin leer. */
export function terminText(link: PlanLink | null, sprache: string): string {
  if (!link || link.starts_at == null) return "";
  const von = zeitText(link.starts_at, sprache);
  if (link.ends_at == null) return von;
  const ende = new Intl.DateTimeFormat(sprache, {
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(link.ends_at));
  return `${von} – ${ende}`;
}
