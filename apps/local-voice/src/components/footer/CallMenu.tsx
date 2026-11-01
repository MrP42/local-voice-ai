import React, { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, ChevronDown, MessageSquarePlus, Settings } from "lucide-react";
import { useSettings } from "../../hooks/useSettings";
import { useVoiceCallStore } from "@/stores/voiceCallStore";
import { MicLevelMeter } from "../settings/MicLevelMeter";
import { VOICE_MANAGE, useVoiceChoice } from "../settings/tts/useVoiceChoice";
import { useServerPhase } from "../settings/meetings/ReadAloudBar";
import { TEMPO_PRESETS, TEMPOS, tempoOf } from "../voice-call/tempo";

const ITEM =
  "flex w-full items-center gap-2 px-3 py-1.5 text-start text-xs transition-colors cursor-pointer hover:bg-mid-gray/10 disabled:cursor-not-allowed disabled:opacity-50";

const Heading: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <p
    className="px-3 pt-2 pb-1 text-[11px] font-medium text-text/50"
    role="presentation"
  >
    {children}
  </p>
);

const Switch: React.FC<{ on: boolean }> = ({ on }) => (
  <span
    aria-hidden="true"
    className={`ms-auto relative w-8 h-4 rounded-full shrink-0 ${on ? "bg-logo-primary" : "bg-mid-gray/40"}`}
  >
    <span
      className={`absolute top-0.5 w-3 h-3 rounded-full bg-white ${on ? "end-0.5" : "start-0.5"}`}
    />
  </span>
);

/**
 * Pfeil hinter dem Telefon der Fußleiste, solange ein Gespräch läuft oder die
 * Seite Sprachgespräch offen ist: Mikrofon mit Pegel, Stimme, Sprachfluss.
 * Alles Weitere öffnet das Panel der Seite. Aufbau und Schließen wie bei der
 * Mikrofonauswahl.
 */
