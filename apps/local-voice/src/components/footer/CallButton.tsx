import React from "react";
import { useTranslation } from "react-i18next";
import { Phone } from "lucide-react";
import { Tooltip as FluentTooltip } from "@fluentui/react-components";
import { useSettings } from "../../hooks/useSettings";
import { useOsType } from "@/hooks/useOsType";
import { formatKeyCombination } from "@/lib/utils/keyboard";
import { useVoiceCallStore } from "@/stores/voiceCallStore";

/**
 * Telefon in der Fußleiste, direkt hinter dem Mikrofon: grau, solange kein
 * Gespräch läuft, grün, sobald eines läuft. Ein Klick startet das feste
 * Hauptgespräch (wie das Tastenkürzel) oder beendet das laufende Gespräch.
 */
export const CallButton: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const osType = useOsType();
  const state = useVoiceCallStore((s) => s.state);
  const switching = useVoiceCallStore((s) => s.switching);
  const running = state !== "idle";

  const binding = getSetting("bindings")?.voice_call?.current_binding ?? "";
  const keys = binding ? formatKeyCombination(binding, osType) : "";
  const label = running
    ? t("voiceCall.footer.phone.end")
    : keys
      ? t("voiceCall.footer.phone.startWithKeys", { keys })
      : t("voiceCall.footer.phone.start");

  const onClick = () => {
    const s = useVoiceCallStore.getState();
    if (s.state !== "idle") void s.stopCall();
    else void s.toggleMainCall();
  };

  return (
    <FluentTooltip
      relationship="label"
      withArrow
      positioning="above-end"
      content={label}
    >
      <button
        type="button"
        onClick={onClick}
        disabled={switching}
        aria-pressed={running}
        data-call-button
        data-state={running ? "active" : "idle"}
        className={`lv-status-btn lv-call-btn ${running ? "is-active" : ""}`}
      >
        <Phone
          className="w-3.5 h-3.5 shrink-0"
          aria-hidden="true"
          fill={running ? "currentColor" : "none"}
        />
      </button>
    </FluentTooltip>
  );
};
