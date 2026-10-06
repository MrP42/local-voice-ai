import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ShieldAlert, ShieldCheck, ShieldX } from "lucide-react";
import type { ShieldLevel } from "@/bindings";
import {
  COMPLIANCE_CHANGED,
  useComplianceStore,
} from "@/stores/complianceStore";
import { ModelBadges, reasonsText } from "./ModelBadges";

const COLOR: Record<ShieldLevel, string> = {
  green: "text-green-500",
  yellow: "text-yellow-500",
  red: "text-red-500",
};

const ICON: Record<ShieldLevel, React.FC<{ className?: string }>> = {
  green: ShieldCheck,
  yellow: ShieldAlert,
  red: ShieldX,
};

/**
 * Das Schild unten rechts: grün = sicher und regelkonform, gelb = erlaubt mit
 * Bedingung oder kleineres Problem, rot = Verstoß oder gesperrter Aufruf.
 * Ein Klick erklärt jeden Prüfpunkt.
 */
export const ComplianceShield: React.FC = () => {
  const { t } = useTranslation();
  const { status, refresh } = useComplianceStore();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  // Alle 20 s und sofort nach jeder Änderung an Regelwerk oder Modell.
  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), 20_000);
    const onChange = () => void refresh();
    window.addEventListener(COMPLIANCE_CHANGED, onChange);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener(COMPLIANCE_CHANGED, onChange);
    };
  }, [refresh]);

  useEffect(() => {
    if (!open) return;
    void refresh();
    const close = (event: MouseEvent) => {
      if (!ref.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [open, refresh]);

  if (!status) return null;
  const Icon = ICON[status.level];
  const summary = t(`compliance.shield.${status.level}`);

  return (
    <div className="relative" ref={ref}>
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className={`flex items-center ${COLOR[status.level]} hover:opacity-80`}
        title={summary}
        aria-label={summary}
        aria-expanded={open}
        data-compliance-shield={status.level}
      >
        <Icon className="h-4 w-4" />
      </button>
      {open && (
        <div
          className="absolute bottom-full end-0 mb-2 w-80 max-h-[70vh] overflow-y-auto rounded-lg border border-mid-gray/20 bg-background p-3 shadow-lg z-50"
          role="dialog"
          aria-label={t("compliance.shield.title")}
          data-compliance-panel
        >
          <div
            className={`flex items-center gap-2 font-semibold ${COLOR[status.level]}`}
          >
            <Icon className="h-4 w-4" />
            <span>{summary}</span>
          </div>
          <ul className="mt-2 space-y-2">
            {status.checks.map((check) => {
              const CheckIcon = ICON[check.level];
              return (
                <li
                  key={check.id}
                  className="flex items-start gap-2 text-xs text-text/80"
                  data-check={check.id}
                  data-level={check.level}
                >
                  <CheckIcon
                    className={`mt-0.5 h-3.5 w-3.5 shrink-0 ${COLOR[check.level]}`}
                  />
                  <span className="min-w-0">
                    {t(`compliance.checks.${check.id}`, {
                      detail: check.detail ?? "",
                      defaultValue: check.id,
                    })}
                    {check.id.startsWith("model_") &&
                      check.id !== "model_none" &&
                      status.active && (
                        <span className="mt-1 flex items-center gap-1.5 text-text/60">
                          <ModelBadges assessment={status.active} />
                          <span>{reasonsText(t, status.active)}</span>
                        </span>
                      )}
                  </span>
                </li>
              );
            })}
          </ul>
          <p className="mt-3 border-t border-mid-gray/20 pt-2 text-[11px] text-text/50">
            {t("compliance.shield.where")}
          </p>
          {status.active?.sources && status.active.sources.length > 0 && (
            <p className="mt-1 text-[11px] text-text/50">
              {t("compliance.shield.sources", {
                date: status.active.checked ?? "",
              })}
              <span className="block break-all">
                {status.active.sources.join(" · ")}
              </span>
            </p>
          )}
          <p className="mt-1 text-[11px] text-text/40">
            {t("compliance.shield.disclaimer")}
          </p>
        </div>
      )}
    </div>
  );
};

export default ComplianceShield;