export const CallMenu: React.FC = () => {
  const { t } = useTranslation();
  const {
    getSetting,
    updateSetting,
    isUpdating,
    audioDevices,
    refreshAudioDevices,
  } = useSettings();
  const turn = useVoiceCallStore((s) => s.config.turn);
  const setConfig = useVoiceCallStore((s) => s.setConfig);
  const openSettings = useVoiceCallStore((s) => s.openSettings);
  const pageOpen = useVoiceCallStore((s) => s.active);
  const hasMain = useVoiceCallStore((s) => s.mainConversationId !== null);
  const phase = useServerPhase();
  const { options, voiceValue, applyVoiceValue, openVoiceManagement } =
    useVoiceChoice(phase);
  const [open, setOpen] = useState(false);
  const [section, setSection] = useState<"voice" | "tempo" | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuId = useId();
  /** Schließen; der Fokus kehrt zum Auslöser zurück (Tastatur). */
  const close = () => {
    setOpen(false);
    triggerRef.current?.focus();
  };

  const mic =
    getSetting("selected_microphone") === "default"
      ? "Default"
      : getSetting("selected_microphone") || "Default";
  const tempo = tempoOf(turn);
  const voiceLabel = options.find((o) => o.value === voiceValue)?.label ?? "–";

  useEffect(() => {
    if (!open) return;
    void refreshAudioDevices();
    menuRef.current?.querySelector<HTMLElement>('[role^="menuitem"]')?.focus();
    const onClickOutside = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node))
        setOpen(false);
    };
    const onEscape = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpen(false);
        triggerRef.current?.focus();
      }
    };
    document.addEventListener("mousedown", onClickOutside);
    document.addEventListener("keydown", onEscape);
    return () => {
      document.removeEventListener("mousedown", onClickOutside);
      document.removeEventListener("keydown", onEscape);
    };
  }, [open, refreshAudioDevices]);

  const onMenuKey = (e: React.KeyboardEvent) => {
    if (e.key === "Tab") {
      close();
      return;
    }
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) return;
    const items = Array.from(
      menuRef.current?.querySelectorAll<HTMLElement>(
        '[role^="menuitem"]:not(:disabled)',
      ) ?? [],
    );
    if (items.length === 0) return;
    e.preventDefault();
    const at = items.indexOf(document.activeElement as HTMLElement);
    const next =
      e.key === "Home"
        ? 0
        : e.key === "End"
          ? items.length - 1
          : (at + (e.key === "ArrowDown" ? 1 : -1) + items.length) %
            items.length;
    items[next].focus();
  };

  const tempoLabel =
    tempo === "custom"
      ? t("voiceCall.tempo.custom")
      : t(`voiceCall.tempo.${tempo}`);

  return (
    <div className="relative" ref={ref}>
      <button
        ref={triggerRef}
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        aria-haspopup="menu"
        title={t("voiceCall.footer.title")}
        aria-label={t("voiceCall.footer.title")}
        className="lv-status-btn lv-call-menu-btn"
      >
        <ChevronDown
          className={`w-3 h-3 transition-transform ${open ? "rotate-180" : ""}`}
          aria-hidden="true"
        />
      </button>

      {open && (
        <div
          ref={menuRef}
          id={menuId}
          role="menu"
          aria-label={t("voiceCall.footer.label")}
          onKeyDown={onMenuKey}
          className="absolute bottom-full mb-2 end-0 w-80 bg-background border border-mid-gray/40 rounded-lg shadow-lg z-50 overflow-hidden pb-1"
        >
          <Heading>{t("voiceCall.footer.microphone")}</Heading>
          <div className="max-h-40 overflow-y-auto">
            {audioDevices.length === 0 ? (
              <p className="px-3 py-2 text-xs text-text/50">
                {t("settings.sound.microphone.loading")}
              </p>
            ) : (
              audioDevices.map((d) => {
                const on = d.name === mic;
                return (
                  <button
                    key={d.name}
                    type="button"
                    role="menuitemradio"
                    aria-checked={on}
                    disabled={isUpdating("selected_microphone")}
                    onClick={() =>
                      void updateSetting("selected_microphone", d.name)
                    }
                    className={`${ITEM} ${on ? "bg-logo-primary/20 text-logo-primary font-semibold" : ""}`}
                  >
                    <Check
                      className={`w-3 h-3 shrink-0 ${on ? "" : "invisible"}`}
                      aria-hidden="true"
                    />
                    <span className="truncate">
                      {d.name === "Default" ? t("micSelector.default") : d.name}
                    </span>
                  </button>
                );
              })
            )}
          </div>
          <div className="px-3 pt-1 pb-2 border-b border-mid-gray/20">
            <MicLevelMeter compact />
          </div>

          <Heading>{t("voiceCall.footer.output")}</Heading>
          <button
            type="button"
            role="menuitem"
            aria-expanded={section === "voice"}
            className={ITEM}
            onClick={() => setSection((s) => (s === "voice" ? null : "voice"))}
          >
            <span>{t("voiceCall.settings.voice")}</span>
            <span className="ms-auto truncate max-w-40 text-text/55">
              {voiceLabel}
            </span>
            <ChevronDown className="w-3 h-3 shrink-0" aria-hidden="true" />
          </button>
          {section === "voice" && (
            <div className="max-h-40 overflow-y-auto bg-mid-gray/5">
              {options.map((o) => {
                const on = o.value === voiceValue;
                return (
                  <button
                    key={o.value}
                    type="button"
                    role="menuitemradio"
                    aria-checked={on}
                    disabled={o.isDisabled}
                    onClick={() => {
                      if (o.value === VOICE_MANAGE) {
                        close();
                        openVoiceManagement();
                      } else applyVoiceValue(o.value);
                    }}
                    className={`${ITEM} ps-6 ${on ? "text-logo-primary font-semibold" : ""}`}
                  >
                    <Check
                      className={`w-3 h-3 shrink-0 ${on ? "" : "invisible"}`}
                      aria-hidden="true"
                    />
                    <span className="truncate">{o.label}</span>
                  </button>
                );
              })}
            </div>
          )}

          <div className="my-1 h-px bg-mid-gray/20" role="separator" />
          <Heading>{t("voiceCall.footer.flow")}</Heading>
          <button
            type="button"
            role="menuitemcheckbox"
            aria-checked={turn.bargeIn}
            className={ITEM}
            onClick={() =>
              setConfig((c) => ({
                ...c,
                turn: { ...c.turn, bargeIn: !c.turn.bargeIn },
              }))
            }
          >
            <span>{t("voiceCall.footer.bargeIn")}</span>
            <Switch on={turn.bargeIn} />
          </button>
          <button
            type="button"
            role="menuitem"
            aria-expanded={section === "tempo"}
            className={ITEM}
            onClick={() => setSection((s) => (s === "tempo" ? null : "tempo"))}
          >
            <span>{t("voiceCall.settings.tempo")}</span>
            <span className="ms-auto text-text/55">{tempoLabel}</span>
            <ChevronDown className="w-3 h-3 shrink-0" aria-hidden="true" />
          </button>
          {section === "tempo" && (
            <div className="bg-mid-gray/5">
              {TEMPOS.map((k) => {
                const on = tempo === k;
                return (
                  <button
                    key={k}
                    type="button"
                    role="menuitemradio"
                    aria-checked={on}
                    onClick={() =>
                      setConfig((c) => ({
                        ...c,
                        turn: { ...c.turn, ...TEMPO_PRESETS[k] },
                      }))
                    }
                    className={`${ITEM} ps-6 ${on ? "text-logo-primary font-semibold" : ""}`}
                  >
                    <Check
                      className={`w-3 h-3 shrink-0 ${on ? "" : "invisible"}`}
                      aria-hidden="true"
                    />
                    {t(`voiceCall.tempo.${k}`)}
                  </button>
                );
              })}
              {tempo === "custom" && (
                <p className="ps-9 pe-3 py-1 text-xs text-text/55">
                  {t("voiceCall.tempo.custom")}
                </p>
              )}
            </div>
          )}

          <div className="my-1 h-px bg-mid-gray/20" role="separator" />
          {hasMain && (
            <button
              type="button"
              role="menuitem"
              className={ITEM}
              onClick={() => {
                close();
                void useVoiceCallStore.getState().newMainConversation();
              }}
            >
              <MessageSquarePlus
                className="w-3 h-3 shrink-0"
                aria-hidden="true"
              />
              {t("voiceCall.footer.newMain")}
            </button>
          )}
          <button
            type="button"
            role="menuitem"
            className={ITEM}
            onClick={() => {
              close();
              // Das Panel gehört zur Seite: läuft das Gespräch ohne sie, erst dorthin.
              if (!pageOpen)
                window.dispatchEvent(
                  new CustomEvent("lv-navigate", {
                    detail: { section: "voice" },
                  }),
                );
              openSettings();
            }}
          >
            <Settings className="w-3 h-3 shrink-0" aria-hidden="true" />
            {t("voiceCall.footer.allSettings")}
          </button>
        </div>
      )}
    </div>
  );
};
