import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Button as FluentButton,
  MessageBar,
  MessageBarBody,
  Popover,
  PopoverSurface,
  PopoverTrigger,
  Slider as FluentSlider,
  Switch as FluentSwitch,
  Tooltip as FluentTooltip,
} from "@fluentui/react-components";
import { commands, type SystemMemory } from "@/bindings";
import { useSettings } from "../../hooks/useSettings";

/** Wie oft die Fußleiste nachmisst. Drei Sekunden: träge genug, um nichts
 *  zu kosten, schnell genug, um einen Modellstart zu sehen. */
const POLL_MS = 3000;

export const GUARD_MIN = 60;
export const GUARD_MAX = 98;
export const GUARD_DEFAULT = 90;

const gb = (mb: number) => (mb / 1024).toFixed(1).replace(".", ",");

const percentOf = (part: number, total: number) =>
  total > 0 ? Math.min(100, Math.max(0, (part / total) * 100)) : 0;

/** Ab 80 % orange, ab 90 % rot (Systemauslastung der Säule). */
export const levelOf = (percent: number): "ok" | "warn" | "crit" =>
  percent >= 90 ? "crit" : percent >= 80 ? "warn" : "ok";

interface ColumnProps {
  label: string;
  /** Systemauslastung in Prozent (Füllhöhe). */
  system: number;
  /** Anteil der App in Prozent, `null`: nicht messbar. */
  app: number | null;
  /** Schutzgrenze in Prozent, `null`: keine Linie. */
  limit: number | null;
  testId: string;
}

const Column: React.FC<ColumnProps> = ({
  label,
  system,
  app,
  limit,
  testId,
}) => (
  <span className="lv-meter__col" data-meter-col={testId}>
    <span className="lv-meter__bar" aria-hidden="true">
      <span
        className="lv-meter__fill"
        data-level={levelOf(system)}
        style={{ height: `${system}%` }}
      />
      {app !== null && (
        <span className="lv-meter__app" style={{ height: `${app}%` }} />
      )}
      {limit !== null && (
        <span className="lv-meter__limit" style={{ bottom: `${limit}%` }} />
      )}
    </span>
    <span className="lv-meter__label hidden md:inline">{label}</span>
  </span>
);

interface StackProps {
  appMb: number | null;
  usedMb: number;
  totalMb: number;
}

/** Balken App / Andere / Frei. */
const Stack: React.FC<StackProps> = ({ appMb, usedMb, totalMb }) => {
  const app = appMb === null ? 0 : Math.min(appMb, usedMb);
  const appPct = percentOf(app, totalMb);
  const otherPct = percentOf(usedMb - app, totalMb);
  return (
    <div className="lv-mem__stack" role="presentation">
      <span
        style={{
          width: `${appPct}%`,
          background: "var(--colorBrandBackground)",
        }}
      />
      <span
        style={{
          width: `${otherPct}%`,
          background: "var(--colorNeutralForeground3)",
        }}
      />
    </div>
  );
};

/**
 * RAM und GPU-Speicher in der Fußleiste: zwei Säulen 7×14 px mit Kürzel,
 * Klick öffnet das Speicher-Popover (Verteilung, „Speicher freigeben“,
 * Speicherschutz). Beschriftet, wie gemessen wird: Auf Windows ist die
 * GPU-Zahl das Budget des Treibers und seine Belegung, kein „freier VRAM“;
 * eine iGPU bekommt den Zusatz „gemeinsam“. Ist nichts messbar, verschwindet
 * die Anzeige, statt eine Zahl zu erfinden.
 */
