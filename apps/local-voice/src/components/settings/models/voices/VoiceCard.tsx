import React from "react";
import { useTranslation } from "react-i18next";
import {
  Check,
  Cpu,
  Download,
  Loader2,
  Play,
  Server,
  Settings2,
  TriangleAlert,
  Wrench,
  Zap,
} from "lucide-react";
import { formatModelSize } from "@/lib/utils/format";
import { Button } from "../../../ui/Button";
import { RowChip } from "../ModelRow";
import { SamplePlayButton } from "../../tts/providers/SamplePlayButton";
import { VoiceTryButton } from "../../tts/providers/VoiceTryButton";
import type { DeviceGroup, VoiceItem } from "./model";
import { useVoiceLabels } from "./labels";
import type { VoiceCatalog } from "./useVoiceCatalog";

export interface VoiceActions {
  catalog: VoiceCatalog;
  openProvider: (providerId: string) => void;
}

const DEVICE_ICON: Record<
  DeviceGroup,
  React.ComponentType<{ className?: string }>
> = {
  cpu: Cpu,
  gpu: Zap,
  server: Server,
};

/** Hoerprobe der Stimme: fertige Probe, erzeugte Probe oder ein gesperrter Knopf mit Grund. */
export const VoicePlay: React.FC<{ voice: VoiceItem }> = ({ voice }) => {
  const { t } = useTranslation();
  if (voice.samplePath !== undefined) {
    return (
      <SamplePlayButton
        media
        path={voice.samplePath}
        name={voice.name}
        testId={`voice-play-${voice.key}`}
      />
    );
  }
  if (voice.tryTarget) {
    return (
      <VoiceTryButton
        media
        target={voice.tryTarget}
        name={voice.name}
        testId={`voice-play-${voice.key}`}
      />
    );
  }
  const reason =
    voice.state === "load" || voice.state === "downloading"
      ? t("settings.models.voiceManager.voice.sampleAfterLoad")
      : voice.state === "setup" || voice.state === "runtime"
        ? t("settings.models.voiceManager.voice.sampleAfterSetup")
        : t("settings.models.voiceManager.voice.noSample");
  return (
    <button
      type="button"
      disabled
      aria-label={`${t("tts.providers.voices.sample.play", { name: voice.name })} – ${reason}`}
      title={reason}
      className="mbtn mbtn--primary opacity-40"
      style={
        { "--_mb-size": "44px", "--_mb-ico": "1.1rem" } as React.CSSProperties
      }
    >
      <Play aria-hidden="true" />
    </button>
  );
};

/** Chips einer Stimme: Geraet, Lizenz, Besonderheiten. */
export const VoiceChips: React.FC<{ voice: VoiceItem }> = ({ voice }) => {
  const { t } = useTranslation();
  const label = useVoiceLabels();
  return (
    <>
      {voice.devices.map((d) => {
        const Icon = DEVICE_ICON[d];
        return (
          <RowChip key={d}>
            <Icon className="h-3 w-3" />
            {label.device(d)}
          </RowChip>
        );
      })}
      {voice.license === "nonCommercial" && (
        <RowChip tone="warning" title={voice.licenseName ?? undefined}>
          <TriangleAlert className="h-3 w-3" aria-hidden="true" />
          {label.license("nonCommercial")}
        </RowChip>
      )}
      {voice.license === "unverified" && (
        <RowChip tone="warning" title={voice.licenseName ?? undefined}>
          {label.license("unverified")}
        </RowChip>
      )}
      {voice.license === "free" && (
        <RowChip tone="success" title={voice.licenseName ?? undefined}>
          {label.license("free")}
        </RowChip>
      )}
      {voice.hiddenInReadAloud && (
        <RowChip>{t("settings.models.voiceManager.voice.hidden")}</RowChip>
      )}
    </>
  );
};

