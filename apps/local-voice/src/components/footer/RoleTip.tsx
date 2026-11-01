import React from "react";
import { Tooltip as FluentTooltip } from "@fluentui/react-components";

interface RoleTipProps {
  /** Rolle des Modells, fett („Spracherkennung“). */
  role: string;
  /** Name des aktuell gewählten Modells. */
  name: string;
  /** Wofür das Modell zuständig ist, ein Satz. */
  purpose: string;
  /** Zusatzzeile, etwa eine Statusmeldung. */
  extra?: string;
  children: React.ReactElement;
}

/**
 * Tooltip der Modell-Auslöser in der Fußleiste: Rolle fett, darunter der
 * Modellname und wozu das Modell dient. Das Symbol allein sagt nicht, welches
 * Modell wofür zuständig ist.
 */
export const RoleTip: React.FC<RoleTipProps> = ({
  role,
  name,
  purpose,
  extra,
  children,
}) => (
  <FluentTooltip
    relationship="description"
    withArrow
    positioning="above-start"
    content={
      <div className="lv-roletip">
        <strong>{role}</strong>
        <div>{name}</div>
        <div className="lv-roletip__purpose">{purpose}</div>
        {extra && <div className="lv-roletip__purpose">{extra}</div>}
      </div>
    }
  >
    {children}
  </FluentTooltip>
);
