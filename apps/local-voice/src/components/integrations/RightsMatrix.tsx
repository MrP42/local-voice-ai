import React from "react";
import { useTranslation } from "react-i18next";
import { Lock } from "lucide-react";
import type { CallerMode, CapabilityView, GrantMode } from "@/bindings";
import { capabilityKey, GRANT_MODES } from "./model";

interface RightsMatrixProps {
  capabilities: CapabilityView[];
  busy: boolean;
  /** `mode = null`: zurueck auf die Vorgabe. */
  onChange: (
    capability: CapabilityView["capability"],
    caller: CallerMode["caller"],
    mode: GrantMode | null,
  ) => void;
}

/** Was in der Matrix gewaehlt erscheint: gespeichert, sonst die Vorgabe. */
const configured = (m: CallerMode): GrantMode => m.stored ?? m.default_mode;

/**
 * Rechte-Matrix: je Faehigkeit und Aufrufer ein Recht (aus / fragen / erlaubt).
 * Angezeigt wird das EINGESTELLTE Recht; gilt etwas anderes (die Richtung der
 * Integration sperrt die Faehigkeit, die Integration ist aus), steht darunter,
 * warum. „Aufnahme starten“ bietet „erlaubt“ nie an (Einwilligung, § 201 StGB).
 */
export const RightsMatrix: React.FC<RightsMatrixProps> = ({
  capabilities,
  busy,
  onChange,
}) => {
  const { t } = useTranslation();

  return (
    <div className="space-y-3" data-testid="rights-matrix">
      {capabilities.map((cap) => {
        const base = capabilityKey(cap.capability);
        const blockedReasons = Array.from(
          new Set(
            cap.modes
              .filter((m) => m.off_reason === "direction_blocks")
              .map((m) => m.off_reason as string),
          ),
        );
        return (
          <fieldset
            key={cap.capability}
            className="min-w-0 rounded-lg border border-mid-gray/20 p-3"
            data-testid={`capability-${cap.capability}`}
          >
            <legend className="px-1 text-sm font-medium">
              {t(`${base}.title`)}
              <span className="ms-2 rounded-full bg-mid-gray/20 px-2 py-0.5 text-xs font-normal text-text-muted">
                {cap.writes
                  ? t("integrations.matrix.writes")
                  : t("integrations.matrix.reads")}
              </span>
            </legend>
            <p className="mb-2 text-xs text-text-muted">
              {t(`${base}.description`)}
            </p>
            {blockedReasons.map((reason) => (
              <p
                key={reason}
                className="mb-2 text-xs text-status-amber"
                data-testid="capability-blocked"
              >
                {t(`integrations.matrix.blocked.${reason}`)}
              </p>
            ))}
            {cap.never_allow && (
              <p className="mb-2 flex items-start gap-1 text-xs text-text-muted">
                <Lock size={12} className="mt-0.5 shrink-0" aria-hidden="true" />
                {t("integrations.matrix.neverAllow")}
              </p>
            )}
            <div className="grid gap-3 md:grid-cols-3">
              {cap.modes.map((m) => {
                const current = configured(m);
                const callerLabel = t(`integrations.callers.${m.caller}`);
                return (
                  <div key={m.caller} className="min-w-0">
                    <div className="mb-1 text-xs font-medium text-text">
                      {callerLabel}
                    </div>
                    <div
                      role="radiogroup"
                      aria-label={`${t(`${base}.title`)}: ${callerLabel}`}
                      className="inline-flex max-w-full overflow-hidden rounded-lg border border-mid-gray/30"
                    >
                      {GRANT_MODES.map((mode) => {
                        const selected = current === mode;
                        const disabled =
                          busy || (cap.never_allow && mode === "allow");
                        return (
                          <button
                            key={mode}
                            type="button"
                            role="radio"
                            aria-checked={selected}
                            disabled={disabled}
                            title={
                              cap.never_allow && mode === "allow"
                                ? t("integrations.matrix.neverAllow")
                                : undefined
                            }
                            data-testid={`grant-${cap.capability}-${m.caller}-${mode}`}
                            onClick={() => {
                              if (!selected) onChange(cap.capability, m.caller, mode);
                            }}
                            className={`min-h-9 min-w-[4.25rem] cursor-pointer px-2 py-1 text-xs font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50 focus:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary ${
                              selected
                                ? mode === "off"
                                  ? "bg-mid-gray/30 text-text"
                                  : "bg-logo-primary text-on-accent"
                                : "bg-transparent text-text hover:bg-mid-gray/15"
                            }`}
                          >
                            {t(`integrations.modes.${mode}`)}
                          </button>
                        );
                      })}
                    </div>
                    {m.effective !== current && (
                      <p
                        className="mt-1 text-xs text-text-muted"
                        data-testid="grant-effective"
                      >
                        {t("integrations.matrix.effective", {
                          mode: t(`integrations.modes.${m.effective}`),
                        })}
                      </p>
                    )}
                  </div>
                );
              })}
            </div>
          </fieldset>
        );
      })}
    </div>
  );
};
