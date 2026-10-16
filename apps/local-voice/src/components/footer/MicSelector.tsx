import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, ChevronDown, Mic } from "lucide-react";
import { useSettings } from "../../hooks/useSettings";
import { MicLevelMeter } from "../settings/MicLevelMeter";

/**
 * Which microphone the app will record from, in the status bar next to the
 * model — the two facts that decide whether a dictation works at all, and the
 * two you want to check without opening settings.
 *
 * Opening the menu shows the live level, because "which device is selected"
 * and "does that device hear me" are the same question in practice. The meter
 * only exists while the menu is open: it holds a real microphone stream, and
 * keeping that open permanently would light the recording indicator of every
 * webcam in the room for no reason.
 */
export const MicSelector: React.FC = () => {
  const { t } = useTranslation();
  const {
    getSetting,
    updateSetting,
    isUpdating,
    audioDevices,
    refreshAudioDevices,
  } = useSettings();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  const selected =
    getSetting("selected_microphone") === "default"
      ? "Default"
      : getSetting("selected_microphone") || "Default";

  useEffect(() => {
    if (!open) return;
    // The device list goes stale as headsets come and go; refresh on open
    // rather than polling.
    void refreshAudioDevices();
    const onClickOutside = (event: MouseEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) {
        setOpen(false);
      }
    };
    const onEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onClickOutside);
    document.addEventListener("keydown", onEscape);
    return () => {
      document.removeEventListener("mousedown", onClickOutside);
      document.removeEventListener("keydown", onEscape);
    };
  }, [open, refreshAudioDevices]);

  const label = selected === "Default" ? t("micSelector.default") : selected;

  return (
    <div className="relative" ref={ref}>
      <button
        type="button"
        onClick={() => setOpen((wasOpen) => !wasOpen)}
        aria-expanded={open}
        title={t("micSelector.title", { device: label })}
        className="flex items-center gap-2 hover:text-text/80 transition-colors cursor-pointer"
      >
        <Mic className="w-3 h-3 shrink-0" />
        <span className="max-w-28 truncate">{label}</span>
        <ChevronDown
          className={`w-3 h-3 transition-transform ${open ? "rotate-180" : ""}`}
        />
      </button>

      {open && (
        <div className="absolute bottom-full mb-2 start-0 w-72 bg-background border border-mid-gray/40 rounded-lg shadow-lg z-50 overflow-hidden">
          <div className="px-3 pt-3 pb-2 border-b border-mid-gray/20">
            <p className="text-xs font-medium text-text/50 mb-2">
              {t("micSelector.level")}
            </p>
            <MicLevelMeter compact />
          </div>
          <div className="max-h-56 overflow-y-auto py-1">
            {audioDevices.length === 0 ? (
              <p className="px-3 py-2 text-xs text-text/50">
                {t("settings.sound.microphone.loading")}
              </p>
            ) : (
              audioDevices.map((device) => {
                const isActive = device.name === selected;
                return (
                  <button
                    key={device.name}
                    type="button"
                    disabled={isUpdating("selected_microphone")}
                    onClick={() => {
                      void updateSetting("selected_microphone", device.name);
                      setOpen(false);
                    }}
                    className={`flex w-full items-center gap-2 px-3 py-1.5 text-start text-xs transition-colors cursor-pointer disabled:cursor-not-allowed ${
                      isActive
                        ? "bg-logo-primary/20 text-logo-primary font-semibold"
                        : "hover:bg-mid-gray/10"
                    }`}
                  >
                    <Check
                      className={`w-3 h-3 shrink-0 ${isActive ? "" : "invisible"}`}
                    />
                    <span className="truncate">
                      {device.name === "Default"
                        ? t("micSelector.default")
                        : device.name}
                    </span>
                  </button>
                );
              })
            )}
          </div>
        </div>
      )}
    </div>
  );
};