/** Die Hauptaktion einer Stimme je nach Zustand. */
export const VoiceAction: React.FC<{
  voice: VoiceItem;
  actions: VoiceActions;
  fullWidth?: boolean;
}> = ({ voice, actions, fullWidth = false }) => {
  const { t } = useTranslation();
  const { catalog, openProvider } = actions;
  const cls = `${fullWidth ? "w-full" : ""} min-h-[44px] md:min-h-8 flex items-center justify-center gap-1.5`;
  const name = voice.name;

  if (voice.selected && voice.state === "ready") {
    return (
      <span
        className={`${fullWidth ? "w-full " : ""}inline-flex min-h-[44px] items-center gap-1.5 text-sm font-medium text-logo-primary md:min-h-8`}
        data-testid={`voice-selected-${voice.key}`}
      >
        <Check className="h-4 w-4" aria-hidden="true" />
        {t("settings.models.voiceManager.voice.selected")}
      </span>
    );
  }
  switch (voice.state) {
    case "ready":
      return (
        <Button
          type="button"
          variant="secondary"
          size="sm"
          className={cls}
          aria-label={t("settings.models.voiceManager.voice.useNamed", {
            name,
          })}
          data-testid={`voice-use-${voice.key}`}
          onClick={() => catalog.select(voice)}
        >
          {t("settings.models.voiceManager.voice.use")}
        </Button>
      );
    case "load":
      return voice.downloadId ? (
        <Button
          type="button"
          variant="secondary"
          size="sm"
          className={cls}
          aria-label={t("settings.models.voiceManager.voice.loadNamed", {
            name,
            size: formatModelSize(voice.sizeMb ?? 0),
          })}
          data-testid={`voice-load-${voice.key}`}
          onClick={() => void catalog.install(voice.downloadId as string)}
        >
          <Download className="h-3.5 w-3.5" aria-hidden="true" />
          {t("settings.models.voiceManager.voice.load", {
            size: formatModelSize(voice.sizeMb ?? 0),
          })}
        </Button>
      ) : null;
    case "downloading": {
      const progress = catalog.progressOf(voice.downloadId ?? "");
      return (
        <span
          className="inline-flex min-h-[44px] items-center gap-1.5 text-xs text-text/70 md:min-h-8"
          role="status"
        >
          <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />
          {progress === "verifying"
            ? t("modelSelector.verifyingGeneric")
            : t("settings.models.row.downloading", {
                percentage: Math.round(progress),
              })}
        </span>
      );
    }
    case "runtime":
      return catalog.runtimeId ? (
        <Button
          type="button"
          variant="secondary"
          size="sm"
          className={cls}
          data-testid={`voice-runtime-${voice.key}`}
          onClick={() => void catalog.install(catalog.runtimeId as string)}
        >
          <Wrench className="h-3.5 w-3.5" aria-hidden="true" />
          {t("settings.models.ttsVoices.actions.installRuntime")}
        </Button>
      ) : null;
    case "setup":
      return (
        <Button
          type="button"
          variant="secondary"
          size="sm"
          className={cls}
          data-testid={`voice-setup-${voice.key}`}
          onClick={() => openProvider(voice.providerId)}
        >
          <TriangleAlert className="h-3.5 w-3.5" aria-hidden="true" />
          {t("settings.models.voiceManager.voice.setUp")}
        </Button>
      );
    default:
      return null;
  }
};

const StateChips: React.FC<{ voice: VoiceItem }> = ({ voice }) => {
  const { t } = useTranslation();
  if (voice.state === "load")
    return (
      <RowChip>
        <Download className="h-3 w-3" aria-hidden="true" />
        {t("settings.models.voiceManager.voice.notLoaded")}
      </RowChip>
    );
  if (voice.state === "runtime")
    return (
      <RowChip tone="warning">
        {t("settings.models.ttsVoices.status.runtimeMissing")}
      </RowChip>
    );
  if (voice.state === "unsupported")
    return (
      <RowChip tone="warning">
        {t("settings.models.ttsVoices.status.unsupported")}
      </RowChip>
    );
  if (voice.state === "setup")
    return (
      <RowChip tone="warning">
        {t("settings.models.voiceManager.voice.providerNotReady")}
      </RowChip>
    );
  return null;
};