export const ResourceMeter: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting } = useSettings();
  const [memory, setMemory] = useState<SystemMemory | null>(null);
  const [open, setOpen] = useState(false);
  const [releasing, setReleasing] = useState(false);
  const [releaseNote, setReleaseNote] = useState<string | null>(null);

  const guardEnabled = getSetting("memory_guard_enabled") ?? true;
  const guardPercent = getSetting("memory_guard_percent") ?? GUARD_DEFAULT;
  const [sliderValue, setSliderValue] = useState(guardPercent);
  const sliderTimer = useRef<number | undefined>(undefined);
  useEffect(() => setSliderValue(guardPercent), [guardPercent]);
  useEffect(() => () => window.clearTimeout(sliderTimer.current), []);

  useEffect(() => {
    let cancelled = false;
    let running = false;
    const tick = async () => {
      // Versteckte Fenster nicht messen; ein langsamer Tick blockiert den nächsten.
      if (document.hidden || running) return;
      running = true;
      try {
        const result = await commands.systemMemory();
        if (!cancelled && result.status === "ok") setMemory(result.data);
      } catch {
        // Ohne Backend (Browser-Test) bleibt die Anzeige leer.
      } finally {
        running = false;
      }
    };
    void tick();
    const timer = window.setInterval(() => void tick(), POLL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  if (!memory || memory.ram_total_mb === 0) return null;

  // Die erste Grafikkarte mit eigenem Speicher — die, auf der Modelle
  // laufen. Gibt es nur eine iGPU, wird die gezeigt, aber als gemeinsam.
  const gpu = memory.gpus.find((g) => !g.shared) ?? memory.gpus[0] ?? null;

  // Anteil der App (samt Modell-Servern). Fehlt er oder ist er nicht
  // messbar (`null`), steht nur die Systemzahl da — keine erfundene Null.
  const appRam =
    typeof memory.app_ram_mb === "number" && memory.app_ram_mb > 0
      ? memory.app_ram_mb
      : null;
  const appGpu =
    gpu && !gpu.shared && typeof memory.app_gpu_mb === "number"
      ? memory.app_gpu_mb
      : null;

  const ramPct = percentOf(memory.ram_used_mb, memory.ram_total_mb);
  const gpuPct = gpu ? percentOf(gpu.used_mb, gpu.budget_mb) : 0;

  const ramText = t(
    appRam === null ? "resourceMeter.ram" : "resourceMeter.ramApp",
    {
      used: gb(memory.ram_used_mb),
      total: gb(memory.ram_total_mb),
      app: appRam === null ? "" : gb(appRam),
    },
  );
  const gpuText = gpu
    ? t(
        gpu.shared
          ? "resourceMeter.gpuShared"
          : appGpu === null
            ? "resourceMeter.gpu"
            : "resourceMeter.gpuApp",
        {
          used: gb(gpu.used_mb),
          total: gb(gpu.budget_mb),
          app: appGpu === null ? "" : gb(appGpu),
        },
      )
    : null;

  const release = async () => {
    setReleasing(true);
    setReleaseNote(null);
    try {
      const result = await commands.releaseMemory();
      if (result.status === "ok") {
        setReleaseNote(
          result.data > 0
            ? t("resourceMeter.released", { mb: result.data })
            : t("resourceMeter.releasedNothing"),
        );
      } else {
        setReleaseNote(t("resourceMeter.releaseFailed"));
      }
    } catch {
      setReleaseNote(t("resourceMeter.releaseFailed"));
    } finally {
      setReleasing(false);
    }
  };

  const commitSlider = (value: number) => {
    setSliderValue(value);
    window.clearTimeout(sliderTimer.current);
    sliderTimer.current = window.setTimeout(
      () => void updateSetting("memory_guard_percent", value),
      250,
    );
  };

  const otherRam = Math.max(
    0,
    memory.ram_used_mb - (appRam === null ? 0 : appRam),
  );
  const freeRam = Math.max(0, memory.ram_total_mb - memory.ram_used_mb);

  return (
    <Popover
      open={open}
      onOpenChange={(_e, data) => setOpen(data.open)}
      positioning="above-end"
      withArrow
    >
      <FluentTooltip
        relationship="description"
        withArrow
        positioning="above-end"
        visible={open ? false : undefined}
        content={
          <div className="lv-roletip">
            <div>{ramText}</div>
            {gpuText && <div>{gpuText}</div>}
            <div className="lv-roletip__purpose">{t("resourceMeter.hint")}</div>
          </div>
        }
      >
        <PopoverTrigger disableButtonEnhancement>
          <button
            type="button"
            className="lv-status-btn"
            data-resource-meter
            aria-label={[ramText, gpuText].filter(Boolean).join(" · ")}
            aria-haspopup="dialog"
            aria-expanded={open}
            title={undefined}
          >
            <span className="lv-meter">
              <Column
                label={t("resourceMeter.labelRam")}
                system={ramPct}
                app={
                  appRam === null
                    ? null
                    : percentOf(appRam, memory.ram_total_mb)
                }
                limit={guardEnabled ? guardPercent : null}
                testId="ram"
              />
              {gpu && (
                <Column
                  label={t("resourceMeter.labelGpu")}
                  system={gpuPct}
                  app={
                    appGpu === null ? null : percentOf(appGpu, gpu.budget_mb)
                  }
                  limit={null}
                  testId="gpu"
                />
              )}
            </span>
          </button>
        </PopoverTrigger>
      </FluentTooltip>
      <PopoverSurface aria-label={t("resourceMeter.open")}>
        <div className="lv-mem" data-memory-popover>
          <h3>{t("resourceMeter.ramTitle")}</h3>
          <Stack
            appMb={appRam}
            usedMb={memory.ram_used_mb}
            totalMb={memory.ram_total_mb}
          />
          <div className="lv-mem__legend">
            {appRam !== null && (
              <span>
                <i
                  className="lv-mem__dot"
                  style={{ background: "var(--colorBrandBackground)" }}
                />
                {t("resourceMeter.legendApp")} <strong>{gb(appRam)} GB</strong>
              </span>
            )}
            <span>
              <i
                className="lv-mem__dot"
                style={{ background: "var(--colorNeutralForeground3)" }}
              />
              {t("resourceMeter.legendOther")} {gb(otherRam)} GB
            </span>
            <span>
              <i
                className="lv-mem__dot"
                style={{ background: "var(--colorNeutralStroke2)" }}
              />
              {t("resourceMeter.legendFree")} {gb(freeRam)} GB
            </span>
          </div>
          <div className="lv-mem__row">
            <span>{t("resourceMeter.system")}</span>
            <span>
              {t("resourceMeter.systemValue", {
                used: gb(memory.ram_used_mb),
                total: gb(memory.ram_total_mb),
                percent: Math.round(ramPct),
              })}
            </span>
          </div>

          {gpu && (
            <>
              <h3>{t("resourceMeter.gpuTitle")}</h3>
              <Stack
                appMb={appGpu}
                usedMb={gpu.used_mb}
                totalMb={gpu.budget_mb}
              />
              <div className="lv-mem__row">
                <span>
                  {gpu.shared
                    ? `${gpu.name} · ${t("resourceMeter.gpuSharedNote")}`
                    : t("resourceMeter.gpuBudget", { name: gpu.name })}
                </span>
                <span>
                  {t("resourceMeter.gpuValue", {
                    used: gb(gpu.used_mb),
                    total: gb(gpu.budget_mb),
                    percent: Math.round(gpuPct),
                  })}
                </span>
              </div>
            </>
          )}

          <hr className="lv-mem__sep" />
          <div className="lv-mem__row" style={{ alignItems: "center" }}>
            <span aria-live="polite">
              {releaseNote ?? t("resourceMeter.releaseHint")}
            </span>
            <FluentButton
              size="small"
              onClick={() => void release()}
              disabled={releasing}
              data-release-memory
            >
              {releasing
                ? t("resourceMeter.releasing")
                : t("resourceMeter.release")}
            </FluentButton>
          </div>

          <hr className="lv-mem__sep" />
          <div
            className="lv-mem__row"
            style={{ alignItems: "center", color: "inherit" }}
          >
            <h3>{t("resourceMeter.guardTitle")}</h3>
            <FluentSwitch
              checked={guardEnabled}
              onChange={(_e, data) =>
                void updateSetting("memory_guard_enabled", data.checked)
              }
              aria-label={t("resourceMeter.guardSwitch")}
              data-memory-guard-switch
            />
          </div>
          {guardEnabled && (
            <>
              <div className="lv-mem__row">
                <span id="lv-guard-limit">{t("resourceMeter.guardLimit")}</span>
                <strong>{sliderValue} %</strong>
              </div>
              <FluentSlider
                min={GUARD_MIN}
                max={GUARD_MAX}
                step={1}
                value={sliderValue}
                onChange={(_e, data) => commitSlider(data.value)}
                aria-labelledby="lv-guard-limit"
                data-memory-guard-slider
              />
              <p style={{ margin: 0, color: "var(--colorNeutralForeground2)" }}>
                {t("resourceMeter.guardExplain")}
              </p>
            </>
          )}
          {!guardEnabled && (
            <MessageBar intent="warning" data-memory-guard-warning>
              <MessageBarBody>
                {t("resourceMeter.guardOffWarning")}
              </MessageBarBody>
            </MessageBar>
          )}
        </div>
      </PopoverSurface>
    </Popover>
  );
};

export default ResourceMeter;
