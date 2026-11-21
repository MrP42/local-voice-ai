import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../hooks/useSettings";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { Slider } from "../ui/Slider";
import { Alert } from "../ui/Alert";
import { GUARD_DEFAULT, GUARD_MAX, GUARD_MIN } from "../footer/ResourceMeter";

interface MemoryGuardSettingsProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

/**
 * Speicherschutz als Abschnitt der Einstellungen (lva#654). Dieselben zwei
 * Einstellungen wie im Speicher-Popover der Fußleiste; dorthin fand niemand,
 * der unter Einstellungen suchte. Ziel des Knopfs „Grenze ändern“ in jeder
 * Speicherschutz-Meldung.
 */
export const MemoryGuardSettings: React.FC<MemoryGuardSettingsProps> = ({
  descriptionMode = "tooltip",
  grouped = false,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const enabled = getSetting("memory_guard_enabled") ?? true;
  const percent = getSetting("memory_guard_percent") ?? GUARD_DEFAULT;

  // Der Regler speichert erst nach einer kurzen Pause, nicht bei jedem Schritt.
  const [value, setValue] = useState(percent);
  const timer = useRef<number | undefined>(undefined);
  // Noch nicht gespeicherter Wert: wird beim Verlassen sofort gespeichert.
  const pending = useRef<number | null>(null);
  const save = useRef(updateSetting);
  save.current = updateSetting;
  useEffect(() => setValue(percent), [percent]);
  useEffect(
    () => () => {
      window.clearTimeout(timer.current);
      if (pending.current !== null)
        void save.current("memory_guard_percent", pending.current);
    },
    [],
  );
  const change = (next: number) => {
    setValue(next);
    pending.current = next;
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      pending.current = null;
      void updateSetting("memory_guard_percent", next);
    }, 250);
  };

  return (
    <>
      <ToggleSwitch
        checked={enabled}
        onChange={(checked) =>
          void updateSetting("memory_guard_enabled", checked)
        }
        isUpdating={isUpdating("memory_guard_enabled")}
        label={t("memoryGuard.switch")}
        description={t("memoryGuard.switchDescription")}
        descriptionMode={descriptionMode}
        grouped={grouped}
        testId="memory-guard-settings-switch"
      />
      {enabled ? (
        <Slider
          value={value}
          onChange={(next) => change(Math.round(next))}
          min={GUARD_MIN}
          max={GUARD_MAX}
          step={1}
          label={t("memoryGuard.limit")}
          description={t("memoryGuard.limitDescription")}
          descriptionMode={descriptionMode}
          grouped={grouped}
          formatValue={(v) => `${Math.round(v)} %`}
        />
      ) : (
        <div className="px-4 py-3">
          <Alert variant="warning">{t("resourceMeter.guardOffWarning")}</Alert>
        </div>
      )}
    </>
  );
};