/** Knopf "Anbieter oeffnen" (Seitenblatt): 44 px auf Touch-Geraeten. */
const OpenProviderButton: React.FC<{
  voice: VoiceItem;
  actions: VoiceActions;
}> = ({ voice, actions }) => {
  const { t } = useTranslation();
  const label = t("settings.models.voiceManager.voice.openProvider", {
    name: voice.providerLabel,
  });
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      data-testid="voice-open-provider"
      onClick={() => actions.openProvider(voice.providerId)}
      className="inline-flex h-[44px] w-[44px] shrink-0 items-center justify-center rounded-md text-text/60 hover:bg-mid-gray/15 hover:text-text focus-visible:outline-2 focus-visible:outline-logo-primary md:h-8 md:w-8"
    >
      <Settings2 className="h-4 w-4" aria-hidden="true" />
    </button>
  );
};

/** Stimmkarte: Hoerprobe, Name, Anbieter, Sprache, Chips, Einschaetzung, Aktion. */
export const VoiceCard: React.FC<{
  voice: VoiceItem;
  actions: VoiceActions;
}> = ({ voice, actions }) => {
  const { t } = useTranslation();
  const label = useVoiceLabels();
  const gender = label.gender(voice.gender);
  const meta = [voice.providerLabel, label.language(voice.language), gender]
    .filter(Boolean)
    .join(" · ");
  return (
    <li
      className={`flex flex-col gap-2 rounded-lg border p-3 ${
        voice.selected
          ? "border-logo-primary bg-logo-primary/5"
          : "border-mid-gray/25"
      }`}
      data-voice-card={voice.key}
      data-state={voice.state}
      data-selected={voice.selected ? "true" : undefined}
    >
      <div className="flex items-start gap-2">
        <VoicePlay voice={voice} />
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-sm font-semibold text-text">
            {voice.name}
          </h3>
          <p className="truncate text-xs text-text/60">{meta}</p>
        </div>
        <OpenProviderButton voice={voice} actions={actions} />
      </div>
      <div className="flex flex-wrap items-center gap-1.5">
        <VoiceChips voice={voice} />
        <StateChips voice={voice} />
      </div>
      <p className="rounded-md bg-mid-gray/10 px-2 py-1.5 text-xs text-text/60">
        <span className="font-medium">
          {t("settings.models.voiceManager.voice.assessment")}:
        </span>{" "}
        {t("settings.models.voiceManager.voice.quality")}{" "}
        <em>{t("settings.models.voiceManager.voice.notMeasured")}</em>
        {" · "}
        {t("settings.models.voiceManager.voice.effort")}{" "}
        <em>{t("settings.models.voiceManager.voice.notMeasured")}</em>
      </p>
      {voice.description && (
        <p className="line-clamp-3 text-sm text-text/80">{voice.description}</p>
      )}
      <div className="mt-auto pt-1">
        <VoiceAction voice={voice} actions={actions} fullWidth />
      </div>
    </li>
  );
};

/** Zeile der Listenansicht: dieselben Angaben, eine Zeile hoch. */
export const VoiceListRow: React.FC<{
  voice: VoiceItem;
  actions: VoiceActions;
}> = ({ voice, actions }) => {
  const { t } = useTranslation();
  const label = useVoiceLabels();
  const gender = label.gender(voice.gender);
  const meta = [voice.providerLabel, label.language(voice.language), gender]
    .filter(Boolean)
    .join(" · ");
  return (
    <li
      className={`flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-mid-gray/15 px-3 py-2 last:border-b-0 ${
        voice.selected ? "bg-logo-primary/5" : ""
      }`}
      data-voice-row={voice.key}
      data-state={voice.state}
      data-selected={voice.selected ? "true" : undefined}
    >
      <VoicePlay voice={voice} />
      <div className="min-w-0 flex-1 basis-40">
        <p className="truncate text-sm font-medium text-text">{voice.name}</p>
        <p className="truncate text-xs text-text/60">{meta}</p>
      </div>
      <div className="flex flex-wrap items-center gap-1.5">
        <VoiceChips voice={voice} />
        <StateChips voice={voice} />
      </div>
      <div className="ms-auto flex items-center gap-1">
        <VoiceAction voice={voice} actions={actions} />
        <OpenProviderButton voice={voice} actions={actions} />
      </div>
    </li>
  );
};
